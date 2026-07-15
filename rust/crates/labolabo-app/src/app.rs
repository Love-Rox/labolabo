//! The gpui root view: the Task sidebar, the selected Task's tile/tab tree
//! (`labolabo_core::tiling::PaneTilingModel`, one per loaded Task), key/click
//! routing, and Task persistence.
//!
//! ## Wave 5b-3: one window, many Tasks (`plans/012-task-model-and-control-
//! cli.md` §1)
//!
//! Wave 5b-2's `TerminalApp` owned exactly one [`PaneTilingModel`] + one
//! `PaneRuntime` map for the whole window (see that wave's doc comment,
//! still accurate for what it describes -- it's the shape this wave
//! generalizes). This wave replaces that with the plan's Task model: **one
//! Task owns one [`crate::task_workspace::TaskWorkspace`]** (the same
//! `PaneTilingModel` + per-pane `Terminal` runtimes, just Task-scoped
//! instead of window-scoped), a left sidebar lists every Task grouped by
//! repo (`crate::sidebar`), and clicking a Task switches which
//! `TaskWorkspace` is rendered/receives keyboard input -- exactly wave
//! 5b-2's tab-switch semantics ("hidden" Tasks' ptys/scrollback stay alive
//! in [`LaboLaboApp::workspaces`]), just one level up the hierarchy.
//!
//! Tasks and their `TileLayout` are persisted to a new, Rust-only SQLite
//! database (`labolabo_core::store::TaskDatabase` -- see its module doc
//! comment for the on-disk location and why it's a separate file from the
//! Swift app's `SessionDatabase`). On launch, every `Active` Task is
//! restored (`ensure_workspace_loaded` for the previously selected one,
//! others lazily on first selection); the layout (split/tab structure +
//! each leaf's selected tab) is restored from `TileLayout`, and every
//! `terminal`-kind pane in it gets a **fresh** shell spawned in the Task's
//! working directory (`Task::working_directory`) -- restoring the
//! *container*, not terminal scrollback/agent-session content (that's
//! future hooks-integration work, same caveat wave 5b-2 already carried for
//! the single-Task case).
//!
//! **Persistence timing**: every action handler that can change a Task's
//! layout (add tab / split / close / select tab) calls
//! [`LaboLaboApp::persist_workspace`] synchronously afterward, which
//! snapshots the Task's current `TileLayout` and upserts the Task row. This
//! is simpler than the plan's parenthetical example ("revision 変化で
//! debounce 保存") -- there's no separate dirty-flag/timer, every layout-
//! affecting action just re-saves immediately (a single cheap SQLite
//! upsert) -- but satisfies the same requirement ("変更時に随時"). Selecting
//! a Task also re-saves (`selectedTask` app-state key) so a restart resumes
//! on the same Task.
//!
//! **Out of this wave's scope** (per `plans/012` §1, and this wave's
//! brief): Task rename/done/archive, DnD reordering (plan §3), the control
//! CLI (plan §2), and restoring *which* pane had keyboard focus (only the
//! tile/tab structure round-trips -- a freshly restored Task's focus
//! defaults to its tree's first leaf's selected tab, see
//! `TaskWorkspace::new`'s doc comment).

use std::collections::HashMap;
use std::path::Path;

use gpui::{
    actions, div, prelude::*, rgb, Context, FocusHandle, IntoElement, KeyDownEvent,
    PathPromptOptions, Render, Window,
};

use labolabo_core::{
    PaneId, PaneItem, PaneKind, PaneTilingModel, Task, TaskDatabase, TaskStatus, TileNode,
    TileOrientation,
};
use labolabo_term::{ColorScheme, Terminal};

use crate::focus;
use crate::ghostty_config::FontConfig;
use crate::grid;
use crate::keys::keystroke_to_bytes;
use crate::new_task;
use crate::render::RenderSpec;
use crate::sidebar;
use crate::task_workspace::{self, TaskWorkspace};

