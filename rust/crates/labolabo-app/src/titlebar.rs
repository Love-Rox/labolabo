//! Window-integrated toolbar chrome (第13波b §1 -- モダン化第2弾、本丸).
//!
//! Previously the app used the OS's native titlebar as-is (a plain
//! `Some(TitlebarOptions::default())`, i.e. `appears_transparent: false` --
//! a normal title bar with a blank title, sitting above an unrelated
//! sidebar/workspace row that started its own chrome from scratch). This
//! wave integrates the two on macOS: the native titlebar is hidden
//! (`appears_transparent: true`), the traffic lights are repositioned to
//! sit inline in [`render`]'s own top row (rather than floating over empty
//! space), and that row doubles as the window's drag handle
//! (`WindowControlArea::Drag`, gpui 0.2's real API for this -- confirmed in
//! `elements/div.rs`/`window.rs`, not a hand-rolled mouse-drag hack) with a
//! status pill on its right showing the selected Task at a glance (title +
//! attached/worktree kind + branch + changed-file count -- "Swift 版の
//! `SessionStatusPill` 相当の情報量" per the wave's brief,
//! `app/Sources/SessionStatusBar.swift`).
//!
//! ## Why macOS-only (`#[cfg(target_os = "macos")]`)
//!
//! [`gpui::TitlebarOptions::appears_transparent`]'s own doc comment: "macOS
//! and Windows only." Setting it unconditionally would also hide *Windows'*
//! native titlebar -- not what this wave wants (brief: "Linux/Windows は
//! ネイティブ装飾のまま"). So [`window_titlebar_options`] is cfg-gated, not
//! branched at runtime: the non-mac build never even constructs a
//! transparent `TitlebarOptions`, and [`render`]'s own
//! `.window_control_area(Drag)` call is likewise mac-only (Linux/Windows
//! keep their native titlebar's own drag handling -- marking a second
//! region draggable would be redundant, and this row isn't replacing their
//! titlebar the way it replaces macOS's). The status pill itself **does**
//! render on every platform (it's just informational content, not part of
//! "is this OS chrome or app chrome") -- only the traffic-light inlining and
//! drag-region wiring are mac-specific.
//!
//! ## Power principle
//!
//! No new continuous animation. The pill's content only changes when
//! `LaboLaboApp::render` re-runs, which already happens exactly on the
//! existing Git-refresh/selection-change triggers (`crate::git_pane`'s
//! `apply_git_refresh`, `LaboLaboApp::select_task`) -- this module adds no
//! new subscription, timer, or `cx.notify()` call of its own.

use gpui::{div, prelude::*, px, rgb, FontWeight, IntoElement, SharedString, TitlebarOptions};
#[cfg(target_os = "macos")]
use gpui::{point, Pixels, Point};
use rust_i18n::t;

use labolabo_core::TaskKind;

use crate::icons::{self, Icon};
use crate::pr_status;
use crate::theme;

/// Height of the app-drawn top chrome row -- both the macOS custom-titlebar
/// row and (on every platform) the plain toolbar row below the OS's own
/// titlebar. Same "just a constant, no resize handle" simplification
/// `git_pane::GIT_PANE_WIDTH` still has for its own fixed dimension (the
/// sidebar's width grew a drag handle as of `plans` 第16波 #1, but this
/// row's height has no such need).
pub const HEIGHT: f32 = 38.0;

/// macOS only: horizontal space reserved at the row's left edge so its own
/// content never overlaps the inline traffic-light cluster AppKit paints at
/// [`traffic_light_position`]'s origin. `12px` (the cluster's left inset,
/// matching [`traffic_light_position`]) + `~52px` (three 12px buttons with
/// AppKit's own spacing) + a small margin.
#[cfg(target_os = "macos")]
const TRAFFIC_LIGHT_RESERVED_WIDTH: f32 = 78.0;

