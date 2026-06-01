use super::*;
use mfs_memory::heuristics::{build_deterministic_prediction, build_simulate_reaction_prompt};

// ── Heuristic Rules CRUD ──────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(super) struct CreateHeuristicRuleRequest {
    pub rule_text: String,
    pub tags: Vec<String>,
    pub counter_examples: Option<Vec<String>>,
    pub lifecycle_stage: Option<String>,
    pub user_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct CreateHeuristicRuleResponse {
    pub rule_id: String,
    pub lifecycle_stage: String,
}

pub(super) async fn create_heuristic_rule(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateHeuristicRuleRequest>,
) -> HandlerResult<Json<CreateHeuristicRuleResponse>> {
    let account_id = state.config.account_id.clone();
    let user_id = req
        .user_id
        .clone()
        .unwrap_or_else(|| state.config.user_id.clone());

    if req.rule_text.is_empty() {
        return Err(AppError(MfsError::InvalidArgument {
            field: "rule_text".into(),
            reason: "rule_text must not be empty".into(),
        }));
    }

    // Validate and filter tags (roadmap §10.1)
    let valid_tags = mfs_memory::heuristics::validate_tags(&req.tags);

    let rule_id = mfs_uri::short_hash_hex(
        format!("{}:{}:{}", account_id, user_id, req.rule_text).as_bytes(),
        12,
    );
    let tags_json = serde_json::to_string(&valid_tags).unwrap_or_else(|_| "[]".to_owned());
    let counter_examples_json = serde_json::to_string(&req.counter_examples.unwrap_or_default())
        .unwrap_or_else(|_| "[]".to_owned());
    let lifecycle_stage = req.lifecycle_stage.as_deref().unwrap_or("draft");

    state
        .metadata
        .insert_heuristic_rule(&mfs_metadata::HeuristicRuleRecord {
            rule_id: &rule_id,
            account_id: &account_id,
            user_id: &user_id,
            agent_id: Some(&state.config.agent_id),
            tags_json: &tags_json,
            rule_text: &req.rule_text,
            counter_examples_json: &counter_examples_json,
            lifecycle_stage,
            evidence_count: 0,
            aggregate_weight: 0.0,
            last_evidence_at: None,
            source_instance_ids_json: None,
            promoted_at: None,
            user_confirmed: false,
        })
        .map_err(AppError::from_error)?;

    append_audit(
        &state,
        "create_heuristic_rule",
        Some(&rule_id),
        Some("{\"result\":\"ok\"}"),
    );

    Ok(Json(CreateHeuristicRuleResponse {
        rule_id,
        lifecycle_stage: lifecycle_stage.to_owned(),
    }))
}

#[derive(Debug, Serialize)]
pub(super) struct ListHeuristicRulesResponse {
    pub items: Vec<mfs_memory::heuristics::HeuristicEntry>,
    pub rules: Vec<mfs_memory::heuristics::HeuristicEntry>,
    pub next_cursor: Option<String>,
    pub total_count: usize,
    pub total: usize,
    pub limit: usize,
}

pub(super) async fn list_heuristic_rules(
    State(state): State<Arc<AppState>>,
    Query(query): Query<HashMap<String, String>>,
) -> HandlerResult<Json<ListHeuristicRulesResponse>> {
    let account_id = state.config.account_id.clone();
    let user_id = state.config.user_id.clone();
    let limit = query
        .get("limit")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(100);

    let stored_rules = state
        .metadata
        .list_heuristic_rules(&account_id, &user_id)
        .map_err(AppError::from_error)?;

    let rules: Vec<mfs_memory::heuristics::HeuristicEntry> =
        stored_rules.into_iter().map(|r| r.into()).collect();

    let total_count = rules.len();
    let items: Vec<_> = rules.into_iter().take(limit).collect();
    Ok(Json(ListHeuristicRulesResponse {
        items: items.clone(),
        rules: items,
        next_cursor: None,
        total_count,
        total: total_count,
        limit,
    }))
}

#[derive(Debug, Serialize)]
pub(super) struct GetHeuristicRuleResponse {
    pub rule_id: String,
    pub rule_text: String,
    pub tags: Vec<String>,
    pub counter_examples: Vec<String>,
    pub lifecycle_stage: String,
    pub evidence_count: i64,
    pub aggregate_weight: f64,
    pub created_at: String,
}

