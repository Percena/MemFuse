//! HTTP handlers for Active Overlay API (issue/PR-level plan declarations).
//!
//! These handlers are thin: they deserialize requests, validate HTTP-level
//! constraints (overlay_type, conflict_policy constants), call OverlayService
//! for business logic, and format JSON responses.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Query, State};
use serde::Deserialize;
use serde_json::{Value, json};

use mfs_planning::{OverlayService, ProposeInput, RecordConflictInput, parse_status_filter};
use mfs_types::MfsError;

use super::{AppState, HandlerResult};

// ─── Request structs ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(super) struct ProposeOverlayRequest {
    pub repo_id: String,
    pub tracker: Option<String>,
    pub tracker_content_id: String,
    pub tracker_project_item_id: Option<String>,
    pub tracker_identifier: String,
    pub issue_number: Option<i64>,
    pub overlay_type: String,
    pub affected_nodes: Option<Vec<String>>,
    pub affected_edges: Option<Vec<String>>,
    /// Canonical Canvas refs for nodes (canvas://...). If provided, validated.
    /// If not provided, auto-generated from affected_nodes IDs.
    pub affected_node_refs: Option<Vec<String>>,
    /// Canonical Canvas refs for edges (canvas://...). If provided, validated.
    /// If not provided, auto-generated from affected_edges IDs.
    pub affected_edge_refs: Option<Vec<String>>,
    pub content_json: Value,
    pub author: String,
    pub branch: Option<String>,
    pub pr_url: Option<String>,
    pub agent_session_id: Option<String>,
    pub idempotency_key: Option<String>,
    pub conflict_policy: Option<String>,
}

const VALID_CONFLICT_POLICIES: &[&str] = &["fail_on_conflict", "allow_overlap"];

