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
//! 4. **言語 / Language** (`AppSettings::locale`, wave 6f) -- 自動 (OS
//!    locale) / 日本語 / English, applied live (see `crate::i18n`'s module
//!    doc comment) and persisted like everything else here.
//! 5. **アップデートを自動確認** (`AppSettings::update_check_enabled`, RC
//!    release wave) -- mirrors Swift's `checkUpdatesOnLaunch` `@AppStorage`
//!    toggle. Gates `crate::update_check`'s once-per-launch background
//!    GitHub-releases check (`LaboLaboApp::new`); the `LABOLABO_NO_
//!    UPDATE_CHECK` env var is a separate, settings-independent kill switch
//!    (`update_check::update_check_disabled`) for smoke-testing/CI.
//! 6. **テキスト選択を優先** (`AppSettings::prefer_local_selection`, wave
//!    20) -- a mouse-tracking-aware program running in a pane (e.g. Claude
//!    Code's own TUI, which enables DECSET `1000`/`1002`/`1003`/`1006`)
//!    normally captures every click/drag, so this app only falls back to
//!    its own local text selection when Shift is held (Ghostty's own
//!    `mouse-shift-capture` convention -- see `crate::mouse_report::
//!    is_click_reporting_active`'s doc comment). Some users want the
//!    opposite: a plain drag always selects text locally (so `⌘C` copies
//!    it), and Shift is what forwards to the program instead. Default
//!    `false` (Ghostty's convention, this port's pre-existing behavior);
//!    a whole-app toggle rather than a per-program heuristic, since this
//!    port has no notion of "which program is running in this pane" to
//!    key a smarter default off of. Deliberately does **not** affect
//!    scroll-wheel forwarding (`mouse_report::is_scroll_reporting_active`
//!    takes no such parameter) -- inverting wheel scroll too would break
//!    scrolling inside the very programs (like Claude Code) this setting
//!    exists for.
//!
//! **Deliberately not here**: font/color settings -- the project's own
//! policy (`CLAUDE.md` for this wave) is that Ghostty config remains the
//! single source of truth for those (`crate::ghostty_config`); duplicating
//! them into this app's own settings would create two competing sources of
//! truth for the same rendering knobs.
//!
//! ## Persistence
//!
//! All these settings round-trip through `TaskDatabase`'s `appState` table
//! (`TaskDatabase::auto_resume_enabled`/`set_auto_resume_enabled`, etc. --
//! see that module's doc comment) -- the same key/value store the selected-
//! Task pointer already uses, per the wave brief's "保存は既存の appState
//! テーブルへ". Every toggle/adjustment in this module writes through
//! immediately (no separate "Save" step), matching this codebase's general
//! "persist on every mutating action" idiom (`LaboLaboApp::persist_workspace`
//! is the same shape for Task layouts).

use gpui::{
    div, prelude::*, px, rgb, rgba, Animation, AnimationExt, AnyElement, Context, IntoElement,
    MouseButton, MouseDownEvent,
};
use rust_i18n::t;

use labolabo_core::TaskDatabase;

use crate::app::LaboLaboApp;
use crate::i18n::LocaleSetting;
use crate::motion;
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
    /// UI language (wave 6f, `crate::i18n`). Defaults to `Auto` (OS-
    /// detected) -- see [`crate::i18n::LocaleSetting`]'s doc comment.
    pub locale: LocaleSetting,
    /// "アップデートを自動確認" (RC release wave) -- gates
    /// `crate::update_check`'s once-per-launch background check. See this
    /// module's doc comment, item 5.
    pub update_check_enabled: bool,
    /// "テキスト選択を優先" (wave 20) -- inverts the Shift/local-selection
    /// tie-break in `crate::mouse_report::is_click_reporting_active`. See
    /// this module's doc comment, item 6.
    pub prefer_local_selection: bool,
}

