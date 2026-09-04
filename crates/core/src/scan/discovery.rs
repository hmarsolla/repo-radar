//! Repository discovery walker (FR-1, DESIGN §6.6).
//!
//! Walks each scan root with the `ignore` crate, records every directory
//! that contains a `.git` entry (directory, or a worktree pointer *file*),
//! and **stops descending there** so a vendored dependency carrying its own
//! `.git` is never reported as a user project (FR-1.3). Bare repositories
//! are detected and flagged (FR-1.6); symlinks are never followed (FR-1.8);
//! permission errors become warnings rather than aborting (FR-1.10).
//!
//! Submodules are *not* discovered here — the walk stops at the parent's
//! `.git`. They are attached from the parent's `.gitmodules` in
//! [`crate::scan::submodule`] (M1-2).

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use ignore::{WalkBuilder, WalkState};

use crate::model::{RepoIdentity, Warning, WarningKind, WarningScope};
use crate::scan::CancelToken;

/// Inputs to a discovery pass.
pub struct DiscoveryConfig {
    /// Directory names never descended into (FR-1.4), from settings.
    pub prune_dirs: Vec<String>,
}

impl Default for DiscoveryConfig {
    fn default() -> Self {
        Self {
            prune_dirs: DEFAULT_PRUNE_DIRS.iter().map(|s| s.to_string()).collect(),
        }
    }
}

/// The prune list from FR-1.4. `src-tauri` overrides this from user
/// settings; kept here so `core` tests and any headless use have a sane
/// default.
pub const DEFAULT_PRUNE_DIRS: &[&str] = &[
    "node_modules",
    "target",
    ".venv",
    "venv",
    "__pycache__",
    "dist",
    "build",
    ".next",
    ".nuxt",
    "vendor",
    ".cargo",
    ".gradle",
    "Pods",
    ".terraform",
];

/// Result of walking one root.
#[derive(Debug, Default)]
pub struct Discovery {
    pub repos: Vec<RepoIdentity>,
    pub warnings: Vec<Warning>,
}

/// Walk `root` and return every repository found under it. Deterministic
/// order (sorted by path) so callers and tests see a stable list.
pub fn discover(root: &Path, config: &DiscoveryConfig, cancel: &CancelToken) -> Discovery {
    let found: Mutex<Vec<RepoIdentity>> = Mutex::new(Vec::new());
    let warnings: Mutex<Vec<Warning>> = Mutex::new(Vec::new());

    if !root.exists() {
        return Discovery {
            repos: Vec::new(),
            warnings: vec![Warning::new(
                WarningScope::Scan,
                WarningKind::Other,
                format!("scan root does not exist: {}", root.display()),
            )],
        };
    }

    let prune: Vec<String> = config.prune_dirs.clone();

    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(false) //  .git is hidden; we must see it
        .follow_links(false) // FR-1.8 — no symlink cycles
        .git_ignore(false) // we are above the repo level (DESIGN §6.6)
        .git_global(false)
        .git_exclude(false)
        .ignore(false)
        .parents(false)
        .require_git(false)
        .filter_entry(move |entry| {
            // Prune by directory name (FR-1.4). Files are never pruned here.
            if entry.file_type().is_some_and(|ft| ft.is_dir()) {
                if let Some(name) = entry.file_name().to_str() {
                    return !prune.iter().any(|p| p == name);
                }
            }
            true
        });

    builder.build_parallel().run(|| {
        let found = &found;
        let warnings = &warnings;
        let cancel = cancel.clone();
        Box::new(move |result| {
            if cancel.is_cancelled() {
                return WalkState::Quit;
            }

            let entry = match result {
                Ok(e) => e,
                Err(err) => {
                    warnings.lock().unwrap().push(classify_walk_error(&err));
                    return WalkState::Continue;
                }
            };

            // Only directories can be repository roots.
            if !entry.file_type().is_some_and(|ft| ft.is_dir()) {
                return WalkState::Continue;
            }
            let path = entry.path();

            match probe_repo(path) {
                RepoProbe::NotARepo => WalkState::Continue,
                RepoProbe::Repo(identity) => {
                    found.lock().unwrap().push(identity);
                    // FR-1.3 — do not descend into a discovered repository.
                    WalkState::Skip
                }
                RepoProbe::Broken(msg) => {
                    warnings.lock().unwrap().push(Warning::new(
                        WarningScope::Scan,
                        WarningKind::GitError,
                        msg,
                    ));
                    // It looked like a repo; still don't descend into it.
                    WalkState::Skip
                }
            }
        })
    });

    let mut repos = found.into_inner().unwrap();
    repos.sort_by(|a, b| a.path.cmp(&b.path));
    repos.dedup_by(|a, b| a.path == b.path);

    Discovery {
        repos,
        warnings: warnings.into_inner().unwrap(),
    }
}

