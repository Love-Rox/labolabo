//! `labolabo-app`: a gpui terminal-shell binary for LaboLabo's Rust
//! cross-platform port. Wave 5b-2 replaces the wave 5a placeholder flat tab
//! bar with the real tile/tab tree (`labolabo_core::tiling::
//! PaneTilingModel`): split panes, each with its own tab group, keyboard
//! navigation, and a real `labolabo-term` `Terminal` session per tab. Not
//! the production UI: see `crates/labolabo-app/README.md` for scope and
//! known TODOs (IME, Linux gpui build support, layout persistence, drag &
//! drop).

mod app;
mod focus;
mod ghostty_config;
mod grid;
mod keys;
mod render;

use gpui::{
    prelude::*, px, size, App, Application, Bounds, KeyBinding, WindowBounds, WindowOptions,
};

use app::{
    CloseTab, FocusNextPane, FocusPrevPane, NewTab, SelectTab1, SelectTab2, SelectTab3, SelectTab4,
    SelectTab5, SelectTab6, SelectTab7, SelectTab8, SelectTab9, SplitDown, SplitRight, TerminalApp,
};

/// Initial window size -- purely a starting point. The initial terminal
/// grid size is derived from this via the same `grid::grid_size_for_window`
/// function window-resize uses (see `TerminalApp::viewport_grid_size`), so
/// there is no separately-hardcoded initial column/row count to keep in
/// sync with it.
const INITIAL_WIDTH: f32 = 900.0;
const INITIAL_HEIGHT: f32 = 600.0;

fn main() {
    // Read the user's Ghostty config (font-family/font-size, plus
    // background/foreground/cursor-color/palette/theme) once, up front --
    // pure file I/O, no gpui needed. Missing config just means Ghostty-
    // default font settings and each backend's own built-in colors.
    let font_config = ghostty_config::load_user_font_config();
    let color_config = ghostty_config::load_user_color_config();

    Application::new().run(move |cx: &mut App| {
        // Tile/tab keybindings (see `app.rs`'s `actions!` list for the
        // handlers; README.md documents this table for users). Cmd-modified
        // keystrokes never reach a terminal's own input (`keys::
        // keystroke_to_bytes` reserves the whole `platform` modifier for
        // application shortcuts), so there's no conflict with typing into a
        // pane.
        cx.bind_keys([
            KeyBinding::new("cmd-t", NewTab, None),
            KeyBinding::new("cmd-w", CloseTab, None),
            KeyBinding::new("cmd-d", SplitRight, None),
            KeyBinding::new("cmd-shift-d", SplitDown, None),
            KeyBinding::new("cmd-]", FocusNextPane, None),
            KeyBinding::new("cmd-[", FocusPrevPane, None),
            KeyBinding::new("cmd-1", SelectTab1, None),
            KeyBinding::new("cmd-2", SelectTab2, None),
            KeyBinding::new("cmd-3", SelectTab3, None),
            KeyBinding::new("cmd-4", SelectTab4, None),
            KeyBinding::new("cmd-5", SelectTab5, None),
            KeyBinding::new("cmd-6", SelectTab6, None),
            KeyBinding::new("cmd-7", SelectTab7, None),
            KeyBinding::new("cmd-8", SelectTab8, None),
            KeyBinding::new("cmd-9", SelectTab9, None),
        ]);

        let bounds = Bounds::centered(None, size(px(INITIAL_WIDTH), px(INITIAL_HEIGHT)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| TerminalApp::new(&font_config, &color_config, window, cx)),
        )
        .expect("failed to open labolabo-app window");
        cx.activate(true);
    });
}
