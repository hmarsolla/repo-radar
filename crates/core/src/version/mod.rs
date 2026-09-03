//! Per-ecosystem version schemes (DESIGN §8.5, FR-4.6).
//!
//! These are deliberately **not** a shared code path: comparing PyPI
//! versions with SemVer mis-orders `rc`/`post`/epoch releases and silently
//! produces wrong findings. Each ecosystem gets its own comparator and its
//! own parsed representation ([`Ver`] variant).
//!
//! An unparseable version is never silently skipped. The caller turns
//! [`VersionError`] into a [`crate::model::Warning`] and marks the
//! dependency unmatchable — "we could not check this" and "this is fine"
//! must never look the same.

pub mod golang;
pub mod pep440;
pub mod semver;

use std::cmp::Ordering;

use crate::model::Ecosystem;

/// A parsed version. Each scheme parses into and compares only its own
/// variant; the matcher always uses one scheme per ecosystem, so a
/// cross-variant comparison is a bug (it falls back to `Equal` under
/// `debug_assert`).
#[derive(Debug, Clone)]
pub enum Ver {
    SemVer(::semver::Version),
    Pep440(::pep440_rs::Version),
    Go(golang::GoVer),
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("could not parse {input:?} as a {scheme} version: {detail}")]
pub struct VersionError {
    pub input: String,
    pub scheme: &'static str,
    pub detail: String,
}

/// Parse and totally order versions for one ecosystem.
pub trait VersionScheme: Send + Sync {
    fn name(&self) -> &'static str;
    fn parse(&self, s: &str) -> Result<Ver, VersionError>;
    fn cmp(&self, a: &Ver, b: &Ver) -> Ordering;

    /// `a >= b`.
    fn gte(&self, a: &Ver, b: &Ver) -> bool {
        self.cmp(a, b) != Ordering::Less
    }
    /// `a > b`.
    fn gt(&self, a: &Ver, b: &Ver) -> bool {
        self.cmp(a, b) == Ordering::Greater
    }
    /// `a < b`.
    fn lt(&self, a: &Ver, b: &Ver) -> bool {
        self.cmp(a, b) == Ordering::Less
    }
}

/// The scheme for an ecosystem (DESIGN §8.5). npm and crates.io share a
/// SemVer implementation *instance* but never a code path with PyPI or Go.
pub fn scheme_for(ecosystem: Ecosystem) -> &'static dyn VersionScheme {
    match ecosystem {
        Ecosystem::Npm | Ecosystem::CratesIo => &semver::SemVerScheme,
        Ecosystem::PyPI => &pep440::Pep440Scheme,
        Ecosystem::Go => &golang::GoScheme,
    }
}

/// Helper for scheme impls: a cross-variant `cmp` is a caller bug.
fn mismatched_cmp(scheme: &'static str) -> Ordering {
    debug_assert!(false, "{scheme}: compared versions of different schemes");
    Ordering::Equal
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheme_selection_matches_design_8_5() {
        assert_eq!(scheme_for(Ecosystem::Npm).name(), "semver");
        assert_eq!(scheme_for(Ecosystem::CratesIo).name(), "semver");
        assert_eq!(scheme_for(Ecosystem::PyPI).name(), "pep440");
        assert_eq!(scheme_for(Ecosystem::Go).name(), "go");
    }
}
