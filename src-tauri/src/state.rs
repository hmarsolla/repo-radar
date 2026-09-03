//! Application state (DESIGN §12.3).

use std::sync::{Arc, Mutex};

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
}

/// The running scan's id plus the token that cancels it.
pub struct ScanHandle {
    pub scan_id: i64,
    pub cancel: CancelToken,
}
