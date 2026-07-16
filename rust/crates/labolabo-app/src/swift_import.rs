//! Thin `labolabo-app` glue around `labolabo_core::import_from_swift`
//! (`labolabo-core/src/store/swift_import.rs`, which owns every actual
//! conversion rule and the read-only-against-the-Swift-database contract —
//! see that module's doc comment). This file only: locates the Swift app's
//! `labolabo.db`, persists whatever Tasks come back (`TaskDatabase::
//! upsert_task`), appends them to the caller's in-memory `tasks` list, and
//! formats the one-line result banner (`plans` W6e's "結果（n 件取込/スキッ
//! プ/警告）をサイドバー上部に一行表示（閉じられる）"). Both of `app.rs`'s
//! two triggers — automatic on first launch (`LaboLaboApp::new`, only when
//! `tasks`/`archived_tasks` are both empty) and the manual "ファイル >
//! Swift 版からインポート…" menu item (`ImportFromSwift`) — call [`run`].
//!
//! No `#[cfg(target_os = "macos")]` gate here: `labolabo_core::
//! SessionDatabase::default_path()` already resolves to a per-OS path Swift
//! only ever wrote to on macOS (see that function's doc comment), so
//! `run`'s own `.is_file()` check already degrades to "no Swift database"
//! on every other OS without needing an explicit `cfg` — and even in the
//! astronomically unlikely case some unrelated file sits at that path on
//! Linux/Windows, `labolabo_core::store::swift_import`'s read path degrades
//! to zero sessions (no `session` table) rather than erroring, so this is
//! safe either way.
//!
//! [`swift_db_path`] adds one more override on top of that default:
//! `LABOLABO_SWIFT_DB_PATH` (see its own doc comment) — this repo's smoke-
//! run recipe (`LABOLABO_RS_DATA_DIR=$(mktemp -d) cargo run -p labolabo-
//! app`) must never read a real user's Swift `labolabo.db`, and
//! `LABOLABO_RS_DATA_DIR` alone only relocates *this* port's own `tasks.db`
//! -- it has no effect on `SessionDatabase::default_path()`, a completely
//! separate, Swift-owned directory.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use labolabo_core::{SessionDatabase, Task, TaskDatabase};
use rust_i18n::t;

/// One [`run`] call's outcome, already summarized for the banner text —
/// see [`format_banner`].
pub struct ImportRunSummary {
    pub imported: usize,
    pub skipped_duplicate: usize,
    pub warnings: Vec<String>,
}

/// Runs one import pass against the Swift app's `labolabo.db`
/// (`SessionDatabase::default_path()`), persisting every newly converted
/// Task into `db` and appending it to `tasks`.
///
/// `existing_directories` is the caller's own union of every already-known
/// Task directory (active *and* archived — see `labolabo_core::
/// import_from_swift`'s doc comment on why archived Tasks count too), used
/// for the duplicate-directory skip rule; the auto-launch trigger only ever
/// calls this when both lists are empty (so an empty set), the manual menu
/// trigger passes the real thing.
///
/// - `None`: no Swift database found at `SessionDatabase::default_path()`
///   at all — the ordinary "Swift was never installed here" case. The
///   auto-launch trigger treats this as "nothing to show"; the manual menu
///   trigger surfaces it as its own banner text (see `app.rs`'s
///   `import_from_swift_menu`).
/// - `Some(Ok(summary))`: the import ran (however many sessions it actually
///   found — `summary.imported`/`skipped_duplicate` may both be `0`).
/// - `Some(Err(message))`: a structural read/write failure (not an
///   individual session's degrade — those are folded into
///   `summary.warnings` and never reach this branch), already formatted for
///   display.
pub fn run(
    db: &TaskDatabase,
    tasks: &mut Vec<Task>,
    existing_directories: &HashSet<String>,
) -> Option<Result<ImportRunSummary, String>> {
    run_against(&swift_db_path(), db, tasks, existing_directories)
}

