use super::*;
use chrono::Utc;
use mfs_memory::FactEntry;
use mfs_memory::episodes::compute_hotness;
use mfs_memory::{compute_staleness_note, fact_is_procedural, format_noted_date, parse_timestamp};
use mfs_types::DecayConfig;
use mfs_types::MemoryType;

// ── Phase 2-3: Memory Archive ─────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(in crate::http) struct MemoryArchiveRequest {
    /// Hotness threshold below which episodes are archived (default: 0.1)
    pub hotness_threshold: Option<f64>,
    /// Minimum age in days before an episode can be archived (default: 30)
    pub min_age_days: Option<f64>,
}

#[derive(Debug, Serialize)]
pub(in crate::http) struct MemoryArchiveResponse {
    pub archived_episodes: usize,
    pub user_id: String,
}

/// Archive cold episodes whose hotness score falls below the threshold.
///
/// Hotness is computed by `mfs_memory::episodes::compute_hotness` — the same
/// formula used by `rerank_episodes` — so archive decisions are consistent with
/// retrieval ranking.  Episodes are only archived if they are older than
/// `min_age_days`.
///
/// Security: this endpoint operates on the server's configured `user_id` only.
/// Callers cannot specify a different user_id to access another user's data.
pub(in crate::http) async fn memory_archive(
    State(state): State<Arc<AppState>>,
    req: Json<MemoryArchiveRequest>,
) -> HandlerResult<Json<MemoryArchiveResponse>> {
    let account_id = state.config.account_id.clone();
    let user_id = state.config.user_id.clone();
    let hotness_threshold = req.hotness_threshold.unwrap_or(0.1);
    let min_age_days = req.min_age_days.unwrap_or(30.0);

    let metadata = state.metadata.clone();
    let episodes = metadata
        .get_episodes_by_user(&account_id, &user_id, None)
        .map_err(AppError::from_error)?;

    let now = Utc::now();
    let mut archived = 0usize;

    for ep in &episodes {
        // Skip already-archived episodes
        if ep.archived_at.is_some() {
            continue;
        }

        // Age check — parse both RFC3339 and SQLite CURRENT_TIMESTAMP formats
        let age_days = parse_timestamp(&ep.created_at)
            .map(|dt| (now - dt).num_seconds() as f64 / 86400.0)
            .unwrap_or(0.0);
        if age_days < min_age_days {
            continue;
        }

        // Hotness check — delegates to the canonical formula in mfs-memory
        let hotness = compute_hotness(
            ep.salience_score,
            ep.strength_score,
            ep.recall_count as usize,
            ep.emotional_intensity,
        );

        if hotness < hotness_threshold {
            let ts = now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
            if metadata.archive_episode(&ep.episode_id, &ts).is_ok() {
                archived += 1;
            }
        }
    }

    append_audit(
        &state,
        "memory_archive",
        Some(&format!("user_id={user_id}")),
        Some(&format!("{{\"archived\":{archived}}}")),
    );

    Ok(Json(MemoryArchiveResponse {
        archived_episodes: archived,
        user_id,
    }))
}

// ── Phase 2-4: Eval Recall ────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(in crate::http) struct EvalRecallRequest {
    pub query: String,
    pub expected_facts: Vec<String>,
    /// k for recall@k (default: 10)
    pub k: Option<usize>,
}

#[derive(Debug, Serialize)]
pub(in crate::http) struct EvalRecallResponse {
    pub recall_at_k: f64,
    pub retrieved_count: usize,
    pub expected_count: usize,
    pub matched_count: usize,
    pub missing_facts: Vec<String>,
}

