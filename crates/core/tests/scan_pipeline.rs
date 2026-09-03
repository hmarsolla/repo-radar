//! M1-5 acceptance: a 20-repo fixture scan completes, results land in the
//! DB, and a recording `ScanReporter` sees `repo_done` for every repo
//! *before* `finished` — proving results stream rather than batch.
//!
//! Also the M1-14 integration-harness shape: `tempdir` fixture tree +
//! in-memory SQLite + recording reporter, driving `run_scan` with no Tauri.

mod support;

use repo_radar_core::db::{repos as repo_db, Db};
use repo_radar_core::rules::RulePacks;
use repo_radar_core::scan::pipeline::{begin_scan, run_scan, ScanContext, ScanRoot};
use repo_radar_core::scan::progress::{RecordingReporter, ReporterEvent, ScanReporter};
use repo_radar_core::scan::CancelToken;
use repo_radar_core::Paths;
use support::GitFixture;

fn rule_packs() -> RulePacks {
    let tmp = tempfile::tempdir().unwrap();
    RulePacks::load(&Paths::under(tmp.path())).unwrap()
}

/// begin + run in one call, for tests.
fn scan(
    ctx: &ScanContext<'_>,
    roots: &[ScanRoot],
    cancel: &CancelToken,
    reporter: &dyn ScanReporter,
) -> repo_radar_core::scan::progress::ScanSummary {
    let id = begin_scan(ctx.db).unwrap();
    run_scan(ctx, id, roots, cancel, reporter).unwrap()
}

#[test]
fn twenty_repo_scan_streams_results_then_finishes() {
    let fx = GitFixture::new();
    for i in 0..20 {
        fx.init_repo(&format!("group{}/repo{i:02}", i % 3));
    }

    let db = Db::open_in_memory().unwrap();
    let rules = rule_packs();
    let ctx = ScanContext::new(&db, &rules);
    let reporter = RecordingReporter::default();
    let cancel = CancelToken::new();

    let roots = [ScanRoot {
        id: 1,
        path: fx.root().to_path_buf(),
    }];
    // The pipeline foreign-keys repos to scan_roots; register the root.
    db.write(|c| repo_db::add_scan_root(c, &fx.root().to_string_lossy()).map(|_| ()))
        .unwrap();

    let summary = scan(&ctx, &roots, &cancel, &reporter);

    assert_eq!(summary.repos_scanned, 20);
    assert!(!summary.cancelled);

    // All 20 persisted and queryable.
    let listed = db
        .read()
        .map(|c| repo_db::list_top_level(&c))
        .unwrap()
        .unwrap();
    assert_eq!(listed.len(), 20);
    assert!(listed.iter().all(|r| r.primary_language.is_some()));

    // Event ordering: Discovered first, every RepoDone before Finished.
    let events = reporter.events.lock().unwrap();
    let discovered_idx = events
        .iter()
        .position(|e| matches!(e, ReporterEvent::Discovered(20)))
        .expect("discovered(20) emitted");
    let finished_idx = events
        .iter()
        .position(|e| matches!(e, ReporterEvent::Finished(_)))
        .expect("finished emitted");
    let repo_done_count = events
        .iter()
        .filter(|e| matches!(e, ReporterEvent::RepoDone(_)))
        .count();
    let last_repo_done_idx = events
        .iter()
        .rposition(|e| matches!(e, ReporterEvent::RepoDone(_)))
        .expect("at least one repo_done");

    assert_eq!(repo_done_count, 20);
    assert!(
        discovered_idx < last_repo_done_idx,
        "discovered must precede repo_done"
    );
    assert!(
        last_repo_done_idx < finished_idx,
        "every repo_done must precede finished — results stream, not batch"
    );
}

#[test]
fn rescan_updates_rows_in_place() {
    let fx = GitFixture::new();
    fx.init_repo("solo");

    let db = Db::open_in_memory().unwrap();
    let rules = rule_packs();
    let ctx = ScanContext::new(&db, &rules);
    let cancel = CancelToken::new();
    db.write(|c| repo_db::add_scan_root(c, &fx.root().to_string_lossy()).map(|_| ()))
        .unwrap();
    let roots = [ScanRoot {
        id: 1,
        path: fx.root().to_path_buf(),
    }];

    scan(&ctx, &roots, &cancel, &RecordingReporter::default());
    scan(&ctx, &roots, &cancel, &RecordingReporter::default());

    let listed = db
        .read()
        .map(|c| repo_db::list_top_level(&c))
        .unwrap()
        .unwrap();
    assert_eq!(listed.len(), 1, "second scan must update, not duplicate");
}
