//! Episode search — scoring, reranking, and MMR diversity.

use chrono::Utc;

use super::annotate::estimate_query_valence;
use crate::EpisodeSummary;

/// Scored episode for reranking.
#[derive(Debug, Clone)]
pub struct ScoredEpisode {
    pub episode: EpisodeSummary,
    pub score: f64,
}

/// MMR lambda for diversity reranking.
/// 0.7 = favor relevance over diversity (programming assistant context
/// where relevance is more important than broad coverage).
pub const MMR_LAMBDA: f64 = 0.7;

/// Hotness blend factor for unified scoring (OV-P1-5).
/// final_score = (1 - HOTNESS_ALPHA) * embedding_similarity + HOTNESS_ALPHA * hotness_score
pub const HOTNESS_ALPHA: f64 = 0.25;

/// Valence boost weight for query-aware episode ranking (§4.3).
/// Max boost = |valence| × VALENCE_BOOST_WEIGHT ≈ 0.8 × 0.15 = 0.12,
/// which is meaningful but doesn't overwhelm the hotness base.
pub const VALENCE_BOOST_WEIGHT: f64 = 0.15;

/// Rerank episodes by unified scoring formula, then tie-break by
/// episode_id. The unified score blends embedding similarity with
/// salience/strength/recall hotness (OV-P1-5~9).
///
/// Episodes with strong emotional valence (|valence| >= 0.7) receive a
/// modest attention boost proportional to |valence| × VALENCE_BOOST_WEIGHT × 0.3,
/// since high-signal episodes deserve attention regardless of query direction.
/// For query-aware directional valence weighting, see `rerank_episodes_with_query`.
pub fn rerank_episodes(
    scores: Vec<(usize, f64)>,
    episodes: &[EpisodeSummary],
    top_k: usize,
) -> Vec<EpisodeSummary> {
    rerank_episodes_with_query(
        scores,
        episodes,
        top_k,
        None,
        crate::SearchStrategy::Precision,
    )
}

