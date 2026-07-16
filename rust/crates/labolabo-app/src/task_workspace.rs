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
    canvas, div, prelude::*, px, relative, rgb, rgba, Animation, AnimationExt, AnyElement, Bounds,
    Context, CursorStyle, DragMoveEvent, ElementInputHandler, ExternalPaths, FocusHandle,
    IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Render,
    ScrollWheelEvent, SharedString, Task as GpuiTask, Window,
};

use labolabo_core::{
    AgentStatus, AgentUsage, DropEdge, NodeId, PaneId, PaneKind, PaneTilingModel, TileNode,
    TileOrientation,
};
use labolabo_term::{TermEvent, Terminal};
use rust_i18n::t;

use crate::app::{LaboLaboApp, PreeditState};
use crate::commit_pane;
use crate::git_pane::{self, GitPaneState};
use crate::grid;
use crate::motion::{self, DotAnimState};
use crate::render::RenderSpec;
use crate::selection::Selection;
use crate::theme;

/// See `app::EVENT_POLL_TIMEOUT`'s Wave 5b-2 doc comment (unchanged):
/// how long the redraw-bridge thread blocks on `recv_event` between checks
/// of whether its gpui-side `Task` was dropped (pane closed).
const EVENT_POLL_TIMEOUT: Duration = Duration::from_millis(250);

/// See `app::FRAME_INTERVAL`'s Wave 5b-2 doc comment (unchanged): minimum
/// gap between two `cx.notify()` calls for the same pane.
const FRAME_INTERVAL: Duration = Duration::from_millis(16);

/// Accent color for the focused pane's frame border.
const FOCUS_BORDER_COLOR: u32 = theme::ACCENT;
/// Frame border color for every other (unfocused) pane.
const IDLE_BORDER_COLOR: u32 = theme::surface::STROKE;

/// Drop-zone highlight for a tab/pane **move** (plan §3's "ドロップゾーンの
/// ハイライト表示"), translucent blue -- the same hue as [`FOCUS_BORDER_COLOR`]
/// so "this pane" and "the drop target" read as one visual family, alpha'd
/// down (`4d` = ~30%) so terminal content underneath stays legible.
const MOVE_DROP_HIGHLIGHT_COLOR: u32 = theme::with_alpha(theme::dnd::MOVE, 0x4d);
/// Drop-zone highlight for an OS file/folder **insert** into a terminal
/// (`plans/012` §3.1's "ファイル挿入" indicator) -- a distinct amber hue from
/// [`MOVE_DROP_HIGHLIGHT_COLOR`] so "move a pane" and "insert a path" never
/// look like the same affordance, per §3.1's explicit "別スタイルにして
/// 「移動」と「ファイル挿入」を区別する" requirement.
const FILE_DROP_HIGHLIGHT_COLOR: u32 = theme::with_alpha(theme::dnd::FILE_INSERT, 0x4d);

/// Payload of an in-progress tab-chip drag (`render_pane_tab_bar`'s
/// `.on_drag`): identifies the dragged tab by [`PaneId`]. `move_pane` (the
/// core model op this eventually calls) only needs a source `PaneId` and a
/// target `PaneId` -- see [`crate::app::LaboLaboApp::finish_pane_drag_drop`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TabDragPayload {
    pub source_pane_id: PaneId,
}

/// The floating view gpui renders under the cursor while a tab chip is
/// being dragged -- just the tab's title in a small chip, echoing
/// `render_pane_tab_bar`'s own chip styling so the drag preview reads as
/// "a copy of the thing you picked up" (mirrors AppKit's default drag-image
/// behavior, which `PaneTabChip.onDrag` got for free from `NSItemProvider`;
/// gpui has no default image for a value-only drag, so this view is what
/// supplies one).
pub struct TabDragPreview(pub SharedString);

impl Render for TabDragPreview {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py_1()
            .rounded_sm()
            .bg(rgb(theme::surface::ACTIVE))
            .text_color(rgb(theme::text::PRIMARY))
            .text_size(px(theme::font_size::CAPTION))
            .child(self.0.clone())
    }
}

/// Payload of an in-progress pane-divider drag (`render_tile`'s split
/// branch): identifies which split node's `ratio` is being dragged, and
/// along which axis. `orientation` is carried here (rather than re-derived
/// from the tree on every drag-move event) so a drag stays well-defined
/// even if `orientation` were ever ambiguous mid-drag -- a resize never
/// changes a split's own orientation, so this is simply the value read
/// once when the drag started (`plans` W5j #2 -- interactive divider
/// drag-resize).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DividerDragPayload {
    pub node_id: NodeId,
    pub orientation: TileOrientation,
}

/// The (deliberately invisible) view gpui's drag-and-drop system requires
/// as a [`DividerDragPayload`] drag's `.on_drag` preview. A divider drag
/// has no meaningful "thing being dragged" to show under the cursor, unlike
/// [`TabDragPreview`]: the two neighboring panes resize live underneath as
/// the ratio updates (`app::LaboLaboApp::update_divider_drag`), which *is*
/// the drag's own visual feedback -- a floating ghost chip following the
/// cursor on top of that would only be visual noise. Renders as a bare,
/// zero-size `div()`.
pub struct DividerDragPreview;

impl Render for DividerDragPreview {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// Which leaf (identified by its anchor -- currently selected -- pane id)
/// is the current tab-drag's drop target, and which [`DropEdge`] zone of it
/// -- `crate::app::LaboLaboApp::update_pane_drag_hover`'s output, consumed
/// by `render_leaf` to paint [`MOVE_DROP_HIGHLIGHT_COLOR`] over the right
/// quadrant. `None` on [`TaskWorkspace`] means no tab drag is currently
/// hovering any leaf of this Task (either no drag is active, or the pointer
/// isn't over any of this Task's leaves, or it's hovering a
/// meaningless-to-drop-here zone -- see that method's doc comment).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaneDragHover {
    pub target_pane_id: PaneId,
    pub edge: DropEdge,
}

/// Status-dot color per [`AgentStatus`] -- deliberately minimal (a plain
/// colored dot, per the wave brief: "見た目は最小限"), not the Swift
/// sidebar's `PhaseAnimator`-driven ping/breathing-halo treatment. `None`
/// means "no dot" (no event observed yet). Shared by this module's tab
/// chips and `crate::sidebar`'s per-Task row (`LaboLaboApp::
/// task_agent_status`'s aggregate).
pub(crate) fn status_dot_color(status: AgentStatus) -> Option<u32> {
    match status {
        AgentStatus::None => None,
        AgentStatus::Starting => Some(theme::status::STARTING),
        AgentStatus::Running => Some(theme::status::RUNNING),
        AgentStatus::WaitingForInput => Some(theme::status::WAITING),
        AgentStatus::Idle => Some(theme::status::IDLE),
        AgentStatus::Ended => Some(theme::status::ENDED),
    }
}

