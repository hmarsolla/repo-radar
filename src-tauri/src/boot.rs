//! Startup outcome and database recovery (DESIGN §15, M5-4).
//!
//! `CoreContext::new` can fail fatally — a corrupt database file, a failed
//! migration, or a schema written by a newer build. Rather than let the
//! window die with a raw OS error dialog, the app always starts: it manages
//! a [`Boot`] value describing what happened, and the frontend shows the
//! recovery screen (**Reset database** / **Open data folder**) when
//! [`Boot::status`] is not `ok`.
//!
//! One fatal case self-heals: a *corrupt* database file is moved aside
//! (`repo-radar.db.corrupt-<timestamp>`) and creation is retried once, so
//! the common corruption case comes back as a note rather than a wall.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use specta::Type;

use repo_radar_core::error::{CoreError, FatalError};
use repo_radar_core::{CoreContext, Paths};

/// What startup produced. Always present in Tauri state; `core` is `Some`
/// only when analysis is available.
pub struct Boot {
    pub core: Option<Arc<CoreContext>>,
    pub data_dir: PathBuf,
    /// Set when startup failed fatally — the message shown on the recovery
    /// screen.
    pub failure: Option<String>,
    /// `true` for [`FatalError::SchemaTooNew`]: **Reset database** would
    /// discard data a newer build wrote, so the screen warns and leads with
    /// **Open data folder** instead.
    pub schema_too_new: bool,
    /// A non-fatal note, e.g. "a corrupt database was moved aside and
    /// recreated" — shown once as a dismissible banner.
    pub note: Option<String>,
}

/// The serializable view the `boot_status` command returns.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct BootStatus {
    /// `true` when analysis is available and the app can run normally.
    pub ok: bool,
    /// Present when `!ok` — a human explanation for the recovery screen.
    pub failure: Option<String>,
    pub schema_too_new: bool,
    /// Present after an automatic recovery (corrupt file quarantined).
    pub note: Option<String>,
}

impl Boot {
    pub fn status(&self) -> BootStatus {
        BootStatus {
            ok: self.core.is_some(),
            failure: self.failure.clone(),
            schema_too_new: self.schema_too_new,
            note: self.note.clone(),
        }
    }
}

/// Build the core context, recovering from a corrupt database once.
pub fn build(paths: Paths) -> Boot {
    let data_dir = paths.data_dir.clone();
    match CoreContext::new(paths.clone()) {
        Ok(core) => Boot {
            core: Some(core),
            data_dir,
            failure: None,
            schema_too_new: false,
            note: None,
        },
        Err(err) if is_corruption(&err) => {
            tracing::warn!(error = %err, "database appears corrupt — quarantining and retrying");
            match quarantine_db(&paths.database_file()).and_then(|_| {
                CoreContext::new(paths.clone())
                    .map_err(|e| std::io::Error::other(format!("recreate after quarantine: {e}")))
            }) {
                Ok(core) => Boot {
                    core: Some(core),
                    data_dir,
                    failure: None,
                    schema_too_new: false,
                    note: Some(
                        "The database file was unreadable and has been moved aside \
                         (repo-radar.db.corrupt-*). A fresh one was created — re-scan \
                         to repopulate it."
                            .into(),
                    ),
                },
                Err(e) => Boot {
                    core: None,
                    data_dir,
                    failure: Some(format!(
                        "The database is corrupt and could not be recreated automatically: {e}"
                    )),
                    schema_too_new: false,
                    note: None,
                },
            }
        }
        Err(err) => {
            let schema_too_new = matches!(&err, CoreError::Fatal(FatalError::SchemaTooNew { .. }));
            Boot {
                core: None,
                data_dir,
                failure: Some(err.to_string()),
                schema_too_new,
                note: None,
            }
        }
    }
}

fn is_corruption(err: &CoreError) -> bool {
    matches!(err, CoreError::Fatal(FatalError::DatabaseCorruption(_)))
}

/// Move `repo-radar.db` (and its `-wal` / `-shm` siblings) to
/// `repo-radar.db.corrupt-<unix_millis>`. The user can still inspect or
/// delete the quarantined copy from **Open data folder**.
fn quarantine_db(db_file: &Path) -> std::io::Result<()> {
    let stamp = chrono::Utc::now().timestamp_millis();
    for suffix in ["", "-wal", "-shm"] {
        let from = with_suffix(db_file, suffix);
        if from.exists() {
            let to = with_suffix(
                &db_file.with_file_name(format!(
                    "{}.corrupt-{stamp}",
                    db_file.file_name().unwrap_or_default().to_string_lossy()
                )),
                suffix,
            );
            std::fs::rename(&from, &to)?;
        }
    }
    Ok(())
}

/// Delete `repo-radar.db` and its `-wal` / `-shm` siblings. Used by the
/// recovery-screen **Reset database** when the core never initialised, so
/// the next launch starts from a clean file.
pub fn delete_db_files(data_dir: &Path) -> std::io::Result<()> {
    let db_file = data_dir.join("repo-radar.db");
    for suffix in ["", "-wal", "-shm"] {
        let p = with_suffix(&db_file, suffix);
        if p.exists() {
            std::fs::remove_file(&p)?;
        }
    }
    Ok(())
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    if suffix.is_empty() {
        path.to_path_buf()
    } else {
        let mut s = path.as_os_str().to_os_string();
        s.push(suffix);
        PathBuf::from(s)
    }
}