pub(super) async fn get_heuristic_rule(
    State(state): State<Arc<AppState>>,
    Path(rule_id): Path<String>,
) -> HandlerResult<Json<GetHeuristicRuleResponse>> {
    let rule = state
        .metadata
        .get_heuristic_rule(&rule_id)
        .map_err(AppError::from_error)?
        .ok_or_else(|| {
            AppError(MfsError::NotFound {
                resource: format!("heuristic_rule:{rule_id}"),
            })
        })?;

    Ok(Json(GetHeuristicRuleResponse {
        rule_id: rule.rule_id,
        rule_text: rule.rule_text,
        tags: serde_json::from_str(&rule.tags_json).unwrap_or_default(),
        counter_examples: serde_json::from_str(&rule.counter_examples_json).unwrap_or_default(),
        lifecycle_stage: rule.lifecycle_stage,
        evidence_count: rule.evidence_count,
        aggregate_weight: rule.aggregate_weight,
        created_at: rule.created_at,
    }))
}

#[derive(Debug, Deserialize)]
pub(super) struct PromoteRuleRequest {
    pub new_stage: String,
}

pub(super) async fn promote_heuristic_rule(
    State(state): State<Arc<AppState>>,
    Path(rule_id): Path<String>,
    Json(req): Json<PromoteRuleRequest>,
) -> HandlerResult<Json<serde_json::Value>> {
    let valid_stages = ["draft", "candidate", "confirmed", "archived"];
    if !valid_stages.contains(&req.new_stage.as_str()) {
        return Err(AppError(MfsError::InvalidArgument {
            field: "new_stage".into(),
            reason: format!(
                "lifecycle stage must be one of: {}",
                valid_stages.join(", ")
            ),
        }));
    }

    state
        .metadata
        .update_rule_lifecycle(&rule_id, &req.new_stage)
        .map_err(AppError::from_error)?;

    append_audit(
        &state,
        "promote_heuristic_rule",
        Some(&rule_id),
        Some(&format!("{{\"new_stage\":\"{}\"}}", req.new_stage)),
    );

    Ok(Json(serde_json::json!({
        "rule_id": rule_id,
        "new_stage": req.new_stage,
        "status": "updated"
    })))
}

// ── Confirm Rule (roadmap §5.4) ──────────────────────────────────────

/// Mark a rule as user-confirmed. User-confirmed rules are exempt from
/// automatic decay, distinct from lifecycle_stage 'confirmed' which is
/// reached via auto-promotion.
pub(super) async fn confirm_heuristic_rule(
    State(state): State<Arc<AppState>>,
    Path(rule_id): Path<String>,
) -> HandlerResult<Json<serde_json::Value>> {
    let account_id = &state.config.account_id;
    let user_id = &state.config.user_id;

    let updated = state
        .metadata
        .confirm_heuristic_rule(&rule_id, account_id, user_id);
    if !updated {
        return Err(AppError(MfsError::NotFound {
            resource: format!("heuristic_rule:{rule_id}"),
        }));
    }

    append_audit(
        &state,
        "confirm_heuristic_rule",
        Some(&rule_id),
        Some("{\"user_confirmed\":true}"),
    );

    Ok(Json(serde_json::json!({
        "rule_id": rule_id,
        "user_confirmed": true,
        "status": "confirmed"
    })))
}

// ── Heuristic Instances CRUD ──────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(super) struct CreateHeuristicInstanceRequest {
    pub context_summary: String,
    pub user_reaction: String,
    pub signal_type: String,
    pub tags: Option<Vec<String>>,
    pub agent_proposal: Option<String>,
    pub outcome: Option<String>,
    pub session_id: Option<String>,
    pub user_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct CreateHeuristicInstanceResponse {
    pub instance_id: String,
    pub signal_type: String,
}

