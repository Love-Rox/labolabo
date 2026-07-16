//! Faithful port of `Sources/LaboLaboStore/SessionDatabase.swift`.
//!
//! ## GRDB compatibility contract
//!
//! An existing user's `~/Library/Application Support/LaboLabo/labolabo.db`
//! was created by GRDB's `DatabaseMigrator`, which tracks applied migrations
//! in a `grdb_migrations(identifier TEXT NOT NULL PRIMARY KEY)` table (see
//! `GRDB/Migration/DatabaseMigrator.swift`'s `runMigrations`). This port:
//!
//! - **Never reads or writes `grdb_migrations`.** It stays exclusively under
//!   the Swift side's management; a database opened by this crate and later
//!   reopened by the Swift app must not confuse GRDB's migrator (e.g. by
//!   appearing to have unregistered/superseded migrations applied).
//! - Reconciles the `session`/`appState` tables to the v3 shape (the final
//!   state of Swift's three migrations: `v1`, `v2-agentSession`,
//!   `v3-adapter`) via **idempotent, existence-checked DDL** rather than its
//!   own migration ledger: `ensure_schema` creates each table outright with
//!   its full v3 definition if the table doesn't exist yet (a brand-new
//!   database), or — if it already exists (an existing GRDB database, at
//!   *any* prior migration level, v1 through v3) — adds only whatever
//!   columns `PRAGMA table_info` shows are missing. This one code path
//!   handles a fresh database, a v1-only database, a v2 database, and an
//!   already-v3 database (a no-op) uniformly, and never touches
//!   `grdb_migrations` in any of those cases.
//!
//! Column types/constraints below are copied from
//! `SessionDatabase.swift`'s migrator verbatim (see the `t.column(...)`
//! calls in `v1`/`v2-agentSession`/`v3-adapter`), confirmed against GRDB's
//! `TableDefinition.primaryKey`/`column` implementations: a non-`.integer`
//! `primaryKey(_:_:)` column gets an explicit `NOT NULL` (GRDB adds this
//! itself to route around a SQLite quirk — see
//! <https://www.sqlite.org/quirks.html#primary_keys_can_sometimes_contain_nulls>
//! — cited in `TableDefinition.swift`'s doc comment), and `.datetime` /
//! `.integer` map to the SQL type keywords `DATETIME` / `INTEGER`.
//!
//! ## `Date` columns
//!
//! `addedAt` is the one non-trivial type-affinity crossing in this schema —
//! see `store::datetime`'s module doc comment for the full read/write
//! contract this module relies on.
//!
//! ## Ported operations
//!
//! All 8 `SessionPersisting` operations (`store::persisting`) are
//! implemented as inherent methods here: `all_sessions`, `upsert`,
//! `delete_session`, `set_selected_session_id`, `selected_session_id`,
//! `set_app_state`, `app_state`, `app_state_entries`.
//!
//! `upsert`'s SQL is an `INSERT ... ON CONFLICT(id) DO UPDATE` rather than a
//! literal port of `record.save(db)`: GRDB's `PersistableRecord.save`
//! (`GRDB/Record/PersistableRecord+Save.swift`) is documented as "if the
//! receiver has a non-nil primary key and a matching row in the database,
//! perform an update; otherwise, insert" — i.e. an upsert keyed on the
//! primary key, updating every persisted column — which is exactly what the
//! `ON CONFLICT` clause below does in one statement.
//!
//! `app_state_entries`'s row-mapping quirk is carried over faithfully: the
//! Swift source reads each row with `if let key: String = row["key"], let
//! value: String = row["value"]` — a *conditional* bind through
//! `Optional<String>: DatabaseValueConvertible`, which GRDB resolves to
//! `nil` for a NULL column. Since it's `if let` (not force-unwrapped), a row
//! whose `value` is NULL fails the binding and is **silently dropped from
//! the result**, not included with an empty string. `app_state_entries`
//! below reproduces this by skipping rows where `value` is `NULL` — see the
//! `null_value_row_is_dropped` test.

use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};

