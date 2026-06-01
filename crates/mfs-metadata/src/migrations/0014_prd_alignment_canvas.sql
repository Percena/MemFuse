-- Migration 0014 (canvas DB only, canvas_separate_db variant)
-- Canvas/overlay ALTER statements that must run in the canvas DB.
-- Note: manifest_id already included in 0013_canvas_overlay_separate.sql,
-- so we only add created_at, updated_at, and overlay timestamp columns.

-- ─── canvas_nodes: PRD §4.2 missing columns ────────────────────────────
-- manifest_id already present from 0013_separate; skip duplicate.
ALTER TABLE canvas_nodes ADD COLUMN created_at TEXT NOT NULL DEFAULT '';
ALTER TABLE canvas_nodes ADD COLUMN updated_at TEXT NOT NULL DEFAULT '';

-- ─── canvas_edges: PRD §4.2 missing columns ─────────────────────────────
-- manifest_id already present from 0013_separate; skip duplicate.
ALTER TABLE canvas_edges ADD COLUMN created_at TEXT NOT NULL DEFAULT '';
ALTER TABLE canvas_edges ADD COLUMN updated_at TEXT NOT NULL DEFAULT '';

-- ─── active_overlays: PRD §4.3 missing columns ──────────────────────────
-- manifest_id already present from 0013_separate; skip duplicate.
ALTER TABLE active_overlays ADD COLUMN accepted_at TEXT;
ALTER TABLE active_overlays ADD COLUMN implemented_at TEXT;
ALTER TABLE active_overlays ADD COLUMN merged_at TEXT;
ALTER TABLE active_overlays ADD COLUMN stale_at TEXT;
ALTER TABLE active_overlays ADD COLUMN abandoned_at TEXT;