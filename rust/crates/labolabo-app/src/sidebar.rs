//! Sidebar: a repo-grouped Task list plus the "+" new-Task affordances
//! (`plans/012-task-model-and-control-cli.md` §1: "サイドバー = リポジトリ
//! ごとにグルーピングされた Task 一覧"). [`group_tasks_by_repo`] is pure and
//! gpui-free (unit-tested below) -- the same "logic lives outside gpui"
//! discipline `crate::focus` already established for the tile/tab tree;
//! [`render`] just walks its output and wires up click/drag/drop handlers.
//!
//! Deliberately minimal per this wave's brief ("title + kind の別が分かる
//! 程度の最小表示" / "本格ダイアログは将来"): no icons beyond a one-glyph
//! kind marker. Drag & drop (plan §3) *is* implemented here: dragging a
//! Task row reorders it within its repo group (`LaboLaboApp::
//! reorder_tasks_in_sidebar`, ordering math in `labolabo_core::
//! reorder_task_ids`), and dropping an OS folder anywhere on the sidebar
//! starts a new attached Task there (`LaboLaboApp::
//! handle_sidebar_folder_drop`).
//!
//! Wave 6c adds: 行ホバーの「…」ボタン（`crate::task_menu` のアーカイブ/
//! 削除/IDE で開くメニュー）と、下部の「アーカイブ済み (n)」折りたたみ
//! セクション（[`render_archived_section`]、復元ボタン付き）。

use gpui::{
    div, prelude::*, px, rgb, rgba, App, Context, ExternalPaths, IntoElement, MouseButton,
    MouseDownEvent, Render, SharedString, Window,
};
use rust_i18n::t;

use labolabo_core::{AgentStatus, Task, TaskKind};

use crate::app::LaboLaboApp;
use crate::task_workspace::status_dot_color;
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

/// Fixed sidebar width -- a simple constant is enough for this wave (no
/// resize handle yet, same simplification the tile tree's divider-drag
/// took in wave 5b-2).
pub const SIDEBAR_WIDTH: f32 = 220.0;

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

/// A one-glyph marker distinguishing a worktree Task from an attached-
/// directory Task in the sidebar row -- the wave's "title + kind の別が
/// 分かる程度の最小表示" bar, nothing more.
fn kind_marker(kind: &TaskKind) -> &'static str {
    match kind {
        TaskKind::Worktree { .. } => "\u{2387}", // ⎇-ish branch glyph
        TaskKind::Attached { .. } => "\u{25CF}", // solid dot: "in place"
    }
}

/// A small square icon button + native tooltip -- the sidebar's "start a
/// new Task" affordances (`plans/012` §1's "同じダイアログの選択肢").
///
/// Previously two text-labeled buttons side by side ("+ Attached"/
/// "+ Worktree"); once translated to Japanese ("+ 既存フォルダ"/"+ 新規
/// worktree") they no longer fit [`SIDEBAR_WIDTH`] at its minimum (reported
/// on-device: the row overflowed the sidebar). Rather than a single
/// full-width button opening a 2-choice overlay (`settings.rs`'s modal
/// pattern, also considered), this keeps both actions a single click away
/// as compact icon buttons, using glyphs already established elsewhere in
/// this module ([`kind_marker`]'s "⎇" = worktree) plus a plain "+", with
/// the full label carried by gpui 0.2's own `Div::tooltip` (a real API,
/// not hand-rolled -- confirmed in `elements/div.rs`: ~500ms show delay,
/// auto-positioning, dismiss-on-scroll/click, all built in) instead of
/// squeezing text onto the button face. No emoji (project policy) -- both
/// glyphs are plain Unicode the UI font already renders elsewhere in this
/// same view.
///
/// Not implemented: the "2 個目以降は遅延なしで即表示" toolbar convention
/// (hovering a second nearby tooltip skips the delay) -- gpui 0.2's
/// `Div::tooltip` applies its ~500ms delay per element with no exposed hook
/// to shortcut it based on a just-dismissed sibling tooltip, and building
/// that ourselves (tracking "was *any* tooltip visible in the last N ms"
/// app-wide) was judged not worth it for two adjacent buttons.
fn icon_button(
    id: &'static str,
    glyph: &'static str,
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
        .text_color(rgb(theme::text::PRIMARY))
        .hover(|el| el.bg(rgb(theme::surface::ACTIVE)))
        .active(|el| el.opacity(0.8))
        .tooltip(move |_window, cx| cx.new(|_| IconTooltip(tooltip_text.clone())).into())
        .on_mouse_down(MouseButton::Left, on_click)
        .child(glyph)
}

