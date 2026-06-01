//! Heuristics module — three-phase retrieval + lifecycle management for heuristic rules.
//!
//! Retrieval strategy (roadmap §5.3):
//! 1. Phase 1: Tag hard-filter (deterministic, O(n) for MVP)
//! 2. Phase 2: Semantic ranking by rule_text embedding similarity
//! 3. Phase 3: Counter-example disambiguation (keyword match + optional embedding)
//!
//! Lifecycle management (roadmap §2.4):
//! Instance → Draft → Candidate → Confirmed → Archived
//! Only rules with sufficient evidence count and independent session coverage
//! may advance to higher lifecycle stages.

use crate::feedback_signal::FeedbackSignal;
use mfs_metadata::{MetadataStore, StoredHeuristicRule};
use serde::{Deserialize, Serialize};

/// Lifecycle stages for heuristic rules (roadmap §2.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LifecycleStage {
    Draft,
    Candidate,
    Confirmed,
    Archived,
}

impl LifecycleStage {
    pub fn as_str(&self) -> &'static str {
        match self {
            LifecycleStage::Draft => "draft",
            LifecycleStage::Candidate => "candidate",
            LifecycleStage::Confirmed => "confirmed",
            LifecycleStage::Archived => "archived",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "draft" => Some(LifecycleStage::Draft),
            "candidate" => Some(LifecycleStage::Candidate),
            "confirmed" => Some(LifecycleStage::Confirmed),
            "archived" => Some(LifecycleStage::Archived),
            _ => None,
        }
    }

    /// Active stages that participate in retrieval.
    pub fn active_stages() -> &'static [&'static str] {
        &["draft", "candidate", "confirmed"]
    }
}

/// Signal types detected from user-agent interaction (roadmap §5.1 + §6.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SignalType {
    ExplicitNegation,
    ImplicitNegation,
    PreferenceDeclaration,
    TradeoffDecision,
    /// User corrects a specific assistant output (§6.2.2).
    CorrectionSignal,
    /// User describes a step-by-step workflow (§6.2.3).
    WorkflowSignal,
    /// Detected repetition of similar content across sessions.
    RepetitionSignal,
}

impl SignalType {
    pub fn as_str(&self) -> &'static str {
        match self {
            SignalType::ExplicitNegation => "explicit_negation",
            SignalType::ImplicitNegation => "implicit_negation",
            SignalType::PreferenceDeclaration => "preference_declaration",
            SignalType::TradeoffDecision => "tradeoff_decision",
            SignalType::CorrectionSignal => "correction_signal",
            SignalType::WorkflowSignal => "workflow_signal",
            SignalType::RepetitionSignal => "repetition_signal",
        }
    }
}

/// Evidence types for rule validation (roadmap §4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvidenceType {
    Support,
    Contradict,
    /// Positive confirmation — "user accepted without modification".
    /// TODO(roadmap §2.1): MVP does not auto-generate PositiveConfirm evidence.
    /// This variant is reserved for future use when the system has a stable
    /// mechanism for attributing "no correction" to a specific rule injection.
    PositiveConfirm,
}

impl EvidenceType {
    pub fn as_str(&self) -> &'static str {
        match self {
            EvidenceType::Support => "support",
            EvidenceType::Contradict => "contradict",
            EvidenceType::PositiveConfirm => "positive_confirm",
        }
    }
}

/// A heuristic rule entry ready for injection into agent context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeuristicEntry {
    pub rule_id: String,
    pub rule_text: String,
    pub tags: Vec<String>,
    pub counter_examples: Vec<String>,
    pub lifecycle_stage: String,
    pub evidence_count: i64,
    pub aggregate_weight: f64,
    pub user_confirmed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
}

impl From<StoredHeuristicRule> for HeuristicEntry {
    fn from(r: StoredHeuristicRule) -> Self {
        HeuristicEntry {
            rule_id: r.rule_id,
            rule_text: r.rule_text,
            tags: serde_json::from_str(&r.tags_json).unwrap_or_default(),
            counter_examples: serde_json::from_str(&r.counter_examples_json).unwrap_or_default(),
            lifecycle_stage: r.lifecycle_stage,
            evidence_count: r.evidence_count,
            aggregate_weight: r.aggregate_weight,
            user_confirmed: r.user_confirmed,
            created_at: Some(r.created_at),
        }
    }
}

/// Instance status for heuristic instances.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InstanceStatus {
    Open,
    Promoted,
    Dismissed,
    Expired,
}

impl InstanceStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            InstanceStatus::Open => "open",
            InstanceStatus::Promoted => "promoted",
            InstanceStatus::Dismissed => "dismissed",
            InstanceStatus::Expired => "expired",
        }
    }
}

// ─── Three-phase Retrieval ────────────────────────────────────────────

