//! The matcher (DESIGN §8.4) — a **pure function** from a dependency and its
//! candidate advisories to findings. No database access: SQL narrows
//! candidates by exact `(ecosystem, package_name)`, then this code walks
//! each range's event list in ecosystem-correct version order and decides.
//!
//! Three traps it guards against (§8.4):
//! - An `introduced` with no `fixed` means *everything from that version on*
//!   is affected — naive interval logic that needs an upper bound misses these.
//! - `last_affected` is inclusive; `fixed` is exclusive.
//! - Events must sort with the ecosystem's comparator, not lexically
//!   (`1.9.0` before `1.10.0`).

use std::cmp::Ordering;

use crate::model::{Ecosystem, FindingKind};
use crate::osv::record::{NormalizedAffected, NormalizedRange, RangeEvent, RangeType};
use crate::version::{scheme_for, Ver, VersionError, VersionScheme};

/// The dependency being checked.
#[derive(Debug, Clone, Copy)]
pub struct Target<'a> {
    pub ecosystem: Ecosystem,
    /// Normalized name (FR-4.5) — must already match `affected.package_name`.
    pub name: &'a str,
    pub version: &'a str,
}

/// One advisory's `affected[]` entry for this package, plus enough of the
/// advisory to build a finding.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub advisory_id: String,
    pub kind: FindingKind,
    pub affected: NormalizedAffected,
}

/// The result of checking one dependency against one candidate.
#[derive(Debug, Clone, PartialEq)]
pub enum Match {
    /// The dependency's version is inside an affected interval, or listed
    /// explicitly. `fixed_version` is the nearest known fix above the
    /// current version, if the advisory names one.
    Affected { fixed_version: Option<String> },
    /// Checked and clear.
    NotAffected,
    /// The dependency's own version string will not parse in this
    /// ecosystem's scheme — "we could not check this" (DESIGN §8.5). The
    /// caller raises a `Warning` and marks the dependency unmatchable.
    Unmatchable(VersionError),
}

/// Check one dependency against one candidate advisory.
pub fn evaluate(target: &Target<'_>, candidate: &Candidate) -> Match {
    let scheme = scheme_for(target.ecosystem);

    // 1. Explicit version enumeration — exact match, with a parsed-equality
    //    fallback so `v1.0.0` vs `1.0.0` still lands.
    let explicit_hit = candidate
        .affected
        .versions
        .iter()
        .any(|v| v == target.version || parsed_equal(scheme, v, target.version));
    if explicit_hit {
        return Match::Affected {
            fixed_version: None,
        };
    }

    // 2. Event-walk each range.
    let dep_ver = match scheme.parse(target.version) {
        Ok(v) => v,
        Err(e) => return Match::Unmatchable(e),
    };

    let mut affected = false;
    let mut best_fix: Option<Ver> = None;
    for range in &candidate.affected.ranges {
        if range.range_type == RangeType::Git {
            continue; // commit hashes, not versions
        }
        let (hit, fix) = walk_range(scheme, &dep_ver, range);
        if hit {
            affected = true;
        }
        if let Some(f) = fix {
            best_fix = Some(match best_fix {
                Some(cur) if scheme.lt(&cur, &f) => cur,
                _ => f,
            });
        }
    }

    if affected {
        Match::Affected {
            fixed_version: best_fix.map(|v| render(scheme, &v)),
        }
    } else {
        Match::NotAffected
    }
}

/// Walk one range's events in ascending version order and decide whether
/// `dep_ver` sits in an affected interval. Also returns the smallest `fixed`
/// bound strictly above `dep_ver`, for the finding's `fixed_version`.
fn walk_range(
    scheme: &dyn VersionScheme,
    dep_ver: &Ver,
    range: &NormalizedRange,
) -> (bool, Option<Ver>) {
    // (bound, event) pairs; `introduced: "0"` sorts as -infinity.
    let mut events: Vec<(Bound, &RangeEvent)> = Vec::with_capacity(range.events.len());
    for e in &range.events {
        let raw = match e {
            RangeEvent::Introduced(v)
            | RangeEvent::Fixed(v)
            | RangeEvent::LastAffected(v)
            | RangeEvent::Limit(v) => v,
        };
        if raw == "0" {
            events.push((Bound::NegInf, e));
        } else if let Ok(v) = scheme.parse(raw) {
            events.push((Bound::At(v), e));
        }
        // An unparseable event bound is dropped: we cannot reason about it,
        // and inventing an ordering would risk a false finding.
    }
    events.sort_by(|(a, _), (b, _)| cmp_bound(scheme, a, b));

    let mut affected = false;
    let mut nearest_fix: Option<Ver> = None;

    for (bound, event) in &events {
        let at_or_before_target = match bound {
            Bound::NegInf => true,
            Bound::At(v) => !scheme.gt(v, dep_ver), // v <= dep_ver
        };

        match event {
            RangeEvent::Introduced(_) if at_or_before_target => affected = true,
            RangeEvent::Fixed(_) if at_or_before_target => affected = false,
            RangeEvent::LastAffected(_) => {
                if let Bound::At(v) = bound {
                    if scheme.gt(dep_ver, v) {
                        affected = false;
                    }
                }
            }
            RangeEvent::Limit(_) if at_or_before_target => affected = false,
            _ => {}
        }

        // Track the smallest fix strictly above the current version.
        if let (RangeEvent::Fixed(_), Bound::At(v)) = (event, bound) {
            if scheme.gt(v, dep_ver) {
                nearest_fix = Some(match nearest_fix {
                    Some(cur) if scheme.lt(&cur, v) => cur,
                    _ => v.clone(),
                });
            }
        }
    }

    (affected, nearest_fix)
}

