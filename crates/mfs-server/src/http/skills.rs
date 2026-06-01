use super::*;

#[derive(Debug, Deserialize)]
pub(super) struct AddSkillRequest {
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct ListSkillsQuery {
    pub limit: Option<usize>,
}

pub(super) async fn add_skill(
    State(state): State<Arc<AppState>>,
    Json(request): Json<AddSkillRequest>,
) -> HandlerResult<Json<serde_json::Value>> {
    let identity = configured_identity(&state.config);
    // Validate path is within workspace root (path traversal protection)
    let validated_path =
        validate_path_within_workspace(&state.config.workspace_root, &request.path)?;
    let result = ingest_skill(
        &state.metadata,
        &state.config.workspace_root,
        &identity,
        &validated_path,
    )
    .await?;
    invalidate_retrieval_cache(&state).await;
    append_audit(
        &state,
        "skill.add",
        Some(&result.skill_uri),
        Some("{\"result\":\"ok\"}"),
    );
    Ok(Json(serde_json::json!({
        "skill_name": result.skill_name,
        "skill_uri": result.skill_uri,
        "indexed_documents": result.indexed_documents,
        "mode": result.mode,
    })))
}

pub(super) async fn list_skills_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ListSkillsQuery>,
) -> HandlerResult<Json<serde_json::Value>> {
    let identity = configured_identity(&state.config);
    let skills = list_skills(&state.config.workspace_root, &identity).await?;
    let limit = query.limit.unwrap_or(100);
    let total_count = skills.len();
    let items: Vec<_> = skills.into_iter().take(limit).collect();
    Ok(Json(serde_json::json!({
        "items": items.clone(),
        "skills": items,
        "next_cursor": null,
        "total_count": total_count,
        "count": total_count,
        "limit": limit,
    })))
}
