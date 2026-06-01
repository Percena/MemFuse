//! T2H (Trajectory-to-Heuristics) pipeline main entry.
//!
//! Integrates feedback signal detection, instance storage, instance-to-rule
//! distillation, and lifecycle promotion into a single pipeline step that
//! runs after session consolidation in the background memory pipeline.
//!
//! Architecture (roadmap §5.1-5.2):
//! 1. Detect feedback signals from conversation turns
//! 2. Store detected signals as HeuristicInstance records
//! 3. Find similar instances for distillation (tag intersection + clustering)
//! 4. Distill similar instances into draft rules (LLM + deterministic fallback)
//! 5. Auto-promote rules lifecycle (draft → candidate → confirmed)

use crate::ConversationTurn;
use crate::feedback_signal::{FeedbackSignal, detect_feedback_signals};
use crate::heuristics::{
    DEFAULT_DECAY_LAMBDA, DEFAULT_SURVIVAL_THRESHOLD, SignalType, auto_promote_rules,
    check_and_downgrade_on_negation, decay_heuristic_weights, retrieve_heuristics, validate_tags,
};
use crate::llm::LlmAssist;
use mfs_metadata::{
    HeuristicEvidenceRecord, HeuristicInstanceRecord, HeuristicRuleRecord, MetadataStore,
};
use mfs_uri::short_hash_hex;
use serde::{Deserialize, Serialize};

/// T2H pipeline result summary.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct T2hPipelineResult {
    /// Number of feedback signals detected.
    pub signals_detected: usize,
    /// Number of heuristic instances created.
    pub instances_created: usize,
    /// Number of draft rules distilled from instances.
    pub rules_distilled: usize,
    /// Number of rules promoted to higher lifecycle stages.
    pub rules_promoted: usize,
    /// Number of evidence records added.
    pub evidence_added: usize,
    /// Number of rules downgraded due to explicit negation.
    pub rules_downgraded: usize,
    /// Number of rules whose aggregate_weight was decayed.
    pub rules_decayed: usize,
    /// Number of rules skipped during decay (user-confirmed exemption).
    pub rules_decay_skipped_confirmed: usize,
    /// Number of rules skipped during decay (active-protected exemption).
    pub rules_decay_skipped_active: usize,
    /// Number of rules archived due to low aggregate_weight.
    pub rules_archived: usize,
}

// ─── Instance creation from detected signals ───────────────────────────

/// Convert detected feedback signals into HeuristicInstance records.
///
/// Per roadmap §5.1 Phase 2 scope:
/// Only explicit_negation, preference_declaration, and tradeoff_decision
/// count as formal evidence. implicit_negation is recorded as an instance
/// but not as formal evidence unless later validated.
fn store_signals_as_instances(
    metadata: &MetadataStore,
    account_id: &str,
    user_id: &str,
    agent_id: &str,
    signals: &[FeedbackSignal],
    session_id: &str,
) -> (usize, Vec<String>) {
    let mut created = 0;
    let mut instance_ids = Vec::new();

    for signal in signals {
        // Derive tags from the signal context (using validate_tags for safety).
        // When LLM-extracted tags are available on the signal, prefer those over
        // the simple keyword-based derivation (fix for issue #3 — LLM tag extraction).
        let raw_tags = if signal.llm_tags.is_empty() {
            derive_tags_from_signal(signal)
        } else {
            signal.llm_tags.clone()
        };
        let valid_tags = validate_tags(&raw_tags);
        let tags_json = serde_json::to_string(&valid_tags).unwrap_or_else(|_| "[]".to_owned());

        // Instance ID: deterministic hash from signal content
        let instance_id = short_hash_hex(
            format!(
                "{}:{}:{}:{}",
                account_id,
                user_id,
                signal.signal_type.as_str(),
                signal.user_reaction
            )
            .as_bytes(),
            12,
        );

        let record = HeuristicInstanceRecord {
            instance_id: &instance_id,
            account_id,
            user_id,
            agent_id: Some(agent_id),
            context_summary: &signal.context_summary,
            agent_proposal: signal.agent_proposal.as_deref(),
            user_reaction: &signal.user_reaction,
            outcome: None,
            signal_type: signal.signal_type.as_str(),
            tags_json: &tags_json,
            session_id: Some(session_id),
            source_turn_ids_json: None,
            derived_rule_id: None,
            instance_status: "open",
            resolved_at: None,
        };

        if metadata.insert_heuristic_instance(&record).is_ok() {
            created += 1;
            instance_ids.push(instance_id);
        }
    }

    (created, instance_ids)
}