/// A short, compact summary of `usage` for a tab chip -- e.g. `"1.2k tok ·
/// $0.08"`, or `"532 tok"` when the model's pricing is unknown
/// (`AgentUsage::estimated_cost_usd`'s `None`, mirrors Swift's
/// `UsagePopover`'s "価格未知（トークンのみ）" fallback, just folded into one
/// line instead of a separate popover row -- this port has no tab-chip
/// tooltip/popover surface yet, see `crate::hooks`'s module doc comment on
/// what *is* ported this wave). `None` if `usage.is_empty()` (nothing
/// observed yet -- same as no dot for [`status_dot_color`]).
pub(crate) fn format_usage_compact(usage: &AgentUsage) -> Option<String> {
    if usage.is_empty() {
        return None;
    }
    let tokens = format_compact_count(usage.total_tokens());
    Some(match usage.estimated_cost_usd() {
        Some(cost) => format!("{tokens} tok \u{b7} ${cost:.2}"),
        None => format!("{tokens} tok"),
    })
}

/// `1234 -> "1.2k"`, `1_234_567 -> "1.2M"`, else the plain decimal --
/// deliberately coarse (one decimal place) since this only ever feeds a
/// space-constrained tab chip, not a precise accounting display (the exact
/// counts remain available via `AgentUsage`'s own fields for any future
/// fuller view). Negative input (shouldn't happen -- token counts are
/// always non-negative sums) is clamped to `0` rather than producing a
/// confusing `"-1.2k"`.
fn format_compact_count(n: i64) -> String {
    let n = n.max(0);
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

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
    /// This pane's canvas's own paint bounds, as of the most recent
    /// prepaint -- same `Rc<Cell<_>>`-for-`Fn`-closure-access shape as
    /// `last_size` (see its doc comment). Mouse handlers (registered via
    /// `cx.listener`, which *does* get `&mut LaboLaboApp`) read this to
    /// convert a window-space `MouseDownEvent`/`MouseMoveEvent` position
    /// into a canvas-local one before calling `grid::cell_at` -- see
    /// `app::LaboLaboApp::begin_selection`.
    pub last_bounds: Rc<Cell<Bounds<Pixels>>>,
    /// This pane's in-progress or finished text selection, if any -- `None`
    /// is the common case (most panes have no active selection most of the
    /// time). Mutated directly by the mouse handlers below (which have
    /// `&mut LaboLaboApp` access, unlike the canvas element's own paint
    /// closures) and read by `render_leaf` to paint the highlight
    /// (`render::paint_grid`) and by `app::LaboLaboApp::action_copy` to
    /// extract text. See `crate::selection`'s module doc comment for what
    /// a selection's coordinates mean against a possibly-scrolled snapshot.
    pub selection: Option<Selection>,
    /// Whether the in-progress left-button gesture (mouse-down through the
    /// matching mouse-up) is being SGR-encoded and forwarded to the child
    /// program's PTY (`crate::mouse_report`), instead of driving local text
    /// selection (`selection` above) -- decided once, at mouse-down
    /// (`app::LaboLaboApp::begin_selection`), from that moment's
    /// `Terminal::mouse_mode()` and whether Shift was held (Shift forces
    /// local selection even while the child has mouse reporting on --
    /// matches real Ghostty's own default `mouse-shift-capture` behavior,
    /// confirmed by reading the vendored Ghostty source), and held fixed
    /// for the rest of that one gesture so a modifier key changing
    /// mid-drag can't switch behavior underneath the user. `selection` and
    /// this flag are mutually exclusive for the duration of one gesture:
    /// when this is `true`, `selection` is left `None` throughout.
    pub reporting_drag: bool,
    /// Fractional scroll-line remainder carried between wheel/trackpad
    /// events for this pane -- see `grid::accumulate_scroll_lines`'s doc
    /// comment for why this needs to persist across events (a slow
    /// trackpad gesture's individual deltas are each smaller than one
    /// cell height).
    pub pending_scroll: f32,
    /// The `LABOLABO_PANE` value this pane's session was spawned with (a
    /// fresh UUID minted at spawn time, see `crate::hooks`'s module doc
    /// comment) -- kept so pane removal can unregister the routing table
    /// entry (`crate::hooks::HookRuntime::unregister_pane`).
    pub pane_uuid: String,
    /// This pane's tab-chip status-dot crossfade state (`plans/014` M1) --
    /// `Cell`-wrapped for the same "mutate from a plain `&` render pass"
    /// reason as `last_size`/`last_bounds` above (see `motion::
    /// status_dot_element`'s doc comment for the full mechanism).
    pub dot_anim: Cell<DotAnimState>,
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
    /// Live [`AgentStatus`] per terminal pane, from hooks events routed to
    /// this Task (`crate::app::LaboLaboApp::handle_agent_event`) -- the tab
    /// chip's status dot reads this (see `render_pane_tab_bar`). A pane
    /// with no entry (never received an event, or was just spawned) shows
    /// no dot -- same as `AgentStatus::None`'s Swift label ("—").
    pub pane_status: HashMap<PaneId, AgentStatus>,
    /// Per-pane transcript usage (tokens/estimated cost), from hooks events
    /// routed to this Task -- mirrors `pane_status` in every respect
    /// (populated by `crate::app::LaboLaboApp::handle_agent_event`, read by
    /// the tab chip via `format_usage_compact`) except that it only ever
    /// updates on an `Idle`/`Ended` status event with a resolvable
    /// transcript path, not on every event (see `handle_agent_event`'s doc
    /// comment: transcript re-reads are hooks-event-triggered, never
    /// polled). A pane with no entry shows no usage label, same as
    /// `pane_status`'s "no dot" default.
    pub pane_usage: HashMap<PaneId, AgentUsage>,
    /// This Task's live tab-drag drop-target highlight, if a tab drag is
    /// currently hovering one of its leaves -- see [`PaneDragHover`]'s doc
    /// comment. Purely UI/render state (never persisted, never affects
    /// `model`), rebuilt continuously while a drag is in flight.
    pub pane_drag_hover: Option<PaneDragHover>,
    /// This Task's Git pane state (branch/status, changed-files list,
    /// selected file's diff/whole-file contents) -- see `crate::git_pane`'s
    /// module doc comment. Its `FileWatcher` is only ever live while this
    /// Task is selected (`LaboLaboApp::activate_git_pane`/
    /// `deactivate_git_pane`), so a freshly constructed `TaskWorkspace` here
    /// starts with none attached.
    pub git: GitPaneState,
    /// This Task's sidebar-row status-dot crossfade state (`plans/014` M1)
    /// -- the Task-level *aggregate* status
    /// (`LaboLaboApp::task_agent_status`), as opposed to `PaneRuntime::
    /// dot_anim`'s per-pane one. Same `Cell`-for-render-time-mutation shape.
    pub dot_anim: Cell<DotAnimState>,
}

