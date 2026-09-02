-- repo-radar schema, migration 0001 (DESIGN §5.3, §5.4).
-- Every table here holds derived data: rebuildable from the user's repos or
-- from OSV. That is why `synchronous = NORMAL` and "Reset database" are
-- acceptable recovery paths.

CREATE TABLE scan_roots (
    id       INTEGER PRIMARY KEY,
    path     TEXT NOT NULL UNIQUE,
    enabled  INTEGER NOT NULL DEFAULT 1,
    added_at TEXT NOT NULL
);

CREATE TABLE repos (
    id                  INTEGER PRIMARY KEY,
    root_id             INTEGER NOT NULL REFERENCES scan_roots(id) ON DELETE CASCADE,
    parent_repo_id      INTEGER REFERENCES repos(id) ON DELETE CASCADE,  -- FR-1.5 submodules
    path                TEXT NOT NULL UNIQUE,
    name                TEXT NOT NULL,
    is_bare             INTEGER NOT NULL DEFAULT 0,
    is_monorepo         INTEGER NOT NULL DEFAULT 0,

    -- git (FR-7)
    head_sha            TEXT,
    branch              TEXT,
    last_commit_at      TEXT,
    last_commit_summary TEXT,
    commits_90d         INTEGER,
    commits_total       INTEGER,
    author_count        INTEGER,
    dirty_modified      INTEGER,
    dirty_staged        INTEGER,
    dirty_untracked     INTEGER,
    ahead               INTEGER,
    behind              INTEGER,
    remote_url          TEXT,
    branch_count        INTEGER,
    has_stash           INTEGER,

    -- classification (FR-3)
    category            TEXT,
    category_confidence TEXT,
    category_scores     TEXT,     -- JSON: full breakdown for FR-3.6
    category_manual     TEXT,     -- FR-3.7 override; NULL when not overridden

    -- health (FR-6)
    health_score        INTEGER,
    health_band         TEXT,
    health_breakdown    TEXT,     -- JSON: every deduction, for FR-6.9

    last_scanned_at     TEXT,
    scan_fingerprint    TEXT      -- §6.5 incremental key
);

CREATE TABLE repo_languages (
    repo_id       INTEGER NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
    language      TEXT NOT NULL,
    code_lines    INTEGER NOT NULL,
    comment_lines INTEGER NOT NULL,
    files         INTEGER NOT NULL,
    percentage    REAL NOT NULL,
    PRIMARY KEY (repo_id, language)
);

CREATE TABLE repo_technologies (
    id       INTEGER PRIMARY KEY,
    repo_id  INTEGER NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
    tech     TEXT NOT NULL,
    kind     TEXT NOT NULL,   -- framework | tooling | package-manager | runtime
    evidence TEXT NOT NULL    -- JSON array of signals — FR-2.4
);

CREATE TABLE manifests (
    id           INTEGER PRIMARY KEY,
    repo_id      INTEGER NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
    path         TEXT NOT NULL,          -- repo-relative
    ecosystem    TEXT NOT NULL,
    kind         TEXT NOT NULL,          -- lockfile | manifest
    content_hash TEXT NOT NULL,          -- §6.5
    UNIQUE (repo_id, path)
);

CREATE TABLE dependencies (
    id          INTEGER PRIMARY KEY,
    repo_id     INTEGER NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
    manifest_id INTEGER NOT NULL REFERENCES manifests(id) ON DELETE CASCADE,
    ecosystem   TEXT NOT NULL,
    name        TEXT NOT NULL,           -- normalized (FR-4.5)
    raw_name    TEXT NOT NULL,
    version     TEXT NOT NULL,
    confidence  TEXT NOT NULL,
    scope       TEXT NOT NULL,
    is_direct   INTEGER NOT NULL
);

CREATE TABLE advisories (
    id         TEXT PRIMARY KEY,       -- OSV id
    kind       TEXT NOT NULL,          -- compromise | vulnerability  (FR-6.1)
    summary    TEXT,
    details    TEXT,
    severity   TEXT NOT NULL,
    cvss_score REAL,
    published  TEXT,
    modified   TEXT NOT NULL,
    aliases    TEXT,                   -- JSON array
    refs       TEXT,                   -- JSON array
    withdrawn  TEXT                    -- non-NULL => excluded from matching
);

-- Event-based ranges, the normal OSV representation (§8.2)
CREATE TABLE affected_ranges (
    id           INTEGER PRIMARY KEY,
    advisory_id  TEXT NOT NULL REFERENCES advisories(id) ON DELETE CASCADE,
    ecosystem    TEXT NOT NULL,
    package_name TEXT NOT NULL,         -- normalized the same way as dependencies.name
    range_type   TEXT NOT NULL,         -- SEMVER | ECOSYSTEM
    events       TEXT NOT NULL          -- JSON: [{"introduced":"0"},{"fixed":"1.2.3"}]
);

-- Explicit version enumeration, when the advisory provides one
CREATE TABLE affected_versions (
    advisory_id  TEXT NOT NULL REFERENCES advisories(id) ON DELETE CASCADE,
    ecosystem    TEXT NOT NULL,
    package_name TEXT NOT NULL,
    version      TEXT NOT NULL,
    PRIMARY KEY (advisory_id, ecosystem, package_name, version)
);

CREATE TABLE findings (
    id            INTEGER PRIMARY KEY,
    repo_id       INTEGER NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
    dependency_id INTEGER NOT NULL REFERENCES dependencies(id) ON DELETE CASCADE,
    advisory_id   TEXT NOT NULL REFERENCES advisories(id) ON DELETE CASCADE,
    kind          TEXT NOT NULL,
    confidence    TEXT NOT NULL,
    fixed_version TEXT,
    deduction     REAL NOT NULL,
    suppressed    INTEGER NOT NULL DEFAULT 0,
    UNIQUE (repo_id, dependency_id, advisory_id)
);

CREATE TABLE outdated_cache (
    ecosystem      TEXT NOT NULL,
    name           TEXT NOT NULL,
    latest_version TEXT,
    checked_at     TEXT NOT NULL,
    error          TEXT,
    PRIMARY KEY (ecosystem, name)
);

CREATE TABLE scans (
    id          INTEGER PRIMARY KEY,
    started_at  TEXT NOT NULL,
    finished_at TEXT,
    repo_count  INTEGER,
    status      TEXT NOT NULL,     -- running | complete | cancelled | failed
    warnings    TEXT               -- JSON array
);

CREATE TABLE sync_log (
    id          INTEGER PRIMARY KEY,
    ecosystem   TEXT NOT NULL,
    started_at  TEXT NOT NULL,
    finished_at TEXT,
    mode        TEXT NOT NULL,     -- full | incremental
    count       INTEGER,
    status      TEXT NOT NULL,
    error       TEXT
);

-- Indexes (DESIGN §5.4).
-- The two that carry the matcher: without these the join in §8.4 degrades
-- to a scan over every advisory range in the database.
CREATE INDEX idx_affected_ranges_pkg   ON affected_ranges(ecosystem, package_name);
CREATE INDEX idx_affected_versions_pkg ON affected_versions(ecosystem, package_name);
CREATE INDEX idx_dependencies_pkg      ON dependencies(ecosystem, name);

CREATE INDEX idx_dependencies_repo     ON dependencies(repo_id);
CREATE INDEX idx_findings_repo         ON findings(repo_id);
CREATE INDEX idx_findings_advisory     ON findings(advisory_id);   -- "which repos does this advisory hit?"
CREATE INDEX idx_repos_root            ON repos(root_id);
CREATE INDEX idx_manifests_repo        ON manifests(repo_id);
