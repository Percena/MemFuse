use super::*;

#[derive(Debug, Deserialize)]
pub(super) struct ListFactsQuery {
    pub account_id: Option<String>,
    pub user_id: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub(super) struct CreateFactRequest {
    pub id: String,
    pub subject: String,
    pub predicate: String,
    pub display_value: String,
    pub confidence: Option<f64>,
    pub agent_id: Option<String>,
    pub value_type: Option<String>,
    pub source_assertion_id: Option<String>,
    pub source_episode_ids_json: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct SupersedeFactRequest {
    pub new_fact_id: String,
}

pub(super) async fn list_facts(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ListFactsQuery>,
) -> HandlerResult<Json<serde_json::Value>> {
    let account_id = query
        .account_id
        .unwrap_or_else(|| state.config.account_id.clone());
    let user_id = query
        .user_id
        .unwrap_or_else(|| state.config.user_id.clone());

    let metadata = state.metadata.clone();
    let facts = metadata
        .get_active_facts(&account_id, &user_id)
        .map_err(AppError::from_error)?;
    let limit = query.limit.unwrap_or(100);
    let total_count = facts.len();
    let items: Vec<_> = facts.into_iter().take(limit).collect();

    Ok(Json(serde_json::json!({
        "items": items.clone(),
        "facts": items,
        "next_cursor": null,
        "total_count": total_count,
        "count": total_count,
        "limit": limit,
    })))
}

pub(super) async fn create_fact(
    State(state): State<Arc<AppState>>,
    Json(request): Json<CreateFactRequest>,
) -> HandlerResult<Json<serde_json::Value>> {
    // Validate confidence is in [0, 1] range
    let confidence = request.confidence.unwrap_or(0.0);
    if !(0.0..=1.0).contains(&confidence) {
        return Err(AppError(mfs_types::MfsError::InvalidArgument {
            field: "confidence".into(),
            reason: format!("confidence must be between 0.0 and 1.0, got {}", confidence),
        }));
    }

    let metadata = state.metadata.clone();
    let valid_from_now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let record = mfs_metadata::FactRecord {
        id: &request.id,
        account_id: &state.config.account_id,
        user_id: &state.config.user_id,
        agent_id: request.agent_id.as_deref(),
        subject: &request.subject,
        predicate: &request.predicate,
        display_value: &request.display_value,
        normalized_value_json: None,
        value_type: &request.value_type.unwrap_or_else(|| "scalar".to_owned()),
        confidence,
        status: "active",
        valid_from: Some(valid_from_now.as_str()),
        valid_to: None,
        source_assertion_id: request.source_assertion_id.as_deref(),
        source_episode_ids_json: request.source_episode_ids_json.as_deref(),
    };
    metadata
        .insert_fact(&record)
        .map_err(AppError::from_error)?;

    append_audit(
        &state,
        "fact.create",
        Some(&format!("fact:{}", request.id)),
        Some("{\"result\":\"ok\"}"),
    );

    Ok(Json(serde_json::json!({
        "ok": true,
        "id": request.id,
    })))
}

pub(super) async fn supersede_fact(
    State(state): State<Arc<AppState>>,
    Path(fact_id): Path<String>,
    Json(request): Json<SupersedeFactRequest>,
) -> HandlerResult<Json<serde_json::Value>> {
    let metadata = state.metadata.clone();
    let valid_to = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    metadata
        .supersede_fact(&fact_id, &request.new_fact_id, &valid_to)
        .map_err(AppError::from_error)?;

    append_audit(
        &state,
        "fact.supersede",
        Some(&format!("fact:{}", fact_id)),
        Some("{\"result\":\"ok\"}"),
    );

    Ok(Json(serde_json::json!({
        "ok": true,
        "superseded": fact_id,
        "superseded_by": request.new_fact_id,
    })))
}

pub(super) async fn retract_fact(
    State(state): State<Arc<AppState>>,
    Path(fact_id): Path<String>,
) -> HandlerResult<Json<serde_json::Value>> {
    let metadata = state.metadata.clone();
    let valid_to = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    metadata
        .retract_fact(&fact_id, &valid_to)
        .map_err(AppError::from_error)?;

    append_audit(
        &state,
        "fact.retract",
        Some(&format!("fact:{}", fact_id)),
        Some("{\"result\":\"ok\"}"),
    );

    Ok(Json(serde_json::json!({
        "ok": true,
        "retracted": fact_id,
    })))
}

/// Trace a fact's provenance — find the source episodes and related assertions.
/// Returns the fact itself, the source episodes (if `source_episode_ids_json` is populated),
/// and the assertions that were extracted from those episodes.
pub(super) async fn trace_fact(
    State(state): State<Arc<AppState>>,
    Path(fact_id): Path<String>,
) -> HandlerResult<Json<serde_json::Value>> {
    let metadata = state.metadata.clone();

    // 1. Get the fact itself
    let fact = metadata.get_fact(&fact_id).map_err(AppError::from_error)?;
    let fact = match fact {
        Some(f) => f,
        None => {
            return Err(AppError(MfsError::NotFound {
                resource: format!("fact:{}", fact_id),
            }));
        }
    };

    // 2. Parse source_episode_ids_json into a Vec<String>
    let source_ep_ids: Vec<String> = fact
        .source_episode_ids_json
        .as_deref()
        .and_then(|j| serde_json::from_str::<Vec<String>>(j).ok())
        .unwrap_or_default();

    // 3. Trace to source episodes — look up each episode ID
    let source_episodes: Vec<serde_json::Value> = source_ep_ids
        .iter()
        .filter_map(|ep_id| {
            metadata.get_episode(ep_id).ok().flatten().map(|ep| {
                serde_json::json!({
                    "episode_id": ep.episode_id,
                    "session_id": ep.session_id,
                    "summary": ep.summary,
                    "created_at": ep.created_at,
                    "salience_score": ep.salience_score,
                })
            })
        })
        .collect();

    // 4. Get assertions from each source episode
    let mut assertions: Vec<serde_json::Value> = Vec::new();
    for ep_id in &source_ep_ids {
        let ep_assertions = metadata
            .get_assertions_by_source(None, Some(ep_id))
            .map_err(AppError::from_error)?;
        for a in ep_assertions {
            assertions.push(serde_json::json!({
                "assertion_id": a.assertion_id,
                "subject": a.subject,
                "predicate": a.predicate,
                "operation": a.operation,
                "confidence": a.confidence,
                "created_at": a.created_at,
            }));
        }
    }

    append_audit(
        &state,
        "fact.trace",
        Some(&format!("fact:{}", fact_id)),
        Some("{\"result\":\"ok\"}"),
    );

    Ok(Json(serde_json::json!({
        "fact": {
            "id": fact.id,
            "subject": fact.subject,
            "predicate": fact.predicate,
            "display_value": fact.display_value,
            "confidence": fact.confidence,
            "status": fact.status,
            "valid_from": fact.valid_from,
            "valid_to": fact.valid_to,
            "source_episode_ids_json": fact.source_episode_ids_json,
            "source_assertion_id": fact.source_assertion_id,
            "created_at": fact.created_at,
        },
        "source_episodes": source_episodes,
        "source_assertions": assertions,
        "assertion_count": assertions.len(),
    })))
}
