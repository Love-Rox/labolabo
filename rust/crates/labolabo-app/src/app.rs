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
use std::ops::Range;
use std::path::Path;

use gpui::{
    actions, div, point, prelude::*, px, rgb, size, Bounds, Context, EntityInputHandler,
    FocusHandle, IntoElement, KeyDownEvent, PathPromptOptions, Pixels, Point, Render,
    Task as GpuiTask, UTF16Selection, Window,
};

use labolabo_core::{
    claude_resume_command, shell_quote, AgentBindings, AgentStatus, AgentStatusEvent,
    ControlCommand, ControlResponse, PaneId, PaneItem, PaneKind, PaneTilingModel, Task,
    TaskDatabase, TaskStatus, TileNode, TileOrientation,
};
use labolabo_term::{ColorScheme, Terminal};

use crate::control::{self, ControlRuntime};
use crate::focus;
use crate::ghostty_config::FontConfig;
use crate::grid;
use crate::hooks::{self, HookRuntime};
use crate::ime;
use crate::keys::keystroke_to_bytes;
use crate::new_task;
use crate::paste;
use crate::render::RenderSpec;
use crate::sidebar;
use crate::task_workspace::{self, PaneRuntime, TaskWorkspace};

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
        Paste,
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
    /// Claude Code hooks integration: the shared socket/bus, the forwarder
    /// binary path, `.claude/settings.local.json` injection bookkeeping, and
    /// the `LABOLABO_PANE` routing table -- see `crate::hooks`'s module doc
    /// comment.
    hooks: HookRuntime,
    /// Keeps the hooks-event bridge task alive for the app's lifetime (see
    /// `hooks::spawn_agent_event_bridge`); dropping it would stop event
    /// delivery.
    _agent_event_task: GpuiTask<()>,
    /// Control-protocol integration (`docs/control-protocol.md`,
    /// `plans/012` §2): the control socket/server -- see `crate::control`'s
    /// module doc comment. Its `socket_path` is injected into every spawned
    /// pane's env as `LABOLABO_CONTROL_SOCKET`.
    control: ControlRuntime,
    /// Keeps the control-bridge task alive for the app's lifetime (see
    /// `control::spawn_control_bridge`); dropping it would stop request
    /// delivery (every in-flight/future `labolabo` CLI request would time
    /// out instead of being answered).
    _control_bridge_task: GpuiTask<()>,
    /// The focused pane's live IME composition (preedit/marked) text, if
    /// any -- see this module's `EntityInputHandler` impl doc comment for
    /// the full IME design. `None` whenever no composition is in progress
    /// (the common case: plain typing never sets this).
    active_preedit: Option<PreeditState>,
}

