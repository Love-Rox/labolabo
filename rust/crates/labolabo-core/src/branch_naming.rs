//! Pure branch-name generation for the Task model's "new worktree" flow
//! (`plans/012-task-model-and-control-cli.md` §1: "worktree ... ブランチ名
//! （既定: 自動生成）"). No Swift counterpart — new-in-Rust product surface,
//! kept here (rather than in `labolabo-app`) so it's usable from a future
//! control-CLI (`plans/012` §2) too, and so it's unit-testable without gpui.

use chrono::NaiveDate;

/// The first available `"<prefix>/<YYYYMMDD>-<n>"` candidate (`n` starting
/// at 1, incrementing past any collision) not already present in
/// `existing_branches` — simple and collision-avoiding, matching the plan's
/// own example (`labolabo/<日付>-<連番>`). `existing_branches` is typically
/// [`crate::git_engine::GitEngine::local_branches`]'s output for the
/// target repo, so a caller doesn't need its own de-duplication pass.
///
/// `existing_branches` is scanned by exact string equality (not by
/// `refs/heads/` prefix — callers pass short names, same shape
/// `local_branches` already returns).
pub fn generate_branch_name(prefix: &str, date: NaiveDate, existing_branches: &[String]) -> String {
    let date_str = date.format("%Y%m%d").to_string();
    let mut n: u32 = 1;
    loop {
        let candidate = format!("{prefix}/{date_str}-{n}");
        if !existing_branches.iter().any(|b| b == &candidate) {
            return candidate;
        }
        n += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn no_collision_starts_at_one() {
        assert_eq!(
            generate_branch_name("labolabo", date(2026, 7, 14), &[]),
            "labolabo/20260714-1"
        );
    }

    #[test]
    fn zero_pads_month_and_day() {
        assert_eq!(
            generate_branch_name("labolabo", date(2026, 1, 5), &[]),
            "labolabo/20260105-1"
        );
    }

    #[test]
    fn skips_past_existing_collisions() {
        let existing = vec![
            "labolabo/20260714-1".to_string(),
            "labolabo/20260714-2".to_string(),
            "main".to_string(),
        ];
        assert_eq!(
            generate_branch_name("labolabo", date(2026, 7, 14), &existing),
            "labolabo/20260714-3"
        );
    }

    #[test]
    fn unrelated_branches_do_not_affect_the_result() {
        let existing = vec!["feature/other".to_string(), "dev".to_string()];
        assert_eq!(
            generate_branch_name("labolabo", date(2026, 7, 14), &existing),
            "labolabo/20260714-1"
        );
    }

    #[test]
    fn different_prefix_does_not_collide() {
        let existing = vec!["other/20260714-1".to_string()];
        assert_eq!(
            generate_branch_name("labolabo", date(2026, 7, 14), &existing),
            "labolabo/20260714-1"
        );
    }
}
