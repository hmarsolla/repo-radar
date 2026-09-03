//! Severity extraction with precedence (DESIGN §8.3).
//!
//! 1. `severity[]` entry of type `CVSS_V4`, else `CVSS_V3` — parse the
//!    vector, take the base score, band it.
//! 2. `database_specific.severity` string (GHSA: `CRITICAL` / `HIGH` /
//!    `MODERATE` / `LOW`) — `MODERATE` maps to `Medium`.
//! 3. `Severity::Unscored` — which carries its own deduction weight
//!    (FR-6.4), not treated as harmless.

use crate::model::Severity;
use crate::osv::record::OsvRecord;

/// Returns the banded severity and, when it came from a CVSS vector, the
/// numeric base score.
pub fn extract(record: &OsvRecord) -> (Severity, Option<f32>) {
    if let Some(score) = cvss_base_score(record) {
        return (band(score), Some(score as f32));
    }
    if let Some(sev) = database_specific_severity(record) {
        return (sev, None);
    }
    (Severity::Unscored, None)
}

fn cvss_base_score(record: &OsvRecord) -> Option<f64> {
    let entries = record.severity.as_ref()?;

    // Prefer v4, then v3.
    let pick = |wanted: &str| entries.iter().find(|e| e.kind.eq_ignore_ascii_case(wanted));

    if let Some(v4) = pick("CVSS_V4") {
        if let Ok(v) = v4.score.parse::<cvss::v4::Vector>() {
            return Some(v.score().value());
        }
    }
    if let Some(v3) = pick("CVSS_V3") {
        if let Ok(base) = v3.score.parse::<cvss::v3::Base>() {
            return Some(base.score().value());
        }
    }
    // A v2 vector, or an unrecognised type — fall through to the string.
    None
}

fn database_specific_severity(record: &OsvRecord) -> Option<Severity> {
    let raw = record
        .database_specific
        .as_ref()?
        .get("severity")?
        .as_str()?
        .trim()
        .to_ascii_uppercase();
    Some(match raw.as_str() {
        "CRITICAL" => Severity::Critical,
        "HIGH" => Severity::High,
        "MODERATE" | "MEDIUM" => Severity::Medium,
        "LOW" => Severity::Low,
        _ => return None,
    })
}

/// CVSS base-score bands (DESIGN §8.3): `≥9.0` Critical, `≥7.0` High,
/// `≥4.0` Medium, `>0` Low, `0` Unscored.
pub fn band(score: f64) -> Severity {
    if score >= 9.0 {
        Severity::Critical
    } else if score >= 7.0 {
        Severity::High
    } else if score >= 4.0 {
        Severity::Medium
    } else if score > 0.0 {
        Severity::Low
    } else {
        Severity::Unscored
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(json: &str) -> OsvRecord {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn cvss_v3_vector_bands_and_reports_score() {
        // AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H  => 9.8, Critical
        let r = rec(r#"{"id":"x","modified":"2024-01-01T00:00:00Z",
                "severity":[{"type":"CVSS_V3","score":"CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H"}]}"#);
        let (sev, score) = extract(&r);
        assert_eq!(sev, Severity::Critical);
        assert!((score.unwrap() - 9.8).abs() < 0.05);
    }

    #[test]
    fn cvss_v4_wins_over_v3() {
        // v4 vector scores ~6.9 (Medium); v3 present but lower precedence.
        let r = rec(r#"{"id":"x","modified":"2024-01-01T00:00:00Z","severity":[
                {"type":"CVSS_V3","score":"CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H"},
                {"type":"CVSS_V4","score":"CVSS:4.0/AV:N/AC:L/AT:N/PR:N/UI:P/VC:L/VI:L/VA:N/SC:N/SI:N/SA:N"}
            ]}"#);
        let (sev, score) = extract(&r);
        assert!(score.is_some());
        // Whatever the exact number, it must be the v4 result, not 9.8.
        assert!(score.unwrap() < 9.0);
        assert!(matches!(
            sev,
            Severity::Low | Severity::Medium | Severity::High
        ));
    }

    #[test]
    fn falls_back_to_database_specific_string() {
        let r = rec(r#"{"id":"x","modified":"2024-01-01T00:00:00Z",
                "database_specific":{"severity":"MODERATE"}}"#);
        let (sev, score) = extract(&r);
        assert_eq!(sev, Severity::Medium, "MODERATE maps to Medium");
        assert_eq!(score, None);
    }

    #[test]
    fn unscored_when_nothing_is_present() {
        let r = rec(r#"{"id":"x","modified":"2024-01-01T00:00:00Z"}"#);
        assert_eq!(extract(&r), (Severity::Unscored, None));
    }

    #[test]
    fn a_malformed_cvss_vector_does_not_panic() {
        let r = rec(r#"{"id":"x","modified":"2024-01-01T00:00:00Z",
                "severity":[{"type":"CVSS_V3","score":"garbage"}],
                "database_specific":{"severity":"HIGH"}}"#);
        // Bad vector → fall through to the string.
        assert_eq!(extract(&r).0, Severity::High);
    }

    #[test]
    fn banding_boundaries() {
        assert_eq!(band(9.0), Severity::Critical);
        assert_eq!(band(8.999), Severity::High);
        assert_eq!(band(7.0), Severity::High);
        assert_eq!(band(4.0), Severity::Medium);
        assert_eq!(band(0.1), Severity::Low);
        assert_eq!(band(0.0), Severity::Unscored);
    }
}
