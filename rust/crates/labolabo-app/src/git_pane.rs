//! The Git pane: LaboLabo's namesake feature -- "ライブな Git 状態とファイル
//!差分を端末の真横で確認" -- ported from the Swift app's `WorkPaneModel`/
//! `WorkPaneView` (`app/Sources/WorkPaneModel.swift`, `WorkPaneView.swift`).
//!
//! ## Fixed right pane, not a tile
//!
//! The Swift app renders `ChangedFilesPane`/`FileDetailPane`/`CommitGraphPane`
//! as ordinary tiles in its `PaneTilingModel` tree (the user can split/move/
//! close them like any terminal pane) -- but per its own doc comment,
//! `WorkPaneModel` is deliberately independent of the tiling tree: "Lives as
//! its own tile so it can be moved/split independently of the diff." This
//! port takes the *lighter* of the two options this wave's brief explicitly
//! allows ("実装が軽い方に倒して良い"): a single fixed pane on the right edge
//! of the selected Task's workspace, toggled by `Cmd+Shift+G`
//! ([`crate::app::ToggleGitPane`]), entirely outside
//! `labolabo_core::tiling::PaneTilingModel` -- no new `PaneKind`, no
//! persistence changes, no drag/split/move affordances for it. This avoids
//! touching the tiling model's persisted `TileLayout` shape at all (a
//! materially larger change: a new `PaneKind` variant is part of every
//! existing `TileLayout` JSON fixture/golden test) for a feature this port's
//! Task model has no Swift-side precedent to follow for "where does a
//! non-terminal pane's placement persist." If per-Task placement (resize,
//! move into the tile tree) is wanted later, revisit as its own wave.
//!
//! ## Ownership and lifecycle
//!
//! One [`GitPaneState`] lives on each [`crate::task_workspace::TaskWorkspace`]
//! (mirrors Swift's one `WorkPaneModel` per `SessionDetailView`). Its
//! [`labolabo_core::FileWatcher`] is only ever live for the **selected**
//! Task's pane, and only while [`GitPaneState::visible`] -- see
//! `LaboLaboApp::activate_git_pane`/`deactivate_git_pane` (`app.rs`), called
//! from `select_task`/the toggle action/app startup. This mirrors the Swift
//! app's `SessionDetailView.onChange(of: isActive)` (`work.start()`/
//! `work.stop()`) and its "非表示なら監視を張らない" module-doc-comment
//! requirement for this wave; unlike Swift (one `WorkPaneModel` per session,
//! all of which may be simultaneously visible in a windowed/tabbed
//! multi-session UI), this port's one-Task-visible-at-a-time window means
//! **at most one** Git-pane `FileWatcher` is ever alive app-wide.
//!
//! ## Refresh coalescing
//!
//! [`GitPaneState::begin_refresh`]/[`GitPaneState::finish_refresh`] are the
//! pure state machine behind `LaboLaboApp::request_git_refresh`/
//! `apply_git_refresh` (`app.rs`): mirrors `WorkPaneModel.scheduleRefresh`'s
//! "実行中の refresh には合流し（多重実行しない）" contract -- a refresh
//! already in flight absorbs any further trigger (FileWatcher event, or a
//! fresh file selection) into a single pending flag, and the in-flight
//! refresh's completion re-triggers **at most one** more when it finishes,
//! rather than queuing one per trigger. See their doc comments and the unit
//! tests below for the exact state machine.
//!
//! ## What runs where
//!
//! [`compute_git_snapshot`] -- the actual `git status`/`numstat`/`diff`/
//! file-read calls -- always runs inside `cx.background_spawn` (see
//! `LaboLaboApp::request_git_refresh`), never on gpui's main/UI thread, per
//! this wave's brief ("git 実行はバックグラウンドスレッドで"). It always
//! recomputes the *whole* snapshot (status + both numstats + the selected
//! file's diff/whole-file contents in one call) rather than Swift's
//! per-repo/per-changed-path partial refresh -- a deliberate simplification
//! this port can afford because a Task's working directory is always
//! exactly one repository (no Swift-style "org directory with multiple
//! repos" scanning to short-circuit), so a full `git status` is already
//! cheap.
//!
//! ## Diff ⇄ Whole file
//!
//! Both the selected file's diff and its whole-file contents are fetched
//! together on every refresh (mirrors `WorkPaneModel.loadSelection`, which
//! does the same); [`GitPaneState::view_mode`] just picks which one
//! [`render_git_pane`] shows, so toggling is instant (no extra fetch) --
//! ported in full, not left as a TODO.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use futures::channel::mpsc;
use futures::StreamExt;
use gpui::{
    div, prelude::*, px, relative, rgb, AnyElement, Context, IntoElement, MouseButton,
    MouseDownEvent, SharedString, Task as GpuiTask,
};
use rust_i18n::t;

use labolabo_core::{
    CommitGraphRow, DiffLine, FileDiff, FileWatcher, GitEngine, GitStatus, Kind, LineKind,
    NumstatEntry, DEFAULT_COMMIT_GRAPH_LIMIT,
};

use crate::app::LaboLaboApp;
use crate::render::RenderSpec;
use crate::theme;

/// Fixed width of the Git pane -- same "no resize handle yet" simplification
/// `sidebar::SIDEBAR_WIDTH` already made for the Task sidebar.
pub const GIT_PANE_WIDTH: f32 = 340.0;

/// Watch latency for the Git pane's [`FileWatcher`] -- matches the Swift
/// `FileWatcher`'s own default (`FileWatcher.init(path:latency:onChange:)`'s
/// `latency: TimeInterval = 0.4`), which this port's `file_watcher::
/// debounce_loop` plays the equivalent coalescing role for (see that
/// module's doc comment -- `notify` has no built-in FSEventStream-style
/// batching, so this port's debounce thread stands in for it).
const GIT_PANE_WATCH_LATENCY: Duration = Duration::from_millis(400);

// ============================================================================
// Pure data: changed-file rows (ported from `WorkPaneModel.ChangedFileItem`)
// ============================================================================

/// Mirrors Swift's `ChangedFileItem.Section`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileSection {
    Staged,
    Unstaged,
    Untracked,
}

impl FileSection {
    /// Localized section heading (wave 6f) -- previously hardcoded English
    /// ("Staged"/"Unstaged"/"Untracked") even in the otherwise-Japanese UI;
    /// the ja locale now translates these (one of the deliberate ja-text
    /// changes listed in the wave's PR description).
    fn label(self) -> String {
        match self {
            FileSection::Staged => t!("git.status.staged").to_string(),
            FileSection::Unstaged => t!("git.status.unstaged").to_string(),
            FileSection::Untracked => t!("git.status.untracked").to_string(),
        }
    }

