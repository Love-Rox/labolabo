//! Sidebar: a repo-grouped Task list plus the "+" new-Task affordances
//! (`plans/012-task-model-and-control-cli.md` §1: "サイドバー = リポジトリ
//! ごとにグルーピングされた Task 一覧"). [`group_tasks_by_repo`] is pure and
//! gpui-free (unit-tested below) -- the same "logic lives outside gpui"
//! discipline `crate::focus` already established for the tile/tab tree;
//! [`render`] just walks its output and wires up click/drag/drop handlers.
//!
//! Deliberately minimal per this wave's brief ("title + kind の別が分かる
//! 程度の最小表示" / "本格ダイアログは将来"): title + a small kind marker.
//! Drag & drop (plan §3) *is* implemented here: dragging a Task row
//! reorders it within its repo group (`LaboLaboApp::
//! reorder_tasks_in_sidebar`, ordering math in `labolabo_core::
//! reorder_task_ids`), and dropping an OS folder anywhere on the sidebar
//! starts a new attached Task there (`LaboLaboApp::
//! handle_sidebar_folder_drop`).
//!
//! Wave 6c adds: 行ホバーの「…」ボタン（`crate::task_menu` のアーカイブ/
//! 削除/IDE で開くメニュー）と、下部の「アーカイブ済み (n)」折りたたみ
//! セクション（[`render_archived_section`]、復元ボタン付き）。
//!
//! 第13波b §2 (SVG アイコン体系): every glyph in this module (worktree
//! marker, conflict/missing badges, "…" menu button, archived-section
//! chevron, banner dismiss "×") is now `crate::icons`-drawn instead of a
//! plain Unicode text child -- see that module's doc comment for the
//! rendering model (single-tone, tinted via `.text_color(..)`).

use gpui::{
    div, prelude::*, px, rgb, rgba, App, Context, CursorStyle, ExternalPaths, FontWeight,
    IntoElement, MouseButton, MouseDownEvent, Render, SharedString, Window,
};
use rust_i18n::t;

use labolabo_core::{AgentStatus, Task, TaskKind};

use crate::app::LaboLaboApp;
use crate::icons::{self, Icon};
use crate::motion;
use crate::pr_status;
use crate::task_workspace::{self, status_dot_color};
use crate::theme;

/// Background tint applied to a Task row while a same-repo row is being
/// dragged over it (`.drag_over::<TaskDragPayload>`) -- "drop here" for
/// reordering. A different hue from
/// `task_workspace::MOVE_DROP_HIGHLIGHT_COLOR`/`FILE_DROP_HIGHLIGHT_COLOR`
/// (green rather than blue/amber) simply because it's a different
/// affordance (list reorder, not pane split/merge or file insert) in a
/// different part of the UI -- there's no shared-meaning requirement
/// across the two DnD systems the way §3.1 requires within the
/// terminal-pane one.
const TASK_ROW_DROP_HIGHLIGHT_COLOR: u32 = theme::with_alpha(theme::dnd::REORDER, 0x4d);
/// Background tint applied to the whole sidebar while an OS folder is
/// being dragged over it (`.drag_over::<ExternalPaths>`).
const SIDEBAR_FOLDER_DROP_HIGHLIGHT_COLOR: u32 = theme::with_alpha(theme::dnd::REORDER, 0x2a);

/// Payload of an in-progress Task-row drag (`render`'s per-row `.on_drag`):
/// the dragged Task's id plus its repo key, so a drop target's `can_drop`
/// can reject a cross-repo drop without needing to look the Task back up
/// (`plans/012` §3: "同一リポジトリグループ内の並び替え").
#[derive(Debug, Clone, PartialEq, Eq)]
struct TaskDragPayload {
    task_id: String,
    repo_key: String,
}

/// The floating view rendered under the cursor while a Task row is being
/// dragged -- just its title, echoing `task_workspace::TabDragPreview`'s
/// reasoning (gpui has no default drag image for a value-only drag).
struct TaskDragPreview(SharedString);

impl Render for TaskDragPreview {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py_1()
            .rounded_sm()
            .bg(rgb(theme::surface::ACTIVE))
            .text_color(rgb(theme::text::PRIMARY))
            .text_size(px(theme::font_size::CAPTION))
            .child(self.0.clone())
    }
}

/// A minimal text-only tooltip content view. gpui 0.2's `Div::tooltip`
/// (`elements/div.rs`) already provides the hover-delay (~500ms), auto
/// positioning, and dismiss-on-scroll/click machinery for free -- it just
/// needs a `Render`-implementing view to show, and ships no ready-made one,
/// so this is the same small-`Render`-struct shape as [`TaskDragPreview`]/
/// `task_workspace::TabDragPreview` above.
///
/// Used by [`icon_button`] (the sidebar's two "new Task" icon buttons --
/// see that function's doc comment for why icons + tooltip replaced the
/// previous two-text-button row) and the changed-file conflict badge below.
/// `pub(crate)` so `crate::task_workspace`'s Git-tile-open icon buttons
/// (`plans` W6d) can reuse the same tooltip shape instead of redefining it.
pub(crate) struct IconTooltip(pub SharedString);

impl Render for IconTooltip {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py_1()
            .rounded_sm()
            .bg(rgb(theme::surface::RAISED))
            .border_1()
            .border_color(rgb(theme::surface::STROKE))
            .text_color(rgb(theme::text::PRIMARY))
            .text_size(px(theme::font_size::CAPTION))
            .max_w(px(240.0))
            .child(self.0.clone())
    }
}

/// Sidebar width the app starts with before any drag-resize (`plans` 第16波
/// #1) or persisted `sidebarWidth` (`labolabo_core::TaskDatabase::
/// sidebar_width`) is applied -- unchanged from the previous fixed value.
pub const DEFAULT_SIDEBAR_WIDTH: f32 = 220.0;

/// Narrowest the sidebar can be dragged to -- still wide enough for a
/// worktree row's branch icon + a short title + the "…" menu button without
/// truncating illegibly.
pub const MIN_SIDEBAR_WIDTH: f32 = 180.0;