enum RepoProbe {
    NotARepo,
    Repo(RepoIdentity),
    Broken(String),
}

/// Decide whether `dir` is a repository root without descending into it.
fn probe_repo(dir: &Path) -> RepoProbe {
    let dot_git = dir.join(".git");
    let has_dot_git = dot_git.exists(); // dir (normal) or file (worktree, FR-1.7)
    let looks_bare = looks_like_bare_repo(dir);

    if !has_dot_git && !looks_bare {
        return RepoProbe::NotARepo;
    }

    // Let libgit2 resolve `.git` directories, worktree pointer files, and
    // bare layouts uniformly.
    let open = if looks_bare && !has_dot_git {
        git2::Repository::open_bare(dir)
    } else {
        git2::Repository::open(dir)
    };

    match open {
        Ok(repo) => RepoProbe::Repo(identity_from_repo(dir, &repo)),
        Err(e) => RepoProbe::Broken(format!(
            "{} looks like a repository but could not be opened: {}",
            dir.display(),
            e.message()
        )),
    }
}

/// Cheap heuristic for a bare repo: either a `*.git` directory, or a
/// directory that directly holds a `HEAD` file and an `objects/` directory.
fn looks_like_bare_repo(dir: &Path) -> bool {
    let named_git = dir
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.ends_with(".git"));
    let has_git_layout = dir.join("HEAD").is_file() && dir.join("objects").is_dir();
    named_git || has_git_layout
}

fn identity_from_repo(dir: &Path, repo: &git2::Repository) -> RepoIdentity {
    let is_bare = repo.is_bare();
    let raw_name = dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string();
    let name = if is_bare {
        raw_name
            .strip_suffix(".git")
            .unwrap_or(&raw_name)
            .to_string()
    } else {
        raw_name
    };
    RepoIdentity {
        path: normalize(dir),
        name,
        is_bare,
        parent_path: None,
    }
}

/// Absolute path with forward slashes, so identities are stable across OSes
/// and match how paths are stored in the database.
fn normalize(p: &Path) -> String {
    let abs = p
        .canonicalize()
        .unwrap_or_else(|_| p.to_path_buf())
        .to_string_lossy()
        .into_owned();
    abs.strip_prefix(r"\\?\").unwrap_or(&abs).replace('\\', "/")
}

/// `WalkParallel`'s directory-read errors wrap the underlying `io::Error` at
/// varying depths (`WithDepth { err: WithPath { err: Io(..) } }` for a
/// directory that fails to open, vs. `WithPath { err: Io(..) }` for a
/// single bad entry within an otherwise-readable directory) — the exact
/// nesting is an implementation detail of the `ignore` crate, not something
/// safe to pattern-match on directly. `io_error()`/`find_path()` recurse
/// through every wrapper variant instead, so classification doesn't depend
/// on which shape a given failure happens to produce.
fn classify_walk_error(err: &ignore::Error) -> Warning {
    let is_permission = matches!(
        err.io_error().map(std::io::Error::kind),
        Some(std::io::ErrorKind::PermissionDenied)
    );
    let kind = if is_permission {
        WarningKind::PermissionDenied
    } else {
        WarningKind::Other
    };
    let scope = match find_path(err) {
        Some(path) => WarningScope::File(normalize(path)),
        None => WarningScope::Scan,
    };
    Warning::new(scope, kind, format!("discovery: {err}"))
}

/// Recurse through `ignore::Error`'s wrapper variants to find the path a
/// walk error is attached to, mirroring how the crate's own `io_error()`
/// recurses to find the underlying `io::Error`.
fn find_path(err: &ignore::Error) -> Option<&Path> {
    match err {
        ignore::Error::WithPath { path, .. } => Some(path),
        ignore::Error::WithDepth { err, .. } => find_path(err),
        ignore::Error::WithLineNumber { err, .. } => find_path(err),
        ignore::Error::Partial(errs) if errs.len() == 1 => find_path(&errs[0]),
        _ => None,
    }
}

/// Convenience for callers with more than one root.
pub fn discover_all(
    roots: &[PathBuf],
    config: &DiscoveryConfig,
    cancel: &CancelToken,
) -> Discovery {
    let mut all = Discovery::default();
    for root in roots {
        if cancel.is_cancelled() {
            break;
        }
        let mut d = discover(root, config, cancel);
        all.repos.append(&mut d.repos);
        all.warnings.append(&mut d.warnings);
    }
    all.repos.sort_by(|a, b| a.path.cmp(&b.path));
    all.repos.dedup_by(|a, b| a.path == b.path);
    all
}