/// The Swift database path [`run`] reads. `LABOLABO_SWIFT_DB_PATH` (set and
/// non-empty) overrides the real per-user path -- the same "developer
/// escape hatch" shape as `labolabo_core::store::rust_app_data_dir`'s own
/// `LABOLABO_RS_DATA_DIR`. This lives here rather than on
/// `SessionDatabase::default_path()` itself: that function is a faithful,
/// override-free port of Swift's own `AppDataDirectory` (see its doc
/// comment), and this override exists purely for this port's own smoke-
/// testing safety story -- `LABOLABO_RS_DATA_DIR=$(mktemp -d) cargo run -p
/// labolabo-app` must never read a real user's Swift `labolabo.db`, and
/// `LABOLABO_RS_DATA_DIR` alone can't guarantee that (it only relocates
/// *this* port's own Rust-only `tasks.db`, not the separate, Swift-owned
/// data directory `SessionDatabase::default_path()` resolves under -- see
/// `store::data_dir`'s module doc comment for why the two are deliberately
/// distinct trees). An empty value is treated as unset, matching every
/// other override in this codebase (`rust_app_data_dir`/
/// `ghostty_config`'s `XDG_CONFIG_HOME` handling).
fn swift_db_path() -> PathBuf {
    swift_db_path_from(std::env::var_os("LABOLABO_SWIFT_DB_PATH").as_deref())
}

/// The env-value-as-parameter core of [`swift_db_path`], split out so the
/// override rule is unit-testable without mutating the real process
/// environment (mutating env in tests races other tests in the same
/// process -- same convention `store::data_dir::rust_app_data_dir_from`
/// already established).
fn swift_db_path_from(override_path: Option<&std::ffi::OsStr>) -> PathBuf {
    match override_path {
        Some(path) if !path.is_empty() => PathBuf::from(path),
        _ => SessionDatabase::default_path(),
    }
}

/// [`run`]'s actual work, taking the Swift database path as a parameter so
/// tests can point it at a scratch fixture instead of the real per-user
/// path.
fn run_against(
    swift_db_path: &Path,
    db: &TaskDatabase,
    tasks: &mut Vec<Task>,
    existing_directories: &HashSet<String>,
) -> Option<Result<ImportRunSummary, String>> {
    if !swift_db_path.is_file() {
        return None;
    }

    let starting_sort_order = match db.next_sort_order() {
        Ok(n) => n,
        Err(err) => {
            return Some(Err(
                t!("swift_import.error.sort_order", err = err).to_string()
            ))
        }
    };

    match labolabo_core::import_from_swift(swift_db_path, existing_directories, starting_sort_order)
    {
        Ok(outcome) => {
            for task in &outcome.imported {
                if let Err(err) = db.upsert_task(task) {
                    eprintln!(
                        "labolabo-app: failed to persist task {} imported from the Swift app: {err}",
                        task.id
                    );
                }
            }
            let summary = ImportRunSummary {
                imported: outcome.imported.len(),
                skipped_duplicate: outcome.skipped_duplicate,
                warnings: outcome.warnings,
            };
            tasks.extend(outcome.imported);
            Some(Ok(summary))
        }
        Err(err) => Some(Err(t!("swift_import.error.generic", err = err).to_string())),
    }
}

/// Whether `summary` is worth showing a banner for at all — a Swift
/// database that exists but is simply empty (0 imported, 0 skipped, no
/// warnings; e.g. a fresh Swift install that never opened a session) has
/// nothing informative to say.
pub fn is_notable(summary: &ImportRunSummary) -> bool {
    summary.imported > 0 || summary.skipped_duplicate > 0 || !summary.warnings.is_empty()
}

