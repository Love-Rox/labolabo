//! Claude Code hooks integration: the app-layer I/O half of
//! `labolabo_core::hook_settings`'s pure functions -- see that module's doc
//! comment for the split rationale (pure merge/command-building logic there,
//! filesystem/process/gpui wiring here).
//!
//! Faithful-in-shape port of the Swift app's
//! `app/Sources/AgentSessionModel.swift` (`installLocalSettings`/
//! `removeLocalSettings`, the AF_UNIX bus lifecycle) with one deliberate
//! architecture change: Swift runs **one socket per session** (one worktree/
//! `RepoSession`); this port runs **one socket for the whole app process**
//! and routes incoming events to the right `(task_id, PaneId)` purely via
//! the `LABOLABO_PANE`/`LABOLABO_TASK` env vars injected at pane-spawn time
//! (`docs/hooks-protocol.md` §7, `plans/012` §1's "要設計" note on the
//! same-cwd-multiple-Tasks case) -- see [`HookRuntime`]'s doc comment for
//! why this is both simpler and a better fit for "one window, many Tasks".
//!
//! Three responsibilities:
//!
//! 1. [`HookRuntime::new`]: resolves the `labolabo-hook` forwarder binary,
//!    starts the one shared [`AgentStatusBus`], and returns the raw event
//!    channel for the caller (`LaboLaboApp::new`) to bridge into gpui (see
//!    [`spawn_agent_event_bridge`], mirroring `task_workspace::
//!    spawn_redraw_bridge`'s two-stage thread-to-gpui-task pattern).
//! 2. [`HookRuntime::ensure_injected`]/[`HookRuntime::restore_all`]: the
//!    `.claude/settings.local.json` merge/backup/restore file I/O, called
//!    once per Task working directory (idempotent) and once at app quit
//!    (`gpui::Context::on_app_quit` hands the quit closure `&mut
//!    LaboLaboApp` directly, so no separate shared-ownership handle is
//!    needed to reach `HookRuntime` from there -- see `LaboLaboApp::new`'s
//!    wiring).
//! 3. [`HookRuntime::register_pane`]/[`unregister_pane`]/[`resolve_pane`]:
//!    the in-memory `LABOLABO_PANE` (a fresh UUID minted per spawn, *not*
//!    persisted -- see `labolabo_core::tiling`'s module doc comment on why
//!    `PaneId` itself isn't stable across restarts either, same reasoning
//!    Swift's `PaneItem.id` UUID already has) -> `(task_id, PaneId)` routing
//!    table `crate::app::LaboLaboApp::handle_agent_event` consults.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use futures::channel::mpsc;
use futures::StreamExt;
use gpui::{Context, Task as GpuiTask};

#[cfg(windows)]
use labolabo_core::hook_pipe_name_from_uuid;
#[cfg(not(windows))]
use labolabo_core::socket_path_from_uuid;
use labolabo_core::{hook_command, merge_hooks};
use labolabo_core::{AgentStatusBus, AgentStatusEvent, PaneId};

use crate::app::LaboLaboApp;

/// Base directory for the app's hooks socket on unix (docs/hooks-protocol.md
/// §4: `/tmp/labolabo`, 0700, created by [`AgentStatusBus`] itself on
/// `start()`). **Unix-only** -- Windows has no analogous "socket file on
/// disk" concept; see [`mint_socket_path`] below.
#[cfg(not(windows))]
const SOCKET_BASE_DIR: &str = "/tmp/labolabo";

/// Mints this process's hooks socket identity, platform-appropriate: an
/// AF_UNIX path under [`SOCKET_BASE_DIR`] on unix, or a `\\.\pipe\...` Named
/// Pipe name (`labolabo_core::hook_pipe_name_from_uuid`,
/// docs/hooks-protocol.md §4.2) on Windows -- **not** a filesystem path
/// there, since Named Pipe names live in their own kernel object namespace,
/// not under any directory. Both are handed to `AgentStatusBus::new` through
/// the same `socket_path` slot (`labolabo_core::hooks`'s Windows
/// `NamedPipeEventTransport` doc comment: "the pipe name *is* the
/// socketPath value on Windows"), so every caller of this function stays
/// platform-agnostic past this one point.
fn mint_socket_path(uuid: &str) -> String {
    #[cfg(windows)]
    {
        hook_pipe_name_from_uuid(uuid)
    }
    #[cfg(not(windows))]
    {
        socket_path_from_uuid(uuid, SOCKET_BASE_DIR)
    }
}

