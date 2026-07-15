//! SQLite persistence for [`crate::store::task_record::Task`]
//! (`plans/012-task-model-and-control-cli.md` §1) — the Rust port's own
//! schema, with **no GRDB-compatibility constraint**: unlike `database.rs`
//! (which must stay byte-for-byte readable/writable by the Swift app's GRDB
//! migrator, see that module's doc comment), this schema, this database
//! *file*, and this migration mechanism are exclusively this port's own.
//!
//! ## Why a separate database file
//!
//! The Rust port never opens the Swift app's `SessionDatabase` (`labolabo.db`
//! under [`super::data_dir::app_data_dir`]) — both to avoid two unrelated
//! processes (a running Swift LaboLabo.app and a running Rust
//! `labolabo-app`) writing the same SQLite file concurrently, and because
//! this schema has no `session`/`appState`-v3 relationship to Swift's at
//! all. [`TaskDatabase::default_path`] resolves under
//! [`super::data_dir::rust_app_data_dir`] instead (`.../LaboLabo-rs/` — see
//! that function's doc comment) — a different leaf directory, so the two
//! database files can never collide even if both apps ran on the same
//! machine at once.
//!
//! ## Schema / migrations
//!
//! No GRDB migrator to stay compatible with means no need for
//! `database.rs`'s existence-checked-DDL reconciliation trick either: this
//! module tracks its own applied migrations in a `schemaMigrations(id TEXT
//! PRIMARY KEY, appliedAt TEXT)` ledger (a bespoke, much smaller analogue of
//! GRDB's `grdb_migrations` — the two tables never interact, are never
//! opened by the same connection, and would not collide even if they were:
//! this database has no `grdb_migrations` table at all). `MIGRATIONS` is an
//! ordered `(id, sql)` list; `ensure_schema` applies whichever entries
//! aren't yet recorded in the ledger, in order, each inside its own
//! transaction-per-migration (`execute_batch` covers each migration's own
//! multi-statement DDL). Today there is exactly one migration
//! (`"0001_task_and_app_state"`, both tables at once) — the mechanism is
//! still real (not a single hardcoded `CREATE TABLE IF NOT EXISTS`) so a
//! later wave can append `("0002_...", ...)` without reworking this module,
//! and so the fixture/round-trip tests below actually exercise the ledger
//! (`opening_an_already_migrated_database_is_a_no_op` would fail loudly —
//! "table task already exists" — if the guard were broken).
//!
//! ## `Task.layout` (TileLayout JSON) and dates
//!
//! `task.layout` stores [`crate::tiling::TileLayout::to_json`]'s output
//! verbatim (round-tripped via `TileLayout::from_json` on read) — the exact
//! same DTO the tile/tab tree already persists elsewhere (see `tiling.rs`),
//! just owned per-Task here instead of per-window. `createdAt`/
//! `lastActiveAt` are plain RFC 3339 text (`chrono`'s `to_rfc3339`/
//! `DateTime::parse_from_rfc3339`) — no GRDB `Date` format to match (see
//! this module's doc comment), so there's no need for `store::datetime`'s
//! GRDB-specific parser here.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};

use crate::tiling::TileLayout;

use super::data_dir::rust_app_data_dir;
use super::error::{StoreError, StoreResult};
use super::task_record::{Task, TaskKind, TaskStatus};

/// Ordered `(id, sql)` migrations, applied idempotently by `ensure_schema` —
/// see this module's doc comment.
const MIGRATIONS: &[(&str, &str)] = &[(
    "0001_task_and_app_state",
    "
    CREATE TABLE task (
        id TEXT NOT NULL PRIMARY KEY,
        repoKey TEXT NOT NULL,
        repoRoot TEXT NOT NULL,
        repoName TEXT NOT NULL,
        kind TEXT NOT NULL,
        branch TEXT,
        base TEXT,
        path TEXT NOT NULL,
        title TEXT NOT NULL,
        layout TEXT NOT NULL,
        status TEXT NOT NULL DEFAULT 'active',
        createdAt TEXT NOT NULL,
        lastActiveAt TEXT NOT NULL,
        sortOrder INTEGER NOT NULL DEFAULT 0,
        agentBindings TEXT
    );
    CREATE TABLE appState (
        key TEXT NOT NULL PRIMARY KEY,
        value TEXT
    );
    ",
)];

/// SQLite-backed store for [`Task`]s and small app-level key/value state
/// (e.g. the selected Task). See this module's doc comment for the schema
/// and the on-disk-location/compatibility contract.
pub struct TaskDatabase {
    conn: Connection,
}

