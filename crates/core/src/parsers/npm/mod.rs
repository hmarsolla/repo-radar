//! npm / pnpm / yarn parsers (DESIGN §7.3, FR-4).
//!
//! - `package-lock.json` v1 (nested `dependencies`) and v2/v3 (`packages` map)
//! - `pnpm-lock.yaml` (v6 / v9; best-effort v5)
//! - `yarn.lock` v1 (hand-written); berry (v2+) falls back to `package.json`
//!   at `Range` confidence with a warning (PRD R2, DESIGN D7)
//! - `package.json` as the manifest fallback

use serde::Deserialize;

use crate::model::{Confidence, Dependency, Ecosystem, RelPath, Scope};
use crate::parsers::{
    normalize_package_name, LockfileParser, ManifestKind, ParseError, ParseOk, SiblingFiles,
};

const NPM: Ecosystem = Ecosystem::Npm;

fn dep(
    raw_name: &str,
    version: &str,
    scope: Scope,
    is_direct: bool,
    conf: Confidence,
) -> Dependency {
    Dependency {
        ecosystem: NPM,
        name: normalize_package_name(NPM, raw_name),
        raw_name: raw_name.to_string(),
        version: version.to_string(),
        confidence: conf,
        scope,
        is_direct,
        manifest_path: RelPath::new(""), // stamped by the registry
    }
}

/// The declared dependency names of a `package.json`, by scope — the source
/// of truth for directness (DESIGN §7.3).
#[derive(Debug, Default)]
struct Declared {
    runtime: Vec<String>,
    dev: Vec<String>,
    optional: Vec<String>,
    peer: Vec<String>,
}

impl Declared {
    fn from_package_json(content: &str) -> Result<Self, ParseError> {
        #[derive(Deserialize)]
        struct Pj {
            #[serde(default)]
            dependencies: std::collections::BTreeMap<String, String>,
            #[serde(default, rename = "devDependencies")]
            dev_dependencies: std::collections::BTreeMap<String, String>,
            #[serde(default, rename = "optionalDependencies")]
            optional_dependencies: std::collections::BTreeMap<String, String>,
            #[serde(default, rename = "peerDependencies")]
            peer_dependencies: std::collections::BTreeMap<String, String>,
        }
        let pj: Pj = serde_json::from_str(content)
            .map_err(|e| ParseError::malformed("package.json", e.to_string()))?;
        Ok(Self {
            runtime: pj.dependencies.into_keys().collect(),
            dev: pj.dev_dependencies.into_keys().collect(),
            optional: pj.optional_dependencies.into_keys().collect(),
            peer: pj.peer_dependencies.into_keys().collect(),
        })
    }

