//! `Task::agent_bindings`'s decoded shape: the docs/hooks-protocol.md §6(a)
//! "session (here: Task/worktree) -- unit last-known Claude session id"
//! fallback record.
//!
//! This is deliberately narrower than the per-tab session bindings
//! (docs/hooks-protocol.md §6(b), "labolabo_pane_id があれば...の対応をレイ
//! アウトと一緒に永続化") -- those already round-trip byte-for-byte through
//! `Task::layout` (`tiling::PaneItem::agent_session_id`/
//! `agent_transcript_path`, restored positionally by
//! `PaneTilingModel::model_from`; see `tiling.rs`'s module doc comment), so
//! there is nothing left for `agent_bindings` to do at the per-tab level.
//! What `agent_bindings` *does* add, matching §6(a) and the Swift
//! `RepoSession.agentSessionID`/`transcriptPath` (session-level) fields
//! `SessionStore.updateAgentSession` maintains: a Task-scoped fallback that
//! updates on *every* `session_id`-bearing hook event for a resolvable Task
//! (docs/hooks-protocol.md §7's `labolabo_task_id`, or the pane-routing
//! table's task, on the `labolabo-app` side), independent of whether that
//! event's `labolabo_pane_id` also resolved to a still-live pane in this
//! run. `labolabo-app`'s restore-time resume flow (per-tab, see
//! `tiling::PaneItem::is_resumable`) does not currently consult this record
//! -- see its module doc comment/README for why -- but it is available for
//! a future control-CLI/introspection use, which is the reserved column's
//! stated purpose (`store::task_record`'s doc comment).

use serde::{Deserialize, Serialize};

/// The Task-level Claude session fallback (docs/hooks-protocol.md §6(a)).
/// `Task::agent_bindings`'s JSON content, once decoded.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentBindings {
    #[serde(rename = "lastSessionId", skip_serializing_if = "Option::is_none")]
    pub last_session_id: Option<String>,
    #[serde(rename = "lastTranscriptPath", skip_serializing_if = "Option::is_none")]
    pub last_transcript_path: Option<String>,
}

impl AgentBindings {
    /// Serializes to the `Task::agent_bindings` column's stored JSON text.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".to_string())
    }

    /// Parses `Task::agent_bindings`'s stored JSON text. `None` for `None`
    /// (no binding yet) or unparseable content (treated the same as absent
    /// -- a corrupt/foreign value should not crash restore, just start
    /// fresh, matching this crate's general "unknown/invalid persisted data
    /// degrades gracefully" posture, e.g. `TaskStatus::parse`/
    /// `PaneKind::from_raw_value`).
    pub fn from_json(json: Option<&str>) -> Self {
        json.and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default()
    }

    /// Records a newly observed `(session_id, transcript_path)` pair,
    /// keeping the previous `last_transcript_path` if this event didn't
    /// carry one (mirrors `tiling::PaneTilingModel::record_agent_session`'s
    /// `transcript_path.or_else(...)` fallback and Swift's
    /// `SessionStore.updateAgentSession`). Returns whether anything actually
    /// changed, so the caller can skip a redundant DB write on a dedup hit
    /// (same "同値なら再保存しない" rule, docs/hooks-protocol.md §6).
    pub fn record(&mut self, session_id: &str, transcript_path: Option<&str>) -> bool {
        let new_transcript = transcript_path
            .map(str::to_string)
            .or_else(|| self.last_transcript_path.clone());
        if self.last_session_id.as_deref() == Some(session_id)
            && self.last_transcript_path == new_transcript
        {
            return false;
        }
        self.last_session_id = Some(session_id.to_string());
        self.last_transcript_path = new_transcript;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_json_none_is_empty_default() {
        assert_eq!(AgentBindings::from_json(None), AgentBindings::default());
    }

    #[test]
    fn from_json_invalid_json_degrades_to_default() {
        assert_eq!(
            AgentBindings::from_json(Some("{ not json")),
            AgentBindings::default()
        );
    }

    #[test]
    fn to_json_round_trips_through_from_json() {
        let mut bindings = AgentBindings::default();
        bindings.record("sid-1", Some("/tmp/t.jsonl"));
        let json = bindings.to_json();
        assert_eq!(AgentBindings::from_json(Some(&json)), bindings);
    }

    #[test]
    fn empty_bindings_serialize_to_empty_object() {
        assert_eq!(AgentBindings::default().to_json(), "{}");
    }

    #[test]
    fn record_sets_both_fields_on_first_call() {
        let mut bindings = AgentBindings::default();
        let changed = bindings.record("sid-1", Some("/tmp/t.jsonl"));
        assert!(changed);
        assert_eq!(bindings.last_session_id.as_deref(), Some("sid-1"));
        assert_eq!(
            bindings.last_transcript_path.as_deref(),
            Some("/tmp/t.jsonl")
        );
    }

    #[test]
    fn record_keeps_previous_transcript_path_when_new_event_omits_it() {
        let mut bindings = AgentBindings::default();
        bindings.record("sid-1", Some("/tmp/t.jsonl"));
        let changed = bindings.record("sid-1", None);
        // session id + transcript path both unchanged -> no-op.
        assert!(!changed);
        assert_eq!(
            bindings.last_transcript_path.as_deref(),
            Some("/tmp/t.jsonl")
        );
    }

    #[test]
    fn record_updates_transcript_path_when_session_id_unchanged_but_path_changes() {
        let mut bindings = AgentBindings::default();
        bindings.record("sid-1", Some("/tmp/t.jsonl"));
        let changed = bindings.record("sid-1", Some("/tmp/other.jsonl"));
        assert!(changed);
        assert_eq!(
            bindings.last_transcript_path.as_deref(),
            Some("/tmp/other.jsonl")
        );
    }

    #[test]
    fn record_returns_false_on_exact_duplicate() {
        let mut bindings = AgentBindings::default();
        bindings.record("sid-1", Some("/tmp/t.jsonl"));
        assert!(!bindings.record("sid-1", Some("/tmp/t.jsonl")));
    }

    #[test]
    fn record_returns_true_when_session_id_changes() {
        let mut bindings = AgentBindings::default();
        bindings.record("sid-1", Some("/tmp/t.jsonl"));
        assert!(bindings.record("sid-2", Some("/tmp/t.jsonl")));
        assert_eq!(bindings.last_session_id.as_deref(), Some("sid-2"));
    }
}
