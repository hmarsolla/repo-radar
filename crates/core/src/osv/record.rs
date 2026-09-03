//! OSV record deserialization, classification, and normalization
//! (DESIGN §8.1, §8.2; spike findings in §8.2.1).
//!
//! Only the fields repo-radar needs are deserialized; unknown fields are
//! ignored so OSV schema additions never break ingestion.

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::model::{Ecosystem, FindingKind, Reference, Severity};
use crate::parsers::normalize_package_name;

// ---------------------------------------------------------------------------
// Raw OSV JSON subset
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct OsvRecord {
    pub id: String,
    pub modified: String,
    #[serde(default)]
    pub published: Option<String>,
    #[serde(default)]
    pub withdrawn: Option<String>,
    #[serde(default)]
    pub aliases: Option<Vec<String>>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub details: Option<String>,
    #[serde(default)]
    pub severity: Option<Vec<OsvSeverity>>,
    #[serde(default)]
    pub affected: Option<Vec<OsvAffected>>,
    #[serde(default)]
    pub references: Option<Vec<OsvReference>>,
    #[serde(default)]
    pub database_specific: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OsvSeverity {
    #[serde(rename = "type")]
    pub kind: String, // "CVSS_V4" | "CVSS_V3" | "CVSS_V2" | ...
    pub score: String, // a CVSS vector string
}

#[derive(Debug, Clone, Deserialize)]
pub struct OsvAffected {
    pub package: OsvPackage,
    #[serde(default)]
    pub ranges: Option<Vec<OsvRange>>,
    #[serde(default)]
    pub versions: Option<Vec<String>>,
    #[serde(default)]
    pub database_specific: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OsvPackage {
    pub ecosystem: String,
    pub name: String,
    #[serde(default)]
    pub purl: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OsvRange {
    #[serde(rename = "type")]
    pub range_type: String, // "SEMVER" | "ECOSYSTEM" | "GIT"
    #[serde(default)]
    pub events: Vec<OsvEvent>,
}

/// One event object: exactly one of these keys is set in practice.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct OsvEvent {
    #[serde(default)]
    pub introduced: Option<String>,
    #[serde(default)]
    pub fixed: Option<String>,
    #[serde(default)]
    pub last_affected: Option<String>,
    #[serde(default)]
    pub limit: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OsvReference {
    #[serde(rename = "type")]
    pub kind: String,
    pub url: String,
}

// ---------------------------------------------------------------------------
// Classification (DESIGN §8.2)
// ---------------------------------------------------------------------------

/// Compromise vs Vulnerability (FR-6.1).
pub fn classify(record: &OsvRecord) -> FindingKind {
    if record.id.starts_with("MAL-") {
        return FindingKind::Compromise;
    }
    if record
        .aliases
        .iter()
        .flatten()
        .any(|a| a.starts_with("MAL-"))
    {
        return FindingKind::Compromise;
    }
    if has_malicious_marker(record.database_specific.as_ref()) {
        return FindingKind::Compromise;
    }
    FindingKind::Vulnerability
}

/// The OpenSSF Malicious Packages marker (spike §8.2.1): a present, non-empty
/// `database_specific["malicious-packages-origins"]` array. Isolated as one
/// small function so a change to OSV's marker shape touches one place.
pub fn has_malicious_marker(database_specific: Option<&serde_json::Value>) -> bool {
    database_specific
        .and_then(|v| v.get("malicious-packages-origins"))
        .and_then(|v| v.as_array())
        .map(|a| !a.is_empty())
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Per-ecosystem `MAL-` coverage (spike §8.2.1, D1)
// ---------------------------------------------------------------------------

/// How complete OSV's malicious-package data is for an ecosystem. Hard-coded
/// from the M2-12 spike; revisited each release.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, specta::Type)]
pub enum MalCoverage {
    /// Systematic automated feeds — "no compromise findings" is meaningful.
    Strong,
    /// Only a handful of hand-filed reports. "No compromise findings" means
    /// "the few known cases were checked", not "this is clean". The UI must
    /// say so.
    Thin,
}

impl MalCoverage {
    pub fn for_ecosystem(eco: Ecosystem) -> Self {
        match eco {
            Ecosystem::Npm | Ecosystem::PyPI => MalCoverage::Strong,
            Ecosystem::CratesIo | Ecosystem::Go => MalCoverage::Thin,
        }
    }
}

// ---------------------------------------------------------------------------
// Normalized form for ingestion + matching
// ---------------------------------------------------------------------------

/// A normalized advisory ready to persist (DESIGN §5.3 `advisories`).
#[derive(Debug, Clone, PartialEq)]
pub struct NormalizedAdvisory {
    pub id: String,
    pub kind: FindingKind,
    pub summary: String,
    pub details: String,
    pub severity: Severity,
    pub cvss_score: Option<f32>,
    pub published: Option<DateTime<Utc>>,
    pub modified: DateTime<Utc>,
    pub withdrawn: Option<DateTime<Utc>>,
    pub aliases: Vec<String>,
    pub references: Vec<Reference>,
}

/// One `affected[]` entry, normalized (DESIGN §5.3 `affected_ranges` /
/// `affected_versions`).
#[derive(Debug, Clone, PartialEq)]
pub struct NormalizedAffected {
    pub ecosystem: Ecosystem,
    /// Normalized the same way as `dependencies.name` (FR-4.5).
    pub package_name: String,
    pub ranges: Vec<NormalizedRange>,
    pub versions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NormalizedRange {
    pub range_type: RangeType,
    pub events: Vec<RangeEvent>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeType {
    Semver,
    Ecosystem,
    /// `GIT` ranges carry commit hashes, not versions — unusable for
    /// package matching, kept only so a range list round-trips.
    Git,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RangeEvent {
    Introduced(String),
    Fixed(String),
    LastAffected(String),
    /// `limit` — rarely used; treated as a hard upper bound like `fixed`.
    Limit(String),
}

/// Parse and normalize a raw record. Returns `None` only when the record is
/// so malformed there is nothing to store (no id, unparseable `modified`).
/// A withdrawn advisory is still returned — it is retained but excluded from
/// matching downstream (DESIGN §8.1).
pub fn normalize(record: &OsvRecord) -> Option<(NormalizedAdvisory, Vec<NormalizedAffected>)> {
    if record.id.is_empty() {
        return None;
    }
    let modified = parse_ts(&record.modified)?;
    let kind = classify(record);
    let (severity, cvss_score) = crate::osv::severity::extract(record);

    let advisory = NormalizedAdvisory {
        id: record.id.clone(),
        kind,
        summary: record.summary.clone().unwrap_or_default(),
        details: record.details.clone().unwrap_or_default(),
        severity,
        cvss_score,
        published: record.published.as_deref().and_then(parse_ts),
        modified,
        withdrawn: record.withdrawn.as_deref().and_then(parse_ts),
        aliases: record.aliases.clone().unwrap_or_default(),
        references: record
            .references
            .iter()
            .flatten()
            .map(|r| Reference {
                kind: r.kind.clone(),
                url: r.url.clone(),
            })
            .collect(),
    };

    let mut affected = Vec::new();
    for a in record.affected.iter().flatten() {
        let Some(eco) = Ecosystem::from_osv_id(&a.package.ecosystem) else {
            continue; // an ecosystem repo-radar does not cover
        };
        let package_name = normalize_package_name(eco, &a.package.name);

        let ranges = a
            .ranges
            .iter()
            .flatten()
            .filter_map(normalize_range)
            .collect();
        let versions = a.versions.clone().unwrap_or_default();

        affected.push(NormalizedAffected {
            ecosystem: eco,
            package_name,
            ranges,
            versions,
        });
    }

    Some((advisory, affected))
}

/// Parse the `affected_ranges.events` JSON blob
/// (`[{"introduced":"0"},{"fixed":"1.2.3"}]`) back into events — the inverse
/// of what sync writes, used by the matcher's Phase-1 read.
pub fn parse_events_json(json: &str) -> Vec<RangeEvent> {
    let Ok(arr) = serde_json::from_str::<Vec<serde_json::Map<String, serde_json::Value>>>(json)
    else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for obj in arr {
        if let Some(v) = obj.get("introduced").and_then(|v| v.as_str()) {
            out.push(RangeEvent::Introduced(v.to_string()));
        } else if let Some(v) = obj.get("fixed").and_then(|v| v.as_str()) {
            out.push(RangeEvent::Fixed(v.to_string()));
        } else if let Some(v) = obj.get("last_affected").and_then(|v| v.as_str()) {
            out.push(RangeEvent::LastAffected(v.to_string()));
        } else if let Some(v) = obj.get("limit").and_then(|v| v.as_str()) {
            out.push(RangeEvent::Limit(v.to_string()));
        }
    }
    out
}

fn normalize_range(r: &OsvRange) -> Option<NormalizedRange> {
    let range_type = match r.range_type.as_str() {
        "SEMVER" => RangeType::Semver,
        "ECOSYSTEM" => RangeType::Ecosystem,
        "GIT" => RangeType::Git,
        _ => return None,
    };
    let mut events = Vec::new();
    for e in &r.events {
        if let Some(v) = &e.introduced {
            events.push(RangeEvent::Introduced(v.clone()));
        } else if let Some(v) = &e.fixed {
            events.push(RangeEvent::Fixed(v.clone()));
        } else if let Some(v) = &e.last_affected {
            events.push(RangeEvent::LastAffected(v.clone()));
        } else if let Some(v) = &e.limit {
            events.push(RangeEvent::Limit(v.clone()));
        }
    }
    if events.is_empty() {
        return None;
    }
    Some(NormalizedRange { range_type, events })
}

/// OSV timestamps are RFC 3339. Be lenient about a trailing `Z` vs offset.
fn parse_ts(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s.trim())
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(json: &str) -> OsvRecord {
        serde_json::from_str(json).expect("valid OSV json")
    }

    #[test]
    fn classify_by_mal_id() {
        let r = rec(r#"{"id":"MAL-2024-1","modified":"2024-01-01T00:00:00Z"}"#);
        assert_eq!(classify(&r), FindingKind::Compromise);
    }

    #[test]
    fn classify_by_mal_alias() {
        let r = rec(
            r#"{"id":"GHSA-xxxx","modified":"2024-01-01T00:00:00Z","aliases":["CVE-2024-1","MAL-2024-9"]}"#,
        );
        assert_eq!(classify(&r), FindingKind::Compromise);
    }

    #[test]
    fn classify_by_origins_marker() {
        let r = rec(r#"{"id":"GHSA-yyyy","modified":"2024-01-01T00:00:00Z",
                "database_specific":{"malicious-packages-origins":[{"source":"ghsa-malware"}]}}"#);
        assert_eq!(classify(&r), FindingKind::Compromise);
    }

    #[test]
    fn empty_origins_array_is_not_a_marker() {
        let r = rec(r#"{"id":"GHSA-zzzz","modified":"2024-01-01T00:00:00Z",
                "database_specific":{"malicious-packages-origins":[]}}"#);
        assert_eq!(classify(&r), FindingKind::Vulnerability);
    }

    #[test]
    fn plain_cve_is_a_vulnerability() {
        let r = rec(r#"{"id":"GHSA-abcd","modified":"2024-01-01T00:00:00Z"}"#);
        assert_eq!(classify(&r), FindingKind::Vulnerability);
    }

    #[test]
    fn normalize_a_compromise_with_versions_only() {
        let r = rec(
            r#"{"id":"MAL-2023-8429","modified":"2026-07-23T07:49:55Z","published":"2023-11-03T21:05:03Z",
                "summary":"Malicious code in littest (crates.io)",
                "affected":[{"package":{"name":"littest","ecosystem":"crates.io"},"versions":["0.3.1"]}]}"#,
        );
        let (adv, aff) = normalize(&r).unwrap();
        assert_eq!(adv.kind, FindingKind::Compromise);
        assert_eq!(adv.severity, Severity::Unscored);
        assert_eq!(aff.len(), 1);
        assert_eq!(aff[0].ecosystem, Ecosystem::CratesIo);
        assert_eq!(aff[0].package_name, "littest");
        assert_eq!(aff[0].versions, vec!["0.3.1"]);
        assert!(aff[0].ranges.is_empty());
    }

    #[test]
    fn normalize_a_range_with_introduced_and_fixed() {
        let r = rec(r#"{"id":"GHSA-x","modified":"2024-01-01T00:00:00Z",
                "affected":[{"package":{"name":"Left-Pad","ecosystem":"npm"},
                "ranges":[{"type":"SEMVER","events":[{"introduced":"0"},{"fixed":"1.2.3"}]}]}]}"#);
        let (_, aff) = normalize(&r).unwrap();
        assert_eq!(aff[0].package_name, "left-pad", "npm name is lowercased");
        assert_eq!(aff[0].ranges.len(), 1);
        assert_eq!(
            aff[0].ranges[0].events,
            vec![
                RangeEvent::Introduced("0".into()),
                RangeEvent::Fixed("1.2.3".into())
            ]
        );
    }

    #[test]
    fn withdrawn_is_retained_not_dropped() {
        let r = rec(
            r#"{"id":"GHSA-w","modified":"2024-02-01T00:00:00Z","withdrawn":"2024-03-01T00:00:00Z",
                "affected":[{"package":{"name":"foo","ecosystem":"npm"},"versions":["1.0.0"]}]}"#,
        );
        let (adv, _) = normalize(&r).unwrap();
        assert!(adv.withdrawn.is_some());
    }

    #[test]
    fn unknown_ecosystem_affected_entry_is_skipped() {
        let r = rec(r#"{"id":"GHSA-e","modified":"2024-01-01T00:00:00Z",
                "affected":[{"package":{"name":"log4j","ecosystem":"Maven"},"versions":["2.0"]}]}"#);
        let (_, aff) = normalize(&r).unwrap();
        assert!(aff.is_empty());
    }

    #[test]
    fn mal_coverage_matches_spike() {
        assert_eq!(
            MalCoverage::for_ecosystem(Ecosystem::Npm),
            MalCoverage::Strong
        );
        assert_eq!(
            MalCoverage::for_ecosystem(Ecosystem::PyPI),
            MalCoverage::Strong
        );
        assert_eq!(
            MalCoverage::for_ecosystem(Ecosystem::CratesIo),
            MalCoverage::Thin
        );
        assert_eq!(MalCoverage::for_ecosystem(Ecosystem::Go), MalCoverage::Thin);
    }
}
