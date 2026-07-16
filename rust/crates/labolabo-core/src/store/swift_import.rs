//! One-time importer that turns the Swift app's persisted sessions
//! (`SessionDatabase`/`labolabo.db`, `database.rs`) into this port's own
//! [`crate::store::Task`]s, so upgrading from the Swift app to this Rust
//! port restores the same open directories/worktrees, tab layout, and
//! per-tab Claude `--resume` state instead of starting from an empty
//! sidebar. There is no Swift source for this: it is new-in-Rust glue
//! between two schemas that otherwise never talk to each other (see
//! `task_database`'s module doc comment on why the two databases are
//! deliberately separate files/schemas).
//!
//! ## Read-only contract
//!
//! [`SwiftSessionReader::open`] opens the Swift database with
//! `SQLITE_OPEN_READ_ONLY` and never calls `database::SessionDatabase`'s
//! `ensure_schema` (which runs `ALTER TABLE`/`CREATE TABLE` DDL) — this
//! module must be safe to run while a real Swift `LaboLabo.app` is
//! concurrently running and writing to the *same* `labolabo.db` file. Every
//! read below tolerates a pre-v3 (v1/v2 GRDB migration) schema by checking
//! `PRAGMA table_info`/`sqlite_master` itself rather than assuming the v3
//! shape `database.rs` reconciles on write-open. See
//! `read_only_open_never_mutates_the_fixture_file` for the byte-identity
//! test this contract is held to.
//!
//! ## Conversion rules (one [`SessionRecord`] -> one [`Task`])
//!
//! - **`kind`**: [`resolve_task_kind`] classifies the session's
//!   `worktreePath` via [`GitEngine`] — `Worktree { branch, base: "", path }`
//!   only when the directory is *exactly* a linked (non-main) worktree's
//!   root (an exact match against a `git worktree list` entry other than
//!   the first/main one); everything else (a plain directory, the main
//!   worktree itself, a subdirectory of either, or any `git` failure)
//!   degrades to `Attached { directory }`. `branch` is the directory's
//!   *current* HEAD branch (`GitEngine::status`), not whatever branch name
//!   the Swift session happened to have cached; `base` is left empty — the
//!   Swift schema never recorded what a worktree branched from, so there is
//!   nothing faithful to carry over (see `TaskKind::Worktree`'s `base`
//!   field).
//! - **`layout`**: the Swift `appState["paneLayout:" + session.id]` JSON,
//!   decoded via [`TileLayout::from_json`] **verbatim** — this is the
//!   design's whole compatibility point, so it is never re-shaped. A
//!   session with no such key (common: it predates per-tab layouts/never
//!   had one saved) silently gets [`PaneTilingModel::default_layout`], no
//!   warning — that is the normal, expected state, not a failure. A session
//!   *with* a `paneLayout:` value that fails to parse also falls back to
//!   the default layout, but *does* append a warning (this can only happen
//!   from corruption/a future incompatible Swift version, so it is worth
//!   surfacing). Same degrade-with-warning treatment for an unparseable
//!   `addedAt`/appState read failure — never abort the whole import over
//!   one session.
//! - **`agent_bindings`**: built from every terminal [`PaneItem`] in the
//!   *decoded* layout that carries a Claude `agentSessionId` (last one wins,
//!   tree order — the same "last observed" rule
//!   [`crate::store::AgentBindings::record`] already documents), falling
//!   back to the Swift session record's own (session-level, pre-tabs)
//!   `agentSessionId`/`transcriptPath` columns only when the layout carried
//!   none at all. That fallback is deliberately broader than "layout only":
//!   `AgentBindings` is documented (`agent_bindings.rs`) as a *Task-level*
//!   fallback independent of per-tab plumbing, and many existing Swift
//!   installs predate the per-tab-resume feature entirely (their sessions
//!   have a real session-level Claude id but never wrote a `paneLayout:`
//!   tab payload at all) — dropping that id on import would silently break
//!   `--resume` for exactly the users this importer exists for.
//! - **`title`**/**`sort_order`**: carried over from the Swift
//!   `SessionRecord`'s `name` and (relative, re-based to start after every
//!   existing Rust Task's `sort_order` — see `starting_sort_order`)
//!   `sortOrder`, per the porting brief.
//! - **`created_at`**: the Swift session's `addedAt`, not "now" — closer to
//!   "same task, same original creation time" than
//!   `Task::new_worktree`/`new_attached`'s default of stamping both
//!   timestamps with the import moment.
//! - **Duplicate directories**: a Swift session whose `worktreePath` exactly
//!   matches an already-known directory (an existing Task the caller passed
//!   in via `existing_directories`, *or* an earlier session already
//!   imported in this same run) is skipped outright — see
//!   [`import_from_swift`]'s doc comment.

use std::collections::HashSet;
use std::path::Path;

use rusqlite::{params, Connection, OpenFlags};