impl Default for AppSettings {
    /// Matches this port's pre-settings-screen behavior exactly: auto-
    /// resume was always on, the Git pane always defaulted to visible
    /// (`GitPaneState::default()`), and scrollback was always
    /// `DEFAULT_SCROLLBACK_LINES`. A brand-new database (every key absent)
    /// therefore behaves identically to before this wave. `locale` defaults
    /// to `Auto`, matching this port's pre-i18n-wave behavior for a `ja*`
    /// system (everything was hardcoded Japanese). `update_check_enabled`
    /// defaults to `true` -- same "opt-out, not opt-in" posture as Swift's
    /// own `checkUpdatesOnLaunch` default. `prefer_local_selection` defaults
    /// to `false` -- Ghostty's own convention, and this port's behavior
    /// before this setting existed.
    fn default() -> Self {
        Self {
            auto_resume_enabled: true,
            git_pane_default_visible: true,
            scrollback_lines: DEFAULT_SCROLLBACK_LINES,
            locale: LocaleSetting::Auto,
            update_check_enabled: true,
            prefer_local_selection: false,
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
            locale: crate::i18n::load_locale_setting(db),
            update_check_enabled: db
                .update_check_enabled()
                .ok()
                .flatten()
                .unwrap_or(defaults.update_check_enabled),
            prefer_local_selection: db
                .prefer_local_selection()
                .ok()
                .flatten()
                .unwrap_or(defaults.prefer_local_selection),
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

const OVERLAY_BG: u32 = theme::OVERLAY_SCRIM;
const PANEL_BG: u32 = theme::surface::ROOT;
const BORDER_COLOR: u32 = theme::surface::STROKE;
const BUTTON_BG: u32 = theme::surface::RAISED;
const TEXT_PRIMARY: u32 = theme::text::PRIMARY;
const TEXT_SECONDARY: u32 = theme::text::SECONDARY;
/// Panel width -- fixed, used both for layout and as the M4 entrance
/// animation's "gather in" base (see [`render_settings_overlay`]'s doc
/// comment on why this animates the panel's own width rather than a real
/// 2D scale transform).
const PANEL_WIDTH: f32 = 420.0;

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
///
/// ## Entrance animation (`plans/014` M4)
///
/// Because this element only exists in the tree while `settings_open()`,
/// every time it (re)appears is a fresh mount from gpui's point of view --
/// exactly the "first frame this element id shows up" moment
/// [`gpui::AnimationElement`] needs to start a brand new, correctly-timed
/// oneshot, so no extra open/close bookkeeping is needed here beyond the
/// existing `settings_open` flag.
///
/// The panel fades in (`opacity 0 -> 1`) over [`motion::OVERLAY_ENTER`] with
/// [`motion::ease_out_strong`], and -- unless [`motion::reduce_motion`] is
/// set -- also "gathers in" by animating its own width from ~97% to 100%
/// of [`PANEL_WIDTH`]. gpui 0.2's `Styled` trait has no generic 2D
/// scale/transform (only `svg` elements get a transformation matrix, per
/// `elements/svg.rs`), so a literal `scale(0.97 -> 1.0)` centered transform
/// -- what `plans/014` M4 asks for -- isn't directly expressible; animating
/// the panel's own width is used as an approximation instead, since the
/// parent's `.items_center().justify_center()` re-centers it every frame as
/// its size changes, which reads as "growing from the center" for this
/// rectangular panel without needing a true transform. This is a documented
/// deviation from the plan's literal wording, not an omission: it satisfies
/// the "feel チェック" note ("僅かに「寄ってくる」こと") while staying
/// within gpui 0.2's public API. The backdrop fades in over the same
/// duration, per the plan's "背景の薄暗幕も同時にフェード".
///
/// Exit is instant (no fade-out): implementing one would need the element
/// to keep rendering for [`motion::OVERLAY_EXIT`] *after* `settings_open`
/// flips false (a "closing" phase with its own timer, since this function
/// simply returns `None` and the element vanishes from the tree the moment
/// `settings_open` is false) -- a small state machine disproportionate to a
/// settings panel's close action. `plans/014` M4 explicitly allows this
/// trade-off ("コスト高なら即時クローズで可 -- 判断を PR に明記"); recorded
/// here as that judgment call.
pub fn render_settings_overlay(
    app: &LaboLaboApp,
    cx: &mut Context<LaboLaboApp>,
) -> Option<AnyElement> {
    if !app.settings_open() {
        return None;
    }
    let settings = *app.settings();
    let reduce_motion = motion::reduce_motion();

    let panel = div()
        .flex()
        .flex_col()
        .gap_3()
        .w(px(PANEL_WIDTH))
        .p_4()
        .rounded(px(theme::radius::OVERLAY))
        .bg(rgb(PANEL_BG))
        .border_1()
        .border_color(rgb(BORDER_COLOR))
        .shadow(theme::shadow::overlay())
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
                        .child(t!("settings.title").to_string()),
                )
                .child(close_button(cx)),
        )
        .child(
            toggle_row(
                "auto-resume",
                t!("settings.auto_resume.label").to_string(),
                t!("settings.auto_resume.footer").to_string(),
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
                t!("settings.git_pane_default_visible.label").to_string(),
                t!("settings.git_pane_default_visible.footer").to_string(),
                settings.git_pane_default_visible,
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _window, cx| {
                    this.set_git_pane_default_visible(
                        !this.settings().git_pane_default_visible,
                        cx,
                    );
                }),
            ),
        )
        .child(scrollback_row(settings.scrollback_lines, cx))
        .child(language_row(settings.locale, cx))
        .child(
            toggle_row(
                "update-check-enabled",
                t!("settings.update_check.label").to_string(),
                t!("settings.update_check.footer").to_string(),
                settings.update_check_enabled,
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _window, cx| {
                    this.set_update_check_enabled(!this.settings().update_check_enabled, cx);
                }),
            ),
        )
        .child(
            toggle_row(
                "prefer-local-selection",
                t!("settings.prefer_local_selection.label").to_string(),
                t!("settings.prefer_local_selection.footer").to_string(),
                settings.prefer_local_selection,
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _window, cx| {
                    this.set_prefer_local_selection(!this.settings().prefer_local_selection, cx);
                }),
            ),
        );

    // The "gather in" width nudge lives on the panel alone (no `.opacity()`
    // here) -- `opacity` composes multiplicatively with an ancestor's
    // (`Window::with_element_opacity`), so animating it on both this panel
    // *and* the backdrop below would fade the panel in as `t * t`, visibly
    // lagging behind the backdrop instead of appearing "together". A single
    // shared fade on the backdrop (which the panel is a child of) already
    // satisfies "背景の薄暗幕も同時にフェード" -- they fade as one unit.
    let panel = panel.with_animation(
        "settings-panel-enter",
        Animation::new(motion::OVERLAY_ENTER).with_easing(motion::ease_out_strong()),
        move |el, t| {
            if reduce_motion {
                // Reduce Motion (`plans/014` principle 4): position/size
                // movement is dropped, so the panel appears at its final
                // width immediately -- only the shared backdrop-level
                // opacity fade below still plays.
                el
            } else {
                let width = PANEL_WIDTH * (0.97 + 0.03 * t);
                el.w(px(width))
            }
        },
    );

    let backdrop = div()
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(rgba(OVERLAY_BG))
        .child(panel)
        .with_animation(
            "settings-backdrop-enter",
            Animation::new(motion::OVERLAY_ENTER).with_easing(motion::ease_out_strong()),
            |el, t| el.opacity(t),
        );

    Some(backdrop.into_any_element())
}

