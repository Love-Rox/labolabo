//! `labolabo-app`: a gpui binary for LaboLabo's Rust cross-platform port.
//! Wave 5b-3 layers the Task model (`plans/012-task-model-and-control-
//! cli.md` §1) over wave 5b-2's tile/tab tree: a left sidebar lists Tasks
//! grouped by repo, each Task owns its own `PaneTilingModel` (split panes +
//! tab groups, each tab a real `labolabo-term` `Terminal` session spawned
//! in the Task's working directory), and Tasks + layouts persist to a
//! Rust-only SQLite database (restored on relaunch). Not the production
//! UI: see `crates/labolabo-app/README.md` for scope and known TODOs (IME,
//! Linux gpui build support, drag & drop, Task rename/done/archive).
//!
//! The control CLI (`docs/control-protocol.md`, `plans/012-task-model-and-
//! control-cli.md` §2) is implemented: `crate::control` wires
//! `labolabo_core::control::ControlServer` into this window (see that
//! module and `app.rs`'s `LaboLaboApp::dispatch_control`), and the
//! `labolabo` CLI bin (`src/bin/labolabo.rs`) is the client.

mod app;
mod control;
mod focus;
mod ghostty_config;
mod git_pane;
mod grid;
mod hooks;
mod ime;
mod keys;
mod new_task;
mod paste;
mod render;
mod selection;
mod sidebar;
mod task_workspace;

use gpui::{
    prelude::*, px, size, App, Application, Bounds, KeyBinding, WindowBounds, WindowOptions,
};

use app::{
    CloseTab, Copy, FocusNextPane, FocusPrevPane, LaboLaboApp, NewTab, Paste, SelectTab1,
    SelectTab2, SelectTab3, SelectTab4, SelectTab5, SelectTab6, SelectTab7, SelectTab8, SelectTab9,
    SplitDown, SplitRight, ToggleGitPane,
};

/// Initial window size -- purely a starting point. The initial terminal
/// grid size is derived from this via the same `grid::grid_size_for_window`
/// function window-resize uses (see `LaboLaboApp::viewport_grid_size`), so
/// there is no separately-hardcoded initial column/row count to keep in
/// sync with it. Wider than wave 5b-2's 900 to leave room for the sidebar.
const INITIAL_WIDTH: f32 = 1120.0;
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
            KeyBinding::new("cmd-v", Paste, None),
            KeyBinding::new("cmd-c", Copy, None),
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
            // Git pane (`crate::git_pane`) visibility toggle -- the task
            // brief's own suggested binding ("Cmd+Shift+G 等").
            KeyBinding::new("cmd-shift-g", ToggleGitPane, None),
        ]);

        let bounds = Bounds::centered(None, size(px(INITIAL_WIDTH), px(INITIAL_HEIGHT)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| LaboLaboApp::new(&font_config, &color_config, window, cx)),
        )
        .expect("failed to open labolabo-app window");
        cx.activate(true);
    });
}
