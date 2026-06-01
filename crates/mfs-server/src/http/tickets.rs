use super::{AppState, HandlerResult};
use axum::Json;
use axum::extract::{Query, State};
use mfs_types::MfsError;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

#[derive(Debug, Deserialize)]
pub(super) struct TicketHistoryQuery {
    pub repo_id: String,
    pub tracker_identifier: Option<String>,
}

pub(super) async fn ticket_history(
    State(state): State<Arc<AppState>>,
    Query(query): Query<TicketHistoryQuery>,
) -> HandlerResult<Json<Value>> {
    let metadata = state.metadata.clone();

    // Ticket history is derived from overlays + writebacks associated with
    // the tracker_identifier. Overlays carry tracker_identifier and lifecycle
    // timestamps; writebacks carry run evidence and validation results.
    let overlays = metadata
        .list_overlays_by_tracker(&query.repo_id, query.tracker_identifier.as_deref())
        .map_err(|e| MfsError::Internal {
            message: e.to_string(),
        })?;

    let writebacks = metadata
        .list_run_writebacks_by_tracker(&query.repo_id, query.tracker_identifier.as_deref())
        .map_err(|e| MfsError::Internal {
            message: e.to_string(),
        })?;

    // Merge overlays and writebacks into a unified ticket trace view.
    // Each overlay is one "ticket execution attempt"; writebacks enrich
    // the overlay with run evidence.
    let items: Vec<Value> = overlays
        .into_iter()
        .map(|overlay| {
            // Find matching writeback by overlay tracker_identifier
            let matching_writebacks: Vec<&Value> = writebacks
                .iter()
                .filter(|wb| {
                    wb.get("tracker_identifier").and_then(|v| v.as_str())
                        == Some(&overlay.tracker_identifier)
                })
                .collect();

            let wb_evidence = matching_writebacks
                .first()
                .and_then(|wb| wb.get("run_evidence"))
                .cloned()
                .unwrap_or(json!(null));

            let wb_validation = matching_writebacks
                .first()
                .and_then(|wb| wb.get("validation_results"))
                .cloned()
                .unwrap_or(json!(null));

            json!({
                "overlay_id": overlay.id,
                "tracker_identifier": overlay.tracker_identifier,
                "tracker_content_id": overlay.tracker_content_id,
                "overlay_type": overlay.overlay_type,
                "status": overlay.status,
                "affected_nodes": overlay.affected_nodes,
                "affected_edges": overlay.affected_edges,
                "content_json": overlay.content_json,
                "created_at": overlay.created_at,
                "implemented_at": overlay.implemented_at,
                "run_evidence": wb_evidence,
                "validation_results": wb_validation,
            })
        })
        .collect();

    // Compute latest context version from the most recent overlay or writeback
    let latest_context_version = items
        .iter()
        .filter_map(|item| item.get("implemented_at").and_then(|v| v.as_str()))
        .max()
        .map(|s| s.to_string())
        .or_else(|| {
            items
                .iter()
                .filter_map(|item| item.get("created_at").and_then(|v| v.as_str()))
                .max()
                .map(|s| s.to_string())
        })
        .unwrap_or_default();

    Ok(Json(json!({
        "status": "ok",
        "data": {
            "items": items,
            "total_count": items.len(),
            "cursor": null,
            "latest_context_version": latest_context_version,
        },
        "hint": null,
        "version_hash": latest_context_version,
    })))
}
