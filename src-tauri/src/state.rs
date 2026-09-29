//! Application state (DESIGN §12.3).

use std::sync::{Arc, Mutex, MutexGuard};

use repo_radar_core::scan::CancelToken;
use repo_radar_core::CoreContext;

/// Held in Tauri's managed state and handed to every command.
pub struct AppState {
    /// DB pool, rule packs, injected paths — the shared analysis context.
    pub core: Arc<CoreContext>,
    /// The scan in flight, if any: its id and cancel token
    /// (`scan_start` / `scan_cancel`, M1-6). At most one scan runs at a time.
    pub active_scan: Mutex<Option<ScanHandle>>,
    /// At most one advisory sync at a time; a manual **Sync now** must not
    /// race the scheduled sync into the same tables (DESIGN §12.3). Read
    /// from **M2-15**.
    #[allow(dead_code)]
    pub sync_lock: Arc<tokio::sync::Mutex<()>>,
}

impl AppState {
    pub fn new(core: Arc<CoreContext>) -> Self {
        Self {
            core,
            active_scan: Mutex::new(None),
            sync_lock: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    /// Lock [`Self::active_scan`], recovering if a panic poisoned it.
    ///
    /// The guarded value is a plain `Option<ScanHandle>` with no invariant a
    /// panic could leave half-built, so poison carries no information worth
    /// propagating — whereas propagating it (`.lock().unwrap()`) meant one
    /// panic anywhere near the scan slot made **every** later `scan_start`,
    /// `scan_cancel`, and slot release panic for the rest of the session.
    pub fn lock_active_scan(&self) -> MutexGuard<'_, Option<ScanHandle>> {
        self.active_scan
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// The running scan's id plus the token that cancels it.
pub struct ScanHandle {
    pub scan_id: i64,
    pub cancel: CancelToken,
}
