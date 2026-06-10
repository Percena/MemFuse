use super::api_types::SearchStrategy;
use super::*;
use chrono::Utc;

/// Read markdown memory files (profile, preferences, entities, events) from the
/// user memories directory and return them as a formatted string.
/// Uses async I/O to avoid blocking the tokio runtime.
async fn read_markdown_memories(
    workspace_root: &std::path::Path,
    account_id: &str,
    user_id: &str,
) -> String {
    let memories_root = workspace_root
        .join("tenants")
        .join(account_id)
        .join(user_id)
        .join("user")
        .join("memories");

    let mut sections = Vec::new();

    // Read profile.md
    let profile_path = memories_root.join("profile.md");
    if let Ok(content) = tokio::fs::read_to_string(&profile_path).await {
        let trimmed = content.trim();
        if !trimmed.is_empty() && trimmed.len() > 20 {
            sections.push(trimmed.to_owned());
        }
    }

    // Read preferences/general.md
    let prefs_path = memories_root.join("preferences").join("general.md");
    if let Ok(content) = tokio::fs::read_to_string(&prefs_path).await {
        let trimmed = content.trim();
        if !trimmed.is_empty() && trimmed.len() > 20 {
            sections.push(trimmed.to_owned());
        }
    }

    // Read entity files (sorted for deterministic output)
    let entities_dir = memories_root.join("entities");
    if let Ok(mut entries) = collect_md_files(&entities_dir).await {
        entries.sort();
        for path in entries {
            if let Ok(content) = tokio::fs::read_to_string(&path).await {
                let trimmed = content.trim();
                if !trimmed.is_empty() && trimmed.len() > 20 {
                    sections.push(trimmed.to_owned());
                }
            }
        }
    }

    // Read event files (sorted for deterministic output)
    let events_dir = memories_root.join("events");
    if let Ok(mut entries) = collect_md_files(&events_dir).await {
        entries.sort();
        for path in entries {
            if let Ok(content) = tokio::fs::read_to_string(&path).await {
                let trimmed = content.trim();
                if !trimmed.is_empty() && trimmed.len() > 20 {
                    sections.push(trimmed.to_owned());
                }
            }
        }
    }

    sections.join("\n\n")
}

/// Collect all non-hidden .md files in a directory.
async fn collect_md_files(
    dir: &std::path::Path,
) -> Result<Vec<std::path::PathBuf>, std::io::Error> {
    let mut files = Vec::new();
    let mut entries = tokio::fs::read_dir(dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "md")
            && !path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .starts_with('.')
        {
            files.push(path);
        }
    }
    Ok(files)
}

// ── Context Resolve Handler ────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(in crate::http) struct ResolveMemoryContextRequest {
    pub query: String,
    pub session_id: Option<String>,
    pub token_budget: Option<usize>,
    pub user_id: Option<String>,
    pub resource_id: Option<String>,
    /// Search strategy preset: precision (default), diverse, recent, comprehensive
    pub strategy: Option<SearchStrategy>,
    /// Point-in-time query: return facts that were
    /// effective at this ISO 8601 timestamp. When set, uses `get_facts_at_time`
    /// instead of `get_active_facts`, filtering by valid_from/valid_to window.
    /// Example: "2026-04-01T00:00:00Z" returns facts effective on April 1.
    pub at_time: Option<String>,
    /// Who triggered this recall. `"auto"` (passive hook injection at
    /// SessionStart / UserPromptSubmit / PreCompact) skips the recall
    /// reinforcement writeback so the forgetting curve only strengthens on
    /// explicit retrieval (agent-initiated calls, cite_memories).
    pub recall_source: Option<String>,
}

#[derive(Debug, Serialize)]
pub(in crate::http) struct ResolveMemoryContextResponse {
    pub sections: MemoryContextSections,
    pub artifacts: MemoryContextArtifacts,
    pub detail_handles: Vec<String>,
    pub rendered_markdown: String,
}

