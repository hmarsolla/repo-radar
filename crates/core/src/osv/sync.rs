//! Advisory sync (FR-5, DESIGN §8.6).
//!
//! **Full sync** streams each in-use ecosystem's `all.zip` to a temp file
//! (bytes on disk, not RAM), then iterates entries one at a time,
//! normalizing and inserting inside a *single per-ecosystem transaction* —
//! atomic replacement (FR-5.7): a failed sync rolls back and the previous
//! snapshot stays intact. Deserialized records are dropped after each
//! insert, so peak memory stays proportional to a single record, not the
//! advisory count (FR-5.10).
//!
//! This is the only module in `core` that performs outbound network IO, and
//! only to the two OSV destinations in DESIGN §18.

use std::io::Read;
use std::path::{Path, PathBuf};

use rusqlite::Connection;

use crate::db::Db;
use crate::error::{CoreResult, OperationError};
use crate::model::Ecosystem;
use crate::osv::record::{normalize, NormalizedAdvisory, NormalizedRange, OsvRecord, RangeType};

const BUCKET: &str = "https://osv-vulnerabilities.storage.googleapis.com";
const USER_AGENT: &str = concat!(
    "repo-radar/",
    env!("CARGO_PKG_VERSION"),
    " (+https://osv.dev)"
);

#[derive(Debug, Clone)]
pub struct SyncOptions {
    /// Only these ecosystems are downloaded (DESIGN §8.6 — the ones in use).
    pub ecosystems: Vec<Ecosystem>,
    /// Scratch dir for the in-flight zip.
    pub cache_dir: PathBuf,
}

#[derive(Debug, Default, Clone)]
pub struct SyncSummary {
    /// `(ecosystem, advisories written)`.
    pub per_ecosystem: Vec<(Ecosystem, usize)>,
}

impl SyncSummary {
    pub fn total(&self) -> usize {
        self.per_ecosystem.iter().map(|(_, n)| n).sum()
    }
}

/// Progress sink for a sync (DESIGN §12.3). `src-tauri` emits `sync:*`
/// events; tests use a no-op.
pub trait SyncReporter: Send + Sync {
    fn phase(&self, ecosystem: Ecosystem, phase: &str, done: usize, total: usize);
}

/// A reporter that does nothing.
pub struct NoopSyncReporter;
impl SyncReporter for NoopSyncReporter {
    fn phase(&self, _: Ecosystem, _: &str, _: usize, _: usize) {}
}

/// Full sync of every ecosystem in `opts.ecosystems`.
pub fn full_sync(
    db: &Db,
    opts: &SyncOptions,
    reporter: &dyn SyncReporter,
) -> Result<SyncSummary, OperationError> {
    std::fs::create_dir_all(&opts.cache_dir)
        .map_err(|e| OperationError::Sync(format!("cache dir: {e}")))?;

    let client = reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| OperationError::Network(e.to_string()))?;

    let mut summary = SyncSummary::default();
    for &eco in &opts.ecosystems {
        let count =
            sync_one_ecosystem(db, &client, eco, &opts.cache_dir, reporter).inspect_err(|e| {
                // Record the failure; the previous snapshot is untouched
                // because the transaction rolled back.
                let _ =
                    db.write(|c| log_sync(c, eco, "full", None, "failed", Some(&e.to_string())));
            })?;
        summary.per_ecosystem.push((eco, count));
    }
    Ok(summary)
}

