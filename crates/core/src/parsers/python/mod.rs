//! Python parsers (DESIGN §7.3, FR-4.9).
//!
//! - `poetry.lock`, `uv.lock` — TOML `[[package]]`, exact versions
//! - `Pipfile.lock` — JSON `default` / `develop`
//! - `requirements.txt` — **`Exact` only if every line is `==`-pinned**; a
//!   mixed file is `Range` throughout (FR-4.9)
//! - `pyproject.toml` — PEP 621 `[project]` or `[tool.poetry]`

use serde::Deserialize;

use crate::model::{Confidence, Dependency, Ecosystem, RelPath, Scope};
use crate::parsers::{
    normalize_package_name, LockfileParser, ManifestKind, ParseError, ParseOk, SiblingFiles,
};

const PY: Ecosystem = Ecosystem::PyPI;

fn dep(name: &str, version: &str, scope: Scope, direct: bool, conf: Confidence) -> Dependency {
    Dependency {
        ecosystem: PY,
        name: normalize_package_name(PY, name),
        raw_name: name.to_string(),
        version: version.to_string(),
        confidence: conf,
        scope,
        is_direct: direct,
        manifest_path: RelPath::new(""),
    }
}

/// Declared top-level dependency names from a `pyproject.toml`, PEP 621 or
/// poetry style.
fn declared_from_pyproject(content: &str) -> std::collections::BTreeSet<String> {
    let mut out = std::collections::BTreeSet::new();
    let Ok(doc) = content.parse::<toml::Table>() else {
        return out;
    };

    // PEP 621: [project] dependencies = ["flask>=2", ...]
    if let Some(deps) = doc
        .get("project")
        .and_then(|p| p.get("dependencies"))
        .and_then(|d| d.as_array())
    {
        for v in deps {
            if let Some(s) = v.as_str() {
                if let Some(name) = pep508_name(s) {
                    out.insert(normalize_package_name(PY, name));
                }
            }
        }
    }
    // PEP 621 optional-dependencies
    if let Some(groups) = doc
        .get("project")
        .and_then(|p| p.get("optional-dependencies"))
        .and_then(|d| d.as_table())
    {
        for (_g, arr) in groups {
            for v in arr.as_array().into_iter().flatten() {
                if let Some(name) = v.as_str().and_then(pep508_name) {
                    out.insert(normalize_package_name(PY, name));
                }
            }
        }
    }
    // poetry: [tool.poetry.dependencies] flask = "^2"
    if let Some(tbl) = doc
        .get("tool")
        .and_then(|t| t.get("poetry"))
        .and_then(|p| p.get("dependencies"))
        .and_then(|d| d.as_table())
    {
        for k in tbl.keys() {
            if k.eq_ignore_ascii_case("python") {
                continue;
            }
            out.insert(normalize_package_name(PY, k));
        }
    }
    out
}

/// Extract the package name from a PEP 508 requirement string
/// (`flask[async] >= 2.0 ; python_version >= "3.8"` → `flask`).
fn pep508_name(s: &str) -> Option<&str> {
    let s = s.trim();
    let end = s
        .find(|c: char| c.is_whitespace() || "[<>=!~;(".contains(c))
        .unwrap_or(s.len());
    let name = &s[..end];
    (!name.is_empty()).then_some(name)
}

// ---------------------------------------------------------------------------
// poetry.lock / uv.lock  (identical enough to share)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct TomlLock {
    #[serde(default, rename = "package")]
    packages: Vec<TomlPkg>,
}
#[derive(Deserialize)]
struct TomlPkg {
    name: String,
    #[serde(default)]
    version: Option<String>,
    /// poetry: "main" | "dev"; uv omits it.
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    optional: Option<bool>,
}

fn parse_toml_lock(
    format: &'static str,
    content: &str,
    declared: &std::collections::BTreeSet<String>,
) -> Result<Vec<Dependency>, ParseError> {
    let lock: TomlLock =
        toml::from_str(content).map_err(|e| ParseError::malformed(format, e.to_string()))?;
    let mut out = Vec::new();
    for p in lock.packages {
        let Some(version) = p.version else { continue };
        let norm = normalize_package_name(PY, &p.name);
        let direct = declared.contains(&norm);
        let scope = if p.category.as_deref() == Some("dev") {
            Scope::Dev
        } else if p.optional == Some(true) {
            Scope::Optional
        } else {
            Scope::Runtime
        };
        out.push(dep(&p.name, &version, scope, direct, Confidence::Exact));
    }
    Ok(out)
}

