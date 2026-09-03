//! Lockfile and manifest parsers (DESIGN §7, FR-4).
//!
//! Every format implements [`LockfileParser`]; parsers are registered in a
//! `Vec<Box<dyn LockfileParser>>` and selected per manifest directory.
//! Adding an ecosystem is a new impl plus an [`crate::model::Ecosystem`]
//! variant — the pipeline, matcher, and scorer are untouched.
//!
//! Trait and registry: **M2-2**. Per-ecosystem parsers: **M2-4 … M2-10**.

pub mod cargo;
pub mod golang;
pub mod normalize;
pub mod npm;
pub mod python;

pub use normalize::normalize_package_name;

use std::path::Path;

use crate::model::{Dependency, Ecosystem, RelPath};

/// Whether a parser consumes a lockfile (exact versions) or a manifest
/// (ranges). Selection uses this: a present lockfile wins and its output is
/// `Confidence::Exact`; otherwise the manifest parser runs at
/// `Confidence::Range` (DESIGN §7.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManifestKind {
    Lockfile,
    Manifest,
}

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("malformed {format}: {detail}")]
    Malformed {
        format: &'static str,
        detail: String,
    },
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub trait LockfileParser: Send + Sync {
    fn ecosystem(&self) -> Ecosystem;
    fn kind(&self) -> ManifestKind;
    /// Match by file name only — cheap, no IO.
    fn matches(&self, path: &Path) -> bool;
    fn parse(&self, content: &str, path: &RelPath) -> Result<Vec<Dependency>, ParseError>;
}
