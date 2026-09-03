//! Scan pipeline orchestration (DESIGN §6).
//!
//! `run_scan` executes on a blocking thread with rayon parallelism inside,
//! one repo as the unit of work. Stages: discover → analyze (parallel) →
//! persist (single writer) → match + score → emit. Each `RepoAnalysis`
//! streams to the writer as it completes so the UI populates progressively
//! (Journey A).
//!
//! Implemented across **M1-1 … M1-8** and **M2-18**.

pub mod discovery;
pub mod git;
pub mod languages;
pub mod manifests;
pub mod pipeline;
pub mod progress;
pub mod submodule;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Cooperative cancellation (DESIGN §6.3). Checked between discovery
/// batches, at the top of each repo's analysis, and in the writer loop.
/// Cancellation is not an error — partial results persist and the scan row
/// is marked `cancelled`.
#[derive(Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}
