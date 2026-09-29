//! Scan pipeline orchestration (DESIGN §6.1, §6.2, §6.4).
//!
//! ```text
//! discover ─▶ analyze (rayon par_iter, one repo = one unit)
//!                 │  git metadata · language stats · [deps · tech · category — M2/M3]
//!                 ▼  RepoAnalysis, streamed over an mpsc channel as each completes
//!            persist (single writer thread, batched transactions)
//!                 ▼
//!            [match + score — stage 4, M2-18]
//!                 ▼
//!            emit  (repo_done per repo, then finished)
//! ```
//!
//! The single-writer design (DESIGN §1) means analysis threads produce
//! *values*, never database writes — that sidesteps SQLite write contention
//! instead of managing it. Each `RepoAnalysis` is handed to the writer the
//! moment it is ready, so the UI sees repos appear progressively (Journey A).

use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::sync::mpsc;

use rayon::prelude::*;

use crate::db::Db;
use crate::db::{repos as repo_db, scans};
use crate::error::CoreResult;
use crate::model::{
    Category, CategoryScores, Classification, ConfidenceLevel, RepoAnalysis, RepoIdentity, Warning,
    WarningKind, WarningScope,
};
use crate::rules::RulePacks;
use crate::scan::progress::{RepoSummary, ScanReporter, ScanSummary};
use crate::scan::{discovery, git, languages, submodule, CancelToken};

/// Everything a scan needs that outlives it: database handles, rule packs,
/// and the per-stage configs (prune list, git timeout).
pub struct ScanContext<'a> {
    pub db: &'a Db,
    pub rules: &'a RulePacks,
    pub discovery: discovery::DiscoveryConfig,
    pub git: git::GitConfig,
    pub languages: languages::LanguageConfig,
    pub parsers: crate::parsers::ParserRegistry,
}

impl<'a> ScanContext<'a> {
    pub fn new(db: &'a Db, rules: &'a RulePacks) -> Self {
        Self {
            db,
            rules,
            discovery: discovery::DiscoveryConfig::default(),
            git: git::GitConfig::default(),
            languages: languages::LanguageConfig::default(),
            parsers: crate::parsers::ParserRegistry::builtin(),
        }
    }

    fn rule_pack_version(&self) -> &str {
        &self.rules.version
    }
}

// ---------------------------------------------------------------------------
// Incremental re-scan fingerprint (DESIGN §6.5, M1-12)
// ---------------------------------------------------------------------------

/// `blake3( head_sha ‖ Σ sorted(manifest_path ‖ content_hash) ‖ rule_pack_version )`.
///
/// When this is unchanged from the stored value, stages 2–3 (git, languages,
/// parsing, classification) are skipped and the existing row is reused.
/// **Stage 4 (match + score) always re-runs regardless** — the advisory
/// database moves independently of the code, so a repo nobody touched can
/// still become unhealthy (enforced by test in M2-18). Fingerprinting is an
/// optimisation for *parsing*, never for *matching*.
pub fn compute_fingerprint(
    head_sha: Option<&str>,
    manifest_hashes: &[(String, String)],
    rule_pack_version: &str,
) -> String {
    let mut sorted: Vec<&(String, String)> = manifest_hashes.iter().collect();
    sorted.sort();

    let mut hasher = blake3::Hasher::new();
    hasher.update(head_sha.unwrap_or("<no-head>").as_bytes());
    hasher.update(b"\x1f");
    for (path, hash) in sorted {
        hasher.update(path.as_bytes());
        hasher.update(b"\x1e");
        hasher.update(hash.as_bytes());
        hasher.update(b"\x1e");
    }
    hasher.update(b"\x1f");
    hasher.update(rule_pack_version.as_bytes());
    hasher.finalize().to_hex().to_string()
}

/// Fast HEAD read for the fingerprint — just the commit id, no history walk
/// or status. Returns `None` for a bare or unborn repo.
fn cheap_head_sha(path: &Path) -> Option<String> {
    let repo = git2::Repository::open(path).ok()?;
    let head = repo.head().ok()?;
    let commit = head.peel_to_commit().ok()?;
    let sha = commit.id().to_string();
    Some(sha)
}

