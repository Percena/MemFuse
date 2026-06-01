use super::*;

pub(super) async fn task_status(
    State(state): State<Arc<AppState>>,
    Path(task_id): Path<String>,
) -> HandlerResult<Json<serde_json::Value>> {
    let task = state.session_engine.task_status(&task_id).await;
    if let Some(task) = task {
        return Ok(Json(serde_json::to_value(task).map_err(|e| {
            AppError(mfs_types::MfsError::Internal {
                message: e.to_string(),
            })
        })?));
    }

    let metadata = state.metadata.clone();
    Ok(Json(
        match metadata.get_task(&task_id).map_err(AppError::from_error)? {
            Some(task) => serde_json::json!({
                "task_key": task.task_key,
                "state": task.state,
                "summary": task.summary,
                "last_error": task.last_error,
                "attempt_count": task.attempt_count,
                "max_attempts": task.max_attempts,
                "retry_state": task.retry_state,
                "processing_mode": task.processing_mode,
            }),
            None => serde_json::json!({ "status": "not_found", "task_id": task_id }),
        },
    ))
}

pub(super) async fn wait_task(
    State(state): State<Arc<AppState>>,
    Path(task_id): Path<String>,
    Query(query): Query<WaitQuery>,
) -> HandlerResult<Json<serde_json::Value>> {
    let timeout = std::time::Duration::from_millis(query.timeout_ms.unwrap_or(5_000));
    let poll = std::time::Duration::from_millis(query.poll_ms.unwrap_or(50));
    let outcome = wait_for_task_completion(
        &state.metadata,
        &state.session_engine,
        &task_id,
        timeout,
        poll,
    )
    .await?;

    Ok(Json(match outcome {
        WaitTaskOutcome::Session(task) => serde_json::to_value(task).map_err(|e| {
            AppError(mfs_types::MfsError::Internal {
                message: e.to_string(),
            })
        })?,
        WaitTaskOutcome::Metadata(task) => serde_json::to_value(task).map_err(|e| {
            AppError(mfs_types::MfsError::Internal {
                message: e.to_string(),
            })
        })?,
        WaitTaskOutcome::Timeout { task_id } => {
            serde_json::json!({ "status": "timeout", "task_id": task_id })
        }
    }))
}

pub(super) async fn list_tasks(
    State(state): State<Arc<AppState>>,
    Query(query): Query<AuditQuery>,
) -> HandlerResult<Json<serde_json::Value>> {
    let limit = query.limit.unwrap_or(20);
    let mut tasks = Vec::new();

    for task in state.session_engine.list_tasks(limit).await? {
        tasks.push(serde_json::json!({
            "kind": "session",
            "task_id": task.task_id,
            "status": format!("{:?}", task.status),
            "archive_uri": task.archive_uri,
        }));
    }

    let metadata = state.metadata.clone();

    let completed_ttl: u32 = std::env::var("MEMFUSE_TASK_COMPLETED_TTL_HOURS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(24);
    let failed_ttl: u32 = std::env::var("MEMFUSE_TASK_FAILED_TTL_HOURS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(168);
    let max_tasks: usize = std::env::var("MEMFUSE_MAX_TASKS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10_000);
    let _ = metadata.evict_expired_tasks(completed_ttl, failed_ttl);
    let _ = metadata.evict_oldest_tasks_fifo(max_tasks);

    for task in metadata
        .list_tasks(&state.config.account_id, &state.config.user_id, limit)
        .map_err(AppError::from_error)?
    {
        tasks.push(serde_json::json!({
            "kind": "metadata",
            "task_id": task.task_key,
            "status": task.state,
            "summary": task.summary,
        }));
    }

    let total_count = tasks.len();
    let items: Vec<_> = tasks.into_iter().take(limit).collect();

    Ok(Json(serde_json::json!({
        "items": items.clone(),
        "tasks": items,
        "next_cursor": null,
        "total_count": total_count,
        "limit": limit,
    })))
}

pub(super) async fn evict_tasks(
    State(state): State<Arc<AppState>>,
) -> HandlerResult<Json<serde_json::Value>> {
    let completed_ttl: u32 = std::env::var("MEMFUSE_TASK_COMPLETED_TTL_HOURS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(24);
    let failed_ttl: u32 = std::env::var("MEMFUSE_TASK_FAILED_TTL_HOURS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(168);
    let max_tasks: usize = std::env::var("MEMFUSE_MAX_TASKS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10000);

    let metadata = state.metadata.clone();
    let ttl_evicted = metadata
        .evict_expired_tasks(completed_ttl, failed_ttl)
        .unwrap_or(0);
    let fifo_evicted = metadata.evict_oldest_tasks_fifo(max_tasks).unwrap_or(0);

    Ok(Json(serde_json::json!({
        "ttl_evicted": ttl_evicted,
        "fifo_evicted": fifo_evicted,
        "completed_ttl_hours": completed_ttl,
        "failed_ttl_hours": failed_ttl,
        "max_tasks": max_tasks,
    })))
}
