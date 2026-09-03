//! Categorization engine (FR-3.1–3.5, DESIGN §10.3).
//!
//! Accumulate weights per [`Category`] from every rule that fires, then:
//!
//! - top score below `settings.floor` → [`Category::Unknown`] (FR-3.5) —
//!   admitting ignorance beats guessing;
//! - top two are Frontend and Backend, both at or above
//!   `settings.fullstack_threshold`, within `settings.margin` →
//!   [`Category::Fullstack`] (FR-3.4) — this case is common and an arbitrary
//!   pick would read as a bug;
//! - otherwise the highest wins.
//!
//! Confidence comes from the margin between the top two: `> 5` High, `> 2`
//! Medium, else Low. The full per-rule and per-category breakdown is
//! returned in [`Classification::scores`] and serialized for FR-3.6.

use crate::model::{Category, CategoryScores, Classification, ConfidenceLevel, FiredRule};
use crate::rules::signals::RepoSignals;
use crate::rules::{CategoryRule, CategorySettings};

/// Classify one repo from its signals and the merged category rules.
pub fn classify(
    settings: &CategorySettings,
    rules: &[CategoryRule],
    sig: &RepoSignals<'_>,
) -> Classification {
    let mut fired: Vec<FiredRule> = Vec::new();
    let mut totals: std::collections::BTreeMap<Category, f32> = std::collections::BTreeMap::new();

    for rule in rules {
        let Some(signal) = fired_signal(rule, sig) else {
            continue;
        };
        for (cat, weight) in &rule.weights {
            if *cat == Category::Unknown {
                continue; // weighting toward Unknown is meaningless
            }
            *totals.entry(*cat).or_insert(0.0) += weight.0;
            fired.push(FiredRule {
                rule_id: rule.id.clone(),
                signal: signal.clone(),
                category: *cat,
                weight: weight.0,
            });
        }
    }

    // Rank categories by accumulated weight, descending; stable tiebreak on
    // the enum's declaration order via the BTreeMap key.
    let mut ranked: Vec<(Category, f32)> = totals.into_iter().collect();
    ranked.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| format!("{:?}", a.0).cmp(&format!("{:?}", b.0)))
    });

    let scores = CategoryScores {
        totals: ranked.clone(),
        fired,
    };

    let (category, confidence) = resolve(&ranked, settings);

    Classification {
        category,
        confidence,
        scores,
        manual: None,
    }
}

/// The winning category and its confidence, given the ranked totals.
fn resolve(ranked: &[(Category, f32)], settings: &CategorySettings) -> (Category, ConfidenceLevel) {
    let Some(&(top_cat, top_score)) = ranked.first() else {
        return (Category::Unknown, ConfidenceLevel::Low);
    };

    if top_score < settings.floor {
        return (Category::Unknown, ConfidenceLevel::Low);
    }

    let second = ranked.get(1).copied();
    let second_score = second.map(|(_, s)| s).unwrap_or(0.0);
    let margin = top_score - second_score;

    // Fullstack special case: the top two are exactly {Frontend, Backend},
    // both clear the threshold, and they are close.
    if let Some((second_cat, _)) = second {
        let pair = [top_cat, second_cat];
        let is_fe_be = pair.contains(&Category::Frontend) && pair.contains(&Category::Backend);
        if is_fe_be
            && top_score >= settings.fullstack_threshold
            && second_score >= settings.fullstack_threshold
            && margin <= settings.margin
        {
            // Confidence from how far the pair sits above the next category.
            let third_score = ranked.get(2).map(|(_, s)| *s).unwrap_or(0.0);
            return (
                Category::Fullstack,
                confidence_from_margin(second_score - third_score),
            );
        }
    }

    (top_cat, confidence_from_margin(margin))
}

fn confidence_from_margin(margin: f32) -> ConfidenceLevel {
    if margin > 5.0 {
        ConfidenceLevel::High
    } else if margin > 2.0 {
        ConfidenceLevel::Medium
    } else {
        ConfidenceLevel::Low
    }
}

