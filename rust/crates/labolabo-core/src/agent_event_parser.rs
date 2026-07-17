//! Faithful port of `Sources/LaboLaboEngine/Agent/AgentEventParser.swift`.
//!
//! Interprets one message (raw JSON bytes) from the hook forwarder into an
//! `AgentStatusEvent`. Transport-independent by design (mirrors the Swift
//! split between `AgentEventParser` — this file — and the `AgentEventTransport`
//! trait implemented by AF_UNIX on macOS/Linux and, eventually, something
//! else on Windows): cross-platform work only needs to swap the transport;
//! this interpretation layer and the wire spec (`docs/hooks-protocol.md`)
//! are shared across all platforms. The `AgentEventTransport`/`AgentStatusBus`
//! socket-transport layer itself (`AgentStatusBus.swift`) is process/socket
//! infrastructure, not pure logic, and is out of scope for this crate.
//!
//! Verified against `docs/hooks-protocol.md` §5 (field table + "破棄規則")
//! directly: field names, the drop-on-unknown-event rule, and the
//! empty/malformed-payload drop rule all match this port with no divergence
//! found.

use serde_json::Value;

use crate::agent_status::{AgentStatus, AgentStatusEvent};

/// Interprets one raw hook-event payload. Invalid JSON, a non-object top
/// level, or an unknown/missing `hook_event_name` all silently return
/// `None` (mirrors the Swift `guard ... else { return nil }` chain — the
/// caller drops the event with no error, no log).
pub fn parse(data: &[u8]) -> Option<AgentStatusEvent> {
    if data.is_empty() {
        return None;
    }
    let value: Value = serde_json::from_slice(data).ok()?;
    let object = value.as_object()?;
    let hook_event = object
        .get("hook_event_name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let source = object
        .get("source")
        .and_then(Value::as_str)
        .map(String::from);
    let status = AgentStatus::from_hook_event(&hook_event, source.as_deref())?;
    Some(AgentStatusEvent {
        hook_event,
        status,
        source,
        session_id: object
            .get("session_id")
            .and_then(Value::as_str)
            .map(String::from),
        transcript_path: object
            .get("transcript_path")
            .and_then(Value::as_str)
            .map(String::from),
        cwd: object.get("cwd").and_then(Value::as_str).map(String::from),
        pane_id: object
            .get("labolabo_pane_id")
            .and_then(Value::as_str)
            .map(String::from),
        task_id: object
            .get("labolabo_task_id")
            .and_then(Value::as_str)
            .map(String::from),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Ported 1:1 from Tests/LaboLaboEngineTests/AgentEventParserTests.swift.
    // (AgentStatusBusTransportInjectionTests, the socket round-trip test in
    // the same Swift file, exercises AgentStatusBus/AgentEventTransport —
    // out of scope here, see module doc comment.)

    fn parse_str(json: &str) -> Option<AgentStatusEvent> {
        parse(json.as_bytes())
    }

    #[test]
    fn parses_full_event() {
        let event = parse_str(
            r#"{"hook_event_name":"SessionStart","session_id":"s1","transcript_path":"/t.jsonl","cwd":"/w","labolabo_pane_id":"P1","labolabo_task_id":"T1"}"#,
        )
        .expect("event");
        assert_eq!(event.status, AgentStatus::Starting);
        assert_eq!(event.hook_event, "SessionStart");
        assert_eq!(event.session_id.as_deref(), Some("s1"));
        assert_eq!(event.transcript_path.as_deref(), Some("/t.jsonl"));
        assert_eq!(event.cwd.as_deref(), Some("/w"));
        assert_eq!(event.pane_id.as_deref(), Some("P1"));
        assert_eq!(event.task_id.as_deref(), Some("T1"));
    }

    #[test]
    fn optional_fields_may_be_absent() {
        let event = parse_str(r#"{"hook_event_name":"Stop"}"#).expect("event");
        assert_eq!(event.status, AgentStatus::Idle);
        assert_eq!(event.source, None);
        assert_eq!(event.session_id, None);
        assert_eq!(event.transcript_path, None);
        assert_eq!(event.cwd, None);
        assert_eq!(event.pane_id, None);
        assert_eq!(event.task_id, None);
    }

    /// End-to-end regression coverage for the `SessionStart`/`source:
    /// "compact"` fix (`AgentStatus::from_hook_event`'s doc comment): the
    /// real wire payload Claude Code sends after auto/manual context
    /// compaction must parse to `Running`, not `Starting`, and `source`
    /// itself must round-trip onto the event.
    #[test]
    fn session_start_with_compact_source_maps_to_running() {
        let event =
            parse_str(r#"{"hook_event_name":"SessionStart","source":"compact","session_id":"s1"}"#)
                .expect("event");
        assert_eq!(event.status, AgentStatus::Running);
        assert_eq!(event.source.as_deref(), Some("compact"));
    }

    /// A `source` other than `"compact"` (or none at all) is unaffected --
    /// still maps `SessionStart` to `Starting`, and `source` still
    /// round-trips onto the event for a non-`SessionStart` hook too (the
    /// field is parsed unconditionally, not gated on `hook_event_name`).
    #[test]
    fn source_field_is_parsed_for_any_hook_event() {
        let startup =
            parse_str(r#"{"hook_event_name":"SessionStart","source":"startup"}"#).expect("event");
        assert_eq!(startup.status, AgentStatus::Starting);
        assert_eq!(startup.source.as_deref(), Some("startup"));

        // `source` isn't documented on non-SessionStart events, but nothing
        // stops the parser from picking it up if present -- matches
        // docs/hooks-protocol.md §9's forward-compatible "未知フィールドは無視"
        // stance applied in reverse (known field, unexpected event).
        let stop = parse_str(r#"{"hook_event_name":"Stop","source":"startup"}"#).expect("event");
        assert_eq!(stop.status, AgentStatus::Idle);
        assert_eq!(stop.source.as_deref(), Some("startup"));
    }

    /// `labolabo_task_id` can be present without `labolabo_pane_id` (e.g. a
    /// future task-level-only forwarder annotation) -- the two fields are
    /// parsed independently.
    #[test]
    fn task_id_parses_independently_of_pane_id() {
        let event = parse_str(r#"{"hook_event_name":"SessionEnd","labolabo_task_id":"T9"}"#)
            .expect("event");
        assert_eq!(event.pane_id, None);
        assert_eq!(event.task_id.as_deref(), Some("T9"));
    }

    #[test]
    fn unknown_hook_event_is_dropped() {
        assert_eq!(parse_str(r#"{"hook_event_name":"Mystery"}"#), None);
    }

    #[test]
    fn malformed_or_empty_payload_is_dropped() {
        assert_eq!(parse_str("{ broken"), None);
        assert_eq!(parse(&[]), None);
    }

    /// 未知フィールドは無視される（仕様書の前方互換方針: フィールド追加は互換）。
    #[test]
    fn unknown_fields_are_ignored() {
        let event =
            parse_str(r#"{"hook_event_name":"Notification","future_field":123}"#).expect("event");
        assert_eq!(event.status, AgentStatus::WaitingForInput);
    }

    /// Not part of the ported Swift suite: documents that a top-level JSON
    /// value that isn't an object (matching Swift's `as? [String: Any]`
    /// failing for a non-dictionary top level) is dropped too.
    #[test]
    fn non_object_top_level_is_dropped() {
        assert_eq!(parse_str("[1,2,3]"), None);
    }
}