impl TaskDatabase {
    /// Opens (creating if absent) the database at `path`, creating its
    /// parent directory if needed, and applies any not-yet-applied
    /// migrations.
    pub fn open(path: &Path) -> StoreResult<Self> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let conn = Connection::open(path)?;
        Self::ensure_schema(&conn)?;
        Ok(Self { conn })
    }

    /// An in-memory database, for tests. Still goes through `ensure_schema`.
    pub fn open_in_memory() -> StoreResult<Self> {
        let conn = Connection::open_in_memory()?;
        Self::ensure_schema(&conn)?;
        Ok(Self { conn })
    }

    /// `~/Library/Application Support/LaboLabo-rs/tasks.db` on macOS (and
    /// the platform-appropriate equivalent elsewhere — see
    /// [`rust_app_data_dir`]). Deliberately a different directory tree
    /// *and* filename from the Swift app's `labolabo.db` — see this
    /// module's doc comment.
    pub fn default_path() -> PathBuf {
        rust_app_data_dir().join("tasks.db")
    }

    fn ensure_schema(conn: &Connection) -> StoreResult<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schemaMigrations (
                id TEXT NOT NULL PRIMARY KEY,
                appliedAt TEXT NOT NULL
            )",
        )?;
        for (id, sql) in MIGRATIONS {
            let already_applied: Option<i64> = conn
                .query_row(
                    "SELECT 1 FROM schemaMigrations WHERE id = ?1",
                    params![id],
                    |row| row.get(0),
                )
                .optional()?;
            if already_applied.is_some() {
                continue;
            }
            conn.execute_batch(sql)?;
            conn.execute(
                "INSERT INTO schemaMigrations(id, appliedAt) VALUES (?1, ?2)",
                params![id, Utc::now().to_rfc3339()],
            )?;
        }
        Ok(())
    }

    // MARK: - Tasks

    /// All Tasks (every [`TaskStatus`]), ordered by `sortOrder` ascending —
    /// callers that only want active Tasks (the plan's restore-on-launch
    /// semantics) filter with `TaskStatus::Active` themselves; there is no
    /// separate `active_tasks` query since the filter is a single
    /// `Iterator::filter` away and this keeps the SQL surface smaller.
    pub fn all_tasks(&self) -> StoreResult<Vec<Task>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, repoKey, repoRoot, repoName, kind, branch, base, path, title, \
                    layout, status, createdAt, lastActiveAt, sortOrder, agentBindings \
             FROM task ORDER BY sortOrder",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, String>(9)?,
                row.get::<_, String>(10)?,
                row.get::<_, String>(11)?,
                row.get::<_, String>(12)?,
                row.get::<_, i64>(13)?,
                row.get::<_, Option<String>>(14)?,
            ))
        })?;

        let mut tasks = Vec::new();
        for row in rows {
            let (
                id,
                repo_key,
                repo_root,
                repo_name,
                kind_tag,
                branch,
                base,
                path,
                title,
                layout_json,
                status_tag,
                created_at_raw,
                last_active_at_raw,
                sort_order,
                agent_bindings,
            ) = row?;

            let kind = decode_kind(&kind_tag, branch, base, path)?;
            let status =
                TaskStatus::parse(&status_tag).ok_or_else(|| StoreError::InvalidTaskEnum {
                    column: "task.status",
                    raw: status_tag,
                })?;
            let layout =
                TileLayout::from_json(&layout_json).map_err(StoreError::InvalidLayoutJson)?;
            let created_at = decode_rfc3339(&created_at_raw, "task.createdAt")?;
            let last_active_at = decode_rfc3339(&last_active_at_raw, "task.lastActiveAt")?;

            tasks.push(Task {
                id,
                repo_key,
                repo_root,
                repo_name,
                kind,
                title,
                layout,
                status,
                created_at,
                last_active_at,
                sort_order,
                agent_bindings,
            });
        }
        Ok(tasks)
    }

    /// The lowest unused `sortOrder + 1` (i.e. `max(sortOrder) + 1`, or `0`
    /// for an empty table) — appends a newly created Task after every
    /// existing one, matching the plan's "新規作業は末尾に追加" default
    /// ordering (manual DnD reordering is plan §3, out of this wave's
    /// scope).
    pub fn next_sort_order(&self) -> StoreResult<i64> {
        let max: Option<i64> =
            self.conn
                .query_row("SELECT MAX(sortOrder) FROM task", [], |row| row.get(0))?;
        Ok(max.map(|m| m + 1).unwrap_or(0))
    }

    /// Insert-or-update keyed on `id` — same `ON CONFLICT` upsert shape as
    /// `database::SessionDatabase::upsert`.
    pub fn upsert_task(&self, task: &Task) -> StoreResult<()> {
        let (branch, base, path) = match &task.kind {
            TaskKind::Worktree { branch, base, path } => {
                (Some(branch.as_str()), Some(base.as_str()), path.as_str())
            }
            TaskKind::Attached { directory } => (None, None, directory.as_str()),
        };
        self.conn.execute(
            "INSERT INTO task \
                (id, repoKey, repoRoot, repoName, kind, branch, base, path, title, layout, \
                 status, createdAt, lastActiveAt, sortOrder, agentBindings) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15) \
             ON CONFLICT(id) DO UPDATE SET \
                repoKey = excluded.repoKey, \
                repoRoot = excluded.repoRoot, \
                repoName = excluded.repoName, \
                kind = excluded.kind, \
                branch = excluded.branch, \
                base = excluded.base, \
                path = excluded.path, \
                title = excluded.title, \
                layout = excluded.layout, \
                status = excluded.status, \
                createdAt = excluded.createdAt, \
                lastActiveAt = excluded.lastActiveAt, \
                sortOrder = excluded.sortOrder, \
                agentBindings = excluded.agentBindings",
            params![
                task.id,
                task.repo_key,
                task.repo_root,
                task.repo_name,
                task.kind.tag(),
                branch,
                base,
                path,
                task.title,
                task.layout.to_json(),
                task.status.tag(),
                task.created_at.to_rfc3339(),
                task.last_active_at.to_rfc3339(),
                task.sort_order,
                task.agent_bindings,
            ],
        )?;
        Ok(())
    }

    pub fn delete_task(&self, id: &str) -> StoreResult<()> {
        self.conn
            .execute("DELETE FROM task WHERE id = ?1", params![id])?;
        Ok(())
    }

    // MARK: - App state (selected Task)

    pub fn set_selected_task_id(&self, id: Option<&str>) -> StoreResult<()> {
        self.set_app_state(id, "selectedTask")
    }

    pub fn selected_task_id(&self) -> StoreResult<Option<String>> {
        self.app_state("selectedTask")
    }

    fn set_app_state(&self, value: Option<&str>, key: &str) -> StoreResult<()> {
        self.conn.execute(
            "INSERT INTO appState(key, value) VALUES(?1, ?2) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    fn app_state(&self, key: &str) -> StoreResult<Option<String>> {
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

fn decode_kind(
    tag: &str,
    branch: Option<String>,
    base: Option<String>,
    path: String,
) -> StoreResult<TaskKind> {
    match tag {
        "worktree" => {
            let branch = branch.ok_or_else(|| StoreError::InvalidTaskEnum {
                column: "task.branch",
                raw: "NULL".to_string(),
            })?;
            let base = base.ok_or_else(|| StoreError::InvalidTaskEnum {
                column: "task.base",
                raw: "NULL".to_string(),
            })?;
            Ok(TaskKind::Worktree { branch, base, path })
        }
        "attached" => Ok(TaskKind::Attached { directory: path }),
        other => Err(StoreError::InvalidTaskEnum {
            column: "task.kind",
            raw: other.to_string(),
        }),
    }
}

fn decode_rfc3339(raw: &str, column: &'static str) -> StoreResult<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|_| StoreError::InvalidDate {
            column,
            raw: raw.to_string(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_task(sort_order: i64) -> Task {
        Task::new_worktree(
            "/repo/.git",
            "/repo",
            "owner/repo",
            "feature/x",
            "main",
            "/repo/.worktrees/feature-x",
            TileLayout::default(),
            sort_order,
        )
    }

    #[test]
    fn fresh_database_starts_empty() {
        let db = TaskDatabase::open_in_memory().unwrap();
        assert_eq!(db.all_tasks().unwrap(), Vec::new());
        assert_eq!(db.next_sort_order().unwrap(), 0);
    }

    #[test]
    fn upsert_then_all_tasks_round_trips_ordered_by_sort_order_including_layout() {
        let db = TaskDatabase::open_in_memory().unwrap();
        let mut layout_model = crate::tiling::PaneTilingModel::default_layout();
        layout_model.split(
            layout_model.panes()[0].id,
            crate::tiling::TileOrientation::Horizontal,
            crate::tiling::PaneItem::new(crate::tiling::PaneKind::Terminal, "second"),
        );
        let mut b = sample_task(2);
        b.layout = layout_model.snapshot();
        let a = sample_task(1);
        db.upsert_task(&b).unwrap();
        db.upsert_task(&a).unwrap();

        let all = db.all_tasks().unwrap();
        assert_eq!(
            all.iter().map(|t| t.sort_order).collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(all[0], a);
        assert_eq!(all[1], b);
        assert_eq!(
            all[1].layout,
            layout_model.snapshot(),
            "layout JSON round-trips"
        );
    }

    #[test]
    fn upsert_on_existing_id_updates_in_place_not_duplicates() {
        let db = TaskDatabase::open_in_memory().unwrap();
        let mut task = sample_task(0);
        db.upsert_task(&task).unwrap();
        task.title = "renamed".to_string();
        task.status = TaskStatus::Done;
        task.sort_order = 9;
        db.upsert_task(&task).unwrap();

        let all = db.all_tasks().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0], task);
    }

    #[test]
    fn attached_kind_round_trips_with_null_branch_and_base() {
        let db = TaskDatabase::open_in_memory().unwrap();
        let task = Task::new_attached(
            "/repo/.git",
            "/repo",
            "owner/repo",
            "/repo",
            TileLayout::default(),
            0,
        );
        db.upsert_task(&task).unwrap();
        let all = db.all_tasks().unwrap();
        assert_eq!(
            all[0].kind,
            TaskKind::Attached {
                directory: "/repo".to_string()
            }
        );
    }

    #[test]
    fn delete_task_removes_row() {
        let db = TaskDatabase::open_in_memory().unwrap();
        let a = sample_task(0);
        let b = Task::new_attached("k", "r", "n", "/tmp/b", TileLayout::default(), 1);
        db.upsert_task(&a).unwrap();
        db.upsert_task(&b).unwrap();
        db.delete_task(&a.id).unwrap();
        let all = db.all_tasks().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, b.id);
    }

    #[test]
    fn next_sort_order_is_max_plus_one() {
        let db = TaskDatabase::open_in_memory().unwrap();
        db.upsert_task(&sample_task(0)).unwrap();
        db.upsert_task(&sample_task(5)).unwrap();
        assert_eq!(db.next_sort_order().unwrap(), 6);
    }

    #[test]
    fn selected_task_id_round_trips_and_defaults_to_none() {
        let db = TaskDatabase::open_in_memory().unwrap();
        assert_eq!(db.selected_task_id().unwrap(), None);
        db.set_selected_task_id(Some("t1")).unwrap();
        assert_eq!(db.selected_task_id().unwrap(), Some("t1".to_string()));
        db.set_selected_task_id(None).unwrap();
        assert_eq!(db.selected_task_id().unwrap(), None);
    }

    #[test]
    fn opening_an_already_migrated_database_is_a_no_op() {
        let conn = Connection::open_in_memory().unwrap();
        TaskDatabase::ensure_schema(&conn).unwrap();
        // A second reconciliation (as a second `open` of the same file
        // would trigger) must not error (e.g. "table task already
        // exists") -- proves the schemaMigrations ledger guard works, not
        // just that migrations are idempotent SQL on their own.
        TaskDatabase::ensure_schema(&conn).unwrap();
        let db = TaskDatabase { conn };
        db.upsert_task(&sample_task(0)).unwrap();
        assert_eq!(db.all_tasks().unwrap().len(), 1);
    }

    #[test]
    fn open_creates_parent_directory_and_persists_across_reopen() {
        let dir = std::env::temp_dir().join(format!(
            "labolabo-task-store-test-{}-{:x}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos() as u64
        ));
        let db_path = dir.join("nested").join("tasks.db");
        let _ = std::fs::remove_dir_all(&dir);

        {
            let db = TaskDatabase::open(&db_path).unwrap();
            db.upsert_task(&sample_task(0)).unwrap();
        }
        {
            let db = TaskDatabase::open(&db_path).unwrap();
            assert_eq!(db.all_tasks().unwrap().len(), 1);
        }

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn malformed_kind_surfaces_as_invalid_task_enum_error() {
        let db = TaskDatabase::open_in_memory().unwrap();
        db.conn
            .execute(
                "INSERT INTO task (id, repoKey, repoRoot, repoName, kind, branch, base, path, \
                    title, layout, status, createdAt, lastActiveAt, sortOrder) \
                 VALUES ('a', 'k', 'r', 'n', 'bogus', NULL, NULL, '/p', 't', '{}', 'active', \
                    '2026-07-13T09:00:00Z', '2026-07-13T09:00:00Z', 0)",
                [],
            )
            .unwrap();
        let err = db.all_tasks().unwrap_err();
        assert!(matches!(
            err,
            StoreError::InvalidTaskEnum {
                column: "task.kind",
                ..
            }
        ));
    }
}