/// Where an incoming event's `labolabo_pane_id` routes to.
#[derive(Debug, Clone)]
pub struct PaneRoute {
    pub task_id: String,
    pub pane_id: PaneId,
}

/// One directory's injected-hooks cleanup bookkeeping -- what
/// [`HookRuntime::restore_all`] needs to put `settings_path` back the way it
/// found it. Mirrors the two pieces of state `AgentSessionModel` keeps per
/// session (the `.labolabo-bak` file's mere existence *is* the Swift class's
/// `createdSettings == false` branch; `created` here is this port's
/// in-memory equivalent of `createdSettings`).
struct InjectedDir {
    settings_path: PathBuf,
    backup_path: PathBuf,
    /// `true` if this run created `settings_path` fresh (no valid prior
    /// file) -- see `labolabo_core::hook_settings::MergedSettings::created`.
    created: bool,
}

/// Owns the app-wide hooks socket, the forwarder binary path, the per-
/// directory injection bookkeeping, and the `LABOLABO_PANE` routing table.
/// One instance lives on [`LaboLaboApp`] for the process's lifetime.
pub struct HookRuntime {
    /// Kept alive for the process's lifetime (never stopped/read again
    /// after `start()` in [`HookRuntime::new`]) purely so its accept-loop
    /// thread and bound socket stay live -- dropping this would tear the
    /// listener down. Not explicitly stopped at quit either: process exit
    /// reclaims the thread/fd, and the socket file itself is unlinked-then-
    /// rebound on next launch regardless (docs/hooks-protocol.md §4).
    _bus: AgentStatusBus,
    /// This process's hooks socket path -- every injected directory's hook
    /// entry points at the same one (see this module's doc comment for why
    /// one shared socket, not one per Task/directory, was chosen).
    pub socket_path: String,
    /// `None` if `labolabo-hook` couldn't be resolved next to the running
    /// executable -- [`ensure_injected`](Self::ensure_injected) then no-ops
    /// (a warning was already printed once, at [`HookRuntime::new`]).
    binary_path: Option<PathBuf>,
    injected: HashMap<PathBuf, InjectedDir>,
    routes: HashMap<String, PaneRoute>,
}

impl HookRuntime {
    /// Resolves the forwarder binary, starts the shared socket bus, and
    /// returns `(runtime, raw_event_channel)` -- the caller bridges the
    /// channel into gpui itself (see [`spawn_agent_event_bridge`]) rather
    /// than this constructor taking a `Context` and doing so internally, to
    /// keep this module's only gpui dependency at the very edge (the bridge
    /// function), matching `task_workspace::spawn_redraw_bridge`'s split.
    pub fn new() -> (Self, mpsc::UnboundedReceiver<AgentStatusEvent>) {
        let socket_uuid = uuid::Uuid::new_v4().to_string();
        Self::new_at(mint_socket_path(&socket_uuid))
    }

    /// [`HookRuntime::new`]'s whole construction, parameterized on the
    /// socket path -- private, split out so the end-to-end test can run the
    /// real bus/channel/binary-resolution path against a socket under the
    /// OS temp dir instead of littering the real [`SOCKET_BASE_DIR`] with
    /// one stale socket file per `cargo test` run (the accept-loop thread
    /// holds the listener until process exit, so a test process never
    /// unlinks it itself).
    fn new_at(socket_path: String) -> (Self, mpsc::UnboundedReceiver<AgentStatusEvent>) {
        let mut bus = AgentStatusBus::new(socket_path.clone());
        let (tx, rx) = mpsc::unbounded();
        bus.set_on_event(move |event| {
            // The bus's accept-loop thread calls this (see AgentStatusBus's
            // module doc comment: "呼び出しスレッドは実装依存") -- hand off
            // to the gpui-side bridge task via the channel rather than
            // touching any app state directly from this thread.
            let _ = tx.unbounded_send(event);
        });
        bus.start();

        let binary_path = resolve_hook_binary();
        if binary_path.is_none() {
            eprintln!(
                "labolabo-app: labolabo-hook binary not found next to the running executable \
                 ({:?}); Claude Code hooks injection will be skipped for this run (no agent \
                 status display, no session memory/resume)",
                std::env::current_exe().ok()
            );
        }

        (
            Self {
                _bus: bus,
                socket_path,
                binary_path,
                injected: HashMap::new(),
                routes: HashMap::new(),
            },
            rx,
        )
    }

