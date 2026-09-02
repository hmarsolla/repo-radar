//! Rule packs (DESIGN §10, FR-2.6, FR-3.8).
//!
//! A shipped TOML pack is embedded with `include_str!`; a user pack in
//! `<config>/rules/*.toml` is merged by rule `id` (same id replaces, new id
//! appends). A malformed user pack is a startup [`crate::model::Warning`],
//! not a startup failure. `rule_pack_version` is the hash of the merged
//! pack and feeds the scan fingerprint (§6.5).
//!
//! Full loading/merging and the engines are **M3-1 … M3-3**. This stub
//! carries just enough for `CoreContext::new` to construct.

pub mod categories;
pub mod technologies;

use crate::error::CoreResult;
use crate::paths::Paths;

/// The shipped category rules, embedded at build time.
pub const SHIPPED_CATEGORIES_TOML: &str = include_str!("../../assets/categories.toml");
/// The shipped technology rules, embedded at build time.
pub const SHIPPED_TECHNOLOGIES_TOML: &str = include_str!("../../assets/technologies.toml");

/// The merged (shipped ⊕ user) rule packs plus the version hash that
/// participates in the scan fingerprint.
#[derive(Debug, Clone)]
pub struct RulePacks {
    /// `blake3` of the effective merged pack text (DESIGN §10.2).
    pub version: String,
    /// Warnings raised while loading a user pack (surfaced at startup).
    pub load_warnings: Vec<crate::model::Warning>,
}

impl RulePacks {
    /// Load the shipped packs, merge any user packs from `paths.rules_dir()`,
    /// and compute the version hash. **M3-1** fills in real parsing and
    /// merging; for now this hashes the shipped text so the fingerprint is
    /// stable and correct for the shipped-only case.
    pub fn load(_paths: &Paths) -> CoreResult<Self> {
        let mut hasher = blake3::Hasher::new();
        hasher.update(SHIPPED_CATEGORIES_TOML.as_bytes());
        hasher.update(SHIPPED_TECHNOLOGIES_TOML.as_bytes());
        Ok(Self {
            version: hasher.finalize().to_hex().to_string(),
            load_warnings: Vec::new(),
        })
    }
}