fn close_button(cx: &mut Context<LaboLaboApp>) -> impl IntoElement {
    div()
        .id("settings-close")
        .px_2()
        .py_1()
        .rounded_sm()
        .bg(rgb(BUTTON_BG))
        .text_color(rgb(TEXT_PRIMARY))
        .hover(|el| el.bg(rgb(theme::surface::ACTIVE)))
        .active(|el| el.opacity(0.8))
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
/// this row's identity across re-renders). `label`/`footer` take owned
/// `String`s (not `&'static str`) since wave 6f's callers build them from
/// `t!()`, which is locale-dependent at call time, not a compile-time
/// constant.
fn toggle_row(
    key: &'static str,
    label: String,
    footer: String,
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
        .hover(|el| el.bg(rgb(theme::surface::ACTIVE)))
        .active(|el| el.opacity(0.8))
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
                .child(div().child(t!("settings.scrollback.label").to_string()))
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
                .child(t!("settings.scrollback.footer").to_string()),
        )
}

/// The language row: a three-way "自動 / 日本語 / English" picker (wave
/// 6f). `current`'s own label doesn't need `t!()` at all (see
/// [`LocaleSetting::resolve`]'s doc comment) -- but the *other two* pill
/// labels do, since "自動"/"Automatic" is itself translated. Picking a
/// pill calls [`LaboLaboApp::set_locale`] directly, which applies live (see
/// `crate::i18n`'s module doc comment on why this doesn't need a "restart
/// to apply" footer note).
fn language_row(current: LocaleSetting, cx: &mut Context<LaboLaboApp>) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .p_2()
        .rounded_sm()
        .bg(rgb(BUTTON_BG))
        .child(
            div()
                .text_color(rgb(TEXT_PRIMARY))
                .child(t!("settings.language.label").to_string()),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .child(language_pill(
                    "language-auto",
                    t!("settings.language.auto").to_string(),
                    current == LocaleSetting::Auto,
                    cx.listener(|this, _: &MouseDownEvent, _window, cx| {
                        this.set_locale(LocaleSetting::Auto, cx);
                    }),
                ))
                .child(language_pill(
                    "language-ja",
                    t!("settings.language.name_ja").to_string(),
                    current == LocaleSetting::Ja,
                    cx.listener(|this, _: &MouseDownEvent, _window, cx| {
                        this.set_locale(LocaleSetting::Ja, cx);
                    }),
                ))
                .child(language_pill(
                    "language-en",
                    t!("settings.language.name_en").to_string(),
                    current == LocaleSetting::En,
                    cx.listener(|this, _: &MouseDownEvent, _window, cx| {
                        this.set_locale(LocaleSetting::En, cx);
                    }),
                )),
        )
        .child(
            div()
                .text_size(px(theme::font_size::CAPTION))
                .text_color(rgb(TEXT_SECONDARY))
                .child(t!("settings.language.footer").to_string()),
        )
}