/// Widest the sidebar can be dragged to -- past this it would start eating
/// into the terminal/Git-pane workspace for no real benefit (the sidebar's
/// content is a flat list, not a document that benefits from extra width).
pub const MAX_SIDEBAR_WIDTH: f32 = 480.0;

/// Clamps a candidate sidebar width (`app::LaboLaboApp::
/// update_sidebar_width_drag`'s raw drag-position pixel value, or a
/// persisted `sidebarWidth` value read back at startup) into
/// `[MIN_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH]`. Non-finite input (`NaN`/`±inf`
/// -- reachable from a corrupt persisted value, or in principle a
/// degenerate drag position) falls back to [`DEFAULT_SIDEBAR_WIDTH`] rather
/// than propagating garbage into layout, mirroring `grid::
/// ratio_from_drag_position`'s callers' own "reject non-finite, keep the
/// previous value" posture.
pub fn clamp_sidebar_width(width: f32) -> f32 {
    if !width.is_finite() {
        return DEFAULT_SIDEBAR_WIDTH;
    }
    width.clamp(MIN_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH)
}

/// Marker payload for the sidebar-width divider drag (`plans` 第16波 #1) --
/// unlike `task_workspace::DividerDragPayload` there is only one sidebar
/// divider app-wide, so no fields are needed to identify which drag this
/// is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SidebarDividerDragPayload;

/// The (deliberately invisible) drag preview for the sidebar-width divider
/// -- same reasoning as `task_workspace::DividerDragPreview`: the sidebar
/// itself resizing live, underneath the cursor, already *is* the drag's
/// visual feedback.
pub struct SidebarDividerDragPreview;

impl Render for SidebarDividerDragPreview {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// The draggable divider at the sidebar's right edge (`plans` 第16波 #1): an
/// absolutely-positioned handle at `left: sidebar_width px`, shifted back by
/// half `task_workspace::DIVIDER_HIT_WIDTH` so it straddles the boundary --
/// mirrors that module's tile-divider math, just in a raw pixel offset
/// rather than a `ratio * 100%` (the sidebar's own width *is* the value
/// being dragged, no container-relative ratio involved). Reuses that
/// module's hit-width/hover-highlight constants so both dividers look and
/// feel identical.
///
/// Positioned by the *caller* (`app::LaboLaboApp::render`'s `content_row`,
/// which must be `.relative()` for this `.absolute()` child to anchor
/// against), and its `on_drag_move`/`on_drop` are wired there too, not
/// here -- mirrors `task_workspace::render_tile`'s own split between "the
/// handle" (drag source + cursor/hover styling) and "the container that
/// measures the drag" (`event.bounds`, see `app::LaboLaboApp::
/// update_sidebar_width_drag`'s doc comment for why `content_row` itself,
/// not this handle, must be the container the drag is registered on).
pub fn render_sidebar_divider(sidebar_width: f32) -> impl IntoElement {
    div()
        .id("sidebar-divider")
        .absolute()
        .top_0()
        .bottom_0()
        .left(px(sidebar_width))
        .ml(px(-(task_workspace::DIVIDER_HIT_WIDTH / 2.0)))
        .w(px(task_workspace::DIVIDER_HIT_WIDTH))
        .cursor(CursorStyle::ResizeLeftRight)
        .hover(|el| el.bg(rgba(task_workspace::DIVIDER_HOVER_COLOR)))
        .on_drag(
            SidebarDividerDragPayload,
            |_payload, _offset, _window, cx| cx.new(|_cx| SidebarDividerDragPreview),
        )
}

/// One repo's Tasks, in the order they were encountered in the input slice.
pub struct RepoGroup<'a> {
    pub repo_key: &'a str,
    pub repo_name: &'a str,
    pub tasks: Vec<&'a Task>,
}

/// Groups `tasks` by `repo_key`, preserving first-seen group order and
/// each group's internal task order (no sorting) -- callers pass Tasks
/// already ordered by `sort_order` (`TaskDatabase::all_tasks`'s contract),
/// so both the group order and each group's task order end up following
/// `sort_order` too, with no separate sort pass needed here.
pub fn group_tasks_by_repo(tasks: &[Task]) -> Vec<RepoGroup<'_>> {
    let mut groups: Vec<RepoGroup<'_>> = Vec::new();
    for task in tasks {
        if let Some(group) = groups.iter_mut().find(|g| g.repo_key == task.repo_key) {
            group.tasks.push(task);
        } else {
            groups.push(RepoGroup {
                repo_key: &task.repo_key,
                repo_name: &task.repo_name,
                tasks: vec![task],
            });
        }
    }
    groups
}

/// A small marker distinguishing a worktree Task from an attached-directory
/// Task in the sidebar row -- the wave's "title + kind の別が分かる程度の
/// 最小表示" bar, nothing more. Worktree gets the branch icon (第13波b §2,
/// `crate::icons::Icon::Branch` -- previously the "⎇-ish" `\u{2387}`
/// glyph); attached keeps its plain filled dot ("in place"), now drawn as a
/// `div` circle rather than a Unicode `\u{25CF}` glyph so both arms render
/// via the same explicit-`text_color` mechanism instead of mixing a font
/// glyph with an SVG icon. `color` is passed in (rather than relying on the
/// caller's ambient `.text_color(..)`) so this marker tracks the same
/// missing/normal dimming its two call sites already apply to the rest of
/// their row.
fn kind_marker(kind: &TaskKind, color: u32) -> gpui::AnyElement {
    match kind {
        TaskKind::Worktree { .. } => {
            icons::icon_colored(Icon::Branch, 11.0, color).into_any_element()
        }
        TaskKind::Attached { .. } => div()
            .w(px(5.0))
            .h(px(5.0))
            .flex_shrink_0()
            .rounded_full()
            .bg(gpui::rgb(color))
            .into_any_element(),
    }
}