impl TaskWorkspace {
    /// A fresh workspace around `model`, with no runtimes spawned yet
    /// (callers spawn one per terminal pane themselves -- see
    /// `LaboLaboApp::ensure_workspace_loaded`) and focus resolved to the
    /// tree's first leaf's selected tab (a simple, deterministic default;
    /// the plan explicitly scopes persisting *which* leaf had keyboard
    /// focus across restarts out of this wave -- only the tile/tab
    /// structure and each leaf's selected tab round-trip through
    /// `TileLayout`). `git_pane_default_visible` seeds `git.visible` --
    /// `crate::settings::AppSettings::git_pane_default_visible`'s persisted
    /// value, threaded in by every caller rather than always defaulting to
    /// `true` (`GitPaneState::default()`'s own default, still used by
    /// [`GitPaneState::new`] directly for callers -- e.g. tests -- that
    /// don't care about the setting).
    pub fn new(model: PaneTilingModel, git_pane_default_visible: bool) -> Self {
        let focused_pane = model
            .root
            .leaves()
            .first()
            .and_then(|leaf| leaf.selected_pane())
            .map(|p| p.id)
            .or_else(|| model.panes().first().map(|p| p.id))
            .expect("a PaneTilingModel always has at least one pane");
        let mut git = GitPaneState::new();
        git.visible = git_pane_default_visible;
        Self {
            model,
            runtimes: HashMap::new(),
            focused_pane,
            pane_status: HashMap::new(),
            pane_usage: HashMap::new(),
            pane_drag_hover: None,
            git,
            dot_anim: Cell::new(DotAnimState::default()),
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
        // Coalesce a burst of already-buffered messages into whatever this
        // one call observes -- never blocks (`try_recv`), so calling it
        // right after `notify_rx.next().await` already delivered one
        // message just picks up anything that piled up behind it. Only
        // ever used *before* this loop's own `cx.notify()`, so its "did we
        // see `Exit`" return value is the one thing every caller needs;
        // any `Wakeup`s it drains are implicitly covered by the `notify()`
        // that follows in the same loop iteration.
        //
        // W5j bug report #2 (a terminal pane appearing "stuck"/stale after
        // a layout change, e.g. toggling the Git side pane, until real PTY
        // output arrived): an earlier version of this function *also*
        // called `drain` a second time, after the `FRAME_INTERVAL` pacing
        // sleep below, using only its `Exit`-detection return value -- any
        // `Wakeup`(s) that drain silently discarded were never followed by
        // a `cx.notify()` at all, so gpui had no reason to repaint until
        // some later, unrelated event happened to produce a fresh
        // `Wakeup`. A resize is exactly the kind of event prone to
        // producing a second, closely-spaced `Wakeup` within that 16ms
        // window (the resize's own snapshot-republish, immediately
        // followed by the child program's own SIGWINCH-triggered
        // redraw/echo) -- landing precisely in the swallow window. Fixed
        // by removing that second call outright: anything that arrives
        // during the sleep is simply left queued, so `notify_rx.next()
        // .await` at the top of the next iteration resolves immediately
        // (non-blocking in practice) and this same drain-then-notify path
        // runs again for it, guaranteeing every real `Wakeup` is
        // eventually followed by a `cx.notify()` call -- the actual
        // invariant this bridge needs -- while still capping the rate to
        // at most one `cx.notify()` per `FRAME_INTERVAL`, exactly as
        // before.
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
        }
    })
}

/// Registers a freshly spawned session as `pane_id`'s [`PaneRuntime`] inside
/// `runtimes`. Split out of `LaboLaboApp::spawn_runtime_for_task` only to
/// keep the borrow shape simple there (see that method's doc comment).
#[allow(clippy::too_many_arguments)]
pub fn insert_runtime(
    runtimes: &mut HashMap<PaneId, PaneRuntime>,
    pane_id: PaneId,
    session: Arc<Terminal>,
    cols: u16,
    rows: u16,
    pane_uuid: String,
    redraw_task: GpuiTask<()>,
) {
    runtimes.insert(
        pane_id,
        PaneRuntime {
            session,
            last_size: Rc::new(Cell::new((cols, rows))),
            last_bounds: Rc::new(Cell::new(Bounds::default())),
            selection: None,
            reporting_drag: false,
            pending_scroll: 0.0,
            pane_uuid,
            dot_anim: Cell::new(DotAnimState::default()),
            _redraw_task: redraw_task,
        },
    );
}

/// Hit-test width (pixels) of a pane-divider's drag handle, and the
/// visible width of its hover/active highlight -- both the same fixed
/// thickness, centered exactly on the split boundary (`ratio * 100%`) via
/// a negative margin of half this width, regardless of the split
/// container's actual pixel size. Deliberately small (a thin line, not a
/// wide gutter) to keep the two neighboring panes' content flush most of
/// the time, matching most desktop split-pane conventions.
const DIVIDER_HIT_WIDTH: f32 = 6.0;

