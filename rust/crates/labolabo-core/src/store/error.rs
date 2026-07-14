//! Error type for `store::database`. Swift's `SessionPersisting` conformers
//! throw plain `Error` (GRDB's `DatabaseError`, mostly); this enum plays the
//! same role, adding a variant for the one failure mode Swift's `Date`
//! decoding has no analogue for spelling out explicitly (an unparseable
//! stored date — see `database::parse_grdb_datetime`), since Rust has no
//! implicit `Decoder`/`as?` bridging to fall back on.

use std::fmt;

#[derive(Debug)]
pub enum StoreError {
    Sqlite(rusqlite::Error),
    Io(std::io::Error),
    /// A `session.addedAt` (or other `DATETIME` column) value could not be
    /// parsed under GRDB's `Date` decoding contract (see
    /// `database::parse_grdb_datetime`). Carries the table/column and the
    /// raw stored text for diagnostics.
    InvalidDate {
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
                write!(
                    f,
                    "column {column:?} holds an unparseable GRDB date: {raw:?}"
                )
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
