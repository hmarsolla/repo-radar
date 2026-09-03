//! Signal evaluation shared by technology detection (M3-2) and the
//! categorization engine (M3-3).
//!
//! A [`RepoSignals`] bundles the four things a rule can test against — the
//! resolved dependencies, the repo-relative file list, the language
//! breakdown, and the parsed manifests — and exposes one predicate per
//! signal kind from DESIGN §10.1. Both engines evaluate rules through here so
//! "what a rule can see" is defined once.

use crate::model::{Dependency, LanguageStat, ParsedManifest};
use crate::parsers::normalize_package_name;

/// Everything a rule is allowed to inspect about one repository.
pub struct RepoSignals<'a> {
    pub deps: &'a [Dependency],
    /// Repo-relative, `/`-separated paths for every file in the repo (bounded;
    /// see [`crate::scan::manifests`]).
    pub files: &'a [String],
    pub languages: &'a [LanguageStat],
    pub manifests: &'a [ParsedManifest],
}

impl<'a> RepoSignals<'a> {
    /// True when `needle` names one of the resolved dependencies. The rule
    /// pack writes ecosystem-agnostic names (`React`, `scikit_learn`); each
    /// candidate is normalized with the *dependency's* ecosystem rule before
    /// comparing, so `React` matches the npm `react` and
    /// `github.com/gin-gonic/gin` matches verbatim for Go.
    pub fn has_dependency(&self, needle: &str) -> bool {
        self.deps
            .iter()
            .any(|d| normalize_package_name(d.ecosystem, needle) == d.name)
    }

    /// True when every name in `needles` is a resolved dependency.
    pub fn has_all_dependencies(&self, needles: &[String]) -> bool {
        !needles.is_empty() && needles.iter().all(|n| self.has_dependency(n))
    }

    /// The subset of `needles` that are resolved dependencies — used to build
    /// human-readable evidence.
    pub fn matching_dependencies<'n>(&self, needles: &'n [String]) -> Vec<&'n str> {
        needles
            .iter()
            .filter(|n| self.has_dependency(n))
            .map(|s| s.as_str())
            .collect()
    }

    /// Every file path that matches any of `patterns`. A pattern with a `/`
    /// is matched against the whole relative path with `*` not crossing a
    /// separator; a bare pattern is matched against each file's basename.
    pub fn matching_files(&self, patterns: &[String]) -> Vec<String> {
        let mut hits = Vec::new();
        for pat in patterns {
            let has_sep = pat.contains('/');
            let matcher = match globset::GlobBuilder::new(pat)
                .literal_separator(has_sep)
                .build()
            {
                Ok(g) => g.compile_matcher(),
                Err(_) => continue,
            };
            for f in self.files {
                let hay: &str = if has_sep {
                    f
                } else {
                    f.rsplit('/').next().unwrap_or(f)
                };
                if matcher.is_match(hay) && !hits.contains(f) {
                    hits.push(f.clone());
                }
            }
        }
        hits
    }

    /// True when a language at or above `min_percentage` (default 0) matches
    /// `language` (case-insensitive).
    pub fn has_language(&self, language: &str, min_percentage: f32) -> bool {
        self.languages
            .iter()
            .any(|l| l.language.eq_ignore_ascii_case(language) && l.percentage >= min_percentage)
    }

    /// Evaluate a named built-in predicate (DESIGN §10.1, closed set).
    /// `None` means the name is not a known predicate — the caller treats
    /// that as "rule does not fire".
    pub fn predicate(&self, name: &str) -> Option<bool> {
        match name {
            "has_manifest_without_entrypoint" => {
                Some(!self.manifests.is_empty() && !self.has_entrypoint())
            }
            _ => None,
        }
    }

    /// Heuristic for "this repo is a library, not an app": no recognisable
    /// program entrypoint anywhere in the tree.
    fn has_entrypoint(&self) -> bool {
        const ENTRYPOINTS: &[&str] = &[
            "src/main.rs",
            "src/bin/*.rs",
            "main.go",
            "cmd/*/main.go",
            "main.py",
            "__main__.py",
            "app.py",
            "manage.py",
            "wsgi.py",
            "asgi.py",
            "index.js",
            "index.ts",
            "src/index.js",
            "src/index.ts",
            "src/main.js",
            "src/main.ts",
        ];
        let pats: Vec<String> = ENTRYPOINTS.iter().map(|s| s.to_string()).collect();
        !self.matching_files(&pats).is_empty()
    }
}