pub struct PoetryLockParser;
impl LockfileParser for PoetryLockParser {
    fn ecosystem(&self) -> Ecosystem {
        PY
    }
    fn kind(&self) -> ManifestKind {
        ManifestKind::Lockfile
    }
    fn matches_file(&self, f: &str) -> bool {
        f == "poetry.lock"
    }
    fn parse(
        &self,
        primary: &str,
        _p: &RelPath,
        sibling: &SiblingFiles<'_>,
    ) -> Result<ParseOk, ParseError> {
        let declared = sibling("pyproject.toml")
            .map(declared_from_pyproject)
            .unwrap_or_default();
        Ok(parse_toml_lock("poetry.lock", primary, &declared)?.into())
    }
}

pub struct UvLockParser;
impl LockfileParser for UvLockParser {
    fn ecosystem(&self) -> Ecosystem {
        PY
    }
    fn kind(&self) -> ManifestKind {
        ManifestKind::Lockfile
    }
    fn matches_file(&self, f: &str) -> bool {
        f == "uv.lock"
    }
    fn parse(
        &self,
        primary: &str,
        _p: &RelPath,
        sibling: &SiblingFiles<'_>,
    ) -> Result<ParseOk, ParseError> {
        let declared = sibling("pyproject.toml")
            .map(declared_from_pyproject)
            .unwrap_or_default();
        Ok(parse_toml_lock("uv.lock", primary, &declared)?.into())
    }
}

// ---------------------------------------------------------------------------
// Pipfile.lock
// ---------------------------------------------------------------------------

pub struct PipfileLockParser;
impl LockfileParser for PipfileLockParser {
    fn ecosystem(&self) -> Ecosystem {
        PY
    }
    fn kind(&self) -> ManifestKind {
        ManifestKind::Lockfile
    }
    fn matches_file(&self, f: &str) -> bool {
        f == "Pipfile.lock"
    }
    fn parse(
        &self,
        primary: &str,
        _p: &RelPath,
        _s: &SiblingFiles<'_>,
    ) -> Result<ParseOk, ParseError> {
        #[derive(Deserialize)]
        struct Lock {
            #[serde(default)]
            default: std::collections::BTreeMap<String, Entry>,
            #[serde(default)]
            develop: std::collections::BTreeMap<String, Entry>,
        }
        #[derive(Deserialize)]
        struct Entry {
            #[serde(default)]
            version: Option<String>,
        }
        let lock: Lock = serde_json::from_str(primary)
            .map_err(|e| ParseError::malformed("Pipfile.lock", e.to_string()))?;

        let mut out = Vec::new();
        let mut take = |m: std::collections::BTreeMap<String, Entry>, scope: Scope| {
            for (name, e) in m {
                let version = e
                    .version
                    .unwrap_or_default()
                    .trim_start_matches("==")
                    .to_string();
                if version.is_empty() {
                    continue;
                }
                // Pipfile.lock does not record which requirements are direct.
                out.push(dep(&name, &version, scope, true, Confidence::Exact));
            }
        };
        take(lock.default, Scope::Runtime);
        take(lock.develop, Scope::Dev);
        Ok(out.into())
    }
}

// ---------------------------------------------------------------------------
// requirements.txt
// ---------------------------------------------------------------------------

pub struct RequirementsTxtParser;
impl LockfileParser for RequirementsTxtParser {
    fn ecosystem(&self) -> Ecosystem {
        PY
    }
    /// Confidence is decided per file, not per kind — see `parse`.
    fn kind(&self) -> ManifestKind {
        ManifestKind::Lockfile
    }
    fn matches_file(&self, f: &str) -> bool {
        f == "requirements.txt" || (f.starts_with("requirements") && f.ends_with(".txt"))
    }
    fn parse(
        &self,
        primary: &str,
        _p: &RelPath,
        _s: &SiblingFiles<'_>,
    ) -> Result<ParseOk, ParseError> {
        struct Req<'a> {
            name: &'a str,
            spec: &'a str,
            pinned: bool,
        }
        let mut reqs = Vec::new();
        for raw in primary.lines() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() || line.starts_with('-') {
                continue; // options, includes, editable installs — skip
            }
            let Some(name) = pep508_name(line) else {
                continue;
            };
            let rest = &line[name.len()..];
            let rest = rest.split(';').next().unwrap_or(rest).trim();
            let pinned = rest.trim_start().starts_with("==");
            let spec = rest
                .trim_start_matches("==")
                .trim()
                .split(',')
                .next()
                .unwrap_or("")
                .trim();
            reqs.push(Req { name, spec, pinned });
        }
        if reqs.is_empty() {
            return Ok(ParseOk::default());
        }

        // FR-4.9: Exact only if EVERY line is `==`-pinned.
        let all_pinned = reqs.iter().all(|r| r.pinned);
        let conf = if all_pinned {
            Confidence::Exact
        } else {
            Confidence::Range
        };
        let mut notes = Vec::new();
        if !all_pinned {
            notes.push(
                "requirements.txt has unpinned lines; treating every entry as range confidence \
                 (FR-4.9)"
                    .to_string(),
            );
        }

        let deps = reqs
            .into_iter()
            .map(|r| {
                let v = if r.spec.is_empty() { "*" } else { r.spec };
                dep(r.name, v, Scope::Runtime, true, conf)
            })
            .collect::<Vec<_>>();
        Ok(ParseOk { deps, notes })
    }
}

