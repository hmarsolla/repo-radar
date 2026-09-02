//! Per-ecosystem version schemes (DESIGN §8.5, FR-4.6).
//!
//! These are deliberately **not** a shared code path: comparing PyPI
//! versions with SemVer mis-orders `rc`/`post`/epoch releases and silently
//! produces wrong findings. Each ecosystem gets its own comparator.
//!
//! Implemented in **M2-1**.

pub mod golang;
pub mod pep440;
pub mod semver;

use std::cmp::Ordering;

/// An opaque parsed version. Each scheme defines its own representation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ver(pub String);

#[derive(Debug, thiserror::Error)]
#[error("unparseable version {input:?} for {scheme}")]
pub struct VersionError {
    pub input: String,
    pub scheme: &'static str,
}

/// Parse and total-order versions for one ecosystem. An unparseable version
/// is never silently skipped — the caller turns [`VersionError`] into a
/// [`crate::model::Warning`] and marks the dependency unmatchable (§8.5).
pub trait VersionScheme: Send + Sync {
    fn name(&self) -> &'static str;
    fn parse(&self, s: &str) -> Result<Ver, VersionError>;
    fn cmp(&self, a: &Ver, b: &Ver) -> Ordering;
}
