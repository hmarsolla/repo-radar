//! PEP 440 scheme for PyPI (DESIGN §8.5, DESIGN D5).
//!
//! Handles `rc`/`a`/`b` pre-releases, `.post`, `.dev`, local versions, and
//! epoch prefixes (`1!2.0`). Using SemVer here would mis-order all of them.

use std::cmp::Ordering;
use std::str::FromStr;

use super::{mismatched_cmp, Ver, VersionError, VersionScheme};

pub struct Pep440Scheme;

impl VersionScheme for Pep440Scheme {
    fn name(&self) -> &'static str {
        "pep440"
    }

    fn parse(&self, s: &str) -> Result<Ver, VersionError> {
        let trimmed = s.trim().trim_start_matches("==").trim();
        ::pep440_rs::Version::from_str(trimmed)
            .map(Ver::Pep440)
            .map_err(|e| VersionError {
                input: s.to_string(),
                scheme: "pep440",
                detail: e.to_string(),
            })
    }

    fn cmp(&self, a: &Ver, b: &Ver) -> Ordering {
        match (a, b) {
            (Ver::Pep440(x), Ver::Pep440(y)) => x.cmp(y),
            _ => mismatched_cmp("pep440"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Ver {
        Pep440Scheme.parse(s).unwrap()
    }

    #[test]
    fn pre_release_ordering() {
        let s = Pep440Scheme;
        // rc < final
        assert!(s.lt(&v("1.0.0rc1"), &v("1.0.0")));
        // a < b < rc
        assert!(s.lt(&v("1.0.0a1"), &v("1.0.0b1")));
        assert!(s.lt(&v("1.0.0b1"), &v("1.0.0rc1")));
        // dev < a
        assert!(s.lt(&v("1.0.0.dev1"), &v("1.0.0a1")));
    }

    #[test]
    fn post_release_is_above_final() {
        let s = Pep440Scheme;
        assert!(s.gt(&v("1.0.0.post1"), &v("1.0.0")));
        assert!(s.lt(&v("1.0.0"), &v("1.0.0.post1")));
        assert!(s.lt(&v("1.0.0.post1"), &v("1.0.1")));
    }

    #[test]
    fn epoch_dominates() {
        let s = Pep440Scheme;
        // Any epoch > no epoch, and a higher epoch wins outright.
        assert!(s.gt(&v("1!1.0"), &v("999.0")));
        assert!(s.gt(&v("2!0.0"), &v("1!9.9")));
    }

    #[test]
    fn numeric_not_lexical() {
        assert!(Pep440Scheme.lt(&v("1.9"), &v("1.10")));
    }

    #[test]
    fn double_equals_prefix_tolerated() {
        Pep440Scheme.parse("==1.2.3").unwrap();
    }

    #[test]
    fn garbage_is_an_error() {
        assert_eq!(Pep440Scheme.parse("~~~").unwrap_err().scheme, "pep440");
    }
}
