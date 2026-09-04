//! Prompt generation (FR-9, DESIGN §11).
//!
//! A [`context::PromptContext`] is assembled from the database
//! ([`build::build_context`]) plus a file selection ([`selection`]), then
//! rendered through `minijinja` ([`render::render`]). Built-in templates
//! ([`templates`]) ship as `.j2` in `assets/prompts/` and run through the
//! *same* path as user templates — no privileged built-in. Generation
//! returns a `String` decoupled from delivery so the Phase 2
//! [`provider::LlmProvider`] seam needs no generator changes (DESIGN §11.5).
//!
//! Implemented across **M4-1 … M4-7**.

pub mod build;
pub mod context;
pub mod provider;
pub mod render;
pub mod selection;
pub mod templates;

pub use build::{build_context, repo_context};
pub use context::{
    DependencyCounts, DependencySummary, EcosystemCount, EmbeddedFile, FindingSummary,
    HealthSummary, PromptContext, RepoContext, ScopeContext, TreeEntry,
};
pub use provider::{Completion, LlmProvider};
pub use render::{estimate_tokens, render};
pub use selection::{
    bounded_tree, embed_files, list_selectable, ExclusionReason, SelectableFile, SelectionListing,
    SelectionOptions, SkippedFile, DEFAULT_MAX_FILE_BYTES,
};
pub use templates::{
    list as list_templates, load_source as load_template_source, RepoArity, TemplateInfo,
    TemplateSource,
};