use crate::git_engine::GitEngine;
use crate::tiling::{PaneTilingModel, TileLayout};

use super::database::{decode_grdb_date, row_value_owned, table_exists};
use super::error::StoreResult;
use super::record::SessionRecord;
use super::task_record::{Task, TaskKind};
use super::AgentBindings;

/// The result of one [`import_from_swift`] run.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct ImportOutcome {
    /// Newly built Tasks, in the same relative order as the Swift sessions
    /// they came from (ascending `sortOrder`), ready for the caller to
    /// `upsert_task`/append to its in-memory list. Never includes a Task for
    /// a skipped-duplicate session.
    pub imported: Vec<Task>,
    /// How many Swift sessions were skipped because their directory was
    /// already known (an existing Task, or an earlier session in this same
    /// run — see the module doc comment).
    pub skipped_duplicate: usize,
    /// Human-readable, best-effort diagnostics for individual sessions that
    /// degraded rather than failing the whole import (unparseable
    /// `paneLayout:` JSON, an unparseable `addedAt`, or an `appState` read
    /// error) — see the module doc comment's "Conversion rules" section.
    /// Never fatal; surfaced to the user as informational text only.
    pub warnings: Vec<String>,
}

impl ImportOutcome {
    /// The result of finding no Swift database at all (the ordinary "Swift
    /// was never installed on this machine" case) — a no-op, not an error.
    fn empty() -> Self {
        Self::default()
    }
}

/// Reads every session in the Swift app's `labolabo.db` at `swift_db_path`
/// and converts each into a [`Task`], skipping any whose directory is
/// already present in `existing_directories` (the caller's own union of
/// every already-known Task directory — active *and* archived, so a
/// deliberately-archived Task's directory isn't silently re-imported;
/// building that set is the caller's job, not this function's).
///
/// `starting_sort_order` re-bases the imported Tasks' `sort_order` so they
/// sort after every existing Task (typically
/// `TaskDatabase::next_sort_order()`) while preserving their *relative*
/// order from the Swift side — imported Task `i` (0-indexed, counting only
/// actually-imported, non-skipped sessions) gets `starting_sort_order + i`.
///
/// `Ok(ImportOutcome::empty())` (not an error) when `swift_db_path` doesn't
/// exist — see [`SwiftSessionReader::open`]. This function never writes to
/// `swift_db_path` — see the module doc comment's read-only contract.
pub fn import_from_swift(
    swift_db_path: &Path,
    existing_directories: &HashSet<String>,
    starting_sort_order: i64,
) -> StoreResult<ImportOutcome> {
    let Some(reader) = SwiftSessionReader::open(swift_db_path)? else {
        return Ok(ImportOutcome::empty());
    };

    let mut seen: HashSet<String> = existing_directories.clone();
    let mut outcome = ImportOutcome::empty();
    let engine = GitEngine::new();

    for raw in reader.read_sessions()? {
        if seen.contains(&raw.worktree_path) {
            outcome.skipped_duplicate += 1;
            continue;
        }

        let added_at = match decode_grdb_date(&raw.added_at_raw, "session.addedAt") {
            Ok(dt) => dt,
            Err(err) => {
                outcome.warnings.push(format!(
                    "session {} ({}): addedAt が不正なため現在時刻を使用しました ({err})",
                    raw.id, raw.name
                ));
                chrono::Utc::now()
            }
        };
        let record = SessionRecord::new(
            raw.id.clone(),
            raw.worktree_path.clone(),
            raw.name.clone(),
            raw.branch.clone(),
            added_at,
            raw.sort_order,
            raw.agent_session_id.clone(),
            raw.transcript_path.clone(),
            raw.adapter_id.clone(),
        );

        let (repo_key, repo_root, repo_name, kind) =
            resolve_task_kind(&engine, &record.worktree_path);
        let (layout, agent_bindings) =
            resolve_layout_and_bindings(&reader, &record, &mut outcome.warnings);

        let sort_order = starting_sort_order + outcome.imported.len() as i64;
        let mut task = match kind {
            TaskKind::Worktree { branch, base, path } => Task::new_worktree(
                repo_key, repo_root, repo_name, branch, base, path, layout, sort_order,
            ),
            TaskKind::Attached { directory } => Task::new_attached(
                repo_key, repo_root, repo_name, directory, layout, sort_order,
            ),
        };
        task.title = record.name.clone();
        task.created_at = record.added_at;
        task.agent_bindings = agent_bindings;

        seen.insert(record.worktree_path.clone());
        outcome.imported.push(task);
    }

    Ok(outcome)
}

