//! MemoryService — unified facade for memory context resolution and search.
//!
//! Encapsulates the full "data fetch → transform → compute → render" pipeline
//! so that HTTP handlers only need to parse params → call service → format response.
//!
//! Previously this orchestration lived in mfs-server's memory.rs handler (250+ lines),
//! mixing MetadataStore calls, mfs-memory algorithm functions, and data transformations
//! inline. This module restores the "thin handler" principle by providing a single
//! entry point that handles all business logic.

use mfs_metadata::MetadataStore;
use mfs_semantic::{EmbeddingProvider, cosine_similarity, parse_embedding_json};

use crate::budget::{cap_episodes_by_budget, cap_facts_by_budget, plan_section_budgets};
use crate::episodes::rerank_episodes_with_query;
use crate::facts::filter_facts_for_injection;
use crate::heuristics::retrieve_heuristics;
use crate::intent::{classify_intent, classify_intent_keywords, route_facts_for_intent};
use crate::llm::LlmAssist;
use crate::overlay::build_overlay_entries;
use crate::render::render_memory_injection;
use crate::{
    ConversationTurn, DEFAULT_EPISODIC_TOP_K, EpisodeSummary, FactEntry, MemoryContextArtifacts,
    MemoryContextResponse, MemoryContextSections, SearchStrategy,
};

// ─── Input / Output types ────────────────────────────────────────────────────

/// Input parameters for `resolve_context`.
pub struct ResolveContextInput {
    pub account_id: String,
    pub user_id: String,
    pub resource_id: Option<String>,
    pub session_id: Option<String>,
    pub query: String,
    pub budget: usize,
    pub strategy: SearchStrategy,
    /// Point-in-time temporal query (ISO-8601 timestamp).
    pub at_time: Option<String>,
    /// Whether this recall should reinforce memory strength
    /// (increment recall_count / last_recalled_at and append access log).
    /// False for passive hook injections (recall_source = "auto") so the
    /// forgetting curve only strengthens on explicit retrieval.
    pub reinforce_recall: bool,
}

/// Output of `resolve_context` — everything the handler needs except
/// filesystem markdown memories (which remain handler-side I/O).
pub struct ResolveContextOutput {
    pub sections: MemoryContextSections,
    pub artifacts: MemoryContextArtifacts,
    pub detail_handles: Vec<String>,
    pub rendered_markdown: String,
}

// ─── Context resolution ──────────────────────────────────────────────────────