    fn scope_of(&self, name: &str) -> Option<Scope> {
        let n = normalize_package_name(NPM, name);
        let hit = |v: &[String]| v.iter().any(|x| normalize_package_name(NPM, x) == n);
        if hit(&self.runtime) {
            Some(Scope::Runtime)
        } else if hit(&self.dev) {
            Some(Scope::Dev)
        } else if hit(&self.optional) {
            Some(Scope::Optional)
        } else if hit(&self.peer) {
            Some(Scope::Peer)
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------
// package-lock.json
// ---------------------------------------------------------------------------

pub struct PackageLockParser;

impl LockfileParser for PackageLockParser {
    fn ecosystem(&self) -> Ecosystem {
        NPM
    }
    fn kind(&self) -> ManifestKind {
        ManifestKind::Lockfile
    }
    fn matches_file(&self, file_name: &str) -> bool {
        file_name == "package-lock.json" || file_name == "npm-shrinkwrap.json"
    }

    fn parse(
        &self,
        primary: &str,
        _path: &RelPath,
        sibling: &SiblingFiles<'_>,
    ) -> Result<ParseOk, ParseError> {
        #[derive(Deserialize)]
        struct Lock {
            #[serde(default, rename = "lockfileVersion")]
            #[allow(dead_code)]
            lockfile_version: u32,
            #[serde(default)]
            packages: std::collections::BTreeMap<String, PkgEntry>,
            #[serde(default)]
            dependencies: std::collections::BTreeMap<String, V1Entry>,
        }
        #[derive(Deserialize)]
        struct PkgEntry {
            #[serde(default)]
            version: Option<String>,
            #[serde(default)]
            dev: bool,
            #[serde(default)]
            optional: bool,
            #[serde(default)]
            peer: bool,
            #[serde(default)]
            link: bool,
            #[serde(default)]
            dependencies: std::collections::BTreeMap<String, String>,
            #[serde(default, rename = "devDependencies")]
            dev_dependencies: std::collections::BTreeMap<String, String>,
            #[serde(default, rename = "optionalDependencies")]
            optional_dependencies: std::collections::BTreeMap<String, String>,
            #[serde(default, rename = "peerDependencies")]
            peer_dependencies: std::collections::BTreeMap<String, String>,
        }

        let lock: Lock = serde_json::from_str(primary)
            .map_err(|e| ParseError::malformed("package-lock.json", e.to_string()))?;

        let mut out = Vec::new();

        if !lock.packages.is_empty() {
            // v2 / v3 — the `packages` map. Directness + scope from the `""`
            // root entry's dependency keys.
            let root = lock.packages.get("");
            let declared = root.map(|r| Declared {
                runtime: r.dependencies.keys().cloned().collect(),
                dev: r.dev_dependencies.keys().cloned().collect(),
                optional: r.optional_dependencies.keys().cloned().collect(),
                peer: r.peer_dependencies.keys().cloned().collect(),
            });

            for (key, entry) in &lock.packages {
                if key.is_empty() || entry.link {
                    continue;
                }
                let Some(name) = key.rsplit("node_modules/").next().filter(|n| !n.is_empty())
                else {
                    continue;
                };
                let Some(version) = entry.version.as_deref() else {
                    continue;
                };
                let declared_scope = declared.as_ref().and_then(|d| d.scope_of(name));
                let is_direct = declared_scope.is_some();
                let scope = declared_scope.unwrap_or(if entry.optional {
                    Scope::Optional
                } else if entry.peer {
                    Scope::Peer
                } else if entry.dev {
                    Scope::Dev
                } else {
                    Scope::Runtime
                });
                out.push(dep(name, version, scope, is_direct, Confidence::Exact));
            }
        } else if !lock.dependencies.is_empty() {
            // v1 — nested `dependencies`. Directness/scope from package.json.
            let declared = sibling("package.json")
                .and_then(|c| Declared::from_package_json(c).ok())
                .unwrap_or_default();
            flatten_v1(&lock.dependencies, &declared, true, &mut out);
        }

        Ok(out.into())
    }
}

/// A `package-lock.json` v1 nested-`dependencies` node.
#[derive(Deserialize)]
struct V1Entry {
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    dev: bool,
    #[serde(default)]
    optional: bool,
    #[serde(default)]
    dependencies: std::collections::BTreeMap<String, V1Entry>,
}

fn flatten_v1(
    tree: &std::collections::BTreeMap<String, V1Entry>,
    declared: &Declared,
    at_root: bool,
    out: &mut Vec<Dependency>,
) {
    for (name, entry) in tree {
        if let Some(version) = entry.version.as_deref() {
            let declared_scope = declared.scope_of(name);
            let is_direct = at_root && declared_scope.is_some();
            let scope = declared_scope.unwrap_or(if entry.dev {
                Scope::Dev
            } else if entry.optional {
                Scope::Optional
            } else {
                Scope::Runtime
            });
            out.push(dep(name, version, scope, is_direct, Confidence::Exact));
        }
        flatten_v1(&entry.dependencies, declared, false, out);
    }
}

// ---------------------------------------------------------------------------
// package.json (manifest fallback)
// ---------------------------------------------------------------------------

pub struct PackageJsonParser;

impl LockfileParser for PackageJsonParser {
    fn ecosystem(&self) -> Ecosystem {
        NPM
    }
    fn kind(&self) -> ManifestKind {
        ManifestKind::Manifest
    }
    fn matches_file(&self, file_name: &str) -> bool {
        file_name == "package.json"
    }

    fn parse(
        &self,
        primary: &str,
        _path: &RelPath,
        _sibling: &SiblingFiles<'_>,
    ) -> Result<ParseOk, ParseError> {
        let declared = Declared::from_package_json(primary)?;
        Ok(manifest_deps_from_declared(&declared).into())
    }
}

fn manifest_deps_from_declared(declared: &Declared) -> Vec<Dependency> {
    let mut out = Vec::new();
    let mut push = |names: &[String], scope: Scope| {
        for n in names {
            out.push(dep(n, "*", scope, true, Confidence::Range));
        }
    };
    push(&declared.runtime, Scope::Runtime);
    push(&declared.dev, Scope::Dev);
    push(&declared.optional, Scope::Optional);
    push(&declared.peer, Scope::Peer);
    out
}

// ---------------------------------------------------------------------------
// pnpm-lock.yaml
// ---------------------------------------------------------------------------

pub struct PnpmLockParser;

impl LockfileParser for PnpmLockParser {
    fn ecosystem(&self) -> Ecosystem {
        NPM
    }
    fn kind(&self) -> ManifestKind {
        ManifestKind::Lockfile
    }
    fn matches_file(&self, file_name: &str) -> bool {
        file_name == "pnpm-lock.yaml"
    }

    fn parse(
        &self,
        primary: &str,
        _path: &RelPath,
        _sibling: &SiblingFiles<'_>,
    ) -> Result<ParseOk, ParseError> {
        let doc: serde_yaml_ng::Value = serde_yaml_ng::from_str(primary)
            .map_err(|e| ParseError::malformed("pnpm-lock.yaml", e.to_string()))?;

        let mut out = Vec::new();

        // Direct deps: `importers.<name>.{dependencies,devDependencies,...}`.
        // Older lockfiles put these at the top level instead of under
        // `importers`.
        let importers = doc.get("importers").and_then(|v| v.as_mapping());
        let direct_sources: Vec<&serde_yaml_ng::Value> = match importers {
            Some(map) => map.values().collect(),
            None => vec![&doc],
        };
        let mut declared = Declared::default();
        for imp in direct_sources {
            collect_pnpm_declared(imp, "dependencies", &mut declared.runtime);
            collect_pnpm_declared(imp, "devDependencies", &mut declared.dev);
            collect_pnpm_declared(imp, "optionalDependencies", &mut declared.optional);
            collect_pnpm_declared(imp, "peerDependencies", &mut declared.peer);
        }

        // Resolved set: `packages` keyed `/foo@1.2.3`, `/@scope/foo@1.2.3`,
        // or (v9) `foo@1.2.3` / `@scope/foo@1.2.3`.
        if let Some(pkgs) = doc.get("packages").and_then(|v| v.as_mapping()) {
            for key in pkgs.keys() {
                let Some(k) = key.as_str() else { continue };
                let k = k.strip_prefix('/').unwrap_or(k);
                let Some((name, version)) = split_pnpm_key(k) else {
                    continue;
                };
                let declared_scope = declared.scope_of(name);
                let is_direct = declared_scope.is_some();
                let scope = declared_scope.unwrap_or(Scope::Runtime);
                out.push(dep(name, version, scope, is_direct, Confidence::Exact));
            }
        }

        Ok(out.into())
    }
}

fn collect_pnpm_declared(importer: &serde_yaml_ng::Value, key: &str, into: &mut Vec<String>) {
    if let Some(map) = importer.get(key).and_then(|v| v.as_mapping()) {
        for k in map.keys() {
            if let Some(name) = k.as_str() {
                into.push(name.to_string());
            }
        }
    }
}

/// Split a pnpm package key on the **last** `@` so scoped names parse
/// (DESIGN §7.3). Trailing `(peer)` suffixes and `_` build tags are dropped.
fn split_pnpm_key(key: &str) -> Option<(&str, &str)> {
    let core = key.split('(').next().unwrap_or(key);
    let core = core.split('_').next().unwrap_or(core);
    let at = core.rfind('@')?;
    if at == 0 {
        return None; // `@scope` with no version
    }
    let (name, version) = core.split_at(at);
    Some((name, &version[1..]))
}

// ---------------------------------------------------------------------------
// yarn.lock
// ---------------------------------------------------------------------------

pub struct YarnLockParser;

impl LockfileParser for YarnLockParser {
    fn ecosystem(&self) -> Ecosystem {
        NPM
    }
    fn kind(&self) -> ManifestKind {
        ManifestKind::Lockfile
    }
    fn matches_file(&self, file_name: &str) -> bool {
        file_name == "yarn.lock"
    }

    fn parse(
        &self,
        primary: &str,
        _path: &RelPath,
        sibling: &SiblingFiles<'_>,
    ) -> Result<ParseOk, ParseError> {
        // Berry (v2+) — `__metadata:` present, YAML-ish. Per PRD R2 / D7:
        // do not ship a half-correct berry parser. Fall back to the manifest
        // at `Range` confidence with a visible notice.
        if primary.contains("\n__metadata:") || primary.starts_with("__metadata:") {
            let declared = sibling("package.json")
                .map(Declared::from_package_json)
                .transpose()?
                .unwrap_or_default();
            let mut deps = manifest_deps_from_declared(&declared);
            for d in &mut deps {
                d.confidence = Confidence::Range;
            }
            if deps.is_empty() {
                return Err(ParseError::Unsupported {
                    format: "yarn.lock (berry)",
                    detail: "yarn berry lockfile; no package.json to fall back to".into(),
                });
            }
            return Ok(ParseOk {
                deps,
                notes: vec![
                    "yarn berry (v2+) lockfile is not parsed; dependency versions \
                     come from package.json at range confidence"
                        .to_string(),
                ],
            });
        }

        let declared = sibling("package.json")
            .and_then(|c| Declared::from_package_json(c).ok())
            .unwrap_or_default();

        Ok(parse_yarn_v1(primary, &declared).into())
    }
}

/// Parse a yarn v1 lockfile. Blocks are separated by blank lines; a block
/// header is one or more comma-separated `name@range` specs ending in `:`,
/// followed by indented `key value` lines including `version "X"`.
fn parse_yarn_v1(content: &str, declared: &Declared) -> Vec<Dependency> {
    let mut out = Vec::new();
    let mut header_specs: Vec<String> = Vec::new();
    let mut version: Option<String> = None;

    let flush =
        |specs: &mut Vec<String>, version: &mut Option<String>, out: &mut Vec<Dependency>| {
            if let Some(ver) = version.take() {
                // All specs in a block resolve to the same package; take the name
                // from the first.
                if let Some(first) = specs.first() {
                    if let Some(name) = yarn_spec_name(first) {
                        let declared_scope = declared.scope_of(name);
                        out.push(dep(
                            name,
                            &ver,
                            declared_scope.unwrap_or(Scope::Runtime),
                            declared_scope.is_some(),
                            Confidence::Exact,
                        ));
                    }
                }
            }
            specs.clear();
        };

    for line in content.lines() {
        if line.trim().is_empty() || line.starts_with('#') {
            flush(&mut header_specs, &mut version, &mut out);
            continue;
        }
        if !line.starts_with([' ', '\t']) && line.trim_end().ends_with(':') {
            // New block header.
            flush(&mut header_specs, &mut version, &mut out);
            let header = line.trim_end().trim_end_matches(':');
            for raw in header.split(',') {
                let spec = raw.trim().trim_matches('"');
                if !spec.is_empty() {
                    header_specs.push(spec.to_string());
                }
            }
        } else {
            let t = line.trim();
            if let Some(rest) = t.strip_prefix("version ") {
                version = Some(rest.trim().trim_matches('"').to_string());
            }
        }
    }
    flush(&mut header_specs, &mut version, &mut out);
    out
}

/// `left-pad@^1.0.0` → `left-pad`; `@scope/pkg@^1.0.0` → `@scope/pkg`.
fn yarn_spec_name(spec: &str) -> Option<&str> {
    let at = spec.rfind('@')?;
    if at == 0 {
        return None;
    }
    Some(&spec[..at])
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

    fn find<'a>(deps: &'a [Dependency], name: &str) -> &'a Dependency {
        deps.iter()
            .find(|d| d.name == name)
            .unwrap_or_else(|| panic!("no dep {name}"))
    }

    const PJ: &str = r#"{
        "dependencies": { "left-pad": "^1.3.0", "@scope/pkg": "^2.0.0" },
        "devDependencies": { "jest": "^29.0.0" }
    }"#;

