//! Four-phase dream consolidation pipeline (§5.3).
//!
//! Upgrades Dream from "batch re-run of consolidate_and_persist" to a structured
//! Orient → Gather → Consolidate → Prune pipeline,
//! implemented as function-style pipeline (no agent loop — MemFuse is data plane).
//!
//! Phase 1 — Orient: scan current memory state, compute priority.
//! Phase 2 — Gather: run standard consolidation on eligible sessions.
//! Phase 3 — Consolidate: resolve fact conflicts, compress stale episodes,
//!           run sleep consolidation for heuristic evolution.
//! Phase 4 — Prune: archive decayed items, refresh briefs, audit log.

use crate::ConversationTurn;
use crate::TurnRole;
use crate::consolidation::consolidate_and_persist;
use crate::consolidation_sleep::run_sleep_consolidation;
use crate::llm::LlmAssist;
use mfs_metadata::{EpisodeRow, MetadataStore, SessionRow, StoredFact};
use mfs_types::MfsError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{debug, info, warn};

// ─── Dream Phase Types ──────────────────────────────────────────────

/// Orientation scan result — snapshot of current memory state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrientScan {
    /// Number of active facts, grouped by predicate prefix.
    pub facts_by_prefix: HashMap<String, usize>,
    /// Total active facts.
    pub total_active_facts: usize,
    /// Total active episodes.
    pub total_active_episodes: usize,
    /// Episodes with salience < threshold (candidates for compression).
    pub low_salience_episodes: usize,
    /// Heuristic rules by lifecycle stage.
    pub rules_by_stage: HashMap<String, usize>,
    /// Total heuristic rules.
    pub total_rules: usize,
}

/// Gather result — what was consolidated from eligible sessions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatherResult {
    /// Number of sessions processed.
    pub sessions_processed: usize,
    /// New episodes created.
    pub new_episodes: usize,
    /// New facts created.
    pub new_facts: usize,
    /// Fact conflicts detected (same predicate, different value).
    pub fact_conflicts: usize,
    /// Conflicting predicate details.
    pub conflicting_predicates: Vec<String>,
}

/// Consolidate result — what was merged/evolved.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsolidateResult {
    /// Fact conflicts resolved (superseded count).
    pub facts_resolved: usize,
    /// Stale episodes archived (low-salience + zero-recall).
    pub episodes_archived_stale: usize,
    /// Sleep consolidation stats.
    pub sleep_result: Option<SleepConsolidationSummary>,
}

/// Summary of sleep consolidation (subset of full result for DreamReport).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SleepConsolidationSummary {
    pub pairs_discovered: usize,
    pub latent_variables_found: usize,
    pub rules_merged: usize,
    pub rules_archived: usize,
}

/// Prune result — what was archived/cleaned.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PruneResult {
    /// Episodes archived (salience < threshold).
    pub episodes_archived: usize,
    /// Heuristic rules archived (weight < threshold).
    pub rules_archived: usize,
    /// Briefs refreshed (1 if refresh succeeded, 0 if refresh failed).
    /// Note: a value of 1 may mean "no brief needed" (refresh_briefs returns Ok
    /// even when no brief content exists), rather than "brief was actually rebuilt".
    #[serde(alias = "briefs_exist")]
    pub briefs_refreshed: usize,
}

/// Complete dream report — all 4 phases combined.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DreamReport {
    pub orient: OrientScan,
    pub gather: GatherResult,
    pub consolidate: ConsolidateResult,
    pub prune: PruneResult,
}

// ─── Threshold Constants ─────────────────────────────────────────────

/// Salience threshold for low-salience episode detection.
const LOW_SALIENCE_THRESHOLD: f64 = 0.2;
/// Rule survival threshold for pruning (rules with aggregate_weight below this are archived).
const RULE_SURVIVAL_THRESHOLD: f64 = 0.3;

// ─── Helper: convert metadata store errors ──────────────────────────

