//! File selection and embedding (M4-4, FR-9.3, DESIGN §11.2).
//!
//! The user picks files from a checkbox tree. Everything that is *not*
//! offered is still listed, with the reason it was withheld — a binary file
//! that happens to end in `.txt` is excluded and says why, rather than
//! silently vanishing.
//!
//! Exclusion reasons, checked in this order: pruned directory, gitignored,
//! extension-excluded (a user setting, FR-10.1), oversized, binary. Binary is
//! decided by **content**, not by extension: the first 8 KiB is read and a
//! NUL byte (or invalid UTF-8, since the body is embedded as a string)
//! condemns the file.
//!
//! Only the repository's top-level `.gitignore` and `.git/info/exclude` are
//! consulted in v1; nested `.gitignore` files are not fully honored.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use ignore::gitignore::{Gitignore, GitignoreBuilder};
use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::error::{CoreError, CoreResult};
use crate::prompt::context::EmbeddedFile;
use crate::scan::discovery::DEFAULT_PRUNE_DIRS;

/// Default per-file size ceiling (FR-9.3). Files above this are listed but
/// not offered for inclusion.
pub const DEFAULT_MAX_FILE_BYTES: u64 = 256 * 1024;

/// How many bytes to sniff when deciding whether a file is binary.
const SNIFF_BYTES: usize = 8192;

/// A hard cap on how many entries the tree returns, so a pathological repo
/// cannot exhaust memory building the picker (DESIGN §17).
const DEFAULT_MAX_ENTRIES: usize = 20_000;

#[derive(Debug, Clone)]
pub struct SelectionOptions {
    pub max_file_bytes: u64,
    /// Directory names never walked into (FR-1.4). `.git` is always added.
    pub prune_dirs: Vec<String>,
    /// Extensions (no dot, lowercase) withheld from selection in addition to
    /// the content-sniffed binary check (FR-10.1). Empty by default.
    pub excluded_extensions: Vec<String>,
    pub max_entries: usize,
}

impl Default for SelectionOptions {
    fn default() -> Self {
        Self {
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            prune_dirs: DEFAULT_PRUNE_DIRS.iter().map(|s| s.to_string()).collect(),
            excluded_extensions: Vec::new(),
            max_entries: DEFAULT_MAX_ENTRIES,
        }
    }
}

/// Why a path is not offered for inclusion. Serialized tag-first so the UI
/// can switch on `reason` and show the detail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "reason", rename_all = "camelCase")]
pub enum ExclusionReason {
    /// Inside a pruned directory (or the directory entry itself).
    Pruned,
    /// Matched `.gitignore` / `.git/info/exclude`.
    Gitignored,
    /// Extension is in the user's exclusion list (FR-10.1).
    ExtensionExcluded { ext: String },
    /// Larger than `max_file_bytes`.
    Oversized { bytes: u64 },
    /// Contains a NUL byte or is not valid UTF-8 in its first 8 KiB.
    Binary,
    /// Could not be read (permissions, a broken symlink, a race).
    Unreadable { detail: String },
}

/// One row in the selection tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SelectableFile {
    /// Repo-relative, `/`-separated.
    pub path: String,
    pub is_dir: bool,
    pub bytes: u64,
    pub language: Option<String>,
    /// `None` when the file is selectable; `Some` explains why it is not.
    pub excluded: Option<ExclusionReason>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SelectionListing {
    pub repo_root: String,
    /// Sorted by path. Directories that were pruned appear once, with
    /// `excluded = Pruned`, and their contents are not listed.
    pub files: Vec<SelectableFile>,
    /// `true` when `max_entries` was hit and the listing is incomplete.
    pub truncated: bool,
}

