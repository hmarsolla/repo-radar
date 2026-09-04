//! M4-1 / M4-2 acceptance: every built-in template renders against a fully
//! populated context — repo metadata, dependencies, findings with advisory
//! freshness, and embedded file bodies — and produces a coherent prompt.
//!
//! This is the "runs through the same path as user templates" guarantee
//! (DESIGN §11.1): the test drives the exact public surface
//! (`list_templates` → `load_template_source` → `build_context` → `render`)
//! that the Tauri command layer uses.

use std::collections::HashMap;

use repo_radar_core::db::Db;
use repo_radar_core::model::Freshness;
use repo_radar_core::prompt::{
    build_context, estimate_tokens, list_templates, load_template_source, render, EmbeddedFile,
    ScopeContext, TreeEntry,
};
use repo_radar_core::Paths;

fn seed(db: &Db) {
    db.write(|c| {
        c.execute_batch(
            r#"
            INSERT INTO scan_roots (id, path, added_at) VALUES (1, '/r', '2026-01-01T00:00:00Z');
            INSERT INTO repos (id, root_id, path, name, category, category_scores,
                               health_score, health_band, branch, last_commit_at, last_commit_summary)
                VALUES
                (1, 1, '/r/api', 'api', 'Backend',
                 '{"totals":[["Backend",7.0]],"fired":[{"rule_id":"backend-express","signal":"dependency:express","category":"Backend","weight":7.0}]}',
                 64, 'fair', 'main', '2026-02-10T00:00:00Z', 'tidy up routes'),
                (2, 1, '/r/web', 'web', 'Frontend',
                 '{"totals":[["Frontend",5.0]],"fired":[{"rule_id":"frontend-react","signal":"dependency:react","category":"Frontend","weight":5.0}]}',
                 81, 'good', 'main', '2026-02-11T00:00:00Z', 'ship nav');
            INSERT INTO repo_languages (repo_id, language, code_lines, comment_lines, files, percentage)
                VALUES (1, 'TypeScript', 4200, 300, 60, 88.0),
                       (1, 'JavaScript', 570, 40, 9, 12.0),
                       (2, 'TypeScript', 3100, 210, 44, 95.0);
            INSERT INTO repo_technologies (repo_id, tech, kind, evidence)
                VALUES (1, 'Express', 'framework', '["dependency:express"]'),
                       (2, 'React', 'framework', '["dependency:react"]');
            INSERT INTO manifests (id, repo_id, path, ecosystem, kind, content_hash)
                VALUES (1, 1, 'package-lock.json', 'npm', 'lockfile', 'h1'),
                       (2, 2, 'package-lock.json', 'npm', 'lockfile', 'h2');
            INSERT INTO dependencies (id, repo_id, manifest_id, ecosystem, name, raw_name, version, confidence, scope, is_direct)
                VALUES (1, 1, 1, 'npm', 'express', 'express', '4.17.1', 'exact', 'runtime', 1),
                       (2, 1, 1, 'npm', 'lodash', 'lodash', '4.17.20', 'exact', 'runtime', 1),
                       (3, 1, 1, 'npm', 'ms', 'ms', '2.0.0', 'exact', 'runtime', 0),
                       (4, 2, 2, 'npm', 'react', 'react', '18.2.0', 'exact', 'runtime', 1),
                       (5, 2, 2, 'npm', 'lodash', 'lodash', '4.17.21', 'exact', 'runtime', 1);
            INSERT INTO advisories (id, kind, severity, summary, modified)
                VALUES ('GHSA-redos', 'vulnerability', 'high', 'ReDoS via crafted header', '2026-01-01T00:00:00Z'),
                       ('GHSA-proto', 'vulnerability', 'medium', 'Prototype pollution', '2026-01-01T00:00:00Z');
            INSERT INTO findings (id, repo_id, dependency_id, advisory_id, kind, confidence, fixed_version, deduction)
                VALUES (1, 1, 1, 'GHSA-redos', 'vulnerability', 'exact', '4.18.0', 9.0),
                       (2, 1, 2, 'GHSA-proto', 'vulnerability', 'range', NULL, 4.0);
            "#,
        )
        .map_err(Into::into)
    })
    .unwrap();
}

