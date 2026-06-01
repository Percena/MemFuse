use super::*;
use chrono::Utc;
use mfs_memory::ConversationTurn;
use mfs_memory::FactAssertion;
use mfs_memory::TurnRole;
use mfs_memory::consolidation::consolidate_and_persist;
use mfs_memory::facts::extract_facts;
use mfs_memory::llm::LlmAssist;

#[derive(Debug, Deserialize)]
pub(in crate::http) struct MemoryConsolidateRequest {
    pub session_id: String,
    pub user_id: String,
    pub resource_id: Option<String>,
}

pub(in crate::http) async fn memory_consolidate(
    State(state): State<Arc<AppState>>,
    req: Json<MemoryConsolidateRequest>,
) -> HandlerResult<Json<serde_json::Value>> {
    if req.user_id.is_empty() {
        return Err(AppError(MfsError::InvalidArgument {
            field: "user_id".into(),
            reason: "user_id must not be empty".into(),
        }));
    }
    let metadata = state.metadata.clone();
    let turns = if let Ok(stored_turns) = metadata.get_turns_by_session(&req.session_id) {
        if stored_turns.is_empty() {
            let context = state
                .session_engine
                .get_session_context(&req.session_id, 128_000)
                .await?;
            session_context_to_conversation_turns(
                &metadata,
                &req.session_id,
                &req.user_id,
                &context.messages,
            )
            .map_err(AppError::from_error)?
        } else {
            stored_turns
                .into_iter()
                .map(|turn| ConversationTurn {
                    turn_id: turn.turn_id,
                    turn_seq: turn.turn_seq,
                    session_id: turn.session_id,
                    user_id: turn.user_id,
                    role: TurnRole::from_str(&turn.role),
                    content_text: turn.content_text,
                    token_count: turn.token_count as usize,
                    created_at: turn.created_at,
                })
                .collect::<Vec<_>>()
        }
    } else {
        let context = state
            .session_engine
            .get_session_context(&req.session_id, 128_000)
            .await?;
        session_context_to_conversation_turns(
            &metadata,
            &req.session_id,
            &req.user_id,
            &context.messages,
        )
        .map_err(AppError::from_error)?
    };

    let account_id = state.config.account_id.clone();
    let agent_id = state.config.agent_id.clone();
    let session_id = req.session_id.clone();
    let user_id = req.user_id.clone();
    let resource_id = req.resource_id.clone();
    let llm = LlmAssist::from_env();
    let result = consolidate_and_persist(
        &metadata,
        &account_id,
        &user_id,
        &agent_id,
        &session_id,
        resource_id.as_deref(),
        &turns,
        &llm,
    )
    .await
    .map_err(AppError::from_error)?;

    let result_json = serde_json::json!({
        "session_id": req.session_id,
        "user_id": req.user_id,
        "status": "completed",
        "range_start_turn_id": result.range_start_turn_id,
        "range_end_turn_id": result.range_end_turn_id,
        "episode_count": result.episode_count,
        "assertion_count": result.assertion_count,
        "fact_count": result.fact_count,
        "turn_count": result.turn_count,
    });
    append_audit(
        &state,
        "memory_consolidate",
        Some(&format!("mfs://session/{}", req.session_id)),
        Some(&result_json.to_string()),
    );
    Ok(Json(result_json))
}

#[derive(Debug, Deserialize)]
pub(in crate::http) struct MemoryExtractFactsRequest {
    pub texts: Vec<String>,
    pub user_id: String,
}

#[derive(Debug, Serialize)]
pub(in crate::http) struct MemoryExtractFactsResponse {
    pub assertions: Vec<FactAssertion>,
}

pub(in crate::http) async fn memory_extract_facts(
    State(_state): State<Arc<AppState>>,
    req: Json<MemoryExtractFactsRequest>,
) -> HandlerResult<Json<MemoryExtractFactsResponse>> {
    if req.user_id.is_empty() {
        return Err(AppError(MfsError::InvalidArgument {
            field: "user_id".into(),
            reason: "user_id must not be empty".into(),
        }));
    }
    let now = Utc::now();
    let turns: Vec<ConversationTurn> = req
        .texts
        .iter()
        .enumerate()
        .map(|(i, text)| ConversationTurn {
            turn_id: format!("tmp-{}", i),
            turn_seq: i as i64,
            session_id: "extract".to_owned(),
            user_id: req.user_id.clone(),
            role: TurnRole::User,
            content_text: text.clone(),
            token_count: text.len() / 4,
            created_at: now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        })
        .collect();
    let llm = LlmAssist::from_env();
    let assertions = extract_facts(&turns, &llm).await;
    Ok(Json(MemoryExtractFactsResponse { assertions }))
}

// ── Private helpers ──────────────────────────────────────────────────

fn session_context_to_conversation_turns(
    metadata: &MetadataStore,
    session_id: &str,
    user_id: &str,
    messages: &[mfs_session::SessionMessageView],
) -> Result<Vec<ConversationTurn>, MfsError> {
    let existing_turns =
        metadata
            .get_turns_by_session(session_id)
            .map_err(|error| MfsError::Internal {
                message: error.to_string(),
            })?;
    let seq_offset = existing_turns.len() as i64;
    let now = Utc::now();

    Ok(messages
        .iter()
        .enumerate()
        .map(|(index, msg)| {
            let offset =
                chrono::Duration::seconds((messages.len().saturating_sub(index + 1)) as i64 * 30);
            let ts = now - offset;
            ConversationTurn {
                turn_id: format!("{session_id}:http:pending:{:03}", index + 1),
                turn_seq: seq_offset + index as i64 + 1,
                session_id: session_id.to_owned(),
                user_id: user_id.to_owned(),
                role: TurnRole::from_str(&msg.role),
                content_text: msg.content.clone(),
                token_count: (msg.content.len() / 4).max(1),
                created_at: ts.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            }
        })
        .collect())
}