/// Resolve memory context for a given query.
///
/// This is the primary Agent read path — called by SessionStart/UserPromptSubmit/PreCompact hooks.
/// It orchestrates: overlay → facts → episodes → intent → budget → filter → rerank → heuristics → render.
///
/// The handler only needs to:
/// 1. Get conversation turns from session engine
/// 2. Call this function
/// 3. Optionally append filesystem markdown memories
/// 4. Write audit log
pub async fn resolve_context(
    metadata: &MetadataStore,
    input: &ResolveContextInput,
    conversation_turns: &[ConversationTurn],
    llm: &LlmAssist,
    llm_enabled: bool,
    embed_timeout_ms: u64,
    embedding_provider: &dyn EmbeddingProvider,
) -> Result<ResolveContextOutput, String> {
    let account_id = &input.account_id;
    let user_id = &input.user_id;
    let query = &input.query;
    let budget = input.budget;
    let strategy = input.strategy;

    // Step 4: Build overlay entries from conversation turns.
    let overlay_entries = build_overlay_entries(conversation_turns);

    // Step 5-6: Load and transform facts.
    let all_fact_entries =
        load_and_transform_facts(metadata, account_id, user_id, query, &input.at_time)
            .map_err(|e| e.to_string())?;

    // Step 7: Load episodes.
    let all_episode_summaries =
        load_and_transform_episodes(metadata, account_id, user_id, input.resource_id.as_deref())
            .map_err(|e| e.to_string())?;

    // Step 8: Intent classification — classify_intent always returns IntentResult (has keyword fallback).
    let intent = if strategy == SearchStrategy::Comprehensive || llm_enabled {
        classify_intent(query, llm).await
    } else {
        classify_intent_keywords(query)
    };

    // Step 9: Budget planning.
    let (fact_budget, episode_budget) = plan_section_budgets(
        budget,
        &overlay_entries,
        all_fact_entries.len(),
        all_episode_summaries.len(),
        strategy,
    );

    // Step 10: Filter and cap facts.
    let final_facts = if intent.matched_predicates.is_empty() {
        let filtered = filter_facts_for_injection(&all_fact_entries);
        cap_facts_by_budget(&filtered, fact_budget)
    } else {
        let routed = route_facts_for_intent(&all_fact_entries, query, &intent);
        cap_facts_by_budget(&routed, fact_budget)
    };

    // Step 11-12: Embed query and rerank episodes.
    let query_embedding =
        embed_query_with_timeout(query, embedding_provider, embed_timeout_ms, strategy).await;

    let final_episodes = if let Some(query_emb) = &query_embedding {
        // Compute similarity scores for each episode.
        let scores: Vec<(usize, f64)> = all_episode_summaries
            .iter()
            .enumerate()
            .filter_map(|(i, ep)| {
                let ep_emb = parse_embedding_json(&ep.embedding_json)?;
                let sim = cosine_similarity(query_emb, &ep_emb);
                Some((i, sim))
            })
            .collect();

        rerank_episodes_with_query(
            scores,
            &all_episode_summaries,
            DEFAULT_EPISODIC_TOP_K,
            Some(query),
            strategy,
        )
    } else {
        cap_episodes_by_budget(&all_episode_summaries, episode_budget)
    };

    // Step 13: Cross-thread detection.
    let is_cross_thread = intent.is_cross_thread;
    let detail_handles = if is_cross_thread {
        all_episode_summaries
            .iter()
            .take(DEFAULT_EPISODIC_TOP_K)
            .map(|ep| ep.episode_id.clone())
            .collect()
    } else {
        Vec::new()
    };

    // Step 14: Retrieve heuristics.
    let behavioral_heuristics =
        retrieve_heuristics(metadata, account_id, user_id, &Vec::new(), query, 10);

    // Assemble response.
    let sections = MemoryContextSections {
        current_facts: final_facts.clone(),
        recent_updates: overlay_entries,
        relevant_history: final_episodes.clone(),
        behavioral_heuristics,
    };
    let artifacts = MemoryContextArtifacts {
        cross_thread_briefs: Vec::new(),
    };

    // Step 15: Render markdown.
    let context_response = MemoryContextResponse {
        sections: sections.clone(),
        artifacts: artifacts.clone(),
        detail_handles: detail_handles.clone(),
    };
    let rendered_markdown = render_memory_injection(&context_response);

    // Step 16: Write recall statistics and access log — only for explicit
    // recalls. Passive hook injections (every prompt) must not inflate
    // reinforcement, or decay semantics become meaningless.
    if input.reinforce_recall {
        writeback_recall_and_access_log(
            metadata,
            &final_facts,
            &final_episodes,
            account_id,
            user_id,
        )
        .map_err(|e| e.to_string())?;
    }

    Ok(ResolveContextOutput {
        sections,
        artifacts,
        detail_handles,
        rendered_markdown,
    })
}

// ─── Internal helpers (previously in handler) ────────────────────────────────

