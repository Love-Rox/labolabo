//! `labolabo-app`: a gpui binary for LaboLabo's Rust cross-platform port.
//! Wave 5b-3 layers the Task model (`plans/012-task-model-and-control-
//! cli.md` §1) over wave 5b-2's tile/tab tree: a left sidebar lists Tasks
//! grouped by repo, each Task owns its own `PaneTilingModel` (split panes +
//! tab groups, each tab a real `labolabo-term` `Terminal` session spawned
//! in the Task's working directory), and Tasks + layouts persist to a
//! Rust-only SQLite database (restored on relaunch). Not the production
//! UI: see `crates/labolabo-app/README.md` for scope and known TODOs
//! (e.g. Linux builds/tests run in CI since wave 7a, but a real Linux
//! desktop launch is still unverified -- that README's "Linux" section).
//!
//! The control CLI (`docs/control-protocol.md`, `plans/012-task-model-and-
//! control-cli.md` §2) is implemented: `crate::control` wires
//! `labolabo_core::control::ControlServer` into this window (see that
//! module and `app.rs`'s `LaboLaboApp::dispatch_control`), and the
//! `labolabo` CLI bin (`src/bin/labolabo.rs`) is the client.
//!
//! Wave 6c adds the menu bar (`crate::menus`), Task archive/delete
//! (`crate::task_menu`/`crate::task_lifecycle`), and window-bounds
//! persistence (`crate::window_bounds`) -- see those modules' doc comments.

mod app;
mod commit_pane;
mod control;
mod focus;
mod ghostty_config;
mod git_pane;
mod grid;
mod hooks;
mod i18n;
mod ide_open;
mod ime;
mod import_prompt;
mod keys;
mod menus;
mod missing_dir;
mod motion;
mod mouse_report;
mod new_task;
mod paste;
mod render;
mod selection;
mod settings;
mod sidebar;
mod swift_import;
mod task_lifecycle;
mod task_menu;
mod task_workspace;
mod theme;
mod update_check;
mod window_bounds;

// i18n wave (6f, `crate::i18n`): loads `locales/{ja,en}.yml`, compiled in at
// build time, and defines the `t!()` macro every other module imports
// (`use rust_i18n::t;`) to look strings up. `fallback = "en"` means a key
// present in only one locale file still renders (in English) rather than
// showing the raw key -- the quality gate that actually blocks a missing
// translation is the `locales_have_the_same_keys` test in `i18n.rs`'s
// sibling test module (`tests/i18n_parity.rs`), not this fallback.
rust_i18n::i18n!("locales", fallback = "en");

use gpui::{
    prelude::*, px, size, App, Application, Bounds, KeyBinding, Pixels, WindowBounds, WindowOptions,
};

use labolabo_core::TaskDatabase;

use app::{
    CloseTab, Copy, FocusNextPane, FocusPrevPane, LaboLaboApp, MinimizeWindow, NewTab, Paste, Quit,
    SelectTab1, SelectTab2, SelectTab3, SelectTab4, SelectTab5, SelectTab6, SelectTab7, SelectTab8,
    SelectTab9, SplitDown, SplitRight, ToggleGitPane, ToggleSettings,
};

/// Initial window size -- purely a starting point (used when there are no
/// persisted window bounds yet, or the persisted ones no longer intersect
/// any connected display -- see `crate::window_bounds`). The initial
/// terminal grid size is derived from this via the same
/// `grid::grid_size_for_window` function window-resize uses (see
/// `LaboLaboApp::viewport_grid_size`), so there is no separately-hardcoded
/// initial column/row count to keep in sync with it. Wider than wave
/// 5b-2's 900 to leave room for the sidebar.
const INITIAL_WIDTH: f32 = 1120.0;
const INITIAL_HEIGHT: f32 = 600.0;

