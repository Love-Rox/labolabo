//! Pure logic for the app-layer side of `docs/hooks-protocol.md` §2/§3/§7:
//! building the `.claude/settings.local.json` hooks injection (merged with
//! any existing content, matching the Swift app's
//! `app/Sources/AgentSessionModel.swift`'s `installLocalSettings`/
//! `hookEntry`/`shellQuoted`), the `labolabo-hook <socket>` forwarder
//! command string, the per-session AF_UNIX socket path, and the Claude
//! `--resume` launch command.
//!
//! Deliberately pure/testable: every function here takes plain strings in
//! and returns plain strings/structs out, with no filesystem or process
//! access. The actual file I/O (reading the existing settings file,
//! snapshotting/restoring the `.labolabo-bak` backup, writing the merged
//! result) is `labolabo-app`'s job -- see that crate's `hooks` module --
//! mirroring how `Sources/LaboLaboEngine/Agent/AgentStatusBus.swift`'s
//! socket transport is out of scope for this crate's pure `agent_status`/
//! `agent_event_parser` modules.
//!
//! No Swift source module maps 1:1 onto this file: the merge/backup logic
//! lives inline in `AgentSessionModel.installLocalSettings`/
//! `removeLocalSettings` there (a `@MainActor` class method, not a pure
//! function) rather than factored out, so [`merge_hooks`] is a *port of that
//! logic's shape*, not a port of an existing standalone Swift function.

use serde_json::{Map, Value};

/// The 7 Claude Code hook events LaboLabo listens on (docs/hooks-protocol.md
/// §2), in the same order `AgentSessionModel.swift`'s `events` array lists
/// them. Order doesn't affect the merged JSON's observable content (object
/// keys carry no ordering guarantee, and `merge_hooks`'s output is sorted --
/// see its doc comment) -- kept in this order only for readability/diffing
/// against the Swift source.
pub const HOOK_EVENTS: [&str; 7] = [
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "Notification",
    "Stop",
    "SessionEnd",
];

/// Single-quotes `value` for embedding in a `sh -c`-executed command string,
/// escaping any embedded single quote as `'\''` (close-quote, escaped quote,
/// reopen-quote). Faithful port of Swift's `AgentAdapter.shellQuoted`/
/// `AgentSessionModel.shellQuoted` (identical algorithm, ported twice on the
/// Swift side; this crate has one copy).
pub fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// The command string for one hook's `command` field: `'<binary>' --hook
/// '<socket>'`, timeout applied separately by the entry that embeds this
/// (see [`merge_hooks`]). Port of `AgentSessionModel.hookEntry`'s command
/// construction.
pub fn hook_command(binary_path: &str, socket_path: &str) -> String {
    format!(
        "{} --hook {}",
        shell_quote(binary_path),
        shell_quote(socket_path)
    )
}

/// The `claude` launch command: bare `claude` with no `resume_id`, or `claude
/// --resume '<id>'` otherwise (an empty `resume_id` is treated the same as
/// none). Port of `AgentAdapters.claude`'s `resumeArgumentTemplate` ("--resume
/// %@") applied via `AgentAdapter.launchCommand(resumeID:)`.
pub fn claude_resume_command(resume_id: Option<&str>) -> String {
    match resume_id {
        Some(id) if !id.is_empty() => format!("claude --resume {}", shell_quote(id)),
        _ => "claude".to_string(),
    }
}

/// The per-session AF_UNIX socket path (docs/hooks-protocol.md §4):
/// `<base_dir>/<first 10 lowercase hex chars of uuid, hyphens stripped>.sock`.
/// `base_dir`'s trailing slash (if any) is stripped before joining, so
/// `"/tmp/labolabo"` and `"/tmp/labolabo/"` produce the same path.
pub fn socket_path_from_uuid(uuid: &str, base_dir: &str) -> String {
    let short: String = uuid
        .chars()
        .filter(|c| *c != '-')
        .collect::<String>()
        .to_lowercase()
        .chars()
        .take(10)
        .collect();
    format!("{}/{short}.sock", base_dir.trim_end_matches('/'))
}

/// Result of [`merge_hooks`]: the new file content, and whether `existing`
/// was absent/unparseable (i.e. this content was built from an empty root,
/// not a real prior file) -- the caller needs this to decide how to restore
/// on cleanup (delete the file it created vs. restore a backup of the real
/// prior content). Mirrors the two `AgentSessionModel.installLocalSettings`
/// branches' `createdSettings` flag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergedSettings {
    pub content: String,
    /// `true` when `existing` was `None` or not a valid JSON object --
    /// including a syntactically-invalid or non-object top level, matching
    /// Swift's `(try? JSONSerialization.jsonObject(...)) as? [String: Any]`
    /// failing silently into the "we're creating this file" branch (which,
    /// notably, does **not** snapshot a backup either -- see this crate's
    /// and `labolabo-app::hooks`'s doc comments).
    pub created: bool,
}