/// A small square icon button + native tooltip -- the sidebar's "start a
/// new Task" affordances (`plans/012` §1's "同じダイアログの選択肢").
///
/// Previously two text-labeled buttons side by side ("+ Attached"/
/// "+ Worktree"); once translated to Japanese ("+ 既存フォルダ"/"+ 新規
/// worktree") they no longer fit [`MIN_SIDEBAR_WIDTH`] (reported
/// on-device: the row overflowed the sidebar). Rather than a single
/// full-width button opening a 2-choice overlay (`settings.rs`'s modal
/// pattern, also considered), this keeps both actions a single click away
/// as compact icon buttons, with the full label carried by gpui 0.2's own
/// `Div::tooltip` (a real API, not hand-rolled -- confirmed in
/// `elements/div.rs`: ~500ms show delay, auto-positioning,
/// dismiss-on-scroll/click, all built in) instead of squeezing text onto
/// the button face. `content` is `crate::icons`-drawn (第13波b §2 -- no
/// emoji, project policy) -- a single icon for the plain "+" button, or
/// [`plus_branch_icon`]'s composed pair for the worktree one.
///
/// Not implemented: the "2 個目以降は遅延なしで即表示" toolbar convention
/// (hovering a second nearby tooltip skips the delay) -- gpui 0.2's
/// `Div::tooltip` applies its ~500ms delay per element with no exposed hook
/// to shortcut it based on a just-dismissed sibling tooltip, and building
/// that ourselves (tracking "was *any* tooltip visible in the last N ms"
/// app-wide) was judged not worth it for two adjacent buttons.
fn icon_button(
    id: &'static str,
    content: impl IntoElement,
    tooltip_text: String,
    on_click: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let tooltip_text: SharedString = tooltip_text.into();
    div()
        .id(id)
        .w(px(28.0))
        .h(px(28.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded_sm()
        .bg(rgb(theme::surface::RAISED))
        .hover(|el| el.bg(rgb(theme::surface::ACTIVE)))
        .active(|el| el.opacity(0.8))
        .tooltip(move |_window, cx| cx.new(|_| IconTooltip(tooltip_text.clone())).into())
        .on_mouse_down(MouseButton::Left, on_click)
        .child(content)
}

/// A small "+" next to a smaller branch glyph, side by side -- the
/// "new worktree Task" button's icon (previously the single combined glyph
/// `"+\u{2387}"`). Two `crate::icons` icons rather than one hand-drawn
/// composite SVG, since the icon set has no "plus-with-a-branch" glyph of
/// its own and this reads just as clearly at 28px.
fn plus_branch_icon() -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .child(icons::icon_colored(Icon::Plus, 12.0, theme::text::PRIMARY))
        .child(icons::icon_colored(
            Icon::Branch,
            11.0,
            theme::text::PRIMARY,
        ))
}

/// タスク行の高さ(第8波a §3/第10波 §2 から値そのものは変更なし)。行の
/// 見せ方を「統合ドット→PR バッジ→タイトル→(右端)使用量」に整理した
/// 第16波 #4 の指示("後からスクリーンショットで微調整できるよう定数化")
/// に従い、以前はここへのリテラル埋め込みだった値を named const に昇格
/// させてある。
const TASK_ROW_HEIGHT: f32 = 30.0;

/// PR バッジ(`plans` 第16波 #3)の寸法 -- 行の高さに収まる小さなピル。
/// [`TASK_ROW_HEIGHT`]同様、後から実 UI のスクリーンショットを見て
/// 微調整できるよう定数化してある(第16波 #4)。
const PR_BADGE_HEIGHT: f32 = 16.0;
/// バッジ内の状態ドット径。
const PR_BADGE_DOT_SIZE: f32 = 6.0;

/// A Task row's PR badge (`plans` 第16波 #3): `#<number>` + a small dot in
/// `pr_status::badge_color(info.state)` (draft=灰/open=緑/merged=紫/
/// closed=赤, GitHub's own dark-theme state colors -- `theme::pr`). The
/// sidebar already groups by `owner/repo`, so the badge itself only needs
/// the bare number (`plans/012`'s per-repo grouping already answers "which
/// repo"); the tooltip spells out the full `owner/repo#number title` for
/// when that context isn't visually obvious (a scrolled-away group header,
/// or just wanting the title without opening the PR). Clicking opens the
/// PR page in the browser (`LaboLaboApp::open_task_pr_page`, same
/// background-`open`-process contract as `crate::ide_open`'s "IDE で開く").
fn pr_badge_element(
    task_id: &str,
    repo_name: &str,
    info: &pr_status::PrInfo,
    cx: &mut Context<LaboLaboApp>,
) -> impl IntoElement {
    let color = pr_status::badge_color(info.state);
    let label: SharedString = format!("#{}", info.number).into();
    let tooltip: SharedString = format!("{repo_name}#{} {}", info.number, info.title).into();
    let badge_id: SharedString = format!("pr-badge-{task_id}").into();
    let open_task_id = task_id.to_string();
    div()
        .id(badge_id)
        .flex()
        .flex_row()
        .items_center()
        .gap_1()
        .px_1()
        .h(px(PR_BADGE_HEIGHT))
        .flex_shrink_0()
        .rounded_sm()
        .bg(rgba(theme::with_alpha(color, 0x22)))
        .text_color(rgb(color))
        .text_size(px(theme::font_size::CAPTION))
        .tooltip(move |_window, cx| cx.new(|_| IconTooltip(tooltip.clone())).into())
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _: &MouseDownEvent, _window, cx| {
                // 行本体の on_mouse_down（タスク選択）まで届かせない --
                // `menu_button`と同じ意図。
                cx.stop_propagation();
                this.open_task_pr_page(&open_task_id, cx);
            }),
        )
        .child(
            div()
                .w(px(PR_BADGE_DOT_SIZE))
                .h(px(PR_BADGE_DOT_SIZE))
                .rounded_full()
                .flex_shrink_0()
                .bg(rgb(color)),
        )
        .child(label)
}

