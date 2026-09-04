//! System / diagnostics commands.

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_opener::OpenerExt;

use repo_radar_core::db::maintenance;
use repo_radar_core::model::Warning;

use crate::boot::{self, Boot, BootStatus};
use crate::error::{CommandError, CommandResult};
use crate::state::AppState;

/// Round-trips a value through the Rust↔TS boundary. Exists to prove the
/// binding pipeline end to end (M0-5): change this struct in Rust and the
/// generated `bindings.ts` must change with no hand-editing.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct Pong {
    pub echo: String,
    pub schema_version: i64,
    pub pid: u32,
}

#[tauri::command]
#[specta::specta]
pub fn ping(state: State<'_, AppState>, message: String) -> CommandResult<Pong> {
    let schema_version = state.core.db.schema_version()?;
    Ok(Pong {
        echo: message,
        schema_version,
        pid: std::process::id(),
    })
}

/// Warnings raised while loading rule packs at startup (DESIGN §10.2).
/// Returns an empty list in recovery mode (no core context).
#[tauri::command]
#[specta::specta]
pub fn get_startup_warnings(app: AppHandle) -> CommandResult<Vec<Warning>> {
    Ok(app
        .try_state::<AppState>()
        .map(|s| s.core.rules.load_warnings.clone())
        .unwrap_or_default())
}

/// Startup outcome (M5-4). The frontend calls this before rendering: when
/// `ok` is false it shows the recovery screen instead of the app.
#[tauri::command]
#[specta::specta]
pub fn boot_status(boot: State<'_, Boot>) -> CommandResult<BootStatus> {
    Ok(boot.status())
}

/// **Reset database** (FR-10.3). Clears every scanned repository, its health
/// and classification data, all findings, the advisory database, and the
/// outdated-version cache. Configured scan roots and preferences are kept.
/// The UI confirms before calling this; it is also the recovery action on
/// the fatal-error screen (DESIGN §15).
///
/// In recovery mode (the core never initialised) there is no live database
/// to clear, so the `.db` files are deleted outright and the next launch
/// starts from a clean file.
#[tauri::command]
#[specta::specta]
pub fn reset_database(app: AppHandle, boot: State<'_, Boot>) -> CommandResult<()> {
    match app.try_state::<AppState>() {
        Some(state) => {
            state.core.db.write(maintenance::reset_derived_data)?;
            tracing::warn!("database reset — all derived data cleared");
        }
        None => {
            boot::delete_db_files(&boot.data_dir).map_err(|e| CommandError::Internal {
                message: format!("could not delete the database file: {e}"),
            })?;
            tracing::warn!("recovery reset — database files deleted; restart required");
        }
    }
    Ok(())
}

/// **Open data folder** (FR-10.3) — reveal the OS data directory (which
/// holds `repo-radar.db` and `logs/`) in the system file manager. Works in
/// recovery mode too.
#[tauri::command]
#[specta::specta]
pub fn open_data_folder(app: AppHandle, boot: State<'_, Boot>) -> CommandResult<()> {
    let dir = boot.data_dir.clone();
    app.opener()
        .open_path(dir.to_string_lossy(), None::<&str>)
        .map_err(|e| CommandError::Internal {
            message: format!("could not open {}: {e}", dir.display()),
        })
}
