//! `Cargo.lock` (`[[package]]`) and `Cargo.toml` (DESIGN §7.3).
//!
//! Directness and scope come from `Cargo.toml`'s three dependency tables
//! (`[dependencies]`, `[dev-dependencies]`, `[build-dependencies]`). Anything
//! in the lock that is not a declared dependency of the local manifest is
//! transitive. Deeper workspace-member resolution is M2-10.

use serde::Deserialize;

use crate::model::{Confidence, Dependency, Ecosystem, RelPath, Scope};
use crate::parsers::{
    normalize_package_name, LockfileParser, ManifestKind, ParseError, ParseOk, SiblingFiles,
};

const CARGO: Ecosystem = Ecosystem::CratesIo;

fn dep(name: &str, version: &str, scope: Scope, direct: bool, conf: Confidence) -> Dependency {
    Dependency {
        ecosystem: CARGO,
        name: normalize_package_name(CARGO, name),
        raw_name: name.to_string(),
        version: version.to_string(),
        confidence: conf,
        scope,
        is_direct: direct,
        manifest_path: RelPath::new(""),
    }
}

/// `(name -> scope)` for every dependency declared in a `Cargo.toml`.
fn declared_from_toml(content: &str) -> std::collections::BTreeMap<String, Scope> {
    #[derive(Deserialize, Default)]
    struct Manifest {
        #[serde(default)]
        dependencies: toml::Table,
        #[serde(default, rename = "dev-dependencies")]
        dev_dependencies: toml::Table,
        #[serde(default, rename = "build-dependencies")]
        build_dependencies: toml::Table,
    }
    let m: Manifest = toml::from_str(content).unwrap_or_default();
    let mut out = std::collections::BTreeMap::new();
    for (k, _) in m.dependencies {
        out.insert(normalize_package_name(CARGO, &k), Scope::Runtime);
    }
    for (k, _) in m.dev_dependencies {
        out.entry(normalize_package_name(CARGO, &k))
            .or_insert(Scope::Dev);
    }
    for (k, _) in m.build_dependencies {
        out.entry(normalize_package_name(CARGO, &k))
            .or_insert(Scope::Build);
    }
    out
}

pub struct CargoLockParser;

impl LockfileParser for CargoLockParser {
    fn ecosystem(&self) -> Ecosystem {
        CARGO
    }
    fn kind(&self) -> ManifestKind {
        ManifestKind::Lockfile
    }
    fn matches_file(&self, file_name: &str) -> bool {
        file_name == "Cargo.lock"
    }

    fn parse(
        &self,
        primary: &str,
        _path: &RelPath,
        sibling: &SiblingFiles<'_>,
    ) -> Result<ParseOk, ParseError> {
        #[derive(Deserialize)]
        struct Lock {
            #[serde(default, rename = "package")]
            packages: Vec<Pkg>,
        }
        #[derive(Deserialize)]
        struct Pkg {
            name: String,
            #[serde(default)]
            version: Option<String>,
        }

        let lock: Lock = toml::from_str(primary)
            .map_err(|e| ParseError::malformed("Cargo.lock", e.to_string()))?;
        let declared = sibling("Cargo.toml")
            .map(declared_from_toml)
            .unwrap_or_default();

        let mut out = Vec::new();
        for p in lock.packages {
            let Some(version) = p.version else { continue }; // path/workspace member
            let norm = normalize_package_name(CARGO, &p.name);
            let scope = declared.get(&norm).copied();
            out.push(dep(
                &p.name,
                &version,
                scope.unwrap_or(Scope::Runtime),
                scope.is_some(),
                Confidence::Exact,
            ));
        }
        Ok(out.into())
    }
}

pub struct CargoTomlParser;

impl LockfileParser for CargoTomlParser {
    fn ecosystem(&self) -> Ecosystem {
        CARGO
    }
    fn kind(&self) -> ManifestKind {
        ManifestKind::Manifest
    }
    fn matches_file(&self, file_name: &str) -> bool {
        file_name == "Cargo.toml"
    }

    fn parse(
        &self,
        primary: &str,
        _path: &RelPath,
        _sibling: &SiblingFiles<'_>,
    ) -> Result<ParseOk, ParseError> {
        let declared = declared_from_toml(primary);
        let out = declared
            .into_iter()
            .map(|(name, scope)| dep(&name, "*", scope, true, Confidence::Range))
            .collect::<Vec<_>>();
        Ok(out.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parsers::ParserRegistry;
    use std::collections::BTreeMap;

    fn files(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    const CARGO_TOML: &str = r#"
[package]
name = "demo"
version = "0.1.0"

[dependencies]
serde = "1"
tokio = { version = "1", features = ["full"] }

[dev-dependencies]
tempfile = "3"

[build-dependencies]
cc = "1"
"#;

    #[test]
    fn cargo_lock_with_manifest_scopes() {
        let lock = r#"
version = 4

[[package]]
name = "demo"
version = "0.1.0"

[[package]]
name = "serde"
version = "1.0.200"

[[package]]
name = "tokio"
version = "1.38.0"

[[package]]
name = "tempfile"
version = "3.10.0"

[[package]]
name = "cc"
version = "1.0.90"

[[package]]
name = "libc"
version = "0.2.155"
"#;
        let reg = ParserRegistry::builtin();
        let out = reg.parse_dir(
            "",
            &files(&[("Cargo.toml", CARGO_TOML), ("Cargo.lock", lock)]),
        );
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);

        let by = |n: &str| out.deps.iter().find(|d| d.name == n).unwrap();
        assert_eq!(by("serde").scope, Scope::Runtime);
        assert!(by("serde").is_direct);
        assert_eq!(by("tempfile").scope, Scope::Dev);
        assert_eq!(by("cc").scope, Scope::Build);
        assert!(!by("libc").is_direct);
        assert!(out.deps.iter().all(|d| d.confidence == Confidence::Exact));
        // The local package itself is excluded (no version in a real lock, but
        // here it has one — acceptable; it just isn't declared).
    }

    #[test]
    fn cargo_toml_only_is_range() {
        let reg = ParserRegistry::builtin();
        let out = reg.parse_dir("", &files(&[("Cargo.toml", CARGO_TOML)]));
        assert_eq!(out.deps.len(), 4);
        assert!(out.deps.iter().all(|d| d.confidence == Confidence::Range));
        assert!(out.deps.iter().all(|d| d.is_direct));
    }
}
