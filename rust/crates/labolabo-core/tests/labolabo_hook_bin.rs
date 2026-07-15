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
//! Whole file is `#[cfg(unix)]`: both tests drive a real `std::os::unix::
//! net::UnixListener` end to end, and `labolabo-hook`'s forwarding itself
//! is `#[cfg(unix)]` (see `src/bin/labolabo-hook.rs`) -- there is nothing
//! to exercise here on Windows yet.
#![cfg(unix)]

use std::io::{Read, Write};
use std::os::unix::net::UnixListener;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

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
