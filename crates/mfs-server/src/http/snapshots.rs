use super::*;

pub(super) async fn snapshots(
    State(state): State<Arc<AppState>>,
    Query(query): Query<AuditQuery>,
) -> HandlerResult<Json<serde_json::Value>> {
    let metadata = state.metadata.clone();
    let limit = query.limit.unwrap_or(50);
    let snapshots = metadata
        .list_snapshots(
            &state.config.account_id,
            &state.config.user_id,
            Some(&resource_projection_view_id(&state.config)),
            limit,
        )
        .map_err(AppError::from_error)?;
    let total_count = snapshots.len();
    Ok(Json(serde_json::json!({
        "items": snapshots.clone(),
        "snapshots": snapshots,
        "next_cursor": null,
        "total_count": total_count,
        "limit": limit,
    })))
}

pub(super) async fn audit(
    State(state): State<Arc<AppState>>,
    Query(query): Query<AuditQuery>,
) -> HandlerResult<Json<serde_json::Value>> {
    let metadata = state.metadata.clone();
    let limit = query.limit.unwrap_or(50);
    let audit = metadata
        .list_audit(&state.config.account_id, &state.config.user_id, limit)
        .map_err(AppError::from_error)?;
    let total_count = audit.len();
    Ok(Json(serde_json::json!({
        "items": audit.clone(),
        "audit": audit.clone(),
        "entries": audit,
        "next_cursor": null,
        "total_count": total_count,
        "limit": limit,
    })))
}
