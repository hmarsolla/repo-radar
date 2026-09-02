//! Query module for the `repos` aggregate and its children (`repo_languages`,
//! `repo_technologies`, `manifests`, `dependencies`), plus the `scan_roots`
//! table that parents them.
//!
//! **Filtering and sorting for `list_repos` execute here in SQL**, not in
//! the client (DESIGN §12.1) — that is why `RepoFilter` will be a typed
//! struct. Repo query bodies land with **M1-9**; the writer's upsert path
//! with **M1-5**. The scan-root helpers below are needed now for M0-9.

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::error::CoreResult;

/// A configured scan root (FR-10.1). `id` is the SQLite rowid.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ScanRoot {
    pub id: i64,
    pub path: String,
    pub enabled: bool,
    /// RFC 3339 timestamp.
    pub added_at: String,
}

fn row_to_root(row: &rusqlite::Row<'_>) -> rusqlite::Result<ScanRoot> {
    Ok(ScanRoot {
        id: row.get("id")?,
        path: row.get("path")?,
        enabled: row.get::<_, i64>("enabled")? != 0,
        added_at: row.get("added_at")?,
    })
}

/// Every configured scan root, oldest first.
pub fn list_scan_roots(conn: &Connection) -> CoreResult<Vec<ScanRoot>> {
    let mut stmt =
        conn.prepare("SELECT id, path, enabled, added_at FROM scan_roots ORDER BY added_at, id")?;
    let rows = stmt
        .query_map([], row_to_root)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Add a scan root. Idempotent on `path` (the column is `UNIQUE`): adding an
/// existing path returns the existing row rather than erroring.
pub fn add_scan_root(conn: &Connection, path: &str) -> CoreResult<ScanRoot> {
    if let Some(existing) = conn
        .query_row(
            "SELECT id, path, enabled, added_at FROM scan_roots WHERE path = ?1",
            [path],
            row_to_root,
        )
        .optional()?
    {
        return Ok(existing);
    }

    let added_at = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO scan_roots (path, enabled, added_at) VALUES (?1, 1, ?2)",
        rusqlite::params![path, added_at],
    )?;
    let id = conn.last_insert_rowid();
    Ok(ScanRoot {
        id,
        path: path.to_string(),
        enabled: true,
        added_at,
    })
}

/// Remove a scan root by id. Cascades to its repos and their children
/// (FK `ON DELETE CASCADE`). Removing a missing id is a no-op.
pub fn remove_scan_root(conn: &Connection, id: i64) -> CoreResult<()> {
    conn.execute("DELETE FROM scan_roots WHERE id = ?1", [id])?;
    Ok(())
}

/// Enable/disable a scan root without deleting its data.
pub fn set_scan_root_enabled(conn: &Connection, id: i64, enabled: bool) -> CoreResult<()> {
    conn.execute(
        "UPDATE scan_roots SET enabled = ?2 WHERE id = ?1",
        rusqlite::params![id, enabled as i64],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    #[test]
    fn scan_roots_crud_round_trips() {
        let db = Db::open_in_memory().unwrap();

        let a = db
            .write(|c| add_scan_root(c, "/home/dev/projects"))
            .unwrap();
        assert_eq!(a.id, 1);
        assert!(a.enabled);

        // Idempotent on path.
        let again = db
            .write(|c| add_scan_root(c, "/home/dev/projects"))
            .unwrap();
        assert_eq!(again.id, a.id);

        db.write(|c| add_scan_root(c, "/work/src")).unwrap();
        let all = db.read().map(|c| list_scan_roots(&c)).unwrap().unwrap();
        assert_eq!(all.len(), 2);

        db.write(|c| set_scan_root_enabled(c, a.id, false)).unwrap();
        db.write(|c| remove_scan_root(c, a.id)).unwrap();
        let all = db.read().map(|c| list_scan_roots(&c)).unwrap().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].path, "/work/src");
    }
}