/// Merges LaboLabo's hook entry into `existing` (a `.claude/settings.local.json`
/// file's raw content, or `None`/unparseable if there was none) for all 7
/// [`HOOK_EVENTS`], appending (never replacing) so any other tool's -- or
/// another LaboLabo instance's, see docs/hooks-protocol.md's "同一ディレクトリ
/// の同時使用は非推奨" caveat -- existing hooks for the same event are kept.
/// `command` is the full `command` field value (build it with
/// [`hook_command`] first).
///
/// Output is pretty-printed with alphabetically sorted keys (this crate's
/// `serde_json` dependency has no `preserve_order` feature, so
/// `serde_json::Map` is a `BTreeMap`) -- not a byte-for-byte match for
/// Swift's `JSONSerialization(.prettyPrinted, .sortedKeys)` formatting (2
/// vs. 4-space indent, etc.), which doesn't matter: this file is read by
/// Claude Code's own JSON hooks-config loader, not diffed against a Swift-
/// written fixture, and any valid JSON round-trips through both.
///
/// Faithful port of `AgentSessionModel.installLocalSettings`'s merge shape:
/// same per-event-array-append behavior, same entry shape (`{"matcher": "",
/// "hooks": [{"type": "command", "command": ..., "timeout": 5}]}`), same
/// "unparseable existing content is treated as absent, and not backed up"
/// rule (see [`MergedSettings::created`]'s doc comment).
pub fn merge_hooks(existing: Option<&str>, command: &str) -> MergedSettings {
    let (mut root, created) = match existing.and_then(|s| serde_json::from_str::<Value>(s).ok()) {
        Some(Value::Object(map)) => (map, false),
        _ => (Map::new(), true),
    };

    let mut hooks = match root.remove("hooks") {
        Some(Value::Object(map)) => map,
        _ => Map::new(),
    };

    for event in HOOK_EVENTS {
        let mut array = match hooks.remove(event) {
            Some(Value::Array(arr)) => arr,
            _ => Vec::new(),
        };
        array.push(hook_entry(command));
        hooks.insert(event.to_string(), Value::Array(array));
    }
    root.insert("hooks".to_string(), Value::Object(hooks));

    let content =
        serde_json::to_string_pretty(&Value::Object(root)).unwrap_or_else(|_| "{}".to_string());
    MergedSettings { content, created }
}