    #[test]
    fn package_lock_v3_packages_map() {
        let lock = r#"{
          "lockfileVersion": 3,
          "packages": {
            "": { "dependencies": { "left-pad": "^1.3.0", "@scope/pkg": "^2.0.0" },
                  "devDependencies": { "jest": "^29.0.0" } },
            "node_modules/left-pad": { "version": "1.3.0" },
            "node_modules/@scope/pkg": { "version": "2.1.4" },
            "node_modules/jest": { "version": "29.7.0", "dev": true },
            "node_modules/transitive-dep": { "version": "0.5.0" }
          }
        }"#;
        let reg = ParserRegistry::builtin();
        let out = reg.parse_dir(
            "",
            &files(&[("package.json", PJ), ("package-lock.json", lock)]),
        );
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);

        assert_eq!(out.deps.len(), 4);
        let lp = find(&out.deps, "left-pad");
        assert_eq!(lp.version, "1.3.0");
        assert_eq!(lp.confidence, Confidence::Exact);
        assert!(lp.is_direct);
        assert_eq!(lp.scope, Scope::Runtime);

        let scoped = find(&out.deps, "@scope/pkg");
        assert_eq!(scoped.version, "2.1.4");
        assert!(scoped.is_direct);

        let jest = find(&out.deps, "jest");
        assert_eq!(jest.scope, Scope::Dev);
        assert!(jest.is_direct);

        let trans = find(&out.deps, "transitive-dep");
        assert!(!trans.is_direct, "tree position, not a package.json key");
        assert_eq!(trans.scope, Scope::Runtime);
    }

    #[test]
    fn package_lock_v1_nested_tree_with_manifest_for_directness() {
        let lock = r#"{
          "lockfileVersion": 1,
          "dependencies": {
            "left-pad": { "version": "1.3.0", "dependencies": {
              "inner": { "version": "9.9.9" }
            }},
            "jest": { "version": "29.7.0", "dev": true }
          }
        }"#;
        let reg = ParserRegistry::builtin();
        let out = reg.parse_dir(
            "",
            &files(&[("package.json", PJ), ("package-lock.json", lock)]),
        );
        assert_eq!(out.deps.len(), 3);
        assert!(find(&out.deps, "left-pad").is_direct);
        assert!(!find(&out.deps, "inner").is_direct);
        assert_eq!(find(&out.deps, "jest").scope, Scope::Dev);
        assert!(out.deps.iter().all(|d| d.confidence == Confidence::Exact));
    }

    #[test]
    fn pnpm_v9_keys_split_on_last_at() {
        let lock = r#"
lockfileVersion: '9.0'
importers:
  .:
    dependencies:
      left-pad:
        specifier: ^1.3.0
        version: 1.3.0
      '@scope/pkg':
        specifier: ^2.0.0
        version: 2.1.4
    devDependencies:
      jest:
        specifier: ^29.0.0
        version: 29.7.0
packages:
  left-pad@1.3.0: {}
  '@scope/pkg@2.1.4': {}
  jest@29.7.0: {}
  '@babel/core@7.24.0': {}
"#;
        let reg = ParserRegistry::builtin();
        let out = reg.parse_dir("", &files(&[("pnpm-lock.yaml", lock)]));
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
        assert_eq!(out.deps.len(), 4);
        assert_eq!(find(&out.deps, "@scope/pkg").version, "2.1.4");
        assert_eq!(find(&out.deps, "@babel/core").version, "7.24.0");
        assert!(find(&out.deps, "left-pad").is_direct);
        assert!(!find(&out.deps, "@babel/core").is_direct);
        assert_eq!(find(&out.deps, "jest").scope, Scope::Dev);
    }

    #[test]
    fn yarn_v1_blocks() {
        let lock = r#"# THIS IS AN AUTOGENERATED FILE.

"left-pad@^1.3.0", "left-pad@~1.3.1":
  version "1.3.0"
  resolved "https://registry.yarnpkg.com/left-pad/-/left-pad-1.3.0.tgz#..."

"@scope/pkg@^2.0.0":
  version "2.1.4"
  resolved "https://registry.yarnpkg.com/@scope/pkg/-/pkg-2.1.4.tgz#..."
  dependencies:
    inner "^9.0.0"

inner@^9.0.0:
  version "9.9.9"
  resolved "..."
"#;
        let reg = ParserRegistry::builtin();
        let out = reg.parse_dir("", &files(&[("package.json", PJ), ("yarn.lock", lock)]));
        assert_eq!(out.deps.len(), 3, "{:?}", out.deps);
        assert_eq!(find(&out.deps, "left-pad").version, "1.3.0");
        assert_eq!(find(&out.deps, "@scope/pkg").version, "2.1.4");
        assert!(find(&out.deps, "left-pad").is_direct);
        assert!(!find(&out.deps, "inner").is_direct);
        assert!(out.deps.iter().all(|d| d.confidence == Confidence::Exact));
    }

    #[test]
    fn yarn_berry_falls_back_to_manifest_with_a_notice() {
        let berry = "__metadata:\n  version: 8\n  cacheKey: 10\n\n\"left-pad@npm:^1.3.0\":\n  version: 1.3.0\n";
        let reg = ParserRegistry::builtin();
        let out = reg.parse_dir("", &files(&[("package.json", PJ), ("yarn.lock", berry)]));
        assert!(!out.deps.is_empty());
        assert!(
            out.deps.iter().all(|d| d.confidence == Confidence::Range),
            "berry => range"
        );
        assert!(
            out.warnings.iter().any(|(_, m)| m.contains("berry")),
            "a visible notice is emitted: {:?}",
            out.warnings
        );
    }
}