#[test]
fn lists_the_three_built_ins() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = Paths::under(tmp.path());
    paths.ensure_dirs().unwrap();

    let list = list_templates(&paths).unwrap();
    let ids: Vec<&str> = list.iter().map(|t| t.id.as_str()).collect();
    assert!(ids.contains(&"cross_repo_similarity"));
    assert!(ids.contains(&"perf_security_opportunities"));
    assert!(ids.contains(&"code_review"));
}

#[test]
fn cross_repo_similarity_renders_over_n_repos() {
    let db = Db::open_in_memory().unwrap();
    seed(&db);
    let tmp = tempfile::tempdir().unwrap();
    let paths = Paths::under(tmp.path());
    paths.ensure_dirs().unwrap();
    let conn = db.read().unwrap();

    let ctx = build_context(
        &conn,
        &[1, 2],
        ScopeContext::WholeRepo,
        vec![],
        &HashMap::new(),
        Freshness::Fresh,
    )
    .unwrap();

    let src = load_template_source(&paths, "cross_repo_similarity").unwrap();
    let out = render("cross_repo_similarity", &src, &ctx).unwrap();

    assert!(out.contains("REPOSITORY: api"));
    assert!(out.contains("REPOSITORY: web"));
    assert!(out.contains("express@4.17.1"));
    assert!(out.contains("SHARED DEPENDENCIES"));
    assert!(out.contains("synced within the last 7 days"));
    assert!(estimate_tokens(&out) > 50);
}

#[test]
fn perf_security_separates_confirmed_from_speculative() {
    let db = Db::open_in_memory().unwrap();
    seed(&db);
    let tmp = tempfile::tempdir().unwrap();
    let paths = Paths::under(tmp.path());
    paths.ensure_dirs().unwrap();
    let conn = db.read().unwrap();

    let files = vec![EmbeddedFile {
        path: "src/routes.ts".into(),
        language: Some("TypeScript".into()),
        content: "export const routes = () => {}\n".into(),
        bytes: 30,
    }];
    let mut trees = HashMap::new();
    trees.insert(
        1i64,
        vec![TreeEntry {
            path: "src".into(),
            is_dir: true,
            depth: 0,
        }],
    );

    let ctx = build_context(
        &conn,
        &[1],
        ScopeContext::Files {
            paths: vec!["src/routes.ts".into()],
        },
        files,
        &trees,
        Freshness::Stale,
    )
    .unwrap();

    let src = load_template_source(&paths, "perf_security_opportunities").unwrap();
    let out = render("perf_security_opportunities", &src, &ctx).unwrap();

    assert!(out.contains("REPOSITORY PROFILE"));
    assert!(out.contains("Confirmed issues"));
    assert!(out.contains("Speculative opportunities"));
    // express finding is exact → CONFIRMED; lodash finding is range → SPECULATIVE.
    assert!(out.contains("[CONFIRMED] VULNERABILITY GHSA-redos"));
    assert!(out.contains("[SPECULATIVE] VULNERABILITY GHSA-proto"));
    assert!(out.contains("src/routes.ts"));
    assert!(out.contains("export const routes"));
    assert!(out.contains("last synced more than 7 days ago"));
}

#[test]
fn code_review_adapts_to_scope() {
    let db = Db::open_in_memory().unwrap();
    seed(&db);
    let tmp = tempfile::tempdir().unwrap();
    let paths = Paths::under(tmp.path());
    paths.ensure_dirs().unwrap();
    let conn = db.read().unwrap();

    let diff_ctx = build_context(
        &conn,
        &[1],
        ScopeContext::Diff {
            description: "diff --git a/x b/x\n+added line".into(),
        },
        vec![],
        &HashMap::new(),
        Freshness::Never,
    )
    .unwrap();
    let src = load_template_source(&paths, "code_review").unwrap();
    let out = render("code_review", &src, &diff_ctx).unwrap();

    assert!(out.contains("Review the change itself"));
    assert!(out.contains("+added line"));
    assert!(out.contains("Blocking"));
    assert!(!out.contains("whole repository as it currently stands"));
}
