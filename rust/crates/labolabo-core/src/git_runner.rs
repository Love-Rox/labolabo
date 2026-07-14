//! Faithful port of `Sources/LaboLaboEngine/Git/GitRunner.swift`.
//!
//! Runs the system `git` binary and returns its stdout, capping total
//! concurrent invocations and resolving `git`'s absolute path through
//! [`ToolLocating`] (falling back to `/usr/bin/env git` when resolution
//! fails, so PATH search is still delegated to `env` rather than failing
//! outright).
//!
//! # Thread contract
//!
//! Like `process::run`, [`run`]/[`run_with_locator`] **block the calling
//! thread** for the duration of the `git` invocation, plus however long it
//! waits for a free concurrency-gate slot. Callers must invoke this from a
//! worker thread, never directly from a UI/async-executor thread -- see
//! `process`'s module doc comment for the full rationale (no async runtime
//! dependency by design).

use crate::process;
use crate::tool_locator::{ToolLocating, ToolLocator};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Condvar, Mutex};

/// Thrown when a `git` invocation exits non-zero.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitCommandError {
    pub arguments: Vec<String>,
    pub exit_code: i32,
    pub stderr: String,
}

impl fmt::Display for GitCommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "git {} failed (exit {}): {}",
            self.arguments.join(" "),
            self.exit_code,
            self.stderr.trim()
        )
    }
}

impl std::error::Error for GitCommandError {}

/// Errors from [`run`]/[`run_with_locator`]: either the process could not be
/// launched at all, or it launched and exited non-zero.
///
/// The Swift source has a single throwing surface (either
/// `ProcessRunner.run`'s launch error propagating through, or a synthesized
/// `GitCommandError`) -- both are just `Error` there, indistinguishable by
/// type. Splitting them into an explicit two-variant enum here is an
/// idiomatic-Rust interface choice, not a behavior change: callers that only
/// care about success/failure can match either arm the same way, and callers
/// that want the stderr/exit-code detail can match `Command` specifically.
#[derive(Debug)]
pub enum GitRunError {
    /// `git` could not be launched at all (binary missing, exec permission
    /// denied, ...).
    Spawn(std::io::Error),
    /// `git` launched and exited non-zero.
    Command(GitCommandError),
}

