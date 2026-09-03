//! Lockfile and manifest parsers (DESIGN §7, FR-4).
//!
//! Every format implements [`LockfileParser`]. The [`ParserRegistry`] holds
//! them and, per manifest directory, applies the selection rule (§7.2):
//! **a present lockfile wins and its output is `Confidence::Exact`;
//! otherwise the manifest parser runs at `Confidence::Range`.** A parser may
//! read sibling files in the same directory (e.g. a manifest alongside a
//! lockfile, to determine directness).
//!
//! Adding an ecosystem is a new impl plus an [`crate::model::Ecosystem`]
//! variant — the pipeline, matcher, and scorer are untouched.

pub mod cargo;
pub mod golang;
pub mod normalize;
pub mod npm;
pub mod python;

pub use normalize::normalize_package_name;

use std::collections::BTreeMap;

use crate::model::{Confidence, Dependency, Ecosystem, RelPath};

/// Whether a parser consumes a lockfile (exact versions) or a manifest
/// (ranges).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManifestKind {
    Lockfile,
    Manifest,
}

impl ManifestKind {
    /// The confidence a parser of this kind normally emits (§7.2). A parser
    /// may deviate — e.g. a yarn-berry fallback emits `Range` from a
    /// `Lockfile`-kind parser.
    pub fn confidence(self) -> Confidence {
        match self {
            ManifestKind::Lockfile => Confidence::Exact,
            ManifestKind::Manifest => Confidence::Range,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ManifestKind::Lockfile => "lockfile",
            ManifestKind::Manifest => "manifest",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("malformed {format}: {detail}")]
    Malformed {
        format: &'static str,
        detail: String,
    },
    #[error("unsupported {format}: {detail}")]
    Unsupported {
        format: &'static str,
        detail: String,
    },
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

impl ParseError {
    pub fn malformed(format: &'static str, detail: impl Into<String>) -> Self {
        ParseError::Malformed {
            format,
            detail: detail.into(),
        }
    }
}

/// Fetches another file's content from the same directory, by file name.
pub type SiblingFiles<'a> = dyn Fn(&str) -> Option<&'a str> + 'a;

/// What a parser produces: the dependencies, plus any non-fatal notes the
/// user should see (a yarn-berry fallback, an unparseable version line).
#[derive(Debug, Default)]
pub struct ParseOk {
    pub deps: Vec<Dependency>,
    pub notes: Vec<String>,
}

impl From<Vec<Dependency>> for ParseOk {
    fn from(deps: Vec<Dependency>) -> Self {
        Self {
            deps,
            notes: vec![],
        }
    }
}

/// One parseable manifest format.
pub trait LockfileParser: Send + Sync {
    fn ecosystem(&self) -> Ecosystem;
    fn kind(&self) -> ManifestKind;

    /// Match by file name only — cheap, no IO. `file_name` is the final path
    /// component, e.g. `"package-lock.json"`.
    fn matches_file(&self, file_name: &str) -> bool;

    /// Parse `primary` (the matched file's content). `sibling` gives access
    /// to other files in the same directory (e.g. `package.json` for
    /// directness). `primary_path` is the repo-relative path of the primary
    /// file, stamped onto every returned dependency (FR-4.7).
    fn parse(
        &self,
        primary: &str,
        primary_path: &RelPath,
        sibling: &SiblingFiles<'_>,
    ) -> Result<ParseOk, ParseError>;
}

/// A manifest that contributed dependencies, for the `manifests` table and
/// the scan fingerprint (§6.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedManifest {
    pub path: RelPath,
    pub ecosystem: Ecosystem,
    pub kind: ManifestKind,
    /// `blake3` hex of the file content.
    pub content_hash: String,
}

/// The result of parsing one directory's manifests.
#[derive(Debug, Default)]
pub struct DirParse {
    pub deps: Vec<Dependency>,
    pub manifests: Vec<ParsedManifest>,
    /// `(file, message)` for recoverable parse failures — the caller turns
    /// these into [`crate::model::Warning`]s (FR-4.8).
    pub warnings: Vec<(String, String)>,
}

/// Holds every parser and applies the selection rule.
pub struct ParserRegistry {
    parsers: Vec<Box<dyn LockfileParser>>,
}

impl ParserRegistry {
    /// Every built-in parser (M2-4 … M2-10).
    pub fn builtin() -> Self {
        Self {
            parsers: vec![
                // npm family
                Box::new(npm::PackageLockParser),
                Box::new(npm::PnpmLockParser),
                Box::new(npm::YarnLockParser),
                Box::new(npm::PackageJsonParser),
                // cargo
                Box::new(cargo::CargoLockParser),
                Box::new(cargo::CargoTomlParser),
                // go
                Box::new(golang::GoModParser),
                // python
                Box::new(python::PoetryLockParser),
                Box::new(python::UvLockParser),
                Box::new(python::PipfileLockParser),
                Box::new(python::RequirementsTxtParser),
                Box::new(python::PyprojectTomlParser),
            ],
        }
    }

