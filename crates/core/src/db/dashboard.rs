//! Dashboard aggregates (PRD §6, M3-6 / M3-7).
//!
//! One round trip builds every number the Dashboard renders: the repo
//! count, the health-band histogram, the category donut, the language bar,
//! the stalest and worst-health repos, and the list of repos with a
//! confirmed compromise (which is what gates the M3-7 banner — it renders
//! *only* when that list is non-empty).
//!
//! Every figure is read straight from the persisted `repos` / `findings` /
//! `repo_languages` rows; nothing is recomputed here.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::error::CoreResult;

/// A `(label, count)` pair for a histogram or donut slice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Bucket {
    pub label: String,
    pub count: i64,
}

/// A repo referenced from one of the "needs attention" lists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RepoRef {
    pub id: i64,
    pub name: String,
    /// Present on the stalest list.
    pub last_commit_at: Option<String>,
    /// Present on the worst-health list.
    pub health_score: Option<i64>,
    pub health_band: Option<String>,
    /// Confirmed-compromise finding count (0 unless this is a compromise row).
    pub compromise_count: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DashboardStats {
    /// Top-level repositories (submodule children excluded).
    pub repo_count: i64,
    pub dirty_count: i64,
    /// Health band → repo count, ordered worst-first. A repo with no band
    /// yet counts as `unknown` (never as healthy — DESIGN §14.4).
    pub health_distribution: Vec<Bucket>,
    /// Effective category → repo count, largest first. The manual override
    /// (FR-3.7) wins over the computed value here.
    pub category_distribution: Vec<Bucket>,
    /// Language → summed code lines across all repos, largest first (top 8).
    pub language_distribution: Vec<Bucket>,
    /// Up to 5 repos with the oldest last commit.
    pub stalest: Vec<RepoRef>,
    /// Up to 5 repos with the lowest health score.
    pub worst_health: Vec<RepoRef>,
    /// Every repo with at least one un-suppressed compromise finding. The
    /// M3-7 banner renders iff this is non-empty (FR-6.3).
    pub compromised: Vec<RepoRef>,
}

const BANDS: [&str; 6] = ["unknown", "critical", "poor", "fair", "good", "excellent"];

