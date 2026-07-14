//! `labolabo-core`: the OS/UI-independent pure-logic core of LaboLabo,
//! ported from the Swift `LaboLaboEngine` package (`Sources/LaboLaboEngine/Git/`).
//!
//! This is the first wave of the cross-platform (Rust) migration. It covers
//! only the pure parsers with no process/filesystem/concurrency dependencies:
//!
//! - `git_models`: data types ported from `GitModels.swift`.
//! - `porcelain`: `git status --porcelain=v2 -z` parser, ported from
//!   `PorcelainStatusParser.swift`.
//! - `unified_diff`: unified `git diff` parser, ported from
//!   `UnifiedDiffParser.swift`.
//!
//! Correctness is anchored to the Swift implementation as the "golden
//! oracle": `tests/golden.rs` runs this crate's parsers over the same input
//! corpus the Swift parsers were run over (see `fixtures/`) and asserts
//! byte-identical canonical JSON output. See `README.md` for how the
//! expected-output fixtures were generated and how to regenerate them.

pub mod git_models;
pub mod porcelain;
pub mod unified_diff;
mod util;

pub use git_models::{Change, GitFileEntry, GitStatus, Kind};
pub use unified_diff::{DiffHunk, DiffLine, FileDiff, LineKind};
