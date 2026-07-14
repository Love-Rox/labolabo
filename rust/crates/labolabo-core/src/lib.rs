//! `labolabo-core`: the OS/UI-independent pure-logic core of LaboLabo,
//! ported from the Swift `LaboLaboEngine` package.
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
//! Correctness is anchored to the Swift implementation as the "golden
//! oracle": `tests/golden.rs` runs this crate's parsers over the same input
//! corpus the Swift parsers were run over (see `fixtures/`) and asserts
//! byte-identical canonical JSON output for the parser modules (porcelain,
//! unified_diff, worktree, transcript_usage, agent_event_parser). The pure
//! algorithm modules (commit_graph, cross_session_conflicts, release_version)
//! are covered by unit tests ported 1:1 from the corresponding Swift XCTest
//! suites instead — see `README.md` for why (and for how the golden fixtures
//! were generated / how to regenerate them).

pub mod agent_event_parser;
pub mod agent_status;
pub mod commit_graph;
pub mod cross_session_conflicts;
pub mod git_models;
pub mod porcelain;
pub mod release_version;
pub mod transcript_usage;
pub mod unified_diff;
mod util;
pub mod worktree;

pub use agent_status::{AgentStatus, AgentStatusEvent};
pub use git_models::{Change, GitFileEntry, GitStatus, Kind};
pub use transcript_usage::AgentUsage;
pub use unified_diff::{DiffHunk, DiffLine, FileDiff, LineKind};
pub use worktree::Worktree;
