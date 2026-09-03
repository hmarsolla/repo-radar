//! Language statistics (FR-2.1, FR-2.2) via `tokei` used as a library.
//!
//! The prune list is passed straight to `tokei` as an exclude set, so
//! vendored / build directories contribute nothing (FR-2.2). The full
//! per-language breakdown is returned; the 2%-of-code-lines threshold for
//! the *primary* list is applied by [`primary_languages`] at read time so
//! the underlying data is never lost.

use std::path::Path;

use tokei::{Config, LanguageType, Languages};

use crate::model::LanguageStat;

/// Minimum share of total code lines for a language to appear in the
/// primary list (FR-2.2).
pub const PRIMARY_THRESHOLD_PCT: f32 = 2.0;

pub struct LanguageConfig {
    /// Directory names / globs excluded from counting (the prune list).
    pub excluded: Vec<String>,
}

impl Default for LanguageConfig {
    fn default() -> Self {
        Self {
            excluded: crate::scan::discovery::DEFAULT_PRUNE_DIRS
                .iter()
                .map(|s| s.to_string())
                .collect(),
        }
    }
}

/// Count lines per language under `repo_path`. Returns every language found
/// (outside the exclude set), each with its share of total code lines,
/// sorted by code lines descending.
pub fn analyze(repo_path: &Path, cfg: &LanguageConfig) -> Vec<LanguageStat> {
    let excluded: Vec<&str> = cfg.excluded.iter().map(String::as_str).collect();
    let mut languages = Languages::new();
    let config = Config::default();
    languages.get_statistics(&[repo_path], &excluded, &config);

    let total_code: u64 = languages.values().map(|l| l.code as u64).sum();

    let mut stats: Vec<LanguageStat> = languages
        .iter()
        .filter(|(_, l)| l.code > 0 || !l.reports.is_empty())
        .map(|(ty, l)| {
            let code = l.code as u64;
            LanguageStat {
                language: display_name(*ty).to_string(),
                code_lines: code,
                comment_lines: l.comments as u64,
                files: l.reports.len() as u64,
                percentage: if total_code == 0 {
                    0.0
                } else {
                    (code as f64 / total_code as f64 * 100.0) as f32
                },
            }
        })
        .collect();

    stats.sort_by(|a, b| {
        b.code_lines
            .cmp(&a.code_lines)
            .then_with(|| a.language.cmp(&b.language))
    });
    stats
}

/// The subset of a breakdown that belongs in the primary language list:
/// at least [`PRIMARY_THRESHOLD_PCT`] of code lines (FR-2.2).
pub fn primary_languages(all: &[LanguageStat]) -> Vec<LanguageStat> {
    all.iter()
        .filter(|s| s.percentage >= PRIMARY_THRESHOLD_PCT)
        .cloned()
        .collect()
}

fn display_name(ty: LanguageType) -> &'static str {
    ty.name()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn counts_by_language_and_excludes_pruned_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        // ~10 lines of Rust in the tree proper.
        fs::write(
            root.join("main.rs"),
            "fn main() {\n    // a comment\n    let x = 1;\n    let y = 2;\n    println!(\"{}\", x + y);\n}\n",
        )
        .unwrap();
        // A big pile of JS that must NOT be counted (vendored).
        let nm = root.join("node_modules/leftpad");
        fs::create_dir_all(&nm).unwrap();
        fs::write(nm.join("index.js"), "var a = 1;\n".repeat(500)).unwrap();

        let stats = analyze(root, &LanguageConfig::default());

        let rust = stats
            .iter()
            .find(|s| s.language == "Rust")
            .expect("Rust counted");
        assert!(
            rust.code_lines >= 4 && rust.code_lines <= 6,
            "{}",
            rust.code_lines
        );
        assert!(
            !stats.iter().any(|s| s.language == "JavaScript"),
            "node_modules JavaScript leaked into the count: {stats:?}"
        );
        assert!((rust.percentage - 100.0).abs() < 0.01);
    }

    #[test]
    fn empty_tree_is_empty_not_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        assert!(analyze(dir.path(), &LanguageConfig::default()).is_empty());
    }

    #[test]
    fn primary_list_drops_sub_two_percent() {
        let all = vec![
            LanguageStat {
                language: "Rust".into(),
                code_lines: 980,
                comment_lines: 0,
                files: 9,
                percentage: 98.0,
            },
            LanguageStat {
                language: "TOML".into(),
                code_lines: 20,
                comment_lines: 0,
                files: 1,
                percentage: 1.0,
            },
        ];
        let primary = primary_languages(&all);
        assert_eq!(primary.len(), 1);
        assert_eq!(primary[0].language, "Rust");
    }
}