impl fmt::Display for GitRunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GitRunError::Spawn(e) => write!(f, "failed to launch git: {e}"),
            GitRunError::Command(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for GitRunError {}

/// Blocking counting semaphore capping concurrent `git` invocations at 16 --
/// same limit as the Swift `ConcurrencyGate`. Unlike the Swift version (an
/// `actor` that suspends a `Task` without blocking a thread), this is a
/// **blocking** semaphore built on `Condvar`: acceptable here because
/// `process::run`/`run_with_timeout` already block their calling thread for
/// the invocation's duration (see their thread contract), so there is no
/// async executor thread to protect from blocking in the first place -- the
/// caller is already expected to be on a dedicated worker thread before it
/// ever reaches `acquire`.
struct ConcurrencyGate {
    limit: usize,
    running: Mutex<usize>,
    available: Condvar,
}

impl ConcurrencyGate {
    const fn new(limit: usize) -> Self {
        Self {
            limit,
            running: Mutex::new(0),
            available: Condvar::new(),
        }
    }

    fn acquire(&self) {
        let mut running = self.running.lock().unwrap();
        while *running >= self.limit {
            running = self.available.wait(running).unwrap();
        }
        *running += 1;
    }

    fn release(&self) {
        let mut running = self.running.lock().unwrap();
        *running -= 1;
        // FIFO isn't guaranteed by `Condvar::notify_one` the way the Swift
        // `ConcurrencyGate`'s explicit waiter queue guarantees it, but
        // fairness order among queued `git` invocations isn't part of the
        // observable contract (only the concurrency cap is).
        self.available.notify_one();
    }
}

static GATE: ConcurrencyGate = ConcurrencyGate::new(16);

/// Runs `git` with the real [`ToolLocator`]. See [`run_with_locator`] for
/// the full contract, including the locator-injection seam tests use.
pub fn run(arguments: &[String], directory: &Path) -> Result<String, GitRunError> {
    run_with_locator(arguments, directory, &ToolLocator)
}

/// `git` invocation with an injectable locator -- mirrors the Swift
/// `locator: ToolLocating.Type = ToolLocator.self` parameter that
/// `GitRunnerTests`' fake-locator tests use to verify the resolved
/// executable is actually the one invoked (and that resolution failure
/// falls back to `/usr/bin/env git`).
pub fn run_with_locator(
    arguments: &[String],
    directory: &Path,
    locator: &dyn ToolLocating,
) -> Result<String, GitRunError> {
    GATE.acquire();
    let (executable, spawn_arguments) = resolve_git(locator, arguments);
    let result = process::run(&executable, &spawn_arguments, Some(directory), None);
    GATE.release();

    let output = result.map_err(GitRunError::Spawn)?;
    if output.status != 0 {
        return Err(GitRunError::Command(GitCommandError {
            arguments: arguments.to_vec(),
            exit_code: output.status,
            stderr: output.stderr,
        }));
    }
    Ok(output.stdout)
}

/// Builds the `git` invocation: the locator's resolved absolute path if it
/// found one, otherwise a `/usr/bin/env git` fallback so PATH search is
/// still delegated to `env` (preserves pre-`ToolLocating` behavior when
/// resolution fails, e.g. `git` not in any fixed candidate/PATH/login
/// shell).
fn resolve_git(locator: &dyn ToolLocating, arguments: &[String]) -> (PathBuf, Vec<String>) {
    if let Some(git) = locator.locate("git") {
        (git, arguments.to_vec())
    } else {
        let mut args = vec!["git".to_string()];
        args.extend(arguments.iter().cloned());
        (PathBuf::from("/usr/bin/env"), args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    // Ported from Tests/LaboLaboEngineTests/GitRunnerTests.swift, including
    // the fake-locator injection tests (added in Swift PR #90).

    /// Unique scratch directory per test, mirroring the Swift tests'
    /// `NSTemporaryDirectory().appendingPathComponent("labolabo-...-\(UUID())")`.
    fn scratch_dir(prefix: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("{prefix}-{}-{nanos}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// `git` を偽の実行ファイルへすり替えるロケータ。`GitRunner` がロケータ解決パスを
    /// 実際に使っていること（ハードコードされた `/usr/bin/env git` に戻っていないこと）
    /// を確認するためのテスト用フェイク。
    struct FakeGitLocator;
    impl ToolLocating for FakeGitLocator {
        fn locate(&self, name: &str) -> Option<PathBuf> {
            if name == "git" {
                Some(PathBuf::from("/bin/echo"))
            } else {
                None
            }
        }
    }

    /// 何を渡しても解決に失敗するロケータ。「PATH に無い」場合のフォールバック
    /// （`/usr/bin/env git`）が維持されていることを確認する。
    struct NeverResolvingLocator;
    impl ToolLocating for NeverResolvingLocator {
        fn locate(&self, _name: &str) -> Option<PathBuf> {
            None
        }
    }

    #[test]
    fn run_returns_stdout() {
        let dir = scratch_dir("labolabo-gitrunner");
        let out = run(&["--version".to_string()], &dir).unwrap();
        assert!(out.starts_with("git version"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ロケータが解決した絶対パスを実際に起動していることを確認する。
    /// `git` を `/bin/echo` にすり替えると、渡した引数がそのまま echo される
    /// （`/usr/bin/env git` へフォールバックしていれば git のエラーになるはず）。
    #[test]
    fn run_uses_locator_resolved_executable() {
        let dir = scratch_dir("labolabo-gitrunner");
        let out = run_with_locator(&["hello-seam".to_string()], &dir, &FakeGitLocator).unwrap();
        assert_eq!(out, "hello-seam\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ロケータが解決できない場合は、これまで通り `/usr/bin/env git` にフォールバック
    /// し、通常の git 呼び出しと同じ挙動を保つ。
    #[test]
    fn run_falls_back_to_env_git_when_locator_fails() {
        let dir = scratch_dir("labolabo-gitrunner");
        let out =
            run_with_locator(&["--version".to_string()], &dir, &NeverResolvingLocator).unwrap();
        assert!(out.starts_with("git version"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn non_zero_exit_returns_git_command_error_with_stderr() {
        let dir = scratch_dir("labolabo-gitrunner");
        // 非 repo ディレクトリでの rev-parse は非ゼロ exit + stderr を返す。
        let err = run(
            &["rev-parse".to_string(), "--show-toplevel".to_string()],
            &dir,
        )
        .unwrap_err();
        match err {
            GitRunError::Command(e) => {
                assert_ne!(e.exit_code, 0);
                assert!(!e.stderr.is_empty());
            }
            other => panic!("unexpected error variant: {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 大量の並行 git 呼び出しがすべて完走すること。
    ///
    /// 旧 Swift 実装（グローバルキューで waitUntilExit + group.wait）は呼び出しごとに
    /// GCD ワーカーを塞ぐため、並行数がプール上限に達すると読み取り側が永遠にスケジュール
    /// されずデッドロックした。この Rust 版は呼び出しごとに専用スレッドを使う（`run`
    /// 自体が呼び出し元スレッドをブロックする設計のため）ので、同種の枯渇は起こり得ないが、
    /// ゲートの並行数制御そのものは同じ契約として検証しておく。
    #[test]
    fn many_concurrent_invocations_all_complete() {
        let dir = scratch_dir("labolabo-gitrunner");
        let handles: Vec<_> = (0..100)
            .map(|_| {
                let dir = dir.clone();
                std::thread::spawn(move || run(&["--version".to_string()], &dir).is_ok())
            })
            .collect();
        let results: Vec<bool> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(results.len(), 100);
        assert!(results.iter().all(|&ok| ok));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