/// Evaluate recall quality: given a query and expected facts, measure how many
/// expected facts appear in the top-k facts sorted by confidence.
///
/// Matching is case-insensitive substring: an expected fact is "found" if any
/// retrieved fact's display_value contains the expected string.
///
/// Security: this endpoint operates on the server's configured `user_id` only.
/// Callers cannot specify a different user_id to read another user's facts.
pub(in crate::http) async fn eval_recall(
    State(state): State<Arc<AppState>>,
    req: Json<EvalRecallRequest>,
) -> HandlerResult<Json<EvalRecallResponse>> {
    if req.query.is_empty() {
        return Err(AppError(MfsError::InvalidArgument {
            field: "query".into(),
            reason: "query must not be empty".into(),
        }));
    }

    let account_id = state.config.account_id.clone();
    let user_id = state.config.user_id.clone();
    let k = req.k.unwrap_or(10);

    let metadata = state.metadata.clone();
    let stored_facts = metadata
        .get_active_facts(&account_id, &user_id)
        .map_err(AppError::from_error)?;

    // Sort by confidence desc (same as resolve_context Phase 2-2)
    let mut fact_entries: Vec<FactEntry> = stored_facts
        .iter()
        .map(|f| {
            let staleness_note = compute_staleness_note(
                &f.created_at,
                f.last_recalled_at.as_deref(),
                f.recall_count,
                f.valid_from.as_deref(),
                &f.predicate,
            );
            // §10.2.2: Append noted-date to procedure/convention/environment facts
            let display_value = if fact_is_procedural(&f.predicate) {
                let date_note = format_noted_date(&f.created_at);
                format!("{} ({})", f.display_value, date_note)
            } else {
                f.display_value.clone()
            };
            FactEntry {
                fact_id: f.id.clone(),
                predicate: f.predicate.clone(),
                display_value,
                confidence: f.confidence,
                staleness_note,
                valid_from: f.valid_from.clone(),
            }
        })
        .collect();
    fact_entries.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let top_k_facts: Vec<&FactEntry> = fact_entries.iter().take(k).collect();

    // Check which expected facts are covered
    let mut matched = 0usize;
    let mut missing = Vec::new();
    for expected in &req.expected_facts {
        let expected_lower = expected.to_lowercase();
        let found = top_k_facts.iter().any(|f| {
            f.display_value.to_lowercase().contains(&expected_lower)
                || f.predicate.to_lowercase().contains(&expected_lower)
        });
        if found {
            matched += 1;
        } else {
            missing.push(expected.clone());
        }
    }

    let expected_count = req.expected_facts.len();
    let recall = if expected_count == 0 {
        1.0
    } else {
        matched as f64 / expected_count as f64
    };

    Ok(Json(EvalRecallResponse {
        recall_at_k: recall,
        retrieved_count: top_k_facts.len(),
        expected_count,
        matched_count: matched,
        missing_facts: missing,
    }))
}

// ── Memory Export/Import (P0-2) ───────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(in crate::http) struct MemoryExportQuery {
    pub user_id: Option<String>,
}

/// Export active facts and confirmed heuristic rules as editable Markdown.
pub(in crate::http) async fn memory_export(
    State(state): State<Arc<AppState>>,
    Query(query): Query<MemoryExportQuery>,
) -> HandlerResult<Json<serde_json::Value>> {
    let user_id = query.user_id.as_deref().unwrap_or("default");
    let account_id = "default";
    let metadata = state.metadata.clone();

    let facts = metadata
        .get_active_facts(account_id, user_id)
        .map_err(AppError::from_error)?;

    let rules = metadata
        .get_active_heuristic_rules(account_id, user_id, &["confirmed", "candidate"])
        .map_err(AppError::from_error)?;

    let mut md = String::from("# MemFuse Memory Export\n\n");
    md.push_str(&format!(
        "> Exported at: {}  \n> User: {}\n\n",
        Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        user_id
    ));

    // Facts section
    md.push_str("## Facts\n\n");
    md.push_str("<!-- Edit values or delete rows. Re-import to apply changes. -->\n\n");
    if facts.is_empty() {
        md.push_str("_No active facts._\n\n");
    } else {
        md.push_str("| ID | Predicate | Value | Confidence |\n");
        md.push_str("|---|---|---|---|\n");
        for f in &facts {
            md.push_str(&format!(
                "| `{}` | {} | {} | {:.2} |\n",
                f.id, f.predicate, f.display_value, f.confidence
            ));
        }
        md.push('\n');
    }

    // Heuristic rules section
    md.push_str("## Heuristic Rules\n\n");
    md.push_str("<!-- Edit rule_text or delete rows. Re-import to apply changes. -->\n\n");
    if rules.is_empty() {
        md.push_str("_No active heuristic rules._\n\n");
    } else {
        for r in &rules {
            let tags: Vec<String> = serde_json::from_str(&r.tags_json).unwrap_or_default();
            let confirmed = if r.user_confirmed { " ★" } else { "" };
            md.push_str(&format!(
                "### `{}`{}\n\n- **Tags**: {}\n- **Stage**: {}\n- **Rule**: {}\n\n",
                r.rule_id,
                confirmed,
                tags.join(", "),
                r.lifecycle_stage,
                r.rule_text,
            ));
        }
    }

    Ok(Json(serde_json::json!({
        "markdown": md,
        "fact_count": facts.len(),
        "rule_count": rules.len(),
    })))
}