/// Recursively render one node of `task_id`'s tile tree -- identical
/// tree-walk to wave 5b-2's `app::render_tile`, just carrying `task_id`
/// through so leaf click handlers route back to the right Task, plus (W5j
/// #2) a draggable divider between a split's two children and
/// `divider_drag_active` threaded through to every leaf so their canvases'
/// `prepaint` closures can suppress `Terminal::resize` while any divider
/// anywhere in the tree is actively being dragged (see `render_leaf`'s doc
/// comment on this parameter).
#[allow(clippy::too_many_arguments)]
pub fn render_tile(
    task_id: &str,
    node: &TileNode,
    runtimes: &HashMap<PaneId, PaneRuntime>,
    pane_status: &HashMap<PaneId, AgentStatus>,
    pane_usage: &HashMap<PaneId, AgentUsage>,
    git_state: &GitPaneState,
    focused_pane: PaneId,
    spec: &RenderSpec,
    focus_handle: &FocusHandle,
    active_preedit: Option<&PreeditState>,
    pane_drag_hover: Option<PaneDragHover>,
    divider_drag_active: bool,
    breathing_enabled: bool,
    cx: &mut Context<LaboLaboApp>,
) -> AnyElement {
    if node.is_leaf() {
        return render_leaf(
            task_id,
            node,
            runtimes,
            pane_status,
            pane_usage,
            git_state,
            focused_pane,
            spec,
            focus_handle,
            active_preedit,
            pane_drag_hover,
            divider_drag_active,
            breathing_enabled,
            cx,
        );
    }

    let Some(first_child) = node.children.first() else {
        return div().size_full().into_any_element();
    };
    let Some(second_child) = node.children.get(1) else {
        return render_tile(
            task_id,
            first_child,
            runtimes,
            pane_status,
            pane_usage,
            git_state,
            focused_pane,
            spec,
            focus_handle,
            active_preedit,
            pane_drag_hover,
            divider_drag_active,
            breathing_enabled,
            cx,
        );
    };

    let is_row = node.orientation == TileOrientation::Horizontal;
    let ratio = (node.ratio as f32).clamp(
        labolabo_core::MIN_SPLIT_RATIO as f32,
        labolabo_core::MAX_SPLIT_RATIO as f32,
    );

    let first_el = render_tile(
        task_id,
        first_child,
        runtimes,
        pane_status,
        pane_usage,
        git_state,
        focused_pane,
        spec,
        focus_handle,
        active_preedit,
        pane_drag_hover,
        divider_drag_active,
        breathing_enabled,
        cx,
    );
    let second_el = render_tile(
        task_id,
        second_child,
        runtimes,
        pane_status,
        pane_usage,
        git_state,
        focused_pane,
        spec,
        focus_handle,
        active_preedit,
        pane_drag_hover,
        divider_drag_active,
        breathing_enabled,
        cx,
    );

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

    // The draggable divider: an absolutely-positioned handle centered on
    // the boundary line (`ratio * 100%` along the split axis, shifted back
    // by half its own width via a negative margin so it straddles the
    // boundary regardless of the container's actual pixel size), on top of
    // (rendered after, in `container`'s child order) the two panes so it
    // can catch mouse events right at their shared edge. Cursor and hover
    // highlight communicate "draggable" the same way real desktop split
    // panes do; the drag itself uses gpui's native drag-and-drop system
    // (`on_drag`/`on_drag_move`/`on_drop`) rather than raw mouse-move --
    // the same mechanism this file's tab-chip drag (`TabDragPayload`)
    // already uses -- specifically because `on_drag_move`/`on_drop`
    // deliver events based on the *drag's* live pointer position against
    // whichever element they're registered on (here, `container` itself,
    // spanning exactly the pixel region a ratio needs to be computed
    // against), independent of the thin divider's own hit-test bounds --
    // unlike a raw `on_mouse_move` listener, which only fires while the
    // cursor is still hovering the *specific* element it's registered on,
    // and would lose the drag the instant a fast flick outran a thin
    // handle's own hitbox.
    let divider_node_id = node.id;
    let divider_id: SharedString = format!("divider-{task_id}-{:?}", node.id).into();
    let divider = if is_row {
        div()
            .id(divider_id)
            .absolute()
            .top_0()
            .bottom_0()
            .left(relative(ratio))
            .ml(px(-(DIVIDER_HIT_WIDTH / 2.0)))
            .w(px(DIVIDER_HIT_WIDTH))
            .cursor(CursorStyle::ResizeLeftRight)
            .hover(|el| el.bg(rgba(theme::with_alpha(theme::ACCENT, 0x66))))
    } else {
        div()
            .id(divider_id)
            .absolute()
            .left_0()
            .right_0()
            .top(relative(ratio))
            .mt(px(-(DIVIDER_HIT_WIDTH / 2.0)))
            .h(px(DIVIDER_HIT_WIDTH))
            .cursor(CursorStyle::ResizeUpDown)
            .hover(|el| el.bg(rgba(theme::with_alpha(theme::ACCENT, 0x66))))
    };
    let divider = divider.on_drag(
        DividerDragPayload {
            node_id: divider_node_id,
            orientation: node.orientation,
        },
        |_payload, _offset, _window, cx| cx.new(|_cx| DividerDragPreview),
    );

    let drag_task_id = task_id.to_string();
    let drop_task_id = task_id.to_string();
    let mut container = div()
        .relative()
        .flex()
        .size_full()
        .on_drag_move::<DividerDragPayload>(cx.listener(
            move |app, event: &DragMoveEvent<DividerDragPayload>, _window, cx| {
                app.update_divider_drag(&drag_task_id, event, cx);
            },
        ))
        .on_drop::<DividerDragPayload>(cx.listener(
            move |app, _payload: &DividerDragPayload, _window, cx| {
                app.finish_divider_drag(&drop_task_id, cx);
            },
        ));
    container = if is_row {
        container.flex_row()
    } else {
        container.flex_col()
    };
    container
        .child(first_wrap)
        .child(second_wrap)
        .child(divider)
        .into_any_element()
}

