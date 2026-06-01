use super::*;

pub(super) async fn ls(
    State(state): State<Arc<AppState>>,
    Query(query): Query<UriQuery>,
) -> HandlerResult<Json<Vec<mfs_workspace::DirEntry>>> {
    let fs = resolved_fs(&state.config, Some(&query.uri)).await?;
    let listing = fs.ls(&query.uri).await?;
    append_audit(&state, "ls", Some(&query.uri), Some("{\"result\":\"ok\"}"));
    Ok(Json(listing))
}

pub(super) async fn tree(
    State(state): State<Arc<AppState>>,
    Query(query): Query<TreeQuery>,
) -> HandlerResult<String> {
    let fs = resolved_fs(&state.config, Some(&query.uri)).await?;
    let node = fs.tree(&query.uri, query.depth.unwrap_or(3)).await?;
    append_audit(
        &state,
        "tree",
        Some(&query.uri),
        Some("{\"result\":\"ok\"}"),
    );
    Ok(render_tree(&node, 0))
}

pub(super) async fn stat(
    State(state): State<Arc<AppState>>,
    Query(query): Query<UriQuery>,
) -> HandlerResult<Json<serde_json::Value>> {
    let fs = resolved_fs(&state.config, Some(&query.uri)).await?;
    let stat = fs.stat(&query.uri).await?;
    append_audit(
        &state,
        "stat",
        Some(&query.uri),
        Some("{\"result\":\"ok\"}"),
    );
    Ok(Json(serde_json::json!({
        "path": stat.path,
        "is_dir": stat.is_dir,
        "size_bytes": stat.size_bytes,
    })))
}

pub(super) async fn read(
    State(state): State<Arc<AppState>>,
    Query(query): Query<UriQuery>,
) -> HandlerResult<String> {
    let fs = resolved_fs(&state.config, Some(&query.uri)).await?;
    let result = fs.read(&query.uri).await?;
    append_audit(
        &state,
        "read",
        Some(&query.uri),
        Some("{\"result\":\"ok\"}"),
    );
    Ok(result)
}

pub(super) async fn glob(
    State(state): State<Arc<AppState>>,
    Query(query): Query<GlobQuery>,
) -> HandlerResult<Json<Vec<String>>> {
    let fs = resolved_fs(&state.config, Some(&query.uri)).await?;
    let matches = fs.glob(&query.uri, &query.pattern).await?;
    append_audit(
        &state,
        "glob",
        Some(&query.uri),
        Some("{\"result\":\"ok\"}"),
    );
    Ok(Json(matches))
}

pub(super) async fn mkdir(
    State(state): State<Arc<AppState>>,
    Json(request): Json<UriQuery>,
) -> HandlerResult<Json<serde_json::Value>> {
    let identity = configured_identity(&state.config);
    let result = mkdir_owned_path(
        &state.metadata,
        &state.config.workspace_root,
        &identity,
        &request.uri,
    )
    .await?;
    invalidate_retrieval_cache(&state).await;
    append_audit(
        &state,
        "mkdir",
        Some(&request.uri),
        Some("{\"result\":\"ok\"}"),
    );
    Ok(Json(serde_json::json!({
        "uri": result.primary_uri,
        "indexed_paths": result.indexed_paths,
        "scopes_reindexed": result.scopes_reindexed,
    })))
}

pub(super) async fn write(
    State(state): State<Arc<AppState>>,
    Json(request): Json<WriteRequest>,
) -> HandlerResult<Json<serde_json::Value>> {
    let identity = configured_identity(&state.config);
    let result = write_owned_path(
        &state.metadata,
        &state.config.workspace_root,
        &identity,
        &request.uri,
        &request.content,
    )
    .await?;
    invalidate_retrieval_cache(&state).await;
    append_audit(
        &state,
        "write",
        Some(&request.uri),
        Some("{\"result\":\"ok\"}"),
    );
    Ok(Json(serde_json::json!({
        "uri": result.primary_uri,
        "indexed_paths": result.indexed_paths,
        "scopes_reindexed": result.scopes_reindexed,
    })))
}

