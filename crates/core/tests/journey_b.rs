//! M2-18 / Journey B: the advisory database changes independently of the
//! code, so a repo nobody touched can become unhealthy. Stage 4 (match +
//! score) must re-run on every scan regardless of the incremental
//! fingerprint (DESIGN §6.5) — this test is what guarantees it.

mod support;

use repo_radar_core::db::{repos as repo_db, Db};
use repo_radar_core::rules::RulePacks;
use repo_radar_core::scan::pipeline::{begin_scan, run_scan, ScanContext, ScanRoot};
use repo_radar_core::scan::progress::RecordingReporter;
use repo_radar_core::scan::CancelToken;
use repo_radar_core::Paths;
use support::{insert_advisory, GitFixture};

const LOCK: &str = r#"{
  "lockfileVersion": 3,
  "packages": {
    "": { "dependencies": { "left-pad": "1.0.0" } },
    "node_modules/left-pad": { "version": "1.0.0" }
  }
}"#;

fn scanned_repo() -> (GitFixture, Db, RulePacks) {
    let fx = GitFixture::new();
    fx.init_repo("app");
    fx.write(
        "app/package.json",
        r#"{"dependencies":{"left-pad":"1.0.0"}}"#,
    );
    fx.write("app/package-lock.json", LOCK);
    fx.stage_and_commit("app", "add deps");

    let db = Db::open_in_memory().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let rules = RulePacks::load(&Paths::under(tmp.path())).unwrap();
    db.write(|c| repo_db::add_scan_root(c, &fx.root().to_string_lossy()).map(|_| ()))
        .unwrap();
    (fx, db, rules)
}

fn rescan(db: &Db, rules: &RulePacks, root: &std::path::Path) {
    let ctx = ScanContext::new(db, rules);
    let roots = [ScanRoot {
        id: 1,
        path: root.to_path_buf(),
    }];
    let id = begin_scan(db).unwrap();
    run_scan(
        &ctx,
        id,
        &roots,
        &CancelToken::new(),
        &RecordingReporter::default(),
    )
    .unwrap();
}

fn health(db: &Db) -> (Option<i64>, Option<String>, i64) {
    let conn = db.read().unwrap();
    let (score, band): (Option<i64>, Option<String>) = conn
        .query_row(
            "SELECT health_score, health_band FROM repos WHERE parent_repo_id IS NULL LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    let findings: i64 = conn
        .query_row("SELECT COUNT(*) FROM findings", [], |r| r.get(0))
        .unwrap();
    (score, band, findings)
}

#[test]
fn a_new_vulnerability_lowers_an_untouched_repos_score_on_rescan() {
    let (fx, db, rules) = scanned_repo();

    // First scan: no advisories synced → Unknown health, no findings.
    rescan(&db, &rules, fx.root());
    let (score1, band1, findings1) = health(&db);
    assert_eq!(findings1, 0);
    assert_eq!(
        band1.as_deref(),
        Some("unknown"),
        "no sync => unknown, not healthy"
    );
    assert_eq!(score1, Some(100));

    // The advisory database changes: left-pad < 1.3.0 is now vulnerable.
    insert_advisory(
        &db,
        "GHSA-left-pad-1",
        "vulnerability",
        "high",
        "npm",
        "left-pad",
        r#"[{"introduced":"0"},{"fixed":"1.3.0"}]"#,
    );

    // Re-scan. Nothing in the repo changed — the fingerprint is identical —
    // but stage 4 must still run.
    rescan(&db, &rules, fx.root());
    let (score2, band2, findings2) = health(&db);

    assert_eq!(
        findings2, 1,
        "the new advisory produced a finding on rescan"
    );
    assert!(score2.unwrap() < 100, "score dropped: {score2:?}");
    assert_ne!(
        band2.as_deref(),
        Some("unknown"),
        "advisories exist now: {band2:?}"
    );
}

#[test]
fn a_new_compromise_caps_the_repo_at_critical_on_rescan() {
    let (fx, db, rules) = scanned_repo();
    rescan(&db, &rules, fx.root());

    insert_advisory(
        &db,
        "MAL-left-pad-666",
        "compromise",
        "unscored",
        "npm",
        "left-pad",
        r#"[{"introduced":"0"}]"#,
    );

    rescan(&db, &rules, fx.root());
    let (score, band, findings) = health(&db);
    assert_eq!(findings, 1);
    assert!(score.unwrap() <= 39, "compromise cap: {score:?}");
    assert_eq!(band.as_deref(), Some("critical"));
}

#[test]
fn withdrawing_the_advisory_restores_health_on_rescan() {
    let (fx, db, rules) = scanned_repo();
    insert_advisory(
        &db,
        "GHSA-temp",
        "vulnerability",
        "high",
        "npm",
        "left-pad",
        r#"[{"introduced":"0"},{"fixed":"1.3.0"}]"#,
    );
    rescan(&db, &rules, fx.root());
    assert!(health(&db).2 == 1);

    // Mark it withdrawn — the matcher must now ignore it.
    db.write(|c| {
        c.execute(
            "UPDATE advisories SET withdrawn = '2026-02-01T00:00:00Z' WHERE id = 'GHSA-temp'",
            [],
        )
        .map_err(Into::into)
        .map(|_| ())
    })
    .unwrap();

    rescan(&db, &rules, fx.root());
    let (score, _band, findings) = health(&db);
    assert_eq!(findings, 0, "withdrawn advisory excluded from matching");
    assert_eq!(score, Some(100));
}