use super::data_dir::app_data_dir;
use super::datetime::{format_grdb_datetime, parse_grdb_datetime};
use super::error::{StoreError, StoreResult};
use super::record::SessionRecord;

const SESSION_TABLE_V3_DDL: &str = "
CREATE TABLE session (
    id TEXT NOT NULL PRIMARY KEY,
    worktreePath TEXT NOT NULL,
    name TEXT NOT NULL,
    branch TEXT,
    addedAt DATETIME NOT NULL,
    sortOrder INTEGER NOT NULL DEFAULT 0,
    agentSessionId TEXT,
    transcriptPath TEXT,
    adapterId TEXT
)";

const APP_STATE_TABLE_DDL: &str = "
CREATE TABLE appState (
    key TEXT NOT NULL PRIMARY KEY,
    value TEXT
)";

/// Columns `v2-agentSession`/`v3-adapter` add via `ALTER TABLE ... ADD
/// COLUMN`, in migration order. `ensure_schema` adds whichever of these are
/// still missing from an existing (older) GRDB database.
const SESSION_ALTER_COLUMNS: &[(&str, &str)] = &[
    ("agentSessionId", "TEXT"),
    ("transcriptPath", "TEXT"),
    ("adapterId", "TEXT"),
];

/// SQLite-backed store for app-owned session metadata, opened against
/// either a brand-new database file or an existing GRDB-created one. See
/// this module's doc comment for the compatibility contract.
pub struct SessionDatabase {
    conn: Connection,
}

impl SessionDatabase {
    /// Opens (creating if absent) the database at `path`, creating its
    /// parent directory if needed, and reconciles its schema to the v3
    /// shape. Mirrors Swift's `init(url:)`.
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

    /// An in-memory database, for tests. Still goes through
    /// `ensure_schema`, so it always starts at the v3 shape.
    pub fn open_in_memory() -> StoreResult<Self> {
        let conn = Connection::open_in_memory()?;
        Self::ensure_schema(&conn)?;
        Ok(Self { conn })
    }

    /// `~/Library/Application Support/LaboLabo/labolabo.db` on macOS (and
    /// the platform-appropriate equivalent elsewhere — see
    /// `store::data_dir`).
    pub fn default_path() -> PathBuf {
        app_data_dir().join("labolabo.db")
    }

    fn ensure_schema(conn: &Connection) -> StoreResult<()> {
        if table_exists(conn, "session")? {
            for (column, decl) in SESSION_ALTER_COLUMNS {
                add_column_if_missing(conn, "session", column, decl)?;
            }
        } else {
            conn.execute_batch(SESSION_TABLE_V3_DDL)?;
        }

        if !table_exists(conn, "appState")? {
            conn.execute_batch(APP_STATE_TABLE_DDL)?;
        }

        Ok(())
    }

    // MARK: - Sessions

