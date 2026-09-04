//! Assemble a [`PromptContext`] from the database plus a caller-supplied file
//! selection (M4-1). No filesystem access happens here — [`super::selection`]
//! reads the file bodies and hands them in as [`EmbeddedFile`]s, and the
//! caller supplies each repo's bounded directory tree.

use std::collections::HashMap;

use rusqlite::Connection;

use crate::db::repos::category_from_str;
use crate::error::{CoreError, CoreResult};
use crate::model::{Category, CategoryScores, DetectedTech, Freshness, GitInfo, LanguageStat};

use super::context::{
    DependencyCounts, DependencySummary, EcosystemCount, EmbeddedFile, FindingSummary,
    HealthSummary, PromptContext, RepoContext, ScopeContext, TreeEntry,
};

/// Build the full context for one or more repos. `trees` maps a repo id to
/// its bounded directory listing; a repo missing from the map simply renders
/// with an empty tree.
pub fn build_context(
    conn: &Connection,
    repo_ids: &[i64],
    scope: ScopeContext,
    files: Vec<EmbeddedFile>,
    trees: &HashMap<i64, Vec<TreeEntry>>,
    freshness: Freshness,
) -> CoreResult<PromptContext> {
    if repo_ids.is_empty() {
        return Err(CoreError::Prompt("no repositories selected".into()));
    }
    let mut repos = Vec::with_capacity(repo_ids.len());
    for &id in repo_ids {
        repos.push(repo_context(
            conn,
            id,
            trees.get(&id).cloned().unwrap_or_default(),
        )?);
    }
    Ok(PromptContext {
        generated_at: chrono::Utc::now(),
        repos,
        scope,
        files,
        advisory_freshness: freshness,
    })
}

/// One repo's slice of the context.
pub fn repo_context(
    conn: &Connection,
    repo_id: i64,
    tree: Vec<TreeEntry>,
) -> CoreResult<RepoContext> {
    let row = conn
        .query_row(
            "SELECT name, category, category_manual, category_scores,
                    health_score, health_band,
                    head_sha, branch, last_commit_at, last_commit_summary,
                    commits_90d, commits_total, author_count,
                    dirty_modified, dirty_staged, dirty_untracked,
                    ahead, behind, remote_url, branch_count, has_stash
               FROM repos WHERE id = ?1",
            [repo_id],
            |r| {
                Ok(RepoRow {
                    name: r.get("name")?,
                    category: r.get("category")?,
                    category_manual: r.get("category_manual")?,
                    category_scores: r.get("category_scores")?,
                    health_score: r.get("health_score")?,
                    health_band: r.get("health_band")?,
                    head_sha: r.get("head_sha")?,
                    branch: r.get("branch")?,
                    last_commit_at: r.get("last_commit_at")?,
                    last_commit_summary: r.get("last_commit_summary")?,
                    commits_90d: r.get("commits_90d")?,
                    commits_total: r.get("commits_total")?,
                    author_count: r.get("author_count")?,
                    dirty_modified: r.get("dirty_modified")?,
                    dirty_staged: r.get("dirty_staged")?,
                    dirty_untracked: r.get("dirty_untracked")?,
                    ahead: r.get("ahead")?,
                    behind: r.get("behind")?,
                    remote_url: r.get("remote_url")?,
                    branch_count: r.get("branch_count")?,
                    has_stash: r.get::<_, Option<i64>>("has_stash")?,
                })
            },
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => {
                CoreError::Prompt(format!("repository {repo_id} not found"))
            }
            other => CoreError::Db(other),
        })?;

    let category = effective_category(row.category.as_deref(), row.category_manual.as_deref());
    let category_signals = signal_lines(row.category_scores.as_deref());
    let languages = load_languages(conn, repo_id)?;
    let technologies = load_technologies(conn, repo_id)?;
    let (findings, advisories_by_dep) = load_findings(conn, repo_id)?;
    let direct_dependencies = load_direct_deps(conn, repo_id, &advisories_by_dep)?;
    let dependency_counts = load_dependency_counts(conn, repo_id)?;
    let health = match (row.health_score, row.health_band.clone()) {
        (Some(score), Some(band)) if band != "unknown" => Some(HealthSummary {
            score: score.clamp(0, 100) as u8,
            band,
        }),
        _ => None,
    };
    let git = git_from_row(&row);

    Ok(RepoContext {
        name: row.name,
        category,
        category_signals,
        languages,
        technologies,
        direct_dependencies,
        dependency_counts,
        findings,
        health,
        git,
        tree,
    })
}

