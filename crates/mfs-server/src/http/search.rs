use super::*;

pub(super) async fn search(
    State(state): State<Arc<AppState>>,
    Query(query): Query<SearchQuery>,
) -> HandlerResult<Json<SearchResult>> {
    query.validate_query_length(1024)?;
    let resolved_target =
        resolve_alias_target(&state.metadata, query.target.as_deref()).or(query.target.clone());
    let fs = resolved_fs(&state.config, resolved_target.as_deref()).await?;
    let engine = retrieval_engine_for_fs(state.as_ref(), &fs).await?;
    let target = resolved_target;
    let session_ctx = query.session_context.clone();
    let guard = engine.lock().await;
    let result = guard
        .search(&query.query, target.as_deref(), session_ctx.as_deref())
        .await
        .map_err(AppError::from_error)?;
    drop(guard);
    append_audit(
        &state,
        "search",
        query.target.as_deref(),
        Some("{\"result\":\"ok\"}"),
    );
    Ok(Json(result))
}

pub(super) async fn find(
    State(state): State<Arc<AppState>>,
    Query(query): Query<SearchQuery>,
) -> HandlerResult<Json<SearchResult>> {
    query.validate_query_length(1024)?;
    let resolved_target =
        resolve_alias_target(&state.metadata, query.target.as_deref()).or(query.target.clone());
    let fs = resolved_fs(&state.config, resolved_target.as_deref()).await?;
    let engine = retrieval_engine_for_fs(state.as_ref(), &fs).await?;
    let target = resolved_target;
    let guard = engine.lock().await;
    let result = guard
        .find(&query.query, target.as_deref())
        .await
        .map_err(AppError::from_error)?;
    drop(guard);
    append_audit(
        &state,
        "find",
        query.target.as_deref(),
        Some("{\"result\":\"ok\"}"),
    );
    Ok(Json(result))
}

pub(super) async fn grep(
    State(state): State<Arc<AppState>>,
    Query(query): Query<SearchQuery>,
) -> HandlerResult<Json<SearchResult>> {
    query.validate_query_length(1024)?;
    let resolved_target =
        resolve_alias_target(&state.metadata, query.target.as_deref()).or(query.target.clone());
    let fs = resolved_fs(&state.config, resolved_target.as_deref()).await?;
    let engine = retrieval_engine_for_fs(state.as_ref(), &fs).await?;
    let target = resolved_target;
    let limit = query.limit;
    let guard = engine.lock().await;
    let result = guard
        .grep(&query.query, target.as_deref(), limit)
        .await
        .map_err(AppError::from_error)?;
    drop(guard);
    append_audit(
        &state,
        "grep",
        query.target.as_deref(),
        Some("{\"result\":\"ok\"}"),
    );
    Ok(Json(result))
}

pub(super) async fn rebuild(
    State(state): State<Arc<AppState>>,
) -> HandlerResult<Json<serde_json::Value>> {
    let fs = resolved_fs(&state.config, Some(&state.config.target_uri)).await?;
    let metadata = state.metadata.clone();
    let identity = configured_identity(&state.config);
    let rebuild = rebuild_metadata_entries(
        &metadata,
        &identity,
        fs.projection_root(),
        fs.projection_uri(),
    )?;
    invalidate_retrieval_cache(&state).await;
    append_audit(
        &state,
        "rebuild",
        Some(fs.projection_uri()),
        Some("{\"result\":\"ok\"}"),
    );
    Ok(Json(serde_json::json!({
        "indexed_paths": rebuild.indexed_paths,
        "projection_uri": rebuild.projection_uri,
    })))
}

pub(super) async fn refresh(
    State(state): State<Arc<AppState>>,
) -> HandlerResult<Json<serde_json::Value>> {
    let identity = configured_identity(&state.config);
    let refresh = refresh_projection(
        &state.metadata,
        &state.config.workspace_root,
        &identity,
        &state.config.source_kind,
        state
            .config
            .source_path
            .to_str()
            .expect("source path utf-8"),
        &state.config.target_uri,
    )
    .await?;
    invalidate_retrieval_cache(&state).await;
    append_audit(
        &state,
        "refresh",
        Some(&refresh.projection_uri),
        Some("{\"result\":\"ok\"}"),
    );
    Ok(Json(serde_json::json!({
        "snapshot_id": refresh.snapshot_id,
        "indexed_paths": refresh.indexed_paths,
        "projection_uri": refresh.projection_uri,
    })))
}