/// The focused pane's in-progress IME composition, tracked by
/// `LaboLaboApp::active_preedit` and rendered inline by
/// `task_workspace::render_leaf` (via `render::paint_preedit`).
///
/// Tagged with the `(task, pane)` it belongs to -- not just carried as a
/// bare `String` -- so that if focus moves to a different pane mid-
/// composition without the platform ever calling `unmark_text` (an edge
/// case; the OS is expected to always clean up on focus loss, but this is
/// cheap insurance against a stale overlay leaking onto the wrong pane),
/// `task_workspace::render_leaf` simply stops rendering it for any pane
/// other than the one it was recorded against.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PreeditState {
    pub(crate) task_id: String,
    pub(crate) pane_id: PaneId,
    pub(crate) text: String,
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

        // Claude Code hooks integration (docs/hooks-protocol.md): one shared
        // socket/bus for the whole app process (see `hooks`'s module doc
        // comment for why, vs. Swift's one-per-session design), bridged into
        // gpui via an unbounded channel + a coalescing-free redraw-bridge-
        // style task (`hooks::spawn_agent_event_bridge`).
        let (hooks, hooks_rx) = HookRuntime::new();
        let agent_event_task = hooks::spawn_agent_event_bridge(hooks_rx, cx);

        // Control-protocol integration (docs/control-protocol.md,
        // `plans/012` §2): a second, separate socket/server (see
        // `crate::control`'s module doc comment for why this needs a
        // `WindowHandle`-routed bridge rather than the hooks bridge's plain
        // `WeakEntity` update). `window.window_handle()` is safe to call
        // here because this window's root view -- the one being constructed
        // right now -- is always `Self` (see `main.rs`'s `cx.open_window`
        // call, the only place a `LaboLaboApp` window is ever opened).
        let window_handle: gpui::WindowHandle<Self> =
            gpui::WindowHandle::new(window.window_handle().window_id());
        let (control_runtime, control_rx) = ControlRuntime::new();
        let control_bridge_task = control::spawn_control_bridge(control_rx, window_handle, cx);

        // Restore every injected directory's `settings.local.json` at quit
        // (docs/hooks-protocol.md §2's "終了時に原本へ復元"). `Context::
        // on_app_quit` (unlike the plain `gpui::App::on_app_quit`) hands the
        // closure `&mut LaboLaboApp` directly, so this can just call
        // `HookRuntime::restore_all` through `self` -- no separately shared
        // handle needed.
        cx.on_app_quit(|this, _cx| {
            this.hooks.restore_all();
            std::future::ready(())
        })
        .detach();

        let mut this = Self {
            db,
            tasks,
            workspaces: HashMap::new(),
            selected_task_id: selected_task_id.clone(),
            focus_handle,
            spec,
            colors: color_config.clone(),
            new_task_error: None,
            hooks,
            _agent_event_task: agent_event_task,
            control: control_runtime,
            _control_bridge_task: control_bridge_task,
            active_preedit: None,
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

        // Inject Claude Code hooks into this Task's working directory
        // before any pane spawns (idempotent per directory -- see
        // `HookRuntime::ensure_injected`'s doc comment).
        self.hooks
            .ensure_injected(Path::new(task.working_directory()));

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
            self.spawn_runtime_for_task(task_id, pane_id, cols, rows, None, cx);
        }
    }

    /// Spawns a new `terminal`-kind pane's session and registers its redraw
    /// bridge. No-op (with a stderr warning), returning `None`, if the spawn
    /// itself fails, or if `task_id` has no loaded workspace to register
    /// into -- mirrors wave 5a/5b-2's `spawn_runtime`. Returns `Some(pane_uuid)`
    /// on success -- the same `LABOLABO_PANE` value registered in the hooks
    /// routing table below, and (via `open_tab_for_control`) the control
    /// protocol's `tab_open` response's `pane_id` (docs/control-protocol.md
    /// §5.1).
    ///
    /// Hooks-integration additions over wave 5b-2/5b-3's plain shell spawn:
    ///
    /// - **Env injection** (docs/hooks-protocol.md §7,
    ///   docs/control-protocol.md §4.1): every spawned pane gets
    ///   `LABOLABO_PANE=<fresh UUID>`, `LABOLABO_TASK=<task_id>`, and
    ///   `LABOLABO_CONTROL_SOCKET=<this process's control socket path>` in
    ///   its environment, and the pane UUID is registered in `self.hooks`'
    ///   routing table so `handle_agent_event`/`dispatch_control`'s
    ///   `focus --pane` can route back to `(task_id, pane_id)`.
    /// - **Resume-at-spawn** (docs/hooks-protocol.md §6's resume guard,
    ///   `tiling::PaneItem::is_resumable`): if `override_command` is `None`
    ///   and the pane already carries a Claude session id from its
    ///   persisted `TileLayout` (a Task restored from the database -- see
    ///   `PaneTilingModel::model_from`; a freshly created pane never does),
    ///   and its recorded transcript path either doesn't exist or wasn't
    ///   recorded, spawn `claude --resume <id>` directly as the pane's
    ///   command instead of a plain shell -- this port's version of the
    ///   Swift app's `triggerAutoResumeIfNeeded` (which instead types the
    ///   resume command into an already-running shell after the fact;
    ///   spawning it directly is simpler here and avoids the "was the shell
    ///   ready yet" race that approach has to guard against).
    ///
    /// `override_command`, when `Some`, always wins over the resume-at-spawn
    /// command above -- the control protocol's `tab_open --` command
    /// (docs/control-protocol.md §5.1) is this wave's only caller that
    /// passes one; every other caller passes `None` (unchanged wave
    /// 5b-2/5b-3 behavior).
    fn spawn_runtime_for_task(
        &mut self,
        task_id: &str,
        pane_id: PaneId,
        cols: u16,
        rows: u16,
        override_command: Option<String>,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        let task = self.tasks.iter().find(|t| t.id == task_id)?;
        let cwd = task.working_directory().to_string();
        let colors = self.colors.clone();

        let pane_snapshot = self.workspaces.get(task_id).and_then(|workspace| {
            workspace
                .model
                .panes()
                .into_iter()
                .find(|p| p.id == pane_id)
                .cloned()
        });
        let command = override_command.or_else(|| {
            pane_snapshot.as_ref().and_then(|pane| {
                let transcript_exists = pane
                    .agent_transcript_path
                    .as_deref()
                    .map(|path| Path::new(path).exists())
                    .unwrap_or(false);
                pane.is_resumable(transcript_exists)
                    .then(|| claude_resume_command(pane.agent_session_id.as_deref()))
            })
        });

        let pane_uuid = uuid::Uuid::new_v4().to_string();
        let env = vec![
            ("LABOLABO_PANE".to_string(), pane_uuid.clone()),
            ("LABOLABO_TASK".to_string(), task_id.to_string()),
            (
                "LABOLABO_CONTROL_SOCKET".to_string(),
                self.control.socket_path.clone(),
            ),
        ];

        let session = match Terminal::spawn_with_cwd_options(
            cols,
            rows,
            command.as_deref(),
            &env,
            &colors,
            Some(Path::new(&cwd)),
        ) {
            Ok(session) => std::sync::Arc::new(session),
            Err(err) => {
                eprintln!(
                    "labolabo-app: failed to spawn terminal session for task {task_id}: {err:#}"
                );
                return None;
            }
        };

        self.hooks
            .register_pane(pane_uuid.clone(), task_id.to_string(), pane_id);

        let redraw_task =
            task_workspace::spawn_redraw_bridge(session.clone(), task_id.to_string(), pane_id, cx);
        if let Some(workspace) = self.workspaces.get_mut(task_id) {
            task_workspace::insert_runtime(
                &mut workspace.runtimes,
                pane_id,
                session,
                cols,
                rows,
                pane_uuid.clone(),
                redraw_task,
            );
        }
        Some(pane_uuid)
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

    // MARK: - Claude Code hooks events (docs/hooks-protocol.md §6)

    /// Consumes one parsed hook event (delivered by
    /// `hooks::spawn_agent_event_bridge`): updates the routed pane's live
    /// [`AgentStatus`] (for the tab-chip dot), and -- for events carrying a
    /// `session_id` -- records the per-tab Claude session binding (into the
    /// Task's `TileLayout`, via `PaneTilingModel::record_agent_session`,
    /// exactly like the Swift app's tab-resume feature) and the Task-level
    /// `agent_bindings` fallback (docs/hooks-protocol.md §6(a); see
    /// `labolabo_core::AgentBindings`'s module doc comment for why these are
    /// two separate records).
    pub(crate) fn handle_agent_event(&mut self, event: AgentStatusEvent, cx: &mut Context<Self>) {
        let route = event
            .pane_id
            .as_deref()
            .and_then(|id| self.hooks.resolve_pane(id));

        if let Some(route) = &route {
            if let Some(workspace) = self.workspaces.get_mut(&route.task_id) {
                workspace.pane_status.insert(route.pane_id, event.status);
            }
        }

        if let Some(session_id) = &event.session_id {
            // Prefer the event's own `labolabo_task_id` (only trusted if it
            // names a Task still known to this run); fall back to the
            // routed pane's Task, matching docs/hooks-protocol.md §7's
            // "labolabo_task_id が予約済み" -- as of the forwarder's current
            // annotation, this is always the same Task as `route`'s when
            // both are present, but keeping them independently resolved
            // means a future task-id-only event (no pane_id) still records
            // the §6(a) fallback correctly.
            let binding_task_id = event
                .task_id
                .clone()
                .filter(|id| self.tasks.iter().any(|t| &t.id == id))
                .or_else(|| route.as_ref().map(|r| r.task_id.clone()));

            if let Some(task_id) = &binding_task_id {
                self.record_agent_binding(task_id, session_id, event.transcript_path.as_deref());
            }

            if let Some(route) = &route {
                if let Some(workspace) = self.workspaces.get_mut(&route.task_id) {
                    workspace.model.record_agent_session(
                        session_id.clone(),
                        route.pane_id,
                        event.transcript_path.clone(),
                    );
                }
                self.persist_workspace(&route.task_id);
            }
        }

        cx.notify();
    }

    /// Updates `task_id`'s `agent_bindings` column (docs/hooks-protocol.md
    /// §6(a) fallback) and persists it, unless the new observation is
    /// identical to what's already recorded (`AgentBindings::record`'s
    /// dedup check -- avoids a write on every `PreToolUse`/`PostToolUse` of
    /// an already-known session).
    fn record_agent_binding(
        &mut self,
        task_id: &str,
        session_id: &str,
        transcript_path: Option<&str>,
    ) {
        let Some(task) = self.tasks.iter_mut().find(|t| t.id == task_id) else {
            return;
        };
        let mut bindings = AgentBindings::from_json(task.agent_bindings.as_deref());
        if !bindings.record(session_id, transcript_path) {
            return;
        }
        task.agent_bindings = Some(bindings.to_json());
        if let Err(err) = self.db.upsert_task(task) {
            eprintln!("labolabo-app: failed to persist agent binding for task {task_id}: {err}");
        }
    }

    /// The aggregate [`AgentStatus`] shown on `task_id`'s sidebar row: the
    /// highest-priority status across its panes (priority order, highest
    /// first: waiting-for-input, running, starting, idle, ended/none/
    /// unknown), or `None` if the Task has no loaded workspace or no pane
    /// has reported a status yet. Deliberately a simple max, not
    /// last-writer-wins across panes -- Swift's sidebar dot has one status
    /// per *session* (1 worktree = 1 agent), so there's no direct analogue
    /// for "which of several tabs' statuses wins"; picking the most
    /// attention-worthy one seemed like the least surprising choice for this
    /// port's per-Task, multi-tab sidebar row.
    pub(crate) fn task_agent_status(&self, task_id: &str) -> Option<AgentStatus> {
        let workspace = self.workspaces.get(task_id)?;
        workspace
            .pane_status
            .values()
            .copied()
            .max_by_key(|status| status_priority(*status))
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
        self.open_tab_for_control(task_id, anchor_pane_id, None, None, window, cx);
    }

    /// Shared by the UI "+" tab button ([`Self::add_tab_to`], always
    /// `title: None, command: None`) and the control protocol's `tab_open`
    /// command (docs/control-protocol.md §5.1, `LaboLaboApp::
    /// control_tab_open`): adds a new tab to `anchor_pane_id`'s tab group
    /// and spawns its terminal session, optionally with a custom `title`
    /// and/or a shell `command` to run instead of the default resume/shell
    /// logic (see `spawn_runtime_for_task`'s doc comment).
    ///
    /// Returns the new pane's `LABOLABO_PANE` uuid on success (the control
    /// protocol's `tab_open` response's `pane_id`), or `None` if the anchor
    /// pane's tab group couldn't be found or the spawn itself failed --
    /// mirrors `spawn_runtime_for_task`'s own `None`-on-failure contract.
    pub(crate) fn open_tab_for_control(
        &mut self,
        task_id: &str,
        anchor_pane_id: PaneId,
        title: Option<String>,
        command: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        let pane = match &title {
            Some(title) => PaneItem::new(PaneKind::Terminal, title.clone()),
            None => PaneItem::new(PaneKind::Terminal, PaneKind::Terminal.default_title()),
        };
        let new_id = pane.id;
        let added = self
            .workspaces
            .get_mut(task_id)
            .map(|workspace| workspace.model.add_tab(anchor_pane_id, pane))
            .unwrap_or(false);
        if !added {
            return None;
        }
        let pane_uuid = self.spawn_runtime_for_task(
            task_id,
            new_id,
            DEFAULT_PANE_COLS,
            DEFAULT_PANE_ROWS,
            command,
            cx,
        );
        if let Some(workspace) = self.workspaces.get_mut(task_id) {
            workspace.focused_pane = new_id;
        }
        window.focus(&self.focus_handle);
        self.persist_workspace(task_id);
        cx.notify();
        pane_uuid
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
        self.spawn_runtime_for_task(
            task_id,
            new_id,
            DEFAULT_PANE_COLS,
            DEFAULT_PANE_ROWS,
            None,
            cx,
        );
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
                    workspace.pane_status.remove(&pane_id);
                    if let Some(runtime) = workspace.runtimes.remove(&pane_id) {
                        self.hooks.unregister_pane(&runtime.pane_uuid);
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
                workspace.pane_status.remove(&pane_id);
                if let Some(runtime) = workspace.runtimes.remove(&pane_id) {
                    self.hooks.unregister_pane(&runtime.pane_uuid);
                }
            }
            cx.notify();
            return;
        }

        if let Some(workspace) = self.workspaces.get_mut(task_id) {
            workspace.pane_status.remove(&pane_id);
            if let Some(runtime) = workspace.runtimes.remove(&pane_id) {
                self.hooks.unregister_pane(&runtime.pane_uuid);
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
            // The *picked* directory is the Task's attached directory (the
            // user may deliberately pick a subdirectory of a repo -- the
            // shell should open there, not at the repo root); the resolved
            // repo identity is only for sidebar grouping/labeling.
            let (directory, (repo_key, repo_root, repo_name)) = cx
                .background_spawn(async move {
                    let repo = new_task::resolve_attached_repo(&dir);
                    (dir.to_string_lossy().into_owned(), repo)
                })
                .await;
            let _ = this.update(cx, |app, cx| {
                app.finish_new_attached_task(directory, repo_key, repo_root, repo_name, cx)
            });
        })
        .detach();
    }

    fn finish_new_attached_task(
        &mut self,
        directory: String,
        repo_key: String,
        repo_root: String,
        repo_name: String,
        cx: &mut Context<Self>,
    ) {
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

    // MARK: - control protocol (docs/control-protocol.md, `plans/012` §2)

    /// Executes one control-protocol request and returns the serialized
    /// response (docs/control-protocol.md §6) -- `crate::control`'s bridge
    /// (`spawn_control_bridge`) calls this via `WindowHandle::update`, which
    /// is why a live `&mut Window` is available here (needed by
    /// `open_tab_for_control`/`select_task`/`select_pane`, all of which move
    /// keyboard focus).
    pub(crate) fn dispatch_control(
        &mut self,
        request_bytes: &[u8],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<u8> {
        let request = match labolabo_core::parse_request(request_bytes) {
            Ok(request) => request,
            Err(err) => return ControlResponse::err(err).to_bytes(),
        };
        let command = match ControlCommand::from_request(&request) {
            Ok(command) => command,
            Err(err) => return ControlResponse::err(err).to_bytes(),
        };
        let ambient_task = request.labolabo_task_id.as_deref();

        let response = match command {
            ControlCommand::TabOpen {
                task,
                title,
                command,
            } => self.control_tab_open(task.as_deref(), ambient_task, title, command, window, cx),
            ControlCommand::TaskList => self.control_task_list(),
            ControlCommand::TabList { task, all } => {
                self.control_tab_list(task.as_deref(), ambient_task, all)
            }
            ControlCommand::Focus { task, pane } => {
                self.control_focus(task.as_deref(), pane.as_deref(), window, cx)
            }
        };
        response.to_bytes()
    }

    /// `tab_open` (docs/control-protocol.md §5.1): resolves the target
    /// Task, loads its workspace if this is the first control/UI action to
    /// touch it, and opens a new tab in its currently focused pane's tab
    /// group via [`Self::open_tab_for_control`] -- the exact path the UI's
    /// "+" tab button uses, so env injection/hooks routing/persistence all
    /// go through the same code (docs/control-protocol.md §7).
    fn control_tab_open(
        &mut self,
        explicit_task: Option<&str>,
        ambient_task: Option<&str>,
        title: Option<String>,
        command_argv: Option<Vec<String>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> ControlResponse {
        let task_id = match labolabo_core::resolve_target_task(explicit_task, ambient_task) {
            Ok(id) => id,
            Err(err) => return ControlResponse::err(err),
        };
        if !self.tasks.iter().any(|t| t.id == task_id) {
            return ControlResponse::err(format!("unknown task id: {task_id}"));
        }

        let (cols, rows) = self.viewport_grid_size(window);
        self.ensure_workspace_loaded(&task_id, cols, rows, cx);
        let Some(anchor) = self.workspaces.get(&task_id).map(|w| w.focused_pane) else {
            return ControlResponse::err(format!("failed to load task workspace: {task_id}"));
        };

        // Each argv element is shell-quoted individually and space-joined
        // into the single command string `Terminal::spawn_with_cwd_options`
        // execs via `/bin/sh -c` (docs/control-protocol.md §5.1) -- the same
        // quoting rule `labolabo_core::shell_quote` documents for its other
        // callers (the hooks forwarder's `claude --resume` command string).
        let command = command_argv.map(|argv| {
            argv.iter()
                .map(|arg| shell_quote(arg))
                .collect::<Vec<_>>()
                .join(" ")
        });

        match self.open_tab_for_control(&task_id, anchor, title, command, window, cx) {
            Some(pane_uuid) => {
                ControlResponse::ok(serde_json::json!({ "task_id": task_id, "pane_id": pane_uuid }))
            }
            None => ControlResponse::err("failed to open a new tab (spawn failed)".to_string()),
        }
    }

    /// `task_list` (docs/control-protocol.md §5.2).
    fn control_task_list(&self) -> ControlResponse {
        let tasks: Vec<serde_json::Value> = self
            .tasks
            .iter()
            .map(|task| {
                serde_json::json!({
                    "id": task.id,
                    "title": task.title,
                    "kind": task.kind.tag(),
                    "repo_name": task.repo_name,
                    "working_directory": task.working_directory(),
                    "status": task.status.tag(),
                })
            })
            .collect();
        ControlResponse::ok(serde_json::json!({ "tasks": tasks }))
    }

    /// `tab_list` (docs/control-protocol.md §5.3): `all` overrides both
    /// `explicit_task` and `ambient_task` (lists every loaded Task's tabs);
    /// otherwise the target Task is `explicit_task.or(ambient_task)`, or --
    /// if neither is present -- every loaded Task's tabs (same as `all`,
    /// just reached by "nothing to filter on" rather than an explicit
    /// request).
    fn control_tab_list(
        &self,
        explicit_task: Option<&str>,
        ambient_task: Option<&str>,
        all: bool,
    ) -> ControlResponse {
        let target = if all {
            None
        } else {
            explicit_task.or(ambient_task)
        };
        if let Some(target) = target {
            if !self.tasks.iter().any(|t| t.id == target) {
                return ControlResponse::err(format!("unknown task id: {target}"));
            }
        }

        let mut tabs = Vec::new();
        for task in &self.tasks {
            if let Some(target) = target {
                if task.id != target {
                    continue;
                }
            }
            let Some(workspace) = self.workspaces.get(&task.id) else {
                continue;
            };
            for pane in workspace.model.panes() {
                let pane_uuid = workspace
                    .runtimes
                    .get(&pane.id)
                    .map(|runtime| runtime.pane_uuid.clone());
                tabs.push(serde_json::json!({
                    "task_id": task.id,
                    "pane_id": pane_uuid,
                    "title": pane.title,
                    "kind": pane.kind.raw_value(),
                    "focused": pane.id == workspace.focused_pane,
                }));
            }
        }
        ControlResponse::ok(serde_json::json!({ "tabs": tabs }))
    }

    /// `focus` (docs/control-protocol.md §5.4): exactly one of `task`/`pane`
    /// is `Some` (`ControlCommand::from_request` already validated this).
    /// Both are literal ids -- no `--task current`/ambient resolution here
    /// (see docs/control-protocol.md §5.4's note on why `focus` is
    /// deliberately excluded from that convenience).
    fn control_focus(
        &mut self,
        task: Option<&str>,
        pane: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> ControlResponse {
        if let Some(pane_uuid) = pane {
            let Some(route) = self.hooks.resolve_pane(pane_uuid) else {
                return ControlResponse::err(format!("unknown pane id: {pane_uuid}"));
            };
            if !self.tasks.iter().any(|t| t.id == route.task_id) {
                return ControlResponse::err(format!("task no longer exists: {}", route.task_id));
            }
            self.select_task(route.task_id.clone(), window, cx);
            self.select_pane(&route.task_id, route.pane_id, window, cx);
            return ControlResponse::ok(
                serde_json::json!({ "task_id": route.task_id, "pane_id": pane_uuid }),
            );
        }

        let Some(task_id) = task else {
            return ControlResponse::err("focus: --task or --pane is required".to_string());
        };
        if !self.tasks.iter().any(|t| t.id == task_id) {
            return ControlResponse::err(format!("unknown task id: {task_id}"));
        }
        self.select_task(task_id.to_string(), window, cx);
        ControlResponse::ok(serde_json::json!({ "task_id": task_id }))
    }

    // MARK: - input routing

    /// The currently focused pane's live runtime, if any -- shared by
    /// [`Self::write_focused_pane_input`], [`Self::action_paste`], and the
    /// `EntityInputHandler` impl below (`bounds_for_range` needs the
    /// runtime's live cursor position; the others need `session` itself).
    fn focused_pane_runtime(&self) -> Option<&PaneRuntime> {
        let task_id = self.selected_task_id.as_deref()?;
        let workspace = self.workspaces.get(task_id)?;
        workspace.runtimes.get(&workspace.focused_pane)
    }

    /// Write `bytes` to the focused pane's PTY. Returns whether there was a
    /// focused pane to write to (used by [`Self::key_down`] to decide
    /// whether to claim the keystroke -- see that method's doc comment).
    fn write_focused_pane_input(&self, bytes: &[u8]) -> bool {
        if let Some(runtime) = self.focused_pane_runtime() {
            runtime.session.write_input(bytes);
            true
        } else {
            false
        }
    }

    /// Handles every keystroke this app's root `div` sees. Only the keys
    /// `keys::keystroke_to_bytes` recognizes (Enter/Backspace/Tab/Escape/
    /// arrows, a bare Ctrl-<letter>) are written directly here -- plain
    /// printable text (including space) is deliberately left unhandled so
    /// it reaches the platform's IME/text-input machinery instead, arriving
    /// via this struct's `EntityInputHandler` impl below (see `keys.rs`'s
    /// module doc comment for the full reasoning).
    ///
    /// `cx.stop_propagation()` on a claimed keystroke is what prevents gpui
    /// from *also* forwarding it to the input handler (macOS's
    /// `NSTextInputContext`, or the X11/Wayland equivalent) once one is
    /// registered -- without it, e.g. Ctrl-A would additionally reach
    /// `doCommandBySelector:` on macOS (Cocoa's default Emacs-style text
    /// key bindings map it to `moveToBeginningOfLine:`), re-dispatching this
    /// same handler a second time for the one keystroke.
    fn key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(bytes) = keystroke_to_bytes(&event.keystroke) {
            if self.write_focused_pane_input(&bytes) {
                cx.stop_propagation();
            }
        }
    }

    // MARK: - action handlers (see the `actions!` list + main.rs's `bind_keys`)

    fn selected_task_and_focused_pane(&self) -> Option<(String, PaneId)> {
        let task_id = self.selected_task_id.clone()?;
        let focused = self.workspaces.get(&task_id)?.focused_pane;
        Some((task_id, focused))
    }

    /// Cmd+V: writes the system clipboard's text to the focused pane's PTY,
    /// newline-normalized and (when the pane's `Terminal::bracketed_paste()`
    /// reports DECSET `2004` is enabled) wrapped in `ESC[200~...ESC[201~` --
    /// see `paste::encode_paste`'s doc comment for the full contract. A
    /// clipboard with no text (empty, or an image-only entry) or no
    /// currently focused pane is a silent no-op. Copy (selection -> system
    /// clipboard) is out of this wave's scope -- text selection itself
    /// isn't implemented yet.
    fn action_paste(&mut self, _: &Paste, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        if text.is_empty() {
            return;
        }
        let Some(runtime) = self.focused_pane_runtime() else {
            return;
        };
        let bytes = paste::encode_paste(&text, runtime.session.bracketed_paste());
        runtime.session.write_input(&bytes);
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

/// IME (input method) integration: gpui's platform-agnostic surface over
/// macOS's `NSTextInputClient` (and the X11/Wayland IBus/fcitx equivalents).
/// `task_workspace::render_leaf` registers an `ElementInputHandler<Self>`
/// (via `Window::handle_input`) against the focused pane's canvas every
/// frame; the platform then routes text input -- both plain typing and IME
/// composition -- through the methods below instead of (or in addition to,
/// see `keys.rs`'s module doc comment for how the two are kept from
/// double-sending) raw `KeyDownEvent`s.
///
/// A terminal has no editable "document" of its own -- once text is
/// written to the PTY it's the child program's problem, not ours -- so
/// every method below treats the (nonexistent) document as always empty
/// except for whatever the *current* IME composition contributes: there is
/// no persistent selection, and `replace_text_in_range`/
/// `replace_and_mark_text_in_range` both ignore whatever `range` the
/// platform passes (there's nothing addressable to replace) and act purely
/// on the given text.
impl EntityInputHandler for LaboLaboApp {
    /// Only ever asked about the live preedit string (there is no other
    /// "document" -- see this impl's doc comment); `None` otherwise.
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let preedit = self.active_preedit.as_ref()?;
        *adjusted_range = Some(range_utf16.clone());
        Some(ime::utf16_slice(&preedit.text, range_utf16))
    }

    /// A terminal never has a persistent text selection to report; while
    /// composing, the caret is always at the end of the preedit string (we
    /// don't support moving it within an in-progress composition).
    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let preedit = self.active_preedit.as_ref()?;
        let len = ime::utf16_len(&preedit.text);
        Some(UTF16Selection {
            range: len..len,
            reversed: false,
        })
    }

    /// `Some(0..len)` while an IME composition is in progress, `None`
    /// otherwise -- this is what the platform (and `keys.rs`'s design,
    /// via macOS's `is_composing` check) uses to decide whether a keystroke
    /// belongs to the IME or to plain dispatch.
    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.active_preedit
            .as_ref()
            .map(|preedit| 0..ime::utf16_len(&preedit.text))
    }

    /// IME composition cancelled (e.g. Escape while composing, or focus
    /// loss) without committing -- clear the preedit and redraw. Never
    /// writes to the PTY.
    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.active_preedit.take().is_some() {
            cx.notify();
        }
    }

    /// A commit: either a plain (non-composing) character/string being
    /// typed, or an IME composition's final confirmed text. Either way, any
    /// in-progress preedit is cleared and `text` is written verbatim to the
    /// focused pane's PTY as UTF-8 bytes.
    fn replace_text_in_range(
        &mut self,
        _range: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.active_preedit = None;
        if !text.is_empty() {
            self.write_focused_pane_input(text.as_bytes());
        }
        cx.notify();
    }

    /// An IME composition update (`setMarkedText:`): `new_text` is the
    /// current preedit string. Never written to the PTY -- only tracked, so
    /// `task_workspace::render_leaf` can paint it inline over the focused
    /// pane's cursor (`render::paint_preedit`) -- until a later
    /// `replace_text_in_range` commits it or `unmark_text` cancels it.
    fn replace_and_mark_text_in_range(
        &mut self,
        _range: Option<Range<usize>>,
        new_text: &str,
        _new_selected_range: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.active_preedit = if new_text.is_empty() {
            None
        } else {
            self.selected_task_and_focused_pane()
                .map(|(task_id, pane_id)| PreeditState {
                    task_id,
                    pane_id,
                    text: new_text.to_string(),
                })
        };
        cx.notify();
    }

    /// The focused pane's current cursor cell, in the input-handling
    /// canvas's own coordinate space (`element_bounds`, exactly as captured
    /// when `task_workspace::render_leaf` constructed the
    /// `ElementInputHandler` this frame) -- used by the platform to
    /// position the IME candidate window right over the terminal cursor.
    fn bounds_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let runtime = self.focused_pane_runtime()?;
        let cursor = runtime.session.snapshot().cursor;
        let origin = element_bounds.origin
            + point(
                px(cursor.col as f32 * self.spec.cell_width),
                px(cursor.row as f32 * self.spec.cell_height),
            );
        Some(Bounds::new(
            origin,
            size(px(self.spec.cell_width), px(self.spec.cell_height)),
        ))
    }

    /// The reverse of `bounds_for_range` (a screen point -> a document
    /// offset) -- not supported, for the same "no addressable document"
    /// reason `text_for_range` mostly isn't either.
    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