/// The `WindowOptions::titlebar` value for the app's one window
/// (`main.rs`). See this module's doc comment for the mac-only rationale.
///
/// Deliberately returns `Some(TitlebarOptions::default())` (not `None`) on
/// non-mac platforms -- `TitlebarOptions::default()` is exactly what
/// `WindowOptions::default()` itself supplies
/// (`appears_transparent: false`, a normal native titlebar), so this keeps
/// today's Linux/Windows behavior byte-for-byte unchanged rather than
/// leaning on `None`'s own (differently-defaulted, per-platform) fallback
/// semantics.
#[cfg(target_os = "macos")]
pub fn window_titlebar_options() -> Option<TitlebarOptions> {
    Some(TitlebarOptions {
        title: None,
        appears_transparent: true,
        // Vertically centered in `HEIGHT` (AppKit's traffic-light cluster
        // is ~12px tall); `x = 12.0` matches
        // `TRAFFIC_LIGHT_RESERVED_WIDTH`'s own left inset.
        traffic_light_position: Some(traffic_light_position()),
    })
}

#[cfg(not(target_os = "macos"))]
pub fn window_titlebar_options() -> Option<TitlebarOptions> {
    Some(TitlebarOptions::default())
}

#[cfg(target_os = "macos")]
fn traffic_light_position() -> Point<Pixels> {
    point(px(12.0), px((HEIGHT - 12.0) / 2.0))
}

/// Pure data [`render`] draws the status pill from -- collected by
/// `LaboLaboApp::titlebar_pill_data` (`app.rs`) from the selected Task, its
/// `TaskWorkspace::git` state, and nothing else gpui-specific. Kept
/// gpui-free so the "assembly" logic (kind resolution, branch fallback) is
/// unit-testable without a `Context`/`Window` -- the quality gate this
/// wave's brief calls out ("ピルの表示文字列組み立て...はユニットテスト").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PillData {
    pub task_title: String,
    pub is_worktree: bool,
    /// Already resolved to a display fallback (`"-"`) when there's no
    /// branch yet -- mirrors Swift's `SessionStatusPill.branchLabel`
    /// (`status?.branch ?? fallbackBranch ?? "—"`).
    pub branch: String,
    pub changed_count: usize,
    /// The selected Task's PR (`plans` 第16波 #3) -- `None` for an
    /// `attached`-kind Task, or a worktree Task with no fetched PR yet
    /// (`LaboLaboApp::task_pr_info`). Mirrors `crate::sidebar`'s row badge
    /// content (same "worktree Task の owner/repo#number" info), just
    /// rendered as a third pill segment here instead of an inline badge.
    pub pr: Option<PrPillData>,
}

/// The titlebar pill's PR segment (`plans` 第16波 #3) -- just enough to
/// render `#<number>` + a state label; the color/label text themselves are
/// resolved in [`render_pill`] (same split `PillData::build`'s other
/// fields already have: this struct carries raw data, [`render_pill`] owns
/// theme/i18n lookups), so this struct's own equality/tests don't need a
/// locale or gpui `Context` on hand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrPillData {
    pub number: u64,
    pub state: pr_status::PrState,
}

impl PillData {
    /// Builds the pill's data from raw inputs: the selected Task's title +
    /// kind, its Git pane's branch (`None` while still loading, or for a
    /// non-repo `attached` Task), its changed-file count, and its PR
    /// (`plans` 第16波 #3, `None` for anything without one -- see
    /// [`PrPillData`]'s doc comment). The only "logic" here -- the branch
    /// fallback -- mirrors Swift's own `branchLabel` computed property (see
    /// this struct's doc comment).
    pub fn build(
        task_title: &str,
        is_worktree: bool,
        branch: Option<&str>,
        changed_count: usize,
        pr: Option<PrPillData>,
    ) -> Self {
        let branch = branch
            .map(str::trim)
            .filter(|b| !b.is_empty())
            .unwrap_or("-")
            .to_string();
        Self {
            task_title: task_title.to_string(),
            is_worktree,
            branch,
            changed_count,
            pr,
        }
    }
}

/// `TaskKind` -> [`PillData::is_worktree`]. A tiny free function (rather
/// than inlining `matches!` at every call site) purely so
/// [`PillData::build`]'s own doc comment can point at one place for "how is
/// worktree-ness decided" -- mirrors `sidebar::kind_marker`'s match, kept
/// separate since that one returns a glyph/icon, not a bool.
pub fn is_worktree_kind(kind: &TaskKind) -> bool {
    matches!(kind, TaskKind::Worktree { .. })
}

