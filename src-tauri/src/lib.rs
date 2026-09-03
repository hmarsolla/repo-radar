//! The Tauri shell (DESIGN §12). A thin adapter over `repo-radar-core`: it
//! owns [`AppState`], exposes commands, emits events, and constructs the
//! injected [`Paths`]. All analysis logic lives in the core crate.

mod commands;
mod error;
mod events;
mod logging;
mod scan_reporter;
mod settings;
mod state;

use std::sync::Arc;

use repo_radar_core::{CoreContext, Paths};
use tauri::Manager;
use tauri_specta::{collect_commands, collect_events, Builder};

use crate::state::AppState;

/// Absolute path of the generated bindings file, anchored to this crate's
/// manifest dir so it lands in `<repo>/src/bindings.ts` no matter what the
/// process's working directory is.
pub const BINDINGS_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../src/bindings.ts");

/// Build the tauri-specta [`Builder`]: the command set and the event set,
/// in one place so the runtime handler and the binding exporter can never
/// drift apart.
pub(crate) fn specta_builder() -> Builder<tauri::Wry> {
    Builder::<tauri::Wry>::new()
        // Every `i64` that crosses this boundary is a SQLite rowid or a
        // small count — all far inside JS's 2^53 safe-integer range. Export
        // them as `number` so the frontend isn't forced to thread `bigint`
        // through query keys and route params.
        .dangerously_cast_bigints_to_number()
        .commands(collect_commands![
            commands::system::ping,
            commands::system::get_startup_warnings,
            commands::settings::get_settings,
            commands::settings::set_settings,
            commands::settings::list_scan_roots,
            commands::settings::add_scan_root,
            commands::settings::remove_scan_root,
            commands::repos::scan_start,
            commands::repos::scan_cancel,
            commands::repos::list_repos,
            commands::repos::get_repo_detail,
        ])
        .events(collect_events![
            events::ScanProgress,
            events::ScanRepoDone,
            events::ScanWarning,
            events::ScanComplete,
            events::ScanError,
            events::SyncProgress,
            events::SyncComplete,
        ])
}

/// Resolve the three OS directories repo-radar writes to (DESIGN §13.1).
/// Nothing here ever points inside a scanned repo (FR-10.2).
fn resolve_paths(app: &tauri::App) -> Result<Paths, Box<dyn std::error::Error>> {
    let resolver = app.path();
    Ok(Paths::new(
        resolver.app_data_dir()?,
        resolver.app_config_dir()?,
        resolver.app_cache_dir()?,
    ))
}

/// Write `src/bindings.ts` from the command/event set. Called by the
/// `gen-bindings` binary (CI + `npm run bindings`) and, opportunistically,
/// on every debug run of the app.
pub fn export_bindings() -> Result<(), Box<dyn std::error::Error>> {
    specta_builder().export(specta_typescript::Typescript::default(), BINDINGS_PATH)?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = specta_builder();

    // Regenerate the TypeScript bindings on every dev run so a Rust type
    // change surfaces immediately in the frontend (DESIGN §19). CI checks
    // for drift by running `gen-bindings` then `git diff --exit-code`.
    #[cfg(debug_assertions)]
    if let Err(e) = export_bindings() {
        eprintln!("warning: failed to export typescript bindings: {e}");
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_store::Builder::new().build())
        .invoke_handler(builder.invoke_handler())
        .setup(move |app| {
            builder.mount_events(app);

            let paths = resolve_paths(app)?;
            logging::init(&paths);
            tracing::info!(version = env!("CARGO_PKG_VERSION"), "repo-radar starting");

            let core: Arc<CoreContext> = CoreContext::new(paths).map_err(|e| {
                tracing::error!(error = %e, "failed to initialise core context");
                e
            })?;
            app.manage(AppState::new(core));

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
