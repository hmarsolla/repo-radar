//! The matcher (DESIGN §8.4) — a **pure function** from a dependency and its
//! candidate advisories to findings. It never touches the database: SQL
//! narrows candidates by exact `(ecosystem, package_name)`, then this code
//! walks each range's event list in ecosystem-correct version order and
//! decides.
//!
//! The three traps it guards against (§8.4): an `introduced` with no
//! `fixed` means everything onward is affected; `last_affected` is
//! inclusive while `fixed` is exclusive; events must sort with the
//! ecosystem's comparator, not lexically (`1.10.0` vs `1.9.0`).
//!
//! Implemented in **M2-16**. Its test suite (DESIGN §16.2) is the
//! highest-value set of tests in the project.
