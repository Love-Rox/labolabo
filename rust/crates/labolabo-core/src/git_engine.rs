//! Faithful port of `Sources/LaboLaboEngine/Git/GitEngine.swift`.
//!
//! High-level git operations for one or more worktrees, built on
//! [`crate::git_runner`] and the porcelain/unified-diff/commit-graph/
//! worktree-list parsers ported in earlier waves.
//!
//! The Swift source is an `actor` so that mutating worktree operations
//! (`addWorktree`/`removeWorktree`) are serialized against each other, while
//! read operations (`status`/`diff`/...) stay safe to call concurrently. This
//! port is a plain, stateless struct instead: `git_runner`'s concurrency
//! gate already caps total concurrent `git` subprocesses crate-wide, and
//! with no async runtime in this crate (see `process`'s module doc comment)
//! there is no `actor`-equivalent primitive to reach for without pulling one
//! in. Serializing mutating calls against each other, if ever needed, is
//! left to the caller (or a future wave) -- a deliberate, documented
//! simplification, not an oversight.

use crate::git_models::GitStatus;
use crate::git_runner::{self, GitRunError};
use crate::unified_diff::FileDiff;
use crate::worktree::Worktree;
use crate::{commit_graph, porcelain, unified_diff};
use std::path::{Path, PathBuf};

/// Stateless handle for git operations against one or more worktrees. See
/// the module doc comment for why this isn't an actor/lock like the Swift
/// source.
pub struct GitEngine;

impl Default for GitEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl GitEngine {
    pub fn new() -> Self {
        GitEngine
    }

    // MARK: - Read

    /// `git status --porcelain=v2 --branch -z`
    pub fn status(&self, worktree: &Path) -> Result<GitStatus, GitRunError> {
        let raw = git_runner::run(
            &[
                "status".to_string(),
                "--porcelain=v2".to_string(),
                "--branch".to_string(),
                "-z".to_string(),
            ],
            worktree,
        )?;
        Ok(porcelain::parse(&raw))
    }

    /// 直近コミットの件名（PR タイトルの初期値などに使う）。
    pub fn last_commit_subject(&self, worktree: &Path) -> Result<String, GitRunError> {
        let raw = git_runner::run(
            &[
                "log".to_string(),
                "-1".to_string(),
                "--format=%s".to_string(),
            ],
            worktree,
        )?;
        Ok(raw.trim().to_string())
    }

    /// ローカルブランチ名の一覧（最近のコミット順）。New Session のベースブランチ選択に使う。
    pub fn local_branches(&self, worktree: &Path) -> Result<Vec<String>, GitRunError> {
        let raw = git_runner::run(
            &[
                "for-each-ref".to_string(),
                "--format=%(refname:short)".to_string(),
                "--sort=-committerdate".to_string(),
                "refs/heads".to_string(),
            ],
            worktree,
        )?;
        Ok(raw
            .split('\n')
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect())
    }

    /// Unified diff for the whole worktree. `staged: true` uses the index (`--cached`).
    pub fn diff(&self, worktree: &Path, staged: bool) -> Result<Vec<FileDiff>, GitRunError> {
        let mut args = vec!["diff".to_string()];
        if staged {
            args.push("--cached".to_string());
        }
        let raw = git_runner::run(&args, worktree)?;
        Ok(unified_diff::parse(&raw))
    }

    /// Unified diff for a single path.
    pub fn diff_path(
        &self,
        worktree: &Path,
        path: &str,
        staged: bool,
    ) -> Result<Option<FileDiff>, GitRunError> {
        let mut args = vec!["diff".to_string()];
        if staged {
            args.push("--cached".to_string());
        }
        args.push("--".to_string());
        args.push(path.to_string());
        let raw = git_runner::run(&args, worktree)?;
        Ok(unified_diff::parse(&raw).into_iter().next())
    }

