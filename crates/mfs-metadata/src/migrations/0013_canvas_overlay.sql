-- Migration 0013: Canvas/Overlay data model (5 tables)
-- Canvas is a brand-new data model, independent from Fact/Episode/Resource tables.

-- 1. Component/module/type/function nodes
CREATE TABLE IF NOT EXISTS canvas_nodes (
    id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    node_type TEXT NOT NULL CHECK(node_type IN ('component','module','type','function','entry_point','config','test_suite')),
    name TEXT NOT NULL,
    path TEXT,
    language TEXT,
    purpose TEXT,
    confidence TEXT NOT NULL DEFAULT 'deterministic' CHECK(confidence IN ('deterministic','reviewed','inferred','stale')),
    generator TEXT NOT NULL,
    generated_at TEXT NOT NULL,
    version_hash TEXT NOT NULL,
    source TEXT,
    FOREIGN KEY (repo_id) REFERENCES manifest_repo_identity(repo_id)
);

CREATE INDEX IF NOT EXISTS idx_canvas_nodes_repo ON canvas_nodes(repo_id);
CREATE INDEX IF NOT EXISTS idx_canvas_nodes_type ON canvas_nodes(node_type);
CREATE INDEX IF NOT EXISTS idx_canvas_nodes_name ON canvas_nodes(name);

-- 2. Dependency/call/contract edges
CREATE TABLE IF NOT EXISTS canvas_edges (
    id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    edge_type TEXT NOT NULL CHECK(edge_type IN ('import','call','contract','depends_on','implements','tests')),
    source_node_id TEXT NOT NULL,
    target_node_id TEXT NOT NULL,
    contract_spec TEXT,
    confidence TEXT NOT NULL DEFAULT 'deterministic' CHECK(confidence IN ('deterministic','reviewed','inferred','stale')),
    generator TEXT NOT NULL,
    generated_at TEXT NOT NULL,
    version_hash TEXT NOT NULL,
    FOREIGN KEY (repo_id) REFERENCES manifest_repo_identity(repo_id),
    FOREIGN KEY (source_node_id) REFERENCES canvas_nodes(id),
    FOREIGN KEY (target_node_id) REFERENCES canvas_nodes(id)
);

CREATE INDEX IF NOT EXISTS idx_canvas_edges_repo ON canvas_edges(repo_id);
CREATE INDEX IF NOT EXISTS idx_canvas_edges_source ON canvas_edges(source_node_id);
CREATE INDEX IF NOT EXISTS idx_canvas_edges_target ON canvas_edges(target_node_id);

-- 3. Active Overlay (issue/PR-level plan declaration — different semantics from overlay.rs)
CREATE TABLE IF NOT EXISTS active_overlays (
    id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    overlay_type TEXT NOT NULL CHECK(overlay_type IN ('planned_change','planned_contract','conflict_declaration','planned_test','planned_config')),
    tracker TEXT NOT NULL DEFAULT 'github_projects',
    tracker_content_id TEXT NOT NULL,
    tracker_project_item_id TEXT,
    tracker_identifier TEXT NOT NULL,
    issue_number INTEGER,
    branch TEXT,
    pr_url TEXT,
    agent_session_id TEXT,
    author TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'proposed' CHECK(status IN ('proposed','accepted','implemented','merged','abandoned','stale','rejected')),
    content_json TEXT NOT NULL,
    affected_nodes TEXT,
    affected_edges TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    superseded_by TEXT,
    FOREIGN KEY (repo_id) REFERENCES manifest_repo_identity(repo_id)
);

CREATE INDEX IF NOT EXISTS idx_active_overlays_repo ON active_overlays(repo_id);
CREATE INDEX IF NOT EXISTS idx_active_overlays_tracker_id ON active_overlays(tracker, tracker_identifier);
CREATE INDEX IF NOT EXISTS idx_active_overlays_project_item ON active_overlays(tracker_project_item_id);
CREATE INDEX IF NOT EXISTS idx_active_overlays_status ON active_overlays(status);

-- 4. Overlay state transition log
CREATE TABLE IF NOT EXISTS overlay_state_transitions (
    id TEXT PRIMARY KEY,
    overlay_id TEXT NOT NULL,
    from_status TEXT NOT NULL CHECK(from_status IN ('(none)','proposed','accepted','implemented','merged','abandoned','stale','rejected')),
    to_status TEXT NOT NULL CHECK(to_status IN ('proposed','accepted','implemented','merged','abandoned','stale','rejected')),
    triggered_by TEXT NOT NULL CHECK(triggered_by IN ('human','agent','consolidation','drift_detection','periodic_hygiene')),
    reason TEXT,
    created_at TEXT NOT NULL,
    FOREIGN KEY (overlay_id) REFERENCES active_overlays(id)
);

CREATE INDEX IF NOT EXISTS idx_overlay_transitions_overlay ON overlay_state_transitions(overlay_id);

-- 5. Immutable history snapshots (post-merge)
CREATE TABLE IF NOT EXISTS canvas_snapshots (
    id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    merge_commit TEXT NOT NULL,
    snapshot_type TEXT NOT NULL CHECK(snapshot_type IN ('structural','contract','status','full')),
    snapshot_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    immutable INTEGER NOT NULL DEFAULT 1,
    FOREIGN KEY (repo_id) REFERENCES manifest_repo_identity(repo_id)
);

CREATE INDEX IF NOT EXISTS idx_canvas_snapshots_repo ON canvas_snapshots(repo_id);
CREATE INDEX IF NOT EXISTS idx_canvas_snapshots_commit ON canvas_snapshots(merge_commit);

CREATE TRIGGER IF NOT EXISTS reject_immutable_canvas_snapshot_update
BEFORE UPDATE ON canvas_snapshots
FOR EACH ROW
WHEN OLD.immutable = 1
BEGIN
    SELECT RAISE(ABORT, 'canvas snapshot is immutable and cannot be updated');
END;

CREATE TRIGGER IF NOT EXISTS reject_immutable_canvas_snapshot_delete
BEFORE DELETE ON canvas_snapshots
FOR EACH ROW
WHEN OLD.immutable = 1
BEGIN
    SELECT RAISE(ABORT, 'canvas snapshot is immutable and cannot be deleted');
END;
