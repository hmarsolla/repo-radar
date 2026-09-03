//! Multi-manifest / monorepo discovery within a repository (FR-4.7, DESIGN D3).
//!
//! Walks the repo (honoring the prune list), finds every directory that
//! holds a recognised manifest file, and runs [`ParserRegistry::parse_dir`]
//! per directory. Every dependency keeps its `manifest_path` (FR-4.7); a
//! repo with more than one manifest root is tagged `monorepo`.

use std::collections::BTreeMap;
use std::path::Path;

use ignore::WalkBuilder;

use crate::model::{Dependency, ParsedManifest, Warning, WarningKind, WarningScope};
use crate::parsers::ParserRegistry;
use crate::scan::discovery::DiscoveryConfig;

/// Largest manifest file we will read into memory. A 5 MB lockfile is
/// already pathological; beyond that we skip with a warning rather than
/// risk the memory budget.
const MAX_MANIFEST_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug, Default)]
pub struct ManifestScan {
    pub dependencies: Vec<Dependency>,
    pub manifests: Vec<ParsedManifest>,
    pub monorepo: bool,
    pub warnings: Vec<Warning>,
}

/// Discover and parse every manifest under `repo_path`.
pub fn scan_manifests(
    repo_path: &Path,
    registry: &ParserRegistry,
    discovery: &DiscoveryConfig,
) -> ManifestScan {
    let prune = discovery.prune_dirs.clone();

    // dir (repo-relative, "" = root) -> { file name -> content }
    let mut by_dir: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    let mut warnings: Vec<Warning> = Vec::new();

    let mut builder = WalkBuilder::new(repo_path);
    builder
        .hidden(false)
        .follow_links(false)
        .git_ignore(true) // respect the repo's own .gitignore for build output
        .git_global(false)
        .parents(false)
        .require_git(false)
        .filter_entry(move |entry| {
            if entry.file_type().is_some_and(|ft| ft.is_dir()) {
                if let Some(name) = entry.file_name().to_str() {
                    return !prune.iter().any(|p| p == name);
                }
            }
            true
        });

    for result in builder.build() {
        let entry = match result {
            Ok(e) => e,
            Err(_) => continue,
        };
        if !entry.file_type().is_some_and(|ft| ft.is_file()) {
            continue;
        }
        let Some(file_name) = entry.file_name().to_str() else {
            continue;
        };
        if !registry.is_manifest_file(file_name) {
            continue;
        }

        let abs = entry.path();
        let rel_dir = abs
            .parent()
            .and_then(|p| p.strip_prefix(repo_path).ok())
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();

        match std::fs::metadata(abs) {
            Ok(m) if m.len() > MAX_MANIFEST_BYTES => {
                warnings.push(Warning::new(
                    WarningScope::File(rel_join(&rel_dir, file_name)),
                    WarningKind::ParseFailed,
                    format!("{file_name} is {} bytes; skipped", m.len()),
                ));
                continue;
            }
            Ok(_) => {}
            Err(_) => continue,
        }

        match std::fs::read_to_string(abs) {
            Ok(content) => {
                by_dir
                    .entry(rel_dir)
                    .or_default()
                    .insert(file_name.to_string(), content);
            }
            Err(e) => {
                warnings.push(Warning::new(
                    WarningScope::File(rel_join(&rel_dir, file_name)),
                    WarningKind::ParseFailed,
                    format!("could not read {file_name}: {e}"),
                ));
            }
        }
    }

    let mut out = ManifestScan::default();
    let mut roots_with_deps = 0usize;

    for (dir, files) in &by_dir {
        let parsed = registry.parse_dir(dir, files);
        if !parsed.deps.is_empty() {
            roots_with_deps += 1;
        }
        for (file, msg) in parsed.warnings {
            out.warnings.push(Warning::new(
                WarningScope::File(file),
                WarningKind::ParseFailed,
                msg,
            ));
        }
        out.dependencies.extend(parsed.deps);
        out.manifests.extend(parsed.manifests);
    }

    out.warnings.extend(warnings);
    out.monorepo = roots_with_deps > 1;
    out
}

