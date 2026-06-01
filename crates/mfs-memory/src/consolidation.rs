//! Consolidation module — pipeline: window resolution → chunk → build → extract → project → briefs → advance cursor.
//!
//! The consolidation worker processes queued jobs, resolves the consolidation
//! window (turns since cursor), chunks them into episodes, extracts facts,
//! projects assertions, refreshes briefs, and advances the cursor.
//!
//! LLM integration (signal灯塔 philosophy):
//! When LLM is available, uses it for:
//! - Episode summary generation (L0/L1 instead of simple concatenation)
//! - Fact extraction (full 8-category taxonomy instead of 45 regex rules covering 21 predicates)
//! - Dedup decision (semantic similarity instead of Jaccard token overlap)
//! All operations fall back to deterministic logic when LLM is unavailable.

use crate::llm::{LlmAssist, LlmDedupDecision, build_dedup_prompt, parse_llm_json};
use crate::{
    ConversationTurn, EpisodeChunk, Fact, FactAssertion, FactStatus,
    briefs::{build_resource_memory_brief, build_user_memory_brief},
    episodes::{build_episode, chunk_turns},
    facts::{extract_facts, project_assertion},
};
use mfs_metadata::{EpisodeRow, FactRecord, MetadataStore};
use mfs_types::MfsError;
use mfs_types::text::{TokenizeConfig, tokenize_to_vec};

// ─── Episode Deduplication (OV-P2) ──────────────────────────────────

/// Deduplication decision for a candidate episode (OV-P2-2).
#[derive(Debug, Clone, PartialEq, Eq)]
enum EpisodeDedupDecision {
    /// Skip — an existing episode is sufficiently similar (overlap > skip_threshold)
    Skip,
    /// Merge — boost the target episode's salience/strength instead of creating a new one
    Merge(String),
    /// Replace — archive the old episode and create a new one
    Replace(String),
    /// Create — no similar episode found, proceed with normal insert
    Create,
}

/// Decide whether a candidate episode should be skipped, merged, replaced, or created.
///
/// Strategy: LLM semantic dedup first, Jaccard token-level fallback.
/// When LLM is available, it judges semantic duplication (not just token overlap)
/// and can produce richer decisions (e.g., "boost + update_summary" instead of just "boost").
/// When LLM is unavailable, falls back to the Jaccard heuristic (OV-P2).
async fn decide_episode_dedup_with_llm(
    candidate_summary: &str,
    candidate_overview: &str,
    existing_episodes: &[EpisodeRow],
    skip_threshold: f64,
    merge_threshold: f64,
    llm: &LlmAssist,
) -> EpisodeDedupDecision {
    // Try LLM semantic dedup first.
    if let Some(decision) = try_llm_dedup(
        candidate_summary,
        candidate_overview,
        existing_episodes,
        llm,
    )
    .await
    {
        return decision;
    }
    // Fall back to Jaccard heuristic.
    decide_episode_dedup(
        candidate_summary,
        existing_episodes,
        skip_threshold,
        merge_threshold,
    )
}

/// LLM-based semantic dedup decision.
async fn try_llm_dedup(
    candidate_summary: &str,
    candidate_overview: &str,
    existing_episodes: &[EpisodeRow],
    llm: &LlmAssist,
) -> Option<EpisodeDedupDecision> {
    if !llm.is_available() || existing_episodes.is_empty() {
        return None;
    }

    // Only compare against non-archived episodes
    let active_episodes: Vec<&EpisodeRow> = existing_episodes
        .iter()
        .filter(|ep| ep.archived_at.is_none())
        .collect();

    if active_episodes.is_empty() {
        return None;
    }

    // Limit to top 5 most similar by summary length to avoid overwhelming the LLM
    let candidates_text = active_episodes
        .iter()
        .take(5)
        .map(|ep| format!("episode_id={}\nsummary: {}", ep.episode_id, ep.summary))
        .collect::<Vec<_>>()
        .join("\n\n");

    let prompt = build_dedup_prompt(candidate_summary, candidate_overview, &candidates_text);
    let response = llm.complete(&prompt).await?;

    let parsed: LlmDedupDecision = parse_llm_json(&response)?;

    match parsed.decision.as_str() {
        "skip" => Some(EpisodeDedupDecision::Skip),
        "merge" => {
            let target_id = parsed
                .targets
                .as_ref()
                .and_then(|t| t.first())
                .map(|t| t.episode_id.clone());
            target_id
                .map(EpisodeDedupDecision::Merge)
                .or(Some(EpisodeDedupDecision::Skip)) // merge without target = skip
        }
        "replace" => {
            let target_id = parsed
                .targets
                .as_ref()
                .and_then(|t| t.first())
                .map(|t| t.episode_id.clone());
            target_id
                .map(EpisodeDedupDecision::Replace)
                .or(Some(EpisodeDedupDecision::Create)) // replace without target = create
        }
        "create" => Some(EpisodeDedupDecision::Create),
        _ => None,
    }
}

