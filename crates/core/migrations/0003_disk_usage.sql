-- Migration 0003: per-repository disk accounting.
--
-- Feeds the Cleanup view: how much space each repository occupies and how much
-- of that is regenerable build output (`node_modules`, `target`, `.venv`, …)
-- that can be deleted and rebuilt on demand.
--
-- Columns are nullable with no default so "never measured" stays
-- distinguishable from "measured as zero" — a repo scanned by an older build
-- must not be reported as occupying 0 bytes.

ALTER TABLE repos ADD COLUMN disk_total_bytes       INTEGER;
ALTER TABLE repos ADD COLUMN disk_git_bytes         INTEGER;
ALTER TABLE repos ADD COLUMN disk_reclaimable_bytes INTEGER;
ALTER TABLE repos ADD COLUMN disk_file_count        INTEGER;
-- 1 when the walk hit its entry cap, so every figure above is a lower bound.
ALTER TABLE repos ADD COLUMN disk_truncated         INTEGER;
ALTER TABLE repos ADD COLUMN disk_measured_at       TEXT;

-- One row per regenerable directory, so the UI can show *what* to delete
-- rather than only a total. Replaced wholesale on each measurement.
CREATE TABLE repo_disk_dirs (
    id         INTEGER PRIMARY KEY,
    repo_id    INTEGER NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
    rel_path   TEXT    NOT NULL,   -- repo-relative, '/'-separated
    kind       TEXT    NOT NULL,   -- the directory name that matched
    bytes      INTEGER NOT NULL,
    file_count INTEGER NOT NULL,
    UNIQUE (repo_id, rel_path)
);

CREATE INDEX idx_repo_disk_dirs_repo  ON repo_disk_dirs(repo_id);
CREATE INDEX idx_repo_disk_dirs_bytes ON repo_disk_dirs(bytes DESC);

-- The Cleanup view's default ordering.
CREATE INDEX idx_repos_reclaimable ON repos(disk_reclaimable_bytes DESC);
