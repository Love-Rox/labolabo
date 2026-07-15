//! `Task`: the Rust port's own persisted unit of work, ported from the
//! design in `plans/012-task-model-and-control-cli.md` §1 ("タスクモデル仕様").
//! There is no Swift source for this type — the Swift app's unit is still
//! "one directory = one session" (`SessionRecord`, see `record.rs`); the
//! Task model is new-in-Rust product surface, decided (2026-07-14) to ship
//! only in this port.
//!
//! One `Task` is "one worktree (or one directory attached-in-place) = one
//! [`crate::tiling::TileLayout`]" — the sidebar's unit, replacing the
//! Swift app's flatter "one repo/worktree = one session" model. A Task's
//! `layout` is the *same* `TileLayout` DTO `Sources/.../PaneTilingModel`
//! already persists (see `tiling.rs`), just owned per-Task instead of
//! per-window.

use chrono::{DateTime, Utc};

use crate::tiling::TileLayout;

/// How a Task's working directory came to exist. Mirrors the plan's
/// `kind: worktree { branch, base, path } | attached { directory }` sum
/// type; `working_directory()` is the one thing callers usually need
/// regardless of which variant they have.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskKind {
    /// A dedicated `git worktree add`-created checkout (`GitEngine::
    /// add_worktree`) — `branch` is the branch it was created on, `base` is
    /// the ref it branched from, `path` is the worktree's filesystem path.
    Worktree {
        branch: String,
        base: String,
        path: String,
    },
    /// Work done directly in an existing checkout, no worktree created —
    /// the plan's "ディレクトリ直付け作業". Multiple `Attached` Tasks may
    /// share the same `directory` (the plan explicitly allows this).
    Attached { directory: String },
}

impl TaskKind {
    /// `"worktree"` / `"attached"` — the `task.kind` column's stored value.
    pub fn tag(&self) -> &'static str {
        match self {
            TaskKind::Worktree { .. } => "worktree",
            TaskKind::Attached { .. } => "attached",
        }
    }

    /// The directory a pane spawned for this Task should start in: the
    /// worktree's path, or the attached directory.
    pub fn working_directory(&self) -> &str {
        match self {
            TaskKind::Worktree { path, .. } => path,
            TaskKind::Attached { directory } => directory,
        }
    }
}

/// A Task's lifecycle state. Only `Active` Tasks are created/restored by
/// this wave — `Done`/`Archived` exist as a schema-level reservation for the
/// done/archive flow the plan (§1 "作業の完了") explicitly scopes to a later
/// wave; nothing in this port transitions a Task to either state yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatus {
    Active,
    Done,
    Archived,
}

impl TaskStatus {
    /// The `task.status` column's stored value.
    pub fn tag(self) -> &'static str {
        match self {
            TaskStatus::Active => "active",
            TaskStatus::Done => "done",
            TaskStatus::Archived => "archived",
        }
    }

    /// Inverse of [`Self::tag`]; `None` for anything else (see
    /// `StoreError::InvalidTaskEnum`).
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "active" => Some(TaskStatus::Active),
            "done" => Some(TaskStatus::Done),
            "archived" => Some(TaskStatus::Archived),
            _ => None,
        }
    }
}

/// One persisted Task: a repo-scoped unit of work with its own
/// [`TileLayout`] (the plan's §1 data model).
#[derive(Debug, Clone, PartialEq)]
pub struct Task {
    /// UUID v4 string, minted once by [`Task::new_worktree`]/
    /// [`Task::new_attached`] and stable for the Task's lifetime.
    pub id: String,
    /// `GitEngine::RepoInfo::key` — the shared git directory's absolute
    /// path. The sidebar's grouping key (same repo -> same group), same
    /// role `SessionRecord`'s implicit repo grouping plays in the Swift app.
    pub repo_key: String,
    /// `GitEngine::RepoInfo::root` — the repo's (non-worktree) root path.
    pub repo_root: String,
    /// `GitEngine::RepoInfo::name` — the sidebar group's display label
    /// (`owner/repo` when a remote is configured, else the folder name).
    pub repo_name: String,
    pub kind: TaskKind,
    /// Display title — defaults to the branch name (worktree) or the
    /// directory's last path component (attached) at creation time; free-
    /// form after that (manual rename is out of this wave's scope, but the
    /// field itself is already plain, renamable `String`).
    pub title: String,
    pub layout: TileLayout,
    pub status: TaskStatus,
    pub created_at: DateTime<Utc>,
    pub last_active_at: DateTime<Utc>,
    /// Sidebar manual ordering — same role `SessionRecord::sort_order`
    /// plays today. DnD reordering (plan §3) is out of this wave's scope;
    /// new Tasks are appended (see `TaskDatabase::next_sort_order`).
    pub sort_order: i64,
    /// The plan's §1 `agentBindings`: `Some(json)` (see
    /// `crate::store::AgentBindings::to_json`/`from_json`) once a hooks
    /// event with a `session_id` has been observed for this Task, `None`
    /// until then. Scoped to the Task-level docs/hooks-protocol.md §6(a)
    /// "last known session id/transcript path" fallback only -- per-tab
    /// bindings live in `layout` instead (`tiling::PaneItem::
    /// agent_session_id`/`agent_transcript_path`); see
    /// `crate::store::agent_bindings`'s module doc comment for why.
    pub agent_bindings: Option<String>,
}