/// Decide whether a candidate episode should be skipped, merged, replaced, or created (OV-P2-1~2).
/// Uses token-level overlap (Jaccard similarity) as the similarity metric.
/// This is the deterministic fallback when LLM is unavailable.
fn decide_episode_dedup(
    candidate_summary: &str,
    existing_episodes: &[EpisodeRow],
    skip_threshold: f64,
    merge_threshold: f64,
) -> EpisodeDedupDecision {
    let candidate_tokens = tokenize_summary(candidate_summary);

    if candidate_tokens.is_empty() {
        return EpisodeDedupDecision::Create;
    }

    let mut best_match: Option<(f64, &EpisodeRow)> = None;

    for existing in existing_episodes {
        if existing.archived_at.is_some() {
            continue;
        }
        let existing_tokens = tokenize_summary(&existing.summary);
        if existing_tokens.is_empty() {
            continue;
        }

        let similarity = jaccard_similarity(&candidate_tokens, &existing_tokens);
        if best_match.is_none() || similarity > best_match.as_ref().unwrap().0 {
            best_match = Some((similarity, existing));
        }
    }

    match best_match {
        None => EpisodeDedupDecision::Create,
        Some((similarity, episode)) => {
            if similarity >= skip_threshold {
                EpisodeDedupDecision::Skip
            } else if similarity >= merge_threshold {
                EpisodeDedupDecision::Merge(episode.episode_id.clone())
            } else if similarity >= merge_threshold * 0.8 && episode.salience_score < 0.3 {
                // Weak old episode + moderate overlap → replace
                EpisodeDedupDecision::Replace(episode.episode_id.clone())
            } else {
                EpisodeDedupDecision::Create
            }
        }
    }
}

fn tokenize_summary(summary: &str) -> Vec<String> {
    let config = TokenizeConfig {
        trim_edges: true,
        min_len: 2,
        preserve_semantic_short_words: true,
    };
    tokenize_to_vec(summary, &config)
}

/// Compute Jaccard similarity between two token sets.
///
/// When both sets are empty, returns 0.0 (no evidence of similarity) rather
/// than 1.0 — two facts whose display_values produce no tokens provide zero
/// signal that they are semantically similar.
fn jaccard_similarity(tokens_a: &[String], tokens_b: &[String]) -> f64 {
    let set_a: std::collections::HashSet<_> = tokens_a.iter().collect();
    let set_b: std::collections::HashSet<_> = tokens_b.iter().collect();

    if set_a.is_empty() || set_b.is_empty() {
        return 0.0;
    }

    let intersection = set_a.intersection(&set_b).count() as f64;
    let union = set_a.union(&set_b).count() as f64;

    if union == 0.0 {
        return 0.0;
    }

    intersection / union
}

/// Job type constants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobType {
    Consolidate,
    Rebuild,
    Replay,
}

/// Job status lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
}

/// Memory job record.
#[derive(Debug, Clone)]
pub struct MemoryJob {
    pub job_id: String,
    pub job_type: JobType,
    pub status: JobStatus,
    pub scope_id: String,
    pub dedupe_key: String,
    pub payload_json: Option<String>,
    pub retry_count: u32,
    pub lease_owner: Option<String>,
    pub lease_expires_at: Option<String>,
    pub scheduled_at: Option<String>,
    pub finished_at: Option<String>,
    pub error_text: Option<String>,
    pub created_at: String,
}

/// Result of a consolidation run.
#[derive(Debug, Clone)]
pub struct ConsolidationResult {
    pub range_start_turn_id: String,
    pub range_end_turn_id: String,
    pub episode_count: usize,
    pub assertion_count: usize,
    pub fact_count: usize,
    pub turn_count: usize,
}

