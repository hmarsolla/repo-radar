//! Submodule handling (FR-1.5, DESIGN §6.6).
//!
//! The discovery walk stops at a repository's `.git`, so a submodule's own
//! repository is never reached by traversal. Instead it is attached here as
//! a *child* of its parent: it shows in the parent's detail view, never as a
//! top-level row, and its dependencies roll into the parent's `repo_id`.

use std::path::Path;

use crate::model::RepoIdentity;

/// Read the parent's `.gitmodules` (via libgit2) and return one
/// [`RepoIdentity`] per declared submodule, each with `parent_path` set.
/// A parent with no `.gitmodules`, or one that cannot be opened, yields an
/// empty list rather than an error — a missing submodule is not a scan
/// failure.
pub fn attach_submodules(parent: &RepoIdentity) -> Vec<RepoIdentity> {
    if parent.is_bare {
        return Vec::new();
    }
    let Ok(repo) = git2::Repository::open(&parent.path) else {
        return Vec::new();
    };
    let Ok(submodules) = repo.submodules() else {
        return Vec::new();
    };

    submodules
        .iter()
        .filter_map(|sm| {
            let rel = sm.path().to_str()?.trim_end_matches('/').to_string();
            if rel.is_empty() {
                return None;
            }
            let abs = join_rel(&parent.path, &rel);
            let name = sm
                .name()
                .map(|n| n.rsplit('/').next().unwrap_or(n).to_string())
                .unwrap_or_else(|_| {
                    Path::new(&rel)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or(&rel)
                        .to_string()
                });
            Some(RepoIdentity {
                path: abs,
                name,
                is_bare: false,
                parent_path: Some(parent.path.clone()),
            })
        })
        .collect()
}

/// Join a repo-relative submodule path onto the parent's normalised
/// (forward-slash) absolute path.
fn join_rel(parent_path: &str, rel: &str) -> String {
    let base = parent_path.trim_end_matches('/');
    let rel = rel.trim_start_matches("./").replace('\\', "/");
    format!("{base}/{rel}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_git_dir_yields_empty() {
        let id = RepoIdentity {
            path: "/nonexistent/repo".into(),
            name: "repo".into(),
            is_bare: false,
            parent_path: None,
        };
        assert!(attach_submodules(&id).is_empty());
    }

    #[test]
    fn join_rel_forms_forward_slash_paths() {
        assert_eq!(join_rel("/a/b/", "vendor/x"), "/a/b/vendor/x");
        assert_eq!(join_rel("/a/b", "./vendor/x"), "/a/b/vendor/x");
    }
}