/// Renders the top chrome row: on macOS, the window's drag handle (see this
/// module's doc comment) with the traffic lights' reserved space at its
/// left; on every platform, the status pill at its right (`None` when no
/// Task is selected -- just an empty, still-draggable bar).
pub fn render(pill_data: Option<PillData>) -> impl IntoElement {
    let bar = div()
        .flex()
        .flex_row()
        .items_center()
        .justify_end()
        .h(px(HEIGHT))
        .w_full()
        .flex_shrink_0()
        .pr_3()
        .bg(rgb(theme::surface::RAISED))
        .border_b_1()
        .border_color(rgb(theme::surface::STROKE));

    let bar = apply_leading_inset(bar);
    let bar = apply_drag_region(bar);

    bar.children(pill_data.map(render_pill))
}

#[cfg(target_os = "macos")]
fn apply_leading_inset(bar: gpui::Div) -> gpui::Div {
    bar.pl(px(TRAFFIC_LIGHT_RESERVED_WIDTH))
}

#[cfg(not(target_os = "macos"))]
fn apply_leading_inset(bar: gpui::Div) -> gpui::Div {
    bar.pl_3()
}

/// The bar itself is the window's drag handle on macOS (see module doc
/// comment) -- `gpui::WindowControlArea::Drag`, gpui 0.2's real API for
/// this (`Div::window_control_area`/`InteractiveElement::
/// window_control_area`, confirmed in `elements/div.rs`), not a hand-rolled
/// `on_mouse_down` + platform-move hack. A no-op on other platforms, whose
/// native titlebar (kept intact, see [`window_titlebar_options`]) already
/// owns window dragging.
#[cfg(target_os = "macos")]
fn apply_drag_region(bar: gpui::Div) -> gpui::Div {
    bar.window_control_area(gpui::WindowControlArea::Drag)
}

#[cfg(not(target_os = "macos"))]
fn apply_drag_region(bar: gpui::Div) -> gpui::Div {
    bar
}

/// A 12px-tall hairline between pill segments -- mirrors Swift's `Divider()
/// .frame(height: 12)` in `SessionStatusPill`.
fn pill_divider() -> impl IntoElement {
    div().w(px(1.0)).h(px(12.0)).bg(rgb(theme::surface::STROKE))
}

fn pill_segment(marker: impl IntoElement, label: impl Into<SharedString>) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap_1()
        .child(marker)
        .child(label.into())
}

/// The kind segment's marker -- mirrors `sidebar::kind_marker`'s own
/// branch-icon-vs-filled-dot grammar exactly (same two shapes, same
/// meaning) rather than introducing a third "what does attached look like"
/// visual language just for the pill.
fn kind_marker(is_worktree: bool) -> gpui::AnyElement {
    if is_worktree {
        icons::icon_colored(Icon::Branch, 11.0, theme::text::SECONDARY).into_any_element()
    } else {
        div()
            .w(px(5.0))
            .h(px(5.0))
            .flex_shrink_0()
            .rounded_full()
            .bg(rgb(theme::text::SECONDARY))
            .into_any_element()
    }
}