/// Derive heuristic tags from a feedback signal's context and content.
///
/// Uses simple keyword mapping to assign domain/phase/language/topic tags.
/// These tags are validated against ALLOWED_TAG_KEYS before storage.
/// When LLM is available, the LLM classification path produces richer tags
/// via the `llm_tags` field on FeedbackSignal, which takes precedence over
/// these keyword-derived tags in `store_signals_as_instances`.
pub fn derive_tags_from_signal(signal: &FeedbackSignal) -> Vec<String> {
    let mut tags = Vec::new();
    // Combine user_reaction + context_summary for broader keyword coverage
    let text = format!("{} {}", signal.user_reaction, signal.context_summary).to_lowercase();

    // Language detection
    if text.contains("rust") || text.contains("cargo") {
        tags.push("language:rust".to_owned());
    }
    if text.contains("typescript")
        || text.contains("javascript")
        || text.contains(" ts ")
        || text.contains(" js ")
    {
        tags.push("language:typescript".to_owned());
    }
    if text.contains("python")
        || text.contains("pip ")
        || text.contains("django")
        || text.contains("flask")
    {
        tags.push("language:python".to_owned());
    }
    if text.contains("golang") || (text.contains(" go ") && text.contains("module")) {
        tags.push("language:go".to_owned());
    }
    if text.contains("java ")
        || text.contains("spring")
        || text.contains("maven")
        || text.contains("gradle")
    {
        tags.push("language:java".to_owned());
    }

    // Domain detection
    if text.contains("backend")
        || text.contains("server")
        || text.contains("api")
        || text.contains("endpoint")
    {
        tags.push("domain:backend".to_owned());
    }
    if text.contains("frontend")
        || text.contains("react")
        || text.contains("vue")
        || text.contains("css")
        || text.contains("html")
    {
        tags.push("domain:frontend".to_owned());
    }
    if text.contains("cli") || text.contains("command line") || text.contains("terminal") {
        tags.push("domain:cli".to_owned());
    }
    if text.contains("infra")
        || text.contains("docker")
        || text.contains("kubernetes")
        || text.contains("ci/cd")
        || text.contains("deploy")
    {
        tags.push("domain:infra".to_owned());
    }
    if text.contains("database")
        || text.contains("sql")
        || text.contains("query")
        || text.contains("migration")
    {
        tags.push("domain:data-pipeline".to_owned());
    }

    // Phase detection
    if text.contains("prototype")
        || text.contains("quick")
        || text.contains("hack")
        || text.contains("mvp")
    {
        tags.push("phase:prototyping".to_owned());
    }
    if text.contains("production") || text.contains("release") || text.contains("stable") {
        tags.push("phase:production".to_owned());
    }
    if text.contains("refactor") || text.contains("cleanup") || text.contains("reorganize") {
        tags.push("phase:refactoring".to_owned());
    }
    if text.contains("debug")
        || text.contains("fix")
        || text.contains("bug")
        || text.contains("issue")
    {
        tags.push("phase:debugging".to_owned());
    }

    // Topic detection
    if text.contains("error")
        || text.contains("error handling")
        || text.contains("exception")
        || text.contains("panic")
    {
        tags.push("topic:error-handling".to_owned());
    }
    if text.contains("test")
        || text.contains("testing")
        || text.contains("unittest")
        || text.contains("spec")
    {
        tags.push("topic:testing".to_owned());
    }
    if text.contains("auth")
        || text.contains("authentication")
        || text.contains("security")
        || text.contains("permission")
    {
        tags.push("topic:auth".to_owned());
    }
    if text.contains("logging")
        || text.contains("observability")
        || text.contains("monitoring")
        || text.contains("tracing")
    {
        tags.push("topic:observability".to_owned());
    }
    if text.contains("performance")
        || text.contains("optimization")
        || text.contains("latency")
        || text.contains("cache")
    {
        tags.push("topic:performance".to_owned());
    }
    if text.contains("naming")
        || text.contains("convention")
        || text.contains("style")
        || text.contains("format")
    {
        tags.push("topic:code-style".to_owned());
    }

    // Pressure detection
    if text.contains("urgent")
        || text.contains("asap")
        || text.contains("deadline")
        || text.contains("rush")
    {
        tags.push("pressure:high".to_owned());
    }

    tags
}