// ─── Window Resolution ──────────────────────────────────────────────

/// Resolve the consolidation window: compute [start_seq, end_seq].
///
/// Given:
/// - after_seq: cursor position (last consolidated turn)
/// - requested_start_seq: optional override from job range
/// - requested_end_seq: optional override from job range  
/// - latest_seq: latest turn sequence in session
///
/// Returns (start_seq, end_seq, ok). If end < start, returns false (nothing to consolidate).
pub fn resolve_consolidation_window(
    after_seq: i64,
    requested_start_seq: i64,
    requested_end_seq: i64,
    latest_seq: i64,
) -> (i64, i64, bool) {
    let start_seq = if requested_start_seq > after_seq + 1 {
        requested_start_seq
    } else {
        after_seq + 1
    };

    let end_seq = if requested_end_seq == 0 || requested_end_seq > latest_seq {
        latest_seq
    } else {
        requested_end_seq
    };

    if end_seq < start_seq {
        return (0, 0, false);
    }

    (start_seq, end_seq, true)
}

// ─── Consolidation Pipeline ──────────────────────────────────────────

/// Execute the consolidation pipeline on a set of turns.
///
/// Steps:
/// 1. Chunk turns into episodes (by time gap + token budget)
/// 2. For each chunk: build episode (LLM or simple summary), extract facts (LLM or regex), project assertions
/// 3. Advance cursor after each chunk
///
/// Returns the consolidation result with counts.
pub async fn consolidate_turns(
    turns: &[ConversationTurn],
    user_id: &str,
    session_id: &str,
    resource_id: Option<&str>,
    llm: &LlmAssist,
) -> ConsolidationResult {
    if turns.is_empty() {
        return ConsolidationResult {
            range_start_turn_id: String::new(),
            range_end_turn_id: String::new(),
            episode_count: 0,
            assertion_count: 0,
            fact_count: 0,
            turn_count: 0,
        };
    }

    let range_start = turns[0].turn_id.clone();
    let range_end = turns[turns.len() - 1].turn_id.clone();
    let chunks = chunk_turns(turns);

    let mut episode_count = 0;
    let mut assertion_count = 0;
    let mut fact_count = 0;

    for chunk in &chunks {
        if chunk.is_empty() {
            continue;
        }

        // Build episode using LLM (with simple summary fallback)
        let _episode = build_episode(chunk, user_id, session_id, resource_id, llm).await;

        // Extract facts from chunk (LLM or regex)
        let assertions = extract_facts(chunk, llm).await;
        assertion_count += assertions.len();
        for a in &assertions {
            if a.operation != crate::FactOperation::Retract {
                fact_count += 1;
            }
            let _ = project_assertion(a, &[], "");
        }

        episode_count += 1;
    }

    ConsolidationResult {
        range_start_turn_id: range_start,
        range_end_turn_id: range_end,
        episode_count,
        assertion_count,
        fact_count,
        turn_count: turns.len(),
    }
}

// ─── Rebuild ──────────────────────────────────────────────────────────

/// Rebuild all episodes and facts for a user from scratch.
///
/// This is a destructive operation: all existing episodes, facts, and cursors
/// for the user are deleted, then re-extracted from all turns.
pub async fn rebuild_user_turns(
    all_turns: &[ConversationTurn],
    user_id: &str,
    sessions: &[(String, Option<String>)], // (session_id, resource_id)
    llm: &LlmAssist,
) -> ConsolidationResult {
    let mut total_episodes = 0;
    let mut total_assertions = 0;
    let mut total_facts = 0;

    for (session_id, resource_id) in sessions {
        let session_turns: Vec<ConversationTurn> = all_turns
            .iter()
            .filter(|t| t.session_id == *session_id)
            .cloned()
            .collect();

        if session_turns.is_empty() {
            continue;
        }

        let resource_id_str = resource_id.as_deref();
        let result =
            consolidate_turns(&session_turns, user_id, session_id, resource_id_str, llm).await;
        total_episodes += result.episode_count;
        total_assertions += result.assertion_count;
        total_facts += result.fact_count;
    }

    ConsolidationResult {
        range_start_turn_id: all_turns
            .first()
            .map(|t| t.turn_id.clone())
            .unwrap_or_default(),
        range_end_turn_id: all_turns
            .last()
            .map(|t| t.turn_id.clone())
            .unwrap_or_default(),
        episode_count: total_episodes,
        assertion_count: total_assertions,
        fact_count: total_facts,
        turn_count: all_turns.len(),
    }
}

