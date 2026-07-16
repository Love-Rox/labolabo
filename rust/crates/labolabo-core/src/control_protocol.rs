//! Pure logic for LaboLabo's **control protocol** (`docs/control-protocol.md`,
//! repo root): the JSON request/response schema the `labolabo` CLI and
//! `labolabo-app` speak over the control socket. Symmetric split with
//! `hook_settings`/`hooks`: hooks is the agent-to-app receive-only wire
//! (`docs/hooks-protocol.md`); control is a bidirectional CLI/agent-to-app
//! RPC (`plans/012-task-model-and-control-cli.md` §2), a **separate**
//! channel. This module is the pure, I/O-free half (request/response
//! (de)serialization, the socket-path naming convention, and the `--task
//! current`/"ambient context" resolution rules -- docs/control-protocol.md
//! §4) -- unit-tested directly, no socket involved. The actual AF_UNIX
//! transport (bidirectional, one request/one response per connection, per
//! docs/control-protocol.md §3) lives in `crate::control`, mirroring how
//! `hooks.rs` is `hook_settings.rs`'s I/O counterpart.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

// MARK: - Wire types (docs/control-protocol.md §6)

/// One control request's wire shape: the command name, its command-specific
/// `params`, and the *ambient* Task/pane context the CLI read from
/// `LABOLABO_TASK`/`LABOLABO_PANE` when it ran (docs/control-protocol.md
/// §4.2) -- mirrors `hooks::annotate_ids`'s `labolabo_pane_id`/
/// `labolabo_task_id` annotation, but attached by the CLI itself here
/// rather than a forwarder downstream of Claude Code.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ControlRequest {
    pub command: String,
    #[serde(default = "default_params")]
    pub params: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub labolabo_task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub labolabo_pane_id: Option<String>,
}

fn default_params() -> Value {
    Value::Object(serde_json::Map::new())
}

impl ControlRequest {
    pub fn new(command: impl Into<String>, params: Value) -> Self {
        Self {
            command: command.into(),
            params,
            labolabo_task_id: None,
            labolabo_pane_id: None,
        }
    }

    /// Attaches the ambient `LABOLABO_TASK`/`LABOLABO_PANE` context read
    /// from `env` (docs/control-protocol.md §4.2). Empty values are treated
    /// as absent -- same rule `hooks::annotate_ids` uses for the hooks wire.
    pub fn with_ambient_context(mut self, env: &HashMap<String, String>) -> Self {
        self.labolabo_task_id = env.get("LABOLABO_TASK").filter(|v| !v.is_empty()).cloned();
        self.labolabo_pane_id = env.get("LABOLABO_PANE").filter(|v| !v.is_empty()).cloned();
        self
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("ControlRequest always serializes")
    }
}

/// Parses a request's raw wire bytes (docs/control-protocol.md §6). Unlike
/// the hooks wire (which silently drops anything malformed --
/// docs/hooks-protocol.md §5's "破棄規則"), a control request that fails to
/// parse gets an explicit [`ControlResponse::err`] reply: the caller is a
/// synchronous CLI waiting for a response, not a fire-and-forget hook (see
/// docs/control-protocol.md §6's note on this deliberate difference).
pub fn parse_request(bytes: &[u8]) -> Result<ControlRequest, String> {
    serde_json::from_slice(bytes).map_err(|err| format!("invalid request JSON: {err}"))
}

/// One control response's wire shape (docs/control-protocol.md §6):
/// `{"ok": true, "result": {...}}` or `{"ok": false, "error": "..."}`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ControlResponse {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl ControlResponse {
    pub fn ok(result: Value) -> Self {
        Self {
            ok: true,
            result: Some(result),
            error: None,
        }
    }

    pub fn ok_empty() -> Self {
        Self {
            ok: true,
            result: None,
            error: None,
        }
    }

    pub fn err(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            result: None,
            error: Some(message.into()),
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self)
            .unwrap_or_else(|_| br#"{"ok":false,"error":"failed to serialize response"}"#.to_vec())
    }
}

