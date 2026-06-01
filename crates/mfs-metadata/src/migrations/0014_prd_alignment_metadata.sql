-- Migration 0014 (metadata DB only, canvas_separate_db variant)
-- Only contains manifest_repo_identity ALTER statements.
-- Canvas/overlay ALTER statements go to canvas DB via 0014_canvas.sql.

-- ─── manifest_repo_identity: PRD §4.1.1 missing columns ─────────────────
ALTER TABLE manifest_repo_identity ADD COLUMN repo_name TEXT;
ALTER TABLE manifest_repo_identity ADD COLUMN repo_path TEXT;
ALTER TABLE manifest_repo_identity ADD COLUMN last_commit_hash TEXT;
ALTER TABLE manifest_repo_identity ADD COLUMN last_commit_date TEXT;
ALTER TABLE manifest_repo_identity ADD COLUMN manifest_version TEXT NOT NULL DEFAULT '1';
ALTER TABLE manifest_repo_identity ADD COLUMN yaml_hash TEXT;
ALTER TABLE manifest_repo_identity ADD COLUMN source_roots_json TEXT NOT NULL DEFAULT '[]';
ALTER TABLE manifest_repo_identity ADD COLUMN quality_gates_json TEXT NOT NULL DEFAULT '{}';
ALTER TABLE manifest_repo_identity ADD COLUMN updated_at TEXT NOT NULL DEFAULT '';
UPDATE manifest_repo_identity SET updated_at = datetime('now') WHERE updated_at = '';