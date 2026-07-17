//! Faithful port of `Sources/LaboLaboEngine/Agent/AgentStatus.swift`.
//!
//! The wire protocol these types model is specified in full in
//! `docs/hooks-protocol.md` (checked at the repo root) — that document is
//! the canonical spec; this port was cross-checked against it directly
//! (see `agent_event_parser.rs` for the parsing side). No divergence was
//! found between the doc and the Swift source for the pieces ported here.

/// The live state of one agent (Claude Code, etc.) session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgentStatus {
    /// 未起動 / 未接続
    None,
    /// SessionStart
    Starting,
    /// UserPromptSubmit / PreToolUse / PostToolUse（思考・ツール実行中）
    Running,
    /// Notification（入力・許可待ち）
    WaitingForInput,
    /// Stop（応答完了・待機）
    Idle,
    /// SessionEnd
    Ended,
}

impl AgentStatus {
    /// Mirrors the Swift `RawRepresentable` (`enum AgentStatus: String`)
    /// conformance: the persisted/wire string spelling of each case.
    pub fn raw_value(self) -> &'static str {
        match self {
            AgentStatus::None => "none",
            AgentStatus::Starting => "starting",
            AgentStatus::Running => "running",
            AgentStatus::WaitingForInput => "waitingForInput",
            AgentStatus::Idle => "idle",
            AgentStatus::Ended => "ended",
        }
    }

    /// Inverse of `raw_value`; unknown strings are `None` (Rust `Option`,
    /// not the `AgentStatus::None` case — mirrors Swift's failable
    /// `init?(rawValue:)`).
    pub fn from_raw_value(raw: &str) -> Option<AgentStatus> {
        match raw {
            "none" => Some(AgentStatus::None),
            "starting" => Some(AgentStatus::Starting),
            "running" => Some(AgentStatus::Running),
            "waitingForInput" => Some(AgentStatus::WaitingForInput),
            "idle" => Some(AgentStatus::Idle),
            "ended" => Some(AgentStatus::Ended),
            _ => None,
        }
    }

    /// Maps a Claude hook's `hook_event_name` (plus, for `SessionStart`
    /// only, its `source` field) to a status (unknown events -> `None`).
    ///
    /// `SessionStart` fires with `source: "startup" | "resume" | "clear" |
    /// "compact"` (Claude Code hooks reference). `"compact"` fires after
    /// auto/manual context compaction **mid-task** -- the agent is not
    /// starting, it is continuing the same turn it was already running
    /// before compaction kicked in. Mapping it to `Starting` like the other
    /// three sources would flip a session that was showing `Running` back
    /// to `Starting` even though no work stopped, which is the root cause
    /// of the "indicator doesn't stay green while the session is actually
    /// running" report this port fixes. `"startup"`/`"resume"`/`"clear"`
    /// (and a missing/unrecognized `source`, for forward/backward
    /// compatibility with payloads that omit it) keep the original
    /// unconditional `Starting` mapping.
    ///
    /// **Known divergence from the Swift app**: `Sources/LaboLaboEngine/
    /// Agent/AgentStatus.swift`'s `AgentStatus.from(hookEvent:)` does not
    /// take a `source` parameter at all and always maps `SessionStart` to
    /// `.starting`, so the Swift app still shows this same regression after
    /// compaction. Not backported there as part of this fix (Rust-only
    /// task); see `docs/hooks-protocol.md` §5's mapping table for the
    /// tracked note.
    pub fn from_hook_event(hook_event: &str, source: Option<&str>) -> Option<AgentStatus> {
        match hook_event {
            "SessionStart" => Some(match source {
                Some("compact") => AgentStatus::Running,
                _ => AgentStatus::Starting,
            }),
            "UserPromptSubmit" | "PreToolUse" | "PostToolUse" => Some(AgentStatus::Running),
            "Notification" => Some(AgentStatus::WaitingForInput),
            "Stop" | "SubagentStop" => Some(AgentStatus::Idle),
            "SessionEnd" => Some(AgentStatus::Ended),
            _ => None,
        }
    }
}

