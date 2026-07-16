//! Faithful port of the *observable contract* of
//! `Sources/LaboLaboEngine/Process/ProcessRunner.swift`: executable + args +
//! cwd + env -> `{status, stdout, stderr}`, with signal death mapped to the
//! shell convention (128 + signal number) and stdout/stderr drained
//! concurrently so a full pipe buffer on one stream can never deadlock the
//! other while the process is still writing to both.
//!
//! The Swift source has two flavors: a macOS-only `posix_spawn`/kqueue
//! implementation that occupies **zero** GCD worker threads while waiting
//! (`ProcessRunner.run`, `async`), and a plain `Process`-based synchronous
//! version with an optional timeout (`ProcessRunner.runSync`). This port
//! must not depend on an async runtime (a future UI layer brings its own
//! executor -- gpui -- and this crate must stay neutral to it), so both
//! collapse into one **synchronous**, `std::process::Command`-based
//! implementation here: [`run`] mirrors `ProcessRunner.run`'s contract
//! (throws/errors only on launch failure) and [`run_with_timeout`] mirrors
//! `ProcessRunner.runSync`'s contract (adds a wall-clock timeout that kills
//! the child and reports it as "no result").
//!
//! One deliberate interface difference from `runSync`: Swift's `runSync`
//! collapses *both* launch failure and timeout to `nil` (no way to tell them
//! apart). `run_with_timeout` keeps them distinguishable --
//! `Err(io::Error)` for launch failure, `Ok(None)` for timeout -- which is
//! more idiomatic `Result`/`Option` usage and does not change process
//! *behavior* for any given input, only how failure surfaces to the Rust
//! caller (in line with this crate's stated porting principle -- see
//! `rust/README.md`). Callers that want the Swift-identical "both collapse
//! to `None`" shape can do so themselves with `.ok().flatten()`, exactly as
//! `tool_locator::locate_via_login_shell` does.
//!
//! Another small unification: the Swift `runSync` never explicitly redirects
//! stdin (it inherits the parent's, unlike `run`'s explicit `/dev/null`),
//! which looks like an oversight rather than an intentional difference --
//! its only caller (`ToolLocator`'s login-shell probe) never expects to read
//! stdin. Both `run` and `run_with_timeout` here always redirect stdin to
//! the null device, matching `run`'s (the primary, production macOS path)
//! behavior.
//!
//! # Thread contract
//!
//! `run`/`run_with_timeout` **block the calling OS thread** until the child
//! exits (or, for `run_with_timeout`, until the timeout elapses). Callers on
//! a UI thread or inside an async executor must invoke these from a
//! dedicated worker thread (e.g. `std::thread::spawn`, a blocking-task
//! pool, ...) -- exactly as the Swift call sites already do (`Task.detached`
//! / a background `DispatchQueue`). This crate does not spin up its own
//! thread pool or async runtime to hide that; it is purely a blocking
//! primitive, by design (see the module doc comment above).

use std::collections::HashMap;
use std::io::{self, Read};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

/// Result of a completed process invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Runs `executable` with `arguments`, blocking the calling thread until it
/// exits.
///
/// `directory` defaults to the caller's current directory when `None`.
/// `environment`, when `Some`, **replaces** the child's entire environment
/// (mirrors Swift's `process.environment = environment`, an assignment, not
/// a merge); when `None`, the child inherits this process's environment
/// unchanged (mirrors Swift's `ProcessInfo.processInfo.environment`
/// fallback).
///
/// Returns `Err` only if the process could not be launched at all (mirrors
/// `ProcessRunner.run`'s `throws` contract). A non-zero exit or signal death
/// is reported via `Output::status`, never as an `Err` -- signal death is
/// mapped to the shell convention of `128 + signal_number`.
pub fn run(
    executable: &Path,
    arguments: &[String],
    directory: Option<&Path>,
    environment: Option<&HashMap<String, String>>,
) -> io::Result<Output> {
    let mut command = build_command(executable, arguments, directory, environment);
    let mut child = command.spawn()?;
    let mut stdout = child.stdout.take().expect("stdout was piped");
    let mut stderr = child.stderr.take().expect("stderr was piped");
    let out_handle = thread::spawn(move || read_all(&mut stdout));
    let err_handle = thread::spawn(move || read_all(&mut stderr));

    let status = child.wait()?;
    let stdout_bytes = out_handle.join().unwrap_or_default();
    let stderr_bytes = err_handle.join().unwrap_or_default();
    Ok(Output {
        status: map_exit_status(status),
        stdout: String::from_utf8_lossy(&stdout_bytes).into_owned(),
        stderr: String::from_utf8_lossy(&stderr_bytes).into_owned(),
    })
}