    /// Badge color per section -- amber/lime/rose semantics ported from
    /// `ChangedFileRow.sectionColor` (`WorkPaneView.swift`), now sourced
    /// from `crate::theme` (`plans/013`) instead of ad hoc hex.
    fn badge_color(self) -> u32 {
        match self {
            FileSection::Staged => theme::diff::ADD,
            FileSection::Unstaged => theme::status::STARTING,
            FileSection::Untracked => theme::status::CONFLICT,
        }
    }
}

/// Mirrors Swift's `FileViewMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FileViewMode {
    #[default]
    Diff,
    Whole,
}

/// One row in the changed-files list. Mirrors `WorkPaneModel.
/// ChangedFileItem`, minus the multi-repo `repoRoot`/display-path-prefixing
/// fields (`path` is already repo-relative -- a Task's working directory is
/// always exactly one repo, see this module's doc comment) and the
/// `modifiedAt`/"更新順" sort (out of scope for this pass -- the pane always
/// shows staged/unstaged/untracked grouped, matching Swift's default
/// "変更" tree mode; "更新順" is a TODO, see `render_git_pane`'s doc comment).
#[derive(Debug, Clone, PartialEq)]
pub struct ChangedFileItem {
    pub path: String,
    pub section: FileSection,
    pub adds: Option<i64>,
    pub dels: Option<i64>,
}

/// `git status` + both `numstat`s -> display rows. Pure, no I/O -- the I/O
/// (`GitEngine::status`/`numstat`) is [`compute_git_snapshot`]'s job, kept
/// separate so this conversion is unit-testable without a real repo (per
/// this wave's brief: "status→表示行の変換...はユニットテスト").
///
/// Row order and section membership are a faithful port of
/// `WorkPaneModel.refresh()`'s three loops: staged entries, then unstaged
/// entries **excluding unmerged/conflicted ones** (Swift's `where entry.kind
/// != .unmerged` -- conflicts are surfaced elsewhere, not as a plain
/// "unstaged" row), then untracked entries (no numstat -- `git diff` never
/// covers untracked paths).
pub fn build_changed_items(
    status: &GitStatus,
    staged_numstat: &[NumstatEntry],
    unstaged_numstat: &[NumstatEntry],
) -> Vec<ChangedFileItem> {
    let staged_counts: HashMap<&str, &NumstatEntry> = staged_numstat
        .iter()
        .map(|n| (n.path.as_str(), n))
        .collect();
    let unstaged_counts: HashMap<&str, &NumstatEntry> = unstaged_numstat
        .iter()
        .map(|n| (n.path.as_str(), n))
        .collect();

    let mut items = Vec::new();

    for entry in status.staged() {
        let n = staged_counts.get(entry.path.as_str());
        items.push(ChangedFileItem {
            path: entry.path.clone(),
            section: FileSection::Staged,
            adds: n.and_then(|n| n.additions),
            dels: n.and_then(|n| n.deletions),
        });
    }

    for entry in status.unstaged() {
        if entry.kind == Kind::Unmerged {
            continue;
        }
        let n = unstaged_counts.get(entry.path.as_str());
        items.push(ChangedFileItem {
            path: entry.path.clone(),
            section: FileSection::Unstaged,
            adds: n.and_then(|n| n.additions),
            dels: n.and_then(|n| n.deletions),
        });
    }

    for entry in status.untracked() {
        items.push(ChangedFileItem {
            path: entry.path.clone(),
            section: FileSection::Untracked,
            adds: None,
            dels: None,
        });
    }

    items
}

/// The full set of paths `status` reports as changed -- every entry whose
/// `kind` isn't [`Kind::Ignored`] (matching the Swift app's
/// `SessionStore.refreshChangedFiles`: `status.entries.filter { $0.kind !=
/// .ignored }`), including both sides of a rename/copy (`path` *and*
/// `original_path`, mirroring Swift's `[entry.path, entry.originalPath]
/// .compactMap { $0 }`). Deliberately broader than [`build_changed_items`]'s
/// display rows (which drop unmerged/conflicted entries from the unstaged
/// section) -- this is the *input* to cross-session conflict detection
/// (`crate::app::LaboLaboApp::task_conflicts`,
/// `labolabo_core::cross_session_conflicts`), where a conflicted path is
/// exactly the kind of overlap worth flagging, not one to hide.
pub fn changed_paths(status: &GitStatus) -> HashSet<String> {
    status
        .entries
        .iter()
        .filter(|e| e.kind != Kind::Ignored)
        .flat_map(|e| std::iter::once(e.path.clone()).chain(e.original_path.clone()))
        .collect()
}

// ============================================================================
// Background refresh (real I/O -- always run via `cx.background_spawn`)
// ============================================================================

/// The result of one full Git-pane refresh -- everything [`GitPaneState::
/// apply`] needs to update the UI in one shot.
pub struct GitSnapshot {
    pub status: Option<GitStatus>,
    pub items: Vec<ChangedFileItem>,
    pub diff: Option<FileDiff>,
    pub whole_text: Option<String>,
    /// The commit-history graph (`GitEngine::commit_graph`,
    /// `labolabo_core::commit_graph::build`'s laid-out rows), fetched
    /// alongside everything else in [`compute_git_snapshot`] -- this
    /// module's doc comment's "常に全体のsnapshotを再計算する" design (a
    /// Task's working directory is always exactly one repo, so one more
    /// bounded `git log` call per refresh is cheap enough to always include
    /// rather than gate on whether a `Commits`-kind pane happens to be
    /// visible this round). Empty (never `None`) when `git log` itself
    /// fails for a reason other than "not a repo" (that case is folded into
    /// [`Self::load_error`] like every other field here) -- a `Commits` tile
    /// pane simply shows no rows rather than surfacing a second error UI.
    pub commits: Vec<CommitGraphRow>,
    /// `Some` only when `git status` itself failed (e.g. `working_dir` isn't
    /// a git repository at all -- possible for an `attached`-kind Task,
    /// which places no git-repo requirement on the directory the user
    /// picked). Everything else is left empty rather than partially
    /// populated in that case.
    pub load_error: Option<String>,
}

/// Runs `git status`/`numstat`/(if `selected_path`) `diff`+file-read against
/// `working_dir` and returns the full snapshot. Blocking, real subprocess
/// I/O -- callers must run this on a background thread (`cx.background_spawn`
/// in `LaboLaboApp::request_git_refresh`), never gpui's main thread.
pub fn compute_git_snapshot(working_dir: &Path, selected_path: Option<&str>) -> GitSnapshot {
    let engine = GitEngine::new();

    let status = match engine.status(working_dir) {
        Ok(status) => status,
        Err(err) => {
            return GitSnapshot {
                status: None,
                items: Vec::new(),
                diff: None,
                whole_text: None,
                commits: Vec::new(),
                load_error: Some(err.to_string()),
            };
        }
    };

    let staged_numstat = engine.numstat(working_dir, true).unwrap_or_default();
    let unstaged_numstat = engine.numstat(working_dir, false).unwrap_or_default();
    let items = build_changed_items(&status, &staged_numstat, &unstaged_numstat);

    let mut diff = None;
    let mut whole_text = None;
    if let Some(path) = selected_path {
        let staged = items
            .iter()
            .find(|item| item.path == path)
            .is_some_and(|item| item.section == FileSection::Staged);
        diff = engine.diff_path(working_dir, path, staged).ok().flatten();
        whole_text = engine.file_contents(working_dir, path).ok();
    }

    let commits = engine
        .commit_graph(working_dir, DEFAULT_COMMIT_GRAPH_LIMIT)
        .unwrap_or_default();

    GitSnapshot {
        status: Some(status),
        items,
        diff,
        whole_text,
        commits,
        load_error: None,
    }
}