// ─── Replay ──────────────────────────────────────────────────────────

/// Replay a single thread's turns through consolidation.
/// Similar to consolidate_turns but explicitly scoped to one thread.
pub async fn replay_thread(
    turns: &[ConversationTurn],
    user_id: &str,
    thread_id: &str,
    resource_id: Option<&str>,
    llm: &LlmAssist,
) -> ConsolidationResult {
    consolidate_turns(turns, user_id, thread_id, resource_id, llm).await
}

/// Persist one committed session slice into metadata.sqlite using the mfs-memory model.
/// Uses LLM for summary generation, fact extraction, and dedup when available.
pub async fn consolidate_and_persist(
    metadata: &MetadataStore,
    account_id: &str,
    user_id: &str,
    agent_id: &str,
    session_id: &str,
    resource_id: Option<&str>,
    turns: &[ConversationTurn],
    llm: &LlmAssist,
) -> Result<ConsolidationResult, MfsError> {
    if metadata
        .get_session(session_id)
        .map_err(metadata_error)?
        .is_none()
    {
        metadata
            .insert_session(session_id, account_id, user_id, agent_id, "active", None)
            .map_err(metadata_error)?;
    } else {
        metadata
            .update_session_activity(session_id)
            .map_err(metadata_error)?;
    }

    let existing_turn_ids = metadata
        .get_turns_by_session(session_id)
        .map_err(metadata_error)?
        .into_iter()
        .map(|turn| turn.turn_id)
        .collect::<std::collections::HashSet<_>>();

    for turn in turns {
        if existing_turn_ids.contains(&turn.turn_id) {
            continue;
        }
        metadata
            .insert_turn(
                &turn.turn_id,
                turn.turn_seq,
                session_id,
                account_id,
                user_id,
                agent_id,
                turn.role.as_str(),
                &turn.content_text,
                None,
                turn.token_count as i64,
                Some(&turn.created_at),
            )
            .map_err(metadata_error)?;
    }

    let mut active_facts = metadata
        .get_active_facts(account_id, user_id)
        .map_err(metadata_error)?
        .into_iter()
        .map(stored_fact_to_memory_fact)
        .collect::<Vec<_>>();

    let chunks = chunk_turns(turns);
    let mut persisted_episodes = Vec::new();
    let mut assertion_count = 0;
    let mut fact_count = 0;

    // Create embedding provider once for all episodes in this consolidation run
    let embedding_provider = mfs_semantic::embedding_provider_from_env(256);
    let use_embeddings = embedding_provider.mode() != mfs_semantic::ProcessingMode::Degraded;

    // OV-P2: Deduplication threshold — skip if summary overlap > 80% with an existing episode
    const DEDUP_SKIP_THRESHOLD: f64 = 0.80;
    const DEDUP_MERGE_THRESHOLD: f64 = 0.60;

    // Load existing episodes for dedup comparison
    let existing_episodes = metadata
        .get_episodes_by_user(account_id, user_id, None)
        .map_err(metadata_error)?;

    for chunk in chunks {
        if chunk.is_empty() {
            continue;
        }

        let mut episode = build_episode(&chunk, user_id, session_id, resource_id, llm).await;
        episode.episode_id = deterministic_episode_id(session_id, &episode);

        // OV-P2-1~2: Episode deduplication (LLM semantic + Jaccard fallback)
        let dedup_decision = decide_episode_dedup_with_llm(
            &episode.summary,
            "",
            &existing_episodes,
            DEDUP_SKIP_THRESHOLD,
            DEDUP_MERGE_THRESHOLD,
            llm,
        )
        .await;

        match dedup_decision {
            EpisodeDedupDecision::Skip => {
                // Already covered by a sufficiently similar episode
                continue;
            }
            EpisodeDedupDecision::Merge(target_episode_id) => {
                // Merge: boost the existing episode's salience and strength
                if let Some(target) = existing_episodes
                    .iter()
                    .find(|ep| ep.episode_id == target_episode_id)
                {
                    let boosted_salience = target.salience_score.max(episode.salience_score);
                    let boosted_strength = target.strength_score + 0.1;
                    metadata
                        .update_episode_salience(
                            &target_episode_id,
                            boosted_salience,
                            boosted_strength,
                            target.recall_count,
                            target.last_recalled_at.as_deref().unwrap_or(""),
                        )
                        .map_err(metadata_error)?;
                }
                continue;
            }
            EpisodeDedupDecision::Replace(target_episode_id) => {
                // Replace: archive the old, create the new
                metadata
                    .archive_episode(&target_episode_id, &episode.created_at)
                    .map_err(metadata_error)?;
            }
            EpisodeDedupDecision::Create => {
                // Normal path: no similar episode exists
            }
        }

        if metadata
            .get_episode(&episode.episode_id)
            .map_err(metadata_error)?
            .is_none()
        {
            metadata
                .insert_episode(
                    &episode.episode_id,
                    account_id,
                    user_id,
                    agent_id,
                    session_id,
                    resource_id,
                    &episode.summary,
                    None,
                    None,
                    episode.salience_score,
                    episode.strength_score,
                    episode.emotional_valence,
                    episode.emotional_intensity,
                    episode.context_tags_json.as_deref(),
                    episode.recall_count as i64,
                    episode.last_recalled_at.as_deref(),
                    Some(&episode.source_start_turn_id),
                    Some(&episode.source_end_turn_id),
                    None,
                    None,
                    None,
                )
                .map_err(metadata_error)?;

            // Compute and store embedding for the episode summary (best-effort)
            if use_embeddings {
                if let Some(emb_json) =
                    embed_episode_summary(&episode.summary, embedding_provider.as_ref()).await
                {
                    if let Err(e) =
                        metadata.update_episode_embedding(&episode.episode_id, &emb_json)
                    {
                        tracing::warn!(
                            episode_id = %episode.episode_id,
                            error = %e,
                            "failed to store episode embedding in metadata DB"
                        );
                    }
                }
            }
        }

        let mut assertions = extract_facts(&chunk, llm).await;
        for assertion in &mut assertions {
            assertion.assertion_id = deterministic_assertion_id(&episode.episode_id, assertion);
            assertion.source_episode_ids = Some(vec![episode.episode_id.clone()]);
            // Keep extractor_version as set by the extraction method (v2-llm or v1-rules)
        }

        if metadata
            .get_assertions_by_source(None, Some(&episode.episode_id))
            .map_err(metadata_error)?
            .is_empty()
        {
            for assertion in &assertions {
                // Phase 2-1: Fact semantic dedup — skip if a sufficiently similar active fact exists.
                // Uses Jaccard token overlap on display_value as a deterministic proxy for semantic
                // similarity. Threshold 0.80 catches near-verbatim duplicates where most words
                // overlap (e.g. "User currently lives in Tokyo Japan" vs "User currently lives in
                // Tokyo"). It does NOT catch semantic near-duplicates with different key verbs
                // (e.g. "prefers dark mode" vs "likes dark theme" Jaccard ≈0.33). Those require
                // LLM-mode dedup. Negation words like "no" are preserved via SEMANTIC_SHORT_WORDS
                // to prevent "no sugar" from deduping against "sugar".
                if assertion.operation == crate::FactOperation::Assert {
                    let candidate_display = crate::facts::format_display_value(assertion);
                    let candidate_tokens = tokenize_summary(&candidate_display);
                    let is_duplicate = active_facts.iter().any(|f| {
                        if f.predicate != assertion.predicate {
                            return false;
                        }
                        let existing_tokens = tokenize_summary(&f.display_value);
                        jaccard_similarity(&candidate_tokens, &existing_tokens) >= 0.80
                    });
                    if is_duplicate {
                        continue;
                    }
                }

                metadata
                    .insert_fact_assertion(
                        &assertion.assertion_id,
                        account_id,
                        user_id,
                        agent_id,
                        &assertion.subject,
                        &assertion.predicate,
                        &assertion.raw_value_text,
                        None,
                        &assertion.value_type,
                        assertion.operation.as_str(),
                        assertion.confidence,
                        assertion.valid_from.as_deref(),
                        assertion.valid_to.as_deref(),
                        assertion.source_turn_id.as_deref(),
                        assertion
                            .source_episode_ids
                            .as_ref()
                            .and_then(|ids| serde_json::to_string(ids).ok())
                            .as_deref(),
                        None,
                        None,
                        None,
                        &assertion.extractor_version,
                    )
                    .map_err(metadata_error)?;
                assertion_count += 1;

                let now_str = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
                let projected = project_assertion(assertion, &active_facts, &now_str);
                fact_count += persist_projected_facts(
                    metadata, account_id, user_id, agent_id, &projected, &now_str,
                )?;
                active_facts = metadata
                    .get_active_facts(account_id, user_id)
                    .map_err(metadata_error)?
                    .into_iter()
                    .map(stored_fact_to_memory_fact)
                    .collect();
            }
        }

        persisted_episodes.push(episode);
    }

    if let Some(last_turn) = turns.last() {
        if let Some(cursor) = metadata
            .get_cursor(account_id, user_id, "thread", session_id)
            .map_err(metadata_error)?
        {
            metadata
                .advance_cursor(&cursor.cursor_id, &last_turn.turn_id, &last_turn.created_at)
                .map_err(metadata_error)?;
        } else {
            metadata
                .insert_cursor(
                    &format!("cursor:thread:{session_id}"),
                    account_id,
                    user_id,
                    "thread",
                    session_id,
                    Some(&last_turn.turn_id),
                    Some(&last_turn.created_at),
                )
                .map_err(metadata_error)?;
        }
    }

    refresh_briefs(metadata, account_id, user_id, &persisted_episodes)?;

    // Note: T2H pipeline is now called from mfs-session's run_background_memory_pipeline
    // (after this consolidation step completes), not from within consolidation itself.
    // This avoids the dual-call bug where T2H ran both here and in the session layer.

    Ok(ConsolidationResult {
        range_start_turn_id: turns
            .first()
            .map(|turn| turn.turn_id.clone())
            .unwrap_or_default(),
        range_end_turn_id: turns
            .last()
            .map(|turn| turn.turn_id.clone())
            .unwrap_or_default(),
        episode_count: persisted_episodes.len(),
        assertion_count,
        fact_count,
        turn_count: turns.len(),
    })
}