/// Depth of the analyze→persist queue. Several times the writer's batch size
/// so the writer always has a full batch ready, small enough that the
/// in-flight backlog stays a constant rather than growing with repo count.
const ANALYSIS_QUEUE_BOUND: usize = 64;

/// One scan root as the pipeline needs it.
#[derive(Debug, Clone)]
pub struct ScanRoot {
    pub id: i64,
    pub path: std::path::PathBuf,
}

impl From<repo_db::ScanRoot> for ScanRoot {
    fn from(r: repo_db::ScanRoot) -> Self {
        Self {
            id: r.id,
            path: std::path::PathBuf::from(r.path),
        }
    }
}

/// Insert the `running` scan row and return its id. The GUI calls this
/// synchronously so `scan_start` can return the id immediately, then hands
/// the id to [`run_scan`] on a background thread (DESIGN §12.1).
pub fn begin_scan(db: &Db) -> CoreResult<i64> {
    db.write(|c| scans::begin(c))
}

/// Run a full scan under a pre-created `scan_id`. Blocking and CPU/IO-bound
/// by design — callers put it on a dedicated thread (`spawn_blocking` from
/// the Tauri side).
pub fn run_scan(
    ctx: &ScanContext<'_>,
    scan_id: i64,
    roots: &[ScanRoot],
    cancel: &CancelToken,
    reporter: &dyn ScanReporter,
) -> CoreResult<ScanSummary> {
    let mut all_warnings: Vec<Warning> = Vec::new();

    // ---- Stage 1: discover -------------------------------------------------
    let mut work: Vec<WorkItem> = Vec::new();
    for root in roots {
        if cancel.is_cancelled() {
            break;
        }
        let found = discovery::discover(&root.path, &ctx.discovery, cancel);
        all_warnings.extend(found.warnings);
        for identity in found.repos {
            // Attach submodules as children of this repo (FR-1.5).
            for child in submodule::attach_submodules(&identity) {
                work.push(WorkItem {
                    root_id: root.id,
                    identity: child,
                });
            }
            work.push(WorkItem {
                root_id: root.id,
                identity,
            });
        }
    }

    // Path-sorted (within parent/child groups) so `dedup_by` below collapses
    // duplicate paths. Parent-before-child ordering here does NOT by itself
    // guarantee a child is written after its parent — analysis runs in
    // parallel and messages reach the writer in completion order, not this
    // Vec's order. The writer (`writer_loop`) enforces that guarantee by
    // deferring every child write until all non-child writes are flushed.
    work.sort_by(|a, b| {
        a.identity
            .parent_path
            .is_some()
            .cmp(&b.identity.parent_path.is_some())
            .then_with(|| a.identity.path.cmp(&b.identity.path))
    });
    work.dedup_by(|a, b| a.identity.path == b.identity.path);

    reporter.discovered(work.len());

    // Prior fingerprints, loaded once so the parallel stage does not contend
    // on the read pool (DESIGN §6.5).
    let prior: HashMap<String, Option<String>> = ctx
        .db
        .read()
        .map(|c| repo_db::all_fingerprints(&c))??
        .into_iter()
        .collect();
    let rule_pack_version = ctx.rule_pack_version().to_string();

    // ---- Stages 2 + 3: analyze (parallel) → persist (single writer) -------
    //
    // A *bounded* channel is load-bearing, not a detail. Analysis is
    // CPU-parallel across every core; persistence is one serialized SQLite
    // writer, so on any sizeable tree analysis outruns the writer
    // permanently. With the unbounded channel this used to be, the backlog
    // grew without limit and each queued `RepoAnalysis` holds that repo's
    // whole dependency set and language breakdown — a few thousand
    // dependency-heavy repos is enough to exhaust memory and have the OS kill
    // the process, with nothing in the log to say why. `sync_channel` makes a
    // full queue block the producing rayon worker instead, which costs
    // nothing (that worker has no useful work the writer isn't already
    // behind on) and caps the backlog at a constant.
    let (tx, rx) = mpsc::sync_channel::<AnalysisMsg>(ANALYSIS_QUEUE_BOUND);
    let mut persisted = 0usize;
    let mut stage4_targets: Vec<(i64, String)> = Vec::new();

    std::thread::scope(|scope| -> CoreResult<()> {
        let writer = scope.spawn(|| writer_loop(ctx.db, rx, reporter, cancel));

        work.par_iter().for_each_with(tx.clone(), |tx, item| {
            if cancel.is_cancelled() {
                return;
            }

            // The whole per-repo body runs under one panic boundary. The
            // fingerprint probe is *not* safe to leave outside it:
            // `manifest_hashes` reads and parses the repo's own lockfiles, so
            // it is exactly as panic-prone as full analysis. A panic escaping
            // this closure aborted the rayon iteration and dropped the
            // repository with no message and no warning.
            let (fingerprint, disk, outcome) = match catch_unwind(AssertUnwindSafe(|| {
                let path = Path::new(&item.identity.path);

                // Disk accounting runs for every repository on every scan,
                // *before* the fingerprint branch below decides whether to
                // re-analyse: `npm install` and `cargo build` change a repo's
                // footprint without touching anything the fingerprint covers.
                //
                // Submodule children are skipped: they live inside the parent's
                // working tree, so the parent's walk has already counted those
                // bytes and measuring the child again would double-count them
                // in every total the Cleanup view adds up.
                let disk = if item.identity.parent_path.is_some() {
                    crate::scan::disk::DiskUsage::default()
                } else {
                    crate::scan::disk::measure(path, cancel)
                };

                // Incremental skip (M1-12): recompute the fingerprint from
                // cheap inputs and compare. Manifest hashes join the formula
                // with the parts: HEAD sha + every manifest file's content
                // hash + the rule-pack version (§6.5).
                let head_sha = if item.identity.is_bare {
                    None
                } else {
                    cheap_head_sha(path)
                };
                let manifest_hashes = if item.identity.is_bare {
                    Vec::new()
                } else {
                    crate::scan::manifests::manifest_hashes(path, &ctx.parsers, &ctx.discovery)
                };
                let fingerprint =
                    compute_fingerprint(head_sha.as_deref(), &manifest_hashes, &rule_pack_version);

                let already = prior
                    .get(&item.identity.path)
                    .map(|stored| stored.as_deref() == Some(fingerprint.as_str()))
                    .unwrap_or(false);

                let outcome = if already {
                    Outcome::Unchanged
                } else {
                    Outcome::Analyzed(Box::new(analyze_repo(ctx, &item.identity, cancel)))
                };
                (fingerprint, disk, outcome)
            })) {
                Ok(triple) => triple,
                // An empty fingerprint never matches a stored one, so the
                // repository is retried in full on the next scan rather than
                // being cached as "unchanged" in its broken state.
                Err(_) => (
                    String::new(),
                    crate::scan::disk::DiskUsage::default(),
                    Outcome::Analyzed(Box::new(panicked_analysis(&item.identity))),
                ),
            };

            if tx
                .send(AnalysisMsg {
                    root_id: item.root_id,
                    identity: item.identity.clone(),
                    fingerprint,
                    disk,
                    outcome,
                })
                .is_err()
            {
                // The writer thread is gone (it returned an error or
                // panicked). Nothing left to do but stop quietly; the writer's
                // own failure is what gets reported.
                tracing::debug!(
                    path = %item.identity.path,
                    "scan writer channel closed; dropping analysis"
                );
            }
        });
        drop(tx); // close the channel so the writer loop ends

        let outcome = writer.join().map_err(|_| {
            crate::error::CoreError::Operation(crate::error::OperationError::Sync(
                "scan writer thread panicked".into(),
            ))
        })??;
        persisted = outcome.repos_persisted;
        all_warnings.extend(outcome.warnings);
        stage4_targets = outcome.touched;
        Ok(())
    })?;

    // ---- Stage 4: match + score (M2-18) --------------------------------
    // Runs for EVERY repo touched this scan, fingerprint-unchanged ones
    // included (DESIGN §6.5) — the advisory database moves independently of
    // the code. This is Journey B.
    let weights = crate::score::Weights::default();
    for (repo_id, path) in &stage4_targets {
        if cancel.is_cancelled() {
            break;
        }
        match ctx
            .db
            .write(|c| crate::db::findings::match_score_and_persist(c, *repo_id, path, &weights))
        {
            Ok(res) => all_warnings.extend(res.warnings),
            Err(e) => {
                all_warnings.push(Warning::new(
                    WarningScope::Repo(path.clone()),
                    WarningKind::Other,
                    format!("scoring failed: {e}"),
                ));
            }
        }
    }

    // ---- Finalise -------------------------------------------------------
    let cancelled = cancel.is_cancelled();
    let status = if cancelled {
        scans::ScanStatus::Cancelled
    } else {
        scans::ScanStatus::Complete
    };
    ctx.db
        .write(|c| scans::finish(c, scan_id, status, persisted, &all_warnings))?;

    let summary = ScanSummary {
        scan_id,
        repos_scanned: persisted,
        warnings: all_warnings.len(),
        cancelled,
    };
    for w in &all_warnings {
        reporter.warning(w);
    }
    reporter.finished(&summary);
    Ok(summary)
}