fn sync_one_ecosystem(
    db: &Db,
    client: &reqwest::blocking::Client,
    eco: Ecosystem,
    cache_dir: &Path,
    reporter: &dyn SyncReporter,
) -> Result<usize, OperationError> {
    let started = chrono::Utc::now();

    // --- download to a temp file (streamed) -----------------------------
    reporter.phase(eco, "download", 0, 0);
    let url = format!("{BUCKET}/{}/all.zip", eco.osv_id());
    let tmp = cache_dir.join(format!("{}.zip.part", sanitize(eco.osv_id())));
    let mut resp = client
        .get(&url)
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| OperationError::Network(format!("{url}: {e}")))?;
    {
        let mut file = std::fs::File::create(&tmp)
            .map_err(|e| OperationError::Sync(format!("create {}: {e}", tmp.display())))?;
        std::io::copy(&mut resp, &mut file)
            .map_err(|e| OperationError::Network(format!("download {url}: {e}")))?;
    }

    // --- ingest in one atomic per-ecosystem transaction ---------------
    reporter.phase(eco, "ingest", 0, 0);
    let file =
        std::fs::File::open(&tmp).map_err(|e| OperationError::Sync(format!("open zip: {e}")))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| OperationError::Sync(format!("bad zip: {e}")))?;
    let entry_count = archive.len();

    let written = db
        .write(|conn| ingest_archive(conn, eco, &mut archive, entry_count, reporter))
        .map_err(|e| OperationError::Sync(e.to_string()))?;

    let _ = std::fs::remove_file(&tmp);

    db.write(|c| log_sync(c, eco, "full", Some(written), "complete", None))
        .map_err(|e| OperationError::Sync(e.to_string()))?;

    reporter.phase(eco, "done", written, written);
    tracing::info!(
        ecosystem = eco.osv_id(),
        written,
        secs = (chrono::Utc::now() - started).num_seconds(),
        "advisory ecosystem synced"
    );
    Ok(written)
}

/// Delete this ecosystem's range/version rows and re-insert from the
/// archive, one entry at a time. Runs inside the caller's transaction.
fn ingest_archive(
    conn: &mut Connection,
    eco: Ecosystem,
    archive: &mut zip::ZipArchive<std::fs::File>,
    entry_count: usize,
    reporter: &dyn SyncReporter,
) -> CoreResult<usize> {
    let tx = conn.transaction()?;
    let eco_id = eco.osv_id();

    tx.execute("DELETE FROM affected_ranges WHERE ecosystem = ?1", [eco_id])?;
    tx.execute(
        "DELETE FROM affected_versions WHERE ecosystem = ?1",
        [eco_id],
    )?;

    let mut written = 0usize;
    let mut buf = String::new();

    for i in 0..entry_count {
        let mut entry = archive.by_index(i).map_err(|e| {
            crate::error::CoreError::Operation(OperationError::Sync(format!("zip entry {i}: {e}")))
        })?;
        if !entry.name().ends_with(".json") {
            continue;
        }
        buf.clear();
        if entry.read_to_string(&mut buf).is_err() {
            continue; // non-UTF8 / unreadable member — skip
        }
        let record: OsvRecord = match serde_json::from_str(&buf) {
            Ok(r) => r,
            Err(_) => continue, // truncated / malformed member (spike §8.2.1)
        };
        let Some((advisory, affected)) = normalize(&record) else {
            continue;
        };
        // Keep only affected entries for the ecosystem we're syncing.
        let relevant: Vec<_> = affected
            .into_iter()
            .filter(|a| a.ecosystem == eco)
            .collect();
        if relevant.is_empty() {
            continue;
        }

        upsert_advisory(&tx, &advisory)?;
        for a in &relevant {
            for range in &a.ranges {
                insert_range(&tx, &advisory.id, eco_id, &a.package_name, range)?;
            }
            for v in &a.versions {
                tx.execute(
                    "INSERT OR IGNORE INTO affected_versions
                        (advisory_id, ecosystem, package_name, version)
                     VALUES (?1, ?2, ?3, ?4)",
                    rusqlite::params![advisory.id, eco_id, a.package_name, v],
                )?;
            }
        }
        written += 1;

        if written.is_multiple_of(1000) {
            reporter.phase(eco, "ingest", i + 1, entry_count);
        }
    }

    tx.commit()?;
    Ok(written)
}