/// Parses a response's raw wire bytes -- the CLI-side counterpart of
/// [`parse_request`]. A response that isn't valid JSON (a crashed/wedged
/// server closing the connection early, say) becomes a `String` error the
/// caller surfaces as its own app-side error, not a panic.
pub fn parse_response(bytes: &[u8]) -> Result<ControlResponse, String> {
    serde_json::from_slice(bytes).map_err(|err| format!("invalid response JSON: {err}"))
}

// MARK: - Socket path (docs/control-protocol.md §3)

/// The control socket's path convention: `<base_dir>/control-<first 10
/// lowercase hex chars of uuid, hyphens stripped>.sock` -- the same
/// short-hex scheme as `hook_settings::socket_path_from_uuid`, kept as a
/// separate function (not a shared prefix parameter) so neither call site
/// risks accidentally colliding on the other's namespace. One socket per
/// app instance; base dir shared with hooks (`/tmp/labolabo`, 0700), but
/// the `control-` prefix keeps the two socket families visually and
/// lexically distinct in that directory.
pub fn control_socket_path_from_uuid(uuid: &str, base_dir: &str) -> String {
    let short: String = uuid
        .chars()
        .filter(|c| *c != '-')
        .collect::<String>()
        .to_lowercase()
        .chars()
        .take(10)
        .collect();
    format!("{}/control-{short}.sock", base_dir.trim_end_matches('/'))
}

/// The control channel's Windows Named Pipe name
/// (docs/control-protocol.md §9):
/// `\\.\pipe\labolabo-control-<first 10 lowercase hex chars of uuid,
/// hyphens stripped>` -- the Windows counterpart of
/// [`control_socket_path_from_uuid`], mirroring how
/// `hook_settings::hook_pipe_name_from_uuid` mirrors
/// `hook_settings::socket_path_from_uuid`: same 10-hex instance token, no
/// base directory (pipe names live in the flat `\\.\pipe\` namespace, not
/// the filesystem), and the `-control-` infix keeps the two pipe families
/// lexically distinct in that namespace the way the `control-` file-name
/// prefix does under `/tmp/labolabo` on unix. Pure string logic, compiled
/// and unit-tested on every platform; only the Windows transport
/// (`control::ControlServer`) consumes it in production.
pub fn control_pipe_name_from_uuid(uuid: &str) -> String {
    let short: String = uuid
        .chars()
        .filter(|c| *c != '-')
        .collect::<String>()
        .to_lowercase()
        .chars()
        .take(10)
        .collect();
    format!(r"\\.\pipe\labolabo-control-{short}")
}

// MARK: - CLI-side resolution (pure, docs/control-protocol.md §4)

/// Resolves the control socket path the CLI should connect to
/// (docs/control-protocol.md §4.1): `--socket` flag first, then env
/// `LABOLABO_CONTROL_SOCKET`, else an error. **No auto-discovery** --
/// deliberate: with multiple LaboLabo instances running, guessing a socket
/// would risk wiring a command to the wrong instance (`plans/012` §2's
/// security-boundary note).
pub fn resolve_socket_path(
    flag: Option<&str>,
    env: &HashMap<String, String>,
) -> Result<String, String> {
    if let Some(path) = flag {
        if !path.is_empty() {
            return Ok(path.to_string());
        }
    }
    if let Some(path) = env.get("LABOLABO_CONTROL_SOCKET") {
        if !path.is_empty() {
            return Ok(path.clone());
        }
    }
    Err(
        "no control socket: pass --socket <path> or run inside a LaboLabo-spawned pane \
         (LABOLABO_CONTROL_SOCKET)"
            .to_string(),
    )
}

/// Resolves a `--task <id|current>` flag's *client-side* meaning
/// (docs/control-protocol.md §4.2, used by `tab_open`/`tab_list` --
/// `focus`'s `--task`/`--pane` are always literal, see §5.4 and
/// `ControlCommand::from_request`'s doc comment): a concrete id is passed
/// straight through as the request's `params.task`; the literal value
/// `"current"`, or the flag being entirely absent, both collapse to `None`
/// -- "use whatever `labolabo_task_id` the request's ambient context
/// carries" ([`ControlRequest::with_ambient_context`]), which is exactly
/// `--task current`'s definition (env `LABOLABO_TASK`). An empty string is
/// treated the same as absent.
pub fn resolve_task_flag(flag: Option<&str>) -> Option<String> {
    match flag {
        None | Some("") | Some("current") => None,
        Some(id) => Some(id.to_string()),
    }
}