/// Render one leaf (tab group) of `task_id`'s tree. Identical to wave
/// 5b-2's `app::render_leaf` (see its doc comment for the per-pane sizing
/// rationale -- unchanged) other than threading `task_id` through the click
/// handler, wiring up IME input handling and the preedit overlay for
/// whichever pane is the app's *focused* one, and (W5j #2)
/// `divider_drag_active`, which the canvas's `prepaint` closure consults to
/// suppress `Terminal::resize` while a divider anywhere in this Task's tree
/// is being dragged -- see that closure's own comment.
#[allow(clippy::too_many_arguments)]
fn render_leaf(
    task_id: &str,
    node: &TileNode,
    runtimes: &HashMap<PaneId, PaneRuntime>,
    pane_status: &HashMap<PaneId, AgentStatus>,
    pane_usage: &HashMap<PaneId, AgentUsage>,
    git_state: &GitPaneState,
    focused_pane: PaneId,
    spec: &RenderSpec,
    focus_handle: &FocusHandle,
    active_preedit: Option<&PreeditState>,
    pane_drag_hover: Option<PaneDragHover>,
    divider_drag_active: bool,
    breathing_enabled: bool,
    cx: &mut Context<LaboLaboApp>,
) -> AnyElement {
    let is_focused_leaf = node.panes.iter().any(|p| p.id == focused_pane);
    let selected_id = node.selected_pane().map(|p| p.id);
    let selected_kind = node.selected_pane().map(|p| p.kind);
    let runtime = selected_id.and_then(|id| runtimes.get(&id));
    let is_terminal_leaf = selected_kind == Some(PaneKind::Terminal);
    let leaf_pane_ids: Vec<PaneId> = node.panes.iter().map(|p| p.id).collect();
    // This leaf's selected tab *is* the app's single focused pane -- the
    // only canvas that should register the IME input handler / paint the
    // preedit overlay this frame (there's exactly one focused pane app-
    // wide, so at most one leaf's canvas ever matches).
    let is_input_target = selected_id == Some(focused_pane);

    // Terminal panes get the real canvas + PTY-driven mouse/IME wiring
    // below; a Files/Diff/Commits pane has no PTY (no `PaneRuntime` is ever
    // spawned for it, see `crate::app::LaboLaboApp::ensure_workspace_loaded`)
    // and gets its content from the shared per-Task `GitPaneState` instead
    // (`plans` W6d) -- no IME/text-selection/mouse-report wiring, and (per
    // `plans/012` §3.1) it never accepts an OS file drop either (that gate
    // lives on `is_terminal_leaf` further down, unchanged by this branch).
    let content: AnyElement = if is_terminal_leaf {
        let session_for_resize = runtime.map(|rt| rt.session.clone());
        let last_size = runtime.map(|rt| rt.last_size.clone());
        let last_bounds = runtime.map(|rt| rt.last_bounds.clone());
        let last_bounds_for_prepaint = last_bounds.clone();
        let snapshot = runtime.map(|rt| rt.session.snapshot());
        let selection = runtime.and_then(|rt| rt.selection);
        let prepaint_spec = spec.clone();
        let paint_spec = spec.clone();

        // `ElementInputHandler::new` needs the bounds `canvas` only hands us
        // inside the paint closure, so just the (focus_handle, entity) pair is
        // captured here -- constructed fresh every frame, matching
        // `Window::handle_input`'s "active for the upcoming frame only"
        // contract.
        let input_handler_setup = is_input_target.then(|| (focus_handle.clone(), cx.entity()));
        let preedit_text = is_input_target
            .then(|| {
                active_preedit
                    .filter(|p| p.task_id == task_id && p.pane_id == focused_pane)
                    .map(|p| p.text.clone())
            })
            .flatten();

        let canvas_el = canvas(
            move |bounds, _window, _cx| {
                if let Some(last_bounds) = &last_bounds_for_prepaint {
                    last_bounds.set(bounds);
                }
                // While any divider anywhere in this Task's tree is actively
                // being dragged, `Terminal::resize` is suppressed here --
                // `last_size` (and therefore whether a resize is due) is left
                // untouched, so the *first* prepaint after the drag ends (once
                // `divider_drag_active` goes back to `false` -- `app::
                // LaboLaboApp::finish_divider_drag` clears it and calls
                // `cx.notify()`) still detects the full accumulated size
                // change against whatever `last_size` was before the drag
                // started, and resizes exactly once with the final bounds.
                // This is the "間引き" (throttle) `plans` W5j #2 asks for,
                // applied as "finalize once at drag-end" rather than a
                // time-based interval -- deliberately simpler to reason about
                // than a mid-drag 16ms timer, and the task's own wording
                // permits either.
                if !divider_drag_active {
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
                }
            },
            move |bounds, _, window, cx| {
                if let Some((focus_handle, entity)) = input_handler_setup {
                    window.handle_input(
                        &focus_handle,
                        ElementInputHandler::new(bounds, entity),
                        cx,
                    );
                }
                if let Some(snapshot) = &snapshot {
                    crate::render::paint_grid(
                        snapshot,
                        &paint_spec,
                        selection.as_ref(),
                        bounds,
                        window,
                        cx,
                    );
                    if let Some(text) = &preedit_text {
                        crate::render::paint_preedit(
                            text,
                            &snapshot.cursor,
                            snapshot.cols,
                            &paint_spec,
                            bounds,
                            window,
                            cx,
                        );
                    }
                }
            },
        )
        .size_full();

        // Mouse wiring for this leaf's canvas: click-to-focus (pre-existing)
        // plus mouse-down/move/up for text selection *or* SGR mouse-report
        // forwarding (`app::LaboLaboApp::begin_selection` decides which, per
        // gesture -- see `crate::mouse_report`'s module doc comment) and
        // wheel/trackpad scroll, all keyed off `click_target` (this leaf's
        // selected pane -- the one a click or scroll here should act on) and
        // `task_id` (so a handler fired later, on whichever Task happens to be
        // selected then, still routes back to *this* leaf's own Task -- see
        // this function's doc comment). Each handler needs its own `move`
        // capture of `task_id` since `cx.listener` closures can't share one.
        let click_target = selected_id;
        let mousedown_task_id = task_id.to_string();
        let canvas_area = div().flex_1().w_full().on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                if let Some(id) = click_target {
                    this.select_pane(&mousedown_task_id, id, window, cx);
                    this.begin_selection(&mousedown_task_id, id, event, cx);
                }
            }),
        );

        let move_task_id = task_id.to_string();
        let canvas_area = canvas_area.on_mouse_move(cx.listener(
            move |this, event: &MouseMoveEvent, _window, cx| {
                // Only an active left-button drag extends a selection or a
                // mouse-report gesture -- a plain hover (no button held) does
                // neither. Bare-hover mouse-move forwarding (relevant only
                // under `MouseTracking::Any`) is out of scope for this wave --
                // see `crate::mouse_report`'s module doc comment.
                if !event.dragging() {
                    return;
                }
                if let Some(id) = click_target {
                    this.extend_selection(&move_task_id, id, event, cx);
                }
            },
        ));

        let mouseup_task_id = task_id.to_string();
        let canvas_area = canvas_area.on_mouse_up(
            MouseButton::Left,
            cx.listener(move |this, event: &MouseUpEvent, _window, cx| {
                if let Some(id) = click_target {
                    this.finish_selection(&mouseup_task_id, id, event, cx);
                }
            }),
        );

        // Right/middle-button clicks have no *local* behavior in this app (no
        // context menu, no paste-on-middle-click) -- they're wired only for
        // SGR mouse-report forwarding, a silent no-op when the pane's program
        // hasn't requested mouse tracking (or Shift is held).
        let right_down_task_id = task_id.to_string();
        let canvas_area = canvas_area.on_mouse_down(
            MouseButton::Right,
            cx.listener(move |this, event: &MouseDownEvent, _window, cx| {
                if let Some(id) = click_target {
                    this.report_mouse_click(
                        &right_down_task_id,
                        id,
                        crate::mouse_report::MouseButtonKind::Right,
                        event,
                        cx,
                    );
                }
            }),
        );
        let right_up_task_id = task_id.to_string();
        let canvas_area = canvas_area.on_mouse_up(
            MouseButton::Right,
            cx.listener(move |this, event: &MouseUpEvent, _window, _cx| {
                if let Some(id) = click_target {
                    this.report_mouse_release(
                        &right_up_task_id,
                        id,
                        crate::mouse_report::MouseButtonKind::Right,
                        event,
                    );
                }
            }),
        );
        let middle_down_task_id = task_id.to_string();
        let canvas_area = canvas_area.on_mouse_down(
            MouseButton::Middle,
            cx.listener(move |this, event: &MouseDownEvent, _window, cx| {
                if let Some(id) = click_target {
                    this.report_mouse_click(
                        &middle_down_task_id,
                        id,
                        crate::mouse_report::MouseButtonKind::Middle,
                        event,
                        cx,
                    );
                }
            }),
        );
        let middle_up_task_id = task_id.to_string();
        let canvas_area = canvas_area.on_mouse_up(
            MouseButton::Middle,
            cx.listener(move |this, event: &MouseUpEvent, _window, _cx| {
                if let Some(id) = click_target {
                    this.report_mouse_release(
                        &middle_up_task_id,
                        id,
                        crate::mouse_report::MouseButtonKind::Middle,
                        event,
                    );
                }
            }),
        );

        let scroll_task_id = task_id.to_string();
        let canvas_area = canvas_area.on_scroll_wheel(cx.listener(
            move |this, event: &ScrollWheelEvent, _window, cx| {
                if let Some(id) = click_target {
                    this.handle_pane_scroll(&scroll_task_id, id, event, cx);
                }
            },
        ));

        let canvas_area = canvas_area.child(canvas_el);
        canvas_area.into_any_element()
    } else {
        render_git_kind_body(task_id, selected_kind, selected_id, git_state, spec, cx)
    };

    let tab_bar = render_pane_tab_bar(
        task_id,
        node,
        runtimes,
        pane_status,
        pane_usage,
        spec,
        is_focused_leaf,
        breathing_enabled,
        cx,
    );

    let mut leaf = div()
        // A positioning context for the absolutely-positioned drop-zone
        // highlight overlay below (`move_drop_highlight_overlay`).
        .relative()
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
        .child(content);

    // DnD drop-target wiring (`plans/012-task-model-and-control-cli.md`
    // §3): this leaf accepts both an in-app tab-chip drag
    // (`TabDragPayload`, move/split/merge -- `on_drag`/`on_drag_move`/
    // `on_drop` on `InteractiveElement` require no `.id()`, unlike the
    // *source* chip's `on_drag`, see `render_pane_tab_bar`) and an OS
    // file/folder drag (gpui's `ExternalPaths`, terminal leaves only --
    // §3.1). `selected_id` (this leaf's anchor/target pane) doubles as the
    // leaf's drop-target identity everywhere below, matching
    // `PaneFrameView.performDragOperation`'s `node.selectedPane?.id` use.
    if let Some(anchor_pane_id) = selected_id {
        let hover_task_id = task_id.to_string();
        let hover_pane_ids = leaf_pane_ids.clone();
        leaf = leaf.on_drag_move::<TabDragPayload>(cx.listener(
            move |app, event: &DragMoveEvent<TabDragPayload>, _window, cx| {
                app.update_pane_drag_hover(
                    &hover_task_id,
                    anchor_pane_id,
                    &hover_pane_ids,
                    event,
                    cx,
                );
            },
        ));

        let drop_task_id = task_id.to_string();
        leaf = leaf.on_drop::<TabDragPayload>(cx.listener(
            move |app, payload: &TabDragPayload, window, cx| {
                app.finish_pane_drag_drop(&drop_task_id, anchor_pane_id, payload, window, cx);
            },
        ));

        // §3.1: files dropped on a non-terminal pane (diff/files/commits)
        // are "無反応" -- `can_drop` gates the drop itself, `drag_over`
        // gates the visual feedback, matching each other so the hover
        // highlight never lies about whether a drop will do anything.
        leaf = leaf
            .can_drop(move |any, _window, _cx| {
                any.downcast_ref::<ExternalPaths>()
                    .map(|_| is_terminal_leaf)
                    .unwrap_or(true)
            })
            .drag_over::<ExternalPaths>(move |style, _paths, _window, _cx| {
                if is_terminal_leaf {
                    style.bg(rgba(FILE_DROP_HIGHLIGHT_COLOR))
                } else {
                    style
                }
            });

        let file_task_id = task_id.to_string();
        leaf = leaf.on_drop::<ExternalPaths>(cx.listener(
            move |app, paths: &ExternalPaths, _window, cx| {
                app.handle_file_drop(&file_task_id, anchor_pane_id, paths, cx);
            },
        ));

        if let Some(hover) = pane_drag_hover {
            if hover.target_pane_id == anchor_pane_id {
                leaf = leaf.child(move_drop_highlight_overlay(
                    task_id,
                    anchor_pane_id,
                    hover.edge,
                ));
            }
        }
    }

    leaf.into_any_element()
}

