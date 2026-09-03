//! Advisory sync + status commands (DESIGN §12.1, M2-15 / M2-20 / M2-21).

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager, State};
use tauri_specta::Event as _;

use repo_radar_core::db::advisories::{self, SyncStatus};
use repo_radar_core::db::findings;
use repo_radar_core::db::repos as repo_db;
use repo_radar_core::model::Ecosystem;
use repo_radar_core::osv::sync::{
    self, SyncMode, SyncOptions, SyncReporter,
};
use repo_radar_core::CoreContext;

use crate::error::{CommandError, CommandResult};
use crate::events::{SyncComplete, SyncProgress};
use crate::state::AppState;

/// Start an advisory sync in the background. Returns immediately; progress
/// via `sync:progress`, finish via `sync:complete`. A manual sync and the
/// scheduled sync cannot overlap (the `sync_lock`, DESIGN §12.3).
#[tauri::command]
#[specta::specta]
pub fn sync_advisories(app: AppHandle, state: State<'_, AppState>, mode: SyncMode) -> CommandResult<()> {
    // Non-blocking: if a sync already holds the lock, say so.
    let guard = match Arc::clone(&state.sync_lock).try_lock_owned() {
        Ok(g) => g,
        Err(_) => {
            return Err(CommandError::Operation {
                message: "a sync is already running".into(),
            })
        }
    };

    let core: Arc<CoreContext> = Arc::clone(&state.core);
    let thread_app = app.clone();
    std::thread::Builder::new()
        .name("repo-radar-sync".into())
        .spawn(move || {
            let _guard = guard; // released when the thread ends
            let outcome = run_sync(&core, mode, &thread_app);
            let (ok, message) = match &outcome {
                Ok(n) => (true, Some(format!("{n} advisories updated"))),
                Err(e) => (false, Some(e.clone())),
            };
            if outcome.is_ok() {
                rescore_all(&core, &thread_app);
            }
            let _ = SyncComplete { ok, message }.emit(&thread_app);
        })
        .map_err(|e| CommandError::Internal {
            message: format!("could not start sync thread: {e}"),
        })?;
    Ok(())
}

fn run_sync(core: &CoreContext, mode: SyncMode, app: &AppHandle) -> Result<usize, String> {
    let ecosystems = sync::ecosystems_in_use(&core.db)
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| Ecosystem::ALL.to_vec());
    let opts = SyncOptions {
        ecosystems,
        cache_dir: core.paths.cache_dir.clone(),
    };
    let reporter = EventSyncReporter { app: app.clone() };
    sync::sync_with_retry(&core.db, mode, &opts, &reporter)
        .map(|s| s.total())
        .map_err(|e| e.to_string())
}

/// Re-run stage 4 for every repo so scores reflect the new advisory data.
fn rescore_all(core: &CoreContext, app: &AppHandle) {
    let weights = repo_radar_core::score::Weights::default();
    let repos: Vec<(i64, String)> = match core
        .db
        .read()
        .and_then(|c| repo_db::list_top_level(&c))
    {
        Ok(list) => list.into_iter().map(|r| (r.id, r.path)).collect(),
        Err(_) => return,
    };
    for (id, path) in repos {
        let _ = core
            .db
            .write(|c| findings::match_score_and_persist(c, id, &path, &weights));
    }
    // Nudge the UI to refetch repo health.
    let _ = crate::events::ScanComplete {
        scan_id: -1,
        repos_scanned: 0,
        warnings: 0,
        cancelled: false,
    }
    .emit(app);
}

struct EventSyncReporter {
    app: AppHandle,
}
impl SyncReporter for EventSyncReporter {
    fn phase(&self, ecosystem: Ecosystem, phase: &str, done: usize, total: usize) {
        let _ = SyncProgress {
            ecosystem: ecosystem.osv_id().to_string(),
            phase: phase.to_string(),
            done: done as u32,
            total: total as u32,
        }
        .emit(&self.app);
    }
}

/// Current advisory-database status (drives the freshness indicator and the
/// Advisories screen).
#[tauri::command]
#[specta::specta]
pub fn get_sync_status(state: State<'_, AppState>) -> CommandResult<SyncStatus> {
    let conn = state.core.db.read()?;
    Ok(advisories::sync_status(&conn)?)
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AdvisoryImpact {
    pub repo_id: i64,
    pub repo_name: String,
}

/// Which repos a given advisory currently affects (cross-repo impact view).
#[tauri::command]
#[specta::specta]
pub fn list_advisory_impact(
    state: State<'_, AppState>,
    advisory_id: String,
) -> CommandResult<Vec<AdvisoryImpact>> {
    let conn = state.core.db.read()?;
    Ok(findings::repos_affected_by(&conn, &advisory_id)?
        .into_iter()
        .map(|(repo_id, repo_name)| AdvisoryImpact { repo_id, repo_name })
        .collect())
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LiveQueryResult {
    pub advisory_ids: Vec<String>,
}

/// FR-5.9 live query — opt-in, per-dependency. **This sends the package name
/// and version to `api.osv.dev`.** Never reachable from a scan or any
/// automatic path (M2-21).
#[tauri::command]
#[specta::specta]
pub fn live_query(
    ecosystem: String,
    name: String,
    version: String,
) -> CommandResult<LiveQueryResult> {
    let eco = Ecosystem::from_osv_id(&ecosystem).ok_or_else(|| CommandError::Internal {
        message: format!("unknown ecosystem {ecosystem}"),
    })?;
    let advisory_ids = repo_radar_core::osv::sync::live_query(eco, &name, &version)
        .map_err(|e| CommandError::Operation {
            message: e.to_string(),
        })?;
    Ok(LiveQueryResult { advisory_ids })
}

/// The scheduled-sync tick (DESIGN §13.2, M2-15). An hourly interval with a
/// 24-hour condition: a suspended laptop catches up on wake instead of
/// waiting out a dead timer.
pub fn spawn_scheduler(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(3600));
        loop {
            tick.tick().await;
            let Some(state) = app.try_state::<AppState>() else { break };
            let due = {
                let conn = match state.core.db.read() {
                    Ok(c) => c,
                    Err(_) => continue,
                };
                match advisories::sync_status(&conn) {
                    Ok(s) => s
                        .last_success
                        .as_deref()
                        .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
                        .map(|t| {
                            chrono::Utc::now() - t.with_timezone(&chrono::Utc)
                                > chrono::Duration::hours(24)
                        })
                        .unwrap_or(true),
                    Err(_) => false,
                }
            };
            if !due {
                continue;
            }
            if let Ok(guard) = Arc::clone(&state.sync_lock).try_lock_owned() {
                let core = Arc::clone(&state.core);
                let app2 = app.clone();
                std::thread::spawn(move || {
                    let _g = guard;
                    if run_sync(&core, SyncMode::Incremental, &app2).is_ok() {
                        rescore_all(&core, &app2);
                    }
                    let _ = SyncComplete {
                        ok: true,
                        message: None,
                    }
                    .emit(&app2);
                });
            }
        }
    });
}