/// Resolve memory context by wiring through the full mfs-memory pipeline:
///
/// 1. Get session turns from SessionEngine → build ConversationTurns
/// 2. Build overlay entries from recent turns (overlay module)
/// 3. Get active facts from MetadataStore → convert to FactEntry
/// 4. Get relevant episodes from MetadataStore → convert to EpisodeSummary
/// 5. Classify user intent from the query (intent module)
/// 6. Plan section budgets based on overlay size, fact/episode counts (budget module)
/// 7. Filter facts by intent and budget (intent + budget modules)
/// 8. Limit episodes by budget (budget module)
/// 9. Build MemoryContextResponse from all sections
/// Resolve memory context for a given query.
///
/// Thin handler: delegates all business logic (fact/episode loading, intent classification,
/// budget planning, filtering, reranking, rendering) to `mfs_memory::service::resolve_context`.
/// Handler retains: session turn fetching, markdown file I/O, audit logging.
pub(in crate::http) async fn resolve_memory_context(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ResolveMemoryContextRequest>,
) -> HandlerResult<Json<ResolveMemoryContextResponse>> {
    if request.query.is_empty() && request.at_time.is_none() {
        return Err(AppError(MfsError::InvalidArgument {
            field: "query".into(),
            reason: "query must not be empty when at_time is not specified".into(),
        }));
    }

    let account_id = state.config.account_id.clone();
    let user_id = request
        .user_id
        .clone()
        .unwrap_or_else(|| state.config.user_id.clone());

    // 1. Get session turns from SessionEngine — this is I/O that doesn't belong to mfs-memory.
    let session_id = request.session_id.clone().unwrap_or_default();
    let mut conversation_turns: Vec<ConversationTurn> = Vec::new();

    if session_id.is_empty() {
        let sessions = state
            .session_engine
            .list_sessions(
                &state.config.account_id,
                &state.config.user_id,
                &state.config.agent_id,
            )
            .await?;

        if let Some(latest) = sessions.first() {
            if let Ok(ctx) = state
                .session_engine
                .get_session_context(&latest.session_id, 200)
                .await
            {
                conversation_turns = ctx
                    .messages
                    .iter()
                    .enumerate()
                    .map(|(i, msg)| {
                        let offset = chrono::Duration::seconds(
                            (ctx.messages.len().saturating_sub(i + 1)) as i64 * 30,
                        );
                        let ts = Utc::now() - offset;
                        ConversationTurn {
                            turn_id: format!("{}-{}", latest.session_id, i),
                            turn_seq: i as i64,
                            session_id: latest.session_id.clone(),
                            user_id: user_id.clone(),
                            role: TurnRole::from_str(&msg.role),
                            content_text: msg.content.clone(),
                            token_count: msg.content.len() / 4,
                            created_at: ts.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                        }
                    })
                    .collect();
            }
        }
    } else if let Ok(ctx) = state
        .session_engine
        .get_session_context(&session_id, 200)
        .await
    {
        conversation_turns = ctx
            .messages
            .iter()
            .enumerate()
            .map(|(i, msg)| {
                let offset = chrono::Duration::seconds(
                    (ctx.messages.len().saturating_sub(i + 1)) as i64 * 30,
                );
                let ts = Utc::now() - offset;
                ConversationTurn {
                    turn_id: format!("{}-{}", session_id, i),
                    turn_seq: i as i64,
                    session_id: session_id.clone(),
                    user_id: user_id.clone(),
                    role: TurnRole::from_str(&msg.role),
                    content_text: msg.content.clone(),
                    token_count: msg.content.len() / 4,
                    created_at: ts.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                }
            })
            .collect();
    }

    // 2. Delegate all business logic to the MemoryService facade.
    let strategy: mfs_memory::SearchStrategy = request.strategy.unwrap_or_default().into();
    let input = mfs_memory::service::ResolveContextInput {
        account_id: account_id.clone(),
        user_id: user_id.clone(),
        resource_id: request.resource_id.clone(),
        session_id: request.session_id.clone(),
        query: request.query.clone(),
        budget: request.token_budget.unwrap_or(DEFAULT_INJECTION_BUDGET),
        strategy,
        at_time: request.at_time.clone(),
        reinforce_recall: request.recall_source.as_deref() != Some("auto"),
    };

    let result = mfs_memory::service::resolve_context(
        &state.metadata,
        &input,
        &conversation_turns,
        &state.read_llm,
        state.read_llm_enabled,
        state.read_embed_timeout_ms,
        state.embedding_provider.as_ref(),
    )
    .await
    .map_err(|e| AppError(MfsError::Internal { message: e }))?;

    // 3. Append rich narrative memories from markdown files (I/O, not business logic).
    let mut rendered_markdown = result.rendered_markdown;
    let markdown_memories =
        read_markdown_memories(&state.config.workspace_root, &account_id, &user_id).await;
    if !markdown_memories.is_empty() {
        rendered_markdown.push_str("\n\n[Memory Files]\n");
        rendered_markdown.push_str(&markdown_memories);
    }

    // 4. Audit log.
    append_audit(
        &state,
        "resolve_memory_context",
        Some(&format!("query={}", &request.query)),
        Some("{\"result\":\"ok\"}"),
    );

    Ok(Json(ResolveMemoryContextResponse {
        sections: result.sections,
        artifacts: result.artifacts,
        detail_handles: result.detail_handles,
        rendered_markdown,
    }))
}
