-- Multi-episode provenance:
-- Rename source_episode_id → source_episode_ids_json to support
-- facts derived from multiple episodes (JSON array of episode IDs).
-- Also applies to fact_assertions table.

-- ── facts table ──────────────────────────────────────────────────────────

-- Drop old index (column name will change)
DROP INDEX IF EXISTS idx_facts_source_episode;

-- Rename column: source_episode_id → source_episode_ids_json
ALTER TABLE facts RENAME COLUMN source_episode_id TO source_episode_ids_json;

-- Backfill: convert any existing single episode_id value to JSON array format
-- e.g. "ep_123" → ["ep_123"], NULL stays NULL
UPDATE facts
SET source_episode_ids_json = json_array(source_episode_ids_json)
WHERE source_episode_ids_json IS NOT NULL
  AND source_episode_ids_json NOT LIKE '[%';

-- Recreate index on renamed column
CREATE INDEX IF NOT EXISTS idx_facts_source_episodes
    ON facts(account_id, user_id, source_episode_ids_json);

-- ── fact_assertions table ────────────────────────────────────────────────

-- Rename column: source_episode_id → source_episode_ids_json
ALTER TABLE fact_assertions RENAME COLUMN source_episode_id TO source_episode_ids_json;

-- Backfill: same conversion for assertions
UPDATE fact_assertions
SET source_episode_ids_json = json_array(source_episode_ids_json)
WHERE source_episode_ids_json IS NOT NULL
  AND source_episode_ids_json NOT LIKE '[%';