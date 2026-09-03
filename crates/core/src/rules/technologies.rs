//! Technology detection (FR-2.3–2.5, DESIGN §10.1).
//!
//! Each [`TechRule`] tests dependency signals and marker-file globs. A
//! detection records its evidence: entries prefixed `dependency:` are
//! dependency-confirmed, entries prefixed `file:` are marker-only. FR-2.4
//! asks the UI to render marker-only detections with lower prominence, so
//! the distinction is preserved in the evidence rather than collapsed here.
//! Package managers fall out of this the same way (FR-2.5): a rule keyed on
//! `any_file = ["pnpm-lock.yaml"]`.

use crate::model::DetectedTech;
use crate::rules::signals::RepoSignals;
use crate::rules::TechRule;

/// Run every rule in `rules` against `sig`, returning one [`DetectedTech`]
/// per rule that produced at least one piece of evidence. Sorted by
/// technology name for stable output.
pub fn detect(rules: &[TechRule], sig: &RepoSignals<'_>) -> Vec<DetectedTech> {
    let mut out = Vec::new();

    for rule in rules {
        let mut evidence: Vec<String> = Vec::new();

        for dep in sig.matching_dependencies(&rule.any_dependency) {
            evidence.push(format!("dependency:{dep}"));
        }
        for path in sig.matching_files(&rule.any_file) {
            evidence.push(format!("file:{path}"));
        }

        if !evidence.is_empty() {
            out.push(DetectedTech {
                tech: rule.name.clone(),
                kind: rule.kind.clone(),
                evidence,
            });
        }
    }

    out.sort_by(|a, b| a.tech.cmp(&b.tech));
    out
}

/// True when this detection has at least one `dependency:` evidence entry —
/// the higher-prominence case for FR-2.4. Kept here so the frontend contract
/// (any evidence string starting `dependency:`) has one authoritative
/// definition.
pub fn is_dependency_confirmed(tech: &DetectedTech) -> bool {
    tech.evidence.iter().any(|e| e.starts_with("dependency:"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Confidence, Dependency, Ecosystem, RelPath, Scope};
    use crate::rules::RulePacks;
    use crate::Paths;

    fn dep(eco: Ecosystem, name: &str) -> Dependency {
        Dependency {
            ecosystem: eco,
            name: name.to_string(),
            raw_name: name.to_string(),
            version: "1.0.0".to_string(),
            confidence: Confidence::Exact,
            scope: Scope::Runtime,
            is_direct: true,
            manifest_path: RelPath::new("package.json"),
        }
    }

    fn shipped() -> RulePacks {
        RulePacks::load(&Paths::under(tempfile::tempdir().unwrap())).unwrap()
    }

    #[test]
    fn dependency_hit_is_confirmed_marker_hit_is_not() {
        let packs = shipped();
        let deps = vec![dep(Ecosystem::Npm, "react")];
        let files = vec!["tsconfig.json".to_string(), "src/App.tsx".to_string()];
        let sig = RepoSignals {
            deps: &deps,
            files: &files,
            languages: &[],
            manifests: &[],
        };
        let found = detect(&packs.technologies, &sig);

        let react = found
            .iter()
            .find(|t| t.tech == "React")
            .expect("react detected");
        assert!(is_dependency_confirmed(react));
        assert_eq!(react.evidence, vec!["dependency:react"]);

        let ts = found
            .iter()
            .find(|t| t.tech == "TypeScript")
            .expect("typescript detected via marker");
        assert!(!is_dependency_confirmed(ts));
        assert_eq!(ts.evidence, vec!["file:tsconfig.json"]);
    }

    #[test]
    fn package_manager_derives_from_lockfile_presence() {
        let packs = shipped();
        let files = vec!["pnpm-lock.yaml".to_string()];
        let sig = RepoSignals {
            deps: &[],
            files: &files,
            languages: &[],
            manifests: &[],
        };
        let found = detect(&packs.technologies, &sig);
        assert!(found
            .iter()
            .any(|t| t.tech == "pnpm" && t.kind == "package-manager"));
        assert!(!found.iter().any(|t| t.tech == "npm"));
    }

    #[test]
    fn no_signals_no_detections() {
        let packs = shipped();
        let sig = RepoSignals {
            deps: &[],
            files: &[],
            languages: &[],
            manifests: &[],
        };
        assert!(detect(&packs.technologies, &sig).is_empty());
    }
}
