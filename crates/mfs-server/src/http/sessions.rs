use super::*;

use super::api_types::TurnRole;

#[derive(Debug, Deserialize)]
pub(super) struct CreateSessionRequest {
    pub session_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct AddMessageRequest {
    pub role: TurnRole,
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct AddObservationRequest {
    pub tool_name: String,
    pub tool_input: String,
    pub tool_output: String,
    pub content: String,
    pub platform: Option<String>,
    /// Trust level of the observation source: "internal" (default), "external", "mixed".
    /// External sources (MCP tools, web search) produce facts with reduced confidence.
    pub source_trust: Option<String>,
    /// Structured metadata about the observation (tool_type, summary, outcome, etc.).
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub(super) struct UsedContextRequest {
    pub uri: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct UsedSkillRequest {
    pub skill_uri: String,
    pub success: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct UsedToolRequest {
    pub tool_uri: String,
    pub success: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct CommitSessionRequest {
    pub user_id: Option<String>,
    pub thread_id: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct SessionCreatedResponse {
    pub session_id: String,
}

pub(super) async fn create_session(
    State(state): State<Arc<AppState>>,
    Json(request): Json<CreateSessionRequest>,
) -> HandlerResult<Json<SessionCreatedResponse>> {
    let session_id = match request.session_id.as_deref() {
        Some(session_id) => {
            state
                .session_engine
                .new_session_with_id(
                    &state.config.account_id,
                    &state.config.user_id,
                    &state.config.agent_id,
                    session_id,
                )
                .await?
        }
        None => {
            state
                .session_engine
                .new_session(
                    &state.config.account_id,
                    &state.config.user_id,
                    &state.config.agent_id,
                )
                .await?
        }
    };
    append_audit(&state, "session.create", None, Some("{\"result\":\"ok\"}"));
    Ok(Json(SessionCreatedResponse { session_id }))
}

pub(super) async fn list_sessions(
    State(state): State<Arc<AppState>>,
    Query(query): Query<AuditQuery>,
) -> HandlerResult<Json<serde_json::Value>> {
    let limit = query.limit.unwrap_or(20);
    let sessions = state
        .session_engine
        .list_sessions(
            &state.config.account_id,
            &state.config.user_id,
            &state.config.agent_id,
        )
        .await?;
    let total_count = sessions.len();
    let items: Vec<_> = sessions.into_iter().take(limit).collect();
    Ok(Json(serde_json::json!({
        "items": items.clone(),
        "sessions": items,
        "next_cursor": null,
        "total_count": total_count,
        "limit": limit,
    })))
}

pub(super) async fn get_session(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> HandlerResult<Json<serde_json::Value>> {
    let session = state.session_engine.get_session(&session_id).await?;
    Ok(Json(serde_json::to_value(session).map_err(|e| {
        AppError(mfs_types::MfsError::Internal {
            message: e.to_string(),
        })
    })?))
}

pub(super) async fn delete_session(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> HandlerResult<Json<serde_json::Value>> {
    state.session_engine.delete_session(&session_id).await?;
    append_audit(
        &state,
        "session.delete",
        Some(&session_id),
        Some("{\"result\":\"ok\"}"),
    );
    Ok(Json(serde_json::json!({ "deleted": true })))
}

pub(super) async fn add_message(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Json(request): Json<AddMessageRequest>,
) -> HandlerResult<Json<serde_json::Value>> {
    let result = state
        .session_engine
        .add_message(&session_id, request.role.as_str(), &request.content)
        .await?;
    append_audit(
        &state,
        "session.add_message",
        None,
        Some("{\"result\":\"ok\"}"),
    );
    Ok(Json(serde_json::json!({
        "ok": true,
        "session_id": session_id,
        "auto_committed": result.auto_committed,
        "archive_uri": result.archive_uri,
        "task_id": result.task_id,
    })))
}

pub(super) async fn get_session_context(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Query(query): Query<SessionContextQuery>,
) -> HandlerResult<Json<serde_json::Value>> {
    let context = state
        .session_engine
        .get_session_context(&session_id, query.token_budget.unwrap_or(128_000))
        .await?;
    Ok(Json(serde_json::to_value(context).map_err(|e| {
        AppError(mfs_types::MfsError::Internal {
            message: e.to_string(),
        })
    })?))
}

pub(super) async fn get_session_archive(
    State(state): State<Arc<AppState>>,
    Path((session_id, archive_id)): Path<(String, String)>,
) -> HandlerResult<Json<serde_json::Value>> {
    let archive = state
        .session_engine
        .get_session_archive(&session_id, &archive_id)
        .await?;
    Ok(Json(serde_json::to_value(archive).map_err(|e| {
        AppError(mfs_types::MfsError::Internal {
            message: e.to_string(),
        })
    })?))
}

pub(super) async fn used_context(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Json(request): Json<UsedContextRequest>,
) -> HandlerResult<Json<serde_json::Value>> {
    state
        .session_engine
        .used_context(&session_id, &request.uri)
        .await?;
    append_audit(
        &state,
        "session.used_context",
        Some(&request.uri),
        Some("{\"result\":\"ok\"}"),
    );
    Ok(Json(serde_json::json!({ "ok": true })))
}

pub(super) async fn used_skill(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Json(request): Json<UsedSkillRequest>,
) -> HandlerResult<Json<serde_json::Value>> {
    state
        .session_engine
        .used_skill(&session_id, &request.skill_uri, request.success)
        .await?;
    append_audit(
        &state,
        "session.used_skill",
        Some(&request.skill_uri),
        Some("{\"result\":\"ok\"}"),
    );
    Ok(Json(serde_json::json!({ "ok": true })))
}

pub(super) async fn used_tool(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Json(request): Json<UsedToolRequest>,
) -> HandlerResult<Json<serde_json::Value>> {
    state
        .session_engine
        .used_tool(&session_id, &request.tool_uri, request.success)
        .await?;
    append_audit(
        &state,
        "session.used_tool",
        Some(&request.tool_uri),
        Some("{\"result\":\"ok\"}"),
    );
    Ok(Json(serde_json::json!({ "ok": true })))
}

pub(super) async fn commit_session(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Json(request): Json<CommitSessionRequest>,
) -> HandlerResult<Json<serde_json::Value>> {
    let result = state.session_engine.commit(&session_id).await?;
    append_audit(
        &state,
        "session.commit",
        Some(&result.archive_uri),
        Some(
            &serde_json::json!({
                "user_id": request.user_id,
                "thread_id": request.thread_id,
                "reason": request.reason,
            })
            .to_string(),
        ),
    );
    Ok(Json(serde_json::json!({
        "archive_uri": result.archive_uri,
        "task_id": result.task_id,
    })))
}

pub(super) async fn add_observation(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Json(request): Json<AddObservationRequest>,
) -> HandlerResult<Json<serde_json::Value>> {
    match state.session_engine.get_session(&session_id).await {
        Ok(_) => {}
        Err(mfs_session::SessionError::NotFound(_)) => {
            state
                .session_engine
                .new_session_with_id(
                    &state.config.account_id,
                    &state.config.user_id,
                    &state.config.agent_id,
                    &session_id,
                )
                .await?;
        }
        Err(err) => return Err(err.into()),
    }

    let platform = request.platform.unwrap_or_else(|| "mcp".to_string());
    let turn_id = format!(
        "{}:obs:{}:{}",
        session_id,
        request.tool_name,
        uuid::Uuid::new_v4()
    );

    let source_trust_str = request.source_trust.as_deref();
    let safe_tool_input = mfs_types::sanitize_secrets(&request.tool_input);
    let safe_tool_output = mfs_types::sanitize_secrets(&request.tool_output);
    let safe_content = mfs_types::sanitize_secrets(&request.content);
    let metadata_str = request
        .metadata
        .as_ref()
        .and_then(|v| serde_json::to_string(v).ok())
        .map(|value| mfs_types::sanitize_secrets(&value));

    let result = state
        .session_engine
        .add_observation(
            &session_id,
            &request.tool_name,
            &safe_tool_input,
            &safe_tool_output,
            &safe_content,
            &platform,
            source_trust_str,
            metadata_str.as_deref(),
        )
        .await?;

    append_audit(
        &state,
        "session.add_observation",
        Some(&format!("session:{}", session_id)),
        Some("{\"result\":\"ok\"}"),
    );

    Ok(Json(serde_json::json!({
        "ok": true,
        "session_id": session_id,
        "turn_id": turn_id,
        "auto_committed": result.auto_committed,
        "archive_uri": result.archive_uri,
        "task_id": result.task_id,
    })))
}

pub(super) async fn session_timeline(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> HandlerResult<Json<serde_json::Value>> {
    let archive_ids = state
        .session_engine
        .list_session_archives(&session_id)
        .await?;

    let mut timeline = Vec::new();
    for archive_id in &archive_ids {
        if let Ok(archive) = state
            .session_engine
            .get_session_archive(&session_id, archive_id)
            .await
        {
            timeline.push(serde_json::json!({
                "archive_id": archive_id,
                "abstract": archive.abstract_text,
                "message_count": archive.messages.len(),
            }));
        }
    }

    Ok(Json(serde_json::json!({
        "session_id": session_id,
        "timeline": timeline,
        "count": timeline.len(),
    })))
}
