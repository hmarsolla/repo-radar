//! Typed events emitted to the frontend (DESIGN §12.2).
//!
//! Every long operation starts work and reports through these rather than
//! blocking an `invoke`, so the UI stays responsive and cancellable
//! (PRD §11). Payload shapes are refined as the emitting code lands
//! (scan events in M1-7, sync events in M2-15); the set is declared here now
//! so it appears in the generated bindings from the start.

use repo_radar_core::model::Warning;
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri_specta::Event;

/// `scan:progress` — drives the global progress bar.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct ScanProgress {
    pub scan_id: i64,
    pub discovered: u32,
    pub completed: u32,
}

/// `scan:repo_done` — one repo finished; the UI invalidates `['repos']` and
/// `['repo', id]` (DESIGN §14.1), populating the list incrementally.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct ScanRepoDone {
    pub scan_id: i64,
    pub repo_id: i64,
    pub name: String,
    pub path: String,
    pub warning_count: u32,
}

/// `scan:warning` — a recoverable problem, surfaced as it occurs.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct ScanWarning {
    pub scan_id: i64,
    pub warning: Warning,
}

/// `scan:complete`.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct ScanComplete {
    pub scan_id: i64,
    pub repos_scanned: u32,
    pub warnings: u32,
    pub cancelled: bool,
}

/// `scan:error` — scan-level failure only, never per-repo.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct ScanError {
    pub message: String,
}

/// `sync:progress`.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct SyncProgress {
    pub ecosystem: String,
    pub phase: String,
    pub done: u32,
    pub total: u32,
}

/// `sync:complete` — triggers a findings refresh in the UI.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct SyncComplete {
    pub ok: bool,
    pub message: Option<String>,
}
