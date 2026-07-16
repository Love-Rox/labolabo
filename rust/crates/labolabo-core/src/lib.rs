//! `labolabo-core`: the OS/UI-independent core of LaboLabo, ported from the
//! Swift `LaboLaboEngine` package (waves 1-2), the app's `PaneTilingModel`
//! (wave 3), and `LaboLaboStore` (wave 4c). "Pure-logic" describes waves
//! 1-3 (parsers and in-memory models, no I/O); `store` (wave 4c) is real,
//! fallible SQLite persistence, still OS/UI-framework-independent but no
//! longer side-effect-free — see its module doc comment and `README.md`'s
//! "Wave 4c" section.
//!
//! ## Wave 1 (`Sources/LaboLaboEngine/Git/`, no runtime deps)
//!
//! The pure parsers with no process/filesystem/concurrency dependencies:
//!
//! - `git_models`: data types ported from `GitModels.swift`.
//! - `porcelain`: `git status --porcelain=v2 -z` parser, ported from
//!   `PorcelainStatusParser.swift`.
//! - `unified_diff`: unified `git diff` parser, ported from
//!   `UnifiedDiffParser.swift`.
//!
//! ## Wave 2 (commit graph, worktree list, agent status/usage)
//!
//! - `commit_graph`: commit-graph lane layout, ported from the pure
//!   `CommitGraphLayout.build` in `Git/CommitGraph.swift` (the `GitEngine`
//!   extension that shells out to `git log` is out of scope — process
//!   execution, not pure logic).
//! - `worktree`: `git worktree list --porcelain` parser, ported from
//!   `Git/Worktree.swift`.
//! - `transcript_usage`: agent transcript (JSONL) usage aggregation, ported
//!   from `Agent/TranscriptUsage.swift`.
//! - `agent_status`: hook-event -> live-status mapping, ported from
//!   `Agent/AgentStatus.swift`.
//! - `agent_event_parser`: hook-event JSON interpretation, ported from
//!   `Agent/AgentEventParser.swift`. The wire protocol is specified in full
//!   by `docs/hooks-protocol.md` at the repo root; this port was
//!   cross-checked against it directly (no divergence found). The
//!   `AgentStatusBus`/`AgentEventTransport` socket-transport layer itself is
//!   out of scope (process/socket infrastructure, not pure logic).
//! - `cross_session_conflicts`: cross-session "same file, same repo, both
//!   changed" detection, ported from `Git/CrossSessionConflicts.swift`.
//! - `release_version`: dotted numeric version comparison, ported from
//!   `Update/ReleaseVersion.swift`.
//!
//! `transcript_usage` and `agent_event_parser` need real JSON parsing
//! (`serde_json`, a runtime dependency starting this wave) to faithfully
//! reproduce Foundation's `JSONSerialization` + `as? T` bridging behavior —
//! see `transcript_usage::as_int`'s doc comment for the specific quirk this
//! preserves.
//!
//! ## Wave 3 (tiling/tab tree model)
//!
//! - `tiling`: the pane tiling/tab tree model (`TileNode`/`PaneItem`/
//!   `PaneTilingModel`) and its persisted-JSON shape (`TileLayout`/
//!   `PanePayload`/`LayoutPreset`), ported from the app target's
//!   `app/Sources/PaneTilingModel.swift` (not part of `LaboLaboEngine` —
//!   this is the first ported module that lives in the app, not the
//!   library). `TileLayout`/`PanePayload` are real `Codable` DTOs the app
//!   round-trips through `JSONEncoder`/`JSONDecoder` for persistence (GRDB
//!   `appState.paneLayout`), so `tiling`'s `#[derive(Serialize,
//!   Deserialize)]` types are production code, not a test-only view —
//!   unlike every module above, whose JSON views exist only in
//!   `tests/golden.rs`. See the `tiling` module doc comment for how its
//!   golden fixtures were produced (a separate small oracle script, since
//!   `PaneTilingModel.swift` isn't reachable through the
//!   `LaboLaboEngine`-linking trick `fixtures/generate.swift` uses) and for
//!   the JSON-compatibility caveats (key order, float formatting, `/`
//!   escaping) that come with matching a real `JSONEncoder` byte-for-byte.
//!
//! Correctness is anchored to the Swift implementation as the "golden
//! oracle": `tests/golden.rs` runs this crate's parsers over the same input
//! corpus the Swift parsers were run over (see `fixtures/`) and asserts
//! byte-identical canonical JSON output for the parser modules (porcelain,
//! unified_diff, worktree, transcript_usage, agent_event_parser). The pure
//! algorithm modules (commit_graph, cross_session_conflicts, release_version)
//! are covered by unit tests ported 1:1 from the corresponding Swift XCTest
//! suites instead — see `README.md` for why (and for how the golden fixtures
//! were generated / how to regenerate them). `tiling` has its own golden
//! test (`tests/tiling_golden.rs`) plus ported-1:1 unit tests in the module
//! itself — see its doc comment.
//!
//! ## Wave 4b (hooks bus + forwarder)
//!
//! - `hooks`: the `AgentEventTransport` trait, its AF_UNIX implementation
//!   (`UnixSocketEventTransport`, `#[cfg(unix)]`), the `AgentStatusBus` that
//!   composes a transport with `agent_event_parser`, and the
//!   `forward_hook`/`labolabo-hook` forwarder logic -- ported from
//!   `Sources/LaboLaboEngine/Agent/AgentStatusBus.swift` and
//!   `app/Sources/HookForwarder.swift`. `docs/hooks-protocol.md` (repo root)
//!   is the canonical wire-protocol spec this was cross-checked against
//!   directly; no divergence found. See the module doc comment for the
//!   deliberate (non-observable-behavior) differences from the Swift socket
//!   plumbing, and `src/bin/labolabo-hook.rs` for the thin binary that calls
//!   `forward_hook`.

