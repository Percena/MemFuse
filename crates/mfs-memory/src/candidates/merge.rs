use std::collections::HashSet;

use mfs_types::text::{TokenizeConfig, tokenize_to_set};

use super::schema::{
    MemoryCandidate, MemoryCategory, MemoryDecision, MemoryMergeDecision, MemoryRecord,
};

/// Decide how to merge a new `candidate` into `existing` records.
///
/// When a `ChatProvider` is available the decision is delegated to the LLM
/// (mirrors MemFuse's `dedup_decision.yaml`).  Otherwise the deterministic
/// Jaccard-overlap heuristic is used as a fallback.
pub async fn decide_memory_merge(
    category: MemoryCategory,
    candidate: &MemoryCandidate,
    existing: &[MemoryRecord],
) -> MemoryMergeDecision {
    if !category.is_mergeable() {
        return MemoryMergeDecision {
            primary: MemoryDecision::Create,
            target_uri: None,
            delete_uris: Vec::new(),
        };
    }

    // Try LLM-based dedup decision.
    if let Some(decision) = try_llm_dedup(candidate, existing).await {
        return decision;
    }

    // Deterministic fallback.
    deterministic_merge(category, &candidate.content, existing)
}

// ─── LLM dedup ───────────────────────────────────────────────────────────────

async fn try_llm_dedup(
    candidate: &MemoryCandidate,
    existing: &[MemoryRecord],
) -> Option<MemoryMergeDecision> {
    use mfs_semantic::ProcessingMode;
    use mfs_semantic::chat_provider_from_env;

    if existing.is_empty() {
        return None;
    }

    let provider = chat_provider_from_env();
    if provider.mode() == ProcessingMode::Degraded {
        return None;
    }

    let existing_text = existing
        .iter()
        .enumerate()
        .map(|(i, r)| format!("{}. uri={}\n{}", i + 1, r.uri, r.content))
        .collect::<Vec<_>>()
        .join("\n\n");

    let prompt = format!(
        r#"You are deciding how to update long-term memory with a new candidate memory.

Candidate memory:
- Abstract: {abstract}
- Overview: {overview}
- Content: {content}

Existing similar memories:
{existing_text}

Goal: Keep memory consistent and useful while minimizing destructive edits.

Candidate-level decision:
- skip: Candidate adds no useful new information (duplicate or too weak). No memory changes.
- create: Candidate is a valid new memory to store as a separate item. May optionally delete fully-invalidated existing memories.
- none: Candidate itself should not be stored, but existing memories should be reconciled.

Existing-memory per-item action:
- merge: Existing memory and candidate are about the same subject and should be unified.
- delete: Existing memory must be removed only if candidate fully invalidates the entire existing memory.

Hard constraints:
- If decision is "skip", do not return "list".
- If any list item uses "merge", decision must be "none".
- If decision is "create", list can be empty or contain delete items only.
- Use uri exactly from existing memories list.
- Return JSON only, no prose.

Return JSON:
{{
  "decision": "skip|create|none",
  "reason": "short reason",
  "list": [
    {{
      "uri": "<existing memory uri>",
      "decide": "merge|delete",
      "reason": "short reason"
    }}
  ]
}}"#,
        abstract = candidate.abstract_text,
        overview = candidate.overview_text,
        content = candidate.content,
        existing_text = existing_text,
    );

    let response = provider.complete(&prompt).await?;
    parse_dedup_response(&response)
}

