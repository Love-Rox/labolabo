//! One Task's live workspace: its [`PaneTilingModel`] (split/tab tree) plus
//! a real [`labolabo_term::Terminal`] session (+ redraw bridge) for every
//! `terminal`-kind pane in that tree, and the recursive render tree that
//! turns it into split panes with tab bars.
//!
//! This is wave 5b-2's `TerminalApp`-owned single tile tree
//! (`app::TerminalApp::model`/`runtimes`), lifted out unchanged in shape and
//! made **per-Task**: wave 5b-3 (`plans/012-task-model-and-control-cli.md`
//! §1) replaces "one window = one `PaneTilingModel`" with "one Task = one
//! `PaneTilingModel`", so [`crate::app::LaboLaboApp`] now keeps a
//! [`TaskWorkspace`] per loaded Task (`HashMap<String /* Task id */,
//! TaskWorkspace>`) instead of a single one of these fields directly. Every
//! render/action-routing function below therefore takes an explicit
//! `task_id: &str` to thread through click-handler closures (so a click
//! inside one Task's pane always routes back to *that* Task, regardless of
//! which one is currently selected when the closure eventually fires) --
//! everything else (the tree-walk, the tab-bar chips, the canvas
//! resize/paint wiring) is unchanged from wave 5b-2's `app.rs`.

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use futures::channel::mpsc;
use futures::StreamExt;
use gpui::{
    canvas, div, prelude::*, px, relative, rgb, AnyElement, Context, IntoElement, MouseButton,
    MouseDownEvent, SharedString, Task as GpuiTask,
};

use labolabo_core::{PaneId, PaneTilingModel, TileNode, TileOrientation};
use labolabo_term::{TermEvent, Terminal};

use crate::app::LaboLaboApp;
use crate::grid;
use crate::render::RenderSpec;

/// See `app::EVENT_POLL_TIMEOUT`'s Wave 5b-2 doc comment (unchanged):
/// how long the redraw-bridge thread blocks on `recv_event` between checks
/// of whether its gpui-side `Task` was dropped (pane closed).
const EVENT_POLL_TIMEOUT: Duration = Duration::from_millis(250);

/// See `app::FRAME_INTERVAL`'s Wave 5b-2 doc comment (unchanged): minimum
/// gap between two `cx.notify()` calls for the same pane.
const FRAME_INTERVAL: Duration = Duration::from_millis(16);

/// Accent color for the focused pane's frame border.
const FOCUS_BORDER_COLOR: u32 = 0x5e9eff;
/// Frame border color for every other (unfocused) pane.
const IDLE_BORDER_COLOR: u32 = 0x1c1c1c;

/// What the per-pane bridge thread forwards to its gpui-side task.
enum BridgeMsg {
    Wakeup,
    Exit,
}

/// The live resources backing one `terminal`-kind [`labolabo_core::
/// PaneItem`] within a single Task's tree: its real `labolabo_term::
/// Terminal` session and the redraw bridge that keeps gpui notified of it.
/// Kept for every terminal pane in the tree -- including hidden (non-
/// selected tab) ones, and including every pane in a Task that isn't the
/// currently *selected* Task -- so pty/scrollback survive tab switches,
/// splits/closes elsewhere in the tree, and switching away from a Task
/// entirely (the plan's "表示中でない Task の TileLayout/pty はメモリ上に
/// 温存 = タブ切替と同じ意味論").
pub struct PaneRuntime {
    pub session: Arc<Terminal>,
    /// Last (cols, rows) this pane's session was resized to. Shared
    /// (`Rc<Cell<_>>`) because the canvas element's `prepaint` closure runs
    /// without a `&mut LaboLaboApp` borrow available to it -- see
    /// `render_leaf`'s doc comment.
    pub last_size: Rc<Cell<(u16, u16)>>,
    /// Keeps the redraw-bridge task alive for the pane's lifetime; dropping
    /// it (on pane close) is the signal the bridge thread uses to stop.
    _redraw_task: GpuiTask<()>,
}

/// One Task's live workspace: its tile/tab tree plus a [`PaneRuntime`] for
/// every terminal pane in it, and which pane currently has keyboard focus
/// (see `crate::focus`'s module doc comment for the "focus is a `PaneId`"
/// invariant, unchanged from wave 5b-2 -- just now one instance per Task
/// instead of one for the whole app).
pub struct TaskWorkspace {
    pub model: PaneTilingModel,
    pub runtimes: HashMap<PaneId, PaneRuntime>,
    pub focused_pane: PaneId,
}