/// Build every Dashboard figure in one pass.
pub fn stats(conn: &Connection) -> CoreResult<DashboardStats> {
    let repo_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM repos WHERE parent_repo_id IS NULL",
        [],
        |r| r.get(0),
    )?;

    let dirty_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM repos
          WHERE parent_repo_id IS NULL
            AND (COALESCE(dirty_modified,0)+COALESCE(dirty_staged,0)+COALESCE(dirty_untracked,0)) > 0",
        [],
        |r| r.get(0),
    )?;

    // Health distribution — seed every band at 0 so the histogram has a
    // stable shape even before a sync.
    let mut health_distribution: Vec<Bucket> = BANDS
        .iter()
        .map(|b| Bucket {
            label: (*b).to_string(),
            count: 0,
        })
        .collect();
    {
        let mut stmt = conn.prepare(
            "SELECT COALESCE(health_band, 'unknown') AS band, COUNT(*) AS n
               FROM repos WHERE parent_repo_id IS NULL GROUP BY band",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, String>("band")?, r.get::<_, i64>("n")?))
        })?;
        for row in rows {
            let (band, n) = row?;
            match health_distribution.iter_mut().find(|b| b.label == band) {
                Some(bucket) => bucket.count = n,
                None => health_distribution.push(Bucket {
                    label: band,
                    count: n,
                }),
            }
        }
    }

    // Category distribution — the override wins.
    let mut cat_stmt = conn.prepare(
        "SELECT COALESCE(category_manual, category, 'Unknown') AS cat, COUNT(*) AS n
           FROM repos WHERE parent_repo_id IS NULL
          GROUP BY cat ORDER BY n DESC, cat",
    )?;
    let category_distribution: Vec<Bucket> = cat_stmt
        .query_map([], |r| {
            Ok(Bucket {
                label: r.get("cat")?,
                count: r.get("n")?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;

    let mut lang_stmt = conn.prepare(
        "SELECT language, SUM(code_lines) AS lines
           FROM repo_languages
           JOIN repos ON repos.id = repo_languages.repo_id
          WHERE repos.parent_repo_id IS NULL
          GROUP BY language ORDER BY lines DESC LIMIT 8",
    )?;
    let language_distribution: Vec<Bucket> = lang_stmt
        .query_map([], |r| {
            Ok(Bucket {
                label: r.get("language")?,
                count: r.get::<_, i64>("lines")?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;

    let mut stale_stmt = conn.prepare(
        "SELECT id, name, last_commit_at, health_score, health_band
           FROM repos
          WHERE parent_repo_id IS NULL AND is_bare = 0 AND last_commit_at IS NOT NULL
          ORDER BY last_commit_at ASC LIMIT 5",
    )?;
    let stalest: Vec<RepoRef> = stale_stmt
        .query_map([], row_to_ref)?
        .collect::<rusqlite::Result<_>>()?;

    let mut worst_stmt = conn.prepare(
        "SELECT r.id, r.name, r.last_commit_at, r.health_score, r.health_band,
                (SELECT COUNT(*) FROM findings f
                  WHERE f.repo_id = r.id AND f.kind = 'compromise' AND f.suppressed = 0) AS comp
           FROM repos r
          WHERE r.parent_repo_id IS NULL AND r.health_score IS NOT NULL
          ORDER BY r.health_score ASC, r.name LIMIT 5",
    )?;
    let worst_health: Vec<RepoRef> = worst_stmt
        .query_map([], row_to_ref_with_comp)?
        .collect::<rusqlite::Result<_>>()?;

    let mut comp_stmt = conn.prepare(
        "SELECT r.id, r.name, r.last_commit_at, r.health_score, r.health_band,
                COUNT(*) AS comp
           FROM repos r
           JOIN findings f ON f.repo_id = r.id
          WHERE r.parent_repo_id IS NULL AND f.kind = 'compromise' AND f.suppressed = 0
          GROUP BY r.id ORDER BY comp DESC, r.name",
    )?;
    let compromised: Vec<RepoRef> = comp_stmt
        .query_map([], row_to_ref_with_comp)?
        .collect::<rusqlite::Result<_>>()?;

    Ok(DashboardStats {
        repo_count,
        dirty_count,
        health_distribution,
        category_distribution,
        language_distribution,
        stalest,
        worst_health,
        compromised,
    })
}

fn row_to_ref(r: &rusqlite::Row<'_>) -> rusqlite::Result<RepoRef> {
    Ok(RepoRef {
        id: r.get("id")?,
        name: r.get("name")?,
        last_commit_at: r.get("last_commit_at")?,
        health_score: r.get("health_score")?,
        health_band: r.get("health_band")?,
        compromise_count: 0,
    })
}

fn row_to_ref_with_comp(r: &rusqlite::Row<'_>) -> rusqlite::Result<RepoRef> {
    Ok(RepoRef {
        compromise_count: r.get("comp")?,
        ..row_to_ref(r)?
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    fn seed(db: &Db) {
        db.write(|c| {
            c.execute_batch(
                r#"
                INSERT INTO scan_roots (id, path, added_at) VALUES (1, '/r', '2026-01-01T00:00:00Z');
                INSERT INTO repos (id, root_id, path, name, last_commit_at, health_score, health_band, category)
                    VALUES (1, 1, '/r/a', 'a', '2020-01-01T00:00:00Z', 20, 'critical', 'Backend');
                INSERT INTO repos (id, root_id, path, name, last_commit_at, health_score, health_band, category, category_manual)
                    VALUES (2, 1, '/r/b', 'b', '2026-01-01T00:00:00Z', 95, 'excellent', 'Frontend', 'Fullstack');
                INSERT INTO repos (id, root_id, path, name, dirty_modified)
                    VALUES (3, 1, '/r/c', 'c', 2);
                INSERT INTO repo_languages (repo_id, language, code_lines, comment_lines, files, percentage)
                    VALUES (1, 'Rust', 500, 0, 5, 100.0);
                INSERT INTO repo_languages (repo_id, language, code_lines, comment_lines, files, percentage)
                    VALUES (2, 'TypeScript', 300, 0, 3, 100.0);
                INSERT INTO advisories (id, kind, severity, modified)
                    VALUES ('MAL-1', 'compromise', 'critical', '2026-01-01T00:00:00Z');
                INSERT INTO manifests (id, repo_id, path, ecosystem, kind, content_hash)
                    VALUES (1, 1, 'Cargo.lock', 'crates.io', 'lockfile', 'h');
                INSERT INTO dependencies (id, repo_id, manifest_id, ecosystem, name, raw_name, version, confidence, scope, is_direct)
                    VALUES (1, 1, 1, 'crates.io', 'evil', 'evil', '1.0.0', 'exact', 'runtime', 1);
                INSERT INTO findings (id, repo_id, dependency_id, advisory_id, kind, confidence, deduction)
                    VALUES (1, 1, 1, 'MAL-1', 'compromise', 'exact', 80.0);
                "#,
            )?;
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn aggregates_cover_every_dashboard_panel() {
        let db = Db::open_in_memory().unwrap();
        seed(&db);
        let s = db.read().map(|c| stats(&c)).unwrap().unwrap();

        assert_eq!(s.repo_count, 3);
        assert_eq!(s.dirty_count, 1);

        // Bands seeded; 'c' has no band → unknown.
        let band = |l: &str| {
            s.health_distribution
                .iter()
                .find(|b| b.label == l)
                .unwrap()
                .count
        };
        assert_eq!(band("unknown"), 1);
        assert_eq!(band("critical"), 1);
        assert_eq!(band("excellent"), 1);

        // Override wins: repo b counts as Fullstack, not Frontend.
        let cat = |l: &str| {
            s.category_distribution
                .iter()
                .find(|b| b.label == l)
                .map(|b| b.count)
                .unwrap_or(0)
        };
        assert_eq!(cat("Fullstack"), 1);
        assert_eq!(cat("Frontend"), 0);
        assert_eq!(cat("Backend"), 1);
        assert_eq!(cat("Unknown"), 1);

        assert_eq!(s.language_distribution[0].label, "Rust");
        assert_eq!(s.language_distribution[0].count, 500);

        assert_eq!(s.stalest[0].name, "a");
        assert_eq!(s.worst_health[0].name, "a");
        assert_eq!(s.worst_health[0].compromise_count, 1);

        assert_eq!(s.compromised.len(), 1);
        assert_eq!(s.compromised[0].name, "a");
    }

    #[test]
    fn compromised_is_empty_on_a_clean_fleet() {
        let db = Db::open_in_memory().unwrap();
        db.write(|c| {
            c.execute_batch(
                "INSERT INTO scan_roots (id, path, added_at) VALUES (1, '/r', '2026-01-01T00:00:00Z');
                 INSERT INTO repos (id, root_id, path, name) VALUES (1, 1, '/r/a', 'a');",
            )?;
            Ok(())
        })
        .unwrap();
        let s = db.read().map(|c| stats(&c)).unwrap().unwrap();
        assert!(s.compromised.is_empty(), "no banner on a clean fleet");
    }
}