/// Load facts from MetadataStore and transform to FactEntry with staleness notes.
fn load_and_transform_facts(
    metadata: &MetadataStore,
    account_id: &str,
    user_id: &str,
    query: &str,
    at_time: &Option<String>,
) -> rusqlite::Result<Vec<FactEntry>> {
    // Fetch facts.
    let stored_facts = if let Some(at) = at_time {
        metadata.get_facts_at_time(account_id, user_id, at)?
    } else {
        metadata.get_active_facts(account_id, user_id)?
    };

    // Transform StoredFact → FactEntry.
    let mut entries: Vec<FactEntry> = stored_facts
        .into_iter()
        .map(|f| {
            let staleness_note = compute_staleness_note(
                &f.created_at,
                f.last_recalled_at.as_deref(),
                f.recall_count,
                f.valid_from.as_deref(),
                &f.predicate,
            );
            let display_value = if fact_is_procedural(&f.predicate) {
                let date_note = format_noted_date(&f.created_at);
                format!("{} ({})", f.display_value, date_note)
            } else {
                f.display_value
            };
            FactEntry {
                fact_id: f.id,
                predicate: f.predicate,
                display_value,
                confidence: f.confidence,
                staleness_note,
                valid_from: f.valid_from,
            }
        })
        .collect();

    // Sort by confidence descending.
    entries.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // §10.2.1 FTS5 lexical boost: push BM25-matched facts to the front
    // so lexically relevant facts survive budget truncation.
    if query.len() >= 2 {
        if let Ok(fts_hits) = metadata.search_facts_fts(account_id, user_id, query, 20) {
            let fts_ids: Vec<&str> = fts_hits.iter().map(|f| f.id.as_str()).collect();
            if !fts_ids.is_empty() {
                entries.sort_by(|a, b| {
                    let a_pos = fts_ids.iter().position(|id| *id == a.fact_id);
                    let b_pos = fts_ids.iter().position(|id| *id == b.fact_id);
                    match (a_pos, b_pos) {
                        (Some(ai), Some(bi)) => ai.cmp(&bi),
                        (Some(_), None) => std::cmp::Ordering::Less,
                        (None, Some(_)) => std::cmp::Ordering::Greater,
                        (None, None) => b
                            .confidence
                            .partial_cmp(&a.confidence)
                            .unwrap_or(std::cmp::Ordering::Equal),
                    }
                });
            }
        }
    }

    Ok(entries)
}

/// Load episodes from MetadataStore and transform to EpisodeSummary.
fn load_and_transform_episodes(
    metadata: &MetadataStore,
    account_id: &str,
    user_id: &str,
    resource_id: Option<&str>,
) -> rusqlite::Result<Vec<EpisodeSummary>> {
    let episode_rows = metadata.get_episodes_by_user(account_id, user_id, resource_id)?;
    Ok(episode_rows
        .into_iter()
        .map(|ep| EpisodeSummary {
            episode_id: ep.episode_id,
            summary: ep.summary,
            salience: ep.salience_score,
            strength: ep.strength_score,
            recall_count: ep.recall_count as usize,
            emotional_valence: ep.emotional_valence,
            emotional_intensity: ep.emotional_intensity,
            context_tags_json: ep.context_tags_json,
            embedding_json: ep.embedding_json,
            created_at: Some(ep.created_at),
        })
        .collect())
}