/// Retrieve heuristic rules matching a query context via three-phase pipeline.
///
/// Phase 1: Tag hard-filter — load active rules, compute tag overlap specificity
/// Phase 2: Semantic ranking — sort by tag overlap count (MVP; embedding ranking deferred)
/// Phase 3: Counter-example disambiguation — exclude rules whose counter_examples match context
pub fn retrieve_heuristics(
    metadata: &MetadataStore,
    account_id: &str,
    user_id: &str,
    query_tags: &[String],
    query_text: &str,
    top_k: usize,
) -> Vec<HeuristicEntry> {
    // Phase 1: Load active rules by lifecycle stage
    let active_rules = metadata
        .get_active_heuristic_rules(account_id, user_id, LifecycleStage::active_stages())
        .unwrap_or_default();

    // Phase 1 continued: Tag hard-filter — compute tag overlap
    let query_tag_set: Vec<String> = query_tags.iter().map(|t| t.to_lowercase()).collect();
    let filtered: Vec<(StoredHeuristicRule, usize)> = active_rules
        .into_iter()
        .filter_map(|rule| {
            let rule_tags: Vec<String> = serde_json::from_str::<Vec<String>>(&rule.tags_json)
                .unwrap_or_default()
                .into_iter()
                .map(|t: String| t.to_lowercase())
                .collect();
            let overlap = rule_tags
                .iter()
                .filter(|rt| query_tag_set.iter().any(|qt| qt == rt.as_str()))
                .count();
            // If no query tags specified, keep all (no filtering)
            // If query tags specified but overlap is 0, filter out
            if query_tag_set.is_empty() || overlap > 0 {
                Some((rule, overlap))
            } else {
                None
            }
        })
        .collect();

    // Phase 2: Semantic ranking (MVP: tag overlap + keyword match in rule_text)
    let mut ranked: Vec<(StoredHeuristicRule, usize, f64)> = filtered
        .into_iter()
        .map(|(rule, overlap)| {
            // Simple keyword matching for MVP (embedding deferred to Phase 3 roadmap)
            let lower_rule = rule.rule_text.to_lowercase();
            let lower_query = query_text.to_lowercase();
            let keyword_score = if query_text.is_empty() {
                0.0
            } else {
                lower_query
                    .split(|ch: char| !ch.is_alphanumeric())
                    .filter(|term| !term.is_empty() && lower_rule.contains(term))
                    .count() as f64
            };
            let specificity_score = overlap as f64;
            let final_score = specificity_score * 2.0 + keyword_score + rule.aggregate_weight * 0.1;
            (rule, overlap, final_score)
        })
        .collect();

    ranked.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
    ranked.truncate(top_k);

    // Phase 3: Counter-example disambiguation
    let lower_query = query_text.to_lowercase();
    let phase3: Vec<HeuristicEntry> = ranked
        .into_iter()
        .filter(|(rule, _, _)| {
            let counter_examples: Vec<String> =
                serde_json::from_str(&rule.counter_examples_json).unwrap_or_default();
            !counter_examples.iter().any(|ce| {
                let lower_ce = ce.to_lowercase();
                lower_query
                    .split(|ch: char| !ch.is_alphanumeric())
                    .filter(|term| !term.is_empty())
                    .any(|term| lower_ce.contains(term))
            })
        })
        .map(|(rule, _, _)| HeuristicEntry {
            rule_id: rule.rule_id,
            rule_text: rule.rule_text,
            tags: serde_json::from_str(&rule.tags_json).unwrap_or_default(),
            counter_examples: serde_json::from_str(&rule.counter_examples_json).unwrap_or_default(),
            lifecycle_stage: rule.lifecycle_stage,
            evidence_count: rule.evidence_count,
            aggregate_weight: rule.aggregate_weight,
            user_confirmed: rule.user_confirmed,
            created_at: None,
        })
        .collect();

    // Phase 4: Tag-group conflict resolution (roadmap §9)
    // "同一 tag 组合下只保留最高 lifecycle_stage 的规则"
    // When multiple rules share the same tag set, only the highest lifecycle_stage survives.
    // Lifecycle priority: confirmed > candidate > draft.
    let stage_rank = |stage: &str| -> i32 {
        match stage {
            "confirmed" => 3,
            "candidate" => 2,
            "draft" => 1,
            _ => 0,
        }
    };

    let mut best_by_tags: std::collections::HashMap<Vec<String>, HeuristicEntry> =
        std::collections::HashMap::new();
    for entry in phase3 {
        let key = {
            let mut sorted = entry.tags.clone();
            sorted.sort();
            sorted
        };
        let owned_entry = entry.clone();
        best_by_tags
            .entry(key)
            .and_modify(|existing| {
                // Keep the one with higher lifecycle stage; on tie, keep higher aggregate_weight
                if stage_rank(&owned_entry.lifecycle_stage) > stage_rank(&existing.lifecycle_stage)
                    || (stage_rank(&owned_entry.lifecycle_stage)
                        == stage_rank(&existing.lifecycle_stage)
                        && owned_entry.aggregate_weight > existing.aggregate_weight)
                {
                    *existing = owned_entry.clone();
                }
            })
            .or_insert(entry);
    }

    let result: Vec<HeuristicEntry> = best_by_tags.into_values().collect();
    result
}

/// Retrieve L0-level confirmed rules for session-start injection.
/// Returns top-N confirmed rules by aggregate_weight.
pub fn retrieve_l0_confirmed(
    metadata: &MetadataStore,
    account_id: &str,
    user_id: &str,
    max_rules: usize,
) -> Vec<HeuristicEntry> {
    let confirmed = metadata
        .get_active_heuristic_rules(account_id, user_id, &["confirmed"])
        .unwrap_or_default();

    confirmed
        .into_iter()
        .take(max_rules)
        .map(|rule| HeuristicEntry {
            rule_id: rule.rule_id,
            rule_text: rule.rule_text,
            tags: serde_json::from_str(&rule.tags_json).unwrap_or_default(),
            counter_examples: serde_json::from_str(&rule.counter_examples_json).unwrap_or_default(),
            lifecycle_stage: rule.lifecycle_stage,
            evidence_count: rule.evidence_count,
            aggregate_weight: rule.aggregate_weight,
            user_confirmed: rule.user_confirmed,
            created_at: None,
        })
        .collect()
}

// ─── Lifecycle Management ─────────────────────────────────────────────

