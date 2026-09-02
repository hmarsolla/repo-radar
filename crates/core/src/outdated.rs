//! Outdated-dependency checks (FR-8). Registry lookups (npm, PyPI JSON,
//! crates.io, Go proxy), rate-limited, cached in `outdated_cache` with a
//! 24-hour reuse window. **Only ever runs from an explicit user action**,
//! and outdated-ness must not move the health score. Implemented in **M5-1**.
