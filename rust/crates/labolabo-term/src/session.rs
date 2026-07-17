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
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};

use crate::backend::VtBackend;
use crate::color::ColorScheme;
use crate::mouse::MouseMode;
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

/// Default scrollback cap (lines of history retained past the live
/// viewport), used by every `spawn_*` entry point that doesn't take an
/// explicit `max_scrollback` -- see [`TermSession::spawn_with_scrollback_options`].
/// Both backends previously hardcoded this same value; `labolabo-app`'s
/// settings screen is the first caller to ever pass something else.
pub const DEFAULT_MAX_SCROLLBACK: usize = 1000;

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
    /// A scroll-by-`delta_lines` request from the caller -- see
    /// [`TermSession::scroll`]/[`crate::backend::VtBackend::
    /// scroll_display`] for the sign convention.
    Scroll(i64),
    /// A "jump to the live tail" request -- see [`TermSession::
    /// scroll_to_bottom`].
    ScrollToBottom,
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
    /// Mirrors `VtBackend::bracketed_paste` for the caller thread -- updated
    /// by the worker after every processed PTY byte batch (see
    /// `run_worker`), read non-blockingly by [`Self::bracketed_paste`].
    bracketed_paste: Arc<AtomicBool>,
    /// Mirrors `VtBackend::kitty_disambiguate` for the caller thread -- same
    /// refresh cadence/shape as `bracketed_paste` above. Read by
    /// [`Self::kitty_disambiguate`], which `labolabo-app`'s `keys::
    /// keystroke_to_bytes` uses to decide whether a modifier-carrying
    /// Enter/Tab should be re-encoded as a Kitty-protocol `CSI u` sequence.
    kitty_disambiguate: Arc<AtomicBool>,
    /// Mirrors `VtBackend::alt_screen_active` for the caller thread -- same
    /// refresh cadence and non-blocking read shape as `bracketed_paste`
    /// above, read by [`Self::alt_screen_active`]. `labolabo-app`'s wheel
    /// handler uses this to decide whether to scroll this crate's own
    /// viewport or send cursor-key sequences instead (see
    /// `VtBackend::alt_screen_active`'s doc comment).
    alt_screen: Arc<AtomicBool>,
    /// Mirrors `VtBackend::alternate_scroll_active` for the caller thread --
    /// same refresh cadence/shape as `alt_screen` above. Read by
    /// [`Self::alternate_scroll_active`].
    alternate_scroll: Arc<AtomicBool>,
    /// Mirrors `VtBackend::mouse_mode` for the caller thread -- same refresh
    /// cadence as `bracketed_paste`/`alt_screen` above (a plain flag the
    /// worker thread refreshes after every processed PTY byte batch), just
    /// packed into a single `AtomicU8` (see `MouseMode::to_bits`/
    /// `from_bits`) rather than an `AtomicBool`, since it carries more than
    /// one bit of state. Read by [`Self::mouse_mode`].
    mouse_mode: Arc<AtomicU8>,
    /// Mirrors `VtBackend::title` for the caller thread -- same refresh
    /// cadence/shape as `bracketed_paste`/`alt_screen`/`mouse_mode` above (a
    /// plain slot the worker thread refreshes after every processed PTY byte
    /// batch), just an `Option<String>` behind a `Mutex` instead of an atomic
    /// bit-packed flag, since a title isn't representable in one atomic word.
    /// Read by [`Self::title`].
    title: Arc<Mutex<Option<String>>>,
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
    /// - `command`: `Some(cmd)` execs a one-shot shell invocation of `cmd`
    ///   directly as the child (the equivalent of `ghostty -e` -- no login
    ///   shell, no typed input) -- `/bin/sh -c <cmd>` on unix,
    ///   `%ComSpec% /C <cmd>` (`cmd.exe`, no PowerShell preference) on
    ///   Windows; `None` launches an interactive default shell -- unix keeps
    ///   `CommandBuilder::new_default_prog` (i.e. `$SHELL`) unchanged, while
    ///   Windows prefers `pwsh.exe` -> `powershell.exe` -> `%ComSpec%`
    ///   (`cmd.exe`) in that order (this module's private `windows::
    ///   default_prog` -- see its doc comment for the rationale).
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
    ///
    /// Equivalent to [`Self::spawn_with_cwd_options`] with `cwd: None` (the
    /// child inherits this process's own working directory, `portable-pty`'s
    /// default) -- kept as a separate, narrower entry point so existing call
    /// sites that don't care about the child's working directory aren't
    /// forced to thread a `cwd` through.
    pub fn spawn_with_options(
        cols: u16,
        rows: u16,
        command: Option<&str>,
        env: &[(String, String)],
        colors: &ColorScheme,
    ) -> anyhow::Result<Self> {
        Self::spawn_with_cwd_options(cols, rows, command, env, colors, None)
    }

    /// Like [`Self::spawn_with_options`], with an additional `cwd`: the
    /// child's initial working directory (`chdir`'d before exec, same as a
    /// real terminal opened in that directory). `None` leaves it unset --
    /// `portable-pty`'s `CommandBuilder` then defaults to this process's own
    /// working directory. This is the mechanism `labolabo-app`'s Task model
    /// (`plans/012-task-model-and-control-cli.md` §1) uses to spawn a Task's
    /// panes in that Task's worktree/attached directory rather than wherever
    /// the app itself happens to be running from.
    ///
    /// The directory is not validated here (no existence/is-a-directory
    /// check) -- an invalid `cwd` surfaces as a spawn failure from the
    /// underlying `CommandBuilder`/PTY exec, same as passing a bogus
    /// executable path would.
    pub fn spawn_with_cwd_options(
        cols: u16,
        rows: u16,
        command: Option<&str>,
        env: &[(String, String)],
        colors: &ColorScheme,
        cwd: Option<&Path>,
    ) -> anyhow::Result<Self> {
        Self::spawn_with_scrollback_options(
            cols,
            rows,
            command,
            env,
            colors,
            cwd,
            DEFAULT_MAX_SCROLLBACK,
        )
    }

    /// Like [`Self::spawn_with_cwd_options`], with an additional
    /// `max_scrollback`: how many lines of history the grid retains past
    /// the live viewport (both backends' `VtBackend::new` -- see that
    /// trait method's doc comment). `labolabo-app`'s settings screen is
    /// this method's only caller that passes anything other than
    /// [`DEFAULT_MAX_SCROLLBACK`] -- every other `spawn_*` entry point
    /// above funnels down to `spawn_with_cwd_options`, which passes that
    /// default, so existing callers/tests are unaffected by this method's
    /// addition.
    pub fn spawn_with_scrollback_options(
        cols: u16,
        rows: u16,
        command: Option<&str>,
        env: &[(String, String)],
        colors: &ColorScheme,
        cwd: Option<&Path>,
        max_scrollback: usize,
    ) -> anyhow::Result<Self> {
        let mut cmd = match command {
            Some(c) => {
                #[cfg(windows)]
                let mut cmd = CommandBuilder::new(windows::comspec());
                #[cfg(not(windows))]
                let mut cmd = CommandBuilder::new("/bin/sh");
                #[cfg(windows)]
                cmd.arg("/C");
                #[cfg(not(windows))]
                cmd.arg("-c");
                cmd.arg(c);
                cmd
            }
            #[cfg(windows)]
            None => match windows::default_prog() {
                Some(shell) => CommandBuilder::new(shell),
                None => CommandBuilder::new_default_prog(),
            },
            #[cfg(not(windows))]
            None => CommandBuilder::new_default_prog(),
        };
        cmd.env("TERM", "xterm-256color");
        // `TERM_PROGRAM=ghostty` -- **not** cosmetic. Real terminal-aware
        // programs (Claude Code's own TUI is the motivating case; see
        // `crate` root docs / `rust/README.md`'s "Wave 15 followup") decide
        // whether to attempt the Kitty keyboard protocol handshake (`CSI >
        // 1 u`, the mechanism `keys::keystroke_to_bytes`'s `kitty_
        // disambiguate` parameter needs to see fire at all) purely from a
        // static terminal-identity allowlist keyed on `TERM_PROGRAM` (and a
        // few backup env vars for terminals that don't set it, like Kitty
        // itself) -- **not** from any live capability probe/query-response
        // round trip. Confirmed by reading Claude Code's own compiled CLI:
        // its terminal-name resolver checks `TERM=="xterm-ghostty"`, then
        // `TERM.includes("kitty")`, then falls back to `TERM_PROGRAM`
        // verbatim; the result is matched against an allowlist that
        // includes `"ghostty"` (also `"iTerm.app"`/`"kitty"`/`"WezTerm"`/
        // `"tmux"`/`"windows-terminal"`/`"WarpTerminal"`) before it will
        // ever write the push sequence to this pane's stdin. Without this,
        // a Kitty-protocol-*capable* terminal (this crate, once `VtBackend
        // ::kitty_disambiguate` landed) still never gets asked to prove
        // it -- the actual root cause of the wave 15 bug report ("Shift+
        // Enter still doesn't work") surviving that wave's own fix, since
        // that fix made this crate *able* to relay a push correctly but
        // did nothing to make Claude Code ever attempt one.
        //
        // `"ghostty"` (not e.g. `"iTerm.app"`) is the honest choice here,
        // not an arbitrary pick to unlock the feature: this crate's
        // intended production `VtBackend` **is** `libghostty-vt`, the same
        // VT engine real Ghostty embeds (see `backend/ghostty.rs`'s module
        // doc comment), and the crates.io-only `backend-alacritty` fallback
        // is deliberately held to the same integration-test contract (see
        // `backend/mod.rs`) specifically so it stays behaviorally
        // interchangeable -- claiming to be Ghostty-compatible is accurate
        // for both, not just whichever backend a given build happens to use.
        //
        // Deliberately `TERM_PROGRAM` only, not also `TERM=xterm-ghostty`:
        // `TERM` drives terminfo/termcap lookups for every program in the
        // child (not just Claude Code), and an `xterm-ghostty` terminfo
        // entry isn't guaranteed to be installed on a machine that has
        // never had real Ghostty on it -- a wrong/missing terminfo entry
        // risks breaking `tput`/ncurses-based programs system-wide for a
        // benefit (this one program's Kitty-protocol detection) that
        // `TERM_PROGRAM` alone already delivers with none of that risk
        // (virtually nothing outside terminal-identity feature-detection
        // reads `TERM_PROGRAM`, unlike `TERM`).
        cmd.env("TERM_PROGRAM", "ghostty");
        for (key, value) in env {
            cmd.env(key, value);
        }
        if let Some(dir) = cwd {
            cmd.cwd(dir);
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
        let bracketed_paste = Arc::new(AtomicBool::new(false));
        let kitty_disambiguate = Arc::new(AtomicBool::new(false));
        let alt_screen = Arc::new(AtomicBool::new(false));
        // Defaults `true` -- matches both backends' own documented default
        // for DECSET 1007 (see `VtBackend::alternate_scroll_active`'s doc
        // comment), so a session's very first snapshot (before any worker
        // update has landed) reports the same value the backend itself
        // would report once queried.
        let alternate_scroll = Arc::new(AtomicBool::new(true));
        let mouse_mode = Arc::new(AtomicU8::new(MouseMode::OFF.to_bits()));
        let title = Arc::new(Mutex::new(None));
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
            let bracketed_paste = bracketed_paste.clone();
            let kitty_disambiguate = kitty_disambiguate.clone();
            let alt_screen = alt_screen.clone();
            let alternate_scroll = alternate_scroll.clone();
            let mouse_mode = mouse_mode.clone();
            let title = title.clone();
            thread::spawn(move || {
                run_worker::<B>(
                    cols,
                    rows,
                    writer,
                    latest,
                    bracketed_paste,
                    kitty_disambiguate,
                    alt_screen,
                    alternate_scroll,
                    mouse_mode,
                    title,
                    worker_rx,
                    event_tx,
                    child,
                    colors,
                    max_scrollback,
                );
            });
        }

        Ok(Self {
            writer,
            master,
            latest,
            bracketed_paste,
            kitty_disambiguate,
            alt_screen,
            alternate_scroll,
            mouse_mode,
            title,
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

    /// Scroll the viewport by `delta_lines` -- see
    /// [`crate::backend::VtBackend::scroll_display`]'s doc comment for the
    /// sign convention (positive = up/into history) and clamping behavior.
    /// Applied on the worker thread; a fresh [`GridSnapshot`] reflecting the
    /// new `scroll_offset` is published (and a [`TermEvent::Wakeup`] fired)
    /// immediately, same as [`Self::resize`] -- there may be no PTY output
    /// to otherwise trigger a redraw.
    pub fn scroll(&self, delta_lines: i64) {
        if let Ok(tx) = self.worker_tx.lock() {
            let _ = tx.send(WorkerMsg::Scroll(delta_lines));
        }
    }

    /// Jump the viewport back to the live tail (`scroll_offset` `0`) --
    /// see [`crate::backend::VtBackend::scroll_to_bottom`]'s doc comment.
    /// `labolabo-app` calls this on every keystroke that reaches the PTY
    /// (the terminal-UI convention: typing while scrolled back returns you
    /// to the live output).
    pub fn scroll_to_bottom(&self) {
        if let Ok(tx) = self.worker_tx.lock() {
            let _ = tx.send(WorkerMsg::ScrollToBottom);
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

    /// Whether bracketed paste mode (DECSET `2004`) is currently enabled --
    /// a cheap, non-blocking read of the flag the worker thread refreshes
    /// after every processed PTY byte batch (see `run_worker` and
    /// `VtBackend::bracketed_paste`). `labolabo-app`'s Cmd+V paste handler
    /// uses this to decide whether to wrap the pasted text in
    /// `ESC[200~...ESC[201~`.
    pub fn bracketed_paste(&self) -> bool {
        self.bracketed_paste.load(Ordering::Relaxed)
    }

    /// Whether the running program has enabled the Kitty keyboard
    /// protocol's "disambiguate escape codes" flag -- a cheap, non-blocking
    /// read of the flag the worker thread refreshes after every processed
    /// PTY byte batch (see `run_worker` and `VtBackend::kitty_disambiguate`).
    /// `labolabo-app`'s `keys::keystroke_to_bytes` uses this to decide
    /// whether a modifier-carrying Enter/Tab (Shift+Enter, Shift+Tab, ...)
    /// should be re-encoded as a Kitty-protocol `CSI u` sequence instead of
    /// its plain legacy byte.
    pub fn kitty_disambiguate(&self) -> bool {
        self.kitty_disambiguate.load(Ordering::Relaxed)
    }

    /// Whether the alternate screen buffer is currently active -- a cheap,
    /// non-blocking read of the flag the worker thread refreshes after
    /// every processed PTY byte batch (see `run_worker` and
    /// `VtBackend::alt_screen_active`). `labolabo-app`'s wheel handler uses
    /// this to decide whether to scroll this crate's own viewport or send
    /// cursor-key sequences to the PTY instead.
    pub fn alt_screen_active(&self) -> bool {
        self.alt_screen.load(Ordering::Relaxed)
    }

    /// Whether "alternate scroll mode" (DECSET `1007`) is currently active
    /// -- a cheap, non-blocking read of the flag the worker thread
    /// refreshes after every processed PTY byte batch (see `run_worker`
    /// and `VtBackend::alternate_scroll_active`, including its doc comment
    /// for the `true` default). `labolabo-app`'s wheel handler checks this
    /// (only once mouse reporting is confirmed off -- see `Self::
    /// mouse_mode`) to decide whether an alt-screen scroll gesture should
    /// convert to cursor-key sequences at all.
    pub fn alternate_scroll_active(&self) -> bool {
        self.alternate_scroll.load(Ordering::Relaxed)
    }

    /// The running program's currently requested mouse-reporting
    /// configuration -- a cheap, non-blocking read of the flag the worker
    /// thread refreshes after every processed PTY byte batch (see
    /// `run_worker` and `VtBackend::mouse_mode`). `labolabo-app`'s
    /// mouse-event routing uses this to decide whether a click/drag/scroll
    /// should be SGR-encoded and forwarded to the child instead of driving
    /// this crate's own text-selection/scrollback UI.
    pub fn mouse_mode(&self) -> MouseMode {
        MouseMode::from_bits(self.mouse_mode.load(Ordering::Relaxed))
    }

    /// The terminal title most recently set by the running program via OSC
    /// `0`/`2` -- a cheap, non-blocking read of the value the worker thread
    /// refreshes after every processed PTY byte batch (see `run_worker` and
    /// [`crate::backend::VtBackend::title`]). `None` means no title has been
    /// set (or it was explicitly reset) since this session was spawned --
    /// never an empty string.
    ///
    /// Purely a live, in-memory mirror of the running program's own state:
    /// not persisted, and lost (reverting to `None`) if the session is torn
    /// down and respawned -- exactly like every mainstream terminal
    /// emulator's own window/tab title. `labolabo-app`'s tab chips prefer
    /// this over the pane's persisted display name whenever it's `Some`,
    /// which is how Claude Code's own conversation title (or a shell's
    /// `\e]0;...\a` prompt) ends up on the tab instead of a generic "端末 N".
    pub fn title(&self) -> Option<String> {
        match self.title.lock() {
            Ok(t) => t.clone(),
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
    bracketed_paste: Arc<AtomicBool>,
    kitty_disambiguate: Arc<AtomicBool>,
    alt_screen: Arc<AtomicBool>,
    alternate_scroll: Arc<AtomicBool>,
    mouse_mode: Arc<AtomicU8>,
    title: Arc<Mutex<Option<String>>>,
    rx: Receiver<WorkerMsg>,
    event_tx: Sender<TermEvent>,
    mut child: Box<dyn portable_pty::Child + Send + Sync>,
    colors: ColorScheme,
    max_scrollback: usize,
) {
    let mut backend = match B::new(cols, rows, writer, &colors, max_scrollback) {
        Ok(b) => b,
        Err(_) => return,
    };

    // Start one interval in the past so the very first output snapshots
    // immediately instead of being swallowed by the throttle.
    let mut last_snapshot = Instant::now() - FRAME_INTERVAL;
    // Whether `backend`'s state has changed since the last *published*
    // snapshot -- sticky across a run of throttled `Bytes` messages. This
    // closes a real gap the throttle above would otherwise leave: if an
    // entire burst of PTY output lands within one `FRAME_INTERVAL` window
    // (routine for anything that prints a lot at once -- `ls`, `cat`, a
    // TUI's initial screen draw -- since the reader thread's `read()`s
    // typically come back within microseconds of each other), every
    // `Bytes` message in that burst can hit the `elapsed() < FRAME_INTERVAL`
    // branch and skip publishing, one after another. If the very next event
    // this worker sees is far in the future (or never comes at all -- e.g.
    // the child then idles at a shell prompt or inside `sleep`), the last,
    // truest state of the burst would otherwise never reach a published
    // `GridSnapshot` until *something else* happens to nudge it (a keypress,
    // a resize, or the child eventually exiting). `dirty` plus the
    // `recv_timeout` below fixes that: once set, the loop wakes up on its
    // own right when the throttle window ends and force-publishes, even
    // with zero new messages.
    let mut dirty = false;

    loop {
        let msg = if dirty {
            let remaining = FRAME_INTERVAL.saturating_sub(last_snapshot.elapsed());
            match rx.recv_timeout(remaining) {
                Ok(msg) => msg,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    last_snapshot = Instant::now();
                    publish_snapshot(&mut backend, &latest, &event_tx);
                    dirty = false;
                    continue;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        } else {
            // No pending unpublished change -- block indefinitely rather
            // than polling, so a genuinely idle session costs nothing (the
            // same "parked in a blocking wait" idle-CPU property the reader
            // thread's tight `read()` loop and this design overall rely on).
            match rx.recv() {
                Ok(msg) => msg,
                Err(_) => break,
            }
        };

        match msg {
            WorkerMsg::Bytes(bytes) => {
                backend.feed(&bytes);
                // Refresh the bracketed-paste/alt-screen flags on every
                // processed batch (unlike the snapshot, not throttled --
                // they're single cheap bools, and either can change at any
                // time between frames).
                bracketed_paste.store(backend.bracketed_paste(), Ordering::Relaxed);
                kitty_disambiguate.store(backend.kitty_disambiguate(), Ordering::Relaxed);
                alt_screen.store(backend.alt_screen_active(), Ordering::Relaxed);
                alternate_scroll.store(backend.alternate_scroll_active(), Ordering::Relaxed);
                mouse_mode.store(backend.mouse_mode().to_bits(), Ordering::Relaxed);
                if let Ok(mut slot) = title.lock() {
                    *slot = backend.title();
                }
                if last_snapshot.elapsed() >= FRAME_INTERVAL {
                    last_snapshot = Instant::now();
                    publish_snapshot(&mut backend, &latest, &event_tx);
                    dirty = false;
                } else {
                    dirty = true;
                }
            }
            WorkerMsg::Resize(c, r) => {
                backend.resize(c, r);
                // Always snapshot on resize: the dimensions changed, and there
                // may be no PTY output to otherwise trigger a redraw.
                last_snapshot = Instant::now();
                publish_snapshot(&mut backend, &latest, &event_tx);
                dirty = false;
            }
            WorkerMsg::Scroll(delta_lines) => {
                backend.scroll_display(delta_lines);
                // Always snapshot: a scroll is a discrete, user-visible
                // action that typically has no PTY output of its own to
                // otherwise trigger a redraw -- same reasoning as Resize.
                last_snapshot = Instant::now();
                publish_snapshot(&mut backend, &latest, &event_tx);
                dirty = false;
            }
            WorkerMsg::ScrollToBottom => {
                backend.scroll_to_bottom();
                last_snapshot = Instant::now();
                publish_snapshot(&mut backend, &latest, &event_tx);
                dirty = false;
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

/// Windows-only shell resolution (Rust port's Windows app wave -- see
/// `rust/crates/labolabo-app/README.md`'s "Windows" section for the product-
/// level writeup of this decision). Kept self-contained (no dependency on
/// `labolabo_core::ToolLocator`, even though its Windows `PATHEXT`-aware
/// `PATH` scan solves a near-identical problem) so this crate stays usable
/// on its own, per its README's stated design goal -- pulling in
/// `labolabo-core` (GRDB-compatible SQLite persistence, the tiling model,
/// hooks/control protocols, ...) just for a ~15-line `PATH` scan would be a
/// much heavier coupling than the problem calls for.
#[cfg(windows)]
mod windows {
    use std::env;
    use std::ffi::{OsStr, OsString};
    use std::path::Path;

    /// `%ComSpec%`, falling back to the bare `cmd.exe` (resolved via `PATH`
    /// by `portable-pty`'s own `CommandBuilder::search_path`, same fallback
    /// `portable-pty` itself uses for `new_default_prog`'s Windows arm) when
    /// unset -- every real Windows install sets `ComSpec`, so the fallback is
    /// defense in depth, not an expected path.
    pub(super) fn comspec() -> OsString {
        env::var_os("ComSpec").unwrap_or_else(|| OsString::from("cmd.exe"))
    }

    /// The interactive default shell for a Windows terminal pane (the `None`
    /// -- no explicit command -- case): prefers PowerShell over the bare
    /// `%ComSpec%` (`cmd.exe`) `portable-pty`'s own `CommandBuilder::
    /// new_default_prog` would otherwise always pick (it never looks at
    /// PowerShell at all -- see `session.rs`'s call site), since PowerShell
    /// is the nicer modern default most Windows developers actually expect
    /// from a new terminal tab (mirroring what Windows Terminal itself
    /// defaults to when no profile is configured). Search order, first
    /// match on `PATH` wins:
    ///
    /// 1. `pwsh.exe` -- PowerShell 7+ (the actively developed, cross-platform
    ///    line), not installed by default but common on developer machines
    ///    and preinstalled on GitHub's `windows-latest` Actions runner.
    /// 2. `powershell.exe` -- Windows PowerShell 5.1, present on every stock
    ///    Windows install since Windows 7 -- the safe universal fallback.
    /// 3. `None` -- caller falls back to `CommandBuilder::new_default_prog`
    ///    (`%ComSpec%`, i.e. `cmd.exe`) -- reached only on a stripped-down
    ///    environment with neither PowerShell binary on `PATH`.
    ///
    /// Deliberately no unix-style login-shell flag (`-l` has no PowerShell/
    /// cmd.exe equivalent, and Windows has no analogous "login shell profile
    /// wasn't sourced yet" problem `-l` solves on unix -- a plain
    /// interactive launch already reads `$PROFILE`/registry `AutoRun`
    /// itself).
    pub(super) fn default_prog() -> Option<OsString> {
        resolve_shell_in(env::var_os("PATH").as_deref(), path_has_file)
    }

    /// Pure resolution core, parameterized on the `PATH` value and an
    /// existence predicate -- see the crate's other Windows-adjacent
    /// modules (e.g. `labolabo_core::tool_locator`) for the same "pure
    /// core + thin OS-reading wrapper" split, which is what keeps this
    /// testable without mutating the real process-global `PATH` env var
    /// (risky under `cargo test`'s multi-threaded default test runner).
    fn resolve_shell_in(
        path_env: Option<&OsStr>,
        exists: impl Fn(&Path) -> bool,
    ) -> Option<OsString> {
        let path_env = path_env?;
        for name in ["pwsh.exe", "powershell.exe"] {
            for dir in env::split_paths(path_env) {
                let candidate = dir.join(name);
                if exists(&candidate) {
                    return Some(candidate.into_os_string());
                }
            }
        }
        None
    }

    fn path_has_file(path: &Path) -> bool {
        path.is_file()
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::fs;

        fn touch(dir: &std::path::Path, name: &str) {
            fs::write(dir.join(name), b"").unwrap();
        }

        #[test]
        fn no_path_resolves_to_none() {
            assert_eq!(resolve_shell_in(None, path_has_file), None);
        }

        #[test]
        fn prefers_pwsh_over_powershell_when_both_present() {
            let dir = std::env::temp_dir()
                .join(format!("labolabo-term-shell-test-{}-a", std::process::id()));
            let _ = fs::create_dir_all(&dir);
            touch(&dir, "pwsh.exe");
            touch(&dir, "powershell.exe");
            let path_env = OsString::from(dir.as_os_str());
            let resolved = resolve_shell_in(Some(&path_env), path_has_file);
            assert_eq!(resolved, Some(dir.join("pwsh.exe").into_os_string()));
            let _ = fs::remove_dir_all(&dir);
        }

        #[test]
        fn falls_back_to_powershell_when_pwsh_absent() {
            let dir = std::env::temp_dir()
                .join(format!("labolabo-term-shell-test-{}-b", std::process::id()));
            let _ = fs::create_dir_all(&dir);
            touch(&dir, "powershell.exe");
            let path_env = OsString::from(dir.as_os_str());
            let resolved = resolve_shell_in(Some(&path_env), path_has_file);
            assert_eq!(resolved, Some(dir.join("powershell.exe").into_os_string()));
            let _ = fs::remove_dir_all(&dir);
        }

        #[test]
        fn none_when_neither_is_present() {
            let dir = std::env::temp_dir()
                .join(format!("labolabo-term-shell-test-{}-c", std::process::id()));
            let _ = fs::create_dir_all(&dir);
            let path_env = OsString::from(dir.as_os_str());
            let resolved = resolve_shell_in(Some(&path_env), path_has_file);
            assert_eq!(resolved, None);
            let _ = fs::remove_dir_all(&dir);
        }
    }
}
