-- Migration 0002: explicit scan-root ordering (FR-10.1, "add/remove/reorder").
-- 0001 had no ordering column, so the settings list could only sort by
-- insertion time. `position` lets the user reorder roots; existing rows seed
-- from their rowid so the current order is preserved.

ALTER TABLE scan_roots ADD COLUMN position INTEGER NOT NULL DEFAULT 0;
UPDATE scan_roots SET position = id;
