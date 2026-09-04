//! Outdated-dependency checks (FR-8). Registry lookups (npm, PyPI JSON,
//! crates.io, Go proxy), cached in `outdated_cache` with a 24-hour reuse
//! window (FR-8.5).
//!
//! Two invariants this module exists to hold:
//!
//! * **It never runs automatically** (FR-8.1). There is no scheduler hook,
//!   no scan-pipeline call site — the only caller is an explicit Tauri
//!   command behind a user click. Being three minor versions behind is a
//!   maintenance fact, not a security fact.
//! * **Outdated-ness never moves the health score** (FR-8.6). Nothing here
//!   writes to `repos.health_*` or `findings`; the report is a standalone
//!   read-model.
//!
//! This is the second (and last) outbound-network module in `core`, after
//! [`crate::osv::sync`]. The four registry hosts are enumerated in
//! [`REGISTRY_HOSTS`] and the user-facing network policy (DESIGN §18).

use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::db::Db;
use crate::error::{CoreResult, OperationError};
use crate::model::Ecosystem;
use crate::version::{scheme_for, Ver};

const USER_AGENT: &str = concat!(
    "repo-radar/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/repo-radar/repo-radar)"
);

/// The registry endpoints this module contacts, for the network policy and
/// any future CSP enumeration (DESIGN §18). Contacted **only** on the
/// explicit FR-8 action.
pub const REGISTRY_HOSTS: &[&str] = &[
    "registry.npmjs.org",
    "pypi.org",
    "crates.io",
    "proxy.golang.org",
];

/// A repeat check within this window reuses the cached latest version
/// instead of re-querying the registry (FR-8.5). A forced refresh ignores
/// it.
pub const CACHE_TTL_HOURS: i64 = 24;

/// Worker threads for the registry fan-out. Kept modest — a single repo's
/// distinct-package count is small, and registries dislike bursts.
const FETCH_CONCURRENCY: usize = 8;

// ---------------------------------------------------------------------------
// Report model (crosses the Tauri boundary — camelCase)
// ---------------------------------------------------------------------------

