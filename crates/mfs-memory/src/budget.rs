//! Budget module — token budget allocation across overlay/facts/episodes.
//!
//! Pure arithmetic — overlay takes its actual cost, remaining is split
//! 2/3 facts / 1/3 episodes (with minimum episode budget floor).
//! Also provides cap functions to truncate facts/episodes by budget.

use crate::{
    DEFAULT_INJECTION_BUDGET, EPISODE_BUDGET_DENOMINATOR, EpisodeSummary, FactEntry,
    MIN_EPISODE_BUDGET_TOKENS, OverlayEntry, SearchStrategy,
};

/// Budget allocation result for context injection.
#[derive(Debug, Clone)]
pub struct BudgetAllocation {
    /// Token budget for overlay section.
    pub overlay_budget: usize,
    /// Token budget for facts section.
    pub facts_budget: usize,
    /// Token budget for episodes section.
    pub episodes_budget: usize,
    /// Total budget.
    pub total_budget: usize,
}

/// Allocate token budget across overlay, facts, and episodes.
///
/// Overlay takes its actual token cost; remaining budget is split
/// 1/EPISODE_BUDGET_DENOMINATOR to episodes (with MIN_EPISODE_BUDGET_TOKENS floor),
/// rest to facts.
///
/// `strategy` controls budget multiplier:
/// - `Comprehensive`: budget ×2 for maximum recall
/// - Other strategies: budget unchanged
pub fn plan_section_budgets(
    request_budget: usize,
    overlay: &[OverlayEntry],
    fact_count: usize,
    episode_count: usize,
    strategy: SearchStrategy,
) -> (usize, usize) {
    let budget = if request_budget == 0 {
        DEFAULT_INJECTION_BUDGET
    } else {
        request_budget
    };
    let effective_budget = if strategy == SearchStrategy::Comprehensive {
        budget * 2
    } else {
        budget
    };
    let remaining = effective_budget.saturating_sub(estimate_overlay_tokens(overlay));
    if remaining == 0 {
        return (0, 0);
    }
    if fact_count == 0 {
        return (0, remaining);
    }
    if episode_count == 0 {
        return (remaining, 0);
    }

    let episode_budget = if remaining < MIN_EPISODE_BUDGET_TOKENS * 2 {
        0 // not enough room for episodes; give all to facts
    } else {
        std::cmp::max(
            remaining / EPISODE_BUDGET_DENOMINATOR,
            MIN_EPISODE_BUDGET_TOKENS,
        )
    };
    let fact_budget = remaining - episode_budget;
    (fact_budget, episode_budget)
}

/// Estimate overlay tokens (~4 chars per token + 2 overhead per entry).
pub fn estimate_overlay_tokens(overlay: &[OverlayEntry]) -> usize {
    overlay.iter().map(|o| (o.content.len() / 4) + 2).sum()
}

/// Compute priority for a fact based on its predicate.
/// Lower number = higher priority (sorted first).
/// - procedure/convention/environment = 0 (highest)
/// - preference/style = 1
/// - others = 2
fn fact_priority(predicate: &str) -> u8 {
    let lower = predicate.to_lowercase();
    if lower.contains("procedure") || lower.contains("convention") || lower.contains("environment")
    {
        0
    } else if lower.contains("preference") || lower.contains("style") {
        1
    } else {
        2
    }
}

/// Cap facts by token budget.
/// Sorts facts by predicate priority (procedure > preference > other),
/// then by confidence descending, before greedy truncation.
pub fn cap_facts_by_budget(facts: &[FactEntry], budget: usize) -> Vec<FactEntry> {
    if budget == 0 {
        return Vec::new();
    }

    let mut sorted: Vec<&FactEntry> = facts.iter().collect();
    sorted.sort_by(|a, b| {
        let pa = fact_priority(&a.predicate);
        let pb = fact_priority(&b.predicate);
        pa.cmp(&pb).then(
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal),
        )
    });

    let mut kept: Vec<FactEntry> = Vec::new();
    let mut used: usize = 0;
    for f in sorted {
        let cost = (f.display_value.len() / 4) + 2;
        if !kept.is_empty() && used + cost > budget {
            break;
        }
        kept.push(f.clone());
        used += cost;
    }
    kept
}

