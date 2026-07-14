//! Faithful port of `Sources/LaboLaboStore/SessionPersisting.swift`.
//!
//! The persistence operations the app actually uses, decoupled from the
//! concrete storage engine. `SessionDatabase` (`rusqlite`) is the only
//! conformer today; call sites should depend on this trait (and the plain
//! `SessionRecord` type), never on `SessionDatabase`/`rusqlite` directly —
//! same rationale as the Swift original (GRDB is unsupported on Windows and
//! unofficial on Linux; swapping storage engines later only needs a new
//! conformer here).

use std::collections::HashMap;

use super::error::StoreResult;
use super::record::SessionRecord;

pub trait SessionPersisting {
    // MARK: - Sessions

    fn all_sessions(&self) -> StoreResult<Vec<SessionRecord>>;
    fn upsert(&self, record: &SessionRecord) -> StoreResult<()>;
    fn delete_session(&self, id: &str) -> StoreResult<()>;

    // MARK: - App state (e.g. last selection)

    fn set_selected_session_id(&self, id: Option<&str>) -> StoreResult<()>;
    fn selected_session_id(&self) -> StoreResult<Option<String>>;

    // MARK: - Generic key-value app state

    fn set_app_state(&self, value: Option<&str>, key: &str) -> StoreResult<()>;
    fn app_state(&self, key: &str) -> StoreResult<Option<String>>;
    /// `prefix` で始まるキーの全エントリ（キー→値）。
    fn app_state_entries(&self, prefix: &str) -> StoreResult<HashMap<String, String>>;
}

impl SessionPersisting for super::database::SessionDatabase {
    fn all_sessions(&self) -> StoreResult<Vec<SessionRecord>> {
        super::database::SessionDatabase::all_sessions(self)
    }

    fn upsert(&self, record: &SessionRecord) -> StoreResult<()> {
        super::database::SessionDatabase::upsert(self, record)
    }

    fn delete_session(&self, id: &str) -> StoreResult<()> {
        super::database::SessionDatabase::delete_session(self, id)
    }

    fn set_selected_session_id(&self, id: Option<&str>) -> StoreResult<()> {
        super::database::SessionDatabase::set_selected_session_id(self, id)
    }

    fn selected_session_id(&self) -> StoreResult<Option<String>> {
        super::database::SessionDatabase::selected_session_id(self)
    }

    fn set_app_state(&self, value: Option<&str>, key: &str) -> StoreResult<()> {
        super::database::SessionDatabase::set_app_state(self, value, key)
    }

    fn app_state(&self, key: &str) -> StoreResult<Option<String>> {
        super::database::SessionDatabase::app_state(self, key)
    }

    fn app_state_entries(&self, prefix: &str) -> StoreResult<HashMap<String, String>> {
        super::database::SessionDatabase::app_state_entries(self, prefix)
    }
}
