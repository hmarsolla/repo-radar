//! Template catalog (M4-2 built-ins, M4-3 user templates).
//!
//! Built-in templates are embedded from `assets/prompts/*.j2`. User templates
//! are loaded from `<config>/prompts/*.j2`. Both are handed to the same
//! [`super::render::render`] call — there is no privileged built-in path
//! (DESIGN §11.1). A user template whose file stem collides with a built-in
//! id shadows it, the same way a user rule pack overrides a shipped rule.

use std::path::Path;

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::error::CoreResult;
use crate::paths::Paths;

/// How many repos a template expects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum RepoArity {
    /// Exactly one repo (T2, T3).
    Single,
    /// Two or more repos (T1).
    Multi,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum TemplateSource {
    BuiltIn,
    User,
}

/// One entry in the template picker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TemplateInfo {
    /// Stable id — the file stem. Passed back to [`load_source`].
    pub id: String,
    pub name: String,
    pub description: String,
    pub source: TemplateSource,
    pub arity: RepoArity,
    /// `true` when this template embeds file bodies (T2/T3) and therefore
    /// drives the file picker; `false` for metadata-only comparisons (T1).
    pub uses_files: bool,
}

struct BuiltIn {
    id: &'static str,
    name: &'static str,
    description: &'static str,
    arity: RepoArity,
    uses_files: bool,
    body: &'static str,
}

const BUILTINS: &[BuiltIn] = &[
    BuiltIn {
        id: "cross_repo_similarity",
        name: "Cross-repo similarity",
        description: "Compare several repositories for overlapping dependencies, \
                      duplicated functionality, and consolidation opportunities.",
        arity: RepoArity::Multi,
        uses_files: false,
        body: include_str!("../../assets/prompts/cross_repo_similarity.j2"),
    },
    BuiltIn {
        id: "perf_security_opportunities",
        name: "Performance & security opportunities",
        description: "Review one repository for performance and security improvements, \
                      with findings separated into confirmed and speculative.",
        arity: RepoArity::Single,
        uses_files: true,
        body: include_str!("../../assets/prompts/perf_security_opportunities.j2"),
    },
    BuiltIn {
        id: "code_review",
        name: "Code review",
        description: "A focused code review of one repository, scoped to the whole \
                      tree, a directory, a file list, or a diff.",
        arity: RepoArity::Single,
        uses_files: true,
        body: include_str!("../../assets/prompts/code_review.j2"),
    },
];

/// Every template available: built-ins first, then user templates sorted by
/// id. A user template shadows a built-in with the same id.
pub fn list(paths: &Paths) -> CoreResult<Vec<TemplateInfo>> {
    let mut out: Vec<TemplateInfo> = BUILTINS
        .iter()
        .map(|b| TemplateInfo {
            id: b.id.to_string(),
            name: b.name.to_string(),
            description: b.description.to_string(),
            source: TemplateSource::BuiltIn,
            arity: b.arity,
            uses_files: b.uses_files,
        })
        .collect();

    let mut user = user_templates(&paths.prompts_dir())?;
    user.sort_by(|a, b| a.id.cmp(&b.id));
    for u in user {
        if let Some(existing) = out.iter_mut().find(|t| t.id == u.id) {
            *existing = u; // user file shadows the built-in
        } else {
            out.push(u);
        }
    }
    Ok(out)
}

/// The raw template body for `id`. A user file wins over a built-in of the
/// same id.
pub fn load_source(paths: &Paths, id: &str) -> CoreResult<String> {
    let user_path = paths.prompts_dir().join(format!("{id}.j2"));
    if user_path.is_file() {
        return Ok(std::fs::read_to_string(&user_path)?);
    }
    if let Some(b) = BUILTINS.iter().find(|b| b.id == id) {
        return Ok(b.body.to_string());
    }
    Err(crate::error::CoreError::Prompt(format!(
        "no template named {id:?}"
    )))
}

fn user_templates(dir: &Path) -> CoreResult<Vec<TemplateInfo>> {
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(e.into()),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("j2") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        // A user template's arity/uses_files is not declared anywhere, so
        // assume the least restrictive: single repo, offers the file picker.
        // The user can select one repo and any files; the template ignores
        // what it does not reference.
        out.push(TemplateInfo {
            id: stem.to_string(),
            name: stem.replace(['_', '-'], " "),
            description: format!("User template · {}", path.display()),
            source: TemplateSource::User,
            arity: RepoArity::Single,
            uses_files: true,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_ins_are_listed_and_loadable() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::under(tmp.path());
        paths.ensure_dirs().unwrap();

        let list = list(&paths).unwrap();
        assert_eq!(list.len(), 3);
        assert!(list.iter().all(|t| t.source == TemplateSource::BuiltIn));

        for t in &list {
            let src = load_source(&paths, &t.id).unwrap();
            assert!(!src.trim().is_empty(), "{} is empty", t.id);
        }
    }

    #[test]
    fn user_template_appears_and_can_shadow_a_built_in() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::under(tmp.path());
        paths.ensure_dirs().unwrap();

        std::fs::write(
            paths.prompts_dir().join("my_review.j2"),
            "hi {{ repos | length }}",
        )
        .unwrap();
        std::fs::write(
            paths.prompts_dir().join("code_review.j2"),
            "shadowed {{ scope.kind }}",
        )
        .unwrap();

        let list = list(&paths).unwrap();
        let mine = list.iter().find(|t| t.id == "my_review").unwrap();
        assert_eq!(mine.source, TemplateSource::User);
        assert_eq!(mine.name, "my review");

        let cr = list.iter().find(|t| t.id == "code_review").unwrap();
        assert_eq!(cr.source, TemplateSource::User);
        assert_eq!(
            load_source(&paths, "code_review").unwrap(),
            "shadowed {{ scope.kind }}"
        );

        // Still just 3 ids — the shadow replaced, not appended.
        assert_eq!(list.len(), 4);
    }

    #[test]
    fn unknown_id_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::under(tmp.path());
        paths.ensure_dirs().unwrap();
        assert!(load_source(&paths, "nope").is_err());
    }
}