struct WorkItem {
    root_id: i64,
    identity: RepoIdentity,
}

struct AnalysisMsg {
    root_id: i64,
    identity: RepoIdentity,
    fingerprint: String,
    /// Measured on every scan, independent of [`Outcome`] — see
    /// [`crate::db::repos::replace_disk_usage`] for why it is not cached
    /// against the fingerprint.
    disk: crate::scan::disk::DiskUsage,
    outcome: Outcome,
}

enum Outcome {
    /// Fresh analysis — upsert the row and its children, store the new
    /// fingerprint. Boxed because `RepoAnalysis` is large and the
    /// `Unchanged` arm carries nothing.
    Analyzed(Box<RepoAnalysis>),
    /// Fingerprint matched the stored value — reuse the existing row, just
    /// bump `last_scanned_at`.
    Unchanged,
}

struct WriterOutcome {
    repos_persisted: usize,
    warnings: Vec<Warning>,
    /// `(repo_id, path)` for every top-level repo the writer touched — the
    /// input to stage 4, which re-runs for all of them regardless of
    /// fingerprint (DESIGN §6.5).
    touched: Vec<(i64, String)>,
}

/// Placeholder classification until the categorizer lands (M3-3).
fn unknown_classification() -> Classification {
    Classification {
        category: Category::Unknown,
        confidence: ConfidenceLevel::Low,
        scores: CategoryScores::default(),
        manual: None,
    }
}

