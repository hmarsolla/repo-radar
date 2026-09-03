//! Stage 4 (DESIGN §6.2): match every dependency against the advisory
//! database, score the repo, and persist `findings` + the health columns.
//!
//! **This runs for every repo on every scan**, including
//! fingerprint-unchanged ones (DESIGN §6.5) — the advisory database moves
//! independently of the code, so a repo nobody touched can become
//! unhealthy. That is Journey B; M2-18's test guards it.

use rusqlite::Connection;

use crate::db::advisories::{advisory_count, candidates_for_repo};
use crate::error::CoreResult;
use crate::model::{Confidence, FindingKind, HealthResult, Warning, WarningKind, WarningScope};
use crate::osv::matcher::{evaluate, Match, Target};
use crate::score::{score, HygieneInputs, ScoredFinding, Weights};

/// Result of scoring one repo: its health plus any "could not check"
/// warnings (unparseable dependency versions, DESIGN §8.5).
pub struct RepoScore {
    pub health: HealthResult,
    pub warnings: Vec<Warning>,
}

/// Match, score, and persist for one repo. Idempotent: replaces the repo's
/// `findings` rows and health columns each call.
pub fn match_score_and_persist(
    conn: &mut Connection,
    repo_id: i64,
    repo_path: &str,
    weights: &Weights,
) -> CoreResult<RepoScore> {
    let advisories_synced = advisory_count(conn)? > 0;
    let hygiene = load_hygiene(conn, repo_id)?;

    let deps = candidates_for_repo(conn, repo_id)?;
    let mut scored: Vec<ScoredFinding> = Vec::new();
    let mut finding_rows: Vec<FindingInsert> = Vec::new();
    let mut warnings: Vec<Warning> = Vec::new();

    for (dep, candidates) in &deps {
        for sc in candidates {
            let target = Target {
                ecosystem: dep.ecosystem,
                name: &dep.name,
                version: &dep.version,
            };
            match evaluate(&target, &sc.candidate) {
                Match::Affected { fixed_version } => {
                    scored.push(ScoredFinding {
                        ecosystem: dep.ecosystem,
                        package_name: dep.name.clone(),
                        advisory_id: sc.candidate.advisory_id.clone(),
                        kind: sc.candidate.kind,
                        severity: sc.severity,
                        confidence: dep.confidence,
                        scope: dep.scope,
                        suppressed: false,
                    });
                    finding_rows.push(FindingInsert {
                        dependency_id: dep.dependency_id,
                        advisory_id: sc.candidate.advisory_id.clone(),
                        kind: sc.candidate.kind,
                        confidence: dep.confidence,
                        fixed_version,
                    });
                }
                Match::NotAffected => {}
                Match::Unmatchable(err) => {
                    warnings.push(Warning::new(
                        WarningScope::Repo(repo_path.to_string()),
                        WarningKind::UnparseableVersion,
                        format!(
                            "{}@{} could not be checked against advisories: {}",
                            dep.name, dep.version, err
                        ),
                    ));
                }
            }
        }
    }

    let health = score(&scored, &hygiene, weights, advisories_synced);

    // -- persist -------------------------------------------------------
    let tx = conn.transaction()?;
    tx.execute("DELETE FROM findings WHERE repo_id = ?1", [repo_id])?;

    // Per-advisory deduction from the breakdown, for the row's `deduction`.
    let mut ded_by_advisory: std::collections::HashMap<&str, f32> =
        std::collections::HashMap::new();
    for d in &health.breakdown {
        if let crate::model::DeductionCause::Advisory(id) = &d.cause {
            *ded_by_advisory.entry(id.as_str()).or_default() += d.amount;
        }
    }

    {
        let mut stmt = tx.prepare_cached(
            "INSERT OR IGNORE INTO findings
                (repo_id, dependency_id, advisory_id, kind, confidence, fixed_version, deduction)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )?;
        for f in &finding_rows {
            stmt.execute(rusqlite::params![
                repo_id,
                f.dependency_id,
                f.advisory_id,
                kind_str(f.kind),
                conf_str(f.confidence),
                f.fixed_version,
                *ded_by_advisory.get(f.advisory_id.as_str()).unwrap_or(&0.0) as f64,
            ])?;
        }
    }

    tx.execute(
        "UPDATE repos
            SET health_score = ?2, health_band = ?3, health_breakdown = ?4
          WHERE id = ?1",
        rusqlite::params![
            repo_id,
            health.score as i64,
            band_str(health.band),
            serde_json::to_string(&health.breakdown).unwrap_or_else(|_| "[]".into()),
        ],
    )?;
    tx.commit()?;

    Ok(RepoScore { health, warnings })
}

struct FindingInsert {
    dependency_id: i64,
    advisory_id: String,
    kind: FindingKind,
    confidence: Confidence,
    fixed_version: Option<String>,
}

fn load_hygiene(conn: &Connection, repo_id: i64) -> CoreResult<HygieneInputs> {
    let (last_commit_at, dirt): (Option<String>, i64) = conn.query_row(
        "SELECT last_commit_at,
                COALESCE(dirty_modified,0)+COALESCE(dirty_staged,0)+COALESCE(dirty_untracked,0)
           FROM repos WHERE id = ?1",
        [repo_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let has_lockfile: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM manifests WHERE repo_id = ?1 AND kind = 'lockfile')",
            [repo_id],
            |r| r.get::<_, i64>(0),
        )
        .map(|n| n != 0)
        .unwrap_or(false);

    let days_since_commit = last_commit_at
        .as_deref()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| {
            (chrono::Utc::now() - dt.with_timezone(&chrono::Utc))
                .num_days()
                .max(0) as u32
        });

    Ok(HygieneInputs {
        has_lockfile,
        days_since_commit,
        dirty: dirt > 0,
    })
}

/// Which repos a given advisory currently affects — the Advisories screen's
/// cross-repo impact view (uses `idx_findings_advisory`).
pub fn repos_affected_by(conn: &Connection, advisory_id: &str) -> CoreResult<Vec<(i64, String)>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT r.id, r.name
           FROM findings f JOIN repos r ON r.id = f.repo_id
          WHERE f.advisory_id = ?1
          ORDER BY r.name",
    )?;
    let rows = stmt
        .query_map([advisory_id], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn kind_str(k: FindingKind) -> &'static str {
    match k {
        FindingKind::Compromise => "compromise",
        FindingKind::Vulnerability => "vulnerability",
    }
}
fn conf_str(c: Confidence) -> &'static str {
    match c {
        Confidence::Exact => "exact",
        Confidence::Range => "range",
    }
}
fn band_str(b: crate::model::Band) -> &'static str {
    match b {
        crate::model::Band::Unknown => "unknown",
        crate::model::Band::Critical => "critical",
        crate::model::Band::Poor => "poor",
        crate::model::Band::Fair => "fair",
        crate::model::Band::Good => "good",
        crate::model::Band::Excellent => "excellent",
    }
}