/// One event received from the hook forwarder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentStatusEvent {
    pub hook_event: String,
    pub status: AgentStatus,
    /// The hook payload's `source` field (`SessionStart` only in practice --
    /// `"startup" | "resume" | "clear" | "compact"`; `None` for every other
    /// `hook_event` and for payloads that omit it). Kept on the event
    /// alongside `status` (rather than only consulted transiently while
    /// computing `status`) so a consumer that logs/inspects raw events can
    /// see which `SessionStart` sub-case produced a given status -- see
    /// [`AgentStatus::from_hook_event`]'s doc comment for why `"compact"`
    /// maps to `Running` instead of `Starting`.
    pub source: Option<String>,
    pub session_id: Option<String>,
    pub transcript_path: Option<String>,
    pub cwd: Option<String>,
    /// Terminal pane id (UUID string) the forwarder attached from the
    /// `LABOLABO_PANE` environment variable. `None` for events from
    /// anything other than a LaboLabo-spawned terminal (e.g. an external one).
    pub pane_id: Option<String>,
    /// Work/task id (UUID string) the forwarder attached from the
    /// `LABOLABO_TASK` environment variable (`docs/hooks-protocol.md` §7's
    /// reserved `labolabo_task_id` field; `plans/012` §1's Task model).
    /// `None` for events without a resolvable task -- routing then falls
    /// back to whatever `pane_id` resolves to on the consumer side.
    pub task_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    // Ported 1:1 from Tests/LaboLaboEngineTests/AgentStatusMappingTests.swift.

    #[test]
    fn session_start_maps_to_starting() {
        assert_eq!(
            AgentStatus::from_hook_event("SessionStart", None),
            Some(AgentStatus::Starting)
        );
    }

    /// `source: "startup" | "resume" | "clear"` all keep the original
    /// unconditional `Starting` mapping -- only `"compact"` is special (see
    /// `session_start_compact_maps_to_running` below).
    #[test]
    fn session_start_non_compact_sources_map_to_starting() {
        for source in ["startup", "resume", "clear"] {
            assert_eq!(
                AgentStatus::from_hook_event("SessionStart", Some(source)),
                Some(AgentStatus::Starting),
                "source={source}"
            );
        }
    }

    /// The regression this fix targets: auto/manual compaction fires
    /// `SessionStart` mid-task (`docs/hooks-protocol.md` §5), not at the
    /// start of a fresh session -- it must not flip a running indicator back
    /// to `Starting`.
    #[test]
    fn session_start_compact_maps_to_running() {
        assert_eq!(
            AgentStatus::from_hook_event("SessionStart", Some("compact")),
            Some(AgentStatus::Running)
        );
    }

    #[test]
    fn running_events_map_to_running() {
        // 思考・ツール実行中はすべて .running に集約される。
        assert_eq!(
            AgentStatus::from_hook_event("UserPromptSubmit", None),
            Some(AgentStatus::Running)
        );
        assert_eq!(
            AgentStatus::from_hook_event("PreToolUse", None),
            Some(AgentStatus::Running)
        );
        assert_eq!(
            AgentStatus::from_hook_event("PostToolUse", None),
            Some(AgentStatus::Running)
        );
    }

    #[test]
    fn notification_maps_to_waiting_for_input() {
        assert_eq!(
            AgentStatus::from_hook_event("Notification", None),
            Some(AgentStatus::WaitingForInput)
        );
    }

    #[test]
    fn stop_events_map_to_idle() {
        // Stop も SubagentStop も応答完了＝待機。
        assert_eq!(
            AgentStatus::from_hook_event("Stop", None),
            Some(AgentStatus::Idle)
        );
        assert_eq!(
            AgentStatus::from_hook_event("SubagentStop", None),
            Some(AgentStatus::Idle)
        );
    }

    #[test]
    fn session_end_maps_to_ended() {
        assert_eq!(
            AgentStatus::from_hook_event("SessionEnd", None),
            Some(AgentStatus::Ended)
        );
    }

    #[test]
    fn unknown_and_empty_events_map_to_none() {
        assert_eq!(AgentStatus::from_hook_event("", None), None);
        assert_eq!(AgentStatus::from_hook_event("Bogus", None), None);
        assert_eq!(AgentStatus::from_hook_event("sessionstart", None), None); // 大文字小文字は区別される
        assert_eq!(AgentStatus::from_hook_event(" SessionStart", None), None); // 前後空白も未知扱い
        assert_eq!(AgentStatus::from_hook_event("PreToolUse ", None), None); // 末尾空白も未知扱い
    }

    #[test]
    fn raw_values() {
        assert_eq!(AgentStatus::None.raw_value(), "none");
        assert_eq!(AgentStatus::Starting.raw_value(), "starting");
        assert_eq!(AgentStatus::Running.raw_value(), "running");
        assert_eq!(AgentStatus::WaitingForInput.raw_value(), "waitingForInput");
        assert_eq!(AgentStatus::Idle.raw_value(), "idle");
        assert_eq!(AgentStatus::Ended.raw_value(), "ended");
    }

    #[test]
    fn raw_value_round_trip() {
        // raw value から復元でき、対称であること。
        assert_eq!(
            AgentStatus::from_raw_value("waitingForInput"),
            Some(AgentStatus::WaitingForInput)
        );
        assert_eq!(AgentStatus::from_raw_value("unknown-status"), None);
    }

    #[test]
    fn event_stores_all_fields() {
        let event = AgentStatusEvent {
            hook_event: "Notification".to_string(),
            status: AgentStatus::WaitingForInput,
            source: None,
            session_id: Some("sess-42".to_string()),
            transcript_path: Some("/tmp/transcript.jsonl".to_string()),
            cwd: Some("/Users/dev/repo".to_string()),
            pane_id: None,
            task_id: None,
        };
        assert_eq!(event.hook_event, "Notification");
        assert_eq!(event.status, AgentStatus::WaitingForInput);
        assert_eq!(event.session_id.as_deref(), Some("sess-42"));
        assert_eq!(
            event.transcript_path.as_deref(),
            Some("/tmp/transcript.jsonl")
        );
        assert_eq!(event.cwd.as_deref(), Some("/Users/dev/repo"));
    }

    #[test]
    fn event_allows_none_optional_fields() {
        // sessionID / transcriptPath / cwd は省略（None）可能。
        let event = AgentStatusEvent {
            hook_event: "SessionEnd".to_string(),
            status: AgentStatus::Ended,
            source: None,
            session_id: None,
            transcript_path: None,
            cwd: None,
            pane_id: None,
            task_id: None,
        };
        assert_eq!(event.hook_event, "SessionEnd");
        assert_eq!(event.status, AgentStatus::Ended);
        assert_eq!(event.session_id, None);
        assert_eq!(event.transcript_path, None);
        assert_eq!(event.cwd, None);
    }
}
