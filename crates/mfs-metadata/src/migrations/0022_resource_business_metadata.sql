ALTER TABLE resource_sources ADD COLUMN repo_id TEXT;
ALTER TABLE resource_sources ADD COLUMN tracker TEXT;
ALTER TABLE resource_sources ADD COLUMN tracker_project_identifier TEXT;

CREATE INDEX IF NOT EXISTS idx_resource_sources_repo_id
    ON resource_sources(account_id, user_id, repo_id);