fn language_pill(
    id: &'static str,
    label: String,
    active: bool,
    on_click: impl Fn(&MouseDownEvent, &mut gpui::Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .px_2()
        .py_1()
        .rounded_sm()
        .text_size(px(theme::font_size::LABEL))
        .when(active, |el| {
            el.bg(rgb(theme::ACCENT))
                .text_color(rgb(theme::text::ON_ACCENT))
        })
        .when(!active, |el| {
            el.bg(rgb(theme::surface::ACTIVE))
                .text_color(rgb(TEXT_PRIMARY))
        })
        .hover(|el| el.opacity(0.85))
        .active(|el| el.opacity(0.7))
        .on_mouse_down(MouseButton::Left, on_click)
        .child(label)
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
        .hover(|el| el.opacity(0.85))
        .active(|el| el.opacity(0.7))
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
        assert_eq!(defaults.locale, LocaleSetting::Auto);
        assert!(defaults.update_check_enabled);
        assert!(!defaults.prefer_local_selection);
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
        db.set_locale("ja").unwrap();
        db.set_update_check_enabled(false).unwrap();
        db.set_prefer_local_selection(true).unwrap();

        let loaded = AppSettings::load(&db);
        assert_eq!(
            loaded,
            AppSettings {
                auto_resume_enabled: false,
                git_pane_default_visible: false,
                scrollback_lines: 5000,
                locale: LocaleSetting::Ja,
                update_check_enabled: false,
                prefer_local_selection: true,
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
        assert!(loaded.update_check_enabled);
        assert!(!loaded.prefer_local_selection);
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