/// Same contract as [`run`], plus a wall-clock `timeout`. If the child is
/// still running when `timeout` elapses, it is killed (`SIGTERM`, escalating
/// to `SIGKILL` after another second if still alive -- mirrors
/// `ProcessRunner.runSync`'s escalation) and `Ok(None)` is returned instead
/// of an `Output`. See the module doc comment for how this differs from
/// `runSync`'s "collapse everything to `nil`" shape.
pub fn run_with_timeout(
    executable: &Path,
    arguments: &[String],
    directory: Option<&Path>,
    environment: Option<&HashMap<String, String>>,
    timeout: Duration,
) -> io::Result<Option<Output>> {
    let mut command = build_command(executable, arguments, directory, environment);
    let mut child = command.spawn()?;
    let pid = child.id();
    let mut stdout = child.stdout.take().expect("stdout was piped");
    let mut stderr = child.stderr.take().expect("stderr was piped");
    let out_handle = thread::spawn(move || read_all(&mut stdout));
    let err_handle = thread::spawn(move || read_all(&mut stderr));

    // `std::process::Child` has no `wait_timeout`, so a dedicated reaper
    // thread owns the child, blocks on `wait()`, and reports back over a
    // channel that the caller can poll with a deadline.
    let (tx, rx) = mpsc::channel();
    spawn_reaper(child, tx);

    let status = match rx.recv_timeout(timeout) {
        Ok(status) => status?,
        Err(_) => {
            terminate(pid);
            if rx.recv_timeout(Duration::from_secs(1)).is_err() {
                kill_forcefully(pid);
                let _ = rx.recv(); // SIGKILL is not interruptible: this always completes.
            }
            let _ = out_handle.join();
            let _ = err_handle.join();
            return Ok(None);
        }
    };

    let stdout_bytes = out_handle.join().unwrap_or_default();
    let stderr_bytes = err_handle.join().unwrap_or_default();
    Ok(Some(Output {
        status: map_exit_status(status),
        stdout: String::from_utf8_lossy(&stdout_bytes).into_owned(),
        stderr: String::from_utf8_lossy(&stderr_bytes).into_owned(),
    }))
}

fn spawn_reaper(mut child: Child, tx: mpsc::Sender<io::Result<std::process::ExitStatus>>) {
    thread::spawn(move || {
        let status = child.wait();
        let _ = tx.send(status);
    });
}

fn build_command(
    executable: &Path,
    arguments: &[String],
    directory: Option<&Path>,
    environment: Option<&HashMap<String, String>>,
) -> Command {
    let mut command = Command::new(executable);
    command.args(arguments);
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    if let Some(dir) = directory {
        command.current_dir(dir);
    }
    if let Some(env) = environment {
        command.env_clear();
        command.envs(env);
    }
    command
}

fn read_all(reader: &mut impl Read) -> Vec<u8> {
    let mut buffer = Vec::new();
    let _ = reader.read_to_end(&mut buffer);
    buffer
}

#[cfg(unix)]
fn map_exit_status(status: std::process::ExitStatus) -> i32 {
    use std::os::unix::process::ExitStatusExt;
    match status.signal() {
        Some(signal) => 128 + signal,
        None => status.code().unwrap_or(-1),
    }
}

#[cfg(not(unix))]
fn map_exit_status(status: std::process::ExitStatus) -> i32 {
    status.code().unwrap_or(-1)
}

/// Sends `SIGTERM` (via `/bin/kill`'s default signal, same as the Swift
/// `Process.terminate()` escalation step).
#[cfg(unix)]
fn terminate(pid: u32) {
    let _ = Command::new("/bin/kill").arg(pid.to_string()).status();
}

