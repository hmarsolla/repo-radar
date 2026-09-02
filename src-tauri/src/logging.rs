//! Tracing setup (DESIGN §15, M0-7).
//!
//! A daily-rolling file appender under `<data>/logs/` plus a stderr layer
//! for `tauri dev`. The non-blocking writer's `WorkerGuard` must outlive the
//! process, so it is parked in a `OnceLock`.

use std::sync::OnceLock;

use repo_radar_core::Paths;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

static GUARD: OnceLock<WorkerGuard> = OnceLock::new();

/// Initialise the global subscriber. Idempotent — a second call is ignored,
/// which keeps tests that construct several apps from panicking.
pub fn init(paths: &Paths) {
    if GUARD.get().is_some() {
        return;
    }

    let log_dir = paths.log_dir();
    let _ = std::fs::create_dir_all(&log_dir);
    let file_appender = tracing_appender::rolling::daily(&log_dir, "repo-radar.log");
    let (file_writer, guard) = tracing_appender::non_blocking(file_appender);

    // `RUST_LOG` overrides; default keeps our crates at debug, deps at warn.
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("repo_radar_lib=debug,repo_radar_core=debug,warn"));

    let registry = tracing_subscriber::registry().with(filter).with(
        fmt::layer()
            .with_ansi(false)
            .with_target(true)
            .with_writer(file_writer),
    );

    #[cfg(debug_assertions)]
    let registry = registry.with(fmt::layer().with_writer(std::io::stderr));

    if registry.try_init().is_ok() {
        let _ = GUARD.set(guard);
        tracing::info!(log_dir = %log_dir.display(), "logging initialised");
    }
}