/// If `rule` fires, return a short human-readable description of the signal
/// that fired it (for the explainability breakdown, FR-3.6).
fn fired_signal(rule: &CategoryRule, sig: &RepoSignals<'_>) -> Option<String> {
    let deps = sig.matching_dependencies(&rule.any_dependency);
    if !deps.is_empty() {
        return Some(format!("any_dependency: {}", deps.join(", ")));
    }

    if sig.has_all_dependencies(&rule.all_dependencies) {
        return Some(format!(
            "all_dependencies: {}",
            rule.all_dependencies.join(", ")
        ));
    }

    let files = sig.matching_files(&rule.any_file);
    if !files.is_empty() {
        return Some(format!("any_file: {}", files.join(", ")));
    }

    for req in &rule.any_language {
        let min = req.min_percentage.map(|w| w.0).unwrap_or(0.0);
        if sig.has_language(&req.language, min) {
            return Some(if min > 0.0 {
                format!("any_language: {} ≥ {min}%", req.language)
            } else {
                format!("any_language: {}", req.language)
            });
        }
    }

    if let Some(name) = &rule.predicate {
        if sig.predicate(name) == Some(true) {
            return Some(format!("predicate: {name}"));
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ManifestKind;
    use crate::model::{
        Confidence, Dependency, Ecosystem, LanguageStat, ParsedManifest, RelPath, Scope,
    };
    use crate::rules::RulePacks;
    use crate::Paths;

    fn packs() -> RulePacks {
        RulePacks::load(&Paths::under(tempfile::tempdir().unwrap())).unwrap()
    }

    fn dep(eco: Ecosystem, name: &str) -> Dependency {
        Dependency {
            ecosystem: eco,
            name: name.to_string(),
            raw_name: name.to_string(),
            version: "1.0.0".into(),
            confidence: Confidence::Exact,
            scope: Scope::Runtime,
            is_direct: true,
            manifest_path: RelPath::new("manifest"),
        }
    }

    fn manifest() -> ParsedManifest {
        ParsedManifest {
            path: RelPath::new("Cargo.toml"),
            ecosystem: Ecosystem::CratesIo,
            kind: ManifestKind::Manifest,
            content_hash: "x".into(),
        }
    }

    fn classify_with(
        deps: &[Dependency],
        files: &[String],
        langs: &[LanguageStat],
        manifests: &[ParsedManifest],
    ) -> Classification {
        let p = packs();
        classify(
            &p.settings,
            &p.categories,
            &RepoSignals {
                deps,
                files,
                languages: langs,
                manifests,
            },
        )
    }

    #[test]
    fn frontend_repo_classifies_frontend() {
        let deps = [dep(Ecosystem::Npm, "react"), dep(Ecosystem::Npm, "vite")];
        let c = classify_with(&deps, &[], &[], &[]);
        assert_eq!(c.category, Category::Frontend);
    }

    #[test]
    fn backend_repo_classifies_backend() {
        let deps = [dep(Ecosystem::CratesIo, "axum")];
        let c = classify_with(&deps, &[], &[], &[]);
        assert_eq!(c.category, Category::Backend);
    }

    #[test]
    fn frontend_and_backend_together_yield_fullstack_not_a_coin_flip() {
        let deps = [
            dep(Ecosystem::Npm, "react"),
            dep(Ecosystem::Npm, "next"),
            dep(Ecosystem::Npm, "express"),
        ];
        let c = classify_with(&deps, &[], &[], &[]);
        assert_eq!(c.category, Category::Fullstack);
    }

    #[test]
    fn signal_less_repo_is_unknown_not_a_guess() {
        let c = classify_with(&[], &[], &[], &[]);
        assert_eq!(c.category, Category::Unknown);
        assert_eq!(c.confidence, ConfidenceLevel::Low);
        assert!(c.scores.fired.is_empty());
    }

    #[test]
    fn a_lone_marker_below_floor_stays_unknown() {
        // `frontend-build-tooling` alone weighs 2, below the floor of 3.
        let deps = [dep(Ecosystem::Npm, "webpack")];
        let c = classify_with(&deps, &[], &[], &[]);
        assert_eq!(c.category, Category::Unknown);
    }

    #[test]
    fn library_predicate_fires_for_manifest_without_entrypoint() {
        let files = ["src/lib.rs".to_string(), "README.md".to_string()];
        let c = classify_with(&[], &files, &[], &[manifest()]);
        assert_eq!(c.category, Category::Library);
        assert!(c
            .scores
            .fired
            .iter()
            .any(|f| f.rule_id == "publishable-library" && f.signal.contains("predicate")));
    }

    #[test]
    fn an_entrypoint_suppresses_the_library_predicate() {
        let files = ["src/main.rs".to_string()];
        let c = classify_with(&[], &files, &[], &[manifest()]);
        assert_ne!(c.category, Category::Library);
    }

    #[test]
    fn breakdown_records_every_rule_that_fired() {
        let deps = [dep(Ecosystem::Npm, "react")];
        let c = classify_with(&deps, &[], &[], &[]);
        let fe: f32 = c
            .scores
            .totals
            .iter()
            .find(|(cat, _)| *cat == Category::Frontend)
            .map(|(_, s)| *s)
            .unwrap();
        assert_eq!(fe, 5.0);
        assert!(c.scores.fired.iter().any(|f| f.rule_id == "react-frontend"));
    }
}
