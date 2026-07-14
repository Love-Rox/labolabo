//! Faithful port of `Sources/LaboLaboStore/SessionRecord.swift`.

use chrono::{DateTime, Utc};

/// A persisted, opened repository/worktree session. Phase-0 restore-on-launch
/// reopens these; richer state (terminal tabs/panes, agent session ids) is
/// added in later migrations following the cmux model.
///
/// Field-for-field port of the Swift `SessionRecord` (`Codable,
/// FetchableRecord, PersistableRecord, Identifiable, Equatable, Sendable`).
/// `added_at` is `chrono::DateTime<Utc>` rather than a bespoke type: GRDB
/// stores `Date` columns as `"yyyy-MM-dd HH:mm:ss.SSS"` text in UTC
/// (millisecond precision — see `store::database`'s `format_grdb_datetime`/
/// `parse_grdb_datetime` doc comments for the exact compatibility contract),
/// so round-tripping through this crate never needs more than millisecond
/// resolution; `DateTime<Utc>` carries that faithfully without forcing a
/// timezone-naive representation on callers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRecord {
    /// UUID string (stable across launches).
    pub id: String,
    pub worktree_path: String,
    pub name: String,
    pub branch: Option<String>,
    pub added_at: DateTime<Utc>,
    pub sort_order: i64,
    /// 直近のエージェント（Claude）セッション ID。次回起動時の `--resume` に使う。
    pub agent_session_id: Option<String>,
    /// 直近の transcript(JSONL) パス。usage/cost の best-effort 取得などに使う。
    pub transcript_path: Option<String>,
    /// このセッションのエージェント種別（"claude" / "codex" / "gemini"）。`None` は既定＝Claude。
    pub adapter_id: Option<String>,
}

impl SessionRecord {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<String>,
        worktree_path: impl Into<String>,
        name: impl Into<String>,
        branch: Option<String>,
        added_at: DateTime<Utc>,
        sort_order: i64,
        agent_session_id: Option<String>,
        transcript_path: Option<String>,
        adapter_id: Option<String>,
    ) -> Self {
        Self {
            id: id.into(),
            worktree_path: worktree_path.into(),
            name: name.into(),
            branch,
            added_at,
            sort_order,
            agent_session_id,
            transcript_path,
            adapter_id,
        }
    }

    /// `"session"` — mirrors Swift's `static let databaseTableName`.
    pub const TABLE_NAME: &'static str = "session";
}

// `PartialEq`/`Eq` derive above requires `DateTime<Utc>: Eq`, which it is
// (unlike `f64`-backed types, `DateTime` compares its integer nanosecond
// timestamp).
