//! The gpui root view: window state, the (deliberately minimal) tab model,
//! key/click routing, and the render tree (tab bar + active terminal
//! canvas).
//!
//! ## Tab model (temporary)
//!
//! TODO(W5b): `Tab`/`tabs: Vec<Tab>` here is a placeholder, not the app's
//! real session/layout model. `plans/012-task-model-and-control-cli.md`
//! describes a real task/tile model (ported to `labolabo-core::tiling` from
//! the Swift `PaneTilingModel`) that a later wave will wire this window up
//! to instead. Do not build further UI on top of this `Tab` type expecting
//! it to survive that replacement.
//!
//! ## Session lifecycle
//!
//! A tab closes two ways, both funneling through [`TerminalApp::remove_tab`]
//! (removal is by tab **id**, never by index -- indices shift as tabs come
//! and go):
//!
//! - **The shell exits** (`exit`, or the child dying): the session's
//!   redraw bridge sees [`TermEvent::Exit`] and calls
//!   [`TerminalApp::handle_session_exit`], which removes the tab. The child
//!   is already dead; no teardown needed.
//! - **The user clicks the tab's "x"**: [`TerminalApp::close_tab`] calls
//!   [`labolabo_term::TermSession::shutdown`] (SIGHUP to the child -- what a
//!   real terminal sends on window close) and removes the tab. The session's
//!   PTY/worker threads then wind down via the normal exit path.
//!
//! When the last tab closes (either way), the app quits (`cx.quit()`),
//! matching Ghostty's close-last-surface behavior.

use std::sync::Arc;
use std::thread;
use std::time::Duration;

use futures::channel::mpsc;
use futures::StreamExt;
use gpui::{
    canvas, div, prelude::*, px, rgb, ClickEvent, Context, FocusHandle, IntoElement, KeyDownEvent,
    Render, SharedString, Task, Timer, Window,
};

use labolabo_term::{TermEvent, Terminal};

use crate::ghostty_config::FontConfig;
use crate::grid::{grid_size_for_window, TAB_BAR_HEIGHT};
use crate::keys::keystroke_to_bytes;
use crate::render::{paint_grid, RenderSpec};

/// How long the redraw-bridge thread blocks on `recv_event` between checks
/// of whether its gpui-side `Task` was dropped (tab closed). Real events
/// (`TermEvent::Wakeup`/`Exit`) are delivered the instant they happen,
/// regardless of this value -- it only bounds how quickly a *closed* tab's
/// bridge thread notices there's no one left to notify and exits. This is
/// not a redraw poll: `Render::render` only ever re-runs from an actual
/// `cx.notify()` call, which only ever happens in response to a real
/// `TermEvent`.
const EVENT_POLL_TIMEOUT: Duration = Duration::from_millis(250);

/// Minimum gap between two `cx.notify()` calls for the same tab, mirroring
/// `labolabo_term::session`'s own ~60fps snapshot-construction throttle so
/// this UI layer never asks gpui to redraw faster than the terminal core
/// itself paces snapshots.
const FRAME_INTERVAL: Duration = Duration::from_millis(16);

/// What the per-tab bridge thread forwards to its gpui-side task.
enum BridgeMsg {
    /// A new snapshot is available -- repaint (coalesced + frame-paced).
    Wakeup,
    /// The session's child exited -- close the tab. Terminal: the bridge
    /// thread stops after sending this.
    Exit,
}

struct Tab {
    id: u64,
    title: SharedString,
    session: Arc<Terminal>,
    cols: u16,
    rows: u16,
    /// Keeps the redraw-bridge task alive for the tab's lifetime; dropping
    /// it (on tab close) is the signal the bridge thread uses to stop.
    _redraw_task: Task<()>,
}

pub struct TerminalApp {
    tabs: Vec<Tab>,
    active: usize,
    focus_handle: FocusHandle,
    next_id: u64,
    spec: RenderSpec,
}

