//! End-to-end test for the `labolabo` bin target
//! (`src/bin/labolabo.rs`): spawns the real compiled binary as a
//! subprocess -- feeding it argv/env exactly as a human or an agent would
//! (docs/control-protocol.md §4/§8) -- against a real
//! `labolabo_core::control::ControlServer` bound to a real AF_UNIX socket,
//! with a stub handler standing in for `LaboLaboApp::dispatch_control`
//! (parses the request with the real `control_protocol` pure logic and
//! replies with a canned `ControlResponse`, but performs no gpui/Task
//! mutation).
//!
//! Mirrors `labolabo-core`'s `tests/labolabo_hook_bin.rs`: that is the wave
//! 4b/5c precedent for "run the actual compiled binary as a subprocess,
//! assert on the actual bytes a real listener receives/a real client
//! prints" rather than only unit-testing the pure logic pieces in-process.
//! The task brief for this wave calls this out explicitly: "CLI が実際に発行
//! するバイト列と、サーバが実際に読む契約を、実行経路そのままで突き合わせる
//! テストを必ず入れる（モック同士で握手させない）".

use std::collections::HashMap;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;

use labolabo_core::control::ControlServer;
use labolabo_core::{parse_request, ControlCommand, ControlRequest, ControlResponse};

fn temp_socket_path(label: &str) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir()
        .join(format!("lb-cli-{label}-{}-{n}.sock", std::process::id()))
        .to_string_lossy()
        .into_owned()
}

/// Starts a real `ControlServer` on a scratch socket whose handler parses
/// every request with the real `control_protocol` logic, forwards the
/// parsed `ControlRequest` to the test over `report_tx` (so assertions can
/// inspect exactly what the CLI put on the wire -- ambient context
/// included), and replies with whatever `respond` computes for that
/// request. Returns the socket path and the server (kept alive for the
/// caller to `stop()` when done).
fn start_stub_server(
    label: &str,
    respond: impl Fn(&ControlRequest, &ControlCommand) -> ControlResponse + Send + Sync + 'static,
    report_tx: mpsc::Sender<ControlRequest>,
) -> (String, ControlServer) {
    let socket_path = temp_socket_path(label);
    let mut server = ControlServer::new(socket_path.clone());
    server.set_handler(Box::new(move |bytes| {
        let request = match parse_request(&bytes) {
            Ok(request) => request,
            Err(err) => return ControlResponse::err(err).to_bytes(),
        };
        let command = match ControlCommand::from_request(&request) {
            Ok(command) => command,
            Err(err) => return ControlResponse::err(err).to_bytes(),
        };
        let response = respond(&request, &command);
        let _ = report_tx.send(request);
        response.to_bytes()
    }));
    server.start();
    (socket_path, server)
}

struct CliOutput {
    status: i32,
    stdout: String,
    stderr: String,
}

