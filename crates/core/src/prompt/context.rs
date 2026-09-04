//! `PromptContext` and its sub-structs (DESIGN §11.1) — the single serde
//! value rendered into every template, built-in or user-supplied, and
//! documented for user templates in `docs/prompt-context.md` (FR-9.2).
//! Implemented in **M4-1**.
//!
//! Field names here are **snake_case**: that is the Jinja convention and the
//! surface a user template writes against. The one exception is
//! [`ScopeContext`], which is also a Tauri command argument and so follows
//! the frontend's camelCase like every other binding type — templates see it
//! as `scope.kind` with values `wholeRepo | directory | files | diff`.
//!
//! Nothing here does IO. [`super::build`] assembles a `PromptContext` from
//! the database plus a caller-supplied file selection; this module only
//! defines the shape the templates see.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::model::{Category, DetectedTech, Freshness, GitInfo, LanguageStat};

/// The whole value a template renders against. One struct, so a user
/// template (FR-9.2) has exactly the same surface as a built-in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct PromptContext {
    /// When this prompt was generated, so the model can reason about how
    /// current everything below it is (RFC 3339).
    pub generated_at: DateTime<Utc>,
    /// One entry for a single-repo template (T2/T3), N for a cross-repo one (T1).
    pub repos: Vec<RepoContext>,
    /// What the user chose to include — whole repo, a subtree, an explicit
    /// file list, or a diff.
    pub scope: ScopeContext,
    /// The file bodies actually embedded, after binary/size/ignore exclusion
    /// (FR-9.3). Empty for T1, which compares metadata rather than source.
    pub files: Vec<EmbeddedFile>,
    /// How fresh the advisory data behind every finding is (FR-5.6). One of
    /// `Never | Fresh | Stale | VeryStale`; pipe through the
    /// `freshness_phrase` filter for a readable sentence.
    pub advisory_freshness: Freshness,
}

/// A human-readable rendering of [`Freshness`] for templates that want prose
/// rather than an enum name. Exposed to templates as the `freshness_phrase`
/// filter.
pub fn freshness_phrase(f: Freshness) -> &'static str {
    match f {
        Freshness::Never => "never synced — findings below are not yet trustworthy",
        Freshness::Fresh => "synced within the last 7 days",
        Freshness::Stale => "last synced more than 7 days ago",
        Freshness::VeryStale => "last synced more than 30 days ago — findings may be out of date",
    }
}

/// Everything one repository contributes to a prompt (DESIGN §11.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct RepoContext {
    pub name: String,
    /// The effective category — a manual override (FR-3.7) wins over the
    /// computed one. One of the [`Category`] names, e.g. `Backend`.
    pub category: Category,
    /// Plain-language lines describing the rules that drove the category
    /// (FR-3.6), e.g. `"react-app (dependency:react) → Frontend +3.0"`.
    pub category_signals: Vec<String>,
    pub languages: Vec<LanguageStat>,
    pub technologies: Vec<DetectedTech>,
    /// Direct dependencies only, each annotated with any advisories that hit
    /// it so a template can flag risky ones inline (FR-9.1).
    pub direct_dependencies: Vec<DependencySummary>,
    pub dependency_counts: DependencyCounts,
    /// Security findings for this repo, compromise first (FR-6.1). Each
    /// carries a `confirmed` flag so T2 can separate confirmed from
    /// speculative issues.
    pub findings: Vec<FindingSummary>,
    /// Stored health score/band, or `null` when no advisory sync has ever
    /// completed (health is *unknown*, not healthy — DESIGN §14.4).
    pub health: Option<HealthSummary>,
    pub git: Option<GitInfo>,
    /// A bounded-depth directory listing (DESIGN §11.1). Depth and entry
    /// count are capped so a monorepo cannot blow the context.
    pub tree: Vec<TreeEntry>,
}

/// One direct dependency, as a template sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct DependencySummary {
    pub ecosystem: String,
    pub name: String,
    pub version: String,
    /// `runtime | dev | build | optional | peer`.
    pub scope: String,
    /// `exact` (from a lockfile) or `range` (manifest only). A `range`
    /// dependency's findings are speculative — the resolved version might
    /// not be the affected one (FR-4.2, FR-9.1).
    pub version_confidence: String,
    /// Advisory ids matched to this dependency, if any.
    pub advisories: Vec<String>,
}

/// Direct/transitive counts per ecosystem (DESIGN §11.1).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct DependencyCounts {
    pub per_ecosystem: Vec<EcosystemCount>,
    pub direct_total: u32,
    pub transitive_total: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct EcosystemCount {
    pub ecosystem: String,
    pub direct: u32,
    pub transitive: u32,
}

/// One security finding, flattened for template use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct FindingSummary {
    pub advisory_id: String,
    /// `compromise` | `vulnerability`.
    pub kind: String,
    /// `critical | high | medium | low | unscored`.
    pub severity: String,
    pub package: String,
    pub version: String,
    pub fixed_version: Option<String>,
    pub summary: String,
    /// `true` when the match came from an exact (lockfile) version. A `false`
    /// here is the "speculative" bucket T2 must call out separately (FR-9.1).
    pub confirmed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct HealthSummary {
    pub score: u8,
    /// `critical | poor | fair | good | excellent`.
    pub band: String,
}

/// One node in a repo's bounded directory tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct TreeEntry {
    /// Repo-relative, `/`-separated.
    pub path: String,
    pub is_dir: bool,
    /// 0 for a top-level entry.
    pub depth: u32,
}

/// A single file's body, embedded into the prompt (FR-9.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct EmbeddedFile {
    /// Repo-relative, `/`-separated. Prefixed with the repo name when a
    /// prompt spans more than one repo.
    pub path: String,
    /// A display language guess from the extension, or `null`.
    pub language: Option<String>,
    pub content: String,
    pub bytes: u64,
}

/// What the user selected to include (DESIGN §11.2). Also a Tauri command
/// argument, so this one is camelCase: templates see `scope.kind` as
/// `wholeRepo | directory | files | diff`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ScopeContext {
    /// Everything selectable in the repo.
    WholeRepo,
    /// A single subtree.
    Directory { path: String },
    /// An explicit list of files.
    Files { paths: Vec<String> },
    /// A diff supplied as text (e.g. `git diff` output).
    Diff { description: String },
}

impl ScopeContext {
    /// A short label for logs and the preview header.
    pub fn label(&self) -> String {
        match self {
            ScopeContext::WholeRepo => "whole repository".into(),
            ScopeContext::Directory { path } => format!("directory {path}"),
            ScopeContext::Files { paths } => format!("{} file(s)", paths.len()),
            ScopeContext::Diff { .. } => "diff".into(),
        }
    }
}