// ---------------------------------------------------------------------------
// pyproject.toml (manifest fallback)
// ---------------------------------------------------------------------------

pub struct PyprojectTomlParser;
impl LockfileParser for PyprojectTomlParser {
    fn ecosystem(&self) -> Ecosystem {
        PY
    }
    fn kind(&self) -> ManifestKind {
        ManifestKind::Manifest
    }
    fn matches_file(&self, f: &str) -> bool {
        f == "pyproject.toml"
    }
    fn parse(
        &self,
        primary: &str,
        _p: &RelPath,
        _s: &SiblingFiles<'_>,
    ) -> Result<ParseOk, ParseError> {
        let out = declared_from_pyproject(primary)
            .into_iter()
            .map(|name| dep(&name, "*", Scope::Runtime, true, Confidence::Range))
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

    #[test]
    fn requirements_txt_mixed_is_range_throughout() {
        let reqs = "flask==2.3.3\nrequests>=2.28\nurllib3==2.0.4\n";
        let reg = ParserRegistry::builtin();
        let out = reg.parse_dir("", &files(&[("requirements.txt", reqs)]));
        assert_eq!(out.deps.len(), 3);
        assert!(
            out.deps.iter().all(|d| d.confidence == Confidence::Range),
            "one unpinned line makes the whole file range (FR-4.9)"
        );
        assert!(out.warnings.iter().any(|(_, m)| m.contains("unpinned")));
    }

    #[test]
    fn requirements_txt_fully_pinned_is_exact() {
        let reqs = "flask==2.3.3\nrequests==2.31.0\n";
        let reg = ParserRegistry::builtin();
        let out = reg.parse_dir("", &files(&[("requirements.txt", reqs)]));
        assert_eq!(out.deps.len(), 2);
        assert!(out.deps.iter().all(|d| d.confidence == Confidence::Exact));
        assert_eq!(
            out.deps.iter().find(|d| d.name == "flask").unwrap().version,
            "2.3.3"
        );
    }

    #[test]
    fn poetry_lock_with_pyproject_directness() {
        let pyproject = r#"
[project]
name = "demo"
dependencies = ["flask>=2.0", "requests"]
"#;
        let lock = r#"
[[package]]
name = "flask"
version = "2.3.3"
category = "main"

[[package]]
name = "requests"
version = "2.31.0"

[[package]]
name = "werkzeug"
version = "2.3.7"

[[package]]
name = "pytest"
version = "7.4.0"
category = "dev"
"#;
        let reg = ParserRegistry::builtin();
        let out = reg.parse_dir(
            "",
            &files(&[("pyproject.toml", pyproject), ("poetry.lock", lock)]),
        );
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
        let by = |n: &str| out.deps.iter().find(|d| d.name == n).unwrap();
        assert!(by("flask").is_direct);
        assert!(by("requests").is_direct);
        assert!(!by("werkzeug").is_direct);
        assert_eq!(by("pytest").scope, Scope::Dev);
        assert!(out.deps.iter().all(|d| d.confidence == Confidence::Exact));
    }

    #[test]
    fn name_normalization_pep503() {
        let reqs = "Django-REST-Framework==3.14.0\nzope.interface==6.0\n";
        let reg = ParserRegistry::builtin();
        let out = reg.parse_dir("", &files(&[("requirements.txt", reqs)]));
        let names: Vec<_> = out.deps.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&"django-rest-framework"));
        assert!(names.contains(&"zope-interface"));
    }
}
