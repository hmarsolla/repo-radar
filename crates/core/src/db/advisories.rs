//! Query module for `advisories`, `affected_ranges`, and `affected_versions`.
//!
//! Holds the matcher's **Phase 1 narrow** (DESIGN §8.4): a repo's
//! dependencies joined against advisory ranges/versions on the exact
//! normalized `(ecosystem, package_name)`. The version-interval decision
//! happens in Rust ([`crate::osv::matcher`]); this only cheaply narrows.
//! Full sync writes land in [`crate::osv::sync`].

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::error::CoreResult;
use crate::model::{Confidence, Ecosystem, FindingKind, Freshness, Scope, Severity};
use crate::osv::matcher::Candidate;
use crate::osv::record::{
    parse_events_json, MalCoverage, NormalizedAffected, NormalizedRange, RangeType,
};

/// A repo dependency row as the matcher needs it.
#[derive(Debug, Clone)]
pub struct DepRow {
    pub dependency_id: i64,
    pub ecosystem: Ecosystem,
    pub name: String,
    pub version: String,
    pub confidence: Confidence,
    pub scope: Scope,
    pub is_direct: bool,
}

/// A candidate advisory plus its stored severity (for scoring).
#[derive(Debug, Clone)]
pub struct SevCandidate {
    pub candidate: Candidate,
    pub severity: Severity,
}