/// How far behind a dependency is. Derived by comparing the installed
/// version's `(major, minor, patch)` against the registry's latest under the
/// ecosystem's own version scheme (never a shared code path — DESIGN §8.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum OutdatedStatus {
    /// Installed version is >= the registry's latest.
    UpToDate,
    OutdatedPatch,
    OutdatedMinor,
    OutdatedMajor,
    /// A version string would not parse, or the registry lookup failed — we
    /// could not decide. Never conflated with `UpToDate` (FR-4.8 spirit).
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct OutdatedEntry {
    /// OSV/registry ecosystem id (`npm`, `PyPI`, `crates.io`, `Go`).
    pub ecosystem: String,
    /// Normalized name (the cache key).
    pub name: String,
    /// Name as written in the manifest, for display.
    pub raw_name: String,
    pub current_version: String,
    /// Latest **stable** version from the registry, or `None` when the
    /// lookup failed or the package is unknown.
    pub latest_version: Option<String>,
    pub status: OutdatedStatus,
    pub is_direct: bool,
    /// `runtime` | `dev` | `build` | `optional` | `peer`.
    pub scope: String,
    /// Repo-relative manifest this dependency came from (FR-4.7).
    pub manifest_path: String,
    /// RFC3339 timestamp of the registry lookup the latest version came
    /// from (may be up to `CACHE_TTL_HOURS` old — the UI shows the age,
    /// FR-8.5).
    pub checked_at: String,
    /// True when `latest_version` came from `outdated_cache` rather than a
    /// fresh request in this run.
    pub from_cache: bool,
    /// Registry error for this package, if the lookup failed.
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct OutdatedReport {
    pub repo_id: i64,
    /// When this report was assembled.
    pub generated_at: String,
    /// One row per distinct `(ecosystem, name, version)` in the repo.
    pub entries: Vec<OutdatedEntry>,
    /// Packages whose latest version could not be fetched this run (network
    /// failures). Surfaced so a partial result is not mistaken for a clean
    /// bill of health.
    pub failed: Vec<String>,
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Check every dependency of `repo_id` against its registry (FR-8).
///
/// `force_refresh` bypasses the 24-hour cache and re-queries every package.
///
/// **Only ever called from an explicit user action** — see the module docs.
pub fn check_repo_outdated(
    db: &Db,
    repo_id: i64,
    force_refresh: bool,
) -> Result<OutdatedReport, OperationError> {
    let deps = load_repo_deps(db, repo_id).map_err(|e| OperationError::Registry(e.to_string()))?;
    if deps.is_empty() {
        return Ok(OutdatedReport {
            repo_id,
            generated_at: now_rfc3339(),
            entries: Vec::new(),
            failed: Vec::new(),
        });
    }

    // Distinct (ecosystem, name) pairs — the unit of a registry lookup and
    // of the cache. Skip ecosystems whose version string never parses so we
    // do not spend a request to then report `Unknown` anyway is *not* done
    // here: an unparseable pin should still show the latest so the user can
    // see what to move to.
    let pairs: BTreeSet<(Ecosystem, String)> =
        deps.iter().map(|d| (d.ecosystem, d.name.clone())).collect();

    // Which pairs already have a fresh cache row?
    let cached = load_cache(db, &pairs).map_err(|e| OperationError::Registry(e.to_string()))?;
    let cutoff = chrono::Utc::now() - chrono::Duration::hours(CACHE_TTL_HOURS);

    let to_fetch: Vec<(Ecosystem, String)> = pairs
        .iter()
        .filter(|pair| {
            if force_refresh {
                return true;
            }
            match cached.get(&cache_key(pair.0, &pair.1)) {
                Some(row) => row
                    .checked_at
                    .as_deref()
                    .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
                    .map(|t| t.with_timezone(&chrono::Utc) < cutoff)
                    .unwrap_or(true),
                None => true,
            }
        })
        .cloned()
        .collect();

    let fetched = fetch_latest_batch(&to_fetch);

    // Persist successes and failures to the cache (FR-8.5).
    if !fetched.is_empty() {
        let now = now_rfc3339();
        db.write(|conn| {
            let tx = conn.transaction()?;
            for ((eco, name), result) in &fetched {
                let (latest, err) = match result {
                    Ok(v) => (v.clone(), None),
                    Err(e) => (None, Some(e.as_str())),
                };
                tx.execute(
                    "INSERT INTO outdated_cache (ecosystem, name, latest_version, checked_at, error)
                     VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT(ecosystem, name) DO UPDATE SET
                         latest_version = excluded.latest_version,
                         checked_at = excluded.checked_at,
                         error = excluded.error",
                    rusqlite::params![eco.osv_id(), name, latest, now, err],
                )?;
            }
            tx.commit()?;
            Ok(())
        })
        .map_err(|e| OperationError::Registry(e.to_string()))?;
    }

    // Assemble one entry per distinct (ecosystem, name, version).
    let fresh: HashMap<String, &Result<Option<String>, String>> = fetched
        .iter()
        .map(|((eco, name), r)| (cache_key(*eco, name), r))
        .collect();

    let mut entries = Vec::with_capacity(deps.len());
    let mut failed = BTreeSet::new();
    for d in &deps {
        let key = cache_key(d.ecosystem, &d.name);
        let (latest, checked_at, from_cache, error) = match fresh.get(&key) {
            Some(Ok(v)) => (v.clone(), now_rfc3339(), false, None),
            Some(Err(e)) => (None, now_rfc3339(), false, Some(e.clone())),
            None => match cached.get(&key) {
                Some(row) => (
                    row.latest_version.clone(),
                    row.checked_at.clone().unwrap_or_else(now_rfc3339),
                    true,
                    row.error.clone(),
                ),
                None => (None, now_rfc3339(), false, None),
            },
        };

        if latest.is_none() && error.is_some() {
            failed.insert(format!("{} {}", d.ecosystem.osv_id(), d.raw_name));
        }

        let status = match &latest {
            Some(l) => classify_bump(d.ecosystem, &d.version, l),
            None => OutdatedStatus::Unknown,
        };

        entries.push(OutdatedEntry {
            ecosystem: d.ecosystem.osv_id().to_string(),
            name: d.name.clone(),
            raw_name: d.raw_name.clone(),
            current_version: d.version.clone(),
            latest_version: latest,
            status,
            is_direct: d.is_direct,
            scope: d.scope.clone(),
            manifest_path: d.manifest_path.clone(),
            checked_at,
            from_cache,
            error,
        });
    }

    // Direct first, then behind-ness, then name — the order the UI wants.
    entries.sort_by(|a, b| {
        b.is_direct
            .cmp(&a.is_direct)
            .then_with(|| status_rank(b.status).cmp(&status_rank(a.status)))
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.current_version.cmp(&b.current_version))
    });

    Ok(OutdatedReport {
        repo_id,
        generated_at: now_rfc3339(),
        entries,
        failed: failed.into_iter().collect(),
    })
}

fn status_rank(s: OutdatedStatus) -> u8 {
    match s {
        OutdatedStatus::OutdatedMajor => 4,
        OutdatedStatus::OutdatedMinor => 3,
        OutdatedStatus::OutdatedPatch => 2,
        OutdatedStatus::Unknown => 1,
        OutdatedStatus::UpToDate => 0,
    }
}

// ---------------------------------------------------------------------------
// Version-delta classification
// ---------------------------------------------------------------------------

/// Compare an installed version against a registry latest and bucket the
/// gap. Uses the ecosystem's own [`crate::version::VersionScheme`]; an
/// unparseable version on either side yields [`OutdatedStatus::Unknown`]
/// rather than a guess.
pub fn classify_bump(ecosystem: Ecosystem, current: &str, latest: &str) -> OutdatedStatus {
    let scheme = scheme_for(ecosystem);
    let (Ok(cur), Ok(lat)) = (scheme.parse(current), scheme.parse(latest)) else {
        return OutdatedStatus::Unknown;
    };
    if scheme.gte(&cur, &lat) {
        return OutdatedStatus::UpToDate;
    }
    let ((c_major, c_minor, _), _) = version_parts(&cur);
    let ((l_major, l_minor, _), _) = version_parts(&lat);
    if l_major != c_major {
        OutdatedStatus::OutdatedMajor
    } else if l_minor != c_minor {
        OutdatedStatus::OutdatedMinor
    } else {
        OutdatedStatus::OutdatedPatch
    }
}

/// `((major, minor, patch), is_prerelease)` for a parsed version, across all
/// three schemes. PEP 440 releases can have any arity — missing components
/// read as 0.
fn version_parts(v: &Ver) -> ((u64, u64, u64), bool) {
    match v {
        Ver::SemVer(x) => ((x.major, x.minor, x.patch), !x.pre.is_empty()),
        Ver::Go(g) => (
            (g.semver.major, g.semver.minor, g.semver.patch),
            !g.semver.pre.is_empty(),
        ),
        Ver::Pep440(p) => {
            let r = p.release();
            let at = |i: usize| r.get(i).copied().unwrap_or(0);
            ((at(0), at(1), at(2)), p.any_prerelease())
        }
    }
}

// ---------------------------------------------------------------------------
// Registry fan-out
// ---------------------------------------------------------------------------

type FetchResult = ((Ecosystem, String), Result<Option<String>, String>);

/// Fetch the latest stable version for each `(ecosystem, name)` with bounded
/// concurrency. A per-host throttle keeps request rates polite (crates.io in
/// particular asks for ~1 req/s). Network failures come back as `Err`; a
/// package the registry does not know is `Ok(None)`.
fn fetch_latest_batch(pairs: &[(Ecosystem, String)]) -> Vec<FetchResult> {
    if pairs.is_empty() {
        return Vec::new();
    }

    let client = match reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(20))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            // Whole batch fails identically.
            return pairs
                .iter()
                .map(|p| (p.clone(), Err(format!("http client: {e}"))))
                .collect();
        }
    };

    let throttle = Throttle::default();
    let cursor = AtomicUsize::new(0);
    let out: Mutex<Vec<FetchResult>> = Mutex::new(Vec::with_capacity(pairs.len()));

    std::thread::scope(|scope| {
        for _ in 0..FETCH_CONCURRENCY.min(pairs.len()) {
            scope.spawn(|| loop {
                let i = cursor.fetch_add(1, AtomicOrdering::SeqCst);
                let Some((eco, name)) = pairs.get(i) else {
                    break;
                };
                let result = fetch_latest(&client, &throttle, *eco, name);
                out.lock().unwrap().push(((*eco, name.clone()), result));
            });
        }
    });

    out.into_inner().unwrap()
}

