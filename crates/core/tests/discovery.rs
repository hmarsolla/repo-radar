//! M1-1 acceptance: against a fixture tree with a normal repo, a nested repo
//! inside `node_modules`, a worktree, a bare repo, and a symlink cycle,
//! exactly the expected repos are found, the `node_modules` repo is not
//! among them, and the walk terminates.

mod support;

use repo_radar_core::scan::discovery::{discover, DiscoveryConfig};
use repo_radar_core::scan::CancelToken;
use support::{norm, GitFixture};

#[test]
fn finds_exactly_the_expected_repos_and_terminates() {
    let fx = GitFixture::new();

    // A normal working repo.
    let normal = fx.init_repo("projects/alpha");

    // A repo vendored inside node_modules — must NOT be discovered (FR-1.3/1.4).
    fx.init_repo("projects/alpha/node_modules/vendored-pkg");

    // A second normal repo, with a nested repo *not* under a pruned dir:
    // the nested one must also be skipped because we stop at the parent's
    // `.git` (FR-1.3).
    let beta = fx.init_repo("projects/beta");
    fx.init_repo("projects/beta/embedded/thirdparty");

    // A linked worktree — `.git` is a pointer file (FR-1.7).
    let worktree = fx.add_worktree("projects/beta", "worktrees/beta-wt");

    // A bare repo (FR-1.6).
    let bare = fx.init_bare("mirrors/gamma.git");

    // A symlink cycle: link back to the tree root. Must not hang (FR-1.8).
    let made_symlink = fx.symlink_dir("", "projects/alpha/loop");

    let found = discover(fx.root(), &DiscoveryConfig::default(), &CancelToken::new());

    let paths: Vec<String> = found.repos.iter().map(|r| r.path.clone()).collect();

    let mut expected = vec![norm(&normal), norm(&beta), norm(&worktree), norm(&bare)];
    expected.sort();
    let mut got = paths.clone();
    got.sort();
    assert_eq!(got, expected, "discovered set mismatch");

    // The vendored + embedded repos are excluded.
    assert!(
        !paths.iter().any(|p| p.contains("node_modules")),
        "a repo under node_modules was discovered: {paths:?}"
    );
    assert!(
        !paths.iter().any(|p| p.ends_with("embedded/thirdparty")),
        "a repo nested inside another repo was discovered: {paths:?}"
    );

    // Bare repo flagged correctly.
    let bare_repo = found
        .repos
        .iter()
        .find(|r| r.path == norm(&bare))
        .expect("bare repo present");
    assert!(bare_repo.is_bare);
    assert_eq!(bare_repo.name, "gamma");

    // Non-bare repos flagged correctly.
    for r in found.repos.iter().filter(|r| r.path != norm(&bare)) {
        assert!(!r.is_bare, "{} wrongly flagged bare", r.path);
    }

    if !made_symlink {
        eprintln!(
            "note: directory symlink could not be created on this platform/permissions; \
                   cycle-termination still exercised by follow_links(false)"
        );
    }
}

#[test]
fn empty_root_yields_no_repos_and_no_panic() {
    let fx = GitFixture::new();
    fx.write("just/a/file.txt", "hi");
    let found = discover(fx.root(), &DiscoveryConfig::default(), &CancelToken::new());
    assert!(found.repos.is_empty());
}

#[test]
fn missing_root_is_a_warning_not_a_panic() {
    let cancel = CancelToken::new();
    let found = discover(
        std::path::Path::new("/definitely/not/a/real/path/xyzzy"),
        &DiscoveryConfig::default(),
        &cancel,
    );
    assert!(found.repos.is_empty());
    assert_eq!(found.warnings.len(), 1);
}

#[test]
fn cancellation_stops_the_walk() {
    let fx = GitFixture::new();
    for i in 0..10 {
        fx.init_repo(&format!("r{i}"));
    }
    let cancel = CancelToken::new();
    cancel.cancel();
    let found = discover(fx.root(), &DiscoveryConfig::default(), &cancel);
    assert!(
        found.repos.len() < 10,
        "cancelled walk still returned everything"
    );
}
