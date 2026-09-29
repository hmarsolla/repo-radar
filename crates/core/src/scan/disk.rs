//! Per-repository disk accounting: how much space a repo occupies, and how
//! much of that is regenerable build or vendor output.
//!
//! # Why this needs its own walk
//!
//! Every other stage of the scan *prunes* `node_modules`, `target`, `.venv`
//! and friends — that is the whole point of the prune list, because their
//! contents are not the user's code. Disk accounting wants exactly the
//! opposite: those directories are the interesting part, because they are the
//! space a developer can get back for free by deleting something a tool will
//! rebuild on demand.
//!
//! So this walk descends into pruned directories, but only to *total* them: it
//! sums a reclaimable directory's subtree and never looks at what is inside,
//! which keeps the cost to one `metadata` call per file rather than any
//! parsing or hashing.
//!
//! # Cost and bounds
//!
//! This is the most IO-heavy part of a scan — a single `node_modules` can hold
//! more files than the rest of the repository combined. Two bounds keep it
//! predictable:
//!
//! * [`CancelToken`] is checked as the walk proceeds, so **Cancel scan** stops
//!   it promptly rather than after the current repo finishes.
//! * [`MAX_ENTRIES`] caps entries visited per repository. Past the cap the walk
//!   stops and the result is marked [`DiskUsage::truncated`], so the UI can say
//!   "at least this much" instead of quietly under-reporting.
//!
//! Symlinks are never followed (a link into a shared store would be counted
//! against a repo that does not own the bytes, and a cycle would not
//! terminate), matching discovery's FR-1.8 behaviour.

use std::collections::BTreeMap;
use std::path::Path;

use crate::scan::CancelToken;

/// Upper bound on filesystem entries visited per repository. A quarter-million
/// covers a large monorepo with several `node_modules` trees; beyond it the
/// numbers are already good enough to act on and the walk is not worth the
/// wait.
pub const MAX_ENTRIES: usize = 250_000;

/// Directory names whose contents a build tool can regenerate.
///
/// Deliberately **not** the same list as
/// [`crate::scan::discovery::DEFAULT_PRUNE_DIRS`], even though they overlap
/// heavily. Pruning answers "is this the user's code?"; this answers "can this
/// be deleted and rebuilt?" — and the two differ. `vendor/` is pruned but is
/// often committed (Go's `vendor/`, PHP deployments), so deleting it can lose
/// work and it is not listed here. `.git` is not pruned but must never be
/// listed here either.
pub const RECLAIMABLE_DIRS: &[&str] = &[
    "node_modules",
    "target",
    ".venv",
    "venv",
    "__pycache__",
    ".next",
    ".nuxt",
    ".svelte-kit",
    ".turbo",
    ".parcel-cache",
    ".pytest_cache",
    ".mypy_cache",
    ".ruff_cache",
    ".gradle",
    "Pods",
    ".terraform",
    ".dart_tool",
];

/// One regenerable directory found inside a repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReclaimableDir {
    /// Repo-relative, `/`-separated path (e.g. `packages/web/node_modules`).
    pub rel_path: String,
    /// The directory's own name — which entry of [`RECLAIMABLE_DIRS`] matched.
    pub kind: String,
    pub bytes: u64,
    pub file_count: u64,
}

/// What a repository costs on disk, and how much of that is regenerable.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiskUsage {
    /// Every regular file under the repo, reclaimable directories included.
    pub total_bytes: u64,
    pub file_count: u64,
    /// Bytes inside `.git` — not reclaimable, but worth separating: a repo
    /// that is mostly history is a candidate for `git gc`, not for deletion.
    pub git_bytes: u64,
    /// Regenerable directories, largest first.
    pub reclaimable: Vec<ReclaimableDir>,
    /// [`MAX_ENTRIES`] was hit; every figure is a lower bound.
    pub truncated: bool,
}