fn meta_err<T: std::fmt::Display>(e: T) -> MfsError {
    MfsError::Internal {
        message: e.to_string(),
    }
}

// ─── Phase 1: Orient ──────────────────────────────────────────────────

/// Scan current memory state to determine consolidation priorities.
pub async fn orient_scan(
    metadata: &MetadataStore,
    account_id: &str,
    user_id: &str,
) -> Result<OrientScan, MfsError> {
    // Active facts by predicate prefix
    let facts: Vec<StoredFact> = metadata
        .get_active_facts(account_id, user_id)
        .map_err(meta_err)?;
    let mut facts_by_prefix: HashMap<String, usize> = HashMap::new();
    for f in &facts {
        let prefix = f.predicate.split('.').next().unwrap_or("unknown");
        *facts_by_prefix.entry(prefix.to_owned()).or_insert(0) += 1;
    }
    let total_active_facts = facts.len();

    // Active episodes + low-salience count
    let episodes = metadata
        .get_episodes_by_user(account_id, user_id, None)
        .map_err(meta_err)?;
    let total_active_episodes = episodes.len();
    let low_salience_episodes = episodes
        .iter()
        .filter(|ep| ep.salience_score < LOW_SALIENCE_THRESHOLD)
        .count();

    // Heuristic rules by lifecycle stage
    let all_stages = ["draft", "candidate", "confirmed", "archived"];
    let mut rules_by_stage: HashMap<String, usize> = HashMap::new();
    let mut total_rules = 0;
    for stage in &all_stages {
        let rules = metadata
            .get_active_heuristic_rules(account_id, user_id, &[stage])
            .map_err(meta_err)?;
        let count = rules.len();
        rules_by_stage.insert(stage.to_string(), count);
        total_rules += count;
    }

    Ok(OrientScan {
        facts_by_prefix,
        total_active_facts,
        total_active_episodes,
        low_salience_episodes,
        rules_by_stage,
        total_rules,
    })
}

// ─── Phase 2: Gather ──────────────────────────────────────────────────

/// Consolidate eligible sessions and detect fact conflicts.
pub async fn gather_new_sessions(
    metadata: &MetadataStore,
    account_id: &str,
    user_id: &str,
    agent_id: &str,
    eligible_sessions: &[SessionRow],
    llm: &LlmAssist,
) -> Result<GatherResult, MfsError> {
    let mut sessions_processed = 0;
    let mut new_episodes = 0;
    let mut new_facts = 0;

    // Collect current facts before consolidation for conflict detection
    let predicates_before: Vec<String> = metadata
        .get_active_facts(account_id, user_id)
        .map_err(meta_err)?
        .iter()
        .map(|f| f.predicate.clone())
        .collect();

    for session in eligible_sessions {
        let turns = match metadata.get_turns_by_session(&session.session_id) {
            Ok(t) if !t.is_empty() => t
                .into_iter()
                .map(|t| ConversationTurn {
                    turn_id: t.turn_id,
                    turn_seq: t.turn_seq,
                    session_id: t.session_id,
                    user_id: t.user_id,
                    role: TurnRole::from_str(&t.role),
                    content_text: t.content_text,
                    token_count: t.token_count as usize,
                    created_at: t.created_at,
                })
                .collect::<Vec<_>>(),
            _ => continue,
        };

        match consolidate_and_persist(
            metadata,
            account_id,
            user_id,
            agent_id,
            &session.session_id,
            None,
            &turns,
            llm,
        )
        .await
        {
            Ok(r) => {
                sessions_processed += 1;
                new_episodes += r.episode_count;
                new_facts += r.fact_count;
            }
            Err(e) => {
                warn!("Gather: failed for session {}: {e}", session.session_id);
            }
        }
    }

    // Detect fact conflicts: compare facts_after vs facts_before
    let predicates_after: Vec<String> = metadata
        .get_active_facts(account_id, user_id)
        .map_err(meta_err)?
        .iter()
        .map(|f| f.predicate.clone())
        .collect();

    let mut fact_conflicts = 0;
    let mut conflicting_predicates = Vec::new();

    // Find predicates where count changed (potential superseding)
    let before_map: HashMap<String, usize> =
        predicates_before
            .into_iter()
            .fold(HashMap::new(), |mut m, k| {
                *m.entry(k).or_insert(0) += 1;
                m
            });
    let after_map: HashMap<String, usize> =
        predicates_after
            .into_iter()
            .fold(HashMap::new(), |mut m, k| {
                *m.entry(k).or_insert(0) += 1;
                m
            });

    for (predicate, after_count) in &after_map {
        let before_count = before_map.get(predicate).copied().unwrap_or(0);
        // If a scalar predicate has fewer active facts after consolidation,
        // some were superseded (conflict resolution occurred)
        if after_count < &before_count && *after_count > 0 {
            fact_conflicts += before_count - *after_count;
            conflicting_predicates.push(predicate.clone());
        }
    }

    Ok(GatherResult {
        sessions_processed,
        new_episodes,
        new_facts,
        fact_conflicts,
        conflicting_predicates,
    })
}

