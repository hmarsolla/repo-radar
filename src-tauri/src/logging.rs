//! Tracing setup (DESIGN §15, M0-7).
//!
//! A daily-rolling file appender under `<data>/logs/` plus a stderr layer
//! for `tauri dev`.
//!
//! # Why the writer is synchronous
//!
//! This used to wrap the appender in [`tracing_appender::non_blocking`],
//! whose `WorkerGuard` was parked in a `static` so it would outlive the
//! process. That combination silently loses log lines: the buffering worker
//! thread only drains periodically, and a `static` is never dropped, so the
//! guard's drop-time flush never runs. Anything still buffered when the
//! process ended — which is precisely the output describing a crash — was
//! discarded. Diagnosing a crash from the log was therefore impossible.
//!
//! The rolling appender is now used directly as a blocking writer. Each event
//! is handed to `File::write`, so it reaches the OS immediately and survives
//! even an abrupt process death. The cost is a write syscall per event on the
//! emitting thread, which is irrelevant at repo-radar's log volume (and less
//! than the volume this crate used to emit before `tokei` was quietened
//! below).

use std::sync::atomic::{AtomicBool, Ordering};

use repo_radar_core::Paths;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

static INITIALISED: AtomicBool = AtomicBool::new(false);

/// Initialise the global subscriber. Idempotent — a second call is ignored,
/// which keeps tests that construct several apps from panicking.
pub fn init(paths: &Paths) {
    if INITIALISED.load(Ordering::SeqCst) {
        return;
    }

    let log_dir = paths.log_dir();
    let _ = std::fs::create_dir_all(&log_dir);
    let file_writer = tracing_appender::rolling::daily(&log_dir, "repo-radar.log");

    // `RUST_LOG` overrides; default keeps our crates at debug, deps at warn.
    //
    // `tokei` is pinned to `error` because it logs a WARN per file whose
    // extension it does not recognise — one line per `.png`, `.pdf`, `.lock`
    // in every repository scanned. On a real tree that is tens of thousands
    // of lines per scan, which buried the messages that actually matter.
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new("repo_radar_lib=debug,repo_radar_core=debug,warn,tokei=error")
    });

    let registry = tracing_subscriber::registry().with(filter).with(
        fmt::layer()
            .with_ansi(false)
            .with_target(true)
            .with_writer(file_writer),
    );

    #[cfg(debug_assertions)]
    let registry = registry.with(fmt::layer().with_writer(std::io::stderr));

    if registry.try_init().is_ok() {
        INITIALISED.store(true, Ordering::SeqCst);
        install_panic_hook();
        tracing::info!(log_dir = %log_dir.display(), "logging initialised");
    }
}

/// Route panics into the log before the default hook runs.
///
/// Without this, a panic printed to stderr and nothing else: in a release
/// build there is no console attached, so the log — the only artefact a user
/// can send — showed a scan starting and then simply stopping, with no record
/// that anything went wrong. Since most of repo-radar's work happens on
/// background threads (the scan thread, rayon workers, the sync scheduler),
/// and a panic on a worker thread does not abort the process, these failures
/// were completely invisible.
///
/// The default hook is chained rather than replaced, so `tauri dev` still
/// gets the usual stderr output.
fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // `PanicHookInfo::payload` is the `&str`/`String` a `panic!` carried.
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "<non-string panic payload>".to_string());

        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "<unknown location>".to_string());

        tracing::error!(
            thread = std::thread::current().name().unwrap_or("<unnamed>"),
            location = %location,
            backtrace = %std::backtrace::Backtrace::force_capture(),
            "panic: {payload}"
        );

        default_hook(info);
    }));
}
