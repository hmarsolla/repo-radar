//! The database layer (DESIGN §5).
//!
//! [`Db`] owns the connection strategy from [`pool`] and runs [`migrations`]
//! on open. Query logic lives in one module per aggregate ([`repos`],
//! [`advisories`], [`findings`]); those modules take a `&Connection` or a
//! [`pool::PooledConn`] and never reach for global state.

pub mod advisories;
pub mod dashboard;
pub mod findings;
pub mod maintenance;
pub mod migrations;
pub mod pool;
pub mod repos;
pub mod scans;

use std::path::Path;

use crate::error::CoreResult;
use pool::{PooledConn, Pools};

/// The application's handle to persistent storage.
pub struct Db {
    pools: Pools,
}

impl Db {
    /// Open (creating if needed) the database at `path` and bring the schema
    /// up to date.
    pub fn open(path: &Path) -> CoreResult<Self> {
        let pools = Pools::open_file(path)?;
        let db = Self { pools };
        db.pools.with_write(migrations::run)?;
        Ok(db)
    }

    /// Open a fresh in-memory database with migrations applied. Tests only
    /// (DESIGN §16.5).
    pub fn open_in_memory() -> CoreResult<Self> {
        let pools = Pools::open_in_memory()?;
        let db = Self { pools };
        db.pools.with_write(migrations::run)?;
        Ok(db)
    }

    /// Borrow a read connection from the pool.
    pub fn read(&self) -> CoreResult<PooledConn> {
        self.pools.read()
    }

    /// Run a closure holding the single write connection.
    pub fn write<T>(
        &self,
        f: impl FnOnce(&mut rusqlite::Connection) -> CoreResult<T>,
    ) -> CoreResult<T> {
        self.pools.with_write(f)
    }

    /// The schema version currently recorded in the database.
    pub fn schema_version(&self) -> CoreResult<i64> {
        let conn = self.read()?;
        migrations::current(&conn)
    }

    /// Access the underlying pools (for the scan writer thread, which needs
    /// the raw write `Arc<Mutex<_>>`).
    pub fn pools(&self) -> &Pools {
        &self.pools
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// M0-4 done-when: migrations apply to a fresh database, are idempotent
    /// on re-open, and a row round-trips through every table.
    #[test]
    fn migrations_apply_and_are_idempotent() {
        let db = Db::open_in_memory().expect("open");
        assert_eq!(db.schema_version().unwrap(), migrations::CURRENT_VERSION);

        // Re-running the runner on the same connection changes nothing.
        db.write(migrations::run).expect("rerun");
        assert_eq!(db.schema_version().unwrap(), migrations::CURRENT_VERSION);
    }

    #[test]
    fn every_table_round_trips_a_row() {
        let db = Db::open_in_memory().expect("open");
        db.write(|c| {
            let tx = c.transaction()?;
            tx.execute_batch(
                r#"
                INSERT INTO scan_roots (id, path, added_at) VALUES (1, '/tmp/root', '2026-01-01T00:00:00Z');
                INSERT INTO repos (id, root_id, path, name) VALUES (1, 1, '/tmp/root/a', 'a');
                INSERT INTO repo_languages (repo_id, language, code_lines, comment_lines, files, percentage)
                    VALUES (1, 'Rust', 100, 10, 3, 87.5);
                INSERT INTO repo_technologies (repo_id, tech, kind, evidence)
                    VALUES (1, 'Tauri', 'framework', '["dependency: tauri"]');
                INSERT INTO manifests (id, repo_id, path, ecosystem, kind, content_hash)
                    VALUES (1, 1, 'Cargo.lock', 'crates.io', 'lockfile', 'abc123');
                INSERT INTO dependencies (id, repo_id, manifest_id, ecosystem, name, raw_name, version, confidence, scope, is_direct)
                    VALUES (1, 1, 1, 'crates.io', 'serde', 'serde', '1.0.0', 'exact', 'runtime', 1);
                INSERT INTO advisories (id, kind, severity, modified)
                    VALUES ('MAL-2026-0001', 'compromise', 'critical', '2026-01-01T00:00:00Z');
                INSERT INTO affected_ranges (id, advisory_id, ecosystem, package_name, range_type, events)
                    VALUES (1, 'MAL-2026-0001', 'crates.io', 'serde', 'SEMVER', '[{"introduced":"0"}]');
                INSERT INTO affected_versions (advisory_id, ecosystem, package_name, version)
                    VALUES ('MAL-2026-0001', 'crates.io', 'serde', '1.0.0');
                INSERT INTO findings (id, repo_id, dependency_id, advisory_id, kind, confidence, deduction)
                    VALUES (1, 1, 1, 'MAL-2026-0001', 'compromise', 'exact', 61.0);
                INSERT INTO outdated_cache (ecosystem, name, latest_version, checked_at)
                    VALUES ('crates.io', 'serde', '1.0.200', '2026-01-01T00:00:00Z');
                INSERT INTO scans (id, started_at, status) VALUES (1, '2026-01-01T00:00:00Z', 'complete');
                INSERT INTO sync_log (id, ecosystem, started_at, mode, status)
                    VALUES (1, 'crates.io', '2026-01-01T00:00:00Z', 'full', 'complete');
                "#,
            )?;
            tx.commit()?;
            Ok(())
        })
        .expect("insert");

        let conn = db.read().expect("read conn");
        for (table, expected) in [
            ("scan_roots", 1),
            ("repos", 1),
            ("repo_languages", 1),
            ("repo_technologies", 1),
            ("manifests", 1),
            ("dependencies", 1),
            ("advisories", 1),
            ("affected_ranges", 1),
            ("affected_versions", 1),
            ("findings", 1),
            ("outdated_cache", 1),
            ("scans", 1),
            ("sync_log", 1),
        ] {
            let n: i64 = conn
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                .unwrap();
            assert_eq!(n, expected, "row count for {table}");
        }
    }

    #[test]
    fn foreign_keys_are_enforced() {
        let db = Db::open_in_memory().expect("open");
        let err = db.write(|c| {
            c.execute(
                "INSERT INTO repos (root_id, path, name) VALUES (999, '/x', 'x')",
                [],
            )
            .map_err(Into::into)
        });
        assert!(err.is_err(), "insert with dangling root_id must fail");
    }
}
