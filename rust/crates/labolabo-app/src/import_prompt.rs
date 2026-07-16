//! The first-launch Swift-import confirmation prompt (第8波d) — replaces
//! W6e's confirmation-less automatic import (`crate::swift_import`'s module
//! doc comment covers what that used to look like and what still calls
//! [`crate::swift_import::run`] today).
//!
//! ## Why a prompt instead of "just do it"
//!
//! W6e ran the import silently the moment `tasks.db` was empty and a Swift
//! `labolabo.db` existed. User feedback: migration from the Swift app is a
//! one-time event, not something worth a permanent menu entry, and a silent
//! auto-import is surprising the one time it actually matters (a genuinely
//! fresh install). This module replaces that with an explicit yes/no
//! overlay, asked **at most once ever** — see [`should_show_import_prompt`].
//!
//! ## The state machine
//!
//! [`should_show_import_prompt`] is a pure gate over three independent
//! facts, computed once at startup (`LaboLaboApp::new`):
//!
//! - `has_any_task`: the sidebar already has at least one Task (active or
//!   archived) — an already-populated install never prompts, preserving
//!   W6e's original "初回起動" scoping (`!tasks.is_empty() ||
//!   !archived_tasks.is_empty()`, the same union `LaboLaboApp::new` already
//!   computed before this wave).
//! - `swift_db_exists`: the Swift app's `labolabo.db` is actually present
//!   (`crate::swift_import::swift_db_exists`) — nothing to offer otherwise.
//! - `prompt_answered`: the user has already answered this prompt once
//!   (`labolabo_core::TaskDatabase::swift_import_prompt_answered`, persisted
//!   so the prompt is a true one-shot — the task brief's "1 回限り"). The
//!   caller passes `false` here regardless of what's persisted when
//!   `crate::swift_import::force_import_prompt` is set
//!   (`LABOLABO_FORCE_IMPORT_PROMPT=1`, a developer escape hatch — see
//!   `rust/README.md`), so the prompt can be re-tested without wiping
//!   `tasks.db`.
//!
//! All three must line up (no tasks yet, a Swift db to offer, never
//! answered) for the prompt to show. Whichever button the user clicks,
//! `LaboLaboApp::accept_swift_import_prompt`/`decline_swift_import_prompt`
//! persist the answered flag immediately, so reopening the app — even with
//! the Swift db still present — never asks again short of the two escape
//! hatches documented in `rust/README.md` (delete `tasks.db`, or the env
//! var above).
//!
//! ## UI
//!
//! Same centered-panel + scrim style as `task_menu.rs`'s delete-confirm
//! modal / `settings.rs`'s panel (`theme::radius::OVERLAY` corners,
//! `theme::OVERLAY_SCRIM` backdrop, `motion::OVERLAY_ENTER` fade-in) — two
//! buttons, no click-outside-to-close: unlike `task_menu.rs`'s popover menu
//! (where an outside click is just "never mind, close the menu"), this
//! prompt's "no answer yet" state has no discardable meaning — every path
//! out of it must be an explicit 取り込む/取り込まない so the persisted
//! answered flag always reflects a real choice. A small note under the body
//! text ("この確認は今回限りです" / its `once_notice` translation) tells the
//! user up front that 取り込まない is not a "remind me later" — see
//! `rust/README.md` for the documented way back in if they change their
//! mind afterward.

use gpui::{
    div, prelude::*, px, rgb, rgba, Animation, AnimationExt, AnyElement, App, Context, IntoElement,
    MouseButton, MouseDownEvent, SharedString,
};
use rust_i18n::t;

use crate::app::LaboLaboApp;
use crate::motion;
use crate::theme;

/// Confirm modal width — same as `task_menu.rs`'s `CONFIRM_WIDTH` /
/// `settings.rs`'s `PANEL_WIDTH`, the codebase's one shared "centered
/// dialog" size.
const PANEL_WIDTH: f32 = 420.0;
const OVERLAY_BG: u32 = theme::OVERLAY_SCRIM;

/// Whether the first-launch Swift-import confirmation prompt should be
/// shown, given the three independent facts this module's doc comment
/// describes. Pure and gpui-free so every one of the 2×2×2 combinations is
/// directly unit-testable without a `TaskDatabase`/`LaboLaboApp` in the
/// loop.
pub fn should_show_import_prompt(
    has_any_task: bool,
    swift_db_exists: bool,
    prompt_answered: bool,
) -> bool {
    !has_any_task && swift_db_exists && !prompt_answered
}

