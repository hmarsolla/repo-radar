//! The error type Tauri commands return (DESIGN §15).
//!
//! `CoreError` from the core crate is mapped into a serializable shape the
//! frontend can branch on: a `tier` discriminant (`fatal` drops the UI to
//! the recovery screen; `operation` shows an inline retry notice) plus a
//! human message.

use repo_radar_core::CoreError;
use serde::Serialize;
use specta::Type;

#[derive(Debug, Serialize, Type)]
#[serde(tag = "tier", rename_all = "snake_case")]
pub enum CommandError {
    /// Unrecoverable — show the recovery screen (Reset database / Open data folder).
    Fatal { message: String },
    /// A whole operation failed; previous state is intact, offer retry.
    Operation { message: String },
    /// Anything else — a bad argument, a transient query error.
    Internal { message: String },
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CommandError::Fatal { message }
            | CommandError::Operation { message }
            | CommandError::Internal { message } => f.write_str(message),
        }
    }
}

impl std::error::Error for CommandError {}

impl From<CoreError> for CommandError {
    fn from(e: CoreError) -> Self {
        match &e {
            CoreError::Fatal(_) => CommandError::Fatal {
                message: e.to_string(),
            },
            CoreError::Operation(_) => CommandError::Operation {
                message: e.to_string(),
            },
            _ => CommandError::Internal {
                message: e.to_string(),
            },
        }
    }
}

impl From<tauri::Error> for CommandError {
    fn from(e: tauri::Error) -> Self {
        CommandError::Internal {
            message: e.to_string(),
        }
    }
}

impl From<tauri_plugin_store::Error> for CommandError {
    fn from(e: tauri_plugin_store::Error) -> Self {
        CommandError::Internal {
            message: e.to_string(),
        }
    }
}

impl From<serde_json::Error> for CommandError {
    fn from(e: serde_json::Error) -> Self {
        CommandError::Internal {
            message: e.to_string(),
        }
    }
}

impl From<std::io::Error> for CommandError {
    fn from(e: std::io::Error) -> Self {
        CommandError::Internal {
            message: e.to_string(),
        }
    }
}

pub type CommandResult<T> = Result<T, CommandError>;
