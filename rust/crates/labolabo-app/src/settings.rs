//! The Cmd+, settings screen (`plans` wave 5i §3) -- LaboLabo's Rust port
//! is a single-window gpui app with no native macOS `Settings` scene (that's
//! a SwiftUI-only concept), so this is an in-window overlay toggled by
//! [`crate::app::ToggleSettings`], not a separate OS window.
//!
//! ## What's here vs. what isn't
//!
//! Per the wave brief, only the settings that (a) exist in the Swift app
//! and (b) still mean something in this port's architecture:
//!
//! 1. **Claude セッションの自動 resume** (`AppSettings::auto_resume_enabled`)
//!    -- mirrors Swift's `autoResumeAgentOnRestore` `@AppStorage` toggle.
//!    Previously always-on in this port (`spawn_runtime_for_task`'s resume-
//!    at-spawn logic had no gate); this setting makes that a user choice.
//! 2. **Git ペインの既定表示** (`AppSettings::git_pane_default_visible`) --
//!    seeds `GitPaneState::visible` for every *newly loaded* Task workspace
//!    (`TaskWorkspace::new`). Swift has no direct analogue (its WorkPane
//!    tiles are part of the persisted per-session `TileLayout`), but this
//!    port's fixed (non-tile) Git pane needs *some* default, and exposing
//!    it is more useful than a silent hardcoded `true`.
//! 3. **スクロールバック行数** (`AppSettings::scrollback_lines`) -- how many
//!    lines of history `labolabo_term::Terminal` retains past the live
//!    viewport (`labolabo_term::DEFAULT_MAX_SCROLLBACK` = `1000` today,
//!    hardcoded in both VT backends before this wave). Swift has no
//!    analogue at all (libghostty's own scrollback is configured through
//!    Ghostty's own config file, not the Swift app) -- this is a genuinely
//!    Rust-port-only knob, included because the task brief calls it out as
//!    "Rust 版に意味があるもの". Takes effect at the next pane spawn, not
//!    retroactively (a live VT core's history buffer isn't resizable) --
//!    the settings panel's footer says so explicitly.
//!
//! **Deliberately not here**: font/color settings -- the project's own
//! policy (`CLAUDE.md` for this wave) is that Ghostty config remains the
//! single source of truth for those (`crate::ghostty_config`); duplicating
//! them into this app's own settings would create two competing sources of
//! truth for the same rendering knobs.
//!
//! ## Persistence
//!
//! All three settings round-trip through `TaskDatabase`'s `appState` table
//! (`TaskDatabase::auto_resume_enabled`/`set_auto_resume_enabled`, etc. --
//! see that module's doc comment) -- the same key/value store the selected-
//! Task pointer already uses, per the wave brief's "保存は既存の appState
//! テーブルへ". Every toggle/adjustment in this module writes through
//! immediately (no separate "Save" step), matching this codebase's general
//! "persist on every mutating action" idiom (`LaboLaboApp::persist_workspace`
//! is the same shape for Task layouts).

use gpui::{
    div, prelude::*, px, rgb, rgba, AnyElement, Context, IntoElement, MouseButton, MouseDownEvent,
};

use labolabo_core::TaskDatabase;

use crate::app::LaboLaboApp;
use crate::theme;

/// Default scrollback-line-count -- re-exported from `labolabo_term` (the
/// crate that actually owns what "scrollback" means) rather than redefined
/// here, so this module's default can never drift from the VT backends'
/// own (see `labolabo_term::session::DEFAULT_MAX_SCROLLBACK`'s doc comment).
pub const DEFAULT_SCROLLBACK_LINES: usize = labolabo_term::DEFAULT_MAX_SCROLLBACK;
/// Lower bound the -/+ steppers (and any future direct-entry UI) clamp to
/// -- small enough to be obviously "not useful" without risking a
/// zero/negative history buffer no VT backend is designed to handle.
pub const MIN_SCROLLBACK_LINES: usize = 100;
/// Upper bound -- generous, but not unbounded (an unreasonably large value
/// would grow every pane's memory footprint for no real benefit; this is a
/// terminal history buffer, not a log archive).
pub const MAX_SCROLLBACK_LINES: usize = 20_000;
/// The -/+ steppers' fixed increment.
pub const SCROLLBACK_STEP: usize = 500;