    /// 1 コミットの差分（全ファイル）。`git show --format= <hash>` を unified diff としてパース。
    pub fn commit_diff(&self, worktree: &Path, hash: &str) -> Result<Vec<FileDiff>, GitRunError> {
        let raw = git_runner::run(
            &[
                "show".to_string(),
                "--no-color".to_string(),
                "--format=".to_string(),
                hash.to_string(),
            ],
            worktree,
        )?;
        Ok(unified_diff::parse(&raw))
    }

    /// Per-file added/deleted line counts via `git diff --numstat`.
    /// Binary files report `None` counts (numstat prints `-`).
    pub fn numstat(&self, worktree: &Path, staged: bool) -> Result<Vec<NumstatEntry>, GitRunError> {
        let mut args = vec!["diff".to_string(), "--numstat".to_string()];
        if staged {
            args.push("--cached".to_string());
        }
        let raw = git_runner::run(&args, worktree)?;
        Ok(raw
            .split('\n')
            .filter(|line| !line.is_empty())
            .filter_map(|line| {
                let cols: Vec<&str> = line.split('\t').filter(|c| !c.is_empty()).collect();
                if cols.len() < 3 {
                    return None;
                }
                Some(NumstatEntry {
                    additions: cols[0].parse().ok(),
                    deletions: cols[1].parse().ok(),
                    path: cols[2].to_string(),
                })
            })
            .collect())
    }

    /// Current on-disk contents of a file in the worktree (for the "whole file" view).
    pub fn file_contents(&self, worktree: &Path, path: &str) -> std::io::Result<String> {
        std::fs::read_to_string(worktree.join(path))
    }

    pub fn list_worktrees(&self, repo: &Path) -> Result<Vec<Worktree>, GitRunError> {
        let raw = git_runner::run(
            &[
                "worktree".to_string(),
                "list".to_string(),
                "--porcelain".to_string(),
            ],
            repo,
        )?;
        Ok(crate::worktree::parse(&raw))
    }

    /// 追跡中 + 未追跡（ただし .gitignore 対象は除外）の全ファイル相対パス。
    /// `git ls-files --cached --others --exclude-standard -z`。全体ツリー表示に使う。
    pub fn list_files(&self, worktree: &Path) -> Result<Vec<String>, GitRunError> {
        let raw = git_runner::run(
            &[
                "ls-files".to_string(),
                "--cached".to_string(),
                "--others".to_string(),
                "--exclude-standard".to_string(),
                "-z".to_string(),
            ],
            worktree,
        )?;
        Ok(raw
            .split('\0')
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect())
    }

    /// worktree が属するリポジトリの識別（同一 repo の worktree はグルーピング用に同じ key を持つ）。
    /// key = 共有 git ディレクトリの絶対パス。name = origin リモート（owner/repo）優先、無ければフォルダ名。
    pub fn repo_info(&self, worktree: &Path) -> Result<RepoInfo, GitRunError> {
        let common = git_runner::run(
            &[
                "rev-parse".to_string(),
                "--path-format=absolute".to_string(),
                "--git-common-dir".to_string(),
            ],
            worktree,
        )?
        .trim()
        .to_string();

        let common_path = Path::new(&common);
        let root: String = if common_path.file_name().and_then(|s| s.to_str()) == Some(".git") {
            common_path
                .parent()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default()
        } else {
            common.clone()
        };

        let mut name = Path::new(&root)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(root.as_str())
            .to_string();

        if let Ok(remote) = git_runner::run(
            &[
                "remote".to_string(),
                "get-url".to_string(),
                "origin".to_string(),
            ],
            worktree,
        ) {
            let remote = remote.trim();
            if !remote.is_empty() {
                if let Some(parsed) = repo_name_from_remote(remote) {
                    name = parsed;
                }
            }
        }

        Ok(RepoInfo {
            key: common,
            name,
            root,
        })
    }

