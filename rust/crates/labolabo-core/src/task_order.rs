//! Pure ordering logic for the sidebar's Task drag & drop reorder --
//! `plans/012-task-model-and-control-cli.md` §3's "サイドバーの作業（Task）
//!並び替え": DnD reordering, scoped to *within one repo group*
//! (cross-repo reordering isn't part of this wave's scope).
//!
//! No Swift source module maps onto this file -- the Swift app has no Task
//! model at all (see `store::task_record`'s module doc comment); this is
//! new-in-Rust product surface, same as `branch_naming`/`store::
//! task_database`.
//!
//! Deliberately gpui-free (only `crate::store::Task`), so it's unit-
//! testable without a gpui `Application`/window, and so the drop-position
//! math lives in one place shared by whatever UI eventually renders the
//! sidebar (currently `labolabo-app`'s `crate::sidebar`).

use crate::store::Task;

/// Computes the full list of Task ids in their new display order after
/// dragging `moved_id` to just before `before_id` within its repo group (or
/// to that group's end if `before_id` is `None`).
///
/// `tasks` must already be in the current display order
/// (`TaskDatabase::all_tasks`'s `sort_order` order, which is also
/// `crate::store::TaskDatabase::all_tasks`'s contract and
/// `labolabo-app::sidebar::group_tasks_by_repo`'s input assumption). Only
/// the relative order *within* `moved_id`'s repo group changes: every task
/// belonging to a different repo keeps the exact slot it already occupies
/// in the returned order, so groups stay interleaved exactly as they were
/// (a caller renumbering `sort_order` densely from the returned order
/// therefore never disturbs another repo's Tasks relative to each other or
/// to this one).
///
/// Returns `tasks`' current id order unchanged (a no-op) in every case
/// where there's nothing sensible to do: `moved_id` not found, `moved_id ==
/// before_id` (dropped onto itself), `before_id` naming an unknown id, or
/// `before_id` naming a Task in a *different* repo than `moved_id`
/// (cross-repo reordering is out of this wave's scope -- see this module's
/// doc comment).
pub fn reorder_task_ids(tasks: &[Task], moved_id: &str, before_id: Option<&str>) -> Vec<String> {
    let current_order = || tasks.iter().map(|t| t.id.clone()).collect::<Vec<_>>();

    let Some(moved) = tasks.iter().find(|t| t.id == moved_id) else {
        return current_order();
    };
    if before_id == Some(moved_id) {
        return current_order();
    }
    if let Some(before_id) = before_id {
        let Some(before) = tasks.iter().find(|t| t.id == before_id) else {
            return current_order();
        };
        if before.repo_key != moved.repo_key {
            return current_order();
        }
    }

    let repo_key = moved.repo_key.clone();
    let mut new_group: Vec<&str> = tasks
        .iter()
        .filter(|t| t.repo_key == repo_key && t.id != moved_id)
        .map(|t| t.id.as_str())
        .collect();
    let insert_at = match before_id {
        Some(before_id) => new_group
            .iter()
            .position(|id| *id == before_id)
            .unwrap_or(new_group.len()),
        None => new_group.len(),
    };
    new_group.insert(insert_at, moved_id);

    let mut group_iter = new_group.into_iter();
    tasks
        .iter()
        .map(|t| {
            if t.repo_key == repo_key {
                group_iter
                    .next()
                    .expect("new_group has exactly one entry per same-repo task")
                    .to_string()
            } else {
                t.id.clone()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tiling::TileLayout;

    fn task(id: &str, repo_key: &str, sort_order: i64) -> Task {
        let mut t = Task::new_attached(
            repo_key,
            repo_key,
            repo_key,
            format!("/tmp/{id}"),
            TileLayout::default(),
            sort_order,
        );
        t.id = id.to_string();
        t
    }

    #[test]
    fn moves_a_task_before_another_within_the_same_group() {
        let tasks = vec![task("a", "R", 0), task("b", "R", 1), task("c", "R", 2)];
        // Drag c before a: a, b, c -> c, a, b
        assert_eq!(
            reorder_task_ids(&tasks, "c", Some("a")),
            vec!["c", "a", "b"]
        );
    }

    #[test]
    fn moves_a_task_to_the_end_when_before_id_is_none() {
        let tasks = vec![task("a", "R", 0), task("b", "R", 1), task("c", "R", 2)];
        assert_eq!(reorder_task_ids(&tasks, "a", None), vec!["b", "c", "a"]);
    }

    #[test]
    fn other_repos_keep_their_exact_original_slots() {
        // Interleaved repos: R1 a, R2 x, R1 b, R2 y, R1 c.
        let tasks = vec![
            task("a", "R1", 0),
            task("x", "R2", 1),
            task("b", "R1", 2),
            task("y", "R2", 3),
            task("c", "R1", 4),
        ];
        // Drag c before a within R1: R1's group a,b,c -> c,a,b, spliced back
        // into the exact same slots (indices 0, 2, 4) the R1 tasks occupied.
        assert_eq!(
            reorder_task_ids(&tasks, "c", Some("a")),
            vec!["c", "x", "a", "y", "b"]
        );
    }

    #[test]
    fn dropping_onto_itself_is_a_no_op() {
        let tasks = vec![task("a", "R", 0), task("b", "R", 1)];
        assert_eq!(reorder_task_ids(&tasks, "a", Some("a")), vec!["a", "b"]);
    }

    #[test]
    fn unknown_moved_id_is_a_no_op() {
        let tasks = vec![task("a", "R", 0), task("b", "R", 1)];
        assert_eq!(reorder_task_ids(&tasks, "ghost", Some("a")), vec!["a", "b"]);
    }

    #[test]
    fn unknown_before_id_is_a_no_op() {
        let tasks = vec![task("a", "R", 0), task("b", "R", 1)];
        assert_eq!(reorder_task_ids(&tasks, "a", Some("ghost")), vec!["a", "b"]);
    }

    #[test]
    fn cross_repo_before_id_is_a_no_op() {
        let tasks = vec![task("a", "R1", 0), task("x", "R2", 1)];
        assert_eq!(
            reorder_task_ids(&tasks, "a", Some("x")),
            vec!["a", "x"],
            "R1's `a` can't be dropped before R2's `x` -- cross-repo reorder is out of scope"
        );
    }

    #[test]
    fn single_task_group_reorder_is_a_no_op_shape() {
        let tasks = vec![task("a", "R", 0)];
        assert_eq!(reorder_task_ids(&tasks, "a", None), vec!["a"]);
    }
}