/// Every dependency of `repo_id`, paired with its candidate advisories
/// (already filtered to `withdrawn IS NULL`). A dependency with no
/// candidates still appears, with an empty vec.
pub fn candidates_for_repo(
    conn: &Connection,
    repo_id: i64,
) -> CoreResult<Vec<(DepRow, Vec<SevCandidate>)>> {
    // 1. The repo's dependencies.
    let mut dep_stmt = conn.prepare(
        "SELECT id, ecosystem, name, version, confidence, scope, is_direct
           FROM dependencies WHERE repo_id = ?1",
    )?;
    let deps: Vec<DepRow> = dep_stmt
        .query_map([repo_id], |r| {
            Ok(DepRow {
                dependency_id: r.get("id")?,
                ecosystem: eco_from_str(&r.get::<_, String>("ecosystem")?),
                name: r.get("name")?,
                version: r.get("version")?,
                confidence: conf_from_str(&r.get::<_, String>("confidence")?),
                scope: scope_from_str(&r.get::<_, String>("scope")?),
                is_direct: r.get::<_, i64>("is_direct")? != 0,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;

    // 2. Ranges narrowed by (ecosystem, name) for this repo.
    let mut range_stmt = conn.prepare(
        "SELECT d.name AS dep_name, d.ecosystem AS eco,
                a.id AS advisory_id, a.kind AS kind, a.severity AS severity,
                ar.range_type AS range_type, ar.events AS events
           FROM dependencies d
           JOIN affected_ranges ar
             ON ar.ecosystem = d.ecosystem AND ar.package_name = d.name
           JOIN advisories a
             ON a.id = ar.advisory_id AND a.withdrawn IS NULL
          WHERE d.repo_id = ?1",
    )?;
    // 3. Explicit versions, same narrow.
    let mut ver_stmt = conn.prepare(
        "SELECT d.name AS dep_name, d.ecosystem AS eco,
                a.id AS advisory_id, a.kind AS kind, a.severity AS severity,
                av.version AS version
           FROM dependencies d
           JOIN affected_versions av
             ON av.ecosystem = d.ecosystem AND av.package_name = d.name
           JOIN advisories a
             ON a.id = av.advisory_id AND a.withdrawn IS NULL
          WHERE d.repo_id = ?1",
    )?;

    // key: (ecosystem, name, advisory_id) -> (kind, severity, NormalizedAffected)
    use std::collections::HashMap;
    let mut acc: HashMap<(String, String, String), (FindingKind, Severity, NormalizedAffected)> =
        HashMap::new();
    let blank = |eco_s: &str, name: &str, kind, sev| {
        (
            kind,
            sev,
            NormalizedAffected {
                ecosystem: eco_from_str(eco_s),
                package_name: name.to_string(),
                ranges: vec![],
                versions: vec![],
            },
        )
    };

    let mut range_rows = range_stmt.query([repo_id])?;
    while let Some(row) = range_rows.next()? {
        let name: String = row.get("dep_name")?;
        let eco_s: String = row.get("eco")?;
        let advisory_id: String = row.get("advisory_id")?;
        let kind = kind_from_str(&row.get::<_, String>("kind")?);
        let sev = sev_from_str(&row.get::<_, String>("severity")?);
        let range_type = match row.get::<_, String>("range_type")?.as_str() {
            "SEMVER" => RangeType::Semver,
            "ECOSYSTEM" => RangeType::Ecosystem,
            _ => RangeType::Git,
        };
        let events = parse_events_json(&row.get::<_, String>("events")?);
        acc.entry((eco_s.clone(), name.clone(), advisory_id))
            .or_insert_with(|| blank(&eco_s, &name, kind, sev))
            .2
            .ranges
            .push(NormalizedRange { range_type, events });
    }

    let mut ver_rows = ver_stmt.query([repo_id])?;
    while let Some(row) = ver_rows.next()? {
        let name: String = row.get("dep_name")?;
        let eco_s: String = row.get("eco")?;
        let advisory_id: String = row.get("advisory_id")?;
        let kind = kind_from_str(&row.get::<_, String>("kind")?);
        let sev = sev_from_str(&row.get::<_, String>("severity")?);
        let version: String = row.get("version")?;
        acc.entry((eco_s.clone(), name.clone(), advisory_id))
            .or_insert_with(|| blank(&eco_s, &name, kind, sev))
            .2
            .versions
            .push(version);
    }

    // Group candidates back onto their dependency.
    let mut by_dep: std::collections::HashMap<(String, String), Vec<SevCandidate>> =
        std::collections::HashMap::new();
    for ((eco_s, name, advisory_id), (kind, severity, affected)) in acc {
        by_dep.entry((eco_s, name)).or_default().push(SevCandidate {
            candidate: Candidate {
                advisory_id,
                kind,
                affected,
            },
            severity,
        });
    }

    Ok(deps
        .into_iter()
        .map(|d| {
            let cands = by_dep
                .remove(&(eco_to_str(d.ecosystem).to_string(), d.name.clone()))
                .unwrap_or_default();
            (d, cands)
        })
        .collect())
}

/// Count of non-withdrawn advisories currently stored.
pub fn advisory_count(conn: &Connection) -> CoreResult<i64> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM advisories WHERE withdrawn IS NULL",
        [],
        |r| r.get(0),
    )?)
}

/// Per-ecosystem advisory + compromise counts for the Advisories screen.
pub fn counts_by_ecosystem(conn: &Connection) -> CoreResult<Vec<(String, i64, i64)>> {
    let mut stmt = conn.prepare(
        "SELECT ar.ecosystem,
                COUNT(DISTINCT a.id) AS total,
                COUNT(DISTINCT CASE WHEN a.kind = 'compromise' THEN a.id END) AS mal
           FROM affected_ranges ar
           JOIN advisories a ON a.id = ar.advisory_id AND a.withdrawn IS NULL
          GROUP BY ar.ecosystem
          UNION
         SELECT av.ecosystem,
                COUNT(DISTINCT a.id),
                COUNT(DISTINCT CASE WHEN a.kind = 'compromise' THEN a.id END)
           FROM affected_versions av
           JOIN advisories a ON a.id = av.advisory_id AND a.withdrawn IS NULL
          GROUP BY av.ecosystem",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

// ---------------------------------------------------------------------------
// Sync status (FR-5.6, M2-20 / M2-22)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    /// RFC 3339 time of the most recent `complete` sync, any ecosystem.
    pub last_success: Option<String>,
    /// `false` on a fresh install — health is *unknown*, not healthy
    /// (DESIGN §14.4, M2-22).
    pub ever_synced: bool,
    pub advisory_count: i64,
    pub freshness: Freshness,
    /// The most recent sync error, if the last attempt failed.
    pub last_error: Option<String>,
    pub ecosystems: Vec<EcosystemStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct EcosystemStatus {
    pub ecosystem: String,
    pub advisory_count: i64,
    pub compromise_count: i64,
    pub last_success: Option<String>,
    /// `Thin` for crates.io and Go — the UI must caveat a clean result
    /// there (spike §8.2.1, D1).
    pub mal_coverage: MalCoverage,
}

pub fn sync_status(conn: &Connection) -> CoreResult<SyncStatus> {
    let advisory_count = advisory_count(conn)?;
    let last_success: Option<String> = conn
        .query_row(
            "SELECT MAX(finished_at) FROM sync_log WHERE status = 'complete'",
            [],
            |r| r.get(0),
        )
        .ok()
        .flatten();
    let last_error: Option<String> = conn
        .query_row(
            "SELECT error FROM sync_log
              WHERE status = 'failed'
                AND started_at > COALESCE((SELECT MAX(finished_at) FROM sync_log WHERE status='complete'), '')
              ORDER BY started_at DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .ok()
        .flatten();

    let freshness = match last_success.as_deref().and_then(parse_rfc3339) {
        None => Freshness::Never,
        Some(t) => {
            let age = chrono::Utc::now() - t;
            if age > chrono::Duration::days(30) {
                Freshness::VeryStale
            } else if age > chrono::Duration::days(7) {
                Freshness::Stale
            } else {
                Freshness::Fresh
            }
        }
    };

    let counts = counts_by_ecosystem(conn)?;
    let mut ecosystems = Vec::new();
    for (eco_s, total, mal) in counts {
        let Some(eco) = Ecosystem::from_osv_id(&eco_s) else {
            continue;
        };
        let eco_last: Option<String> = conn
            .query_row(
                "SELECT MAX(finished_at) FROM sync_log
                  WHERE ecosystem = ?1 AND status = 'complete'",
                [eco_s.as_str()],
                |r| r.get(0),
            )
            .ok()
            .flatten();
        ecosystems.push(EcosystemStatus {
            ecosystem: eco_s,
            advisory_count: total,
            compromise_count: mal,
            last_success: eco_last,
            mal_coverage: MalCoverage::for_ecosystem(eco),
        });
    }
    ecosystems.sort_by(|a, b| a.ecosystem.cmp(&b.ecosystem));

    Ok(SyncStatus {
        last_success,
        ever_synced: advisory_count > 0,
        advisory_count,
        freshness,
        last_error,
        ecosystems,
    })
}

fn parse_rfc3339(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(s.trim())
        .ok()
        .map(|d| d.with_timezone(&chrono::Utc))
}

// -- string <-> enum helpers (DB stores lowercase strings) -----------------

pub(crate) fn eco_from_str(s: &str) -> Ecosystem {
    Ecosystem::from_osv_id(s).unwrap_or(Ecosystem::Npm)
}
pub(crate) fn eco_to_str(e: Ecosystem) -> &'static str {
    e.osv_id()
}
pub(crate) fn conf_from_str(s: &str) -> Confidence {
    match s {
        "exact" => Confidence::Exact,
        _ => Confidence::Range,
    }
}
pub(crate) fn scope_from_str(s: &str) -> Scope {
    match s {
        "dev" => Scope::Dev,
        "build" => Scope::Build,
        "optional" => Scope::Optional,
        "peer" => Scope::Peer,
        _ => Scope::Runtime,
    }
}
pub(crate) fn kind_from_str(s: &str) -> FindingKind {
    match s {
        "compromise" => FindingKind::Compromise,
        _ => FindingKind::Vulnerability,
    }
}
pub(crate) fn sev_from_str(s: &str) -> Severity {
    match s {
        "critical" => Severity::Critical,
        "high" => Severity::High,
        "medium" => Severity::Medium,
        "low" => Severity::Low,
        _ => Severity::Unscored,
    }
}
