//! Health scoring (FR-6, DESIGN §9).
//!
//! [`score`] is a pure function with no IO. Rules it enforces (property-tested
//! in DESIGN §16.3):
//!
//! - Compromise is a **cap** at 39, not a subtraction — a good score
//!   elsewhere must not average away a backdoored package (FR-6.3).
//! - `×0.4` for `Confidence::Range`, `×0.5` for dev/build scope (FR-6.5/6.6).
//! - Per-`(ecosystem, name)` diminishing returns: `1 / (1 + index)`.
//! - Hygiene deductions floor the score at 60 — they signal neglect, not
//!   danger (FR-6.8).

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::model::{
    Band, Confidence, Deduction, DeductionCause, FindingKind, HealthResult, Scope, Severity,
};

/// The highest score a confirmed compromise may leave (FR-6.3).
pub const COMPROMISE_CAP: u8 = 39;
/// Hygiene deductions cannot push the score below this (FR-6.8).
pub const HYGIENE_FLOOR: u8 = 60;

/// One finding, reduced to what scoring needs. Built in stage 4 (M2-18) from
/// the matcher output joined against advisory rows.
#[derive(Debug, Clone, PartialEq)]
pub struct ScoredFinding {
    pub ecosystem: crate::model::Ecosystem,
    pub package_name: String,
    pub advisory_id: String,
    pub kind: FindingKind,
    pub severity: Severity,
    pub confidence: Confidence,
    pub scope: Scope,
    pub suppressed: bool,
}

/// Repository hygiene signals (FR-6.7).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct HygieneInputs {
    pub has_lockfile: bool,
    /// Days since the last commit; `None` for a repo with no commits.
    pub days_since_commit: Option<u32>,
    pub dirty: bool,
}

/// Tunable weights (FR-6.10). Deserialized from a config file with these
/// values as `Default`; not exposed in settings UI in phase 1.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Type)]
#[serde(default)]
pub struct Weights {
    pub sev_critical: f32,
    pub sev_high: f32,
    pub sev_medium: f32,
    pub sev_low: f32,
    pub sev_unscored: f32,
    pub range_multiplier: f32,
    pub dev_build_multiplier: f32,
    pub no_lockfile: f32,
    pub stale_365d: f32,
    pub stale_730d: f32,
    pub dirty_tree: f32,
}

impl Default for Weights {
    fn default() -> Self {
        Self {
            sev_critical: 15.0,
            sev_high: 8.0,
            sev_medium: 3.0,
            sev_low: 1.0,
            sev_unscored: 2.0,
            range_multiplier: 0.4,
            dev_build_multiplier: 0.5,
            no_lockfile: 5.0,
            stale_365d: 5.0,
            stale_730d: 10.0,
            dirty_tree: 2.0,
        }
    }
}

impl Weights {
    fn severity_base(&self, s: Severity) -> f32 {
        match s {
            Severity::Critical => self.sev_critical,
            Severity::High => self.sev_high,
            Severity::Medium => self.sev_medium,
            Severity::Low => self.sev_low,
            Severity::Unscored => self.sev_unscored,
        }
    }
}

