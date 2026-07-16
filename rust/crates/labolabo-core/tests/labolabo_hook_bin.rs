//! End-to-end test for the `labolabo-hook` bin target
//! (`src/bin/labolabo-hook.rs`): spawns the real compiled binary as a
//! subprocess -- feeding it stdin/argv/env exactly as Claude Code's hook
//! mechanism would (docs/hooks-protocol.md §2/§3) -- and asserts a real
//! AF_UNIX listener receives the pane-id-annotated payload.
//!
//! This is the one test in wave 4b that exercises the actual compiled
//! binary rather than calling `forward_hook`/`annotate_pane` directly
//! in-process; the three LABOLABO_PANE present/absent/non-JSON scenarios
//! are covered by `hooks.rs`'s in-process unit tests instead (see the wave
//! 4b porting brief: "3 系統（純関数テスト）+ bin の end-to-end 1 件").
//!
//! The first two tests are `#[cfg(unix)]`: they drive a real
//! `std::os::unix::net::UnixListener` end to end (and the second one runs
//! the exact `hook_command` string through `sh -c`, which only exists
//! there). The Windows counterpart at the bottom
//! (`labolabo_hook_bin_appends_pane_id_end_to_end_over_named_pipe`) drives
//! the same binary against a real `\\.\pipe\...` listener
//! (docs/hooks-protocol.md §4.2) -- no `sh -c` variant, because the
//! settings.local.json command string for Windows is the app wave's
//! concern (see docs/hooks-protocol.md §2's Windows note).
#![cfg(any(unix, windows))]

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::net::UnixListener;

