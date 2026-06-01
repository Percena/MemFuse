use super::{AppState, HandlerResult};
use axum::Json;
use axum::extract::State;
use mfs_types::MfsError;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

#[derive(Debug, Deserialize)]
pub(super) struct WritebackRequest {
    pub repo_id: String,
    pub run_id: String,
    pub tracker: Option<String>,
    pub tracker_identifier: Option<String>,
    pub base_commit: Option<String>,
    pub head_commit: Option<String>,
    pub changed_files: Option<Value>,
    pub validation_results: Option<Value>,
    pub run_evidence: Option<Value>,
    pub memory_update_proposal: Option<Value>,
    pub followup_issues: Option<Value>,
    pub idempotency_key: Option<String>,
}

pub(super) async fn writeback_run(
    State(state): State<Arc<AppState>>,
    Json(request): Json<WritebackRequest>,
) -> HandlerResult<Json<Value>> {
    let metadata = state.metadata.clone();

    let tracker = request.tracker.as_deref().unwrap_or("github_projects");
    let tracker_identifier = request.tracker_identifier.as_deref().unwrap_or("");
    let idempotency_key = request.idempotency_key.as_deref().unwrap_or("");

    // Build the writeback payload JSON from all provided fields
    let payload = json!({
        "repo_id": request.repo_id,
        "run_id": request.run_id,
        "tracker": tracker,
        "tracker_identifier": tracker_identifier,
        "base_commit": request.base_commit,
        "head_commit": request.head_commit,
        "changed_files": request.changed_files,
        "validation_results": request.validation_results,
        "run_evidence": request.run_evidence,
        "memory_update_proposal": request.memory_update_proposal,
        "followup_issues": request.followup_issues,
        "idempotency_key": idempotency_key,
    });

    let now = chrono::Utc::now().to_rfc3339();

    metadata
        .insert_run_writeback(
            &request.repo_id,
            &request.run_id,
            tracker,
            tracker_identifier,
            idempotency_key,
            &payload.to_string(),
            &now,
        )
        .map_err(|e| MfsError::Internal {
            message: e.to_string(),
        })?;

    Ok(Json(json!({
        "status": "ok",
        "data": {
            "repo_id": request.repo_id,
            "run_id": request.run_id,
            "writeback_id": format!("wb_{}:{}", request.repo_id, request.run_id),
            "persisted_memory_version": now,
        },
        "hint": null,
        "version_hash": now,
    })))
}
