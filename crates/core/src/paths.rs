//! Injected filesystem locations (DESIGN §13.1).
//!
//! The core never asks the OS where it lives. `src-tauri` constructs [`Paths`]
//! from Tauri's path resolver and passes it in; tests construct one pointing
//! at a `tempdir`. Per FR-10.2, nothing here ever points inside a scanned
//! repository or the source tree.

use std::path::{Path, PathBuf};

use crate::error::CoreResult;

/// The three OS-provided directories repo-radar writes to. All app state is
/// derived and rebuildable, so losing any of these costs a re-scan, never
/// user data.
#[derive(Debug, Clone)]
pub struct Paths {
    /// Holds `repo-radar.db`.
    pub data_dir: PathBuf,
    /// Holds `settings.json`, `rules/`, and `prompts/`.
    pub config_dir: PathBuf,
    /// Scratch space for in-flight OSV downloads before atomic swap.
    pub cache_dir: PathBuf,
}

impl Paths {
    /// Construct from three explicit directories.
    pub fn new(
        data_dir: impl Into<PathBuf>,
        config_dir: impl Into<PathBuf>,
        cache_dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            data_dir: data_dir.into(),
            config_dir: config_dir.into(),
            cache_dir: cache_dir.into(),
        }
    }

    /// Derive all three from a single base directory. Used by tests and by
    /// the `--portable` path; production uses [`Paths::new`] with the OS dirs.
    pub fn under(base: impl AsRef<Path>) -> Self {
        let base = base.as_ref();
        Self::new(base.join("data"), base.join("config"), base.join("cache"))
    }

    /// The SQLite database file.
    pub fn database_file(&self) -> PathBuf {
        self.data_dir.join("repo-radar.db")
    }

    /// The log directory (a `logs/` subdir of the data dir).
    pub fn log_dir(&self) -> PathBuf {
        self.data_dir.join("logs")
    }

    /// User rule-pack directory: `<config>/rules/`.
    pub fn rules_dir(&self) -> PathBuf {
        self.config_dir.join("rules")
    }

    /// User prompt-template directory: `<config>/prompts/`.
    pub fn prompts_dir(&self) -> PathBuf {
        self.config_dir.join("prompts")
    }

    /// The settings file: `<config>/settings.json`.
    pub fn settings_file(&self) -> PathBuf {
        self.config_dir.join("settings.json")
    }

    /// Create every directory this struct names, if absent. Idempotent.
    pub fn ensure_dirs(&self) -> CoreResult<()> {
        for dir in [
            &self.data_dir,
            &self.config_dir,
            &self.cache_dir,
            &self.log_dir(),
            &self.rules_dir(),
            &self.prompts_dir(),
        ] {
            std::fs::create_dir_all(dir)?;
        }
        Ok(())
    }
}