    /// True if `file_name` is a manifest any parser recognises — used by
    /// manifest discovery (M2-10) to find roots.
    pub fn is_manifest_file(&self, file_name: &str) -> bool {
        self.parsers.iter().any(|p| p.matches_file(file_name))
    }

    /// Parse one directory. `files` maps file name → content for every
    /// manifest-ish file in that directory. `dir` is the repo-relative
    /// directory path (`""` for the repo root).
    pub fn parse_dir(&self, dir: &str, files: &BTreeMap<String, String>) -> DirParse {
        let mut out = DirParse::default();

        for eco in Ecosystem::ALL {
            let eco_parsers: Vec<&dyn LockfileParser> = self
                .parsers
                .iter()
                .map(|p| p.as_ref())
                .filter(|p| p.ecosystem() == eco)
                .collect();

            // Present lockfile parsers for this ecosystem.
            let lockfiles: Vec<_> = eco_parsers
                .iter()
                .copied()
                .filter(|p| p.kind() == ManifestKind::Lockfile)
                .filter_map(|p| present_file(files, p).map(|(name, content)| (p, name, content)))
                .collect();

            let chosen: Vec<_> = if !lockfiles.is_empty() {
                lockfiles
            } else {
                eco_parsers
                    .iter()
                    .copied()
                    .filter(|p| p.kind() == ManifestKind::Manifest)
                    .filter_map(|p| {
                        present_file(files, p).map(|(name, content)| (p, name, content))
                    })
                    .collect()
            };

            for (parser, file_name, content) in chosen {
                let rel = join_rel(dir, &file_name);
                let sibling = |name: &str| files.get(name).map(String::as_str);
                match parser.parse(content, &rel, &sibling) {
                    Ok(ParseOk { mut deps, notes }) => {
                        // The parser owns `confidence` (a berry fallback, for
                        // instance, emits `Range` from a `Lockfile`-kind
                        // parser); the registry only stamps the path.
                        for d in &mut deps {
                            d.manifest_path = rel.clone();
                        }
                        for note in notes {
                            out.warnings.push((rel.to_string(), note));
                        }
                        out.manifests.push(ParsedManifest {
                            path: rel.clone(),
                            ecosystem: eco,
                            kind: parser.kind(),
                            content_hash: blake3::hash(content.as_bytes()).to_hex().to_string(),
                        });
                        out.deps.append(&mut deps);
                    }
                    Err(e) => {
                        out.warnings.push((rel.to_string(), e.to_string()));
                    }
                }
            }
        }

        // Stable, de-duplicated output.
        out.deps.sort_by(|a, b| {
            (a.manifest_path.as_str(), &a.name, &a.version).cmp(&(
                b.manifest_path.as_str(),
                &b.name,
                &b.version,
            ))
        });
        out.deps.dedup_by(|a, b| {
            a.manifest_path == b.manifest_path
                && a.name == b.name
                && a.version == b.version
                && a.scope == b.scope
        });
        out
    }
}

fn present_file<'a>(
    files: &'a BTreeMap<String, String>,
    parser: &dyn LockfileParser,
) -> Option<(String, &'a str)> {
    files
        .iter()
        .find(|(name, _)| parser.matches_file(name))
        .map(|(name, content)| (name.clone(), content.as_str()))
}

fn join_rel(dir: &str, file: &str) -> RelPath {
    if dir.is_empty() {
        RelPath::new(file)
    } else {
        RelPath::new(format!("{}/{}", dir.trim_end_matches('/'), file))
    }
}

#[cfg(test)]
mod selection_tests {
    use super::*;

    fn files(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn lockfile_plus_manifest_yields_exact() {
        let reg = ParserRegistry::builtin();
        let f = files(&[
            ("package.json", r#"{"dependencies":{"left-pad":"^1.0.0"}}"#),
            (
                "package-lock.json",
                r#"{"lockfileVersion":3,"packages":{"":{"dependencies":{"left-pad":"^1.0.0"}},"node_modules/left-pad":{"version":"1.3.0"}}}"#,
            ),
        ]);
        let out = reg.parse_dir("", &f);
        assert!(!out.deps.is_empty(), "warnings: {:?}", out.warnings);
        assert!(out.deps.iter().all(|d| d.confidence == Confidence::Exact));
    }

    #[test]
    fn manifest_only_yields_range() {
        let reg = ParserRegistry::builtin();
        let f = files(&[("package.json", r#"{"dependencies":{"left-pad":"^1.0.0"}}"#)]);
        let out = reg.parse_dir("", &f);
        assert_eq!(out.deps.len(), 1);
        assert_eq!(out.deps[0].confidence, Confidence::Range);
        assert_eq!(out.deps[0].name, "left-pad");
    }
}
