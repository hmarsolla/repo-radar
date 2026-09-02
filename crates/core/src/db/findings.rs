//! Query module for `findings`: the per-repo write after match+score (stage
//! 4, which runs for every repo on every scan regardless of fingerprint —
//! DESIGN §6.5), and the cross-repo impact query backing the Advisories
//! screen via `idx_findings_advisory`. Bodies land with **M2-18** and **M2-20**.
