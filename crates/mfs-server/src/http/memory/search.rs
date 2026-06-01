use super::api_types::SearchStrategy;
use super::*;
use mfs_memory::episodes::{HOTNESS_ALPHA, MMR_LAMBDA, ScoredEpisode, rerank_episodes_with_mmr};
use mfs_metadata::EpisodeRow;

// ── Memory-specific handlers (MemFuse) ──────────────────────────

#[derive(Debug, Deserialize)]
pub(in crate::http) struct MemorySearchRequest {
    pub query: String,
    pub top_k: Option<usize>,
    pub limit: Option<usize>,
    pub user_id: Option<String>,
    pub session_id: Option<String>,
    pub thread_id: Option<String>,
    /// Search strategy preset: precision (default), diverse, recent, comprehensive
    pub strategy: Option<SearchStrategy>,
}

#[derive(Debug, Clone, Serialize)]
pub(in crate::http) struct MemorySearchHit {
    pub episode_id: String,
    pub session_id: String,
    pub summary: String,
    pub salience_score: f64,
    pub score: f64,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub(in crate::http) struct MemorySearchResponse {
    pub results: Vec<MemorySearchHit>,
    pub query: String,
    pub total: usize,
}

pub(in crate::http) async fn memory_search(
    State(state): State<Arc<AppState>>,
    req: Json<MemorySearchRequest>,
) -> HandlerResult<Json<MemorySearchResponse>> {
    if let Some(uid) = &req.user_id {
        if uid.is_empty() {
            return Err(AppError(MfsError::InvalidArgument {
                field: "user_id".into(),
                reason: "user_id must not be empty".into(),
            }));
        }
    }

    let metadata = state.metadata.clone();
    let user_id = req
        .user_id
        .as_deref()
        .unwrap_or(state.config.user_id.as_str());
    let session_scope = req.session_id.as_deref().or(req.thread_id.as_deref());
    let top_k = req.top_k.or(req.limit).unwrap_or(10);
    let strategy: mfs_memory::SearchStrategy = req.strategy.unwrap_or_default().into();
    let terms = normalized_query_terms(&req.query);
    let query_embedding =
        mfs_semantic::try_embed_query(&req.query, state.embedding_provider.as_ref()).await;
    let mut episodes = metadata
        .get_episodes_by_user(&state.config.account_id, user_id, None)
        .map_err(AppError::from_error)?;
    if let Some(session_id) = session_scope {
        episodes.retain(|episode| episode.session_id == session_id);
    }

    let mut hits: Vec<_> = episodes
        .into_iter()
        .filter_map(|episode| {
            let score = if let Some(ref qemb) = query_embedding {
                // Use embedding-based scoring when available
                if let Some(ep_emb) = mfs_semantic::parse_embedding_json(&episode.embedding_json) {
                    let sim = mfs_semantic::cosine_similarity(qemb, &ep_emb);
                    (1.0 - HOTNESS_ALPHA) * sim + HOTNESS_ALPHA * episode.salience_score
                } else {
                    // No embedding on this episode — fall back to lexical
                    episode_search_score(&episode, &terms)
                }
            } else {
                episode_search_score(&episode, &terms)
            };
            if !terms.is_empty() && score <= 0.0 {
                return None;
            }
            Some(MemorySearchHit {
                episode_id: episode.episode_id,
                session_id: episode.session_id,
                summary: episode.summary,
                salience_score: episode.salience_score,
                score,
                created_at: episode.created_at,
            })
        })
        .collect();
    hits.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                right
                    .salience_score
                    .partial_cmp(&left.salience_score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| right.created_at.cmp(&left.created_at))
            .then_with(|| left.episode_id.cmp(&right.episode_id))
    });

    // §2.2: Strategy-aware episode reranking (MMR for Diverse, recency boost for Recent)
    if hits.len() > top_k && strategy == mfs_memory::SearchStrategy::Diverse {
        let scored: Vec<ScoredEpisode> = hits
            .iter()
            .map(|h| ScoredEpisode {
                episode: mfs_memory::EpisodeSummary {
                    episode_id: h.episode_id.clone(),
                    summary: h.summary.clone(),
                    salience: h.salience_score,
                    strength: 0.0,
                    recall_count: 0,
                    emotional_valence: Some(0.0),
                    emotional_intensity: Some(0.0),
                    context_tags_json: None,
                    embedding_json: None,
                    created_at: Some(h.created_at.clone()),
                },
                score: h.score,
            })
            .collect();
        let reranked = rerank_episodes_with_mmr(scored, MMR_LAMBDA, top_k);
        // Rebuild hits from reranked results, preserving original score for display
        let reranked_ids: Vec<String> = reranked.iter().map(|e| e.episode_id.clone()).collect();
        hits.retain(|h| reranked_ids.contains(&h.episode_id));
        hits.sort_by(|a, b| {
            let ai = reranked_ids.iter().position(|id| *id == a.episode_id);
            let bi = reranked_ids.iter().position(|id| *id == b.episode_id);
            ai.cmp(&bi)
        });
    }

    let total = hits.len();
    hits.truncate(top_k);

    Ok(Json(MemorySearchResponse {
        results: hits,
        query: req.query.clone(),
        total,
    }))
}

fn normalized_query_terms(query: &str) -> Vec<String> {
    let config = mfs_types::text::TokenizeConfig {
        trim_edges: false,
        min_len: 1,
        preserve_semantic_short_words: false,
    };
    mfs_types::text::tokenize_to_vec(query, &config)
}

fn episode_search_score(episode: &EpisodeRow, terms: &[String]) -> f64 {
    if terms.is_empty() {
        return episode.salience_score;
    }

    let haystack = format!(
        "{} {}",
        episode.summary.to_ascii_lowercase(),
        episode
            .keywords_json
            .as_deref()
            .unwrap_or_default()
            .to_ascii_lowercase(),
    );
    terms.iter().fold(0.0, |score, term| {
        if haystack.contains(term) {
            score + 1.0
        } else {
            score
        }
    })
}