/// The Rust port's minimal, in-window settings -- see this module's doc
/// comment for what each field means and why it exists. Loaded once at
/// startup (`Self::load`) and kept in sync with `TaskDatabase`'s `appState`
/// table by every setter in `crate::app::LaboLaboApp` that mutates it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppSettings {
    pub auto_resume_enabled: bool,
    pub git_pane_default_visible: bool,
    pub scrollback_lines: usize,
}

impl Default for AppSettings {
    /// Matches this port's pre-settings-screen behavior exactly: auto-
    /// resume was always on, the Git pane always defaulted to visible
    /// (`GitPaneState::default()`), and scrollback was always
    /// `DEFAULT_SCROLLBACK_LINES`. A brand-new database (every key absent)
    /// therefore behaves identically to before this wave.
    fn default() -> Self {
        Self {
            auto_resume_enabled: true,
            git_pane_default_visible: true,
            scrollback_lines: DEFAULT_SCROLLBACK_LINES,
        }
    }
}

impl AppSettings {
    /// Reads every setting from `db`, falling back to [`Self::default`]'s
    /// field for whichever key is absent (fresh database) or, for
    /// `scrollback_lines`, unparseable (`TaskDatabase::scrollback_lines`'s
    /// own "degrades gracefully" contract). Read errors (I/O failure on an
    /// already-open connection -- shouldn't happen in practice) degrade the
    /// same way as "absent", rather than panicking `LaboLaboApp::new`.
    pub fn load(db: &TaskDatabase) -> Self {
        let defaults = Self::default();
        Self {
            auto_resume_enabled: db
                .auto_resume_enabled()
                .ok()
                .flatten()
                .unwrap_or(defaults.auto_resume_enabled),
            git_pane_default_visible: db
                .git_pane_default_visible()
                .ok()
                .flatten()
                .unwrap_or(defaults.git_pane_default_visible),
            scrollback_lines: db
                .scrollback_lines()
                .ok()
                .flatten()
                .unwrap_or(defaults.scrollback_lines),
        }
    }
}

/// Applies a `delta` (positive or negative, typically ±[`SCROLLBACK_STEP`])
/// to `current`, clamped to `[MIN_SCROLLBACK_LINES, MAX_SCROLLBACK_LINES]`.
/// Pure and gpui-free so the stepper math is unit-testable without a
/// `TaskDatabase`/`LaboLaboApp` in the loop.
pub fn adjust_scrollback_lines(current: usize, delta: i64) -> usize {
    let next = current as i64 + delta;
    next.clamp(MIN_SCROLLBACK_LINES as i64, MAX_SCROLLBACK_LINES as i64) as usize
}

const OVERLAY_BG: u32 = theme::with_alpha(0x000000, 0xb3); // ~70% black backdrop
const PANEL_BG: u32 = theme::surface::ROOT;
const BORDER_COLOR: u32 = theme::surface::STROKE;
const BUTTON_BG: u32 = theme::surface::RAISED;
const TEXT_PRIMARY: u32 = theme::text::PRIMARY;
const TEXT_SECONDARY: u32 = theme::text::SECONDARY;