fn parse_dedup_response(response: &str) -> Option<MemoryMergeDecision> {
    let json_str = strip_code_fences(response);
    let value: serde_json::Value = serde_json::from_str(json_str).ok()?;

    let decision_str = value.get("decision")?.as_str()?;
    let list = value.get("list").and_then(|v| v.as_array());

    match decision_str {
        "skip" => Some(MemoryMergeDecision {
            primary: MemoryDecision::Skip,
            target_uri: None,
            delete_uris: Vec::new(),
        }),
        "create" => {
            let delete_uris = list
                .map(|items| {
                    items
                        .iter()
                        .filter(|item| {
                            item.get("decide").and_then(|v| v.as_str()) == Some("delete")
                        })
                        .filter_map(|item| {
                            item.get("uri").and_then(|v| v.as_str()).map(str::to_owned)
                        })
                        .collect()
                })
                .unwrap_or_default();
            Some(MemoryMergeDecision {
                primary: MemoryDecision::Create,
                target_uri: None,
                delete_uris,
            })
        }
        "none" => {
            // Find the first merge target.
            let merge_uri = list.and_then(|items| {
                items
                    .iter()
                    .find(|item| item.get("decide").and_then(|v| v.as_str()) == Some("merge"))
                    .and_then(|item| item.get("uri").and_then(|v| v.as_str()).map(str::to_owned))
            });
            let delete_uris = list
                .map(|items| {
                    items
                        .iter()
                        .filter(|item| {
                            item.get("decide").and_then(|v| v.as_str()) == Some("delete")
                        })
                        .filter_map(|item| {
                            item.get("uri").and_then(|v| v.as_str()).map(str::to_owned)
                        })
                        .collect()
                })
                .unwrap_or_default();
            Some(MemoryMergeDecision {
                primary: if merge_uri.is_some() {
                    MemoryDecision::Merge
                } else {
                    MemoryDecision::Skip
                },
                target_uri: merge_uri,
                delete_uris,
            })
        }
        _ => None,
    }
}

/// Strip markdown code fences from LLM response text.
/// Shared utility — previously duplicated in extract.rs and merge.rs.
pub fn strip_code_fences(s: &str) -> &str {
    let s = s.trim();
    // ```json ... ``` or ``` ... ```
    if let Some(inner) = s.strip_prefix("```json") {
        if let Some(inner) = inner.strip_suffix("```") {
            return inner.trim();
        }
    }
    if let Some(inner) = s.strip_prefix("```") {
        if let Some(inner) = inner.strip_suffix("```") {
            return inner.trim();
        }
    }
    s
}

// ─── LLM merge bundle ────────────────────────────────────────────────────────

/// Merge `candidate` into `existing_content` using the LLM.
/// Returns the merged L0/L1/L2 triple, or `None` if LLM is unavailable.
pub async fn llm_merge_bundle(
    candidate: &MemoryCandidate,
    existing_content: &str,
    existing_abstract: &str,
    existing_overview: &str,
) -> Option<(String, String, String)> {
    use mfs_semantic::ProcessingMode;
    use mfs_semantic::chat_provider_from_env;

    let provider = chat_provider_from_env();
    if provider.mode() == ProcessingMode::Degraded {
        return None;
    }

    let prompt = format!(
        r#"You are merging one existing memory with one new memory update.

Category: {category}
Target Output Language: auto

Existing memory:
- Abstract (L0): {existing_abstract}
- Overview (L1): {existing_overview}
- Content (L2): {existing_content}

New memory:
- Abstract (L0): {new_abstract}
- Overview (L1): {new_overview}
- Content (L2): {new_content}

Requirements:
- Merge into a single coherent memory.
- Keep non-conflicting details from existing memory.
- Update conflicting details to reflect the newer fact.
- Return JSON only.

Output JSON schema:
{{
  "decision": "merge",
  "abstract": "one-line L0 summary",
  "overview": "structured markdown L1 summary",
  "content": "full merged L2 content",
  "reason": "short reason"
}}"#,
        category = candidate.category.slug(),
        existing_abstract = existing_abstract,
        existing_overview = existing_overview,
        existing_content = existing_content,
        new_abstract = candidate.abstract_text,
        new_overview = candidate.overview_text,
        new_content = candidate.content,
    );

    let response = provider.complete(&prompt).await?;
    let json_str = strip_code_fences(&response);
    let value: serde_json::Value = serde_json::from_str(json_str).ok()?;

    let abstract_text = value.get("abstract")?.as_str()?.trim().to_owned();
    let overview_text = value
        .get("overview")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_owned();
    let content = value.get("content")?.as_str()?.trim().to_owned();

    if abstract_text.is_empty() || content.is_empty() {
        return None;
    }

    Some((abstract_text, overview_text, content))
}

