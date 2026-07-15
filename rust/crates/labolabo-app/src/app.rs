//! The gpui root view: window state, the tile/tab tree (`labolabo_core::
//! tiling::PaneTilingModel`), key/click routing, and the recursive render
//! tree (split panes, each with its own tab bar + active terminal canvas).
//!
//! ## Tile/tab tree (wave 5b-2)
//!
//! `TerminalApp` used to own a flat `Vec<Tab>` (one tab group for the whole
//! window, no splits) -- a deliberate placeholder, per that type's own doc
//! comment. This wave replaces it with `labolabo_core::tiling::
//! PaneTilingModel`, the same tile/tab tree ported from the Swift app's
//! `PaneTilingModel.swift` (see that crate's `tiling.rs` module doc comment
//! for the full porting story). One window is one [`PaneTilingModel`]; a
//! [`PaneRuntime`] (a real `labolabo_term::Terminal` session + its redraw
//! bridge) is kept for every `terminal`-kind [`labolabo_core::PaneItem`] in
//! the tree, keyed by its stable [`PaneId`] -- including hidden (non-selected
//! tab) panes, so their pty/scrollback survive tab switches, splits, and
//! closes elsewhere in the tree, matching the Swift app's `contentCache`
//! behavior. Only `PaneKind::Terminal` panes are ever created this wave
//! (Files/Diff/Commits panes -- the changed-files/diff/commit-history views
//! -- are a future wave's concern; `PaneTilingModel::default_layout()` isn't
//! used here for that reason, see `TerminalApp::new`).
//!
//! Focus (which pane's tab receives keystrokes) is tracked as a single
//! `PaneId`, not a `NodeId` -- see `crate::focus`'s module doc comment for
//! why, and for the pure (gpui-independent, unit-tested) focus-resolution
//! logic split/close/tab-cycle rely on.
//!
//! Each split pane's terminal grid is sized from its own laid-out on-screen
//! area (not the whole window) -- see [`render_leaf`]'s doc comment for how
//! that reacts to both window resizes and split-ratio changes without this
//! module having to reimplement gpui's own flex layout math.
//!
//! ## Session lifecycle
//!
//! A tab closes three ways, all funneling through [`TerminalApp::remove_pane`]
//! (removal is always by pane **id**, never by tree position -- positions
//! shift as the tree changes):
//!
//! - **The shell exits** (`exit`, or the child dying): the session's redraw
//!   bridge sees [`TermEvent::Exit`] and calls [`TerminalApp::handle_pane_exit`],
//!   which removes the pane without signaling it (the child is already dead).
//! - **The user clicks a tab's "x"**, or **presses Cmd+W** (closes the
//!   focused pane's active tab): [`TerminalApp::close_pane_user`] calls
//!   [`labolabo_term::Terminal::shutdown`] (SIGHUP to the child -- what a real
//!   terminal sends on window close) before removing the pane.
//!
//! When the tree's last pane's last tab closes (either way), the app quits
//! (`cx.quit()`), matching Ghostty's close-last-surface behavior -- same as
//! wave 5a, just decided from `model.panes().len() == 1` instead of an empty
//! `Vec<Tab>`.

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use futures::channel::mpsc;
use futures::StreamExt;
use gpui::{
    actions, canvas, div, prelude::*, px, relative, rgb, AnyElement, Context, FocusHandle,
    IntoElement, KeyDownEvent, MouseButton, MouseDownEvent, Render, SharedString, Task, Timer,
    Window,
};

use labolabo_core::{PaneId, PaneItem, PaneKind, PaneTilingModel, TileNode, TileOrientation};
use labolabo_term::{ColorScheme, TermEvent, Terminal};

use crate::focus;
use crate::ghostty_config::FontConfig;
use crate::grid;
use crate::keys::keystroke_to_bytes;
use crate::render::{self, RenderSpec};

