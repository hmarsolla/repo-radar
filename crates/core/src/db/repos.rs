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
use crate::model::{GitInfo, LanguageStat, RepoIdentity};

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

// ---------------------------------------------------------------------------
// Repo upsert + child-table writes (used by the scan writer thread — M1-5)
// ---------------------------------------------------------------------------

fn rfc3339(opt: &Option<chrono::DateTime<chrono::Utc>>) -> Option<String> {
    opt.map(|d| d.to_rfc3339())
}

/// Insert or update a repository row, keyed on its unique `path`. Returns
/// the repo's id. Git columns are populated from `git` when present; a bare
/// repo or a failed git read leaves them NULL.
#[allow(clippy::too_many_arguments)]
pub fn upsert_repo(
    conn: &Connection,
    root_id: i64,
    identity: &RepoIdentity,
    git: Option<&GitInfo>,
    parent_repo_id: Option<i64>,
    fingerprint: Option<&str>,
    is_monorepo: bool,
) -> CoreResult<i64> {
    let g = git.cloned().unwrap_or_default();
    conn.execute(
        "INSERT INTO repos (
             root_id, parent_repo_id, path, name, is_bare, is_monorepo,
             head_sha, branch, last_commit_at, last_commit_summary,
             commits_90d, commits_total, author_count,
             dirty_modified, dirty_staged, dirty_untracked,
             ahead, behind, remote_url, branch_count, has_stash,
             last_scanned_at, scan_fingerprint
         ) VALUES (
             ?1, ?2, ?3, ?4, ?5, ?23,
             ?6, ?7, ?8, ?9,
             ?10, ?11, ?12,
             ?13, ?14, ?15,
             ?16, ?17, ?18, ?19, ?20,
             ?21, ?22
         )
         ON CONFLICT(path) DO UPDATE SET
             root_id = excluded.root_id,
             parent_repo_id = excluded.parent_repo_id,
             name = excluded.name,
             is_bare = excluded.is_bare,
             is_monorepo = excluded.is_monorepo,
             head_sha = excluded.head_sha,
             branch = excluded.branch,
             last_commit_at = excluded.last_commit_at,
             last_commit_summary = excluded.last_commit_summary,
             commits_90d = excluded.commits_90d,
             commits_total = excluded.commits_total,
             author_count = excluded.author_count,
             dirty_modified = excluded.dirty_modified,
             dirty_staged = excluded.dirty_staged,
             dirty_untracked = excluded.dirty_untracked,
             ahead = excluded.ahead,
             behind = excluded.behind,
             remote_url = excluded.remote_url,
             branch_count = excluded.branch_count,
             has_stash = excluded.has_stash,
             last_scanned_at = excluded.last_scanned_at,
             scan_fingerprint = excluded.scan_fingerprint",
        rusqlite::params![
            root_id,
            parent_repo_id,
            identity.path,
            identity.name,
            identity.is_bare as i64,
            g.head_sha,
            g.branch,
            rfc3339(&g.last_commit_at),
            g.last_commit_summary,
            g.commits_90d,
            g.commits_total,
            g.author_count,
            g.dirty_modified,
            g.dirty_staged,
            g.dirty_untracked,
            g.ahead,
            g.behind,
            g.remote_url,
            g.branch_count,
            g.has_stash.map(|b| b as i64),
            chrono::Utc::now().to_rfc3339(),
            fingerprint,
            is_monorepo as i64,
        ],
    )?;

    let id = conn.query_row(
        "SELECT id FROM repos WHERE path = ?1",
        [&identity.path],
        |r| r.get(0),
    )?;
    Ok(id)
}