/// Analyse one repository. Every recoverable failure becomes a [`Warning`]
/// on the returned [`RepoAnalysis`]; a panic inside analysis is caught at
/// this boundary (DESIGN §15) and turned into a `Panic` warning so one bad
/// repo cannot abort the scan (M1-8).
fn analyze_repo(
    ctx: &ScanContext<'_>,
    identity: &RepoIdentity,
    cancel: &CancelToken,
) -> RepoAnalysis {
    guard_analysis(identity, || analyze_repo_inner(ctx, identity, cancel))
}

/// Run `f`, converting any panic into a `Panic` [`Warning`] on an otherwise
/// empty [`RepoAnalysis`] for `identity`. One malformed lockfile triggering
/// a parser panic must not take down a 50-repo scan (DESIGN §15). Public so
/// the panic-to-warning boundary can be exercised directly (M1-8).
pub fn guard_analysis(identity: &RepoIdentity, f: impl FnOnce() -> RepoAnalysis) -> RepoAnalysis {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(analysis) => analysis,
        Err(_) => panicked_analysis(identity),
    }
}

/// An otherwise-empty [`RepoAnalysis`] carrying a single `Panic` warning, so
/// a repository whose analysis blew up still reaches the writer and still
/// appears in the list — flagged — rather than vanishing.
fn panicked_analysis(identity: &RepoIdentity) -> RepoAnalysis {
    RepoAnalysis {
        repo: identity.clone(),
        git: None,
        languages: Vec::new(),
        dependencies: Vec::new(),
        manifests: Vec::new(),
        technologies: Vec::new(),
        classification: unknown_classification(),
        is_monorepo: false,
        warnings: vec![Warning::new(
            WarningScope::Repo(identity.path.clone()),
            WarningKind::Panic,
            "analysis panicked; repository skipped",
        )],
    }
}