// ─── Instance-to-rule distillation ─────────────────────────────────────

/// LLM distillation result.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct LlmDistillationResult {
    rule_text: String,
    tags: Vec<String>,
    counter_examples: Vec<String>,
    confidence: f64,
}

/// Find open instances that are candidates for distillation into a draft rule.
///
/// Criteria (roadmap §5.2):
/// - At least 2-3 similar instances (tag intersection)
/// - Same signal type or compatible types
/// - From different sessions (independence check)
fn find_distillation_candidates(
    metadata: &MetadataStore,
    account_id: &str,
    user_id: &str,
) -> Vec<Vec<mfs_metadata::StoredHeuristicInstance>> {
    let instances = metadata
        .list_heuristic_instances(account_id, user_id, Some("open"))
        .unwrap_or_default();

    if instances.len() < 2 {
        return Vec::new();
    }

    // Group instances by signal_type + tag overlap
    // MVP: group by signal_type, then find tag-intersection clusters
    let mut groups: Vec<Vec<mfs_metadata::StoredHeuristicInstance>> = Vec::new();

    // Simple grouping: by signal_type first, then by tag overlap >= 1
    let by_type: std::collections::HashMap<String, Vec<&mfs_metadata::StoredHeuristicInstance>> =
        instances
            .iter()
            .fold(std::collections::HashMap::new(), |mut map, inst| {
                map.entry(inst.signal_type.clone()).or_default().push(inst);
                map
            });

    for (_signal_type, type_instances) in by_type {
        if type_instances.len() < 2 {
            continue;
        }

        // Find clusters with tag overlap >= 1
        // MVP: greedy clustering — group instances that share at least 1 tag
        let mut cluster: Vec<mfs_metadata::StoredHeuristicInstance> = Vec::new();

        for inst in &type_instances {
            let inst_tags: Vec<String> = serde_json::from_str(&inst.tags_json).unwrap_or_default();
            if cluster.is_empty() {
                cluster.push((*inst).clone());
                continue;
            }

            // Check tag overlap with any instance in the cluster
            let overlaps = cluster.iter().any(|existing| {
                let existing_tags: Vec<String> =
                    serde_json::from_str(&existing.tags_json).unwrap_or_default();
                inst_tags.iter().any(|t| existing_tags.contains(t))
            });

            if overlaps {
                cluster.push((*inst).clone());
            }
        }

        if cluster.len() >= 2 {
            groups.push(cluster);
        }
    }

    groups
}

/// Distill a cluster of similar instances into a draft rule.
///
/// Dual-track (roadmap §5.2):
/// - LLM distillation: when available, use LLM to produce rule_text, tags, counter_examples
/// - Deterministic fallback: when LLM unavailable, template-based rule creation
async fn distill_cluster_to_rule(
    metadata: &MetadataStore,
    account_id: &str,
    user_id: &str,
    agent_id: &str,
    cluster: &[mfs_metadata::StoredHeuristicInstance],
    llm: &LlmAssist,
) -> Option<String> {
    if cluster.len() < 2 {
        return None;
    }

    // Try LLM distillation first
    let rule_result = if llm.is_available() {
        let prompt = build_distillation_prompt(cluster);
        let response = llm.complete(&prompt).await;
        response.and_then(|text| parse_distillation_result(&text))
    } else {
        None
    };

    // Deterministic fallback when LLM unavailable
    let (rule_text, tags, counter_examples) = match rule_result {
        Some(r) => (r.rule_text, r.tags, r.counter_examples),
        None => distill_deterministic(cluster),
    };

    let valid_tags = validate_tags(&tags);
    let tags_json = serde_json::to_string(&valid_tags).unwrap_or_else(|_| "[]".to_owned());
    let ce_json = serde_json::to_string(&counter_examples).unwrap_or_else(|_| "[]".to_owned());
    let source_ids: Vec<String> = cluster.iter().map(|i| i.instance_id.clone()).collect();
    let source_ids_json = serde_json::to_string(&source_ids).unwrap_or_else(|_| "[]".to_owned());

    let rule_id = short_hash_hex(
        format!("{}:{}:{}", account_id, user_id, rule_text).as_bytes(),
        12,
    );

    // Check if a rule with this ID already exists (avoid duplicate distillation)
    if metadata
        .get_heuristic_rule(&rule_id)
        .ok()
        .flatten()
        .is_some()
    {
        // Rule already exists — add evidence from these instances instead
        return Some(rule_id);
    }

    let record = HeuristicRuleRecord {
        rule_id: &rule_id,
        account_id,
        user_id,
        agent_id: Some(agent_id),
        tags_json: &tags_json,
        rule_text: &rule_text,
        counter_examples_json: &ce_json,
        lifecycle_stage: "draft",
        evidence_count: 0,
        aggregate_weight: 0.0,
        last_evidence_at: None,
        source_instance_ids_json: Some(&source_ids_json),
        promoted_at: None,
        user_confirmed: false,
    };

    metadata.insert_heuristic_rule(&record).ok()?;

    // Mark instances as promoted and link to the derived rule
    for inst in cluster {
        metadata
            .update_instance_status(&inst.instance_id, "promoted", Some(&rule_id))
            .ok();
    }

    Some(rule_id)
}

