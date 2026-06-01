-- Migration 0017 (canvas DB variant for canvas_separate_db mode)
-- Same schema as 0017_overlay_refs_manifest_cache.sql.
-- These tables belong to canvas.sqlite (local canvas daemon), not metadata.sqlite.

-- ── overlay_refs ──────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS overlay_refs (
    overlay_id TEXT NOT NULL,
    repo_id TEXT NOT NULL,
    ref_kind TEXT NOT NULL CHECK(ref_kind IN ('node','edge')),
    canonical_ref TEXT NOT NULL,
    local_node_id TEXT,
    local_edge_id TEXT,
    resolved INTEGER NOT NULL DEFAULT 0,
    unresolved_reason TEXT,
    synced_at TEXT NOT NULL,
    PRIMARY KEY (overlay_id, canonical_ref)
);

CREATE INDEX IF NOT EXISTS idx_overlay_refs_overlay ON overlay_refs(overlay_id);
CREATE INDEX IF NOT EXISTS idx_overlay_refs_node ON overlay_refs(local_node_id);
CREATE INDEX IF NOT EXISTS idx_overlay_refs_repo ON overlay_refs(repo_id);
CREATE INDEX IF NOT EXISTS idx_overlay_refs_canonical ON overlay_refs(canonical_ref);

-- ── manifest_cache ────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS manifest_cache (
    repo_id TEXT PRIMARY KEY,
    default_branch TEXT NOT NULL,
    primary_languages TEXT NOT NULL,
    source_roots_json TEXT NOT NULL DEFAULT '[]',
    last_synced_at TEXT NOT NULL,
    cloud_version_hash TEXT
);