//! Regression tests for the stability fixes: panic containment, write-mutex
//! poison recovery, and rows disappearing mid-scan.
//!
//! Each of these failure modes previously bricked the running process rather
//! than degrading one repository, which is what made repo-radar look like it
//! "crashed constantly".

mod support;

use repo_radar_core::db::{repos as repo_db, Db};
use repo_radar_core::model::{RepoIdentity, WarningKind};
use repo_radar_core::rules::RulePacks;
use repo_radar_core::scan::pipeline::{self, ScanContext, ScanRoot};
use repo_radar_core::scan::progress::RecordingReporter;
use repo_radar_core::scan::CancelToken;
use repo_radar_core::Paths;

use support::GitFixture;

fn rule_packs() -> RulePacks {
    let tmp = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(
        tmp.path().join("data"),
        tmp.path().join("cfg"),
        tmp.path().join("cache"),
    );
    RulePacks::load(&paths).expect("load shipped rule packs")
}

/// A panic inside a `db.write` closure must not make every *later* write
/// panic. The write connection used to sit behind a `Mutex` unlocked with
/// `.expect(...)`, so a single panic poisoned it and permanently broke every
/// scan, sync, and settings change for the life of the process.
#[test]
fn a_panicking_write_does_not_break_later_writes() {
    let db = Db::open_in_memory().expect("open");

    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        db.write(|_c| -> repo_radar_core::CoreResult<()> {
            panic!("boom inside a write closure");
        })
    }));
    assert!(panicked.is_err(), "the closure should have panicked");

    // The whole point: the pool recovers instead of propagating the poison.
    let root = db
        .write(|c| repo_db::add_scan_root(c, "/some/root"))
        .expect("a write after a poisoning panic must still succeed");
    assert_eq!(root.path, "/some/root");
}

/// A transaction left open by a panicking closure must be rolled back before
/// the connection is handed out again, or the next write fails with "cannot
/// start a transaction within a transaction".
#[test]
fn a_transaction_left_open_by_a_panic_is_rolled_back() {
    let db = Db::open_in_memory().expect("open");

    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        db.write(|c| -> repo_radar_core::CoreResult<()> {
            // Raw BEGIN: no RAII guard, so nothing rolls this back on unwind.
            c.execute_batch("BEGIN")?;
            c.execute_batch(
                "INSERT INTO scan_roots (path, added_at) VALUES ('/rolled/back', '2026-01-01T00:00:00Z')",
            )?;
            panic!("boom mid-transaction");
        })
    }));
    assert!(panicked.is_err());

    db.write(|c| repo_db::add_scan_root(c, "/after"))
        .expect("the next write must not hit a dangling transaction");

    // The panicking transaction's insert must not have survived.
    let conn = db.read().expect("read");
    let n: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM scan_roots WHERE path = '/rolled/back'",
            [],
            |r| r.get(0),
        )
        .expect("count");
    assert_eq!(n, 0, "the rolled-back insert must not be visible");
}

/// `guard_analysis` turns a panic into a warning on an otherwise-empty
/// analysis, so the repository still reaches the writer and still shows up in
/// the list — flagged — rather than silently vanishing.
#[test]
fn a_panicking_analysis_becomes_a_warning() {
    let identity = RepoIdentity {
        path: "/x/y".into(),
        name: "y".into(),
        is_bare: false,
        parent_path: None,
    };
    let analysis = pipeline::guard_analysis(&identity, || panic!("parser exploded"));

    assert_eq!(analysis.repo.path, "/x/y");
    assert_eq!(analysis.warnings.len(), 1);
    assert_eq!(analysis.warnings[0].kind, WarningKind::Panic);
}

/// A reporter that deletes every not-yet-written repository row the moment the
/// writer commits its first batch. That reproduces the real race — removing a
/// scan root (which cascades to its repos) or resetting the database while a
/// scan is in flight — deterministically, because `repo_done` fires from the
/// writer thread right after a batch commits and before later batches are
/// resolved.
struct DeleteRowsAfterFirstBatch<'a> {
    db: &'a Db,
    reported: std::sync::Mutex<Vec<String>>,
    fired: std::sync::atomic::AtomicBool,
    warnings: std::sync::Mutex<Vec<repo_radar_core::model::Warning>>,
}

