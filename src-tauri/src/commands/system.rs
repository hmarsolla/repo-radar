//! System / diagnostics commands.

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::State;

use repo_radar_core::model::Warning;

use crate::error::CommandResult;
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

/// Warnings raised while loading rule packs at startup (DESIGN §10.2). Wired
/// up now so the [`Warning`] type is exercised across the boundary (M0-7);
/// the scan surfaces per-repo warnings the same way from M1-8.
#[tauri::command]
#[specta::specta]
pub fn get_startup_warnings(state: State<'_, AppState>) -> CommandResult<Vec<Warning>> {
    Ok(state.core.rules.load_warnings.clone())
}
