//! Domain model (DESIGN §4).
//!
//! These types are the vocabulary the rest of the core speaks in. They
//! depend only on `serde`, `chrono`, and `specta` (for TypeScript binding
//! generation) — never on `db`, `scan`, or anything with IO. Keeping this
//! module leaf-level is what lets the parsers and the matcher be unit-tested
//! against hand-written values.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use specta::Type;

/// A repository-relative path, always stored with `/` separators regardless
/// of platform so fingerprints and DB rows are stable across OSes.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Type)]
pub struct RelPath(pub String);

impl RelPath {
    pub fn new(s: impl Into<String>) -> Self {
        RelPath(s.into().replace('\\', "/"))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for RelPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

// ---------------------------------------------------------------------------
// Ecosystems and dependency shape
// ---------------------------------------------------------------------------

/// The four package ecosystems v1 covers. Adding Java/.NET/PHP/Ruby later is
/// a new variant plus a `LockfileParser` impl — nothing else in the
/// pipeline changes (DESIGN §7.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
pub enum Ecosystem {
    Npm,
    PyPI,
    CratesIo,
    Go,
}

impl Ecosystem {
    /// The identifier OSV uses, and the path segment in bulk-download URLs.
    pub fn osv_id(self) -> &'static str {
        match self {
            Ecosystem::Npm => "npm",
            Ecosystem::PyPI => "PyPI",
            Ecosystem::CratesIo => "crates.io",
            Ecosystem::Go => "Go",
        }
    }

    /// Parse the OSV ecosystem string. OSV sometimes suffixes a variant
    /// after a colon (`Alpine:v3.4`); only the prefix is significant here.
    pub fn from_osv_id(s: &str) -> Option<Self> {
        match s.split(':').next().unwrap_or(s) {
            "npm" => Some(Ecosystem::Npm),
            "PyPI" => Some(Ecosystem::PyPI),
            "crates.io" => Some(Ecosystem::CratesIo),
            "Go" => Some(Ecosystem::Go),
            _ => None,
        }
    }

    pub const ALL: [Ecosystem; 4] = [
        Ecosystem::Npm,
        Ecosystem::PyPI,
        Ecosystem::CratesIo,
        Ecosystem::Go,
    ];
}

/// How much to trust a dependency's version string (FR-4.2). `Exact` comes
/// from a lockfile; `Range` from a manifest with no lock present.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
pub enum Confidence {
    Exact,
    Range,
}

/// Dependency relationship kind (FR-4.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
pub enum Scope {
    Runtime,
    Dev,
    Build,
    Optional,
    Peer,
}

impl Scope {
    /// Dev and build dependencies get a reduced health deduction (FR-6.6).
    pub fn is_dev_or_build(self) -> bool {
        matches!(self, Scope::Dev | Scope::Build)
    }
}

/// One resolved dependency of a repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct Dependency {
    pub ecosystem: Ecosystem,
    /// Normalized per FR-4.5 / DESIGN §7.4 — the join key against advisories.
    pub name: String,
    /// As written in the manifest, for display.
    pub raw_name: String,
    pub version: String,
    pub confidence: Confidence,
    pub scope: Scope,
    pub is_direct: bool,
    /// The manifest this dependency came from, always retained (FR-4.7).
    pub manifest_path: RelPath,
}

/// Whether a parsed file pins exact versions (a lockfile) or declares ranges
/// (a manifest). Drives the confidence a parser normally emits (§7.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum ManifestKind {
    Lockfile,
    Manifest,
}

impl ManifestKind {
    pub fn confidence(self) -> Confidence {
        match self {
            ManifestKind::Lockfile => Confidence::Exact,
            ManifestKind::Manifest => Confidence::Range,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            ManifestKind::Lockfile => "lockfile",
            ManifestKind::Manifest => "manifest",
        }
    }
}

/// A manifest that contributed dependencies — one `manifests` table row, and
/// an input to the scan fingerprint (§6.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ParsedManifest {
    pub path: RelPath,
    pub ecosystem: Ecosystem,
    pub kind: ManifestKind,
    /// `blake3` hex of the file content.
    pub content_hash: String,
}

// ---------------------------------------------------------------------------
// Advisories and findings
// ---------------------------------------------------------------------------

/// The headline distinction the whole health model exists to make (FR-6.1):
/// a backdoored package is not the same class of problem as a CVE.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
pub enum FindingKind {
    Compromise,
    Vulnerability,
}