    /// Injects LaboLabo's hook entry into `dir`'s
    /// `.claude/settings.local.json` (idempotent: a no-op if `dir` was
    /// already injected this run). No-op if the forwarder binary wasn't
    /// resolved (see [`HookRuntime::new`]'s warning).
    ///
    /// Faithful port of `AgentSessionModel.installLocalSettings`'s file
    /// dance: restore-then-snapshot-then-merge-then-write, using the same
    /// `.labolabo-bak` filename convention (docs/hooks-protocol.md §2) so a
    /// stale backup from a previous crash of *either* app is recovered from
    /// the same way.
    pub fn ensure_injected(&mut self, dir: &Path) {
        let Some(binary_path) = self.binary_path.clone() else {
            return;
        };
        let dir = dir.to_path_buf();
        if self.injected.contains_key(&dir) {
            return;
        }

        let claude_dir = dir.join(".claude");
        let settings_path = claude_dir.join("settings.local.json");
        let backup_path = claude_dir.join("settings.local.json.labolabo-bak");

        if let Err(err) = std::fs::create_dir_all(&claude_dir) {
            eprintln!("labolabo-app: failed to create {claude_dir:?}: {err}");
            return;
        }

        // A backup left over from a previous crash: restore the original
        // first, so we snapshot *that* (not our own prior injection) below
        // -- docs/hooks-protocol.md §2's "二重注入防止".
        if backup_path.exists() {
            let _ = std::fs::remove_file(&settings_path);
            let _ = std::fs::rename(&backup_path, &settings_path);
        }

        let existing = std::fs::read_to_string(&settings_path).ok();
        let command = hook_command(&binary_path.to_string_lossy(), &self.socket_path);
        let merged = merge_hooks(existing.as_deref(), &command);

        // Snapshot the real prior content *only* when there was one (mirrors
        // Swift: an absent/unparseable file is not backed up, see
        // `MergedSettings::created`'s doc comment) -- write the backup
        // before overwriting the real file so a crash between these two
        // writes still leaves a recoverable original.
        if !merged.created {
            if let Some(existing) = &existing {
                if let Err(err) = std::fs::write(&backup_path, existing) {
                    eprintln!("labolabo-app: failed to back up {settings_path:?}: {err}");
                    return;
                }
            }
        }

        if let Err(err) = std::fs::write(&settings_path, &merged.content) {
            eprintln!("labolabo-app: failed to write {settings_path:?}: {err}");
            return;
        }

        self.injected.insert(
            dir,
            InjectedDir {
                settings_path,
                backup_path,
                created: merged.created,
            },
        );
    }

    /// Restores every injected directory's original `settings.local.json`
    /// (or deletes it if this run created it fresh) -- see [`InjectedDir`]'s
    /// doc comment. Called from `LaboLaboApp::new`'s `cx.on_app_quit`
    /// closure, which `gpui::Context::on_app_quit` hands `&mut LaboLaboApp`
    /// directly (unlike the plain `gpui::App::on_app_quit`), so this can
    /// simply be a method here rather than needing a separately shared
    /// handle onto `injected`.
    ///
    /// Best-effort: filesystem errors are logged, not propagated -- quit
    /// must not hang or crash on an I/O hiccup during shutdown.
    pub fn restore_all(&self) {
        for entry in self.injected.values() {
            if entry.backup_path.exists() {
                if let Err(err) = std::fs::remove_file(&entry.settings_path) {
                    eprintln!(
                        "labolabo-app: failed to remove {:?} while restoring backup: {err}",
                        entry.settings_path
                    );
                }
                if let Err(err) = std::fs::rename(&entry.backup_path, &entry.settings_path) {
                    eprintln!(
                        "labolabo-app: failed to restore {:?} from backup: {err}",
                        entry.settings_path
                    );
                }
            } else if entry.created {
                let _ = std::fs::remove_file(&entry.settings_path);
            }
        }
    }

