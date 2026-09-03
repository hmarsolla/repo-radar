//! M1-2 (submodules), M1-6 (cancellation), M1-8 (warnings end to end),
//! M1-12 (incremental fingerprint), M1-13 (read-only invariant).

mod support;

use std::collections::BTreeMap;

use repo_radar_core::db::{repos as repo_db, scans, Db};
use repo_radar_core::model::{RepoIdentity, WarningKind};
use repo_radar_core::rules::RulePacks;
use repo_radar_core::scan::pipeline::{begin_scan, run_scan, ScanContext, ScanRoot};
#[cfg(unix)]
use repo_radar_core::scan::progress::ReporterEvent;
use repo_radar_core::scan::progress::{RecordingReporter, ScanReporter, ScanSummary};
use repo_radar_core::scan::submodule::attach_submodules;
use repo_radar_core::scan::CancelToken;
use repo_radar_core::Paths;
use support::{norm, GitFixture};

fn rules() -> RulePacks {
    let tmp = tempfile::tempdir().unwrap();
    RulePacks::load(&Paths::under(tmp.path())).unwrap()
}

fn register_root(db: &Db, fx: &GitFixture) -> Vec<ScanRoot> {
    db.write(|c| repo_db::add_scan_root(c, &fx.root().to_string_lossy()).map(|_| ()))
        .unwrap();
    vec![ScanRoot {
        id: 1,
        path: fx.root().to_path_buf(),
    }]
}

/// begin + run in one call, for tests.
fn scan(
    ctx: &ScanContext<'_>,
    roots: &[ScanRoot],
    cancel: &CancelToken,
    reporter: &dyn ScanReporter,
) -> ScanSummary {
    let id = begin_scan(ctx.db).unwrap();
    run_scan(ctx, id, roots, cancel, reporter).unwrap()
}

// ---------------------------------------------------------------------------
// M1-2 — submodules
// ---------------------------------------------------------------------------

#[test]
fn submodule_is_a_child_not_a_top_level_row() {
    let fx = GitFixture::new();
    let parent = fx.init_repo("parent");
    fx.init_repo("libdep");
    fx.add_submodule("parent", "libdep", "vendor/libdep");

    // Unit: attach_submodules reads .gitmodules and yields a child.
    let parent_id = RepoIdentity {
        path: norm(&parent),
        name: "parent".into(),
        is_bare: false,
        parent_path: None,
    };
    let subs = attach_submodules(&parent_id);
    assert_eq!(subs.len(), 1);
    assert_eq!(
        subs[0].parent_path.as_deref(),
        Some(parent_id.path.as_str())
    );
    assert!(subs[0].path.ends_with("vendor/libdep"));

    // End to end: one top-level row, the submodule not in the repo list.
    let db = Db::open_in_memory().unwrap();
    let rp = rules();
    let ctx = ScanContext::new(&db, &rp);
    let roots = register_root(&db, &fx);
    scan(
        &ctx,
        &roots,
        &CancelToken::new(),
        &RecordingReporter::default(),
    );

    let listed = db
        .read()
        .map(|c| repo_db::list_top_level(&c))
        .unwrap()
        .unwrap();
    let names: Vec<_> = listed.iter().map(|r| r.name.as_str()).collect();
    assert!(names.contains(&"parent"));
    assert!(
        names.contains(&"libdep"),
        "the standalone libdep repo is still top-level"
    );
    // The submodule copy under parent/vendor is NOT a top-level row.
    assert!(
        !listed
            .iter()
            .any(|r| r.path.contains("parent/vendor/libdep")),
        "submodule leaked into the repo list: {listed:?}"
    );
    // ...but it exists as a child row.
    let child_count: i64 = db
        .read()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM repos WHERE parent_repo_id IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(child_count, 1);
}

// ---------------------------------------------------------------------------
// M1-6 — cancellation
// ---------------------------------------------------------------------------

#[test]
fn cancelling_mid_scan_persists_partial_and_marks_cancelled() {
    let fx = GitFixture::new();
    for i in 0..12 {
        fx.init_repo(&format!("r{i:02}"));
    }
    let db = Db::open_in_memory().unwrap();
    let rp = rules();
    let ctx = ScanContext::new(&db, &rp);
    let roots = register_root(&db, &fx);

    // Cancel from another thread shortly after the scan starts.
    let cancel = CancelToken::new();
    let c2 = cancel.clone();
    let stopper = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(5));
        c2.cancel();
    });

    let summary = scan(&ctx, &roots, &cancel, &RecordingReporter::default());
    stopper.join().unwrap();

    assert!(summary.cancelled);
    // Whatever was persisted stays queryable.
    let listed = db
        .read()
        .map(|c| repo_db::list_top_level(&c))
        .unwrap()
        .unwrap();
    assert_eq!(listed.len(), summary.repos_scanned);
    // The scans row reads `cancelled`.
    let status = db
        .read()
        .map(|c| scans::latest_status(&c))
        .unwrap()
        .unwrap();
    assert_eq!(status, Some(scans::ScanStatus::Cancelled));
}

// ---------------------------------------------------------------------------
// M1-8 — warnings end to end
// ---------------------------------------------------------------------------

#[test]
fn a_panicking_analysis_becomes_a_warning_not_a_crash() {
    use repo_radar_core::scan::pipeline::guard_analysis;
    let id = RepoIdentity {
        path: "/x/y".into(),
        name: "y".into(),
        is_bare: false,
        parent_path: None,
    };
    let out = guard_analysis(&id, || panic!("boom in a parser"));
    assert_eq!(out.warnings.len(), 1);
    assert_eq!(out.warnings[0].kind, WarningKind::Panic);
    assert_eq!(out.repo.name, "y");
}

