//! Sleep consolidation for heuristic rules (roadmap §5.5).
//!
//! Periodic batch task that discovers contrast pairs, extracts latent variables,
//! and merges rules into higher-dimensional versions.
//!
//! Three steps:
//! 1. Contrast-pair discovery: find rules with similar tags but different counter_examples
//! 2. Latent variable extraction (LLM-assisted, experimental): identify what causes different preferences
//! 3. Rule merging: create higher-dimension rules, archive originals
//!
//! Deterministic fallback: when LLM is unavailable, skip Steps 2-3 and queue
//! contrast pairs for later LLM processing.

use crate::heuristics::{LifecycleStage, validate_tags};
use crate::llm::LlmAssist;
use mfs_metadata::{HeuristicRuleRecord, MetadataStore, StoredHeuristicRule};
use mfs_uri::short_hash_hex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ─── Contrast Pair Discovery ──────────────────────────────────────────

/// A contrast pair: two rules with overlapping tags but conflicting preferences.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContrastPair {
    /// First rule in the pair.
    pub rule_a: StoredHeuristicRule,
    /// Second rule in the pair.
    pub rule_b: StoredHeuristicRule,
    /// Shared tags between the two rules.
    pub shared_tags: Vec<String>,
    /// Tags unique to rule_a.
    pub tags_a_only: Vec<String>,
    /// Tags unique to rule_b.
    pub tags_b_only: Vec<String>,
}

/// Result of sleep consolidation pass.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SleepConsolidationResult {
    /// Number of contrast pairs discovered.
    pub pairs_discovered: usize,
    /// Number of latent variables identified (LLM-assisted).
    pub latent_variables_found: usize,
    /// Number of merged rules created.
    pub rules_merged: usize,
    /// Number of original rules archived after merging.
    pub rules_archived: usize,
    /// Pending contrast pairs queued for later LLM processing.
    pub pending_pairs: usize,
    /// Number of affinity clusters discovered.
    pub clusters_found: usize,
    /// Number of merge candidates archived via affinity clustering.
    pub cluster_archived: usize,
}

/// Discover contrast pairs among active heuristic rules.
///
/// Scans same tag-group rules for pairs where rule_text is semantically similar
/// but counter_examples differ. These pairs indicate boundary conditions that
/// haven't been explicitly expressed as tags.
pub fn discover_contrast_pairs(
    metadata: &MetadataStore,
    account_id: &str,
    user_id: &str,
) -> Vec<ContrastPair> {
    let active_rules = metadata
        .get_active_heuristic_rules(account_id, user_id, LifecycleStage::active_stages())
        .unwrap_or_default();

    let mut pairs = Vec::new();

    // Group rules by their sorted tag set (same tag combination)
    let mut by_tags: HashMap<Vec<String>, Vec<&StoredHeuristicRule>> = HashMap::new();
    for rule in &active_rules {
        let tags: Vec<String> = serde_json::from_str(&rule.tags_json).unwrap_or_default();
        let mut key = tags.clone();
        key.sort();
        by_tags.entry(key).or_default().push(rule);
    }

    // Within each tag group, find pairs with different counter_examples
    for rules in by_tags.values() {
        if rules.len() < 2 {
            continue;
        }
        for i in 0..rules.len() {
            for j in (i + 1)..rules.len() {
                let rule_a = rules[i];
                let rule_b = rules[j];

                let ce_a: Vec<String> =
                    serde_json::from_str(&rule_a.counter_examples_json).unwrap_or_default();
                let ce_b: Vec<String> =
                    serde_json::from_str(&rule_b.counter_examples_json).unwrap_or_default();

                // Only pair rules that have different counter_examples
                // (same counter_examples means they agree, not a contrast)
                if ce_a.is_empty() && ce_b.is_empty() {
                    continue;
                }
                // Check if counter_examples are substantially different
                // Use Jaccard similarity: |intersection| / |union|
                // Require < 0.5 Jaccard similarity → at least half the examples are different
                let ce_a_set: std::collections::HashSet<&str> =
                    ce_a.iter().map(|s| s.as_str()).collect();
                let ce_b_set: std::collections::HashSet<&str> =
                    ce_b.iter().map(|s| s.as_str()).collect();
                let intersection = ce_a_set.intersection(&ce_b_set).count();
                let union = ce_a_set.union(&ce_b_set).count();
                if union == 0 {
                    continue;
                }
                let jaccard = intersection as f64 / union as f64;
                if jaccard >= 0.5 {
                    continue; // Too similar — not a real contrast
                }

                let tags_a: Vec<String> =
                    serde_json::from_str(&rule_a.tags_json).unwrap_or_default();
                let tags_b: Vec<String> =
                    serde_json::from_str(&rule_b.tags_json).unwrap_or_default();

                pairs.push(ContrastPair {
                    rule_a: rule_a.clone(),
                    rule_b: rule_b.clone(),
                    shared_tags: tags_a
                        .iter()
                        .filter(|t| tags_b.contains(t))
                        .cloned()
                        .collect(),
                    tags_a_only: tags_a
                        .iter()
                        .filter(|t| !tags_b.contains(t))
                        .cloned()
                        .collect(),
                    tags_b_only: tags_b
                        .iter()
                        .filter(|t| !tags_a.contains(t))
                        .cloned()
                        .collect(),
                });
            }
        }
    }

    pairs
}

