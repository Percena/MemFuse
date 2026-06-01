use async_trait::async_trait;
use mfs_index::SearchHit;
use mfs_types::text::{TokenizeConfig, tokenize_to_set};

use crate::hierarchical::RankedLayeredHit;

#[async_trait]
pub trait Reranker: Send + Sync {
    async fn rerank_search_hits(&self, query: &str, hits: Vec<SearchHit>) -> Vec<SearchHit>;
    async fn rerank_ranked_hits(
        &self,
        query: &str,
        hits: Vec<RankedLayeredHit>,
    ) -> Vec<RankedLayeredHit>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DeterministicReranker;

#[async_trait]
impl Reranker for DeterministicReranker {
    async fn rerank_search_hits(&self, query: &str, mut hits: Vec<SearchHit>) -> Vec<SearchHit> {
        hits.sort_by(|left, right| {
            score_text(query, &right.excerpt, &right.uri)
                .total_cmp(&score_text(query, &left.excerpt, &left.uri))
                .then_with(|| left.score.total_cmp(&right.score))
                .then_with(|| left.uri.cmp(&right.uri))
        });
        hits
    }

    async fn rerank_ranked_hits(
        &self,
        query: &str,
        mut hits: Vec<RankedLayeredHit>,
    ) -> Vec<RankedLayeredHit> {
        hits.sort_by(|left, right| {
            score_text(query, &right.hit.excerpt, &right.hit.uri)
                .total_cmp(&score_text(query, &left.hit.excerpt, &left.hit.uri))
                .then_with(|| right.matched_levels.len().cmp(&left.matched_levels.len()))
                .then_with(|| left.hit.score.total_cmp(&right.hit.score))
                .then_with(|| left.hit.uri.cmp(&right.hit.uri))
        });
        hits
    }
}

pub(crate) fn score_text(query: &str, excerpt: &str, uri: &str) -> f64 {
    let config = TokenizeConfig {
        trim_edges: true,
        min_len: 3,
        preserve_semantic_short_words: false,
    };
    let query_terms = tokenize_to_set(query, &config);
    if query_terms.is_empty() {
        return 0.0;
    }

    let text_terms = tokenize_to_set(&format!("{excerpt} {uri}"), &config);
    let overlap = query_terms
        .iter()
        .filter(|term| text_terms.contains(term.as_str()))
        .count() as f64;
    overlap / query_terms.len() as f64
}