/// Classifies `directory` via [`GitEngine`]: `(repo_key, repo_root,
/// repo_name, kind)`. See the module doc comment's "kind" bullet for the
/// exact-linked-worktree-or-else-attached rule; any `git` failure
/// (`repo_info` erroring — not a git repository, `git` missing, ...)
/// degrades straight to `Attached { directory }` using `directory` itself
/// for `repo_key`/`repo_root`/`repo_name`, same fallback
/// `new_task::resolve_attached_repo` uses for the (unrelated) "new Task"
/// flow.
fn resolve_task_kind(engine: &GitEngine, directory: &str) -> (String, String, String, TaskKind) {
    let path = Path::new(directory);
    let Ok(repo) = engine.repo_info(path) else {
        let dir = directory.to_string();
        return (
            dir.clone(),
            dir.clone(),
            dir.clone(),
            TaskKind::Attached { directory: dir },
        );
    };

    let is_linked_worktree = engine
        .list_worktrees(path)
        .map(|worktrees| {
            worktrees
                .iter()
                .enumerate()
                .any(|(index, wt)| index > 0 && paths_match(&wt.path, directory))
        })
        .unwrap_or(false);

    if is_linked_worktree {
        let branch = engine
            .status(path)
            .ok()
            .and_then(|status| status.branch)
            .unwrap_or_default();
        (
            repo.key,
            repo.root,
            repo.name,
            TaskKind::Worktree {
                branch,
                base: String::new(),
                path: directory.to_string(),
            },
        )
    } else {
        (
            repo.key,
            repo.root,
            repo.name,
            TaskKind::Attached {
                directory: directory.to_string(),
            },
        )
    }
}

/// Best-effort path equality: canonicalizes both sides (resolving symlinks/
/// relative components) so a worktree listed via a slightly different
/// (but equivalent) spelling still matches; falls back to a plain string
/// compare if either side can't be canonicalized (e.g. the directory was
/// since deleted — `git worktree list` can still report a now-missing
/// worktree's recorded path).
fn paths_match(a: &str, b: &str) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => a == b,
    }
}

/// Resolves a session's persisted per-session `TileLayout` (or the default
/// layout, with a warning on outright JSON corruption — see the module doc
/// comment) and its derived `Task::agent_bindings`.
fn resolve_layout_and_bindings(
    reader: &SwiftSessionReader,
    record: &SessionRecord,
    warnings: &mut Vec<String>,
) -> (TileLayout, Option<String>) {
    let key = format!("paneLayout:{}", record.id);
    let layout = match reader.app_state(&key) {
        Ok(Some(json)) => match TileLayout::from_json(&json) {
            Ok(layout) => layout,
            Err(err) => {
                warnings.push(format!(
                    "session {} ({}): 保存済みレイアウトの JSON を解釈できず既定レイアウトを使用しました ({err})",
                    record.id, record.name
                ));
                PaneTilingModel::default_layout().snapshot()
            }
        },
        Ok(None) => PaneTilingModel::default_layout().snapshot(),
        Err(err) => {
            warnings.push(format!(
                "session {} ({}): 保存済みレイアウトの読み取りに失敗し既定レイアウトを使用しました ({err})",
                record.id, record.name
            ));
            PaneTilingModel::default_layout().snapshot()
        }
    };

    let mut bindings = AgentBindings::default();
    if let Some(model) = PaneTilingModel::model_from(&layout) {
        for pane in model.panes() {
            if let Some(session_id) = &pane.agent_session_id {
                bindings.record(session_id, pane.agent_transcript_path.as_deref());
            }
        }
    }
    if bindings.last_session_id.is_none() {
        if let Some(session_id) = &record.agent_session_id {
            bindings.record(session_id, record.transcript_path.as_deref());
        }
    }

    let agent_bindings = if bindings.last_session_id.is_some() {
        Some(bindings.to_json())
    } else {
        None
    };
    (layout, agent_bindings)
}

/// One `session` row read straight off the Swift database, before
/// `import_from_swift` decodes `addedAt` (a per-row degrade, not a whole-
/// read failure — see the module doc comment).
struct RawSwiftSession {
    id: String,
    worktree_path: String,
    name: String,
    branch: Option<String>,
    added_at_raw: super::database::RawDate,
    sort_order: i64,
    agent_session_id: Option<String>,
    transcript_path: Option<String>,
    adapter_id: Option<String>,
}

/// A strictly read-only window onto the Swift app's `labolabo.db` — see the
/// module doc comment's read-only contract.
struct SwiftSessionReader {
    conn: Connection,
}

impl SwiftSessionReader {
    /// `Ok(None)` when `path` doesn't exist at all (the ordinary "Swift was
    /// never installed here" case — not an error). Otherwise opens strictly
    /// `SQLITE_OPEN_READ_ONLY`: never creates the file, never runs
    /// `database::SessionDatabase::ensure_schema`'s DDL, so a concurrently
    /// running Swift `LaboLabo.app` can keep writing to this same file
    /// throughout.
    fn open(path: &Path) -> StoreResult<Option<Self>> {
        if !path.is_file() {
            return Ok(None);
        }
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        Ok(Some(Self { conn }))
    }