/// Renders the settings overlay (backdrop + centered panel) when
/// `app.settings_open()` -- callers append this as the last child of the
/// root render tree so it paints on top of everything else (`app.rs`'s
/// `Render for LaboLaboApp`). Returns `None` when the panel isn't open, so
/// the caller can `.children(..)` it in without an extra `if` at the call
/// site.
///
/// No click-outside-to-close: this port has no established modal/backdrop
/// convention yet (nothing else in the app renders an overlay), and gpui's
/// event-bubbling semantics for "did this click land on the backdrop vs. a
/// child" aren't exercised anywhere else in this codebase to copy from --
/// safer to ship an explicit "閉じる" button (and `Cmd+,` toggles the panel
/// closed again) than to guess at bubbling behavior and risk clicks inside
/// the panel closing it.
pub fn render_settings_overlay(
    app: &LaboLaboApp,
    cx: &mut Context<LaboLaboApp>,
) -> Option<AnyElement> {
    if !app.settings_open() {
        return None;
    }
    let settings = *app.settings();

    let panel = div()
        .flex()
        .flex_col()
        .gap_3()
        .w(px(420.0))
        .p_4()
        .rounded_md()
        .bg(rgb(PANEL_BG))
        .border_1()
        .border_color(rgb(BORDER_COLOR))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_size(px(15.0))
                        .text_color(rgb(TEXT_PRIMARY))
                        .child("設定"),
                )
                .child(close_button(cx)),
        )
        .child(
            toggle_row(
                "auto-resume",
                "Claude セッションの自動 resume",
                "復元したタスクを開いたとき、前回のセッションを自動的に再開します。",
                settings.auto_resume_enabled,
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _window, cx| {
                    this.set_auto_resume_enabled(!this.settings().auto_resume_enabled, cx);
                }),
            ),
        )
        .child(
            toggle_row(
                "git-pane-default-visible",
                "Git ペインを既定で表示",
                "新しく開くタスクの Git ペイン表示/非表示の既定値です（開いているタスクには影響しません）。",
                settings.git_pane_default_visible,
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _window, cx| {
                    this.set_git_pane_default_visible(!this.settings().git_pane_default_visible, cx);
                }),
            ),
        )
        .child(scrollback_row(settings.scrollback_lines, cx));

    Some(
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgba(OVERLAY_BG))
            .child(panel)
            .into_any_element(),
    )
}

fn close_button(cx: &mut Context<LaboLaboApp>) -> impl IntoElement {
    div()
        .id("settings-close")
        .px_2()
        .py_1()
        .rounded_sm()
        .bg(rgb(BUTTON_BG))
        .text_color(rgb(TEXT_PRIMARY))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _: &MouseDownEvent, _window, cx| {
                this.close_settings(cx);
            }),
        )
        .child("\u{d7}")
}

/// One checkbox-style setting row: a `[x]`/`[ ]` glyph + label, footer copy
/// below. Returns a concrete `Stateful<Div>` (not `impl IntoElement`) so the
/// caller can chain `.on_mouse_down(..)` onto it directly, same shape as
/// `sidebar::render`'s per-Task row (`.id(row_id)...on_mouse_down(..)`) --
/// clickable anywhere on the row (the whole row is the hit target, not just
/// the glyph -- larger, easier-to-hit affordance for a "minimal" settings
/// UI with no real checkbox widget available). `key` only needs to be
/// unique among this panel's rows (used as the element id so gpui can track
/// this row's identity across re-renders).
fn toggle_row(
    key: &'static str,
    label: &'static str,
    footer: &'static str,
    enabled: bool,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(key)
        .flex()
        .flex_col()
        .gap_1()
        .p_2()
        .rounded_sm()
        .bg(rgb(BUTTON_BG))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .text_color(rgb(TEXT_PRIMARY))
                .child(if enabled { "\u{2611}" } else { "\u{2610}" })
                .child(label),
        )
        .child(
            div()
                .text_size(px(theme::font_size::CAPTION))
                .text_color(rgb(TEXT_SECONDARY))
                .child(footer),
        )
}

