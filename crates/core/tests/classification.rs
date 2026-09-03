//! M3-2 / M3-3 acceptance: a fixture tree with one repo per category scans,
//! and each repo's stored `category`, `category_scores`, and
//! `repo_technologies` reflect the rule engines — an ambiguous repo lands on
//! `Fullstack`, a signal-less repo stays `Unknown`.

mod support;

use repo_radar_core::db::{repos as repo_db, Db};
use repo_radar_core::rules::RulePacks;
use repo_radar_core::scan::pipeline::{begin_scan, run_scan, ScanContext, ScanRoot};
use repo_radar_core::scan::progress::RecordingReporter;
use repo_radar_core::scan::CancelToken;
use repo_radar_core::Paths;
use support::GitFixture;

fn category_of(db: &Db, name: &str) -> String {
    let detail = db
        .read()
        .map(|c| {
            let id: i64 = c
                .query_row("SELECT id FROM repos WHERE name = ?1", [name], |r| r.get(0))
                .unwrap();
            repo_db::get_repo_detail(&c, id)
        })
        .unwrap()
        .unwrap()
        .unwrap();
    detail.repo.category.unwrap_or_else(|| "<none>".into())
}

fn tech_names(db: &Db, name: &str) -> Vec<String> {
    db.read()
        .map(|c| {
            let mut stmt = c
                .prepare(
                    "SELECT t.tech FROM repo_technologies t
                       JOIN repos r ON r.id = t.repo_id
                      WHERE r.name = ?1 ORDER BY t.tech",
                )
                .unwrap();
            let v: Vec<String> = stmt
                .query_map([name], |r| r.get(0))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            v
        })
        .unwrap()
}

#[test]
fn fixture_repos_classify_by_category_and_record_technologies() {
    let fx = GitFixture::new();

    // Frontend: React + Vite.
    fx.init_repo("frontend");
    fx.write(
        "frontend/package.json",
        r#"{"name":"fe","dependencies":{"react":"18.2.0","vite":"5.0.0"}}"#,
    );
    fx.write("frontend/tsconfig.json", "{}");
    fx.stage_and_commit("frontend", "add manifest");

    // Backend: axum server, Rust.
    fx.init_repo("backend");
    fx.write(
        "backend/Cargo.toml",
        "[package]\nname=\"be\"\nversion=\"0.1.0\"\n\n[dependencies]\naxum=\"0.7\"\n",
    );
    fx.write("backend/src/main.rs", "fn main() {}\n");
    fx.stage_and_commit("backend", "add manifest");

    // Fullstack: React + Express in one package.
    fx.init_repo("fullstack");
    fx.write(
        "fullstack/package.json",
        r#"{"name":"fs","dependencies":{"react":"18.2.0","next":"14.0.0","express":"4.19.0"}}"#,
    );
    fx.stage_and_commit("fullstack", "add manifest");

    // Library: a crate with a manifest and no entrypoint.
    fx.init_repo("library");
    fx.write(
        "library/Cargo.toml",
        "[package]\nname=\"lib\"\nversion=\"0.1.0\"\n\n[dependencies]\n",
    );
    fx.write("library/src/lib.rs", "pub fn hello() {}\n");
    fx.stage_and_commit("library", "add manifest");

    // Data/ML: numpy + pandas.
    fx.init_repo("dataml");
    fx.write(
        "dataml/requirements.txt",
        "numpy==1.26.0\npandas==2.1.0\nscikit-learn==1.3.0\n",
    );
    fx.stage_and_commit("dataml", "add manifest");

    // Unknown: a git repo with nothing to go on.
    fx.init_repo("mystery");

    let db = Db::open_in_memory().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let rules = RulePacks::load(&Paths::under(tmp.path())).unwrap();
    let ctx = ScanContext::new(&db, &rules);
    let cancel = CancelToken::new();

    db.write(|c| repo_db::add_scan_root(c, &fx.root().to_string_lossy()).map(|_| ()))
        .unwrap();
    let roots = [ScanRoot {
        id: 1,
        path: fx.root().to_path_buf(),
    }];

    let id = begin_scan(&db).unwrap();
    run_scan(&ctx, id, &roots, &cancel, &RecordingReporter::default()).unwrap();

    assert_eq!(category_of(&db, "frontend"), "Frontend");
    assert_eq!(category_of(&db, "backend"), "Backend");
    assert_eq!(
        category_of(&db, "fullstack"),
        "Fullstack",
        "frontend + backend signals must not be an arbitrary pick"
    );
    assert_eq!(category_of(&db, "library"), "Library");
    assert_eq!(category_of(&db, "dataml"), "DataMl");
    assert_eq!(
        category_of(&db, "mystery"),
        "Unknown",
        "no signals must yield Unknown, not a guess"
    );

    // Technologies are recorded with evidence.
    let fe_tech = tech_names(&db, "frontend");
    assert!(fe_tech.contains(&"React".to_string()));
    assert!(fe_tech.contains(&"TypeScript".to_string()));
    assert!(tech_names(&db, "backend").contains(&"axum".to_string()));

    // The breakdown JSON is persisted for the explainability UI (FR-3.6).
    let scores: String = db
        .read()
        .map(|c| {
            c.query_row(
                "SELECT category_scores FROM repos WHERE name = 'frontend'",
                [],
                |r| r.get(0),
            )
            .unwrap()
        })
        .unwrap();
    assert!(scores.contains("react-frontend"), "fired rules serialized");
}
