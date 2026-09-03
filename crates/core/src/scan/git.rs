//! Git metadata extraction (FR-7) via `git2`.
//!
//! All reads are local: ahead/behind comes from `graph_ahead_behind` on
//! refs already on disk — repo-radar **never** performs a network fetch
//! against a user's repository (FR-7.6). Empty repos (FR-7.9) and detached
//! HEAD (FR-7.1) are normal outcomes, not errors. Every call is bounded by a
//! timeout (FR-7.10) so one pathological repo cannot stall a scan.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use chrono::{DateTime, TimeZone, Utc};

use crate::model::GitInfo;

/// History walks (commit counts, author set) stop after this many commits.
/// A repo with a million-commit history should not make a scan crawl; the
/// resulting counts are labelled approximate above this bound (R9).
const MAX_HISTORY_WALK: usize = 20_000;

/// Commits within this window count toward `commits_90d` (FR-7.3).
const RECENT_WINDOW: chrono::Duration = chrono::Duration::days(90);

#[derive(Debug, Clone)]
pub struct GitConfig {
    /// Per-repo wall-clock budget (FR-7.10).
    pub timeout: Duration,
}

impl Default for GitConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(15),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("git extraction exceeded the {0:?} timeout")]
    Timeout(Duration),
    #[error("git worker thread panicked")]
    Panicked,
    #[error(transparent)]
    Libgit2(#[from] git2::Error),
}

/// Extract [`GitInfo`] for the repo at `path`, bounded by `cfg.timeout`.
///
/// `git2::Repository` is not `Send`, so the repo is opened *inside* the
/// worker thread. If the worker exceeds the budget it is abandoned (it will
/// finish and exit on its own); the scan continues with a `GitTimeout`
/// warning rather than blocking.
pub fn extract(path: &Path, cfg: &GitConfig) -> Result<GitInfo, GitError> {
    let owned = path.to_path_buf();
    run_with_timeout(cfg.timeout, move || extract_blocking(&owned))?
}

/// The actual extraction, without the timeout wrapper. Public for callers
/// that manage their own timing; most code wants [`extract`].
pub fn extract_blocking(path: &PathBuf) -> Result<GitInfo, GitError> {
    let repo = git2::Repository::open(path)?;
    let mut info = GitInfo::default();

    read_head_and_last_commit(&repo, &mut info)?;
    read_history_stats(&repo, &mut info);
    read_upstream(&repo, &mut info);
    read_remote(&repo, &mut info);
    read_branches_and_stash(&repo, &mut info);
    if !repo.is_bare() {
        read_worktree_status(&repo, &mut info);
    }

    Ok(info)
}

/// Run `f` on a worker thread, returning its value or [`GitError::Timeout`].
/// Exposed (crate-internal) so the timeout path can be tested with an
/// arbitrary slow closure.
pub(crate) fn run_with_timeout<T, F>(timeout: Duration, f: F) -> Result<T, GitError>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        // If the receiver is gone (timed out), the send just fails; drop it.
        let _ = tx.send(f());
    });
    match rx.recv_timeout(timeout) {
        Ok(v) => Ok(v),
        Err(mpsc::RecvTimeoutError::Timeout) => Err(GitError::Timeout(timeout)),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(GitError::Panicked),
    }
}

fn git_time_to_utc(t: git2::Time) -> Option<DateTime<Utc>> {
    Utc.timestamp_opt(t.seconds(), 0).single()
}

fn read_head_and_last_commit(repo: &git2::Repository, info: &mut GitInfo) -> Result<(), GitError> {
    let head = match repo.head() {
        Ok(h) => h,
        Err(e) if e.code() == git2::ErrorCode::UnbornBranch => {
            // FR-7.9 — a freshly `git init`-ed repo. Everything stays None;
            // the branch name is still meaningful.
            info.branch = repo
                .find_reference("HEAD")
                .ok()
                .and_then(|r| r.symbolic_target().ok().flatten().map(str::to_string))
                .and_then(|s| s.strip_prefix("refs/heads/").map(str::to_string));
            return Ok(());
        }
        Err(e) if e.code() == git2::ErrorCode::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };

    if repo.head_detached().unwrap_or(false) {
        // FR-7.1 — detached HEAD: no branch, carry the short SHA in branch
        // display via head_sha only.
        info.branch = None;
    } else {
        info.branch = head.shorthand().ok().map(|s| s.to_string());
    }

    if let Ok(commit) = head.peel_to_commit() {
        info.head_sha = Some(commit.id().to_string());
        info.last_commit_summary = commit.summary().ok().flatten().map(|s| s.to_string());
        info.last_commit_at = git_time_to_utc(commit.time());
    }
    Ok(())
}

fn read_history_stats(repo: &git2::Repository, info: &mut GitInfo) {
    let Ok(mut walk) = repo.revwalk() else { return };
    if walk.push_head().is_err() {
        return;
    }
    let cutoff = Utc::now() - RECENT_WINDOW;
    let mut total = 0u32;
    let mut recent = 0u32;
    let mut authors: std::collections::HashSet<String> = std::collections::HashSet::new();

    for oid in walk.flatten().take(MAX_HISTORY_WALK) {
        let Ok(commit) = repo.find_commit(oid) else {
            continue;
        };
        total += 1;
        if let Some(when) = git_time_to_utc(commit.time()) {
            if when >= cutoff {
                recent += 1;
            }
        }
        let sig = commit.author();
        if let Ok(email) = sig.email() {
            authors.insert(email.to_lowercase());
        }
    }

    info.commits_total = Some(total);
    info.commits_90d = Some(recent);
    info.author_count = Some(authors.len() as u32);
}

