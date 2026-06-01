//! Extended reranker trait for API-based reranking (e.g. Jina reranker v3)
//! that returns explicit relevance scores.
//!
//! The default implementation produces pseudo-scores from token overlap,
//! so existing providers satisfy this trait with an empty impl block
//! that inherits all defaults.

use async_trait::async_trait;

use crate::rerank::{DeterministicReranker, Reranker, score_text};

/// Score-based rerank result used by MemFuse's episodic search for
/// score-based filtering and tie-breaking.
#[derive(Debug, Clone, PartialEq)]
pub struct RerankScore {
    /// Original document index in the input array.
    pub index: usize,
    /// Relevance score from the reranking model (0.0–1.0 for Jina).
    pub relevance_score: f64,
}

/// Extended reranker interface for API-based reranking that returns
/// explicit relevance scores.
///
/// The default implementation produces pseudo-scores from
/// `DeterministicReranker`'s token overlap logic.  Existing providers
/// satisfy this trait with an empty impl block that inherits all defaults.
#[async_trait]
pub trait RerankerExt: Reranker {
    /// Rerank raw documents and return explicit relevance scores.
    ///
    /// This is used by MemFuse's `ReadService` for score-based
    /// episodic filtering and tie-breaking.
    async fn rerank_with_scores(
        &self,
        query: &str,
        documents: &[String],
        top_n: usize,
    ) -> Vec<RerankScore> {
        let mut scores: Vec<RerankScore> = documents
            .iter()
            .enumerate()
            .map(|(i, doc)| {
                let overlap = score_text(query, doc, "");
                RerankScore {
                    index: i,
                    relevance_score: overlap,
                }
            })
            .collect();
        scores.sort_by(|a, b| b.relevance_score.total_cmp(&a.relevance_score));
        scores.truncate(top_n);
        scores
    }
}

// ─── Default impl for existing providers ──────────────────────────────────

#[async_trait]
impl RerankerExt for DeterministicReranker {}
