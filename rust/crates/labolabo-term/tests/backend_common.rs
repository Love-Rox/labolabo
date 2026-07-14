//! Backend-agnostic integration tests.
//!
//! Every test here names only `labolabo_term::Terminal` (the active backend),
//! so this exact file runs unchanged against **both** backends:
//!
//! - default:            `cargo test -p labolabo-term`
//! - ghostty (opt-in):   `cargo test -p labolabo-term \
//!                          --no-default-features --features backend-ghostty-vt`
//!
//! They are headless (no window): a PTY child writes to the grid, and we
//! assert on the extracted `GridSnapshot`. That is the whole point of the
//! plain-data snapshot design -- the render surface is testable without a UI.

use std::time::Duration;

use labolabo_term::Terminal;

const TIMEOUT: Duration = Duration::from_secs(5);

/// `echo hello` -> the text lands in a snapshot.
#[test]
fn echo_hello_appears_in_snapshot() {
    let term =
        Terminal::spawn_with_command(80, 24, Some("echo hello && sleep 0.2"), &[]).expect("spawn");
    let snap = term.wait_for(TIMEOUT, |g| g.contains_text("hello"));
    assert!(
        snap.is_some(),
        "expected 'hello' in grid, got:\n{}",
        term.snapshot().to_text()
    );
}

/// Env injection reaches the child: `$LABOLABO_PANE` echoed back shows up.
/// This is the mechanism LaboLabo's hooks protocol relies on to tag a pane.
#[test]
fn env_injection_reaches_child() {
    let env = vec![("LABOLABO_PANE".to_string(), "pane-42".to_string())];
    let term = Terminal::spawn_with_command(
        80,
        24,
        Some("printf '%s' \"$LABOLABO_PANE\"; sleep 0.2"),
        &env,
    )
    .expect("spawn");
    let snap = term.wait_for(TIMEOUT, |g| g.contains_text("pane-42"));
    assert!(
        snap.is_some(),
        "expected injected env 'pane-42' in grid, got:\n{}",
        term.snapshot().to_text()
    );
}

/// Resizing updates the reported grid dimensions.
#[test]
fn resize_changes_grid_dimensions() {
    // A live child so the session stays up across the resize (it self-exits,
    // so no orphan is left behind).
    let term = Terminal::spawn_with_command(80, 24, Some("sleep 2"), &[]).expect("spawn");

    // The initial (pre-output) snapshot reflects the spawn dimensions.
    let initial = term.snapshot();
    assert_eq!(initial.cols, 80, "initial cols");
    assert_eq!(initial.rows, 24, "initial rows");

    term.resize(100, 40);

    let resized = term.wait_for(TIMEOUT, |g| g.cols == 100 && g.rows == 40);
    let latest = term.snapshot();
    assert!(
        resized.is_some(),
        "expected 100x40 after resize, got {}x{}",
        latest.cols,
        latest.rows
    );
    // The cell buffer is re-sized to match the reported dimensions.
    assert_eq!(latest.cells.len(), 100 * 40, "cell count matches new grid");
}

/// A never-producing child still yields the blank spawn-size snapshot up front
/// and an `Exit` event when it ends -- exercising the event channel directly.
#[test]
fn exit_event_fires_when_child_ends() {
    let term = Terminal::spawn_with_command(40, 10, Some("true"), &[]).expect("spawn");
    assert!(
        wait_for_exit(&term, TIMEOUT),
        "expected an Exit event after the child finished"
    );
}

/// `shutdown` terminates a child that would otherwise outlive the test by
/// far (`sleep 30` vs. the 5s event timeout), and the session then follows
/// the normal exit path: a final `Exit` event fires. If `shutdown` didn't
/// actually kill the child, no EOF would reach the reader and this test
/// would time out waiting for `Exit`.
#[test]
fn shutdown_kills_child_and_fires_exit() {
    let term = Terminal::spawn_with_command(40, 10, Some("sleep 30"), &[]).expect("spawn");
    term.shutdown();
    assert!(
        wait_for_exit(&term, TIMEOUT),
        "expected an Exit event shortly after shutdown()"
    );
}

/// `shutdown` is idempotent: calling it repeatedly, and again after the
/// child has already exited on its own, must not panic or misbehave.
#[test]
fn shutdown_is_idempotent_and_safe_after_natural_exit() {
    let term = Terminal::spawn_with_command(40, 10, Some("true"), &[]).expect("spawn");
    assert!(
        wait_for_exit(&term, TIMEOUT),
        "expected the child to exit on its own first"
    );
    // Child is gone (and by now reaped by the worker); these must be no-ops.
    term.shutdown();
    term.shutdown();
}

/// Drain events until `Exit` (Wakeups may precede it), bounded by `timeout`.
fn wait_for_exit(term: &Terminal, timeout: Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        match term.recv_event(Duration::from_millis(200)) {
            Some(labolabo_term::TermEvent::Exit) => return true,
            _ => continue,
        }
    }
    false
}