/// Body for a non-terminal (`Files`/`Diff`/`Commits`) leaf (`plans` W6d) --
/// reuses `crate::git_pane`'s existing `Files`/`Diff` content-rendering
/// functions verbatim (the *same* bodies the fixed right pane shows, not a
/// second implementation of the same feature -- see [`git_pane::
/// render_file_list`]/[`git_pane::render_detail`]'s own doc comments) and
/// `crate::commit_pane` for the commit graph. `git_state` is `task_id`'s
/// shared per-Task [`GitPaneState`] (`TaskWorkspace::git`) -- the same
/// state object the fixed pane reads, so selecting a file in a `Files` tile
/// updates the very state a `Diff` tile (tile *or* fixed pane) reads back,
/// keeping "the selected file" in sync everywhere it's shown at once.
///
/// A click anywhere in the body focuses this leaf (mirrors the terminal
/// canvas's click-to-focus), so tab switching/DnD/the focus border behave
/// the same regardless of pane kind -- but unlike the terminal branch there
/// is no PTY here, so no IME/text-selection/mouse-report wiring at all: a
/// keystroke while this leaf is focused simply has nowhere to go (`plans`
/// W6d's "非 Terminal ペインは...キー入力はフォーカスされても PTY に流さ
/// ない").
fn render_git_kind_body(
    task_id: &str,
    kind: Option<PaneKind>,
    anchor_pane_id: Option<PaneId>,
    git_state: &GitPaneState,
    spec: &RenderSpec,
    cx: &mut Context<LaboLaboApp>,
) -> AnyElement {
    let inner = match kind {
        Some(PaneKind::Files) => {
            git_pane::render_file_list(task_id, git_state, spec, cx).into_any_element()
        }
        Some(PaneKind::Diff) => git_pane::render_detail(task_id, git_state, spec, cx),
        Some(PaneKind::Commits) => {
            commit_pane::render_commits_pane(task_id, &git_state.commits, spec, cx)
        }
        Some(PaneKind::Terminal) | None => div().size_full().into_any_element(),
    };

    let focus_task_id = task_id.to_string();
    div()
        .id(SharedString::from(format!(
            "git-tile-body-{task_id}-{anchor_pane_id:?}"
        )))
        .flex_1()
        .w_full()
        .min_h(px(0.0))
        .overflow_hidden()
        .bg(rgb(theme::surface::SUNKEN))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                if let Some(id) = anchor_pane_id {
                    this.select_pane(&focus_task_id, id, window, cx);
                }
            }),
        )
        .child(inner)
        .into_any_element()
}