// ─── Deterministic fallback ───────────────────────────────────────────────────

/// Jaccard-overlap heuristic used when no LLM is available.
pub fn deterministic_merge(
    category: MemoryCategory,
    candidate_content: &str,
    existing: &[MemoryRecord],
) -> MemoryMergeDecision {
    if !category.is_mergeable() {
        return MemoryMergeDecision {
            primary: MemoryDecision::Create,
            target_uri: None,
            delete_uris: Vec::new(),
        };
    }

    let config = TokenizeConfig {
        trim_edges: false,
        min_len: 3,
        preserve_semantic_short_words: false,
    };
    let candidate_terms = tokenize_to_set(candidate_content, &config);
    if candidate_terms.is_empty() {
        return MemoryMergeDecision {
            primary: MemoryDecision::Create,
            target_uri: None,
            delete_uris: Vec::new(),
        };
    }

    let best_match = existing
        .iter()
        .filter(|record| record.category == category)
        .map(|record| {
            (
                record,
                overlap_score(&candidate_terms, &tokenize_to_set(&record.content, &config)),
            )
        })
        .max_by(|left, right| left.1.total_cmp(&right.1));

    if let Some((record, score)) = best_match {
        if score >= 0.6 {
            return MemoryMergeDecision {
                primary: MemoryDecision::Merge,
                target_uri: Some(record.uri.clone()),
                delete_uris: Vec::new(),
            };
        }
    }

    MemoryMergeDecision {
        primary: MemoryDecision::Create,
        target_uri: None,
        delete_uris: Vec::new(),
    }
}

fn overlap_score(candidate_terms: &HashSet<String>, existing_terms: &HashSet<String>) -> f64 {
    if candidate_terms.is_empty() {
        return 0.0;
    }
    let overlap = candidate_terms
        .iter()
        .filter(|term| existing_terms.contains(*term))
        .count() as f64;
    overlap / candidate_terms.len() as f64
}

#[cfg(test)]
mod tests {
    use super::super::schema::{MemoryCategory, MemoryDecision, MemoryRecord};
    use super::*;

    /// Unicode-aware tokenization treats CJK character runs as single
    /// tokens (because `is_alphanumeric` recognises them as alphanumeric).
    /// This is an intentional improvement over the previous ASCII-only
    /// split which discarded CJK entirely.
    #[test]
    fn tokenize_preserves_cjk_runs() {
        // Pure CJK: "用户偏好暗色主题" → one continuous alphanumeric run → single token
        let terms = tokenize_to_set(
            "用户偏好暗色主题",
            &TokenizeConfig {
                trim_edges: false,
                min_len: 1,
                preserve_semantic_short_words: false,
            },
        );
        assert!(terms.contains("用户偏好暗色主题"));

        // CJK and English separated by space: "用户偏好 dark theme"
        // → "用户偏好" (CJK run), "dark" (ASCII run), "theme" (ASCII run)
        let mixed = tokenize_to_set(
            "用户偏好 dark theme",
            &TokenizeConfig {
                trim_edges: false,
                min_len: 1,
                preserve_semantic_short_words: false,
            },
        );
        assert!(mixed.contains("用户偏好"));
        assert!(mixed.contains("dark"));
        assert!(mixed.contains("theme"));
    }

    /// Verify that identical Chinese content produces high overlap and
    /// triggers merge (not create).
    #[test]
    fn deterministic_merge_detects_overlap_with_chinese() {
        let category = MemoryCategory::Preferences;
        let candidate_content = "用户偏好暗色主题而非亮色";
        let existing = vec![MemoryRecord {
            uri: "mem://pref/dark-mode".to_owned(),
            category: MemoryCategory::Preferences,
            content: "用户偏好暗色主题而非亮色".to_owned(),
        }];
        let decision = deterministic_merge(category, candidate_content, &existing);
        assert_eq!(decision.primary, MemoryDecision::Merge);
        assert_eq!(decision.target_uri.as_deref(), Some("mem://pref/dark-mode"));
    }
}