pub fn render(
    app: &LaboLaboApp,
    breathing_enabled: bool,
    cx: &mut Context<LaboLaboApp>,
) -> impl IntoElement {
    let groups = group_tasks_by_repo(app.tasks());
    let selected = app.selected_task_id().map(str::to_string);

    let mut list = div().flex().flex_col().gap_2().flex_1();
    for group in groups {
        let mut group_el = div().flex().flex_col().gap_1().child(
            div()
                .text_color(rgb(theme::text::SECONDARY))
                .text_size(px(theme::font_size::CAPTION))
                .px_2()
                .child(SharedString::from(group.repo_name.to_string())),
        );
        for task in group.tasks {
            let is_selected = selected.as_deref() == Some(task.id.as_str());
            let status = app.task_agent_status(&task.id);
            let status_color = status.and_then(status_dot_color);
            let is_running = status == Some(AgentStatus::Running);
            let dot_el = app.task_dot_anim(&task.id).and_then(|anim| {
                crate::motion::status_dot_element(
                    format!("sidebar-dot-{}", task.id),
                    status_color,
                    is_running,
                    breathing_enabled,
                    anim,
                )
            });
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
                .text_color(rgb(theme::text::SECONDARY))
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
                .child("\u{22EF}"); // ⋯
            let row = div()
                .id(row_id)
                .group(row_group)
                .flex()
                .flex_row()
                .items_center()
                .gap_1()
                .px_2()
                .py_1()
                .rounded_sm()
                .when(is_selected, |el| el.bg(rgb(theme::surface::ACTIVE)))
                // Hover feedback (`plans/014` M5, scoped down -- see that
                // plan's doc comment for why this is an instant `.hover()`
                // tint rather than an eased transition) only applies to
                // unselected rows, so hovering a selected row can never
                // read as "losing the selection".
                .when(!is_selected, |el| {
                    el.hover(|el| el.bg(rgb(theme::surface::RAISED)))
                })
                .text_color(rgb(theme::text::PRIMARY))
                .text_size(px(theme::font_size::LABEL))
                .child(kind_marker(&task.kind))
                .child(SharedString::from(task.title.clone()))
                .children(dot_el)
                .when(has_conflict, move |el| {
                    el.child(
                        div()
                            .id(conflict_badge_id)
                            .text_size(px(theme::font_size::CAPTION))
                            .text_color(rgb(theme::status::CONFLICT))
                            .tooltip(move |_window, cx| {
                                cx.new(|_| IconTooltip(conflict_tooltip.clone())).into()
                            })
                            .child("\u{26A0}"),
                    )
                })
                .child(div().flex_1())
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
            "+",
            t!("sidebar.new_attached_task_tooltip").to_string(),
            cx.listener(|this, _: &MouseDownEvent, window, cx| {
                this.start_new_attached_task(window, cx);
            }),
        ))
        .child(icon_button(
            "new-worktree-task",
            "+\u{2387}",
            t!("sidebar.new_worktree_task_tooltip").to_string(),
            cx.listener(|this, _: &MouseDownEvent, window, cx| {
                this.start_new_worktree_task(window, cx);
            }),
        ));

    // Swift 版インポータのバナー (`render_import_banner`): "+" 行より上、
    // サイドバー最上部に出す。`app`/`cx` の再借用を避けるため `sidebar`
    // 本体の構築より先に評価する。
    let import_banner = render_import_banner(app, cx);

    let mut sidebar = div()
        .flex()
        .flex_col()
        .w(px(SIDEBAR_WIDTH))
        .h_full()
        .bg(rgb(theme::surface::SUNKEN))
        .border_1()
        .border_color(rgb(theme::surface::STROKE))
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
        .children(import_banner)
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

/// Swift 版インポータ (`crate::swift_import`, `plans` W6e) の結果一行バナー
/// -- サイドバー最上部（新規作業の "+" 行より上）に出す。`new_task_error`
/// と違い、閉じる（"×"）ボタンで明示的に消すまで残る（`LaboLaboApp::
/// dismiss_import_banner`）。`app.import_banner()` が `None` なら何も描画
/// しない。
fn render_import_banner(
    app: &LaboLaboApp,
    cx: &mut Context<LaboLaboApp>,
) -> Option<impl IntoElement> {
    let text = app.import_banner()?.to_string();
    Some(
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .px_2()
            .py_1()
            .bg(rgb(theme::surface::RAISED))
            .border_b_1()
            .border_color(rgb(theme::surface::STROKE))
            .child(
                div()
                    .flex_1()
                    .text_color(rgb(theme::text::SECONDARY))
                    .text_size(px(theme::font_size::CAPTION))
                    .child(SharedString::from(text)),
            )
            .child(
                div()
                    .id("import-banner-dismiss")
                    .px_1()
                    .rounded_sm()
                    .text_color(rgb(theme::text::MUTED))
                    .text_size(px(theme::font_size::CAPTION))
                    .hover(|el| el.bg(rgb(theme::surface::ACTIVE)))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseDownEvent, _window, cx| {
                            this.dismiss_import_banner(cx);
                        }),
                    )
                    .child("×"),
            ),
    )
}

/// 「アーカイブ済み (n)」セクション。アーカイブが 1 件も無ければ `None`
/// （見出しごと出さない）。
fn render_archived_section(
    app: &LaboLaboApp,
    cx: &mut Context<LaboLaboApp>,
) -> Option<impl IntoElement> {
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
        .child(if expanded { "\u{25BE}" } else { "\u{25B8}" }) // ▾ / ▸
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
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .text_color(rgb(theme::text::MUTED))
                    .text_size(px(theme::font_size::LABEL))
                    .child(kind_marker(&task.kind))
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

    Some(section)
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
