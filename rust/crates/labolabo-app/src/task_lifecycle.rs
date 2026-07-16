//! タスクのアーカイブ / 削除 (wave 6c §2) の純ロジックと git 実処理。
//!
//! - [`next_selected_id`][]: アーカイブ/削除で選択中タスクが消えるときの
//!   「次にどのタスクを選択するか」(純関数)。
//! - [`remove_worktree_and_maybe_branch`][]: worktree 型タスクの削除本体。
//!   `git worktree remove`（**force しない** -- 未コミット変更があれば git
//!   自身が拒否し、それを [`WorktreeRemoveOutcome::Refused`] として返す。
//!   呼び出し側は DB からも消さず中断する）→ 成功時のみ、チェック時に
//!   `git branch -d`（`-D` にしない -- マージ未済なら失敗し、その旨を
//!   `branch_warning` で返す。worktree 削除自体は完了扱い）。
//!   git 実行は `GitEngine`/`git_runner` 経由。**ブロッキング**なので必ず
//!   バックグラウンドスレッド（`cx.background_spawn`）から呼ぶこと。
//!
//! attached 型タスクの「削除」は DB からの登録解除のみ（実ディレクトリには
//! 一切触れない）なので、この module に対応する処理はない -- `app.rs` の
//! `execute_delete_task` が直接 `TaskDatabase::delete_task` を呼ぶ。

use std::path::Path;

use labolabo_core::{git_runner, GitEngine, GitRunError};
use rust_i18n::t;

/// 選択中タスク `removed` が一覧から消えるとき、次に選択すべきタスク id。
/// 削除位置の「次」（一覧上で同じ位置に来るタスク）を優先し、末尾なら
/// 「前」、他に何もなければ `None`（空状態）。`removed` が一覧にない場合も
/// `None`（呼び出し側バグの安全弁 -- 選択を勝手に動かさない、ではなく
/// 「動かす根拠がない」ので触らない判断は呼び出し側に委ねる）。
pub fn next_selected_id(ordered_ids: &[&str], removed: &str) -> Option<String> {
    let index = ordered_ids.iter().position(|id| *id == removed)?;
    ordered_ids
        .get(index + 1)
        .or_else(|| index.checked_sub(1).and_then(|i| ordered_ids.get(i)))
        .map(|id| (*id).to_string())
}

/// [`remove_worktree_and_maybe_branch`] の結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorktreeRemoveOutcome {
    /// worktree は削除された。`branch_warning` は「ブランチも削除」に
    /// チェックがあり、かつ `git branch -d` が失敗した（マージ未済等）
    /// ときの表示用メッセージ -- worktree 削除自体は完了扱い。
    Removed { branch_warning: Option<String> },
    /// worktree 削除自体が失敗（未コミット変更で git が拒否、等）。
    /// タスクは DB にも一覧にも残す。
    Refused { message: String },
}

/// worktree 削除の本体。ブロッキング -- バックグラウンドスレッドから呼ぶ。
///
/// `locale` (`"ja"`/`"en"`) is threaded through **explicitly** to
/// [`remove_error_message`]/[`delete_branch_gently`] rather than read via
/// `rust_i18n::locale()` ambiently at the point those error strings are
/// built -- this whole call runs on a background thread
/// (`cx.background_spawn`, `app.rs`'s `execute_delete_task`) started from
/// whatever the UI locale was at click time, so passing it down as a plain
/// argument is both correct (no risk of the global locale changing out from
/// under a long `git worktree remove` if the user flips the language
/// setting mid-operation) and keeps this module's own unit tests below
/// deterministic without mutating `rust_i18n`'s process-global current
/// locale (a shared mutable global `cargo test`'s default parallel
/// execution would otherwise race across every other test in this binary).
pub fn remove_worktree_and_maybe_branch(
    repo_root: &Path,
    worktree_path: &Path,
    branch: &str,
    delete_branch: bool,
    locale: &str,
) -> WorktreeRemoveOutcome {
    let engine = GitEngine::new();
    // force = false 固定: 未コミット変更（変更・未追跡ファイル）があれば
    // git が非ゼロ exit で拒否する。それを砕けない安全弁として使う。
    if let Err(err) = engine.remove_worktree(repo_root, worktree_path, false) {
        return WorktreeRemoveOutcome::Refused {
            message: remove_error_message(&err, locale),
        };
    }
    let branch_warning = if delete_branch {
        delete_branch_gently(repo_root, branch, locale).err()
    } else {
        None
    };
    WorktreeRemoveOutcome::Removed { branch_warning }
}

