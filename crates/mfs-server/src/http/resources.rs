use super::*;

#[derive(Debug, Deserialize)]
pub(super) struct CreateResourceRequest {
    pub source_kind: Option<String>,
    pub source_path: Option<String>,
    pub logical_name: Option<String>,
    pub branch: Option<String>,
    pub revision: Option<String>,
    pub repo_id: Option<String>,
    pub tracker: Option<String>,
    pub tracker_project_identifier: Option<String>,
    pub file_name: Option<String>,
    pub content: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ResourceListQuery {
    pub limit: Option<usize>,
    pub repo_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct CreateResourcesBatchRequest {
    pub resources: Vec<CreateResourceRequest>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ResourceExportRequest {
    pub output_path: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct ResourceImportRequest {
    pub pack_path: String,
    pub logical_name: Option<String>,
}

pub(super) async fn create_resource(
    State(state): State<Arc<AppState>>,
    Json(request): Json<CreateResourceRequest>,
) -> HandlerResult<Json<serde_json::Value>> {
    let identity = configured_identity(&state.config);
    let prepared = match (request.file_name.as_deref(), request.content.as_deref()) {
        (Some(file_name), Some(content)) => {
            prepare_inline_resource_ingest(
                &state.metadata,
                &state.config.workspace_root,
                &identity,
                file_name,
                content,
                request.logical_name.as_deref(),
            )
            .await?
        }
        (None, None) => {
            let source_kind = request.source_kind.as_deref().unwrap_or("localfs");
            let valid_source_kinds = ["localfs", "git", "git_url", "url", "inline", "import"];
            if !valid_source_kinds.contains(&source_kind) {
                return Err(AppError(mfs_types::MfsError::InvalidArgument {
                    field: "source_kind".into(),
                    reason: format!(
                        "unsupported source_kind '{}', expected one of: {}",
                        source_kind,
                        valid_source_kinds.join(", ")
                    ),
                }));
            }
            prepare_resource_ingest(
                &state.metadata,
                &state.config.workspace_root,
                &identity,
                source_kind,
                request
                    .source_path
                    .as_deref()
                    .expect("source_path required"),
                request.logical_name.as_deref(),
                request.branch.as_deref(),
                request.revision.as_deref(),
            )
            .await?
        }
        _ => {
            return Err(AppError(mfs_types::MfsError::InvalidArgument {
                field: "file_name/content".into(),
                reason: "file_name and content must be provided together".into(),
            }));
        }
    };
    persist_resource_business_metadata(
        &state.metadata,
        &prepared.resource_id,
        request.repo_id.as_deref(),
        request.tracker.as_deref(),
        request.tracker_project_identifier.as_deref(),
    )?;
    let metadata = state.metadata.clone();
    let task_key = semantic_task_key("ingest", &prepared.resource_id);
    upsert_semantic_task(
        &metadata,
        &state.config,
        &task_key,
        "pending",
        Some("resources"),
        Some(&format!("ingest {}", prepared.root_uri)),
        None,
        0,
        2,
        "queued",
        None,
    );
    update_resource_status(&metadata, &prepared.resource_id, "processing");
    let state_for_task = Arc::clone(&state);
    let task_key_for_task = task_key.clone();
    let prepared_for_task = prepared.clone();
    let child_token = state.shutdown_token.child_token();
    let handle = tokio::spawn(async move {
        let task_key_for_log = task_key_for_task.clone();
        tokio::select! {
            () = child_token.cancelled() => {
                tracing::info!(task_key = %task_key_for_log, "resource ingest cancelled on shutdown");
            }
            () = run_resource_task(
                Arc::clone(&state_for_task),
                task_key_for_task,
                Some(prepared_for_task.resource_id.clone()),
                "resource.ingest",
                move || {
                    let state = Arc::clone(&state_for_task);
                    let prepared = prepared_for_task.clone();
                    async move {
                        complete_prepared_resource_ingest(
                            &state.metadata,
                            &state.config.workspace_root,
                            &configured_identity(&state.config),
                            &prepared,
                        )
                        .await
                        .map(|result| (Some(prepared.root_uri), Some(result.mode)))
                        .map_err(|error| error.to_string())
                    }
                },
            ) => {}
        }
    });
    state.push_background_handle(handle)?;
    Ok(Json(serde_json::json!({
        "task_key": task_key,
        "resource_id": prepared.resource_id,
        "logical_name": prepared.logical_name,
        "root_uri": prepared.root_uri,
        "repo_id": request.repo_id,
        "tracker": request.tracker,
        "tracker_project_identifier": request.tracker_project_identifier,
        "state": "pending",
    })))
}

pub(super) async fn create_resources_batch(
    State(state): State<Arc<AppState>>,
    Json(request): Json<CreateResourcesBatchRequest>,
) -> HandlerResult<Json<serde_json::Value>> {
    if request.resources.is_empty() {
        return Err(AppError(mfs_types::MfsError::InvalidArgument {
            field: "resources".into(),
            reason: "batch must contain at least one resource".into(),
        }));
    }
    if request.resources.len() > 50 {
        return Err(AppError(mfs_types::MfsError::InvalidArgument {
            field: "resources".into(),
            reason: "batch must contain at most 50 resources".into(),
        }));
    }

    let identity = configured_identity(&state.config);
    let mut results = Vec::with_capacity(request.resources.len());
    let semaphore = Arc::new(tokio::sync::Semaphore::new(10));

    for (batch_index, item) in request.resources.into_iter().enumerate() {
        let prepared = match (item.file_name.as_deref(), item.content.as_deref()) {
            (Some(file_name), Some(content)) => {
                match prepare_inline_resource_ingest(
                    &state.metadata,
                    &state.config.workspace_root,
                    &identity,
                    file_name,
                    content,
                    item.logical_name.as_deref(),
                )
                .await
                {
                    Ok(p) => p,
                    Err(e) => {
                        results.push(serde_json::json!({
                            "index": batch_index,
                            "error": {
                                "category": "Internal",
                                "message": format!("inline ingest preparation failed: {}", mfs_types::sanitize_secrets(&e.to_string())),
                                "retryable": false,
                            },
                            "state": "failed",
                        }));
                        continue;
                    }
                }
            }
            (None, None) => {
                let source_kind = item.source_kind.as_deref().unwrap_or("localfs");
                let valid_source_kinds = ["localfs", "git", "git_url", "url", "inline", "import"];
                if !valid_source_kinds.contains(&source_kind) {
                    results.push(serde_json::json!({
                        "index": batch_index,
                        "error": {
                            "category": "InvalidArgument",
                            "message": format!("invalid source_kind '{}'", source_kind),
                            "retryable": false,
                        },
                        "state": "failed",
                    }));
                    continue;
                }

                let source_path = match item.source_path.as_deref() {
                    Some(p) => p,
                    None => {
                        results.push(serde_json::json!({
                            "index": batch_index,
                            "error": {
                                "category": "InvalidArgument",
                                "message": "source_path is required when file_name/content are not provided",
                                "retryable": false,
                            },
                            "state": "failed",
                        }));
                        continue;
                    }
                };

                match prepare_resource_ingest(
                    &state.metadata,
                    &state.config.workspace_root,
                    &identity,
                    source_kind,
                    source_path,
                    item.logical_name.as_deref(),
                    item.branch.as_deref(),
                    item.revision.as_deref(),
                )
                .await
                {
                    Ok(p) => p,
                    Err(e) => {
                        results.push(serde_json::json!({
                            "index": batch_index,
                            "error": {
                                "category": "Internal",
                                "message": format!("resource ingest preparation failed: {}", mfs_types::sanitize_secrets(&e.to_string())),
                                "retryable": false,
                            },
                            "state": "failed",
                        }));
                        continue;
                    }
                }
            }
            _ => {
                results.push(serde_json::json!({
                    "index": batch_index,
                    "error": {
                        "category": "InvalidArgument",
                        "message": "file_name and content must be provided together",
                        "retryable": false,
                    },
                    "state": "failed",
                }));
                continue;
            }
        };

        persist_resource_business_metadata(
            &state.metadata,
            &prepared.resource_id,
            item.repo_id.as_deref(),
            item.tracker.as_deref(),
            item.tracker_project_identifier.as_deref(),
        )?;
        let metadata = state.metadata.clone();
        let task_key = semantic_task_key("ingest", &prepared.resource_id);
        upsert_semantic_task(
            &metadata,
            &state.config,
            &task_key,
            "pending",
            Some("resources"),
            Some(&format!("ingest {}", prepared.root_uri)),
            None,
            0,
            2,
            "queued",
            None,
        );
        update_resource_status(&metadata, &prepared.resource_id, "processing");

        let state_for_task = Arc::clone(&state);
        let task_key_for_task = task_key.clone();
        let prepared_for_task = prepared.clone();
        let semaphore_for_task = Arc::clone(&semaphore);
        let child_token = state.shutdown_token.child_token();
        let handle = tokio::spawn(async move {
            let _permit = semaphore_for_task.acquire().await.unwrap();
            let task_key_for_log = task_key_for_task.clone();
            tokio::select! {
                () = child_token.cancelled() => {
                    tracing::info!(task_key = %task_key_for_log, "resource batch ingest cancelled on shutdown");
                }
                () = run_resource_task(
                    Arc::clone(&state_for_task),
                    task_key_for_task,
                    Some(prepared_for_task.resource_id.clone()),
                    "resource.ingest_batch",
                    move || {
                        let state = Arc::clone(&state_for_task);
                        let prepared = prepared_for_task.clone();
                        async move {
                            complete_prepared_resource_ingest(
                                &state.metadata,
                                &state.config.workspace_root,
                                &configured_identity(&state.config),
                                &prepared,
                            )
                            .await
                            .map(|result| (Some(prepared.root_uri), Some(result.mode)))
                            .map_err(|error| error.to_string())
                        }
                    },
                ) => {}
            }
        });
        state.push_background_handle(handle)?;

        results.push(serde_json::json!({
            "index": batch_index,
            "task_key": task_key,
            "resource_id": prepared.resource_id,
            "logical_name": prepared.logical_name,
            "root_uri": prepared.root_uri,
            "repo_id": item.repo_id,
            "tracker": item.tracker,
            "tracker_project_identifier": item.tracker_project_identifier,
            "state": "pending",
        }));
    }

    Ok(Json(serde_json::json!({
        "results": results,
        "count": results.len(),
    })))
}

pub(super) async fn import_resource(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ResourceImportRequest>,
) -> HandlerResult<Json<serde_json::Value>> {
    let identity = configured_identity(&state.config);
    // Validate pack_path is within workspace root (path traversal protection)
    let validated_pack_path =
        validate_path_within_workspace(&state.config.workspace_root, &request.pack_path)?;
    let prepared = import_resource_pack(
        &state.metadata,
        &state.config.workspace_root,
        &identity,
        &validated_pack_path,
        request.logical_name.as_deref(),
    )
    .await?;
    let metadata = state.metadata.clone();
    let task_key = semantic_task_key("import", &prepared.resource_id);
    upsert_semantic_task(
        &metadata,
        &state.config,
        &task_key,
        "pending",
        Some("resources"),
        Some(&format!("import {}", prepared.root_uri)),
        None,
        0,
        2,
        "queued",
        None,
    );
    update_resource_status(&metadata, &prepared.resource_id, "processing");
    let state_for_task = Arc::clone(&state);
    let task_key_for_task = task_key.clone();
    let prepared_for_task = prepared.clone();
    let child_token = state.shutdown_token.child_token();
    let handle = tokio::spawn(async move {
        let task_key_for_log = task_key_for_task.clone();
        tokio::select! {
            () = child_token.cancelled() => {
                tracing::info!(task_key = %task_key_for_log, "resource import cancelled on shutdown");
            }
            () = run_resource_task(
                Arc::clone(&state_for_task),
                task_key_for_task,
                Some(prepared_for_task.resource_id.clone()),
                "resource.import",
                move || {
                    let state = Arc::clone(&state_for_task);
                    let prepared = prepared_for_task.clone();
                    async move {
                        complete_prepared_resource_ingest(
                            &state.metadata,
                            &state.config.workspace_root,
                            &configured_identity(&state.config),
                            &prepared,
                        )
                        .await
                        .map(|result| (Some(prepared.root_uri), Some(result.mode)))
                        .map_err(|error| error.to_string())
                    }
                },
            ) => {}
        }
    });
    state.push_background_handle(handle)?;
    Ok(Json(serde_json::json!({
        "task_key": task_key,
        "resource_id": prepared.resource_id,
        "logical_name": prepared.logical_name,
        "root_uri": prepared.root_uri,
        "state": "pending",
    })))
}

pub(super) async fn list_resources(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ResourceListQuery>,
) -> HandlerResult<Json<serde_json::Value>> {
    let limit = query.limit.unwrap_or(100);
    let metadata = state.metadata.clone();
    let resources: Vec<_> = metadata
        .list_resource_sources(
            &state.config.account_id,
            &state.config.user_id,
            limit,
            query.repo_id.as_deref(),
        )
        .map_err(AppError::from_error)?
        .into_iter()
        .map(|resource| {
            serde_json::json!({
                "resource_id": resource.resource_id,
                "logical_name": resource.logical_name,
                "source_kind": resource.source_kind,
                "source_identifier": resource.source_identifier,
                "root_uri": resource.canonical_root_uri,
                "source_host": resource.source_host,
                "source_namespace": resource.source_namespace,
                "source_repo": resource.source_repo,
                "source_ref": resource.source_ref,
                "repo_id": resource.repo_id,
                "tracker": resource.tracker,
                "tracker_project_identifier": resource.tracker_project_identifier,
                "status": resource.status,
                "last_snapshot_id": resource.last_snapshot_id,
            })
        })
        .collect();
    let total_count = resources.len();
    Ok(Json(serde_json::json!({
        "items": resources.clone(),
        "resources": resources,
        "next_cursor": null,
        "total_count": total_count,
        "limit": limit,
    })))
}

fn persist_resource_business_metadata(
    metadata: &mfs_metadata::MetadataStore,
    resource_id: &str,
    repo_id: Option<&str>,
    tracker: Option<&str>,
    tracker_project_identifier: Option<&str>,
) -> HandlerResult<()> {
    if repo_id.is_none() && tracker.is_none() && tracker_project_identifier.is_none() {
        return Ok(());
    }

    metadata
        .update_resource_business_metadata(
            resource_id,
            repo_id,
            tracker,
            tracker_project_identifier,
        )
        .map(|_| ())
        .map_err(AppError::from_error)
}

pub(super) async fn refresh_resource(
    State(state): State<Arc<AppState>>,
    Path(resource_id): Path<String>,
) -> HandlerResult<Json<serde_json::Value>> {
    let metadata = state.metadata.clone();
    let resource = metadata
        .get_resource_source(&resource_id)
        .map_err(AppError::from_error)?
        .ok_or_else(|| {
            AppError(mfs_types::MfsError::NotFound {
                resource: resource_id.clone(),
            })
        })?;
    let task_key = semantic_task_key("refresh", &resource_id);
    let task_key_for_task = task_key.clone();
    upsert_semantic_task(
        &metadata,
        &state.config,
        &task_key,
        "pending",
        Some("resources"),
        Some(&format!("refresh {}", resource.canonical_root_uri)),
        None,
        0,
        2,
        "queued",
        None,
    );
    update_resource_status(&metadata, &resource_id, "processing");
    let state_for_task = Arc::clone(&state);
    let state_for_worker = Arc::clone(&state);
    let resource_id_for_task = resource_id.clone();
    let resource_id_for_worker = resource_id.clone();
    let child_token = state.shutdown_token.child_token();
    let handle = tokio::spawn(async move {
        let task_key_for_log = task_key_for_task.clone();
        tokio::select! {
            () = child_token.cancelled() => {
                tracing::info!(task_key = %task_key_for_log, "resource refresh cancelled on shutdown");
            }
            () = run_resource_task(
                state_for_task,
                task_key_for_task,
                Some(resource_id_for_task.clone()),
                "refresh",
                move || {
                    let state = Arc::clone(&state_for_worker);
                    let resource_id = resource_id_for_worker.clone();
                    async move {
                        refresh_registered_resource(
                            &state.metadata,
                            &state.config.workspace_root,
                            &configured_identity(&state.config),
                            &resource_id,
                        )
                        .await
                        .map(|result| (Some(result.root_uri), Some(result.mode)))
                        .map_err(|error| error.to_string())
                    }
                },
            ) => {}
        }
    });
    state.push_background_handle(handle)?;
    Ok(Json(serde_json::json!({
        "task_key": task_key,
        "resource_id": resource.resource_id,
        "root_uri": resource.canonical_root_uri,
        "state": "pending",
    })))
}

pub(super) async fn rebuild_resource(
    State(state): State<Arc<AppState>>,
    Path(resource_id): Path<String>,
) -> HandlerResult<Json<serde_json::Value>> {
    let metadata = state.metadata.clone();
    let resource = metadata
        .get_resource_source(&resource_id)
        .map_err(AppError::from_error)?
        .ok_or_else(|| {
            AppError(mfs_types::MfsError::NotFound {
                resource: resource_id.clone(),
            })
        })?;
    let task_key = semantic_task_key("rebuild", &resource_id);
    let task_key_for_task = task_key.clone();
    upsert_semantic_task(
        &metadata,
        &state.config,
        &task_key,
        "pending",
        Some("resources"),
        Some(&format!("rebuild {}", resource.canonical_root_uri)),
        None,
        0,
        2,
        "queued",
        None,
    );
    let state_for_task = Arc::clone(&state);
    let state_for_worker = Arc::clone(&state);
    let resource_id_for_task = resource_id.clone();
    let resource_id_for_worker = resource_id.clone();
    let child_token = state.shutdown_token.child_token();
    let handle = tokio::spawn(async move {
        let task_key_for_log = task_key_for_task.clone();
        tokio::select! {
            () = child_token.cancelled() => {
                tracing::info!(task_key = %task_key_for_log, "resource rebuild cancelled on shutdown");
            }
            () = run_resource_task(
                state_for_task,
                task_key_for_task,
                Some(resource_id_for_task.clone()),
                "rebuild",
                move || {
                    let state = Arc::clone(&state_for_worker);
                    let resource_id = resource_id_for_worker.clone();
                    async move {
                        rebuild_registered_resource(
                            &state.metadata,
                            &state.config.workspace_root,
                            &configured_identity(&state.config),
                            &resource_id,
                        )
                        .await
                        .map(|result| (Some(result.root_uri), Some(result.mode)))
                        .map_err(|error| error.to_string())
                    }
                },
            ) => {}
        }
    });
    state.push_background_handle(handle)?;
    Ok(Json(serde_json::json!({
        "task_key": task_key,
        "resource_id": resource.resource_id,
        "root_uri": resource.canonical_root_uri,
        "state": "pending",
    })))
}

pub(super) async fn export_resource(
    State(state): State<Arc<AppState>>,
    Path(resource_id): Path<String>,
    Json(request): Json<ResourceExportRequest>,
) -> HandlerResult<Json<serde_json::Value>> {
    let identity = configured_identity(&state.config);
    // Validate output_path is within workspace root (path traversal protection)
    let validated_output_path =
        validate_path_within_workspace(&state.config.workspace_root, &request.output_path)?;
    let manifest = export_resource_pack(
        &state.metadata,
        &state.config.workspace_root,
        &identity,
        &resource_id,
        &validated_output_path,
    )
    .await?;
    append_audit(
        &state,
        "resource.export",
        Some(&manifest.canonical_root_uri),
        Some("{\"result\":\"ok\"}"),
    );
    Ok(Json(serde_json::json!({
        "output_path": request.output_path,
        "logical_name": manifest.logical_name,
        "root_uri": manifest.canonical_root_uri,
    })))
}