    /// `directory` 配下の git リポジトリ（.git を持つディレクトリ）を検出する。
    /// `directory` 自身がリポジトリならそれ 1 つ。そうでなければ（org ディレクトリ等）
    /// 子孫を `max_depth` 段まで走査し、見つけたリポジトリには降りずに列挙する。
    pub fn discover_repos(&self, directory: &Path, max_depth: usize) -> Vec<PathBuf> {
        if Self::is_git_repo(directory) {
            return vec![directory.to_path_buf()];
        }
        let mut results: Vec<PathBuf> = Vec::new();
        scan(directory, 1, max_depth, &mut results);
        // NOTE: Swift sorts with `localizedStandardCompare` (locale-aware
        // "natural" ordering, like Finder). This crate has no locale/ICU
        // dependency (matching this codebase's general "no runtime deps
        // beyond what's strictly needed" stance), so plain lexicographic
        // `Ord` on the path is used instead -- a deliberate, documented
        // simplification. It only changes display order for repo names
        // that differ under natural- vs. lexicographic sort (e.g. "repo2"
        // vs. "repo10"), never which repos are found.
        results.sort();
        results
    }

    /// `.git`（ディレクトリまたはファイル＝worktree/submodule）を持つか。
    fn is_git_repo(url: &Path) -> bool {
        url.join(".git").exists()
    }

    // MARK: - Mutate

    /// `git worktree add -b <branch> <path> <baseRef>`
    pub fn add_worktree(
        &self,
        repo: &Path,
        path: &Path,
        branch: &str,
        base_ref: &str,
    ) -> Result<(), GitRunError> {
        git_runner::run(
            &[
                "worktree".to_string(),
                "add".to_string(),
                "-b".to_string(),
                branch.to_string(),
                path.to_string_lossy().into_owned(),
                base_ref.to_string(),
            ],
            repo,
        )?;
        Ok(())
    }

    /// `git push -u origin HEAD`（現在ブランチを同名でリモートへ。PR 作成の前段）。
    pub fn push(&self, worktree: &Path) -> Result<(), GitRunError> {
        git_runner::run(
            &[
                "push".to_string(),
                "-u".to_string(),
                "origin".to_string(),
                "HEAD".to_string(),
            ],
            worktree,
        )?;
        Ok(())
    }

    /// `git worktree remove [--force] <path>`. Refuses dirty worktrees unless `force`.
    pub fn remove_worktree(
        &self,
        repo: &Path,
        path: &Path,
        force: bool,
    ) -> Result<(), GitRunError> {
        let mut args = vec!["worktree".to_string(), "remove".to_string()];
        if force {
            args.push("--force".to_string());
        }
        args.push(path.to_string_lossy().into_owned());
        git_runner::run(&args, repo)?;
        Ok(())
    }

    /// `Sources/LaboLaboEngine/Git/CommitGraph.swift`'s
    /// `GitEngine.commitGraph(worktree:limit:)` extension: shells out to
    /// `git log` and hands the raw output to the already-ported pure
    /// `commit_graph::build`. Swift defaults `limit` to `300`; Rust has no
    /// default parameters, so callers pass it explicitly (see
    /// `DEFAULT_COMMIT_GRAPH_LIMIT`).
    pub fn commit_graph(
        &self,
        worktree: &Path,
        limit: usize,
    ) -> Result<Vec<commit_graph::CommitGraphRow>, GitRunError> {
        const UNIT_SEPARATOR: char = '\u{1f}';
        let format = format!(
            "%H{us}%h{us}%s{us}%an{us}%at{us}%P{us}%d",
            us = UNIT_SEPARATOR
        );
        let raw = git_runner::run(
            &[
                "log".to_string(),
                "--all".to_string(),
                "--topo-order".to_string(),
                "--color=never".to_string(),
                format!("--pretty=format:{format}"),
                "-n".to_string(),
                limit.to_string(),
            ],
            worktree,
        )?;
        Ok(commit_graph::build(&raw))
    }
}

/// Swift's default for `GitEngine.commitGraph(worktree:limit:)`.
pub const DEFAULT_COMMIT_GRAPH_LIMIT: usize = 300;