/// `git branch -d`（マージ済みのブランチのみ削除できる、安全な方）。
fn delete_branch_gently(repo_root: &Path, branch: &str, locale: &str) -> Result<(), String> {
    git_runner::run(
        &["branch".to_string(), "-d".to_string(), branch.to_string()],
        repo_root,
    )
    .map(|_| ())
    .map_err(|err| {
        t!(
            "task.delete.branch_delete_failed",
            locale = locale,
            branch = branch,
            err = err
        )
        .to_string()
    })
}

/// `git worktree remove` の失敗をユーザー向けメッセージへ。未コミット変更
/// による拒否（`contains modified or untracked files` / `use --force`）は
/// 定型文へ、それ以外は stderr をそのまま添える。
pub fn remove_error_message(err: &GitRunError, locale: &str) -> String {
    if let GitRunError::Command(command_err) = err {
        let stderr = command_err.stderr.to_lowercase();
        if stderr.contains("contains modified or untracked files") || stderr.contains("use --force")
        {
            return t!("task.delete.worktree_dirty", locale = locale).to_string();
        }
    }
    t!("task.delete.worktree_failed", locale = locale, err = err).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    // MARK: - next_selected_id（純ロジック）

    #[test]
    fn next_selection_prefers_the_following_task() {
        assert_eq!(
            next_selected_id(&["a", "b", "c"], "b"),
            Some("c".to_string())
        );
        assert_eq!(
            next_selected_id(&["a", "b", "c"], "a"),
            Some("b".to_string())
        );
    }

    #[test]
    fn next_selection_falls_back_to_the_previous_task_at_the_tail() {
        assert_eq!(
            next_selected_id(&["a", "b", "c"], "c"),
            Some("b".to_string())
        );
    }

    #[test]
    fn next_selection_is_none_when_nothing_remains() {
        assert_eq!(next_selected_id(&["only"], "only"), None);
    }

    #[test]
    fn next_selection_is_none_for_an_unknown_id() {
        assert_eq!(next_selected_id(&["a", "b"], "zz"), None);
        assert_eq!(next_selected_id(&[], "a"), None);
    }

    // MARK: - remove_error_message（純ロジック）

    fn command_error(stderr: &str) -> GitRunError {
        GitRunError::Command(labolabo_core::GitCommandError {
            arguments: vec!["worktree".into(), "remove".into()],
            exit_code: 128,
            stderr: stderr.to_string(),
        })
    }

    #[test]
    fn dirty_worktree_refusal_maps_to_the_fixed_japanese_message() {
        let err = command_error(
            "fatal: '/repo/.worktrees/x' contains modified or untracked files, \
             use --force to delete it",
        );
        let message = remove_error_message(&err, "ja");
        assert!(message.contains("未コミットの変更があるため削除できません"));
    }

    #[test]
    fn dirty_worktree_refusal_maps_to_the_fixed_english_message() {
        let err = command_error(
            "fatal: '/repo/.worktrees/x' contains modified or untracked files, \
             use --force to delete it",
        );
        let message = remove_error_message(&err, "en");
        assert!(message.contains("uncommitted changes"));
    }

    #[test]
    fn other_git_failures_keep_the_underlying_stderr() {
        let err = command_error("fatal: '/nope' is not a working tree");
        let message = remove_error_message(&err, "ja");
        assert!(message.contains("worktree を削除できませんでした"));
        assert!(message.contains("is not a working tree"));
    }

    // MARK: - remove_worktree_and_maybe_branch（一時リポジトリでの統合テスト）
    //
    // new_task.rs のテストと同じ「一時ディレクトリに使い捨てリポジトリを
    // 作る」流儀。実ユーザーの worktree には決して触れない。

    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

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

    fn git(args: &[&str], dir: &Path) {
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        git_runner::run(&args, dir).unwrap();
    }

    fn init_repo_with_commit(dir: &Path) {
        git(&["init", "-b", "main"], dir);
        git(&["config", "user.email", "test@example.com"], dir);
        git(&["config", "user.name", "LaboLabo Test"], dir);
        std::fs::write(dir.join("a.txt"), "one\n").unwrap();
        git(&["add", "."], dir);
        git(&["-c", "commit.gpgsign=false", "commit", "-m", "init"], dir);
    }

    fn add_worktree(repo: &Path, branch: &str) -> PathBuf {
        let path = repo.join(".worktrees").join(branch.replace('/', "-"));
        GitEngine::new()
            .add_worktree(repo, &path, branch, "main")
            .unwrap();
        path
    }

    fn local_branches(repo: &Path) -> Vec<String> {
        GitEngine::new().local_branches(repo).unwrap()
    }

    #[test]
    fn clean_worktree_is_removed_and_merged_branch_deleted_when_requested() {
        let repo = scratch_dir("labolabo-task-lifecycle-clean");
        init_repo_with_commit(&repo);
        let worktree = add_worktree(&repo, "feature/x");

        // ブランチは main と同一コミット = マージ済み扱いなので -d が通る。
        let outcome = remove_worktree_and_maybe_branch(&repo, &worktree, "feature/x", true, "ja");
        assert_eq!(
            outcome,
            WorktreeRemoveOutcome::Removed {
                branch_warning: None
            }
        );
        assert!(!worktree.exists(), "worktree directory must be gone");
        assert!(!local_branches(&repo).iter().any(|b| b == "feature/x"));

        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn clean_worktree_removal_keeps_the_branch_when_not_requested() {
        let repo = scratch_dir("labolabo-task-lifecycle-keep-branch");
        init_repo_with_commit(&repo);
        let worktree = add_worktree(&repo, "feature/keep");

        let outcome =
            remove_worktree_and_maybe_branch(&repo, &worktree, "feature/keep", false, "ja");
        assert_eq!(
            outcome,
            WorktreeRemoveOutcome::Removed {
                branch_warning: None
            }
        );
        assert!(!worktree.exists());
        assert!(local_branches(&repo).iter().any(|b| b == "feature/keep"));

        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn dirty_worktree_is_refused_and_left_in_place() {
        let repo = scratch_dir("labolabo-task-lifecycle-dirty");
        init_repo_with_commit(&repo);
        let worktree = add_worktree(&repo, "feature/dirty");
        // 未追跡ファイル = 未コミットの変更。force しない remove は拒否される。
        std::fs::write(worktree.join("untracked.txt"), "wip\n").unwrap();

        let outcome =
            remove_worktree_and_maybe_branch(&repo, &worktree, "feature/dirty", true, "ja");
        match outcome {
            WorktreeRemoveOutcome::Refused { message } => {
                assert!(
                    message.contains("未コミットの変更があるため削除できません"),
                    "unexpected message: {message}"
                );
            }
            other => panic!("expected Refused, got {other:?}"),
        }
        assert!(worktree.exists(), "worktree must be left in place");
        // 拒否時はブランチにも触れない。
        assert!(local_branches(&repo).iter().any(|b| b == "feature/dirty"));

        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn unmerged_branch_survives_with_a_warning_but_worktree_removal_completes() {
        let repo = scratch_dir("labolabo-task-lifecycle-unmerged");
        init_repo_with_commit(&repo);
        let worktree = add_worktree(&repo, "feature/unmerged");
        // worktree 内でコミット（クリーンだが main へマージされていない）。
        std::fs::write(worktree.join("b.txt"), "two\n").unwrap();
        git(&["add", "."], &worktree);
        git(
            &["-c", "commit.gpgsign=false", "commit", "-m", "wip"],
            &worktree,
        );

        let outcome =
            remove_worktree_and_maybe_branch(&repo, &worktree, "feature/unmerged", true, "ja");
        match outcome {
            WorktreeRemoveOutcome::Removed { branch_warning } => {
                let warning = branch_warning.expect("unmerged branch must produce a warning");
                assert!(warning.contains("feature/unmerged"), "warning: {warning}");
            }
            other => panic!("expected Removed with warning, got {other:?}"),
        }
        assert!(!worktree.exists(), "worktree removal itself must complete");
        // -D ではなく -d なのでブランチは残る。
        assert!(local_branches(&repo)
            .iter()
            .any(|b| b == "feature/unmerged"));

        let _ = std::fs::remove_dir_all(&repo);
    }
}