pub(super) async fn create_heuristic_instance(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateHeuristicInstanceRequest>,
) -> HandlerResult<Json<CreateHeuristicInstanceResponse>> {
    let account_id = state.config.account_id.clone();
    let user_id = req
        .user_id
        .clone()
        .unwrap_or_else(|| state.config.user_id.clone());

    if req.context_summary.is_empty() {
        return Err(AppError(MfsError::InvalidArgument {
            field: "context_summary".into(),
            reason: "context_summary must not be empty".into(),
        }));
    }

    let valid_signal_types = [
        "explicit_negation",
        "implicit_negation",
        "preference_declaration",
        "tradeoff_decision",
    ];
    if !valid_signal_types.contains(&req.signal_type.as_str()) {
        return Err(AppError(MfsError::InvalidArgument {
            field: "signal_type".into(),
            reason: format!(
                "signal_type must be one of: {}",
                valid_signal_types.join(", ")
            ),
        }));
    }

    let valid_tags = mfs_memory::heuristics::validate_tags(&req.tags.unwrap_or_default());
    let instance_id = mfs_uri::short_hash_hex(
        format!(
            "{}:{}:{}:{}",
            account_id, user_id, req.signal_type, req.user_reaction
        )
        .as_bytes(),
        12,
    );

    state
        .metadata
        .insert_heuristic_instance(&mfs_metadata::HeuristicInstanceRecord {
            instance_id: &instance_id,
            account_id: &account_id,
            user_id: &user_id,
            agent_id: Some(&state.config.agent_id),
            context_summary: &req.context_summary,
            agent_proposal: req.agent_proposal.as_deref(),
            user_reaction: &req.user_reaction,
            outcome: req.outcome.as_deref(),
            signal_type: &req.signal_type,
            tags_json: &serde_json::to_string(&valid_tags).unwrap_or_else(|_| "[]".to_owned()),
            session_id: req.session_id.as_deref(),
            source_turn_ids_json: None,
            derived_rule_id: None,
            instance_status: "open",
            resolved_at: None,
        })
        .map_err(AppError::from_error)?;

    append_audit(
        &state,
        "create_heuristic_instance",
        Some(&instance_id),
        Some("{\"result\":\"ok\"}"),
    );

    Ok(Json(CreateHeuristicInstanceResponse {
        instance_id,
        signal_type: req.signal_type,
    }))
}

/// Get a single heuristic instance by ID.
pub(super) async fn get_heuristic_instance(
    State(state): State<Arc<AppState>>,
    Path(instance_id): Path<String>,
) -> HandlerResult<Json<serde_json::Value>> {
    let stored = state
        .metadata
        .get_heuristic_instance(&instance_id)
        .map_err(AppError::from_error)?;

    let Some(inst) = stored else {
        return Err(AppError(MfsError::NotFound {
            resource: format!("heuristic_instance/{}", instance_id),
        }));
    };

    Ok(Json(serde_json::json!({
        "instance_id": inst.instance_id,
        "context_summary": inst.context_summary,
        "agent_proposal": inst.agent_proposal,
        "user_reaction": inst.user_reaction,
        "outcome": inst.outcome,
        "signal_type": inst.signal_type,
        "tags": serde_json::from_str::<Vec<String>>(&inst.tags_json).unwrap_or_default(),
        "session_id": inst.session_id,
        "instance_status": inst.instance_status,
        "derived_rule_id": inst.derived_rule_id,
        "created_at": inst.created_at,
    })))
}

#[derive(Debug, Serialize)]
pub(super) struct ListHeuristicInstancesResponse {
    pub items: Vec<serde_json::Value>,
    pub instances: Vec<serde_json::Value>,
    pub next_cursor: Option<String>,
    pub total_count: usize,
    pub total: usize,
    pub limit: usize,
}

pub(super) async fn list_heuristic_instances(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> HandlerResult<Json<ListHeuristicInstancesResponse>> {
    let account_id = state.config.account_id.clone();
    let user_id = params
        .get("user_id")
        .cloned()
        .unwrap_or_else(|| state.config.user_id.clone());
    let status_filter = params.get("status").map(|s| s.as_str());
    let limit = params
        .get("limit")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(100);

    let stored = state
        .metadata
        .list_heuristic_instances(&account_id, &user_id, status_filter)
        .map_err(AppError::from_error)?;

    let instances: Vec<serde_json::Value> = stored
        .into_iter()
        .map(|i| {
            serde_json::json!({
                "instance_id": i.instance_id,
                "context_summary": i.context_summary,
                "user_reaction": i.user_reaction,
                "signal_type": i.signal_type,
                "tags": serde_json::from_str::<Vec<String>>(&i.tags_json).unwrap_or_default(),
                "instance_status": i.instance_status,
                "created_at": i.created_at,
            })
        })
        .collect();

    let total_count = instances.len();
    let items: Vec<_> = instances.into_iter().take(limit).collect();
    Ok(Json(ListHeuristicInstancesResponse {
        items: items.clone(),
        instances: items,
        next_cursor: None,
        total_count,
        total: total_count,
        limit,
    }))
}

// ── Heuristic Retrieval ───────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(super) struct RetrieveHeuristicsRequest {
    pub query: String,
    pub tags: Option<Vec<String>>,
    pub top_k: Option<usize>,
    pub user_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct RetrieveHeuristicsResponse {
    pub heuristics: Vec<mfs_memory::heuristics::HeuristicEntry>,
    pub total: usize,
}

pub(super) async fn retrieve_heuristics_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RetrieveHeuristicsRequest>,
) -> HandlerResult<Json<RetrieveHeuristicsResponse>> {
    if req.query.is_empty() {
        return Err(AppError(MfsError::InvalidArgument {
            field: "query".into(),
            reason: "query must not be empty".into(),
        }));
    }

    let account_id = state.config.account_id.clone();
    let user_id = req
        .user_id
        .clone()
        .unwrap_or_else(|| state.config.user_id.clone());
    let valid_tags = mfs_memory::heuristics::validate_tags(&req.tags.unwrap_or_default());
    let top_k = req.top_k.unwrap_or(10);

    let heuristics = retrieve_heuristics(
        &state.metadata,
        &account_id,
        &user_id,
        &valid_tags,
        &req.query,
        top_k,
    );

    let total = heuristics.len();
    Ok(Json(RetrieveHeuristicsResponse { heuristics, total }))
}