/// Replace a repo's `manifests` and `dependencies` rows wholesale. A
/// submodule child's dependencies are attributed to the parent's `repo_id`
/// by the caller (FR-1.5); this appends rather than clobbering when
/// `repo_id` is a parent aggregating several children.
pub fn replace_manifests_and_deps(
    conn: &Connection,
    repo_id: i64,
    manifests: &[crate::model::ParsedManifest],
    deps: &[crate::model::Dependency],
) -> CoreResult<()> {
    conn.execute("DELETE FROM dependencies WHERE repo_id = ?1", [repo_id])?;
    conn.execute("DELETE FROM manifests WHERE repo_id = ?1", [repo_id])?;

    // manifest path -> row id, so dependencies can reference the right manifest.
    let mut manifest_ids: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    {
        let mut mstmt = conn.prepare_cached(
            "INSERT INTO manifests (repo_id, path, ecosystem, kind, content_hash)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(repo_id, path) DO UPDATE SET
                 ecosystem = excluded.ecosystem, kind = excluded.kind,
                 content_hash = excluded.content_hash",
        )?;
        for m in manifests {
            mstmt.execute(rusqlite::params![
                repo_id,
                m.path.as_str(),
                m.ecosystem.osv_id(),
                m.kind.as_str(),
                m.content_hash,
            ])?;
            let id: i64 = conn.query_row(
                "SELECT id FROM manifests WHERE repo_id = ?1 AND path = ?2",
                rusqlite::params![repo_id, m.path.as_str()],
                |r| r.get(0),
            )?;
            manifest_ids.insert(m.path.0.clone(), id);
        }
    }

    let mut dstmt = conn.prepare_cached(
        "INSERT INTO dependencies
             (repo_id, manifest_id, ecosystem, name, raw_name, version, confidence, scope, is_direct)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
    )?;
    for d in deps {
        let Some(&manifest_id) = manifest_ids.get(d.manifest_path.as_str()) else {
            continue; // dependency whose manifest didn't record (shouldn't happen)
        };
        dstmt.execute(rusqlite::params![
            repo_id,
            manifest_id,
            d.ecosystem.osv_id(),
            d.name,
            d.raw_name,
            d.version,
            confidence_str(d.confidence),
            scope_str(d.scope),
            d.is_direct as i64,
        ])?;
    }
    Ok(())
}

fn confidence_str(c: crate::model::Confidence) -> &'static str {
    match c {
        crate::model::Confidence::Exact => "exact",
        crate::model::Confidence::Range => "range",
    }
}

fn scope_str(s: crate::model::Scope) -> &'static str {
    match s {
        crate::model::Scope::Runtime => "runtime",
        crate::model::Scope::Dev => "dev",
        crate::model::Scope::Build => "build",
        crate::model::Scope::Optional => "optional",
        crate::model::Scope::Peer => "peer",
    }
}

// ---------------------------------------------------------------------------
// Classification + technologies (M3-2, M3-3)
// ---------------------------------------------------------------------------

/// Wire string for a [`crate::model::Category`]. Matches the enum's serde
/// name so `category_scores` JSON and the `category` column agree.
pub fn category_str(c: crate::model::Category) -> &'static str {
    use crate::model::Category::*;
    match c {
        Frontend => "Frontend",
        Backend => "Backend",
        Fullstack => "Fullstack",
        Mobile => "Mobile",
        DevOps => "DevOps",
        DataMl => "DataMl",
        Library => "Library",
        Cli => "Cli",
        Docs => "Docs",
        Unknown => "Unknown",
    }
}

fn confidence_level_str(c: crate::model::ConfidenceLevel) -> &'static str {
    use crate::model::ConfidenceLevel::*;
    match c {
        Low => "low",
        Medium => "medium",
        High => "high",
    }
}

/// Replace a repo's `repo_technologies` rows wholesale (FR-2.3). Evidence is
/// stored as a JSON array so the UI can distinguish dependency-confirmed
/// from marker-only detections (FR-2.4).
pub fn replace_technologies(
    conn: &Connection,
    repo_id: i64,
    techs: &[crate::model::DetectedTech],
) -> CoreResult<()> {
    conn.execute(
        "DELETE FROM repo_technologies WHERE repo_id = ?1",
        [repo_id],
    )?;
    let mut stmt = conn.prepare_cached(
        "INSERT INTO repo_technologies (repo_id, tech, kind, evidence) VALUES (?1, ?2, ?3, ?4)",
    )?;
    for t in techs {
        let evidence = serde_json::to_string(&t.evidence)?;
        stmt.execute(rusqlite::params![repo_id, t.tech, t.kind, evidence])?;
    }
    Ok(())
}

/// Write the computed classification onto a repo row. The `category_manual`
/// override (FR-3.7) is written by a separate path (M3-5) and is deliberately
/// **not** touched here — a re-scan must never silently discard it.
pub fn set_classification(
    conn: &Connection,
    repo_id: i64,
    c: &crate::model::Classification,
) -> CoreResult<()> {
    let scores = serde_json::to_string(&c.scores)?;
    conn.execute(
        "UPDATE repos
            SET category = ?2, category_confidence = ?3, category_scores = ?4
          WHERE id = ?1",
        rusqlite::params![
            repo_id,
            category_str(c.category),
            confidence_level_str(c.confidence),
            scores,
        ],
    )?;
    Ok(())
}

