CREATE INDEX IF NOT EXISTS idx_episode_recall_candidates_by_resource
    ON episode_chunks(account_id, user_id, resource_id, archived_at, salience_score DESC, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_episode_recall_candidates_all_resources
    ON episode_chunks(account_id, user_id, archived_at, salience_score DESC, created_at DESC);
