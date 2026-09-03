//! Query helpers for the `scans` table — one row per `run_scan` invocation
//! (DESIGN §5.3). Records start time, end time, repo count, terminal status
//! (`running` → `complete` | `cancelled` | `failed`), and the JSON warning
//! list.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::error::CoreResult;
use crate::model::Warning;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum ScanStatus {
    Running,
    Complete,
    Cancelled,
    Failed,
}

impl ScanStatus {
    fn as_str(self) -> &'static str {
        match self {
            ScanStatus::Running => "running",
            ScanStatus::Complete => "complete",
            ScanStatus::Cancelled => "cancelled",
            ScanStatus::Failed => "failed",
        }
    }
}

/// Insert a `running` scan row and return its id.
pub fn begin(conn: &Connection) -> CoreResult<i64> {
    conn.execute(
        "INSERT INTO scans (started_at, status) VALUES (?1, 'running')",
        [chrono::Utc::now().to_rfc3339()],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Finalise a scan row with its terminal status, repo count, and warnings.
pub fn finish(
    conn: &Connection,
    scan_id: i64,
    status: ScanStatus,
    repo_count: usize,
    warnings: &[Warning],
) -> CoreResult<()> {
    let warnings_json = serde_json::to_string(warnings)?;
    conn.execute(
        "UPDATE scans
            SET finished_at = ?2, status = ?3, repo_count = ?4, warnings = ?5
          WHERE id = ?1",
        rusqlite::params![
            scan_id,
            chrono::Utc::now().to_rfc3339(),
            status.as_str(),
            repo_count as i64,
            warnings_json,
        ],
    )?;
    Ok(())
}

/// The most recent scan's status, if any scan has run.
pub fn latest_status(conn: &Connection) -> CoreResult<Option<ScanStatus>> {
    let row = conn
        .query_row(
            "SELECT status FROM scans ORDER BY id DESC LIMIT 1",
            [],
            |r| r.get::<_, String>(0),
        )
        .ok();
    Ok(row.and_then(|s| match s.as_str() {
        "running" => Some(ScanStatus::Running),
        "complete" => Some(ScanStatus::Complete),
        "cancelled" => Some(ScanStatus::Cancelled),
        "failed" => Some(ScanStatus::Failed),
        _ => None,
    }))
}
