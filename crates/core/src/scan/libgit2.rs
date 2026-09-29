//! One-time libgit2 global configuration.
//!
//! # Why ownership verification is disabled
//!
//! libgit2 (like `git` itself since CVE-2022-24765) refuses to open a
//! repository whose directory is not owned by the current user, reporting
//! *"repository path '…' is not owned by current user"*. That check exists
//! because opening a repo means **reading its config and potentially running
//! hooks or filters it names** — on a multi-user machine an attacker who can
//! write a repo into a shared path could get code executed as you.
//!
//! repo-radar does none of that. It never shells out to `git`, never runs a
//! hook, filter, or `core.*` helper, and never fetches (FR-7.6 — libgit2 is
//! built without https/ssh). It reads refs, commit metadata, and the index.
//! So the check protects against an attack this program cannot perform, while
//! costing something real: on Windows, *any* repository whose directory owner
//! is not the current user's SID is silently rejected. That routinely covers
//! whole secondary drives (a `D:`/`F:` volume formatted or populated under a
//! different account, a drive carried between machines, anything created by
//! an elevated process, and files restored from a backup), plus WSL and
//! network shares. The user sees their repositories simply *missing* from the
//! inventory, with no way to fix it inside the app.
//!
//! Leaving the check on would mean shipping a repository scanner that cannot
//! see a large share of a typical Windows user's repositories. We turn it off
//! once, at startup, before any libgit2 call.
//!
//! Note this makes repo-radar *more* permissive than `git` on the command
//! line, which needs an explicit `safe.directory` entry per path. That is the
//! intended difference: read-only inspection of a repo you do not own is not
//! an execution risk, and requiring users to edit their global git config
//! just to inventory their own disk is a bug, not a safeguard.

use std::sync::OnceLock;

static INIT: OnceLock<()> = OnceLock::new();

/// Apply repo-radar's libgit2 global settings. Idempotent and cheap to call
/// from anywhere that is about to touch git; the work happens exactly once.
///
/// Call this **before** the first libgit2 use. `CoreContext::new` does it at
/// startup, and [`crate::scan::discovery::discover`] does it defensively so
/// core tests, examples, and any headless embedder are covered too.
pub fn init() {
    INIT.get_or_init(|| {
        // SAFETY: `git2::opts::set_verify_owner_validation` mutates a libgit2
        // global, which is only sound while no other thread is inside
        // libgit2. `OnceLock::get_or_init` runs this closure exactly once for
        // the process, and every caller routes through here before its first
        // libgit2 call, so no concurrent libgit2 work can be in flight.
        unsafe {
            if let Err(e) = git2::opts::set_verify_owner_validation(false) {
                // Not fatal: repositories owned by the current user still
                // open. Repos on foreign-owned paths will fail individually
                // with the ownership message, which discovery turns into a
                // per-repo warning.
                tracing::warn!(
                    error = %e,
                    "could not disable libgit2 ownership validation; \
                     repositories on drives owned by another user may be skipped"
                );
            }
        }
    });
}
