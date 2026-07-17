//! A shared "nothing here yet" visual tone (第13波b §3 -- モダン化第2弾):
//! a centered muted icon + one line of text, with an optional primary
//! action button below it. Previously every "empty" spot in the app
//! (no Task selected, the Git pane's changed-files list with no changes)
//! rolled its own left-aligned plain-text treatment
//! (`app::empty_state`/`git_pane::render_file_list`'s empty branch) --
//! this module gives them one consistent shape, per the wave's brief
//! ("タスク未選択時...の...領域に、中央配置の簡潔な案内...Git ペインの
//! 「変更なし」も同じトーンで").
//!
//! Deliberately small: this is presentation only (an icon, a `SharedString`
//! message, and an optional label/click-handler pair for the action), no
//! state of its own -- callers decide when to show it and supply
//! already-localized text.
//!
//! [`render`]/[`render_message`] return just the icon+text(+button)
//! *cluster*, sized to its own content -- not a full-bleed centered
//! container. Each call site still builds its own outer `.flex_1()`/
//! `.size_full()` + `.items_center().justify_center()` wrapper (exactly
//! `app::render_missing_task_placeholder`'s existing pattern, which this
//! module doesn't replace), since how much space is available to center
//! within is different at each call site (the whole workspace area vs. the
//! Git pane's ~40%-height file-list box) and isn't this module's business.

use gpui::{
    div, prelude::*, px, rgb, App, IntoElement, MouseButton, MouseDownEvent, SharedString, Window,
};

use crate::icons::{self, Icon};
use crate::theme;

/// A boxed mouse-down handler, matching `Div::on_mouse_down`'s own bound
/// (`impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static`) -- named so
/// [`Action::on_click`] doesn't spell the whole trait object out inline
/// (clippy's `type_complexity` lint).
type ClickHandler = Box<dyn Fn(&MouseDownEvent, &mut Window, &mut App) + 'static>;

/// A clickable primary action rendered below the message (e.g. "＋ 新しい
/// 作業"). `None` (the common case -- the Git pane's "変更なし" has no
/// action) just omits the button entirely.
pub struct Action {
    pub id: &'static str,
    pub label: SharedString,
    pub on_click: ClickHandler,
}

/// Renders the shared empty-state tone: `icon` (muted, 28px -- large enough
/// to read as an illustration, not another inline glyph), `message` (one
/// line, `text::SECONDARY`), and `action` (if given) as a filled button in
/// `theme::BRAND` -- the same lime the sidebar uses for "this is the
/// primary next step" (its selected-Task accent, see `theme::BRAND`'s doc
/// comment), fitting for the one button this empty state offers.
pub fn render(
    icon: Icon,
    message: impl Into<SharedString>,
    action: Option<Action>,
) -> gpui::AnyElement {
    let mut column = div()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_3()
        .child(icons::icon_colored(icon, 28.0, theme::text::MUTED))
        .child(
            div()
                .text_color(rgb(theme::text::SECONDARY))
                .text_size(px(theme::font_size::LABEL))
                .child(message.into()),
        );

    if let Some(action) = action {
        column = column.child(
            div()
                .id(action.id)
                .px_3()
                .py_1p5()
                .rounded(px(theme::radius::ROW))
                .bg(rgb(theme::BRAND))
                .text_color(rgb(theme::text::ON_ACCENT))
                .text_size(px(theme::font_size::LABEL))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .hover(|el| el.opacity(0.9))
                .active(|el| el.opacity(0.8))
                .on_mouse_down(MouseButton::Left, action.on_click)
                .child(action.label),
        );
    }

    column.into_any_element()
}

/// The lighter variant with no action button -- the Git pane's "変更なし"
/// (`git_pane::render_file_list`'s empty branch) and any other "just tell
/// the user nothing's here" spot. A thin wrapper over [`render`] so every
/// no-action caller doesn't have to spell out `None` themselves.
pub fn render_message(icon: Icon, message: impl Into<SharedString>) -> gpui::AnyElement {
    render(icon, message, None)
}