pub fn render(
    app: &LaboLaboApp,
    breathing_enabled: bool,
    cx: &mut Context<LaboLaboApp>,
) -> impl IntoElement {
    let groups = group_tasks_by_repo(app.tasks());
    let selected = app.selected_task_id().map(str::to_string);
    let home = crate::path_abbrev::os_home();

    let mut list = div().flex().flex_col().gap_2().flex_1();
    for group in groups {
        // `plans` 第8波a §3: リポジトリ見出しを「小見出し」スタイルに --
        // 11px(`font_size::CAPTION`)・MUTED(`SECONDARY` より一段沈んだ
        // 明度)。字間を広めにする指示もあるが、gpui 0.2 の `TextStyle` に
        // letter-spacing 相当のフィールドが無い(`style.rs`の`TextStyle`
        // 確認済み)ため、疑似的な空白差し込みのような hack はせずここでは
        // 見送っている -- 未実装の既知の逸脱として明記。
        //
        // 第10波 §3: git 管理外のディレクトリ直付けでは `repo_name` が
        // フルパスになる(`new_task::resolve_attached_repo` のフォール
        // バック)ので、見出しは省略表示 (`path_abbrev`) にし、省略した
        // ときだけフルパスをツールチップで補う。
        let full_name = group.repo_name.to_string();
        let display_name = crate::path_abbrev::abbreviate_if_path(&full_name, home.as_deref());
        let abbreviated = display_name != full_name;
        let header_id: SharedString = format!("repo-group-{}", group.repo_key).into();
        let full_name_tooltip: SharedString = full_name.into();
        let mut header = div()
            .id(header_id)
            .text_color(rgb(theme::text::MUTED))
            .text_size(px(theme::font_size::CAPTION))
            .px_2()
            .child(SharedString::from(display_name));
        if abbreviated {
            header = header.tooltip(move |_window, cx| {
                cx.new(|_| IconTooltip(full_name_tooltip.clone())).into()
            });
        }
        let mut group_el = div().flex().flex_col().gap_1().child(header);
        for task in group.tasks {
            let is_selected = selected.as_deref() == Some(task.id.as_str());
            let status = app.task_agent_status(&task.id);
            let status_color = status.and_then(status_dot_color);
            let is_running = status == Some(AgentStatus::Running);
            // 第10波のカスタム色 (`Task::color`) -- 第16波 #2 で状態ドットと
            // 統合する(このすぐ下の `dot_el` 参照)ため、行の左バーより先に
            // 計算しておく。
            let custom_color = task
                .color
                .as_deref()
                .and_then(crate::color_picker::parse_hex_rgb);
            // 統合ドット (`plans` 第16波 #2): 外輪=カスタム色・中の塗り=
            // 状態色。以前は「状態ドット」と「カスタム色ドット」の 2 個を
            // 別々に描いていたが、`motion::unified_dot_element` に一本化 --
            // 詳細はその doc コメント参照。
            let dot_el = app.task_dot_anim(&task.id).and_then(|anim| {
                motion::unified_dot_element(
                    format!("sidebar-dot-{}", task.id),
                    status_color,
                    custom_color,
                    is_running,
                    breathing_enabled,
                    anim,
                )
            });
            // PR 状態バッジ (`plans` 第16波 #3): worktree タスクだけブランチ
            // が分かるので対象。`gh` 未導入/未認証や PR 未作成は静かに
            // バッジ非表示 -- `LaboLaboApp::task_pr_info`のキャッシュは
            // "取得済みで PR なし"/"未取得"のどちらも`None`に潰れるので、
            // ここでの分岐に違いは出ない(バッジを出さないだけ)。
            let pr_info = app.task_pr_info(&task.id).cloned();
            // 使用量 (第16波 #4: 行右端に集約表示) -- タブチップ側の
            // `format_usage_compact` をそのまま再利用し、複数タブぶんを
            // 合算した `task_agent_usage` を渡す。
            let usage_label: Option<SharedString> = app
                .task_agent_usage(&task.id)
                .and_then(|usage| task_workspace::format_usage_compact(&usage))
                .map(SharedString::from);
            // Cross-session conflict warning (`plans` wave 5i §2): another
            // Task in the same repo has changed one of the same files, per
            // whatever Git status each has cached so far -- see
            // `LaboLaboApp::task_conflicts`'s doc comment for the "only
            // status-fetched Tasks participate" limitation.
            let conflicts = app.task_conflicts(&task.id);
            let has_conflict = !conflicts.is_empty();
            let conflict_tooltip: SharedString = t!(
                "sidebar.conflict_tooltip",
                count = conflicts.len(),
                paths = conflicts
                    .iter()
                    .map(|c| c.path.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
            .to_string()
            .into();
            let conflict_badge_id: SharedString = format!("conflict-badge-{}", task.id).into();
            // 見つからないワークツリー (第8波c): 作業ディレクトリが最後の
            // 確認時点で存在しなかったタスク行は減光 + 専用の警告アイコン
            // （既存の ⚠ = 競合バッジと区別するため別のグリフ ∅ を使う）。
            let missing = app.is_task_missing(&task.id);
            let missing_tooltip: SharedString = t!(
                "sidebar.missing_task_tooltip",
                path = task.working_directory()
            )
            .to_string()
            .into();
            let missing_badge_id: SharedString = format!("missing-badge-{}", task.id).into();
            let row_id: SharedString = format!("task-row-{}", task.id).into();
            // 行ホバーで「…」メニューボタンを出すための group (wave 6c §2)。
            // gpui 0.2 に visible/invisible ヘルパは無いので、opacity 0 で
            // 置いておき `group_hover` で 1.0 へ -- ボタンにポインタが載る
            // には行に載っている必要があるため、不可視のままクリックされる
            // ことはない。
            let row_group: SharedString = format!("task-row-group-{}", task.id).into();
            let drag_task_id = task.id.clone();
            let drag_repo_key = task.repo_key.clone();
            let drag_title: SharedString = task.title.clone().into();
            let drop_before_id = task.id.clone();
            let click_task_id = task.id.clone();
            let menu_task_id = task.id.clone();
            let menu_button_id: SharedString = format!("task-menu-btn-{}", task.id).into();
            let menu_button = div()
                .id(menu_button_id)
                .w(px(20.0))
                .h(px(20.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded_sm()
                .opacity(0.0)
                .group_hover(row_group.clone(), |el| el.opacity(1.0))
                .hover(|el| el.bg(rgb(theme::surface::ACTIVE)))
                .active(|el| el.opacity(0.8))
                .tooltip(move |_window, cx| {
                    cx.new(|_| {
                        IconTooltip(t!("sidebar.task_menu_button_tooltip").to_string().into())
                    })
                    .into()
                })
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, _window, cx| {
                        // 行本体の on_mouse_down（タスク選択）まで届かせない。
                        cx.stop_propagation();
                        this.open_task_menu(&menu_task_id, event.position, cx);
                    }),
                )
                .child(icons::icon_colored(
                    Icon::More,
                    12.0,
                    theme::text::SECONDARY,
                ));

            // PR 状態バッジ (`plans` 第16波 #3) -- `menu_button` と同じく
            // `cx.listener` を使うのでここで(行の `.child()` 連鎖に入る前に)
            // 組み立てておく。`pr_info` が `None`(未取得/gh 不在/PR 無し)
            // なら静かに `None` -- バッジ自体を描かない。
            let pr_badge_el =
                pr_info.map(|info| pr_badge_element(&task.id, &task.repo_name, &info, cx));

            // タイトル: 選択中はセミボールド (第10波 §2)。attached 型は
            // フルパスをツールチップで補う (§3 -- 行のタイトルはディレクトリ
            // 名だけなので、どのディレクトリかの完全な答えはここに残す)。
            let attached_dir: Option<SharedString> = match &task.kind {
                TaskKind::Attached { directory } => Some(SharedString::from(directory.clone())),
                TaskKind::Worktree { .. } => None,
            };
            let title_id: SharedString = format!("task-title-{}", task.id).into();
            let mut title_el = div()
                .id(title_id)
                .flex_1()
                .overflow_hidden()
                .when(is_selected, |el| el.font_weight(FontWeight::SEMIBOLD))
                .child(SharedString::from(task.title.clone()));
            if let Some(dir) = attached_dir {
                title_el = title_el
                    .tooltip(move |_window, cx| cx.new(|_| IconTooltip(dir.clone())).into());
            }

            // `plans` 第8波a §3 + 第10波 §2: タスク行は高さ 30px・角丸 6px。
            // 選択時は BRAND(ライム)の低アルファ背景 + 左 2px バー --
            // フォーカスペインの ACCENT(青)とは意図的に別色
            // (`theme::ACCENT` の doc コメントの使い分け参照)。バーは
            // 非選択時も同じ 2px 幅を確保しておく(選択時だけ幅を足すと
            // 行の横幅が 1px 分ガタつくため)。カスタム色があるときは
            // 左バーをカスタム色が選択状態より優先する -- 「どのタスクか」
            // の識別色は選択中でも変えず、選択は背景のライムで示す
            // (識別と選択の両立)。
            let row = div()
                .id(row_id)
                .group(row_group)
                .flex()
                .flex_row()
                .items_center()
                .gap_1()
                .h(px(TASK_ROW_HEIGHT))
                .px_2()
                .rounded(px(theme::radius::ROW))
                .border_l_2()
                .border_color(rgb(match (custom_color, is_selected) {
                    (Some(color), _) => color,
                    (None, true) => theme::BRAND,
                    (None, false) => theme::surface::SUNKEN,
                }))
                .when(is_selected, |el| {
                    el.bg(rgba(theme::with_alpha(theme::BRAND, 0x1a)))
                })
                // Hover feedback (`plans/014` M5, scoped down -- see that
                // plan's doc comment for why this is an instant `.hover()`
                // tint rather than an eased transition) only applies to
                // unselected rows, so hovering a selected row can never
                // read as "losing the selection".
                .when(!is_selected, |el| {
                    el.hover(|el| el.bg(rgb(theme::surface::RAISED)))
                })
                // 見つからないタスクは行全体を減光 (`text::MUTED`) する --
                // タイトルだけでなくアイコン (`kind_marker`) も含めて全体
                // が「今は使えない」と読めるように。
                .text_color(rgb(if missing {
                    theme::text::MUTED
                } else {
                    theme::text::PRIMARY
                }))
                .text_size(px(theme::font_size::LABEL))
                .child(kind_marker(
                    &task.kind,
                    if missing {
                        theme::text::MUTED
                    } else {
                        theme::text::PRIMARY
                    },
                ))
                // 統合ドット (`plans` 第16波 #2): 外輪=カスタム色/中の塗り=
                // 状態色を 1 個で表現する(以前の「カスタム色 6px ドット」+
                // 「状態ドット」の 2 個表示はここで統合・廃止)。
                // `motion::DOT_RING_SIZE` 固定の枠に中央揃え -- 輪の有無で
                // 行内の占有幅がガタつかないよう、輪なしの行でも同じ枠を
                // 確保する。
                .children(dot_el.map(|el| {
                    div()
                        .w(px(motion::DOT_RING_SIZE))
                        .h(px(motion::DOT_RING_SIZE))
                        .flex()
                        .items_center()
                        .justify_center()
                        .flex_shrink_0()
                        .child(el)
                }))
                // PR 状態バッジ (`plans` 第16波 #3): ドットのすぐ右、タイトル
                // より前 -- 「行の左に状態・PR が凝縮される」情報密度の高い
                // 並び (第16波 #4)。
                .children(pr_badge_el)
                .child(title_el)
                .when(missing, move |el| {
                    el.child(
                        div()
                            .id(missing_badge_id)
                            .tooltip(move |_window, cx| {
                                cx.new(|_| IconTooltip(missing_tooltip.clone())).into()
                            })
                            .child(icons::icon_colored(
                                Icon::NotFound,
                                12.0,
                                theme::status::CONFLICT,
                            )),
                    )
                })
                .when(has_conflict, move |el| {
                    el.child(
                        div()
                            .id(conflict_badge_id)
                            .tooltip(move |_window, cx| {
                                cx.new(|_| IconTooltip(conflict_tooltip.clone())).into()
                            })
                            .child(icons::icon_colored(
                                Icon::Warning,
                                12.0,
                                theme::status::CONFLICT,
                            )),
                    )
                })
                // 使用量 (第16波 #4: 行右端に集約表示、タブチップと同じ
                // `format_usage_compact` の書式)。
                .when_some(usage_label, |el, label| {
                    el.child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(theme::font_size::CAPTION))
                            .text_color(rgb(theme::text::SECONDARY))
                            .child(label),
                    )
                })
                .child(menu_button)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                        this.select_task(click_task_id.clone(), window, cx);
                    }),
                )
                // Drag source: `plans/012` §3's "サイドバーの作業（Task）
                // 並び替え". `.id(..)` above promotes this row to
                // `Stateful<Div>`, required for `.on_drag`.
                .on_drag(
                    TaskDragPayload {
                        task_id: drag_task_id,
                        repo_key: drag_repo_key.clone(),
                    },
                    move |_payload, _offset, _window, cx| {
                        cx.new(|_cx| TaskDragPreview(drag_title.clone()))
                    },
                )
                // Drop target: dropping another row here means "insert the
                // dragged Task just before this one" -- rejected (via
                // `can_drop`) unless the dragged Task is in the same repo
                // group, matching `reorder_task_ids`'s own no-op rule for a
                // cross-repo `before_id`.
                .can_drop(move |any, _window, _cx| {
                    any.downcast_ref::<TaskDragPayload>()
                        .map(|payload| payload.repo_key == drag_repo_key)
                        .unwrap_or(true)
                })
                .drag_over::<TaskDragPayload>(|style, _payload, _window, _cx| {
                    style.bg(rgba(TASK_ROW_DROP_HIGHLIGHT_COLOR))
                })
                .on_drop::<TaskDragPayload>(cx.listener(
                    move |this, payload: &TaskDragPayload, _window, cx| {
                        this.reorder_tasks_in_sidebar(
                            payload.task_id.clone(),
                            Some(drop_before_id.clone()),
                            cx,
                        );
                    },
                ));
            group_el = group_el.child(row);
        }
        list = list.child(group_el);
    }

    let new_task_row = div()
        .flex()
        .flex_row()
        .gap_1()
        .px_2()
        .py_1()
        .child(icon_button(
            "new-attached-task",
            icons::icon_colored(Icon::Plus, 14.0, theme::text::PRIMARY).into_any_element(),
            t!("sidebar.new_attached_task_tooltip").to_string(),
            cx.listener(|this, _: &MouseDownEvent, window, cx| {
                this.start_new_attached_task(window, cx);
            }),
        ))
        .child(icon_button(
            "new-worktree-task",
            plus_branch_icon().into_any_element(),
            t!("sidebar.new_worktree_task_tooltip").to_string(),
            cx.listener(|this, _: &MouseDownEvent, window, cx| {
                this.start_new_worktree_task(window, cx);
            }),
        ));

    // アップデート確認バナー (`render_update_banner`, RC release wave) と
    // Swift 版インポータのバナー (`render_import_banner`): "+" 行より上、
    // サイドバー最上部に出す。`app`/`cx` の再借用を避けるため `sidebar`
    // 本体の構築より先に評価する。
    let update_banner = render_update_banner(app, cx);
    let import_banner = render_import_banner(app, cx);
    // 見つからないワークツリーの気づきバナー (第8波c §5): 他の 2 つより
    // 後（サイドバー最上部から見て 3 番目）に出す -- アップデート/
    // インポートは「アプリからのお知らせ」、こちらは「ユーザーの環境側の
    // 状態」で性質が異なるため、既存 2 つの並び順を崩さず一番下に足す。
    let missing_tasks_banner = render_missing_tasks_banner(app, cx);

    let mut sidebar = div()
        .flex()
        .flex_col()
        .w(px(app.sidebar_width()))
        .h_full()
        .bg(rgb(theme::surface::SUNKEN))
        .border_1()
        .border_color(rgb(theme::surface::STROKE))
        // `plans` 第8波a §1: 「深度の階層」-- ウィンドウ左端に接する側は
        // 直角のまま、ワークスペースに面した右側だけ丸めて「浮いている
        // カード」を演出する。控えめな 1 段シャドウを右向きに落とす。
        .rounded_r(px(theme::radius::PANEL))
        .shadow(theme::shadow::panel(2.0, 0.0))
        // OS folder drop -> new attached Task (`plans/012` §3): any
        // `ExternalPaths` dropped anywhere on the sidebar (including on
        // top of a Task row -- rows have no `on_drop::<ExternalPaths>` of
        // their own, so the event reaches this container) is handed to
        // `handle_sidebar_folder_drop`, which filters to directories.
        .drag_over::<ExternalPaths>(|style, _paths, _window, _cx| {
            style.bg(rgba(SIDEBAR_FOLDER_DROP_HIGHLIGHT_COLOR))
        })
        .on_drop::<ExternalPaths>(cx.listener(|this, paths: &ExternalPaths, _window, cx| {
            this.handle_sidebar_folder_drop(paths, cx);
        }))
        .children(update_banner)
        .children(import_banner)
        .children(missing_tasks_banner)
        .child(new_task_row)
        .child(list);

    // アーカイブ済みセクション (wave 6c §2): サイドバー下部（`list` が
    // flex_1 なので自然に底へ寄る）の折りたたみ見出し + 展開時の「復元」
    // 付き行。既定は折りたたみ（非永続 -- `LaboLaboApp::archived_expanded`）。
    if let Some(section) = render_archived_section(app, cx) {
        sidebar = sidebar.child(section);
    }

    if let Some(error) = app.new_task_error() {
        sidebar = sidebar.child(
            div()
                .px_2()
                .py_1()
                .text_color(rgb(theme::status::CONFLICT))
                .text_size(px(theme::font_size::CAPTION))
                .child(SharedString::from(error.to_string())),
        );
    }

    sidebar
}

