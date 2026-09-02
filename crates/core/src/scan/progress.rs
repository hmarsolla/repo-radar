//! The `ScanReporter` seam (DESIGN §6.4). This is what keeps `core` free of
//! Tauri: `src-tauri` implements it by emitting events, tests implement it
//! by pushing to a `Mutex<Vec<_>>`.

use crate::model::Warning;

/// Progress sink for a running scan. All methods take `&self` and must be
/// cheap and non-blocking — they are called from the writer thread.
pub trait ScanReporter: Send + Sync {
    /// Total repositories discovered, emitted once discovery completes.
    fn discovered(&self, total: usize);
    /// One repo finished analysis and persistence.
    fn repo_done(&self, summary: &RepoSummary);
    /// A recoverable problem occurred (FR-1.10).
    fn warning(&self, w: &Warning);
    /// The scan ended (complete or cancelled).
    fn finished(&self, summary: &ScanSummary);
}

/// The compact per-repo result carried in `scan:repo_done` and used to
/// populate the repo list incrementally (DESIGN §12.2).
#[derive(Debug, Clone)]
pub struct RepoSummary {
    pub repo_id: i64,
    pub name: String,
    pub path: String,
    pub warning_count: usize,
}

/// End-of-scan totals carried in `scan:complete`.
#[derive(Debug, Clone, Default)]
pub struct ScanSummary {
    pub scan_id: i64,
    pub repos_scanned: usize,
    pub warnings: usize,
    pub cancelled: bool,
}

/// A `ScanReporter` that records every call. Used by tests to assert, for
/// example, that `repo_done` fires for each repo *before* `finished`
/// (DESIGN §16.5).
#[derive(Default)]
pub struct RecordingReporter {
    pub events: std::sync::Mutex<Vec<ReporterEvent>>,
}

#[derive(Debug, Clone)]
pub enum ReporterEvent {
    Discovered(usize),
    RepoDone(RepoSummary),
    Warning(Warning),
    Finished(ScanSummary),
}

impl ScanReporter for RecordingReporter {
    fn discovered(&self, total: usize) {
        self.events
            .lock()
            .unwrap()
            .push(ReporterEvent::Discovered(total));
    }
    fn repo_done(&self, summary: &RepoSummary) {
        self.events
            .lock()
            .unwrap()
            .push(ReporterEvent::RepoDone(summary.clone()));
    }
    fn warning(&self, w: &Warning) {
        self.events
            .lock()
            .unwrap()
            .push(ReporterEvent::Warning(w.clone()));
    }
    fn finished(&self, summary: &ScanSummary) {
        self.events
            .lock()
            .unwrap()
            .push(ReporterEvent::Finished(summary.clone()));
    }
}
