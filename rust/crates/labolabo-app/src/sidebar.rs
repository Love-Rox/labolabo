//! Sidebar: a repo-grouped Task list plus the "+" new-Task affordances
//! (`plans/012-task-model-and-control-cli.md` §1: "サイドバー = リポジトリ
//! ごとにグルーピングされた Task 一覧"). [`group_tasks_by_repo`] is pure and
//! gpui-free (unit-tested below) -- the same "logic lives outside gpui"
//! discipline `crate::focus` already established for the tile/tab tree;
//! [`render`] just walks its output and wires up click handlers.
//!
//! Deliberately minimal per this wave's brief ("title + kind の別が分かる
//! 程度の最小表示" / "本格ダイアログは将来"): no icons beyond a one-glyph
//! kind marker, no drag & drop reordering (plan §3), no rename/done/archive
//! affordances (plan §1 "今回スコープ外").

use gpui::{
    div, prelude::*, px, rgb, Context, IntoElement, MouseButton, MouseDownEvent, SharedString,
};

use labolabo_core::{Task, TaskKind};

use crate::app::LaboLaboApp;

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
            let task_id = task.id.clone();
            let is_selected = selected.as_deref() == Some(task.id.as_str());
            let row = div()
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
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                        this.select_task(task_id.clone(), window, cx);
                    }),
                );
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