impl TaskWorkspace {
    /// A fresh workspace around `model`, with no runtimes spawned yet
    /// (callers spawn one per terminal pane themselves -- see
    /// `LaboLaboApp::ensure_workspace_loaded`) and focus resolved to the
    /// tree's first leaf's selected tab (a simple, deterministic default;
    /// the plan explicitly scopes persisting *which* leaf had keyboard
    /// focus across restarts out of this wave -- only the tile/tab
    /// structure and each leaf's selected tab round-trip through
    /// `TileLayout`).
    pub fn new(model: PaneTilingModel) -> Self {
        let focused_pane = model
            .root
            .leaves()
            .first()
            .and_then(|leaf| leaf.selected_pane())
            .map(|p| p.id)
            .or_else(|| model.panes().first().map(|p| p.id))
            .expect("a PaneTilingModel always has at least one pane");
        Self {
            model,
            runtimes: HashMap::new(),
            focused_pane,
        }
    }
}

/// Bridges `labolabo_term`'s blocking [`Terminal::recv_event`] into gpui.
/// Identical two-stage coalesce-then-pace design as wave 5a/5b-2's (see
/// their doc comments for the full rationale) -- the only change is that
/// the exit callback now needs `task_id` too, since a pane's identity within
/// [`LaboLaboApp::workspaces`] is now `(task_id, pane_id)`, not just
/// `pane_id`.
pub fn spawn_redraw_bridge(
    session: Arc<Terminal>,
    task_id: String,
    pane_id: PaneId,
    cx: &mut Context<LaboLaboApp>,
) -> GpuiTask<()> {
    let (notify_tx, mut notify_rx) = mpsc::unbounded::<BridgeMsg>();

    thread::spawn(move || loop {
        match session.recv_event(EVENT_POLL_TIMEOUT) {
            Some(TermEvent::Wakeup) => {
                if notify_tx.unbounded_send(BridgeMsg::Wakeup).is_err() {
                    break;
                }
            }
            Some(TermEvent::Exit) => {
                let _ = notify_tx.unbounded_send(BridgeMsg::Exit);
                break;
            }
            None => {
                if notify_tx.is_closed() {
                    break;
                }
            }
        }
    });

    cx.spawn(async move |this, cx| {
        let drain = |rx: &mut mpsc::UnboundedReceiver<BridgeMsg>| -> bool {
            let mut exited = false;
            while let Ok(msg) = rx.try_recv() {
                if matches!(msg, BridgeMsg::Exit) {
                    exited = true;
                }
            }
            exited
        };

        while let Some(msg) = notify_rx.next().await {
            let mut exited = matches!(msg, BridgeMsg::Exit);
            exited |= drain(&mut notify_rx);
            if exited {
                let _ = this.update(cx, |app, cx| app.handle_pane_exit(&task_id, pane_id, cx));
                break;
            }

            if this.update(cx, |_, cx| cx.notify()).is_err() {
                break;
            }

            gpui::Timer::after(FRAME_INTERVAL).await;
            if drain(&mut notify_rx) {
                let _ = this.update(cx, |app, cx| app.handle_pane_exit(&task_id, pane_id, cx));
                break;
            }
        }
    })
}

/// Registers a freshly spawned session as `pane_id`'s [`PaneRuntime`] inside
/// `runtimes`. Split out of `LaboLaboApp::spawn_runtime_for_task` only to
/// keep the borrow shape simple there (see that method's doc comment).
pub fn insert_runtime(
    runtimes: &mut HashMap<PaneId, PaneRuntime>,
    pane_id: PaneId,
    session: Arc<Terminal>,
    cols: u16,
    rows: u16,
    redraw_task: GpuiTask<()>,
) {
    runtimes.insert(
        pane_id,
        PaneRuntime {
            session,
            last_size: Rc::new(Cell::new((cols, rows))),
            _redraw_task: redraw_task,
        },
    );
}

/// Recursively render one node of `task_id`'s tile tree -- identical
/// tree-walk to wave 5b-2's `app::render_tile`, just carrying `task_id`
/// through so leaf click handlers route back to the right Task.
pub fn render_tile(
    task_id: &str,
    node: &TileNode,
    runtimes: &HashMap<PaneId, PaneRuntime>,
    focused_pane: PaneId,
    spec: &RenderSpec,
    cx: &mut Context<LaboLaboApp>,
) -> AnyElement {
    if node.is_leaf() {
        return render_leaf(task_id, node, runtimes, focused_pane, spec, cx);
    }

    let Some(first_child) = node.children.first() else {
        return div().size_full().into_any_element();
    };
    let Some(second_child) = node.children.get(1) else {
        return render_tile(task_id, first_child, runtimes, focused_pane, spec, cx);
    };

    let is_row = node.orientation == TileOrientation::Horizontal;
    let ratio = (node.ratio as f32).clamp(0.05, 0.95);

    let first_el = render_tile(task_id, first_child, runtimes, focused_pane, spec, cx);
    let second_el = render_tile(task_id, second_child, runtimes, focused_pane, spec, cx);

    let (first_wrap, second_wrap) = if is_row {
        (
            div().h_full().w(relative(ratio)).child(first_el),
            div().h_full().w(relative(1.0 - ratio)).child(second_el),
        )
    } else {
        (
            div().w_full().h(relative(ratio)).child(first_el),
            div().w_full().h(relative(1.0 - ratio)).child(second_el),
        )
    };

    let mut container = div().flex().size_full();
    container = if is_row {
        container.flex_row()
    } else {
        container.flex_col()
    };
    container
        .child(first_wrap)
        .child(second_wrap)
        .into_any_element()
}

