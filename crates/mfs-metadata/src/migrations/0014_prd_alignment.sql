-- Migration 0014: Align data model with PRD-MemFuse §4.1–4.4 field requirements
-- Adds missing columns to manifest_repo_identity, canvas_nodes, canvas_edges, active_overlays
-- Uses ALTER TABLE ADD COLUMN only (safe, no rename, no drop).
-- NOTE: SQLite ALTER TABLE ADD COLUMN requires constant DEFAULT values; non-constant
-- expressions like (datetime('now')) are rejected on existing databases with rows.
-- We use DEFAULT '' and then UPDATE existing rows with a proper timestamp.

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

-- ─── canvas_nodes: PRD §4.2 missing columns ────────────────────────────
ALTER TABLE canvas_nodes ADD COLUMN manifest_id TEXT REFERENCES manifest_repo_identity(repo_id);
ALTER TABLE canvas_nodes ADD COLUMN created_at TEXT NOT NULL DEFAULT '';
ALTER TABLE canvas_nodes ADD COLUMN updated_at TEXT NOT NULL DEFAULT '';
UPDATE canvas_nodes SET created_at = datetime('now'), updated_at = datetime('now') WHERE created_at = '';

-- ─── canvas_edges: PRD §4.2 missing columns ─────────────────────────────
ALTER TABLE canvas_edges ADD COLUMN manifest_id TEXT REFERENCES manifest_repo_identity(repo_id);
ALTER TABLE canvas_edges ADD COLUMN created_at TEXT NOT NULL DEFAULT '';
ALTER TABLE canvas_edges ADD COLUMN updated_at TEXT NOT NULL DEFAULT '';
UPDATE canvas_edges SET created_at = datetime('now'), updated_at = datetime('now') WHERE created_at = '';

-- ─── active_overlays: PRD §4.3 missing columns ──────────────────────────
ALTER TABLE active_overlays ADD COLUMN manifest_id TEXT REFERENCES manifest_repo_identity(repo_id);
ALTER TABLE active_overlays ADD COLUMN accepted_at TEXT;
ALTER TABLE active_overlays ADD COLUMN implemented_at TEXT;
ALTER TABLE active_overlays ADD COLUMN merged_at TEXT;
ALTER TABLE active_overlays ADD COLUMN stale_at TEXT;
ALTER TABLE active_overlays ADD COLUMN abandoned_at TEXT;