    /// All sessions, ordered by `sortOrder` ascending (mirrors
    /// `SessionRecord.order(Column("sortOrder")).fetchAll(db)`).
    pub fn all_sessions(&self) -> StoreResult<Vec<SessionRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, worktreePath, name, branch, addedAt, sortOrder, \
                    agentSessionId, transcriptPath, adapterId \
             FROM session ORDER BY sortOrder",
        )?;
        let rows = stmt.query_map([], |row| {
            let id: String = row.get(0)?;
            let worktree_path: String = row.get(1)?;
            let name: String = row.get(2)?;
            let branch: Option<String> = row.get(3)?;
            let added_at_raw = row_value_owned(row, 4)?;
            let sort_order: i64 = row.get(5)?;
            let agent_session_id: Option<String> = row.get(6)?;
            let transcript_path: Option<String> = row.get(7)?;
            let adapter_id: Option<String> = row.get(8)?;
            Ok((
                id,
                worktree_path,
                name,
                branch,
                added_at_raw,
                sort_order,
                agent_session_id,
                transcript_path,
                adapter_id,
            ))
        })?;

        let mut sessions = Vec::new();
        for row in rows {
            let (
                id,
                worktree_path,
                name,
                branch,
                added_at_raw,
                sort_order,
                agent_session_id,
                transcript_path,
                adapter_id,
            ) = row?;
            let added_at = decode_grdb_date(&added_at_raw, "session.addedAt")?;
            sessions.push(SessionRecord {
                id,
                worktree_path,
                name,
                branch,
                added_at,
                sort_order,
                agent_session_id,
                transcript_path,
                adapter_id,
            });
        }
        Ok(sessions)
    }

    /// Insert-or-update keyed on `id` — see this module's doc comment for
    /// why this is one `ON CONFLICT` statement rather than a literal port
    /// of GRDB's `save(_:)`.
    pub fn upsert(&self, record: &SessionRecord) -> StoreResult<()> {
        self.conn.execute(
            "INSERT INTO session \
                (id, worktreePath, name, branch, addedAt, sortOrder, agentSessionId, transcriptPath, adapterId) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) \
             ON CONFLICT(id) DO UPDATE SET \
                worktreePath = excluded.worktreePath, \
                name = excluded.name, \
                branch = excluded.branch, \
                addedAt = excluded.addedAt, \
                sortOrder = excluded.sortOrder, \
                agentSessionId = excluded.agentSessionId, \
                transcriptPath = excluded.transcriptPath, \
                adapterId = excluded.adapterId",
            params![
                record.id,
                record.worktree_path,
                record.name,
                record.branch,
                format_grdb_datetime(&record.added_at),
                record.sort_order,
                record.agent_session_id,
                record.transcript_path,
                record.adapter_id,
            ],
        )?;
        Ok(())
    }

    pub fn delete_session(&self, id: &str) -> StoreResult<()> {
        self.conn
            .execute("DELETE FROM session WHERE id = ?1", params![id])?;
        Ok(())
    }

    // MARK: - App state (e.g. last selection)

    pub fn set_selected_session_id(&self, id: Option<&str>) -> StoreResult<()> {
        self.set_app_state(id, "selectedSession")
    }

    pub fn selected_session_id(&self) -> StoreResult<Option<String>> {
        self.app_state("selectedSession")
    }

    // MARK: - Generic key-value app state

    pub fn set_app_state(&self, value: Option<&str>, key: &str) -> StoreResult<()> {
        self.conn.execute(
            "INSERT INTO appState(key, value) VALUES(?1, ?2) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    /// `None` both when the key is absent *and* when its stored value is
    /// `NULL` — mirrors GRDB's documented `fetchOne` behavior ("nil if
    /// there is no row, or if there is a row with a null value";
    /// `GRDB/Core/DatabaseValueConvertible.swift`).
    pub fn app_state(&self, key: &str) -> StoreResult<Option<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT value FROM appState WHERE key = ?1")?;
        let mut rows = stmt.query(params![key])?;
        match rows.next()? {
            Some(row) => Ok(row.get::<_, Option<String>>(0)?),
            None => Ok(None),
        }
    }

    /// All entries whose key starts with `prefix`, as a key -> value map.
    /// Rows whose `value` is `NULL` are dropped — see this module's doc
    /// comment.
    pub fn app_state_entries(
        &self,
        prefix: &str,
    ) -> StoreResult<std::collections::HashMap<String, String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT key, value FROM appState WHERE key LIKE ?1")?;
        let like_pattern = format!("{prefix}%");
        let rows = stmt.query_map(params![like_pattern], |row| {
            let key: String = row.get(0)?;
            let value: Option<String> = row.get(1)?;
            Ok((key, value))
        })?;

        let mut result = std::collections::HashMap::new();
        for row in rows {
            let (key, value) = row?;
            if let Some(value) = value {
                result.insert(key, value);
            }
        }
        Ok(result)
    }
}

/// `pub(super)`: reused by `store::swift_import`'s strictly-read-only Swift
/// `labolabo.db` reader, which must tolerate a pre-v3 (v1/v2) schema without
/// ever running this module's `ensure_schema` (that would `ALTER TABLE`,
/// violating the read-only contract).
pub(super) fn table_exists(conn: &Connection, name: &str) -> StoreResult<bool> {
    let exists: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
            params![name],
            |row| row.get(0),
        )
        .optional()?;
    Ok(exists.is_some())
}