/// `(path, scan_fingerprint)` for every repo — loaded once at the start of a
/// scan so the incremental-skip check (DESIGN §6.5) never touches the DB
/// from the parallel analysis stage.
pub fn all_fingerprints(conn: &Connection) -> CoreResult<Vec<(String, Option<String>)>> {
    let mut stmt = conn.prepare("SELECT path, scan_fingerprint FROM repos")?;
    let rows = stmt
        .query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Mark a repo as seen by the current scan without re-analysing it.
pub fn touch_scanned(conn: &Connection, repo_id: i64) -> CoreResult<()> {
    conn.execute(
        "UPDATE repos SET last_scanned_at = ?2 WHERE id = ?1",
        rusqlite::params![repo_id, chrono::Utc::now().to_rfc3339()],
    )?;
    Ok(())
}

/// Replace a repo's language rows wholesale.
pub fn replace_languages(
    conn: &Connection,
    repo_id: i64,
    langs: &[LanguageStat],
) -> CoreResult<()> {
    conn.execute("DELETE FROM repo_languages WHERE repo_id = ?1", [repo_id])?;
    let mut stmt = conn.prepare_cached(
        "INSERT INTO repo_languages
             (repo_id, language, code_lines, comment_lines, files, percentage)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )?;
    for l in langs {
        stmt.execute(rusqlite::params![
            repo_id,
            l.language,
            l.code_lines as i64,
            l.comment_lines as i64,
            l.files as i64,
            l.percentage as f64,
        ])?;
    }
    Ok(())
}

/// Look up a repo id by its normalised path.
pub fn repo_id_by_path(conn: &Connection, path: &str) -> CoreResult<Option<i64>> {
    Ok(conn
        .query_row("SELECT id FROM repos WHERE path = ?1", [path], |r| r.get(0))
        .optional()?)
}

/// Compact per-repo row for lists and the scan reporter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RepoListItem {
    pub id: i64,
    pub name: String,
    pub path: String,
    pub is_bare: bool,
    pub branch: Option<String>,
    pub last_commit_at: Option<String>,
    pub dirty: bool,
    pub primary_language: Option<String>,
    pub health_score: Option<i64>,
    /// `unknown` / `critical` / `poor` / `fair` / `good` / `excellent`.
    pub health_band: Option<String>,
    /// Confirmed compromise findings and ordinary vulnerabilities are kept
    /// in **separate columns** (FR-6.1, M2-23) — a combined "issues" count
    /// would destroy the distinction the health model exists to make.
    pub compromise_count: i64,
    pub vulnerability_count: i64,
}

const REPO_LIST_SELECT: &str = "
    SELECT r.id, r.name, r.path, r.is_bare, r.branch, r.last_commit_at,
           r.health_score AS health_score, r.health_band AS health_band,
           COALESCE(r.dirty_modified,0)+COALESCE(r.dirty_staged,0)+COALESCE(r.dirty_untracked,0) AS dirt,
           (SELECT language FROM repo_languages l
             WHERE l.repo_id = r.id ORDER BY l.percentage DESC LIMIT 1) AS primary_language,
           (SELECT COUNT(*) FROM findings f
             WHERE f.repo_id = r.id AND f.kind = 'compromise' AND f.suppressed = 0) AS compromise_count,
           (SELECT COUNT(*) FROM findings f
             WHERE f.repo_id = r.id AND f.kind = 'vulnerability' AND f.suppressed = 0) AS vulnerability_count
      FROM repos r";

fn row_to_list_item(row: &rusqlite::Row<'_>) -> rusqlite::Result<RepoListItem> {
    Ok(RepoListItem {
        id: row.get("id")?,
        name: row.get("name")?,
        path: row.get("path")?,
        is_bare: row.get::<_, i64>("is_bare")? != 0,
        branch: row.get("branch")?,
        last_commit_at: row.get("last_commit_at")?,
        dirty: row.get::<_, i64>("dirt")? != 0,
        primary_language: row.get("primary_language")?,
        health_score: row.get("health_score")?,
        health_band: row.get("health_band")?,
        compromise_count: row.get("compromise_count")?,
        vulnerability_count: row.get("vulnerability_count")?,
    })
}

/// Every top-level repo (submodule children excluded), newest commit first.
pub fn list_top_level(conn: &Connection) -> CoreResult<Vec<RepoListItem>> {
    list_repos(conn, &RepoFilter::default())
}

/// Sort key for the repo list. Sorting happens in SQL (DESIGN §12.1).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum RepoSort {
    Name,
    #[default]
    LastCommit,
    PrimaryLanguage,
}

