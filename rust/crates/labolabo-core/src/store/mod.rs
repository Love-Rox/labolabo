//! Wave 4c: port of `Sources/LaboLaboStore/` (session persistence — SQLite
//! via GRDB in Swift, `rusqlite` here).
//!
//! | Swift source | Rust module |
//! |---|---|
//! | `SessionRecord.swift` | `record.rs` |
//! | `SessionDatabase.swift` | `database.rs` (+ `datetime.rs` for the `Date` compatibility contract) |
//! | `SessionPersisting.swift` | `persisting.rs` |
//! | `AppDataDirectory.swift` | `data_dir.rs` |
//!
//! Unlike every wave-1/2/3 module, this one is fallible I/O (SQLite),
//! not a pure parser — see `error.rs` for the shared `StoreError`/
//! `StoreResult`, and `database.rs`'s module doc comment for the full GRDB
//! on-disk compatibility contract (schema reconciliation, `grdb_migrations`
//! handling, `Date` column read/write semantics).

mod data_dir;
mod database;
mod datetime;
mod error;
mod persisting;
mod record;

pub use data_dir::app_data_dir;
pub use database::SessionDatabase;
pub use error::{StoreError, StoreResult};
pub use persisting::SessionPersisting;
pub use record::SessionRecord;
