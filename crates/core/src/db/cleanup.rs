//! Cleanup triage: which repositories are safe to reclaim space from, and
//! which hold work that would be lost.
//!
//! # The question this answers
//!
//! "I have 60 repositories and 80 GB of disk. Which of these can I clear out?"
//! is really two questions, and answering only the first is dangerous:
//!
//! 1. **How much is regenerable?** From the disk walk
//!    ([`crate::scan::disk`]) — `node_modules`, `target`, `.venv` and friends,
//!    which a build tool rebuilds on demand.
//! 2. **Would deleting anything here lose work?** From the git signals already
//!    collected: uncommitted changes, unpushed commits, a stash, or no remote
//!    at all. A repository with 4 GB of `node_modules` and three unpushed
//!    commits is *not* a cleanup candidate; it is a backup emergency.
//!
//! Everything here is derived from columns the scan already wrote, so it is a
//! read-only query — the triage is always consistent with the last scan, and
//! there is nothing extra to keep up to date.
//!
//! # Nothing here deletes anything
//!
//! This module reports. Reclaiming space is left to the user, deliberately:
//! recursive deletion of directories inside someone's repositories is the one
//! operation in repo-radar that could destroy data that is not derived, and it
//! should not happen as a side effect of viewing a report.

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::error::CoreResult;

/// How long since the last commit before a repository stops being "active".
/// Chosen to line up with the 90-day commit window the git stage already
/// collects (FR-7).
pub const DORMANT_AFTER_DAYS: i64 = 90;
/// Beyond this, a repository is stale — a plausible archive candidate.
pub const STALE_AFTER_DAYS: i64 = 365;
/// Beyond this, nobody has touched it in years.
pub const ABANDONED_AFTER_DAYS: i64 = 730;

/// Activity verdict, from the last commit date alone. Kept separate from
/// [`CleanupRow::at_risk`] so "nobody has touched this in two years" and "this
/// has unpushed work" are never conflated — a repository is frequently both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum Activity {
    /// Committed to within [`DORMANT_AFTER_DAYS`].
    Active,
    Dormant,
    Stale,
    Abandoned,
    /// No commit date — an unborn repository, or a bare one with no HEAD.
    Unknown,
}

impl Activity {
    fn from_days(days: Option<i64>) -> Self {
        match days {
            None => Activity::Unknown,
            Some(d) if d >= ABANDONED_AFTER_DAYS => Activity::Abandoned,
            Some(d) if d >= STALE_AFTER_DAYS => Activity::Stale,
            Some(d) if d >= DORMANT_AFTER_DAYS => Activity::Dormant,
            Some(_) => Activity::Active,
        }
    }
}

/// A single reason work in this repository exists nowhere else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum RiskKind {
    /// Modified or staged tracked files.
    UncommittedChanges,
    /// Untracked files that are not ignored.
    UntrackedFiles,
    /// Commits on the local branch that the remote does not have.
    UnpushedCommits,
    /// At least one stash entry — invisible in every other view.
    Stash,
    /// No remote configured, so the only copy is this directory.
    NoRemote,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Risk {
    pub kind: RiskKind,
    /// How many files / commits / stash entries, where that is meaningful.
    pub count: Option<i64>,
}

/// One repository as the Cleanup view shows it.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CleanupRow {
    pub repo_id: i64,
    pub name: String,
    pub path: String,
    pub is_bare: bool,

    pub last_commit_at: Option<String>,
    pub days_since_commit: Option<i64>,
    pub activity: Activity,

    /// `null` when this repository has never been measured (scanned by a build
    /// before disk accounting existed). Distinct from `0`.
    pub total_bytes: Option<i64>,
    pub git_bytes: Option<i64>,
    pub reclaimable_bytes: Option<i64>,
    /// Figures are a lower bound — the walk hit its entry cap.
    pub truncated: bool,
    pub measured_at: Option<String>,
    /// The regenerable directories themselves, largest first.
    pub reclaimable_dirs: Vec<ReclaimableDirRow>,

    /// Every reason deleting this repository would lose work. Empty means the
    /// working tree is clean, everything is pushed, and a remote exists.
    pub risks: Vec<Risk>,
}