fn render_pill(data: PillData) -> impl IntoElement {
    let kind_label = if data.is_worktree {
        t!("titlebar.kind_worktree").to_string()
    } else {
        t!("titlebar.kind_attached").to_string()
    };

    let (changes_icon, changes_color, changes_label) = if data.changed_count == 0 {
        (
            Icon::Check,
            theme::diff::ADD,
            t!("titlebar.changes_clean").to_string(),
        )
    } else {
        (
            Icon::Diff,
            theme::status::STARTING,
            t!("titlebar.changes_count", count = data.changed_count).to_string(),
        )
    };

    div()
        .id("titlebar-pill")
        .flex()
        .flex_row()
        .items_center()
        .gap_2()
        .px_3()
        .h(px(24.0))
        .rounded_full()
        .bg(rgb(theme::surface::ACTIVE))
        .text_size(px(theme::font_size::CAPTION))
        .text_color(rgb(theme::text::SECONDARY))
        .child(
            div()
                .text_color(rgb(theme::text::PRIMARY))
                .font_weight(FontWeight::SEMIBOLD)
                .child(SharedString::from(data.task_title)),
        )
        .child(pill_divider())
        .child(pill_segment(kind_marker(data.is_worktree), kind_label))
        .child(pill_divider())
        .child(pill_segment(
            icons::icon_colored(Icon::Branch, 11.0, theme::text::SECONDARY),
            data.branch,
        ))
        .child(pill_divider())
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_1()
                .text_color(rgb(changes_color))
                .child(icons::icon_colored(changes_icon, 11.0, changes_color))
                .child(SharedString::from(changes_label)),
        )
        .when_some(data.pr, |el, pr| {
            let color = pr_status::badge_color(pr.state);
            let state_label = pr_state_label(pr.state);
            el.child(pill_divider()).child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .text_color(rgb(color))
                    .child(
                        div()
                            .w(px(5.0))
                            .h(px(5.0))
                            .flex_shrink_0()
                            .rounded_full()
                            .bg(rgb(color)),
                    )
                    .child(SharedString::from(format!("#{} {state_label}", pr.number))),
            )
        })
        .into_any_element()
}

/// [`pr_status::PrState`] -> the titlebar pill's localized state label
/// (`crate::sidebar`'s row badge doesn't need one of its own -- the color +
/// `#number` alone reads fine at that smaller size, and the tooltip already
/// carries the PR's title; the titlebar pill has room for a word too).
fn pr_state_label(state: pr_status::PrState) -> String {
    match state {
        pr_status::PrState::Draft => t!("titlebar.pr_draft").to_string(),
        pr_status::PrState::Open => t!("titlebar.pr_open").to_string(),
        pr_status::PrState::Merged => t!("titlebar.pr_merged").to_string(),
        pr_status::PrState::Closed => t!("titlebar.pr_closed").to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // MARK: - PillData::build (branch fallback / assembly logic)

    #[test]
    fn a_present_branch_is_used_verbatim() {
        let data = PillData::build("my-task", true, Some("feature/x"), 0, None);
        assert_eq!(data.branch, "feature/x");
        assert_eq!(data.task_title, "my-task");
        assert!(data.is_worktree);
        assert_eq!(data.changed_count, 0);
        assert_eq!(data.pr, None);
    }

    #[test]
    fn no_branch_falls_back_to_a_dash() {
        let data = PillData::build("my-task", false, None, 3, None);
        assert_eq!(data.branch, "-");
        assert!(!data.is_worktree);
        assert_eq!(data.changed_count, 3);
    }

    #[test]
    fn an_empty_or_whitespace_branch_also_falls_back_to_a_dash() {
        // Defensive: a `git status` parse should never actually hand back
        // `Some("")`/`Some("  ")`, but this function's own contract doesn't
        // lean on that -- mirrors the same defensiveness
        // `git_pane::render_branch_bar`'s `unwrap_or_else(|| "-".to_string())`
        // already has for the (separate) fixed-pane branch label.
        assert_eq!(PillData::build("t", true, Some(""), 0, None).branch, "-");
        assert_eq!(PillData::build("t", true, Some("   "), 0, None).branch, "-");
    }

    #[test]
    fn build_carries_the_pr_data_through_verbatim() {
        let pr = PrPillData {
            number: 42,
            state: pr_status::PrState::Open,
        };
        let data = PillData::build("t", true, Some("b"), 0, Some(pr));
        assert_eq!(data.pr, Some(pr));
    }

    #[test]
    fn is_worktree_kind_matches_only_the_worktree_variant() {
        assert!(is_worktree_kind(&TaskKind::Worktree {
            branch: "b".into(),
            base: "main".into(),
            path: "/tmp/x".into(),
        }));
        assert!(!is_worktree_kind(&TaskKind::Attached {
            directory: "/tmp/y".into(),
        }));
    }
}
