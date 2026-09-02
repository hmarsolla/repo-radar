//! repo-radar-core — the analysis core.
//!
//! This crate carries every part of repo-radar that is worth testing in
//! isolation: repository discovery, git metadata extraction, language stats,
//! lockfile parsing, OSV ingestion and matching, health scoring, technology
//! and category classification, and prompt context assembly.
//!
//! # The load-bearing constraint
//!
//! **This crate has no `tauri` dependency and never will** (DESIGN §3). The
//! GUI shell (`src-tauri`) is a thin adapter that owns state, exposes
//! commands, and emits events. Filesystem locations arrive through an
//! injected [`Paths`] value; progress is reported through the
//! [`scan::progress::ScanReporter`] trait. Both seams exist so the core can
//! run under `cargo test` with no webview and no app shell.

pub mod db;
pub mod error;
pub mod model;
pub mod osv;
pub mod outdated;
pub mod parsers;
pub mod paths;
pub mod prompt;
pub mod rules;
pub mod scan;
pub mod score;
pub mod version;

pub use error::{CoreError, CoreResult};
pub use paths::Paths;

use std::sync::Arc;

use crate::db::Db;
use crate::rules::RulePacks;

/// Shared, process-wide analysis context: the database handles, the merged
/// rule packs, and the injected paths. `src-tauri` builds one of these at
/// startup and hands `Arc<CoreContext>` to every command; tests build one
/// against a `tempdir` and an in-memory database.
pub struct CoreContext {
    pub paths: Paths,
    pub db: Db,
    pub rules: RulePacks,
}

impl CoreContext {
    /// Build a context: open (or create) the database at `paths.data_dir`,
    /// run migrations, and load the rule packs.
    pub fn new(paths: Paths) -> CoreResult<Arc<Self>> {
        paths.ensure_dirs()?;
        let db = Db::open(&paths.database_file())?;
        let rules = RulePacks::load(&paths)?;
        Ok(Arc::new(Self { paths, db, rules }))
    }
}
