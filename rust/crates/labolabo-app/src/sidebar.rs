//! Sidebar: a repo-grouped Task list plus the "+" new-Task affordances
//! (`plans/012-task-model-and-control-cli.md` §1: "サイドバー = リポジトリ
//! ごとにグルーピングされた Task 一覧"). [`group_tasks_by_repo`] is pure and
//! gpui-free (unit-tested below) -- the same "logic lives outside gpui"
//! discipline `crate::focus` already established for the tile/tab tree;
//! [`render`] just walks its output and wires up click/drag/drop handlers.
//!
//! Deliberately minimal per this wave's brief ("title + kind の別が分かる
//! 程度の最小表示" / "本格ダイアログは将来"): no icons beyond a one-glyph
//! kind marker, no rename/done/archive affordances (plan §1 "今回スコープ
//! 外"). Drag & drop (plan §3) *is* implemented here: dragging a Task row
//! reorders it within its repo group (`LaboLaboApp::
//! reorder_tasks_in_sidebar`, ordering math in `labolabo_core::
//! reorder_task_ids`), and dropping an OS folder anywhere on the sidebar
//! starts a new attached Task there (`LaboLaboApp::
//! handle_sidebar_folder_drop`).

use gpui::{
    div, prelude::*, px, rgb, rgba, Context, ExternalPaths, IntoElement, MouseButton,
    MouseDownEvent, Render, SharedString, Window,
};

use labolabo_core::{Task, TaskKind};

use crate::app::LaboLaboApp;
use crate::task_workspace::status_dot_color;

/// Background tint applied to a Task row while a same-repo row is being
/// dragged over it (`.drag_over::<TaskDragPayload>`) -- "drop here" for
/// reordering. A different hue from
/// `task_workspace::MOVE_DROP_HIGHLIGHT_COLOR`/`FILE_DROP_HIGHLIGHT_COLOR`
/// (green rather than blue/amber) simply because it's a different
/// affordance (list reorder, not pane split/merge or file insert) in a
/// different part of the UI -- there's no shared-meaning requirement
/// across the two DnD systems the way §3.1 requires within the
/// terminal-pane one.
const TASK_ROW_DROP_HIGHLIGHT_COLOR: u32 = 0x30d1584d;
/// Background tint applied to the whole sidebar while an OS folder is
/// being dragged over it (`.drag_over::<ExternalPaths>`).
const SIDEBAR_FOLDER_DROP_HIGHLIGHT_COLOR: u32 = 0x30d1582a;

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
            .bg(rgb(0x3a3a3a))
            .text_color(rgb(0xe5e5e5))
            .text_size(px(11.0))
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

pub fn render(app: &LaboLaboApp, cx: &mut Context<LaboLaboApp>) -> impl IntoElement {
    let groups = group_tasks_by_repo(app.tasks());
    let selected = app.selected_task_id().map(str::to_string);

    let mut list = div().flex().flex_col().gap_2().flex_1();
    for group in groups {
        let mut group_el = div().flex().flex_col().gap_1().child(
            div()
                .text_color(rgb(0x8a8a8a))
                .text_size(px(11.0))
                .px_2()
                .child(SharedString::from(group.repo_name.to_string())),
        );
        for task in group.tasks {
            let is_selected = selected.as_deref() == Some(task.id.as_str());
            let status_color = app.task_agent_status(&task.id).and_then(status_dot_color);
            let row_id: SharedString = format!("task-row-{}", task.id).into();
            let drag_task_id = task.id.clone();
            let drag_repo_key = task.repo_key.clone();
            let drag_title: SharedString = task.title.clone().into();
            let drop_before_id = task.id.clone();
            let click_task_id = task.id.clone();
            let row = div()
                .id(row_id)
                .flex()
                .flex_row()
                .items_center()
                .gap_1()
                .px_2()
                .py_1()
                .rounded_sm()
                .when(is_selected, |el| el.bg(rgb(0x3a3a3a)))
                .text_color(rgb(0xe5e5e5))
                .child(kind_marker(&task.kind))
                .child(SharedString::from(task.title.clone()))
                .when_some(status_color, |el, color| {
                    el.child(div().w(px(6.0)).h(px(6.0)).rounded_full().bg(rgb(color)))
                })
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
        .child(
            div()
                .px_2()
                .py_1()
                .rounded_sm()
                .bg(rgb(0x2f2f2f))
                .text_color(rgb(0xe5e5e5))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _: &MouseDownEvent, window, cx| {
                        this.start_new_attached_task(window, cx);
                    }),
                )
                .child("+ Attached"),
        )
        .child(
            div()
                .px_2()
                .py_1()
                .rounded_sm()
                .bg(rgb(0x2f2f2f))
                .text_color(rgb(0xe5e5e5))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _: &MouseDownEvent, window, cx| {
                        this.start_new_worktree_task(window, cx);
                    }),
                )
                .child("+ Worktree"),
        );

    let mut sidebar = div()
        .flex()
        .flex_col()
        .w(px(SIDEBAR_WIDTH))
        .h_full()
        .bg(rgb(0x1a1a1a))
        .border_1()
        .border_color(rgb(0x1c1c1c))
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
        .child(new_task_row)
        .child(list);

    if let Some(error) = app.new_task_error() {
        sidebar = sidebar.child(
            div()
                .px_2()
                .py_1()
                .text_color(rgb(0xff6b6b))
                .text_size(px(11.0))
                .child(SharedString::from(error.to_string())),
        );
    }

    sidebar
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