// ── L0 Confirmed Rules ────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(super) struct L0ConfirmedRequest {
    pub max_rules: Option<usize>,
    pub user_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct L0ConfirmedResponse {
    pub rules: Vec<mfs_memory::heuristics::HeuristicEntry>,
    pub total: usize,
}

pub(super) async fn l0_confirmed_rules(
    State(state): State<Arc<AppState>>,
    Json(req): Json<L0ConfirmedRequest>,
) -> HandlerResult<Json<L0ConfirmedResponse>> {
    let account_id = state.config.account_id.clone();
    let user_id = req
        .user_id
        .clone()
        .unwrap_or_else(|| state.config.user_id.clone());
    let max_rules = req.max_rules.unwrap_or(5);

    let rules = retrieve_l0_confirmed(&state.metadata, &account_id, &user_id, max_rules);

    let total = rules.len();
    Ok(Json(L0ConfirmedResponse { rules, total }))
}

// ── Simulate Reaction (L2 injection) ──────────────────────────────────

#[derive(Debug, Deserialize)]
pub(super) struct SimulateReactionRequest {
    pub scenario: String,
    pub tags: Option<Vec<String>>,
    pub user_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct SimulateReactionResponse {
    pub scenario: String,
    pub relevant_rules: Vec<mfs_memory::heuristics::HeuristicEntry>,
    pub rules_summary: String,
    pub prediction: String,
}

pub(super) async fn simulate_reaction_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SimulateReactionRequest>,
) -> HandlerResult<Json<SimulateReactionResponse>> {
    if req.scenario.is_empty() {
        return Err(AppError(MfsError::InvalidArgument {
            field: "scenario".into(),
            reason: "scenario must not be empty".into(),
        }));
    }

    let account_id = state.config.account_id.clone();
    let user_id = req
        .user_id
        .clone()
        .unwrap_or_else(|| state.config.user_id.clone());
    let valid_tags = mfs_memory::heuristics::validate_tags(&req.tags.unwrap_or_default());

    let entries = retrieve_heuristics(
        &state.metadata,
        &account_id,
        &user_id,
        &valid_tags,
        &req.scenario,
        5,
    );

    let rules_summary = entries
        .iter()
        .map(|e| {
            let marker = match e.lifecycle_stage.as_str() {
                "confirmed" => "★",
                "candidate" => "◆",
                "draft" => "○",
                _ => "?",
            };
            format!("{marker} {}: {}", e.lifecycle_stage, e.rule_text)
        })
        .collect::<Vec<_>>()
        .join("\n");

    // LLM-enhanced prediction (roadmap §8 Phase 3): when LLM is available,
    // ask it to synthesize a prediction from the matched rules + scenario.
    // Falls back to a deterministic template when LLM is unavailable.
    let llm = LlmAssist::from_env();
    let prediction = if entries.is_empty() {
        "No relevant heuristic rules found for this scenario.".to_owned()
    } else if llm.is_available() {
        let prompt = build_simulate_reaction_prompt(&req.scenario, &entries);
        match llm.complete(&prompt).await {
            Some(response) => response,
            None => build_deterministic_prediction(&entries),
        }
    } else {
        build_deterministic_prediction(&entries)
    };

    Ok(Json(SimulateReactionResponse {
        scenario: req.scenario,
        relevant_rules: entries,
        rules_summary,
        prediction,
    }))
}