/// Windows analog of the graceful `SIGTERM` step: `taskkill /PID <pid>`
/// (no `/F`), which posts `WM_CLOSE`/console close to the target. Console
/// children with no window (the usual case here) often reject this with
/// "can only be terminated forcefully" -- that is fine: `run_with_timeout`'s
/// escalation then applies [`kill_forcefully`] one second later, the same
/// two-step shape as the unix arm.
#[cfg(windows)]
fn terminate(pid: u32) {
    let _ = Command::new("taskkill")
        .args(["/PID", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(not(any(unix, windows)))]
fn terminate(_pid: u32) {}

/// Sends `SIGKILL` (`kill -9`), matching `Process.terminate()` -> `SIGKILL`
/// escalation in `ProcessRunner.runSync`.
#[cfg(unix)]
fn kill_forcefully(pid: u32) {
    let _ = Command::new("/bin/kill")
        .args(["-9", &pid.to_string()])
        .status();
}

/// Windows analog of `SIGKILL`: `taskkill /F /PID <pid>`
/// (`TerminateProcess`, not refusable by the target).
#[cfg(windows)]
fn kill_forcefully(pid: u32) {
    let _ = Command::new("taskkill")
        .args(["/F", "/PID", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(not(any(unix, windows)))]
fn kill_forcefully(_pid: u32) {}

#[cfg(test)]
mod tests {
    use super::*;

    // Ported from Tests/LaboLaboEngineTests/{ProcessRunnerAsyncTests,ProcessRunnerTests}.swift.
    // The Swift source's two flavors (async `run` / sync-with-timeout
    // `runSync`) collapse into `run` / `run_with_timeout` here -- see the
    // module doc comment.
    //
    // Most tests below spawn `/bin/echo` or `/bin/sh`, which don't exist on
    // Windows -- those are gated `#[cfg(unix)]`, with `cmd /C` counterparts
    // in `windows_tests` below (run for real on the `rust (windows-latest)`
    // CI job). The `missing_executable_*` tests only assert that spawning a
    // nonexistent path errors, which holds on every platform, so they stay
    // cross-platform.

    #[cfg(unix)]
    fn echo() -> &'static Path {
        Path::new("/bin/echo")
    }
    #[cfg(unix)]
    fn sh() -> &'static Path {
        Path::new("/bin/sh")
    }

    #[cfg(unix)]
    #[test]
    fn echo_captures_stdout_and_zero_status() {
        let out = run(echo(), &["hello".to_string()], None, None).unwrap();
        assert_eq!(out.status, 0);
        assert_eq!(out.stdout, "hello\n");
        assert!(out.stderr.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn non_zero_exit_and_stderr_are_propagated() {
        let out = run(
            sh(),
            &["-c".to_string(), "echo bad 1>&2; exit 3".to_string()],
            None,
            None,
        )
        .unwrap();
        assert_eq!(out.status, 3);
        assert!(out.stdout.is_empty());
        assert_eq!(out.stderr.trim(), "bad");
    }

    #[cfg(unix)]
    #[test]
    fn empty_output_completes() {
        let out = run(sh(), &["-c".to_string(), "exit 0".to_string()], None, None).unwrap();
        assert_eq!(out.status, 0);
        assert!(out.stdout.is_empty());
        assert!(out.stderr.is_empty());
    }

    /// Pushes well past a pipe buffer's (~64KB) capacity on both streams at
    /// once, to prove concurrent draining prevents a deadlock.
    #[cfg(unix)]
    #[test]
    fn large_output_on_both_pipes() {
        let size = 300_000;
        let out = run(
            sh(),
            &[
                "-c".to_string(),
                format!("yes a | head -c {size}; yes b | head -c {size} 1>&2"),
            ],
            None,
            None,
        )
        .unwrap();
        assert_eq!(out.status, 0);
        assert_eq!(out.stdout.len(), size);
        assert_eq!(out.stderr.len(), size);
    }

    #[cfg(unix)]
    #[test]
    fn signal_death_maps_to_shell_convention() {
        // SIGKILL against self (undeferrable) -> 128 + 9 = 137.
        let out = run(
            sh(),
            &["-c".to_string(), "kill -9 $$".to_string()],
            None,
            None,
        )
        .unwrap();
        assert_eq!(out.status, 137);
    }

    #[test]
    fn missing_executable_errors() {
        let missing = Path::new("/nonexistent/definitely/not/here-labolabo-process-test");
        assert!(run(missing, &[], None, None).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn runs_in_specified_directory() {
        let base = std::env::temp_dir().join(format!(
            "labolabo-process-cwd-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&base).unwrap();

        let out = run(
            sh(),
            &["-c".to_string(), "pwd".to_string()],
            Some(&base),
            None,
        )
        .unwrap();
        assert_eq!(out.status, 0);
        let reported = std::fs::canonicalize(out.stdout.trim()).unwrap();
        let expected = std::fs::canonicalize(&base).unwrap();
        assert_eq!(reported, expected);

        let _ = std::fs::remove_dir_all(&base);
    }

    #[cfg(unix)]
    #[test]
    fn environment_is_passed_through() {
        let mut env = HashMap::new();
        env.insert("LABO_TEST_VAR".to_string(), "wired".to_string());
        let out = run(
            sh(),
            &["-c".to_string(), "printf %s \"$LABO_TEST_VAR\"".to_string()],
            None,
            Some(&env),
        )
        .unwrap();
        assert_eq!(out.status, 0);
        assert_eq!(out.stdout, "wired");
    }

    #[cfg(unix)]
    #[test]
    fn timeout_returns_none_when_command_outlives_deadline() {
        let out = run_with_timeout(
            sh(),
            &["-c".to_string(), "sleep 5".to_string()],
            None,
            None,
            Duration::from_secs(1),
        )
        .unwrap();
        assert!(out.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn fast_command_completes_within_timeout() {
        let out = run_with_timeout(
            sh(),
            &["-c".to_string(), "exit 0".to_string()],
            None,
            None,
            Duration::from_secs(5),
        )
        .unwrap()
        .unwrap();
        assert_eq!(out.status, 0);
    }

    #[test]
    fn missing_executable_run_with_timeout_errors() {
        let missing = Path::new("/nonexistent/definitely/not/here-labolabo-process-test-2");
        assert!(run_with_timeout(missing, &[], None, None, Duration::from_secs(1)).is_err());
    }
}

// `cmd /C` counterparts of the unix-gated tests above (the contract under
// test -- status/stdout/stderr capture, env/cwd handling, timeout kill --
// is platform-independent `std::process` code; only the spawned commands
// differ). The signal-death and both-pipes-flooded cases stay unix-only:
// Windows has no signal-exit status to map, and the concurrent pipe
// draining they prove is the same code path already exercised there.
#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    /// `%ComSpec%` (practically always `C:\Windows\system32\cmd.exe`), with
    /// the well-known path as fallback.
    fn cmd() -> PathBuf {
        std::env::var_os("ComSpec")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows\System32\cmd.exe"))
    }

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn echo_captures_stdout_and_zero_status() {
        let out = run(&cmd(), &args(&["/C", "echo hello"]), None, None).unwrap();
        assert_eq!(out.status, 0);
        assert_eq!(out.stdout, "hello\r\n");
        assert!(out.stderr.is_empty());
    }

    #[test]
    fn non_zero_exit_and_stderr_are_propagated() {
        let out = run(&cmd(), &args(&["/C", "echo bad 1>&2 & exit 3"]), None, None).unwrap();
        assert_eq!(out.status, 3);
        assert!(out.stdout.is_empty());
        assert_eq!(out.stderr.trim(), "bad");
    }

    #[test]
    fn empty_output_completes() {
        let out = run(&cmd(), &args(&["/C", "exit 0"]), None, None).unwrap();
        assert_eq!(out.status, 0);
        assert!(out.stdout.is_empty());
        assert!(out.stderr.is_empty());
    }

    #[test]
    fn runs_in_specified_directory() {
        let base = std::env::temp_dir().join(format!(
            "labolabo-process-cwd-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&base).unwrap();

        // `cd` with no argument prints the current directory.
        let out = run(&cmd(), &args(&["/C", "cd"]), Some(&base), None).unwrap();
        assert_eq!(out.status, 0);
        let reported = std::fs::canonicalize(Path::new(out.stdout.trim())).unwrap();
        let expected = std::fs::canonicalize(&base).unwrap();
        assert_eq!(reported, expected);

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn environment_is_passed_through() {
        // `environment: Some(..)` replaces the child's whole environment;
        // seed it with the parent's (cmd.exe needs SystemRoot etc. to run)
        // plus the probe variable -- exercising the same env-replacement
        // path the unix test does.
        let mut env: HashMap<String, String> = std::env::vars().collect();
        env.insert("LABO_TEST_VAR".to_string(), "wired".to_string());
        let out = run(
            &cmd(),
            &args(&["/C", "echo %LABO_TEST_VAR%"]),
            None,
            Some(&env),
        )
        .unwrap();
        assert_eq!(out.status, 0);
        assert_eq!(out.stdout.trim(), "wired");
    }

    #[test]
    fn timeout_returns_none_when_command_outlives_deadline() {
        // `ping -n 6 127.0.0.1` ~= `sleep 5` (no `timeout /t` here: that
        // command needs an interactive console and fails under a redirected
        // stdin). The taskkill escalation in `run_with_timeout` reaps it.
        let out = run_with_timeout(
            &cmd(),
            &args(&["/C", "ping -n 6 127.0.0.1 > NUL"]),
            None,
            None,
            Duration::from_secs(1),
        )
        .unwrap();
        assert!(out.is_none());
    }

    #[test]
    fn fast_command_completes_within_timeout() {
        let out = run_with_timeout(
            &cmd(),
            &args(&["/C", "exit 0"]),
            None,
            None,
            Duration::from_secs(5),
        )
        .unwrap()
        .unwrap();
        assert_eq!(out.status, 0);
    }
}
