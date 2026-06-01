//! Memory candidate extraction facade.
//!
//! Delegates to `extract_llm` (LLM-assisted) or `extract_rules` (deterministic fallback).

use super::extract_llm;
use super::extract_rules;
use super::schema::MemoryCandidate;

/// Extract memory candidates from session messages.
///
/// Strategy:
/// 1. If a `ChatProvider` is available (LLM configured), call the LLM with the
///    full conversation and parse the structured JSON response.
/// 2. Fall back to deterministic rule-based extraction when LLM is unavailable
///    or returns an unparseable response.
///
/// The LLM path produces rich L0/L1/L2 candidates with proper category
/// classification.  The rule-based path produces minimal candidates that are
/// still useful for basic memory writeback.
pub async fn extract_memory_candidates(
    messages: &[(String, String)],
    usage: &[(String, String, Option<bool>)],
) -> Vec<MemoryCandidate> {
    // Try LLM extraction first.
    if let Some(candidates) = extract_llm::try_llm_extract(messages, usage).await {
        if !candidates.is_empty() {
            return candidates;
        }
    }
    // Fall back to deterministic rules.
    deterministic_extract(messages, usage)
}

/// Re-export deterministic extraction for direct use.
pub fn deterministic_extract(
    messages: &[(String, String)],
    usage: &[(String, String, Option<bool>)],
) -> Vec<MemoryCandidate> {
    extract_rules::deterministic_extract(messages, usage)
}