    /// Every `session` row, oldest-`sortOrder`-first — tolerates a pre-v3
    /// (v1/v2) schema missing `agentSessionId`/`transcriptPath`/`adapterId`
    /// by checking `PRAGMA table_info` itself rather than assuming the v3
    /// shape (unlike `database::SessionDatabase::all_sessions`, which can
    /// assume it because `ensure_schema` already reconciled the schema on
    /// open). An absent `session` table (a `labolabo.db` that exists but
    /// was never actually populated) reads as zero rows, not an error.
    fn read_sessions(&self) -> StoreResult<Vec<RawSwiftSession>> {
        if !table_exists(&self.conn, "session")? {
            return Ok(Vec::new());
        }

        let mut columns_stmt = self.conn.prepare("PRAGMA table_info(session)")?;
        let existing_columns: Vec<String> = columns_stmt
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<Result<_, _>>()?;
        let has = |c: &str| existing_columns.iter().any(|e| e == c);
        let agent_session_col = if has("agentSessionId") {
            "agentSessionId"
        } else {
            "NULL"
        };
        let transcript_col = if has("transcriptPath") {
            "transcriptPath"
        } else {
            "NULL"
        };
        let adapter_col = if has("adapterId") {
            "adapterId"
        } else {
            "NULL"
        };

        let sql = format!(
            "SELECT id, worktreePath, name, branch, addedAt, sortOrder, \
                    {agent_session_col}, {transcript_col}, {adapter_col} \
             FROM session ORDER BY sortOrder"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| {
            Ok(RawSwiftSession {
                id: row.get(0)?,
                worktree_path: row.get(1)?,
                name: row.get(2)?,
                branch: row.get(3)?,
                added_at_raw: row_value_owned(row, 4)?,
                sort_order: row.get(5)?,
                agent_session_id: row.get(6)?,
                transcript_path: row.get(7)?,
                adapter_id: row.get(8)?,
            })
        })?;

        let mut sessions = Vec::new();
        for row in rows {
            sessions.push(row?);
        }
        Ok(sessions)
    }