/// Build the selection tree for one repository.
pub fn list_selectable(repo_root: &Path, opts: &SelectionOptions) -> CoreResult<SelectionListing> {
    if !repo_root.is_dir() {
        return Err(CoreError::Prompt(format!(
            "{} is not a directory",
            repo_root.display()
        )));
    }

    let gi = build_gitignore(repo_root);

    // Prune set, with `.git` always included.
    let mut prune: Vec<String> = opts.prune_dirs.clone();
    if !prune.iter().any(|d| d == ".git") {
        prune.push(".git".to_string());
    }

    // `filter_entry` runs on the walk's own threads; a shared, locked vec
    // collects the pruned directories it refuses to descend into so we can
    // still show them.
    let pruned_dirs: Arc<Mutex<Vec<PathBuf>>> = Arc::new(Mutex::new(Vec::new()));
    let prune_for_filter = prune.clone();
    let pruned_sink = Arc::clone(&pruned_dirs);

    let walker = WalkBuilder::new(repo_root)
        .hidden(false)
        .parents(false)
        .ignore(false)
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false)
        .require_git(false)
        .follow_links(false)
        .filter_entry(move |entry| {
            let is_dir = entry.file_type().is_some_and(|t| t.is_dir());
            if is_dir {
                if let Some(name) = entry.file_name().to_str() {
                    if prune_for_filter.iter().any(|d| d == name) {
                        pruned_sink.lock().unwrap().push(entry.path().to_path_buf());
                        return false;
                    }
                }
            }
            true
        })
        .build();

    let mut files: Vec<SelectableFile> = Vec::new();
    let mut truncated = false;

    for result in walker {
        if files.len() >= opts.max_entries {
            truncated = true;
            break;
        }
        let entry = match result {
            Ok(e) => e,
            Err(_) => continue,
        };
        let path = entry.path();
        if path == repo_root {
            continue;
        }
        let Some(rel) = rel_path(repo_root, path) else {
            continue;
        };
        let is_dir = entry.file_type().is_some_and(|t| t.is_dir());
        if is_dir {
            // Directories are only emitted implicitly through their files;
            // an empty dir carries nothing a prompt would use.
            continue;
        }

        let bytes = entry.metadata().map(|m| m.len()).unwrap_or(0);
        let language = language_for(&rel);
        let excluded = classify(
            path,
            &rel,
            bytes,
            opts.max_file_bytes,
            &opts.excluded_extensions,
            &gi,
        );

        files.push(SelectableFile {
            path: rel,
            is_dir: false,
            bytes,
            language,
            excluded,
        });
    }

    // Fold in the pruned directories the walk skipped.
    for dir in pruned_dirs.lock().unwrap().iter() {
        if let Some(rel) = rel_path(repo_root, dir) {
            files.push(SelectableFile {
                path: rel,
                is_dir: true,
                bytes: 0,
                language: None,
                excluded: Some(ExclusionReason::Pruned),
            });
        }
    }

    files.sort_by(|a, b| a.path.cmp(&b.path));
    files.dedup_by(|a, b| a.path == b.path);

    Ok(SelectionListing {
        repo_root: repo_root.to_string_lossy().replace('\\', "/"),
        files,
        truncated,
    })
}

/// Synthesize a bounded directory tree from a listing, for [`RepoContext::tree`].
/// Every file and directory at depth `<= max_depth` is included, up to `cap`
/// entries, sorted by path.
pub fn bounded_tree(
    listing: &SelectionListing,
    max_depth: u32,
    cap: usize,
) -> Vec<crate::prompt::context::TreeEntry> {
    use crate::prompt::context::TreeEntry;
    use std::collections::BTreeMap;

    // path -> is_dir; a dir seen as an ancestor stays a dir.
    let mut seen: BTreeMap<String, bool> = BTreeMap::new();
    for f in &listing.files {
        let depth = f.path.split('/').count() as u32 - 1;
        if depth <= max_depth {
            seen.entry(f.path.clone())
                .and_modify(|d| *d = *d || f.is_dir)
                .or_insert(f.is_dir);
        }
        // Ancestor directories.
        let parts: Vec<&str> = f.path.split('/').collect();
        for i in 1..parts.len() {
            let d = (i - 1) as u32;
            if d > max_depth {
                break;
            }
            let anc = parts[..i].join("/");
            seen.entry(anc).or_insert(true);
        }
    }

    seen.into_iter()
        .take(cap)
        .map(|(path, is_dir)| {
            let depth = path.split('/').count() as u32 - 1;
            TreeEntry {
                path,
                is_dir,
                depth,
            }
        })
        .collect()
}

/// A path that was asked for but withheld, with the reason.
pub type SkippedFile = (String, ExclusionReason);

