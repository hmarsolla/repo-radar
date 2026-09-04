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
    /// Added to the built-in [`DEFAULT_PRUNE_LIST`], not a replacement.
    pub prune_list: Vec<String>,
    /// File extensions (no dot, lowercase) withheld from the prompt file
    /// picker in addition to the content-sniffed binary check (FR-10.1).
    pub excluded_extensions: Vec<String>,
    /// How often the scheduled advisory sync runs (DESIGN §13.2). `0` means
    /// **manual only** — the scheduler performs no automatic sync (FR-10.1,
    /// "daily / manual only").
    pub sync_interval_hours: u32,
    /// Prompt token budget for the estimator (FR-9.4).
    pub token_budget: u32,
    pub theme: Theme,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            // Empty by default: these are *added* to the built-in prune list
            // (FR-10.1), not a replacement for it.
            prune_list: Vec::new(),
            excluded_extensions: Vec::new(),
            sync_interval_hours: 24,
            token_budget: 128_000,
            theme: Theme::System,
        }
    }
}

// The built-in prune list is `repo_radar_core::scan::discovery::DEFAULT_PRUNE_DIRS`
// — a single source of truth shared by discovery, language stats, and the
// prompt file picker. `Settings::prune_list` adds to it (FR-10.1); the
// `builtin_prune_dirs` command exposes the built-ins to the Settings UI.
