//! Health scoring (FR-6, DESIGN §9).
//!
//! `score()` is a pure function with no IO: `(findings, hygiene inputs,
//! weights) -> HealthResult`. Key rules it must enforce (property-tested in
//! DESIGN §16.3):
//!
//! - Compromise is a **cap** at 39, not a subtraction — a good score
//!   elsewhere must not average away a backdoored package (FR-6.3).
//! - `×0.4` for `Confidence::Range`, `×0.5` for dev/build scope (FR-6.5/6.6).
//! - Per-`(ecosystem, name)` diminishing returns: `1 / (1 + index)`.
//! - Hygiene deductions floor the score at 60 — they signal neglect, not
//!   danger (FR-6.8).
//!
//! Implemented in **M2-17**.