// ─── Phase 3: Consolidate ──────────────────────────────────────────────

/// Resolve conflicts, compress stale episodes, evolve heuristic rules.
pub async fn consolidate_conflicts(
    metadata: &MetadataStore,
    account_id: &str,
    user_id: &str,
    agent_id: &str,
    llm: &LlmAssist,
) -> Result<ConsolidateResult, MfsError> {
    // Fact conflicts are already resolved by supersede mechanism in project_assertion
    // during Phase 2. Count superseded facts using the dedicated query.
    let superseded = metadata
        .get_facts_by_status(account_id, user_id, "superseded")
        .map_err(meta_err)?
        .len();

    // Episode compression: find low-salience + zero-recall episodes and merge
    let episodes = metadata
        .get_episodes_by_user(account_id, user_id, None)
        .map_err(meta_err)?;
    let stale_episodes: Vec<&EpisodeRow> = episodes
        .iter()
        .filter(|ep| {
            ep.salience_score < LOW_SALIENCE_THRESHOLD
                && ep.recall_count == 0
                && ep.archived_at.is_none()
        })
        .collect();

    let episodes_archived_stale = if stale_episodes.len() >= 2 {
        // Archive stale low-salience + zero-recall episodes
        // (summaries are preserved in memory brief, not in new merged episode)
        let avg_salience = stale_episodes
            .iter()
            .map(|ep| ep.salience_score)
            .sum::<f64>()
            / stale_episodes.len() as f64;

        // Archive all stale episodes
        let now = chrono::Utc::now().to_rfc3339();
        for ep in &stale_episodes {
            metadata
                .archive_episode(&ep.episode_id, &now)
                .map_err(meta_err)?;
        }

        debug!(
            "Archived {} stale episodes (avg salience {:.3})",
            stale_episodes.len(),
            avg_salience
        );

        stale_episodes.len()
    } else {
        0
    };

    // Sleep consolidation for heuristic rules
    // run_sleep_consolidation returns bare struct; if it fails internally,
    // it returns partial results rather than crashing. Phase 4 still executes.
    let sleep_result = run_sleep_consolidation(metadata, account_id, user_id, agent_id, llm).await;
    let sleep_summary = Some(SleepConsolidationSummary {
        pairs_discovered: sleep_result.pairs_discovered,
        latent_variables_found: sleep_result.latent_variables_found,
        rules_merged: sleep_result.rules_merged,
        rules_archived: sleep_result.rules_archived,
    });

    Ok(ConsolidateResult {
        facts_resolved: superseded,
        episodes_archived_stale,
        sleep_result: sleep_summary,
    })
}

// ─── Phase 4: Prune ──────────────────────────────────────────────────