/// A boxed mouse-down handler, matching `Div::on_mouse_down`'s own bound --
/// named so [`BannerAction::on_click`]/[`banner`]'s `on_body_click` don't
/// spell the whole trait object out inline (clippy's `type_complexity`
/// lint).
type ClickHandler = Box<dyn Fn(&MouseDownEvent, &mut Window, &mut App) + 'static>;

/// A secondary text action rendered between the banner body and its close
/// button (e.g. update's "開く") -- see [`banner`].
struct BannerAction {
    id: &'static str,
    label: SharedString,
    on_click: ClickHandler,
}

/// Shared chrome for the sidebar's "app/environment has something to tell
/// you" banners (第13波b §4 -- banner unification): a leading tinted icon,
/// the message (optionally itself clickable via `on_body_click`), an
/// optional secondary text action, and an icon close button -- one visual
/// style, previously three independent hand-rolled `flex` rows in
/// [`render_update_banner`]/[`render_import_banner`]/
/// [`render_missing_tasks_banner`] that agreed on spacing/colors by
/// convention rather than by construction. The leading icon also gives each
/// banner a wordless "what kind of banner is this" cue at a glance:
/// [`Icon::Info`] for the two "the app is telling you something" banners
/// (update/import), [`Icon::Warning`] for the one "your environment needs
/// attention" banner (missing worktrees) -- reusing the same warning tone
/// [`Icon::Warning`] already carries in the Task-row conflict badge above.
#[allow(clippy::too_many_arguments)]
fn banner(
    icon: Icon,
    icon_color: u32,
    body_id: &'static str,
    message: SharedString,
    on_body_click: Option<ClickHandler>,
    action: Option<BannerAction>,
    dismiss_id: &'static str,
    on_dismiss: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let mut body = div()
        .id(body_id)
        .flex_1()
        .rounded_sm()
        .text_color(rgb(theme::text::SECONDARY))
        .text_size(px(theme::font_size::CAPTION))
        .child(message);
    if let Some(on_click) = on_body_click {
        body = body
            .hover(|el| el.bg(rgb(theme::surface::ACTIVE)))
            .on_mouse_down(MouseButton::Left, on_click);
    }

    let mut row = div()
        .flex()
        .items_center()
        .gap_2()
        .px_2()
        .py_1()
        .bg(rgb(theme::surface::RAISED))
        .border_b_1()
        .border_color(rgb(theme::surface::STROKE))
        .child(icons::icon_colored(icon, 13.0, icon_color))
        .child(body);

    if let Some(action) = action {
        row = row.child(
            div()
                .id(action.id)
                .px_1()
                .rounded_sm()
                .text_color(rgb(theme::text::SECONDARY))
                .text_size(px(theme::font_size::CAPTION))
                .hover(|el| el.bg(rgb(theme::surface::ACTIVE)))
                .on_mouse_down(MouseButton::Left, action.on_click)
                .child(action.label),
        );
    }

    row.child(
        div()
            .id(dismiss_id)
            .px_1()
            .rounded_sm()
            .hover(|el| el.bg(rgb(theme::surface::ACTIVE)))
            .on_mouse_down(MouseButton::Left, on_dismiss)
            .child(icons::icon_colored(Icon::Close, 10.0, theme::text::MUTED)),
    )
}