/// Rerank episodes with query-aware valence weighting (§4.3).
/// When a query string is provided, valence boost direction aligns with
/// query polarity: negation queries boost negative-valence episodes,
/// affirmative queries boost positive-valence episodes.
///
/// `strategy` controls recency boost intensity:
/// - `Recent`: 24h → 2.0×, 7d → 1.3× (enhanced)
/// - Other strategies: 24h → 1.5× (standard, unchanged)
pub fn rerank_episodes_with_query(
    scores: Vec<(usize, f64)>,
    episodes: &[EpisodeSummary],
    top_k: usize,
    query: Option<&str>,
    strategy: crate::SearchStrategy,
) -> Vec<EpisodeSummary> {
    // Determine query polarity from keywords (§4.3)
    let query_valence = query.map(estimate_query_valence).unwrap_or(0.0);

    let mut scored: Vec<ScoredEpisode> = episodes
        .iter()
        .enumerate()
        .map(|(i, ep)| {
            let similarity = scores
                .iter()
                .find(|(idx, _)| *idx == i)
                .map(|(_, s)| *s)
                .unwrap_or(-1.0);

            let mut hotness = compute_hotness(
                ep.salience,
                ep.strength,
                ep.recall_count,
                ep.emotional_intensity,
            );

            // §4.3 Valence-weighted boost: align episode valence with query polarity.
            // Applied BEFORE recency boost to avoid saturation: when recency boost
            // clamps hotness to 1.0, an additive valence boost becomes a dead letter.
            // By applying valence first, the directional signal survives the recency clamp.
            let valence_boost = match ep.emotional_valence {
                Some(v) if v * query_valence > 0.0 => {
                    // Same polarity: boost proportional to valence magnitude
                    v.abs() * VALENCE_BOOST_WEIGHT
                }
                Some(v) if v.abs() >= 0.7 && query_valence == 0.0 => {
                    // No query polarity but strong valence: modest boost (high-signal episodes
                    // deserve attention regardless of query direction)
                    v.abs() * VALENCE_BOOST_WEIGHT * 0.3
                }
                _ => 0.0,
            };
            hotness = (hotness + valence_boost).min(1.0);

            // §10.2.2 Recency boost: strategy-aware multiplier
            // Standard (precision/diverse/comprehensive): 24h → 1.5×
            // Recent strategy: 24h → 2.0×, 7d → 1.3×
            if let Some(ref created_at) = ep.created_at {
                if let Ok(created_dt) = chrono::DateTime::parse_from_rfc3339(created_at) {
                    let age_hours = (Utc::now() - created_dt.with_timezone(&Utc)).num_hours();
                    if age_hours >= 0 && age_hours <= 24 {
                        let recency_mult = if strategy == crate::SearchStrategy::Recent {
                            2.0
                        } else {
                            1.5
                        };
                        hotness = (hotness * recency_mult).min(1.0);
                    } else if strategy == crate::SearchStrategy::Recent
                        && age_hours > 24
                        && age_hours <= 168
                    {
                        // 7-day window: 1.3× boost for Recent strategy
                        hotness = (hotness * 1.3).min(1.0);
                    }
                }
            }

            let final_score = (1.0 - HOTNESS_ALPHA) * similarity + HOTNESS_ALPHA * hotness;

            ScoredEpisode {
                episode: ep.clone(),
                score: final_score,
            }
        })
        .collect();

    // Sort: unified score desc, then tie-break by episode_id
    scored.sort_by(|a, b| {
        if a.score == b.score {
            a.episode.episode_id.cmp(&b.episode.episode_id)
        } else {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        }
    });

    // §2.2: Apply MMR post-processing when strategy is Diverse
    if strategy == crate::SearchStrategy::Diverse {
        rerank_episodes_with_mmr(scored, MMR_LAMBDA, top_k)
    } else {
        scored
            .iter()
            .take(top_k)
            .map(|s| s.episode.clone())
            .collect()
    }
}

