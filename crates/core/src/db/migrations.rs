//! Migration runner (DESIGN §5.2).
//!
//! Sequential numbered SQL files embedded with `include_str!`, applied in
//! order inside a transaction each, tracked in `schema_version` (one row per
//! applied version). No ORM, no framework. Down-migrations are not
//! supported — the recovery path for an incompatible database is **Reset
//! database** (FR-10.3), acceptable for derived data.

use rusqlite::Connection;

use crate::error::{CoreError, CoreResult, FatalError};

/// The highest schema version this binary understands. Bump when adding a
/// migration below.
pub const CURRENT_VERSION: i64 = 1;

/// `(version, sql)` in ascending order. Each entry runs exactly once.
const MIGRATIONS: &[(i64, &str)] = &[(1, include_str!("../../migrations/0001_init.sql"))];

/// Apply every migration not yet recorded in `schema_version`. Idempotent:
/// calling it on an up-to-date database is a no-op.
pub fn run(conn: &mut Connection) -> CoreResult<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_version (
             version    INTEGER PRIMARY KEY,
             applied_at TEXT NOT NULL
         );",
    )?;

    let applied_max: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_version",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);

    if applied_max > CURRENT_VERSION {
        return Err(CoreError::Fatal(FatalError::SchemaTooNew {
            found: applied_max,
            supported: CURRENT_VERSION,
        }));
    }

    for &(version, sql) in MIGRATIONS {
        if version <= applied_max {
            continue;
        }
        let tx = conn.transaction()?;
        tx.execute_batch(sql)
            .map_err(|source| CoreError::Fatal(FatalError::MigrationFailed { version, source }))?;
        tx.execute(
            "INSERT INTO schema_version (version, applied_at) VALUES (?1, ?2)",
            rusqlite::params![version, chrono::Utc::now().to_rfc3339()],
        )?;
        tx.commit()?;
        tracing::info!(version, "applied migration");
    }

    Ok(())
}

/// The current schema version recorded in the database (0 if none).
pub fn current(conn: &Connection) -> CoreResult<i64> {
    Ok(conn
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_version",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0))
}