fn upsert_advisory(tx: &Connection, a: &NormalizedAdvisory) -> CoreResult<()> {
    tx.execute(
        "INSERT INTO advisories
            (id, kind, summary, details, severity, cvss_score, published, modified,
             aliases, refs, withdrawn)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
         ON CONFLICT(id) DO UPDATE SET
             kind = excluded.kind, summary = excluded.summary, details = excluded.details,
             severity = excluded.severity, cvss_score = excluded.cvss_score,
             published = excluded.published, modified = excluded.modified,
             aliases = excluded.aliases, refs = excluded.refs, withdrawn = excluded.withdrawn",
        rusqlite::params![
            a.id,
            kind_str(a.kind),
            a.summary,
            a.details,
            sev_str(a.severity),
            a.cvss_score,
            a.published.map(|d| d.to_rfc3339()),
            a.modified.to_rfc3339(),
            serde_json::to_string(&a.aliases).unwrap_or_else(|_| "[]".into()),
            serde_json::to_string(&a.references).unwrap_or_else(|_| "[]".into()),
            a.withdrawn.map(|d| d.to_rfc3339()),
        ],
    )?;
    Ok(())
}

fn insert_range(
    tx: &Connection,
    advisory_id: &str,
    eco_id: &str,
    package_name: &str,
    range: &NormalizedRange,
) -> CoreResult<()> {
    if range.range_type == RangeType::Git {
        return Ok(()); // commit hashes, not versions
    }
    let range_type = match range.range_type {
        RangeType::Semver => "SEMVER",
        RangeType::Ecosystem => "ECOSYSTEM",
        RangeType::Git => unreachable!(),
    };
    let events = serialize_events(&range.events);
    tx.execute(
        "INSERT INTO affected_ranges (advisory_id, ecosystem, package_name, range_type, events)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![advisory_id, eco_id, package_name, range_type, events],
    )?;
    Ok(())
}

/// Serialize events back to the OSV `[{"introduced":"0"},{"fixed":"1.2.3"}]`
/// shape the matcher's Phase-1 query reads.
fn serialize_events(events: &[crate::osv::record::RangeEvent]) -> String {
    use crate::osv::record::RangeEvent::*;
    let arr: Vec<serde_json::Value> = events
        .iter()
        .map(|e| match e {
            Introduced(v) => serde_json::json!({ "introduced": v }),
            Fixed(v) => serde_json::json!({ "fixed": v }),
            LastAffected(v) => serde_json::json!({ "last_affected": v }),
            Limit(v) => serde_json::json!({ "limit": v }),
        })
        .collect();
    serde_json::to_string(&arr).unwrap_or_else(|_| "[]".into())
}

fn log_sync(
    conn: &Connection,
    eco: Ecosystem,
    mode: &str,
    count: Option<usize>,
    status: &str,
    error: Option<&str>,
) -> CoreResult<()> {
    conn.execute(
        "INSERT INTO sync_log (ecosystem, started_at, finished_at, mode, count, status, error)
         VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![
            eco.osv_id(),
            chrono::Utc::now().to_rfc3339(),
            mode,
            count.map(|c| c as i64),
            status,
            error,
        ],
    )?;
    Ok(())
}

fn kind_str(k: crate::model::FindingKind) -> &'static str {
    match k {
        crate::model::FindingKind::Compromise => "compromise",
        crate::model::FindingKind::Vulnerability => "vulnerability",
    }
}

fn sev_str(s: crate::model::Severity) -> &'static str {
    match s {
        crate::model::Severity::Unscored => "unscored",
        crate::model::Severity::Low => "low",
        crate::model::Severity::Medium => "medium",
        crate::model::Severity::High => "high",
        crate::model::Severity::Critical => "critical",
    }
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

/// Which ecosystems currently have at least one dependency in the database —
/// the set worth syncing (DESIGN §8.6).
pub fn ecosystems_in_use(db: &Db) -> CoreResult<Vec<Ecosystem>> {
    let conn = db.read()?;
    let mut stmt = conn.prepare("SELECT DISTINCT ecosystem FROM dependencies")?;
    let rows: Vec<String> = stmt
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(rows
        .iter()
        .filter_map(|s| Ecosystem::from_osv_id(s))
        .collect())
}