pub mod agent_event_parser;
pub mod agent_status;
pub mod commit_graph;
pub mod cross_session_conflicts;
pub mod git_models;
pub mod porcelain;
pub mod release_version;
pub mod tiling;
pub mod transcript_usage;
pub mod unified_diff;
mod util;
pub mod worktree;
// Appended at the tail (rather than sorted alphabetically into the list
// above) to minimize merge conflicts with other in-flight porting-wave
// branches editing this same file.
pub mod hooks;

pub use agent_status::{AgentStatus, AgentStatusEvent};
pub use commit_graph::{Commit, CommitGraphRow, Edge, EdgeShape};
pub use git_models::{Change, GitFileEntry, GitStatus, Kind};
pub use tiling::{
    drop_edge_for_point, DropEdge, LayoutPreset, NodeId, PaneId, PaneItem, PaneKind, PanePayload,
    PaneTilingActions, PaneTilingModel, TileLayout, TileNode, TileOrientation, MAX_SPLIT_RATIO,
    MIN_SPLIT_RATIO,
};
pub use transcript_usage::AgentUsage;
pub use unified_diff::{DiffHunk, DiffLine, FileDiff, LineKind};
pub use worktree::Worktree;

// Wave 4a (process execution + git execution). Appended at the end of this
// file rather than interleaved with the wave 1-3 declarations above to
// minimize merge conflicts with parallel in-flight ports touching the same
// file (w4b/w4c); see `rust/README.md` for the wave breakdown.
//
// - `process`: port of the *observable contract* of
//   `Sources/LaboLaboEngine/Process/ProcessRunner.swift` (executable + args +
//   cwd + env -> `{status, stdout, stderr}`), collapsed into one synchronous
//   `std::process::Command`-based implementation (no async runtime
//   dependency -- see the module doc comment for why).
// - `tool_locator`: port of
//   `Sources/LaboLaboEngine/Process/{ToolLocating,ToolLocator}.swift`.
// - `git_runner`: port of `Sources/LaboLaboEngine/Git/GitRunner.swift`.
// - `git_engine`: port of `Sources/LaboLaboEngine/Git/GitEngine.swift`,
//   wiring the wave 1/2 parsers (`porcelain`/`unified_diff`/`commit_graph`/
//   `worktree`) to real `git` invocations via `git_runner`.
pub mod git_engine;
pub mod git_runner;
pub mod process;
pub mod tool_locator;

