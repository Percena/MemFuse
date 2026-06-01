use super::*;

#[derive(Debug, Deserialize)]
pub(super) struct ResourceWatchRequest {
    pub interval_seconds: u32,
}

#[derive(Debug, Deserialize)]
pub(super) struct ResourceWatchLoopRequest {
    pub iterations: usize,
    pub sleep_ms: u64,
}

#[derive(Debug, Deserialize)]
pub(super) struct WatchServiceRequest {
    pub poll_ms: u64,
}

#[derive(Debug, Deserialize)]
pub(super) struct ListWatchesQuery {
    pub limit: Option<usize>,
}

pub(super) async fn register_watch(
    State(state): State<Arc<AppState>>,
    Path(resource_id): Path<String>,
    Json(request): Json<ResourceWatchRequest>,
) -> HandlerResult<Json<serde_json::Value>> {
    let identity = configured_identity(&state.config);
    let watch = register_resource_watch(
        &state.metadata,
        &identity,
        &resource_id,
        request.interval_seconds,
    )?;
    append_audit(
        &state,
        "resource.watch.register",
        Some(&resource_id),
        Some("{\"result\":\"ok\"}"),
    );
    Ok(Json(serde_json::to_value(watch).map_err(|e| {
        AppError(mfs_types::MfsError::Internal {
            message: e.to_string(),
        })
    })?))
}

pub(super) async fn run_watch(
    State(state): State<Arc<AppState>>,
    Path(resource_id): Path<String>,
) -> HandlerResult<Json<serde_json::Value>> {
    let identity = configured_identity(&state.config);
    let result = run_resource_watch(
        &state.metadata,
        &state.config.workspace_root,
        &identity,
        &resource_id,
    )
    .await?;
    invalidate_retrieval_cache(&state).await;
    append_audit(
        &state,
        "resource.watch.run",
        Some(&result.root_uri),
        Some("{\"result\":\"ok\"}"),
    );
    Ok(Json(serde_json::json!({
        "resource_id": result.resource_id,
        "refreshed": result.refreshed,
        "root_uri": result.root_uri,
        "snapshot_id": result.snapshot_id,
    })))
}

pub(super) async fn disable_watch(
    State(state): State<Arc<AppState>>,
    Path(resource_id): Path<String>,
) -> HandlerResult<Json<serde_json::Value>> {
    let watch = disable_resource_watch(
        &state.metadata,
        &state.config.account_id,
        &state.config.user_id,
        &resource_id,
    )?;
    append_audit(
        &state,
        "resource.watch.disable",
        Some(&resource_id),
        Some("{\"result\":\"ok\"}"),
    );
    Ok(Json(serde_json::to_value(watch).map_err(|e| {
        AppError(mfs_types::MfsError::Internal {
            message: e.to_string(),
        })
    })?))
}

pub(super) async fn run_due_watches(
    State(state): State<Arc<AppState>>,
) -> HandlerResult<Json<serde_json::Value>> {
    let identity = configured_identity(&state.config);
    let runs = run_due_resource_watches(
        &state.metadata,
        &state.config.workspace_root,
        &identity,
        100,
    )
    .await?;
    if !runs.is_empty() {
        invalidate_retrieval_cache(&state).await;
    }
    Ok(Json(serde_json::json!({ "runs": runs })))
}

pub(super) async fn run_watch_loop_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ResourceWatchLoopRequest>,
) -> HandlerResult<Json<serde_json::Value>> {
    let identity = configured_identity(&state.config);
    let result = run_resource_watch_loop(
        &state.metadata,
        &state.config.workspace_root,
        &identity,
        request.iterations,
        std::time::Duration::from_millis(request.sleep_ms),
        100,
    )
    .await?;
    if result.total_runs > 0 {
        invalidate_retrieval_cache(&state).await;
    }
    Ok(Json(serde_json::to_value(result).map_err(|e| {
        AppError(mfs_types::MfsError::Internal {
            message: e.to_string(),
        })
    })?))
}

pub(super) async fn list_watches(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ListWatchesQuery>,
) -> HandlerResult<Json<serde_json::Value>> {
    let limit = query.limit.unwrap_or(100);
    let watches = list_resource_watch_statuses(
        &state.metadata,
        &state.config.account_id,
        &state.config.user_id,
        limit,
    )?;
    let total_count = watches.len();
    Ok(Json(serde_json::json!({
        "items": watches.clone(),
        "watches": watches,
        "next_cursor": null,
        "total_count": total_count,
        "count": total_count,
        "limit": limit,
    })))
}