/// How long the redraw-bridge thread blocks on `recv_event` between checks
/// of whether its gpui-side `Task` was dropped (pane closed). Real events
/// (`TermEvent::Wakeup`/`Exit`) are delivered the instant they happen,
/// regardless of this value -- it only bounds how quickly a *closed* pane's
/// bridge thread notices there's no one left to notify and exits. This is
/// not a redraw poll: `Render::render` only ever re-runs from an actual
/// `cx.notify()` call, which only ever happens in response to a real
/// `TermEvent`.
const EVENT_POLL_TIMEOUT: Duration = Duration::from_millis(250);

/// Minimum gap between two `cx.notify()` calls for the same pane, mirroring
/// `labolabo_term::session`'s own ~60fps snapshot-construction throttle so
/// this UI layer never asks gpui to redraw faster than the terminal core
/// itself paces snapshots.
const FRAME_INTERVAL: Duration = Duration::from_millis(16);

/// Initial grid size for a pane created after startup (new tab via Cmd+T/"+",
/// or a split via Cmd+D/Cmd+Shift+D): a conventional terminal default,
/// immediately corrected by the pane's own canvas once gpui lays it out (see
/// [`render_leaf`]) -- unlike the app's *first* pane (sized from the full
/// window viewport in [`TerminalApp::new`]), a freshly split/tabbed pane's
/// eventual on-screen area isn't known until that first layout pass, so
/// there's no better estimate to start from.
const DEFAULT_PANE_COLS: u16 = 80;
const DEFAULT_PANE_ROWS: u16 = 24;

/// Accent color for the focused pane's frame border (a restrained highlight,
/// per the wave's brief -- not a full glow/shadow treatment).
const FOCUS_BORDER_COLOR: u32 = 0x5e9eff;
/// Frame border color for every other (unfocused) pane.
const IDLE_BORDER_COLOR: u32 = 0x1c1c1c;

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

/// What the per-pane bridge thread forwards to its gpui-side task.
enum BridgeMsg {
    /// A new snapshot is available -- repaint (coalesced + frame-paced).
    Wakeup,
    /// The session's child exited -- close the pane. Terminal: the bridge
    /// thread stops after sending this.
    Exit,
}

/// The live resources backing one `terminal`-kind [`PaneItem`]: its real
/// `labolabo_term::Terminal` session and the redraw bridge that keeps gpui
/// notified of it. Kept for every terminal pane in the tree -- including
/// hidden ones -- so pty/scrollback survive tab switches and split/close
/// elsewhere in the tree.
struct PaneRuntime {
    session: Arc<Terminal>,
    /// Last (cols, rows) this pane's session was resized to. Shared
    /// (`Rc<Cell<_>>`, not a plain field) because the canvas element's
    /// `prepaint` closure -- the one place a pane's actual laid-out pixel
    /// area is known, see [`render_leaf`] -- runs without a `&mut
    /// TerminalApp` borrow available to it; this is the one piece of state
    /// it needs to read and write across repaints without one.
    last_size: Rc<Cell<(u16, u16)>>,
    /// Keeps the redraw-bridge task alive for the pane's lifetime; dropping
    /// it (on pane close) is the signal the bridge thread uses to stop.
    _redraw_task: Task<()>,
}

pub struct TerminalApp {
    model: PaneTilingModel,
    runtimes: HashMap<PaneId, PaneRuntime>,
    /// The tab with keyboard focus. Always resolvable via
    /// `model.root.find_leaf(focused_pane)`, and always that leaf's
    /// currently selected tab -- see `crate::focus`'s module doc comment for
    /// why this invariant makes `PaneId` (not a leaf `NodeId`) the right
    /// thing to track.
    focused_pane: PaneId,
    focus_handle: FocusHandle,
    spec: RenderSpec,
    /// The user's Ghostty color configuration, applied to every pane's
    /// `Terminal` at spawn time (see `spawn_runtime`) -- stored so panes
    /// created after startup (new tab, split) get it too.
    colors: ColorScheme,
}

impl TerminalApp {
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