fn read_upstream(repo: &git2::Repository, info: &mut GitInfo) {
    // FR-7.6 — local refs only.
    let Ok(head) = repo.head() else { return };
    if !head.is_branch() {
        return;
    }
    let Ok(shorthand) = head.shorthand() else {
        return;
    };
    let Ok(local) = repo.find_branch(shorthand, git2::BranchType::Local) else {
        return;
    };
    let Ok(upstream) = local.upstream() else {
        return;
    };

    let (Some(local_oid), Some(up_oid)) = (local.get().target(), upstream.get().target()) else {
        return;
    };
    if let Ok((ahead, behind)) = repo.graph_ahead_behind(local_oid, up_oid) {
        info.ahead = Some(ahead as u32);
        info.behind = Some(behind as u32);
    }
}

fn read_remote(repo: &git2::Repository, info: &mut GitInfo) {
    let Ok(remote) = repo.find_remote("origin") else {
        return;
    };
    info.remote_url = remote.url().ok().map(normalize_remote_url);
}

/// FR-7.7 — strip credentials, render `git@host:path` as `host/path`.
fn normalize_remote_url(raw: &str) -> String {
    // scp-like: git@host:group/repo.git
    if !raw.contains("://") {
        if let Some((prefix, path)) = raw.split_once(':') {
            let host = prefix.rsplit('@').next().unwrap_or(prefix);
            return format!("{host}/{}", path.trim_start_matches('/'))
                .trim_end_matches(".git")
                .to_string();
        }
    }
    // URL form: scheme://[user[:pass]@]host[:port]/path
    if let Some((_scheme, rest)) = raw.split_once("://") {
        let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
        let host = authority.rsplit('@').next().unwrap_or(authority);
        let host = host.split(':').next().unwrap_or(host); // drop :port
        return format!("{host}/{path}")
            .trim_end_matches(".git")
            .to_string();
    }
    raw.trim_end_matches(".git").to_string()
}

fn read_branches_and_stash(repo: &git2::Repository, info: &mut GitInfo) {
    if let Ok(branches) = repo.branches(Some(git2::BranchType::Local)) {
        info.branch_count = Some(branches.flatten().count() as u32);
    }
    // Presence of `refs/stash` — cheaper than `stash_foreach`, and does not
    // need `&mut Repository`.
    info.has_stash = Some(repo.find_reference("refs/stash").is_ok());
}

fn read_worktree_status(repo: &git2::Repository, info: &mut GitInfo) {
    let mut opts = git2::StatusOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(true)
        .include_ignored(false) // FR-7.5 — untracked counted respecting .gitignore
        .exclude_submodules(true);

    let Ok(statuses) = repo.statuses(Some(&mut opts)) else {
        return;
    };

    let mut modified = 0u32;
    let mut staged = 0u32;
    let mut untracked = 0u32;
    for entry in statuses.iter() {
        let s = entry.status();
        if s.contains(git2::Status::WT_NEW) {
            untracked += 1;
        }
        if s.intersects(
            git2::Status::WT_MODIFIED
                | git2::Status::WT_DELETED
                | git2::Status::WT_RENAMED
                | git2::Status::WT_TYPECHANGE,
        ) {
            modified += 1;
        }
        if s.intersects(
            git2::Status::INDEX_NEW
                | git2::Status::INDEX_MODIFIED
                | git2::Status::INDEX_DELETED
                | git2::Status::INDEX_RENAMED
                | git2::Status::INDEX_TYPECHANGE,
        ) {
            staged += 1;
        }
    }
    info.dirty_modified = Some(modified);
    info.dirty_staged = Some(staged);
    info.dirty_untracked = Some(untracked);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeout_fires_for_a_slow_operation() {
        let start = std::time::Instant::now();
        let r: Result<u32, GitError> = run_with_timeout(Duration::from_millis(20), || {
            std::thread::sleep(Duration::from_secs(5));
            42
        });
        assert!(matches!(r, Err(GitError::Timeout(_))));
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "did not return promptly"
        );
    }

    #[test]
    fn fast_operation_returns_its_value() {
        let r: Result<u32, GitError> = run_with_timeout(Duration::from_secs(5), || 7);
        assert_eq!(r.unwrap(), 7);
    }

    #[test]
    fn remote_url_normalisation() {
        assert_eq!(
            normalize_remote_url("git@github.com:acme/widget.git"),
            "github.com/acme/widget"
        );
        assert_eq!(
            normalize_remote_url("https://user:token@gitlab.com/acme/widget.git"),
            "gitlab.com/acme/widget"
        );
        assert_eq!(
            normalize_remote_url("https://github.com/acme/widget"),
            "github.com/acme/widget"
        );
        assert_eq!(
            normalize_remote_url("ssh://git@example.com:2222/acme/widget.git"),
            "example.com/acme/widget"
        );
    }
}