pub(super) async fn mv(
    State(state): State<Arc<AppState>>,
    Json(request): Json<MoveRequest>,
) -> HandlerResult<Json<serde_json::Value>> {
    let identity = configured_identity(&state.config);
    let result = move_owned_path(
        &state.metadata,
        &state.config.workspace_root,
        &identity,
        &request.from_uri,
        &request.to_uri,
    )
    .await?;
    invalidate_retrieval_cache(&state).await;
    append_audit(
        &state,
        "mv",
        Some(&request.to_uri),
        Some("{\"result\":\"ok\"}"),
    );
    Ok(Json(serde_json::json!({
        "uri": result.primary_uri,
        "indexed_paths": result.indexed_paths,
        "scopes_reindexed": result.scopes_reindexed,
    })))
}

pub(super) async fn rm(
    State(state): State<Arc<AppState>>,
    Query(query): Query<UriQuery>,
) -> HandlerResult<Json<serde_json::Value>> {
    let identity = configured_identity(&state.config);
    let result = remove_owned_path(
        &state.metadata,
        &state.config.workspace_root,
        &identity,
        &query.uri,
    )
    .await?;
    invalidate_retrieval_cache(&state).await;
    append_audit(&state, "rm", Some(&query.uri), Some("{\"result\":\"ok\"}"));
    Ok(Json(serde_json::json!({
        "uri": result.primary_uri,
        "indexed_paths": result.indexed_paths,
        "scopes_reindexed": result.scopes_reindexed,
    })))
}

pub(super) async fn abstract_text(
    State(state): State<Arc<AppState>>,
    Query(query): Query<UriQuery>,
) -> HandlerResult<String> {
    let fs = resolved_fs(&state.config, Some(&query.uri)).await?;
    let result = fs.abstract_text(&query.uri).await?;
    append_audit(
        &state,
        "abstract",
        Some(&query.uri),
        Some("{\"result\":\"ok\"}"),
    );
    Ok(result)
}

pub(super) async fn overview_text(
    State(state): State<Arc<AppState>>,
    Query(query): Query<UriQuery>,
) -> HandlerResult<String> {
    let fs = resolved_fs(&state.config, Some(&query.uri)).await?;
    let result = fs.overview_text(&query.uri).await?;
    append_audit(
        &state,
        "overview",
        Some(&query.uri),
        Some("{\"result\":\"ok\"}"),
    );
    Ok(result)
}

// Request DTOs used only by workspace handlers
#[derive(Debug, Deserialize)]
pub(super) struct WriteRequest {
    pub uri: String,
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct MoveRequest {
    pub from_uri: String,
    pub to_uri: String,
}

pub(super) async fn system_status_handler(
    State(state): State<Arc<AppState>>,
) -> HandlerResult<Json<serde_json::Value>> {
    let identity = configured_identity(&state.config);
    let mut status = serde_json::to_value(
        system_status(&state.metadata, &state.config.workspace_root, &identity).await?,
    )
    .map_err(|e| {
        AppError(mfs_types::MfsError::Internal {
            message: e.to_string(),
        })
    })?;
    status["runtime"] = serde_json::json!({
        "status": "ok",
        "retrieval_cache": {
            "entries": state.retrieval_cache_entry_count(),
            "builds": state.retrieval_cache_build_count(),
            "hits": state.retrieval_cache_hit_count(),
            "invalidations": state.retrieval_cache_invalidation_count(),
        }
    });
    Ok(Json(status))
}

pub(super) async fn observer_status_handler(
    State(state): State<Arc<AppState>>,
) -> HandlerResult<Json<serde_json::Value>> {
    let mut status =
        serde_json::to_value(observer_status(&state.config.workspace_root)?).map_err(|e| {
            AppError(mfs_types::MfsError::Internal {
                message: e.to_string(),
            })
        })?;
    status["runtime"]["retrieval_cache"] = serde_json::json!({
        "entries": state.retrieval_cache_entry_count(),
        "builds": state.retrieval_cache_build_count(),
        "hits": state.retrieval_cache_hit_count(),
        "invalidations": state.retrieval_cache_invalidation_count(),
    });
    Ok(Json(status))
}