impl TerminalApp {
    pub fn new(font_config: &FontConfig, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let spec = RenderSpec::resolve(
            &font_config.families,
            font_config
                .size
                .unwrap_or_else(crate::ghostty_config::default_font_size),
            window,
        );

        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle);

        let mut this = Self {
            tabs: Vec::new(),
            active: 0,
            focus_handle,
            next_id: 0,
            spec,
        };
        this.open_tab(window, cx);

        cx.observe_window_bounds(window, |this, window, cx| {
            this.handle_window_resized(window, cx);
        })
        .detach();

        this
    }

    /// The terminal grid size for the window's current viewport (full
    /// window minus the tab bar strip) at the measured cell size. Shared by
    /// initial-tab spawn and window-resize handling so there is exactly one
    /// place that computes "how big is a tab's grid right now".
    fn viewport_grid_size(&self, window: &Window) -> (u16, u16) {
        let size = window.viewport_size();
        grid_size_for_window(
            size.width.into(),
            size.height.into(),
            self.spec.cell_width,
            self.spec.cell_height,
        )
    }

    /// Spawn a new tab (a fresh login-shell `TermSession`) sized to the
    /// window's current viewport, and make it the active tab.
    fn open_tab(&mut self, window: &Window, cx: &mut Context<Self>) {
        let (cols, rows) = self.viewport_grid_size(window);
        let id = self.next_id;
        self.next_id += 1;

        let session = match Terminal::spawn(cols, rows) {
            Ok(session) => Arc::new(session),
            Err(err) => {
                // TODO(W5a): surface spawn failures in the UI (e.g. a
                // placeholder tab showing the error) instead of only
                // logging to stderr.
                eprintln!("labolabo-app: failed to spawn terminal session: {err:#}");
                return;
            }
        };

        let redraw_task = spawn_redraw_bridge(session.clone(), id, cx);

        self.tabs.push(Tab {
            id,
            title: format!("shell {}", id + 1).into(),
            session,
            cols,
            rows,
            _redraw_task: redraw_task,
        });
        self.active = self.tabs.len() - 1;
        cx.notify();
    }

    /// Remove the tab with `id` (no-op for an unknown id -- e.g. an `Exit`
    /// arriving for a tab the user already closed). Returns whether a tab
    /// was actually removed. Fixes up `active` and quits the app when the
    /// last tab is gone (Ghostty's close-last-surface behavior).
    fn remove_tab(&mut self, id: u64, cx: &mut Context<Self>) -> bool {
        let Some(index) = self.tabs.iter().position(|tab| tab.id == id) else {
            return false;
        };
        self.tabs.remove(index);
        if self.active > index || self.active >= self.tabs.len() {
            self.active = self.active.saturating_sub(1);
        }
        if self.tabs.is_empty() {
            cx.quit();
        } else {
            cx.notify();
        }
        true
    }

    /// User clicked a tab's close button: terminate the session's child
    /// (see `TermSession::shutdown`) and drop the tab.
    fn close_tab(&mut self, id: u64, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.iter().find(|tab| tab.id == id) {
            tab.session.shutdown();
        }
        self.remove_tab(id, cx);
    }

    /// The session's child exited on its own (`TermEvent::Exit`): drop the
    /// tab. The child is already dead, so no shutdown call is needed.
    fn handle_session_exit(&mut self, id: u64, cx: &mut Context<Self>) {
        self.remove_tab(id, cx);
    }

    fn select_tab(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(index) = self.tabs.iter().position(|tab| tab.id == id) {
            if index != self.active {
                self.active = index;
                window.focus(&self.focus_handle);
                cx.notify();
            }
        }
    }

    /// Recompute the grid size for the (new) window viewport and, for any
    /// tab whose size actually changed, resize its `TermSession` (which in
    /// turn resizes the real PTY, so full-screen programs see `SIGWINCH`).
    fn handle_window_resized(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (cols, rows) = self.viewport_grid_size(window);
        for tab in &mut self.tabs {
            if tab.cols != cols || tab.rows != rows {
                tab.cols = cols;
                tab.rows = rows;
                tab.session.resize(cols, rows);
            }
        }
        cx.notify();
    }

    fn key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, _cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        // TODO(W5a): IME composition is not wired up here -- see
        // `keys::keystroke_to_bytes`'s module doc comment.
        if let Some(bytes) = keystroke_to_bytes(&event.keystroke) {
            tab.session.write_input(&bytes);
        }
    }

    fn render_tab_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_row()
            .items_center()
            .h(px(TAB_BAR_HEIGHT))
            .w_full()
            .bg(rgb(0x2a2a2a))
            .px_1()
            .gap_1()
            .children(self.tabs.iter().enumerate().map(|(index, tab)| {
                let selected = index == self.active;
                let tab_id = tab.id;
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
                            .id(("tab-select", tab_id))
                            .px_1()
                            .text_color(rgb(0xe5e5e5))
                            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                this.select_tab(tab_id, window, cx);
                            }))
                            .child(tab.title.clone()),
                    )
                    .child(
                        div()
                            .id(("tab-close", tab_id))
                            .px_1()
                            .text_color(rgb(0x999999))
                            .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                                this.close_tab(tab_id, cx);
                            }))
                            .child("\u{d7}"),
                    )
            }))
            .child(
                div()
                    .id("tab-new")
                    .px_2()
                    .text_color(rgb(0xe5e5e5))
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.open_tab(window, cx);
                    }))
                    .child("+"),
            )
    }
}