/// MMR (Maximal Marginal Relevance) diversity reranking.
/// Selects episodes iteratively: each step picks the candidate with highest
///   MMR(D_i) = λ·Sim(D_i,Q) − (1−λ)·max[Sim(D_i,D_j)] for j in S
/// where S is the already-selected set. This penalizes candidates similar
/// to already-selected episodes, producing a diverse top-k.
///
/// Deterministic fallback: when embedding similarity between candidates
/// is unavailable, uses context_tags_json overlap (Jaccard) as a proxy for
/// inter-episode similarity. This ensures MMR never blocks on LLM/embeddings.
pub fn rerank_episodes_with_mmr(
    scored: Vec<ScoredEpisode>,
    lambda: f64,
    top_k: usize,
) -> Vec<EpisodeSummary> {
    if scored.is_empty() || top_k == 0 {
        return Vec::new();
    }
    // When lambda >= 1.0 or only 1 result needed, MMR degenerates to pure relevance
    if lambda >= 1.0 || top_k == 1 {
        return scored
            .iter()
            .take(top_k)
            .map(|s| s.episode.clone())
            .collect();
    }

    // Build relevance scores (normalized to [0,1] from ScoredEpisode.score)
    let max_score = scored.iter().map(|s| s.score).fold(0.0f64, |a, b| a.max(b));
    let min_score = scored.iter().map(|s| s.score).fold(1.0f64, |a, b| a.min(b));
    let score_range = max_score - min_score;
    let relevance: Vec<f64> = scored
        .iter()
        .map(|s| {
            if score_range > 0.0 {
                (s.score - min_score) / score_range
            } else {
                1.0
            }
        })
        .collect();

    // Compute pairwise similarity between episodes using keywords (deterministic fallback).
    let keywords: Vec<Vec<String>> = scored
        .iter()
        .map(|s| {
            s.episode
                .context_tags_json
                .as_deref()
                .and_then(|json| serde_json::from_str::<Vec<String>>(json).ok())
                .unwrap_or_default()
        })
        .collect();

    let pairwise_sim: Vec<Vec<f64>> = scored
        .iter()
        .enumerate()
        .map(|(i, _)| {
            scored
                .iter()
                .enumerate()
                .map(|(j, _)| {
                    if i == j {
                        return 1.0;
                    }
                    // Jaccard on keywords (deterministic)
                    if !keywords[i].is_empty() && !keywords[j].is_empty() {
                        let set_i: std::collections::HashSet<&String> =
                            keywords[i].iter().collect();
                        let set_j: std::collections::HashSet<&String> =
                            keywords[j].iter().collect();
                        let intersection = set_i.intersection(&set_j).count() as f64;
                        let union = set_i.union(&set_j).count() as f64;
                        if union > 0.0 {
                            intersection / union
                        } else {
                            0.0
                        }
                    } else {
                        // Fallback: summary string word overlap
                        let words_i: std::collections::HashSet<&str> =
                            scored[i].episode.summary.split_whitespace().collect();
                        let words_j: std::collections::HashSet<&str> =
                            scored[j].episode.summary.split_whitespace().collect();
                        if words_i.is_empty() || words_j.is_empty() {
                            0.0
                        } else {
                            let intersection = words_i.intersection(&words_j).count() as f64;
                            let union = words_i.union(&words_j).count() as f64;
                            intersection / union
                        }
                    }
                })
                .collect()
        })
        .collect();

    // Greedy MMR selection
    let mut selected_indices: Vec<usize> = Vec::new();
    let mut remaining: std::collections::HashSet<usize> = (0..scored.len()).collect();

    // First selection: highest relevance
    let first = relevance
        .iter()
        .enumerate()
        .filter(|(i, _)| remaining.contains(i))
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(i, _)| i)
        .unwrap_or(0);
    selected_indices.push(first);
    remaining.remove(&first);

    // Subsequent selections: MMR formula
    while selected_indices.len() < top_k && !remaining.is_empty() {
        let best = remaining
            .iter()
            .map(|&i| {
                let rel = relevance[i];
                // max similarity to any already-selected episode
                let max_sim_to_selected = selected_indices
                    .iter()
                    .map(|&j| pairwise_sim[i][j])
                    .fold(0.0f64, |acc, sim| acc.max(sim));
                let mmr_score = lambda * rel - (1.0 - lambda) * max_sim_to_selected;
                (i, mmr_score)
            })
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(i, _)| i)
            .unwrap_or(*remaining.iter().next().unwrap_or(&0));

        selected_indices.push(best);
        remaining.remove(&best);
    }

    selected_indices
        .iter()
        .map(|&i| scored[i].episode.clone())
        .collect()
}

/// Compute hotness score from salience, strength, and recall count (OV-P1-5).
/// Normalizes each component to [0, 1] range and combines them.
/// Optional emotional_intensity >= 0.7 adds a +0.1 boost.
pub fn compute_hotness(
    salience: f64,
    strength: f64,
    recall_count: usize,
    emotional_intensity: Option<f64>,
) -> f64 {
    // Normalize: salience ∈ [0.1, 1.0] → scale to [0, 1]
    let salience_norm = (salience - 0.1) / 0.9;
    // Normalize: strength ∈ [0.1, 2.0] → scale to [0, 1]
    let strength_norm = (strength - 0.1) / 1.9;
    // Normalize: recall ∈ [0, ∞) → log-scale to [0, 1]
    let recall_norm = if recall_count > 0 {
        (1.0 + recall_count as f64).ln() / 5.0
    } else {
        0.0
    };

    // Weighted blend: salience 40%, strength 40%, recall 20%
    let base = 0.4 * salience_norm + 0.4 * strength_norm + 0.2 * recall_norm;

    // Emotional intensity boost: +0.1 for highly intense episodes
    let emotion_boost = match emotional_intensity {
        Some(i) if i >= 0.7 => 0.1,
        _ => 0.0,
    };

    (base + emotion_boost).min(1.0)
}