/// Cheap pass for the scan fingerprint (§6.5): `(repo-relative path, blake3
/// hex)` for every recognised manifest file, **without parsing**. Sorted by
/// path for a stable fingerprint.
pub fn manifest_hashes(
    repo_path: &Path,
    registry: &ParserRegistry,
    discovery: &DiscoveryConfig,
) -> Vec<(String, String)> {
    let prune = discovery.prune_dirs.clone();
    let mut builder = WalkBuilder::new(repo_path);
    builder
        .hidden(false)
        .follow_links(false)
        .git_ignore(true)
        .parents(false)
        .require_git(false)
        .filter_entry(move |entry| {
            if entry.file_type().is_some_and(|ft| ft.is_dir()) {
                if let Some(name) = entry.file_name().to_str() {
                    return !prune.iter().any(|p| p == name);
                }
            }
            true
        });

    let mut out = Vec::new();
    for entry in builder.build().flatten() {
        if !entry.file_type().is_some_and(|ft| ft.is_file()) {
            continue;
        }
        let Some(name) = entry.file_name().to_str() else {
            continue;
        };
        if !registry.is_manifest_file(name) {
            continue;
        }
        if let Ok(bytes) = std::fs::read(entry.path()) {
            let rel = entry
                .path()
                .strip_prefix(repo_path)
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_else(|_| name.to_string());
            out.push((rel, blake3::hash(&bytes).to_hex().to_string()));
        }
    }
    out.sort();
    out
}

fn rel_join(dir: &str, file: &str) -> String {
    if dir.is_empty() {
        file.to_string()
    } else {
        format!("{}/{}", dir.trim_end_matches('/'), file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, rel: &str, content: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, content).unwrap();
    }

    #[test]
    fn monorepo_yields_deps_grouped_by_subpackage_and_tags_the_repo() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            root,
            "services/api/package.json",
            r#"{"dependencies":{"express":"^4.0.0"}}"#,
        );
        write(
            root,
            "services/api/package-lock.json",
            r#"{"lockfileVersion":3,"packages":{"":{"dependencies":{"express":"^4.0.0"}},"node_modules/express":{"version":"4.19.2"}}}"#,
        );
        write(
            root,
            "libs/util/Cargo.toml",
            "[package]\nname=\"util\"\nversion=\"0.1.0\"\n[dependencies]\nserde=\"1\"\n",
        );
        // A pruned dir must contribute nothing.
        write(
            root,
            "node_modules/leftpad/package.json",
            r#"{"dependencies":{"evil":"1.0.0"}}"#,
        );

        let scan = scan_manifests(
            root,
            &ParserRegistry::builtin(),
            &DiscoveryConfig::default(),
        );
        assert!(scan.monorepo);
        assert!(scan.warnings.is_empty(), "{:?}", scan.warnings);

        let paths: Vec<_> = scan
            .dependencies
            .iter()
            .map(|d| (d.name.as_str(), d.manifest_path.as_str()))
            .collect();
        assert!(paths.contains(&("express", "services/api/package-lock.json")));
        assert!(paths
            .iter()
            .any(|(n, p)| *n == "serde" && p.starts_with("libs/util/")));
        assert!(
            !scan.dependencies.iter().any(|d| d.name == "evil"),
            "node_modules pruned"
        );
        assert_eq!(scan.manifests.len(), 2);
    }

    #[test]
    fn single_root_is_not_a_monorepo() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "go.mod",
            "module x\n\ngo 1.22\n\nrequire github.com/pkg/errors v0.9.1\n",
        );
        let scan = scan_manifests(
            dir.path(),
            &ParserRegistry::builtin(),
            &DiscoveryConfig::default(),
        );
        assert!(!scan.monorepo);
        assert_eq!(scan.dependencies.len(), 1);
    }
}