    /// A single `appState` value by key, tolerating a missing `appState`
    /// table (`Ok(None)`, not an error) exactly like a missing `session`
    /// table above.
    fn app_state(&self, key: &str) -> StoreResult<Option<String>> {
        if !table_exists(&self.conn, "appState")? {
            return Ok(None);
        }
        let mut stmt = self
            .conn
            .prepare("SELECT value FROM appState WHERE key = ?1")?;
        let mut rows = stmt.query(params![key])?;
        match rows.next()? {
            Some(row) => Ok(row.get::<_, Option<String>>(0)?),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // Only used by `saved_layout_round_trips_verbatim_and_seeds_agent_
    // bindings_from_last_pane`, which is `#[cfg(unix)]`-gated -- see the
    // `git`/`init_repo_with_commit` helpers' doc comment above.
    #[cfg(unix)]
    use crate::tiling::{PaneItem, PaneKind, TileOrientation};
    use std::sync::atomic::{AtomicU64, Ordering};

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

    // Every test below that reaches `resolve_task_kind` -- i.e. calls
    // `import_from_swift` with at least one session that isn't skipped as a
    // duplicate -- ends up invoking the real `ToolLocator` (via
    // `GitEngine::repo_info`/`git_runner::run`), whose `#[cfg(not(unix))]`
    // arm is an `unimplemented!()` stub (`tool_locator.rs`'s module doc
    // comment: Windows tool-location support is deferred future work).
    // `#[cfg(unix)]`-gated per test, matching `git_engine.rs`'s own tests'
    // established convention for the same reason (see that module's test
    // section) -- these two helpers are only used by such tests, so they're
    // gated too (avoids an `unused function` warning on non-unix targets).

    #[cfg(unix)]
    fn git(args: &[&str], dir: &Path) {
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        crate::git_runner::run(&args, dir).unwrap();
    }

    #[cfg(unix)]
    fn init_repo_with_commit(dir: &Path) {
        git(&["init", "-b", "main"], dir);
        git(&["config", "user.email", "test@example.com"], dir);
        git(&["config", "user.name", "LaboLabo Test"], dir);
        std::fs::write(dir.join("a.txt"), "one\n").unwrap();
        git(&["add", "."], dir);
        git(&["-c", "commit.gpgsign=false", "commit", "-m", "init"], dir);
    }

    /// Hand-authored (not real-GRDB) v3-shaped `labolabo.db`, for exercising
    /// conversion rules the checked-in `fixtures/store/fixture.db` golden
    /// fixture doesn't cover (it has no `paneLayout:*` appState entries at
    /// all — see `tests/store_golden.rs`'s module doc comment for that
    /// fixture's actual, unrelated purpose). Written directly with
    /// `rusqlite` rather than through `SessionDatabase`/`TileLayout::
    /// to_json`, matching the exact v3 DDL `database::SESSION_TABLE_V3_DDL`/
    /// `APP_STATE_TABLE_DDL` documents, and Swift's real `JSONEncoder`
    /// output shape for `TileLayout` (verified against `tiling.rs`'s module
    /// doc comment's own captured examples) — a deliberate, documented
    /// simplification per this wave's brief (the alternative, a disposable
    /// Swift oracle, is out of scope for this wave); the risk this carries
    /// is that a *future* real GRDB/Swift write could theoretically diverge
    /// from this hand-authored shape in some byte-level way this test
    /// wouldn't catch.
    fn write_hand_authored_swift_db(path: &Path, rows: &[(&str, &str, &str, i64, Option<&str>)]) {
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
        for (id, worktree_path, name, sort_order, layout_json) in rows {
            conn.execute(
                "INSERT INTO session (id, worktreePath, name, addedAt, sortOrder) \
                 VALUES (?1, ?2, ?3, '2026-07-01 09:00:00.000', ?4)",
                params![id, worktree_path, name, sort_order],
            )
            .unwrap();
            if let Some(json) = layout_json {
                conn.execute(
                    "INSERT INTO appState(key, value) VALUES (?1, ?2)",
                    params![format!("paneLayout:{id}"), json],
                )
                .unwrap();
            }
        }
    }

    #[test]
    fn missing_swift_db_is_a_no_op_not_an_error() {
        let dir = scratch_dir("labolabo-swift-import-missing");
        let path = dir.join("does-not-exist.db");
        let outcome = import_from_swift(&path, &HashSet::new(), 0).unwrap();
        assert_eq!(outcome, ImportOutcome::empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)] // see the `git`/`init_repo_with_commit` helpers' doc comment above
    #[test]
    fn attached_directory_imports_with_default_layout_when_no_layout_saved() {
        let dir = scratch_dir("labolabo-swift-import-attached");
        let target = scratch_dir("labolabo-swift-import-attached-target");
        let db_path = dir.join("labolabo.db");
        write_hand_authored_swift_db(
            &db_path,
            &[(
                "11111111-1111-1111-1111-111111111111",
                target.to_str().unwrap(),
                "メイン作業",
                0,
                None,
            )],
        );

        let outcome = import_from_swift(&db_path, &HashSet::new(), 5).unwrap();
        assert_eq!(outcome.skipped_duplicate, 0);
        assert!(outcome.warnings.is_empty(), "{:?}", outcome.warnings);
        assert_eq!(outcome.imported.len(), 1);
        let task = &outcome.imported[0];
        assert_eq!(task.title, "メイン作業");
        assert_eq!(task.sort_order, 5);
        assert_eq!(
            task.kind,
            TaskKind::Attached {
                directory: target.to_str().unwrap().to_string()
            }
        );
        assert_eq!(task.layout, PaneTilingModel::default_layout().snapshot());
        assert_eq!(task.agent_bindings, None);

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&target);
    }

    #[cfg(unix)] // see the `git`/`init_repo_with_commit` helpers' doc comment above
    #[test]
    fn linked_worktree_directory_becomes_a_worktree_task_with_current_branch() {
        let dir = scratch_dir("labolabo-swift-import-wt-db");
        let repo = scratch_dir("labolabo-swift-import-wt-repo");
        init_repo_with_commit(&repo);
        let worktree_path = repo.join("wt-feature");
        git(
            &[
                "worktree",
                "add",
                "-b",
                "feature/imported",
                worktree_path.to_str().unwrap(),
            ],
            &repo,
        );

        let db_path = dir.join("labolabo.db");
        write_hand_authored_swift_db(
            &db_path,
            &[(
                "22222222-2222-2222-2222-222222222222",
                worktree_path.to_str().unwrap(),
                "worktree セッション",
                0,
                None,
            )],
        );

        let outcome = import_from_swift(&db_path, &HashSet::new(), 0).unwrap();
        assert_eq!(outcome.imported.len(), 1);
        let task = &outcome.imported[0];
        assert_eq!(
            task.kind,
            TaskKind::Worktree {
                branch: "feature/imported".to_string(),
                base: String::new(),
                path: worktree_path.to_str().unwrap().to_string(),
            }
        );
        // `repo_root`/`repo_key` come straight from `GitEngine::repo_info`
        // (the main worktree's own root/`.git` dir, shared across every
        // linked worktree) -- not asserted byte-for-byte here (git may
        // report a resolved-symlink path that differs cosmetically from
        // `repo`'s own spelling, e.g. macOS's `/tmp` -> `/private/tmp`);
        // `resolve_task_kind`'s own doc comment covers the exact contract.
        assert!(task.repo_key.ends_with(".git"));

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[cfg(unix)] // see the `git`/`init_repo_with_commit` helpers' doc comment above
    #[test]
    fn main_worktree_directory_is_attached_not_worktree_kind() {
        let dir = scratch_dir("labolabo-swift-import-main-db");
        let repo = scratch_dir("labolabo-swift-import-main-repo");
        init_repo_with_commit(&repo);

        let db_path = dir.join("labolabo.db");
        write_hand_authored_swift_db(
            &db_path,
            &[(
                "33333333-3333-3333-3333-333333333333",
                repo.to_str().unwrap(),
                "メインチェックアウト",
                0,
                None,
            )],
        );

        let outcome = import_from_swift(&db_path, &HashSet::new(), 0).unwrap();
        assert_eq!(outcome.imported.len(), 1);
        assert_eq!(
            outcome.imported[0].kind,
            TaskKind::Attached {
                directory: repo.to_str().unwrap().to_string()
            }
        );

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[cfg(unix)] // see the `git`/`init_repo_with_commit` helpers' doc comment above
    #[test]
    fn non_git_directory_degrades_to_attached() {
        let dir = scratch_dir("labolabo-swift-import-nongit-db");
        let target = scratch_dir("labolabo-swift-import-nongit-target");
        let db_path = dir.join("labolabo.db");
        write_hand_authored_swift_db(
            &db_path,
            &[(
                "44444444-4444-4444-4444-444444444444",
                target.to_str().unwrap(),
                "非 git ディレクトリ",
                0,
                None,
            )],
        );

        let outcome = import_from_swift(&db_path, &HashSet::new(), 0).unwrap();
        assert_eq!(
            outcome.imported[0].kind,
            TaskKind::Attached {
                directory: target.to_str().unwrap().to_string()
            }
        );

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&target);
    }

    #[cfg(unix)] // see the `git`/`init_repo_with_commit` helpers' doc comment above
    #[test]
    fn saved_layout_round_trips_verbatim_and_seeds_agent_bindings_from_last_pane() {
        let dir = scratch_dir("labolabo-swift-import-layout-db");
        let target = scratch_dir("labolabo-swift-import-layout-target");

        let mut model = PaneTilingModel::new(crate::tiling::TileNode::leaf(PaneItem::new(
            PaneKind::Terminal,
            "端末",
        )));
        let first_id = model.panes()[0].id;
        model.split(
            first_id,
            TileOrientation::Horizontal,
            PaneItem::with_agent_session(PaneKind::Terminal, "second", "sid-old", "/tmp/old.jsonl"),
        );
        let second_id = model.panes()[1].id;
        model.record_agent_session(
            "sid-new".to_string(),
            second_id,
            Some("/tmp/new.jsonl".to_string()),
        );
        let layout_json = model.snapshot().to_json();

        let db_path = dir.join("labolabo.db");
        write_hand_authored_swift_db(
            &db_path,
            &[(
                "55555555-5555-5555-5555-555555555555",
                target.to_str().unwrap(),
                "レイアウトあり",
                0,
                Some(&layout_json),
            )],
        );

        let outcome = import_from_swift(&db_path, &HashSet::new(), 0).unwrap();
        assert!(outcome.warnings.is_empty(), "{:?}", outcome.warnings);
        let task = &outcome.imported[0];
        assert_eq!(
            task.layout,
            model.snapshot(),
            "layout must round-trip verbatim"
        );
        let bindings = AgentBindings::from_json(task.agent_bindings.as_deref());
        assert_eq!(bindings.last_session_id.as_deref(), Some("sid-new"));
        assert_eq!(
            bindings.last_transcript_path.as_deref(),
            Some("/tmp/new.jsonl")
        );

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&target);
    }

    #[cfg(unix)] // see the `git`/`init_repo_with_commit` helpers' doc comment above
    #[test]
    fn malformed_layout_json_degrades_to_default_layout_with_a_warning() {
        let dir = scratch_dir("labolabo-swift-import-badjson-db");
        let target = scratch_dir("labolabo-swift-import-badjson-target");
        let db_path = dir.join("labolabo.db");
        write_hand_authored_swift_db(
            &db_path,
            &[(
                "66666666-6666-6666-6666-666666666666",
                target.to_str().unwrap(),
                "壊れたレイアウト",
                0,
                Some("{ this is not valid json"),
            )],
        );

        let outcome = import_from_swift(&db_path, &HashSet::new(), 0).unwrap();
        assert_eq!(outcome.warnings.len(), 1);
        assert!(outcome.warnings[0].contains("66666666-6666-6666-6666-666666666666"));
        assert_eq!(
            outcome.imported[0].layout,
            PaneTilingModel::default_layout().snapshot()
        );

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&target);
    }