/// Server-side final Task-id resolution (docs/control-protocol.md §4.3): an
/// explicit `command_task` (already resolved by the CLI's
/// [`resolve_task_flag`] -- `None` here means "current"/omitted) wins; else
/// fall back to the request's ambient `labolabo_task_id`; else an error --
/// there is no third fallback (e.g. "the app's currently selected Task") by
/// design, so a script run outside any LaboLabo pane with no `--task` gets
/// a clear error instead of silently acting on whatever Task happens to be
/// focused in the UI.
pub fn resolve_target_task(
    command_task: Option<&str>,
    ambient_task_id: Option<&str>,
) -> Result<String, String> {
    command_task
        .or(ambient_task_id)
        .map(str::to_string)
        .ok_or_else(|| {
            "no task context: run this from inside a LaboLabo-spawned pane, or pass --task <id>"
                .to_string()
        })
}

// MARK: - Server-side dispatch parsing (pure, docs/control-protocol.md §5)

/// The v1 command set (docs/control-protocol.md §5). Parsed out of a
/// [`ControlRequest`] by [`ControlCommand::from_request`] -- this is the
/// "command name + params -> typed operation" dispatch step, kept pure and
/// unit-testable independent of any socket or app state; `labolabo-app`
/// matches on this enum to perform the actual Task/tab mutation.
#[derive(Debug, Clone, PartialEq)]
pub enum ControlCommand {
    TabOpen {
        task: Option<String>,
        title: Option<String>,
        command: Option<Vec<String>>,
    },
    TaskList,
    TabList {
        task: Option<String>,
        all: bool,
    },
    /// `task`/`pane` are literal ids straight from the CLI's `--task`/
    /// `--pane` flags -- **not** run through [`resolve_task_flag`]'s
    /// `current`/absent collapsing (docs/control-protocol.md §5.4: `focus`
    /// always needs a concrete target; ambient-context resolution is
    /// `tab_open`/`tab_list`-only). Exactly one is `Some` (validated below).
    Focus {
        task: Option<String>,
        pane: Option<String>,
    },
}

impl ControlCommand {
    /// The initial-scope command set's wire names (docs/control-protocol.md
    /// §5; `plans/012-task-model-and-control-cli.md` §2's "コマンド案" --
    /// the `focus --task`/`focus --pane` split form from this wave's task
    /// brief supersedes that plan doc's older positional `labolabo focus
    /// <id>` sketch, see docs/control-protocol.md §5.4's note). `task_new`
    /// is explicitly out of scope this wave (reserved, §5.5).
    pub const NAMES: [&'static str; 4] = ["tab_open", "task_list", "tab_list", "focus"];

    pub fn from_request(request: &ControlRequest) -> Result<Self, String> {
        match request.command.as_str() {
            "tab_open" => Ok(ControlCommand::TabOpen {
                task: param_string(&request.params, "task")?,
                title: param_string(&request.params, "title")?,
                command: param_string_array(&request.params, "command")?,
            }),
            "task_list" => Ok(ControlCommand::TaskList),
            "tab_list" => Ok(ControlCommand::TabList {
                task: param_string(&request.params, "task")?,
                all: param_bool(&request.params, "all")?.unwrap_or(false),
            }),
            "focus" => {
                let task = param_string(&request.params, "task")?;
                let pane = param_string(&request.params, "pane")?;
                match (&task, &pane) {
                    (Some(_), Some(_)) => {
                        Err("focus: specify exactly one of --task/--pane, not both".to_string())
                    }
                    (None, None) => Err("focus: --task or --pane is required".to_string()),
                    _ => Ok(ControlCommand::Focus { task, pane }),
                }
            }
            other => Err(format!(
                "unknown command {other:?} (known: {})",
                Self::NAMES.join(", ")
            )),
        }
    }
}

fn param_string(params: &Value, key: &str) -> Result<Option<String>, String> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(other) => Err(format!("params.{key} must be a string, got {other}")),
    }
}

