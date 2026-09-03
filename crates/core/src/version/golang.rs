//! Go module version ordering (DESIGN §8.5, DESIGN D5).
//!
//! Go versions are SemVer with a `v` prefix, `+incompatible` build tags, and
//! pseudo-versions (`v0.0.0-20210101000000-abcdef123456`). We normalise the
//! `v` prefix and `+incompatible` away and lean on SemVer's own prerelease
//! comparison, which orders pseudo-versions correctly because the timestamp
//! is a numeric prerelease identifier. If a real fixture ever shows a
//! mis-ordering, this is where a bespoke comparator goes (D5).

use std::cmp::Ordering;

use super::{mismatched_cmp, Ver, VersionError, VersionScheme};

/// A parsed Go version. Wraps a `semver::Version` after normalisation.
#[derive(Debug, Clone)]
pub struct GoVer {
    pub semver: ::semver::Version,
    /// True for a `vX.Y.Z+incompatible` tag (major ≥ 2 without a `/vN` module
    /// path). Ordered *below* the same base with no `+incompatible`, matching
    /// `go`'s behaviour.
    pub incompatible: bool,
}

pub struct GoScheme;

impl GoScheme {
    fn parse_gover(s: &str) -> Result<GoVer, VersionError> {
        let raw = s.trim();
        let raw = raw.strip_prefix('v').unwrap_or(raw);
        let (core, incompatible) = match raw.strip_suffix("+incompatible") {
            Some(base) => (base, true),
            None => (raw, false),
        };
        // Drop any remaining build metadata; `go` ignores it for ordering.
        let core = core.split('+').next().unwrap_or(core);

        let semver = ::semver::Version::parse(core).or_else(|_| {
            // Bare `1` or `1.2` — pad to a full triple.
            let padded = pad_to_triple(core);
            ::semver::Version::parse(&padded)
        });

        semver
            .map(|semver| GoVer {
                semver,
                incompatible,
            })
            .map_err(|e| VersionError {
                input: s.to_string(),
                scheme: "go",
                detail: e.to_string(),
            })
    }
}

fn pad_to_triple(s: &str) -> String {
    let (numeric, rest) = match s.find(['-', '+']) {
        Some(i) => (&s[..i], &s[i..]),
        None => (s, ""),
    };
    let parts: Vec<&str> = numeric.split('.').collect();
    let mut out = String::new();
    for i in 0..3 {
        if i > 0 {
            out.push('.');
        }
        out.push_str(parts.get(i).copied().unwrap_or("0"));
    }
    out.push_str(rest);
    out
}

impl VersionScheme for GoScheme {
    fn name(&self) -> &'static str {
        "go"
    }

    fn parse(&self, s: &str) -> Result<Ver, VersionError> {
        Self::parse_gover(s).map(Ver::Go)
    }

    fn cmp(&self, a: &Ver, b: &Ver) -> Ordering {
        match (a, b) {
            (Ver::Go(x), Ver::Go(y)) => x
                .semver
                .cmp(&y.semver)
                // +incompatible sorts below the same base without it.
                .then_with(|| y.incompatible.cmp(&x.incompatible)),
            _ => mismatched_cmp("go"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Ver {
        GoScheme.parse(s).unwrap()
    }

    #[test]
    fn v_prefix_and_triples() {
        let s = GoScheme;
        assert!(s.lt(&v("v1.2.3"), &v("v1.2.4")));
        assert!(s.lt(&v("v1.9.0"), &v("v1.10.0")));
        assert_eq!(s.cmp(&v("v1"), &v("v1.0.0")), Ordering::Equal);
    }

    #[test]
    fn pseudo_versions_order_by_timestamp() {
        let s = GoScheme;
        let older = v("v0.0.0-20210101000000-aaaaaaaaaaaa");
        let newer = v("v0.0.0-20211231235959-bbbbbbbbbbbb");
        assert!(s.lt(&older, &newer), "pseudo-versions compare by timestamp");
        // A real tagged release outranks a pseudo-version of 0.0.0.
        assert!(s.gt(&v("v0.1.0"), &newer));
    }

    #[test]
    fn incompatible_sorts_below_plain() {
        let s = GoScheme;
        assert!(s.lt(&v("v2.0.0+incompatible"), &v("v2.0.0")));
        assert!(s.lt(&v("v2.0.0+incompatible"), &v("v2.0.1+incompatible")));
    }

    #[test]
    fn prerelease_below_release() {
        assert!(GoScheme.lt(&v("v1.0.0-rc.1"), &v("v1.0.0")));
    }

    #[test]
    fn garbage_is_an_error() {
        assert_eq!(GoScheme.parse("latest").unwrap_err().scheme, "go");
    }
}