/// Ordered so `Severity::Critical > Severity::Low`. `Unscored` is the
/// *lowest* ordinal but still carries a nonzero deduction (FR-6.4) — it
/// means "nobody rated this", not "this is safe".
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Type)]
pub enum Severity {
    Unscored,
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct Reference {
    pub kind: String,
    pub url: String,
}

/// A normalized OSV advisory, post-classification and post-severity-extraction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct Advisory {
    /// `GHSA-…`, `MAL-2024-1234`, `CVE-…`, …
    pub id: String,
    pub kind: FindingKind,
    pub summary: String,
    pub details: String,
    pub severity: Severity,
    pub cvss_score: Option<f32>,
    pub published: DateTime<Utc>,
    pub modified: DateTime<Utc>,
    pub aliases: Vec<String>,
    pub references: Vec<Reference>,
    /// Non-`None` => retained for explanation but excluded from matching
    /// (DESIGN §8.1).
    pub withdrawn: Option<DateTime<Utc>>,
}

/// A dependency matched against an advisory (DESIGN §8.4). Confidence is
/// inherited from the dependency: a `Range` dependency yields a `Range`
/// finding, which the scorer weights down (FR-6.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct Finding {
    pub dependency: Dependency,
    pub advisory_id: String,
    pub kind: FindingKind,
    pub confidence: Confidence,
    pub fixed_version: Option<String>,
}

// ---------------------------------------------------------------------------
// Classification
// ---------------------------------------------------------------------------

/// The closed set of repository categories (FR-3.1). `Unknown` is a real
/// answer — guessing is worse than admitting ignorance (FR-3.5).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Type,
)]
pub enum Category {
    Frontend,
    Backend,
    Fullstack,
    Mobile,
    DevOps,
    DataMl,
    Library,
    Cli,
    Docs,
    Unknown,
}

/// How firmly a category was assigned, derived from the margin between the
/// top two category scores (DESIGN §10.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
pub enum ConfidenceLevel {
    Low,
    Medium,
    High,
}

/// One technology detected in a repo (FR-2.3). `evidence` records *why* —
/// a marker-file-only detection renders with less prominence than a
/// dependency-confirmed one (FR-2.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct DetectedTech {
    pub tech: String,
    /// `framework | tooling | package-manager | runtime`
    pub kind: String,
    pub evidence: Vec<String>,
}

/// Per-language aggregate for a repo (FR-2.1). Never a file list — memory
/// stays bounded (DESIGN §17).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct LanguageStat {
    pub language: String,
    pub code_lines: u64,
    pub comment_lines: u64,
    pub files: u64,
    /// Share of total code lines, 0..=100.
    pub percentage: f32,
}

/// The full classification result for a repo, including the breakdown that
/// backs the explainability UI (FR-3.6).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct Classification {
    pub category: Category,
    pub confidence: ConfidenceLevel,
    /// Per-category accumulated weight and the rules that fired.
    pub scores: CategoryScores,
    /// A user override (FR-3.7); the computed `category` stays visible beside it.
    pub manual: Option<Category>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
pub struct CategoryScores {
    /// `(category, accumulated weight)` for every category that scored.
    pub totals: Vec<(Category, f32)>,
    /// Every rule that fired, with its contribution.
    pub fired: Vec<FiredRule>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct FiredRule {
    pub rule_id: String,
    pub signal: String,
    pub category: Category,
    pub weight: f32,
}

// ---------------------------------------------------------------------------
// Git
// ---------------------------------------------------------------------------

/// Everything FR-7 extracts from a repo's git data. All fields are optional
/// at the type level because an empty repo (FR-7.9) or a detached HEAD
/// leaves some of them undefined, and that is not an error.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct GitInfo {
    pub head_sha: Option<String>,
    pub branch: Option<String>,
    pub last_commit_at: Option<DateTime<Utc>>,
    pub last_commit_summary: Option<String>,
    pub commits_90d: Option<u32>,
    pub commits_total: Option<u32>,
    pub author_count: Option<u32>,
    pub dirty_modified: Option<u32>,
    pub dirty_staged: Option<u32>,
    pub dirty_untracked: Option<u32>,
    /// Ahead/behind vs the upstream — computed from **local refs only**,
    /// never a network fetch (FR-7.6).
    pub ahead: Option<u32>,
    pub behind: Option<u32>,
    pub remote_url: Option<String>,
    pub branch_count: Option<u32>,
    pub has_stash: Option<bool>,
}