#[derive(Debug, Deserialize)]
pub(in crate::http) struct MemoryImportRequest {
    pub markdown: String,
    pub user_id: Option<String>,
}

/// Import facts from Markdown. Supports updating display_value/confidence and retracting deleted facts.
pub(in crate::http) async fn memory_import(
    State(state): State<Arc<AppState>>,
    Json(request): Json<MemoryImportRequest>,
) -> HandlerResult<Json<serde_json::Value>> {
    let user_id = request.user_id.as_deref().unwrap_or("default");
    let account_id = "default";
    let metadata = state.metadata.clone();

    let mut imported_fact_ids: Vec<String> = Vec::new();
    let mut updated = 0u32;
    let mut skipped = 0u32;
    let mut errors = 0u32;
    let mut retracted = 0u32;

    let existing_facts = metadata
        .get_active_facts(account_id, user_id)
        .map_err(AppError::from_error)?;

    for line in request.markdown.lines() {
        let line = line.trim();
        if !line.starts_with("| `") || line.contains("---") {
            continue;
        }
        let cols: Vec<&str> = line.split('|').map(|s| s.trim()).collect();
        if cols.len() < 5 {
            skipped += 1;
            continue;
        }
        let fact_id = cols[1].trim_matches('`').trim();
        let new_value = cols[3].trim();
        let new_confidence: f64 = match cols[4].trim().parse() {
            Ok(v) if v >= 0.0 => v,
            _ => {
                skipped += 1;
                continue;
            }
        };
        if fact_id.is_empty() {
            skipped += 1;
            continue;
        }
        imported_fact_ids.push(fact_id.to_string());

        // Ownership check: only update facts belonging to this user
        if let Some(existing) = existing_facts.iter().find(|f| f.id == fact_id) {
            if existing.user_id != user_id {
                skipped += 1;
                continue;
            }
            if existing.display_value != new_value
                || (existing.confidence - new_confidence).abs() > 0.01
            {
                match metadata.update_fact_value(fact_id, new_value, new_confidence) {
                    Ok(rows) if rows > 0 => updated += 1,
                    Ok(_) => skipped += 1, // fact not active
                    Err(_) => errors += 1,
                }
            }
        }
    }

    // Safe retraction: only retract if the import parsed at least one fact
    // (prevents accidental mass-retraction from empty/malformed markdown)
    if !imported_fact_ids.is_empty() {
        for f in &existing_facts {
            if f.user_id == user_id && !imported_fact_ids.contains(&f.id) {
                match metadata.retract_fact(
                    &f.id,
                    &chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                ) {
                    Ok(()) => retracted += 1,
                    Err(_) => errors += 1,
                }
            }
        }
    }

    Ok(Json(serde_json::json!({
        "updated_facts": updated,
        "retracted_facts": retracted,
        "skipped_rows": skipped,
        "errors": errors,
        "total_imported": imported_fact_ids.len(),
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staleness_note_validity_period_old_preference() {
        // A valid_from > 7 days ago should produce predicate-category-aware validity note
        let old_valid_from = chrono::Utc::now()
            .checked_sub_signed(chrono::Duration::days(14))
            .unwrap()
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let note = compute_staleness_note(
            &old_valid_from,
            None,
            1,
            Some(old_valid_from.as_str()),
            "preference.color",
        );
        assert!(note.is_some());
        let note = note.unwrap();
        assert!(note.contains("preference valid for"));
    }

    #[test]
    fn staleness_note_validity_period_old_procedure() {
        // Procedure predicates should use "procedure" label, not "preference"
        let old_valid_from = chrono::Utc::now()
            .checked_sub_signed(chrono::Duration::days(14))
            .unwrap()
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let note = compute_staleness_note(
            &old_valid_from,
            None,
            1,
            Some(old_valid_from.as_str()),
            "procedure.build_command",
        );
        assert!(note.is_some());
        let note = note.unwrap();
        assert!(note.contains("procedure valid for"));
        assert!(!note.contains("preference"));
    }

    #[test]
    fn staleness_note_validity_period_fresh_preference() {
        // A valid_from <= 1 day ago should produce "fresh preference"
        let recent_valid_from = chrono::Utc::now()
            .checked_sub_signed(chrono::Duration::hours(6))
            .unwrap()
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let note = compute_staleness_note(
            &recent_valid_from,
            None,
            0,
            Some(recent_valid_from.as_str()),
            "preference.color",
        );
        assert!(note.is_some());
        let note = note.unwrap();
        assert!(note.contains("fresh preference"));
    }

    #[test]
    fn staleness_note_timestamp_parsing_handles_rfc3339() {
        let created_at = "2026-01-01T00:00:00Z";
        let note = compute_staleness_note(created_at, None, 0, None, "other.info");
        assert!(note.is_some());
        // Should parse and compute days_old, not return "unparseable"
        let note = note.unwrap();
        assert!(!note.contains("unparseable"));
    }

    #[test]
    fn staleness_note_timestamp_parsing_handles_sqlite_format() {
        let created_at = "2026-01-01 00:00:00";
        let note = compute_staleness_note(created_at, None, 0, None, "other.info");
        assert!(note.is_some());
        // Should parse SQLite format, not return "unparseable"
        let note = note.unwrap();
        assert!(!note.contains("unparseable"));
    }

    #[test]
    fn staleness_note_no_valid_from() {
        // When valid_from is None, no validity annotation should appear
        let created_at = "2026-01-01T00:00:00Z";
        let note = compute_staleness_note(created_at, None, 0, None, "preference.color");
        assert!(note.is_some());
        let note = note.unwrap();
        assert!(!note.contains("valid for"));
        assert!(!note.contains("fresh"));
    }
}

// ─── Retention Scores (Ebbinghaus) ──────────────────────────────────

/// Compute and return Ebbinghaus retention scores for all memories of a given type.
#[derive(Debug, Deserialize)]
pub(in crate::http) struct RetentionScoresQuery {
    /// memory_type filter: "episode", "fact", or "all" (default)
    pub memory_type: Option<String>,
}

pub(in crate::http) async fn retention_scores(
    State(state): State<Arc<AppState>>,
    Query(query): Query<RetentionScoresQuery>,
) -> HandlerResult<Json<serde_json::Value>> {
    // Derive account_id/user_id from server config only — no tenant override
    // from request parameters to prevent cross-tenant data access.
    let account_id = &state.config.account_id;
    let user_id = &state.config.user_id;
    let config = DecayConfig::default();
    let now = chrono::Utc::now();
    let metadata = state.metadata.clone();

    let mut scores = Vec::new();

    // Episodes
    if query.memory_type.as_deref().unwrap_or("all") == "all"
        || query.memory_type.as_deref() == Some("episode")
    {
        let episodes = metadata
            .get_episodes_by_user(account_id, user_id, None)
            .map_err(|e| MfsError::Internal {
                message: e.to_string(),
            })?;

        // Batch-retrieve access history (N+1 → single query)
        let active_ep_ids: Vec<String> = episodes
            .iter()
            .filter(|ep| ep.archived_at.is_none())
            .map(|ep| ep.episode_id.clone())
            .collect();
        let ep_access_batch = metadata
            .get_access_days_since_batch(&active_ep_ids, &now)
            .unwrap_or_default();

        for ep in &episodes {
            if ep.archived_at.is_some() {
                continue;
            }
            // Skip episodes with unparseable timestamps (consistent with
            // decay_episode_salience which also skips+warns)
            let days_since_creation = match parse_timestamp(&ep.created_at) {
                Some(c) => ((now - c).num_seconds() as f64) / 86400.0,
                None => {
                    scores.push(serde_json::json!({
                        "memory_id": ep.episode_id,
                        "memory_type": "episode",
                        "error": "unparseable created_at timestamp",
                        "retention_score": null,
                    }));
                    continue;
                }
            };

            let access_days = ep_access_batch
                .get(&ep.episode_id)
                .cloned()
                .unwrap_or_default();

            let score = config.compute_retention(
                MemoryType::Episode,
                ep.salience_score,
                days_since_creation,
                &access_days,
            );

            scores.push(serde_json::json!({
                "memory_id": ep.episode_id,
                "memory_type": "episode",
                "salience": ep.salience_score,
                "retention_score": score,
                "tier": config.classify_tier(score),
                "days_since_creation": days_since_creation,
                "access_count": access_days.len(),
            }));
        }
    }

    // Facts
    if query.memory_type.as_deref().unwrap_or("all") == "all"
        || query.memory_type.as_deref() == Some("fact")
    {
        let facts = metadata
            .get_active_facts(account_id, user_id)
            .map_err(|e| MfsError::Internal {
                message: e.to_string(),
            })?;

        // Batch-retrieve access history (N+1 → single query)
        let fact_ids: Vec<String> = facts.iter().map(|f| f.id.clone()).collect();
        let fact_access_batch = metadata
            .get_access_days_since_batch(&fact_ids, &now)
            .unwrap_or_default();

        for fact in &facts {
            // Skip facts with unparseable timestamps (consistent with
            // expire_stale_facts which also skips+warns)
            let days_since_creation = match parse_timestamp(&fact.created_at) {
                Some(c) => ((now - c).num_seconds() as f64) / 86400.0,
                None => {
                    scores.push(serde_json::json!({
                        "memory_id": fact.id,
                        "memory_type": "fact",
                        "error": "unparseable created_at timestamp",
                        "retention_score": null,
                    }));
                    continue;
                }
            };

            let access_days = fact_access_batch.get(&fact.id).cloned().unwrap_or_default();

            let score = config.compute_retention(
                MemoryType::Fact,
                fact.confidence,
                days_since_creation,
                &access_days,
            );

            scores.push(serde_json::json!({
                "memory_id": fact.id,
                "memory_type": "fact",
                "salience": fact.confidence,
                "retention_score": score,
                "tier": config.classify_tier(score),
                "days_since_creation": days_since_creation,
                "access_count": access_days.len(),
            }));
        }
    }

    Ok(Json(serde_json::json!({
        "scores": scores,
        "config": {
            "sigma": config.sigma,
            "lambda_episode": config.lambda_for(MemoryType::Episode),
            "lambda_fact": config.lambda_for(MemoryType::Fact),
            "lambda_heuristic": config.lambda_for(MemoryType::Heuristic),
            "tier_thresholds": {
                "hot": config.tier_hot,
                "warm": config.tier_warm,
                "cold": config.tier_cold,
            },
        },
    })))
}

// ─── Temporal Graph: AS OF query on relations ───────────────────────

#[derive(Debug, Deserialize)]
pub(in crate::http) struct RelationsTemporalQueryRequest {
    /// ISO 8601 timestamp for point-in-time query.
    /// Returns relations that were valid at this moment.
    /// If omitted, returns currently valid relations (is_latest=1).
    pub as_of: Option<String>,
    /// Optional filter by relation_type.
    pub relation_type: Option<String>,
    /// Optional filter by from_uri prefix.
    pub from_uri_prefix: Option<String>,
}

pub(in crate::http) async fn relations_temporal_query(
    State(state): State<Arc<AppState>>,
    Json(request): Json<RelationsTemporalQueryRequest>,
) -> HandlerResult<Json<serde_json::Value>> {
    // Derive account_id/user_id from server config only — no tenant override
    // from request body to prevent cross-tenant data access.
    let account_id = &state.config.account_id;
    let user_id = &state.config.user_id;
    let metadata = state.metadata.clone();

    let relations = metadata
        .get_temporal_relations(
            account_id,
            user_id,
            request.as_of.as_deref(),
            request.relation_type.as_deref(),
            request.from_uri_prefix.as_deref(),
        )
        .map_err(|e| MfsError::Internal {
            message: e.to_string(),
        })?;

    let results: Vec<serde_json::Value> = relations
        .iter()
        .map(|r| {
            serde_json::json!({
                "id": r.id,
                "from_uri": r.from_uri,
                "to_uri": r.to_uri,
                "relation_type": r.relation_type,
                "valid_from": r.valid_from,
                "valid_to": r.valid_to,
                "tcommit": r.tcommit,
                "is_latest": r.is_latest,
                "superseded_by": r.superseded_by,
            })
        })
        .collect();

    Ok(Json(serde_json::json!({
        "relations": results,
        "as_of": request.as_of,
        "count": results.len(),
    })))
}