struct RepoRow {
    name: String,
    category: Option<String>,
    category_manual: Option<String>,
    category_scores: Option<String>,
    health_score: Option<i64>,
    health_band: Option<String>,
    head_sha: Option<String>,
    branch: Option<String>,
    last_commit_at: Option<String>,
    last_commit_summary: Option<String>,
    commits_90d: Option<i64>,
    commits_total: Option<i64>,
    author_count: Option<i64>,
    dirty_modified: Option<i64>,
    dirty_staged: Option<i64>,
    dirty_untracked: Option<i64>,
    ahead: Option<i64>,
    behind: Option<i64>,
    remote_url: Option<String>,
    branch_count: Option<i64>,
    has_stash: Option<i64>,
}

fn effective_category(computed: Option<&str>, manual: Option<&str>) -> Category {
    manual
        .and_then(category_from_str)
        .or_else(|| computed.and_then(category_from_str))
        .unwrap_or(Category::Unknown)
}

/// Turn the stored `category_scores` JSON (FR-3.6) into plain lines a model
/// can read: `"react (dependency) → Frontend +3.0"`.
fn signal_lines(scores_json: Option<&str>) -> Vec<String> {
    let Some(json) = scores_json else {
        return Vec::new();
    };
    let Ok(scores) = serde_json::from_str::<CategoryScores>(json) else {
        return Vec::new();
    };
    scores
        .fired
        .iter()
        .map(|f| {
            format!(
                "{} ({}) → {:?} {:+.1}",
                f.rule_id, f.signal, f.category, f.weight
            )
        })
        .collect()
}