/// Typed filter for `list_repos`. A struct rather than loose query params so
/// the whole query — filtering *and* sorting — executes in SQLite and the
/// client never receives rows it will just hide (DESIGN §12.1).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", default)]
pub struct RepoFilter {
    /// Free-text match against name and path (case-insensitive substring).
    pub search: Option<String>,
    /// Only repos whose primary language equals this.
    pub language: Option<String>,
    /// Only repos with a dirty working tree.
    pub dirty_only: bool,
    /// Include bare repos (default: included).
    pub include_bare: bool,
    pub sort: RepoSort,
    pub descending: bool,
}

impl Default for RepoFilter {
    fn default() -> Self {
        Self {
            search: None,
            language: None,
            dirty_only: false,
            include_bare: true,
            sort: RepoSort::default(),
            descending: true,
        }
    }
}

/// List repositories matching `filter`, sorted per `filter.sort`. Submodule
/// children are always excluded — they belong to their parent (FR-1.5).
pub fn list_repos(conn: &Connection, filter: &RepoFilter) -> CoreResult<Vec<RepoListItem>> {
    let mut sql = String::from(REPO_LIST_SELECT);
    sql.push_str(" WHERE r.parent_repo_id IS NULL");

    let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    if let Some(term) = filter.search.as_deref().filter(|s| !s.trim().is_empty()) {
        sql.push_str(" AND (r.name LIKE ? OR r.path LIKE ?)");
        let like = format!("%{}%", term.trim());
        params.push(Box::new(like.clone()));
        params.push(Box::new(like));
    }
    if filter.dirty_only {
        sql.push_str(
            " AND (COALESCE(r.dirty_modified,0)+COALESCE(r.dirty_staged,0)+COALESCE(r.dirty_untracked,0)) > 0",
        );
    }
    if !filter.include_bare {
        sql.push_str(" AND r.is_bare = 0");
    }
    if let Some(lang) = filter.language.as_deref().filter(|s| !s.is_empty()) {
        sql.push_str(
            " AND (SELECT language FROM repo_languages l
                    WHERE l.repo_id = r.id ORDER BY l.percentage DESC LIMIT 1) = ?",
        );
        params.push(Box::new(lang.to_string()));
    }

    let order_col = match filter.sort {
        RepoSort::Name => "r.name COLLATE NOCASE",
        RepoSort::LastCommit => "r.last_commit_at",
        RepoSort::PrimaryLanguage => "primary_language",
    };
    let dir = if filter.descending { "DESC" } else { "ASC" };
    // Keep NULLs last regardless of direction; stable tiebreak on name.
    sql.push_str(&format!(
        " ORDER BY ({order_col} IS NULL), {order_col} {dir}, r.name COLLATE NOCASE ASC"
    ));

    let mut stmt = conn.prepare(&sql)?;
    let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|b| b.as_ref()).collect();
    let rows = stmt
        .query_map(param_refs.as_slice(), row_to_list_item)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Full git columns for one repo, for the detail view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RepoRecord {
    pub id: i64,
    pub name: String,
    pub path: String,
    pub is_bare: bool,
    pub is_monorepo: bool,
    pub parent_repo_id: Option<i64>,
    pub head_sha: Option<String>,
    pub branch: Option<String>,
    pub last_commit_at: Option<String>,
    pub last_commit_summary: Option<String>,
    pub commits_90d: Option<i64>,
    pub commits_total: Option<i64>,
    pub author_count: Option<i64>,
    pub dirty_modified: Option<i64>,
    pub dirty_staged: Option<i64>,
    pub dirty_untracked: Option<i64>,
    pub ahead: Option<i64>,
    pub behind: Option<i64>,
    pub remote_url: Option<String>,
    pub branch_count: Option<i64>,
    pub has_stash: Option<bool>,
    pub last_scanned_at: Option<String>,

    // health (FR-6)
    pub health_score: Option<i64>,
    pub health_band: Option<String>,
    pub category: Option<String>,
}