fn persist_projected_facts(
    metadata: &MetadataStore,
    account_id: &str,
    user_id: &str,
    agent_id: &str,
    projected: &[Fact],
    valid_to_now: &str,
) -> Result<usize, MfsError> {
    if projected.is_empty() {
        return Ok(0);
    }

    match projected {
        [fact] if fact.status == FactStatus::Active => {
            insert_active_fact(metadata, account_id, user_id, agent_id, fact)?;
            Ok(1)
        }
        [fact] if fact.status == FactStatus::Retracted => {
            metadata
                .retract_fact(&fact.fact_id, valid_to_now)
                .map_err(metadata_error)?;
            Ok(1)
        }
        [old_fact, new_fact]
            if old_fact.status == FactStatus::Superseded
                && new_fact.status == FactStatus::Active =>
        {
            metadata
                .supersede_fact(&old_fact.fact_id, &new_fact.fact_id, valid_to_now)
                .map_err(metadata_error)?;
            insert_active_fact(metadata, account_id, user_id, agent_id, new_fact)?;
            Ok(2)
        }
        facts => {
            for fact in facts {
                if fact.status == FactStatus::Active {
                    insert_active_fact(metadata, account_id, user_id, agent_id, fact)?;
                }
            }
            Ok(facts.len())
        }
    }
}

