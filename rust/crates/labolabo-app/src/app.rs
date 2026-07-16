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

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::{Path, PathBuf};

use gpui::{
    actions, div, point, prelude::*, px, rgb, size, Bounds, ClipboardItem, Context, DragMoveEvent,
    EntityInputHandler, ExternalPaths, FocusHandle, IntoElement, KeyDownEvent, Modifiers,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, PathPromptOptions, Pixels, Point, Render,
    ScrollWheelEvent, Task as GpuiTask, UTF16Selection, Window,
};

use labolabo_core::{
    claude_resume_command, cross_session_conflicts, quote_dropped_paths, reorder_task_ids,
    shell_quote, AgentBindings, AgentStatus, AgentStatusEvent, AgentUsage, ControlCommand,
    ControlResponse, DropEdge, PaneId, PaneItem, PaneKind, PaneTilingModel, Task, TaskDatabase,
    TaskStatus, TileNode, TileOrientation,
};
use labolabo_term::{ColorScheme, Terminal};

use crate::control::{self, ControlRuntime};
use crate::focus;
use crate::ghostty_config::FontConfig;
use crate::git_pane::{self, FileViewMode, GitSnapshot};
use crate::grid;
use crate::hooks::{self, HookRuntime};
use crate::ide_open;
use crate::ime;
use crate::keys::keystroke_to_bytes;
use crate::menus;
use crate::motion::DotAnimState;
use crate::mouse_report::{self, MouseAction, MouseButtonKind, MouseMods};
use crate::new_task;
use crate::paste;
use crate::render::RenderSpec;
use crate::selection::{self, CellPos};
use crate::settings::{self, AppSettings};
use crate::sidebar;
use crate::task_lifecycle::{self, WorktreeRemoveOutcome};
use crate::task_menu::{self, TaskMenuState};
use crate::task_workspace::{self, PaneDragHover, PaneRuntime, TabDragPayload, TaskWorkspace};
use crate::theme;
use crate::window_bounds;

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

/// ウィンドウ bounds 保存 (wave 6c §3) のデバウンス幅 -- ドラッグ中の
/// 連続イベントを 1 回の SQLite 書き込みへ集約する。
const WINDOW_BOUNDS_SAVE_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(500);

