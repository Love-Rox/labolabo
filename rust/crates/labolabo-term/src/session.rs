//! The backend-independent terminal session: PTY spawn (`portable-pty`), the
//! read/parse/snapshot machinery, and the public [`TermSession`] API.
//!
//! ## Threading model (the spike's M6 fix, generalized)
//!
//! Two threads per session, plus the caller's thread:
//!
//! - **Reader thread**: a *tight* blocking `read()` loop on the PTY master,
//!   forwarding every chunk to the worker over a channel. It never sleeps and
//!   is never throttled, so PTY backpressure reflects real consumption speed.
//!   The spike (M6, bug #2) found that folding a per-frame `sleep` into the
//!   read loop capped throughput at `pty_buffer / frame_interval` regardless
//!   of real parsing speed; keeping reads tight and pacing *only* snapshot
//!   construction is the fix, which this split makes structural.
//! - **Worker thread**: owns the [`VtBackend`] VT core. It `recv`s a single
//!   [`WorkerMsg`] stream carrying both PTY bytes (from the reader) *and*
//!   resize requests (from the caller). Because it blocks on one channel, a
//!   resize is applied promptly even while no PTY output is arriving -- the
//!   reason bytes and resizes share one channel instead of the worker
//!   blocking directly on `read()`. Snapshot construction (the expensive FFI
//!   cell-walk) is throttled to at most once per [`FRAME_INTERVAL`].
//!
//! The caller holds a [`TermSession`], which is `Send + Sync`: it exposes the
//! latest snapshot, a wakeup/exit event channel, input writing, and resize.

use std::io::{Read, Write};
use std::marker::PhantomData;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};

use crate::backend::VtBackend;
use crate::color::ColorScheme;
use crate::snapshot::GridSnapshot;

/// Shared, mutex-guarded PTY writer. Both the caller's [`TermSession::
/// write_input`] (keystrokes) and the backend's VT-response callback (query
/// replies) write through this single handle -- `take_writer` may be called
/// only once per PTY master, so they share it.
pub type SharedWriter = Arc<Mutex<Box<dyn Write + Send>>>;

/// Minimum gap between two snapshot builds, no matter how fast the PTY
/// produces data -- a ~60fps ceiling enforced at the source (before the
/// expensive cell-walk). Only throttles *snapshot cadence*; the reader thread
/// still drains the PTY as fast as the producer fills it.
const FRAME_INTERVAL: Duration = Duration::from_millis(16);

const READ_BUF_SIZE: usize = 64 * 1024;

/// A notification that something changed. Coalesced: one `Wakeup` may cover
/// many underlying grid updates. Pull the actual content via
/// [`TermSession::snapshot`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TermEvent {
    /// A new [`GridSnapshot`] is available.
    Wakeup,
    /// The child process closed the PTY (exited). A final snapshot has
    /// already been published before this event.
    Exit,
}

/// Internal: the single stream the worker consumes.
enum WorkerMsg {
    /// PTY output bytes from the reader thread.
    Bytes(Vec<u8>),
    /// A resize request from the caller (cols, rows).
    Resize(u16, u16),
    /// The reader hit EOF/error on the PTY master.
    Eof,
}

/// A live terminal session backed by `B`'s VT core.
///
/// This is the unified, backend-independent API required by the design: one
/// generic type whose behavior is identical across backends. Prefer the
/// resolved alias [`crate::Terminal`] (which picks the active backend by
/// feature) so call sites -- including the shared integration tests -- name no
/// backend at all.
pub struct TermSession<B: VtBackend> {
    writer: SharedWriter,
    master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    latest: Arc<Mutex<Arc<GridSnapshot>>>,
    events: Mutex<Receiver<TermEvent>>,
    worker_tx: Mutex<Sender<WorkerMsg>>,
    /// A kill handle split off the child (`Child::clone_killer`) so
    /// [`Self::shutdown`] can signal it even though the `Child` itself is
    /// owned by (and reaped on) the worker thread.
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    // `fn() -> B` keeps `TermSession<B>: Send + Sync` even when `B` is neither
    // (the ghostty backend's VT core is `!Send`): `B` never actually lives in
    // this struct -- it lives on the worker thread.
    _backend: PhantomData<fn() -> B>,
}

impl<B: VtBackend> TermSession<B> {
    /// Spawn a PTY sized `cols` x `rows` and start the read/parse machinery.
    ///
    /// - `command`: `Some(cmd)` execs `/bin/sh -c <cmd>` directly as the
    ///   child (the equivalent of `ghostty -e` -- no login shell, no typed
    ///   input); `None` launches the platform default shell
    ///   (`CommandBuilder::new_default_prog`, i.e. `$SHELL`).
    /// - `env`: extra environment variables injected into the child *on top
    ///   of* `TERM=xterm-256color`. First-class because LaboLabo's hooks
    ///   protocol identifies a pane/task by env (`LABOLABO_PANE`,
    ///   `LABOLABO_TASK`, ...) handed to the spawned agent.
    ///
    /// Equivalent to [`Self::spawn_with_options`] with
    /// `ColorScheme::default()` (every backend's own built-in colors,
    /// unchanged) -- kept as a separate, narrower entry point so existing
    /// call sites that don't care about color configuration aren't forced to
    /// thread a `ColorScheme` through.
    pub fn spawn_with_command(
        cols: u16,
        rows: u16,
        command: Option<&str>,
        env: &[(String, String)],
    ) -> anyhow::Result<Self> {
        Self::spawn_with_options(cols, rows, command, env, &ColorScheme::default())
    }