// ============================================================================
// FileWatcher <-> gpui bridge
// ============================================================================

/// Keeps a Git pane's live [`FileWatcher`] and its gpui bridge task alive
/// together -- stored in [`GitPaneState::watch`]. Dropping this (via
/// [`GitPaneState::detach_watch`]) stops the watcher (see `FileWatcher::
/// stop`'s synchronous-stop guarantee) and ends the bridge task.
pub struct GitWatchHandle {
    /// Never read after construction -- kept alive purely for its `Drop`
    /// (which calls `FileWatcher::stop`, see `GitPaneState::detach_watch`'s
    /// doc comment). Mirrors `task_workspace::PaneRuntime::_redraw_task`'s
    /// same "held for its Drop, not its value" shape.
    _watcher: FileWatcher,
    _bridge_task: GpuiTask<()>,
}

/// Starts watching `working_directory` for `task_id`'s Git pane and bridges
/// its debounced change notifications into a "please refresh" call on
/// [`LaboLaboApp::request_git_refresh`] -- the same "OS-thread callback ->
/// channel -> gpui task" shape as `task_workspace::spawn_redraw_bridge`/
/// `hooks::spawn_agent_event_bridge` (see either's doc comment).
///
/// Only a bare wakeup is threaded through the channel, not the changed
/// paths themselves -- [`compute_git_snapshot`] always does a full refresh
/// (see this module's doc comment for why that's cheap enough here), so the
/// path list `FileWatcher`'s callback receives has no further use once it's
/// decided "something changed."
///
/// Returns `None` (after printing a warning) if the underlying OS watch
/// itself fails to start -- e.g. `working_directory` doesn't exist (an
/// `attached`-kind Task's directory was removed after the Task was created)
/// or -- on Linux -- the process is out of inotify watch descriptors. The
/// pane still shows whatever the caller's own initial refresh fetches, just
/// without live updates, rather than failing the whole Task.
pub fn spawn_git_watch_bridge(
    task_id: String,
    working_directory: PathBuf,
    cx: &mut Context<LaboLaboApp>,
) -> Option<GitWatchHandle> {
    let (tx, mut rx) = mpsc::unbounded::<()>();
    let watcher =
        match FileWatcher::watch(&working_directory, GIT_PANE_WATCH_LATENCY, move |_paths| {
            let _ = tx.unbounded_send(());
        }) {
            Ok(watcher) => watcher,
            Err(err) => {
                eprintln!(
                    "labolabo-app: failed to watch {working_directory:?} for the Git pane: {err} \
                 (showing a one-time snapshot with no live updates)"
                );
                return None;
            }
        };

    let bridge_task = cx.spawn(async move |this, cx| {
        while rx.next().await.is_some() {
            if this
                .update(cx, |app, cx| app.request_git_refresh(&task_id, cx))
                .is_err()
            {
                break;
            }
        }
    });

    Some(GitWatchHandle {
        _watcher: watcher,
        _bridge_task: bridge_task,
    })
}

// ============================================================================
// Per-Task state
// ============================================================================

/// One Task's Git-pane state -- lives on `TaskWorkspace::git`. Contains no
/// gpui types other than [`GitWatchHandle`]'s bridge task, so the
/// refresh-coalescing state machine below is plain-Rust unit-testable.
pub struct GitPaneState {
    /// Pane visibility (`Cmd+Shift+G` toggles this). Defaults to `true` --
    /// matches the Swift app's default layout, which always includes the
    /// WorkPane tiles.
    pub visible: bool,
    pub status: Option<GitStatus>,
    pub items: Vec<ChangedFileItem>,
    pub selected_path: Option<String>,
    pub view_mode: FileViewMode,
    pub diff: Option<FileDiff>,
    pub whole_text: Option<String>,
    /// The commit-history graph -- see [`GitSnapshot::commits`]'s doc
    /// comment. Read by a `Commits`-kind tile pane
    /// (`crate::commit_pane::render_commits_pane`); the fixed right pane
    /// never shows it (this module's doc comment's own scope note).
    pub commits: Vec<CommitGraphRow>,
    pub load_error: Option<String>,
    refreshing: bool,
    refresh_pending: bool,
    watch: Option<GitWatchHandle>,
}

impl Default for GitPaneState {
    fn default() -> Self {
        Self {
            visible: true,
            status: None,
            items: Vec::new(),
            selected_path: None,
            view_mode: FileViewMode::Diff,
            diff: None,
            whole_text: None,
            commits: Vec::new(),
            load_error: None,
            refreshing: false,
            refresh_pending: false,
            watch: None,
        }
    }
}

impl GitPaneState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_watching(&self) -> bool {
        self.watch.is_some()
    }

    pub fn attach_watch(&mut self, handle: GitWatchHandle) {
        self.watch = Some(handle);
    }

    /// Stops the live watcher (if any) -- dropping [`GitWatchHandle`] tears
    /// down the `FileWatcher` synchronously (see its doc comment), so once
    /// this returns no more refreshes will be triggered by filesystem
    /// changes until [`Self::attach_watch`] is called again. Cached
    /// `items`/`status`/`diff` are left untouched (the pane keeps showing
    /// its last-known snapshot while hidden/backgrounded, same as Swift's
    /// `work.stop()`).
    pub fn detach_watch(&mut self) {
        self.watch = None;
    }

    /// Selecting a file: mirrors `WorkPaneModel.select(path:)`'s default
    /// view-mode rule verbatim -- an untracked (or not-yet-known) file
    /// defaults to whole-file view (there's nothing to diff), anything else
    /// defaults to diff view, discarding whatever mode was showing before.
    pub fn select(&mut self, path: String) {
        let is_untracked = self
            .items
            .iter()
            .find(|item| item.path == path)
            .map(|item| item.section == FileSection::Untracked);
        self.view_mode = match is_untracked {
            Some(false) => FileViewMode::Diff,
            Some(true) | None => FileViewMode::Whole,
        };
        self.selected_path = Some(path);
    }

    pub fn apply(&mut self, snapshot: GitSnapshot) {
        self.status = snapshot.status;
        self.items = snapshot.items;
        self.diff = snapshot.diff;
        self.whole_text = snapshot.whole_text;
        self.commits = snapshot.commits;
        self.load_error = snapshot.load_error;
    }

    /// Call before spawning a background refresh. Returns `true` if the
    /// caller should actually spawn one now; `false` means a refresh is
    /// already in flight and this request has been coalesced into it (see
    /// [`Self::finish_refresh`]) -- mirrors `WorkPaneModel.scheduleRefresh`'s
    /// "実行中の refresh には合流し（多重実行しない）".
    pub fn begin_refresh(&mut self) -> bool {
        if self.refreshing {
            self.refresh_pending = true;
            false
        } else {
            self.refreshing = true;
            self.refresh_pending = false;
            true
        }
    }

    /// Call when a background refresh completes (whether or not its result
    /// was applied). Returns `true` if the caller should immediately spawn
    /// **one more** refresh -- a trigger arrived while this one was in
    /// flight ([`Self::begin_refresh`] recorded it as pending rather than
    /// spawning a second overlapping one). Returns `false`, and leaves the
    /// pane idle (no refresh in flight), otherwise.
    pub fn finish_refresh(&mut self) -> bool {
        self.refreshing = false;
        if self.refresh_pending {
            self.refresh_pending = false;
            self.refreshing = true;
            true
        } else {
            false
        }
    }
}

