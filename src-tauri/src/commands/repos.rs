//! Scan control and repository query commands (DESIGN §12.1, M1-7 / M1-9).

use std::sync::Arc;

use tauri::{AppHandle, Manager, State};
use tauri_specta::Event as _;

use repo_radar_core::db::dashboard::{self, DashboardStats};
use repo_radar_core::db::repos::{self as repo_db, RepoDetail, RepoFilter, RepoListItem};
use repo_radar_core::db::scans::{self, ScanSummary};
use repo_radar_core::scan::pipeline::{self, ScanContext, ScanRoot};
use repo_radar_core::scan::CancelToken;
use repo_radar_core::CoreContext;

use crate::error::{CommandError, CommandResult};
use crate::events::ScanError;
use crate::scan_reporter::EventReporter;
use crate::state::{AppState, ScanHandle};

/// Start a scan of every enabled scan root. Returns the scan id immediately;
/// the walk, analysis, and persistence run on a background thread and report
/// through `scan:*` events (DESIGN §12.1).
#[tauri::command]
#[specta::specta]
pub fn scan_start(app: AppHandle, state: State<'_, AppState>) -> CommandResult<i64> {
    {
        let active = state.active_scan.lock().unwrap();
        if active.is_some() {
            return Err(CommandError::Internal {
                message: "a scan is already running".into(),
            });
        }
    }

    let core: Arc<CoreContext> = Arc::clone(&state.core);

    let roots: Vec<ScanRoot> = {
        let conn = core.db.read()?;
        repo_db::list_scan_roots(&conn)?
            .into_iter()
            .filter(|r| r.enabled)
            .map(ScanRoot::from)
            .collect()
    };
    if roots.is_empty() {
        return Err(CommandError::Internal {
            message: "no scan roots configured — add a folder in Settings".into(),
        });
    }

    // FR-10.1: the user's `prune_list` is *added* to the built-ins.
    let extra_prune: Vec<String> = {
        use tauri_plugin_store::StoreExt;
        app.store(crate::settings::STORE_FILE)
            .ok()
            .and_then(|s| s.get(crate::settings::SETTINGS_KEY))
            .and_then(|v| serde_json::from_value::<crate::settings::Settings>(v).ok())
            .map(|s| s.prune_list)
            .unwrap_or_default()
    };

    let scan_id = pipeline::begin_scan(&core.db)?;
    let cancel = CancelToken::new();
    *state.active_scan.lock().unwrap() = Some(ScanHandle {
        scan_id,
        cancel: cancel.clone(),
    });

    let thread_app = app.clone();
    std::thread::Builder::new()
        .name(format!("repo-radar-scan-{scan_id}"))
        .spawn(move || {
            let reporter = EventReporter::new(thread_app.clone(), scan_id);
            let mut ctx = ScanContext::new(&core.db, &core.rules);
            for dir in extra_prune {
                let dir = dir.trim().to_string();
                if !dir.is_empty() && !ctx.discovery.prune_dirs.contains(&dir) {
                    ctx.discovery.prune_dirs.push(dir);
                }
            }
            if let Err(e) = pipeline::run_scan(&ctx, scan_id, &roots, &cancel, &reporter) {
                tracing::error!(scan_id, error = %e, "scan failed");
                let _ = ScanError {
                    message: e.to_string(),
                }
                .emit(&thread_app);
            }
            // Release the slot for the next scan.
            if let Some(state) = thread_app.try_state::<AppState>() {
                let mut active = state.active_scan.lock().unwrap();
                if active.as_ref().is_some_and(|h| h.scan_id == scan_id) {
                    *active = None;
                }
            }
        })
        .map_err(|e| CommandError::Internal {
            message: format!("could not start scan thread: {e}"),
        })?;

    Ok(scan_id)
}

/// Signal the running scan to stop. Already-persisted repos remain; the scan
/// row ends up `cancelled` (FR-1.9). A no-op if `scan_id` is not the scan in
/// flight.
#[tauri::command]
#[specta::specta]
pub fn scan_cancel(state: State<'_, AppState>, scan_id: i64) -> CommandResult<()> {
    if let Some(handle) = state.active_scan.lock().unwrap().as_ref() {
        if handle.scan_id == scan_id {
            handle.cancel.cancel();
            tracing::info!(scan_id, "scan cancellation requested");
        }
    }
    Ok(())
}

/// List repositories, filtered and sorted **in SQL** per `filter`
/// (DESIGN §12.1).
#[tauri::command]
#[specta::specta]
pub fn list_repos(
    state: State<'_, AppState>,
    filter: RepoFilter,
) -> CommandResult<Vec<RepoListItem>> {
    let conn = state.core.db.read()?;
    Ok(repo_db::list_repos(&conn, &filter)?)
}

/// Full detail for one repository: its record, language breakdown,
/// technologies, and submodule children.
#[tauri::command]
#[specta::specta]
pub fn get_repo_detail(state: State<'_, AppState>, id: i64) -> CommandResult<Option<RepoDetail>> {
    let conn = state.core.db.read()?;
    Ok(repo_db::get_repo_detail(&conn, id)?)
}

/// Set or clear the manual category override (FR-3.7). `category = None`
/// (or an unrecognised string) reverts to the computed value; the computed
/// category stays visible beside the override either way.
#[tauri::command]
#[specta::specta]
pub fn set_repo_category(
    state: State<'_, AppState>,
    id: i64,
    category: Option<String>,
) -> CommandResult<()> {
    let parsed = category.as_deref().and_then(repo_db::category_from_str);
    state
        .core
        .db
        .write(|c| repo_db::set_category_manual(c, id, parsed))?;
    Ok(())
}

/// Every figure the Dashboard renders, in one round trip (PRD §6). The
/// compromise banner (FR-6.3) keys off `compromised` being non-empty.
#[tauri::command]
#[specta::specta]
pub fn dashboard_stats(state: State<'_, AppState>) -> CommandResult<DashboardStats> {
    let conn = state.core.db.read()?;
    Ok(dashboard::stats(&conn)?)
}

/// The most recent scan and its persisted warnings (DESIGN §14.4). `None`
/// until a scan has run. The frontend uses it to tell "never scanned" from
/// "scanned, found nothing", and to badge repos that produced warnings even
/// after a reload.
#[tauri::command]
#[specta::specta]
pub fn latest_scan_summary(state: State<'_, AppState>) -> CommandResult<Option<ScanSummary>> {
    let conn = state.core.db.read()?;
    Ok(scans::latest_scan(&conn)?)
}
