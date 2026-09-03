//! Rule packs (DESIGN §10, FR-2.6, FR-3.8).
//!
//! A shipped TOML pack is embedded with `include_str!`; user packs in
//! `<config>/rules/*.toml` are merged by rule `id` (same id replaces, new id
//! appends). A malformed user pack is a startup [`crate::model::Warning`],
//! not a startup failure. `rule_pack_version` is the hash of the merged
//! pack and feeds the scan fingerprint (§6.5), so a rule edit correctly
//! invalidates every repo's cached classification.
//!
//! Signal kinds (DESIGN §10.1): `any_dependency`, `all_dependencies`,
//! `any_file` (glob), `any_language` (with optional `min_percentage`), and
//! `predicate` (a small closed set of named built-ins). Arbitrary
//! expressions are deliberately unsupported — a DSL needs a parser, error
//! messages, and a debugger; the closed set stays diagnosable.

pub mod categories;
pub mod signals;
pub mod technologies;

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::error::CoreResult;
use crate::model::{Category, WarningKind, WarningScope};
use crate::paths::Paths;

/// The shipped category rules, embedded at build time.
pub const SHIPPED_CATEGORIES_TOML: &str = include_str!("../../assets/categories.toml");
/// The shipped technology rules, embedded at build time.
pub const SHIPPED_TECHNOLOGIES_TOML: &str = include_str!("../../assets/technologies.toml");

// ---------------------------------------------------------------------------
// On-disk shape
// ---------------------------------------------------------------------------

/// One TOML rule-pack file: category `settings`, any number of `[[rule]]`
/// (category) entries, and any number of `[[tech]]` entries. Shipped packs
/// keep categories and technologies in separate files; a user pack may put
/// both in one — every `*.toml` under `<config>/rules/` is parsed the same
/// way.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPack {
    #[serde(default)]
    settings: Option<CategorySettings>,
    #[serde(default, rename = "rule")]
    rules: Vec<CategoryRule>,
    #[serde(default, rename = "tech")]
    techs: Vec<TechRule>,
}

/// Category-resolution tuning (DESIGN §10.3). A user pack's `[settings]`
/// replaces the shipped values wholesale.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CategorySettings {
    /// Top score below this → `Unknown` (FR-3.5).
    pub floor: f32,
    /// Frontend and Backend both at or above this, within `margin` → `Fullstack`.
    pub fullstack_threshold: f32,
    /// Maximum gap between the top two for the `Fullstack` special case.
    pub margin: f32,
}

impl Default for CategorySettings {
    fn default() -> Self {
        Self {
            floor: 3.0,
            fullstack_threshold: 4.0,
            margin: 2.0,
        }
    }
}

/// A number that deserializes from a TOML integer *or* float. TOML has no
/// implicit int→float coercion, so `weights = { Frontend = 5 }` would
/// otherwise fail against an `f32` field — and hand-written packs use bare
/// integers constantly.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Weight(pub f32);

impl<'de> Deserialize<'de> for Weight {
    fn deserialize<D>(d: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct V;
        impl serde::de::Visitor<'_> for V {
            type Value = f32;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a number")
            }
            fn visit_i64<E>(self, v: i64) -> Result<f32, E> {
                Ok(v as f32)
            }
            fn visit_u64<E>(self, v: u64) -> Result<f32, E> {
                Ok(v as f32)
            }
            fn visit_f64<E>(self, v: f64) -> Result<f32, E> {
                Ok(v as f32)
            }
        }
        d.deserialize_any(V).map(Weight)
    }
}

/// A minimum-percentage requirement on a language signal.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LanguageReq {
    pub language: String,
    #[serde(default)]
    pub min_percentage: Option<Weight>,
}

/// One category rule. It fires when *any one* of its populated signals
/// matches (signals are OR-ed); `all_dependencies` is the exception — every
/// listed name must be present. A firing rule adds each entry of `weights`
/// to that category's running total.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CategoryRule {
    pub id: String,
    #[serde(default)]
    pub weights: BTreeMap<Category, Weight>,
    #[serde(default)]
    pub any_dependency: Vec<String>,
    #[serde(default)]
    pub all_dependencies: Vec<String>,
    #[serde(default)]
    pub any_file: Vec<String>,
    #[serde(default)]
    pub any_language: Vec<LanguageReq>,
    /// A named built-in from the closed set in [`categories`].
    #[serde(default)]
    pub predicate: Option<String>,
}

/// One technology-detection rule (FR-2.3–2.5).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TechRule {
    pub id: String,
    pub name: String,
    /// `framework | tooling | package-manager | runtime`.
    pub kind: String,
    #[serde(default)]
    pub any_dependency: Vec<String>,
    #[serde(default)]
    pub any_file: Vec<String>,
}

// ---------------------------------------------------------------------------
// Merged, in-memory form
// ---------------------------------------------------------------------------

/// The merged (shipped ⊕ user) rule packs plus the version hash that
/// participates in the scan fingerprint.
#[derive(Debug, Clone)]
pub struct RulePacks {
    /// `blake3` of the effective merged pack text (DESIGN §10.2).
    pub version: String,
    pub settings: CategorySettings,
    /// Category rules in effect, merged by `id`, shipped order preserved with
    /// new user ids appended.
    pub categories: Vec<CategoryRule>,
    /// Technology rules in effect, merged by `id`.
    pub technologies: Vec<TechRule>,
    /// Warnings raised while loading a user pack (surfaced at startup).
    pub load_warnings: Vec<crate::model::Warning>,
}