        // One terminal pane, filling the whole window -- this wave's initial
        // layout. Deliberately not `PaneTilingModel::default_layout()`
        // (terminal + commits/files/diff): those pane kinds have no
        // rendering yet in this crate (see the module doc comment).
        let pane = PaneItem::new(PaneKind::Terminal, PaneKind::Terminal.default_title());
        let pane_id = pane.id;
        let root = TileNode::leaf(pane);

        let mut this = Self {
            model: PaneTilingModel::new(root),
            runtimes: HashMap::new(),
            focused_pane: pane_id,
            focus_handle,
            spec,
            colors: color_config.clone(),
        };

        let (cols, rows) = this.viewport_grid_size(window);
        this.spawn_runtime(pane_id, cols, rows, cx);

        cx.observe_window_bounds(window, |_this, _window, cx| {
            // The per-pane canvas elements (see `render_leaf`) read their
            // own bounds and resize their own session directly at
            // prepaint time; all a window resize needs to do here is force
            // a fresh layout/paint pass so that runs again.
            cx.notify();
        })
        .detach();

        this
    }

    /// The terminal grid size for the window's current viewport (full
    /// window). Used only for the very first pane (see `new`) -- every pane
    /// created afterward (new tab / split) starts at [`DEFAULT_PANE_COLS`]/
    /// [`DEFAULT_PANE_ROWS`] instead, since its eventual on-screen area is a
    /// fraction of the window, not the whole thing.
    fn viewport_grid_size(&self, window: &Window) -> (u16, u16) {
        let size = window.viewport_size();
        grid::grid_size_for_window(
            size.width.into(),
            size.height.into(),
            self.spec.cell_width,
            self.spec.cell_height,
        )
    }

    /// Spawn a new `terminal`-kind pane's session (a fresh login-shell
    /// `Terminal`) and register its redraw bridge. No-op (with a stderr
    /// warning) if the spawn itself fails -- mirrors wave 5a's `open_tab`.
    fn spawn_runtime(&mut self, pane_id: PaneId, cols: u16, rows: u16, cx: &mut Context<Self>) {
        let session = match Terminal::spawn_with_options(cols, rows, None, &[], &self.colors) {
            Ok(session) => Arc::new(session),
            Err(err) => {
                eprintln!("labolabo-app: failed to spawn terminal session: {err:#}");
                return;
            }
        };
        let redraw_task = spawn_redraw_bridge(session.clone(), pane_id, cx);
        self.runtimes.insert(
            pane_id,
            PaneRuntime {
                session,
                last_size: Rc::new(Cell::new((cols, rows))),
                _redraw_task: redraw_task,
            },
        );
    }

    // MARK: - focus / selection

    /// Selects `pane_id`'s tab within its leaf and gives that pane keyboard
    /// focus. Used by tab-chip clicks, clicking into a pane's terminal area,
    /// and every action handler that lands on a specific pane.
    fn select_pane(&mut self, pane_id: PaneId, window: &mut Window, cx: &mut Context<Self>) {
        self.model.select_tab(pane_id);
        self.focused_pane = pane_id;
        window.focus(&self.focus_handle);
        cx.notify();
    }

    fn move_focus(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(next) = focus::adjacent_pane(&self.model, self.focused_pane, forward) {
            self.focused_pane = next;
            window.focus(&self.focus_handle);
            cx.notify();
        }
    }

    fn select_tab_index(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(pane_id) = focus::nth_tab(&self.model, self.focused_pane, index) {
            self.select_pane(pane_id, window, cx);
        }
    }

    // MARK: - mutations

    /// Adds a new terminal tab to `anchor_pane_id`'s tab group and focuses
    /// it. Used by Cmd+T (anchored on the focused pane) and a leaf's "+"
    /// button (anchored on that leaf's own selected pane).
    fn add_tab_to(&mut self, anchor_pane_id: PaneId, window: &mut Window, cx: &mut Context<Self>) {
        let pane = PaneItem::new(PaneKind::Terminal, PaneKind::Terminal.default_title());
        let new_id = pane.id;
        if self.model.add_tab(anchor_pane_id, pane) {
            self.spawn_runtime(new_id, DEFAULT_PANE_COLS, DEFAULT_PANE_ROWS, cx);
            self.focused_pane = new_id;
            window.focus(&self.focus_handle);
            cx.notify();
        }
    }

    /// Splits the focused pane's leaf, opening a new terminal pane on the
    /// `orientation` side and focusing it.
    fn split_focused(
        &mut self,
        orientation: TileOrientation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.model.root.find_leaf(self.focused_pane).is_none() {
            return;
        }
        let pane = PaneItem::new(PaneKind::Terminal, PaneKind::Terminal.default_title());
        let new_id = pane.id;
        self.model.split(self.focused_pane, orientation, pane);
        self.spawn_runtime(new_id, DEFAULT_PANE_COLS, DEFAULT_PANE_ROWS, cx);
        self.focused_pane = new_id;
        window.focus(&self.focus_handle);
        cx.notify();
    }

    /// User-driven close (Cmd+W, or a tab chip's "x"): signals the child
    /// (SIGHUP via `Terminal::shutdown`) before tearing the pane down.
    fn close_pane_user(&mut self, pane_id: PaneId, cx: &mut Context<Self>) {
        self.remove_pane(pane_id, true, cx);
    }

    /// The pane's session exited on its own (`TermEvent::Exit`): the child
    /// is already dead, so no shutdown signal is needed.
    fn handle_pane_exit(&mut self, pane_id: PaneId, cx: &mut Context<Self>) {
        self.remove_pane(pane_id, false, cx);
    }

    /// Removes `pane_id` from the tree (no-op for an unknown id -- e.g. an
    /// `Exit` arriving for a pane the user already closed). Quits the app
    /// (Ghostty's close-last-surface behavior) if `pane_id` was the tree's
    /// only remaining pane; otherwise closes it via the model and, if it had
    /// focus, resolves a new focused pane via [`focus::resolve_close_focus`].
    fn remove_pane(&mut self, pane_id: PaneId, shutdown_child: bool, cx: &mut Context<Self>) {
        if self.model.root.find_leaf(pane_id).is_none() {
            return;
        }
        let is_last_pane = self.model.panes().len() == 1;
        let was_focused = self.focused_pane == pane_id;

        if let Some(runtime) = self.runtimes.remove(&pane_id) {
            if shutdown_child {
                runtime.session.shutdown();
            }
            // `runtime` (and its `_redraw_task`) drops here, ending the
            // bridge thread.
        }

        if is_last_pane {
            cx.quit();
            return;
        }

        let revealed = self.model.close(pane_id);
        if was_focused {
            if let Some(new_focus) = focus::resolve_close_focus(&self.model, revealed) {
                self.focused_pane = new_focus;
            }
        }
        cx.notify();
    }

    // MARK: - input routing

    fn key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, _cx: &mut Context<Self>) {
        let Some(runtime) = self.runtimes.get(&self.focused_pane) else {
            return;
        };
        // TODO(W5a): IME composition is not wired up here -- see
        // `keys::keystroke_to_bytes`'s module doc comment.
        if let Some(bytes) = keystroke_to_bytes(&event.keystroke) {
            runtime.session.write_input(&bytes);
        }
    }

    // MARK: - action handlers (see the `actions!` list + main.rs's `bind_keys`)

    fn action_new_tab(&mut self, _: &NewTab, window: &mut Window, cx: &mut Context<Self>) {
        self.add_tab_to(self.focused_pane, window, cx);
    }

    fn action_close_tab(&mut self, _: &CloseTab, window: &mut Window, cx: &mut Context<Self>) {
        self.close_pane_user(self.focused_pane, cx);
        window.focus(&self.focus_handle);
    }

    fn action_split_right(&mut self, _: &SplitRight, window: &mut Window, cx: &mut Context<Self>) {
        self.split_focused(TileOrientation::Horizontal, window, cx);
    }

    fn action_split_down(&mut self, _: &SplitDown, window: &mut Window, cx: &mut Context<Self>) {
        self.split_focused(TileOrientation::Vertical, window, cx);
    }

    fn action_focus_next_pane(
        &mut self,
        _: &FocusNextPane,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_focus(true, window, cx);
    }

    fn action_focus_prev_pane(
        &mut self,
        _: &FocusPrevPane,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_focus(false, window, cx);
    }

    fn action_select_tab_1(&mut self, _: &SelectTab1, window: &mut Window, cx: &mut Context<Self>) {
        self.select_tab_index(0, window, cx);
    }
    fn action_select_tab_2(&mut self, _: &SelectTab2, window: &mut Window, cx: &mut Context<Self>) {
        self.select_tab_index(1, window, cx);
    }
    fn action_select_tab_3(&mut self, _: &SelectTab3, window: &mut Window, cx: &mut Context<Self>) {
        self.select_tab_index(2, window, cx);
    }
    fn action_select_tab_4(&mut self, _: &SelectTab4, window: &mut Window, cx: &mut Context<Self>) {
        self.select_tab_index(3, window, cx);
    }
    fn action_select_tab_5(&mut self, _: &SelectTab5, window: &mut Window, cx: &mut Context<Self>) {
        self.select_tab_index(4, window, cx);
    }
    fn action_select_tab_6(&mut self, _: &SelectTab6, window: &mut Window, cx: &mut Context<Self>) {
        self.select_tab_index(5, window, cx);
    }
    fn action_select_tab_7(&mut self, _: &SelectTab7, window: &mut Window, cx: &mut Context<Self>) {
        self.select_tab_index(6, window, cx);
    }
    fn action_select_tab_8(&mut self, _: &SelectTab8, window: &mut Window, cx: &mut Context<Self>) {
        self.select_tab_index(7, window, cx);
    }
    fn action_select_tab_9(&mut self, _: &SelectTab9, window: &mut Window, cx: &mut Context<Self>) {
        self.select_tab_index(8, window, cx);
    }
}