fn main() {
    // One-time data-directory migration (1.1.0 rename): move a pre-rename
    // `LaboLabo-rs/tasks.db` into the new `LaboLabo/` directory when the
    // new side has no database yet. Must run before the first
    // `TaskDatabase::open(&TaskDatabase::default_path())` below -- opening
    // first would create an empty database at the new path, which would
    // then win over (and permanently orphan) the user's real legacy data.
    // A no-op on fresh installs, on already-migrated machines, and when
    // `LABOLABO_RS_DATA_DIR` is set -- see the function's doc comment.
    labolabo_core::store::migrate_legacy_rust_data_dir();

    // Read the user's Ghostty config (font-family/font-size, plus
    // background/foreground/cursor-color/palette/theme) once, up front --
    // pure file I/O, no gpui needed. Missing config just means Ghostty-
    // default font settings and each backend's own built-in colors.
    let font_config = ghostty_config::load_user_font_config();
    let color_config = ghostty_config::load_user_color_config();

    // Saved window bounds (wave 6c §3) -- read up front like the Ghostty
    // config (pure file I/O; a second, short-lived SQLite connection to the
    // same database `LaboLaboApp::new` opens later, which is fine for
    // SQLite). Any failure (no database yet, key absent, undecodable JSON)
    // just means the centered default below.
    let saved_bounds = TaskDatabase::open(&TaskDatabase::default_path())
        .ok()
        .and_then(|db| db.window_bounds().ok().flatten())
        .and_then(|json| window_bounds::decode(&json));

    // UI language (wave 6f, `crate::i18n`) -- same "read up front via a
    // short-lived connection" shape as `saved_bounds` above. Absent/corrupt
    // just means `LocaleSetting::Auto` (OS-detected), matching every other
    // `AppSettings` field's "missing key degrades to the pre-settings-screen
    // default" contract (`settings::AppSettings::load`'s doc comment) --
    // here, "OS locale" *is* the pre-i18n-wave behavior (everything was
    // hardcoded Japanese, so `ja` is the closer match for a `ja*` system,
    // `en` otherwise).
    let locale_setting = TaskDatabase::open(&TaskDatabase::default_path())
        .ok()
        .map(|db| i18n::load_locale_setting(&db))
        .unwrap_or_default();
    rust_i18n::set_locale(locale_setting.resolve());

    Application::new().run(move |cx: &mut App| {
        // Tile/tab keybindings (see `app.rs`'s `actions!` list for the
        // handlers; README.md documents this table for users). Cmd-modified
        // keystrokes never reach a terminal's own input (`keys::
        // keystroke_to_bytes` reserves the whole `platform` modifier for
        // application shortcuts), so there's no conflict with typing into a
        // pane. Menu items (`crate::menus`) reference these same actions,
        // and gpui renders each menu item's shortcut from this keymap --
        // so `bind_keys` must run before `set_menus` below.
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
            // Settings overlay (`crate::settings`) -- matches the Swift
            // app's `Cmd+,` (macOS's conventional "Preferences" shortcut).
            KeyBinding::new("cmd-,", ToggleSettings, None),
            // Menu-bar standards (wave 6c §1): quit and minimize.
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("cmd-m", MinimizeWindow, None),
        ]);

        // Quit is a *global* action handler (not a window-scoped
        // `.on_action` in `LaboLaboApp::render`) so the menu item works
        // even with no window focused. `cx.quit()` runs the app's
        // `on_app_quit` cleanup (hooks `settings.local.json` restore --
        // see `LaboLaboApp::new`).
        cx.on_action(|_: &Quit, cx| cx.quit());

        // Menu bar (wave 6c §1) -- after `bind_keys` (see above), and after
        // `rust_i18n::set_locale` above so its labels are already in the
        // right language on first paint (`menus::app_menus` takes the
        // locale explicitly -- see that function's doc comment for why).
        cx.set_menus(menus::app_menus(&rust_i18n::locale()));

        // Window bounds restore (wave 6c §3): saved bounds win if they
        // still intersect a connected display; otherwise (display
        // unplugged, corrupt value, first run) fall back to centered.
        // Fullscreen/maximized windows are restored as normal windows in
        // this first version (README).
        let display_bounds: Vec<Bounds<Pixels>> = cx
            .displays()
            .iter()
            .map(|display| display.bounds())
            .collect();
        let bounds = saved_bounds
            .and_then(|saved| window_bounds::restore_bounds(saved, &display_bounds))
            .unwrap_or_else(|| {
                Bounds::centered(None, size(px(INITIAL_WIDTH), px(INITIAL_HEIGHT)), cx)
            });
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
