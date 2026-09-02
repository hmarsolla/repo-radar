//! Error taxonomy (DESIGN §15).
//!
//! repo-radar sorts failures into three tiers, and the type a function
//! returns says which tier it is in:
//!
//! | Tier | Type | Handling |
//! |------|------|----------|
//! | Recoverable, per-item | [`crate::model::Warning`] | recorded, surfaced, processing continues |
//! | Recoverable, per-operation | [`OperationError`] | operation marked failed in its log table, previous state kept, user notified |
//! | Fatal | [`FatalError`] | error screen offering **Reset database** and **Open data folder** |
//!
//! [`CoreError`] is the crate-wide sum used by call sites that can surface
//! any of the above (command handlers, `CoreContext::new`). Per-module error
//! enums (`ParseError`, `SyncError`, `GitError`, …) live next to the code
//! that produces them and convert into this on the way out.

use std::path::Path;

/// Convenience alias for fallible core operations that may fail in any tier.
pub type CoreResult<T> = Result<T, CoreError>;

/// Crate-wide error sum. Individual subsystems return their own narrower
/// enums; this is what a Tauri command or `CoreContext::new` yields.
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    /// Unrecoverable: the app cannot continue without user intervention.
    #[error(transparent)]
    Fatal(#[from] FatalError),

    /// A single operation failed; prior state is intact and the user can retry.
    #[error(transparent)]
    Operation(#[from] OperationError),

    /// SQLite reported an error that is not itself fatal (constraint
    /// violation, type mismatch in a query). A corrupt-database error is
    /// mapped to [`FatalError::DatabaseCorruption`] before it reaches here.
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),

    /// Connection-pool checkout failure.
    #[error("database pool error: {0}")]
    Pool(#[from] r2d2::Error),

    /// Filesystem error outside a per-item context (creating the data dir,
    /// reading the settings file).
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON (de)serialization of a stored blob or a config file.
    #[error("serialization error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Tier 3 — fatal. Reaching one of these means the UI drops to the
/// recovery screen (DESIGN §15, §14.4).
#[derive(Debug, thiserror::Error)]
pub enum FatalError {
    #[error("the database file is corrupt or unreadable: {0}")]
    DatabaseCorruption(String),

    #[error("database migration {version} failed: {source}")]
    MigrationFailed {
        version: i64,
        #[source]
        source: rusqlite::Error,
    },

    /// The database was written by a newer build with a higher schema
    /// version than this binary knows how to read.
    #[error("database schema version {found} is newer than supported version {supported}")]
    SchemaTooNew { found: i64, supported: i64 },
}

/// Tier 2 — a whole operation (an advisory sync, a registry lookup batch)
/// failed. The operation's log table records the failure, the previous
/// snapshot stays in use, and the user gets a retry affordance.
#[derive(Debug, thiserror::Error)]
pub enum OperationError {
    #[error("advisory sync failed: {0}")]
    Sync(String),

    #[error("registry lookup failed: {0}")]
    Registry(String),

    #[error("network unavailable: {0}")]
    Network(String),
}

impl CoreError {
    /// True when the UI must fall back to the recovery screen rather than
    /// showing an inline notice.
    pub fn is_fatal(&self) -> bool {
        matches!(self, CoreError::Fatal(_))
    }
}

/// Map a raw `rusqlite` error to a fatal error when it signals corruption or
/// a not-a-database file, leaving everything else as an ordinary
/// [`CoreError::Db`].
pub fn classify_sqlite(err: rusqlite::Error, file: &Path) -> CoreError {
    use rusqlite::ffi::ErrorCode;
    if let rusqlite::Error::SqliteFailure(e, _) = &err {
        if matches!(e.code, ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase) {
            return CoreError::Fatal(FatalError::DatabaseCorruption(format!(
                "{}: {err}",
                file.display()
            )));
        }
    }
    CoreError::Db(err)
}
