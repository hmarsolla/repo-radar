//! `go.mod` parser (DESIGN §7.3, PRD R6).
//!
//! **`go.mod`'s `require` block is the dependency set.** `go.sum` is *not*
//! used as an inventory — it holds hashes for modules considered during
//! resolution, often multiple versions of the same module, and using it
//! would systematically over-report. `// indirect` marks a transitive
//! requirement.

use crate::model::{Confidence, Dependency, Ecosystem, RelPath, Scope};
use crate::parsers::{
    normalize_package_name, LockfileParser, ManifestKind, ParseError, ParseOk, SiblingFiles,
};

const GO: Ecosystem = Ecosystem::Go;

pub struct GoModParser;

impl LockfileParser for GoModParser {
    fn ecosystem(&self) -> Ecosystem {
        GO
    }
    /// `go.mod` pins exact versions, so it is treated as a lockfile.
    fn kind(&self) -> ManifestKind {
        ManifestKind::Lockfile
    }
    fn matches_file(&self, file_name: &str) -> bool {
        file_name == "go.mod"
    }

    fn parse(
        &self,
        primary: &str,
        _path: &RelPath,
        _sibling: &SiblingFiles<'_>,
    ) -> Result<ParseOk, ParseError> {
        let mut out = Vec::new();
        let mut in_block = false;

        for raw in primary.lines() {
            let line = strip_line_comment_keeping_indirect(raw);
            let trimmed = line.trim();

            if in_block {
                if trimmed == ")" {
                    in_block = false;
                    continue;
                }
                if trimmed.is_empty() {
                    continue;
                }
                if let Some(d) = parse_require_entry(trimmed) {
                    out.push(d);
                }
                continue;
            }

            if let Some(rest) = trimmed.strip_prefix("require") {
                let rest = rest.trim();
                if rest == "(" {
                    in_block = true;
                } else if !rest.is_empty() {
                    // single-line: `require module/path v1.2.3`
                    if let Some(d) = parse_require_entry(rest) {
                        out.push(d);
                    }
                }
            }
        }

        if out.is_empty() {
            return Err(ParseError::malformed(
                "go.mod",
                "no `require` directives found",
            ));
        }
        Ok(out.into())
    }
}

/// Keep an `// indirect` marker but drop any other trailing comment.
fn strip_line_comment_keeping_indirect(line: &str) -> String {
    match line.find("//") {
        Some(i) => {
            let (code, comment) = line.split_at(i);
            if comment.contains("indirect") {
                format!("{code}// indirect")
            } else {
                code.to_string()
            }
        }
        None => line.to_string(),
    }
}

fn parse_require_entry(s: &str) -> Option<Dependency> {
    let indirect = s.contains("// indirect");
    let mut parts = s.split_whitespace();
    let path = parts.next()?;
    let version = parts.next()?;
    if !version.starts_with('v') {
        return None; // not a version token
    }
    Some(Dependency {
        ecosystem: GO,
        name: normalize_package_name(GO, path),
        raw_name: path.to_string(),
        version: version.to_string(),
        confidence: Confidence::Exact,
        scope: Scope::Runtime, // Go has no dev/build dependency distinction
        is_direct: !indirect,
        manifest_path: RelPath::new(""),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parsers::ParserRegistry;
    use std::collections::BTreeMap;

    #[test]
    fn require_block_with_indirect_and_gosum_ignored() {
        let go_mod = r#"
module github.com/acme/demo

go 1.22

require (
	github.com/gin-gonic/gin v1.9.1
	github.com/stretchr/testify v1.9.0
	golang.org/x/sys v0.18.0 // indirect
)

require github.com/spf13/cobra v1.8.0
"#;
        // go.sum lists a superseded gin version — must not appear.
        let go_sum =
            "github.com/gin-gonic/gin v1.8.0 h1:xxxx=\ngithub.com/gin-gonic/gin v1.9.1 h1:yyyy=\n";

        let reg = ParserRegistry::builtin();
        let files: BTreeMap<String, String> = [("go.mod", go_mod), ("go.sum", go_sum)]
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let out = reg.parse_dir("", &files);
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);

        assert_eq!(out.deps.len(), 4);
        let gin = out.deps.iter().find(|d| d.name.ends_with("/gin")).unwrap();
        assert_eq!(gin.version, "v1.9.1", "go.mod wins, not the go.sum entries");
        assert!(gin.is_direct);

        let sys = out.deps.iter().find(|d| d.name.contains("x/sys")).unwrap();
        assert!(!sys.is_direct, "// indirect");

        let cobra = out.deps.iter().find(|d| d.name.contains("cobra")).unwrap();
        assert_eq!(cobra.version, "v1.8.0");
        assert!(cobra.is_direct);

        assert!(out.deps.iter().all(|d| d.confidence == Confidence::Exact));
    }
}
