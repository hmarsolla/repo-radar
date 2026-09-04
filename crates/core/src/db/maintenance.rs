//! Database maintenance — the **Reset database** action (FR-10.3).
//!
//! Every table repo-radar writes holds *derived* data: rebuildable from the
//! user's repositories (a re-scan) or from OSV (a re-sync). That is what
//! makes a hard reset an acceptable recovery path for a corrupt or
//! incompatible database (DESIGN §5.2, §15).
//!
//! What a reset keeps: the configured `scan_roots` (they are settings, not
//! results) and the `schema_version` ledger. Everything else goes.

use rusqlite::Connection;

use crate::error::CoreResult;

/// Clear all scan results, findings, advisories, and caches, keeping only
/// the configured scan roots and the schema version. Runs in one
/// transaction; `VACUUM` afterwards reclaims the file space.
pub fn reset_derived_data(conn: &mut Connection) -> CoreResult<()> {
    {
        let tx = conn.transaction()?;
        // `repos` cascades to repo_languages, repo_technologies, manifests,
        // dependencies, and findings (all FK `ON DELETE CASCADE`).
        tx.execute("DELETE FROM repos", [])?;
        // `advisories` cascades to affected_ranges and affected_versions.
        tx.execute("DELETE FROM advisories", [])?;
        tx.execute("DELETE FROM outdated_cache", [])?;
        tx.execute("DELETE FROM scans", [])?;
        tx.execute("DELETE FROM sync_log", [])?;
        tx.commit()?;
    }
    // VACUUM cannot run inside a transaction.
    conn.execute_batch("VACUUM")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::db::Db;

    #[test]
    fn reset_clears_derived_data_but_keeps_scan_roots() {
        let db = Db::open_in_memory().unwrap();
        db.write(|c| {
            c.execute_batch(
                r#"
                INSERT INTO scan_roots (id, path, added_at) VALUES (1, '/r', '2026-01-01T00:00:00Z');
                INSERT INTO repos (id, root_id, path, name) VALUES (1, 1, '/r/a', 'a');
                INSERT INTO manifests (id, repo_id, path, ecosystem, kind, content_hash)
                    VALUES (1, 1, 'Cargo.lock', 'crates.io', 'lockfile', 'h');
                INSERT INTO dependencies (id, repo_id, manifest_id, ecosystem, name, raw_name, version, confidence, scope, is_direct)
                    VALUES (1, 1, 1, 'crates.io', 'serde', 'serde', '1.0.0', 'exact', 'runtime', 1);
                INSERT INTO advisories (id, kind, severity, modified)
                    VALUES ('MAL-1', 'compromise', 'critical', '2026-01-01T00:00:00Z');
                INSERT INTO affected_versions (advisory_id, ecosystem, package_name, version)
                    VALUES ('MAL-1', 'crates.io', 'serde', '1.0.0');
                INSERT INTO outdated_cache (ecosystem, name, latest_version, checked_at)
                    VALUES ('crates.io', 'serde', '1.0.9', '2026-01-01T00:00:00Z');
                INSERT INTO scans (id, started_at, status) VALUES (1, '2026-01-01T00:00:00Z', 'complete');
                "#,
            )
            .map_err(Into::into)
        })
        .unwrap();

        db.write(super::reset_derived_data).unwrap();

        let conn = db.read().unwrap();
        for table in [
            "repos",
            "dependencies",
            "manifests",
            "advisories",
            "affected_versions",
            "outdated_cache",
            "scans",
        ] {
            let n: i64 = conn
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                .unwrap();
            assert_eq!(n, 0, "{table} should be empty after reset");
        }
        let roots: i64 = conn
            .query_row("SELECT COUNT(*) FROM scan_roots", [], |r| r.get(0))
            .unwrap();
        assert_eq!(roots, 1, "scan roots are kept");
    }
}
