//! HTTP handlers for Repo Canvas API.

pub(super) mod conflict;
pub(super) mod generator;
pub(super) mod handlers;

pub(super) use handlers::{
    create_snapshot, get_latest_snapshot, get_version_hash, query_canvas, refresh_canvas,
    sync_status,
};

use serde::Deserialize;

use super::{AppState, HandlerResult};

// ─── Query / Request structs ──────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(super) struct CanvasQueryParams {
    pub repo_id: String,
    pub component: Option<String>,
    #[serde(rename = "type")]
    pub canvas_type: Option<String>,
    pub node_type: Option<String>,
    pub status: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct CanvasRefreshRequest {
    pub repo_id: String,
    pub generator: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct CanvasSnapshotRequest {
    pub repo_id: String,
    pub merge_commit: String,
    pub snapshot_type: Option<String>,
}