/// Compute staleness annotation for a fact.
///
/// Matches the handler's detailed staleness logic: includes predicate-category labels,
/// creation age, recall status (unverified / may-be-outdated), and validity-period annotations.
pub fn compute_staleness_note(
    created_at: &str,
    last_recalled_at: Option<&str>,
    recall_count: i64,
    valid_from: Option<&str>,
    predicate: &str,
) -> Option<String> {
    let now = chrono::Utc::now();
    let mut parts: Vec<String> = Vec::new();

    // Derive a predicate-category label for staleness annotations.
    let kind = if predicate.starts_with("preference.") {
        "preference"
    } else if predicate.starts_with("procedure.") {
        "procedure"
    } else if predicate.starts_with("convention.") {
        "convention"
    } else if predicate.starts_with("environment.") {
        "environment"
    } else if predicate.starts_with("identity.") {
        "identity"
    } else if predicate.starts_with("location.") {
        "location"
    } else {
        "fact"
    };

    // Check creation age.
    let created = match parse_timestamp(created_at) {
        Some(dt) => dt,
        None => return Some("Fact timestamp is unparseable — cannot verify freshness.".to_owned()),
    };
    let days_old = (now - created).num_days();
    if days_old < 0 {
        return Some(
            "Fact timestamp is in the future — possible data integrity concern.".to_owned(),
        );
    }
    if days_old >= 1 {
        parts.push(format!("recorded {} day(s) ago", days_old));
    }

    // Check recall status.
    if recall_count == 0 {
        parts.push("(unverified — never yet retrieved as relevant)".to_owned());
    } else if let Some(lr) = last_recalled_at {
        if let Some(last_recall) = parse_timestamp(lr) {
            let days_since_recall = (now - last_recall).num_days();
            if days_since_recall >= 7 {
                parts.push(format!(
                    "(may be outdated — last recalled {} days ago)",
                    days_since_recall
                ));
            }
        }
    }

    // Validity-period staleness annotation.
    if let Some(vf) = valid_from {
        if let Some(valid_from_dt) = parse_timestamp(vf) {
            let validity_days = (now - valid_from_dt).num_days();
            if validity_days > 7 {
                parts.push(format!(
                    "{} valid for {} days — may no longer reflect current {}",
                    kind, validity_days, kind
                ));
            } else if validity_days <= 1 {
                parts.push(format!("fresh {}", kind));
            }
        }
    }

    if parts.is_empty() {
        return None;
    }

    // Compose: join with "; " and append verification guidance.
    let mut note = parts.join("; ");
    note.push_str(". Verify against current state before asserting as fact.");
    Some(note)
}

/// Parse a timestamp string into a UTC DateTime.
pub fn parse_timestamp(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    // Try RFC 3339 first, then common ISO-8601 variants.
    chrono::DateTime::parse_from_rfc3339(s.trim())
        .ok()
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .or_else(|| {
            // Fallback: try YYYY-MM-DD HH:MM:SS (SQLite format).
            chrono::NaiveDateTime::parse_from_str(s.trim(), "%Y-%m-%d %H:%M:%S")
                .ok()
                .map(|ndt| chrono::DateTime::from_naive_utc_and_offset(ndt, chrono::Utc))
        })
}

/// Format a "noted YYYY-MM-DD" string from an ISO-8601 timestamp.
pub fn format_noted_date(created_at: &str) -> String {
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(created_at) {
        let date = dt.format("%Y-%m-%d");
        format!("noted {}", date)
    } else {
        "noted date unknown".to_owned()
    }
}

/// Check if a fact predicate belongs to a procedural category.
pub fn fact_is_procedural(predicate: &str) -> bool {
    predicate.starts_with("procedure.")
        || predicate.starts_with("convention.")
        || predicate.starts_with("environment.")
}

/// Embed query with optional timeout (Comprehensive strategy has no timeout).
async fn embed_query_with_timeout(
    query: &str,
    embedding_provider: &dyn EmbeddingProvider,
    embed_timeout_ms: u64,
    strategy: SearchStrategy,
) -> Option<Vec<f32>> {
    if embed_timeout_ms == 0 || strategy == SearchStrategy::Comprehensive {
        mfs_semantic::try_embed_query(query, embedding_provider).await
    } else {
        tokio::time::timeout(
            std::time::Duration::from_millis(embed_timeout_ms),
            mfs_semantic::try_embed_query(query, embedding_provider),
        )
        .await
        .ok()
        .flatten()
    }
}

/// Write recall_count, last_recalled_at, and access_log for final facts and episodes.
fn writeback_recall_and_access_log(
    metadata: &MetadataStore,
    facts: &[FactEntry],
    episodes: &[EpisodeSummary],
    account_id: &str,
    user_id: &str,
) -> rusqlite::Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    for fact in facts {
        metadata.increment_fact_recall(&fact.fact_id, &now)?;
        metadata.append_access_log(&fact.fact_id, "fact", &now, account_id, user_id)?;
    }
    for episode in episodes {
        metadata.increment_episode_recall(&episode.episode_id, &now)?;
        metadata.append_access_log(&episode.episode_id, "episode", &now, account_id, user_id)?;
    }
    Ok(())
}