impl GitInfo {
    pub fn is_dirty(&self) -> bool {
        self.dirty_modified.unwrap_or(0)
            + self.dirty_staged.unwrap_or(0)
            + self.dirty_untracked.unwrap_or(0)
            > 0
    }
}

// ---------------------------------------------------------------------------
// Health
// ---------------------------------------------------------------------------

/// Health band (FR-6). Derived from the final score; a confirmed compromise
/// forces `Critical` regardless of the number (DESIGN §9 step 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
pub enum Band {
    /// No advisory sync has ever completed — health is *unknown*, not
    /// healthy (DESIGN §14.4). Showing green here would be the worst bug
    /// this product could ship.
    Unknown,
    Critical,
    Poor,
    Fair,
    Good,
    Excellent,
}

/// One line item in a health breakdown. The UI renders these directly
/// rather than recomputing, so the number shown and the number explained
/// cannot drift (FR-6.9).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct Deduction {
    pub cause: DeductionCause,
    pub label: String,
    pub amount: f32,
    /// Each multiplier applied, named, e.g. `("dev scope", 0.5)`.
    pub multipliers: Vec<(String, f32)>,
}

/// What a deduction traces back to — an advisory id or a git fact. Every
/// digit of a score must be attributable to one of these (PRD §11).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub enum DeductionCause {
    Advisory(String),
    NoLockfile,
    StaleCommits,
    DirtyTree,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct HealthResult {
    pub score: u8,
    pub band: Band,
    pub breakdown: Vec<Deduction>,
    /// Set when the compromise cap (FR-6.3) held the score down; names the
    /// advisory responsible.
    pub capped_by: Option<String>,
}

// ---------------------------------------------------------------------------
// Warnings (DESIGN §4.1, §15)
// ---------------------------------------------------------------------------

/// A recoverable, per-item problem. Nothing in a scan is fatal at the scan
/// level (FR-1.10, FR-4.8): every recoverable failure becomes one of these,
/// is persisted with the scan, and badges the affected repo. Silent
/// degradation — a repo whose lockfile failed to parse looking identical to
/// a clean one — is exactly what this type exists to prevent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct Warning {
    pub scope: WarningScope,
    pub kind: WarningKind,
    pub message: String,
}

impl Warning {
    pub fn new(scope: WarningScope, kind: WarningKind, message: impl Into<String>) -> Self {
        Self {
            scope,
            kind,
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub enum WarningScope {
    Scan,
    Repo(String),
    File(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
pub enum WarningKind {
    PermissionDenied,
    ParseFailed,
    GitTimeout,
    GitError,
    UnparseableVersion,
    RulePackInvalid,
    Panic,
    Other,
}

// ---------------------------------------------------------------------------
// Scan aggregates
// ---------------------------------------------------------------------------

/// The minimum needed to identify a repo before analysis: where it is, what
/// to call it, and whether it is a submodule of something already found
/// (FR-1.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct RepoIdentity {
    /// Absolute path on disk.
    pub path: String,
    pub name: String,
    pub is_bare: bool,
    /// `Some(parent path)` when this was attached as a submodule child.
    pub parent_path: Option<String>,
}

/// Everything one repo's analysis produces. Built on a worker thread, sent
/// over the channel to the single writer, and never written to the DB by
/// the thread that built it (DESIGN §1 decision 3, §6.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct RepoAnalysis {
    pub repo: RepoIdentity,
    pub git: Option<GitInfo>,
    pub languages: Vec<LanguageStat>,
    pub dependencies: Vec<Dependency>,
    /// The manifest files those dependencies came from (FR-4.7, §6.5).
    pub manifests: Vec<ParsedManifest>,
    pub technologies: Vec<DetectedTech>,
    pub classification: Classification,
    /// `true` when the repo has more than one manifest root (FR-4.7).
    pub is_monorepo: bool,
    /// Recoverable problems hit while analysing this repo (FR-1.10, FR-4.8).
    pub warnings: Vec<Warning>,
}

/// How current the advisory data is, carried into prompts and shown in the
/// global freshness indicator (FR-5.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
pub enum Freshness {
    /// Never synced — findings are not trustworthy yet.
    Never,
    Fresh,
    /// Older than 7 days.
    Stale,
    /// Older than 30 days.
    VeryStale,
}
