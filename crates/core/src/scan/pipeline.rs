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
}

impl<'a> ScanContext<'a> {
    pub fn new(db: &'a Db, rules: &'a RulePacks) -> Self {
        Self {
            db,
            rules,
            discovery: discovery::DiscoveryConfig::default(),
            git: git::GitConfig::default(),
            languages: languages::LanguageConfig::default(),
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

    // Parents (no parent_path) before children, each group path-sorted, so a
    // child's parent row always exists by the time it is written.
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
    let (tx, rx) = mpsc::channel::<AnalysisMsg>();
    let mut persisted = 0usize;

    std::thread::scope(|scope| -> CoreResult<()> {
        let writer = scope.spawn(|| writer_loop(ctx.db, rx, reporter, cancel));

        work.par_iter().for_each_with(tx.clone(), |tx, item| {
            if cancel.is_cancelled() {
                return;
            }
            let path = Path::new(&item.identity.path);

            // Incremental skip (M1-12): recompute the fingerprint from cheap
            // inputs and compare. Manifest hashes join the formula with the
            // parsers in M2-10; for now it is HEAD + rule-pack version.
            let head_sha = if item.identity.is_bare {
                None
            } else {
                cheap_head_sha(path)
            };
            let fingerprint = compute_fingerprint(head_sha.as_deref(), &[], &rule_pack_version);

            let already = prior
                .get(&item.identity.path)
                .map(|stored| stored.as_deref() == Some(fingerprint.as_str()))
                .unwrap_or(false);

            let outcome = if already {
                Outcome::Unchanged
            } else {
                Outcome::Analyzed(Box::new(analyze_repo(ctx, &item.identity, cancel)))
            };

            let _ = tx.send(AnalysisMsg {
                root_id: item.root_id,
                identity: item.identity.clone(),
                fingerprint,
                outcome,
            });
        });
        drop(tx); // close the channel so the writer loop ends

        let outcome = writer.join().map_err(|_| {
            crate::error::CoreError::Operation(crate::error::OperationError::Sync(
                "scan writer thread panicked".into(),
            ))
        })??;
        persisted = outcome.repos_persisted;
        all_warnings.extend(outcome.warnings);
        Ok(())
    })?;

    // ---- Stage 4: match + score -----------------------------------------
    // Runs for every repo on every scan, fingerprint or not (DESIGN §6.5).
    // Lands in M2-18.

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
        Err(_) => RepoAnalysis {
            repo: identity.clone(),
            git: None,
            languages: Vec::new(),
            dependencies: Vec::new(),
            technologies: Vec::new(),
            classification: unknown_classification(),
            warnings: vec![Warning::new(
                WarningScope::Repo(identity.path.clone()),
                WarningKind::Panic,
                "analysis panicked; repository skipped",
            )],
        },
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

    RepoAnalysis {
        repo: identity.clone(),
        git,
        languages: langs,
        dependencies: Vec::new(),                 // M2
        technologies: Vec::new(),                 // M3
        classification: unknown_classification(), // M3
        warnings,
    }
}

/// The single writer thread: drains analyses, batches them into
/// transactions, and reports each repo as it lands.
fn writer_loop(
    db: &Db,
    rx: mpsc::Receiver<AnalysisMsg>,
    reporter: &dyn ScanReporter,
    _cancel: &CancelToken,
) -> CoreResult<WriterOutcome> {
    const BATCH: usize = 16;

    let mut persisted = 0usize;
    let mut warnings: Vec<Warning> = Vec::new();
    let mut batch: Vec<AnalysisMsg> = Vec::with_capacity(BATCH);

    loop {
        match rx.recv() {
            Ok(msg) => {
                batch.push(msg);
                if batch.len() >= BATCH {
                    flush(db, &mut batch, reporter, &mut persisted, &mut warnings)?;
                }
            }
            Err(_) => {
                // Channel closed — analysis is done.
                flush(db, &mut batch, reporter, &mut persisted, &mut warnings)?;
                break;
            }
        }
    }

    Ok(WriterOutcome {
        repos_persisted: persisted,
        warnings,
    })
}

fn flush(
    db: &Db,
    batch: &mut Vec<AnalysisMsg>,
    reporter: &dyn ScanReporter,
    persisted: &mut usize,
    warnings: &mut Vec<Warning>,
) -> CoreResult<()> {
    if batch.is_empty() {
        return Ok(());
    }
    // Collect the summaries to report *after* the transaction commits, so a
    // subscriber never sees a repo that isn't yet queryable.
    let mut done: Vec<RepoSummary> = Vec::with_capacity(batch.len());

    db.write(|conn| {
        let tx = conn.transaction()?;
        for msg in batch.iter() {
            let is_child = msg.identity.parent_path.is_some();
            let parent_repo_id = match &msg.identity.parent_path {
                Some(p) => repo_db::repo_id_by_path(&tx, p)?,
                None => None,
            };

            let (repo_id, warning_count) = match &msg.outcome {
                Outcome::Analyzed(analysis) => {
                    let id = repo_db::upsert_repo(
                        &tx,
                        msg.root_id,
                        &analysis.repo,
                        analysis.git.as_ref(),
                        parent_repo_id,
                        Some(&msg.fingerprint),
                    )?;
                    repo_db::replace_languages(&tx, id, &analysis.languages)?;
                    (id, analysis.warnings.len())
                }
                Outcome::Unchanged => {
                    // Row already exists; just record that it was seen.
                    let id = repo_db::repo_id_by_path(&tx, &msg.identity.path)?
                        .expect("Unchanged implies an existing row");
                    repo_db::touch_scanned(&tx, id)?;
                    (id, 0)
                }
            };

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

    for msg in batch.drain(..) {
        if let Outcome::Analyzed(analysis) = msg.outcome {
            warnings.extend(analysis.warnings);
        }
    }
    for summary in &done {
        reporter.repo_done(summary);
        *persisted += 1;
    }
    Ok(())
}