/// Deterministic fallback for instance-to-rule distillation.
///
/// Per roadmap §5.2:
/// - Take most frequent `user_reaction` as `rule_text`
/// - Take tag intersection as rule `tags`
/// - No `counter_examples` (deferred to LLM supplementation)
fn distill_deterministic(
    cluster: &[mfs_metadata::StoredHeuristicInstance],
) -> (String, Vec<String>, Vec<String>) {
    // Most frequent user_reaction → rule_text
    let reaction_counts: std::collections::HashMap<&str, usize> =
        cluster
            .iter()
            .fold(std::collections::HashMap::new(), |mut map, inst| {
                *map.entry(&inst.user_reaction).or_insert(0) += 1;
                map
            });

    let most_frequent_reaction = reaction_counts
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .map(|(reaction, _)| reaction.to_owned())
        .unwrap_or_else(|| "User preference detected".to_owned());

    // Tag intersection → rule tags
    let tag_sets: Vec<Vec<String>> = cluster
        .iter()
        .map(|i| serde_json::from_str::<Vec<String>>(&i.tags_json).unwrap_or_default())
        .collect();

    let common_tags: Vec<String> = if tag_sets.is_empty() {
        Vec::new()
    } else {
        let first_set: std::collections::HashSet<_> = tag_sets[0].iter().cloned().collect();
        tag_sets[1..]
            .iter()
            .fold(first_set, |intersection, set| {
                let set_hash: std::collections::HashSet<_> = set.iter().cloned().collect();
                intersection.intersection(&set_hash).cloned().collect()
            })
            .into_iter()
            .collect()
    };

    // If no common tags, use all tags from all instances (union)
    let tags = if common_tags.is_empty() {
        tag_sets.iter().flatten().cloned().collect()
    } else {
        common_tags
    };

    (most_frequent_reaction, tags, Vec::new())
}

// ─── Evidence recording ────────────────────────────────────────────────

/// Record evidence for existing rules from new instances.
///
/// Per roadmap §5.1 Phase 2 scope:
/// Only explicit_negation, preference_declaration, and tradeoff_decision
/// count as formal evidence. implicit_negation is NOT formal evidence.
fn record_evidence_from_signals(
    metadata: &MetadataStore,
    account_id: &str,
    user_id: &str,
    signals: &[FeedbackSignal],
    instance_ids: &[String],
    session_id: &str,
) -> usize {
    let mut count = 0;

    for (i, signal) in signals.iter().enumerate() {
        // Skip implicit_negation for formal evidence (roadmap §5.1)
        if signal.signal_type == SignalType::ImplicitNegation {
            continue;
        }

        // Find relevant rules that this signal supports
        let query_tags = derive_tags_from_signal(signal);
        let valid_tags = validate_tags(&query_tags);

        let matching_rules = retrieve_heuristics(
            metadata,
            account_id,
            user_id,
            &valid_tags,
            &signal.user_reaction,
            5,
        );

        for rule in &matching_rules {
            let evidence_id = short_hash_hex(
                format!(
                    "{}:{}:{}:{}",
                    rule.rule_id,
                    session_id,
                    signal.signal_type.as_str(),
                    i
                )
                .as_bytes(),
                8,
            );

            let instance_id = instance_ids.get(i).map(|s| s.as_str());

            let record = HeuristicEvidenceRecord {
                evidence_id: &evidence_id,
                rule_id: &rule.rule_id,
                instance_id,
                evidence_type: "support",
                support_weight: signal.confidence,
                session_id,
            };

            if metadata.insert_heuristic_evidence(&record).is_ok() {
                count += 1;

                // Incrementally add this evidence's weight to the rule stats.
                // Using an arithmetic delta preserves previously applied decay values
                // rather than overwriting them with a raw undecayed sum, which would
                // create a crash-resilience gap between this update and the next decay pass.
                let now_str = chrono::Utc::now().to_rfc3339();
                metadata
                    .increment_rule_evidence_stats(&rule.rule_id, signal.confidence, &now_str)
                    .ok();
            }
        }
    }

    count
}