/// The tab/pane-move drag's drop-zone highlight for one [`DropEdge`]
/// quadrant: half the leaf for `Left`/`Right`/`Top`/`Bottom`, the whole
/// leaf for `Center` (tab merge) -- mirrors `PaneFrameView.highlightRect
/// (for:)`. Expressed purely in fractions of the parent (`relative(..)`),
/// not pixel bounds, so it needs no separate knowledge of the leaf's actual
/// on-screen size -- it just fills the right quadrant of whatever the
/// parent (a `.relative()` leaf div) currently measures.
///
/// Fades in over [`motion::DROP_ZONE_FADE_IN`] (`plans/014` M3) -- this
/// element only exists in the tree while a drag is actively hovering this
/// exact `(anchor_pane_id, edge)` combination, so an id keyed on both
/// (`target_pane_id`/`edge`, mixed into the element id below) means a
/// zone change mounts a genuinely new element and restarts the fade from
/// scratch (`plans/014` M3's "対象ゾーンが変わったら新ゾーンで再スタート
/// （単発アニメの掛け直しで可）"). Disappearance is instant (the element
/// is simply removed once `pane_drag_hover` no longer matches), matching
/// M3's "消滅は即時でよい（ドロップ/キャンセルの応答は snappy に）".
fn move_drop_highlight_overlay(
    task_id: &str,
    target_pane_id: PaneId,
    edge: DropEdge,
) -> impl IntoElement {
    let (left, top, width, height): (f32, f32, f32, f32) = match edge {
        DropEdge::Left => (0.0, 0.0, 0.5, 1.0),
        DropEdge::Right => (0.5, 0.0, 0.5, 1.0),
        DropEdge::Top => (0.0, 0.0, 1.0, 0.5),
        DropEdge::Bottom => (0.0, 0.5, 1.0, 0.5),
        DropEdge::Center => (0.0, 0.0, 1.0, 1.0),
    };
    let id: SharedString = format!("drop-hover-{task_id}-{target_pane_id:?}-{edge:?}").into();
    div()
        .absolute()
        .left(relative(left))
        .top(relative(top))
        .w(relative(width))
        .h(relative(height))
        .bg(rgba(MOVE_DROP_HIGHLIGHT_COLOR))
        .with_animation(
            id,
            Animation::new(motion::DROP_ZONE_FADE_IN).with_easing(motion::ease_out_strong()),
            |el, t| el.opacity(t),
        )
}