// ─── Affinity Clustering ──────────────────────────────────────────────

/// A cluster of rules with similar tags that are candidates for merging.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AffinityCluster {
    /// The primary rule (highest aggregate_weight).
    pub primary: StoredHeuristicRule,
    /// Rules that are merge candidates (Jaccard > 0.5 on rule_text).
    pub merge_candidates: Vec<StoredHeuristicRule>,
    /// Shared tags within the cluster.
    pub tags: Vec<String>,
}

/// Jaccard similarity between two sets of words.
fn word_jaccard(a: &str, b: &str) -> f64 {
    let set_a: std::collections::HashSet<&str> = a.split_whitespace().collect();
    let set_b: std::collections::HashSet<&str> = b.split_whitespace().collect();
    if set_a.is_empty() && set_b.is_empty() {
        return 0.0;
    }
    let intersection = set_a.intersection(&set_b).count();
    let union = set_a.union(&set_b).count();
    if union == 0 {
        0.0
    } else {
        intersection as f64 / union as f64
    }
}

/// Tag Jaccard similarity threshold for clustering.
const TAG_JACCARD_THRESHOLD: f64 = 0.8;

/// Rule-text Jaccard threshold for merge candidacy.
const RULE_TEXT_JACCARD_THRESHOLD: f64 = 0.5;