impl DiskUsage {
    /// Total bytes across [`Self::reclaimable`].
    pub fn reclaimable_bytes(&self) -> u64 {
        self.reclaimable.iter().map(|d| d.bytes).sum()
    }

    /// Bytes that are neither regenerable output nor git history — the
    /// repository's actual content.
    pub fn source_bytes(&self) -> u64 {
        self.total_bytes
            .saturating_sub(self.reclaimable_bytes())
            .saturating_sub(self.git_bytes)
    }
}

/// Measure `repo_path`.
///
/// Never fails: an unreadable subtree contributes nothing and is skipped, the
/// same way discovery treats a permission error as a warning rather than an
/// abort (FR-1.10). Callers get a lower bound, never an error.
pub fn measure(repo_path: &Path, cancel: &CancelToken) -> DiskUsage {
    let mut usage = DiskUsage::default();
    // rel_path -> (kind, bytes, files), keyed so nested matches stay distinct.
    let mut reclaimable: BTreeMap<String, ReclaimableDir> = BTreeMap::new();
    let mut visited = 0usize;

    // Explicit stack rather than recursion: repository trees can nest
    // arbitrarily deep (a `node_modules` chain especially), and a recursive
    // walk would risk a stack overflow, which on Windows kills the process
    // outright with no unwind and no log line.
    let mut stack: Vec<std::path::PathBuf> = vec![repo_path.to_path_buf()];

    while let Some(dir) = stack.pop() {
        if cancel.is_cancelled() || visited >= MAX_ENTRIES {
            usage.truncated = usage.truncated || visited >= MAX_ENTRIES;
            break;
        }

        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue; // unreadable directory — skip, do not abort
        };

        for entry in entries.flatten() {
            visited += 1;
            if visited >= MAX_ENTRIES {
                usage.truncated = true;
                break;
            }

            // `DirEntry::metadata` deliberately does *not* traverse a symlink
            // (unlike `fs::metadata`), so a link is identified as a link and
            // skipped below rather than being resolved and double-counted.
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            let file_type = meta.file_type();

            if file_type.is_symlink() {
                continue;
            }

            if file_type.is_dir() {
                let name = entry.file_name().to_string_lossy().into_owned();

                if name == ".git" {
                    let (bytes, files) = subtree_total(&entry.path(), cancel, &mut visited);
                    usage.git_bytes += bytes;
                    usage.total_bytes += bytes;
                    usage.file_count += files;
                    continue;
                }

                if RECLAIMABLE_DIRS.contains(&name.as_str()) {
                    let path = entry.path();
                    let (bytes, files) = subtree_total(&path, cancel, &mut visited);
                    usage.total_bytes += bytes;
                    usage.file_count += files;

                    let rel_path = relative(repo_path, &path);
                    // A repeated path cannot happen from one walk, but merging
                    // rather than overwriting keeps this total-safe.
                    let slot = reclaimable
                        .entry(rel_path.clone())
                        .or_insert(ReclaimableDir {
                            rel_path,
                            kind: name,
                            bytes: 0,
                            file_count: 0,
                        });
                    slot.bytes += bytes;
                    slot.file_count += files;
                    continue;
                }

                stack.push(entry.path());
                continue;
            }

            if file_type.is_file() {
                usage.total_bytes += meta.len();
                usage.file_count += 1;
            }
        }
    }

    usage.reclaimable = reclaimable.into_values().collect();
    // Largest first: that is the order the user wants to act in.
    usage.reclaimable.sort_by(|a, b| {
        b.bytes
            .cmp(&a.bytes)
            .then_with(|| a.rel_path.cmp(&b.rel_path))
    });
    usage
}