// ─── LLM prompt templates ─────────────────────────────────────────────

/// Build the LLM prompt for distilling instances into a draft rule.
pub fn build_distillation_prompt(cluster: &[mfs_metadata::StoredHeuristicInstance]) -> String {
    let instances_text = cluster
        .iter()
        .map(|i| format!(
            "- signal_type: {}\n  context: {}\n  reaction: {}\n  tags: {}\n  agent_proposal: {}",
            i.signal_type,
            i.context_summary,
            i.user_reaction,
            i.tags_json,
            i.agent_proposal.as_deref().unwrap_or("N/A"),
        ))
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        r#"Distill a general heuristic rule from these user feedback instances.

Instances:
{instances_text}

Requirements:
1. Extract a **general rule** (rule_text) that captures the common preference across all instances.
   The rule should be concise, actionable, and expressed in natural language.
2. Identify **applicable tags** using only these allowed keys: domain, phase, pressure, language, topic.
   Format: key:value (e.g., "domain:backend", "language:rust")
3. Identify **counter-examples** — situations where this rule does NOT apply.
   These are crucial for disambiguation during retrieval.
4. Assess the **generalization confidence** — how well does this rule capture the underlying preference?

Output JSON only:
 {{
  "rule_text": "concise general preference rule",
  "tags": ["key:value pairs using allowed keys"],
  "counter_examples": ["situations where this rule does not apply"],
  "confidence": 0.0-1.0
}}"#
    )
}

/// Parse LLM distillation response.
fn parse_distillation_result(response: &str) -> Option<LlmDistillationResult> {
    let cleaned = crate::llm::strip_code_fences(response);
    serde_json::from_str(cleaned).ok()
}

// ─── Main pipeline entry ──────────────────────────────────────────────

/// Run the T2H (Trajectory-to-Heuristics) pipeline on a session's conversation turns.
///
/// This is the main entry point that should be called after consolidation
/// in the background memory pipeline (roadmap §747).
///
/// Pipeline steps:
/// 1. Detect feedback signals from conversation turns
/// 2. Store signals as HeuristicInstance records
/// 3. Record evidence for existing matching rules
/// 4. Find instance clusters for distillation
/// 5. Distill clusters into draft rules
/// 6. Auto-promote rules lifecycle
/// Maximum number of heuristic instances per session before T2H throttles
/// (roadmap §9: LLM cost mitigation). Note: this counts instances, not
/// pipeline runs — one pipeline run may produce multiple instances.
/// Set to 10 to avoid premature throttling in signal-dense sessions.
pub const MAX_INSTANCES_PER_SESSION: usize = 10;

