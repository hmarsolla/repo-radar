//! Cleanup / disk-reclaim commands.
//!
//! Read-only: these report what a repository costs on disk and whether it holds
//! work that exists nowhere else. Deleting the regenerable directories is left
//! to the user (see [`reveal_path`]), because recursive deletion inside
//! someone's repositories is the one thing repo-radar could do that destroys
//! data it did not derive.

use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

use repo_radar_core::db::cleanup::{self, CleanupFilter, CleanupRow, CleanupSummary};

use crate::error::{CommandError, CommandResult};
use crate::state::AppState;

/// Per-repository cleanup triage: footprint, regenerable directories, and the
/// reasons deleting the repository would lose work.
#[tauri::command]
#[specta::specta]
pub fn cleanup_list(
    state: State<'_, AppState>,
    filter: CleanupFilter,
) -> CommandResult<Vec<CleanupRow>> {
    let conn = state.core.db.read()?;
    Ok(cleanup::list(&conn, &filter)?)
}

/// Totals for the Cleanup view's summary tiles, over every repository
/// regardless of the active filter.
#[tauri::command]
#[specta::specta]
pub fn cleanup_summary(state: State<'_, AppState>) -> CommandResult<CleanupSummary> {
    let conn = state.core.db.read()?;
    Ok(cleanup::summary(&conn)?)
}

/// Show a path in the OS file manager.
///
/// This is how the Cleanup view hands off: the user sees exactly which
/// directory is worth deleting and then deletes it with the tool they already
/// trust for that, rather than repo-radar recursively removing directories on
/// their behalf.
///
/// The path is validated against the database before being opened, so only
/// directories repo-radar actually measured can be revealed — a path from
/// anywhere else is refused rather than passed to the shell.
#[tauri::command]
#[specta::specta]
pub fn reveal_path(
    app: AppHandle,
    state: State<'_, AppState>,
    repo_id: i64,
    rel_path: Option<String>,
) -> CommandResult<()> {
    let conn = state.core.db.read()?;
    let resolved = cleanup::resolve_reveal_path(&conn, repo_id, rel_path.as_deref())?;

    let Some(target) = resolved else {
        return Err(CommandError::Internal {
            message: "that path is not a directory repo-radar measured".into(),
        });
    };

    app.opener()
        .open_path(&target, None::<&str>)
        .map_err(|e| CommandError::Internal {
            message: format!("could not open {target}: {e}"),
        })
}
