//! OSV ingestion and matching (DESIGN §8) — the highest-risk logic in the
//! project.
//!
//! - [`record`] — the OSV JSON subset, classification, severity extraction
//! - [`sync`] — bulk + incremental download and ingest (FR-5)
//! - [`matcher`] — the two-phase match: SQL narrows, Rust decides (§8.4)
//! - [`severity`] — CVSS vector parsing with precedence (§8.3)
//!
//! Implemented across **M2-11 … M2-16**. **M2-12 (the OSV data spike) runs
//! before any of it.**

pub mod matcher;
pub mod record;
pub mod severity;
pub mod sync;

pub use record::{
    classify, has_malicious_marker, normalize, MalCoverage, NormalizedAdvisory, NormalizedAffected,
    NormalizedRange, OsvRecord, RangeEvent, RangeType,
};