pub use git_engine::{GitEngine, NumstatEntry, RepoInfo, DEFAULT_COMMIT_GRAPH_LIMIT};
pub use git_runner::{GitCommandError, GitRunError};
pub use process::Output as ProcessOutput;
pub use tool_locator::{ToolLocating, ToolLocator};

// Wave 4b (hooks bus + forwarder). Appended at the tail, same reasoning as
// the wave 4a block above.
#[cfg(any(unix, windows))]
pub use hooks::forward_hook;
#[cfg(windows)]
pub use hooks::NamedPipeEventTransport;
#[cfg(unix)]
pub use hooks::UnixSocketEventTransport;
pub use hooks::{AgentEventTransport, AgentStatusBus, OnEvent, OnMessage};

// Wave 4c (session persistence). Appended at the end of this file rather
// than interleaved with the wave 1-3 declarations above to minimize merge
// conflicts with parallel in-flight ports touching the same file (w4a/w4b);
// see `rust/README.md`'s "Wave 4c" section for the full writeup.
//
// - `store`: port of `Sources/LaboLaboStore/` (`SessionRecord`,
//   `SessionDatabase`, `SessionPersisting`, `AppDataDirectory`) -- SQLite
//   session/appState persistence, via `rusqlite` instead of GRDB. The first
//   module in this crate that is fallible I/O rather than a pure
//   parser/model -- see `store`'s module doc comment (and
//   `store::database`'s, for the GRDB on-disk compatibility contract) for
//   details. Golden coverage is a fixture SQLite database written by real
//   GRDB (`fixtures/store/`, `tests/store_golden.rs`), the same oracle
//   philosophy as waves 1/2 but comparing database contents instead of
//   JSON.
pub mod store;

pub use store::{SessionDatabase, SessionPersisting, SessionRecord, StoreError, StoreResult};
pub use store::{Task, TaskDatabase, TaskKind, TaskStatus};

// Wave 5b-3 (`plans/012-task-model-and-control-cli.md` §1's "new worktree
// Task" flow). Appended at the tail, same reasoning as the wave 4a/4b/4c
// blocks above: minimizes merge conflicts with other in-flight
// porting-wave branches editing this same file.
//
// - `branch_naming`: pure branch-name generation for the Task model's
//   "new worktree" flow (`labolabo-app`'s UI calls this, then
//   `GitEngine::add_worktree`) -- no Swift counterpart, new-in-Rust
//   product surface (see `store::task_database`'s module doc comment for
//   why the Task model itself has none either).
pub mod branch_naming;

// Wave 5c (`plans/012-task-model-and-control-cli.md`'s hooks-integration
// follow-up): Claude Code hooks wiring for `labolabo-app` -- agent status
// display, per-tab session memory, and resume-on-restart. Appended at the
// tail, same reasoning as the wave 4a/4b/4c/5b-3 blocks above.
//
// - `hook_settings`: pure functions for the `.claude/settings.local.json`
//   merge, the forwarder command string, the socket path, and the Claude
//   `--resume` launch command -- ported from the shape of
//   `app/Sources/AgentSessionModel.swift`'s `installLocalSettings`/
//   `hookEntry`/`shellQuoted` and `Sources/LaboLaboEngine/Agent/
//   AgentAdapter.swift`'s `launchCommand`/`shellQuoted`. The actual file
//   I/O (backup/restore) is `labolabo-app`'s job -- see this module's own
//   doc comment.
pub mod hook_settings;

pub use hook_settings::{
    claude_resume_command, hook_command, hook_pipe_name_from_uuid, merge_hooks,
    quote_dropped_paths, shell_quote, socket_path_from_uuid, MergedSettings, HOOK_EVENTS,
};