impl Render for TerminalApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let spec = self.spec.clone();
        let focused_pane = self.focused_pane;
        let tree = render_tile(&self.model.root, &self.runtimes, focused_pane, &spec, cx);

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
            .flex_col()
            .size_full()
            .bg(rgb(0x000000))
            .child(tree)
    }
}

/// Recursively render one node of the tile tree: a leaf becomes a pane frame
/// ([`render_leaf`]); a split becomes a flex row/column (per
/// [`TileOrientation`]) with its two children sized by `node.ratio`.
fn render_tile(
    node: &TileNode,
    runtimes: &HashMap<PaneId, PaneRuntime>,
    focused_pane: PaneId,
    spec: &RenderSpec,
    cx: &mut Context<TerminalApp>,
) -> AnyElement {
    if node.is_leaf() {
        return render_leaf(node, runtimes, focused_pane, spec, cx);
    }

    let Some(first_child) = node.children.first() else {
        return div().size_full().into_any_element();
    };
    let Some(second_child) = node.children.get(1) else {
        // A split with fewer than 2 children shouldn't happen (the model
        // always creates 2-way splits) -- render the one child we do have
        // rather than drop its content on the floor.
        return render_tile(first_child, runtimes, focused_pane, spec, cx);
    };

    // Horizontal orientation lays children out side by side (a row);
    // vertical stacks them top/bottom (a column) -- see `tiling.rs`'s
    // `default_layout` doc comment for the canonical example this mirrors.
    let is_row = node.orientation == TileOrientation::Horizontal;
    let ratio = (node.ratio as f32).clamp(0.05, 0.95);

    let first_el = render_tile(first_child, runtimes, focused_pane, spec, cx);
    let second_el = render_tile(second_child, runtimes, focused_pane, spec, cx);

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

/// Render one leaf (tab group): a tab bar + the active tab's terminal
/// canvas, framed with a focus-indicating border.
///
/// ## Per-pane sizing
///
/// The canvas's `prepaint` closure (not `paint`) does the resize-diffing:
/// `canvas`'s two callbacks are the one place gpui hands this element its
/// actual laid-out `Bounds<Pixels>` (post-flex/post-ratio, so it reacts to
/// both window resizes and split changes without this module reimplementing
/// gpui's flex math itself), and `prepaint` is `FnOnce` per repaint, so
/// calling `Terminal::resize` there -- guarded by the pane's own
/// `last_size` cache -- runs it exactly once per frame where the size
/// actually changed. Both closures run outside any `&mut TerminalApp`
/// borrow (gpui drives them later, during its own paint pass, not
/// synchronously from `render`), which is why the resize path only needs
/// `&Terminal` (an `Arc` clone) and a shared `Rc<Cell<_>>`, not app state.
fn render_leaf(
    node: &TileNode,
    runtimes: &HashMap<PaneId, PaneRuntime>,
    focused_pane: PaneId,
    spec: &RenderSpec,
    cx: &mut Context<TerminalApp>,
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
                render::paint_grid(snapshot, &paint_spec, bounds, window, cx);
            }
        },
    )
    .size_full();

    let click_target = selected_id;
    let canvas_area = div()
        .flex_1()
        .w_full()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                if let Some(id) = click_target {
                    this.select_pane(id, window, cx);
                }
            }),
        )
        .child(canvas_el);

    let tab_bar = render_pane_tab_bar(node, is_focused_leaf, cx);

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