/// One leaf's tab bar. Identical to wave 5b-2's `app::render_pane_tab_bar`
/// other than threading `task_id` through both click handlers, plus
/// (`plans/013`/`plans/014`) the selected tab's `ACCENT` top border, the
/// usage label's terminal-font `CAPTION` styling (needs `spec`), and the
/// status dot's crossfade/breathing (needs each pane's own `PaneRuntime::
/// dot_anim` from `runtimes`, and `breathing_enabled` -- see
/// `motion::status_dot_element`'s doc comment).
///
/// Deliberately **no** hover/press feedback on the chip itself or the tab
/// switch it triggers (`plans/014` principle 2: "タブ切替...には遷移を
/// 足さない" -- a keyboard-frequency, high-repetition action stays
/// intentionally unadorned) -- only its `×`/`+` sub-actions (closing/
/// opening a tab, not switching one) get the M5 hover/press treatment.
#[allow(clippy::too_many_arguments)]
fn render_pane_tab_bar(
    task_id: &str,
    node: &TileNode,
    runtimes: &HashMap<PaneId, PaneRuntime>,
    pane_status: &HashMap<PaneId, AgentStatus>,
    pane_usage: &HashMap<PaneId, AgentUsage>,
    spec: &RenderSpec,
    is_focused: bool,
    breathing_enabled: bool,
    cx: &mut Context<LaboLaboApp>,
) -> impl IntoElement {
    let anchor = node.selected_pane().map(|p| p.id);

    div()
        .flex()
        .flex_row()
        .items_center()
        .h(px(grid::TAB_BAR_HEIGHT))
        .w_full()
        .bg(rgb(if is_focused {
            theme::surface::RAISED
        } else {
            theme::surface::SUNKEN
        }))
        .px_1()
        .gap_1()
        .children(node.panes.iter().map(|pane| {
            let selected = anchor == Some(pane.id);
            let pane_id = pane.id;
            let title: SharedString = pane.title.clone().into();
            let select_task_id = task_id.to_string();
            let close_task_id = task_id.to_string();
            let status = pane_status.get(&pane_id).copied();
            let status_color = status.and_then(status_dot_color);
            let is_running = status == Some(AgentStatus::Running);
            let dot_el = runtimes.get(&pane_id).and_then(|runtime| {
                motion::status_dot_element(
                    format!("tab-dot-{task_id}-{pane_id:?}"),
                    status_color,
                    is_running,
                    breathing_enabled,
                    &runtime.dot_anim,
                )
            });
            let usage_label: Option<SharedString> = pane_usage
                .get(&pane_id)
                .and_then(format_usage_compact)
                .map(SharedString::from);
            // `.id(..)` promotes this chip to `Stateful<Div>`, the only
            // element kind `.on_drag` (`StatefulInteractiveElement`) is
            // available on -- mirrors Swift's `PaneTabChip.onDrag`, one drag
            // source per chip (`plans/012-task-model-and-control-cli.md`
            // §3's "各チップが個別に onDrag").
            let chip_id: SharedString = format!("tab-chip-{task_id}-{pane_id:?}").into();
            let preview_title = title.clone();
            div()
                .id(chip_id)
                .flex()
                .flex_row()
                .items_center()
                .gap_1()
                .px_2()
                .rounded_sm()
                // `plans/013` §4: the selected tab connects to the terminal
                // below it via an `ACCENT` top border, on top of the
                // `ACTIVE` fill; unselected tabs are plain `RAISED`, no
                // border -- both switches are instant (no easing), per
                // this function's own doc comment on tab-switch being
                // deliberately unadorned.
                .when(selected, |el| {
                    el.bg(rgb(theme::surface::ACTIVE))
                        .border_t(px(2.0))
                        .border_color(rgb(theme::ACCENT))
                })
                .when(!selected, |el| el.bg(rgb(theme::surface::RAISED)))
                .children(dot_el)
                .when_some(usage_label, |el, label| {
                    el.child(
                        div()
                            .font(spec.font.clone())
                            .text_size(px(theme::font_size::CAPTION))
                            .text_color(rgb(theme::text::SECONDARY))
                            .child(label),
                    )
                })
                .on_drag(
                    TabDragPayload {
                        source_pane_id: pane_id,
                    },
                    move |_payload, _offset, _window, cx| {
                        cx.new(|_cx| TabDragPreview(preview_title.clone()))
                    },
                )
                .child(
                    div()
                        .px_1()
                        .text_color(rgb(theme::text::PRIMARY))
                        .text_size(px(theme::font_size::LABEL))
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
                        .id(SharedString::from(format!(
                            "tab-close-{task_id}-{pane_id:?}"
                        )))
                        .px_1()
                        .rounded_sm()
                        .text_color(rgb(theme::text::SECONDARY))
                        .hover(|el| el.bg(rgb(theme::surface::ACTIVE)))
                        .active(|el| el.opacity(0.7))
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
                .id(SharedString::from(format!("tab-add-{task_id}")))
                .px_2()
                .rounded_sm()
                .text_color(rgb(theme::text::PRIMARY))
                .hover(|el| el.bg(rgb(theme::surface::ACTIVE)))
                .active(|el| el.opacity(0.7))
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
        // `plans` W6d §3.1: "フォーカスペインのタブバー「+」の隣...に
        // 「Git ペインを開く ▸ 変更ファイル/Diff/コミット」" -- only the
        // *focused* pane's tab bar gets these (not every leaf's), so they
        // don't clutter every tab bar in a busy layout.
        .when(is_focused, |el| {
            el.child(render_open_git_tile_buttons(task_id, cx))
        })
}

/// The three small icon buttons (`plans` W6d §3.1) that open this Task's
/// `Files`/`Diff`/`Commits` Git-tile panes -- brings the pane to front if
/// it already exists somewhere in the tree (possibly as a hidden tab),
/// otherwise splits a new one off the current layout (see
/// `LaboLaboApp::open_git_tile_pane`). Same small-square-icon-+-native-
/// tooltip shape as `crate::sidebar::icon_button` (reusing its
/// [`crate::sidebar::IconTooltip`] view rather than a second one), just
/// sized to fit a tab bar rather than the sidebar.
fn render_open_git_tile_buttons(task_id: &str, cx: &mut Context<LaboLaboApp>) -> impl IntoElement {
    // Plain Unicode glyphs, no emoji (project policy -- see
    // `crate::sidebar::icon_button`'s doc comment): ▤ (list/file lines) for
    // Files, ± for Diff, ⧖ (hourglass -- history) for Commits.
    let buttons = [
        (
            PaneKind::Files,
            "\u{25a4}",
            t!("git.tile_button.files_tooltip").to_string(),
        ),
        (
            PaneKind::Diff,
            "\u{00b1}",
            t!("git.tile_button.diff_tooltip").to_string(),
        ),
        (
            PaneKind::Commits,
            "\u{29d6}",
            t!("git.tile_button.commits_tooltip").to_string(),
        ),
    ];
    let mut row = div().flex().flex_row().items_center().gap_1().pl_1();
    for (kind, glyph, tooltip) in buttons {
        let open_task_id = task_id.to_string();
        let id: SharedString = format!("tab-open-git-{}-{task_id}", kind.raw_value()).into();
        let tooltip_text: SharedString = tooltip.into();
        row = row.child(
            div()
                .id(id)
                .w(px(20.0))
                .h(px(20.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded_sm()
                .text_color(rgb(theme::text::SECONDARY))
                .text_size(px(11.0))
                .hover(|el| el.bg(rgb(theme::surface::ACTIVE)))
                .active(|el| el.opacity(0.7))
                .tooltip(move |_window, cx| {
                    cx.new(|_| crate::sidebar::IconTooltip(tooltip_text.clone()))
                        .into()
                })
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                        this.open_git_tile_pane(&open_task_id, kind, window, cx);
                    }),
                )
                .child(glyph),
        );
    }
    row
}

#[cfg(test)]
mod tests {
    use super::*;

    // MARK: - format_usage_compact / format_compact_count (tab-chip label)

    #[test]
    fn empty_usage_has_no_label() {
        assert_eq!(format_usage_compact(&AgentUsage::default()), None);
    }

    #[test]
    fn small_token_counts_are_shown_verbatim() {
        let usage = AgentUsage {
            input_tokens: 100,
            output_tokens: 32,
            ..Default::default()
        };
        assert_eq!(format_usage_compact(&usage).as_deref(), Some("132 tok"));
    }

    #[test]
    fn thousands_are_compacted_with_one_decimal() {
        let usage = AgentUsage {
            input_tokens: 1_234,
            ..Default::default()
        };
        assert_eq!(format_usage_compact(&usage).as_deref(), Some("1.2k tok"));
    }

    #[test]
    fn millions_are_compacted_with_one_decimal() {
        let usage = AgentUsage {
            input_tokens: 2_500_000,
            ..Default::default()
        };
        assert_eq!(format_usage_compact(&usage).as_deref(), Some("2.5M tok"));
    }

    #[test]
    fn known_model_pricing_appends_estimated_cost() {
        let usage = AgentUsage {
            input_tokens: 1_000_000,
            output_tokens: 1_000_000,
            model: Some("claude-opus-4-8".to_string()),
            ..Default::default()
        };
        // opus: $15 input + $75 output per Mtok = $90 for 1M/1M.
        assert_eq!(
            format_usage_compact(&usage).as_deref(),
            Some("2.0M tok \u{b7} $90.00")
        );
    }

    #[test]
    fn unknown_model_pricing_omits_cost() {
        let usage = AgentUsage {
            input_tokens: 500,
            model: Some("some-unknown-model".to_string()),
            ..Default::default()
        };
        assert_eq!(format_usage_compact(&usage).as_deref(), Some("500 tok"));
    }

    #[test]
    fn turns_only_with_zero_tokens_is_not_empty_and_shows_zero() {
        // `AgentUsage::is_empty()` requires *both* turns == 0 and
        // total_tokens() == 0 -- a turn with all-zero usage fields (e.g. a
        // parse edge case) still counts as "something observed", matching
        // `AgentUsage::is_empty`'s own contract.
        let usage = AgentUsage {
            turns: 1,
            ..Default::default()
        };
        assert_eq!(format_usage_compact(&usage).as_deref(), Some("0 tok"));
    }
}