#[cfg(unix)]
#[test]
fn labolabo_hook_bin_appends_pane_id_end_to_end() {
    let socket_path = std::env::temp_dir().join(format!("lb-hook-e2e-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&socket_path);
    let listener = UnixListener::bind(&socket_path).expect("bind test listener");

    // Accept the forwarder's single connection on a background thread so we
    // can spawn+feed the child process concurrently (accept() blocks).
    let (tx, rx) = mpsc::channel();
    let accept_thread = thread::spawn(move || {
        let (mut stream, _addr) = listener
            .accept()
            .expect("accept the forwarder's connection");
        let mut data = Vec::new();
        let _ = stream.read_to_end(&mut data);
        let _ = tx.send(data);
    });

    let mut child = Command::new(env!("CARGO_BIN_EXE_labolabo-hook"))
        .arg(&socket_path)
        .env("LABOLABO_PANE", "PANE-e2e")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn labolabo-hook");

    child
        .stdin
        .take()
        .expect("child stdin should be piped")
        .write_all(br#"{"hook_event_name":"SessionStart","session_id":"e2e-1"}"#)
        .expect("write hook event JSON to labolabo-hook's stdin");

    let status = child.wait().expect("wait for labolabo-hook to exit");
    assert!(
        status.success(),
        "labolabo-hook must always exit 0 (docs/hooks-protocol.md §3), got {status:?}"
    );

    let received = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("the test listener should receive exactly one connection");
    accept_thread
        .join()
        .expect("accept thread should not panic");

    let value: serde_json::Value =
        serde_json::from_slice(&received).expect("received payload should be valid JSON");
    assert_eq!(value["hook_event_name"], "SessionStart");
    assert_eq!(value["session_id"], "e2e-1");
    assert_eq!(
        value["labolabo_pane_id"], "PANE-e2e",
        "labolabo-hook should annotate labolabo_pane_id from LABOLABO_PANE"
    );

    let _ = std::fs::remove_file(&socket_path);
}

/// Regression test for the wave-5c on-device failure: the command string
/// `hook_settings::hook_command` writes into `.claude/settings.local.json`
/// is `'<binary>' --hook '<socket>'` (docs/hooks-protocol.md §2), but the
/// binary originally only read the socket path from argv[1] positionally --
/// so every real hook invocation connected to a socket literally named
/// `--hook`, failed, and exited 0, silently dropping every event (no status
/// dots, no session recording, no resume). This test runs the **exact**
/// `hook_command` output through a real `sh -c`, the same way Claude Code
/// executes hook commands, so any future drift between the written command
/// string and the binary's argv contract fails loudly here.
#[cfg(unix)]
#[test]
fn labolabo_hook_bin_accepts_the_exact_hook_command_string_via_sh() {
    let socket_path = std::env::temp_dir().join(format!("lb-hook-cmd-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&socket_path);
    let listener = UnixListener::bind(&socket_path).expect("bind test listener");

    let (tx, rx) = mpsc::channel();
    let accept_thread = thread::spawn(move || {
        let (mut stream, _addr) = listener
            .accept()
            .expect("accept the forwarder's connection");
        let mut data = Vec::new();
        let _ = stream.read_to_end(&mut data);
        let _ = tx.send(data);
    });

    let command = labolabo_core::hook_settings::hook_command(
        env!("CARGO_BIN_EXE_labolabo-hook"),
        socket_path.to_str().expect("temp socket path is UTF-8"),
    );
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(&command)
        .env("LABOLABO_PANE", "PANE-cmd")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn sh -c '<hook_command>'");

    child
        .stdin
        .take()
        .expect("child stdin should be piped")
        .write_all(br#"{"hook_event_name":"Stop","session_id":"cmd-1"}"#)
        .expect("write hook event JSON to the shell's stdin");

    let status = child.wait().expect("wait for the shell to exit");
    assert!(status.success(), "hook command must exit 0, got {status:?}");

    let received = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("the listener should receive the event forwarded via sh -c");
    accept_thread
        .join()
        .expect("accept thread should not panic");

    let value: serde_json::Value =
        serde_json::from_slice(&received).expect("received payload should be valid JSON");
    assert_eq!(value["hook_event_name"], "Stop");
    assert_eq!(value["labolabo_pane_id"], "PANE-cmd");

    let _ = std::fs::remove_file(&socket_path);
}

/// Windows counterpart of `labolabo_hook_bin_appends_pane_id_end_to_end`:
/// the same compiled binary, the same argv/stdin/env contract, but the
/// listener is a real byte-mode Named Pipe (docs/hooks-protocol.md §4.2) --
/// the `--hook` flag form is used here since that is what the injected
/// settings command runs (§2). Runs for real on the `rust (windows-latest)`
/// CI job.
#[cfg(windows)]
#[test]
fn labolabo_hook_bin_appends_pane_id_end_to_end_over_named_pipe() {
    use interprocess::os::windows::named_pipe::{pipe_mode, PipeListenerOptions, PipeMode};

    let pipe_name = format!(r"\\.\pipe\lb-hook-e2e-{}", std::process::id());
    let listener = PipeListenerOptions::new()
        .path(pipe_name.as_str())
        .mode(PipeMode::Bytes)
        .create_recv_only::<pipe_mode::Bytes>()
        .expect("bind test pipe listener");

    // Accept the forwarder's single connection on a background thread so we
    // can spawn+feed the child process concurrently (accept() blocks).
    let (tx, rx) = mpsc::channel();
    let accept_thread = thread::spawn(move || {
        let mut stream = listener
            .accept()
            .expect("accept the forwarder's connection");
        let mut data = Vec::new();
        let _ = stream.read_to_end(&mut data);
        let _ = tx.send(data);
    });

    let mut child = Command::new(env!("CARGO_BIN_EXE_labolabo-hook"))
        .args(["--hook", &pipe_name])
        .env("LABOLABO_PANE", "PANE-e2e-win")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn labolabo-hook");

    child
        .stdin
        .take()
        .expect("child stdin should be piped")
        .write_all(br#"{"hook_event_name":"SessionStart","session_id":"e2e-win-1"}"#)
        .expect("write hook event JSON to labolabo-hook's stdin");

    let status = child.wait().expect("wait for labolabo-hook to exit");
    assert!(
        status.success(),
        "labolabo-hook must always exit 0 (docs/hooks-protocol.md §3), got {status:?}"
    );

    let received = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("the test pipe listener should receive exactly one connection");
    accept_thread
        .join()
        .expect("accept thread should not panic");

    let value: serde_json::Value =
        serde_json::from_slice(&received).expect("received payload should be valid JSON");
    assert_eq!(value["hook_event_name"], "SessionStart");
    assert_eq!(value["session_id"], "e2e-win-1");
    assert_eq!(
        value["labolabo_pane_id"], "PANE-e2e-win",
        "labolabo-hook should annotate labolabo_pane_id from LABOLABO_PANE"
    );
}