fn hook_entry(command: &str) -> Value {
    serde_json::json!({
        "matcher": "",
        "hooks": [{"type": "command", "command": command, "timeout": 5}],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // MARK: - shell_quote

    #[test]
    fn shell_quote_wraps_in_single_quotes() {
        assert_eq!(
            shell_quote("/usr/local/bin/labolabo-hook"),
            "'/usr/local/bin/labolabo-hook'"
        );
    }

    #[test]
    fn shell_quote_escapes_embedded_single_quotes() {
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
    }

    #[test]
    fn shell_quote_empty_string() {
        assert_eq!(shell_quote(""), "''");
    }

    // MARK: - hook_command / claude_resume_command

    #[test]
    fn hook_command_shell_quotes_both_arguments() {
        assert_eq!(
            hook_command("/bin/labolabo-hook", "/tmp/labolabo/abc123.sock"),
            "'/bin/labolabo-hook' --hook '/tmp/labolabo/abc123.sock'"
        );
    }

    #[test]
    fn claude_resume_command_without_id_is_bare_claude() {
        assert_eq!(claude_resume_command(None), "claude");
        assert_eq!(claude_resume_command(Some("")), "claude");
    }

    #[test]
    fn claude_resume_command_with_id_appends_quoted_resume_flag() {
        assert_eq!(
            claude_resume_command(Some("sess-42")),
            "claude --resume 'sess-42'"
        );
    }

    #[test]
    fn claude_resume_command_shell_quotes_the_id() {
        assert_eq!(
            claude_resume_command(Some("a'b")),
            "claude --resume 'a'\\''b'"
        );
    }

    // MARK: - socket_path_from_uuid

    #[test]
    fn socket_path_from_uuid_strips_hyphens_lowercases_and_truncates_to_10() {
        assert_eq!(
            socket_path_from_uuid("ABCDEF01-2345-6789-ABCD-EF0123456789", "/tmp/labolabo"),
            "/tmp/labolabo/abcdef0123.sock"
        );
    }

    #[test]
    fn socket_path_from_uuid_trims_trailing_slash_on_base_dir() {
        assert_eq!(
            socket_path_from_uuid("abcdef01-2345-6789-abcd-ef0123456789", "/tmp/labolabo/"),
            "/tmp/labolabo/abcdef0123.sock"
        );
    }

    // MARK: - merge_hooks

    #[test]
    fn merge_hooks_with_no_existing_file_creates_fresh_root_and_reports_created() {
        let result = merge_hooks(None, "'/bin/hook' --hook '/tmp/s.sock'");
        assert!(result.created);
        let parsed: Value = serde_json::from_str(&result.content).unwrap();
        for event in HOOK_EVENTS {
            let entries = parsed["hooks"][event].as_array().expect("array");
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0]["matcher"], "");
            assert_eq!(
                entries[0]["hooks"][0]["command"],
                "'/bin/hook' --hook '/tmp/s.sock'"
            );
            assert_eq!(entries[0]["hooks"][0]["type"], "command");
            assert_eq!(entries[0]["hooks"][0]["timeout"], 5);
        }
    }

    #[test]
    fn merge_hooks_with_malformed_existing_content_is_treated_as_absent() {
        let result = merge_hooks(Some("{ not json"), "cmd");
        assert!(
            result.created,
            "malformed JSON should be treated as no prior file"
        );
    }

    #[test]
    fn merge_hooks_with_non_object_top_level_is_treated_as_absent() {
        let result = merge_hooks(Some("[1,2,3]"), "cmd");
        assert!(result.created);
    }

    #[test]
    fn merge_hooks_preserves_other_top_level_keys() {
        let result = merge_hooks(Some(r#"{"env": {"FOO": "bar"}}"#), "cmd");
        assert!(!result.created);
        let parsed: Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(parsed["env"]["FOO"], "bar");
    }

    #[test]
    fn merge_hooks_preserves_existing_hook_entries_for_the_same_event() {
        // Another tool's (or another LaboLabo instance's) hook entry for
        // SessionStart must survive the merge -- docs/hooks-protocol.md's
        // "既存 hooks は保持" rule.
        let existing = r#"{
            "hooks": {
                "SessionStart": [
                    {"matcher": "", "hooks": [{"type": "command", "command": "echo other-tool"}]}
                ]
            }
        }"#;
        let result = merge_hooks(Some(existing), "cmd");
        assert!(!result.created);
        let parsed: Value = serde_json::from_str(&result.content).unwrap();
        let entries = parsed["hooks"]["SessionStart"].as_array().unwrap();
        assert_eq!(entries.len(), 2, "the other tool's entry plus ours");
        assert_eq!(entries[0]["hooks"][0]["command"], "echo other-tool");
        assert_eq!(entries[1]["hooks"][0]["command"], "cmd");
    }

    #[test]
    fn merge_hooks_preserves_entries_for_events_not_in_hook_events() {
        // An event LaboLabo doesn't listen on (e.g. a hypothetical
        // "PreCompact") must be left completely untouched.
        let existing = r#"{"hooks": {"PreCompact": [{"matcher": "", "hooks": []}]}}"#;
        let result = merge_hooks(Some(existing), "cmd");
        let parsed: Value = serde_json::from_str(&result.content).unwrap();
        assert!(parsed["hooks"]["PreCompact"].is_array());
    }

    #[test]
    fn merge_hooks_is_idempotent_shaped_but_appends_on_repeated_calls() {
        // Calling merge_hooks twice with the same command (e.g. two Tasks
        // sharing a directory, or a stale double-injection) appends a
        // second identical entry rather than deduplicating -- documents the
        // (deliberate, Swift-matching) behavior so a caller relying on
        // idempotency knows to guard at a higher level (labolabo-app tracks
        // "already injected this directory" itself, see its `hooks` module).
        let once = merge_hooks(None, "cmd");
        let twice = merge_hooks(Some(&once.content), "cmd");
        let parsed: Value = serde_json::from_str(&twice.content).unwrap();
        assert_eq!(parsed["hooks"]["SessionStart"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn merge_hooks_covers_all_seven_events() {
        let result = merge_hooks(None, "cmd");
        let parsed: Value = serde_json::from_str(&result.content).unwrap();
        let hooks = parsed["hooks"].as_object().unwrap();
        assert_eq!(hooks.len(), 7);
        for event in HOOK_EVENTS {
            assert!(hooks.contains_key(event), "missing {event}");
        }
    }
}