/// Archive decayed items, refresh briefs, write audit log.
///
/// Phase 4 now runs Ebbinghaus decay BEFORE archival, replacing
/// the old static-threshold comparison. Episodes get their salience
/// recomputed via `decay_episode_salience` (incremental decay), then
/// those below survival_threshold (0.1) are archived by the decay
/// function itself. Phase 4b no longer re-fetches and re-archives —
/// `decay_episode_salience` is the single point of archival decision.
///
/// Also runs fact expiry (`expire_stale_facts`) and access_log
/// pruning (`prune_access_log`) as part of the Dream maintenance cycle.
pub async fn prune_and_index(
    metadata: &MetadataStore,
    account_id: &str,
    user_id: &str,
    agent_id: &str,
) -> Result<PruneResult, MfsError> {
    let mut rules_archived = 0;

    // ── Phase 4a: Ebbinghaus decay for episodes ──
    let decay_config = mfs_types::DecayConfig::default();
    let episode_decay_result =
        crate::episodes::decay_episode_salience(metadata, account_id, user_id, &decay_config);
    let episodes_archived = episode_decay_result.archived;

    // ── Phase 4b: Fact expiry ──
    let fact_expiry_result =
        crate::facts::expire_stale_facts(metadata, account_id, user_id, &decay_config);

    // ── Phase 4c: Prune stale access log entries ──
    // Keep 180 days of access history; older entries contribute near-zero
    // to spacing_factor and slow down get_access_days_since_batch.
    metadata.prune_access_log(180.0).ok();

    // Archive heuristic rules with aggregate_weight below survival threshold
    let rules = metadata
        .list_heuristic_rules(account_id, user_id)
        .map_err(meta_err)?;
    for rule in &rules {
        if rule.aggregate_weight < RULE_SURVIVAL_THRESHOLD
            && rule.lifecycle_stage != "archived"
            && rule.archived_at.is_none()
        {
            metadata
                .update_rule_lifecycle(&rule.rule_id, "archived")
                .map_err(meta_err)?;
            rules_archived += 1;
        }
    }

    // Refresh memory briefs (§5.3 Phase 4 — actual refresh, not just existence check).
    // Uses consolidation::refresh_briefs to rebuild the user-level brief
    // from current episode data, ensuring the brief reflects post-prune state.
    // Note: resource-level briefs are not refreshed here because recent_episodes
    // is empty; only the user-level brief is rebuilt.
    let briefs_refreshed =
        match crate::consolidation::refresh_briefs(metadata, account_id, user_id, &[]) {
            Ok(()) => 1,
            Err(e) => {
                warn!("Dream: brief refresh failed ({e}), continuing");
                0
            }
        };

    // Write audit log (includes Ebbinghaus decay + fact expiry metrics)
    let details = serde_json::json!({
        "episodes_archived": episodes_archived,
        "rules_archived": rules_archived,
        "briefs_refreshed": briefs_refreshed,
        "episode_decay": {
            "decayed": episode_decay_result.decayed,
            "archived_by_decay": episode_decay_result.archived,
        },
        "fact_expiry": {
            "checked": fact_expiry_result.checked,
            "expired": fact_expiry_result.expired,
        },
    })
    .to_string();
    metadata
        .append_audit(&mfs_metadata::AuditEventRecord {
            account_id,
            user_id,
            agent_id: Some(agent_id),
            projection_view_id: None,
            event_type: "dream_prune",
            subject_uri: None,
            actor: Some("dream_prune"),
            details_json: Some(&details),
        })
        .map_err(meta_err)?;

    Ok(PruneResult {
        episodes_archived,
        rules_archived,
        briefs_refreshed,
    })
}

// ─── Main Pipeline ──────────────────────────────────────────────────

