//! Advisory sync (FR-5, DESIGN §8.6).
//!
//! Full sync streams per-ecosystem zips — never fully buffered — and ingests
//! in ~1,000-record transactions with per-ecosystem atomic replacement.
//! Incremental sync walks `modified_id.csv` and fetches deltas, falling back
//! to a full zip above 2,000 changed IDs.
//!
//! Implemented in **M2-13 … M2-15**. This module is the one place in core
//! that performs outbound network IO, and only to the two OSV destinations
//! enumerated in DESIGN §18.