/// The prompt overlay (`app.import_prompt_open()` のときだけ `Some`).
/// Callers append this as a child of the root render tree
/// (`app.rs`'s `Render for LaboLaboApp`), same `.children(..)` pattern as
/// `settings::render_settings_overlay`/`task_menu::render_task_menu_overlay`.
pub fn render_import_prompt_overlay(
    app: &LaboLaboApp,
    cx: &mut Context<LaboLaboApp>,
) -> Option<AnyElement> {
    if !app.import_prompt_open() {
        return None;
    }

    let panel = div()
        .flex()
        .flex_col()
        .gap_3()
        .w(px(PANEL_WIDTH))
        .p_4()
        .rounded(px(theme::radius::OVERLAY))
        .bg(rgb(theme::surface::ROOT))
        .border_1()
        .border_color(rgb(theme::surface::STROKE))
        .shadow(theme::shadow::overlay())
        // パネル内クリックはバックドロップの「閉じる」まで届かせない
        // （`task_menu.rs` module doc コメントのクリック伝播設計と同じ --
        // このオーバーレイにバックドロップの「閉じる」ハンドラ自体は無い
        // が、将来の変更で誤って足された場合に備えて他オーバーレイと同じ
        // 防御を揃えておく）。
        .on_mouse_down(MouseButton::Left, |_event, _window, cx: &mut App| {
            cx.stop_propagation();
        })
        .child(
            div()
                .text_size(px(15.0))
                .text_color(rgb(theme::text::PRIMARY))
                .child(t!("import_prompt.title").to_string()),
        )
        .child(
            div()
                .text_size(px(theme::font_size::LABEL))
                .text_color(rgb(theme::text::PRIMARY))
                .child(t!("import_prompt.body").to_string()),
        )
        .child(
            div()
                .text_size(px(theme::font_size::CAPTION))
                .text_color(rgb(theme::text::MUTED))
                .child(t!("import_prompt.once_notice").to_string()),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .justify_end()
                .gap_2()
                .child(dialog_button(
                    "swift-import-prompt-decline",
                    t!("import_prompt.decline").to_string().into(),
                    cx.listener(|this, _: &MouseDownEvent, _window, cx| {
                        this.decline_swift_import_prompt(cx);
                    }),
                ))
                .child(dialog_button(
                    "swift-import-prompt-accept",
                    t!("import_prompt.accept").to_string().into(),
                    cx.listener(|this, _: &MouseDownEvent, _window, cx| {
                        this.accept_swift_import_prompt(cx);
                    }),
                )),
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
            "swift-import-prompt-backdrop-enter",
            Animation::new(motion::OVERLAY_ENTER).with_easing(motion::ease_out_strong()),
            |el, t| el.opacity(t),
        );

    Some(backdrop.into_any_element())
}

fn dialog_button(
    id: &'static str,
    label: SharedString,
    on_click: impl Fn(&MouseDownEvent, &mut gpui::Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .px_3()
        .py_1()
        .rounded_sm()
        .bg(rgb(theme::surface::RAISED))
        .text_color(rgb(theme::text::PRIMARY))
        .hover(|el| el.bg(rgb(theme::surface::ACTIVE)))
        .active(|el| el.opacity(0.8))
        .on_mouse_down(MouseButton::Left, on_click)
        .child(label)
}

#[cfg(test)]
mod tests {
    use super::*;

    // MARK: - should_show_import_prompt (全 2x2x2 組み合わせ)

    #[test]
    fn shows_only_when_no_tasks_yet_a_swift_db_exists_and_never_answered() {
        assert!(should_show_import_prompt(false, true, false));
    }

    #[test]
    fn never_shows_when_any_task_already_exists() {
        assert!(!should_show_import_prompt(true, true, false));
        assert!(!should_show_import_prompt(true, true, true));
        assert!(!should_show_import_prompt(true, false, false));
        assert!(!should_show_import_prompt(true, false, true));
    }

    #[test]
    fn never_shows_when_no_swift_database_exists() {
        assert!(!should_show_import_prompt(false, false, false));
        assert!(!should_show_import_prompt(false, false, true));
    }

    #[test]
    fn never_shows_once_already_answered() {
        assert!(!should_show_import_prompt(false, true, true));
    }

    #[test]
    fn every_combination_matches_the_pure_and_of_all_three_gates() {
        for has_any_task in [false, true] {
            for swift_db_exists in [false, true] {
                for prompt_answered in [false, true] {
                    let expected = !has_any_task && swift_db_exists && !prompt_answered;
                    assert_eq!(
                        should_show_import_prompt(has_any_task, swift_db_exists, prompt_answered),
                        expected,
                        "has_any_task={has_any_task} swift_db_exists={swift_db_exists} prompt_answered={prompt_answered}"
                    );
                }
            }
        }
    }
}