// ============================================================================
// Rendering
// ============================================================================

const PANEL_BG: u32 = theme::surface::SUNKEN;
const BORDER_COLOR: u32 = theme::surface::STROKE;
const HEADER_BG: u32 = theme::surface::RAISED;
/// A selected changed-file row: deliberately `ACTIVE` rather than the
/// `RAISED` most other former-`0x2f2f2f` uses map to (`plans/013`'s own
/// mapping table calls this `0x2f2f2f -> RAISED` in general) -- a selected
/// row is a "selection" affordance like a selected tab chip, not a raised
/// chrome surface, so `ACTIVE` is the semantically correct token here even
/// though the literal hex value used to coincide with the `RAISED` group.
const SELECTED_ROW_BG: u32 = theme::surface::ACTIVE;
const HUNK_HEADER_BG: u32 = theme::surface::RAISED;
const ADDITION_BG: u32 = theme::diff::ADD_BG;
const ADDITION_FG: u32 = theme::diff::ADD;
const DELETION_BG: u32 = theme::diff::DEL_BG;
const DELETION_FG: u32 = theme::diff::DEL;

/// Renders `task_id`'s Git pane -- branch/status bar, the changed-files
/// list (staged/unstaged/untracked, per `build_changed_items`'s ordering),
/// and the selected file's diff (or whole-file contents, `state.view_mode`)
/// below it.
///
/// TODO: Swift's "更新順" (most-recently-modified) and "全体" (whole-tree,
/// not just changed files) list modes, and the commit-graph tile, are not
/// ported -- this pane only shows the changed-files tree, matching the
/// lighter-weight fixed-pane design this module's doc comment settled on.
pub fn render_git_pane(
    task_id: &str,
    state: &GitPaneState,
    spec: &RenderSpec,
    cx: &mut Context<LaboLaboApp>,
) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .w(px(GIT_PANE_WIDTH))
        .h_full()
        .flex_shrink_0()
        .bg(rgb(PANEL_BG))
        .border_l_1()
        .border_color(rgb(BORDER_COLOR))
        .child(render_branch_bar(task_id, state, cx))
        .child(
            div()
                .h(relative(0.4))
                .flex_shrink_0()
                .overflow_hidden()
                .border_b_1()
                .border_color(rgb(BORDER_COLOR))
                .child(render_file_list(task_id, state, spec, cx)),
        )
        .child(
            div()
                .flex_1()
                .min_h(px(0.0))
                .overflow_hidden()
                .child(render_detail(task_id, state, spec, cx)),
        )
        .into_any_element()
}

fn render_branch_bar(
    task_id: &str,
    state: &GitPaneState,
    cx: &mut Context<LaboLaboApp>,
) -> impl IntoElement {
    let branch_text: SharedString = match &state.status {
        Some(status) => status
            .branch
            .clone()
            .unwrap_or_else(|| "-".to_string())
            .into(),
        None => t!("git.branch.loading").to_string().into(),
    };
    let ahead = state.status.as_ref().map(|s| s.ahead).unwrap_or(0);
    let behind = state.status.as_ref().map(|s| s.behind).unwrap_or(0);
    let dirty = state.status.as_ref().map(|s| s.is_dirty()).unwrap_or(false);

    let close_task_id = task_id.to_string();
    let promote_task_id = task_id.to_string();

    div()
        .flex()
        .flex_row()
        .items_center()
        .gap_2()
        .px_2()
        .h(px(30.0))
        .flex_shrink_0()
        .bg(rgb(HEADER_BG))
        .text_size(px(11.0))
        .text_color(rgb(theme::text::PRIMARY))
        .child(SharedString::from("\u{2387}")) // branch glyph, matches sidebar::kind_marker
        .child(branch_text)
        .when(ahead > 0, |el| el.child(format!("\u{2191}{ahead}")))
        .when(behind > 0, |el| el.child(format!("\u{2193}{behind}")))
        .when(dirty, |el| {
            el.child(
                div()
                    .text_color(rgb(theme::status::STARTING))
                    .child(SharedString::from("\u{25cf}")),
            )
        })
        .child(div().flex_1())
        .when_some(state.load_error.as_ref(), |el, err| {
            el.child(
                div()
                    .text_color(rgb(theme::status::CONFLICT))
                    .child(SharedString::from(err.clone())),
            )
        })
        .child(
            // `plans` W6d §3: "右固定ペインのヘッダに「タイルとして開く」
            // ボタン" -- moves this Task's Git state into the ordinary
            // tile tree (a `Files` + `Diff` pane, split off the current
            // layout) and hides this fixed pane, so it can then be
            // tabbed/split/dragged/persisted exactly like any other pane
            // (`LaboLaboApp::promote_git_pane_to_tiles`'s doc comment).
            div()
                .id("git-pane-promote-to-tiles")
                .px_1p5()
                .py_0p5()
                .rounded_sm()
                .text_color(rgb(theme::text::SECONDARY))
                .hover(|el| el.bg(rgb(theme::surface::ACTIVE)))
                .active(|el| el.opacity(0.8))
                .tooltip(move |_window, cx| {
                    cx.new(|_| {
                        crate::sidebar::IconTooltip(
                            t!("git.pane.promote_tooltip").to_string().into(),
                        )
                    })
                    .into()
                })
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                        this.promote_git_pane_to_tiles(&promote_task_id, window, cx);
                    }),
                )
                .child(SharedString::from("\u{25a6}")), // ▦ grid/tile glyph, plain Unicode
        )
        .child(
            div()
                .px_1()
                .text_color(rgb(theme::text::SECONDARY))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _: &MouseDownEvent, _window, cx| {
                        this.set_git_pane_visible(&close_task_id, false, cx);
                    }),
                )
                .child(SharedString::from("\u{d7}")),
        )
}

