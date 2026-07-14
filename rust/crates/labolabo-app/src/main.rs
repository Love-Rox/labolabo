//! `labolabo-app`: a gpui terminal-shell binary for LaboLabo's Rust
//! cross-platform port -- wave 5a's bootable skeleton (one window, a real
//! `labolabo-term` `TermSession` per tab, a minimal tab bar, event-driven
//! redraw). Not the production UI: see `crates/labolabo-app/README.md` for
//! scope and known TODOs (IME, the tab model's planned replacement, Linux
//! gpui build support).

mod app;
mod grid;
mod keys;
mod render;

use gpui::{prelude::*, px, size, App, Application, Bounds, WindowBounds, WindowOptions};

use app::TerminalApp;

/// Initial window size -- purely a starting point. The initial terminal
/// grid size is derived from this via the same `grid::grid_size_for_window`
/// function window-resize uses (see `TerminalApp::viewport_grid_size`), so
/// there is no separately-hardcoded initial column/row count to keep in
/// sync with it.
const INITIAL_WIDTH: f32 = 900.0;
const INITIAL_HEIGHT: f32 = 600.0;

fn main() {
    Application::new().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(INITIAL_WIDTH), px(INITIAL_HEIGHT)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| TerminalApp::new(window, cx)),
        )
        .expect("failed to open labolabo-app window");
        cx.activate(true);
    });
}
