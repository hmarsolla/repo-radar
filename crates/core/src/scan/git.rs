//! Git metadata extraction (FR-7) via `git2`. Ahead/behind from local refs
//! only — never a network fetch against a user repo (FR-7.6). Empty repos
//! and detached HEAD handled without error. Per-repo timeout (FR-7.10).
//! Implemented in **M1-3**.