/// The changed-files list content (staged/unstaged/untracked rows) --
/// `pub(crate)` so `crate::task_workspace::render_leaf` can reuse it
/// verbatim as a `Files`-kind tile pane's body (`plans` W6d), the same
/// content the fixed pane's own [`render_git_pane`] already shows. Clicking
/// a row calls [`LaboLaboApp::select_git_file`], which mutates the shared
/// per-Task [`GitPaneState`] -- so a `Files` tile and a `Diff` tile (or the
/// fixed pane) always agree on "the selected file," wherever each is drawn.
pub(crate) fn render_file_list(
    task_id: &str,
    state: &GitPaneState,
    spec: &RenderSpec,
    cx: &mut Context<LaboLaboApp>,
) -> impl IntoElement {
    if state.items.is_empty() {
        return div()
            .p_2()
            .text_size(px(11.0))
            .text_color(rgb(theme::text::MUTED))
            .child(SharedString::from(t!("git.file_list.empty").to_string()))
            .into_any_element();
    }

    let mut list = div().flex().flex_col().overflow_hidden();
    for section in [
        FileSection::Staged,
        FileSection::Unstaged,
        FileSection::Untracked,
    ] {
        let rows: Vec<&ChangedFileItem> = state
            .items
            .iter()
            .filter(|item| item.section == section)
            .collect();
        if rows.is_empty() {
            continue;
        }
        list = list.child(
            div()
                .px_2()
                .pt_1()
                .text_size(px(theme::font_size::CAPTION))
                .text_color(rgb(section.badge_color()))
                .child(SharedString::from(format!(
                    "{} ({})",
                    section.label(),
                    rows.len()
                ))),
        );
        for item in rows {
            list = list.child(render_file_row(task_id, state, item, spec, cx));
        }
    }
    list.into_any_element()
}

fn render_file_row(
    task_id: &str,
    state: &GitPaneState,
    item: &ChangedFileItem,
    spec: &RenderSpec,
    cx: &mut Context<LaboLaboApp>,
) -> impl IntoElement {
    let is_selected = state.selected_path.as_deref() == Some(item.path.as_str());
    let file_name = item
        .path
        .rsplit('/')
        .next()
        .unwrap_or(item.path.as_str())
        .to_string();
    let click_task_id = task_id.to_string();
    let click_path = item.path.clone();

    div()
        .flex()
        .flex_row()
        .items_center()
        .gap_1()
        .px_2()
        .py_0p5()
        .text_size(px(11.0))
        .when(is_selected, |el| el.bg(rgb(SELECTED_ROW_BG)))
        .text_color(rgb(theme::text::PRIMARY))
        .child(
            div()
                .flex_1()
                .overflow_hidden()
                .child(SharedString::from(file_name)),
        )
        // `plans/013` §3: the +/- counts are set in the user's own
        // terminal font (`spec.font`) at `CAPTION` size, same "numbers
        // stay visually tied to the terminal" rationale as the tab chip's
        // usage label (`task_workspace::render_pane_tab_bar`).
        .when_some(item.adds.filter(|a| *a > 0), |el, adds| {
            el.child(
                div()
                    .font(spec.font.clone())
                    .text_size(px(theme::font_size::CAPTION))
                    .text_color(rgb(theme::diff::ADD))
                    .child(SharedString::from(format!("+{adds}"))),
            )
        })
        .when_some(item.dels.filter(|d| *d > 0), |el, dels| {
            el.child(
                div()
                    .font(spec.font.clone())
                    .text_size(px(theme::font_size::CAPTION))
                    .text_color(rgb(theme::diff::DEL))
                    .child(SharedString::from(format!("-{dels}"))),
            )
        })
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _: &MouseDownEvent, _window, cx| {
                this.select_git_file(&click_task_id, click_path.clone(), cx);
            }),
        )
}

/// The selected file's detail (header w/ Diff⇄Whole toggle + the diff or
/// whole-file content) -- `pub(crate)` so `crate::task_workspace::render_leaf`
/// can reuse it verbatim as a `Diff`-kind tile pane's body (`plans` W6d),
/// exactly what [`render_git_pane`] already shows below the fixed pane's own
/// file list. See [`render_file_list`]'s doc comment for how the two panes
/// stay in sync on "which file."
pub(crate) fn render_detail(
    task_id: &str,
    state: &GitPaneState,
    spec: &RenderSpec,
    cx: &mut Context<LaboLaboApp>,
) -> AnyElement {
    let Some(path) = state.selected_path.clone() else {
        return div()
            .p_2()
            .text_size(px(11.0))
            .text_color(rgb(theme::text::MUTED))
            .child(SharedString::from(
                t!("git.file_list.select_file_prompt").to_string(),
            ))
            .into_any_element();
    };

    div()
        .flex()
        .flex_col()
        .size_full()
        .overflow_hidden()
        .child(render_detail_header(task_id, state, &path, cx))
        .child(
            div()
                .flex_1()
                .min_h(px(0.0))
                .overflow_hidden()
                .child(match state.view_mode {
                    FileViewMode::Diff => render_diff(state.diff.as_ref(), spec),
                    FileViewMode::Whole => render_whole_text(state.whole_text.as_deref(), spec),
                }),
        )
        .into_any_element()
}

fn render_detail_header(
    task_id: &str,
    state: &GitPaneState,
    path: &str,
    cx: &mut Context<LaboLaboApp>,
) -> impl IntoElement {
    let diff_task_id = task_id.to_string();
    let whole_task_id = task_id.to_string();

    div()
        .flex()
        .flex_row()
        .items_center()
        .gap_2()
        .px_2()
        .h(px(26.0))
        .flex_shrink_0()
        .bg(rgb(HEADER_BG))
        .text_size(px(11.0))
        .child(
            div()
                .flex_1()
                .overflow_hidden()
                .text_color(rgb(theme::text::SECONDARY))
                .child(SharedString::from(path.to_string())),
        )
        .child(render_mode_pill(
            t!("git.detail.mode_diff").to_string(),
            state.view_mode == FileViewMode::Diff,
            cx.listener(move |this, _: &MouseDownEvent, _window, cx| {
                this.set_git_view_mode(&diff_task_id, FileViewMode::Diff, cx);
            }),
        ))
        .child(render_mode_pill(
            t!("git.detail.mode_whole").to_string(),
            state.view_mode == FileViewMode::Whole,
            cx.listener(move |this, _: &MouseDownEvent, _window, cx| {
                this.set_git_view_mode(&whole_task_id, FileViewMode::Whole, cx);
            }),
        ))
}