    // MARK: - LABOLABO_PANE routing table

    pub fn register_pane(&mut self, pane_uuid: String, task_id: String, pane_id: PaneId) {
        self.routes
            .insert(pane_uuid, PaneRoute { task_id, pane_id });
    }

    pub fn unregister_pane(&mut self, pane_uuid: &str) {
        self.routes.remove(pane_uuid);
    }

    pub fn resolve_pane(&self, pane_uuid: &str) -> Option<PaneRoute> {
        self.routes.get(pane_uuid).cloned()
    }

    /// Test-only constructor: like [`HookRuntime::new`], but takes
    /// `binary_path`/`socket_path` directly instead of resolving them from
    /// the real environment (there is no `labolabo-hook` sibling next to a
    /// `cargo test` binary, and tests shouldn't share the real `/tmp/
    /// labolabo` socket namespace with a running app). Lets
    /// `ensure_injected`/`restore_all`'s real file I/O be exercised without
    /// depending on a workspace build having produced the forwarder binary.
    #[cfg(test)]
    fn for_test(binary_path: PathBuf, socket_path: String) -> Self {
        Self {
            _bus: AgentStatusBus::new(socket_path.clone()),
            socket_path,
            binary_path: Some(binary_path),
            injected: HashMap::new(),
            routes: HashMap::new(),
        }
    }
}

/// Resolves `labolabo-hook`'s path: the sibling of the running executable
/// (i.e. the same directory `cargo build`/`cargo run` places both
/// `labolabo-app` and `labolabo-core`'s `labolabo-hook` bin target in for a
/// workspace build). Per the task brief: "見つからない場合は注入スキップ +
/// stderr 警告で可" -- no further search (e.g. `$PATH`, a separately
/// resolved `target/` directory) is attempted; a development setup that
/// only ever ran `cargo run -p labolabo-app` without also building
/// `labolabo-core`'s bin target will see hooks injection skipped, with the
/// warning [`HookRuntime::new`] prints explaining why.
///
/// `std::env::consts::EXE_SUFFIX` (portable: `""` on unix, `".exe"` on
/// Windows) so the candidate filename matches what `cargo build` actually
/// produces on each platform -- without it, this always misses on Windows
/// (`labolabo-hook` vs. the real `labolabo-hook.exe`), silently skipping
/// hooks injection on every Windows run.
fn resolve_hook_binary() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let candidate = dir.join(format!("labolabo-hook{}", std::env::consts::EXE_SUFFIX));
    candidate.exists().then_some(candidate)
}