/// The sidebar banner's one-line text (`plans` W6e's "n 件取込/スキップ/
/// 警告"). Warning *counts* only, not the full text — the full messages
/// (worktree/session ids, JSON parse errors) are logged to stderr by
/// `import_from_swift`'s caller-visible `ImportOutcome::warnings`
/// (`labolabo-core`'s own doc comment covers what each one means); a
/// one-line sidebar banner is not the right surface for a multi-paragraph
/// diagnostic dump.
pub fn format_banner(summary: &ImportRunSummary) -> String {
    let mut text = t!("swift_import.banner.imported", count = summary.imported).to_string();
    if summary.skipped_duplicate > 0 {
        text.push_str(&t!(
            "swift_import.banner.skipped",
            count = summary.skipped_duplicate
        ));
    }
    if !summary.warnings.is_empty() {
        text.push_str(&t!(
            "swift_import.banner.warnings",
            count = summary.warnings.len()
        ));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{params, Connection};
    use std::ffi::OsStr;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn swift_db_path_override_is_used_verbatim_when_set() {
        assert_eq!(
            swift_db_path_from(Some(OsStr::new("/tmp/labolabo-swift-scratch.db"))),
            PathBuf::from("/tmp/labolabo-swift-scratch.db")
        );
    }

    #[test]
    fn swift_db_path_empty_override_is_treated_as_unset() {
        assert_eq!(
            swift_db_path_from(Some(OsStr::new(""))),
            swift_db_path_from(None)
        );
    }

    #[test]
    fn swift_db_path_absent_override_falls_back_to_the_real_default_path() {
        assert_eq!(swift_db_path_from(None), SessionDatabase::default_path());
    }

    fn scratch_dir(prefix: &str) -> std::path::PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("{prefix}-{}-{nanos}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_minimal_swift_db(path: &Path, sessions: &[(&str, &str, &str, i64)]) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            "CREATE TABLE session (
                id TEXT NOT NULL PRIMARY KEY,
                worktreePath TEXT NOT NULL,
                name TEXT NOT NULL,
                branch TEXT,
                addedAt DATETIME NOT NULL,
                sortOrder INTEGER NOT NULL DEFAULT 0,
                agentSessionId TEXT,
                transcriptPath TEXT,
                adapterId TEXT
            );
            CREATE TABLE appState (key TEXT NOT NULL PRIMARY KEY, value TEXT);",
        )
        .unwrap();
        for (id, worktree_path, name, sort_order) in sessions {
            conn.execute(
                "INSERT INTO session (id, worktreePath, name, addedAt, sortOrder) \
                 VALUES (?1, ?2, ?3, '2026-07-01 09:00:00.000', ?4)",
                params![id, worktree_path, name, sort_order],
            )
            .unwrap();
        }
    }

    #[test]
    fn missing_swift_db_returns_none() {
        let dir = scratch_dir("labolabo-app-swift-import-missing");
        let db = TaskDatabase::open_in_memory().unwrap();
        let mut tasks = Vec::new();
        let result = run_against(
            &dir.join("does-not-exist.db"),
            &db,
            &mut tasks,
            &HashSet::new(),
        );
        assert!(result.is_none());
        assert!(tasks.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_persists_tasks_and_appends_to_the_in_memory_list() {
        let dir = scratch_dir("labolabo-app-swift-import-persist");
        let target = scratch_dir("labolabo-app-swift-import-persist-target");
        let swift_db = dir.join("labolabo.db");
        write_minimal_swift_db(
            &swift_db,
            &[("id-1", target.to_str().unwrap(), "取り込み対象", 0)],
        );

        let db = TaskDatabase::open_in_memory().unwrap();
        let mut tasks = Vec::new();
        let result = run_against(&swift_db, &db, &mut tasks, &HashSet::new());
        let summary = result
            .expect("Swift db exists")
            .expect("import should succeed");
        assert_eq!(summary.imported, 1);
        assert_eq!(summary.skipped_duplicate, 0);
        assert!(summary.warnings.is_empty());
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].title, "取り込み対象");

        // Persisted into `db`, not just the in-memory list.
        let all = db.all_tasks().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, tasks[0].id);

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&target);
    }

    #[test]
    fn existing_directories_are_respected_for_the_skip_rule() {
        let dir = scratch_dir("labolabo-app-swift-import-skip");
        let target = scratch_dir("labolabo-app-swift-import-skip-target");
        let swift_db = dir.join("labolabo.db");
        write_minimal_swift_db(
            &swift_db,
            &[("id-1", target.to_str().unwrap(), "既存と同じ", 0)],
        );

        let db = TaskDatabase::open_in_memory().unwrap();
        let mut tasks = Vec::new();
        let mut existing = HashSet::new();
        existing.insert(target.to_str().unwrap().to_string());
        let summary = run_against(&swift_db, &db, &mut tasks, &existing)
            .unwrap()
            .unwrap();
        assert_eq!(summary.imported, 0);
        assert_eq!(summary.skipped_duplicate, 1);
        assert!(tasks.is_empty());
        assert!(db.all_tasks().unwrap().is_empty());

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&target);
    }

    #[test]
    fn is_notable_is_false_only_for_a_fully_empty_summary() {
        assert!(!is_notable(&ImportRunSummary {
            imported: 0,
            skipped_duplicate: 0,
            warnings: Vec::new(),
        }));
        assert!(is_notable(&ImportRunSummary {
            imported: 1,
            skipped_duplicate: 0,
            warnings: Vec::new(),
        }));
        assert!(is_notable(&ImportRunSummary {
            imported: 0,
            skipped_duplicate: 1,
            warnings: Vec::new(),
        }));
        assert!(is_notable(&ImportRunSummary {
            imported: 0,
            skipped_duplicate: 0,
            warnings: vec!["warn".to_string()],
        }));
    }

    #[test]
    fn format_banner_includes_skip_and_warning_counts() {
        let text = format_banner(&ImportRunSummary {
            imported: 3,
            skipped_duplicate: 2,
            warnings: vec!["a".to_string()],
        });
        assert!(text.contains('3'));
        assert!(text.contains('2'));
        assert!(text.contains('1'));
    }
}