impl Task {
    /// A new `worktree`-kind Task, freshly created (both timestamps `now`,
    /// `status: Active`). `title` defaults to the branch name, matching the
    /// plan's "既定はブランチ名 or エージェントのセッションタイトル".
    #[allow(clippy::too_many_arguments)]
    pub fn new_worktree(
        repo_key: impl Into<String>,
        repo_root: impl Into<String>,
        repo_name: impl Into<String>,
        branch: impl Into<String>,
        base: impl Into<String>,
        path: impl Into<String>,
        layout: TileLayout,
        sort_order: i64,
    ) -> Self {
        let branch = branch.into();
        let title = branch.clone();
        Self::new(
            repo_key,
            repo_root,
            repo_name,
            TaskKind::Worktree {
                branch,
                base: base.into(),
                path: path.into(),
            },
            title,
            layout,
            sort_order,
        )
    }

    /// A new `attached`-kind Task. `title` defaults to the directory's last
    /// path component (falling back to the full path for a root-like
    /// directory with no component, e.g. `/`).
    pub fn new_attached(
        repo_key: impl Into<String>,
        repo_root: impl Into<String>,
        repo_name: impl Into<String>,
        directory: impl Into<String>,
        layout: TileLayout,
        sort_order: i64,
    ) -> Self {
        let directory = directory.into();
        let title = std::path::Path::new(&directory)
            .file_name()
            .and_then(|n| n.to_str())
            .map(str::to_string)
            .unwrap_or_else(|| directory.clone());
        Self::new(
            repo_key,
            repo_root,
            repo_name,
            TaskKind::Attached { directory },
            title,
            layout,
            sort_order,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn new(
        repo_key: impl Into<String>,
        repo_root: impl Into<String>,
        repo_name: impl Into<String>,
        kind: TaskKind,
        title: impl Into<String>,
        layout: TileLayout,
        sort_order: i64,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            repo_key: repo_key.into(),
            repo_root: repo_root.into(),
            repo_name: repo_name.into(),
            kind,
            title: title.into(),
            layout,
            status: TaskStatus::Active,
            created_at: now,
            last_active_at: now,
            sort_order,
            agent_bindings: None,
        }
    }

    /// The directory a pane spawned for this Task should start in. See
    /// [`TaskKind::working_directory`].
    pub fn working_directory(&self) -> &str {
        self.kind.working_directory()
    }

    /// `"task"` — mirrors `SessionRecord::TABLE_NAME`'s naming convention.
    pub const TABLE_NAME: &'static str = "task";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_worktree_defaults_title_to_branch_and_is_active() {
        let task = Task::new_worktree(
            "/repo/.git",
            "/repo",
            "owner/repo",
            "feature/x",
            "main",
            "/repo/.worktrees/feature-x",
            TileLayout::default(),
            0,
        );
        assert_eq!(task.title, "feature/x");
        assert_eq!(task.status, TaskStatus::Active);
        assert_eq!(task.working_directory(), "/repo/.worktrees/feature-x");
        assert_eq!(task.created_at, task.last_active_at);
        assert!(!task.id.is_empty());
    }

    #[test]
    fn new_attached_defaults_title_to_directory_leaf() {
        let task = Task::new_attached(
            "/repo/.git",
            "/repo",
            "owner/repo",
            "/Users/me/scratch",
            TileLayout::default(),
            1,
        );
        assert_eq!(task.title, "scratch");
        assert_eq!(task.working_directory(), "/Users/me/scratch");
    }

    #[test]
    fn two_tasks_get_distinct_ids() {
        let a = Task::new_attached("k", "r", "n", "/tmp/a", TileLayout::default(), 0);
        let b = Task::new_attached("k", "r", "n", "/tmp/b", TileLayout::default(), 0);
        assert_ne!(a.id, b.id);
    }

    #[test]
    fn task_status_tag_round_trips() {
        for status in [TaskStatus::Active, TaskStatus::Done, TaskStatus::Archived] {
            assert_eq!(TaskStatus::parse(status.tag()), Some(status));
        }
        assert_eq!(TaskStatus::parse("bogus"), None);
    }

    #[test]
    fn task_kind_tag_matches_variant() {
        let worktree = TaskKind::Worktree {
            branch: "b".into(),
            base: "main".into(),
            path: "/p".into(),
        };
        assert_eq!(worktree.tag(), "worktree");
        let attached = TaskKind::Attached {
            directory: "/d".into(),
        };
        assert_eq!(attached.tag(), "attached");
    }
}