// `store::agent_bindings`: the Task-level (docs/hooks-protocol.md §6(a))
// "last session id/transcript path" fallback record, persisted into
// `Task::agent_bindings` (reserved since wave 5b-3). See its module doc
// comment for why this is deliberately *not* where per-tab bindings live
// (those already round-trip through `Task::layout`/`tiling::PaneItem`, see
// `tiling.rs`'s module doc comment).
pub use store::AgentBindings;

// Rust wave 6e: the Swift-app-to-Rust-port session importer
// (`store::swift_import`) -- converts the Swift app's persisted sessions
// (`SessionDatabase`/`labolabo.db`) into this port's own `Task`s so
// upgrading from the Swift app restores the same open directories/
// worktrees, tab layout, and per-tab Claude `--resume` state. Strictly
// read-only against the Swift database -- see that module's doc comment.
pub use store::{import_from_swift, ImportOutcome};

// Control CLI wave (`plans/012-task-model-and-control-cli.md` §2,
// `docs/control-protocol.md`): the bidirectional CLI/agent-to-app RPC
// channel that lets `labolabo tab open`/`task list`/`tab list`/`focus`
// operate a running `labolabo-app` -- a separate channel from the
// receive-only hooks bus (wave 5c above). Appended at the tail, same
// reasoning as every other wave block in this file.
//
// - `control_protocol`: pure request/response (de)serialization, the
//   control socket/pipe naming conventions, and the `--task current`/
//   ambient-context resolution rules -- the `hook_settings` of this pair.
// - `control`: the request/response transport (`ControlServer` +
//   `send_control_request`) -- the `hooks` of this pair. AF_UNIX on unix
//   (docs/control-protocol.md §3), a message-mode Named Pipe on Windows
//   (§9, added by the Windows core wave) -- identical surface on both, so
//   the re-export below covers `any(unix, windows)`.
pub mod control;
pub mod control_protocol;

pub use control::ControlHandler;
#[cfg(any(unix, windows))]
pub use control::{send_control_request, ControlServer};
pub use control_protocol::{
    control_pipe_name_from_uuid, control_socket_path_from_uuid, parse_request, parse_response,
    resolve_socket_path, resolve_target_task, resolve_task_flag, ControlCommand, ControlRequest,
    ControlResponse,
};

// Drag & drop wave (`plans/012-task-model-and-control-cli.md` §3): pure
// ordering logic for the sidebar's Task reorder DnD. Appended at the tail,
// same reasoning as every other wave block in this file. The pane-tiling
// drop-edge geometry (`drop_edge_for_point`) and terminal file-drop path
// encoding (`quote_dropped_paths`) live alongside their existing modules
// (`tiling`/`hook_settings`, exported above) rather than getting a new
// module each -- see those functions' doc comments.
pub mod task_order;

pub use task_order::reorder_task_ids;

// Wave 5h (Git pane): a cross-platform (notify-based) port of
// `Sources/LaboLaboEngine/Git/FileWatcher.swift` -- watches a worktree for
// changes so `labolabo-app`'s Git pane can re-run `git status`/`diff`
// live. See the module doc comment for the debounce design and the
// `.git/` filtering this port adds over the Swift source. Appended at the
// tail, same reasoning as every other wave block in this file.
pub mod file_watcher;

pub use file_watcher::FileWatcher;

// Windows core wave (feature/rust-core-windows): the same-user DACL shared
// by both Named Pipe servers (`hooks::NamedPipeEventTransport` and
// `control::ControlServer`) -- the Windows counterpart of the unix
// transports' `chmod 0600` (docs/hooks-protocol.md §4.2/§8,
// docs/control-protocol.md §9). Crate-internal: consumers configure
// nothing here, the transports apply it themselves. Appended at the tail,
// same reasoning as every other wave block in this file.
#[cfg(windows)]
mod windows_pipe_security;