impl repo_radar_core::scan::progress::ScanReporter for DeleteRowsAfterFirstBatch<'_> {
    fn discovered(&self, _total: usize) {}

    fn repo_done(&self, summary: &repo_radar_core::scan::progress::RepoSummary) {
        let mut reported = self.reported.lock().unwrap();
        reported.push(summary.path.clone());

        // The writer's batch size is 16, so the first flush reports 16 repos
        // and leaves the rest still to be resolved.
        if reported.len() < 16 || self.fired.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        let keep = reported.clone();
        drop(reported);

        // The write mutex is free here: `flush` commits its transaction and
        // releases the connection before calling `repo_done`.
        self.db
            .write(|c| {
                let placeholders = std::iter::repeat_n("?", keep.len())
                    .collect::<Vec<_>>()
                    .join(",");
                let sql = format!("DELETE FROM repos WHERE path NOT IN ({placeholders})");
                c.execute(&sql, rusqlite::params_from_iter(keep.iter()))
                    .map(|_| ())
                    .map_err(Into::into)
            })
            .expect("delete not-yet-written rows");
    }

    fn warning(&self, w: &repo_radar_core::model::Warning) {
        self.warnings.lock().unwrap().push(w.clone());
    }

    fn finished(&self, _summary: &repo_radar_core::scan::progress::ScanSummary) {}
}

/// Deleting a repository's row while a scan is in flight used to hit
/// `.expect("Unchanged implies an existing row")` *inside* `db.write`, which
/// panicked the writer thread **and poisoned the write mutex** — so from then
/// on every write in the process panicked too. It must now degrade to a
/// warning, with the scan finishing and the database still writable.
#[test]
fn rows_deleted_mid_scan_are_warnings_not_a_panic() {
    // More than the writer's batch size of 16, so at least one repository is
    // still unresolved when the first batch commits. Empty repos need no
    // commit, which keeps 20 `git init`s cheap, and still fingerprint stably.
    const REPOS: usize = 20;

    let fx = GitFixture::new();
    for i in 0..REPOS {
        fx.init_empty_repo(&format!("repo{i:02}"));
    }

    let db = Db::open_in_memory().expect("open");
    let rules = rule_packs();
    let ctx = ScanContext::new(&db, &rules);
    let root_path = fx.root().to_string_lossy().to_string();
    let root = db
        .write(|c| repo_db::add_scan_root(c, &root_path))
        .expect("add root");
    let roots = vec![ScanRoot {
        id: root.id,
        path: fx.root().to_path_buf(),
    }];

    // First scan: populate every row and store its fingerprint, so the second
    // scan classifies them all as `Unchanged`.
    let first = pipeline::begin_scan(&db).expect("begin");
    let summary = pipeline::run_scan(
        &ctx,
        first,
        &roots,
        &CancelToken::new(),
        &RecordingReporter::default(),
    )
    .expect("first scan");
    assert_eq!(
        summary.repos_scanned, REPOS,
        "first scan persists every repo"
    );

    // Second scan, with the rows pulled out from under the writer mid-flight.
    let reporter = DeleteRowsAfterFirstBatch {
        db: &db,
        reported: std::sync::Mutex::new(Vec::new()),
        fired: std::sync::atomic::AtomicBool::new(false),
        warnings: std::sync::Mutex::new(Vec::new()),
    };
    let second = pipeline::begin_scan(&db).expect("begin second");
    let result = pipeline::run_scan(&ctx, second, &roots, &CancelToken::new(), &reporter);

    assert!(
        result.is_ok(),
        "the scan must survive rows vanishing: {result:?}"
    );

    // The skipped rows are reported rather than swallowed.
    let warnings = reporter.warnings.lock().unwrap();
    assert!(
        warnings.iter().any(|w| w.message.contains("disappeared")),
        "expected a 'row disappeared' warning, got: {:?}",
        warnings.iter().map(|w| &w.message).collect::<Vec<_>>()
    );
    drop(warnings);

    // The whole point of the fix: the write connection is still usable.
    db.write(|c| repo_db::add_scan_root(c, "/still/works"))
        .expect("writes must still work after the scan");
}