fn analyze_repo_inner(
    ctx: &ScanContext<'_>,
    identity: &RepoIdentity,
    _cancel: &CancelToken,
) -> RepoAnalysis {
    let mut warnings = Vec::new();
    let path = Path::new(&identity.path);

    let git = if identity.is_bare {
        None // FR-1.6 — no working tree; git columns come from discovery only
    } else {
        match git::extract(path, &ctx.git) {
            Ok(info) => Some(info),
            Err(git::GitError::Timeout(d)) => {
                warnings.push(Warning::new(
                    WarningScope::Repo(identity.path.clone()),
                    WarningKind::GitTimeout,
                    format!("git metadata timed out after {d:?}"),
                ));
                None
            }
            Err(e) => {
                warnings.push(Warning::new(
                    WarningScope::Repo(identity.path.clone()),
                    WarningKind::GitError,
                    format!("git metadata unavailable: {e}"),
                ));
                None
            }
        }
    };

    let langs = if identity.is_bare {
        Vec::new()
    } else {
        languages::analyze(path, &ctx.languages)
    };

    // Dependency inventory (M2-10). Bare repos have no working tree.
    let manifests = if identity.is_bare {
        crate::scan::manifests::ManifestScan::default()
    } else {
        crate::scan::manifests::scan_manifests(path, &ctx.parsers, &ctx.discovery)
    };
    warnings.extend(manifests.warnings);

    // Technology detection (M3-2) and categorization (M3-3) run off the same
    // signal bundle: resolved deps, the repo-relative file list, languages,
    // and parsed manifests. A bare repo has no working tree, so both come
    // back empty rather than guessing from a name.
    let signals = crate::rules::signals::RepoSignals {
        deps: &manifests.dependencies,
        files: &manifests.files,
        languages: &langs,
        manifests: &manifests.manifests,
    };
    let technologies = crate::rules::technologies::detect(&ctx.rules.technologies, &signals);
    let classification =
        crate::rules::categories::classify(&ctx.rules.settings, &ctx.rules.categories, &signals);

    RepoAnalysis {
        repo: identity.clone(),
        git,
        languages: langs,
        dependencies: manifests.dependencies,
        manifests: manifests.manifests,
        technologies,
        classification,
        is_monorepo: manifests.monorepo,
        warnings,
    }
}

/// The single writer thread: drains analyses, batches them into
/// transactions, and reports each repo as it lands.
///
/// Submodule children are held back from the streaming `batch` and written
/// only after every non-child message has been flushed (M1-2/M1-9). Analysis
/// runs in parallel (`work.par_iter()`), so messages reach this loop in
/// completion order, not discovery order — a child can easily finish (and
/// arrive) before its own parent. Looking up `parent_repo_id` for a child
/// that hasn't been written yet silently orphans it (`parent_repo_id` =
/// NULL), which is exactly the race this deferral avoids: by the time
/// children are flushed, every parent's `WorkItem` has already been through
/// this same channel and been committed.
fn writer_loop(
    db: &Db,
    rx: mpsc::Receiver<AnalysisMsg>,
    reporter: &dyn ScanReporter,
    _cancel: &CancelToken,
) -> CoreResult<WriterOutcome> {
    const BATCH: usize = 16;

    let mut persisted = 0usize;
    let mut warnings: Vec<Warning> = Vec::new();
    let mut touched: Vec<(i64, String)> = Vec::new();
    let mut batch: Vec<AnalysisMsg> = Vec::with_capacity(BATCH);
    let mut children: Vec<AnalysisMsg> = Vec::new();

    loop {
        match rx.recv() {
            Ok(msg) => {
                if msg.identity.parent_path.is_some() {
                    children.push(msg);
                    continue;
                }
                batch.push(msg);
                if batch.len() >= BATCH {
                    flush(
                        db,
                        &mut batch,
                        reporter,
                        &mut persisted,
                        &mut warnings,
                        &mut touched,
                    )?;
                }
            }
            Err(_) => {
                // Channel closed — analysis is done. Flush any remaining
                // parents first, then every deferred child, in batches.
                flush(
                    db,
                    &mut batch,
                    reporter,
                    &mut persisted,
                    &mut warnings,
                    &mut touched,
                )?;
                while !children.is_empty() {
                    let take = children.len().min(BATCH);
                    let mut chunk: Vec<AnalysisMsg> = children.drain(..take).collect();
                    flush(
                        db,
                        &mut chunk,
                        reporter,
                        &mut persisted,
                        &mut warnings,
                        &mut touched,
                    )?;
                }
                break;
            }
        }
    }

    Ok(WriterOutcome {
        repos_persisted: persisted,
        warnings,
        touched,
    })
}