    #[cfg(unix)] // see the `git`/`init_repo_with_commit` helpers' doc comment above
    #[test]
    fn session_level_agent_fields_seed_bindings_when_layout_has_none() {
        let dir = scratch_dir("labolabo-swift-import-legacy-agent-db");
        let target = scratch_dir("labolabo-swift-import-legacy-agent-target");
        let db_path = dir.join("labolabo.db");
        let conn_setup_path = db_path.clone();
        write_hand_authored_swift_db(
            &conn_setup_path,
            &[(
                "77777777-7777-7777-7777-777777777777",
                target.to_str().unwrap(),
                "pre-tabs セッション",
                0,
                None, // No paneLayout saved -- predates per-tab layouts.
            )],
        );
        {
            let conn = Connection::open(&db_path).unwrap();
            conn.execute(
                "UPDATE session SET agentSessionId = ?1, transcriptPath = ?2 WHERE id = ?3",
                params![
                    "legacy-sid",
                    "/tmp/legacy.jsonl",
                    "77777777-7777-7777-7777-777777777777"
                ],
            )
            .unwrap();
        }

        let outcome = import_from_swift(&db_path, &HashSet::new(), 0).unwrap();
        let bindings = AgentBindings::from_json(outcome.imported[0].agent_bindings.as_deref());
        assert_eq!(bindings.last_session_id.as_deref(), Some("legacy-sid"));
        assert_eq!(
            bindings.last_transcript_path.as_deref(),
            Some("/tmp/legacy.jsonl")
        );

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&target);
    }

    #[test]
    fn duplicate_directory_against_existing_task_is_skipped() {
        let dir = scratch_dir("labolabo-swift-import-dup-existing-db");
        let target = scratch_dir("labolabo-swift-import-dup-existing-target");
        let db_path = dir.join("labolabo.db");
        write_hand_authored_swift_db(
            &db_path,
            &[(
                "88888888-8888-8888-8888-888888888888",
                target.to_str().unwrap(),
                "既存と同じ",
                0,
                None,
            )],
        );

        let mut existing = HashSet::new();
        existing.insert(target.to_str().unwrap().to_string());
        let outcome = import_from_swift(&db_path, &existing, 0).unwrap();
        assert_eq!(outcome.imported.len(), 0);
        assert_eq!(outcome.skipped_duplicate, 1);

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&target);
    }

    #[cfg(unix)] // see the `git`/`init_repo_with_commit` helpers' doc comment above
    #[test]
    fn duplicate_directory_within_the_same_batch_is_skipped_after_the_first() {
        let dir = scratch_dir("labolabo-swift-import-dup-batch-db");
        let target = scratch_dir("labolabo-swift-import-dup-batch-target");
        let db_path = dir.join("labolabo.db");
        write_hand_authored_swift_db(
            &db_path,
            &[
                (
                    "99999999-9999-9999-9999-999999999999",
                    target.to_str().unwrap(),
                    "1つ目",
                    0,
                    None,
                ),
                (
                    "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
                    target.to_str().unwrap(),
                    "2つ目（同じディレクトリ）",
                    1,
                    None,
                ),
            ],
        );

        let outcome = import_from_swift(&db_path, &HashSet::new(), 0).unwrap();
        assert_eq!(outcome.imported.len(), 1);
        assert_eq!(outcome.skipped_duplicate, 1);
        assert_eq!(outcome.imported[0].title, "1つ目");

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&target);
    }

    #[cfg(unix)] // see the `git`/`init_repo_with_commit` helpers' doc comment above
    #[test]
    fn sort_order_is_rebased_after_starting_sort_order_preserving_relative_order() {
        let dir = scratch_dir("labolabo-swift-import-sortorder-db");
        let target_a = scratch_dir("labolabo-swift-import-sortorder-a");
        let target_b = scratch_dir("labolabo-swift-import-sortorder-b");
        let db_path = dir.join("labolabo.db");
        write_hand_authored_swift_db(
            &db_path,
            &[
                ("b1", target_b.to_str().unwrap(), "b", 0, None),
                ("a1", target_a.to_str().unwrap(), "a", 1, None),
            ],
        );

        let outcome = import_from_swift(&db_path, &HashSet::new(), 10).unwrap();
        assert_eq!(outcome.imported[0].sort_order, 10);
        assert_eq!(outcome.imported[1].sort_order, 11);
        assert_eq!(outcome.imported[0].title, "b");
        assert_eq!(outcome.imported[1].title, "a");

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&target_a);
        let _ = std::fs::remove_dir_all(&target_b);
    }

    #[cfg(unix)] // see the `git`/`init_repo_with_commit` helpers' doc comment above
    #[test]
    fn created_at_is_carried_over_from_added_at() {
        let dir = scratch_dir("labolabo-swift-import-createdat-db");
        let target = scratch_dir("labolabo-swift-import-createdat-target");
        let db_path = dir.join("labolabo.db");
        write_hand_authored_swift_db(
            &db_path,
            &[(
                "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb",
                target.to_str().unwrap(),
                "日付確認",
                0,
                None,
            )],
        );

        let outcome = import_from_swift(&db_path, &HashSet::new(), 0).unwrap();
        assert_eq!(
            outcome.imported[0].created_at,
            chrono::DateTime::parse_from_rfc3339("2026-07-01T09:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc)
        );

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&target);
    }

    /// The golden `fixtures/store/fixture.db` (real GRDB output — see
    /// `tests/store_golden.rs`) has no `paneLayout:*` keys at all, so every
    /// one of its 4 sessions must import with the default layout and no
    /// warnings. Two of the four (`メイン作業ブランチ`/`Codex セッション...`)
    /// *do* carry a real session-level `agentSessionId`/`transcriptPath`
    /// (see `expected_sessions` in `tests/store_golden.rs`) with no
    /// `paneLayout:` ever saved for them — exactly the "pre-tabs install"
    /// shape `session_level_agent_fields_seed_bindings_when_layout_has_none`
    /// exercises with a hand-authored DB; this test confirms the same
    /// fallback fires against real GRDB-written data too. Exercised here
    /// mainly for the read-only byte-identity guarantee (see the next test)
    /// and as a second, independently-written-DB smoke test beyond the
    /// hand-authored fixtures above.
    #[cfg(unix)] // see the `git`/`init_repo_with_commit` helpers' doc comment above
    #[test]
    fn imports_every_fixture_session_with_default_layout() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/store/fixture.db");
        let dir = scratch_dir("labolabo-swift-import-fixture-smoke");
        let path = dir.join("fixture.db");
        std::fs::copy(&fixture, &path).unwrap();

        let outcome = import_from_swift(&path, &HashSet::new(), 0).unwrap();
        assert_eq!(outcome.imported.len(), 4);
        assert_eq!(outcome.skipped_duplicate, 0);
        assert!(outcome.warnings.is_empty(), "{:?}", outcome.warnings);
        for task in &outcome.imported {
            assert_eq!(task.layout, PaneTilingModel::default_layout().snapshot());
        }
        // sortOrder order preserved (see `expected_sessions` in
        // `tests/store_golden.rs` for the fixture's actual row contents).
        assert_eq!(
            outcome
                .imported
                .iter()
                .map(|t| t.title.as_str())
                .collect::<Vec<_>>(),
            vec![
                "メイン作業ブランチ",
                "Codex セッション/絵文字🚀テスト & <tags> \"quotes\" 'apostrophe' back\\slash",
                "テスト用ワークツリー",
                "Gemini セッション"
            ]
        );
        let bindings_0 = AgentBindings::from_json(outcome.imported[0].agent_bindings.as_deref());
        assert_eq!(bindings_0.last_session_id.as_deref(), Some("agent-aaa-111"));
        let bindings_1 = AgentBindings::from_json(outcome.imported[1].agent_bindings.as_deref());
        assert_eq!(
            bindings_1.last_session_id.as_deref(),
            Some("codex-session-999")
        );
        assert_eq!(outcome.imported[2].agent_bindings, None);
        assert_eq!(outcome.imported[3].agent_bindings, None);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The read-only contract's central guarantee: running a full import
    /// against a copy of the real-GRDB golden fixture must not change a
    /// single byte of that file, and must not create any sibling journal/
    /// WAL/SHM file next to it either.
    #[cfg(unix)] // see the `git`/`init_repo_with_commit` helpers' doc comment above
    #[test]
    fn read_only_open_never_mutates_the_fixture_file() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/store/fixture.db");
        let dir = scratch_dir("labolabo-swift-import-readonly");
        let path = dir.join("fixture.db");
        std::fs::copy(&fixture, &path).unwrap();
        let before = std::fs::read(&path).unwrap();

        let outcome = import_from_swift(&path, &HashSet::new(), 0).unwrap();
        assert_eq!(outcome.imported.len(), 4);

        let after = std::fs::read(&path).unwrap();
        assert_eq!(before, after, "fixture.db bytes must be unchanged");
        for suffix in ["-journal", "-wal", "-shm"] {
            let sibling = dir.join(format!("fixture.db{suffix}"));
            assert!(!sibling.exists(), "read-only import created {sibling:?}");
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn opening_a_swift_db_via_sqlite_open_read_only_rejects_a_write() {
        // Direct unit check of `SwiftSessionReader::open`'s flag choice --
        // if this ever regressed to a writable open mode, this test would
        // catch it even before the byte-identity test above noticed.
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/store/fixture.db");
        let dir = scratch_dir("labolabo-swift-import-readonly-flag");
        let path = dir.join("fixture.db");
        std::fs::copy(&fixture, &path).unwrap();

        let reader = SwiftSessionReader::open(&path).unwrap().unwrap();
        let err = reader
            .conn
            .execute("DELETE FROM session WHERE id = 'does-not-matter'", [])
            .unwrap_err();
        assert!(matches!(err, rusqlite::Error::SqliteFailure(..)));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
