//! Settings and scan-root commands (DESIGN §12.1, FR-10.1/10.2).

use tauri::{AppHandle, State};
use tauri_plugin_store::StoreExt;

use repo_radar_core::db::repos::{self, ScanRoot};

use crate::error::{CommandError, CommandResult};
use crate::settings::{Settings, SETTINGS_KEY, STORE_FILE};
use crate::state::AppState;

/// Read settings from the store, falling back to defaults when the value is
/// missing or unparseable.
#[tauri::command]
#[specta::specta]
pub fn get_settings(app: AppHandle) -> CommandResult<Settings> {
    let store = app.store(STORE_FILE)?;
    let settings = store
        .get(SETTINGS_KEY)
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    Ok(settings)
}

/// Replace the whole settings blob and flush to disk.
#[tauri::command]
#[specta::specta]
pub fn set_settings(app: AppHandle, settings: Settings) -> CommandResult<()> {
    let store = app.store(STORE_FILE)?;
    let value = serde_json::to_value(&settings).map_err(|e| CommandError::Internal {
        message: e.to_string(),
    })?;
    store.set(SETTINGS_KEY, value);
    store.save()?;
    Ok(())
}

/// List configured scan roots.
#[tauri::command]
#[specta::specta]
pub fn list_scan_roots(state: State<'_, AppState>) -> CommandResult<Vec<ScanRoot>> {
    let conn = state.core.db.read()?;
    Ok(repos::list_scan_roots(&conn)?)
}

/// Add a scan root after validating the path exists and is a readable
/// directory (M0-9). Idempotent on the path.
#[tauri::command]
#[specta::specta]
pub fn add_scan_root(state: State<'_, AppState>, path: String) -> CommandResult<ScanRoot> {
    validate_scan_root(&path)?;
    let canonical = std::fs::canonicalize(&path)
        .map(|p| normalize_path(&p.to_string_lossy()))
        .unwrap_or_else(|_| normalize_path(&path));
    let root = state
        .core
        .db
        .write(|c| repos::add_scan_root(c, &canonical))?;
    tracing::info!(path = %root.path, id = root.id, "scan root added");
    Ok(root)
}

/// Canonical form for storage: forward slashes, and without the Windows
/// `\\?\` verbatim prefix that `std::fs::canonicalize` adds.
fn normalize_path(p: &str) -> String {
    p.strip_prefix(r"\\?\").unwrap_or(p).replace('\\', "/")
}

/// Remove a scan root and everything derived from it (FK cascade).
#[tauri::command]
#[specta::specta]
pub fn remove_scan_root(state: State<'_, AppState>, id: i64) -> CommandResult<()> {
    state.core.db.write(|c| repos::remove_scan_root(c, id))?;
    tracing::info!(id, "scan root removed");
    Ok(())
}

fn validate_scan_root(path: &str) -> CommandResult<()> {
    let meta = std::fs::metadata(path).map_err(|e| CommandError::Internal {
        message: format!("cannot access {path}: {e}"),
    })?;
    if !meta.is_dir() {
        return Err(CommandError::Internal {
            message: format!("{path} is not a directory"),
        });
    }
    // Readability probe: listing the directory.
    std::fs::read_dir(path).map_err(|e| CommandError::Internal {
        message: format!("{path} is not readable: {e}"),
    })?;
    Ok(())
}