actions!(
    labolabo_app,
    [
        NewTab,
        CloseTab,
        SplitRight,
        SplitDown,
        Paste,
        Copy,
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
        ToggleGitPane,
        ToggleSettings,
        // メニューバー (wave 6c §1, `crate::menus`)。`Quit` だけはウィンドウ
        // 非依存なので `main.rs` がグローバル `cx.on_action` で処理し、他は
        // 従来どおりルート要素の `.on_action`（`Render` 参照）。
        About,
        Quit,
        NewAttachedTask,
        NewWorktreeTask,
        MinimizeWindow,
        ZoomWindow,
        OpenSelectedInIde,
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
    /// The Cmd+, settings screen's persisted values -- see `crate::settings`'s
    /// module doc comment. Loaded once at startup (`AppSettings::load`) and
    /// kept in sync with `db`'s `appState` table by every `set_*`/`adjust_*`
    /// method below.
    settings: AppSettings,
    /// Whether the settings overlay (`crate::settings::render_settings_overlay`)
    /// is currently shown -- purely transient UI state, never persisted
    /// (unlike `settings` itself).
    settings_open: bool,
    /// Cache of each Task's most-recently-fetched Git status changed paths,
    /// keyed by `Task::id` -- the input to [`Self::task_conflicts`]
    /// (`labolabo_core::cross_session_conflicts`). Populated by
    /// [`Self::apply_git_refresh`], which today only ever runs for the
    /// *selected* Task (the Git pane's `FileWatcher` is only ever attached
    /// there -- see `crate::git_pane`'s module doc comment), so a Task that
    /// has never been selected simply has no entry here and never
    /// contributes to (or triggers) a conflict on its own -- the wave
    /// brief's explicitly accepted "status 取得済みのタスク間のみで検出"
    /// limitation (see `crates/labolabo-app/README.md`).
    changed_files_cache: HashMap<String, HashSet<String>>,
    /// Whether a pane-divider drag-resize (`plans` W5j #2) is currently in
    /// progress, anywhere in any Task's tree -- a single app-wide flag
    /// (not per-Task/per-node) because at most one divider can be dragged
    /// at a time regardless of which Task's tree it's in. Set `true` on
    /// every `update_divider_drag` call (harmless to set repeatedly) and
    /// back to `false` on `finish_divider_drag`. Threaded down through
    /// `task_workspace::render_tile`/`render_leaf` so every terminal
    /// pane's canvas can suppress `Terminal::resize` while a drag is live
    /// -- see `render_leaf`'s `prepaint` closure for why.
    divider_drag_active: bool,
    /// アーカイブ済み (`TaskStatus::Archived`) タスク -- サイドバー下部の
    /// 折りたたみセクション（既定折りたたみ）の内容 (wave 6c §2)。`tasks`
    /// と違い workspace は持たない（アーカイブ時に shutdown 済み）。復元で
    /// `tasks` へ戻る。
    archived_tasks: Vec<Task>,
    /// サイドバー「アーカイブ済み (n)」セクションの開閉 -- 純粋な UI 状態
    /// （非永続、起動時は常に折りたたみ）。
    archived_expanded: bool,
    /// タスク行「…」メニュー / 削除確認オーバーレイの状態 (wave 6c §2、
    /// `crate::task_menu`)。`None` = 閉じている。
    task_menu: Option<TaskMenuState>,
    /// About オーバーレイ (`crate::menus::render_about_overlay`) の開閉 --
    /// `settings_open` と同じく純粋な UI 状態。
    about_open: bool,
    /// 「IDE で開く」のインストール済みエディタ検出結果 (wave 6c 追加要望、
    /// `crate::ide_open`)。起動時にバックグラウンドで一度だけ Spotlight
    /// (`mdfind`) 検出を走らせてキャッシュする。`None` = 検出未完了
    /// （メニューにはまだエディタを出さない -- 「Finder で表示」は常に
    /// 出る）。
    installed_editors: Option<Vec<ide_open::EditorCandidate>>,
    /// ウィンドウ bounds 保存 (wave 6c §3) のデバウンス世代カウンタ --
    /// bounds 変化のたびに進め、~500ms 後にまだ最新世代だったものだけが
    /// 保存を実行する（`schedule_window_bounds_save`）。
    bounds_save_generation: u64,
    /// 直近の（まだ保存されていない）ウィンドウ bounds。
    pending_window_bounds: Option<Bounds<Pixels>>,
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
        let all_tasks: Vec<Task> = db.all_tasks().unwrap_or_else(|err| {
            eprintln!("labolabo-app: failed to load tasks ({err}); starting with an empty list");
            Vec::new()
        });
        let mut tasks: Vec<Task> = Vec::new();
        let mut archived_tasks: Vec<Task> = Vec::new();
        for task in all_tasks {
            match task.status {
                TaskStatus::Active => tasks.push(task),
                // アーカイブ済みはサイドバー下部の折りたたみセクションへ
                // (wave 6c §2)。workspace は復元しない（復元操作で戻す）。
                TaskStatus::Archived => archived_tasks.push(task),
                // `Done` は W5b-3 のスキーマ予約のみで、遷移させる UI は
                // まだ無い -- 読み飛ばす（DB には残る）。
                TaskStatus::Done => {}
            }
        }

        let selected_task_id = db
            .selected_task_id()
            .ok()
            .flatten()
            .filter(|id| tasks.iter().any(|t| &t.id == id))
            .or_else(|| tasks.first().map(|t| t.id.clone()));

        // Cmd+, settings screen (`plans` wave 5i §3) -- loaded once here so
        // every field below that depends on a setting (the Git pane's
        // default visibility, the resume-at-spawn gate, the scrollback cap)
        // sees the persisted value from the very first Task load, not just
        // after the settings panel is opened once.
        let settings = AppSettings::load(&db);

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

        // 「IDE で開く」のエディタ検出 (wave 6c、`crate::ide_open`):
        // Spotlight (`mdfind`) を候補ごとに 1 回ずつ叩くブロッキング処理
        // なので、起動時に一度だけバックグラウンドで走らせて結果を
        // キャッシュする。完了までは `installed_editors == None`（メニュー
        // にエディタが出ないだけで、他は全て動く）。
        cx.spawn(async move |this, cx| {
            let editors = cx
                .background_spawn(async move { ide_open::detect_installed_editors() })
                .await;
            let _ = this.update(cx, |app, cx| {
                app.installed_editors = Some(editors);
                cx.notify();
            });
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
            settings,
            settings_open: false,
            changed_files_cache: HashMap::new(),
            divider_drag_active: false,
            archived_tasks,
            archived_expanded: false,
            task_menu: None,
            about_open: false,
            installed_editors: None,
            bounds_save_generation: 0,
            pending_window_bounds: None,
        };

        if let Some(id) = selected_task_id {
            let (cols, rows) = this.viewport_grid_size(window);
            this.ensure_workspace_loaded(&id, cols, rows, cx);
            this.activate_git_pane(&id, cx);
        }

        this.dev_force_running_if_requested(cx);

        // ウィンドウ移動/リサイズの追従: 再描画に加えて、bounds を ~500ms
        // デバウンスで appState (`windowBounds`) へ保存する (wave 6c §3、
        // `crate::window_bounds`)。
        cx.observe_window_bounds(window, |this, window, cx| {
            this.schedule_window_bounds_save(window, cx);
            cx.notify();
        })
        .detach();

        this
    }

    /// ウィンドウ bounds の保存をデバウンス付きで予約する。イベントごとに
    /// 世代カウンタを進めて 500ms 待つ小さなタスクを積み、満了時にまだ
    /// 最新世代だったタスクだけが実際に保存する（ドラッグ中の連続イベント
    /// では古い世代が全て捨てられ、手を離して 500ms 後に 1 回だけ書く）。
    /// フルスクリーン/最大化中は `WindowBounds::get_bounds()` が返す
    /// restore サイズを保存し、復元は常に通常ウィンドウ（README 参照）。
    fn schedule_window_bounds_save(&mut self, window: &Window, cx: &mut Context<Self>) {
        let bounds = window.window_bounds().get_bounds();
        self.pending_window_bounds = Some(bounds);
        self.bounds_save_generation = self.bounds_save_generation.wrapping_add(1);
        let generation = self.bounds_save_generation;
        cx.spawn(async move |this, cx| {
            gpui::Timer::after(WINDOW_BOUNDS_SAVE_DEBOUNCE).await;
            let _ = this.update(cx, |app, _cx| {
                if app.bounds_save_generation != generation {
                    return; // 新しい bounds 変化に置き換えられた
                }
                let Some(bounds) = app.pending_window_bounds.take() else {
                    return;
                };
                let json =
                    window_bounds::encode(window_bounds::SavedWindowBounds::from_bounds(bounds));
                if let Err(err) = app.db.set_window_bounds(&json) {
                    eprintln!("labolabo-app: failed to persist window bounds: {err}");
                }
            });
        })
        .detach();
    }

    /// Development-only hook for `plans/014`'s M2 power verification
    /// ("Running 状態はテスト用に status を直接注入する開発フックで再現し
    /// てよい"): with `LABOLABO_DEV_FORCE_RUNNING=1` set, forces a Task's
    /// first pane into [`AgentStatus::Running`] immediately at startup, so
    /// the status-dot breathing animation (`motion::status_dot_element`)
    /// can be observed/measured without a real Claude Code agent attached
    /// and without any synthetic keyboard/mouse input (this repo's
    /// automation policy forbids the latter). If no Task exists yet (a
    /// throwaway `LABOLABO_RS_DATA_DIR`'s fresh database, the common case
    /// for this verification), creates one via the exact same
    /// `Task::new_attached`/`add_task_and_select` path `sidebar::
    /// icon_button`'s attached-Task button uses (real code, real PTY
    /// spawn -- just skipping the
    /// (forbidden) UI click that normally triggers it), at a scratch temp
    /// directory. A no-op (and the env var check itself is the only cost)
    /// whenever the var isn't set -- never reachable from the real hooks/
    /// control-protocol event paths, so it has no effect on normal use.
    fn dev_force_running_if_requested(&mut self, cx: &mut Context<Self>) {
        if std::env::var("LABOLABO_DEV_FORCE_RUNNING").as_deref() != Ok("1") {
            return;
        }
        if self.selected_task_id.is_none() {
            let dir = std::env::temp_dir()
                .join(format!("labolabo-dev-force-running-{}", std::process::id()));
            if std::fs::create_dir_all(&dir).is_err() {
                return;
            }
            let (repo_key, repo_root, repo_name) = new_task::resolve_attached_repo(&dir);
            let layout = single_terminal_layout();
            let sort_order = self.next_sort_order();
            let task = Task::new_attached(
                repo_key,
                repo_root,
                repo_name,
                dir.to_string_lossy().into_owned(),
                layout,
                sort_order,
            );
            self.add_task_and_select(task, cx);
        }

        let Some(task_id) = self.selected_task_id.clone() else {
            return;
        };
        let Some(workspace) = self.workspaces.get_mut(&task_id) else {
            return;
        };
        let Some(pane_id) = workspace.model.panes().first().map(|p| p.id) else {
            return;
        };
        workspace.pane_status.insert(pane_id, AgentStatus::Running);
        eprintln!(
            "labolabo-app: LABOLABO_DEV_FORCE_RUNNING=1 -- forcing task {task_id} pane {pane_id:?} to AgentStatus::Running"
        );
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

    pub(crate) fn settings(&self) -> &AppSettings {
        &self.settings
    }

    pub(crate) fn settings_open(&self) -> bool {
        self.settings_open
    }

    pub(crate) fn about_open(&self) -> bool {
        self.about_open
    }

    pub(crate) fn close_about(&mut self, cx: &mut Context<Self>) {
        if self.about_open {
            self.about_open = false;
            cx.notify();
        }
    }

    pub(crate) fn archived_tasks(&self) -> &[Task] {
        &self.archived_tasks
    }

    pub(crate) fn archived_expanded(&self) -> bool {
        self.archived_expanded
    }

    pub(crate) fn toggle_archived_section(&mut self, cx: &mut Context<Self>) {
        self.archived_expanded = !self.archived_expanded;
        cx.notify();
    }

    pub(crate) fn task_menu(&self) -> Option<&TaskMenuState> {
        self.task_menu.as_ref()
    }

    /// 検出済みのインストール済みエディタ（検出未完了なら空）。
    pub(crate) fn installed_editors(&self) -> &[ide_open::EditorCandidate] {
        self.installed_editors.as_deref().unwrap_or(&[])
    }

    // MARK: - タスク行「…」メニュー / アーカイブ / 削除 (wave 6c §2)

    /// サイドバー行の「…」ボタンから開く（`anchor` はクリック位置 =
    /// ウィンドウ座標）。
    pub(crate) fn open_task_menu(
        &mut self,
        task_id: &str,
        anchor: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let Some(task) = self.tasks.iter().find(|t| t.id == task_id) else {
            return;
        };
        self.task_menu = Some(TaskMenuState::new(task, anchor));
        cx.notify();
    }

    /// メニュー/確認オーバーレイを閉じる（git 実行中は閉じない --
    /// `TaskMenuState::can_dismiss`）。
    pub(crate) fn close_task_menu(&mut self, cx: &mut Context<Self>) {
        if let Some(menu) = &self.task_menu {
            if !menu.can_dismiss() {
                return;
            }
        }
        if self.task_menu.take().is_some() {
            cx.notify();
        }
    }

    /// タスクの作業ディレクトリを指定エディタで開く（`crate::ide_open`、
    /// macOS のみメニューに出る）。`open` の起動はバックグラウンド。
    pub(crate) fn open_task_in_editor(
        &mut self,
        task_id: &str,
        bundle_id: &'static str,
        cx: &mut Context<Self>,
    ) {
        let Some(task) = self.tasks.iter().find(|t| t.id == task_id) else {
            return;
        };
        let dir = PathBuf::from(task.working_directory());
        cx.background_spawn(async move {
            if let Err(err) = ide_open::open_in_editor(bundle_id, &dir) {
                eprintln!("labolabo-app: failed to open editor: {err}");
            }
        })
        .detach();
        self.task_menu = None;
        cx.notify();
    }

    /// タスクの作業ディレクトリを Finder で表示（macOS のみメニューに出る）。
    pub(crate) fn reveal_task_in_finder(&mut self, task_id: &str, cx: &mut Context<Self>) {
        let Some(task) = self.tasks.iter().find(|t| t.id == task_id) else {
            return;
        };
        let dir = PathBuf::from(task.working_directory());
        cx.background_spawn(async move {
            if let Err(err) = ide_open::reveal_in_finder(&dir) {
                eprintln!("labolabo-app: failed to reveal in Finder: {err}");
            }
        })
        .detach();
        self.task_menu = None;
        cx.notify();
    }

    /// タスクのセッションを shutdown して workspace を破棄する（アーカイブ
    /// /削除の前段）。pty へ SIGHUP 相当を送り、hooks のルーティング表から
    /// も外す。workspace は `workspaces` から取り除くので、後で同じタスクを
    /// 再選択/復元すれば `ensure_workspace_loaded` が新しいシェルを張り直す
    /// （自己修復 -- worktree 削除が git に拒否された後もタスクは使える）。
    fn shutdown_workspace(&mut self, task_id: &str) {
        self.deactivate_git_pane(task_id);
        if let Some(workspace) = self.workspaces.remove(task_id) {
            for runtime in workspace.runtimes.into_values() {
                self.hooks.unregister_pane(&runtime.pane_uuid);
                runtime.session.shutdown();
            }
        }
        self.changed_files_cache.remove(task_id);
    }

    /// `task_id` を `tasks` から外し、それが選択中だったら選択を隣の
    /// active タスクへ移す（無ければ空状態）。外した `Task` を返す。
    /// 次の選択は `task_lifecycle::next_selected_id`（純ロジック）で決める。
    fn remove_active_task_entry(
        &mut self,
        task_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Task> {
        let index = self.tasks.iter().position(|t| t.id == task_id)?;
        let was_selected = self.selected_task_id.as_deref() == Some(task_id);
        let ids: Vec<&str> = self.tasks.iter().map(|t| t.id.as_str()).collect();
        let next = task_lifecycle::next_selected_id(&ids, task_id);
        let task = self.tasks.remove(index);
        if was_selected {
            self.selected_task_id = None;
            match next {
                Some(next_id) => self.select_task(next_id, window, cx),
                None => {
                    if let Err(err) = self.db.set_selected_task_id(None) {
                        eprintln!("labolabo-app: failed to persist selected task: {err}");
                    }
                }
            }
        }
        Some(task)
    }

    /// アーカイブ: セッションを shutdown し、`status = archived` で保存して
    /// サイドバー下部の「アーカイブ済み」セクションへ移す。選択中だったら
    /// 他の active タスクへ選択を移す（無ければ空状態）。
    pub(crate) fn archive_task(
        &mut self,
        task_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.tasks.iter().any(|t| t.id == task_id) {
            return;
        }
        self.task_menu = None;
        self.shutdown_workspace(task_id);
        if let Some(mut task) = self.remove_active_task_entry(task_id, window, cx) {
            task.status = TaskStatus::Archived;
            if let Err(err) = self.db.upsert_task(&task) {
                eprintln!("labolabo-app: failed to archive task {task_id}: {err}");
            }
            self.archived_tasks.push(task);
        }
        cx.notify();
    }

    /// 復元: `status = active` へ戻して末尾に並べ（`sort_order` は採番し
    /// 直し）、選択する。workspace は `select_task` 経由の
    /// `ensure_workspace_loaded` が新しく張る。
    pub(crate) fn restore_task(
        &mut self,
        task_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.archived_tasks.iter().position(|t| t.id == task_id) else {
            return;
        };
        let mut task = self.archived_tasks.remove(index);
        task.status = TaskStatus::Active;
        task.sort_order = self.next_sort_order();
        task.last_active_at = chrono::Utc::now();
        if let Err(err) = self.db.upsert_task(&task) {
            eprintln!("labolabo-app: failed to restore task {task_id}: {err}");
        }
        let id = task.id.clone();
        self.tasks.push(task);
        self.select_task(id, window, cx);
        cx.notify();
    }

    /// メニューの「削除…」: 確認相へ（実削除はしない）。
    pub(crate) fn request_delete_task(&mut self, cx: &mut Context<Self>) {
        if let Some(menu) = &mut self.task_menu {
            menu.request_delete();
            cx.notify();
        }
    }

    /// 確認モーダルの「ブランチも削除」トグル。
    pub(crate) fn toggle_delete_branch(&mut self, cx: &mut Context<Self>) {
        if let Some(menu) = &mut self.task_menu {
            menu.toggle_delete_branch();
            cx.notify();
        }
    }

    /// 確認モーダルの実行ボタン。attached 型は DB からの登録解除のみ
    /// （**実ディレクトリには絶対に触れない**）。worktree 型はセッション
    /// shutdown → バックグラウンドで `git worktree remove`（force しない）
    /// → 成功時のみ（チェック時）`git branch -d` → DB から削除
    /// （`crate::task_lifecycle`）。失敗（未コミット変更による拒否等）は
    /// 確認モーダル内に表示して中断し、DB からも消さない。
    pub(crate) fn execute_delete_task(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (task_id, worktree, delete_branch) = {
            let Some(menu) = &mut self.task_menu else {
                return;
            };
            if !menu.begin_execution() {
                return;
            }
            (
                menu.task_id.clone(),
                menu.worktree.clone(),
                menu.delete_branch_requested(),
            )
        };
        cx.notify();

        match worktree {
            None => {
                // attached 型: 登録解除のみ。ファイルには触れない。
                self.shutdown_workspace(&task_id);
                if let Err(err) = self.db.delete_task(&task_id) {
                    if let Some(menu) = &mut self.task_menu {
                        menu.fail(format!("登録を解除できませんでした: {err}"));
                    }
                    cx.notify();
                    return;
                }
                self.remove_active_task_entry(&task_id, window, cx);
                self.task_menu = None;
                cx.notify();
            }
            Some(info) => {
                self.shutdown_workspace(&task_id);
                let repo_root = PathBuf::from(info.repo_root);
                let worktree_path = PathBuf::from(info.path);
                let branch = info.branch;
                cx.spawn_in(window, async move |this, cx| {
                    let outcome = cx
                        .background_spawn(async move {
                            task_lifecycle::remove_worktree_and_maybe_branch(
                                &repo_root,
                                &worktree_path,
                                &branch,
                                delete_branch,
                            )
                        })
                        .await;
                    let _ = this.update_in(cx, |app, window, cx| {
                        app.finish_worktree_delete(&task_id, outcome, window, cx);
                    });
                })
                .detach();
            }
        }
    }

    /// worktree 削除のバックグラウンド処理の完了側。
    fn finish_worktree_delete(
        &mut self,
        task_id: &str,
        outcome: WorktreeRemoveOutcome,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match outcome {
            WorktreeRemoveOutcome::Refused { message } => {
                // タスクは DB にも一覧にも残す。shutdown 済みの workspace
                // は次の選択時に張り直される（`shutdown_workspace` 参照）。
                if let Some(menu) = &mut self.task_menu {
                    menu.fail(message);
                }
            }
            WorktreeRemoveOutcome::Removed { branch_warning } => {
                if let Err(err) = self.db.delete_task(task_id) {
                    eprintln!("labolabo-app: failed to delete task row {task_id}: {err}");
                }
                self.remove_active_task_entry(task_id, window, cx);
                match branch_warning {
                    // worktree 削除自体は完了扱い -- ブランチ削除の失敗
                    // だけを後日談として表示する。
                    Some(warning) => {
                        if let Some(menu) = &mut self.task_menu {
                            menu.show_notice(format!("worktree は削除しました。{warning}"));
                        }
                    }
                    None => {
                        self.task_menu = None;
                    }
                }
            }
        }
        cx.notify();
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

        self.workspaces.insert(
            task_id.to_string(),
            TaskWorkspace::new(model, self.settings.git_pane_default_visible),
        );

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
    ///   ready yet" race that approach has to guard against). Gated on
    ///   `self.settings.auto_resume_enabled` (`plans` wave 5i §3's settings
    ///   screen) -- when disabled, every pane spawns a plain shell
    ///   regardless of what's recorded, app-wide rather than per-call.
    /// - **Scrollback cap**: every pane spawns with
    ///   `self.settings.scrollback_lines` (`plans` wave 5i §3), not the VT
    ///   backends' own hardcoded default -- see `labolabo_term::TermSession::
    ///   spawn_with_scrollback_options`'s doc comment. A change to this
    ///   setting only affects panes spawned *after* it (an already-running
    ///   pane's VT core isn't resized retroactively), matching the settings
    ///   panel's own footer copy.
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
        let auto_resume_enabled = self.settings.auto_resume_enabled;
        let scrollback_lines = self.settings.scrollback_lines;

        let pane_snapshot = self.workspaces.get(task_id).and_then(|workspace| {
            workspace
                .model
                .panes()
                .into_iter()
                .find(|p| p.id == pane_id)
                .cloned()
        });
        let command = override_command.or_else(|| {
            if !auto_resume_enabled {
                return None;
            }
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

        let session = match Terminal::spawn_with_scrollback_options(
            cols,
            rows,
            command.as_deref(),
            &env,
            &colors,
            Some(Path::new(&cwd)),
            scrollback_lines,
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

        // Transcript usage (`plans` wave 5i §1) -- best-effort token/cost
        // aggregation, re-read only on hook-event arrival, never polled
        // (mirrors Swift's `AgentSessionModel.refreshUsage`'s identical
        // "応答完了/終了時に transcript から使用量を集計" trigger): once a
        // pane's agent turn completes (`Idle`) or its session ends
        // (`Ended`), re-read and re-parse its transcript file in the
        // background. The transcript path prefers this event's own
        // `transcript_path`, falling back to whatever was already recorded
        // for the pane (mirrors Swift's `lastTranscriptPath`, which
        // persists across events that don't carry a fresh path of their
        // own -- most `Idle`/`Stop` events do carry one, but this guards
        // against a malformed/older hook payload that doesn't).
        if matches!(event.status, AgentStatus::Idle | AgentStatus::Ended) {
            if let Some(route) = &route {
                let transcript_path = event.transcript_path.clone().or_else(|| {
                    self.workspaces.get(&route.task_id).and_then(|workspace| {
                        workspace
                            .model
                            .panes()
                            .into_iter()
                            .find(|p| p.id == route.pane_id)
                            .and_then(|p| p.agent_transcript_path.clone())
                    })
                });
                if let Some(path) = transcript_path {
                    self.refresh_pane_usage(route.task_id.clone(), route.pane_id, path, cx);
                }
            }
        }

        cx.notify();
    }

    /// Kicks off a background read+parse of `transcript_path`
    /// (`labolabo_core::transcript_usage::read`, real file I/O -- never run
    /// on gpui's main thread, same `cx.background_spawn`-then-`this.update`
    /// shape as `Self::request_git_refresh`) and applies the result to
    /// `task_id`'s `pane_id` on completion. A transcript that doesn't parse
    /// to any usage (`read`'s `None` -- e.g. the file doesn't exist yet, or
    /// has no `assistant` turns) leaves `pane_usage` untouched rather than
    /// clearing a previously observed value, since a later event for the
    /// same pane always supersedes it via the same path.
    fn refresh_pane_usage(
        &mut self,
        task_id: String,
        pane_id: PaneId,
        transcript_path: String,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            let usage = cx
                .background_spawn(async move {
                    labolabo_core::transcript_usage::read(std::path::Path::new(&transcript_path))
                })
                .await;
            if let Some(usage) = usage {
                let _ = this.update(cx, |app, cx| {
                    app.apply_pane_usage(&task_id, pane_id, usage, cx)
                });
            }
        })
        .detach();
    }

    /// Applies a freshly parsed [`AgentUsage`] to `task_id`'s `pane_id` --
    /// see [`Self::refresh_pane_usage`]. A no-op (no `cx.notify()`) if the
    /// Task's workspace vanished while the background read was in flight
    /// (Task closed/switched away mid-refresh).
    fn apply_pane_usage(
        &mut self,
        task_id: &str,
        pane_id: PaneId,
        usage: AgentUsage,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.workspaces.get_mut(task_id) else {
            return;
        };
        workspace.pane_usage.insert(pane_id, usage);
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

    /// `task_id`'s sidebar-row status-dot crossfade state (`plans/014` M1,
    /// `TaskWorkspace::dot_anim`) -- `sidebar::render`'s read-only access to
    /// it, mirroring `task_agent_status` right above. `None` only when
    /// `task_id` has no loaded workspace (never selected yet).
    pub(crate) fn task_dot_anim(&self, task_id: &str) -> Option<&Cell<DotAnimState>> {
        self.workspaces
            .get(task_id)
            .map(|workspace| &workspace.dot_anim)
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
        let previously_selected = self.selected_task_id.clone();
        let (cols, rows) = self.viewport_grid_size(window);
        self.ensure_workspace_loaded(&task_id, cols, rows, cx);

        // Only the *selected* Task's Git pane watches live -- see
        // `crate::git_pane`'s module doc comment ("非フォーカスタスクの監視
        // は止める"). Stop the outgoing Task's before starting the
        // incoming one's below.
        if let Some(previous) = previously_selected {
            self.deactivate_git_pane(&previous);
        }

        self.selected_task_id = Some(task_id.clone());
        if let Err(err) = self.db.set_selected_task_id(Some(&task_id)) {
            eprintln!("labolabo-app: failed to persist selected task: {err}");
        }
        if let Some(task) = self.tasks.iter_mut().find(|t| t.id == task_id) {
            task.last_active_at = chrono::Utc::now();
            let _ = self.db.upsert_task(task);
        }
        self.activate_git_pane(&task_id, cx);

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
        if let Some(previous) = self.selected_task_id.clone() {
            self.deactivate_git_pane(&previous);
        }
        self.selected_task_id = Some(id.clone());
        let _ = self.db.set_selected_task_id(Some(&id));
        self.activate_git_pane(&id, cx);
        cx.notify();
    }

    // MARK: - drag & drop (`plans/012-task-model-and-control-cli.md` §3)

    /// Tracks a tab-chip drag's current drop-zone highlight for `task_id`,
    /// as it moves over one of that Task's leaves
    /// (`task_workspace::render_leaf`'s `.on_drag_move::<TabDragPayload>`,
    /// one registration per leaf -- see that call site's doc comment).
    /// `anchor_pane_id` identifies *which* leaf this registration belongs
    /// to (its selected/anchor pane); `event.bounds` is that same leaf's
    /// own on-screen bounds this frame, and `event.event.position` is the
    /// live cursor position -- both handed to us fresh by gpui on every
    /// mouse-move while a `TabDragPayload` drag is active, regardless of
    /// whether the pointer is actually over *this* particular leaf (every
    /// leaf's registration fires on every move -- see `DragMoveEvent`'s doc
    /// comment), hence the explicit bounds-contains check below rather than
    /// relying on hit-testing.
    ///
    /// Mirrors `PaneFrameView.update(_:)`'s "same-leaf, meaningless-edge"
    /// guard: dropping a tab onto its own group's center (already merged)
    /// or onto its own group's edge when it's the group's only tab is a
    /// no-op, so those cases clear the highlight instead of showing one
    /// (matching Swift's `highlight.isHidden = true` early return there).
    pub(crate) fn update_pane_drag_hover(
        &mut self,
        task_id: &str,
        anchor_pane_id: PaneId,
        leaf_pane_ids: &[PaneId],
        event: &DragMoveEvent<TabDragPayload>,
        cx: &mut Context<Self>,
    ) {
        let local_x = f32::from(event.event.position.x) - f32::from(event.bounds.origin.x);
        let local_y = f32::from(event.event.position.y) - f32::from(event.bounds.origin.y);
        let width = f32::from(event.bounds.size.width);
        let height = f32::from(event.bounds.size.height);
        let within = local_x >= 0.0 && local_y >= 0.0 && local_x <= width && local_y <= height;

        let Some(workspace) = self.workspaces.get_mut(task_id) else {
            return;
        };

        if !within {
            let hovering_this_leaf = matches!(
                workspace.pane_drag_hover,
                Some(hover) if hover.target_pane_id == anchor_pane_id
            );
            if hovering_this_leaf {
                workspace.pane_drag_hover = None;
                cx.notify();
            }
            return;
        }

        let edge = labolabo_core::drop_edge_for_point(width, height, local_x, local_y);
        let source_id = event
            .dragged_item()
            .downcast_ref::<TabDragPayload>()
            .map(|payload| payload.source_pane_id);
        let meaningless = match source_id {
            Some(source_id) => {
                leaf_pane_ids.contains(&source_id)
                    && (edge == DropEdge::Center || leaf_pane_ids.len() == 1)
            }
            None => false,
        };
        let new_hover = if meaningless {
            None
        } else {
            Some(PaneDragHover {
                target_pane_id: anchor_pane_id,
                edge,
            })
        };
        if workspace.pane_drag_hover != new_hover {
            workspace.pane_drag_hover = new_hover;
            cx.notify();
        }
    }

    /// Completes a tab-chip drop onto `anchor_pane_id`'s leaf: resolves the
    /// drop edge from whatever `update_pane_drag_hover` last computed for
    /// this leaf (falling back to `DropEdge::Center` -- a plain merge -- if
    /// somehow no hover was recorded, e.g. a drop that arrives without a
    /// preceding move event), then delegates to
    /// `PaneTilingModel::move_pane` -- the same core op `plans/012`'s brief
    /// says the UI must not reimplement. Mirrors
    /// `TilingCoordinator.handleDrop`: only steals keyboard focus for the
    /// moved tab when it actually moved *and* is a terminal pane (matching
    /// Swift's "端末なら" condition -- moving a diff/files/commits pane
    /// shouldn't yank focus away from whatever terminal the user was
    /// typing into).
    pub(crate) fn finish_pane_drag_drop(
        &mut self,
        task_id: &str,
        anchor_pane_id: PaneId,
        payload: &TabDragPayload,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let edge = self
            .workspaces
            .get(task_id)
            .and_then(|w| w.pane_drag_hover)
            .filter(|h| h.target_pane_id == anchor_pane_id)
            .map(|h| h.edge)
            .unwrap_or(DropEdge::Center);

        let Some(workspace) = self.workspaces.get_mut(task_id) else {
            return;
        };
        workspace.pane_drag_hover = None;
        let moved = workspace
            .model
            .move_pane(payload.source_pane_id, anchor_pane_id, edge);
        if moved {
            let is_terminal = workspace
                .model
                .panes()
                .into_iter()
                .find(|p| p.id == payload.source_pane_id)
                .map(|p| p.kind == PaneKind::Terminal)
                .unwrap_or(false);
            if is_terminal {
                workspace.focused_pane = payload.source_pane_id;
            }
        }
        window.focus(&self.focus_handle);
        self.persist_workspace(task_id);
        cx.notify();
    }

    /// Live-updates a pane-divider's ratio as it's dragged (`plans` W5j
    /// #2): derives the new ratio from the drag's current pointer position
    /// against `event.bounds` (the split container's own bounds -- see
    /// `task_workspace::render_tile`'s `container.on_drag_move::
    /// <DividerDragPayload>` wiring for why this, rather than the thin
    /// divider handle's own bounds, is what's registered here) via
    /// `grid::ratio_from_drag_position` along whichever axis
    /// `event.dragged_item()`'s `orientation` says the split runs, then
    /// applies it (clamped) through `PaneTilingModel::set_split_ratio`.
    /// Sets [`Self::divider_drag_active`] `true` on every call (harmless
    /// to repeat) so every terminal pane's canvas suppresses `Terminal::
    /// resize` for the duration -- see `render_leaf`'s `prepaint` closure.
    /// Deliberately does not persist -- ratio changes are cheap, per-frame,
    /// in-memory-only updates during the drag; [`Self::
    /// finish_divider_drag`] persists once, on drop, mirroring
    /// `PaneTilingModel::set_split_ratio`'s own doc comment (which mirrors
    /// the Swift source's original design for this exact reason).
    pub(crate) fn update_divider_drag(
        &mut self,
        task_id: &str,
        event: &DragMoveEvent<task_workspace::DividerDragPayload>,
        cx: &mut Context<Self>,
    ) {
        let payload = *event.drag(cx);
        self.divider_drag_active = true;

        let local_x = f32::from(event.event.position.x - event.bounds.origin.x);
        let local_y = f32::from(event.event.position.y - event.bounds.origin.y);
        let ratio = match payload.orientation {
            TileOrientation::Horizontal => {
                grid::ratio_from_drag_position(local_x, f32::from(event.bounds.size.width))
            }
            TileOrientation::Vertical => {
                grid::ratio_from_drag_position(local_y, f32::from(event.bounds.size.height))
            }
        };

        let Some(workspace) = self.workspaces.get_mut(task_id) else {
            return;
        };
        if workspace.model.set_split_ratio(payload.node_id, ratio) {
            cx.notify();
        }
    }

    /// Finishes a pane-divider drag (`plans` W5j #2) on drop: clears
    /// [`Self::divider_drag_active`] (letting every terminal pane's canvas
    /// resume normal `Terminal::resize`-on-bounds-change behavior, applying
    /// the drag's final size in one shot -- see `render_leaf`'s `prepaint`
    /// closure) and persists the Task's layout once (mirroring
    /// `finish_pane_drag_drop`'s own "persist once, on drop" shape --
    /// `update_divider_drag` above never persists, matching
    /// `PaneTilingModel::set_split_ratio`'s documented design). The dragged
    /// node's own ratio needs no further action here -- it was already
    /// applied live, during the drag, by `update_divider_drag`.
    pub(crate) fn finish_divider_drag(&mut self, task_id: &str, cx: &mut Context<Self>) {
        self.divider_drag_active = false;
        self.persist_workspace(task_id);
        cx.notify();
    }

    /// Completes an OS file/folder drop onto `anchor_pane_id`'s terminal
    /// pane (`plans/012` §3.1): shell-quotes and space-joins every dropped
    /// path (`labolabo_core::quote_dropped_paths`) and writes the
    /// resulting text directly to the pane's PTY -- no newline, so nothing
    /// runs until the user presses Enter themselves (§3.1). A silent no-op
    /// if the leaf's anchor pane isn't a terminal (shouldn't happen --
    /// `render_leaf`'s `can_drop` already restricts the drop to terminal
    /// leaves -- but this is cheap, load-bearing insurance against acting
    /// on a non-terminal pane if that guard is ever loosened) or has no
    /// live runtime yet.
    pub(crate) fn handle_file_drop(
        &mut self,
        task_id: &str,
        anchor_pane_id: PaneId,
        paths: &ExternalPaths,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.workspaces.get(task_id) else {
            return;
        };
        let is_terminal = workspace
            .model
            .panes()
            .into_iter()
            .any(|p| p.id == anchor_pane_id && p.kind == PaneKind::Terminal);
        if !is_terminal {
            return;
        }
        let Some(runtime) = workspace.runtimes.get(&anchor_pane_id) else {
            return;
        };
        let path_strings: Vec<String> = paths
            .paths()
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        let text = quote_dropped_paths(&path_strings);
        if !text.is_empty() {
            runtime.session.write_input(text.as_bytes());
        }
        cx.notify();
    }

    // MARK: - sidebar drag & drop (`plans/012` §3: Task reorder, folder drop)

    /// Reorders the sidebar's Task list (`crate::sidebar`'s row DnD):
    /// dragging `moved_id` to just before `before_id` within its repo
    /// group (see `labolabo_core::reorder_task_ids`'s doc comment for the
    /// exact rule -- cross-repo drops and self-drops are no-ops).
    /// Renumbers every Task's `sort_order` densely (0, 1, 2, ...) in the
    /// new order and persists each row -- simpler than trying to preserve
    /// fractional gaps, and `sort_order`'s only contract is relative
    /// order, not specific values.
    pub(crate) fn reorder_tasks_in_sidebar(
        &mut self,
        moved_id: String,
        before_id: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let new_order = reorder_task_ids(&self.tasks, &moved_id, before_id.as_deref());
        let unchanged = new_order
            .iter()
            .zip(self.tasks.iter())
            .all(|(id, task)| id == &task.id);
        if unchanged {
            return;
        }

        let mut reordered = Vec::with_capacity(self.tasks.len());
        for id in &new_order {
            if let Some(pos) = self.tasks.iter().position(|t| &t.id == id) {
                reordered.push(self.tasks.remove(pos));
            }
        }
        for (index, task) in reordered.iter_mut().enumerate() {
            task.sort_order = index as i64;
            if let Err(err) = self.db.upsert_task(task) {
                eprintln!("labolabo-app: failed to persist task order: {err}");
            }
        }
        self.tasks = reordered;
        cx.notify();
    }

    /// A folder dropped on the sidebar (`plans/012` §3: "フォルダをサイド
    /// バー/ウィンドウへドロップ → ... 「新しい作業」の開始"): starts a
    /// new `attached`-kind Task at that directory, exactly like
    /// "+ Attached"'s tail (`finish_new_attached_task`) but skipping the
    /// file-picker (the directory is already known). Every *directory*
    /// among the dropped paths becomes its own Task (multi-drop support);
    /// plain files are silently skipped (§3 only specifies folder drops
    /// here -- dropping a bare file onto the sidebar has no defined
    /// meaning). No confirmation UI (deferred per this wave's brief -- see
    /// the crate README's TODO list): a dropped folder just becomes a
    /// Task, matching "+ Attached"'s own no-confirmation flow.
    pub(crate) fn handle_sidebar_folder_drop(
        &mut self,
        paths: &ExternalPaths,
        cx: &mut Context<Self>,
    ) {
        let dirs: Vec<std::path::PathBuf> = paths
            .paths()
            .iter()
            .filter(|p| p.is_dir())
            .cloned()
            .collect();
        for dir in dirs {
            cx.spawn(async move |this, cx| {
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
    ///
    /// Every call also snaps that pane's viewport back to the live tail
    /// (`Terminal::scroll_to_bottom`) -- the terminal-UI convention this
    /// app follows: typing while scrolled back returns you to the live
    /// output, matching every mainstream terminal. This is the single
    /// choke point both `key_down`'s direct keystroke bytes and the
    /// `EntityInputHandler` impl's IME-committed text (`replace_text_in_
    /// range`, below) already write through, so one change here covers
    /// both input paths.
    fn write_focused_pane_input(&self, bytes: &[u8]) -> bool {
        if let Some(runtime) = self.focused_pane_runtime() {
            runtime.session.write_input(bytes);
            runtime.session.scroll_to_bottom();
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

    // MARK: - text selection, mouse reporting & wheel scroll
    // (`task_workspace::render_leaf`'s mouse handlers on a leaf's canvas --
    // W5j widened this section from local text-selection-only to also
    // cover SGR mouse-report forwarding, see `crate::mouse_report`'s module
    // doc comment for the overall design and its scope limits)

    /// Convert a window-space mouse `position` into the `(col, row)` cell it
    /// falls on within `pane_id`'s canvas -- `None` if the pane has no live
    /// runtime. Shared by every mouse handler below: all of them need "what
    /// cell is the mouse over right now," whether for local selection or
    /// for SGR-encoding a report.
    fn pane_cell_at(
        &self,
        task_id: &str,
        pane_id: PaneId,
        position: Point<Pixels>,
    ) -> Option<CellPos> {
        let runtime = self.workspaces.get(task_id)?.runtimes.get(&pane_id)?;
        let snapshot = runtime.session.snapshot();
        let bounds = runtime.last_bounds.get();
        let local_x = f32::from(position.x - bounds.origin.x);
        let local_y = f32::from(position.y - bounds.origin.y);
        let (col, row) = grid::cell_at(
            local_x,
            local_y,
            self.spec.cell_width,
            self.spec.cell_height,
            snapshot.cols,
            snapshot.rows,
        );
        Some(CellPos { row, col })
    }

    /// Begin (or restart) a left-button gesture in `pane_id`'s canvas at
    /// `event.position`: either starts SGR-encoding it and forwarding to
    /// the child program's PTY (when the child has requested mouse
    /// tracking and Shift isn't held -- see `mouse_report::
    /// is_click_reporting_active`), or begins local text selection (the
    /// pre-existing behavior, now click-count-aware -- a double-click
    /// selects the word under the mouse, a triple-click the whole line;
    /// see `selection::selection_for_click`). Which of the two applies is
    /// decided once here and held fixed for the rest of this one gesture
    /// (see `PaneRuntime::reporting_drag`'s doc comment for why). Called
    /// alongside (not instead of) `select_pane` from `render_leaf`'s
    /// mouse-down handler, so starting a gesture also focuses the pane
    /// it's in, same as before either behavior existed.
    pub(crate) fn begin_selection(
        &mut self,
        task_id: &str,
        pane_id: PaneId,
        event: &MouseDownEvent,
        cx: &mut Context<Self>,
    ) {
        let Some(pos) = self.pane_cell_at(task_id, pane_id, event.position) else {
            return;
        };
        let Some(runtime) = self
            .workspaces
            .get_mut(task_id)
            .and_then(|w| w.runtimes.get_mut(&pane_id))
        else {
            return;
        };
        let mouse_mode = runtime.session.mouse_mode();
        let reporting = mouse_report::is_click_reporting_active(mouse_mode, event.modifiers.shift);
        runtime.reporting_drag = reporting;
        if reporting {
            runtime.selection = None;
            if let Some(bytes) = mouse_report::encode_sgr(
                mouse_mode.tracking,
                MouseAction::Press,
                Some(MouseButtonKind::Left),
                mouse_mods(event.modifiers),
                pos.col,
                pos.row,
            ) {
                runtime.session.write_input(&bytes);
            }
        } else {
            let snapshot = runtime.session.snapshot();
            runtime.selection = Some(selection::selection_for_click(
                &snapshot,
                pos,
                event.click_count,
            ));
        }
        cx.notify();
    }

    /// Extend `pane_id`'s in-progress left-button gesture as the mouse
    /// moves while the button is held: either SGR-encodes and forwards a
    /// motion report (when [`Self::begin_selection`] started this gesture
    /// as a mouse-report one, and the child's tracking mode reports motion
    /// -- `Button`/`Any`; a `Normal`-tracking gesture reports nothing
    /// between press and release, matching `mouse_report::should_report`),
    /// or extends the local text selection cell-by-cell (the pre-existing
    /// behavior -- continuing to drag after a double/triple-click does
    /// *not* re-snap to word/line boundaries, a documented simplification,
    /// see `selection::selection_for_click`'s doc comment). Called from
    /// `render_leaf`'s mouse-move handler only while the left button is
    /// held (`MouseMoveEvent::dragging()`).
    pub(crate) fn extend_selection(
        &mut self,
        task_id: &str,
        pane_id: PaneId,
        event: &MouseMoveEvent,
        cx: &mut Context<Self>,
    ) {
        let Some(pos) = self.pane_cell_at(task_id, pane_id, event.position) else {
            return;
        };
        let Some(runtime) = self
            .workspaces
            .get_mut(task_id)
            .and_then(|w| w.runtimes.get_mut(&pane_id))
        else {
            return;
        };
        if runtime.reporting_drag {
            let mouse_mode = runtime.session.mouse_mode();
            if let Some(bytes) = mouse_report::encode_sgr(
                mouse_mode.tracking,
                MouseAction::Motion,
                Some(MouseButtonKind::Left),
                mouse_mods(event.modifiers),
                pos.col,
                pos.row,
            ) {
                runtime.session.write_input(&bytes);
            }
            return;
        }
        let Some(selection) = runtime.selection.as_mut() else {
            return;
        };
        selection.cursor = pos;
        cx.notify();
    }

    /// Finish `pane_id`'s left-button gesture on mouse-up: either
    /// SGR-encodes and forwards a release report (finishing a mouse-report
    /// gesture -- `mouse_report::encode_sgr` itself is the source of truth
    /// for whether a release is reportable, so it isn't re-checked
    /// separately here), or finishes local text selection (the
    /// pre-existing behavior: a selection that never grew past its
    /// zero-length starting point -- a plain click, no drag -- is cleared
    /// outright, so an ordinary click-to-focus never leaves a stray
    /// "selected" highlight or blocks a later Cmd+C with an empty range).
    pub(crate) fn finish_selection(
        &mut self,
        task_id: &str,
        pane_id: PaneId,
        event: &MouseUpEvent,
        cx: &mut Context<Self>,
    ) {
        let Some(pos) = self.pane_cell_at(task_id, pane_id, event.position) else {
            return;
        };
        let Some(runtime) = self
            .workspaces
            .get_mut(task_id)
            .and_then(|w| w.runtimes.get_mut(&pane_id))
        else {
            return;
        };
        if runtime.reporting_drag {
            runtime.reporting_drag = false;
            let mouse_mode = runtime.session.mouse_mode();
            if let Some(bytes) = mouse_report::encode_sgr(
                mouse_mode.tracking,
                MouseAction::Release,
                Some(MouseButtonKind::Left),
                mouse_mods(event.modifiers),
                pos.col,
                pos.row,
            ) {
                runtime.session.write_input(&bytes);
            }
            return;
        }
        if runtime.selection.is_some_and(|s| s.is_empty()) {
            runtime.selection = None;
            cx.notify();
        }
    }

    /// Reports a right- or middle-button press to `pane_id`'s child
    /// program via SGR mouse encoding, if it has requested mouse tracking
    /// and Shift isn't held (`mouse_report::is_click_reporting_active`) --
    /// a silent no-op otherwise. Right/middle clicks have no *local*
    /// behavior in this app (no context menu, no paste-on-middle-click,
    /// unlike the left button's text-selection fallback), so there is
    /// nothing else for this to fall back to.
    pub(crate) fn report_mouse_click(
        &mut self,
        task_id: &str,
        pane_id: PaneId,
        button: MouseButtonKind,
        event: &MouseDownEvent,
        cx: &mut Context<Self>,
    ) {
        let Some(pos) = self.pane_cell_at(task_id, pane_id, event.position) else {
            return;
        };
        let Some(runtime) = self
            .workspaces
            .get_mut(task_id)
            .and_then(|w| w.runtimes.get_mut(&pane_id))
        else {
            return;
        };
        let mouse_mode = runtime.session.mouse_mode();
        if !mouse_report::is_click_reporting_active(mouse_mode, event.modifiers.shift) {
            return;
        }
        let Some(bytes) = mouse_report::encode_sgr(
            mouse_mode.tracking,
            MouseAction::Press,
            Some(button),
            mouse_mods(event.modifiers),
            pos.col,
            pos.row,
        ) else {
            return;
        };
        runtime.session.write_input(&bytes);
        if runtime.selection.take().is_some() {
            cx.notify();
        }
    }

    /// Reports a right- or middle-button release -- the release-side
    /// counterpart to [`Self::report_mouse_click`]. No local state to
    /// notify gpui about either way, so this takes no `cx`.
    pub(crate) fn report_mouse_release(
        &mut self,
        task_id: &str,
        pane_id: PaneId,
        button: MouseButtonKind,
        event: &MouseUpEvent,
    ) {
        let Some(pos) = self.pane_cell_at(task_id, pane_id, event.position) else {
            return;
        };
        let Some(runtime) = self
            .workspaces
            .get_mut(task_id)
            .and_then(|w| w.runtimes.get_mut(&pane_id))
        else {
            return;
        };
        let mouse_mode = runtime.session.mouse_mode();
        if !mouse_report::is_click_reporting_active(mouse_mode, event.modifiers.shift) {
            return;
        }
        if let Some(bytes) = mouse_report::encode_sgr(
            mouse_mode.tracking,
            MouseAction::Release,
            Some(button),
            mouse_mods(event.modifiers),
            pos.col,
            pos.row,
        ) {
            runtime.session.write_input(&bytes);
        }
    }

    /// Route one wheel/trackpad scroll event over `pane_id`'s canvas.
    /// Three cases, checked in this priority order (bug report W5j #4: a
    /// mouse-aware alt-screen TUI, e.g. Claude Code's own TUI, was
    /// receiving arrow-key sequences from a scroll gesture and warning
    /// "use PgUp/PgDn to scroll" -- its own mouse tracking was simply never
    /// consulted before falling into the alt-screen branch):
    ///
    /// 1. **The child has requested mouse tracking** (`Terminal::
    ///    mouse_mode`, regardless of alt/primary screen): SGR-encodes each
    ///    whole accumulated line as a wheel-up/down press report and
    ///    forwards it, clearing any local selection -- mirrors real
    ///    Ghostty's own `Surface.scrollCallback` (confirmed by reading the
    ///    vendored source): "If we have an active mouse reporting mode,
    ///    clear the selection... then report mouse events" takes priority
    ///    over both of the other two cases below. Unlike the click/drag
    ///    path, **not** overridden by Shift -- see `mouse_report::
    ///    is_scroll_reporting_active`'s doc comment.
    /// 2. **No mouse tracking, alternate screen active, and alternate
    ///    scroll mode (DECSET `1007`) also active** (`Terminal::
    ///    alternate_scroll_active`, which defaults to `true` -- see its own
    ///    doc comment): converts the accumulated line delta into that many
    ///    Up/Down cursor-key escape sequences written straight to the PTY
    ///    -- the pre-existing behavior for `vim`/`less`/`htop`-style
    ///    programs that manage their own internal scrolling and haven't
    ///    requested real mouse events.
    /// 3. **Neither**: scrolls that pane's own `Terminal` viewport
    ///    (`Terminal::scroll`) -- this crate's own scrollback.
    pub(crate) fn handle_pane_scroll(
        &mut self,
        task_id: &str,
        pane_id: PaneId,
        event: &ScrollWheelEvent,
        cx: &mut Context<Self>,
    ) {
        let cell_height = self.spec.cell_height;
        let Some(pos) = self.pane_cell_at(task_id, pane_id, event.position) else {
            return;
        };
        let Some(runtime) = self
            .workspaces
            .get_mut(task_id)
            .and_then(|w| w.runtimes.get_mut(&pane_id))
        else {
            return;
        };
        let pixel_delta = event.delta.pixel_delta(px(cell_height));
        let delta_y = f32::from(pixel_delta.y);
        let lines =
            grid::accumulate_scroll_lines(&mut runtime.pending_scroll, delta_y, cell_height);
        if lines == 0 {
            return;
        }

        let mouse_mode = runtime.session.mouse_mode();
        if mouse_report::is_scroll_reporting_active(mouse_mode) {
            runtime.selection = None;
            let button = if lines > 0 {
                MouseButtonKind::WheelUp
            } else {
                MouseButtonKind::WheelDown
            };
            let mods = mouse_mods(event.modifiers);
            let mut bytes = Vec::new();
            for _ in 0..lines.unsigned_abs() {
                if let Some(encoded) = mouse_report::encode_sgr(
                    mouse_mode.tracking,
                    MouseAction::Press,
                    Some(button),
                    mods,
                    pos.col,
                    pos.row,
                ) {
                    bytes.extend_from_slice(&encoded);
                }
            }
            if !bytes.is_empty() {
                runtime.session.write_input(&bytes);
            }
            cx.notify();
            return;
        }

        if runtime.session.alt_screen_active() && runtime.session.alternate_scroll_active() {
            // Up arrow for "scroll up" (positive lines, our shared
            // convention -- see `VtBackend::scroll_display`'s doc
            // comment), Down arrow otherwise. Same normal-mode `ESC[A`/
            // `ESC[B` sequences `keys::keystroke_to_bytes` already sends
            // for a literal arrow-key press -- this app doesn't track
            // DECCKM (application cursor-key mode) for either path.
            let seq: &[u8] = if lines > 0 { b"\x1b[A" } else { b"\x1b[B" };
            let bytes = seq.repeat(lines.unsigned_abs() as usize);
            runtime.session.write_input(&bytes);
        } else {
            runtime.session.scroll(lines);
        }
        cx.notify();
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
    /// currently focused pane is a silent no-op.
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

    /// Cmd+C: copies the focused pane's current text selection (if any) to
    /// the system clipboard -- `selection::selected_text`'s extraction over
    /// that pane's live snapshot. A no-op when there is no selection, or the
    /// selection is empty (`Selection::is_empty`, e.g. a plain click that
    /// never got extended -- see `finish_selection`, which already clears
    /// those), or the extracted text happens to be empty. Deliberately does
    /// **not** touch the pane's PTY at all -- `Ctrl+C` (a bare control byte
    /// via `keys::keystroke_to_bytes`) is the only way to send `SIGINT`;
    /// `Cmd+C` and `Ctrl+C` are different keystrokes entirely (`keys.rs`
    /// reserves every `platform`-modified keystroke for application
    /// shortcuts like this one, so there is no ambiguity to resolve here).
    /// The selection itself is left exactly as it was -- copying doesn't
    /// clear it, matching every mainstream terminal's convention of leaving
    /// the highlight in place after a copy.
    fn action_copy(&mut self, _: &Copy, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(runtime) = self.focused_pane_runtime() else {
            return;
        };
        let Some(selection) = runtime.selection else {
            return;
        };
        if selection.is_empty() {
            return;
        }
        let snapshot = runtime.session.snapshot();
        let text = selection::selected_text(&snapshot, &selection);
        if text.is_empty() {
            return;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(text));
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

    // MARK: - Git pane (`crate::git_pane` -- see its module doc comment for
    // the fixed-right-pane design and the refresh/watch lifecycle this
    // section wires up)

    fn action_toggle_git_pane(
        &mut self,
        _: &ToggleGitPane,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(task_id) = self.selected_task_id.clone() else {
            return;
        };
        self.set_git_pane_visible(
            &task_id,
            !self
                .workspaces
                .get(&task_id)
                .map(|w| w.git.visible)
                .unwrap_or(true),
            cx,
        );
    }

    /// Shows/hides `task_id`'s Git pane, starting or stopping its
    /// `FileWatcher` to match (see [`Self::activate_git_pane`]/
    /// [`Self::deactivate_git_pane`]) -- called by both the `Cmd+Shift+G`
    /// action and the pane's own header "×" close button.
    pub(crate) fn set_git_pane_visible(
        &mut self,
        task_id: &str,
        visible: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.workspaces.get_mut(task_id) else {
            return;
        };
        if workspace.git.visible == visible {
            return;
        }
        workspace.git.visible = visible;
        cx.notify();
        if visible {
            self.activate_git_pane(task_id, cx);
        } else {
            self.deactivate_git_pane(task_id);
        }
    }

    /// Starts `task_id`'s Git pane watching (if it's visible and isn't
    /// already) and kicks off an initial refresh -- a no-op if the Task has
    /// no loaded workspace, is already watching, or is currently hidden.
    /// Called whenever `task_id` becomes the *selected* Task (`select_task`/
    /// `LaboLaboApp::new`/`add_task_and_select`) and when its pane is made
    /// visible ([`Self::set_git_pane_visible`]) -- see `crate::git_pane`'s
    /// module doc comment for why this is scoped to the selected Task only.
    pub(crate) fn activate_git_pane(&mut self, task_id: &str, cx: &mut Context<Self>) {
        let Some(workspace) = self.workspaces.get(task_id) else {
            return;
        };
        if !workspace.git.visible || workspace.git.is_watching() {
            return;
        }
        let Some(task) = self.tasks.iter().find(|t| t.id == task_id) else {
            return;
        };
        let working_directory = PathBuf::from(task.working_directory());

        if let Some(handle) =
            git_pane::spawn_git_watch_bridge(task_id.to_string(), working_directory, cx)
        {
            if let Some(workspace) = self.workspaces.get_mut(task_id) {
                workspace.git.attach_watch(handle);
            }
        }
        self.request_git_refresh(task_id, cx);
    }

    /// Stops `task_id`'s Git pane watching, if it was -- called when
    /// switching away from the selected Task, or hiding its pane. Cached
    /// status/items/diff are left in place so re-activating shows the last-
    /// known snapshot immediately, refreshed shortly after.
    pub(crate) fn deactivate_git_pane(&mut self, task_id: &str) {
        if let Some(workspace) = self.workspaces.get_mut(task_id) {
            workspace.git.detach_watch();
        }
    }

    /// A file row was clicked in `task_id`'s Git pane: selects it (picking
    /// the default Diff/Whole view per `GitPaneState::select`'s rule) and
    /// kicks off a refresh to fetch that file's diff/whole-file contents
    /// (the currently cached snapshot may not have them yet, or they may be
    /// stale for a just-changed file).
    pub(crate) fn select_git_file(&mut self, task_id: &str, path: String, cx: &mut Context<Self>) {
        let Some(workspace) = self.workspaces.get_mut(task_id) else {
            return;
        };
        workspace.git.select(path);
        cx.notify();
        self.request_git_refresh(task_id, cx);
    }

    /// The Diff/Whole pill toggle: no fetch needed -- both are already kept
    /// in sync on every refresh (see `crate::git_pane`'s module doc
    /// comment's "Diff ⇄ Whole file" section).
    pub(crate) fn set_git_view_mode(
        &mut self,
        task_id: &str,
        mode: FileViewMode,
        cx: &mut Context<Self>,
    ) {
        if let Some(workspace) = self.workspaces.get_mut(task_id) {
            if workspace.git.view_mode != mode {
                workspace.git.view_mode = mode;
                cx.notify();
            }
        }
    }

    /// Kicks off (or coalesces into an in-flight one, see
    /// `GitPaneState::begin_refresh`) a background refresh of `task_id`'s
    /// Git pane. The actual `git status`/`numstat`/`diff`/file-read calls
    /// (`git_pane::compute_git_snapshot`) run on gpui's background thread
    /// pool (`cx.background_spawn`), never this (UI) thread, per this
    /// wave's brief.
    pub(crate) fn request_git_refresh(&mut self, task_id: &str, cx: &mut Context<Self>) {
        let Some(workspace) = self.workspaces.get_mut(task_id) else {
            return;
        };
        if !workspace.git.begin_refresh() {
            return; // coalesced into the refresh already in flight
        }
        let Some(task) = self.tasks.iter().find(|t| t.id == task_id) else {
            // Task vanished mid-flight (shouldn't happen in practice) --
            // undo the flag so a future call isn't coalesced forever
            // against a refresh that will never complete.
            if let Some(workspace) = self.workspaces.get_mut(task_id) {
                workspace.git.finish_refresh();
            }
            return;
        };
        let working_directory = PathBuf::from(task.working_directory());
        let selected_path = workspace.git.selected_path.clone();
        let task_id = task_id.to_string();

        cx.spawn(async move |this, cx| {
            let snapshot = cx
                .background_spawn(async move {
                    git_pane::compute_git_snapshot(&working_directory, selected_path.as_deref())
                })
                .await;
            let _ = this.update(cx, |app, cx| app.apply_git_refresh(&task_id, snapshot, cx));
        })
        .detach();
    }

    /// A background refresh (`Self::request_git_refresh`) completed:
    /// applies its snapshot and, if another trigger arrived while it was in
    /// flight, immediately spawns exactly one more (`GitPaneState::
    /// finish_refresh`'s coalescing contract). Also updates
    /// `changed_files_cache` for `task_id` -- see [`Self::task_conflicts`]
    /// and that field's doc comment for why this is the *only* place the
    /// cache is written (today: only ever the selected Task, since that's
    /// the only Task whose Git pane ever refreshes).
    fn apply_git_refresh(&mut self, task_id: &str, snapshot: GitSnapshot, cx: &mut Context<Self>) {
        let Some(workspace) = self.workspaces.get_mut(task_id) else {
            return;
        };
        let changed = snapshot.status.as_ref().map(git_pane::changed_paths);
        workspace.git.apply(snapshot);
        let refresh_again = workspace.git.finish_refresh();
        match changed {
            Some(paths) => {
                self.changed_files_cache.insert(task_id.to_string(), paths);
            }
            None => {
                // `git status` itself failed this round (e.g. an
                // `attached`-kind Task whose directory isn't a repo) --
                // drop any stale entry rather than keeping it forever
                // (Swift's `refreshChangedFiles` does the same: a Task
                // that no longer needs conflict tracking gets its record
                // removed, not left stale).
                self.changed_files_cache.remove(task_id);
            }
        }
        cx.notify();
        if refresh_again {
            self.request_git_refresh(task_id, cx);
        }
    }

    /// The [`cross_session_conflicts::Conflict`]s for `task_id`: paths it
    /// has changed that at least one other Task in the *same* repo
    /// (`Task::repo_key`) has also changed, per the last-fetched Git status
    /// cached for each (`changed_files_cache`) -- see that field's doc
    /// comment for the "only status-fetched Tasks participate" limitation
    /// this wave's brief explicitly accepts. Thin wrapper around the pure,
    /// unit-tested [`compute_task_conflicts`] -- kept separate so the
    /// conflict-matching logic itself doesn't need a real `LaboLaboApp`
    /// (gpui `Context`/window) to test.
    pub(crate) fn task_conflicts(&self, task_id: &str) -> Vec<cross_session_conflicts::Conflict> {
        compute_task_conflicts(&self.tasks, &self.changed_files_cache, task_id)
    }

    // MARK: - Settings (`crate::settings` -- Cmd+, overlay, `plans` wave 5i §3)

    fn action_toggle_settings(
        &mut self,
        _: &ToggleSettings,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.settings_open = !self.settings_open;
        cx.notify();
    }

    // MARK: - メニューバー用アクション (wave 6c §1, `crate::menus`)。
    // `Quit` はウィンドウ非依存なので `main.rs` のグローバル
    // `cx.on_action` 側（`cx.quit()`）。

    fn action_about(&mut self, _: &About, _window: &mut Window, cx: &mut Context<Self>) {
        self.about_open = true;
        cx.notify();
    }

    fn action_new_attached_task(
        &mut self,
        _: &NewAttachedTask,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.start_new_attached_task(window, cx);
    }

    fn action_new_worktree_task(
        &mut self,
        _: &NewWorktreeTask,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.start_new_worktree_task(window, cx);
    }

    fn action_minimize_window(
        &mut self,
        _: &MinimizeWindow,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        window.minimize_window();
    }

    fn action_zoom_window(&mut self, _: &ZoomWindow, window: &mut Window, _cx: &mut Context<Self>) {
        window.zoom_window();
    }

    /// 「ファイル → 選択中の作業を IDE で開く」: タスク行「…」メニューの
    /// エディタ列挙の簡易版で、検出済みエディタの**先頭 1 つ**で開く
    /// （メニューは起動時に静的に組むため動的なサブメニューにしない判断 --
    /// `crate::menus` の doc コメント参照）。エディタ未検出なら Finder で
    /// 表示へフォールバック。
    fn action_open_selected_in_ide(
        &mut self,
        _: &OpenSelectedInIde,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(task_id) = self.selected_task_id.clone() else {
            return;
        };
        match self.installed_editors().first() {
            Some(editor) => {
                let bundle_id = editor.bundle_id;
                self.open_task_in_editor(&task_id, bundle_id, cx);
            }
            None => self.reveal_task_in_finder(&task_id, cx),
        }
    }

    pub(crate) fn close_settings(&mut self, cx: &mut Context<Self>) {
        if !self.settings_open {
            return;
        }
        self.settings_open = false;
        cx.notify();
    }

    /// Toggles "Claude セッションの自動 resume" and persists it immediately
    /// (`TaskDatabase::set_auto_resume_enabled`) -- takes effect on the
    /// *next* pane spawn (`spawn_runtime_for_task`'s `auto_resume_enabled`
    /// gate reads `self.settings` fresh every call), not retroactively for
    /// already-running panes.
    pub(crate) fn set_auto_resume_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.settings.auto_resume_enabled == enabled {
            return;
        }
        self.settings.auto_resume_enabled = enabled;
        if let Err(err) = self.db.set_auto_resume_enabled(enabled) {
            eprintln!("labolabo-app: failed to persist auto_resume_enabled: {err}");
        }
        cx.notify();
    }

    /// Toggles "Git ペインを既定で表示" and persists it -- seeds `GitPaneState::
    /// visible` for every Task workspace loaded *after* this call
    /// (`ensure_workspace_loaded` reads `self.settings.git_pane_default_visible`
    /// fresh); already-loaded workspaces are unaffected (use `Cmd+Shift+G`
    /// or the pane's own close button for those, same as before this
    /// setting existed).
    pub(crate) fn set_git_pane_default_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if self.settings.git_pane_default_visible == visible {
            return;
        }
        self.settings.git_pane_default_visible = visible;
        if let Err(err) = self.db.set_git_pane_default_visible(visible) {
            eprintln!("labolabo-app: failed to persist git_pane_default_visible: {err}");
        }
        cx.notify();
    }

    /// Steps `self.settings.scrollback_lines` by `delta` (the settings
    /// panel's -/+ buttons pass `±settings::SCROLLBACK_STEP`), clamped by
    /// `settings::adjust_scrollback_lines`, and persists the result. Takes
    /// effect at the *next* pane spawn only (`spawn_runtime_for_task`'s
    /// `scrollback_lines` capture) -- a live VT core's history buffer isn't
    /// resizable, matching the settings panel's own footer copy.
    pub(crate) fn adjust_scrollback_lines(&mut self, delta: i64, cx: &mut Context<Self>) {
        let next = settings::adjust_scrollback_lines(self.settings.scrollback_lines, delta);
        if next == self.settings.scrollback_lines {
            return;
        }
        self.settings.scrollback_lines = next;
        if let Err(err) = self.db.set_scrollback_lines(next) {
            eprintln!("labolabo-app: failed to persist scrollback_lines: {err}");
        }
        cx.notify();
    }
}

/// Pure conflict computation behind [`LaboLaboApp::task_conflicts`]: builds
/// one [`cross_session_conflicts::Session`] per Task (its `repo_key` and
/// whatever `changed_files` has cached for it -- an empty set for a Task
/// never fetched, which `cross_session_conflicts::conflicts` already
/// treats as "no conflicts from this Task's own side" and, symmetrically,
/// never causes *another* Task to see a conflict against it either), then
/// delegates to `labolabo_core::cross_session_conflicts::conflicts`.
fn compute_task_conflicts(
    tasks: &[Task],
    changed_files: &HashMap<String, HashSet<String>>,
    task_id: &str,
) -> Vec<cross_session_conflicts::Conflict> {
    let sessions: Vec<cross_session_conflicts::Session> = tasks
        .iter()
        .map(|t| {
            cross_session_conflicts::Session::new(
                t.id.clone(),
                Some(t.repo_key.clone()),
                changed_files.get(&t.id).cloned().unwrap_or_default(),
            )
        })
        .collect();
    cross_session_conflicts::conflicts(task_id, &sessions)
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Running dot breathing (`plans/014` M2) is gated on the window
        // actually being the one the user is looking at -- see
        // `motion::status_dot_element`'s doc comment for the power
        // rationale (an unfocused/minimized window would otherwise still
        // pay for a repeating `request_animation_frame` loop it can't see).
        let breathing_enabled = window.is_window_active() && !crate::motion::reduce_motion();

        let sidebar_el = sidebar::render(self, breathing_enabled, cx);

        let workspace_el = if let Some(task_id) = self.selected_task_id.clone() {
            if let Some(workspace) = self.workspaces.get(&task_id) {
                let spec = self.spec.clone();
                let focus_handle = self.focus_handle.clone();
                let active_preedit = self.active_preedit.clone();
                let focused_pane = workspace.focused_pane;
                let pane_drag_hover = workspace.pane_drag_hover;
                task_workspace::render_tile(
                    &task_id,
                    &workspace.model.root,
                    &workspace.runtimes,
                    &workspace.pane_status,
                    &workspace.pane_usage,
                    focused_pane,
                    &spec,
                    &focus_handle,
                    active_preedit.as_ref(),
                    pane_drag_hover,
                    self.divider_drag_active,
                    breathing_enabled,
                    cx,
                )
            } else {
                empty_state("タスクを読み込み中…")
            }
        } else {
            empty_state(
                "タスクが選択されていません。左上の + アイコンからフォルダまたは worktree で開始してください。",
            )
        };

        // The selected Task's Git pane -- a fixed pane to the right of the
        // tile tree, not part of it (see `crate::git_pane`'s module doc
        // comment). `None` whenever no Task is selected or its pane is
        // currently hidden, in which case `.children(..)` below simply adds
        // no third child.
        let git_pane_spec = self.spec.clone();
        let git_pane_el = self.selected_task_id.clone().and_then(|task_id| {
            self.workspaces
                .get(&task_id)
                .filter(|workspace| workspace.git.visible)
                .map(|workspace| {
                    git_pane::render_git_pane(&task_id, &workspace.git, &git_pane_spec, cx)
                })
        });

        // The Cmd+, settings overlay (`crate::settings`) -- `None` (no
        // extra child) unless `settings_open`, painted last so it's always
        // on top of the sidebar/workspace/Git pane below it.
        let settings_el = settings::render_settings_overlay(self, cx);

        // タスク行「…」メニュー / 削除確認 (wave 6c §2) と About
        // (wave 6c §1) -- settings と同じ「開いている間だけ子が存在する」
        // オーバーレイ。
        let task_menu_el = task_menu::render_task_menu_overlay(self, window, cx);
        let about_el = menus::render_about_overlay(self, cx);

        div()
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::key_down))
            .on_action(cx.listener(Self::action_new_tab))
            .on_action(cx.listener(Self::action_close_tab))
            .on_action(cx.listener(Self::action_split_right))
            .on_action(cx.listener(Self::action_split_down))
            .on_action(cx.listener(Self::action_paste))
            .on_action(cx.listener(Self::action_copy))
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
            .on_action(cx.listener(Self::action_toggle_git_pane))
            .on_action(cx.listener(Self::action_toggle_settings))
            .on_action(cx.listener(Self::action_about))
            .on_action(cx.listener(Self::action_new_attached_task))
            .on_action(cx.listener(Self::action_new_worktree_task))
            .on_action(cx.listener(Self::action_minimize_window))
            .on_action(cx.listener(Self::action_zoom_window))
            .on_action(cx.listener(Self::action_open_selected_in_ide))
            .relative()
            .flex()
            .flex_row()
            .size_full()
            .bg(rgb(theme::surface::ROOT))
            .child(sidebar_el)
            .child(workspace_el)
            .children(git_pane_el)
            .children(settings_el)
            .children(task_menu_el)
            .children(about_el)
    }
}

/// gpui's [`Modifiers`] to `crate::mouse_report::MouseMods`'s narrower,
/// SGR-relevant subset (`shift`/`alt`/`ctrl` only -- SGR mouse encoding has
/// no bit for `platform`/`function`). Shared by every mouse handler in
/// `impl LaboLaboApp`'s "text selection, mouse reporting & wheel scroll"
/// section above.
fn mouse_mods(modifiers: Modifiers) -> MouseMods {
    MouseMods {
        shift: modifiers.shift,
        alt: modifiers.alt,
        ctrl: modifiers.control,
    }
}

fn empty_state(message: &'static str) -> gpui::AnyElement {
    div()
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .text_color(rgb(theme::text::SECONDARY))
        .child(message)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use labolabo_core::TileLayout;

    // MARK: - compute_task_conflicts (cross-session conflict detection,
    // `plans` wave 5i §2)

    fn worktree_task(repo_key: &str, id_suffix: &str) -> Task {
        let mut t = Task::new_worktree(
            repo_key,
            repo_key,
            repo_key,
            format!("branch-{id_suffix}"),
            "main",
            format!("/tmp/{id_suffix}"),
            TileLayout::default(),
            0,
        );
        // `Task::new_worktree` mints a fresh random id -- pin a
        // deterministic one so the tests below can build `changed_files`
        // maps keyed by a value they control, not a UUID they'd have to
        // read back out of `t` first.
        t.id = id_suffix.to_string();
        t
    }

    fn changed(pairs: &[(&str, &[&str])]) -> HashMap<String, HashSet<String>> {
        pairs
            .iter()
            .map(|(id, paths)| {
                (
                    id.to_string(),
                    paths.iter().map(|p| p.to_string()).collect(),
                )
            })
            .collect()
    }

    #[test]
    fn no_conflict_when_only_one_task_has_fetched_status() {
        // `changed_files_cache` has no entry at all for "b" (never
        // selected/refreshed) -- must not spuriously conflict with "a".
        let tasks = vec![worktree_task("R", "a"), worktree_task("R", "b")];
        let changed_files = changed(&[("a", &["src/foo.rs"])]);
        assert!(compute_task_conflicts(&tasks, &changed_files, "a").is_empty());
    }

    #[test]
    fn conflict_detected_once_both_tasks_have_fetched_status() {
        let tasks = vec![worktree_task("R", "a"), worktree_task("R", "b")];
        let changed_files = changed(&[
            ("a", &["src/foo.rs", "a-only.rs"]),
            ("b", &["src/foo.rs", "b-only.rs"]),
        ]);
        let result = compute_task_conflicts(&tasks, &changed_files, "a");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].path, "src/foo.rs");
        assert_eq!(result[0].others, vec!["b".to_string()]);
    }

    #[test]
    fn no_conflict_across_different_repos() {
        let tasks = vec![worktree_task("R1", "a"), worktree_task("R2", "b")];
        let changed_files = changed(&[("a", &["foo.rs"]), ("b", &["foo.rs"])]);
        assert!(compute_task_conflicts(&tasks, &changed_files, "a").is_empty());
    }

    #[test]
    fn unknown_task_id_yields_no_conflicts() {
        let tasks = vec![worktree_task("R", "a")];
        let changed_files = changed(&[("a", &["foo.rs"])]);
        assert!(compute_task_conflicts(&tasks, &changed_files, "does-not-exist").is_empty());
    }

    #[test]
    fn empty_tasks_yields_no_conflicts() {
        assert!(compute_task_conflicts(&[], &HashMap::new(), "a").is_empty());
    }
}