impl RulePacks {
    /// Load the shipped packs, merge any user packs from `paths.rules_dir()`,
    /// and compute the version hash.
    ///
    /// The shipped packs are embedded and covered by tests here, so a parse
    /// failure on them is a bug, not a runtime condition — it panics. A user
    /// pack that fails to parse becomes a [`WarningKind::RulePackInvalid`]
    /// and is skipped; the app continues on the shipped pack (FR-3.8).
    pub fn load(paths: &Paths) -> CoreResult<Self> {
        let shipped_categories: RawPack = toml::from_str(SHIPPED_CATEGORIES_TOML)
            .expect("shipped categories.toml is malformed — this is a build bug");
        let shipped_techs: RawPack = toml::from_str(SHIPPED_TECHNOLOGIES_TOML)
            .expect("shipped technologies.toml is malformed — this is a build bug");

        let mut settings = shipped_categories.settings.clone().unwrap_or_default();
        let mut categories = shipped_categories.rules.clone();
        let mut technologies = shipped_techs.techs.clone();

        let mut hasher = blake3::Hasher::new();
        hasher.update(SHIPPED_CATEGORIES_TOML.as_bytes());
        hasher.update(SHIPPED_TECHNOLOGIES_TOML.as_bytes());

        let mut load_warnings = Vec::new();

        for (name, text) in read_user_packs(paths) {
            match toml::from_str::<RawPack>(&text) {
                Ok(pack) => {
                    hasher.update(b"\x1f");
                    hasher.update(name.as_bytes());
                    hasher.update(b"\x1e");
                    hasher.update(text.as_bytes());
                    if let Some(s) = pack.settings {
                        settings = s;
                    }
                    merge_by_id(&mut categories, pack.rules, |r| r.id.clone());
                    merge_by_id(&mut technologies, pack.techs, |t| t.id.clone());
                }
                Err(e) => {
                    load_warnings.push(crate::model::Warning::new(
                        WarningScope::Scan,
                        WarningKind::RulePackInvalid,
                        format!("ignored malformed rule pack {name}: {e}"),
                    ));
                }
            }
        }

        Ok(Self {
            version: hasher.finalize().to_hex().to_string(),
            settings,
            categories,
            technologies,
            load_warnings,
        })
    }
}

/// Merge `incoming` into `base` by a string key: an incoming entry whose key
/// already exists replaces it in place; a new key is appended. This keeps
/// the shipped ordering stable and makes a user override deterministic
/// regardless of how many packs touch the same id.
fn merge_by_id<T>(base: &mut Vec<T>, incoming: Vec<T>, key: impl Fn(&T) -> String) {
    for item in incoming {
        let k = key(&item);
        match base.iter_mut().find(|b| key(b) == k) {
            Some(slot) => *slot = item,
            None => base.push(item),
        }
    }
}

/// Every `*.toml` under `<config>/rules/`, as `(file_name, contents)` sorted
/// by name for a deterministic merge order. A directory that does not exist
/// or cannot be read yields nothing — that is the normal first-run state.
fn read_user_packs(paths: &Paths) -> Vec<(String, String)> {
    let dir = paths.rules_dir();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut packs: Vec<(String, String)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("<unknown>")
            .to_string();
        if let Ok(text) = std::fs::read_to_string(&path) {
            packs.push((name, text));
        }
    }
    packs.sort_by(|a, b| a.0.cmp(&b.0));
    packs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipped_packs_parse_and_hash_is_stable() {
        let a = RulePacks::load(&Paths::under(tempfile::tempdir().unwrap())).unwrap();
        let b = RulePacks::load(&Paths::under(tempfile::tempdir().unwrap())).unwrap();
        assert_eq!(
            a.version, b.version,
            "shipped-only hash must be deterministic"
        );
        assert!(a.load_warnings.is_empty());
        assert!(!a.categories.is_empty());
        assert!(!a.technologies.is_empty());
        // Every shipped category rule names at least one real category weight.
        for r in &a.categories {
            assert!(!r.weights.is_empty(), "rule {} has no weights", r.id);
        }
    }

    #[test]
    fn user_pack_replaces_by_id_and_appends_new_ids() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::under(&tmp);
        paths.ensure_dirs().unwrap();
        std::fs::write(
            paths.rules_dir().join("mine.toml"),
            r#"
[[rule]]
id = "react-frontend"
weights = { Frontend = 99 }
any_dependency = ["react"]

[[rule]]
id = "my-custom"
weights = { Backend = 3 }
any_file = ["serverless.yml"]

[[tech]]
id = "bun"
name = "Bun"
kind = "runtime"
any_file = ["bun.lockb"]
"#,
        )
        .unwrap();

        let base = RulePacks::load(&Paths::under(tempfile::tempdir().unwrap())).unwrap();
        let merged = RulePacks::load(&paths).unwrap();

        assert!(merged.load_warnings.is_empty());
        // Same count for the replaced id, plus one appended.
        assert_eq!(merged.categories.len(), base.categories.len() + 1);
        let react = merged
            .categories
            .iter()
            .find(|r| r.id == "react-frontend")
            .unwrap();
        assert_eq!(react.weights.get(&Category::Frontend), Some(&Weight(99.0)));
        assert!(merged.categories.iter().any(|r| r.id == "my-custom"));
        assert!(merged.technologies.iter().any(|t| t.id == "bun"));
        assert_ne!(merged.version, base.version, "a user pack changes the hash");
    }

    #[test]
    fn malformed_user_pack_is_a_warning_not_a_failure() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::under(&tmp);
        paths.ensure_dirs().unwrap();
        std::fs::write(
            paths.rules_dir().join("broken.toml"),
            "this is not = valid = toml",
        )
        .unwrap();

        let packs = RulePacks::load(&paths).unwrap();
        assert_eq!(packs.load_warnings.len(), 1);
        assert_eq!(packs.load_warnings[0].kind, WarningKind::RulePackInvalid);
        // Shipped rules still loaded.
        assert!(!packs.categories.is_empty());
    }
}