fn add_column_if_missing(
    conn: &Connection,
    table: &str,
    column: &str,
    decl: &str,
) -> StoreResult<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let existing = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<String>, _>>()?;
    if !existing.iter().any(|c| c == column) {
        conn.execute(
            &format!("ALTER TABLE {table} ADD COLUMN {column} {decl}"),
            [],
        )?;
    }
    Ok(())
}

/// SQLite storage-class-aware raw read of a `DATETIME` column, deferring
/// interpretation to `decode_grdb_date` (kept separate so the two numeric
/// storage classes below and the `NOT NULL` violation each get one clear
/// call site instead of being interleaved with `rusqlite`'s row-mapping
/// closure signature).
pub(super) enum RawDate {
    Text(String),
    Numeric(f64),
    Null,
    Blob,
}

pub(super) fn row_value_owned(row: &rusqlite::Row<'_>, idx: usize) -> rusqlite::Result<RawDate> {
    use rusqlite::types::ValueRef;
    Ok(match row.get_ref(idx)? {
        ValueRef::Text(bytes) => RawDate::Text(String::from_utf8_lossy(bytes).into_owned()),
        ValueRef::Integer(i) => RawDate::Numeric(i as f64),
        ValueRef::Real(f) => RawDate::Numeric(f),
        ValueRef::Null => RawDate::Null,
        ValueRef::Blob(_) => RawDate::Blob,
    })
}