enum Bound {
    NegInf,
    At(Ver),
}

fn cmp_bound(scheme: &dyn VersionScheme, a: &Bound, b: &Bound) -> Ordering {
    match (a, b) {
        (Bound::NegInf, Bound::NegInf) => Ordering::Equal,
        (Bound::NegInf, _) => Ordering::Less,
        (_, Bound::NegInf) => Ordering::Greater,
        (Bound::At(x), Bound::At(y)) => scheme.cmp(x, y),
    }
}

fn parsed_equal(scheme: &dyn VersionScheme, a: &str, b: &str) -> bool {
    match (scheme.parse(a), scheme.parse(b)) {
        (Ok(x), Ok(y)) => scheme.cmp(&x, &y) == Ordering::Equal,
        _ => false,
    }
}

/// Best-effort render of a parsed version back to a string for display.
fn render(_scheme: &dyn VersionScheme, v: &Ver) -> String {
    match v {
        Ver::SemVer(x) => x.to_string(),
        Ver::Pep440(x) => x.to_string(),
        Ver::Go(x) => {
            let base = x.semver.to_string();
            if x.incompatible {
                format!("v{base}+incompatible")
            } else {
                format!("v{base}")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::osv::record::{NormalizedRange, RangeType};

    fn range(events: Vec<RangeEvent>) -> NormalizedRange {
        NormalizedRange {
            range_type: RangeType::Semver,
            events,
        }
    }

    fn candidate(ranges: Vec<NormalizedRange>, versions: Vec<&str>) -> Candidate {
        Candidate {
            advisory_id: "GHSA-test".into(),
            kind: FindingKind::Vulnerability,
            affected: NormalizedAffected {
                ecosystem: Ecosystem::Npm,
                package_name: "pkg".into(),
                ranges,
                versions: versions.into_iter().map(String::from).collect(),
            },
        }
    }

    fn target(version: &str) -> Target<'_> {
        Target {
            ecosystem: Ecosystem::Npm,
            name: "pkg",
            version,
        }
    }

    // --- DESIGN §16.2 suite ---------------------------------------------

    #[test]
    fn introduced_with_no_fixed_affects_everything_onward() {
        let c = candidate(
            vec![range(vec![RangeEvent::Introduced("1.0.0".into())])],
            vec![],
        );
        assert_eq!(evaluate(&target("0.9.0"), &c), Match::NotAffected);
        assert!(matches!(
            evaluate(&target("1.0.0"), &c),
            Match::Affected { .. }
        ));
        assert!(matches!(
            evaluate(&target("99.0.0"), &c),
            Match::Affected { .. }
        ));
    }

    #[test]
    fn introduced_zero_and_fixed() {
        let c = candidate(
            vec![range(vec![
                RangeEvent::Introduced("0".into()),
                RangeEvent::Fixed("1.2.3".into()),
            ])],
            vec![],
        );
        assert!(matches!(
            evaluate(&target("1.2.2"), &c),
            Match::Affected { fixed_version: Some(f) } if f == "1.2.3"
        ));
        assert_eq!(
            evaluate(&target("1.2.3"), &c),
            Match::NotAffected,
            "fixed is exclusive"
        );
        assert_eq!(evaluate(&target("2.0.0"), &c), Match::NotAffected);
    }

    #[test]
    fn last_affected_is_inclusive_unlike_fixed() {
        let c = candidate(
            vec![range(vec![
                RangeEvent::Introduced("1.0.0".into()),
                RangeEvent::LastAffected("1.2.3".into()),
            ])],
            vec![],
        );
        assert!(
            matches!(evaluate(&target("1.2.3"), &c), Match::Affected { .. }),
            "last_affected includes 1.2.3"
        );
        assert_eq!(evaluate(&target("1.2.4"), &c), Match::NotAffected);
    }

    #[test]
    fn multiple_disjoint_intervals_in_one_range() {
        let c = candidate(
            vec![range(vec![
                RangeEvent::Introduced("1.0.0".into()),
                RangeEvent::Fixed("1.2.0".into()),
                RangeEvent::Introduced("2.0.0".into()),
                RangeEvent::Fixed("2.3.0".into()),
            ])],
            vec![],
        );
        assert!(matches!(
            evaluate(&target("1.1.0"), &c),
            Match::Affected { .. }
        ));
        assert_eq!(
            evaluate(&target("1.5.0"), &c),
            Match::NotAffected,
            "in the gap"
        );
        assert!(matches!(
            evaluate(&target("2.1.0"), &c),
            Match::Affected { .. }
        ));
        assert_eq!(evaluate(&target("2.3.0"), &c), Match::NotAffected);
    }

    #[test]
    fn explicit_versions_with_no_ranges() {
        let c = candidate(vec![], vec!["1.0.1", "1.0.3"]);
        assert!(matches!(
            evaluate(&target("1.0.1"), &c),
            Match::Affected {
                fixed_version: None
            }
        ));
        assert_eq!(evaluate(&target("1.0.2"), &c), Match::NotAffected);
    }

    #[test]
    fn explicit_version_parsed_equality_fallback() {
        // Go-style: advisory lists "1.0.0", dependency is "v1.0.0".
        let c = Candidate {
            advisory_id: "MAL-x".into(),
            kind: FindingKind::Compromise,
            affected: NormalizedAffected {
                ecosystem: Ecosystem::Go,
                package_name: "github.com/foo/bar".into(),
                ranges: vec![],
                versions: vec!["1.0.0".into()],
            },
        };
        let t = Target {
            ecosystem: Ecosystem::Go,
            name: "github.com/foo/bar",
            version: "v1.0.0",
        };
        assert!(matches!(evaluate(&t, &c), Match::Affected { .. }));
    }

    #[test]
    fn events_sort_by_version_not_lexically() {
        // If sorted lexically, "1.10.0" < "1.9.0" and the walk breaks.
        let c = candidate(
            vec![range(vec![
                RangeEvent::Introduced("1.9.0".into()),
                RangeEvent::Fixed("1.10.0".into()),
            ])],
            vec![],
        );
        assert!(matches!(
            evaluate(&target("1.9.5"), &c),
            Match::Affected { .. }
        ));
        assert_eq!(evaluate(&target("1.10.0"), &c), Match::NotAffected);
        assert_eq!(evaluate(&target("1.8.0"), &c), Match::NotAffected);
    }

    #[test]
    fn pep440_specifics_in_a_range() {
        let scheme_eco = Ecosystem::PyPI;
        let c = Candidate {
            advisory_id: "GHSA-py".into(),
            kind: FindingKind::Vulnerability,
            affected: NormalizedAffected {
                ecosystem: scheme_eco,
                package_name: "pkg".into(),
                ranges: vec![NormalizedRange {
                    range_type: RangeType::Ecosystem,
                    events: vec![
                        RangeEvent::Introduced("1.0.0".into()),
                        RangeEvent::Fixed("1.0.0.post1".into()),
                    ],
                }],
                versions: vec![],
            },
        };
        let aff = |v| Target {
            ecosystem: scheme_eco,
            name: "pkg",
            version: v,
        };
        // rc1 < 1.0.0, so not in [1.0.0, 1.0.0.post1)
        assert_eq!(evaluate(&aff("1.0.0rc1"), &c), Match::NotAffected);
        assert!(matches!(
            evaluate(&aff("1.0.0"), &c),
            Match::Affected { .. }
        ));
        // post1 is the fix boundary (exclusive)
        assert_eq!(evaluate(&aff("1.0.0.post1"), &c), Match::NotAffected);
    }

    #[test]
    fn unparseable_dependency_version_is_unmatchable() {
        let c = candidate(
            vec![range(vec![RangeEvent::Introduced("0".into())])],
            vec![],
        );
        assert!(matches!(
            evaluate(&target("not-a-version"), &c),
            Match::Unmatchable(_)
        ));
    }

    #[test]
    fn range_and_explicit_version_together() {
        // npm MAL- records sometimes carry both.
        let c = candidate(
            vec![range(vec![RangeEvent::Introduced("0".into())])],
            vec!["1.0.0"],
        );
        assert!(matches!(
            evaluate(&target("1.0.0"), &c),
            Match::Affected { .. }
        ));
        assert!(matches!(
            evaluate(&target("5.0.0"), &c),
            Match::Affected { .. }
        ));
    }
}
