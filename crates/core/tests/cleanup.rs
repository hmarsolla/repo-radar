//! End-to-end coverage for the Cleanup view's data: a real scan must produce
//! disk figures and triage that distinguish "safe to clear" from "holds work
//! that exists nowhere else".

mod support;

use repo_radar_core::db::cleanup::{self, Activity, CleanupFilter, CleanupSort, RiskKind};
use repo_radar_core::db::{repos as repo_db, Db};
use repo_radar_core::rules::RulePacks;
use repo_radar_core::scan::pipeline::{self, ScanContext, ScanRoot};
use repo_radar_core::scan::progress::RecordingReporter;
use repo_radar_core::scan::CancelToken;
use repo_radar_core::Paths;

use support::GitFixture;

fn rule_packs() -> (tempfile::TempDir, RulePacks) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(
        tmp.path().join("data"),
        tmp.path().join("cfg"),
        tmp.path().join("cache"),
    );
    let packs = RulePacks::load(&paths).expect("load shipped rule packs");
    (tmp, packs)
}

/// Scan `fx`'s root once and hand back the database.
fn scan(fx: &GitFixture) -> Db {
    let db = Db::open_in_memory().expect("open");
    let (_guard, rules) = rule_packs();
    let ctx = ScanContext::new(&db, &rules);
    let root_path = fx.root().to_string_lossy().to_string();
    let root = db
        .write(|c| repo_db::add_scan_root(c, &root_path))
        .expect("add root");
    let roots = vec![ScanRoot {
        id: root.id,
        path: fx.root().to_path_buf(),
    }];
    let scan_id = pipeline::begin_scan(&db).expect("begin");
    pipeline::run_scan(
        &ctx,
        scan_id,
        &roots,
        &CancelToken::new(),
        &RecordingReporter::default(),
    )
    .expect("scan");
    db
}

#[test]
fn a_scan_records_reclaimable_directories_per_repo() {
    let fx = GitFixture::new();
    fx.init_repo("app");
    fx.write("app/src/index.js", "console.log(1)\n");
    // Regenerable output: pruned from analysis, but the point of this feature.
    fx.write("app/node_modules/left-pad/index.js", &"x".repeat(4096));
    fx.write("app/target/debug/app", &"y".repeat(2048));
    fx.stage_and_commit("app", "work");

    let db = scan(&fx);
    let conn = db.read().expect("read");
    let rows = cleanup::list(&conn, &CleanupFilter::default()).expect("list");

    let app = rows.iter().find(|r| r.name == "app").expect("app row");
    assert!(
        app.measured_at.is_some(),
        "the scan must record a measurement time"
    );
    assert_eq!(app.reclaimable_bytes, Some(4096 + 2048));
    assert!(
        app.total_bytes.unwrap_or(0) > 4096 + 2048,
        "total must include source and .git too, got {:?}",
        app.total_bytes
    );
    assert!(
        app.git_bytes.unwrap_or(0) > 0,
        "a repo with a commit has .git bytes"
    );
    assert!(!app.truncated);

    // Largest directory first, with its repo-relative path.
    let kinds: Vec<&str> = app
        .reclaimable_dirs
        .iter()
        .map(|d| d.kind.as_str())
        .collect();
    assert_eq!(kinds, vec!["node_modules", "target"]);
    assert_eq!(app.reclaimable_dirs[0].rel_path, "node_modules");
}

/// The load-bearing distinction: a repo with unpushed or uncommitted work is not
/// a cleanup candidate however much disk it wastes.
#[test]
fn uncommitted_work_is_flagged_and_excluded_from_safe_totals() {
    let fx = GitFixture::new();
    fx.init_repo("risky");
    fx.write("risky/node_modules/pkg/i.js", &"x".repeat(1024));
    // Uncommitted tracked change plus an untracked file.
    fx.write("risky/README.md", "edited after the commit\n");
    fx.write("risky/scratch.txt", "untracked\n");

    let db = scan(&fx);
    let conn = db.read().expect("read");
    let rows = cleanup::list(&conn, &CleanupFilter::default()).expect("list");
    let risky = rows.iter().find(|r| r.name == "risky").expect("row");

    assert!(risky.at_risk(), "risks: {:?}", risky.risks);
    let kinds: Vec<RiskKind> = risky.risks.iter().map(|r| r.kind).collect();
    assert!(
        kinds.contains(&RiskKind::UncommittedChanges),
        "expected uncommitted changes in {kinds:?}"
    );
    assert!(
        kinds.contains(&RiskKind::UntrackedFiles),
        "expected untracked files in {kinds:?}"
    );
    // A fixture repo has no remote, so its only copy is this directory.
    assert!(
        kinds.contains(&RiskKind::NoRemote),
        "expected no-remote in {kinds:?}"
    );

    let summary = cleanup::summary(&conn).expect("summary");
    assert_eq!(summary.at_risk_count, 1);
    assert_eq!(summary.reclaimable_bytes, 1024);
    assert_eq!(
        summary.safe_reclaimable_bytes, 0,
        "space in an at-risk repo must not be advertised as safe"
    );
}

#[test]
fn summary_totals_add_up_across_repos() {
    let fx = GitFixture::new();
    for (name, size) in [("one", 1000), ("two", 2000), ("three", 3000)] {
        fx.init_repo(name);
        fx.write(&format!("{name}/node_modules/p/i.js"), &"x".repeat(size));
        fx.stage_and_commit(name, "clean");
    }

    let db = scan(&fx);
    let conn = db.read().expect("read");
    let summary = cleanup::summary(&conn).expect("summary");

    assert_eq!(summary.repo_count, 3);
    assert_eq!(summary.measured_count, 3);
    assert_eq!(summary.reclaimable_bytes, 6000);
    // Every repo is committed and clean, but none has a remote — so none counts
    // as safe. That is the intended conservative default.
    assert_eq!(summary.at_risk_count, 3);
    assert!(!summary.any_truncated);
}