/// Read the chosen files into [`EmbeddedFile`]s, re-applying every exclusion
/// rule so a client cannot smuggle a binary or oversized file into a prompt
/// by naming it directly. Paths that fail are returned in `skipped` with
/// their reason rather than silently dropped.
pub fn embed_files(
    repo_root: &Path,
    rel_paths: &[String],
    opts: &SelectionOptions,
    path_prefix: Option<&str>,
) -> CoreResult<(Vec<EmbeddedFile>, Vec<SkippedFile>)> {
    let gi = build_gitignore(repo_root);
    let mut embedded = Vec::new();
    let mut skipped = Vec::new();

    for rel in rel_paths {
        let norm = rel.replace('\\', "/");
        let abs = repo_root.join(&norm);
        // Refuse anything that escapes the repo root.
        if !abs.starts_with(repo_root) || norm.split('/').any(|c| c == "..") {
            skipped.push((
                norm.clone(),
                ExclusionReason::Unreadable {
                    detail: "path escapes the repository".into(),
                },
            ));
            continue;
        }
        let bytes = match std::fs::metadata(&abs) {
            Ok(m) => m.len(),
            Err(e) => {
                skipped.push((
                    norm.clone(),
                    ExclusionReason::Unreadable {
                        detail: e.to_string(),
                    },
                ));
                continue;
            }
        };
        if let Some(reason) = classify(
            &abs,
            &norm,
            bytes,
            opts.max_file_bytes,
            &opts.excluded_extensions,
            &gi,
        ) {
            skipped.push((norm.clone(), reason));
            continue;
        }
        match std::fs::read(&abs) {
            Ok(raw) => match String::from_utf8(raw) {
                Ok(content) => embedded.push(EmbeddedFile {
                    path: match path_prefix {
                        Some(p) => format!("{p}/{norm}"),
                        None => norm.clone(),
                    },
                    language: language_for(&norm),
                    content,
                    bytes,
                }),
                Err(_) => skipped.push((norm.clone(), ExclusionReason::Binary)),
            },
            Err(e) => skipped.push((
                norm.clone(),
                ExclusionReason::Unreadable {
                    detail: e.to_string(),
                },
            )),
        }
    }

    Ok((embedded, skipped))
}

fn build_gitignore(repo_root: &Path) -> Gitignore {
    let mut b = GitignoreBuilder::new(repo_root);
    let _ = b.add(repo_root.join(".gitignore"));
    let _ = b.add(repo_root.join(".git").join("info").join("exclude"));
    b.build().unwrap_or_else(|_| Gitignore::empty())
}

fn rel_path(root: &Path, path: &Path) -> Option<String> {
    path.strip_prefix(root)
        .ok()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .filter(|s| !s.is_empty())
}

fn classify(
    abs: &Path,
    rel: &str,
    bytes: u64,
    max_bytes: u64,
    excluded_exts: &[String],
    gi: &Gitignore,
) -> Option<ExclusionReason> {
    if gi.matched(rel, false).is_ignore() {
        return Some(ExclusionReason::Gitignored);
    }
    if let Some(ext) = Path::new(rel).extension().and_then(|e| e.to_str()) {
        let ext = ext.to_ascii_lowercase();
        if excluded_exts.iter().any(|e| e == &ext) {
            return Some(ExclusionReason::ExtensionExcluded { ext });
        }
    }
    if bytes > max_bytes {
        return Some(ExclusionReason::Oversized { bytes });
    }
    match sniff_is_binary(abs) {
        Ok(true) => Some(ExclusionReason::Binary),
        Ok(false) => None,
        Err(e) => Some(ExclusionReason::Unreadable {
            detail: e.to_string(),
        }),
    }
}

/// Read the first [`SNIFF_BYTES`] and decide: a NUL byte, or a UTF-8 decode
/// error that is not merely a chunk boundary cut through a multi-byte
/// sequence, means binary.
fn sniff_is_binary(path: &Path) -> std::io::Result<bool> {
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let mut buf = [0u8; SNIFF_BYTES];
    let n = f.read(&mut buf)?;
    let chunk = &buf[..n];
    if chunk.contains(&0) {
        return Ok(true);
    }
    match std::str::from_utf8(chunk) {
        Ok(_) => Ok(false),
        Err(e) => {
            // A truncated trailing multi-byte sequence is fine — the rest of
            // the file may complete it. Any earlier invalid byte is not.
            let valid = e.valid_up_to();
            let trailing = n - valid;
            Ok(!(e.error_len().is_none() && trailing <= 3))
        }
    }
}

