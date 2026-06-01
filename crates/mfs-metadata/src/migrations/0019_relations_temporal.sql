-- Migration 0019: temporal fields on relations table
-- Adds valid_from, valid_to, tcommit, is_latest, superseded_by to relations,
-- mirroring the same-table supersession pattern already used by facts.
-- Existing rows are assumed "valid from creation until now" (open-ended).

-- Add temporal columns (idempotent via ensure_column in Rust migration function)
ALTER TABLE relations ADD COLUMN valid_from TEXT;
ALTER TABLE relations ADD COLUMN valid_to TEXT;
ALTER TABLE relations ADD COLUMN tcommit TEXT;
ALTER TABLE relations ADD COLUMN is_latest INTEGER NOT NULL DEFAULT 1;
ALTER TABLE relations ADD COLUMN superseded_by TEXT;

-- Backfill existing rows: valid_from = created_at-equivalent (updated_at),
-- valid_to = NULL (open-ended), is_latest = 1, tcommit = updated_at
UPDATE relations SET
    valid_from = updated_at,
    valid_to = NULL,
    tcommit = updated_at,
    is_latest = 1,
    superseded_by = NULL
WHERE valid_from IS NULL;

-- New index for temporal queries: AS OF filtering on latest edges
CREATE INDEX IF NOT EXISTS idx_relations_temporal_latest
    ON relations (account_id, user_id, is_latest, valid_from, valid_to);