//! Detection for a Task's working directory having disappeared out from
//! under LaboLabo -- deleted outside the app (`rm -rf`, moved, trashed) or
//! an external/network volume that's currently unmounted (第8波c「見つから
//! なくなったワークツリーがサイドバーに残ってしまう」対応).
//!
//! ## Design: never auto-delete
//!
//! An unmounted volume is a legitimate, temporary state -- the directory
//! isn't *gone*, it's just not visible right now. So this module only ever
//! *detects* and *reports* missingness; nothing here removes a Task or its
//! persisted row. `crate::app::LaboLaboApp::missing_task_ids` holds the
//! result purely in memory (never written to `TaskDatabase`) precisely
//! because it's allowed to flip back to "found" on the very next check (a
//! reconnected volume, a "再確認" click) with no persisted state to
//! reconcile.
//!
//! ## Sync stat, called from two places
//!
//! [`is_missing`] is a single `Path::is_dir` stat call -- cheap on a local
//! filesystem, but can in principle block for a while against an
//! unresponsive network mount. It's still called synchronously from two
//! latency-sensitive call sites (`LaboLaboApp::new`'s initial-selection
//! check, and `LaboLaboApp::select_task`'s per-click check) because in both
//! cases the alternative is spawning a shell into that same directory right
//! afterward -- which would have to resolve the same path and therefore
//! block at least as long, worst case. The check adds no meaningfully worse
//! stall than what already existed; it just turns that stall into "found
//! out this Task is missing" instead of "hung and then failed to spawn a
//! shell." [`missing_ids`] (the bulk startup scan over every restored Task,
//! not just the selected one) is the one truly *additive* risk -- previously
//! nothing touched a non-selected Task's directory at all during startup --
//! so its caller (`LaboLaboApp::refresh_missing_task_ids`) runs it on a
//! background thread (`cx.background_spawn`), matching the existing
//! `ide_open::detect_installed_editors` startup-scan pattern.

use std::collections::HashSet;
use std::path::Path;

use labolabo_core::Task;

/// Whether `task`'s working directory can currently be resolved as a
/// directory. See this module's doc comment for the sync-call trade-off.
pub fn is_missing(task: &Task) -> bool {
    !Path::new(task.working_directory()).is_dir()
}

/// The ids of every Task in `tasks` whose working directory is currently
/// missing. Meant to run off the UI thread when scanning many Tasks at once
/// -- see this module's doc comment.
pub fn missing_ids<'a>(tasks: impl IntoIterator<Item = &'a Task>) -> HashSet<String> {
    tasks
        .into_iter()
        .filter(|task| is_missing(task))
        .map(|task| task.id.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use labolabo_core::TileLayout;

    fn attached_task_at(dir: &str) -> Task {
        Task::new_attached("k", "r", "n", dir, TileLayout::default(), 0)
    }

    fn scratch_dir(prefix: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "labolabo-missing-dir-{prefix}-{}-{:x}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos() as u64
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn an_existing_directory_is_not_missing() {
        let dir = scratch_dir("present");
        let task = attached_task_at(dir.to_str().unwrap());
        assert!(!is_missing(&task));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_deleted_directory_is_missing() {
        let dir = scratch_dir("deleted");
        let task = attached_task_at(dir.to_str().unwrap());
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(is_missing(&task));
    }

    /// A path that resolves to a plain file (not a directory) counts as
    /// missing too -- `working_directory()` is only ever meaningful as a
    /// directory, so a file at that path is just as unusable as nothing at
    /// all being there.
    #[test]
    fn a_path_that_is_a_file_not_a_directory_is_missing() {
        let dir = scratch_dir("file-not-dir");
        let file_path = dir.join("not-a-directory");
        std::fs::write(&file_path, "oops").unwrap();
        let task = attached_task_at(file_path.to_str().unwrap());
        assert!(is_missing(&task));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_ids_reports_only_the_tasks_whose_directory_is_gone() {
        let present_dir = scratch_dir("bulk-present");
        let missing_dir = scratch_dir("bulk-missing");
        let present = attached_task_at(present_dir.to_str().unwrap());
        let mut missing_task = attached_task_at(missing_dir.to_str().unwrap());
        std::fs::remove_dir_all(&missing_dir).unwrap();
        missing_task.id = "missing-task-id".to_string();

        let tasks = vec![present.clone(), missing_task.clone()];
        let ids = missing_ids(&tasks);
        assert_eq!(ids.len(), 1);
        assert!(ids.contains(&missing_task.id));
        assert!(!ids.contains(&present.id));

        let _ = std::fs::remove_dir_all(&present_dir);
    }

    #[test]
    fn missing_ids_of_an_empty_slice_is_empty() {
        let tasks: Vec<Task> = Vec::new();
        assert!(missing_ids(&tasks).is_empty());
    }
}
