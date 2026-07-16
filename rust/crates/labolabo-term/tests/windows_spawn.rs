//! Windows-only PTY spawn coverage -- the counterpart of `backend_common.rs`
//! (`#![cfg(unix)]`, POSIX shell syntax throughout) for the Windows shell
//! resolution added in `src/session.rs`'s private `windows` module (see its
//! doc comment: `%ComSpec% /C <cmd>` for a one-shot command, `pwsh.exe` ->
//! `powershell.exe` -> `%ComSpec%` for the interactive default shell).
//!
//! Deliberately narrow -- two smoke tests, not a `backend_common.rs`-sized
//! suite -- because every command string here has to be valid `cmd.exe`
//! syntax (no `&&`, `sleep`, `printf`, ...), and this crate's own CI
//! (`rust` job, `windows-latest`) is the only place these actually run; there
//! is no Windows machine in this project's development loop to iterate
//! against interactively (same "compiles + headless-tested only" caveat as
//! `labolabo-app/README.md`'s "Linux"/"Windows" sections).
#![cfg(windows)]

use std::time::Duration;

use labolabo_term::Terminal;

const TIMEOUT: Duration = Duration::from_secs(5);

/// `Some(cmd)` wraps `cmd` as `%ComSpec% /C <cmd>` (see `session.rs`) --
/// `echo hello` should reach the child and land in a snapshot, proving the
/// one-shot command path actually execs something real rather than failing
/// to spawn a nonexistent `/bin/sh`.
#[test]
fn echo_hello_via_comspec_appears_in_snapshot() {
    let term = Terminal::spawn_with_command(80, 24, Some("echo hello"), &[]).expect("spawn");
    let snap = term.wait_for(TIMEOUT, |g| g.contains_text("hello"));
    assert!(
        snap.is_some(),
        "expected 'hello' in grid, got:\n{}",
        term.snapshot().to_text()
    );
}

/// Env injection reaches the child through `cmd.exe /C` the same way it does
/// through unix's `/bin/sh -c` (`backend_common.rs`'s `env_injection_reaches_child`)
/// -- the mechanism LaboLabo's hooks protocol relies on to tag a pane.
#[test]
fn env_injection_reaches_child_via_comspec() {
    let env = vec![("LABOLABO_PANE".to_string(), "pane-42".to_string())];
    let term =
        Terminal::spawn_with_command(80, 24, Some("echo %LABOLABO_PANE%"), &env).expect("spawn");
    let snap = term.wait_for(TIMEOUT, |g| g.contains_text("pane-42"));
    assert!(
        snap.is_some(),
        "expected injected env 'pane-42' in grid, got:\n{}",
        term.snapshot().to_text()
    );
}

/// `None` (the interactive default shell) spawns *something* that stays up
/// long enough to answer a resize -- not asserting on which shell was
/// chosen (pwsh/PowerShell/cmd.exe are all valid depending on what's on the
/// runner's `PATH`), just that the resolution never fails to produce a
/// runnable child.
#[test]
fn default_shell_spawns_and_survives_a_resize() {
    let term = Terminal::spawn_with_command(80, 24, None, &[]).expect("spawn");
    term.resize(100, 40);
    let resized = term.wait_for(TIMEOUT, |g| g.cols == 100 && g.rows == 40);
    assert!(
        resized.is_some(),
        "expected the default shell to still be alive after a resize"
    );
    term.shutdown();
}