/// Minimum spacing between requests to the same registry host.
#[derive(Default)]
struct Throttle {
    last: Mutex<HashMap<&'static str, Instant>>,
}

impl Throttle {
    fn wait(&self, host: &'static str) {
        let min_gap = match host {
            "crates.io" => Duration::from_millis(1000),
            _ => Duration::from_millis(100),
        };
        let sleep_for = {
            let mut map = self.last.lock().unwrap();
            let now = Instant::now();
            let next = match map.get(host) {
                Some(&prev) => {
                    let earliest = prev + min_gap;
                    if earliest > now {
                        earliest - now
                    } else {
                        Duration::ZERO
                    }
                }
                None => Duration::ZERO,
            };
            map.insert(host, now + next);
            next
        };
        if !sleep_for.is_zero() {
            std::thread::sleep(sleep_for);
        }
    }
}

fn fetch_latest(
    client: &reqwest::blocking::Client,
    throttle: &Throttle,
    eco: Ecosystem,
    name: &str,
) -> Result<Option<String>, String> {
    match eco {
        Ecosystem::Npm => {
            throttle.wait("registry.npmjs.org");
            // Scoped names: the `/` must be percent-encoded in the path.
            let url = format!("https://registry.npmjs.org/{}", name.replace('/', "%2F"));
            let doc = get_json(client, &url)?;
            let Some(doc) = doc else { return Ok(None) };
            Ok(parse_npm_latest(&doc))
        }
        Ecosystem::PyPI => {
            throttle.wait("pypi.org");
            let url = format!("https://pypi.org/pypi/{name}/json");
            let doc = get_json(client, &url)?;
            let Some(doc) = doc else { return Ok(None) };
            Ok(parse_pypi_latest(&doc))
        }
        Ecosystem::CratesIo => {
            throttle.wait("crates.io");
            let url = format!("https://crates.io/api/v1/crates/{name}");
            let doc = get_json(client, &url)?;
            let Some(doc) = doc else { return Ok(None) };
            Ok(parse_crates_latest(&doc))
        }
        Ecosystem::Go => {
            throttle.wait("proxy.golang.org");
            let url = format!("https://proxy.golang.org/{}/@latest", go_escape(name));
            let doc = get_json(client, &url)?;
            let Some(doc) = doc else { return Ok(None) };
            Ok(parse_go_latest(&doc))
        }
    }
}

