//! Prompt generation (FR-9, DESIGN §11).
//!
//! A `PromptContext` is rendered through `minijinja`. Built-in templates
//! ship as `.j2` in `assets/prompts/` and run through the *same* path as
//! user templates — no privileged built-in. Generation returns a `String`
//! decoupled from delivery so the Phase 2 `LlmProvider` seam needs no
//! generator changes (DESIGN §11.5).
//!
//! Implemented across **M4-1 … M4-7**.

pub mod context;