fn render_mode_pill(
    label: String,
    active: bool,
    on_click: impl Fn(&MouseDownEvent, &mut gpui::Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    div()
        .px_1p5()
        .py_0p5()
        .rounded_sm()
        .text_size(px(10.0))
        // `plans/013`: the active Diff/Whole toggle pill is a *selection*,
        // not an "agent running" signal, so it uses `ACCENT` (shared with
        // the focused-pane border/selection highlight) rather than the
        // status green -- keeping the two greens from reading as the same
        // thing this plan's background section calls out as a pre-existing
        // problem ("緑が 2 系統混在").
        .when(active, |el| {
            el.bg(rgb(theme::ACCENT))
                .text_color(rgb(theme::text::ON_ACCENT))
        })
        .when(!active, |el| el.text_color(rgb(theme::text::SECONDARY)))
        .on_mouse_down(MouseButton::Left, on_click)
        .child(SharedString::from(label))
}

fn render_diff(diff: Option<&FileDiff>, spec: &RenderSpec) -> AnyElement {
    let Some(diff) = diff else {
        return placeholder(t!("git.detail.no_diff").to_string());
    };
    if diff.is_binary {
        return placeholder(t!("git.detail.binary_file").to_string());
    }
    if diff.hunks.is_empty() {
        return placeholder(t!("git.detail.no_diff").to_string());
    }

    let mut col = div().flex().flex_col().overflow_hidden();
    for hunk in &diff.hunks {
        col = col.child(
            div()
                .px_2()
                .py_0p5()
                .bg(rgb(HUNK_HEADER_BG))
                .text_size(px(10.0))
                .text_color(rgb(theme::text::SECONDARY))
                .child(SharedString::from(hunk.header.clone())),
        );
        for line in &hunk.lines {
            col = col.child(render_diff_line(line, spec));
        }
    }
    col.into_any_element()
}

// `spec.font.family` (`crate::render::RenderSpec::resolve`'s already-probed
// monospace family -- whatever the user's Ghostty `font-family` resolves
// to, or `RenderSpec`'s own platform fallback) rather than a hardcoded
// `"Menlo"` literal: Menlo only ships with macOS, so a Linux build with no
// gpui-visible font named "Menlo" would otherwise silently fall through to
// gpui's own generic fallback stack (`TextSystem::resolve_font`), which is
// **not** guaranteed to be monospace (e.g. "Cantarell"/"Noto Sans" on a
// GNOME/KDE desktop) -- line-number/diff-gutter columns would visibly
// misalign. Reusing the already-resolved terminal font keeps this pane
// visually consistent with the terminal grid on every platform.
fn render_diff_line(line: &DiffLine, spec: &RenderSpec) -> AnyElement {
    let (bg, fg, sign) = match line.kind {
        LineKind::Addition => (Some(ADDITION_BG), ADDITION_FG, "+"),
        LineKind::Deletion => (Some(DELETION_BG), DELETION_FG, "-"),
        LineKind::NoNewline => (None, theme::text::MUTED, "\\"),
        LineKind::Context => (None, theme::text::PRIMARY, " "),
    };

    let mut row = div()
        .flex()
        .flex_row()
        .px_2()
        .text_size(px(11.0))
        .font_family(spec.font.family.clone());
    if let Some(bg) = bg {
        row = row.bg(rgb(bg));
    }
    row.child(
        div()
            .w(px(28.0))
            .flex_shrink_0()
            .text_color(rgb(theme::text::MUTED))
            .child(SharedString::from(
                line.old_line_number
                    .map(|n| n.to_string())
                    .unwrap_or_default(),
            )),
    )
    .child(
        div()
            .w(px(28.0))
            .flex_shrink_0()
            .text_color(rgb(theme::text::MUTED))
            .child(SharedString::from(
                line.new_line_number
                    .map(|n| n.to_string())
                    .unwrap_or_default(),
            )),
    )
    .child(
        div()
            .w(px(12.0))
            .flex_shrink_0()
            .text_color(rgb(fg))
            .child(SharedString::from(sign)),
    )
    .child(
        div()
            .flex_1()
            .text_color(rgb(fg))
            .child(SharedString::from(if line.text.is_empty() {
                " ".to_string()
            } else {
                line.text.clone()
            })),
    )
    .into_any_element()
}

// See `render_diff_line`'s doc comment for why this reuses `spec.font`
// instead of hardcoding `"Menlo"`.
fn render_whole_text(text: Option<&str>, spec: &RenderSpec) -> AnyElement {
    let Some(text) = text else {
        return placeholder(t!("git.detail.unavailable").to_string());
    };
    if text.is_empty() {
        return placeholder(t!("git.detail.empty_file").to_string());
    }

    let mut col = div().flex().flex_col().overflow_hidden();
    for (index, line) in text.split('\n').enumerate() {
        col = col.child(
            div()
                .flex()
                .flex_row()
                .px_2()
                .text_size(px(11.0))
                .font_family(spec.font.family.clone())
                .child(
                    div()
                        .w(px(32.0))
                        .flex_shrink_0()
                        .text_color(rgb(theme::text::MUTED))
                        .child(SharedString::from((index + 1).to_string())),
                )
                .child(div().flex_1().text_color(rgb(theme::text::PRIMARY)).child(
                    SharedString::from(if line.is_empty() {
                        " ".to_string()
                    } else {
                        line.to_string()
                    }),
                )),
        );
    }
    col.into_any_element()
}

/// A muted, centered-in-nothing placeholder message (e.g. "No changes",
/// "Select a file") -- `pub(crate)` so `crate::commit_pane` can show the
/// same "nothing here yet" treatment for an empty commit list instead of
/// hand-rolling its own. Takes an owned `String` (not `&'static str`) since
/// wave 6f's callers build the text from `t!()`, locale-dependent at call
/// time.
pub(crate) fn placeholder(text: String) -> AnyElement {
    div()
        .p_2()
        .text_size(px(11.0))
        .text_color(rgb(theme::text::MUTED))
        .child(SharedString::from(text))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use labolabo_core::{Change, GitFileEntry};

    fn entry(kind: Kind, path: &str, index: Change, worktree: Change) -> GitFileEntry {
        let mut e = GitFileEntry::new(kind, path);
        e.index = index;
        e.worktree = worktree;
        e
    }

    fn numstat(path: &str, additions: i64, deletions: i64) -> NumstatEntry {
        NumstatEntry {
            additions: Some(additions),
            deletions: Some(deletions),
            path: path.to_string(),
        }
    }

    // MARK: - build_changed_items

    #[test]
    fn staged_unstaged_untracked_are_grouped_in_order() {
        let status = GitStatus {
            entries: vec![
                entry(
                    Kind::Ordinary,
                    "staged.txt",
                    Change::Modified,
                    Change::Unmodified,
                ),
                entry(
                    Kind::Ordinary,
                    "unstaged.txt",
                    Change::Unmodified,
                    Change::Modified,
                ),
                entry(
                    Kind::Untracked,
                    "new.txt",
                    Change::Unmodified,
                    Change::Unmodified,
                ),
            ],
            ..Default::default()
        };
        let items = build_changed_items(&status, &[], &[]);
        assert_eq!(
            items
                .iter()
                .map(|i| (i.path.as_str(), i.section))
                .collect::<Vec<_>>(),
            vec![
                ("staged.txt", FileSection::Staged),
                ("unstaged.txt", FileSection::Unstaged),
                ("new.txt", FileSection::Untracked),
            ]
        );
    }

    #[test]
    fn numstat_counts_are_matched_by_path_per_section() {
        let status = GitStatus {
            entries: vec![
                entry(
                    Kind::Ordinary,
                    "a.txt",
                    Change::Modified,
                    Change::Unmodified,
                ),
                entry(
                    Kind::Ordinary,
                    "a.txt",
                    Change::Unmodified,
                    Change::Modified,
                ),
            ],
            ..Default::default()
        };
        // Same path staged with different counts than unstaged -- the two
        // numstat lists must not cross-contaminate.
        let staged = vec![numstat("a.txt", 3, 1)];
        let unstaged = vec![numstat("a.txt", 7, 2)];
        let items = build_changed_items(&status, &staged, &unstaged);

        let staged_item = items
            .iter()
            .find(|i| i.section == FileSection::Staged)
            .unwrap();
        assert_eq!((staged_item.adds, staged_item.dels), (Some(3), Some(1)));

        let unstaged_item = items
            .iter()
            .find(|i| i.section == FileSection::Unstaged)
            .unwrap();
        assert_eq!((unstaged_item.adds, unstaged_item.dels), (Some(7), Some(2)));
    }

    #[test]
    fn unmerged_entries_are_excluded_from_the_unstaged_list() {
        let status = GitStatus {
            entries: vec![entry(
                Kind::Unmerged,
                "conflict.txt",
                Change::UpdatedButUnmerged,
                Change::UpdatedButUnmerged,
            )],
            ..Default::default()
        };
        let items = build_changed_items(&status, &[], &[]);
        assert!(
            items.is_empty(),
            "unmerged entries must not appear: {items:?}"
        );
    }

    #[test]
    fn untracked_entries_never_get_numstat_counts() {
        let status = GitStatus {
            entries: vec![entry(
                Kind::Untracked,
                "new.txt",
                Change::Unmodified,
                Change::Unmodified,
            )],
            ..Default::default()
        };
        // Even if a (bogus) numstat entry happened to share the path, an
        // untracked row must stay count-less -- matches Swift's `nil`.
        let items = build_changed_items(&status, &[numstat("new.txt", 5, 5)], &[]);
        assert_eq!(items[0].adds, None);
        assert_eq!(items[0].dels, None);
    }

    #[test]
    fn no_entries_yields_no_items() {
        assert!(build_changed_items(&GitStatus::default(), &[], &[]).is_empty());
    }

    // MARK: - changed_paths (cross-session conflict detection input)

    #[test]
    fn changed_paths_collects_staged_unstaged_and_untracked() {
        let status = GitStatus {
            entries: vec![
                entry(
                    Kind::Ordinary,
                    "staged.txt",
                    Change::Modified,
                    Change::Unmodified,
                ),
                entry(
                    Kind::Ordinary,
                    "unstaged.txt",
                    Change::Unmodified,
                    Change::Modified,
                ),
                entry(
                    Kind::Untracked,
                    "new.txt",
                    Change::Unmodified,
                    Change::Unmodified,
                ),
            ],
            ..Default::default()
        };
        let paths = changed_paths(&status);
        assert_eq!(
            paths,
            ["staged.txt", "unstaged.txt", "new.txt"]
                .into_iter()
                .map(String::from)
                .collect()
        );
    }

    #[test]
    fn changed_paths_excludes_ignored_entries() {
        let status = GitStatus {
            entries: vec![entry(
                Kind::Ignored,
                "target/",
                Change::Unmodified,
                Change::Unmodified,
            )],
            ..Default::default()
        };
        assert!(changed_paths(&status).is_empty());
    }

    #[test]
    fn changed_paths_includes_unmerged_entries_unlike_build_changed_items() {
        // Unlike `build_changed_items` (which drops conflicted paths from
        // the display list), `changed_paths` must keep them -- a conflict
        // is exactly the kind of overlap cross-session detection exists to
        // flag.
        let status = GitStatus {
            entries: vec![entry(
                Kind::Unmerged,
                "conflict.txt",
                Change::UpdatedButUnmerged,
                Change::UpdatedButUnmerged,
            )],
            ..Default::default()
        };
        assert_eq!(
            changed_paths(&status),
            ["conflict.txt"].into_iter().map(String::from).collect()
        );
    }

    #[test]
    fn changed_paths_includes_both_sides_of_a_rename() {
        let mut renamed = entry(
            Kind::RenamedOrCopied,
            "new_name.txt",
            Change::Renamed,
            Change::Unmodified,
        );
        renamed.original_path = Some("old_name.txt".to_string());
        let status = GitStatus {
            entries: vec![renamed],
            ..Default::default()
        };
        assert_eq!(
            changed_paths(&status),
            ["new_name.txt", "old_name.txt"]
                .into_iter()
                .map(String::from)
                .collect()
        );
    }

    #[test]
    fn changed_paths_of_empty_status_is_empty() {
        assert!(changed_paths(&GitStatus::default()).is_empty());
    }

    // MARK: - GitPaneState::select (default view-mode rule)

    fn state_with_items(items: Vec<ChangedFileItem>) -> GitPaneState {
        let mut state = GitPaneState::new();
        state.items = items;
        state
    }

    #[test]
    fn selecting_a_changed_tracked_file_defaults_to_diff_mode() {
        let mut state = state_with_items(vec![ChangedFileItem {
            path: "a.txt".into(),
            section: FileSection::Unstaged,
            adds: Some(1),
            dels: Some(1),
        }]);
        state.view_mode = FileViewMode::Whole;
        state.select("a.txt".to_string());
        assert_eq!(state.view_mode, FileViewMode::Diff);
        assert_eq!(state.selected_path.as_deref(), Some("a.txt"));
    }

    #[test]
    fn selecting_an_untracked_file_defaults_to_whole_mode() {
        let mut state = state_with_items(vec![ChangedFileItem {
            path: "new.txt".into(),
            section: FileSection::Untracked,
            adds: None,
            dels: None,
        }]);
        state.view_mode = FileViewMode::Diff;
        state.select("new.txt".to_string());
        assert_eq!(state.view_mode, FileViewMode::Whole);
    }

    #[test]
    fn selecting_a_path_not_in_items_defaults_to_whole_mode() {
        let mut state = GitPaneState::new();
        state.view_mode = FileViewMode::Diff;
        state.select("unknown.txt".to_string());
        assert_eq!(state.view_mode, FileViewMode::Whole);
    }

    // MARK: - refresh coalescing state machine

    #[test]
    fn first_begin_refresh_starts_immediately() {
        let mut state = GitPaneState::new();
        assert!(state.begin_refresh());
    }

    #[test]
    fn a_second_begin_refresh_while_in_flight_is_coalesced_not_started() {
        let mut state = GitPaneState::new();
        assert!(state.begin_refresh());
        assert!(
            !state.begin_refresh(),
            "second call must not start a new one"
        );
        assert!(!state.begin_refresh(), "third call stays coalesced too");
    }

    #[test]
    fn finish_refresh_with_no_pending_trigger_leaves_the_pane_idle() {
        let mut state = GitPaneState::new();
        state.begin_refresh();
        assert!(!state.finish_refresh());
        // Idle again -- a fresh trigger starts immediately.
        assert!(state.begin_refresh());
    }

    #[test]
    fn finish_refresh_with_a_pending_trigger_starts_exactly_one_more() {
        let mut state = GitPaneState::new();
        state.begin_refresh();
        state.begin_refresh(); // coalesced -> pending
        state.begin_refresh(); // still coalesced
        assert!(
            state.finish_refresh(),
            "a pending trigger must start one more refresh"
        );
        // That one more refresh is now "in flight." With nothing new
        // triggered during it, finishing it goes idle...
        assert!(!state.finish_refresh());
        // ...and a fresh trigger after that starts immediately again.
        assert!(state.begin_refresh());
    }

    fn commit_row(subject: &str) -> CommitGraphRow {
        CommitGraphRow {
            id: 0,
            commit: labolabo_core::Commit {
                hash: "abc1234".to_string(),
                subject: subject.to_string(),
                author: "Alice".to_string(),
                date: Some(1_700_000_000),
                refs: String::new(),
            },
            node_lane: 0,
            edges: Vec::new(),
        }
    }

    #[test]
    fn apply_sets_all_snapshot_fields() {
        let mut state = GitPaneState::new();
        let snapshot = GitSnapshot {
            status: Some(GitStatus::default()),
            items: vec![ChangedFileItem {
                path: "a.txt".into(),
                section: FileSection::Staged,
                adds: Some(1),
                dels: Some(0),
            }],
            diff: None,
            whole_text: Some("hello".to_string()),
            commits: vec![commit_row("init")],
            load_error: None,
        };
        state.apply(snapshot);
        assert!(state.status.is_some());
        assert_eq!(state.items.len(), 1);
        assert_eq!(state.whole_text.as_deref(), Some("hello"));
        assert_eq!(state.commits.len(), 1);
        assert_eq!(state.commits[0].commit.subject, "init");
        assert!(state.load_error.is_none());
    }

    #[test]
    fn apply_with_a_load_error_clears_stale_content() {
        let mut state = GitPaneState::new();
        state.items = vec![ChangedFileItem {
            path: "a.txt".into(),
            section: FileSection::Staged,
            adds: None,
            dels: None,
        }];
        state.commits = vec![commit_row("stale")];
        state.apply(GitSnapshot {
            status: None,
            items: Vec::new(),
            diff: None,
            whole_text: None,
            commits: Vec::new(),
            load_error: Some("not a git repository".to_string()),
        });
        assert!(state.items.is_empty());
        assert!(state.commits.is_empty());
        assert_eq!(state.load_error.as_deref(), Some("not a git repository"));
    }

    // MARK: - compute_git_snapshot (real `git`, real temp repo)

    // `#[cfg(unix)]` because it spawns the real `git` binary through
    // `GitEngine`, which resolves it via `labolabo_core::ToolLocator` --
    // `#[cfg(not(unix))]` there is an `unimplemented!()` stub (see
    // `labolabo-core`'s `git_engine.rs` test module doc comment for the
    // same gate on the same grounds).
    #[cfg(unix)]
    fn scratch_repo() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "labolabo-git-pane-{}-{nanos}-{n}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let git = |args: &[&str]| {
            let status = std::process::Command::new("git")
                .args(args)
                .current_dir(&dir)
                .status()
                .expect("git must be on PATH for this test");
            assert!(status.success(), "git {args:?} failed");
        };
        git(&["init", "-b", "main"]);
        git(&["config", "user.email", "test@example.com"]);
        git(&["config", "user.name", "LaboLabo Test"]);
        std::fs::write(dir.join("committed.txt"), "one\ntwo\nthree\n").unwrap();
        git(&["add", "."]);
        git(&["-c", "commit.gpgsign=false", "commit", "-m", "init"]);

        // Now dirty the worktree the way a real Task's directory would be:
        // one staged change, one unstaged change, one untracked file.
        std::fs::write(dir.join("committed.txt"), "one\ntwo changed\nthree\n").unwrap();
        git(&["add", "committed.txt"]);
        std::fs::write(dir.join("committed.txt"), "one\ntwo changed\nthree\nfour\n").unwrap();
        std::fs::write(dir.join("untracked.txt"), "new file\n").unwrap();

        dir
    }

    #[cfg(unix)]
    #[test]
    fn compute_git_snapshot_reflects_a_real_repos_staged_unstaged_and_untracked_state() {
        let repo = scratch_repo();

        let snapshot = compute_git_snapshot(&repo, None);
        assert!(snapshot.load_error.is_none(), "{:?}", snapshot.load_error);
        assert!(snapshot.status.is_some());
        assert_eq!(
            snapshot.status.as_ref().unwrap().branch.as_deref(),
            Some("main")
        );
        // One path, dirty on both the staged and unstaged side -> two rows;
        // plus the untracked file -> three rows total.
        assert_eq!(snapshot.items.len(), 3);
        assert!(snapshot
            .items
            .iter()
            .any(|i| i.path == "committed.txt" && i.section == FileSection::Staged));
        assert!(snapshot
            .items
            .iter()
            .any(|i| i.path == "committed.txt" && i.section == FileSection::Unstaged));
        assert!(snapshot
            .items
            .iter()
            .any(|i| i.path == "untracked.txt" && i.section == FileSection::Untracked));
        // `commit_graph` is fetched alongside status/numstat on every
        // refresh -- the one commit `scratch_repo` made ("init").
        assert_eq!(snapshot.commits.len(), 1);
        assert_eq!(snapshot.commits[0].commit.subject, "init");

        // Selecting the unstaged copy of the file fetches both its diff
        // (against the index) and its current whole-file contents.
        let with_selection = compute_git_snapshot(&repo, Some("committed.txt"));
        let diff = with_selection
            .diff
            .expect("expected a diff for the unstaged file");
        assert!(!diff.hunks.is_empty());
        assert!(with_selection
            .whole_text
            .expect("expected whole-file contents")
            .contains("four"));

        std::fs::remove_dir_all(&repo).ok();
    }

    #[cfg(unix)]
    #[test]
    fn compute_git_snapshot_reports_a_load_error_outside_a_git_repository() {
        let dir = std::env::temp_dir().join(format!(
            "labolabo-git-pane-not-a-repo-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let snapshot = compute_git_snapshot(&dir, None);
        assert!(snapshot.load_error.is_some());
        assert!(snapshot.items.is_empty());
        assert!(snapshot.status.is_none());

        std::fs::remove_dir_all(&dir).ok();
    }
}