/// Execute the full 4-phase dream consolidation pipeline.
pub async fn execute_dream_consolidation(
    metadata: &MetadataStore,
    account_id: &str,
    user_id: &str,
    agent_id: &str,
    eligible_sessions: &[SessionRow],
    llm: &LlmAssist,
) -> Result<DreamReport, MfsError> {
    info!("Dream: Phase 1 — Orient");
    let orient = orient_scan(metadata, account_id, user_id).await?;
    info!(
        "Orient: {} facts, {} episodes ({} low-salience), {} rules",
        orient.total_active_facts,
        orient.total_active_episodes,
        orient.low_salience_episodes,
        orient.total_rules
    );

    info!("Dream: Phase 2 — Gather");
    let gather = gather_new_sessions(
        metadata,
        account_id,
        user_id,
        agent_id,
        eligible_sessions,
        llm,
    )
    .await?;
    info!(
        "Gather: {} sessions → {} episodes, {} facts, {} conflicts",
        gather.sessions_processed, gather.new_episodes, gather.new_facts, gather.fact_conflicts
    );

    info!("Dream: Phase 3 — Consolidate");
    let consolidate =
        match consolidate_conflicts(metadata, account_id, user_id, agent_id, llm).await {
            Ok(r) => r,
            Err(e) => {
                warn!("Dream: Phase 3 failed ({e}), continuing to Phase 4");
                ConsolidateResult {
                    facts_resolved: 0,
                    episodes_archived_stale: 0,
                    sleep_result: None,
                }
            }
        };
    let sleep_desc = match &consolidate.sleep_result {
        Some(s) => format!("{} pairs/{} merged", s.pairs_discovered, s.rules_merged),
        None => "skipped".to_owned(),
    };
    info!(
        "Consolidate: {} facts resolved, {} stale episodes archived, sleep={}",
        consolidate.facts_resolved, consolidate.episodes_archived_stale, sleep_desc,
    );

    info!("Dream: Phase 4 — Prune");
    let prune = prune_and_index(metadata, account_id, user_id, agent_id).await?;
    info!(
        "Prune: {} episodes archived, {} rules archived, briefs_refreshed={}",
        prune.episodes_archived, prune.rules_archived, prune.briefs_refreshed
    );

    Ok(DreamReport {
        orient,
        gather,
        consolidate,
        prune,
    })
}

// ─── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orient_scan_serializes() {
        let scan = OrientScan {
            facts_by_prefix: HashMap::from([
                ("location".to_owned(), 3),
                ("identity".to_owned(), 2),
                ("procedure".to_owned(), 1),
            ]),
            total_active_facts: 6,
            total_active_episodes: 15,
            low_salience_episodes: 3,
            rules_by_stage: HashMap::from([
                ("draft".to_owned(), 2),
                ("candidate".to_owned(), 1),
                ("confirmed".to_owned(), 3),
            ]),
            total_rules: 6,
        };
        let json = serde_json::to_string(&scan).unwrap();
        assert!(json.contains("procedure"));
    }

    #[test]
    fn dream_report_serializes() {
        let report = DreamReport {
            orient: OrientScan {
                facts_by_prefix: HashMap::new(),
                total_active_facts: 0,
                total_active_episodes: 0,
                low_salience_episodes: 0,
                rules_by_stage: HashMap::new(),
                total_rules: 0,
            },
            gather: GatherResult {
                sessions_processed: 2,
                new_episodes: 5,
                new_facts: 3,
                fact_conflicts: 1,
                conflicting_predicates: vec!["location.current_city".to_owned()],
            },
            consolidate: ConsolidateResult {
                facts_resolved: 1,
                episodes_archived_stale: 2,
                sleep_result: Some(SleepConsolidationSummary {
                    pairs_discovered: 1,
                    latent_variables_found: 0,
                    rules_merged: 1,
                    rules_archived: 0,
                }),
            },
            prune: PruneResult {
                episodes_archived: 1,
                rules_archived: 0,
                briefs_refreshed: 1,
            },
        };
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("orient"));
        assert!(json.contains("gather"));
        assert!(json.contains("consolidate"));
        assert!(json.contains("prune"));
    }
}