/// アップデート確認 (`crate::update_check`, RC release wave) の結果一行
/// バナー -- サイドバー最上部（Swift 版インポータのバナーより上）に出す。
/// `render_import_banner` と同じ「閉じるまで残る」動線だが、"開く" ボタンが
/// 追加で付く点が異なる（リリースページを既定ブラウザで開く、
/// `LaboLaboApp::open_update_release_page`）。閉じるアイコンは
/// `LaboLaboApp::dismiss_update_banner` -- 閉じると同時に「このバージョン
/// を通知しない」を appState へ永続化する（同メソッドの doc コメント参照）。
/// `app.update_banner()` が `None` なら何も描画しない。
fn render_update_banner(
    app: &LaboLaboApp,
    cx: &mut Context<LaboLaboApp>,
) -> Option<gpui::AnyElement> {
    let release = app.update_banner()?.clone();
    let message = t!(
        "update.available.message",
        version = release.version.as_str()
    )
    .to_string();
    Some(
        banner(
            Icon::Info,
            theme::text::SECONDARY,
            "update-banner-text",
            message.into(),
            None,
            Some(BannerAction {
                id: "update-banner-open",
                label: t!("update.available.open").to_string().into(),
                on_click: Box::new(cx.listener(|this, _: &MouseDownEvent, _window, cx| {
                    this.open_update_release_page(cx);
                })),
            }),
            "update-banner-dismiss",
            cx.listener(|this, _: &MouseDownEvent, _window, cx| {
                this.dismiss_update_banner(cx);
            }),
        )
        .into_any_element(),
    )
}