/// Limit episodes to top_k.
pub fn limit_episodes(episodes: &[EpisodeSummary], top_k: usize) -> Vec<EpisodeSummary> {
    if episodes.len() <= top_k {
        episodes.to_vec()
    } else {
        episodes[..top_k].to_vec()
    }
}

/// Extract a keyword from a query for fallback search.
/// Takes the longest non-stop-word.
pub fn extract_keyword(query: &str) -> String {
    let stop_words = [
        "the", "a", "an", "is", "are", "was", "were", "i", "my", "me", "我", "的", "了", "是",
        "在", "你", "他", "她", "它", "们",
    ];

    let words = query.split_whitespace();
    let mut best = String::new();

    for w in words {
        let cleaned = w.trim_matches(|c: char| {
            c == '.'
                || c == ','
                || c == '?'
                || c == '!'
                || c == '，'
                || c == '。'
                || c == '？'
                || c == '！'
        });
        if cleaned.len() > best.len() && !stop_words.contains(&cleaned.to_lowercase().as_str()) {
            best = cleaned.to_owned();
        }
    }

    if best.chars().count() > 20 {
        // Truncate by characters (not bytes) for UTF-8 safety
        let truncated: String = best.chars().take(20).collect();
        best = truncated;
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_limit_episodes() {
        let episodes: Vec<EpisodeSummary> = (0..10)
            .map(|i| EpisodeSummary {
                episode_id: format!("ep-{}", i),
                summary: "test".to_owned(),
                salience: 0.5,
                strength: 1.0,
                recall_count: 0,
                emotional_valence: None,
                emotional_intensity: None,
                context_tags_json: None,
                embedding_json: None,
                created_at: None,
            })
            .collect();
        let limited = limit_episodes(&episodes, 5);
        assert_eq!(limited.len(), 5);
    }

    #[test]
    fn test_extract_keyword() {
        assert_eq!(
            extract_keyword("what is the user's location preference"),
            "preference"
        );
        let kw = extract_keyword("我的工作是什么");
        assert!(!kw.is_empty());
        assert_eq!(extract_keyword("location preference details"), "preference");
    }

    #[test]
    fn hotness_with_positive_valence() {
        let episodes = vec![EpisodeSummary {
            episode_id: "ep-pos".to_owned(),
            summary: "User was excited about the feature".to_owned(),
            salience: 0.8,
            strength: 1.0,
            recall_count: 2,
            emotional_valence: Some(0.9),
            emotional_intensity: Some(0.5),
            context_tags_json: Some("[\"mood:excited\"]".to_owned()),
            embedding_json: None,
            created_at: None,
        }];
        let scores: Vec<(usize, f64)> = vec![(0, 0.5)];
        let ranked = rerank_episodes(scores, &episodes, 10);
        assert_eq!(ranked.len(), 1);
    }

    #[test]
    fn hotness_with_negative_valence() {
        let episodes = vec![EpisodeSummary {
            episode_id: "ep-neg".to_owned(),
            summary: "User was frustrated with errors".to_owned(),
            salience: 0.8,
            strength: 1.0,
            recall_count: 2,
            emotional_valence: Some(-0.7),
            emotional_intensity: Some(0.8),
            context_tags_json: Some("[\"mood:frustrated\"]".to_owned()),
            embedding_json: None,
            created_at: None,
        }];
        let scores: Vec<(usize, f64)> = vec![(0, 0.5)];
        let ranked = rerank_episodes(scores, &episodes, 10);
        assert_eq!(ranked.len(), 1);
    }

    #[test]
    fn hotness_with_high_intensity_boost() {
        let base = compute_hotness(0.8, 1.0, 2, None);
        let boosted = compute_hotness(0.8, 1.0, 2, Some(0.9));
        assert!(boosted > base, "high intensity should boost hotness");
        assert!(boosted - base >= 0.09, "boost should be ~0.1");
    }

    #[test]
    fn hotness_low_intensity_no_boost() {
        let base = compute_hotness(0.8, 1.0, 2, None);
        let low = compute_hotness(0.8, 1.0, 2, Some(0.3));
        assert_eq!(base, low, "low intensity should not boost hotness");
    }

    #[test]
    fn valence_boost_negation_query_prefers_negative_episodes() {
        let episodes = vec![
            EpisodeSummary {
                episode_id: "ep-pos".to_owned(),
                summary: "User loved the feature".to_owned(),
                salience: 0.6,
                strength: 1.0,
                recall_count: 0,
                emotional_valence: Some(0.8),
                emotional_intensity: None,
                context_tags_json: None,
                embedding_json: None,
                created_at: None,
            },
            EpisodeSummary {
                episode_id: "ep-neg".to_owned(),
                summary: "User hated the error".to_owned(),
                salience: 0.6,
                strength: 1.0,
                recall_count: 0,
                emotional_valence: Some(-0.7),
                emotional_intensity: None,
                context_tags_json: None,
                embedding_json: None,
                created_at: None,
            },
        ];
        let scores: Vec<(usize, f64)> = vec![(0, 0.5), (1, 0.5)];
        let ranked = rerank_episodes_with_query(
            scores,
            &episodes,
            2,
            Some("don't use this approach"),
            crate::SearchStrategy::Precision,
        );
        assert_eq!(ranked[0].episode_id, "ep-neg");
    }

    #[test]
    fn valence_boost_affirmative_query_prefers_positive_episodes() {
        let episodes = vec![
            EpisodeSummary {
                episode_id: "ep-pos".to_owned(),
                summary: "User loved the feature".to_owned(),
                salience: 0.6,
                strength: 1.0,
                recall_count: 0,
                emotional_valence: Some(0.8),
                emotional_intensity: None,
                context_tags_json: None,
                embedding_json: None,
                created_at: None,
            },
            EpisodeSummary {
                episode_id: "ep-neg".to_owned(),
                summary: "User hated the error".to_owned(),
                salience: 0.6,
                strength: 1.0,
                recall_count: 0,
                emotional_valence: Some(-0.7),
                emotional_intensity: None,
                context_tags_json: None,
                embedding_json: None,
                created_at: None,
            },
        ];
        let scores: Vec<(usize, f64)> = vec![(0, 0.5), (1, 0.5)];
        let ranked = rerank_episodes_with_query(
            scores,
            &episodes,
            2,
            Some("I always prefer this style"),
            crate::SearchStrategy::Precision,
        );
        assert_eq!(ranked[0].episode_id, "ep-pos");
    }

    #[test]
    fn valence_neutral_query_no_directional_boost() {
        let episodes = vec![
            EpisodeSummary {
                episode_id: "ep-pos".to_owned(),
                summary: "User loved the feature".to_owned(),
                salience: 0.6,
                strength: 1.0,
                recall_count: 0,
                emotional_valence: Some(0.5),
                emotional_intensity: None,
                context_tags_json: None,
                embedding_json: None,
                created_at: None,
            },
            EpisodeSummary {
                episode_id: "ep-neutral".to_owned(),
                summary: "Normal conversation".to_owned(),
                salience: 0.6,
                strength: 1.0,
                recall_count: 0,
                emotional_valence: None,
                emotional_intensity: None,
                context_tags_json: None,
                embedding_json: None,
                created_at: None,
            },
        ];
        let scores: Vec<(usize, f64)> = vec![(0, 0.5), (1, 0.5)];
        let ranked_no_query = rerank_episodes(scores.clone(), &episodes, 2);
        let ranked_neutral = rerank_episodes_with_query(
            scores,
            &episodes,
            2,
            Some("explain the architecture"),
            crate::SearchStrategy::Precision,
        );
        assert_eq!(ranked_no_query[0].episode_id, ranked_neutral[0].episode_id);
    }
}
