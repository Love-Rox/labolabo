//! "New Task" flows: resolving a picked directory's repo identity and,
//! for the worktree kind, branch-name generation + `git worktree add`
//! (`plans/012-task-model-and-control-cli.md` §1's "作業の開始（主 CTA）").
//!
//! Both flows are driven from `app.rs`'s `start_new_attached_task`/
//! `start_new_worktree_task`: `cx.prompt_for_paths` (gpui's native OS
//! directory picker) on the gpui foreground executor, then the blocking git
//! work in `cx.background_spawn` so it never blocks the UI thread. The
//! functions here are free functions (not `LaboLaboApp` methods)
//! specifically so they can run inside a `'static + Send` background
//! future without capturing `&LaboLaboApp` -- they only touch `GitEngine`/
//! `branch_naming`, neither of which needs app state.

use std::path::Path;

use labolabo_core::{branch_naming, GitEngine};

/// The git-side outcome of a successful "new worktree Task" flow, handed
/// back to `LaboLaboApp::finish_new_worktree_task` to turn into a persisted
/// [`labolabo_core::Task`].
pub struct PreparedWorktree {
    pub repo_key: String,
    pub repo_root: String,
    pub repo_name: String,
    pub branch: String,
    pub base: String,
    pub worktree_path: String,
}

/// Resolves `repo_path`'s repo identity, generates a fresh branch name
/// (`branch_naming::generate_branch_name`, prefix `"labolabo"`) that
/// doesn't collide with any existing local branch, and runs `git worktree
/// add` for it under `<repo_root>/.worktrees/<branch, slashes as dashes>`
/// -- the same worktree-path convention `GitEngine::add_worktree`'s own
/// test uses (`.worktrees/feature-x` for branch `feature/x`).
///
/// `base` defaults to `repo_path`'s current branch (`GitEngine::status`),
/// falling back to `"main"` if that can't be determined (detached HEAD, or
/// the status call itself failing) -- the plan leaves base-branch
/// selection UI to a future, fuller dialog ("本格ダイアログは将来"); this
/// wave's "+"-menu flow has no field for it.
pub fn create_worktree_task(repo_path: &Path) -> Result<PreparedWorktree, String> {
    let engine = GitEngine::new();
    let repo = engine
        .repo_info(repo_path)
        .map_err(|err| format!("not a git repository ({err})"))?;
    let repo_root = Path::new(&repo.root);

    let existing = engine.local_branches(repo_root).unwrap_or_default();
    let base = engine
        .status(repo_root)
        .ok()
        .and_then(|status| status.branch)
        .unwrap_or_else(|| "main".to_string());
    let branch =
        branch_naming::generate_branch_name("labolabo", chrono::Utc::now().date_naive(), &existing);

    let worktree_dir_name = branch.replace('/', "-");
    let worktree_path = repo_root.join(".worktrees").join(&worktree_dir_name);

    engine
        .add_worktree(repo_root, &worktree_path, &branch, &base)
        .map_err(|err| format!("git worktree add failed ({err})"))?;

    Ok(PreparedWorktree {
        repo_key: repo.key,
        repo_root: repo.root,
        repo_name: repo.name,
        branch,
        base,
        worktree_path: worktree_path.to_string_lossy().into_owned(),
    })
}

/// Resolves `directory`'s repo identity for the "attached" flow. Falls
/// back to using `directory` itself as `repo_key`/`repo_root`/`repo_name`
/// when it isn't inside a git repository (`GitEngine::repo_info` fails) --
/// this wave has no repo-registry UI to fall back to (plan §1's own
/// "既存フォルダを開く" reinterpretation assumes one), so a non-repo
/// directory still produces a working (if group-of-one) sidebar entry
/// rather than failing the whole flow outright.
pub fn resolve_attached_repo(directory: &Path) -> (String, String, String) {
    let engine = GitEngine::new();
    match engine.repo_info(directory) {
        Ok(repo) => (repo.key, repo.root, repo.name),
        Err(_) => {
            let dir = directory.to_string_lossy().into_owned();
            (dir.clone(), dir.clone(), dir)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn scratch_dir(prefix: &str) -> std::path::PathBuf {
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
        labolabo_core::git_runner::run(&args, dir).unwrap();
    }

    fn init_repo_with_commit(dir: &Path) {
        git(&["init", "-b", "main"], dir);
        git(&["config", "user.email", "test@example.com"], dir);
        git(&["config", "user.name", "LaboLabo Test"], dir);
        std::fs::write(dir.join("a.txt"), "one\n").unwrap();
        git(&["add", "."], dir);
        git(&["-c", "commit.gpgsign=false", "commit", "-m", "init"], dir);
    }

    #[test]
    fn create_worktree_task_adds_a_worktree_on_a_generated_branch() {
        let repo = scratch_dir("labolabo-new-task-worktree");
        init_repo_with_commit(&repo);

        let prepared = create_worktree_task(&repo).expect("worktree creation should succeed");
        assert_eq!(prepared.base, "main");
        assert!(prepared.branch.starts_with("labolabo/"));
        assert!(std::path::Path::new(&prepared.worktree_path).is_dir());
        assert!(prepared.repo_key.ends_with(".git"));

        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn create_worktree_task_on_a_non_repo_directory_fails() {
        let dir = scratch_dir("labolabo-new-task-not-a-repo");
        assert!(create_worktree_task(&dir).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_attached_repo_uses_git_engine_inside_a_repo() {
        let repo = scratch_dir("labolabo-new-task-attached-repo");
        init_repo_with_commit(&repo);
        let (key, root, _name) = resolve_attached_repo(&repo);
        assert!(key.ends_with(".git"));
        assert!(!root.ends_with(".git"));
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn resolve_attached_repo_falls_back_to_the_directory_itself_outside_a_repo() {
        let dir = scratch_dir("labolabo-new-task-attached-non-repo");
        let (key, root, name) = resolve_attached_repo(&dir);
        let expected = dir.to_string_lossy().into_owned();
        assert_eq!(key, expected);
        assert_eq!(root, expected);
        assert_eq!(name, expected);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