#[derive(Debug, Deserialize)]
pub(super) struct AcceptOverlayRequest {
    pub overlay_id: String,
    pub acceptor: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct MarkImplementedRequest {
    pub overlay_id: String,
    pub agent_session_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct AbandonOverlayRequest {
    pub overlay_id: String,
    pub reason: Option<String>,
    pub abandoner: Option<String>,
    pub actor: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ReportConflictRequest {
    pub repo_id: String,
    pub overlay_id_1: String,
    pub overlay_id_2: String,
    pub conflict_description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct RecordConflictRequest {
    pub repo_id: String,
    pub run_id: Option<String>,
    pub tracker: Option<String>,
    pub tracker_identifier: Option<String>,
    pub conflict_summary: String,
    pub severity: Option<String>,
    pub evidence: Option<Value>,
    pub author: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ConsolidateRequest {
    pub repo_id: String,
    pub merge_commit: String,
    pub overlay_ids: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ListOverlaysQuery {
    pub repo_id: Option<String>,
    pub status: Option<String>,
}

// ─── Valid overlay_type values ────────────────────────────────────────

const VALID_OVERLAY_TYPES: &[&str] = &[
    "planned_change",
    "planned_contract",
    "conflict_declaration",
    "planned_test",
    "planned_config",
];

// ─── Handlers ─────────────────────────────────────────────────────────

pub(super) async fn propose_overlay(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ProposeOverlayRequest>,
) -> HandlerResult<Json<Value>> {
    // Validate overlay_type (HTTP-level validation)
    if !VALID_OVERLAY_TYPES.contains(&request.overlay_type.as_str()) {
        return Err(MfsError::InvalidArgument {
            field: "overlay_type".into(),
            reason: format!(
                "invalid overlay_type '{}'; valid values: {}",
                request.overlay_type,
                VALID_OVERLAY_TYPES.join(", ")
            ),
        }
        .into());
    }

    // Validate conflict_policy (HTTP-level validation)
    if let Some(policy) = &request.conflict_policy {
        if !VALID_CONFLICT_POLICIES.contains(&policy.as_str()) {
            return Err(MfsError::InvalidArgument {
                field: "conflict_policy".into(),
                reason: format!(
                    "invalid conflict_policy '{}'; valid values: {}",
                    policy,
                    VALID_CONFLICT_POLICIES.join(", ")
                ),
            }
            .into());
        }
    }

    // Validate acceptor (HTTP-level validation for accept)
    // (No acceptor field in propose, so skip)

    let service = OverlayService::new(state.metadata.clone());
    let input = ProposeInput {
        repo_id: request.repo_id,
        overlay_type: request.overlay_type,
        tracker: request.tracker,
        tracker_content_id: request.tracker_content_id,
        tracker_project_item_id: request.tracker_project_item_id,
        tracker_identifier: request.tracker_identifier,
        issue_number: request.issue_number,
        affected_nodes: request.affected_nodes.unwrap_or_default(),
        affected_edges: request.affected_edges.unwrap_or_default(),
        affected_node_refs: request.affected_node_refs,
        affected_edge_refs: request.affected_edge_refs,
        content_json: request.content_json,
        author: request.author,
        branch: request.branch,
        pr_url: request.pr_url,
        agent_session_id: request.agent_session_id,
        idempotency_key: request.idempotency_key,
        conflict_policy: request.conflict_policy,
    };

    let output = service.propose(input)?;

    // Format response based on output status
    Ok(Json(json!({
        "status": output.status,
        "overlay_id": output.data.get("overlay_id").unwrap_or(&Value::Null),
        "overlay_type": output.overlay_type,
        "hint": output.conflict_hint,
        "version_hash": output.version_hash,
        "data": output.data,
    })))
}

pub(super) async fn accept_overlay(
    State(state): State<Arc<AppState>>,
    Json(request): Json<AcceptOverlayRequest>,
) -> HandlerResult<Json<Value>> {
    // Only human can accept overlay (HTTP-level validation)
    if request.acceptor != "human" {
        return Err(MfsError::PermissionDenied {
            reason: "overlay.accept requires acceptor='human'; agents cannot accept overlays"
                .into(),
        }
        .into());
    }

    let service = OverlayService::new(state.metadata.clone());
    let _overlay = service.accept(&request.overlay_id, &request.acceptor)?;

    let now = chrono::Utc::now().to_rfc3339();
    let data = json!({
        "overlay_id": request.overlay_id,
        "status": "accepted",
    });

    Ok(Json(json!({
        "overlay_id": data["overlay_id"],
        "new_status": data["status"],
        "hint": null,
        "status": "ok",
        "version_hash": now,
        "data": data,
    })))
}

pub(super) async fn mark_implemented(
    State(state): State<Arc<AppState>>,
    Json(request): Json<MarkImplementedRequest>,
) -> HandlerResult<Json<Value>> {
    let service = OverlayService::new(state.metadata.clone());
    let _overlay =
        service.mark_implemented(&request.overlay_id, request.agent_session_id.as_deref())?;

    let now = chrono::Utc::now().to_rfc3339();
    let data = json!({
        "overlay_id": request.overlay_id,
        "status": "implemented",
    });

    Ok(Json(json!({
        "overlay_id": data["overlay_id"],
        "new_status": data["status"],
        "hint": null,
        "status": "ok",
        "version_hash": now,
        "data": data,
    })))
}

pub(super) async fn abandon_overlay(
    State(state): State<Arc<AppState>>,
    Json(request): Json<AbandonOverlayRequest>,
) -> HandlerResult<Json<Value>> {
    let service = OverlayService::new(state.metadata.clone());
    let actor = request
        .abandoner
        .as_deref()
        .or(request.actor.as_deref())
        .unwrap_or("agent");
    let _overlay = service.abandon(&request.overlay_id, request.reason.as_deref(), Some(actor))?;

    let now = chrono::Utc::now().to_rfc3339();
    let data = json!({
        "overlay_id": request.overlay_id,
        "status": "abandoned",
        "triggered_by": actor,
    });

    Ok(Json(json!({
        "status": "ok",
        "overlay_id": data["overlay_id"],
        "new_status": data["status"],
        "triggered_by": data["triggered_by"],
        "hint": null,
        "version_hash": now,
        "data": data,
    })))
}

pub(super) async fn report_conflict(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ReportConflictRequest>,
) -> HandlerResult<Json<Value>> {
    let service = OverlayService::new(state.metadata.clone());
    let analysis = service.report_conflict(
        &request.repo_id,
        &request.overlay_id_1,
        &request.overlay_id_2,
        request.conflict_description.as_deref(),
    )?;

    let now = chrono::Utc::now().to_rfc3339();
    let data = json!({
        "conflict_id": analysis.conflict_id,
        "has_conflict": analysis.has_conflict,
        "has_overlap": analysis.has_conflict,
        "requires_human_review": analysis.requires_human_review,
        "overlap_nodes": analysis.overlap_nodes,
        "overlap_edges": analysis.overlap_edges,
        "description": analysis.description,
    });

    Ok(Json(json!({
        "status": "ok",
        "conflict_id": data["conflict_id"],
        "has_conflict": data["has_conflict"],
        "requires_human_review": data["requires_human_review"],
        "overlap_nodes": data["overlap_nodes"],
        "overlap_edges": data["overlap_edges"],
        "hint": null,
        "version_hash": now,
        "data": data,
    })))
}

pub(super) async fn record_conflict(
    State(state): State<Arc<AppState>>,
    Json(request): Json<RecordConflictRequest>,
) -> HandlerResult<Json<Value>> {
    let service = OverlayService::new(state.metadata.clone());
    let input = RecordConflictInput {
        repo_id: request.repo_id,
        run_id: request.run_id,
        tracker: request.tracker,
        tracker_identifier: request.tracker_identifier,
        conflict_summary: request.conflict_summary,
        severity: request.severity,
        evidence: request.evidence,
        author: request.author,
    };

    let stored = service.record_conflict(input)?;

    // Extract fields from the stored overlay for response formatting
    let content_json: Value = serde_json::from_str(&stored.content_json).unwrap_or(json!({}));
    let tracker_identifier = stored.tracker_identifier.clone();
    let severity = content_json
        .get("severity")
        .and_then(Value::as_str)
        .unwrap_or("blocking")
        .to_owned();

    let now = stored.updated_at.clone();
    let data = json!({
        "conflict_id": stored.id,
        "overlay_id": stored.id,
        "run_id": content_json.get("run_id"),
        "tracker_identifier": tracker_identifier,
        "severity": severity,
        "status": stored.status,
    });

    Ok(Json(json!({
        "status": "ok",
        "conflict_id": data["conflict_id"],
        "overlay_id": data["overlay_id"],
        "hint": null,
        "version_hash": now,
        "data": data,
    })))
}

pub(super) async fn consolidate(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ConsolidateRequest>,
) -> HandlerResult<Json<Value>> {
    let service = OverlayService::new(state.metadata.clone());
    let output =
        service.consolidate(&request.repo_id, request.overlay_ids, &request.merge_commit)?;

    let now = chrono::Utc::now().to_rfc3339();
    let data = json!({
        "merged_count": output.merged_count,
        "snapshot_id": output.snapshot_id,
    });

    Ok(Json(json!({
        "status": "ok",
        "merged_count": data["merged_count"],
        "snapshot_id": data["snapshot_id"],
        "hint": null,
        "version_hash": now,
        "data": data,
    })))
}

pub(super) async fn list_overlays(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ListOverlaysQuery>,
) -> HandlerResult<Json<Value>> {
    let repo_id = query
        .repo_id
        .unwrap_or_else(|| state.config.account_id.clone());

    let statuses = parse_status_filter(query.status.as_deref());
    let service = OverlayService::new(state.metadata.clone());

    let overlays = match statuses.as_slice() {
        [] => service.list(&repo_id, None)?,
        [status] => service.list(&repo_id, Some(status.as_str()))?,
        _ => {
            let all = service.list(&repo_id, None)?;
            all.into_iter()
                .filter(|overlay| statuses.contains(&overlay.status))
                .collect()
        }
    };

    Ok(Json(json!({
        "status": "ok",
        "overlays": overlays,
        "items": overlays,
        "total_count": overlays.len(),
        "hint": null,
        "version_hash": null,
    })))
}