/// Discover affinity clusters among active rules.
///
/// Algorithm:
/// 1. Load active rules
/// 2. Greedy clustering by tag Jaccard >= 0.8
/// 3. Within clusters, find rule_text Jaccard > 0.5 merge candidates
/// 4. Primary = highest aggregate_weight in each cluster
pub fn discover_affinity_clusters(
    metadata: &MetadataStore,
    account_id: &str,
    user_id: &str,
) -> Vec<AffinityCluster> {
    let active_rules = metadata
        .get_active_heuristic_rules(account_id, user_id, LifecycleStage::active_stages())
        .unwrap_or_default();

    if active_rules.len() < 2 {
        return Vec::new();
    }

    // Step 2: Greedy clustering by tag Jaccard >= 0.8
    let mut clusters: Vec<Vec<usize>> = Vec::new();
    let mut assigned = vec![false; active_rules.len()];

    for i in 0..active_rules.len() {
        if assigned[i] {
            continue;
        }
        let mut cluster = vec![i];
        assigned[i] = true;
        let tags_i: Vec<String> =
            serde_json::from_str(&active_rules[i].tags_json).unwrap_or_default();

        for j in (i + 1)..active_rules.len() {
            if assigned[j] {
                continue;
            }
            let tags_j: Vec<String> =
                serde_json::from_str(&active_rules[j].tags_json).unwrap_or_default();
            let set_i: std::collections::HashSet<&str> =
                tags_i.iter().map(|s| s.as_str()).collect();
            let set_j: std::collections::HashSet<&str> =
                tags_j.iter().map(|s| s.as_str()).collect();
            let intersection = set_i.intersection(&set_j).count();
            let union = set_i.union(&set_j).count();
            let tag_jaccard = if union == 0 {
                0.0
            } else {
                intersection as f64 / union as f64
            };

            if tag_jaccard >= TAG_JACCARD_THRESHOLD {
                cluster.push(j);
                assigned[j] = true;
            }
        }
        clusters.push(cluster);
    }

    // Step 3-4: Within clusters, find merge candidates and pick primary
    let mut result = Vec::new();
    for cluster_indices in &clusters {
        if cluster_indices.len() < 2 {
            continue;
        }

        let cluster_rules: Vec<&StoredHeuristicRule> =
            cluster_indices.iter().map(|&i| &active_rules[i]).collect();

        // Find merge candidates: pairs with rule_text Jaccard > 0.5
        let mut merge_candidate_ids: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        for i in 0..cluster_rules.len() {
            for j in (i + 1)..cluster_rules.len() {
                let text_jaccard =
                    word_jaccard(&cluster_rules[i].rule_text, &cluster_rules[j].rule_text);
                if text_jaccard > RULE_TEXT_JACCARD_THRESHOLD {
                    merge_candidate_ids.insert(cluster_rules[i].rule_id.clone());
                    merge_candidate_ids.insert(cluster_rules[j].rule_id.clone());
                }
            }
        }

        if merge_candidate_ids.is_empty() {
            continue;
        }

        // Primary = highest aggregate_weight
        let primary = cluster_rules
            .iter()
            .max_by(|a, b| {
                a.aggregate_weight
                    .partial_cmp(&b.aggregate_weight)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap();

        let merge_candidates: Vec<StoredHeuristicRule> = cluster_rules
            .iter()
            .filter(|r| merge_candidate_ids.contains(&r.rule_id) && r.rule_id != primary.rule_id)
            .map(|r| (*r).clone())
            .collect();

        let tags: Vec<String> = serde_json::from_str(&primary.tags_json).unwrap_or_default();

        result.push(AffinityCluster {
            primary: (*primary).clone(),
            merge_candidates,
            tags,
        });
    }

    result
}

// ─── LLM Latent Variable Extraction ───────────────────────────────────

/// Build a prompt for LLM to extract latent variables from contrast pairs.
pub fn build_latent_variable_prompt(pair: &ContrastPair) -> String {
    format!(
        "You are analyzing two behavioral heuristic rules that share similar context but have different preferences.\n\
         \n\
         Rule A: {}\n\
         Tags A: {}\n\
         Counter-examples A: {}\n\
         \n\
         Rule B: {}\n\
         Tags B: {}\n\
         Counter-examples B: {}\n\
         \n\
         Shared tags: {}\n\
         Tags unique to A: {}\n\
         Tags unique to B: {}\n\
         \n\
         What latent factor or boundary condition explains why the user has different preferences \
         in these two situations? Identify the hidden variable that distinguishes the contexts.\n\
         \n\
         Respond in JSON format:\n\
         {{\n\
           \"latent_variable\": \"<description of the hidden factor>\",\n\
           \"new_tag\": \"<suggested tag key:value to capture this factor>\",\n\
           \"merged_rule_text\": \"<unified rule text incorporating the boundary condition>\",\n\
           \"merged_counter_examples\": [<list of counter-examples for the merged rule>],\n\
           \"confidence\": <0.0-1.0 confidence in this analysis>\n\
         }}",
        pair.rule_a.rule_text,
        pair.rule_a.tags_json,
        pair.rule_a.counter_examples_json,
        pair.rule_b.rule_text,
        pair.rule_b.tags_json,
        pair.rule_b.counter_examples_json,
        serde_json::to_string(&pair.shared_tags).unwrap_or_default(),
        serde_json::to_string(&pair.tags_a_only).unwrap_or_default(),
        serde_json::to_string(&pair.tags_b_only).unwrap_or_default(),
    )
}

/// LLM classification result for latent variable extraction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatentVariableResult {
    pub latent_variable: String,
    pub new_tag: String,
    pub merged_rule_text: String,
    pub merged_counter_examples: Vec<String>,
    pub confidence: f64,
}

/// Parse LLM response for latent variable extraction.
pub fn parse_llm_latent_response(response: &str) -> Option<LatentVariableResult> {
    // Try to extract JSON from the response
    let json_start = response.find('{')?;
    let json_end = response.rfind('}')? + 1;
    let json_str = &response[json_start..json_end];

    serde_json::from_str(json_str)
        .ok()
        .and_then(|result: LatentVariableResult| {
            if result.confidence >= 0.6 && !result.latent_variable.is_empty() {
                Some(result)
            } else {
                None
            }
        })
}

// ─── Rule Merging ──────────────────────────────────────────────────────

/// Merge a contrast pair into a higher-dimensional rule.
///
/// Creates a new rule that incorporates the latent variable as a new tag,
/// archives the original pair rules, and transfers their evidence.
pub fn merge_contrast_pair(
    metadata: &MetadataStore,
    account_id: &str,
    user_id: &str,
    agent_id: &str,
    pair: &ContrastPair,
    latent: &LatentVariableResult,
) -> Option<String> {
    // Build merged tags: shared tags + new latent variable tag
    let mut merged_tags = pair.shared_tags.clone();
    if !latent.new_tag.is_empty() {
        let valid = validate_tags(&[latent.new_tag.clone()]);
        merged_tags.extend(valid);
    }
    merged_tags.sort();

    // Compute merged rule_id
    let merged_rule_text = latent.merged_rule_text.clone();
    let rule_id = format!(
        "{}:{}:{}",
        account_id,
        user_id,
        short_hash_hex(merged_rule_text.as_bytes(), 12)
    );

    // Insert merged rule
    let record = HeuristicRuleRecord {
        rule_id: &rule_id,
        account_id,
        user_id,
        agent_id: Some(agent_id),
        tags_json: &serde_json::to_string(&merged_tags).unwrap_or_else(|_| "[]".to_owned()),
        rule_text: &merged_rule_text,
        counter_examples_json: &serde_json::to_string(&latent.merged_counter_examples)
            .unwrap_or_else(|_| "[]".to_owned()),
        lifecycle_stage: "draft",
        evidence_count: 0,
        aggregate_weight: 0.0,
        last_evidence_at: None,
        source_instance_ids_json: None,
        promoted_at: None,
        user_confirmed: false,
    };
    metadata.insert_heuristic_rule(&record).ok()?;

    // Archive the original pair rules
    metadata
        .update_rule_lifecycle(&pair.rule_a.rule_id, "archived")
        .ok();
    metadata
        .update_rule_lifecycle(&pair.rule_b.rule_id, "archived")
        .ok();

    Some(rule_id)
}

// ─── Main Consolidation Pipeline ──────────────────────────────────────

/// Run sleep consolidation pipeline.
///
/// Step 1: Discover contrast pairs (always runs).
/// Step 2: Extract latent variables via LLM (when available).
/// Step 3: Merge rules into higher-dimensional versions (when LLM results are confident).
///
/// When LLM is unavailable, pairs are queued for later processing.
pub async fn run_sleep_consolidation(
    metadata: &MetadataStore,
    account_id: &str,
    user_id: &str,
    agent_id: &str,
    llm: &LlmAssist,
) -> SleepConsolidationResult {
    let mut result = SleepConsolidationResult {
        pairs_discovered: 0,
        latent_variables_found: 0,
        rules_merged: 0,
        rules_archived: 0,
        pending_pairs: 0,
        clusters_found: 0,
        cluster_archived: 0,
    };

    // Step 1: Discover contrast pairs
    let pairs = discover_contrast_pairs(metadata, account_id, user_id);
    result.pairs_discovered = pairs.len();

    if pairs.is_empty() {
        return result;
    }

    // Step 2: LLM-assisted latent variable extraction
    let mut successful_merges = 0;
    let mut archived_originals = 0;
    let mut latent_found = 0;
    let mut pending = 0;

    for pair in &pairs {
        if llm.is_available() {
            let prompt = build_latent_variable_prompt(pair);
            let llm_result = llm.complete(&prompt).await;

            if let Some(response) = llm_result {
                if let Some(latent) = parse_llm_latent_response(&response) {
                    latent_found += 1;

                    // Step 3: Merge into higher-dimensional rule
                    if let Some(_merged_id) =
                        merge_contrast_pair(metadata, account_id, user_id, agent_id, pair, &latent)
                    {
                        successful_merges += 1;
                        archived_originals += 2; // Both pair rules archived
                    }
                } else {
                    pending += 1;
                }
            } else {
                pending += 1;
            }
        } else {
            // Deterministic fallback: queue pair for later LLM processing
            pending += 1;
        }
    }

    result.latent_variables_found = latent_found;
    result.rules_merged = successful_merges;
    result.rules_archived = archived_originals;
    result.pending_pairs = pending;

    // Step 4: Affinity clustering — archive merge candidates
    let clusters = discover_affinity_clusters(metadata, account_id, user_id);
    result.clusters_found = clusters.len();
    for cluster in &clusters {
        for candidate in &cluster.merge_candidates {
            if metadata
                .update_rule_lifecycle(&candidate.rule_id, "archived")
                .is_ok()
            {
                result.cluster_archived += 1;
                result.rules_archived += 1;
            }
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use mfs_metadata::HeuristicRuleRecord;
    use mfs_metadata::MetadataStore;

    fn setup_store() -> MetadataStore {
        MetadataStore::open_in_memory(false).unwrap()
    }

    #[test]
    fn discover_contrast_pairs_finds_conflicting_rules() {
        let store = setup_store();

        // Rule A: prefers pragmatic solutions when prototyping
        store.insert_heuristic_rule(&HeuristicRuleRecord {
            rule_id: "rule_a",
            account_id: "acct",
            user_id: "u1",
            agent_id: None,
            tags_json: "[\"domain:backend\"]",
            rule_text: "User prefers pragmatic solutions for backend",
            counter_examples_json: "[\"But for production APIs, user insists on proper error types\"]",
            lifecycle_stage: "candidate",
            evidence_count: 5,
            aggregate_weight: 3.0,
            last_evidence_at: None,
            source_instance_ids_json: None,
            promoted_at: None,
            user_confirmed: false,
        }).unwrap();

        // Rule B: prefers proper patterns for production
        store
            .insert_heuristic_rule(&HeuristicRuleRecord {
                rule_id: "rule_b",
                account_id: "acct",
                user_id: "u1",
                agent_id: None,
                tags_json: "[\"domain:backend\"]",
                rule_text: "User prefers proper patterns for backend production",
                counter_examples_json: "[\"But for prototyping, user prefers quick and dirty\"]",
                lifecycle_stage: "candidate",
                evidence_count: 5,
                aggregate_weight: 3.0,
                last_evidence_at: None,
                source_instance_ids_json: None,
                promoted_at: None,
                user_confirmed: false,
            })
            .unwrap();

        // Rule C: different tags — should not be paired with A/B
        store
            .insert_heuristic_rule(&HeuristicRuleRecord {
                rule_id: "rule_c",
                account_id: "acct",
                user_id: "u1",
                agent_id: None,
                tags_json: "[\"domain:frontend\"]",
                rule_text: "User prefers TypeScript for frontend",
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

        let pairs = discover_contrast_pairs(&store, "acct", "u1");
        // Should find 1 pair: A-B (same tags, different counter_examples)
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].shared_tags, vec!["domain:backend"]);
    }

    #[test]
    fn no_contrast_when_counter_examples_identical() {
        let store = setup_store();

        store
            .insert_heuristic_rule(&HeuristicRuleRecord {
                rule_id: "rule_x",
                account_id: "acct",
                user_id: "u1",
                agent_id: None,
                tags_json: "[\"domain:backend\"]",
                rule_text: "User prefers X",
                counter_examples_json: "[\"Except when under time pressure\"]",
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
                rule_id: "rule_y",
                account_id: "acct",
                user_id: "u1",
                agent_id: None,
                tags_json: "[\"domain:backend\"]",
                rule_text: "User prefers Y",
                counter_examples_json: "[\"Except when under time pressure\"]",
                lifecycle_stage: "candidate",
                evidence_count: 5,
                aggregate_weight: 3.0,
                last_evidence_at: None,
                source_instance_ids_json: None,
                promoted_at: None,
                user_confirmed: false,
            })
            .unwrap();

        let pairs = discover_contrast_pairs(&store, "acct", "u1");
        assert_eq!(
            pairs.len(),
            0,
            "Same counter_examples → not a contrast pair"
        );
    }

    #[test]
    fn build_latent_variable_prompt_contains_both_rules() {
        let store = setup_store();

        store
            .insert_heuristic_rule(&HeuristicRuleRecord {
                rule_id: "rule_a",
                account_id: "acct",
                user_id: "u1",
                agent_id: None,
                tags_json: "[\"domain:backend\"]",
                rule_text: "Prefer X",
                counter_examples_json: "[\"Except for production\"]",
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
                rule_id: "rule_b",
                account_id: "acct",
                user_id: "u1",
                agent_id: None,
                tags_json: "[\"domain:backend\"]",
                rule_text: "Prefer Y",
                counter_examples_json: "[\"Except for prototyping\"]",
                lifecycle_stage: "candidate",
                evidence_count: 5,
                aggregate_weight: 3.0,
                last_evidence_at: None,
                source_instance_ids_json: None,
                promoted_at: None,
                user_confirmed: false,
            })
            .unwrap();

        let pairs = discover_contrast_pairs(&store, "acct", "u1");
        let prompt = build_latent_variable_prompt(&pairs[0]);
        assert!(prompt.contains("Prefer X"));
        assert!(prompt.contains("Prefer Y"));
        assert!(prompt.contains("latent_variable"));
    }

    #[test]
    fn parse_llm_latent_response_valid() {
        let response = r#"{"latent_variable":"production vs prototyping","new_tag":"phase:production","merged_rule_text":"User prefers proper patterns for production but pragmatic for prototyping","merged_counter_examples":["For quick fixes under time pressure"],"confidence":0.85}"#;
        let result = parse_llm_latent_response(response);
        assert!(result.is_some());
        let latent = result.unwrap();
        assert_eq!(latent.latent_variable, "production vs prototyping");
        assert!(latent.confidence >= 0.6);
    }

    #[test]
    fn parse_llm_latent_response_low_confidence_rejected() {
        let response = r#"{"latent_variable":"maybe something","new_tag":"phase:test","merged_rule_text":"test","merged_counter_examples":[],"confidence":0.3}"#;
        assert!(parse_llm_latent_response(response).is_none());
    }

    // ── Affinity Clustering tests ──

    #[test]
    fn similar_rules_clustered() {
        let store = setup_store();

        // Two rules with same tags and similar rule_text
        store
            .insert_heuristic_rule(&HeuristicRuleRecord {
                rule_id: "aff_a",
                account_id: "acct",
                user_id: "u1",
                agent_id: None,
                tags_json: "[\"domain:backend\",\"phase:production\"]",
                rule_text: "User prefers Rust for all backend services",
                counter_examples_json: "[]",
                lifecycle_stage: "candidate",
                evidence_count: 5,
                aggregate_weight: 4.0,
                last_evidence_at: None,
                source_instance_ids_json: None,
                promoted_at: None,
                user_confirmed: false,
            })
            .unwrap();

        store
            .insert_heuristic_rule(&HeuristicRuleRecord {
                rule_id: "aff_b",
                account_id: "acct",
                user_id: "u1",
                agent_id: None,
                tags_json: "[\"domain:backend\",\"phase:production\"]",
                rule_text: "User prefers Rust for all backend microservices",
                counter_examples_json: "[]",
                lifecycle_stage: "draft",
                evidence_count: 3,
                aggregate_weight: 2.0,
                last_evidence_at: None,
                source_instance_ids_json: None,
                promoted_at: None,
                user_confirmed: false,
            })
            .unwrap();

        let clusters = discover_affinity_clusters(&store, "acct", "u1");
        assert_eq!(clusters.len(), 1, "Should find 1 cluster");
        // Primary should be aff_a (higher aggregate_weight)
        assert_eq!(clusters[0].primary.rule_id, "aff_a");
        // aff_b should be a merge candidate
        assert_eq!(clusters[0].merge_candidates.len(), 1);
        assert_eq!(clusters[0].merge_candidates[0].rule_id, "aff_b");
    }

    #[test]
    fn different_tag_rules_not_clustered() {
        let store = setup_store();

        store
            .insert_heuristic_rule(&HeuristicRuleRecord {
                rule_id: "diff_a",
                account_id: "acct",
                user_id: "u1",
                agent_id: None,
                tags_json: "[\"domain:backend\"]",
                rule_text: "User prefers Rust for backend",
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
                rule_id: "diff_b",
                account_id: "acct",
                user_id: "u1",
                agent_id: None,
                tags_json: "[\"domain:frontend\"]",
                rule_text: "User prefers TypeScript for frontend",
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

        let clusters = discover_affinity_clusters(&store, "acct", "u1");
        assert_eq!(clusters.len(), 0, "Different tags should not cluster");
    }
}