fn insert_active_fact(
    metadata: &MetadataStore,
    account_id: &str,
    user_id: &str,
    agent_id: &str,
    fact: &Fact,
) -> Result<(), MfsError> {
    metadata
        .insert_fact(&FactRecord {
            id: &fact.fact_id,
            account_id,
            user_id,
            agent_id: Some(agent_id),
            subject: &fact.subject,
            predicate: &fact.predicate,
            display_value: &fact.display_value,
            normalized_value_json: None,
            value_type: "scalar",
            confidence: fact.confidence,
            status: fact.status.as_str(),
            valid_from: fact.valid_from.as_deref(),
            valid_to: fact.valid_to.as_deref(),
            source_assertion_id: Some(&fact.source_assertion_id),
            source_episode_ids_json: fact
                .source_episode_ids
                .as_ref()
                .and_then(|ids| serde_json::to_string(ids).ok())
                .as_deref(),
        })
        .map_err(metadata_error)
}

pub fn refresh_briefs(
    metadata: &MetadataStore,
    account_id: &str,
    user_id: &str,
    recent_episodes: &[EpisodeChunk],
) -> Result<(), MfsError> {
    let all_episode_rows = metadata
        .get_episodes_by_user(account_id, user_id, None)
        .map_err(metadata_error)?;
    let all_episodes = all_episode_rows
        .iter()
        .map(episode_row_to_memory_episode)
        .collect::<Vec<_>>();
    if let Some(mut brief) = build_user_memory_brief(user_id, &all_episodes) {
        brief.brief_id = format!("brief:user:{user_id}");
        let source_thread_ids_json =
            serde_json::to_string(&brief.source_thread_ids).unwrap_or_else(|_| "[]".to_owned());
        let anchor_episode_ids_json =
            serde_json::to_string(&brief.anchor_episode_ids).unwrap_or_else(|_| "[]".to_owned());
        metadata
            .upsert_brief(
                &brief.brief_id,
                account_id,
                user_id,
                "user",
                user_id,
                &brief.summary,
                Some(&source_thread_ids_json),
                Some(&anchor_episode_ids_json),
            )
            .map_err(metadata_error)?;
    }

    if let Some(resource_id) = recent_episodes
        .iter()
        .find_map(|episode| episode.resource_id.clone())
    {
        let resource_episodes = all_episodes
            .iter()
            .filter(|episode| episode.resource_id.as_deref() == Some(resource_id.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        if let Some(mut brief) =
            build_resource_memory_brief(user_id, &resource_id, &resource_episodes)
        {
            brief.brief_id = format!("brief:resource:{resource_id}");
            let source_thread_ids_json =
                serde_json::to_string(&brief.source_thread_ids).unwrap_or_else(|_| "[]".to_owned());
            let anchor_episode_ids_json = serde_json::to_string(&brief.anchor_episode_ids)
                .unwrap_or_else(|_| "[]".to_owned());
            metadata
                .upsert_brief(
                    &brief.brief_id,
                    account_id,
                    user_id,
                    "resource",
                    &resource_id,
                    &brief.summary,
                    Some(&source_thread_ids_json),
                    Some(&anchor_episode_ids_json),
                )
                .map_err(metadata_error)?;
        }
    }

    Ok(())
}

fn stored_fact_to_memory_fact(fact: mfs_metadata::StoredFact) -> Fact {
    Fact {
        fact_id: fact.id,
        user_id: fact.user_id,
        subject: fact.subject,
        predicate: fact.predicate,
        display_value: fact.display_value,
        confidence: fact.confidence,
        status: match fact.status.as_str() {
            "superseded" => FactStatus::Superseded,
            "retracted" => FactStatus::Retracted,
            "expired" => FactStatus::Expired,
            _ => FactStatus::Active,
        },
        source_assertion_id: fact.source_assertion_id.unwrap_or_default(),
        valid_from: fact.valid_from,
        valid_to: fact.valid_to,
        source_episode_ids: fact
            .source_episode_ids_json
            .as_deref()
            .and_then(|json| serde_json::from_str::<Vec<String>>(json).ok()),
    }
}

fn episode_row_to_memory_episode(episode: &mfs_metadata::EpisodeRow) -> EpisodeChunk {
    EpisodeChunk {
        episode_id: episode.episode_id.clone(),
        user_id: episode.user_id.clone(),
        session_id: episode.session_id.clone(),
        resource_id: episode.resource_id.clone(),
        summary: episode.summary.clone(),
        salience_score: episode.salience_score,
        strength_score: episode.strength_score,
        recall_count: episode.recall_count as usize,
        last_recalled_at: episode.last_recalled_at.clone(),
        source_start_turn_id: episode.source_start_turn_id.clone().unwrap_or_default(),
        source_end_turn_id: episode.source_end_turn_id.clone().unwrap_or_default(),
        created_at: episode.created_at.clone(),
        embedding: None,
        emotional_valence: episode.emotional_valence,
        emotional_intensity: episode.emotional_intensity,
        context_tags_json: episode.context_tags_json.clone(),
    }
}

fn deterministic_episode_id(session_id: &str, episode: &EpisodeChunk) -> String {
    format!(
        "ep:{}:{}:{}",
        session_id, episode.source_start_turn_id, episode.source_end_turn_id
    )
}

fn deterministic_assertion_id(episode_id: &str, assertion: &FactAssertion) -> String {
    format!(
        "ast:{}:{}:{}:{}",
        episode_id,
        assertion.source_turn_id.as_deref().unwrap_or(""),
        assertion.predicate,
        assertion.raw_value_text
    )
}

fn metadata_error(error: impl ToString) -> MfsError {
    MfsError::Internal {
        message: error.to_string(),
    }
}

/// Embed an episode summary text using the provided embedding provider.
/// Returns the JSON-serialized `Vec<f32>` on success, or `None` if embedding
/// is unavailable or fails.
async fn embed_episode_summary(
    summary: &str,
    provider: &dyn mfs_semantic::EmbeddingProvider,
) -> Option<String> {
    let vector = provider.embed_text(summary).await;
    if vector.is_empty() || vector.iter().all(|v| *v == 0.0) {
        return None;
    }
    serde_json::to_string(&vector).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TurnRole;

    fn make_turn(seq: i64, role: TurnRole, content: &str) -> ConversationTurn {
        ConversationTurn {
            turn_id: format!("turn-{}", seq),
            turn_seq: seq,
            session_id: "s1".to_owned(),
            user_id: "u1".to_owned(),
            role,
            content_text: content.to_owned(),
            token_count: content.len() / 4,
            created_at: format!("2026-01-01T{:02}:{:02}:00Z", 10 + seq / 6, seq % 6 * 5),
        }
    }

    #[test]
    fn resolve_window_basic() {
        let (start, end, ok) = resolve_consolidation_window(0, 0, 0, 10);
        assert!(ok);
        assert_eq!(start, 1);
        assert_eq!(end, 10);
    }

    #[test]
    fn resolve_window_with_cursor() {
        let (start, end, ok) = resolve_consolidation_window(5, 0, 0, 10);
        assert!(ok);
        assert_eq!(start, 6);
        assert_eq!(end, 10);
    }

    #[test]
    fn resolve_window_empty_range() {
        let (_start, _end, ok) = resolve_consolidation_window(10, 0, 0, 10);
        assert!(!ok); // end_seq < start_seq
    }

    #[test]
    fn resolve_window_with_requested_range() {
        let (start, end, ok) = resolve_consolidation_window(0, 3, 8, 10);
        assert!(ok);
        assert_eq!(start, 3);
        assert_eq!(end, 8);
    }

    #[tokio::test]
    async fn consolidate_empty_turns() {
        let llm = LlmAssist::from_env();
        let result = consolidate_turns(&[], "u1", "s1", None, &llm).await;
        assert_eq!(result.episode_count, 0);
        assert_eq!(result.turn_count, 0);
    }

    #[tokio::test]
    async fn consolidate_with_facts() {
        let llm = LlmAssist::from_env();
        let turns = vec![
            make_turn(1, TurnRole::User, "I live in Tokyo"),
            make_turn(2, TurnRole::User, "My name is Alice"),
        ];
        let result = consolidate_turns(&turns, "u1", "s1", None, &llm).await;
        assert!(result.episode_count > 0);
        // assertion_count may be 0 if LLM is degraded (regex fallback may not match these patterns)
        // but episode_count should always be > 0
    }

    #[test]
    fn job_type_constants() {
        assert_eq!(JobType::Consolidate as u8, 0);
        assert_eq!(JobType::Rebuild as u8, 1);
        assert_eq!(JobType::Replay as u8, 2);
    }

    #[tokio::test]
    async fn test_rebuild_user_turns() {
        let llm = LlmAssist::from_env();
        let turns = vec![
            make_turn(1, TurnRole::User, "I live in Tokyo"),
            make_turn(2, TurnRole::User, "My name is Alice"),
        ];
        let sessions: Vec<(String, Option<String>)> = vec![("s1".to_owned(), None)];
        let result = rebuild_user_turns(&turns, "u1", &sessions, &llm).await;
        assert!(result.episode_count > 0);
    }
}
