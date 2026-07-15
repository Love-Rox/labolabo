//! Error type for `store::database` and `store::task_database`. Swift's
//! `SessionPersisting` conformers throw plain `Error` (GRDB's
//! `DatabaseError`, mostly); this enum plays the same role, adding variants
//! for failure modes Swift's `Codable`/`as?` bridging has no analogue for
//! spelling out explicitly, since Rust has no implicit `Decoder` fallback.

use std::fmt;

#[derive(Debug)]
pub enum StoreError {
    Sqlite(rusqlite::Error),
    Io(std::io::Error),
    /// A `session.addedAt` (or other `DATETIME`-ish column) value could not
    /// be parsed. Carries the table/column and the raw stored text for
    /// diagnostics. Used both by `database`'s GRDB-`Date`-compatible parser
    /// (`database::parse_grdb_datetime`) and by `task_database`'s plain
    /// RFC 3339 parser (that module has no GRDB-compatibility constraint —
    /// see its module doc comment).
    InvalidDate {
        column: &'static str,
        raw: String,
    },
    /// `task_database`: a `task.layout` column's stored text was not valid
    /// [`crate::tiling::TileLayout`] JSON.
    InvalidLayoutJson(serde_json::Error),
    /// `task_database`: a `task.kind`/`task.status` column held a string
    /// outside the fixed set this crate writes (`"worktree"`/`"attached"`,
    /// `"active"`/`"done"`/`"archived"`) — e.g. a hand-edited or
    /// future-version database. Carries the column name and raw value.
    InvalidTaskEnum {
        column: &'static str,
        raw: String,
    },
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::Sqlite(e) => write!(f, "sqlite error: {e}"),
            StoreError::Io(e) => write!(f, "io error: {e}"),
            StoreError::InvalidDate { column, raw } => {
                write!(f, "column {column:?} holds an unparseable date: {raw:?}")
            }
            StoreError::InvalidLayoutJson(e) => {
                write!(
                    f,
                    "column \"task.layout\" holds invalid TileLayout JSON: {e}"
                )
            }
            StoreError::InvalidTaskEnum { column, raw } => {
                write!(f, "column {column:?} holds an unrecognized value: {raw:?}")
            }
        }
    }
}

impl std::error::Error for StoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            StoreError::Sqlite(e) => Some(e),
            StoreError::Io(e) => Some(e),
            StoreError::InvalidDate { .. } => None,
            StoreError::InvalidLayoutJson(e) => Some(e),
            StoreError::InvalidTaskEnum { .. } => None,
        }
    }
}

impl From<rusqlite::Error> for StoreError {
    fn from(e: rusqlite::Error) -> Self {
        StoreError::Sqlite(e)
    }
}

impl From<std::io::Error> for StoreError {
    fn from(e: std::io::Error) -> Self {
        StoreError::Io(e)
    }
}

pub type StoreResult<T> = Result<T, StoreError>;