/// A coarse display-language guess from the file extension. Not
/// authoritative — only for the picker and the fenced-block hint in an
/// embedded file.
fn language_for(rel: &str) -> Option<String> {
    let ext = Path::new(rel).extension()?.to_str()?.to_ascii_lowercase();
    let name = match ext.as_str() {
        "rs" => "Rust",
        "ts" | "tsx" | "mts" | "cts" => "TypeScript",
        "js" | "jsx" | "mjs" | "cjs" => "JavaScript",
        "py" | "pyi" => "Python",
        "go" => "Go",
        "java" => "Java",
        "kt" | "kts" => "Kotlin",
        "swift" => "Swift",
        "c" | "h" => "C",
        "cc" | "cpp" | "cxx" | "hpp" | "hxx" => "C++",
        "cs" => "C#",
        "rb" => "Ruby",
        "php" => "PHP",
        "sh" | "bash" | "zsh" => "Shell",
        "sql" => "SQL",
        "html" | "htm" => "HTML",
        "css" => "CSS",
        "scss" | "sass" => "SCSS",
        "json" => "JSON",
        "toml" => "TOML",
        "yaml" | "yml" => "YAML",
        "md" | "markdown" => "Markdown",
        "xml" => "XML",
        "dockerfile" => "Dockerfile",
        _ => return None,
    };
    Some(name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;

    fn write(root: &Path, rel: &str, bytes: &[u8]) {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        let mut f = fs::File::create(p).unwrap();
        f.write_all(bytes).unwrap();
    }

    #[test]
    fn excludes_binary_content_regardless_of_extension() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(root, "notes.txt", b"plain text, definitely fine\n");
        write(root, "sneaky.txt", b"MZ\x00\x00binary payload\x00here");
        write(root, "src/main.rs", b"fn main() {}\n");

        let listing = list_selectable(root, &SelectionOptions::default()).unwrap();
        let by = |p: &str| listing.files.iter().find(|f| f.path == p).unwrap();

        assert!(by("notes.txt").excluded.is_none());
        assert!(by("src/main.rs").excluded.is_none());
        assert_eq!(by("sneaky.txt").excluded, Some(ExclusionReason::Binary));
    }

    #[test]
    fn prunes_node_modules_and_dot_git_but_shows_them() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(root, "index.js", b"console.log(1)\n");
        write(
            root,
            "node_modules/left-pad/index.js",
            b"module.exports = 1\n",
        );
        write(root, ".git/config", b"[core]\n");

        let listing = list_selectable(root, &SelectionOptions::default()).unwrap();

        assert!(listing
            .files
            .iter()
            .any(|f| f.path == "index.js" && f.excluded.is_none()));
        assert!(!listing
            .files
            .iter()
            .any(|f| f.path.starts_with("node_modules/")));
        let nm = listing
            .files
            .iter()
            .find(|f| f.path == "node_modules")
            .unwrap();
        assert_eq!(nm.excluded, Some(ExclusionReason::Pruned));
        assert!(nm.is_dir);
        assert!(!listing.files.iter().any(|f| f.path.starts_with(".git/")));
    }

    #[test]
    fn honors_gitignore_and_size_limit() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(root, ".gitignore", b"secret.env\n*.log\n");
        write(root, "secret.env", b"TOKEN=abc\n");
        write(root, "app.log", b"line\n");
        write(root, "keep.rs", b"// ok\n");
        write(root, "big.rs", &vec![b'a'; 300 * 1024]);

        let listing = list_selectable(root, &SelectionOptions::default()).unwrap();
        let by = |p: &str| listing.files.iter().find(|f| f.path == p).unwrap();

        assert_eq!(by("secret.env").excluded, Some(ExclusionReason::Gitignored));
        assert_eq!(by("app.log").excluded, Some(ExclusionReason::Gitignored));
        assert!(by("keep.rs").excluded.is_none());
        assert!(matches!(
            by("big.rs").excluded,
            Some(ExclusionReason::Oversized { .. })
        ));
    }

    #[test]
    fn embed_files_reapplies_exclusions() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(root, "ok.rs", b"fn main() {}\n");
        write(root, "bin.dat", b"\x00\x01\x02");
        write(root, ".gitignore", b"ignored.rs\n");
        write(root, "ignored.rs", b"// nope\n");

        let (embedded, skipped) = embed_files(
            root,
            &[
                "ok.rs".to_string(),
                "bin.dat".to_string(),
                "ignored.rs".to_string(),
                "../escape.rs".to_string(),
            ],
            &SelectionOptions::default(),
            None,
        )
        .unwrap();

        assert_eq!(embedded.len(), 1);
        assert_eq!(embedded[0].path, "ok.rs");
        assert_eq!(skipped.len(), 3);
    }
}