/// Check whether a draft rule meets promotion criteria to candidate.
/// Requirements (roadmap §2.4):
/// - 5+ supporting evidence from independent sessions
/// - No contradicting evidence
/// - Evidence spans at least 2 days
pub fn can_promote_draft_to_candidate(metadata: &MetadataStore, rule_id: &str) -> bool {
    let evidence = metadata.list_evidence_for_rule(rule_id).unwrap_or_default();

    let supporting = evidence
        .iter()
        .filter(|e| e.evidence_type == "support")
        .count();
    let contradicting = evidence
        .iter()
        .filter(|e| e.evidence_type == "contradict")
        .count();

    if supporting < 5 || contradicting > 0 {
        return false;
    }

    // Check independent sessions (at least 2 unique sessions)
    let unique_sessions: std::collections::HashSet<_> =
        evidence.iter().map(|e| e.session_id.clone()).collect();
    if unique_sessions.len() < 2 {
        return false;
    }

    // Check time span (at least 2 days between earliest and latest evidence)
    let dates: Vec<&str> = evidence.iter().map(|e| e.created_at.as_str()).collect();
    if dates.len() >= 2 {
        // Simple check: different date prefixes (YYYY-MM-DD)
        let date_prefixes: std::collections::HashSet<_> =
            dates.iter().map(|d| d.get(..10).unwrap_or(d)).collect();
        if date_prefixes.len() < 2 {
            return false;
        }
    }

    true
}

/// Check whether a candidate rule meets promotion criteria to confirmed.
/// Requirements: 10+ supporting evidence, cross-session validation, no contradictions.
pub fn can_promote_candidate_to_confirmed(metadata: &MetadataStore, rule_id: &str) -> bool {
    let evidence = metadata.list_evidence_for_rule(rule_id).unwrap_or_default();

    let supporting = evidence
        .iter()
        .filter(|e| e.evidence_type == "support")
        .count();
    let contradicting = evidence
        .iter()
        .filter(|e| e.evidence_type == "contradict")
        .count();

    if supporting < 10 || contradicting > 0 {
        return false;
    }

    // Check independent sessions (at least 3 unique sessions)
    let unique_sessions: std::collections::HashSet<_> =
        evidence.iter().map(|e| e.session_id.clone()).collect();
    if unique_sessions.len() < 3 {
        return false;
    }

    // Check time span (at least 2 different days)
    let date_prefixes: std::collections::HashSet<_> = evidence
        .iter()
        .map(|e| e.created_at.get(..10).unwrap_or("").to_owned())
        .collect();
    if date_prefixes.len() < 2 {
        return false;
    }

    true
}

/// Attempt automatic lifecycle promotion for all draft/candidate rules.
pub fn auto_promote_rules(metadata: &MetadataStore, account_id: &str, user_id: &str) -> usize {
    let draft_rules = metadata
        .get_active_heuristic_rules(account_id, user_id, &["draft"])
        .unwrap_or_default();

    let candidate_rules = metadata
        .get_active_heuristic_rules(account_id, user_id, &["candidate"])
        .unwrap_or_default();

    let mut promoted_count = 0;

    for rule in &draft_rules {
        if can_promote_draft_to_candidate(metadata, &rule.rule_id)
            && metadata
                .update_rule_lifecycle(&rule.rule_id, "candidate")
                .is_ok()
        {
            promoted_count += 1;
        }
    }

    for rule in &candidate_rules {
        if can_promote_candidate_to_confirmed(metadata, &rule.rule_id)
            && metadata
                .update_rule_lifecycle(&rule.rule_id, "confirmed")
                .is_ok()
        {
            promoted_count += 1;
        }
    }

    promoted_count
}

// ─── Exponential Decay (roadmap §5.4) ─────────────────────────────────

/// Default decay coefficient λ. Half-life ≈ 35 days.
pub const DEFAULT_DECAY_LAMBDA: f64 = 0.02;

/// Default survival threshold for rule archival.
pub const DEFAULT_SURVIVAL_THRESHOLD: f64 = 0.5;

/// Compute decayed evidence weight using exponential forgetting curve.
///
/// W_t = W_0 × e^(-λt)
///
/// where t is elapsed days since evidence creation, λ is the decay rate.
/// A λ of 0.02 gives a half-life of ~35 days (ln(2)/0.02 ≈ 34.7).
pub fn compute_evidence_decay(w0: f64, days_elapsed: f64, lambda: f64) -> f64 {
    w0 * std::f64::consts::E.powf(-lambda * days_elapsed)
}

/// Result of decay + archival maintenance pass.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecayMaintenanceResult {
    /// Number of rules whose aggregate_weight was recomputed.
    pub decayed: usize,
    /// Number of rules skipped because user explicitly confirmed them (roadmap §5.4).
    pub skipped_confirmed: usize,
    /// Number of rules skipped because they have recent evidence (active-protected, §5.4).
    pub skipped_active_protected: usize,
    /// Number of rules archived due to aggregate_weight below survival threshold.
    pub archived: usize,
}