/// Mirrors `Date.fromDatabaseValue`: try TEXT parsing first (branch 1 of
/// `store::datetime`'s doc comment), then fall back to interpreting a
/// numeric storage class as `timeIntervalSince1970` **seconds** (branch 2).
pub(super) fn decode_grdb_date(
    raw: &RawDate,
    column: &'static str,
) -> StoreResult<chrono::DateTime<chrono::Utc>> {
    match raw {
        RawDate::Text(s) => parse_grdb_datetime(s).ok_or_else(|| StoreError::InvalidDate {
            column,
            raw: s.clone(),
        }),
        RawDate::Numeric(seconds) => {
            let millis = (seconds * 1000.0).round() as i64;
            chrono::DateTime::<chrono::Utc>::from_timestamp_millis(millis).ok_or_else(|| {
                StoreError::InvalidDate {
                    column,
                    raw: seconds.to_string(),
                }
            })
        }
        RawDate::Null => Err(StoreError::InvalidDate {
            column,
            raw: "NULL".to_string(),
        }),
        RawDate::Blob => Err(StoreError::InvalidDate {
            column,
            raw: "<blob>".to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn sample(id: &str, sort_order: i64) -> SessionRecord {
        SessionRecord::new(
            id,
            format!("/tmp/{id}"),
            format!("session-{id}"),
            Some("main".to_string()),
            chrono::Utc.with_ymd_and_hms(2026, 7, 13, 9, 0, 0).unwrap(),
            sort_order,
            None,
            None,
            None,
        )
    }

    #[test]
    fn fresh_database_starts_empty() {
        let db = SessionDatabase::open_in_memory().unwrap();
        assert_eq!(db.all_sessions().unwrap(), Vec::new());
    }

    #[test]
    fn fresh_database_gets_v3_schema_directly() {
        let db = SessionDatabase::open_in_memory().unwrap();
        let mut stmt = db.conn.prepare("PRAGMA table_info(session)").unwrap();
        let columns: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            columns,
            vec![
                "id",
                "worktreePath",
                "name",
                "branch",
                "addedAt",
                "sortOrder",
                "agentSessionId",
                "transcriptPath",
                "adapterId",
            ]
        );
        // Never touch grdb_migrations, even implicitly.
        assert!(!table_exists(&db.conn, "grdb_migrations").unwrap());
    }

    #[test]
    fn upsert_then_all_sessions_round_trips_ordered_by_sort_order() {
        let db = SessionDatabase::open_in_memory().unwrap();
        db.upsert(&sample("b", 2)).unwrap();
        db.upsert(&sample("a", 1)).unwrap();
        let all = db.all_sessions().unwrap();
        assert_eq!(
            all.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
            vec!["a", "b"]
        );
        assert_eq!(all[0], sample("a", 1));
    }

    #[test]
    fn upsert_on_existing_id_updates_in_place_not_duplicates() {
        let db = SessionDatabase::open_in_memory().unwrap();
        db.upsert(&sample("a", 1)).unwrap();
        let mut updated = sample("a", 1);
        updated.name = "renamed".to_string();
        updated.sort_order = 9;
        updated.agent_session_id = Some("agent-123".to_string());
        db.upsert(&updated).unwrap();

        let all = db.all_sessions().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0], updated);
    }

    #[test]
    fn delete_session_removes_row() {
        let db = SessionDatabase::open_in_memory().unwrap();
        db.upsert(&sample("a", 1)).unwrap();
        db.upsert(&sample("b", 2)).unwrap();
        db.delete_session("a").unwrap();
        let all = db.all_sessions().unwrap();
        assert_eq!(
            all.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
            vec!["b"]
        );
    }

    #[test]
    fn delete_session_on_missing_id_is_a_no_op() {
        let db = SessionDatabase::open_in_memory().unwrap();
        db.delete_session("does-not-exist").unwrap();
        assert!(db.all_sessions().unwrap().is_empty());
    }

    #[test]
    fn selected_session_id_round_trips_and_defaults_to_none() {
        let db = SessionDatabase::open_in_memory().unwrap();
        assert_eq!(db.selected_session_id().unwrap(), None);
        db.set_selected_session_id(Some("a")).unwrap();
        assert_eq!(db.selected_session_id().unwrap(), Some("a".to_string()));
        // Setting to None stores an explicit NULL for the key (row keeps
        // existing), not a delete — matches Swift binding `id: String?`
        // straight into the SQL parameter.
        db.set_selected_session_id(None).unwrap();
        assert_eq!(db.selected_session_id().unwrap(), None);
        assert_eq!(
            db.conn
                .query_row::<i64, _, _>(
                    "SELECT COUNT(*) FROM appState WHERE key = 'selectedSession'",
                    [],
                    |row| row.get(0)
                )
                .unwrap(),
            1
        );
    }

    #[test]
    fn app_state_round_trips_and_missing_key_is_none() {
        let db = SessionDatabase::open_in_memory().unwrap();
        assert_eq!(db.app_state("k").unwrap(), None);
        db.set_app_state(Some("v1"), "k").unwrap();
        assert_eq!(db.app_state("k").unwrap(), Some("v1".to_string()));
        db.set_app_state(Some("v2"), "k").unwrap();
        assert_eq!(db.app_state("k").unwrap(), Some("v2".to_string()));
    }

    #[test]
    fn app_state_entries_filters_by_prefix_and_drops_null_values() {
        let db = SessionDatabase::open_in_memory().unwrap();
        db.set_app_state(Some("1"), "tab.a").unwrap();
        db.set_app_state(Some("2"), "tab.b").unwrap();
        db.set_app_state(None, "tab.c").unwrap(); // NULL value: dropped from the result.
        db.set_app_state(Some("x"), "other").unwrap();

        let entries = db.app_state_entries("tab.").unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries.get("tab.a"), Some(&"1".to_string()));
        assert_eq!(entries.get("tab.b"), Some(&"2".to_string()));
        assert_eq!(
            entries.get("tab.c"),
            None,
            "NULL-valued row must be dropped, not empty-string"
        );
    }

    #[test]
    fn reconciles_v1_only_legacy_schema_up_to_v3() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE session (
                id TEXT NOT NULL PRIMARY KEY,
                worktreePath TEXT NOT NULL,
                name TEXT NOT NULL,
                branch TEXT,
                addedAt DATETIME NOT NULL,
                sortOrder INTEGER NOT NULL DEFAULT 0
             );
             CREATE TABLE appState (key TEXT NOT NULL PRIMARY KEY, value TEXT);
             INSERT INTO session (id, worktreePath, name, branch, addedAt, sortOrder)
                VALUES ('a', '/tmp/a', 'legacy', NULL, '2026-07-13 09:00:00.000', 0);",
        )
        .unwrap();

        SessionDatabase::ensure_schema(&conn).unwrap();
        let db = SessionDatabase { conn };

        let all = db.all_sessions().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, "a");
        assert_eq!(all[0].agent_session_id, None);
        assert_eq!(all[0].transcript_path, None);
        assert_eq!(all[0].adapter_id, None);

        // Reconciliation must not have touched grdb_migrations (it never
        // existed in this legacy fixture, and must still not exist).
        assert!(!table_exists(&db.conn, "grdb_migrations").unwrap());

        // Upserting now succeeds against the reconciled (9-column) shape.
        let mut updated = sample("a", 5);
        updated.adapter_id = Some("codex".to_string());
        db.upsert(&updated).unwrap();
        assert_eq!(db.all_sessions().unwrap()[0], updated);
    }

    #[test]
    fn reconciles_v2_legacy_schema_adding_only_adapter_id() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE session (
                id TEXT NOT NULL PRIMARY KEY,
                worktreePath TEXT NOT NULL,
                name TEXT NOT NULL,
                branch TEXT,
                addedAt DATETIME NOT NULL,
                sortOrder INTEGER NOT NULL DEFAULT 0,
                agentSessionId TEXT,
                transcriptPath TEXT
             );
             CREATE TABLE appState (key TEXT NOT NULL PRIMARY KEY, value TEXT);",
        )
        .unwrap();

        SessionDatabase::ensure_schema(&conn).unwrap();
        let db = SessionDatabase { conn };
        let mut updated = sample("a", 0);
        updated.adapter_id = Some("gemini".to_string());
        db.upsert(&updated).unwrap();
        assert_eq!(db.all_sessions().unwrap()[0], updated);
    }

    #[test]
    fn opening_an_already_v3_database_is_a_no_op_reconciliation() {
        let conn = Connection::open_in_memory().unwrap();
        SessionDatabase::ensure_schema(&conn).unwrap();
        // Calling ensure_schema again (as a second `open` would) must not
        // error or duplicate anything.
        SessionDatabase::ensure_schema(&conn).unwrap();
        let db = SessionDatabase { conn };
        db.upsert(&sample("a", 0)).unwrap();
        assert_eq!(db.all_sessions().unwrap().len(), 1);
    }

    #[test]
    fn numeric_storage_class_falls_back_to_unix_seconds() {
        let db = SessionDatabase::open_in_memory().unwrap();
        conn_insert_raw_added_at(&db.conn, "a", "1752400800"); // INTEGER, not TEXT
        let all = db.all_sessions().unwrap();
        assert_eq!(
            all[0].added_at,
            chrono::DateTime::from_timestamp(1_752_400_800, 0).unwrap()
        );
    }

    #[test]
    fn malformed_text_date_surfaces_as_invalid_date_error() {
        let db = SessionDatabase::open_in_memory().unwrap();
        conn_insert_raw_added_at(&db.conn, "a", "'not a date'");
        let err = db.all_sessions().unwrap_err();
        assert!(matches!(
            err,
            StoreError::InvalidDate {
                column: "session.addedAt",
                ..
            }
        ));
    }

    fn conn_insert_raw_added_at(conn: &Connection, id: &str, added_at_sql_literal: &str) {
        conn.execute_batch(&format!(
            "INSERT INTO session (id, worktreePath, name, addedAt, sortOrder)
             VALUES ('{id}', '/tmp/{id}', '{id}', {added_at_sql_literal}, 0)"
        ))
        .unwrap();
    }

    #[test]
    fn open_creates_parent_directory_and_persists_across_reopen() {
        let dir = std::env::temp_dir().join(format!("labolabo-store-test-{}", std::process::id()));
        let db_path = dir.join("nested").join("labolabo.db");
        let _ = std::fs::remove_dir_all(&dir);

        {
            let db = SessionDatabase::open(&db_path).unwrap();
            db.upsert(&sample("a", 0)).unwrap();
        }
        {
            let db = SessionDatabase::open(&db_path).unwrap();
            assert_eq!(db.all_sessions().unwrap().len(), 1);
        }

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