/// One leaf's tab bar: a chip per tab (click to select, "x" to close) plus a
/// trailing "+" to add another tab to this same group -- the same
/// click/close/add affordances wave 5a's single window-wide tab bar had, now
/// per pane.
fn render_pane_tab_bar(
    node: &TileNode,
    is_focused: bool,
    cx: &mut Context<TerminalApp>,
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
                                this.select_pane(pane_id, window, cx);
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
                                this.close_pane_user(pane_id, cx);
                                window.focus(&this.focus_handle);
                            }),
                        )
                        .child("\u{d7}"),
                )
        }))
        .child(
            div()
                .px_2()
                .text_color(rgb(0xe5e5e5))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                        if let Some(anchor) = anchor {
                            this.add_tab_to(anchor, window, cx);
                        }
                    }),
                )
                .child("+"),
        )
}

/// Bridges `labolabo_term`'s blocking [`Terminal::recv_event`] into gpui,
/// with the same two-stage coalesce-then-pace design as wave 5a's
/// (originally the `gpui-term-poc` spike's `spawn_redraw_task`): drain a
/// burst of already-queued wakeups into one redraw, then enforce
/// `FRAME_INTERVAL` as a minimum gap before the next one, draining anything
/// that arrived during that quiet window too. An `Exit` seen at any point
/// (awaited or drained) closes the pane via [`TerminalApp::handle_pane_exit`]
/// and ends the task.
///
/// `Terminal` exposes no async event stream (`recv_event` blocks the calling
/// thread), so a dedicated OS thread does the blocking wait and forwards
/// [`BridgeMsg`]s to the gpui-side async `Task` over an unbounded channel;
/// the `Task` is the one that actually calls `cx.notify()`, since only
/// gpui's own executor may touch a `Context`. The bridge thread exits when
/// either the session reports `TermEvent::Exit` or the channel closes (the
/// gpui `Task` -- and therefore its receiver -- was dropped because the pane
/// was closed); see `EVENT_POLL_TIMEOUT`'s doc comment for why the latter is
/// only checked periodically rather than instantly.
fn spawn_redraw_bridge(
    session: Arc<Terminal>,
    pane_id: PaneId,
    cx: &mut Context<TerminalApp>,
) -> Task<()> {
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
        // Drain everything queued right now; report whether an Exit was in
        // the batch (Exit is terminal, so nothing follows it anyway).
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
                let _ = this.update(cx, |app, cx| app.handle_pane_exit(pane_id, cx));
                break;
            }

            if this.update(cx, |_, cx| cx.notify()).is_err() {
                break;
            }

            Timer::after(FRAME_INTERVAL).await;
            if drain(&mut notify_rx) {
                let _ = this.update(cx, |app, cx| app.handle_pane_exit(pane_id, cx));
                break;
            }
        }
    })
}