/// A single fresh terminal pane's `TileLayout` -- the initial layout for
/// every newly created Task (both kinds).
fn single_terminal_layout() -> labolabo_core::TileLayout {
    let pane = PaneItem::new(PaneKind::Terminal, PaneKind::Terminal.default_title());
    PaneTilingModel::new(TileNode::leaf(pane)).snapshot()
}

/// Ranks [`AgentStatus`] by how attention-worthy it is, highest first --
/// used by [`LaboLaboApp::task_agent_status`] to pick one status to show for
/// a Task with multiple tabs in different states.
fn status_priority(status: AgentStatus) -> u8 {
    match status {
        AgentStatus::WaitingForInput => 4,
        AgentStatus::Running => 3,
        AgentStatus::Starting => 2,
        AgentStatus::Idle => 1,
        AgentStatus::None | AgentStatus::Ended => 0,
    }
}

impl Render for LaboLaboApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let sidebar_el = sidebar::render(self, cx);

        let workspace_el = if let Some(task_id) = self.selected_task_id.clone() {
            if let Some(workspace) = self.workspaces.get(&task_id) {
                let spec = self.spec.clone();
                let focus_handle = self.focus_handle.clone();
                let active_preedit = self.active_preedit.clone();
                let focused_pane = workspace.focused_pane;
                task_workspace::render_tile(
                    &task_id,
                    &workspace.model.root,
                    &workspace.runtimes,
                    &workspace.pane_status,
                    focused_pane,
                    &spec,
                    &focus_handle,
                    active_preedit.as_ref(),
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
            .on_action(cx.listener(Self::action_paste))
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