#[cfg(unix)]
#[test]
fn an_unreadable_directory_is_a_warning_and_the_scan_completes() {
    use std::os::unix::fs::PermissionsExt;

    let fx = GitFixture::new();
    fx.init_repo("readable");
    let locked = fx.abs("locked");
    std::fs::create_dir_all(&locked).unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();

    let db = Db::open_in_memory().unwrap();
    let rp = rules();
    let ctx = ScanContext::new(&db, &rp);
    let roots = register_root(&db, &fx);
    let reporter = RecordingReporter::default();

    let summary = scan(&ctx, &roots, &CancelToken::new(), &reporter);

    // Restore perms so tempdir cleanup works.
    let _ = std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755));

    assert!(!summary.cancelled);
    assert!(summary.warnings >= 1, "expected a permission warning");
    let saw_permission_warning =
        reporter.events.lock().unwrap().iter().any(
            |e| matches!(e, ReporterEvent::Warning(w) if w.kind == WarningKind::PermissionDenied),
        );
    assert!(saw_permission_warning);
    // The readable repo still made it in.
    let listed = db
        .read()
        .map(|c| repo_db::list_top_level(&c))
        .unwrap()
        .unwrap();
    assert_eq!(listed.len(), 1);
}

// ---------------------------------------------------------------------------
// M1-12 — incremental fingerprint
// ---------------------------------------------------------------------------

#[test]
fn unchanged_tree_skips_reanalysis_and_a_new_commit_re_triggers_one_repo() {
    let fx = GitFixture::new();
    fx.init_repo("a");
    fx.init_repo("b");

    let db = Db::open_in_memory().unwrap();
    let rp = rules();
    let ctx = ScanContext::new(&db, &rp);
    let roots = register_root(&db, &fx);

    scan(
        &ctx,
        &roots,
        &CancelToken::new(),
        &RecordingReporter::default(),
    );
    let fps_after_first: BTreeMap<String, Option<String>> = db
        .read()
        .map(|c| repo_db::all_fingerprints(&c))
        .unwrap()
        .unwrap()
        .into_iter()
        .collect();
    assert!(fps_after_first.values().all(|f| f.is_some()));

    // Second scan, nothing touched: every repo should be recognised as
    // unchanged (fingerprints identical).
    scan(
        &ctx,
        &roots,
        &CancelToken::new(),
        &RecordingReporter::default(),
    );
    let fps_after_second: BTreeMap<String, Option<String>> = db
        .read()
        .map(|c| repo_db::all_fingerprints(&c))
        .unwrap()
        .unwrap()
        .into_iter()
        .collect();
    assert_eq!(
        fps_after_first, fps_after_second,
        "fingerprints must be stable across an unchanged rescan"
    );

    // Touch exactly one repo with a new commit → its fingerprint changes,
    // the other stays put.
    fx.write("a/new.txt", "new\n");
    fx.stage_and_commit("a", "a moves");
    scan(
        &ctx,
        &roots,
        &CancelToken::new(),
        &RecordingReporter::default(),
    );
    let fps_after_third: BTreeMap<String, Option<String>> = db
        .read()
        .map(|c| repo_db::all_fingerprints(&c))
        .unwrap()
        .unwrap()
        .into_iter()
        .collect();

    let a_key = fps_after_third
        .keys()
        .find(|k| k.ends_with("/a"))
        .unwrap()
        .clone();
    let b_key = fps_after_third
        .keys()
        .find(|k| k.ends_with("/b"))
        .unwrap()
        .clone();
    assert_ne!(
        fps_after_second[&a_key], fps_after_third[&a_key],
        "the changed repo's fingerprint must move"
    );
    assert_eq!(
        fps_after_second[&b_key], fps_after_third[&b_key],
        "the untouched repo's fingerprint must not move"
    );
}

// ---------------------------------------------------------------------------
// M1-13 — read-only invariant (PRD Principle 4)
// ---------------------------------------------------------------------------

#[test]
fn a_full_scan_does_not_modify_the_source_tree() {
    let fx = GitFixture::new();
    fx.init_repo("alpha");
    fx.write("alpha/src/lib.rs", "pub fn f() -> u32 { 42 }\n");
    fx.stage_and_commit("alpha", "src");
    fx.init_repo("beta");
    fx.init_bare("gamma.git");

    let before = snapshot(fx.root());

    let db = Db::open_in_memory().unwrap();
    let rp = rules();
    let ctx = ScanContext::new(&db, &rp);
    let roots = register_root(&db, &fx);
    scan(
        &ctx,
        &roots,
        &CancelToken::new(),
        &RecordingReporter::default(),
    );

    let after = snapshot(fx.root());
    assert_eq!(before, after, "the scan changed files under the scan root");
}

/// `path -> (len, mtime_nanos, content_hash)` for every regular file under
/// `root`, so any write, truncate, or touch shows up as a difference.
fn snapshot(root: &std::path::Path) -> BTreeMap<String, (u64, i128, String)> {
    let mut map = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                stack.push(p);
            } else if meta.is_file() {
                let mtime = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_nanos() as i128)
                    .unwrap_or(0);
                let bytes = std::fs::read(&p).unwrap_or_default();
                let hash = blake3_hex(&bytes);
                map.insert(p.to_string_lossy().into_owned(), (meta.len(), mtime, hash));
            }
        }
    }
    map
}

fn blake3_hex(bytes: &[u8]) -> String {
    // A tiny FNV-1a is plenty to detect accidental writes and avoids adding
    // a hashing dependency to the test.
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}