fn param_bool(params: &Value, key: &str) -> Result<Option<bool>, String> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(other) => Err(format!("params.{key} must be a boolean, got {other}")),
    }
}

fn param_string_array(params: &Value, key: &str) -> Result<Option<Vec<String>>, String> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(items)) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                match item {
                    Value::String(s) => out.push(s.clone()),
                    other => {
                        return Err(format!("params.{key} items must be strings, got {other}"))
                    }
                }
            }
            if out.is_empty() {
                Ok(None)
            } else {
                Ok(Some(out))
            }
        }
        Some(other) => Err(format!(
            "params.{key} must be an array of strings, got {other}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // MARK: - ControlRequest / ControlResponse wire round-trips

    #[test]
    fn control_request_round_trips_through_json() {
        let request = ControlRequest::new(
            "tab_open",
            serde_json::json!({"task": null, "title": "reviewer", "command": ["claude"]}),
        );
        let bytes = request.to_bytes();
        let parsed = parse_request(&bytes).expect("valid request");
        assert_eq!(parsed, request);
    }

    #[test]
    fn control_request_missing_optional_fields_default_to_absent() {
        let parsed = parse_request(br#"{"command":"task_list"}"#).expect("valid request");
        assert_eq!(parsed.command, "task_list");
        assert_eq!(parsed.params, serde_json::json!({}));
        assert_eq!(parsed.labolabo_task_id, None);
        assert_eq!(parsed.labolabo_pane_id, None);
    }

    #[test]
    fn parse_request_rejects_malformed_json() {
        assert!(parse_request(b"{ not json").is_err());
    }

    #[test]
    fn with_ambient_context_reads_task_and_pane_env_vars() {
        let mut env = HashMap::new();
        env.insert("LABOLABO_TASK".to_string(), "task-1".to_string());
        env.insert("LABOLABO_PANE".to_string(), "pane-1".to_string());
        let request =
            ControlRequest::new("task_list", serde_json::json!({})).with_ambient_context(&env);
        assert_eq!(request.labolabo_task_id.as_deref(), Some("task-1"));
        assert_eq!(request.labolabo_pane_id.as_deref(), Some("pane-1"));
    }

    #[test]
    fn with_ambient_context_treats_empty_env_values_as_absent() {
        let mut env = HashMap::new();
        env.insert("LABOLABO_TASK".to_string(), "".to_string());
        let request =
            ControlRequest::new("task_list", serde_json::json!({})).with_ambient_context(&env);
        assert_eq!(request.labolabo_task_id, None);
    }

    #[test]
    fn with_ambient_context_leaves_fields_none_when_env_vars_are_unset() {
        let request = ControlRequest::new("task_list", serde_json::json!({}))
            .with_ambient_context(&HashMap::new());
        assert_eq!(request.labolabo_task_id, None);
        assert_eq!(request.labolabo_pane_id, None);
    }

    #[test]
    fn control_response_ok_round_trips() {
        let response = ControlResponse::ok(serde_json::json!({"task_id": "t1"}));
        let parsed = parse_response(&response.to_bytes()).expect("valid response");
        assert_eq!(parsed, response);
        assert!(parsed.ok);
        assert!(parsed.error.is_none());
    }

    #[test]
    fn control_response_err_round_trips() {
        let response = ControlResponse::err("boom");
        let parsed = parse_response(&response.to_bytes()).expect("valid response");
        assert!(!parsed.ok);
        assert_eq!(parsed.error.as_deref(), Some("boom"));
        assert!(parsed.result.is_none());
    }

    #[test]
    fn parse_response_rejects_malformed_json() {
        assert!(parse_response(b"not json at all").is_err());
    }

    // MARK: - control_socket_path_from_uuid

    #[test]
    fn control_socket_path_from_uuid_strips_hyphens_lowercases_truncates_and_prefixes() {
        assert_eq!(
            control_socket_path_from_uuid("ABCDEF01-2345-6789-ABCD-EF0123456789", "/tmp/labolabo"),
            "/tmp/labolabo/control-abcdef0123.sock"
        );
    }

    #[test]
    fn control_socket_path_from_uuid_trims_trailing_slash_on_base_dir() {
        assert_eq!(
            control_socket_path_from_uuid("abcdef01-2345-6789-abcd-ef0123456789", "/tmp/labolabo/"),
            "/tmp/labolabo/control-abcdef0123.sock"
        );
    }

    #[test]
    fn control_socket_path_differs_from_hooks_socket_path_for_the_same_uuid() {
        // The two socket families must never collide on the same path for
        // the same instance UUID (docs/control-protocol.md §3).
        let uuid = "abcdef01-2345-6789-abcd-ef0123456789";
        let hooks_path = crate::hook_settings::socket_path_from_uuid(uuid, "/tmp/labolabo");
        let control_path = control_socket_path_from_uuid(uuid, "/tmp/labolabo");
        assert_ne!(hooks_path, control_path);
    }

    // MARK: - control_pipe_name_from_uuid

    #[test]
    fn control_pipe_name_from_uuid_strips_hyphens_lowercases_truncates_and_prefixes() {
        assert_eq!(
            control_pipe_name_from_uuid("ABCDEF01-2345-6789-ABCD-EF0123456789"),
            r"\\.\pipe\labolabo-control-abcdef0123"
        );
    }

    #[test]
    fn control_pipe_name_differs_from_hooks_pipe_name_for_the_same_uuid() {
        // Same non-collision requirement as the unix socket paths, in the
        // `\\.\pipe\` namespace (docs/control-protocol.md §9).
        let uuid = "abcdef01-2345-6789-abcd-ef0123456789";
        let hooks_pipe = crate::hook_settings::hook_pipe_name_from_uuid(uuid);
        let control_pipe = control_pipe_name_from_uuid(uuid);
        assert_ne!(hooks_pipe, control_pipe);
    }

    // MARK: - resolve_socket_path

    #[test]
    fn resolve_socket_path_prefers_the_flag_over_env() {
        let mut env = HashMap::new();
        env.insert(
            "LABOLABO_CONTROL_SOCKET".to_string(),
            "/env/sock".to_string(),
        );
        assert_eq!(
            resolve_socket_path(Some("/flag/sock"), &env),
            Ok("/flag/sock".to_string())
        );
    }

    #[test]
    fn resolve_socket_path_falls_back_to_env_when_flag_absent() {
        let mut env = HashMap::new();
        env.insert(
            "LABOLABO_CONTROL_SOCKET".to_string(),
            "/env/sock".to_string(),
        );
        assert_eq!(resolve_socket_path(None, &env), Ok("/env/sock".to_string()));
    }

    #[test]
    fn resolve_socket_path_errors_when_neither_flag_nor_env_present() {
        assert!(resolve_socket_path(None, &HashMap::new()).is_err());
    }

    #[test]
    fn resolve_socket_path_treats_empty_flag_and_env_as_absent() {
        let mut env = HashMap::new();
        env.insert("LABOLABO_CONTROL_SOCKET".to_string(), "".to_string());
        assert!(resolve_socket_path(Some(""), &env).is_err());
    }

    // MARK: - resolve_task_flag

    #[test]
    fn resolve_task_flag_none_when_absent() {
        assert_eq!(resolve_task_flag(None), None);
    }

    #[test]
    fn resolve_task_flag_none_for_literal_current() {
        assert_eq!(resolve_task_flag(Some("current")), None);
    }

    #[test]
    fn resolve_task_flag_none_for_empty_string() {
        assert_eq!(resolve_task_flag(Some("")), None);
    }

    #[test]
    fn resolve_task_flag_passes_through_a_concrete_id() {
        assert_eq!(
            resolve_task_flag(Some("task-42")),
            Some("task-42".to_string())
        );
    }

    // MARK: - resolve_target_task

    #[test]
    fn resolve_target_task_prefers_explicit_over_ambient() {
        assert_eq!(
            resolve_target_task(Some("explicit"), Some("ambient")),
            Ok("explicit".to_string())
        );
    }

    #[test]
    fn resolve_target_task_falls_back_to_ambient() {
        assert_eq!(
            resolve_target_task(None, Some("ambient")),
            Ok("ambient".to_string())
        );
    }

    #[test]
    fn resolve_target_task_errors_when_both_absent() {
        assert!(resolve_target_task(None, None).is_err());
    }

    // MARK: - ControlCommand::from_request

    #[test]
    fn parses_tab_open_with_all_params() {
        let request = ControlRequest::new(
            "tab_open",
            serde_json::json!({"task": "t1", "title": "reviewer", "command": ["claude", "-p"]}),
        );
        assert_eq!(
            ControlCommand::from_request(&request).unwrap(),
            ControlCommand::TabOpen {
                task: Some("t1".to_string()),
                title: Some("reviewer".to_string()),
                command: Some(vec!["claude".to_string(), "-p".to_string()]),
            }
        );
    }

    #[test]
    fn parses_tab_open_with_no_params() {
        let request = ControlRequest::new("tab_open", serde_json::json!({}));
        assert_eq!(
            ControlCommand::from_request(&request).unwrap(),
            ControlCommand::TabOpen {
                task: None,
                title: None,
                command: None,
            }
        );
    }

    #[test]
    fn tab_open_empty_command_array_normalizes_to_none() {
        let request = ControlRequest::new("tab_open", serde_json::json!({"command": []}));
        assert_eq!(
            ControlCommand::from_request(&request).unwrap(),
            ControlCommand::TabOpen {
                task: None,
                title: None,
                command: None,
            }
        );
    }

    #[test]
    fn tab_open_rejects_non_string_command_items() {
        let request = ControlRequest::new("tab_open", serde_json::json!({"command": ["ok", 1]}));
        assert!(ControlCommand::from_request(&request).is_err());
    }

    #[test]
    fn parses_task_list_ignoring_params() {
        let request = ControlRequest::new("task_list", serde_json::json!({"whatever": true}));
        assert_eq!(
            ControlCommand::from_request(&request).unwrap(),
            ControlCommand::TaskList
        );
    }

    #[test]
    fn parses_tab_list_with_task_and_all() {
        let request =
            ControlRequest::new("tab_list", serde_json::json!({"task": "t2", "all": true}));
        assert_eq!(
            ControlCommand::from_request(&request).unwrap(),
            ControlCommand::TabList {
                task: Some("t2".to_string()),
                all: true,
            }
        );
    }

    #[test]
    fn tab_list_all_defaults_to_false() {
        let request = ControlRequest::new("tab_list", serde_json::json!({}));
        assert_eq!(
            ControlCommand::from_request(&request).unwrap(),
            ControlCommand::TabList {
                task: None,
                all: false,
            }
        );
    }

    #[test]
    fn parses_focus_with_task() {
        let request = ControlRequest::new("focus", serde_json::json!({"task": "t3"}));
        assert_eq!(
            ControlCommand::from_request(&request).unwrap(),
            ControlCommand::Focus {
                task: Some("t3".to_string()),
                pane: None,
            }
        );
    }

    #[test]
    fn parses_focus_with_pane() {
        let request = ControlRequest::new("focus", serde_json::json!({"pane": "p1"}));
        assert_eq!(
            ControlCommand::from_request(&request).unwrap(),
            ControlCommand::Focus {
                task: None,
                pane: Some("p1".to_string()),
            }
        );
    }

    #[test]
    fn focus_rejects_both_task_and_pane() {
        let request = ControlRequest::new("focus", serde_json::json!({"task": "t1", "pane": "p1"}));
        assert!(ControlCommand::from_request(&request).is_err());
    }

    #[test]
    fn focus_rejects_neither_task_nor_pane() {
        let request = ControlRequest::new("focus", serde_json::json!({}));
        assert!(ControlCommand::from_request(&request).is_err());
    }

    #[test]
    fn unknown_command_is_rejected_with_a_descriptive_error() {
        let request = ControlRequest::new("delete_everything", serde_json::json!({}));
        let err = ControlCommand::from_request(&request).unwrap_err();
        assert!(err.contains("delete_everything"));
        assert!(err.contains("tab_open"));
    }

    #[test]
    fn wrong_param_type_is_rejected() {
        let request = ControlRequest::new("tab_open", serde_json::json!({"task": 42}));
        assert!(ControlCommand::from_request(&request).is_err());
    }
}
