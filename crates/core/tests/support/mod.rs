//! Test-only helpers for building fixture repository trees (DESIGN §16.5).
//!
//! These shell out to the real `git` binary — **only from tests**. Nothing
//! in `src/` ever spawns a subprocess (DESIGN §18). Each fixture lives in
//! its own `tempdir` and is isolated from the developer's global git config.

#![allow(dead_code)] // helpers are shared across several test binaries

use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

/// A temporary directory tree that fixture repos are created inside.
pub struct GitFixture {
    dir: TempDir,
}

impl GitFixture {
    pub fn new() -> Self {
        Self {
            dir: tempfile::tempdir().expect("create tempdir"),
        }
    }

    /// The root of the fixture tree — point discovery at this.
    pub fn root(&self) -> &Path {
        self.dir.path()
    }

    pub fn abs(&self, rel: &str) -> PathBuf {
        self.dir.path().join(rel)
    }

    /// Run `git` in `cwd` with an isolated environment and deterministic
    /// identity/config. Panics on non-zero exit.
    fn git(&self, cwd: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .env(
                "GIT_CONFIG_GLOBAL",
                self.dir.path().join("NON_EXISTENT_GLOBAL"),
            )
            .env(
                "GIT_CONFIG_SYSTEM",
                self.dir.path().join("NON_EXISTENT_SYSTEM"),
            )
            .env("GIT_AUTHOR_NAME", "Fixture")
            .env("GIT_AUTHOR_EMAIL", "fixture@example.com")
            .env("GIT_COMMITTER_NAME", "Fixture")
            .env("GIT_COMMITTER_EMAIL", "fixture@example.com")
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
            .unwrap_or_else(|e| panic!("failed to spawn git {args:?}: {e}"));
        assert!(
            out.status.success(),
            "git {args:?} in {} failed:\n{}",
            cwd.display(),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    pub fn write(&self, rel: &str, contents: &str) {
        let p = self.abs(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, contents).unwrap();
    }

    /// `git init` a working repo at `rel` and make one commit so it has a
    /// HEAD. Returns the repo's absolute path.
    pub fn init_repo(&self, rel: &str) -> PathBuf {
        let path = self.abs(rel);
        std::fs::create_dir_all(&path).unwrap();
        self.git(&path, &["init", "-q", "-b", "main"]);
        self.write(&format!("{rel}/README.md"), "fixture\n");
        self.git(&path, &["add", "-A"]);
        self.git(&path, &["commit", "-qm", "initial", "--no-gpg-sign"]);
        path
    }

    /// `git init` a repo with no commits (FR-7.9).
    pub fn init_empty_repo(&self, rel: &str) -> PathBuf {
        let path = self.abs(rel);
        std::fs::create_dir_all(&path).unwrap();
        self.git(&path, &["init", "-q", "-b", "main"]);
        path
    }

    /// Create a bare repo at `rel` (conventionally `<name>.git`).
    pub fn init_bare(&self, rel: &str) -> PathBuf {
        let path = self.abs(rel);
        std::fs::create_dir_all(&path).unwrap();
        self.git(&path, &["init", "-q", "--bare", "-b", "main"]);
        path
    }

    /// Add a linked worktree of `repo_rel` at `worktree_rel` (its `.git` is
    /// a pointer *file*, FR-1.7).
    pub fn add_worktree(&self, repo_rel: &str, worktree_rel: &str) -> PathBuf {
        let repo = self.abs(repo_rel);
        let wt = self.abs(worktree_rel);
        std::fs::create_dir_all(wt.parent().unwrap()).unwrap();
        self.git(
            &repo,
            &["worktree", "add", "-q", wt.to_str().unwrap(), "-b", "wt"],
        );
        wt
    }

    /// Make `child_rel` a submodule of `parent_rel` at `sub_path`. Uses a
    /// local `file://`-style path, which recent git blocks unless explicitly
    /// allowed.
    pub fn add_submodule(&self, parent_rel: &str, child_rel: &str, sub_path: &str) {
        let parent = self.abs(parent_rel);
        let child = self.abs(child_rel);
        self.git(
            &parent,
            &[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                "-q",
                child.to_str().unwrap(),
                sub_path,
            ],
        );
        self.git(
            &parent,
            &["commit", "-qm", "add submodule", "--no-gpg-sign"],
        );
    }

    pub fn stage_and_commit(&self, repo_rel: &str, msg: &str) {
        let repo = self.abs(repo_rel);
        self.git(&repo, &["add", "-A"]);
        self.git(&repo, &["commit", "-qm", msg, "--no-gpg-sign"]);
    }

    /// Best-effort directory symlink. Returns `false` (rather than failing
    /// the test) on platforms/permissions where it cannot be created — the
    /// walk-termination assertion still holds either way.
    pub fn symlink_dir(&self, target_rel: &str, link_rel: &str) -> bool {
        let target = self.abs(target_rel);
        let link = self.abs(link_rel);
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(target, link).is_ok()
        }
        #[cfg(windows)]
        {
            std::os::windows::fs::symlink_dir(target, link).is_ok()
        }
        #[cfg(not(any(unix, windows)))]
        {
            false
        }
    }
}

impl Default for GitFixture {
    fn default() -> Self {
        Self::new()
    }
}

/// Normalise a path the way `discovery` does, for comparing against
/// discovered `RepoIdentity::path` values.
pub fn norm(p: &Path) -> String {
    let abs = p
        .canonicalize()
        .unwrap_or_else(|_| p.to_path_buf())
        .to_string_lossy()
        .into_owned();
    abs.strip_prefix(r"\\?\").unwrap_or(&abs).replace('\\', "/")
}
