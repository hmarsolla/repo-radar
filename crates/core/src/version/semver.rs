//! SemVer scheme for npm and crates.io (DESIGN §8.5).

use std::cmp::Ordering;

use super::{mismatched_cmp, Ver, VersionError, VersionScheme};

pub struct SemVerScheme;

impl VersionScheme for SemVerScheme {
    fn name(&self) -> &'static str {
        "semver"
    }

    fn parse(&self, s: &str) -> Result<Ver, VersionError> {
        let trimmed = s.trim().trim_start_matches('=').trim();
        // npm and crates.io both tolerate a leading `v` in some contexts.
        let trimmed = trimmed.strip_prefix('v').unwrap_or(trimmed);
        ::semver::Version::parse(trimmed)
            .map(Ver::SemVer)
            .map_err(|e| VersionError {
                input: s.to_string(),
                scheme: "semver",
                detail: e.to_string(),
            })
    }

    fn cmp(&self, a: &Ver, b: &Ver) -> Ordering {
        match (a, b) {
            (Ver::SemVer(x), Ver::SemVer(y)) => x.cmp(y),
            _ => mismatched_cmp("semver"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Ver {
        SemVerScheme.parse(s).unwrap()
    }

    #[test]
    fn ordering_basics() {
        let s = SemVerScheme;
        assert!(s.lt(&v("1.2.3"), &v("1.2.4")));
        assert!(s.lt(&v("1.9.0"), &v("1.10.0")), "numeric, not lexical");
        assert!(s.gt(&v("2.0.0"), &v("1.999.999")));
        assert_eq!(s.cmp(&v("1.0.0"), &v("1.0.0")), Ordering::Equal);
    }

    #[test]
    fn prerelease_is_below_release() {
        let s = SemVerScheme;
        assert!(s.lt(&v("1.0.0-alpha"), &v("1.0.0")));
        assert!(s.lt(&v("1.0.0-alpha.1"), &v("1.0.0-alpha.2")));
        assert!(s.lt(&v("1.0.0-alpha"), &v("1.0.0-beta")));
    }

    #[test]
    fn build_metadata_does_not_affect_release_vs_prerelease() {
        // NB: the `semver` crate uses build metadata as a total-order
        // tiebreaker (a deliberate deviation from SemVer §10). That never
        // matters for OSV matching — range events and lockfile versions do
        // not carry `+meta` — but pin the behaviour so a crate bump that
        // changes it is noticed.
        let s = SemVerScheme;
        assert!(s.lt(&v("1.0.0-rc.1+a"), &v("1.0.0+a")));
        assert_eq!(s.cmp(&v("1.0.0+a"), &v("1.0.0+a")), Ordering::Equal);
    }

    #[test]
    fn leading_v_and_equals_are_tolerated() {
        SemVerScheme.parse("v1.2.3").unwrap();
        SemVerScheme.parse("=1.2.3").unwrap();
    }

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        let err = SemVerScheme.parse("not-a-version").unwrap_err();
        assert_eq!(err.scheme, "semver");
    }
}