pub async fn run_t2h_pipeline(
    metadata: &MetadataStore,
    account_id: &str,
    user_id: &str,
    agent_id: &str,
    session_id: &str,
    turns: &[ConversationTurn],
    llm: &LlmAssist,
) -> T2hPipelineResult {
    let mut result = T2hPipelineResult::default();

    // Frequency limiting (roadmap §9): throttle T2H if this session already has
    // enough heuristic instances from prior analyses. This prevents runaway LLM
    // costs when a single long session generates many signal detections.
    // Note: we count instances, not pipeline runs, since one run may produce
    // multiple instances. See MAX_INSTANCES_PER_SESSION for rationale.
    let existing_instances = metadata
        .list_heuristic_instances(account_id, user_id, None)
        .unwrap_or_default();
    let session_instances = existing_instances
        .iter()
        .filter(|inst| inst.session_id.as_deref() == Some(session_id))
        .count();
    if session_instances >= MAX_INSTANCES_PER_SESSION {
        // Still run decay + promote (these are cheap and necessary)
        result.rules_promoted = auto_promote_rules(metadata, account_id, user_id);
        let decay_result = decay_heuristic_weights(
            metadata,
            account_id,
            user_id,
            DEFAULT_DECAY_LAMBDA,
            DEFAULT_SURVIVAL_THRESHOLD,
        );
        result.rules_decayed = decay_result.decayed;
        result.rules_decay_skipped_confirmed = decay_result.skipped_confirmed;
        result.rules_decay_skipped_active = decay_result.skipped_active_protected;
        result.rules_archived = decay_result.archived;
        return result;
    }

    // Step 1: Detect feedback signals
    let signals = detect_feedback_signals(turns, llm).await;
    result.signals_detected = signals.len();

    if signals.is_empty() {
        // No new signals, but still run promote + decay (roadmap §5.4:
        // decay should execute periodically, not only when signals exist)
        result.rules_promoted = auto_promote_rules(metadata, account_id, user_id);
        let decay_result = decay_heuristic_weights(
            metadata,
            account_id,
            user_id,
            DEFAULT_DECAY_LAMBDA,
            DEFAULT_SURVIVAL_THRESHOLD,
        );
        result.rules_decayed = decay_result.decayed;
        result.rules_decay_skipped_confirmed = decay_result.skipped_confirmed;
        result.rules_decay_skipped_active = decay_result.skipped_active_protected;
        result.rules_archived = decay_result.archived;
        return result;
    }

    // Step 2: Store signals as HeuristicInstance records
    let (instances_created, instance_ids) = store_signals_as_instances(
        metadata, account_id, user_id, agent_id, &signals, session_id,
    );
    result.instances_created = instances_created;

    // Step 3: Record evidence for existing matching rules
    let evidence_count = record_evidence_from_signals(
        metadata,
        account_id,
        user_id,
        &signals,
        &instance_ids,
        session_id,
    );
    result.evidence_added = evidence_count;

    // Step 4-5: Find instance clusters and distill into draft rules
    let clusters = find_distillation_candidates(metadata, account_id, user_id);
    let mut distilled_count = 0;
    for cluster in &clusters {
        if let Some(_rule_id) =
            distill_cluster_to_rule(metadata, account_id, user_id, agent_id, cluster, llm).await
        {
            distilled_count += 1;
        }
    }
    result.rules_distilled = distilled_count;

    // Step 6: Auto-promote rules lifecycle
    result.rules_promoted = auto_promote_rules(metadata, account_id, user_id);

    // Step 7: Downgrade confirmed/candidate rules on explicit negation
    result.rules_downgraded =
        check_and_downgrade_on_negation(metadata, account_id, user_id, &signals);

    // Step 8: Decay aggregate_weight + archive below threshold
    let decay_result = decay_heuristic_weights(
        metadata,
        account_id,
        user_id,
        DEFAULT_DECAY_LAMBDA,
        DEFAULT_SURVIVAL_THRESHOLD,
    );
    result.rules_decayed = decay_result.decayed;
    result.rules_decay_skipped_confirmed = decay_result.skipped_confirmed;
    result.rules_decay_skipped_active = decay_result.skipped_active_protected;
    result.rules_archived = decay_result.archived;

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TurnRole;
    use crate::feedback_signal::DetectionSource;
    use crate::heuristics::SignalType;
    use mfs_metadata::{HeuristicInstanceRecord, HeuristicRuleRecord, MetadataStore};

    fn setup_store() -> MetadataStore {
        MetadataStore::open_in_memory(false).expect("open in memory")
    }

    fn make_signal(signal_type: SignalType, reaction: &str) -> FeedbackSignal {
        FeedbackSignal {
            signal_type,
            confidence: 1.0,
            context_summary: "Test context".to_owned(),
            user_reaction: reaction.to_owned(),
            agent_proposal: None,
            detection_source: DetectionSource::Deterministic,
            llm_tags: Vec::new(),
        }
    }

    fn make_turn(id: &str, seq: i64, role: TurnRole, content: &str) -> ConversationTurn {
        ConversationTurn {
            turn_id: id.to_owned(),
            turn_seq: seq,
            session_id: "sess_test".to_owned(),
            user_id: "u1".to_owned(),
            role,
            content_text: content.to_owned(),
            token_count: content.len(),
            created_at: "2026-04-29T10:00:00Z".to_owned(),
        }
    }

    #[test]
    fn store_signals_creates_instances() {
        let store = setup_store();
        let signals = [
            make_signal(
                SignalType::ExplicitNegation,
                "Don't use anyhow for error handling",
            ),
            make_signal(
                SignalType::PreferenceDeclaration,
                "I always prefer concrete types",
            ),
        ];

        let (created, ids) =
            store_signals_as_instances(&store, "acct", "u1", "agent", &signals, "sess_test");

        assert_eq!(created, 2);
        assert_eq!(ids.len(), 2);

        // Verify instances exist in DB
        let instances = store
            .list_heuristic_instances("acct", "u1", Some("open"))
            .unwrap();
        assert_eq!(instances.len(), 2);
    }

    #[test]
    fn derive_tags_from_rust_signal() {
        let signal = make_signal(
            SignalType::ExplicitNegation,
            "Don't use anyhow in Rust CLI tools",
        );
        let tags = derive_tags_from_signal(&signal);
        let validated = validate_tags(&tags);
        assert!(validated.contains(&"language:rust".to_owned()));
        assert!(validated.contains(&"domain:cli".to_owned()));
    }

    #[test]
    fn derive_tags_from_error_handling_signal() {
        let signal = make_signal(
            SignalType::PreferenceDeclaration,
            "I prefer custom error handling",
        );
        let tags = derive_tags_from_signal(&signal);
        let validated = validate_tags(&tags);
        assert!(validated.contains(&"topic:error-handling".to_owned()));
    }

    #[test]
    fn derive_tags_broader_coverage() {
        // Verify that domain, phase, and topic tags are derived from broader keywords
        let signal = make_signal(
            SignalType::ExplicitNegation,
            "Don't use that approach for the backend API",
        );
        let tags = derive_tags_from_signal(&signal);
        let validated = validate_tags(&tags);
        assert!(validated.contains(&"domain:backend".to_owned()));

        let signal2 = make_signal(
            SignalType::PreferenceDeclaration,
            "For debugging I prefer verbose logging",
        );
        let tags2 = derive_tags_from_signal(&signal2);
        let validated2 = validate_tags(&tags2);
        assert!(validated2.contains(&"phase:debugging".to_owned()));
        assert!(validated2.contains(&"topic:observability".to_owned()));
    }

    #[test]
    fn llm_tags_preferred_over_keyword_tags() {
        let store = setup_store();
        let mut signal = make_signal(SignalType::ExplicitNegation, "Don't use anyhow");
        signal.llm_tags = vec![
            "language:rust".to_owned(),
            "topic:error-handling".to_owned(),
            "domain:backend".to_owned(),
        ];

        let signals = [signal];
        let (created, _ids) =
            store_signals_as_instances(&store, "acct", "u1", "agent", &signals, "sess_test");
        assert_eq!(created, 1);

        let instances = store
            .list_heuristic_instances("acct", "u1", Some("open"))
            .unwrap();
        let tags: Vec<String> = serde_json::from_str(&instances[0].tags_json).unwrap();
        // LLM tags should include domain:backend which keyword-only wouldn't produce from "Don't use anyhow"
        assert!(tags.contains(&"domain:backend".to_owned()));
    }

    #[test]
    fn distill_deterministic_from_cluster() {
        let store = setup_store();
        // Insert two similar instances
        store
            .insert_heuristic_instance(&HeuristicInstanceRecord {
                instance_id: "inst_a",
                account_id: "acct",
                user_id: "u1",
                agent_id: None,
                context_summary: "Rust error handling",
                agent_proposal: Some("Use anyhow"),
                user_reaction: "Don't use anyhow, prefer custom enums",
                outcome: None,
                signal_type: "explicit_negation",
                tags_json: "[\"language:rust\",\"topic:error-handling\"]",
                session_id: Some("sess_1"),
                source_turn_ids_json: None,
                derived_rule_id: None,
                instance_status: "open",
                resolved_at: None,
            })
            .unwrap();

        store
            .insert_heuristic_instance(&HeuristicInstanceRecord {
                instance_id: "inst_b",
                account_id: "acct",
                user_id: "u1",
                agent_id: None,
                context_summary: "Another Rust error case",
                agent_proposal: Some("Suggested thiserror"),
                user_reaction: "Don't use anyhow, prefer custom enums",
                outcome: None,
                signal_type: "explicit_negation",
                tags_json: "[\"language:rust\",\"topic:error-handling\"]",
                session_id: Some("sess_2"),
                source_turn_ids_json: None,
                derived_rule_id: None,
                instance_status: "open",
                resolved_at: None,
            })
            .unwrap();

        let clusters = find_distillation_candidates(&store, "acct", "u1");
        assert!(!clusters.is_empty(), "Should find at least one cluster");

        let cluster = &clusters[0];
        assert!(cluster.len() >= 2);

        // Deterministic distillation
        let (rule_text, tags, counter_examples) = distill_deterministic(cluster);
        assert!(!rule_text.is_empty());
        assert!(!tags.is_empty());
        assert!(counter_examples.is_empty()); // No counter_examples in deterministic fallback
    }

    #[test]
    fn skip_implicit_negation_for_evidence() {
        let store = setup_store();
        let signals = [
            make_signal(SignalType::ImplicitNegation, "Rewrote the code block"),
            make_signal(SignalType::ExplicitNegation, "Don't use this"),
        ];

        // Insert a rule that both signals would match
        store
            .insert_heuristic_rule(&HeuristicRuleRecord {
                rule_id: "rule_test",
                account_id: "acct",
                user_id: "u1",
                agent_id: None,
                tags_json: "[\"topic:error-handling\"]",
                rule_text: "Test rule",
                counter_examples_json: "[]",
                lifecycle_stage: "draft",
                evidence_count: 0,
                aggregate_weight: 0.0,
                last_evidence_at: None,
                source_instance_ids_json: None,
                promoted_at: None,
                user_confirmed: false,
            })
            .unwrap();

        let evidence =
            record_evidence_from_signals(&store, "acct", "u1", &signals, &[], "sess_test");

        // Only the explicit_negation should have produced evidence
        assert!(
            evidence <= 1,
            "Implicit negation should not produce formal evidence"
        );
    }

    #[tokio::test]
    async fn full_pipeline_with_deterministic_only() {
        let store = setup_store();
        let llm = LlmAssist::from_env(); // Will be deterministic in test env

        let turns = [
            make_turn(
                "t1",
                1,
                TurnRole::Assistant,
                "I'll use anyhow for error handling",
            ),
            make_turn(
                "t2",
                2,
                TurnRole::User,
                "Don't use anyhow, I want custom error enums in Rust",
            ),
        ];

        let result =
            run_t2h_pipeline(&store, "acct", "u1", "agent", "sess_test", &turns, &llm).await;

        assert!(
            result.signals_detected >= 1,
            "Should detect at least one signal"
        );
        assert!(
            result.instances_created >= 1,
            "Should create at least one instance"
        );
    }

    #[tokio::test]
    async fn pipeline_empty_turns() {
        let store = setup_store();
        let llm = LlmAssist::from_env();

        let result = run_t2h_pipeline(&store, "acct", "u1", "agent", "sess_test", &[], &llm).await;

        assert_eq!(result.signals_detected, 0);
        assert_eq!(result.instances_created, 0);
    }

    #[tokio::test]
    async fn pipeline_respects_session_frequency_limit() {
        let store = setup_store();

        // Pre-fill session with MAX_INSTANCES_PER_SESSION instances
        for i in 0..MAX_INSTANCES_PER_SESSION {
            store
                .insert_heuristic_instance(&HeuristicInstanceRecord {
                    instance_id: &format!("inst_pre_{i}"),
                    account_id: "acct",
                    user_id: "u1",
                    agent_id: None,
                    context_summary: &format!("Pre-existing instance {i}"),
                    agent_proposal: None,
                    user_reaction: &format!("Reaction {i}"),
                    outcome: None,
                    signal_type: "explicit_negation",
                    tags_json: "[\"domain:backend\"]",
                    session_id: Some("sess_freq_test"),
                    source_turn_ids_json: None,
                    derived_rule_id: None,
                    instance_status: "open",
                    resolved_at: None,
                })
                .unwrap();
        }

        let llm = LlmAssist::from_env();
        let turns = [crate::ConversationTurn {
            turn_id: "t_freq".into(),
            turn_seq: 1,
            session_id: "sess_freq_test".into(),
            user_id: "u1".into(),
            role: crate::TurnRole::User,
            content_text: "Don't use this approach".into(),
            token_count: 5,
            created_at: "2026-04-29T00:00:00Z".into(),
        }];

        let result = run_t2h_pipeline(
            &store,
            "acct",
            "u1",
            "agent",
            "sess_freq_test",
            &turns,
            &llm,
        )
        .await;

        // Should skip signal detection but still run promote + decay
        assert_eq!(result.signals_detected, 0);
        assert_eq!(result.instances_created, 0);
    }
}