#[allow(clippy::too_many_arguments)]
fn flush(
    db: &Db,
    batch: &mut Vec<AnalysisMsg>,
    reporter: &dyn ScanReporter,
    persisted: &mut usize,
    warnings: &mut Vec<Warning>,
    touched: &mut Vec<(i64, String)>,
) -> CoreResult<()> {
    if batch.is_empty() {
        return Ok(());
    }
    // Collect the summaries to report *after* the transaction commits, so a
    // subscriber never sees a repo that isn't yet queryable.
    let mut done: Vec<RepoSummary> = Vec::with_capacity(batch.len());
    // Paths whose row disappeared between the fingerprint load and this
    // write (see the `Outcome::Unchanged` arm below).
    let mut vanished: Vec<String> = Vec::new();

    db.write(|conn| {
        let tx = conn.transaction()?;
        for msg in batch.iter() {
            let is_child = msg.identity.parent_path.is_some();
            let parent_repo_id = match &msg.identity.parent_path {
                Some(p) => repo_db::repo_id_by_path(&tx, p)?,
                None => None,
            };

            let resolved = match &msg.outcome {
                Outcome::Analyzed(analysis) => {
                    // A submodule child's dependencies are attributed to the
                    // parent's `repo_id` (FR-1.5).
                    let owner_id = parent_repo_id;
                    let id = repo_db::upsert_repo(
                        &tx,
                        msg.root_id,
                        &analysis.repo,
                        analysis.git.as_ref(),
                        parent_repo_id,
                        Some(&msg.fingerprint),
                        analysis.is_monorepo,
                    )?;
                    repo_db::replace_languages(&tx, id, &analysis.languages)?;
                    repo_db::replace_manifests_and_deps(
                        &tx,
                        owner_id.unwrap_or(id),
                        &analysis.manifests,
                        &analysis.dependencies,
                    )?;
                    repo_db::replace_technologies(&tx, id, &analysis.technologies)?;
                    repo_db::set_classification(&tx, id, &analysis.classification)?;
                    Some((id, analysis.warnings.len()))
                }
                Outcome::Unchanged => {
                    // The row existed when `all_fingerprints` was loaded at
                    // the start of the scan, but it can be gone by now:
                    // removing a scan root cascades its repos away
                    // (`ON DELETE CASCADE`), and **Reset database** empties
                    // the table outright — both reachable from the UI while a
                    // scan is in flight. This used to be
                    // `.expect("Unchanged implies an existing row")`, which
                    // panicked *inside* `db.write`, poisoning the write mutex
                    // and taking every later write in the process down with
                    // it. Skip the row instead and report it.
                    match repo_db::repo_id_by_path(&tx, &msg.identity.path)? {
                        Some(id) => {
                            repo_db::touch_scanned(&tx, id)?;
                            Some((id, 0))
                        }
                        None => {
                            vanished.push(msg.identity.path.clone());
                            None
                        }
                    }
                }
            };
            let Some((repo_id, warning_count)) = resolved else {
                continue;
            };

            // Applies to both outcomes: an `Unchanged` repo still gets a fresh
            // footprint, which is the whole reason disk usage is measured
            // outside the fingerprint branch.
            repo_db::replace_disk_usage(&tx, repo_id, &msg.disk)?;

            // Submodule children roll into their parent — persisted, but
            // never announced as a list row (FR-1.5).
            if !is_child {
                done.push(RepoSummary {
                    repo_id,
                    name: msg.identity.name.clone(),
                    path: msg.identity.path.clone(),
                    warning_count,
                });
            }
        }
        tx.commit()?;
        Ok(())
    })?;

    for path in vanished {
        warnings.push(Warning::new(
            WarningScope::Repo(path),
            WarningKind::Other,
            "repository row disappeared during the scan (scan root removed, \
             or the database was reset); skipped",
        ));
    }
    for msg in batch.drain(..) {
        if let Outcome::Analyzed(analysis) = msg.outcome {
            warnings.extend(analysis.warnings);
        }
    }
    for summary in &done {
        reporter.repo_done(summary);
        touched.push((summary.repo_id, summary.path.clone()));
        *persisted += 1;
    }
    Ok(())
}