pub(super) async fn start_watch_service(
    State(state): State<Arc<AppState>>,
    Json(request): Json<WatchServiceRequest>,
) -> HandlerResult<Json<serde_json::Value>> {
    {
        let guard = state.watch_service.lock().unwrap();
        if guard.status.lock().unwrap().running {
            return Ok(Json(
                serde_json::to_value(guard.status.lock().unwrap().clone()).map_err(|e| {
                    AppError(mfs_types::MfsError::Internal {
                        message: e.to_string(),
                    })
                })?,
            ));
        }
    }

    let stop_token = CancellationToken::new();
    let status = {
        let guard = state.watch_service.lock().unwrap();
        Arc::clone(&guard.status)
    };
    {
        let mut current = status.lock().unwrap();
        *current = WatchServiceStatus {
            running: true,
            poll_ms: request.poll_ms,
            started_at_ms: Some(now_millis()),
            stopped_at_ms: None,
            last_tick_at_ms: None,
            total_ticks: 0,
            total_runs: 0,
            last_run_count: 0,
        };
    }

    let state_for_task = Arc::clone(&state);
    let stop_for_task = stop_token.clone();
    let shutdown_for_task = state.shutdown_token.child_token();
    let status_for_task = Arc::clone(&status);
    let handle = tokio::spawn(async move {
        let identity = configured_identity(&state_for_task.config);
        loop {
            tokio::select! {
                () = stop_for_task.cancelled() => {
                    let mut status = status_for_task.lock().unwrap();
                    status.running = false;
                    status.stopped_at_ms = Some(now_millis());
                    break;
                }
                () = shutdown_for_task.cancelled() => {
                    let mut status = status_for_task.lock().unwrap();
                    status.running = false;
                    status.stopped_at_ms = Some(now_millis());
                    tracing::info!("Watch service cancelled on server shutdown");
                    break;
                }
                () = async {
                    let runs = run_due_resource_watches(
                        &state_for_task.metadata,
                        &state_for_task.config.workspace_root,
                        &identity,
                        100,
                    )
                    .await
                    .unwrap_or_default();
                    if !runs.is_empty() {
                        invalidate_retrieval_cache(&state_for_task).await;
                    }
                    {
                        let mut status = status_for_task.lock().unwrap();
                        status.last_tick_at_ms = Some(now_millis());
                        status.total_ticks += 1;
                        status.last_run_count = runs.len() as u64;
                        status.total_runs += runs.len() as u64;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(request.poll_ms)).await;
                } => {}
            }
        }
    });

    {
        let mut guard = state.watch_service.lock().unwrap();
        guard.stop = Some(stop_token);
        guard.handle = Some(handle);
    }

    Ok(Json(
        serde_json::to_value(status.lock().unwrap().clone()).map_err(|e| {
            AppError(mfs_types::MfsError::Internal {
                message: e.to_string(),
            })
        })?,
    ))
}

pub(super) async fn watch_service_status(
    State(state): State<Arc<AppState>>,
) -> HandlerResult<Json<serde_json::Value>> {
    let status = {
        let guard = state.watch_service.lock().unwrap();
        guard.status.lock().unwrap().clone()
    };
    Ok(Json(serde_json::to_value(status).map_err(|e| {
        AppError(mfs_types::MfsError::Internal {
            message: e.to_string(),
        })
    })?))
}

pub(super) async fn stop_watch_service(
    State(state): State<Arc<AppState>>,
) -> HandlerResult<Json<serde_json::Value>> {
    let handle = {
        let mut guard = state.watch_service.lock().unwrap();
        if let Some(stop) = guard.stop.take() {
            stop.cancel();
        }
        guard.handle.take()
    };
    if let Some(handle) = handle {
        let _ = handle.await;
    }
    let status = {
        let guard = state.watch_service.lock().unwrap();
        guard.status.lock().unwrap().clone()
    };
    Ok(Json(serde_json::to_value(status).map_err(|e| {
        AppError(mfs_types::MfsError::Internal {
            message: e.to_string(),
        })
    })?))
}