/// Swift 版インポータ (`crate::swift_import`, `plans` W6e) の結果一行バナー
/// -- サイドバー最上部（新規作業の "+" 行より上）に出す。`new_task_error`
/// と違い、閉じるアイコンで明示的に消すまで残る（`LaboLaboApp::
/// dismiss_import_banner`）。`app.import_banner()` が `None` なら何も描画
/// しない。
fn render_import_banner(
    app: &LaboLaboApp,
    cx: &mut Context<LaboLaboApp>,
) -> Option<gpui::AnyElement> {
    let text = app.import_banner()?.to_string();
    Some(
        banner(
            Icon::Info,
            theme::text::SECONDARY,
            "import-banner-text",
            text.into(),
            None,
            None,
            "import-banner-dismiss",
            cx.listener(|this, _: &MouseDownEvent, _window, cx| {
                this.dismiss_import_banner(cx);
            }),
        )
        .into_any_element(),
    )
}

/// 見つからないワークツリーの気づきバナー (第8波c §5)。
/// `app.missing_task_count()` が 0、あるいは既に閉じられている
/// (`missing_banner_dismissed`) なら `None`（見出しごと出さない）。本文の
/// クリックでサイドバー順で最初の該当タスクへジャンプし
/// (`jump_to_first_missing_task`)、閉じるアイコンは
/// `dismiss_missing_tasks_banner`。自動一括削除はしない -- あくまで
/// 気づかせるだけ（設計方針、モジュール冒頭のタスク doc コメント参照）。
fn render_missing_tasks_banner(
    app: &LaboLaboApp,
    cx: &mut Context<LaboLaboApp>,
) -> Option<gpui::AnyElement> {
    if app.missing_banner_dismissed() {
        return None;
    }
    let count = app.missing_task_count();
    if count == 0 {
        return None;
    }
    let text = t!("sidebar.missing_tasks_banner", count = count).to_string();
    Some(
        banner(
            Icon::Warning,
            theme::status::CONFLICT,
            "missing-tasks-banner-text",
            text.into(),
            Some(Box::new(cx.listener(
                |this, _: &MouseDownEvent, window, cx| {
                    this.jump_to_first_missing_task(window, cx);
                },
            ))),
            None,
            "missing-tasks-banner-dismiss",
            cx.listener(|this, _: &MouseDownEvent, _window, cx| {
                this.dismiss_missing_tasks_banner(cx);
            }),
        )
        .into_any_element(),
    )
}