/// Initial grid size for a pane created after startup with no viewport to
/// measure yet (new tab / split within an already-rendered Task, or the
/// single terminal pane of a freshly created Task) -- a conventional
/// terminal default, immediately corrected by the pane's own canvas once
/// gpui lays it out (see `task_workspace::render_leaf`'s doc comment).
/// Unlike wave 5b-2 (one Task, sized from the window viewport once at
/// startup), a Task-switching app has many "first pane" moments -- every
/// one of them except the very first (`LaboLaboApp::new`) and a lazy-load
/// on selection (`select_task`, which *does* have a `Window` to measure)
/// uses this default instead.
const DEFAULT_PANE_COLS: u16 = 80;
const DEFAULT_PANE_ROWS: u16 = 24;

actions!(
    labolabo_app,
    [
        NewTab,
        CloseTab,
        SplitRight,
        SplitDown,
        FocusNextPane,
        FocusPrevPane,
        SelectTab1,
        SelectTab2,
        SelectTab3,
        SelectTab4,
        SelectTab5,
        SelectTab6,
        SelectTab7,
        SelectTab8,
        SelectTab9,
    ]
);

pub struct LaboLaboApp {
    db: TaskDatabase,
    /// Every `Active` Task, ordered by `sort_order` -- the sidebar's source
    /// order (`sidebar::group_tasks_by_repo` groups without re-sorting).
    tasks: Vec<Task>,
    /// Loaded workspaces, keyed by `Task::id`. A Task appears here once it
    /// has ever been selected (or was the restored selection at launch);
    /// entries are never removed for the app's lifetime, so switching away
    /// from a Task keeps its ptys alive -- see this module's doc comment.
    workspaces: HashMap<String, TaskWorkspace>,
    selected_task_id: Option<String>,
    focus_handle: FocusHandle,
    spec: RenderSpec,
    /// The user's Ghostty color configuration, applied to every pane's
    /// `Terminal` at spawn time -- stored so panes created after startup
    /// (new tab, split, new Task) get it too, same as wave 5b-2.
    colors: ColorScheme,
    /// Last "new Task" flow's failure, if any (e.g. the picked directory
    /// isn't a git repository for the worktree flow) -- shown as a small
    /// banner under the sidebar's "+" row. Cleared at the start of the next
    /// attempt. There is no success-path banner; a successful flow just
    /// selects the new Task, which is feedback enough.
    new_task_error: Option<String>,
}

impl LaboLaboApp {
    pub fn new(
        font_config: &FontConfig,
        color_config: &ColorScheme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let spec = RenderSpec::resolve(
            &font_config.families,
            font_config
                .size
                .unwrap_or_else(crate::ghostty_config::default_font_size),
            window,
        );

        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle);

        let db = TaskDatabase::open(&TaskDatabase::default_path()).unwrap_or_else(|err| {
            eprintln!(
                "labolabo-app: failed to open the task database ({err}); \
                 falling back to an in-memory database for this run (nothing will persist)"
            );
            TaskDatabase::open_in_memory().expect("in-memory sqlite must always succeed")
        });
        let tasks: Vec<Task> = db
            .all_tasks()
            .unwrap_or_else(|err| {
                eprintln!(
                    "labolabo-app: failed to load tasks ({err}); starting with an empty list"
                );
                Vec::new()
            })
            .into_iter()
            .filter(|t| t.status == TaskStatus::Active)
            .collect();

        let selected_task_id = db
            .selected_task_id()
            .ok()
            .flatten()
            .filter(|id| tasks.iter().any(|t| &t.id == id))
            .or_else(|| tasks.first().map(|t| t.id.clone()));

        let mut this = Self {
            db,
            tasks,
            workspaces: HashMap::new(),
            selected_task_id: selected_task_id.clone(),
            focus_handle,
            spec,
            colors: color_config.clone(),
            new_task_error: None,
        };

        if let Some(id) = selected_task_id {
            let (cols, rows) = this.viewport_grid_size(window);
            this.ensure_workspace_loaded(&id, cols, rows, cx);
        }