    /// Like [`Self::spawn_with_command`], with an additional [`ColorScheme`]
    /// seeding the VT core's default foreground/background/cursor/palette
    /// colors (see [`crate::backend::VtBackend::new`]). Pass
    /// `&ColorScheme::default()` for the same behavior as
    /// `spawn_with_command`.
    pub fn spawn_with_options(
        cols: u16,
        rows: u16,
        command: Option<&str>,
        env: &[(String, String)],
        colors: &ColorScheme,
    ) -> anyhow::Result<Self> {
        let mut cmd = match command {
            Some(c) => {
                let mut cmd = CommandBuilder::new("/bin/sh");
                cmd.arg("-c");
                cmd.arg(c);
                cmd
            }
            None => CommandBuilder::new_default_prog(),
        };
        cmd.env("TERM", "xterm-256color");
        for (key, value) in env {
            cmd.env(key, value);
        }

        let pty_system = native_pty_system();
        let pair = pty_system.openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;

        let child = pair.slave.spawn_command(cmd)?;
        // Split a kill handle off the child now -- the `Child` itself moves to
        // the worker thread (which blocks in `wait()` to reap it), and
        // `clone_killer` exists precisely to signal a child owned elsewhere.
        let killer = child.clone_killer();
        // Drop our copy of the slave once the child has it -- otherwise our own
        // process keeps the slave fd open and the reader never sees EOF when
        // the child exits.
        drop(pair.slave);

        let reader = pair.master.try_clone_reader()?;
        let writer: SharedWriter = Arc::new(Mutex::new(pair.master.take_writer()?));
        // Keep the master alive (dropping it would close the PTY) and reachable
        // so `resize` can issue the kernel `TIOCSWINSZ`.
        let master = Arc::new(Mutex::new(pair.master));

        let latest = Arc::new(Mutex::new(Arc::new(GridSnapshot::blank(cols, rows))));
        let (event_tx, event_rx) = mpsc::channel::<TermEvent>();
        let (worker_tx, worker_rx) = mpsc::channel::<WorkerMsg>();

        // Reader thread: tight, unthrottled blocking read loop.
        {
            let reader_tx = worker_tx.clone();
            let mut reader = reader;
            thread::spawn(move || {
                let mut buf = vec![0u8; READ_BUF_SIZE];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => {
                            let _ = reader_tx.send(WorkerMsg::Eof);
                            break;
                        }
                        Ok(n) => {
                            if reader_tx.send(WorkerMsg::Bytes(buf[..n].to_vec())).is_err() {
                                break;
                            }
                        }
                        Err(_) => {
                            let _ = reader_tx.send(WorkerMsg::Eof);
                            break;
                        }
                    }
                }
            });
        }

        // Worker thread: owns the VT core, builds throttled snapshots.
        {
            let writer = writer.clone();
            let latest = latest.clone();
            let colors = colors.clone();
            thread::spawn(move || {
                run_worker::<B>(
                    cols, rows, writer, latest, worker_rx, event_tx, child, colors,
                );
            });
        }

        Ok(Self {
            writer,
            master,
            latest,
            events: Mutex::new(event_rx),
            worker_tx: Mutex::new(worker_tx),
            killer: Mutex::new(killer),
            _backend: PhantomData,
        })
    }

    /// Convenience: spawn the default shell with no extra env.
    pub fn spawn(cols: u16, rows: u16) -> anyhow::Result<Self> {
        Self::spawn_with_command(cols, rows, None, &[])
    }

    /// Write bytes to the PTY (i.e. "typed" into the child). Thread-safe --
    /// this is a plain mutex-guarded `Write`, not an OS keyboard event.
    pub fn write_input(&self, bytes: &[u8]) {
        if let Ok(mut w) = self.writer.lock() {
            let _ = w.write_all(bytes);
        }
    }

    /// Resize both the PTY (kernel `TIOCSWINSZ`, so full-screen programs see
    /// the new size via `SIGWINCH`) and the VT core (on the worker thread).
    pub fn resize(&self, cols: u16, rows: u16) {
        if let Ok(master) = self.master.lock() {
            let _ = master.resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            });
        }
        if let Ok(tx) = self.worker_tx.lock() {
            let _ = tx.send(WorkerMsg::Resize(cols, rows));
        }
    }

    /// Terminate the session's child process.
    ///
    /// Signals the child via `portable-pty`'s `ChildKiller` -- on Unix that
    /// is **SIGHUP** (the "terminal hung up" signal a real terminal emulator
    /// delivers on window close; default disposition terminates a shell), on
    /// Windows `TerminateProcess`. Everything else then follows the normal
    /// exit path -- there is no special teardown state: the dying child
    /// closes the PTY slave, the reader thread sees EOF, and the worker
    /// publishes a final snapshot, emits [`TermEvent::Exit`], reaps the
    /// child (`wait`), and both threads finish. Callers that care when
    /// teardown completes should wait for the `Exit` event, same as for a
    /// natural exit.
    ///
    /// Idempotent; calling it again (or after the child already exited) is
    /// harmless in practice -- kill errors are ignored. Caveat, inherited
    /// from `portable-pty`'s pid-based Unix killer: once the child has been
    /// reaped (which only the worker thread's `wait` does, after EOF), a
    /// subsequent `shutdown` signals a stale pid, with the usual theoretical
    /// pid-reuse window every pid-signalling terminal emulator shares.
    ///
    /// Note this signals only the direct child process (the shell), not its
    /// whole process group -- descendants that detach from the PTY and
    /// ignore SIGHUP can outlive the session, same as in other terminals.
    pub fn shutdown(&self) {
        if let Ok(mut killer) = self.killer.lock() {
            let _ = killer.kill();
        }
    }

    /// The most recent grid snapshot. Cheap (`Arc` clone). Never blocks on the
    /// worker -- returns the last one published.
    pub fn snapshot(&self) -> Arc<GridSnapshot> {
        match self.latest.lock() {
            Ok(g) => g.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    /// Wait up to `timeout` for the next [`TermEvent`], or `None` on timeout.
    pub fn recv_event(&self, timeout: Duration) -> Option<TermEvent> {
        let rx = self.events.lock().ok()?;
        rx.recv_timeout(timeout).ok()
    }

    /// Block until `pred` holds for the latest snapshot, or `timeout` elapses.
    ///
    /// Returns the matching snapshot, or `None` if it never matched (including
    /// the case where the child exits first without ever matching). Waits
    /// efficiently on the event channel rather than busy-polling. Useful both
    /// for tests and for a UI that wants to await a particular screen state.
    pub fn wait_for<F>(&self, timeout: Duration, pred: F) -> Option<Arc<GridSnapshot>>
    where
        F: Fn(&GridSnapshot) -> bool,
    {
        let deadline = Instant::now() + timeout;
        loop {
            let snap = self.snapshot();
            if pred(&snap) {
                return Some(snap);
            }
            let remaining = deadline.checked_duration_since(Instant::now())?;
            // Cap each wait so a missed wakeup can't stall past the deadline,
            // and so we re-check promptly after `Exit` (whose final snapshot
            // was published just before the event).
            let slice = remaining.min(Duration::from_millis(50));
            if self.recv_event(slice) == Some(TermEvent::Exit) {
                let snap = self.snapshot();
                return if pred(&snap) { Some(snap) } else { None };
            }
        }
    }
}

/// Publish a freshly-built snapshot: store it as the latest and fire a wakeup.
fn publish_snapshot<B: VtBackend>(
    backend: &mut B,
    latest: &Arc<Mutex<Arc<GridSnapshot>>>,
    event_tx: &Sender<TermEvent>,
) {
    if let Some(snap) = backend.build_snapshot() {
        if let Ok(mut slot) = latest.lock() {
            *slot = Arc::new(snap);
        }
        let _ = event_tx.send(TermEvent::Wakeup);
    }
}

#[allow(clippy::too_many_arguments)]
fn run_worker<B: VtBackend>(
    cols: u16,
    rows: u16,
    writer: SharedWriter,
    latest: Arc<Mutex<Arc<GridSnapshot>>>,
    rx: Receiver<WorkerMsg>,
    event_tx: Sender<TermEvent>,
    mut child: Box<dyn portable_pty::Child + Send + Sync>,
    colors: ColorScheme,
) {
    let mut backend = match B::new(cols, rows, writer, &colors) {
        Ok(b) => b,
        Err(_) => return,
    };

    // Start one interval in the past so the very first output snapshots
    // immediately instead of being swallowed by the throttle.
    let mut last_snapshot = Instant::now() - FRAME_INTERVAL;

    while let Ok(msg) = rx.recv() {
        match msg {
            WorkerMsg::Bytes(bytes) => {
                backend.feed(&bytes);
                if last_snapshot.elapsed() >= FRAME_INTERVAL {
                    last_snapshot = Instant::now();
                    publish_snapshot(&mut backend, &latest, &event_tx);
                }
            }
            WorkerMsg::Resize(c, r) => {
                backend.resize(c, r);
                // Always snapshot on resize: the dimensions changed, and there
                // may be no PTY output to otherwise trigger a redraw.
                last_snapshot = Instant::now();
                publish_snapshot(&mut backend, &latest, &event_tx);
            }
            WorkerMsg::Eof => {
                // Force a final snapshot regardless of throttle, so the last
                // burst of output before exit is never lost to frame-pacing.
                publish_snapshot(&mut backend, &latest, &event_tx);
                let _ = event_tx.send(TermEvent::Exit);
                break;
            }
        }
    }

    // Reap the child so it doesn't linger as a zombie.
    let _ = child.wait();
}