impl Render for TerminalApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tab_bar = self.render_tab_bar(cx);
        let active_snapshot = self.tabs.get(self.active).map(|tab| tab.session.snapshot());
        let spec = self.spec.clone();

        div()
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::key_down))
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(0x000000))
            .child(tab_bar)
            .child(
                div().flex_1().w_full().child(
                    canvas(
                        move |_bounds, _window, _cx| (),
                        move |bounds, _, window, cx| {
                            if let Some(snapshot) = &active_snapshot {
                                paint_grid(snapshot, &spec, bounds, window, cx);
                            }
                        },
                    )
                    .size_full(),
                ),
            )
    }
}

/// Bridges `labolabo_term`'s blocking [`TermSession::recv_event`] into
/// gpui, with the same two-stage coalesce-then-pace design as the
/// `gpui-term-poc` spike's `spawn_redraw_task` (labolabo-spikes
/// `src/main.rs`): drain a burst of already-queued wakeups into one redraw,
/// then enforce `FRAME_INTERVAL` as a minimum gap before the next one,
/// draining anything that arrived during that quiet window too. An `Exit`
/// seen at any point (awaited or drained) closes the tab via
/// [`TerminalApp::handle_session_exit`] and ends the task.
///
/// `TermSession` exposes no async event stream (`recv_event` blocks the
/// calling thread -- see its doc comment), so a dedicated OS thread does the
/// blocking wait and forwards [`BridgeMsg`]s to the gpui-side async `Task`
/// over an unbounded channel; the `Task` is the one that actually calls
/// `cx.notify()`, since only gpui's own executor may touch a `Context`. The
/// bridge thread exits when either the session reports `TermEvent::Exit` or
/// the channel closes (the gpui `Task` -- and therefore its receiver -- was
/// dropped because the tab was closed); see `EVENT_POLL_TIMEOUT`'s doc
/// comment for why the latter is only checked periodically rather than
/// instantly.
fn spawn_redraw_bridge(
    session: Arc<Terminal>,
    tab_id: u64,
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
                let _ = this.update(cx, |app, cx| app.handle_session_exit(tab_id, cx));
                break;
            }

            if this.update(cx, |_, cx| cx.notify()).is_err() {
                break;
            }

            Timer::after(FRAME_INTERVAL).await;
            if drain(&mut notify_rx) {
                let _ = this.update(cx, |app, cx| app.handle_session_exit(tab_id, cx));
                break;
            }
        }
    })
}
