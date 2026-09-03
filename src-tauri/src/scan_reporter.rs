//! The `ScanReporter` seam wired to Tauri events (DESIGN §6.4, §12.2, M1-7).
//!
//! `core` reports progress through the [`ScanReporter`] trait; here that
//! becomes `scan:*` events on the app handle. Tests in `core` implement the
//! same trait with a `Vec`, which is what keeps `core` free of Tauri.

use std::sync::atomic::{AtomicU32, Ordering};

use tauri::AppHandle;
use tauri_specta::Event as _;

use repo_radar_core::model::Warning;
use repo_radar_core::scan::progress::{RepoSummary, ScanReporter, ScanSummary};

use crate::events::{ScanComplete, ScanProgress, ScanRepoDone, ScanWarning};

pub struct EventReporter {
    app: AppHandle,
    scan_id: i64,
    discovered: AtomicU32,
    completed: AtomicU32,
}

impl EventReporter {
    pub fn new(app: AppHandle, scan_id: i64) -> Self {
        Self {
            app,
            scan_id,
            discovered: AtomicU32::new(0),
            completed: AtomicU32::new(0),
        }
    }

    fn emit_progress(&self) {
        let _ = ScanProgress {
            scan_id: self.scan_id,
            discovered: self.discovered.load(Ordering::SeqCst),
            completed: self.completed.load(Ordering::SeqCst),
        }
        .emit(&self.app);
    }
}

impl ScanReporter for EventReporter {
    fn discovered(&self, total: usize) {
        self.discovered.store(total as u32, Ordering::SeqCst);
        self.emit_progress();
    }

    fn repo_done(&self, summary: &RepoSummary) {
        self.completed.fetch_add(1, Ordering::SeqCst);
        let _ = ScanRepoDone {
            scan_id: self.scan_id,
            repo_id: summary.repo_id,
            name: summary.name.clone(),
            path: summary.path.clone(),
            warning_count: summary.warning_count as u32,
        }
        .emit(&self.app);
        self.emit_progress();
    }

    fn warning(&self, w: &Warning) {
        let _ = ScanWarning {
            scan_id: self.scan_id,
            warning: w.clone(),
        }
        .emit(&self.app);
    }

    fn finished(&self, summary: &ScanSummary) {
        let _ = ScanComplete {
            scan_id: self.scan_id,
            repos_scanned: summary.repos_scanned as u32,
            warnings: summary.warnings as u32,
            cancelled: summary.cancelled,
        }
        .emit(&self.app);
    }
}