/// GET a JSON body. `Ok(None)` for a 404 (unknown package); `Err` for any
/// other transport or status failure.
fn get_json(
    client: &reqwest::blocking::Client,
    url: &str,
) -> Result<Option<serde_json::Value>, String> {
    let resp = client
        .get(url)
        .send()
        .map_err(|e| format!("{}: {e}", short_host(url)))?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let resp = resp
        .error_for_status()
        .map_err(|e| format!("{}: {e}", short_host(url)))?;
    resp.json::<serde_json::Value>()
        .map(Some)
        .map_err(|e| format!("{}: malformed response: {e}", short_host(url)))
}

fn short_host(url: &str) -> &str {
    url.strip_prefix("https://")
        .unwrap_or(url)
        .split('/')
        .next()
        .unwrap_or(url)
}

/// Go module proxy path escaping: an uppercase letter `X` becomes `!x`
/// (so case-insensitive filesystems can host the cache).
fn go_escape(module: &str) -> String {
    let mut out = String::with_capacity(module.len() + 4);
    for c in module.chars() {
        if c.is_ascii_uppercase() {
            out.push('!');
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Registry response parsers (pure — unit-tested against captured shapes)
// ---------------------------------------------------------------------------

/// npm: `dist-tags.latest` is the maintainer-blessed latest and already
/// excludes prereleases by npm convention (FR-8.4).
fn parse_npm_latest(doc: &serde_json::Value) -> Option<String> {
    doc.get("dist-tags")?
        .get("latest")?
        .as_str()
        .map(str::to_owned)
}

/// PyPI: `info.version` is the newest non-prerelease release unless the
/// project has only prereleases (FR-8.4).
fn parse_pypi_latest(doc: &serde_json::Value) -> Option<String> {
    let v = doc.get("info")?.get("version")?.as_str()?;
    if v.is_empty() {
        None
    } else {
        Some(v.to_owned())
    }
}

/// crates.io: `crate.max_stable_version` is the highest non-prerelease
/// version; it is null only when every published version is a prerelease,
/// in which case fall back to `max_version` (FR-8.4).
fn parse_crates_latest(doc: &serde_json::Value) -> Option<String> {
    let krate = doc.get("crate")?;
    krate
        .get("max_stable_version")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .or_else(|| krate.get("max_version").and_then(|v| v.as_str()))
        .map(str::to_owned)
}

/// Go module proxy `@latest`: `{"Version": "v1.2.3", "Time": "..."}`. The
/// proxy already resolves this to the highest release tag; prerelease tags
/// on Go modules are uncommon and not separately tracked here.
fn parse_go_latest(doc: &serde_json::Value) -> Option<String> {
    doc.get("Version")?.as_str().map(str::to_owned)
}

// ---------------------------------------------------------------------------
// DB access
// ---------------------------------------------------------------------------

struct DepRow {
    ecosystem: Ecosystem,
    name: String,
    raw_name: String,
    version: String,
    is_direct: bool,
    scope: String,
    manifest_path: String,
}

/// Distinct `(ecosystem, name, version)` for a repo, with a representative
/// manifest path / scope / directness. A monorepo can pin one package at
/// several versions; each is its own row.
fn load_repo_deps(db: &Db, repo_id: i64) -> CoreResult<Vec<DepRow>> {
    let conn = db.read()?;
    let mut stmt = conn.prepare(
        "SELECT d.ecosystem,
                d.name,
                MIN(d.raw_name)  AS raw_name,
                d.version,
                MAX(d.is_direct) AS is_direct,
                MIN(d.scope)     AS scope,
                MIN(m.path)      AS manifest_path
           FROM dependencies d
           JOIN manifests m ON m.id = d.manifest_id
          WHERE d.repo_id = ?1
          GROUP BY d.ecosystem, d.name, d.version",
    )?;
    let rows = stmt
        .query_map([repo_id], |r| {
            let eco_raw: String = r.get("ecosystem")?;
            Ok((
                eco_raw,
                r.get::<_, String>("name")?,
                r.get::<_, String>("raw_name")?,
                r.get::<_, String>("version")?,
                r.get::<_, i64>("is_direct")? != 0,
                r.get::<_, String>("scope")?,
                r.get::<_, String>("manifest_path")?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    Ok(rows
        .into_iter()
        .filter_map(
            |(eco, name, raw_name, version, is_direct, scope, manifest_path)| {
                Ecosystem::from_osv_id(&eco).map(|ecosystem| DepRow {
                    ecosystem,
                    name,
                    raw_name,
                    version,
                    is_direct,
                    scope,
                    manifest_path,
                })
            },
        )
        .collect())
}

struct CacheRow {
    latest_version: Option<String>,
    checked_at: Option<String>,
    error: Option<String>,
}

fn cache_key(eco: Ecosystem, name: &str) -> String {
    format!("{}\u{0}{}", eco.osv_id(), name)
}

fn load_cache(
    db: &Db,
    pairs: &BTreeSet<(Ecosystem, String)>,
) -> CoreResult<HashMap<String, CacheRow>> {
    let conn = db.read()?;
    let mut stmt = conn.prepare(
        "SELECT latest_version, checked_at, error FROM outdated_cache
          WHERE ecosystem = ?1 AND name = ?2",
    )?;
    let mut map = HashMap::new();
    for (eco, name) in pairs {
        let row = stmt
            .query_row(rusqlite::params![eco.osv_id(), name], |r| {
                Ok(CacheRow {
                    latest_version: r.get(0)?,
                    checked_at: r.get(1)?,
                    error: r.get(2)?,
                })
            })
            .ok();
        if let Some(row) = row {
            map.insert(cache_key(*eco, name), row);
        }
    }
    Ok(map)
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_bump_semver_buckets() {
        use OutdatedStatus::*;
        assert_eq!(classify_bump(Ecosystem::Npm, "1.2.3", "1.2.3"), UpToDate);
        assert_eq!(classify_bump(Ecosystem::Npm, "1.2.4", "1.2.3"), UpToDate);
        assert_eq!(
            classify_bump(Ecosystem::Npm, "1.2.3", "1.2.9"),
            OutdatedPatch
        );
        assert_eq!(
            classify_bump(Ecosystem::Npm, "1.2.3", "1.5.0"),
            OutdatedMinor
        );
        assert_eq!(
            classify_bump(Ecosystem::Npm, "1.2.3", "3.0.0"),
            OutdatedMajor
        );
        // 1.9 -> 1.10 is a minor bump, not lexical nonsense.
        assert_eq!(
            classify_bump(Ecosystem::Npm, "1.9.0", "1.10.0"),
            OutdatedMinor
        );
    }

    #[test]
    fn classify_bump_unparseable_is_unknown() {
        assert_eq!(
            classify_bump(Ecosystem::Npm, "not-a-version", "1.0.0"),
            OutdatedStatus::Unknown
        );
        assert_eq!(
            classify_bump(Ecosystem::Npm, "1.0.0", "garbage"),
            OutdatedStatus::Unknown
        );
    }

    #[test]
    fn classify_bump_pep440_release_arity() {
        use OutdatedStatus::*;
        // PEP 440 two-component and four-component releases.
        assert_eq!(
            classify_bump(Ecosystem::PyPI, "2.0", "2.0.1"),
            OutdatedPatch
        );
        assert_eq!(classify_bump(Ecosystem::PyPI, "1.0", "2.0"), OutdatedMajor);
        // A post-release is newer than its base (packaging fix); the base
        // is a patch behind, and being on the post-release is up to date.
        assert_eq!(
            classify_bump(Ecosystem::PyPI, "1.4.0", "1.4.0.post1"),
            OutdatedPatch
        );
        assert_eq!(
            classify_bump(Ecosystem::PyPI, "1.4.0.post1", "1.4.0"),
            UpToDate
        );
        // A prerelease that is behind the stable latest still reports the gap.
        assert_eq!(
            classify_bump(Ecosystem::PyPI, "1.0.0rc1", "1.2.0"),
            OutdatedMinor
        );
    }

    #[test]
    fn classify_bump_go_v_prefix() {
        use OutdatedStatus::*;
        assert_eq!(classify_bump(Ecosystem::Go, "v1.2.3", "v1.2.3"), UpToDate);
        assert_eq!(
            classify_bump(Ecosystem::Go, "v1.2.3", "v1.3.0"),
            OutdatedMinor
        );
        assert_eq!(
            classify_bump(
                Ecosystem::Go,
                "v0.0.0-20210101000000-aaaaaaaaaaaa",
                "v0.1.0"
            ),
            OutdatedMinor
        );
    }

    #[test]
    fn npm_parser_reads_dist_tag() {
        let doc = serde_json::json!({
            "dist-tags": { "latest": "4.18.2", "next": "5.0.0-beta.1" },
            "versions": { "4.18.2": {}, "5.0.0-beta.1": {} }
        });
        assert_eq!(parse_npm_latest(&doc).as_deref(), Some("4.18.2"));
    }

    #[test]
    fn pypi_parser_reads_info_version() {
        let doc = serde_json::json!({
            "info": { "version": "2.31.0" },
            "releases": { "2.31.0": [], "3.0.0a1": [] }
        });
        assert_eq!(parse_pypi_latest(&doc).as_deref(), Some("2.31.0"));
    }

    #[test]
    fn crates_parser_prefers_max_stable() {
        let doc = serde_json::json!({
            "crate": { "max_stable_version": "1.0.203", "max_version": "1.0.204-alpha.1" }
        });
        assert_eq!(parse_crates_latest(&doc).as_deref(), Some("1.0.203"));

        let only_pre = serde_json::json!({
            "crate": { "max_stable_version": serde_json::Value::Null, "max_version": "0.1.0-rc.1" }
        });
        assert_eq!(
            parse_crates_latest(&only_pre).as_deref(),
            Some("0.1.0-rc.1")
        );
    }

    #[test]
    fn go_parser_reads_version_field() {
        let doc = serde_json::json!({ "Version": "v1.9.1", "Time": "2024-01-01T00:00:00Z" });
        assert_eq!(parse_go_latest(&doc).as_deref(), Some("v1.9.1"));
    }

    #[test]
    fn go_escape_lowercases_with_bang() {
        assert_eq!(
            go_escape("github.com/Azure/azure-sdk-for-go"),
            "github.com/!azure/azure-sdk-for-go"
        );
        assert_eq!(go_escape("golang.org/x/net"), "golang.org/x/net");
    }

    #[test]
    fn empty_repo_yields_empty_report() {
        let db = Db::open_in_memory().unwrap();
        db.write(|c| {
            c.execute_batch(
                "INSERT INTO scan_roots (id, path, added_at) VALUES (1, '/r', '2026-01-01T00:00:00Z');
                 INSERT INTO repos (id, root_id, path, name) VALUES (1, 1, '/r/a', 'a');",
            )
            .map_err(Into::into)
        })
        .unwrap();
        let report = check_repo_outdated(&db, 1, false).unwrap();
        assert!(report.entries.is_empty());
        assert!(report.failed.is_empty());
    }

    /// The cache is honored: a fresh row is reused without a network call,
    /// and the delta is computed from it. (No network in tests — if the
    /// cache were *not* consulted this would try to reach npm and fail.)
    #[test]
    fn fresh_cache_row_is_reused_without_network() {
        let db = Db::open_in_memory().unwrap();
        db.write(|c| {
            c.execute_batch(
                r#"
                INSERT INTO scan_roots (id, path, added_at) VALUES (1, '/r', '2026-01-01T00:00:00Z');
                INSERT INTO repos (id, root_id, path, name) VALUES (1, 1, '/r/a', 'a');
                INSERT INTO manifests (id, repo_id, path, ecosystem, kind, content_hash)
                    VALUES (1, 1, 'package.json', 'npm', 'manifest', 'h');
                INSERT INTO dependencies (id, repo_id, manifest_id, ecosystem, name, raw_name, version, confidence, scope, is_direct)
                    VALUES (1, 1, 1, 'npm', 'left-pad', 'left-pad', '1.1.0', 'range', 'runtime', 1);
                "#,
            )
            .map_err(Into::into)
        })
        .unwrap();
        let now = chrono::Utc::now().to_rfc3339();
        db.write(|c| {
            c.execute(
                "INSERT INTO outdated_cache (ecosystem, name, latest_version, checked_at, error)
                 VALUES ('npm', 'left-pad', '1.3.0', ?1, NULL)",
                [&now],
            )
            .map(|_| ())
            .map_err(Into::into)
        })
        .unwrap();

        let report = check_repo_outdated(&db, 1, false).unwrap();
        assert_eq!(report.entries.len(), 1);
        let e = &report.entries[0];
        assert!(e.from_cache);
        assert_eq!(e.latest_version.as_deref(), Some("1.3.0"));
        assert_eq!(e.status, OutdatedStatus::OutdatedMinor);
        assert!(report.failed.is_empty());
    }
}