/// Bridges the raw [`AgentStatusEvent`] channel from [`HookRuntime::new`]
/// into gpui, dispatching each event to `LaboLaboApp::handle_agent_event` on
/// the main gpui task queue. Simpler than `task_workspace::
/// spawn_redraw_bridge` (no coalescing/pacing -- hook events are already
/// infrequent, one per Claude Code lifecycle step, not a firehose of PTY
/// output) but the same "OS-thread-callback -> channel -> gpui task" shape.
pub fn spawn_agent_event_bridge(
    mut events_rx: mpsc::UnboundedReceiver<AgentStatusEvent>,
    cx: &mut Context<LaboLaboApp>,
) -> GpuiTask<()> {
    cx.spawn(async move |this, cx| {
        while let Some(event) = events_rx.next().await {
            if this
                .update(cx, |app, cx| app.handle_agent_event(event, cx))
                .is_err()
            {
                break;
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    // MARK: - LABOLABO_PANE routing table (pure, no filesystem/gpui involved)

    /// A fresh, opaque [`PaneId`] for routing-table tests -- `PaneId`'s only
    /// constructor is private to `labolabo_core::tiling` (never serialized,
    /// never stable across restarts, see its module doc comment), so the
    /// same public route every real caller uses (minting a fresh
    /// `PaneItem`) is also the only way a test can get one.
    fn fresh_pane_id() -> PaneId {
        labolabo_core::PaneItem::new(labolabo_core::PaneKind::Terminal, "t").id
    }

    #[test]
    fn resolve_pane_returns_none_for_an_unregistered_uuid() {
        let runtime = runtime_for_test("resolve-none");
        assert!(runtime.resolve_pane("nope").is_none());
    }

    #[test]
    fn register_then_resolve_pane_round_trips() {
        let mut runtime = runtime_for_test("register-resolve");
        let pane_id = fresh_pane_id();
        runtime.register_pane("pane-uuid-1".to_string(), "task-1".to_string(), pane_id);

        let route = runtime.resolve_pane("pane-uuid-1").expect("registered");
        assert_eq!(route.task_id, "task-1");
        assert_eq!(route.pane_id, pane_id);
    }

    #[test]
    fn unregister_pane_removes_the_route() {
        let mut runtime = runtime_for_test("unregister");
        let pane_id = fresh_pane_id();
        runtime.register_pane("pane-uuid-1".to_string(), "task-1".to_string(), pane_id);
        runtime.unregister_pane("pane-uuid-1");
        assert!(runtime.resolve_pane("pane-uuid-1").is_none());
    }

    // MARK: - End-to-end: real socket, real forwarder logic, real routing

    fn recv_with_timeout(
        rx: &mut mpsc::UnboundedReceiver<AgentStatusEvent>,
        timeout: std::time::Duration,
    ) -> Option<AgentStatusEvent> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            if let Ok(event) = rx.try_recv() {
                return Some(event);
            }
            if std::time::Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    /// Exercises the real construction path `HookRuntime::new` uses
    /// (`new_at`: real [`AgentStatusBus`] binding a real AF_UNIX socket,
    /// real channel, real forwarder-binary resolution): a real
    /// `LABOLABO_PANE`-annotated payload sent through
    /// `labolabo_core::forward_hook` (the same pure logic the standalone
    /// `labolabo-hook` binary calls -- see `labolabo-core`'s
    /// `tests/labolabo_hook_bin.rs` for the equivalent real-subprocess test
    /// at that layer), delivered over the channel, and finally resolved
    /// through the routing table this module owns. Mirrors the wave 4b
    /// porting brief's suggested reference point.
    ///
    /// Uses a socket under the OS temp dir (removed at the end) rather than
    /// the production `SOCKET_BASE_DIR` (unix) / a `\\.\pipe\...` name
    /// (Windows, `hook_pipe_name_from_uuid`'s own uuid-derived token -- no
    /// filesystem path to keep out of a shared directory), so `cargo test`
    /// runs don't leave stale socket files in the real `/tmp/labolabo` --
    /// see `new_at`'s doc comment. Unix path kept short: `sockaddr_un`'s
    /// `sun_path` is only 104 (Darwin) / 108 (Linux) bytes (same constraint
    /// `labolabo-core`'s own socket tests document).
    #[test]
    fn hook_runtime_receives_a_real_socket_event_and_resolves_its_route() {
        #[cfg(windows)]
        let socket_path = hook_pipe_name_from_uuid(&uuid::Uuid::new_v4().to_string());
        #[cfg(not(windows))]
        let socket_path = std::env::temp_dir()
            .join(format!("lb-hr-{}.sock", std::process::id()))
            .to_string_lossy()
            .into_owned();
        let (mut runtime, mut rx) = HookRuntime::new_at(socket_path);
        let pane_uuid = "integration-pane-1".to_string();
        let pane_id = fresh_pane_id();
        runtime.register_pane(pane_uuid.clone(), "task-int-1".to_string(), pane_id);

        let mut env = HashMap::new();
        env.insert("LABOLABO_PANE".to_string(), pane_uuid.clone());
        let payload = br#"{"hook_event_name":"SessionStart","session_id":"sess-int-1"}"#;

        // Retry: the bus's accept-loop thread needs a moment to bind after
        // `start()` (same reasoning as `labolabo-core`'s own socket tests).
        let mut sent = false;
        for _ in 0..150 {
            if labolabo_core::forward_hook(&runtime.socket_path, payload, &env).is_ok() {
                sent = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(
            sent,
            "forward_hook should eventually connect to the bus's socket"
        );

        let event = recv_with_timeout(&mut rx, std::time::Duration::from_secs(3))
            .expect("the bus should deliver the parsed event over the channel");
        assert_eq!(event.status, labolabo_core::AgentStatus::Starting);
        assert_eq!(event.session_id.as_deref(), Some("sess-int-1"));
        assert_eq!(
            event.pane_id.as_deref(),
            Some(pane_uuid.as_str()),
            "forward_hook's LABOLABO_PANE annotation should survive the round trip"
        );

        let route = runtime
            .resolve_pane(event.pane_id.as_deref().unwrap())
            .expect("the event's pane_id should resolve through the routing table");
        assert_eq!(route.task_id, "task-int-1");
        assert_eq!(route.pane_id, pane_id);

        // The bus's accept-loop thread holds the listener until process
        // exit; unlink the socket file ourselves so the test leaves no
        // artifact behind (safe on unix -- the bound listener works via
        // the inode).
        let _ = fs::remove_file(&runtime.socket_path);
    }

    /// Headless integration coverage for the "status indicator gaps" root
    /// cause (13th wave a): sends the exact event sequence a real auto-
    /// compaction mid-task produces -- `SessionStart(startup)` ->
    /// `UserPromptSubmit` -> `PreToolUse` -> `SessionStart(compact)` ->
    /// `PostToolUse` -- through a real AF_UNIX socket (same
    /// `forward_hook`/bus/channel path as the test above, not a synthetic
    /// call into `AgentStatus::from_hook_event` directly) and asserts the
    /// decoded status never regresses to `Starting` once the session has
    /// reached `Running` -- in particular that the `SessionStart(compact)`
    /// event in the middle decodes to `Running`, not `Starting`.
    #[test]
    fn compact_mid_task_session_start_does_not_regress_status_to_starting() {
        #[cfg(windows)]
        let socket_path = hook_pipe_name_from_uuid(&uuid::Uuid::new_v4().to_string());
        #[cfg(not(windows))]
        let socket_path = std::env::temp_dir()
            .join(format!("lb-hr-compact-{}.sock", std::process::id()))
            .to_string_lossy()
            .into_owned();
        let (mut runtime, mut rx) = HookRuntime::new_at(socket_path);
        let pane_uuid = "integration-pane-compact".to_string();
        let pane_id = fresh_pane_id();
        runtime.register_pane(pane_uuid.clone(), "task-compact-1".to_string(), pane_id);

        let mut env = HashMap::new();
        env.insert("LABOLABO_PANE".to_string(), pane_uuid.clone());

        let send = |payload: &[u8]| {
            let mut sent = false;
            for _ in 0..150 {
                if labolabo_core::forward_hook(&runtime.socket_path, payload, &env).is_ok() {
                    sent = true;
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            assert!(
                sent,
                "forward_hook should eventually connect to the bus's socket"
            );
        };

        let recv = |rx: &mut mpsc::UnboundedReceiver<AgentStatusEvent>| {
            recv_with_timeout(rx, std::time::Duration::from_secs(3))
                .expect("the bus should deliver the parsed event over the channel")
        };

        // SessionStart(startup) -- a fresh session: starting (orange), as
        // always.
        send(br#"{"hook_event_name":"SessionStart","source":"startup","session_id":"sess-c1"}"#);
        let e = recv(&mut rx);
        assert_eq!(e.status, labolabo_core::AgentStatus::Starting);

        // UserPromptSubmit -- work begins: running (green).
        send(br#"{"hook_event_name":"UserPromptSubmit","session_id":"sess-c1"}"#);
        let e = recv(&mut rx);
        assert_eq!(e.status, labolabo_core::AgentStatus::Running);

        // PreToolUse -- still running.
        send(br#"{"hook_event_name":"PreToolUse","session_id":"sess-c1"}"#);
        let e = recv(&mut rx);
        assert_eq!(e.status, labolabo_core::AgentStatus::Running);

        // SessionStart(compact) -- auto-compaction fires mid-task. This is
        // the regression this test pins: must stay running, not flip back
        // to starting.
        send(br#"{"hook_event_name":"SessionStart","source":"compact","session_id":"sess-c1"}"#);
        let e = recv(&mut rx);
        assert_eq!(
            e.status,
            labolabo_core::AgentStatus::Running,
            "SessionStart(source=compact) mid-task must not regress the indicator to Starting"
        );
        assert_eq!(e.source.as_deref(), Some("compact"));

        // PostToolUse -- the agent keeps working right after compaction:
        // still running, confirming the sequence never dipped to starting.
        send(br#"{"hook_event_name":"PostToolUse","session_id":"sess-c1"}"#);
        let e = recv(&mut rx);
        assert_eq!(e.status, labolabo_core::AgentStatus::Running);

        let _ = fs::remove_file(&runtime.socket_path);
    }

    #[test]
    fn registering_the_same_uuid_twice_overwrites_the_route() {
        let mut runtime = runtime_for_test("overwrite");
        let pane_a = fresh_pane_id();
        let pane_b = fresh_pane_id();
        runtime.register_pane("pane-uuid-1".to_string(), "task-1".to_string(), pane_a);
        runtime.register_pane("pane-uuid-1".to_string(), "task-2".to_string(), pane_b);

        let route = runtime.resolve_pane("pane-uuid-1").expect("registered");
        assert_eq!(route.task_id, "task-2");
        assert_eq!(route.pane_id, pane_b);
    }

    // MARK: - ensure_injected / restore_all (real filesystem, temp directories)

    fn temp_dir(label: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "labolabo-hooks-test-{label}-{}-{n}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn runtime_for_test(label: &str) -> HookRuntime {
        let dir = temp_dir(label);
        // A file standing in for the forwarder binary -- `ensure_injected`
        // only ever reads its path (`to_string_lossy()` into the command
        // string), never executes it, so it doesn't need to actually be
        // runnable.
        let binary_path = dir.join("labolabo-hook-stub");
        fs::write(&binary_path, b"#!/bin/sh\n").unwrap();
        let socket_path = dir.join("test.sock").to_string_lossy().into_owned();
        HookRuntime::for_test(binary_path, socket_path)
    }

    #[test]
    fn ensure_injected_creates_settings_file_with_all_seven_hook_events() {
        let mut runtime = runtime_for_test("fresh");
        let work_dir = temp_dir("fresh-work");

        runtime.ensure_injected(&work_dir);

        let settings_path = work_dir.join(".claude/settings.local.json");
        let content = fs::read_to_string(&settings_path).expect("settings file written");
        let value: serde_json::Value = serde_json::from_str(&content).unwrap();
        let hooks = value["hooks"].as_object().expect("hooks object");
        assert_eq!(hooks.len(), 7);
        assert!(!work_dir
            .join(".claude/settings.local.json.labolabo-bak")
            .exists());

        fs::remove_dir_all(&work_dir).ok();
    }

    #[test]
    fn ensure_injected_is_idempotent_per_directory_within_one_runtime() {
        let mut runtime = runtime_for_test("idempotent");
        let work_dir = temp_dir("idempotent-work");

        runtime.ensure_injected(&work_dir);
        let first = fs::read_to_string(work_dir.join(".claude/settings.local.json")).unwrap();
        runtime.ensure_injected(&work_dir);
        let second = fs::read_to_string(work_dir.join(".claude/settings.local.json")).unwrap();

        assert_eq!(
            first, second,
            "second call must be a no-op, not append again"
        );

        fs::remove_dir_all(&work_dir).ok();
    }

    #[test]
    fn ensure_injected_preserves_a_pre_existing_files_other_hooks() {
        let mut runtime = runtime_for_test("preserve");
        let work_dir = temp_dir("preserve-work");
        let claude_dir = work_dir.join(".claude");
        fs::create_dir_all(&claude_dir).unwrap();
        fs::write(
            claude_dir.join("settings.local.json"),
            r#"{"hooks": {"SessionStart": [{"matcher": "", "hooks": [{"type": "command", "command": "echo other-tool"}]}]}}"#,
        )
        .unwrap();

        runtime.ensure_injected(&work_dir);

        let content = fs::read_to_string(claude_dir.join("settings.local.json")).unwrap();
        let value: serde_json::Value = serde_json::from_str(&content).unwrap();
        let session_start = value["hooks"]["SessionStart"].as_array().unwrap();
        assert_eq!(session_start.len(), 2, "other tool's entry plus ours");
        assert_eq!(session_start[0]["hooks"][0]["command"], "echo other-tool");

        fs::remove_dir_all(&work_dir).ok();
    }

    #[test]
    fn restore_all_deletes_a_freshly_created_settings_file() {
        let mut runtime = runtime_for_test("restore-created");
        let work_dir = temp_dir("restore-created-work");

        runtime.ensure_injected(&work_dir);
        let settings_path = work_dir.join(".claude/settings.local.json");
        assert!(settings_path.exists());

        runtime.restore_all();

        assert!(
            !settings_path.exists(),
            "a settings file we created fresh must be deleted on restore"
        );

        fs::remove_dir_all(&work_dir).ok();
    }

    #[test]
    fn restore_all_restores_a_pre_existing_files_original_content() {
        let mut runtime = runtime_for_test("restore-original");
        let work_dir = temp_dir("restore-original-work");
        let claude_dir = work_dir.join(".claude");
        fs::create_dir_all(&claude_dir).unwrap();
        let original = r#"{"hooks": {}, "env": {"FOO": "bar"}}"#;
        fs::write(claude_dir.join("settings.local.json"), original).unwrap();

        runtime.ensure_injected(&work_dir);
        runtime.restore_all();

        let settings_path = claude_dir.join("settings.local.json");
        assert!(
            settings_path.exists(),
            "the original file must be restored, not deleted"
        );
        assert_eq!(fs::read_to_string(&settings_path).unwrap(), original);
        assert!(
            !claude_dir.join("settings.local.json.labolabo-bak").exists(),
            "the backup file must be consumed by the restore (renamed back)"
        );

        fs::remove_dir_all(&work_dir).ok();
    }

    #[test]
    fn ensure_injected_recovers_from_a_stale_backup_before_reinjecting() {
        // Simulates a previous crashed run: the real settings file is
        // already our merged version, and a `.labolabo-bak` with the
        // *original* pre-injection content is still sitting there
        // (docs/hooks-protocol.md §2's "二重注入防止").
        let mut runtime = runtime_for_test("stale-backup");
        let work_dir = temp_dir("stale-backup-work");
        let claude_dir = work_dir.join(".claude");
        fs::create_dir_all(&claude_dir).unwrap();
        let original = r#"{"hooks": {}, "env": {"FOO": "original"}}"#;
        fs::write(
            claude_dir.join("settings.local.json.labolabo-bak"),
            original,
        )
        .unwrap();
        fs::write(
            claude_dir.join("settings.local.json"),
            r#"{"hooks": {"SessionStart": [{"matcher": "", "hooks": [{"type": "command", "command": "stale-injection"}]}]}}"#,
        )
        .unwrap();

        runtime.ensure_injected(&work_dir);

        // The stale injection's `stale-injection` command must be gone --
        // we restored the real original before merging our own entry in.
        let content = fs::read_to_string(claude_dir.join("settings.local.json")).unwrap();
        assert!(!content.contains("stale-injection"));
        let value: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(value["env"]["FOO"], "original");
        // And a fresh backup of *that* real original now exists.
        assert_eq!(
            fs::read_to_string(claude_dir.join("settings.local.json.labolabo-bak")).unwrap(),
            original
        );

        fs::remove_dir_all(&work_dir).ok();
    }
}