/// Render one leaf (tab group) of `task_id`'s tree. Identical to wave
/// 5b-2's `app::render_leaf` (see its doc comment for the per-pane sizing
/// rationale -- unchanged) other than threading `task_id` through the
/// click handler.
fn render_leaf(
    task_id: &str,
    node: &TileNode,
    runtimes: &HashMap<PaneId, PaneRuntime>,
    focused_pane: PaneId,
    spec: &RenderSpec,
    cx: &mut Context<LaboLaboApp>,
) -> AnyElement {
    let is_focused_leaf = node.panes.iter().any(|p| p.id == focused_pane);
    let selected_id = node.selected_pane().map(|p| p.id);
    let runtime = selected_id.and_then(|id| runtimes.get(&id));

    let session_for_resize = runtime.map(|rt| rt.session.clone());
    let last_size = runtime.map(|rt| rt.last_size.clone());
    let snapshot = runtime.map(|rt| rt.session.snapshot());
    let prepaint_spec = spec.clone();
    let paint_spec = spec.clone();

    let canvas_el = canvas(
        move |bounds, _window, _cx| {
            if let (Some(session), Some(last_size)) = (&session_for_resize, &last_size) {
                let (cols, rows) = grid::grid_size_for_area(
                    bounds.size.width.into(),
                    bounds.size.height.into(),
                    prepaint_spec.cell_width,
                    prepaint_spec.cell_height,
                );
                if last_size.get() != (cols, rows) {
                    last_size.set((cols, rows));
                    session.resize(cols, rows);
                }
            }
        },
        move |bounds, _, window, cx| {
            if let Some(snapshot) = &snapshot {
                crate::render::paint_grid(snapshot, &paint_spec, bounds, window, cx);
            }
        },
    )
    .size_full();

    let click_target = selected_id;
    let click_task_id = task_id.to_string();
    let canvas_area = div().flex_1().w_full().on_mouse_down(
        MouseButton::Left,
        cx.listener(move |this, _: &MouseDownEvent, window, cx| {
            if let Some(id) = click_target {
                this.select_pane(&click_task_id, id, window, cx);
            }
        }),
    );
    let canvas_area = canvas_area.child(canvas_el);

    let tab_bar = render_pane_tab_bar(task_id, node, is_focused_leaf, cx);

    div()
        .flex()
        .flex_col()
        .size_full()
        .border_1()
        .border_color(rgb(if is_focused_leaf {
            FOCUS_BORDER_COLOR
        } else {
            IDLE_BORDER_COLOR
        }))
        .child(tab_bar)
        .child(canvas_area)
        .into_any_element()
}

/// One leaf's tab bar. Identical to wave 5b-2's `app::render_pane_tab_bar`
/// other than threading `task_id` through both click handlers.
fn render_pane_tab_bar(
    task_id: &str,
    node: &TileNode,
    is_focused: bool,
    cx: &mut Context<LaboLaboApp>,
) -> impl IntoElement {
    let anchor = node.selected_pane().map(|p| p.id);

    div()
        .flex()
        .flex_row()
        .items_center()
        .h(px(grid::TAB_BAR_HEIGHT))
        .w_full()
        .bg(rgb(if is_focused { 0x2f2f2f } else { 0x232323 }))
        .px_1()
        .gap_1()
        .children(node.panes.iter().map(|pane| {
            let selected = anchor == Some(pane.id);
            let pane_id = pane.id;
            let title: SharedString = pane.title.clone().into();
            let select_task_id = task_id.to_string();
            let close_task_id = task_id.to_string();
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_1()
                .px_2()
                .rounded_sm()
                .when(selected, |el| el.bg(rgb(0x454545)))
                .when(!selected, |el| el.bg(rgb(0x333333)))
                .child(
                    div()
                        .px_1()
                        .text_color(rgb(0xe5e5e5))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                                this.select_pane(&select_task_id, pane_id, window, cx);
                            }),
                        )
                        .child(title),
                )
                .child(
                    div()
                        .px_1()
                        .text_color(rgb(0x999999))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                                this.close_pane_user(&close_task_id, pane_id, cx);
                                window.focus(this.focus_handle());
                            }),
                        )
                        .child("\u{d7}"),
                )
        }))
        .child({
            let add_task_id = task_id.to_string();
            div()
                .px_2()
                .text_color(rgb(0xe5e5e5))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                        if let Some(anchor) = anchor {
                            this.add_tab_to(&add_task_id, anchor, window, cx);
                        }
                    }),
                )
                .child("+")
        })
}