fn load_languages(conn: &Connection, repo_id: i64) -> CoreResult<Vec<LanguageStat>> {
    let mut stmt = conn.prepare(
        "SELECT language, code_lines, comment_lines, files, percentage
           FROM repo_languages WHERE repo_id = ?1 ORDER BY percentage DESC",
    )?;
    let rows = stmt
        .query_map([repo_id], |r| {
            Ok(LanguageStat {
                language: r.get("language")?,
                code_lines: r.get::<_, i64>("code_lines")? as u64,
                comment_lines: r.get::<_, i64>("comment_lines")? as u64,
                files: r.get::<_, i64>("files")? as u64,
                percentage: r.get::<_, f64>("percentage")? as f32,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn load_technologies(conn: &Connection, repo_id: i64) -> CoreResult<Vec<DetectedTech>> {
    let mut stmt = conn.prepare(
        "SELECT tech, kind, evidence FROM repo_technologies
          WHERE repo_id = ?1
          ORDER BY (evidence LIKE '%\"dependency:%') DESC, tech",
    )?;
    let rows = stmt
        .query_map([repo_id], |r| {
            let evidence_json: String = r.get("evidence")?;
            Ok(DetectedTech {
                tech: r.get("tech")?,
                kind: r.get("kind")?,
                evidence: serde_json::from_str(&evidence_json).unwrap_or_default(),
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

type AdvisoriesByDep = HashMap<(String, String), Vec<String>>;

fn load_findings(
    conn: &Connection,
    repo_id: i64,
) -> CoreResult<(Vec<FindingSummary>, AdvisoriesByDep)> {
    let mut stmt = conn.prepare(
        "SELECT f.advisory_id, f.kind, f.confidence, f.fixed_version,
                d.name AS pkg, d.version AS ver,
                a.severity AS severity, a.summary AS summary
           FROM findings f
           JOIN dependencies d ON d.id = f.dependency_id
           JOIN advisories a ON a.id = f.advisory_id
          WHERE f.repo_id = ?1 AND f.suppressed = 0
          ORDER BY (f.kind = 'compromise') DESC, a.severity DESC, f.advisory_id",
    )?;
    let mut findings = Vec::new();
    let mut by_dep: AdvisoriesByDep = HashMap::new();
    let mut rows = stmt.query([repo_id])?;
    while let Some(r) = rows.next()? {
        let advisory_id: String = r.get("advisory_id")?;
        let package: String = r.get("pkg")?;
        let version: String = r.get("ver")?;
        let confidence: String = r.get("confidence")?;
        by_dep
            .entry((package.clone(), version.clone()))
            .or_default()
            .push(advisory_id.clone());
        findings.push(FindingSummary {
            advisory_id,
            kind: r.get("kind")?,
            severity: r.get("severity")?,
            package,
            version,
            fixed_version: r.get("fixed_version")?,
            summary: r.get::<_, Option<String>>("summary")?.unwrap_or_default(),
            confirmed: confidence == "exact",
        });
    }
    Ok((findings, by_dep))
}

fn load_direct_deps(
    conn: &Connection,
    repo_id: i64,
    advisories_by_dep: &AdvisoriesByDep,
) -> CoreResult<Vec<DependencySummary>> {
    let mut stmt = conn.prepare(
        "SELECT ecosystem, raw_name, name, version, scope, confidence
           FROM dependencies
          WHERE repo_id = ?1 AND is_direct = 1
          ORDER BY ecosystem, name",
    )?;
    let rows = stmt
        .query_map([repo_id], |r| {
            let name: String = r.get("name")?;
            let raw_name: String = r.get("raw_name")?;
            let version: String = r.get("version")?;
            let confidence: String = r.get("confidence")?;
            Ok((
                r.get::<_, String>("ecosystem")?,
                raw_name,
                name,
                version,
                r.get::<_, String>("scope")?,
                confidence,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    Ok(rows
        .into_iter()
        .map(|(ecosystem, raw_name, name, version, scope, confidence)| {
            let advisories = advisories_by_dep
                .get(&(name.clone(), version.clone()))
                .cloned()
                .unwrap_or_default();
            DependencySummary {
                ecosystem,
                name: raw_name,
                version,
                scope,
                version_confidence: confidence,
                advisories,
            }
        })
        .collect())
}

fn load_dependency_counts(conn: &Connection, repo_id: i64) -> CoreResult<DependencyCounts> {
    let mut stmt = conn.prepare(
        "SELECT ecosystem, is_direct, COUNT(*) AS n
           FROM dependencies WHERE repo_id = ?1
          GROUP BY ecosystem, is_direct",
    )?;
    let mut per: HashMap<String, EcosystemCount> = HashMap::new();
    let mut rows = stmt.query([repo_id])?;
    while let Some(r) = rows.next()? {
        let eco: String = r.get("ecosystem")?;
        let is_direct: i64 = r.get("is_direct")?;
        let n: i64 = r.get("n")?;
        let entry = per.entry(eco.clone()).or_insert_with(|| EcosystemCount {
            ecosystem: eco,
            direct: 0,
            transitive: 0,
        });
        if is_direct != 0 {
            entry.direct += n as u32;
        } else {
            entry.transitive += n as u32;
        }
    }
    let mut per_ecosystem: Vec<EcosystemCount> = per.into_values().collect();
    per_ecosystem.sort_by(|a, b| a.ecosystem.cmp(&b.ecosystem));
    let direct_total = per_ecosystem.iter().map(|e| e.direct).sum();
    let transitive_total = per_ecosystem.iter().map(|e| e.transitive).sum();
    Ok(DependencyCounts {
        per_ecosystem,
        direct_total,
        transitive_total,
    })
}

fn git_from_row(row: &RepoRow) -> Option<GitInfo> {
    // A bare repo or a failed git read leaves every column NULL; render no
    // git block at all in that case.
    let any = row.head_sha.is_some()
        || row.branch.is_some()
        || row.last_commit_at.is_some()
        || row.remote_url.is_some();
    if !any {
        return None;
    }
    Some(GitInfo {
        head_sha: row.head_sha.clone(),
        branch: row.branch.clone(),
        last_commit_at: row
            .last_commit_at
            .as_deref()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|d| d.with_timezone(&chrono::Utc)),
        last_commit_summary: row.last_commit_summary.clone(),
        commits_90d: row.commits_90d.map(|v| v as u32),
        commits_total: row.commits_total.map(|v| v as u32),
        author_count: row.author_count.map(|v| v as u32),
        dirty_modified: row.dirty_modified.map(|v| v as u32),
        dirty_staged: row.dirty_staged.map(|v| v as u32),
        dirty_untracked: row.dirty_untracked.map(|v| v as u32),
        ahead: row.ahead.map(|v| v as u32),
        behind: row.behind.map(|v| v as u32),
        remote_url: row.remote_url.clone(),
        branch_count: row.branch_count.map(|v| v as u32),
        has_stash: row.has_stash.map(|v| v != 0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    fn seed(db: &Db) -> i64 {
        db.write(|c| {
            c.execute_batch(
                r#"
                INSERT INTO scan_roots (id, path, added_at) VALUES (1, '/r', '2026-01-01T00:00:00Z');
                INSERT INTO repos (id, root_id, path, name, category, category_manual, category_scores,
                                   health_score, health_band, branch, last_commit_at)
                    VALUES (1, 1, '/r/app', 'app', 'Backend', NULL,
                            '{"totals":[["Backend",6.0]],"fired":[{"rule_id":"r1","signal":"dependency:express","category":"Backend","weight":6.0}]}',
                            72, 'good', 'main', '2026-02-01T00:00:00Z');
                INSERT INTO repo_languages (repo_id, language, code_lines, comment_lines, files, percentage)
                    VALUES (1, 'TypeScript', 900, 100, 20, 90.0);
                INSERT INTO repo_technologies (repo_id, tech, kind, evidence)
                    VALUES (1, 'Express', 'framework', '["dependency:express"]');
                INSERT INTO manifests (id, repo_id, path, ecosystem, kind, content_hash)
                    VALUES (1, 1, 'package-lock.json', 'npm', 'lockfile', 'h');
                INSERT INTO dependencies (id, repo_id, manifest_id, ecosystem, name, raw_name, version, confidence, scope, is_direct)
                    VALUES (1, 1, 1, 'npm', 'express', 'express', '4.17.1', 'exact', 'runtime', 1),
                           (2, 1, 1, 'npm', 'lodash', 'lodash', '4.17.20', 'exact', 'runtime', 0);
                INSERT INTO advisories (id, kind, severity, summary, modified)
                    VALUES ('GHSA-x', 'vulnerability', 'high', 'ReDoS in express', '2026-01-01T00:00:00Z');
                INSERT INTO findings (id, repo_id, dependency_id, advisory_id, kind, confidence, fixed_version, deduction)
                    VALUES (1, 1, 1, 'GHSA-x', 'vulnerability', 'exact', '4.18.0', 8.0);
                "#,
            )
            .map_err(Into::into)
        })
        .unwrap();
        1
    }

    #[test]
    fn assembles_a_populated_single_repo_context() {
        let db = Db::open_in_memory().unwrap();
        let id = seed(&db);
        let conn = db.read().unwrap();

        let ctx = build_context(
            &conn,
            &[id],
            ScopeContext::WholeRepo,
            vec![],
            &HashMap::new(),
            Freshness::Fresh,
        )
        .unwrap();

        assert_eq!(ctx.repos.len(), 1);
        let repo = &ctx.repos[0];
        assert_eq!(repo.name, "app");
        assert_eq!(repo.category, Category::Backend);
        assert_eq!(repo.category_signals.len(), 1);
        assert!(repo.category_signals[0].contains("Backend"));
        assert_eq!(repo.languages[0].language, "TypeScript");
        assert_eq!(repo.technologies[0].tech, "Express");
        assert_eq!(repo.direct_dependencies.len(), 1);
        assert_eq!(repo.direct_dependencies[0].name, "express");
        assert_eq!(repo.direct_dependencies[0].advisories, vec!["GHSA-x"]);
        assert_eq!(repo.dependency_counts.direct_total, 1);
        assert_eq!(repo.dependency_counts.transitive_total, 1);
        assert_eq!(repo.findings.len(), 1);
        assert!(repo.findings[0].confirmed);
        assert_eq!(repo.health.as_ref().unwrap().score, 72);
        assert_eq!(repo.git.as_ref().unwrap().branch.as_deref(), Some("main"));
    }

    #[test]
    fn unknown_repo_id_is_an_error() {
        let db = Db::open_in_memory().unwrap();
        let conn = db.read().unwrap();
        let err = build_context(
            &conn,
            &[999],
            ScopeContext::WholeRepo,
            vec![],
            &HashMap::new(),
            Freshness::Never,
        )
        .unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    fn manual_override_wins_over_computed_category() {
        let db = Db::open_in_memory().unwrap();
        let id = seed(&db);
        db.write(|c| {
            c.execute("UPDATE repos SET category_manual = 'Cli' WHERE id = 1", [])
                .map_err(Into::into)
        })
        .unwrap();
        let conn = db.read().unwrap();
        let ctx = build_context(
            &conn,
            &[id],
            ScopeContext::WholeRepo,
            vec![],
            &HashMap::new(),
            Freshness::Fresh,
        )
        .unwrap();
        assert_eq!(ctx.repos[0].category, Category::Cli);
    }
}