/// 「アーカイブ済み (n)」セクション。アーカイブが 1 件も無ければ `None`
/// （見出しごと出さない）。
fn render_archived_section(
    app: &LaboLaboApp,
    cx: &mut Context<LaboLaboApp>,
) -> Option<gpui::AnyElement> {
    let archived = app.archived_tasks();
    if archived.is_empty() {
        return None;
    }
    let expanded = app.archived_expanded();

    let header = div()
        .id("archived-section-toggle")
        .flex()
        .flex_row()
        .items_center()
        .gap_1()
        .px_2()
        .py_1()
        .rounded_sm()
        .text_color(rgb(theme::text::SECONDARY))
        .text_size(px(theme::font_size::CAPTION))
        .hover(|el| el.bg(rgb(theme::surface::RAISED)))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _: &MouseDownEvent, _window, cx| {
                this.toggle_archived_section(cx);
            }),
        )
        .child(icons::chevron_element(10.0, expanded).text_color(rgb(theme::text::SECONDARY)))
        .child(SharedString::from(
            t!("sidebar.archived_section_title", count = archived.len()).to_string(),
        ));

    let mut section = div()
        .flex()
        .flex_col()
        .gap_1()
        .py_1()
        .border_t_1()
        .border_color(rgb(theme::surface::STROKE))
        .child(header);

    if expanded {
        for task in archived {
            let restore_id = task.id.clone();
            let row_id: SharedString = format!("archived-row-{}", task.id).into();
            let button_id: SharedString = format!("archived-restore-{}", task.id).into();
            section = section.child(
                div()
                    .id(row_id)
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .h(px(30.0))
                    .px_2()
                    .rounded(px(theme::radius::ROW))
                    .text_color(rgb(theme::text::MUTED))
                    .text_size(px(theme::font_size::LABEL))
                    .child(kind_marker(&task.kind, theme::text::MUTED))
                    .child(SharedString::from(task.title.clone()))
                    .child(div().flex_1())
                    .child(
                        div()
                            .id(button_id)
                            .px_2()
                            .rounded_sm()
                            .bg(rgb(theme::surface::RAISED))
                            .text_color(rgb(theme::text::SECONDARY))
                            .text_size(px(theme::font_size::CAPTION))
                            .hover(|el| el.bg(rgb(theme::surface::ACTIVE)))
                            .active(|el| el.opacity(0.8))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                                    this.restore_task(&restore_id, window, cx);
                                }),
                            )
                            .child(t!("sidebar.restore").to_string()),
                    ),
            );
        }
    }

    Some(section.into_any_element())
}

#[cfg(test)]
mod tests {
    use super::*;
    use labolabo_core::TileLayout;

    fn task(repo_key: &str, repo_name: &str, title: &str) -> Task {
        let mut t = Task::new_attached(
            repo_key,
            repo_key,
            repo_name,
            format!("/tmp/{title}"),
            TileLayout::default(),
            0,
        );
        t.title = title.to_string();
        t
    }

    // MARK: - clamp_sidebar_width (第16波 #1)

    #[test]
    fn clamp_sidebar_width_leaves_an_in_range_value_untouched() {
        assert_eq!(clamp_sidebar_width(260.0), 260.0);
        assert_eq!(clamp_sidebar_width(MIN_SIDEBAR_WIDTH), MIN_SIDEBAR_WIDTH);
        assert_eq!(clamp_sidebar_width(MAX_SIDEBAR_WIDTH), MAX_SIDEBAR_WIDTH);
    }

    #[test]
    fn clamp_sidebar_width_clamps_below_the_minimum() {
        assert_eq!(clamp_sidebar_width(10.0), MIN_SIDEBAR_WIDTH);
        assert_eq!(clamp_sidebar_width(0.0), MIN_SIDEBAR_WIDTH);
        assert_eq!(clamp_sidebar_width(-50.0), MIN_SIDEBAR_WIDTH);
    }

    #[test]
    fn clamp_sidebar_width_clamps_above_the_maximum() {
        assert_eq!(clamp_sidebar_width(1000.0), MAX_SIDEBAR_WIDTH);
    }

    #[test]
    fn clamp_sidebar_width_non_finite_falls_back_to_the_default() {
        assert_eq!(clamp_sidebar_width(f32::NAN), DEFAULT_SIDEBAR_WIDTH);
        assert_eq!(clamp_sidebar_width(f32::INFINITY), DEFAULT_SIDEBAR_WIDTH);
        assert_eq!(
            clamp_sidebar_width(f32::NEG_INFINITY),
            DEFAULT_SIDEBAR_WIDTH
        );
    }

    #[test]
    fn groups_by_repo_key_preserving_first_seen_order() {
        let tasks = vec![
            task("A", "repoA", "a1"),
            task("B", "repoB", "b1"),
            task("A", "repoA", "a2"),
        ];
        let groups = group_tasks_by_repo(&tasks);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].repo_key, "A");
        assert_eq!(
            groups[0]
                .tasks
                .iter()
                .map(|t| t.title.as_str())
                .collect::<Vec<_>>(),
            vec!["a1", "a2"]
        );
        assert_eq!(groups[1].repo_key, "B");
        assert_eq!(groups[1].tasks.len(), 1);
    }

    #[test]
    fn empty_input_yields_no_groups() {
        assert!(group_tasks_by_repo(&[]).is_empty());
    }

    #[test]
    fn single_repo_yields_single_group_with_all_tasks() {
        let tasks = vec![task("A", "repoA", "a1"), task("A", "repoA", "a2")];
        let groups = group_tasks_by_repo(&tasks);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].tasks.len(), 2);
    }
}