/// Compute a repository's health.
///
/// `advisories_synced` is `false` on a fresh install with no advisory data —
/// health is then **Unknown**, not healthy (DESIGN §14.4), regardless of the
/// numeric score.
pub fn score(
    findings: &[ScoredFinding],
    hygiene: &HygieneInputs,
    weights: &Weights,
    advisories_synced: bool,
) -> HealthResult {
    let mut breakdown: Vec<Deduction> = Vec::new();

    // --- Step 2: compromise cap (FR-6.3) --------------------------------
    let capped_by = findings
        .iter()
        .find(|f| {
            f.kind == FindingKind::Compromise && f.confidence == Confidence::Exact && !f.suppressed
        })
        .map(|f| f.advisory_id.clone());

    // --- Steps 3 + 4: vulnerability deductions with diminishing returns -
    // Group vulnerability findings by (ecosystem, name).
    let mut groups: std::collections::HashMap<
        (crate::model::Ecosystem, &str),
        Vec<&ScoredFinding>,
    > = std::collections::HashMap::new();
    for f in findings {
        if f.suppressed || f.kind != FindingKind::Vulnerability {
            continue;
        }
        groups
            .entry((f.ecosystem, f.package_name.as_str()))
            .or_default()
            .push(f);
    }

    let mut vuln_total = 0.0_f32;
    // Deterministic order: by package name.
    let mut group_keys: Vec<_> = groups.keys().copied().collect();
    group_keys.sort_by(|a, b| a.1.cmp(b.1));

    for key in group_keys {
        let mut scaled: Vec<PreScaled<'_>> = groups[&key]
            .iter()
            .map(|f| {
                let mut amount = weights.severity_base(f.severity);
                let mut mults = Vec::new();
                if f.confidence == Confidence::Range {
                    amount *= weights.range_multiplier;
                    mults.push(("range confidence".to_string(), weights.range_multiplier));
                }
                if f.scope.is_dev_or_build() {
                    amount *= weights.dev_build_multiplier;
                    mults.push(("dev/build scope".to_string(), weights.dev_build_multiplier));
                }
                PreScaled {
                    amount,
                    finding: f,
                    multipliers: mults,
                }
            })
            .collect();
        // Sort descending so the worst finding in a package counts fully.
        scaled.sort_by(|a, b| {
            b.amount
                .partial_cmp(&a.amount)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        for (
            index,
            PreScaled {
                amount: base_amount,
                finding: f,
                multipliers: mut mults,
            },
        ) in scaled.into_iter().enumerate()
        {
            let dim = 1.0 / (1.0 + index as f32);
            let amount = base_amount * dim;
            if dim < 1.0 {
                mults.push(("nth in package".to_string(), dim));
            }
            vuln_total += amount;
            breakdown.push(Deduction {
                cause: DeductionCause::Advisory(f.advisory_id.clone()),
                label: format!(
                    "{} in {} ({:?})",
                    match f.severity {
                        Severity::Unscored => "unscored advisory".to_string(),
                        s => format!("{s:?} advisory"),
                    },
                    f.package_name,
                    f.confidence
                ),
                amount,
                multipliers: mults,
            });
        }
    }

    // --- Step 5: hygiene (FR-6.7), floored at 60 (FR-6.8) --------------
    let mut hygiene_total = 0.0_f32;
    if !hygiene.has_lockfile {
        hygiene_total += weights.no_lockfile;
        breakdown.push(Deduction {
            cause: DeductionCause::NoLockfile,
            label: "no lockfile present".into(),
            amount: weights.no_lockfile,
            multipliers: vec![],
        });
    }
    match hygiene.days_since_commit {
        Some(d) if d >= 730 => {
            hygiene_total += weights.stale_730d;
            breakdown.push(Deduction {
                cause: DeductionCause::StaleCommits,
                label: "no commit in over 2 years".into(),
                amount: weights.stale_730d,
                multipliers: vec![],
            });
        }
        Some(d) if d >= 365 => {
            hygiene_total += weights.stale_365d;
            breakdown.push(Deduction {
                cause: DeductionCause::StaleCommits,
                label: "no commit in over a year".into(),
                amount: weights.stale_365d,
                multipliers: vec![],
            });
        }
        _ => {}
    }
    if hygiene.dirty {
        hygiene_total += weights.dirty_tree;
        breakdown.push(Deduction {
            cause: DeductionCause::DirtyTree,
            label: "working tree has uncommitted changes".into(),
            amount: weights.dirty_tree,
            multipliers: vec![],
        });
    }

    // --- Step 6/7: combine, floor hygiene, clamp, cap, band -----------
    let raw = 100.0 - vuln_total - hygiene_total;
    // The hygiene floor: hygiene alone can't drop below 60. Concretely, if
    // removing hygiene would put us at/above 60, clamp the post-hygiene
    // score up to 60.
    let after_vuln = 100.0 - vuln_total;
    let floored = if after_vuln >= HYGIENE_FLOOR as f32 {
        raw.max(HYGIENE_FLOOR as f32)
    } else {
        // vulnerabilities already took it below the floor; hygiene doesn't
        // get to make it *worse* than the vuln-only score in that regime.
        after_vuln
    };

    let mut final_score = floored.clamp(0.0, 100.0).round() as u8;
    if capped_by.is_some() {
        final_score = final_score.min(COMPROMISE_CAP);
    }

    let band = if !advisories_synced {
        Band::Unknown
    } else if capped_by.is_some() {
        Band::Critical
    } else {
        band_for(final_score)
    };

    HealthResult {
        score: final_score,
        band,
        breakdown,
        capped_by,
    }
}

/// One vulnerability finding after severity + multipliers, before the
/// per-package diminishing-returns scale.
struct PreScaled<'a> {
    amount: f32,
    finding: &'a ScoredFinding,
    multipliers: Vec<(String, f32)>,
}

fn band_for(score: u8) -> Band {
    match score {
        0..=39 => Band::Critical,
        40..=59 => Band::Poor,
        60..=74 => Band::Fair,
        75..=89 => Band::Good,
        _ => Band::Excellent,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Ecosystem;

    fn vuln(sev: Severity, conf: Confidence, scope: Scope, pkg: &str, id: &str) -> ScoredFinding {
        ScoredFinding {
            ecosystem: Ecosystem::Npm,
            package_name: pkg.into(),
            advisory_id: id.into(),
            kind: FindingKind::Vulnerability,
            severity: sev,
            confidence: conf,
            scope,
            suppressed: false,
        }
    }

    fn clean_hygiene() -> HygieneInputs {
        HygieneInputs {
            has_lockfile: true,
            days_since_commit: Some(10),
            dirty: false,
        }
    }

    #[test]
    fn perfect_repo_scores_100() {
        let r = score(&[], &clean_hygiene(), &Weights::default(), true);
        assert_eq!(r.score, 100);
        assert_eq!(r.band, Band::Excellent);
        assert!(r.breakdown.is_empty());
    }

    #[test]
    fn unknown_band_when_advisories_never_synced() {
        let r = score(&[], &clean_hygiene(), &Weights::default(), false);
        assert_eq!(r.band, Band::Unknown, "no sync => unknown, not healthy");
    }

    #[test]
    fn a_confirmed_compromise_always_bands_critical() {
        // Even with otherwise perfect inputs and a high numeric floor.
        let f = ScoredFinding {
            kind: FindingKind::Compromise,
            confidence: Confidence::Exact,
            ..vuln(
                Severity::Unscored,
                Confidence::Exact,
                Scope::Runtime,
                "evil",
                "MAL-1",
            )
        };
        let r = score(&[f], &clean_hygiene(), &Weights::default(), true);
        assert!(r.score <= COMPROMISE_CAP);
        assert_eq!(r.band, Band::Critical);
        assert_eq!(r.capped_by.as_deref(), Some("MAL-1"));
    }

    #[test]
    fn range_confidence_compromise_does_not_cap() {
        // FR-6.3 caps only on Confidence::Exact compromises.
        let f = ScoredFinding {
            kind: FindingKind::Compromise,
            confidence: Confidence::Range,
            ..vuln(
                Severity::Unscored,
                Confidence::Range,
                Scope::Runtime,
                "maybe-evil",
                "MAL-2",
            )
        };
        let r = score(&[f], &clean_hygiene(), &Weights::default(), true);
        assert!(r.capped_by.is_none());
        assert!(r.score > COMPROMISE_CAP);
    }

    #[test]
    fn hygiene_only_never_drops_below_60() {
        let bad = HygieneInputs {
            has_lockfile: false,
            days_since_commit: Some(5000),
            dirty: true,
        };

        // Default weights: 5 + 10 + 2 = 17, so 83 — already above the floor.
        let r = score(&[], &bad, &Weights::default(), true);
        assert_eq!(r.score, 83);
        assert_eq!(r.band, Band::Good);

        // Inflated hygiene weights that *would* blow past the floor: it holds.
        let heavy = Weights {
            no_lockfile: 40.0,
            stale_730d: 40.0,
            dirty_tree: 40.0,
            ..Weights::default()
        };
        let r = score(&[], &bad, &heavy, true);
        assert_eq!(r.score, HYGIENE_FLOOR, "hygiene alone is floored at 60");
        assert_eq!(r.band, Band::Fair);
    }

    #[test]
    fn range_and_dev_multipliers_apply() {
        let runtime_exact = score(
            &[vuln(
                Severity::High,
                Confidence::Exact,
                Scope::Runtime,
                "a",
                "G1",
            )],
            &clean_hygiene(),
            &Weights::default(),
            true,
        );
        let dev_range = score(
            &[vuln(
                Severity::High,
                Confidence::Range,
                Scope::Dev,
                "a",
                "G1",
            )],
            &clean_hygiene(),
            &Weights::default(),
            true,
        );
        // 8 vs 8 * 0.4 * 0.5 = 1.6
        assert_eq!(runtime_exact.score, 92);
        assert_eq!(dev_range.score, 98);
    }

    #[test]
    fn diminishing_returns_within_a_package() {
        let many = vec![
            vuln(Severity::High, Confidence::Exact, Scope::Runtime, "x", "G1"),
            vuln(Severity::High, Confidence::Exact, Scope::Runtime, "x", "G2"),
            vuln(Severity::High, Confidence::Exact, Scope::Runtime, "x", "G3"),
        ];
        let r = score(&many, &clean_hygiene(), &Weights::default(), true);
        // 8*(1 + 1/2 + 1/3) = 14.67, not 24.
        assert_eq!(r.score, 100 - 15); // rounds to 15
    }

    #[test]
    fn breakdown_sums_to_100_minus_score_when_uncapped() {
        let findings = vec![
            vuln(
                Severity::Critical,
                Confidence::Exact,
                Scope::Runtime,
                "a",
                "G1",
            ),
            vuln(Severity::Medium, Confidence::Range, Scope::Dev, "b", "G2"),
        ];
        let hygiene = HygieneInputs {
            has_lockfile: false,
            days_since_commit: Some(400),
            dirty: true,
        };
        let r = score(&findings, &hygiene, &Weights::default(), true);
        assert!(r.capped_by.is_none());
        let sum: f32 = r.breakdown.iter().map(|d| d.amount).sum();
        assert!(
            ((100 - r.score as i32) as f32 - sum).abs() <= 1.0,
            "sum {sum} vs 100-{}",
            r.score
        );
    }

    #[test]
    fn score_is_always_in_range_property() {
        // A crude sweep over many shapes.
        for n in 0..40 {
            let findings: Vec<_> = (0..n)
                .map(|i| {
                    vuln(
                        [Severity::Critical, Severity::High, Severity::Low][i % 3],
                        if i % 2 == 0 {
                            Confidence::Exact
                        } else {
                            Confidence::Range
                        },
                        Scope::Runtime,
                        &format!("p{}", i % 7),
                        &format!("G{i}"),
                    )
                })
                .collect();
            let r = score(&findings, &clean_hygiene(), &Weights::default(), true);
            assert!(r.score <= 100);
        }
    }

    #[test]
    fn suppressed_findings_do_not_count() {
        let mut f = vuln(
            Severity::Critical,
            Confidence::Exact,
            Scope::Runtime,
            "a",
            "G1",
        );
        f.suppressed = true;
        let r = score(&[f], &clean_hygiene(), &Weights::default(), true);
        assert_eq!(r.score, 100);
    }
}
