//! Repository discovery walker (FR-1, DESIGN §6.6). `ignore`-based walk that
//! stops descending at `.git`, honors the prune list, follows no symlinks,
//! and handles worktree pointer files and bare repos. Submodules are
//! attached from the parent's `.gitmodules`, never independently discovered.
//! Implemented in **M1-1 / M1-2**.