fn row_to_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<RepoRecord> {
    Ok(RepoRecord {
        id: row.get("id")?,
        name: row.get("name")?,
        path: row.get("path")?,
        is_bare: row.get::<_, i64>("is_bare")? != 0,
        is_monorepo: row.get::<_, i64>("is_monorepo")? != 0,
        parent_repo_id: row.get("parent_repo_id")?,
        head_sha: row.get("head_sha")?,
        branch: row.get("branch")?,
        last_commit_at: row.get("last_commit_at")?,
        last_commit_summary: row.get("last_commit_summary")?,
        commits_90d: row.get("commits_90d")?,
        commits_total: row.get("commits_total")?,
        author_count: row.get("author_count")?,
        dirty_modified: row.get("dirty_modified")?,
        dirty_staged: row.get("dirty_staged")?,
        dirty_untracked: row.get("dirty_untracked")?,
        ahead: row.get("ahead")?,
        behind: row.get("behind")?,
        remote_url: row.get("remote_url")?,
        branch_count: row.get("branch_count")?,
        has_stash: row.get::<_, Option<i64>>("has_stash")?.map(|v| v != 0),
        last_scanned_at: row.get("last_scanned_at")?,
        health_score: row.get("health_score")?,
        health_band: row.get("health_band")?,
        category: row.get("category")?,
    })
}

/// One finding for the Health tab (FR-6.9). Rendered from the stored data —
/// the UI never recomputes a score.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FindingDetail {
    pub advisory_id: String,
    /// `compromise` | `vulnerability`.
    pub kind: String,
    pub severity: String,
    pub confidence: String,
    pub package_name: String,
    pub package_version: String,
    pub fixed_version: Option<String>,
    pub summary: String,
    pub deduction: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RepoDetail {
    pub repo: RepoRecord,
    pub languages: Vec<LanguageStat>,
    /// Submodule children (FR-1.5) — shown here, never in the main list.
    pub submodules: Vec<RepoRecord>,
    /// The stored `health_breakdown` JSON, rendered directly by the UI so
    /// the number shown and the number explained cannot drift (FR-6.9).
    pub health_breakdown: Option<String>,
    /// Compromise findings first, then vulnerabilities (FR-6.1).
    pub findings: Vec<FindingDetail>,
}

/// Full detail for one repo: its record, language breakdown, submodule
/// children, and its findings + health breakdown.
pub fn get_repo_detail(conn: &Connection, id: i64) -> CoreResult<Option<RepoDetail>> {
    let repo = conn
        .query_row("SELECT * FROM repos WHERE id = ?1", [id], row_to_record)
        .optional()?;
    let Some(repo) = repo else { return Ok(None) };

    let mut lang_stmt = conn.prepare(
        "SELECT language, code_lines, comment_lines, files, percentage
           FROM repo_languages WHERE repo_id = ?1 ORDER BY percentage DESC",
    )?;
    let languages = lang_stmt
        .query_map([id], |r| {
            Ok(LanguageStat {
                language: r.get("language")?,
                code_lines: r.get::<_, i64>("code_lines")? as u64,
                comment_lines: r.get::<_, i64>("comment_lines")? as u64,
                files: r.get::<_, i64>("files")? as u64,
                percentage: r.get::<_, f64>("percentage")? as f32,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut sub_stmt =
        conn.prepare("SELECT * FROM repos WHERE parent_repo_id = ?1 ORDER BY name")?;
    let submodules = sub_stmt
        .query_map([id], row_to_record)?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let health_breakdown: Option<String> = conn
        .query_row(
            "SELECT health_breakdown FROM repos WHERE id = ?1",
            [id],
            |r| r.get(0),
        )
        .optional()?
        .flatten();

    // Compromise findings first (FR-6.1), then by deduction descending.
    let mut fstmt = conn.prepare(
        "SELECT f.advisory_id, f.kind, f.confidence, f.fixed_version, f.deduction,
                d.name AS pkg, d.version AS ver,
                a.severity AS severity, a.summary AS summary
           FROM findings f
           JOIN dependencies d ON d.id = f.dependency_id
           JOIN advisories a ON a.id = f.advisory_id
          WHERE f.repo_id = ?1 AND f.suppressed = 0
          ORDER BY (f.kind = 'compromise') DESC, f.deduction DESC, a.severity DESC",
    )?;
    let findings = fstmt
        .query_map([id], |r| {
            Ok(FindingDetail {
                advisory_id: r.get("advisory_id")?,
                kind: r.get("kind")?,
                confidence: r.get("confidence")?,
                fixed_version: r.get("fixed_version")?,
                deduction: r.get("deduction")?,
                package_name: r.get("pkg")?,
                package_version: r.get("ver")?,
                severity: r.get("severity")?,
                summary: r.get::<_, Option<String>>("summary")?.unwrap_or_default(),
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    Ok(Some(RepoDetail {
        repo,
        languages,
        submodules,
        health_breakdown,
        findings,
    }))
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
