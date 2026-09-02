//! User settings (FR-10.1). Persisted as JSON in the OS config dir via the
//! Tauri store plugin — never in the source tree, never in SQLite (that is
//! for derived data only). Scan roots are the exception: they live in the
//! `scan_roots` table because repos foreign-key to them.

use serde::{Deserialize, Serialize};
use specta::Type;

/// The store file name (resolved under the OS config dir by the plugin).
pub const STORE_FILE: &str = "settings.json";
/// The single key inside that file holding the [`Settings`] blob.
pub const SETTINGS_KEY: &str = "settings";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    Light,
    Dark,
    System,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    /// Directory names pruned during discovery and language stats (FR-1.4).
    pub prune_list: Vec<String>,
    /// How often the scheduled advisory sync runs (DESIGN §13.2).
    pub sync_interval_hours: u32,
    /// Prompt token budget for the estimator (FR-9.4).
    pub token_budget: u32,
    pub theme: Theme,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            prune_list: DEFAULT_PRUNE_LIST.iter().map(|s| s.to_string()).collect(),
            sync_interval_hours: 24,
            token_budget: 128_000,
            theme: Theme::System,
        }
    }
}

/// Directories that are almost never worth walking into: dependency stores,
/// build output, VCS internals, virtualenvs (FR-1.4).
pub const DEFAULT_PRUNE_LIST: &[&str] = &[
    "node_modules",
    "target",
    "dist",
    "build",
    "out",
    "vendor",
    ".git",
    ".svn",
    ".hg",
    ".venv",
    "venv",
    "__pycache__",
    ".mypy_cache",
    ".pytest_cache",
    ".gradle",
    ".idea",
    ".next",
    ".nuxt",
    ".cache",
    "Pods",
];
