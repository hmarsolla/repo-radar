//! Application state (DESIGN §12.3).

use std::sync::{Arc, Mutex};

use repo_radar_core::CoreContext;

/// Held in Tauri's managed state and handed to every command.
pub struct AppState {
    /// DB pool, rule packs, injected paths — the shared analysis context.
    pub core: Arc<CoreContext>,
    /// Cancel token + join handle for the scan in flight, if any. Read from
    /// **M1-6** (`scan_start` / `scan_cancel`).
    #[allow(dead_code)]
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
}

/// Placeholder for the running-scan handle; fleshed out in M1-6.
#[allow(dead_code)]
pub struct ScanHandle {
    pub scan_id: i64,
}