/// Recompute aggregate_weight for all active heuristic rules using exponential decay.
///
/// For each rule:
/// 1. Load all evidence records
/// 2. For each evidence, compute W_t = support_weight × e^(-λ × days_since_created)
/// 3. Sum all decayed weights → new aggregate_weight
/// 4. If aggregate_weight < survival_threshold → archive the rule
///
/// **Decay exemptions** (roadmap §5.4):
/// - Confirmed rules skip decay entirely (user explicitly validated them)
/// - Rules with evidence within the last 7 days are "active-protected" and skip decay
pub fn decay_heuristic_weights(
    metadata: &MetadataStore,
    account_id: &str,
    user_id: &str,
    lambda: f64,
    survival_threshold: f64,
) -> DecayMaintenanceResult {
    let active_rules = metadata
        .get_active_heuristic_rules(account_id, user_id, LifecycleStage::active_stages())
        .unwrap_or_default();

    let mut decayed = 0;
    let mut skipped_confirmed = 0;
    let mut skipped_active_protected = 0;
    let mut archived = 0;
    let now = chrono::Utc::now();
    let active_protection_days = 7.0;

    for rule in &active_rules {
        // ── Decay exemption: user-confirmed rules (roadmap §5.4) ──
        // Per §5.4: "用户显式确认的规则不参与自动衰减"
        // This checks user_confirmed (set via confirm_rule MCP tool),
        // NOT lifecycle_stage=="confirmed" which is reached by auto-promotion.
        //
        // Design rationale: auto-promotion to lifecycle_stage "confirmed" is a
        // mechanical threshold crossing (evidence_count + session_span), not an
        // explicit user endorsement. Only the confirm_rule MCP tool (which
        // requires intentional user action) grants permanent decay exemption.
        // An auto-promoted rule that receives no new evidence within 7 days
        // (active-protection window) will gradually decay, which is intentional —
        // stale auto-promoted rules should not persist indefinitely.
        if rule.user_confirmed {
            skipped_confirmed += 1;
            continue;
        }

        let evidence = metadata
            .list_evidence_for_rule(&rule.rule_id)
            .unwrap_or_default();

        // ── Decay exemption: active-protected rules ──
        // Per §5.4: "最近7天内有新evidence的规则不参与衰减"
        let has_recent_evidence = evidence.iter().any(|ev| {
            let days = parse_days_elapsed(&ev.created_at, &now);
            days <= active_protection_days
        });
        if has_recent_evidence {
            skipped_active_protected += 1;
            continue;
        }

        let mut new_weight = 0.0;
        let mut latest_evidence_at: Option<String> = None;

        for ev in &evidence {
            let days_elapsed = parse_days_elapsed(&ev.created_at, &now);
            let decayed_weight = compute_evidence_decay(ev.support_weight, days_elapsed, lambda);
            new_weight += decayed_weight;

            if latest_evidence_at
                .as_deref()
                .is_none_or(|latest| ev.created_at.as_str() > latest)
            {
                latest_evidence_at = Some(ev.created_at.clone());
            }
        }

        let evidence_count = evidence.len() as i64;
        metadata
            .update_rule_evidence_stats(
                &rule.rule_id,
                evidence_count,
                new_weight,
                latest_evidence_at.as_deref(),
            )
            .ok();
        decayed += 1;

        // Archive rules whose aggregate_weight falls below survival threshold
        if new_weight < survival_threshold {
            metadata
                .update_rule_lifecycle(&rule.rule_id, "archived")
                .ok();
            archived += 1;
        }
    }

    DecayMaintenanceResult {
        decayed,
        skipped_confirmed,
        skipped_active_protected,
        archived,
    }
}

/// Parse a timestamp string and compute days elapsed from now.
/// Handles common ISO 8601 formats (with or without timezone suffix).
fn parse_days_elapsed(created_at: &str, now: &chrono::DateTime<chrono::Utc>) -> f64 {
    // Try parsing common timestamp formats
    let parsed = chrono::NaiveDateTime::parse_from_str(created_at, "%Y-%m-%dT%H:%M:%SZ")
        .or_else(|_| chrono::NaiveDateTime::parse_from_str(created_at, "%Y-%m-%dT%H:%M:%S"))
        .or_else(|_| chrono::NaiveDateTime::parse_from_str(created_at, "%Y-%m-%d %H:%M:%S"));

    match parsed {
        Ok(dt) => {
            let created =
                chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(dt, chrono::Utc);
            let duration = now.signed_duration_since(created);
            duration.num_days() as f64
        }
        Err(_) => 0.0, // Unparseable timestamp → assume recent (0 days elapsed)
    }
}

// ─── Rule Downgrade (roadmap §9 risk mitigation) ─────────────────────

/// Downgrade a confirmed/candidate rule to draft when the user explicitly negates it.
///
/// Per roadmap §5.4: "用户显式否定 confirmed 规则 → 立即降级为 draft"
/// This ensures that a rule that the user has explicitly rejected doesn't
/// continue to influence the agent's behavior at a high lifecycle stage.
pub fn downgrade_rule_on_negation(metadata: &MetadataStore, rule_id: &str) {
    metadata.update_rule_lifecycle(rule_id, "draft").ok();
}

/// Check if any detected explicit negation signal matches a confirmed/candidate
/// rule, and downgrade those rules accordingly.
///
/// Called from the T2H pipeline when explicit_negation signals are detected.
pub fn check_and_downgrade_on_negation(
    metadata: &MetadataStore,
    account_id: &str,
    user_id: &str,
    negation_signals: &[FeedbackSignal],
) -> usize {
    let mut downgraded = 0;

    for signal in negation_signals {
        if signal.signal_type != crate::heuristics::SignalType::ExplicitNegation {
            continue;
        }

        // Find confirmed/candidate rules that match the negation context
        let query_tags = crate::t2h::derive_tags_from_signal(signal);
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
            if rule.lifecycle_stage == "confirmed" || rule.lifecycle_stage == "candidate" {
                downgrade_rule_on_negation(metadata, &rule.rule_id);
                downgraded += 1;
            }
        }
    }

    downgraded
}

// ─── Markdown Rendering ───────────────────────────────────────────────

/// Render heuristic rules as a markdown section for injection.
pub fn render_heuristics_section(entries: &[HeuristicEntry]) -> String {
    if entries.is_empty() {
        return String::new();
    }

    let mut output = String::from("[Behavioral Heuristics]\n");

    for entry in entries {
        // Lifecycle stage marker: ★ confirmed, ◆ candidate, ○ draft
        let stage_marker = match entry.lifecycle_stage.as_str() {
            "confirmed" => "★",
            "candidate" => "◆",
            "draft" => "○",
            _ => "?",
        };
        output.push_str("- ");
        output.push_str(stage_marker);
        output.push(' ');
        output.push_str(&entry.rule_text);

        // Tag signal for context matching
        if !entry.tags.is_empty() {
            output.push_str(" 📍 [");
            output.push_str(&entry.tags.join(", "));
            output.push(']');
        }

        // Counter-example hint
        if !entry.counter_examples.is_empty() {
            output.push_str(" ⚠️ except: ");
            output.push_str(&entry.counter_examples.join("; "));
        }

        output.push('\n');
    }

    output.trim_end().to_owned()
}

// ─── Allowed Tag Keys (roadmap §10.1) ─────────────────────────────────

