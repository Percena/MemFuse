-- Migration 0016: manifest_yaml_path nullable
-- SaaS mode: manifest_yaml_path is not required when content is uploaded
-- via /manifest/update (content or manifest_json fields) instead of local file.
-- Cloud manifests use "cloud://{repo_id}" as placeholder.
--
-- SQLite does not support ALTER TABLE ... DROP NOT NULL directly.
-- Workaround: recreate the table with the column as nullable.

-- Step 1: Create new table with manifest_yaml_path as nullable
CREATE TABLE IF NOT EXISTS manifest_repo_identity_new (
    repo_id TEXT PRIMARY KEY,
    resource_uri TEXT NOT NULL UNIQUE,
    default_branch TEXT NOT NULL DEFAULT 'main',
    primary_languages TEXT NOT NULL,
    created_at TEXT NOT NULL,
    last_verified_at TEXT NOT NULL,
    manifest_yaml_path TEXT,
    repo_name TEXT,
    repo_path TEXT,
    last_commit_hash TEXT,
    last_commit_date TEXT,
    manifest_version TEXT NOT NULL DEFAULT '1',
    yaml_hash TEXT,
    source_roots_json TEXT NOT NULL DEFAULT '[]',
    quality_gates_json TEXT NOT NULL DEFAULT '{}',
    updated_at TEXT NOT NULL DEFAULT ''
);

-- Step 2: Copy data from old table
INSERT INTO manifest_repo_identity_new
    SELECT repo_id, resource_uri, default_branch, primary_languages,
           created_at, last_verified_at, manifest_yaml_path,
           repo_name, repo_path, last_commit_hash, last_commit_date,
           manifest_version, yaml_hash, source_roots_json,
           quality_gates_json, updated_at
    FROM manifest_repo_identity;

-- Step 3: Drop old table
DROP TABLE manifest_repo_identity;

-- Step 4: Rename new table
ALTER TABLE manifest_repo_identity_new RENAME TO manifest_repo_identity;

-- Step 5: Recreate index
CREATE INDEX IF NOT EXISTS idx_manifest_repo_uri
    ON manifest_repo_identity(resource_uri);