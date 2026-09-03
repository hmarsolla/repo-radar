//! M1-3 acceptance: fixtures covering a normal repo, detached HEAD, an
//! empty repo, a dirty tree, and a repo with no upstream all produce
//! correct values with no errors. (The timeout path is unit-tested in
//! `scan::git`.)

mod support;

use std::time::Duration;

use repo_radar_core::scan::git::{extract, GitConfig};
use support::GitFixture;

fn cfg() -> GitConfig {
    GitConfig {
        timeout: Duration::from_secs(30),
    }
}

#[test]
fn normal_repo_reports_branch_commit_and_authors() {
    let fx = GitFixture::new();
    let repo = fx.init_repo("proj");
    fx.write("proj/second.txt", "more\n");
    fx.stage_and_commit("proj", "second commit");

    let info = extract(&repo, &cfg()).expect("extract");
    assert_eq!(info.branch.as_deref(), Some("main"));
    assert!(info.head_sha.is_some());
    assert_eq!(info.last_commit_summary.as_deref(), Some("second commit"));
    assert_eq!(info.commits_total, Some(2));
    assert_eq!(info.commits_90d, Some(2));
    assert_eq!(info.author_count, Some(1));
    assert!(!info.is_dirty());
    assert_eq!(info.ahead, None, "no upstream configured");
}

#[test]
fn empty_repo_is_not_an_error() {
    let fx = GitFixture::new();
    let repo = fx.init_empty_repo("fresh");

    let info = extract(&repo, &cfg()).expect("empty repo must not error (FR-7.9)");
    assert_eq!(info.head_sha, None);
    assert_eq!(info.last_commit_at, None);
    assert_eq!(info.commits_total, None);
    // The unborn branch name is still knowable.
    assert_eq!(info.branch.as_deref(), Some("main"));
}

#[test]
fn detached_head_has_no_branch_but_has_sha() {
    let fx = GitFixture::new();
    let repo = fx.init_repo("proj");
    fx.write("proj/x.txt", "x\n");
    fx.stage_and_commit("proj", "c2");
    // Detach at the first commit.
    let first = run_git(&repo, &["rev-list", "--max-parents=0", "HEAD"]);
    run_git(&repo, &["checkout", "-q", first.trim()]);

    let info = extract(&repo, &cfg()).expect("extract");
    assert_eq!(info.branch, None, "detached HEAD reports no branch");
    assert!(info.head_sha.is_some());
}

#[test]
fn dirty_tree_counts_modified_staged_and_untracked() {
    let fx = GitFixture::new();
    let repo = fx.init_repo("proj");

    // Modify the tracked README, stage a new file, leave another untracked.
    fx.write("proj/README.md", "changed\n");
    fx.write("proj/staged.txt", "staged\n");
    fx.write("proj/untracked.txt", "untracked\n");
    run_git(&repo, &["add", "staged.txt"]);

    let info = extract(&repo, &cfg()).expect("extract");
    assert!(info.is_dirty());
    assert_eq!(info.dirty_modified, Some(1));
    assert_eq!(info.dirty_staged, Some(1));
    assert_eq!(info.dirty_untracked, Some(1));
}

#[test]
fn gitignored_untracked_files_are_not_counted() {
    let fx = GitFixture::new();
    let repo = fx.init_repo("proj");
    fx.write("proj/.gitignore", "ignored/\n");
    fx.stage_and_commit("proj", "add gitignore");
    fx.write("proj/ignored/blob.bin", "x\n");

    let info = extract(&repo, &cfg()).expect("extract");
    assert_eq!(
        info.dirty_untracked,
        Some(0),
        "ignored files must not count (FR-7.5)"
    );
}

#[test]
fn upstream_ahead_behind_from_local_refs_only() {
    let fx = GitFixture::new();
    // Bare "remote", a working repo that pushes an initial commit to it and
    // sets up tracking, then one more local commit → 1 ahead / 0 behind.
    let bare = fx.init_bare("origin.git");
    let work = fx.init_repo("work");
    run_git(
        &work,
        &[
            "-c",
            "protocol.file.allow=always",
            "remote",
            "add",
            "origin",
            bare.to_str().unwrap(),
        ],
    );
    run_git(
        &work,
        &[
            "-c",
            "protocol.file.allow=always",
            "push",
            "-u",
            "origin",
            "main",
        ],
    );
    fx.write("work/a.txt", "a\n");
    fx.stage_and_commit("work", "local ahead 1");

    let info = extract(&work, &cfg()).expect("extract");
    assert_eq!(info.ahead, Some(1));
    assert_eq!(info.behind, Some(0));
    assert!(info.remote_url.as_deref().unwrap_or("").ends_with("origin"));
}

fn run_git(cwd: &std::path::Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .env("GIT_AUTHOR_NAME", "Fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.com")
        .env("GIT_COMMITTER_NAME", "Fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.com")
        .output()
        .expect("spawn git");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}
