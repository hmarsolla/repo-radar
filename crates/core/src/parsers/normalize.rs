//! Package-name normalization (FR-4.5, DESIGN §7.4).
//!
//! This function is applied **identically** to `dependencies.name` and to
//! `affected_ranges.package_name`, so the SQL join in §8.4 is a plain
//! equality. It is the single point where a mismatch silently produces
//! false negatives, so every ecosystem's rule is table-tested.

use crate::model::Ecosystem;

/// Normalize `raw` for `ecosystem`. Idempotent.
pub fn normalize_package_name(ecosystem: Ecosystem, raw: &str) -> String {
    let raw = raw.trim();
    match ecosystem {
        // PEP 503: lowercase, and collapse any run of `-`, `_`, `.` to a
        // single `-`.
        Ecosystem::PyPI => {
            let mut out = String::with_capacity(raw.len());
            let mut in_sep = false;
            for ch in raw.chars() {
                if matches!(ch, '-' | '_' | '.') {
                    in_sep = true;
                } else {
                    if in_sep && !out.is_empty() {
                        out.push('-');
                    }
                    in_sep = false;
                    out.extend(ch.to_lowercase());
                }
            }
            out
        }
        // npm: lowercase; the `@scope/` prefix is kept verbatim in shape
        // (still lowercased — npm names are case-insensitive).
        Ecosystem::Npm => raw.to_lowercase(),
        // crates.io: lowercase only. `-` and `_` are **distinct** — collapsing
        // them would merge genuinely different crates.
        Ecosystem::CratesIo => raw.to_lowercase(),
        // Go: verbatim, case-sensitive, including `/v2`+ major suffixes.
        Ecosystem::Go => raw.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Ecosystem::*;

    #[test]
    fn pypi_pep503() {
        for (input, want) in [
            ("Flask", "flask"),
            ("Django-REST-Framework", "django-rest-framework"),
            ("zope.interface", "zope-interface"),
            ("a_b__c---d", "a-b-c-d"),
            ("ruamel.yaml.clib", "ruamel-yaml-clib"),
            ("already-normal", "already-normal"),
        ] {
            assert_eq!(normalize_package_name(PyPI, input), want, "{input}");
            // idempotent
            let once = normalize_package_name(PyPI, input);
            assert_eq!(normalize_package_name(PyPI, &once), once);
        }
    }

    #[test]
    fn npm_lowercase_keeps_scope() {
        assert_eq!(normalize_package_name(Npm, "React"), "react");
        assert_eq!(
            normalize_package_name(Npm, "@Babel/Core"),
            "@babel/core",
            "scope prefix preserved in shape"
        );
        assert_eq!(
            normalize_package_name(Npm, "lodash.merge"),
            "lodash.merge",
            "npm does not collapse dots"
        );
    }

    #[test]
    fn crates_io_keeps_dash_underscore_distinct() {
        assert_eq!(normalize_package_name(CratesIo, "Serde"), "serde");
        assert_eq!(normalize_package_name(CratesIo, "foo-bar"), "foo-bar");
        assert_eq!(normalize_package_name(CratesIo, "foo_bar"), "foo_bar");
        assert_ne!(
            normalize_package_name(CratesIo, "foo-bar"),
            normalize_package_name(CratesIo, "foo_bar"),
            "these are different crates and must stay different"
        );
    }

    #[test]
    fn go_is_verbatim() {
        assert_eq!(
            normalize_package_name(Go, "github.com/Masterminds/semver/v3"),
            "github.com/Masterminds/semver/v3",
        );
        assert_ne!(
            normalize_package_name(Go, "github.com/foo/Bar"),
            normalize_package_name(Go, "github.com/foo/bar"),
            "Go module paths are case-sensitive"
        );
    }
}