fn scan(dir: &Path, depth: usize, max_depth: usize, results: &mut Vec<PathBuf>) {
    if depth > max_depth {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        // `.skipsHiddenFiles` in the Swift source.
        let is_hidden = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.starts_with('.'))
            .unwrap_or(false);
        if is_hidden {
            continue;
        }
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if !is_dir {
            continue;
        }
        if GitEngine::is_git_repo(&path) {
            results.push(path); // リポジトリの中へは降りない
        } else {
            scan(&path, depth + 1, max_depth, results);
        }
    }
}

/// `git@host:owner/repo(.git)` / `https://host/owner/repo(.git)` -> `owner/repo`.
///
/// Stand-in for Foundation's `URL(string:)` + `.host`/`.path`, scoped to what
/// real git remote URLs look like (`scheme://host/path`). This crate has no
/// URL-parsing dependency, so this hand-rolled version only recognizes a
/// `scheme://` prefix as having a "host" -- which matches `URL(string:)`'s
/// behavior for every input shape a git remote actually takes (bare local
/// paths, and `scheme:opaque` forms like a colon-less `git@host` with no
/// following path, have no host in Foundation's parser either, so both sides
/// fall through to `None` the same way).
fn repo_name_from_remote(remote: &str) -> Option<String> {
    let value = remote.strip_suffix(".git").unwrap_or(remote);

    if let Some(rest) = value.strip_prefix("git@") {
        if let Some(idx) = rest.find(':') {
            let path = &rest[idx + 1..];
            return if path.is_empty() {
                None
            } else {
                Some(path.to_string())
            };
        }
        // No colon: Swift's combined `if value.hasPrefix("git@"), let colon =
        // ...` fails as a whole in this case and falls through to the
        // URL-parsing attempt below using the original `value` -- replicated
        // here by simply not returning early.
    }

    let after_scheme = value.split_once("://")?.1;
    let (_host, path) = after_scheme.split_once('/').unwrap_or((after_scheme, ""));
    if path.is_empty() {
        None
    } else {
        Some(path.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoInfo {
    /// グルーピング用の安定キー（共有 git ディレクトリの絶対パス）。
    pub key: String,
    /// 表示名（owner/repo もしくはフォルダ名）。
    pub name: String,
    /// リポジトリのルートパス。
    pub root: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NumstatEntry {
    pub additions: Option<i64>,
    pub deletions: Option<i64>,
    pub path: String,
}

impl NumstatEntry {
    pub fn is_binary(&self) -> bool {
        self.additions.is_none() || self.deletions.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    // Ported from Tests/LaboLaboEngineTests/{GitEngineIntegrationTests,DiscoverReposTests}.swift.

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

    // These helpers (and every test below that calls `init_repo_with_commit`)
    // spawn the real `git` binary through `git_runner::run`, which resolves
    // `git` via the real `ToolLocator` -- `#[cfg(not(unix))]`
    // (`tool_locator.rs`) is an `unimplemented!()` stub, so any of these
    // would panic on Windows. Windows-side `ToolLocator` support is future
    // work (see `tool_locator.rs`'s module doc comment); gated out here
    // rather than working around the stub. `discover_repos_*` and
    // `repo_name_from_remote_variants` below never spawn `git` (pure
    // filesystem / string logic) and stay cross-platform.
    #[cfg(unix)]
    fn git(args: &[&str], dir: &Path) {
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        git_runner::run(&args, dir).unwrap();
    }

    #[cfg(unix)]
    fn write_file(dir: &Path, name: &str, content: &str) {
        std::fs::write(dir.join(name), content).unwrap();
    }

    #[cfg(unix)]
    fn init_repo_with_commit(dir: &Path) {
        git(&["init", "-b", "main"], dir);
        git(&["config", "user.email", "test@example.com"], dir);
        git(&["config", "user.name", "LaboLabo Test"], dir);
        write_file(dir, "a.txt", "one\ntwo\nthree\n");
        git(&["add", "."], dir);
        git(&["-c", "commit.gpgsign=false", "commit", "-m", "init"], dir);
    }

    #[cfg(unix)]
    #[test]
    fn status_diff_numstat_and_file_contents() {
        let repo = scratch_dir("labolabo-git-engine");
        init_repo_with_commit(&repo);
        write_file(&repo, "a.txt", "one\ntwo changed\nthree\nfour\n");
        write_file(&repo, "b.txt", "new file\n");

        let engine = GitEngine::new();

        let status = engine.status(&repo).unwrap();
        assert_eq!(status.branch.as_deref(), Some("main"));
        assert!(status.is_dirty());
        assert!(status.unstaged().iter().any(|e| e.path == "a.txt"));
        assert_eq!(
            status
                .untracked()
                .iter()
                .map(|e| e.path.as_str())
                .collect::<Vec<_>>(),
            vec!["b.txt"]
        );

        let diffs = engine.diff(&repo, false).unwrap();
        let a_diff = diffs
            .iter()
            .find(|d| d.display_path() == "a.txt")
            .expect("a.txt diff present");
        assert!(a_diff.additions() >= 1);
        assert!(a_diff.deletions() >= 1);

        let single = engine.diff_path(&repo, "a.txt", false).unwrap();
        assert_eq!(
            single.map(|d| d.display_path().to_string()),
            Some("a.txt".to_string())
        );

        let numstat = engine.numstat(&repo, false).unwrap();
        assert!(numstat.iter().any(|e| e.path == "a.txt"));

        assert_eq!(engine.file_contents(&repo, "b.txt").unwrap(), "new file\n");

        let _ = std::fs::remove_dir_all(&repo);
    }

    #[cfg(unix)]
    #[test]
    fn worktree_add_list_remove() {
        let repo = scratch_dir("labolabo-git-engine");
        init_repo_with_commit(&repo);

        let engine = GitEngine::new();
        let wt_path = repo.join(".worktrees/feature-x");
        engine
            .add_worktree(&repo, &wt_path, "feature/x", "main")
            .unwrap();

        let listed = engine.list_worktrees(&repo).unwrap();
        assert!(listed.iter().any(|w| w.short_branch() == Some("feature/x")));

        engine.remove_worktree(&repo, &wt_path, true).unwrap();
        let after = engine.list_worktrees(&repo).unwrap();
        assert!(!after.iter().any(|w| w.short_branch() == Some("feature/x")));

        let _ = std::fs::remove_dir_all(&repo);
    }

    fn make_repo(dir: &Path) {
        std::fs::create_dir_all(dir.join(".git")).unwrap();
    }

    #[test]
    fn discover_repos_finds_nested_and_skips_descending_into_them() {
        let tmp = scratch_dir("labolabo-disco");
        let repo_a = tmp.join("repoA");
        let repo_b = tmp.join("sub/repoB");
        make_repo(&repo_a);
        make_repo(&repo_b);
        // repoA 配下にさらに .git を含むディレクトリがあっても、repoA の中へは降りない。
        make_repo(&repo_a.join("nested"));

        let engine = GitEngine::new();
        let mut names: Vec<String> = engine
            .discover_repos(&tmp, 3)
            .iter()
            .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .collect();
        names.sort();
        assert_eq!(names, vec!["repoA".to_string(), "repoB".to_string()]);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn discover_repos_repo_itself_returns_just_itself() {
        let tmp = scratch_dir("labolabo-disco");
        make_repo(&tmp);

        let engine = GitEngine::new();
        let names: Vec<String> = engine
            .discover_repos(&tmp, 3)
            .iter()
            .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .collect();
        assert_eq!(
            names,
            vec![tmp.file_name().unwrap().to_string_lossy().into_owned()]
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn repo_name_from_remote_variants() {
        assert_eq!(
            repo_name_from_remote("git@github.com:owner/repo.git").as_deref(),
            Some("owner/repo")
        );
        assert_eq!(
            repo_name_from_remote("https://github.com/owner/repo.git").as_deref(),
            Some("owner/repo")
        );
        assert_eq!(
            repo_name_from_remote("https://github.com/owner/repo").as_deref(),
            Some("owner/repo")
        );
        assert_eq!(
            repo_name_from_remote("ssh://git@example.com:2222/owner/repo.git").as_deref(),
            Some("owner/repo")
        );
        assert_eq!(repo_name_from_remote("git@nocolon"), None);
        assert_eq!(repo_name_from_remote("/local/path/to/repo"), None);
        assert_eq!(repo_name_from_remote("https://github.com"), None);
    }

    #[cfg(unix)]
    #[test]
    fn repo_info_reports_root_key_and_remote_derived_name() {
        let repo = scratch_dir("labolabo-git-engine-repoinfo");
        init_repo_with_commit(&repo);
        git(
            &[
                "remote",
                "add",
                "origin",
                "git@github.com:Love-Rox/labolabo.git",
            ],
            &repo,
        );

        let engine = GitEngine::new();
        let info = engine.repo_info(&repo).unwrap();
        assert_eq!(info.name, "Love-Rox/labolabo");
        assert!(info.key.ends_with(".git"));
        assert!(!info.root.ends_with(".git"));

        let _ = std::fs::remove_dir_all(&repo);
    }

    #[cfg(unix)]
    #[test]
    fn last_commit_subject_and_local_branches() {
        let repo = scratch_dir("labolabo-git-engine-misc");
        init_repo_with_commit(&repo);

        let engine = GitEngine::new();
        assert_eq!(engine.last_commit_subject(&repo).unwrap(), "init");
        assert_eq!(
            engine.local_branches(&repo).unwrap(),
            vec!["main".to_string()]
        );
        assert_eq!(engine.list_files(&repo).unwrap(), vec!["a.txt".to_string()]);

        let _ = std::fs::remove_dir_all(&repo);
    }

    /// `GitEngine::commit_graph` against a real repo -- `commit_graph::build`
    /// itself already has thorough unit tests against hand-written raw `git
    /// log` output (see that module's tests), so this is deliberately just
    /// an integration smoke test of the process-spawning half: a real `git
    /// log` invocation's output round-trips through `build` into the
    /// expected row count/lane layout (`plans` W6d's "一時 git リポジトリで
    /// の統合テスト（commits 取得→行数/レーン検証）").
    #[cfg(unix)]
    #[test]
    fn commit_graph_against_a_real_repo_reports_rows_newest_first_in_one_lane() {
        let repo = scratch_dir("labolabo-git-engine-commit-graph");
        init_repo_with_commit(&repo); // 1 commit: "init"
        write_file(&repo, "a.txt", "one\ntwo\nthree\nfour\n");
        git(&["add", "."], &repo);
        git(
            &["-c", "commit.gpgsign=false", "commit", "-m", "second"],
            &repo,
        );
        write_file(&repo, "b.txt", "new\n");
        git(&["add", "."], &repo);
        git(
            &["-c", "commit.gpgsign=false", "commit", "-m", "third"],
            &repo,
        );

        let engine = GitEngine::new();
        let rows = engine
            .commit_graph(&repo, DEFAULT_COMMIT_GRAPH_LIMIT)
            .unwrap();

        assert_eq!(rows.len(), 3);
        // `--topo-order` on a linear history without `--reverse`: newest
        // commit first, exactly the `git log` default a UI wants.
        assert_eq!(
            rows.iter()
                .map(|r| r.commit.subject.as_str())
                .collect::<Vec<_>>(),
            vec!["third", "second", "init"]
        );
        // A linear (no branch/merge) history never needs a second lane --
        // every row's node stays in lane 0, mirroring
        // `commit_graph::tests::linear_history_stays_in_one_lane`.
        assert!(
            rows.iter().all(|r| r.node_lane == 0),
            "linear history should stay in a single lane: {rows:?}"
        );
        assert!(
            rows.iter().all(|r| r.commit.hash.len() >= 7),
            "GitEngine::commit_graph should report an abbreviated (%h) hash: {rows:?}"
        );
        assert!(rows.iter().all(|r| r.commit.date.is_some()));

        let _ = std::fs::remove_dir_all(&repo);
    }
}