        cx.observe_window_bounds(window, |_this, _window, cx| {
            cx.notify();
        })
        .detach();

        this
    }

    // MARK: - read-only accessors (for `sidebar::render`)

    pub(crate) fn tasks(&self) -> &[Task] {
        &self.tasks
    }

    pub(crate) fn selected_task_id(&self) -> Option<&str> {
        self.selected_task_id.as_deref()
    }

    pub(crate) fn new_task_error(&self) -> Option<&str> {
        self.new_task_error.as_deref()
    }

    pub(crate) fn focus_handle(&self) -> &FocusHandle {
        &self.focus_handle
    }

    /// The terminal grid size for the window's current viewport (full
    /// window, minus the sidebar and the pane's own tab bar). Used only
    /// where a `Window` is actually on hand (startup, and selecting a
    /// not-yet-loaded Task) -- every other newly spawned pane (new tab,
    /// split, a freshly created Task) uses [`DEFAULT_PANE_COLS`]/
    /// [`DEFAULT_PANE_ROWS`] instead, same reasoning wave 5b-2 documented
    /// for its single-Task case.
    fn viewport_grid_size(&self, window: &Window) -> (u16, u16) {
        let size = window.viewport_size();
        let sidebar_adjusted_width = (f32::from(size.width) - sidebar::SIDEBAR_WIDTH).max(0.0);
        grid::grid_size_for_window(
            sidebar_adjusted_width,
            size.height.into(),
            self.spec.cell_width,
            self.spec.cell_height,
        )
    }

    // MARK: - Task loading / persistence

    /// Loads `task_id`'s [`TaskWorkspace`] into `self.workspaces` if it
    /// isn't there already (a no-op otherwise -- switching back to an
    /// already-loaded Task never re-spawns anything, matching the plan's
    /// "表示中でない Task の...pty はメモリ上に温存" semantics): decodes its
    /// persisted `TileLayout` (falling back to a single fresh terminal pane
    /// if the layout is missing/corrupt), then spawns a real `Terminal`
    /// session at `(cols, rows)` for every `terminal`-kind pane in the
    /// restored tree, in the Task's working directory.
    fn ensure_workspace_loaded(
        &mut self,
        task_id: &str,
        cols: u16,
        rows: u16,
        cx: &mut Context<Self>,
    ) {
        if self.workspaces.contains_key(task_id) {
            return;
        }
        let Some(task) = self.tasks.iter().find(|t| t.id == task_id) else {
            return;
        };

        let model = PaneTilingModel::model_from(&task.layout).unwrap_or_else(|| {
            let pane = PaneItem::new(PaneKind::Terminal, PaneKind::Terminal.default_title());
            PaneTilingModel::new(TileNode::leaf(pane))
        });
        let pane_ids: Vec<PaneId> = model
            .panes()
            .iter()
            .filter(|p| p.kind == PaneKind::Terminal)
            .map(|p| p.id)
            .collect();

        self.workspaces
            .insert(task_id.to_string(), TaskWorkspace::new(model));

        for pane_id in pane_ids {
            self.spawn_runtime_for_task(task_id, pane_id, cols, rows, cx);
        }
    }

    /// Spawns a new `terminal`-kind pane's session (a fresh login-shell
    /// `Terminal`, started in `task_id`'s working directory) and registers
    /// its redraw bridge. No-op (with a stderr warning) if the spawn itself
    /// fails, or if `task_id` has no loaded workspace to register into --
    /// mirrors wave 5a/5b-2's `spawn_runtime`.
    fn spawn_runtime_for_task(
        &mut self,
        task_id: &str,
        pane_id: PaneId,
        cols: u16,
        rows: u16,
        cx: &mut Context<Self>,
    ) {
        let Some(task) = self.tasks.iter().find(|t| t.id == task_id) else {
            return;
        };
        let cwd = task.working_directory().to_string();
        let colors = self.colors.clone();

        let session = match Terminal::spawn_with_cwd_options(
            cols,
            rows,
            None,
            &[],
            &colors,
            Some(Path::new(&cwd)),
        ) {
            Ok(session) => std::sync::Arc::new(session),
            Err(err) => {
                eprintln!(
                    "labolabo-app: failed to spawn terminal session for task {task_id}: {err:#}"
                );
                return;
            }
        };

        let redraw_task =
            task_workspace::spawn_redraw_bridge(session.clone(), task_id.to_string(), pane_id, cx);
        if let Some(workspace) = self.workspaces.get_mut(task_id) {
            task_workspace::insert_runtime(
                &mut workspace.runtimes,
                pane_id,
                session,
                cols,
                rows,
                redraw_task,
            );
        }
    }

    /// Snapshots `task_id`'s current `TileLayout` and upserts its Task row
    /// -- see this module's doc comment for the "save on every layout-
    /// affecting action" timing. A no-op if `task_id` has no loaded
    /// workspace (shouldn't happen given callers only call this right after
    /// mutating that workspace) or isn't a known Task.
    fn persist_workspace(&mut self, task_id: &str) {
        let Some(workspace) = self.workspaces.get(task_id) else {
            return;
        };
        let layout = workspace.model.snapshot();
        if let Some(task) = self.tasks.iter_mut().find(|t| t.id == task_id) {
            task.layout = layout;
            task.last_active_at = chrono::Utc::now();
            if let Err(err) = self.db.upsert_task(task) {
                eprintln!("labolabo-app: failed to persist task {task_id}: {err}");
            }
        }
    }

    // MARK: - Task selection

    /// Switches the selected Task, loading its workspace first if this is
    /// the first time it's been selected. Persists the selection
    /// (`selectedTask` app-state key) so a restart resumes here.
    pub(crate) fn select_task(
        &mut self,
        task_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected_task_id.as_deref() == Some(task_id.as_str()) {
            return;
        }
        let (cols, rows) = self.viewport_grid_size(window);
        self.ensure_workspace_loaded(&task_id, cols, rows, cx);

        self.selected_task_id = Some(task_id.clone());
        if let Err(err) = self.db.set_selected_task_id(Some(&task_id)) {
            eprintln!("labolabo-app: failed to persist selected task: {err}");
        }
        if let Some(task) = self.tasks.iter_mut().find(|t| t.id == task_id) {
            task.last_active_at = chrono::Utc::now();
            let _ = self.db.upsert_task(task);
        }

        window.focus(&self.focus_handle);
        cx.notify();
    }

    // MARK: - focus / selection (within the selected Task's workspace)

    pub(crate) fn select_pane(
        &mut self,
        task_id: &str,
        pane_id: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(workspace) = self.workspaces.get_mut(task_id) {
            workspace.model.select_tab(pane_id);
            workspace.focused_pane = pane_id;
        }
        window.focus(&self.focus_handle);
        self.persist_workspace(task_id);
        cx.notify();
    }

    fn move_focus(
        &mut self,
        task_id: &str,
        forward: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.workspaces.get(task_id) else {
            return;
        };
        let Some(next) = focus::adjacent_pane(&workspace.model, workspace.focused_pane, forward)
        else {
            return;
        };
        if let Some(workspace) = self.workspaces.get_mut(task_id) {
            workspace.focused_pane = next;
        }
        window.focus(&self.focus_handle);
        cx.notify();
    }

    fn select_tab_index(
        &mut self,
        task_id: &str,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.workspaces.get(task_id) else {
            return;
        };
        if let Some(pane_id) = focus::nth_tab(&workspace.model, workspace.focused_pane, index) {
            self.select_pane(task_id, pane_id, window, cx);
        }
    }

    // MARK: - mutations (within the selected Task's workspace)

    pub(crate) fn add_tab_to(
        &mut self,
        task_id: &str,
        anchor_pane_id: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pane = PaneItem::new(PaneKind::Terminal, PaneKind::Terminal.default_title());
        let new_id = pane.id;
        let added = self
            .workspaces
            .get_mut(task_id)
            .map(|workspace| workspace.model.add_tab(anchor_pane_id, pane))
            .unwrap_or(false);
        if !added {
            return;
        }
        self.spawn_runtime_for_task(task_id, new_id, DEFAULT_PANE_COLS, DEFAULT_PANE_ROWS, cx);
        if let Some(workspace) = self.workspaces.get_mut(task_id) {
            workspace.focused_pane = new_id;
        }
        window.focus(&self.focus_handle);
        self.persist_workspace(task_id);
        cx.notify();
    }

    fn split_focused(
        &mut self,
        task_id: &str,
        orientation: TileOrientation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.workspaces.get(task_id) else {
            return;
        };
        let focused = workspace.focused_pane;
        if workspace.model.root.find_leaf(focused).is_none() {
            return;
        }
        let pane = PaneItem::new(PaneKind::Terminal, PaneKind::Terminal.default_title());
        let new_id = pane.id;
        if let Some(workspace) = self.workspaces.get_mut(task_id) {
            workspace.model.split(focused, orientation, pane);
        }
        self.spawn_runtime_for_task(task_id, new_id, DEFAULT_PANE_COLS, DEFAULT_PANE_ROWS, cx);
        if let Some(workspace) = self.workspaces.get_mut(task_id) {
            workspace.focused_pane = new_id;
        }
        window.focus(&self.focus_handle);
        self.persist_workspace(task_id);
        cx.notify();
    }

    pub(crate) fn close_pane_user(
        &mut self,
        task_id: &str,
        pane_id: PaneId,
        cx: &mut Context<Self>,
    ) {
        self.remove_pane(task_id, pane_id, true, cx);
    }

    pub(crate) fn handle_pane_exit(
        &mut self,
        task_id: &str,
        pane_id: PaneId,
        cx: &mut Context<Self>,
    ) {
        self.remove_pane(task_id, pane_id, false, cx);
    }

    /// Removes `pane_id` from `task_id`'s tree.
    ///
    /// A Task's **last** pane is special -- Task lifecycle (done/archive/
    /// delete) is out of this wave's scope (`plans/012` §1), so a Task must
    /// never end up pane-less-and-unrecoverable:
    ///
    /// - If it's also the app's only Task, this mirrors wave 5b-2's
    ///   pre-Task-model behavior and quits (Ghostty's close-last-surface
    ///   convention).
    /// - A **user** close (`shutdown_child: true` -- "x"/Cmd+W) is refused
    ///   outright, *before* touching the runtime, so the shell keeps
    ///   running untouched.
    /// - A **natural exit** (the shell already died) can't be refused: the
    ///   dead runtime is dropped but the pane stays in the tree (rendering
    ///   an empty canvas). The Task stays recoverable -- the pane's id is
    ///   still valid as an anchor, so its "+"/Cmd+T opens a fresh tab
    ///   (spawned in the Task's cwd as usual), after which the dead tab can
    ///   be closed normally. Auto-respawning a fresh shell into the dead
    ///   pane was deliberately not done: an immediately-exiting shell (bad
    ///   `$SHELL`, broken rc file) would respawn-loop with no way to stop.
    fn remove_pane(
        &mut self,
        task_id: &str,
        pane_id: PaneId,
        shutdown_child: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.workspaces.get(task_id) else {
            return;
        };
        if workspace.model.root.find_leaf(pane_id).is_none() {
            return;
        }
        let is_last_pane_in_task = workspace.model.panes().len() == 1;
        let was_focused = workspace.focused_pane == pane_id;

        if is_last_pane_in_task {
            if self.tasks.len() == 1 {
                // Quit path: tear the runtime down (signaling the child on a
                // user-driven close) and quit, like wave 5b-2.
                if let Some(workspace) = self.workspaces.get_mut(task_id) {
                    if let Some(runtime) = workspace.runtimes.remove(&pane_id) {
                        if shutdown_child {
                            runtime.session.shutdown();
                        }
                    }
                }
                cx.quit();
                return;
            }
            if shutdown_child {
                // User close of a Task's last pane: refused (see doc
                // comment). The shell was not signaled and keeps running.
                return;
            }
            // Natural exit of a Task's last pane's shell: drop the dead
            // runtime, keep the pane as a recoverable anchor.
            if let Some(workspace) = self.workspaces.get_mut(task_id) {
                workspace.runtimes.remove(&pane_id);
            }
            cx.notify();
            return;
        }

        if let Some(workspace) = self.workspaces.get_mut(task_id) {
            if let Some(runtime) = workspace.runtimes.remove(&pane_id) {
                if shutdown_child {
                    runtime.session.shutdown();
                }
                // `runtime` (and its `_redraw_task`) drops here, ending the
                // bridge thread.
            }
        }

        let revealed = self
            .workspaces
            .get_mut(task_id)
            .map(|workspace| workspace.model.close(pane_id))
            .unwrap_or(None);
        if was_focused {
            if let Some(workspace) = self.workspaces.get(task_id) {
                if let Some(new_focus) = focus::resolve_close_focus(&workspace.model, revealed) {
                    if let Some(workspace) = self.workspaces.get_mut(task_id) {
                        workspace.focused_pane = new_focus;
                    }
                }
            }
        }
        self.persist_workspace(task_id);
        cx.notify();
    }

    // MARK: - new Task flows (`plans/012` §1's "作業の開始（主 CTA）")

    /// "+ Attached": picks a directory via gpui's native OS directory
    /// picker (`cx.prompt_for_paths` -- this crate's `Path Prompt` gpui API
    /// is the "gpui のネイティブパス選択" the plan asks for) and starts an
    /// `attached`-kind Task there, no worktree created.
    pub(crate) fn start_new_attached_task(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.new_task_error = None;
        let options = PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Attach as a new task".into()),
        };
        let rx = cx.prompt_for_paths(options);
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(mut paths))) = rx.await else {
                return;
            };
            let Some(dir) = paths.pop() else {
                return;
            };
            let (repo_key, repo_root, repo_name) = cx
                .background_spawn(async move { new_task::resolve_attached_repo(&dir.clone()) })
                .await;
            let _ = this.update(cx, |app, cx| {
                app.finish_new_attached_task(repo_key, repo_root, repo_name, cx)
            });
        })
        .detach();
    }

    fn finish_new_attached_task(
        &mut self,
        repo_key: String,
        repo_root: String,
        repo_name: String,
        cx: &mut Context<Self>,
    ) {
        // `resolve_attached_repo`'s no-repo fallback sets `repo_root` to the
        // directory itself, which doubles as the attached directory here.
        let directory = repo_root.clone();
        let layout = single_terminal_layout();
        let sort_order = self.next_sort_order();
        let task = Task::new_attached(
            repo_key, repo_root, repo_name, directory, layout, sort_order,
        );
        self.add_task_and_select(task, cx);
    }

    /// "+ Worktree": picks an existing repository checkout via the same
    /// native directory picker, generates a fresh branch name
    /// (`new_task::create_worktree_task`), runs `git worktree add`, and
    /// starts a `worktree`-kind Task there.
    ///
    /// This wave has no persistent "registered repositories" list to pick
    /// from (that's future work -- see this module's scope note and the
    /// task brief's allowance for a minimal "+"-menu flow); the directory
    /// picker doubles as ad hoc repo selection, resolved fresh via
    /// `GitEngine::repo_info` every time.
    pub(crate) fn start_new_worktree_task(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.new_task_error = None;
        let options = PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose a repository for the new worktree task".into()),
        };
        let rx = cx.prompt_for_paths(options);
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(mut paths))) = rx.await else {
                return;
            };
            let Some(repo_path) = paths.pop() else {
                return;
            };
            let outcome = cx
                .background_spawn(async move { new_task::create_worktree_task(&repo_path) })
                .await;
            match outcome {
                Ok(prepared) => {
                    let _ = this.update(cx, |app, cx| app.finish_new_worktree_task(prepared, cx));
                }
                Err(message) => {
                    let _ = this.update(cx, |app, cx| {
                        app.new_task_error = Some(message);
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    fn finish_new_worktree_task(
        &mut self,
        prepared: new_task::PreparedWorktree,
        cx: &mut Context<Self>,
    ) {
        let layout = single_terminal_layout();
        let sort_order = self.next_sort_order();
        let task = Task::new_worktree(
            prepared.repo_key,
            prepared.repo_root,
            prepared.repo_name,
            prepared.branch,
            prepared.base,
            prepared.worktree_path,
            layout,
            sort_order,
        );
        self.add_task_and_select(task, cx);
    }

    fn next_sort_order(&self) -> i64 {
        self.db.next_sort_order().unwrap_or(self.tasks.len() as i64)
    }

    /// Persists `task`, appends it to `self.tasks`, loads its (single-pane)
    /// workspace, and selects it.
    fn add_task_and_select(&mut self, task: Task, cx: &mut Context<Self>) {
        if let Err(err) = self.db.upsert_task(&task) {
            eprintln!("labolabo-app: failed to persist new task: {err}");
        }
        let id = task.id.clone();
        self.tasks.push(task);
        self.ensure_workspace_loaded(&id, DEFAULT_PANE_COLS, DEFAULT_PANE_ROWS, cx);
        self.selected_task_id = Some(id.clone());
        let _ = self.db.set_selected_task_id(Some(&id));
        cx.notify();
    }

    // MARK: - input routing

    fn key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, _cx: &mut Context<Self>) {
        let Some(task_id) = self.selected_task_id.as_deref() else {
            return;
        };
        let Some(workspace) = self.workspaces.get(task_id) else {
            return;
        };
        let Some(runtime) = workspace.runtimes.get(&workspace.focused_pane) else {
            return;
        };
        // TODO(W5a): IME composition is not wired up here -- see
        // `keys::keystroke_to_bytes`'s module doc comment.
        if let Some(bytes) = keystroke_to_bytes(&event.keystroke) {
            runtime.session.write_input(&bytes);
        }
    }

    // MARK: - action handlers (see the `actions!` list + main.rs's `bind_keys`)

    fn selected_task_and_focused_pane(&self) -> Option<(String, PaneId)> {
        let task_id = self.selected_task_id.clone()?;
        let focused = self.workspaces.get(&task_id)?.focused_pane;
        Some((task_id, focused))
    }

    fn action_new_tab(&mut self, _: &NewTab, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((task_id, anchor)) = self.selected_task_and_focused_pane() {
            self.add_tab_to(&task_id, anchor, window, cx);
        }
    }

    fn action_close_tab(&mut self, _: &CloseTab, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((task_id, pane)) = self.selected_task_and_focused_pane() {
            self.close_pane_user(&task_id, pane, cx);
            window.focus(&self.focus_handle);
        }
    }

    fn action_split_right(&mut self, _: &SplitRight, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(task_id) = self.selected_task_id.clone() {
            self.split_focused(&task_id, TileOrientation::Horizontal, window, cx);
        }
    }

    fn action_split_down(&mut self, _: &SplitDown, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(task_id) = self.selected_task_id.clone() {
            self.split_focused(&task_id, TileOrientation::Vertical, window, cx);
        }
    }

    fn action_focus_next_pane(
        &mut self,
        _: &FocusNextPane,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(task_id) = self.selected_task_id.clone() {
            self.move_focus(&task_id, true, window, cx);
        }
    }

    fn action_focus_prev_pane(
        &mut self,
        _: &FocusPrevPane,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(task_id) = self.selected_task_id.clone() {
            self.move_focus(&task_id, false, window, cx);
        }
    }

    fn action_select_tab_1(&mut self, _: &SelectTab1, window: &mut Window, cx: &mut Context<Self>) {
        self.action_select_tab_index(0, window, cx);
    }
    fn action_select_tab_2(&mut self, _: &SelectTab2, window: &mut Window, cx: &mut Context<Self>) {
        self.action_select_tab_index(1, window, cx);
    }
    fn action_select_tab_3(&mut self, _: &SelectTab3, window: &mut Window, cx: &mut Context<Self>) {
        self.action_select_tab_index(2, window, cx);
    }
    fn action_select_tab_4(&mut self, _: &SelectTab4, window: &mut Window, cx: &mut Context<Self>) {
        self.action_select_tab_index(3, window, cx);
    }
    fn action_select_tab_5(&mut self, _: &SelectTab5, window: &mut Window, cx: &mut Context<Self>) {
        self.action_select_tab_index(4, window, cx);
    }
    fn action_select_tab_6(&mut self, _: &SelectTab6, window: &mut Window, cx: &mut Context<Self>) {
        self.action_select_tab_index(5, window, cx);
    }
    fn action_select_tab_7(&mut self, _: &SelectTab7, window: &mut Window, cx: &mut Context<Self>) {
        self.action_select_tab_index(6, window, cx);
    }
    fn action_select_tab_8(&mut self, _: &SelectTab8, window: &mut Window, cx: &mut Context<Self>) {
        self.action_select_tab_index(7, window, cx);
    }
    fn action_select_tab_9(&mut self, _: &SelectTab9, window: &mut Window, cx: &mut Context<Self>) {
        self.action_select_tab_index(8, window, cx);
    }

    fn action_select_tab_index(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(task_id) = self.selected_task_id.clone() {
            self.select_tab_index(&task_id, index, window, cx);
        }
    }
}

/// A single fresh terminal pane's `TileLayout` -- the initial layout for
/// every newly created Task (both kinds).
fn single_terminal_layout() -> labolabo_core::TileLayout {
    let pane = PaneItem::new(PaneKind::Terminal, PaneKind::Terminal.default_title());
    PaneTilingModel::new(TileNode::leaf(pane)).snapshot()
}

impl Render for LaboLaboApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let sidebar_el = sidebar::render(self, cx);

        let workspace_el = if let Some(task_id) = self.selected_task_id.clone() {
            if let Some(workspace) = self.workspaces.get(&task_id) {
                let spec = self.spec.clone();
                let focused_pane = workspace.focused_pane;
                task_workspace::render_tile(
                    &task_id,
                    &workspace.model.root,
                    &workspace.runtimes,
                    focused_pane,
                    &spec,
                    cx,
                )
            } else {
                empty_state("Loading task...")
            }
        } else {
            empty_state("No task selected. Use \"+ Attached\" or \"+ Worktree\" to start one.")
        };

        div()
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::key_down))
            .on_action(cx.listener(Self::action_new_tab))
            .on_action(cx.listener(Self::action_close_tab))
            .on_action(cx.listener(Self::action_split_right))
            .on_action(cx.listener(Self::action_split_down))
            .on_action(cx.listener(Self::action_focus_next_pane))
            .on_action(cx.listener(Self::action_focus_prev_pane))
            .on_action(cx.listener(Self::action_select_tab_1))
            .on_action(cx.listener(Self::action_select_tab_2))
            .on_action(cx.listener(Self::action_select_tab_3))
            .on_action(cx.listener(Self::action_select_tab_4))
            .on_action(cx.listener(Self::action_select_tab_5))
            .on_action(cx.listener(Self::action_select_tab_6))
            .on_action(cx.listener(Self::action_select_tab_7))
            .on_action(cx.listener(Self::action_select_tab_8))
            .on_action(cx.listener(Self::action_select_tab_9))
            .flex()
            .flex_row()
            .size_full()
            .bg(rgb(0x000000))
            .child(sidebar_el)
            .child(workspace_el)
    }
}

fn empty_state(message: &'static str) -> gpui::AnyElement {
    div()
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .text_color(rgb(0x8a8a8a))
        .child(message)
        .into_any_element()
}