#[test]
fn sorting_and_filtering_behave() {
    let fx = GitFixture::new();
    fx.init_repo("small");
    fx.write("small/node_modules/p/i.js", &"x".repeat(100));
    fx.stage_and_commit("small", "c");
    fx.init_repo("large");
    fx.write("large/node_modules/p/i.js", &"x".repeat(9000));
    fx.stage_and_commit("large", "c");

    let db = scan(&fx);
    let conn = db.read().expect("read");

    let by_reclaimable = cleanup::list(
        &conn,
        &CleanupFilter {
            sort: CleanupSort::Reclaimable,
            ..Default::default()
        },
    )
    .expect("list");
    assert_eq!(by_reclaimable[0].name, "large");

    let by_name = cleanup::list(
        &conn,
        &CleanupFilter {
            sort: CleanupSort::Name,
            ..Default::default()
        },
    )
    .expect("list");
    assert_eq!(
        by_name.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
        vec!["large", "small"]
    );

    // Fresh fixture repos are Active, so a stale-only filter finds nothing.
    let stale = cleanup::list(
        &conn,
        &CleanupFilter {
            stale_only: true,
            ..Default::default()
        },
    )
    .expect("list");
    assert!(stale.is_empty(), "{stale:?}");
    assert!(by_reclaimable
        .iter()
        .all(|r| r.activity == Activity::Active));
}

/// `reveal_path` must only resolve directories the scan actually measured —
/// otherwise the command becomes "open any path on this machine".
#[test]
fn reveal_path_refuses_paths_it_did_not_measure() {
    let fx = GitFixture::new();
    fx.init_repo("app");
    fx.write("app/node_modules/p/i.js", &"x".repeat(64));
    fx.stage_and_commit("app", "c");

    let db = scan(&fx);
    let conn = db.read().expect("read");
    let rows = cleanup::list(&conn, &CleanupFilter::default()).expect("list");
    let app = rows.iter().find(|r| r.name == "app").expect("row");

    // The repo root resolves.
    let root = cleanup::resolve_reveal_path(&conn, app.repo_id, None).expect("resolve");
    assert_eq!(root.as_deref(), Some(app.path.as_str()));

    // A measured directory resolves, under the repo.
    let measured =
        cleanup::resolve_reveal_path(&conn, app.repo_id, Some("node_modules")).expect("resolve");
    assert_eq!(
        measured,
        Some(format!("{}/node_modules", app.path)),
        "a measured directory must resolve beneath its repo"
    );

    // Anything else does not — including traversal attempts and real-but-
    // unmeasured directories.
    for rejected in ["../../../etc", "src", "node_modules/p", ".git", ""] {
        assert_eq!(
            cleanup::resolve_reveal_path(&conn, app.repo_id, Some(rejected)).expect("resolve"),
            None,
            "{rejected:?} must not resolve"
        );
    }

    // An unknown repo id resolves to nothing rather than erroring.
    assert_eq!(
        cleanup::resolve_reveal_path(&conn, 99_999, None).expect("resolve"),
        None
    );
}

/// Disk usage must be refreshed even when the incremental fingerprint says the
/// repository is unchanged — `npm install` changes the footprint without
/// touching anything the fingerprint covers.
#[test]
fn disk_usage_is_remeasured_for_fingerprint_unchanged_repos() {
    let fx = GitFixture::new();
    // `init_repo` already makes the initial commit, so the tree starts clean.
    fx.init_repo("app");

    let db = Db::open_in_memory().expect("open");
    let (_guard, rules) = rule_packs();
    let ctx = ScanContext::new(&db, &rules);
    let root_path = fx.root().to_string_lossy().to_string();
    let root = db
        .write(|c| repo_db::add_scan_root(c, &root_path))
        .expect("add root");
    let roots = vec![ScanRoot {
        id: root.id,
        path: fx.root().to_path_buf(),
    }];

    let first = pipeline::begin_scan(&db).expect("begin");
    pipeline::run_scan(
        &ctx,
        first,
        &roots,
        &CancelToken::new(),
        &RecordingReporter::default(),
    )
    .expect("first scan");

    let before = {
        let conn = db.read().expect("read");
        cleanup::list(&conn, &CleanupFilter::default()).expect("list")[0]
            .reclaimable_bytes
            .unwrap_or(0)
    };
    assert_eq!(before, 0, "no build output yet");

    // Add build output only. `node_modules` is pruned from manifest discovery,
    // so the fingerprint is unchanged and analysis is skipped — the disk
    // measurement must still run.
    fx.write("app/node_modules/pkg/index.js", &"x".repeat(5000));

    let second = pipeline::begin_scan(&db).expect("begin");
    pipeline::run_scan(
        &ctx,
        second,
        &roots,
        &CancelToken::new(),
        &RecordingReporter::default(),
    )
    .expect("second scan");

    let conn = db.read().expect("read");
    let rows = cleanup::list(&conn, &CleanupFilter::default()).expect("list");
    assert_eq!(
        rows[0].reclaimable_bytes,
        Some(5000),
        "an unchanged repo must still get a fresh footprint"
    );
    assert_eq!(
        rows[0].reclaimable_dirs.len(),
        1,
        "and its directory breakdown"
    );
}