/// MVP allowed tag keys to prevent tag key explosion.
pub const ALLOWED_TAG_KEYS: &[&str] = &[
    "domain",   // backend, frontend, cli, infra, data-pipeline, ...
    "phase",    // prototyping, production, refactoring, debugging, ...
    "pressure", // low, normal, high, critical
    "language", // rust, typescript, python, ...
    "topic",    // error-handling, auth, testing, ...
];

/// Validate that tags use only allowed keys.
pub fn validate_tags(tags: &[String]) -> Vec<String> {
    tags.iter()
        .filter(|tag| {
            let key = tag.split(':').next().unwrap_or("");
            ALLOWED_TAG_KEYS.contains(&key)
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mfs_metadata::{
        HeuristicEvidenceRecord, HeuristicInstanceRecord, HeuristicRuleRecord, MetadataStore,
    };

    fn setup_store() -> MetadataStore {
        MetadataStore::open_in_memory(false).expect("open in memory")
    }

    #[test]
    fn lifecycle_stage_conversion() {
        assert_eq!(LifecycleStage::Draft.as_str(), "draft");
        assert_eq!(
            LifecycleStage::from_str("confirmed"),
            Some(LifecycleStage::Confirmed)
        );
        assert_eq!(LifecycleStage::from_str("unknown"), None);
        assert_eq!(
            LifecycleStage::active_stages(),
            &["draft", "candidate", "confirmed"]
        );
    }

    #[test]
    fn signal_type_conversion() {
        assert_eq!(SignalType::ExplicitNegation.as_str(), "explicit_negation");
        assert_eq!(
            SignalType::PreferenceDeclaration.as_str(),
            "preference_declaration"
        );
    }

    #[test]
    fn evidence_type_conversion() {
        assert_eq!(EvidenceType::Support.as_str(), "support");
        assert_eq!(EvidenceType::Contradict.as_str(), "contradict");
    }

    #[test]
    fn validate_tags_filters_disallowed_keys() {
        let tags = vec![
            "domain:backend".to_owned(),
            "scope:server".to_owned(), // disallowed key
            "language:rust".to_owned(),
            "topic:error-handling".to_owned(),
            "area:api".to_owned(), // disallowed key
        ];
        let validated = validate_tags(&tags);
        assert_eq!(
            validated,
            vec![
                "domain:backend".to_owned(),
                "language:rust".to_owned(),
                "topic:error-handling".to_owned(),
            ]
        );
    }

    #[test]
    fn render_heuristics_section_basic() {
        let entries = vec![
            HeuristicEntry {
                rule_id: "r1".to_owned(),
                rule_text: "User prefers concrete types over interfaces in Rust CLI tools"
                    .to_owned(),
                tags: vec!["domain:cli".to_owned(), "language:rust".to_owned()],
                counter_examples: vec![
                    "But for public library crates, user uses trait-based abstractions".to_owned(),
                ],
                lifecycle_stage: "confirmed".to_owned(),
                evidence_count: 12,
                aggregate_weight: 8.5,
                user_confirmed: true,
                created_at: None,
            },
            HeuristicEntry {
                rule_id: "r2".to_owned(),
                rule_text: "User prefers pragmatic solutions when prototyping".to_owned(),
                tags: vec!["phase:prototyping".to_owned()],
                counter_examples: Vec::new(),
                lifecycle_stage: "draft".to_owned(),
                evidence_count: 2,
                aggregate_weight: 1.0,
                user_confirmed: false,
                created_at: None,
            },
        ];
        let rendered = render_heuristics_section(&entries);
        assert!(rendered.contains("[Behavioral Heuristics]"));
        assert!(rendered.contains("★ User prefers concrete types"));
        assert!(rendered.contains("○ User prefers pragmatic"));
        assert!(rendered.contains("📍 [domain:cli, language:rust]"));
        assert!(rendered.contains("⚠️ except:"));
    }

    #[test]
    fn render_heuristics_empty() {
        let rendered = render_heuristics_section(&[]);
        assert!(rendered.is_empty());
    }

    #[test]
    fn insert_and_retrieve_heuristic_rule() {
        let store = setup_store();
        store
            .insert_heuristic_rule(&HeuristicRuleRecord {
                rule_id: "rule_test_01",
                account_id: "test",
                user_id: "user1",
                agent_id: None,
                tags_json: "[\"domain:backend\",\"language:rust\"]",
                rule_text: "User prefers custom error enums over anyhow in Rust CLI tools",
                counter_examples_json: "[\"But for quick scripts, user tolerates anyhow\"]",
                lifecycle_stage: "draft",
                evidence_count: 0,
                aggregate_weight: 0.0,
                last_evidence_at: None,
                source_instance_ids_json: None,
                promoted_at: None,
                user_confirmed: false,
            })
            .expect("insert rule");

        let rule = store
            .get_heuristic_rule("rule_test_01")
            .expect("get rule")
            .expect("rule exists");
        assert_eq!(rule.rule_id, "rule_test_01");
        assert_eq!(rule.lifecycle_stage, "draft");
        assert_eq!(rule.tags_json, "[\"domain:backend\",\"language:rust\"]");
        assert_eq!(
            rule.rule_text,
            "User prefers custom error enums over anyhow in Rust CLI tools"
        );
    }

    #[test]
    fn retrieve_heuristics_with_tag_filter() {
        let store = setup_store();
        // Insert test rules
        store
            .insert_heuristic_rule(&HeuristicRuleRecord {
                rule_id: "r_backend",
                account_id: "acct",
                user_id: "u1",
                agent_id: None,
                tags_json: "[\"domain:backend\",\"language:rust\"]",
                rule_text: "Use custom error enums in backend Rust projects",
                counter_examples_json: "[]",
                lifecycle_stage: "confirmed",
                evidence_count: 10,
                aggregate_weight: 5.0,
                last_evidence_at: None,
                source_instance_ids_json: None,
                promoted_at: None,
                user_confirmed: false,
            })
            .unwrap();

        store
            .insert_heuristic_rule(&HeuristicRuleRecord {
                rule_id: "r_frontend",
                account_id: "acct",
                user_id: "u1",
                agent_id: None,
                tags_json: "[\"domain:frontend\",\"language:typescript\"]",
                rule_text: "Use React hooks pattern in frontend",
                counter_examples_json: "[]",
                lifecycle_stage: "candidate",
                evidence_count: 7,
                aggregate_weight: 3.0,
                last_evidence_at: None,
                source_instance_ids_json: None,
                promoted_at: None,
                user_confirmed: false,
            })
            .unwrap();

        // Retrieve rules matching "domain:backend" tag
        let results = retrieve_heuristics(
            &store,
            "acct",
            "u1",
            &["domain:backend".to_owned(), "language:rust".to_owned()],
            "Rust backend error handling",
            5,
        );

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].rule_id, "r_backend");
        assert_eq!(results[0].lifecycle_stage, "confirmed");
    }

    #[test]
    fn retrieve_heuristics_counter_example_disambiguation() {
        let store = setup_store();

        // Rule with a counter-example that matches the query
        store.insert_heuristic_rule(&HeuristicRuleRecord {
            rule_id: "r_prototype",
            account_id: "acct",
            user_id: "u1",
            agent_id: None,
            tags_json: "[\"phase:prototyping\"]",
            rule_text: "Use quick hacks when prototyping",
            counter_examples_json: "[\"But for production code, user insists on proper types\"]",
            lifecycle_stage: "candidate",
            evidence_count: 5,
            aggregate_weight: 3.0,
            last_evidence_at: None,
            source_instance_ids_json: None,
            promoted_at: None,
            user_confirmed: false,
        }).unwrap();

        // Query about production code — should be excluded by counter-example
        let results = retrieve_heuristics(
            &store,
            "acct",
            "u1",
            &["phase:prototyping".to_owned()],
            "production code types",
            5,
        );

        assert!(
            results.is_empty(),
            "Rule should be excluded by counter-example containing 'production'"
        );
    }

    #[test]
    fn lifecycle_promotion_rules() {
        let store = setup_store();

        store
            .insert_heuristic_rule(&HeuristicRuleRecord {
                rule_id: "rule_promote",
                account_id: "acct",
                user_id: "u1",
                agent_id: None,
                tags_json: "[\"domain:backend\"]",
                rule_text: "Test rule for promotion",
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

        // Add 5 supporting evidence from 2 different sessions spanning 2 days
        for i in 0..5 {
            let session = if i < 3 { "sess_a" } else { "sess_b" };
            store
                .insert_heuristic_evidence(&HeuristicEvidenceRecord {
                    evidence_id: &format!("ev_{i}"),
                    rule_id: "rule_promote",
                    instance_id: None,
                    evidence_type: "support",
                    support_weight: 1.0,
                    session_id: session,
                })
                .unwrap();
            // Note: created_at uses CURRENT_TIMESTAMP in DB, so all evidence gets the same
            // timestamp. The time-span promotion check (≥2 distinct days) will therefore
            // fail in this test. That's expected — production evidence accumulates over days.
        }

        // The promotion will fail due to all evidence having the same CURRENT_TIMESTAMP
        // This is expected behavior for the MVP
        assert!(!can_promote_draft_to_candidate(&store, "rule_promote"));

        // But if we skip the time span check (simulating real-world different timestamps),
        // the other criteria should pass
        let evidence = store.list_evidence_for_rule("rule_promote").unwrap();
        let supporting = evidence
            .iter()
            .filter(|e| e.evidence_type == "support")
            .count();
        assert_eq!(supporting, 5);
        let unique_sessions: std::collections::HashSet<_> =
            evidence.iter().map(|e| e.session_id.clone()).collect();
        assert_eq!(unique_sessions.len(), 2);
    }

    #[test]
    fn update_rule_lifecycle() {
        let store = setup_store();
        store
            .insert_heuristic_rule(&HeuristicRuleRecord {
                rule_id: "rule_lifecycle",
                account_id: "acct",
                user_id: "u1",
                agent_id: None,
                tags_json: "[\"domain:backend\"]",
                rule_text: "Lifecycle test rule",
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

        store
            .update_rule_lifecycle("rule_lifecycle", "candidate")
            .unwrap();
        let rule = store.get_heuristic_rule("rule_lifecycle").unwrap().unwrap();
        assert_eq!(rule.lifecycle_stage, "candidate");
        assert!(rule.promoted_at.is_some());

        store
            .update_rule_lifecycle("rule_lifecycle", "confirmed")
            .unwrap();
        let rule = store.get_heuristic_rule("rule_lifecycle").unwrap().unwrap();
        assert_eq!(rule.lifecycle_stage, "confirmed");

        store
            .update_rule_lifecycle("rule_lifecycle", "archived")
            .unwrap();
        let rule = store.get_heuristic_rule("rule_lifecycle").unwrap().unwrap();
        assert_eq!(rule.lifecycle_stage, "archived");
        assert!(rule.archived_at.is_some());
    }

    #[test]
    fn insert_and_list_instance() {
        let store = setup_store();
        store
            .insert_heuristic_instance(&HeuristicInstanceRecord {
                instance_id: "inst_01",
                account_id: "acct",
                user_id: "u1",
                agent_id: None,
                context_summary: "User working on Rust CLI, asked for error handling",
                agent_proposal: Some("Suggested using anyhow for error handling"),
                user_reaction: "Don't use anyhow, I want custom error enum",
                outcome: Some("User wrote custom AppError enum"),
                signal_type: "explicit_negation",
                tags_json: "[\"domain:rust\",\"domain:cli\",\"topic:error-handling\"]",
                session_id: Some("sess_001"),
                source_turn_ids_json: Some("[\"t1\",\"t2\"]"),
                derived_rule_id: None,
                instance_status: "open",
                resolved_at: None,
            })
            .unwrap();

        let instances = store
            .list_heuristic_instances("acct", "u1", Some("open"))
            .unwrap();
        assert_eq!(instances.len(), 1);
        assert_eq!(instances[0].signal_type, "explicit_negation");
        assert_eq!(
            instances[0].user_reaction,
            "Don't use anyhow, I want custom error enum"
        );
    }

    #[test]
    fn retrieve_l0_confirmed_rules() {
        let store = setup_store();

        for i in 0..5 {
            store
                .insert_heuristic_rule(&HeuristicRuleRecord {
                    rule_id: &format!("confirmed_{i}"),
                    account_id: "acct",
                    user_id: "u1",
                    agent_id: None,
                    tags_json: "[\"domain:backend\"]",
                    rule_text: &format!("Confirmed rule {i}"),
                    counter_examples_json: "[]",
                    lifecycle_stage: "confirmed",
                    evidence_count: 10 + i as i64,
                    aggregate_weight: 5.0 + i as f64,
                    last_evidence_at: None,
                    source_instance_ids_json: None,
                    promoted_at: None,
                    user_confirmed: false,
                })
                .unwrap();
        }

        // L0 should return max 3 confirmed rules
        let l0 = retrieve_l0_confirmed(&store, "acct", "u1", 3);
        assert_eq!(l0.len(), 3);
        // Should be sorted by aggregate_weight DESC (highest first)
        assert_eq!(l0[0].rule_id, "confirmed_4");
        assert_eq!(l0[2].rule_id, "confirmed_2");
    }

    #[test]
    fn exponential_decay_calculation() {
        // λ = 0.02, half-life ≈ 35 days
        let lambda = 0.02;
        // After 35 days, weight should be ~50% of original
        let w35 = compute_evidence_decay(1.0, 35.0, lambda);
        assert!(
            w35 > 0.45 && w35 < 0.55,
            "Half-life ≈ 35 days: W_35 = {w35}"
        );
        // After 0 days, weight should be 1.0
        let w0 = compute_evidence_decay(1.0, 0.0, lambda);
        assert_eq!(w0, 1.0);
    }

    #[test]
    fn decay_weights_and_archive_below_threshold() {
        let store = setup_store();

        // Draft rule WITHOUT recent evidence → should be decayed
        store
            .insert_heuristic_rule(&HeuristicRuleRecord {
                rule_id: "rule_draft",
                account_id: "acct",
                user_id: "u1",
                agent_id: None,
                tags_json: "[\"domain:backend\"]",
                rule_text: "Draft rule subject to decay",
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

        // User-confirmed rule — should be EXEMPT from decay entirely (user_confirmed exemption)
        store
            .insert_heuristic_rule(&HeuristicRuleRecord {
                rule_id: "rule_confirmed_exempt",
                account_id: "acct",
                user_id: "u1",
                agent_id: None,
                tags_json: "[\"domain:frontend\"]",
                rule_text: "Confirmed rule exempt from decay",
                counter_examples_json: "[]",
                lifecycle_stage: "confirmed",
                evidence_count: 10,
                aggregate_weight: 5.0,
                last_evidence_at: None,
                source_instance_ids_json: None,
                promoted_at: None,
                user_confirmed: true,
            })
            .unwrap();

        let result = decay_heuristic_weights(&store, "acct", "u1", 0.02, 0.5);
        // Draft rule was decayed (no evidence → no active protection)
        assert_eq!(result.decayed, 1);
        // Confirmed rule was skipped (user_confirmed exemption)
        assert_eq!(result.skipped_confirmed, 1);
        // No rules archived (draft has 0 weight but threshold only archives if weight < 0.5)
        // A draft with 0 evidence has aggregate_weight = 0.0 → archived!
        assert_eq!(result.archived, 1);

        // User-confirmed rule was untouched (exempt)
        let confirmed = store
            .get_heuristic_rule("rule_confirmed_exempt")
            .unwrap()
            .unwrap();
        assert_eq!(confirmed.aggregate_weight, 5.0);
        assert_eq!(confirmed.lifecycle_stage, "confirmed");

        // Draft rule was archived (0 evidence → 0 weight < 0.5 threshold)
        let draft = store.get_heuristic_rule("rule_draft").unwrap().unwrap();
        assert_eq!(draft.lifecycle_stage, "archived");
    }

    #[test]
    fn decay_skips_recently_active_rules() {
        let store = setup_store();

        // Candidate rule with fresh evidence — should be active-protected
        store
            .insert_heuristic_rule(&HeuristicRuleRecord {
                rule_id: "rule_active",
                account_id: "acct",
                user_id: "u1",
                agent_id: None,
                tags_json: "[\"domain:backend\"]",
                rule_text: "Recently active rule",
                counter_examples_json: "[]",
                lifecycle_stage: "candidate",
                evidence_count: 0,
                aggregate_weight: 3.0,
                last_evidence_at: None,
                source_instance_ids_json: None,
                promoted_at: None,
                user_confirmed: false,
            })
            .unwrap();

        // Add fresh evidence (created_at = CURRENT_TIMESTAMP ≈ today)
        store
            .insert_heuristic_evidence(&HeuristicEvidenceRecord {
                evidence_id: "ev_active_1",
                rule_id: "rule_active",
                instance_id: None,
                evidence_type: "support",
                support_weight: 1.0,
                session_id: "sess_active",
            })
            .unwrap();

        let result = decay_heuristic_weights(&store, "acct", "u1", 0.02, 0.5);
        // Rule has fresh evidence → within 7-day active protection → SKIPPED
        assert_eq!(result.skipped_active_protected, 1);
        assert_eq!(result.decayed, 0);

        // Rule's aggregate_weight unchanged
        let rule = store.get_heuristic_rule("rule_active").unwrap().unwrap();
        assert_eq!(rule.aggregate_weight, 3.0);
    }

    #[test]
    fn test_rule_downgrade_on_negation() {
        let store = setup_store();

        store
            .insert_heuristic_rule(&HeuristicRuleRecord {
                rule_id: "rule_downgrade",
                account_id: "acct",
                user_id: "u1",
                agent_id: None,
                tags_json: "[\"domain:backend\"]",
                rule_text: "Confirmed rule to be downgraded",
                counter_examples_json: "[]",
                lifecycle_stage: "confirmed",
                evidence_count: 5,
                aggregate_weight: 3.0,
                last_evidence_at: None,
                source_instance_ids_json: None,
                promoted_at: None,
                user_confirmed: false,
            })
            .unwrap();

        super::downgrade_rule_on_negation(&store, "rule_downgrade");
        let rule = store.get_heuristic_rule("rule_downgrade").unwrap().unwrap();
        assert_eq!(rule.lifecycle_stage, "draft");
    }

    #[test]
    fn tag_group_conflict_resolution_keeps_highest_stage() {
        let store = setup_store();

        // Three rules with same tags but different lifecycle stages
        store
            .insert_heuristic_rule(&HeuristicRuleRecord {
                rule_id: "rule_draft_backend",
                account_id: "acct",
                user_id: "u1",
                agent_id: None,
                tags_json: "[\"domain:backend\"]",
                rule_text: "Draft backend rule",
                counter_examples_json: "[]",
                lifecycle_stage: "draft",
                evidence_count: 2,
                aggregate_weight: 1.0,
                last_evidence_at: None,
                source_instance_ids_json: None,
                promoted_at: None,
                user_confirmed: false,
            })
            .unwrap();

        store
            .insert_heuristic_rule(&HeuristicRuleRecord {
                rule_id: "rule_candidate_backend",
                account_id: "acct",
                user_id: "u1",
                agent_id: None,
                tags_json: "[\"domain:backend\"]",
                rule_text: "Candidate backend rule",
                counter_examples_json: "[]",
                lifecycle_stage: "candidate",
                evidence_count: 5,
                aggregate_weight: 3.0,
                last_evidence_at: None,
                source_instance_ids_json: None,
                promoted_at: None,
                user_confirmed: false,
            })
            .unwrap();

        store
            .insert_heuristic_rule(&HeuristicRuleRecord {
                rule_id: "rule_confirmed_backend",
                account_id: "acct",
                user_id: "u1",
                agent_id: None,
                tags_json: "[\"domain:backend\"]",
                rule_text: "Confirmed backend rule",
                counter_examples_json: "[]",
                lifecycle_stage: "confirmed",
                evidence_count: 10,
                aggregate_weight: 5.0,
                last_evidence_at: None,
                source_instance_ids_json: None,
                promoted_at: None,
                user_confirmed: false,
            })
            .unwrap();

        // Different tag set — should NOT be deduplicated
        store
            .insert_heuristic_rule(&HeuristicRuleRecord {
                rule_id: "rule_frontend",
                account_id: "acct",
                user_id: "u1",
                agent_id: None,
                tags_json: "[\"domain:frontend\"]",
                rule_text: "Frontend rule",
                counter_examples_json: "[]",
                lifecycle_stage: "draft",
                evidence_count: 1,
                aggregate_weight: 0.5,
                last_evidence_at: None,
                source_instance_ids_json: None,
                promoted_at: None,
                user_confirmed: false,
            })
            .unwrap();

        let entries = retrieve_heuristics(
            &store,
            "acct",
            "u1",
            &["domain:backend".to_owned()],
            "backend",
            10,
        );
        // Only confirmed backend rule should survive (highest lifecycle_stage for same tag set)
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].rule_id, "rule_confirmed_backend");

        // With both tag sets, should get 2 entries (1 per unique tag group)
        let entries_all = retrieve_heuristics(&store, "acct", "u1", &[], "", 10);
        assert_eq!(entries_all.len(), 2);
    }
}

/// Build an LLM prompt for simulate_reaction that synthesizes a prediction
/// from matched heuristic rules and the proposed scenario.
pub fn build_simulate_reaction_prompt(scenario: &str, entries: &[HeuristicEntry]) -> String {
    let rules_text = entries
        .iter()
        .map(|e| {
            let mut line = format!("- [{}] {}", e.lifecycle_stage, e.rule_text);
            if !e.counter_examples.is_empty() {
                line.push_str(&format!(" (except: {})", e.counter_examples.join("; ")));
            }
            line
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        r"You are predicting how a user will react to a proposed action based on their learned behavioral preferences.

Scenario (what the agent is about to do):
{scenario}

Learned user preferences:
{rules_text}

Based on these preferences, predict:
1. Will the user likely accept or reject this approach?
2. What specific aspects might they want changed?
3. What alternative approach would better align with their preferences?

Be concise and actionable. Focus on the most relevant preferences."
    )
}

/// Deterministic fallback prediction when LLM is unavailable.
pub fn build_deterministic_prediction(entries: &[HeuristicEntry]) -> String {
    let confirmed_count = entries
        .iter()
        .filter(|e| e.lifecycle_stage == "confirmed")
        .count();
    let candidate_count = entries
        .iter()
        .filter(|e| e.lifecycle_stage == "candidate")
        .count();

    let mut prediction = format!(
        "Based on {} learned preference(s) ({} confirmed, {} candidate):",
        entries.len(),
        confirmed_count,
        candidate_count,
    );

    for entry in entries {
        let marker = match entry.lifecycle_stage.as_str() {
            "confirmed" => "★",
            "candidate" => "◆",
            _ => "○",
        };
        prediction.push_str(&format!("\n  {marker} {}", entry.rule_text));
        if !entry.counter_examples.is_empty() {
            prediction.push_str(&format!(
                " ⚠️ except: {}",
                entry.counter_examples.join("; ")
            ));
        }
    }

    prediction
        .push_str("\n\nThe user may reject approaches that conflict with the above preferences.");
    prediction
}
