//! M1-9: `list_repos(RepoFilter)` composes filters in SQL and returns
//! quickly; `get_repo_detail` returns the record + languages + submodules.

mod support;

use std::time::Instant;

use repo_radar_core::db::repos::{self as repo_db, RepoFilter, RepoSort};
use repo_radar_core::db::Db;
use repo_radar_core::rules::RulePacks;
use repo_radar_core::scan::pipeline::{begin_scan, run_scan, ScanContext, ScanRoot};
use repo_radar_core::scan::progress::RecordingReporter;
use repo_radar_core::scan::CancelToken;
use repo_radar_core::Paths;
use support::GitFixture;

fn scanned_db(fx: &GitFixture) -> Db {
    let db = Db::open_in_memory().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let rules = RulePacks::load(&Paths::under(tmp.path())).unwrap();
    let ctx = ScanContext::new(&db, &rules);
    db.write(|c| repo_db::add_scan_root(c, &fx.root().to_string_lossy()).map(|_| ()))
        .unwrap();
    let roots = [ScanRoot {
        id: 1,
        path: fx.root().to_path_buf(),
    }];
    let id = begin_scan(&db).unwrap();
    run_scan(
        &ctx,
        id,
        &roots,
        &CancelToken::new(),
        &RecordingReporter::default(),
    )
    .unwrap();
    db
}

#[test]
fn filters_and_sorting_compose_and_run_in_sql() {
    let fx = GitFixture::new();
    // Distinctive names so a `search` term can't accidentally match the
    // random tempdir path segment.
    fx.init_repo("qqz-apex");
    fx.write("qqz-apex/main.rs", "fn main() { let _ = 1; }\n");
    fx.stage_and_commit("qqz-apex", "rust");

    fx.init_repo("qqz-blaze");
    fx.write("qqz-blaze/app.py", "print('hi')\n");
    fx.stage_and_commit("qqz-blaze", "python");

    fx.init_repo("qqz-crux");
    fx.write("qqz-crux/lib.rs", "pub fn f() {}\n");
    fx.stage_and_commit("qqz-crux", "rust2");
    fx.write("qqz-crux/dirty.txt", "uncommitted\n"); // leaves crux dirty

    let db = scanned_db(&fx);
    let conn = db.read().unwrap();
    let names = |r: &[repo_radar_core::db::repos::RepoListItem]| {
        r.iter()
            .map(|x| x.name.trim_start_matches("qqz-").to_string())
            .collect::<Vec<_>>()
    };

    // Search narrows by name.
    let f = RepoFilter {
        search: Some("apex".into()),
        ..Default::default()
    };
    let r = repo_db::list_repos(&conn, &f).unwrap();
    assert_eq!(names(&r), vec!["apex"]);

    // Language filter.
    let f = RepoFilter {
        language: Some("Python".into()),
        ..Default::default()
    };
    let r = repo_db::list_repos(&conn, &f).unwrap();
    assert_eq!(names(&r), vec!["blaze"]);

    // Dirty-only.
    let f = RepoFilter {
        dirty_only: true,
        ..Default::default()
    };
    let r = repo_db::list_repos(&conn, &f).unwrap();
    assert_eq!(names(&r), vec!["crux"]);

    // Sort by name ascending.
    let f = RepoFilter {
        sort: RepoSort::Name,
        descending: false,
        ..Default::default()
    };
    let r = repo_db::list_repos(&conn, &f).unwrap();
    assert_eq!(names(&r), vec!["apex", "blaze", "crux"]);

    // Composed: search + sort. "qqz-" prefixes every fixture name, so it
    // matches all three; sorted by name descending.
    let f = RepoFilter {
        search: Some("qqz-".into()),
        sort: RepoSort::Name,
        descending: true,
        ..Default::default()
    };
    let r = repo_db::list_repos(&conn, &f).unwrap();
    assert_eq!(names(&r), vec!["crux", "blaze", "apex"]);
}

#[test]
fn list_repos_on_fifty_repos_is_well_under_100ms() {
    let fx = GitFixture::new();
    for i in 0..50 {
        fx.init_repo(&format!("r{i:02}"));
    }
    let db = scanned_db(&fx);
    let conn = db.read().unwrap();

    let start = Instant::now();
    let r = repo_db::list_repos(&conn, &RepoFilter::default()).unwrap();
    let elapsed = start.elapsed();

    assert_eq!(r.len(), 50);
    assert!(elapsed.as_millis() < 100, "list_repos took {elapsed:?}");
}

#[test]
fn get_repo_detail_returns_record_languages_and_submodules() {
    let fx = GitFixture::new();
    fx.init_repo("host");
    fx.write("host/main.rs", "fn main() {}\n");
    fx.stage_and_commit("host", "code");
    fx.init_repo("dep");
    fx.add_submodule("host", "dep", "third_party/dep");

    let db = scanned_db(&fx);
    let conn = db.read().unwrap();

    let host = repo_db::list_repos(&conn, &RepoFilter::default())
        .unwrap()
        .into_iter()
        .find(|r| r.name == "host")
        .unwrap();

    let detail = repo_db::get_repo_detail(&conn, host.id).unwrap().unwrap();
    assert_eq!(detail.repo.name, "host");
    assert!(detail.repo.head_sha.is_some());
    assert!(detail.languages.iter().any(|l| l.language == "Rust"));
    assert_eq!(detail.submodules.len(), 1);
    assert_eq!(detail.submodules[0].name, "dep");

    assert!(repo_db::get_repo_detail(&conn, 99999).unwrap().is_none());
}
