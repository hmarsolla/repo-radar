//! Outdated-dependency check command (FR-8, M5-2).
//!
//! This is the **only** call site of [`repo_radar_core::outdated`]. It runs
//! solely from an explicit user click on a repo's *Updates* tab — there is
//! no scheduler hook and the scan pipeline never touches it (FR-8.1). The
//! result is a standalone read-model; nothing here feeds the health score
//! (FR-8.6).

use tauri::State;

use repo_radar_core::outdated::{self, OutdatedReport};

use crate::error::{CommandError, CommandResult};
use crate::state::AppState;

/// Check every dependency of a repo against its package registry (npm, PyPI,
/// crates.io, Go module proxy).
///
/// **This contacts external registries.** The UI states so before the user
/// invokes it (FR-8.7). `force_refresh` bypasses the 24-hour result cache
/// (FR-8.5).
///
/// Synchronous: the call blocks until every lookup resolves. The frontend
/// shows a pending state meanwhile.
#[tauri::command]
#[specta::specta]
pub fn check_outdated(
    state: State<'_, AppState>,
    repo_id: i64,
    force_refresh: bool,
) -> CommandResult<OutdatedReport> {
    outdated::check_repo_outdated(&state.core.db, repo_id, force_refresh).map_err(|e| {
        CommandError::Operation {
            message: e.to_string(),
        }
    })
}