/// Sum every regular file under `dir` without looking at what they are.
///
/// Used for subtrees we only need a number for (`.git`, a reclaimable
/// directory). Shares the caller's `visited` counter so one pathological
/// `node_modules` cannot blow past [`MAX_ENTRIES`] on its own.
fn subtree_total(dir: &Path, cancel: &CancelToken, visited: &mut usize) -> (u64, u64) {
    let mut bytes = 0u64;
    let mut files = 0u64;
    let mut stack = vec![dir.to_path_buf()];

    while let Some(d) = stack.pop() {
        if cancel.is_cancelled() || *visited >= MAX_ENTRIES {
            break;
        }
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for entry in entries.flatten() {
            *visited += 1;
            if *visited >= MAX_ENTRIES {
                break;
            }
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            let ft = meta.file_type();
            if ft.is_symlink() {
                continue;
            }
            if ft.is_dir() {
                stack.push(entry.path());
            } else if ft.is_file() {
                bytes += meta.len();
                files += 1;
            }
        }
    }
    (bytes, files)
}

/// Repo-relative, `/`-separated path, matching how paths are stored elsewhere.
fn relative(repo_path: &Path, path: &Path) -> String {
    path.strip_prefix(repo_path)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(path: &Path, bytes: usize) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, vec![b'x'; bytes]).unwrap();
    }

    #[test]
    fn separates_reclaimable_output_from_source_and_git() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        write(&root.join("src/main.rs"), 100);
        write(&root.join("Cargo.toml"), 50);
        write(&root.join("node_modules/left-pad/index.js"), 400);
        write(&root.join("node_modules/right-pad/index.js"), 600);
        write(&root.join("target/debug/app.exe"), 1500);
        write(&root.join(".git/objects/ab/cdef"), 200);

        let usage = measure(root, &CancelToken::new());

        assert_eq!(usage.total_bytes, 100 + 50 + 400 + 600 + 1500 + 200);
        assert_eq!(usage.git_bytes, 200);
        assert_eq!(usage.reclaimable_bytes(), 400 + 600 + 1500);
        assert_eq!(usage.source_bytes(), 150);
        assert!(!usage.truncated);

        // Largest first.
        let kinds: Vec<&str> = usage.reclaimable.iter().map(|d| d.kind.as_str()).collect();
        assert_eq!(kinds, vec!["target", "node_modules"]);
        let nm = usage
            .reclaimable
            .iter()
            .find(|d| d.kind == "node_modules")
            .unwrap();
        assert_eq!(nm.bytes, 1000);
        assert_eq!(nm.file_count, 2);
        assert_eq!(nm.rel_path, "node_modules");
    }

    #[test]
    fn reports_nested_reclaimable_dirs_with_their_relative_paths() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(&root.join("packages/web/node_modules/a/i.js"), 300);
        write(&root.join("packages/api/node_modules/b/i.js"), 700);
        write(&root.join("packages/web/src/app.ts"), 10);

        let usage = measure(root, &CancelToken::new());

        let paths: Vec<&str> = usage
            .reclaimable
            .iter()
            .map(|d| d.rel_path.as_str())
            .collect();
        assert_eq!(
            paths,
            vec!["packages/api/node_modules", "packages/web/node_modules"]
        );
        assert_eq!(usage.reclaimable_bytes(), 1000);
        assert_eq!(usage.source_bytes(), 10);
    }

    /// `vendor/` is pruned from analysis but is frequently committed, so it must
    /// not be advertised as free space.
    #[test]
    fn vendor_is_not_treated_as_reclaimable() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(&root.join("vendor/pkg/lib.go"), 500);

        let usage = measure(root, &CancelToken::new());
        assert!(usage.reclaimable.is_empty(), "{:?}", usage.reclaimable);
        assert_eq!(usage.source_bytes(), 500);
    }

    #[test]
    fn an_empty_tree_measures_zero() {
        let dir = tempfile::tempdir().unwrap();
        let usage = measure(dir.path(), &CancelToken::new());
        assert_eq!(usage, DiskUsage::default());
    }

    #[test]
    fn a_cancelled_walk_returns_early_without_panicking() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("a/b/c.txt"), 10);
        let cancel = CancelToken::new();
        cancel.cancel();
        let usage = measure(dir.path(), &cancel);
        assert_eq!(usage.total_bytes, 0);
    }
}
