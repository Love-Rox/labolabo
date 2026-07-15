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

// Wave 5b-3 (`plans/012-task-model-and-control-cli.md` §1's Task model).
// Appended at the tail rather than interleaved above, same
// minimize-merge-conflicts reasoning as `lib.rs`'s wave-4a/4b/4c blocks:
// `Task`/`TaskDatabase` have no Swift/GRDB counterpart (see
// `task_database`'s module doc comment) and share nothing with the
// `session`/`appState`-v3 code above beyond this file and `error.rs`.
mod task_database;
mod task_record;

pub use data_dir::rust_app_data_dir;
pub use task_database::TaskDatabase;
pub use task_record::{Task, TaskKind, TaskStatus};
