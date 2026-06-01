-- Migration 0012: manifest_repo_identity
-- Anchor table for Repo Knowledge Manifest, serves as FK target for Canvas/Overlay tables.

CREATE TABLE IF NOT EXISTS manifest_repo_identity (
    repo_id TEXT PRIMARY KEY,
    resource_uri TEXT NOT NULL UNIQUE,
    default_branch TEXT NOT NULL DEFAULT 'main',
    primary_languages TEXT NOT NULL,
    created_at TEXT NOT NULL,
    last_verified_at TEXT NOT NULL,
    manifest_yaml_path TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_manifest_repo_uri
    ON manifest_repo_identity(resource_uri);