impl CleanupRow {
    /// True when [`Self::risks`] is non-empty.
    pub fn at_risk(&self) -> bool {
        !self.risks.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ReclaimableDirRow {
    pub rel_path: String,
    pub kind: String,
    pub bytes: i64,
    pub file_count: i64,
}

/// Totals for the Cleanup view's summary tiles.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CleanupSummary {
    pub repo_count: i64,
    /// Repositories whose footprint has been measured at least once.
    pub measured_count: i64,
    pub total_bytes: i64,
    pub reclaimable_bytes: i64,
    /// Reclaimable bytes in repositories with **no** risk flags — the figure a
    /// user can act on without thinking about it.
    pub safe_reclaimable_bytes: i64,
    pub at_risk_count: i64,
    /// Stale or abandoned, i.e. archive candidates.
    pub stale_count: i64,
    /// At least one measurement was capped, so totals are lower bounds.
    pub any_truncated: bool,
}

/// How to sort [`list`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum CleanupSort {
    /// Most reclaimable space first — the default.
    #[default]
    Reclaimable,
    /// Largest total footprint first.
    TotalSize,
    /// Longest since the last commit first.
    Oldest,
    Name,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CleanupFilter {
    pub sort: CleanupSort,
    /// Only repositories with no risk flags.
    #[serde(default)]
    pub safe_only: bool,
    /// Only repositories that are stale or abandoned.
    #[serde(default)]
    pub stale_only: bool,
}

/// Everything the Cleanup view needs: per-repo triage plus the totals.
///
/// Submodule children are excluded — their bytes are counted inside the
/// parent's footprint (see the pipeline's disk stage), so listing them would
/// double-count.
pub fn list(conn: &Connection, filter: &CleanupFilter) -> CoreResult<Vec<CleanupRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, path, is_bare, last_commit_at,
                disk_total_bytes, disk_git_bytes, disk_reclaimable_bytes,
                disk_truncated, disk_measured_at,
                dirty_modified, dirty_staged, dirty_untracked,
                ahead, remote_url, has_stash
           FROM repos
          WHERE parent_repo_id IS NULL",
    )?;

    let now = chrono::Utc::now();
    let mut rows: Vec<CleanupRow> = stmt
        .query_map([], |r| {
            let last_commit_at: Option<String> = r.get(4)?;
            let days_since_commit = last_commit_at.as_deref().and_then(|s| days_since(s, now));

            let dirty_modified: i64 = r.get::<_, Option<i64>>(10)?.unwrap_or(0);
            let dirty_staged: i64 = r.get::<_, Option<i64>>(11)?.unwrap_or(0);
            let dirty_untracked: i64 = r.get::<_, Option<i64>>(12)?.unwrap_or(0);
            let ahead: i64 = r.get::<_, Option<i64>>(13)?.unwrap_or(0);
            let remote_url: Option<String> = r.get(14)?;
            let has_stash: bool = r.get::<_, Option<i64>>(15)?.unwrap_or(0) != 0;
            let is_bare: bool = r.get::<_, i64>(3)? != 0;

            Ok(CleanupRow {
                repo_id: r.get(0)?,
                name: r.get(1)?,
                path: r.get(2)?,
                is_bare,
                last_commit_at,
                days_since_commit,
                activity: Activity::from_days(days_since_commit),
                total_bytes: r.get(5)?,
                git_bytes: r.get(6)?,
                reclaimable_bytes: r.get(7)?,
                truncated: r.get::<_, Option<i64>>(8)?.unwrap_or(0) != 0,
                measured_at: r.get(9)?,
                reclaimable_dirs: Vec::new(),
                risks: assess_risks(
                    is_bare,
                    dirty_modified,
                    dirty_staged,
                    dirty_untracked,
                    ahead,
                    remote_url.as_deref(),
                    has_stash,
                ),
            })
        })?
        .collect::<Result<_, _>>()?;

    // Attach the per-directory breakdown in one extra query rather than one
    // per repository.
    let mut dirs = conn.prepare(
        "SELECT repo_id, rel_path, kind, bytes, file_count
           FROM repo_disk_dirs
          ORDER BY bytes DESC, rel_path ASC",
    )?;
    let mut by_repo: std::collections::HashMap<i64, Vec<ReclaimableDirRow>> =
        std::collections::HashMap::new();
    let mapped = dirs.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            ReclaimableDirRow {
                rel_path: r.get(1)?,
                kind: r.get(2)?,
                bytes: r.get(3)?,
                file_count: r.get(4)?,
            },
        ))
    })?;
    for row in mapped {
        let (repo_id, dir) = row?;
        by_repo.entry(repo_id).or_default().push(dir);
    }
    for row in &mut rows {
        if let Some(dirs) = by_repo.remove(&row.repo_id) {
            row.reclaimable_dirs = dirs;
        }
    }

    if filter.safe_only {
        rows.retain(|r| !r.at_risk());
    }
    if filter.stale_only {
        rows.retain(|r| matches!(r.activity, Activity::Stale | Activity::Abandoned));
    }

    // Sorted in Rust, not SQL: `activity` and `at_risk` are computed from
    // several columns plus the current date, so SQL could not order by them
    // without duplicating the logic.
    match filter.sort {
        CleanupSort::Reclaimable => rows.sort_by(|a, b| {
            b.reclaimable_bytes
                .unwrap_or(-1)
                .cmp(&a.reclaimable_bytes.unwrap_or(-1))
                .then_with(|| a.name.cmp(&b.name))
        }),
        CleanupSort::TotalSize => rows.sort_by(|a, b| {
            b.total_bytes
                .unwrap_or(-1)
                .cmp(&a.total_bytes.unwrap_or(-1))
                .then_with(|| a.name.cmp(&b.name))
        }),
        // `None` (never committed) sorts last rather than first: an unborn
        // repository is not the oldest thing on disk.
        CleanupSort::Oldest => rows.sort_by(|a, b| {
            b.days_since_commit
                .unwrap_or(i64::MIN)
                .cmp(&a.days_since_commit.unwrap_or(i64::MIN))
                .then_with(|| a.name.cmp(&b.name))
        }),
        CleanupSort::Name => rows.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then_with(|| a.path.cmp(&b.path))
        }),
    }

    Ok(rows)
}