/// Cap episodes by token budget.
pub fn cap_episodes_by_budget(episodes: &[EpisodeSummary], budget: usize) -> Vec<EpisodeSummary> {
    if budget == 0 {
        return Vec::new();
    }

    let mut kept: Vec<EpisodeSummary> = Vec::new();
    let mut used: usize = 0;
    for e in episodes {
        let cost = (e.summary.len() / 4) + 2;
        if !kept.is_empty() && used + cost > budget {
            break;
        }
        kept.push(e.clone());
        used += cost;
    }
    kept
}

/// Estimate fact tokens.
pub fn estimate_fact_tokens(facts: &[FactEntry]) -> usize {
    facts.iter().map(|f| (f.display_value.len() / 4) + 2).sum()
}

/// Estimate episode tokens.
pub fn estimate_episode_tokens(episodes: &[EpisodeSummary]) -> usize {
    episodes.iter().map(|e| (e.summary.len() / 4) + 2).sum()
}

/// Compute remaining unified resource budget.
pub fn remaining_unified_resource_budget(
    request_budget: usize,
    overlay: &[OverlayEntry],
    facts: &[FactEntry],
    episodes: &[EpisodeSummary],
) -> usize {
    let budget = if request_budget == 0 {
        DEFAULT_INJECTION_BUDGET
    } else {
        request_budget
    };
    budget
        .saturating_sub(estimate_overlay_tokens(overlay))
        .saturating_sub(estimate_fact_tokens(facts))
        .saturating_sub(estimate_episode_tokens(episodes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TurnRole;

    fn make_overlay(content: &str) -> OverlayEntry {
        OverlayEntry {
            turn_id: "t1".to_owned(),
            role: TurnRole::User,
            content: content.to_owned(),
        }
    }

    fn make_fact(display: &str) -> FactEntry {
        FactEntry {
            fact_id: "f1".to_owned(),
            predicate: "test".to_owned(),
            display_value: display.to_owned(),
            confidence: 0.9,
            staleness_note: None,
            valid_from: None,
        }
    }

    fn make_episode(summary: &str) -> EpisodeSummary {
        EpisodeSummary {
            episode_id: "ep1".to_owned(),
            summary: summary.to_owned(),
            salience: 0.5,
            strength: 1.0,
            recall_count: 0,
            emotional_valence: None,
            emotional_intensity: None,
            context_tags_json: None,
            embedding_json: None,
            created_at: None,
        }
    }

    #[test]
    fn plan_budget_default() {
        let (facts, episodes) = plan_section_budgets(0, &[], 3, 5, SearchStrategy::Precision);
        assert!(facts > 0);
        assert!(episodes > 0);
        // episodes should be ~1/3 of default budget (1200)
        assert!(episodes >= MIN_EPISODE_BUDGET_TOKENS);
    }

    #[test]
    fn plan_budget_with_overlay() {
        let overlay = vec![make_overlay(&"x".repeat(400))]; // ~102 tokens
        let (facts, episodes) =
            plan_section_budgets(1200, &overlay, 3, 5, SearchStrategy::Precision);
        assert!(facts > episodes); // facts get 2/3 of remaining
    }

    #[test]
    fn plan_budget_no_facts() {
        let (_, episodes) = plan_section_budgets(1200, &[], 0, 5, SearchStrategy::Precision);
        assert_eq!(episodes, 1200); // all remaining goes to episodes
    }

    #[test]
    fn plan_budget_no_episodes() {
        let (facts, _) = plan_section_budgets(1200, &[], 3, 0, SearchStrategy::Precision);
        assert_eq!(facts, 1200); // all remaining goes to facts
    }

    #[test]
    fn plan_budget_comprehensive_doubles_budget() {
        let (facts_prec, ep_prec) =
            plan_section_budgets(1200, &[], 3, 5, SearchStrategy::Precision);
        let (facts_comp, ep_comp) =
            plan_section_budgets(1200, &[], 3, 5, SearchStrategy::Comprehensive);
        assert_eq!(facts_comp + ep_comp, facts_prec * 2 + ep_prec * 2);
    }

    #[test]
    fn test_estimate_overlay_tokens() {
        let overlay = vec![make_overlay(&"x".repeat(100))]; // ~27 tokens
        let tokens = estimate_overlay_tokens(&overlay);
        assert_eq!(tokens, (100 / 4) + 2);
    }

    #[test]
    fn test_cap_facts_by_budget() {
        let facts = vec![
            make_fact(&"x".repeat(100)), // ~27 tokens
            make_fact(&"y".repeat(100)), // ~27 tokens
        ];
        let capped = cap_facts_by_budget(&facts, 30);
        assert_eq!(capped.len(), 1); // only first fits
    }

    #[test]
    fn test_cap_episodes_by_budget() {
        let episodes = vec![
            make_episode(&"x".repeat(100)),
            make_episode(&"y".repeat(100)),
        ];
        let capped = cap_episodes_by_budget(&episodes, 30);
        assert_eq!(capped.len(), 1);
    }

    #[test]
    fn cap_zero_budget() {
        let facts = vec![make_fact("test")];
        let capped = cap_facts_by_budget(&facts, 0);
        assert!(capped.is_empty());
    }

    #[test]
    fn remaining_budget() {
        let overlay = vec![make_overlay(&"x".repeat(100))];
        let facts = vec![make_fact(&"y".repeat(100))];
        let episodes = vec![make_episode(&"z".repeat(100))];
        let remaining = remaining_unified_resource_budget(1200, &overlay, &facts, &episodes);
        assert!(remaining < 1200);
    }

    #[test]
    fn cap_facts_procedure_before_preference() {
        let facts = vec![
            FactEntry {
                fact_id: "f1".to_owned(),
                predicate: "preference.color".to_owned(),
                display_value: "likes blue color very much".to_owned(),
                confidence: 0.9,
                staleness_note: None,
                valid_from: None,
            },
            FactEntry {
                fact_id: "f2".to_owned(),
                predicate: "convention.coding_style".to_owned(),
                display_value: "uses tabs not spaces".to_owned(),
                confidence: 0.8,
                staleness_note: None,
                valid_from: None,
            },
            FactEntry {
                fact_id: "f3".to_owned(),
                predicate: "other.info".to_owned(),
                display_value: "some random info".to_owned(),
                confidence: 0.7,
                staleness_note: None,
                valid_from: None,
            },
        ];
        // Budget only fits 1 fact (each ~8-9 tokens)
        let capped = cap_facts_by_budget(&facts, 8);
        assert_eq!(capped.len(), 1);
        assert_eq!(capped[0].predicate, "convention.coding_style");
    }

    #[test]
    fn cap_facts_same_priority_sorts_by_confidence() {
        let facts = vec![
            FactEntry {
                fact_id: "f1".to_owned(),
                predicate: "preference.lang".to_owned(),
                display_value: "likes Rust language".to_owned(),
                confidence: 0.6,
                staleness_note: None,
                valid_from: None,
            },
            FactEntry {
                fact_id: "f2".to_owned(),
                predicate: "preference.tool".to_owned(),
                display_value: "likes vim editor".to_owned(),
                confidence: 0.95,
                staleness_note: None,
                valid_from: None,
            },
        ];
        // Budget fits 1 fact — should pick the higher confidence preference
        let capped = cap_facts_by_budget(&facts, 7);
        assert_eq!(capped.len(), 1);
        assert_eq!(capped[0].confidence, 0.95);
    }
}
