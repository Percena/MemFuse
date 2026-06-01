use super::*;

#[derive(Debug, Deserialize)]
pub(super) struct RelationRequest {
    pub from_uri: String,
    pub to_uri: String,
    pub relation_type: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct RelationListQuery {
    pub uri: String,
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub(super) struct RelationDeleteQuery {
    pub from_uri: String,
    pub to_uri: String,
    pub relation_type: String,
}

pub(super) async fn link_relation(
    State(state): State<Arc<AppState>>,
    Json(request): Json<RelationRequest>,
) -> HandlerResult<Json<serde_json::Value>> {
    let metadata = state.metadata.clone();
    metadata
        .upsert_relation(&mfs_metadata::RelationRecord {
            account_id: &state.config.account_id,
            user_id: &state.config.user_id,
            agent_id: Some(&state.config.agent_id),
            from_uri: &request.from_uri,
            to_uri: &request.to_uri,
            relation_type: &request.relation_type,
        })
        .map_err(AppError::from_error)?;
    append_audit(
        &state,
        "relation.link",
        Some(&request.from_uri),
        Some("{\"result\":\"ok\"}"),
    );
    Ok(Json(serde_json::json!({ "ok": true })))
}

pub(super) async fn list_relations(
    State(state): State<Arc<AppState>>,
    Query(query): Query<RelationListQuery>,
) -> HandlerResult<Json<serde_json::Value>> {
    let metadata = state.metadata.clone();
    let limit = query.limit.unwrap_or(20);
    let rows = metadata
        .list_relations(
            &state.config.account_id,
            &state.config.user_id,
            &query.uri,
            limit,
        )
        .map_err(AppError::from_error)?
        .into_iter()
        .map(|relation| {
            let (direction, peer_uri) = if relation.from_uri == query.uri {
                ("outbound", relation.to_uri)
            } else {
                ("inbound", relation.from_uri)
            };
            serde_json::json!({
                "relation_type": relation.relation_type,
                "direction": direction,
                "peer_uri": peer_uri,
            })
        })
        .collect::<Vec<_>>();
    let total_count = rows.len();
    Ok(Json(serde_json::json!({
        "items": rows.clone(),
        "relations": rows,
        "next_cursor": null,
        "total_count": total_count,
        "count": total_count,
        "limit": limit,
    })))
}

pub(super) async fn unlink_relation(
    State(state): State<Arc<AppState>>,
    Query(query): Query<RelationDeleteQuery>,
) -> HandlerResult<Json<serde_json::Value>> {
    let metadata = state.metadata.clone();
    metadata
        .remove_relation(
            &state.config.account_id,
            &state.config.user_id,
            &query.from_uri,
            &query.to_uri,
            &query.relation_type,
        )
        .map_err(AppError::from_error)?;
    append_audit(
        &state,
        "relation.unlink",
        Some(&query.from_uri),
        Some("{\"result\":\"ok\"}"),
    );
    Ok(Json(serde_json::json!({ "ok": true })))
}