/// Totals over every top-level repository, ignoring [`CleanupFilter`].
pub fn summary(conn: &Connection) -> CoreResult<CleanupSummary> {
    let rows = list(conn, &CleanupFilter::default())?;
    let mut s = CleanupSummary {
        repo_count: rows.len() as i64,
        ..Default::default()
    };
    for r in &rows {
        if r.measured_at.is_some() {
            s.measured_count += 1;
        }
        s.total_bytes += r.total_bytes.unwrap_or(0);
        let reclaimable = r.reclaimable_bytes.unwrap_or(0);
        s.reclaimable_bytes += reclaimable;
        if r.at_risk() {
            s.at_risk_count += 1;
        } else {
            s.safe_reclaimable_bytes += reclaimable;
        }
        if matches!(r.activity, Activity::Stale | Activity::Abandoned) {
            s.stale_count += 1;
        }
        s.any_truncated |= r.truncated;
    }
    Ok(s)
}

/// Resolve an absolute path to reveal in the OS file manager.
///
/// `rel_path` is `None` for the repository root, or the repo-relative path of
/// one of its regenerable directories.
///
/// The relative path is checked against `repo_disk_dirs` rather than being
/// sanitised as a string: only a directory this repository's own measurement
/// recorded can be resolved. That is what keeps the corresponding command from
/// becoming "open any path on this machine", and rules out `..` traversal
/// without trying to out-guess path normalisation.
///
/// Returns `Ok(None)` when the repository does not exist or `rel_path` is not
/// one of its measured directories.
pub fn resolve_reveal_path(
    conn: &Connection,
    repo_id: i64,
    rel_path: Option<&str>,
) -> CoreResult<Option<String>> {
    let repo_path: Option<String> = conn
        .query_row("SELECT path FROM repos WHERE id = ?1", [repo_id], |r| {
            r.get(0)
        })
        .optional()?;
    let Some(repo_path) = repo_path else {
        return Ok(None);
    };

    let Some(rel) = rel_path else {
        return Ok(Some(repo_path));
    };

    let known: i64 = conn.query_row(
        "SELECT COUNT(*) FROM repo_disk_dirs WHERE repo_id = ?1 AND rel_path = ?2",
        rusqlite::params![repo_id, rel],
        |r| r.get(0),
    )?;
    if known == 0 {
        return Ok(None);
    }

    Ok(Some(format!("{}/{}", repo_path.trim_end_matches('/'), rel)))
}