/// The scrollback-lines row: current value + -/+ steppers.
fn scrollback_row(current: usize, cx: &mut Context<LaboLaboApp>) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .p_2()
        .rounded_sm()
        .bg(rgb(BUTTON_BG))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .text_color(rgb(TEXT_PRIMARY))
                .child(div().child("スクロールバック行数"))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(stepper_button("\u{2212}", -(SCROLLBACK_STEP as i64), cx))
                        .child(div().w(px(56.0)).text_center().child(current.to_string()))
                        .child(stepper_button("+", SCROLLBACK_STEP as i64, cx)),
                ),
        )
        .child(
            div()
                .text_size(px(theme::font_size::CAPTION))
                .text_color(rgb(TEXT_SECONDARY))
                .child("変更は次に開くタブから反映されます（既存のタブには影響しません）。"),
        )
}

fn stepper_button(
    glyph: &'static str,
    delta: i64,
    cx: &mut Context<LaboLaboApp>,
) -> impl IntoElement {
    div()
        .id(glyph)
        .px_2()
        .py_1()
        .rounded_sm()
        .bg(rgb(theme::surface::ACTIVE))
        .text_color(rgb(TEXT_PRIMARY))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _: &MouseDownEvent, _window, cx| {
                this.adjust_scrollback_lines(delta, cx);
            }),
        )
        .child(glyph)
}

#[cfg(test)]
mod tests {
    use super::*;

    // MARK: - AppSettings::default / load

    #[test]
    fn default_matches_pre_settings_screen_behavior() {
        let defaults = AppSettings::default();
        assert!(defaults.auto_resume_enabled);
        assert!(defaults.git_pane_default_visible);
        assert_eq!(defaults.scrollback_lines, DEFAULT_SCROLLBACK_LINES);
    }

    #[test]
    fn load_from_a_fresh_database_yields_defaults() {
        let db = TaskDatabase::open_in_memory().unwrap();
        assert_eq!(AppSettings::load(&db), AppSettings::default());
    }

    #[test]
    fn load_reflects_persisted_overrides() {
        let db = TaskDatabase::open_in_memory().unwrap();
        db.set_auto_resume_enabled(false).unwrap();
        db.set_git_pane_default_visible(false).unwrap();
        db.set_scrollback_lines(5000).unwrap();

        let loaded = AppSettings::load(&db);
        assert_eq!(
            loaded,
            AppSettings {
                auto_resume_enabled: false,
                git_pane_default_visible: false,
                scrollback_lines: 5000,
            }
        );
    }

    #[test]
    fn load_falls_back_per_field_not_all_or_nothing() {
        // Only one of the three keys set -- the other two must still fall
        // back to their own defaults independently, not to some "any key
        // present -> skip all defaults" shortcut.
        let db = TaskDatabase::open_in_memory().unwrap();
        db.set_scrollback_lines(42).unwrap();
        let loaded = AppSettings::load(&db);
        assert!(loaded.auto_resume_enabled);
        assert!(loaded.git_pane_default_visible);
        assert_eq!(loaded.scrollback_lines, 42);
    }

    // MARK: - adjust_scrollback_lines

    #[test]
    fn adjust_scrollback_lines_steps_up_and_down() {
        assert_eq!(adjust_scrollback_lines(1000, 500), 1500);
        assert_eq!(adjust_scrollback_lines(1000, -500), 500);
    }

    #[test]
    fn adjust_scrollback_lines_clamps_at_the_floor() {
        assert_eq!(
            adjust_scrollback_lines(MIN_SCROLLBACK_LINES, -(SCROLLBACK_STEP as i64)),
            MIN_SCROLLBACK_LINES
        );
        assert_eq!(adjust_scrollback_lines(0, -1_000_000), MIN_SCROLLBACK_LINES);
    }

    #[test]
    fn adjust_scrollback_lines_clamps_at_the_ceiling() {
        assert_eq!(
            adjust_scrollback_lines(MAX_SCROLLBACK_LINES, SCROLLBACK_STEP as i64),
            MAX_SCROLLBACK_LINES
        );
        assert_eq!(
            adjust_scrollback_lines(MAX_SCROLLBACK_LINES, 1_000_000),
            MAX_SCROLLBACK_LINES
        );
    }
}