fn run_cli(socket_path: &str, extra_env: &HashMap<String, String>, args: &[&str]) -> CliOutput {
    let mut command = Command::new(env!("CARGO_BIN_EXE_labolabo"));
    command
        .args(args)
        .env("LABOLABO_CONTROL_SOCKET", socket_path)
        .env_remove("LABOLABO_TASK")
        .env_remove("LABOLABO_PANE");
    for (key, value) in extra_env {
        command.env(key, value);
    }
    let output = command.output().expect("spawn labolabo CLI");
    CliOutput {
        status: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

#[test]
fn tab_open_end_to_end_reports_the_new_pane_id() {
    let (tx, rx) = mpsc::channel();
    let (socket_path, mut server) = start_stub_server(
        "tab-open",
        |_request, _command| {
            ControlResponse::ok(serde_json::json!({"task_id": "task-1", "pane_id": "pane-1"}))
        },
        tx,
    );

    let output = run_cli(
        &socket_path,
        &HashMap::new(),
        &[
            "tab", "open", "--task", "task-1", "--title", "reviewer", "--", "claude", "-p",
        ],
    );

    assert_eq!(output.status, 0, "stderr: {}", output.stderr);
    assert_eq!(output.stdout.trim(), "opened pane pane-1 in task task-1");

    let received = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
    assert_eq!(received.command, "tab_open");
    assert_eq!(received.params["task"], "task-1");
    assert_eq!(received.params["title"], "reviewer");
    assert_eq!(
        received.params["command"],
        serde_json::json!(["claude", "-p"])
    );

    server.stop();
}

#[test]
fn tab_open_with_no_task_flag_uses_ambient_labolabo_task_env() {
    // The flagship use case (plans/012 §2): a Claude session running inside
    // a LaboLabo pane calls `labolabo tab open` with no `--task` at all --
    // the CLI must attach LABOLABO_TASK as the request's ambient context
    // (docs/control-protocol.md §4.2), and `params.task` itself must stay
    // null (not resolved client-side into that same id) so the server's
    // resolve_target_task fallback is what's actually being exercised.
    let (tx, rx) = mpsc::channel();
    let (socket_path, mut server) = start_stub_server(
        "ambient-task",
        |_request, _command| {
            ControlResponse::ok(serde_json::json!({"task_id": "ambient-task-1", "pane_id": "p1"}))
        },
        tx,
    );

    let mut env = HashMap::new();
    env.insert("LABOLABO_TASK".to_string(), "ambient-task-1".to_string());
    let output = run_cli(&socket_path, &env, &["tab", "open"]);

    assert_eq!(output.status, 0, "stderr: {}", output.stderr);

    let received = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
    assert_eq!(received.params["task"], serde_json::Value::Null);
    assert_eq!(received.labolabo_task_id.as_deref(), Some("ambient-task-1"));

    server.stop();
}

#[test]
fn task_list_json_flag_prints_the_raw_response() {
    let (tx, rx) = mpsc::channel();
    let (socket_path, mut server) = start_stub_server(
        "task-list-json",
        |_request, _command| ControlResponse::ok(serde_json::json!({"tasks": [{"id": "t1"}]})),
        tx,
    );

    let output = run_cli(&socket_path, &HashMap::new(), &["task", "list", "--json"]);

    assert_eq!(output.status, 0, "stderr: {}", output.stderr);
    let printed: serde_json::Value = serde_json::from_str(output.stdout.trim()).unwrap();
    assert_eq!(printed["ok"], true);
    assert_eq!(printed["result"]["tasks"][0]["id"], "t1");

    let _ = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
    server.stop();
}

#[test]
fn app_side_error_response_yields_exit_code_one() {
    let (tx, rx) = mpsc::channel();
    let (socket_path, mut server) = start_stub_server(
        "app-error",
        |_request, _command| ControlResponse::err("unknown task id: nope"),
        tx,
    );

    let output = run_cli(&socket_path, &HashMap::new(), &["focus", "--task", "nope"]);

    assert_eq!(output.status, 1);
    assert!(output.stderr.contains("unknown task id: nope"));

    let _ = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
    server.stop();
}

#[test]
fn no_socket_configured_yields_exit_code_two_without_contacting_any_server() {
    let mut command = Command::new(env!("CARGO_BIN_EXE_labolabo"));
    command
        .args(["task", "list"])
        .env_remove("LABOLABO_CONTROL_SOCKET")
        .env_remove("LABOLABO_TASK")
        .env_remove("LABOLABO_PANE");
    let output = command.output().expect("spawn labolabo CLI");

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("no control socket"));
}

#[test]
fn connect_failure_yields_exit_code_two() {
    let socket_path = temp_socket_path("nonexistent");
    // No server bound at this path.
    let output = run_cli(&socket_path, &HashMap::new(), &["task", "list"]);
    assert_eq!(output.status, 2);
}

#[test]
fn usage_error_yields_exit_code_two() {
    let mut command = Command::new(env!("CARGO_BIN_EXE_labolabo"));
    command
        .args(["bogus", "subcommand"])
        .env("LABOLABO_CONTROL_SOCKET", "/tmp/does-not-matter.sock");
    let output = command.output().expect("spawn labolabo CLI");
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("unknown subcommand"));
}

#[test]
fn focus_pane_end_to_end_round_trips_literal_pane_id() {
    let (tx, rx) = mpsc::channel();
    let (socket_path, mut server) = start_stub_server(
        "focus-pane",
        |_request, _command| {
            ControlResponse::ok(serde_json::json!({"task_id": "t1", "pane_id": "p1"}))
        },
        tx,
    );

    let output = run_cli(&socket_path, &HashMap::new(), &["focus", "--pane", "p1"]);

    assert_eq!(output.status, 0, "stderr: {}", output.stderr);
    assert_eq!(output.stdout.trim(), "focused pane p1 in task t1");

    let received = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
    assert_eq!(received.command, "focus");
    assert_eq!(received.params["pane"], "p1");
    assert!(received.params.get("task").is_none() || received.params["task"].is_null());

    server.stop();
}