/// Build the risk list for one repository.
///
/// A bare repository has no working tree, so dirt and stash counts are
/// meaningless for it (FR-1.6); only the missing-remote case applies.
fn assess_risks(
    is_bare: bool,
    dirty_modified: i64,
    dirty_staged: i64,
    dirty_untracked: i64,
    ahead: i64,
    remote_url: Option<&str>,
    has_stash: bool,
) -> Vec<Risk> {
    let mut risks = Vec::new();

    if !is_bare {
        let changed = dirty_modified + dirty_staged;
        if changed > 0 {
            risks.push(Risk {
                kind: RiskKind::UncommittedChanges,
                count: Some(changed),
            });
        }
        if dirty_untracked > 0 {
            risks.push(Risk {
                kind: RiskKind::UntrackedFiles,
                count: Some(dirty_untracked),
            });
        }
        if has_stash {
            risks.push(Risk {
                kind: RiskKind::Stash,
                count: None,
            });
        }
    }

    if ahead > 0 {
        risks.push(Risk {
            kind: RiskKind::UnpushedCommits,
            count: Some(ahead),
        });
    }
    if remote_url.is_none_or(str::is_empty) {
        risks.push(Risk {
            kind: RiskKind::NoRemote,
            count: None,
        });
    }

    risks
}

/// Whole days between an RFC-3339 timestamp and `now`. `None` if unparseable,
/// so bad stored data degrades to "unknown" rather than a wrong number.
fn days_since(timestamp: &str, now: chrono::DateTime<chrono::Utc>) -> Option<i64> {
    let then = chrono::DateTime::parse_from_rfc3339(timestamp).ok()?;
    Some((now - then.with_timezone(&chrono::Utc)).num_days().max(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activity_bands_key_off_the_thresholds() {
        assert_eq!(Activity::from_days(None), Activity::Unknown);
        assert_eq!(Activity::from_days(Some(0)), Activity::Active);
        assert_eq!(Activity::from_days(Some(89)), Activity::Active);
        assert_eq!(Activity::from_days(Some(90)), Activity::Dormant);
        assert_eq!(Activity::from_days(Some(364)), Activity::Dormant);
        assert_eq!(Activity::from_days(Some(365)), Activity::Stale);
        assert_eq!(Activity::from_days(Some(729)), Activity::Stale);
        assert_eq!(Activity::from_days(Some(730)), Activity::Abandoned);
    }

    #[test]
    fn a_clean_pushed_repo_with_a_remote_has_no_risks() {
        let risks = assess_risks(false, 0, 0, 0, 0, Some("git@example.com:x/y.git"), false);
        assert!(risks.is_empty(), "{risks:?}");
    }

    #[test]
    fn every_way_of_holding_unique_work_is_flagged() {
        let risks = assess_risks(false, 2, 1, 4, 3, None, true);
        let kinds: Vec<RiskKind> = risks.iter().map(|r| r.kind).collect();
        assert!(kinds.contains(&RiskKind::UncommittedChanges));
        assert!(kinds.contains(&RiskKind::UntrackedFiles));
        assert!(kinds.contains(&RiskKind::Stash));
        assert!(kinds.contains(&RiskKind::UnpushedCommits));
        assert!(kinds.contains(&RiskKind::NoRemote));

        // Modified and staged are summed into one count.
        let changed = risks
            .iter()
            .find(|r| r.kind == RiskKind::UncommittedChanges)
            .unwrap();
        assert_eq!(changed.count, Some(3));
    }

    /// An empty `remote_url` string is as good as no remote — the only copy is
    /// still this directory.
    #[test]
    fn an_empty_remote_url_counts_as_no_remote() {
        let risks = assess_risks(false, 0, 0, 0, 0, Some(""), false);
        assert_eq!(risks.len(), 1);
        assert_eq!(risks[0].kind, RiskKind::NoRemote);
    }

    /// A bare repo has no working tree, so working-tree risks must not be
    /// invented for it.
    #[test]
    fn a_bare_repo_reports_only_remote_risk() {
        let risks = assess_risks(true, 9, 9, 9, 0, Some("git@example.com:x/y.git"), true);
        assert!(risks.is_empty(), "{risks:?}");
    }
}
