-- Migration 0017: overlay_refs mapping table + manifest_cache local sync table
-- Per §5.2 of SaaS-Canvas-Local-Architecture.md:
--   overlay_refs: maps cloud overlay canonical refs to local canvas IDs
--   manifest_cache: caches cloud manifest metadata for local canvas generation

-- ── overlay_refs: cloud overlay ID → local canvas ID mapping ────────────────
-- Enables SaaS mode: cloud stores canonical refs, local daemon resolves them
-- to current canvas_nodes/canvas_edges IDs.
CREATE TABLE IF NOT EXISTS overlay_refs (
    overlay_id TEXT NOT NULL,        -- cloud active_overlays.id (text reference)
    repo_id TEXT NOT NULL,
    ref_kind TEXT NOT NULL CHECK(ref_kind IN ('node','edge')),
    canonical_ref TEXT NOT NULL,     -- e.g. canvas://symphony-gh/node/module/Symphony.Orchestrator
    local_node_id TEXT,              -- local canvas_nodes.id (NULL if unresolved)
    local_edge_id TEXT,              -- local canvas_edges.id (NULL if unresolved)
    resolved INTEGER NOT NULL DEFAULT 0,  -- 0 = unresolved, 1 = resolved
    unresolved_reason TEXT,          -- e.g. "node not found", "renamed component"
    synced_at TEXT NOT NULL,         -- last sync timestamp from cloud
    PRIMARY KEY (overlay_id, canonical_ref)
);

CREATE INDEX IF NOT EXISTS idx_overlay_refs_overlay ON overlay_refs(overlay_id);
CREATE INDEX IF NOT EXISTS idx_overlay_refs_node ON overlay_refs(local_node_id);
CREATE INDEX IF NOT EXISTS idx_overlay_refs_repo ON overlay_refs(repo_id);
CREATE INDEX IF NOT EXISTS idx_overlay_refs_canonical ON overlay_refs(canonical_ref);

-- ── manifest_cache: local cache of cloud manifest metadata ──────────────────
-- Canvas generation reads default_branch, primary_languages, source_roots from
-- this cache instead of requiring network access to cloud every time.
CREATE TABLE IF NOT EXISTS manifest_cache (
    repo_id TEXT PRIMARY KEY,             -- corresponds to cloud manifest_repo_identity.repo_id
    default_branch TEXT NOT NULL,          -- e.g. "main"
    primary_languages TEXT NOT NULL,       -- JSON array, e.g. '["elixir"]'
    source_roots_json TEXT NOT NULL DEFAULT '[]',
    last_synced_at TEXT NOT NULL,          -- last sync timestamp from cloud
    cloud_version_hash TEXT               -- cloud manifest version hash for stale detection
);