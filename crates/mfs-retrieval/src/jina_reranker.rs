//! Jina reranker v3 — API-based reranking via the Jina Cloud API
//! (`https://api.jina.ai/v1/rerank`).
//!
//! This provider implements both `Reranker` and `RerankerExt`,
//! leveraging Jina's relevance scoring for higher-quality search
//! result ordering.
//!
//! **Default disabled** — activated only when `MEMFUSE_RERANK_PROVIDER=jina`
//! and `MEMFUSE_JINA_API_KEY` is set.  Falls back to `DeterministicReranker`
//! on any failure.

use std::sync::Arc;

use async_trait::async_trait;
use mfs_index::SearchHit;
use mfs_semantic::{
    ApiErrorClass, CircuitBreaker, ResilienceConfig, RetryableError, classify_api_error,
    retry_with_backoff,
};

use crate::Reranker;
use crate::hierarchical::RankedLayeredHit;
use crate::rerank::DeterministicReranker;
use crate::rerank_ext::{RerankScore, RerankerExt};

/// Default configuration values.
const DEFAULT_JINA_BASE_URL: &str = "https://api.jina.ai/v1";
const DEFAULT_JINA_RERANK_MODEL: &str = "jina-reranker-v3";

/// Jina reranker v3 provider backed by the Jina Cloud API.
#[derive(Debug)]
pub struct JinaReranker {
    api_base: String,
    api_key: String,
    model: String,
    client: reqwest::Client,
    fallback: DeterministicReranker,
    resilience: Arc<ResilienceConfig>,
    circuit_breaker: Arc<CircuitBreaker>,
}

impl JinaReranker {
    /// Create a new provider from explicit configuration.
    /// Returns `None` if `api_key` is empty.
    pub fn new(api_key: String, api_base: String, model: String) -> Option<Self> {
        if api_key.is_empty() {
            return None;
        }
        let resilience = Arc::new(ResilienceConfig::from_env());
        let circuit_breaker = Arc::new(CircuitBreaker::new(
            resilience.cb_failure_threshold,
            resilience.cb_reset_timeout,
        ));
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap_or_else(|e| {
                tracing::warn!(
                    "JinaReranker client builder failed: {e}, using default (no timeout)"
                );
                reqwest::Client::new()
            });
        Some(Self {
            api_base,
            api_key,
            model,
            client,
            fallback: DeterministicReranker,
            resilience,
            circuit_breaker,
        })
    }

    /// Create from environment variables.
    pub fn from_env() -> Option<Self> {
        let api_key = env_value(&["MEMFUSE_JINA_API_KEY"]).filter(|k| !k.is_empty())?;
        let api_base = env_value(&["MEMFUSE_JINA_BASE_URL"])
            .unwrap_or_else(|| DEFAULT_JINA_BASE_URL.to_owned());
        let model = env_value(&["MEMFUSE_JINA_RERANK_MODEL"])
            .unwrap_or_else(|| DEFAULT_JINA_RERANK_MODEL.to_owned());
        Self::new(api_key, api_base, model)
    }

    /// Execute a single rerank API call.
    async fn rerank_once(
        &self,
        query: &str,
        documents: &[String],
        top_n: usize,
    ) -> Result<Vec<RerankScore>, RetryableError> {
        let request_body = serde_json::json!({
            "model": self.model,
            "query": query,
            "documents": documents,
            "top_n": top_n,
        });

        let response = self
            .client
            .post(format!("{}/rerank", self.api_base.trim_end_matches('/')))
            .header("Content-Type", "application/json")
            .bearer_auth(&self.api_key)
            .json(&request_body)
            .send()
            .await;

        match response {
            Ok(resp) => {
                let status = resp.status().as_u16();
                if status >= 400 {
                    return Err(match classify_api_error(status) {
                        ApiErrorClass::Permanent => RetryableError::Permanent,
                        ApiErrorClass::Transient => RetryableError::Transient { status },
                    });
                }
                let json: serde_json::Value = resp
                    .json()
                    .await
                    .map_err(|_| RetryableError::Transient { status: 200 })?;

                let results = json.get("results").and_then(|r| r.as_array());
                match results {
                    Some(arr) => {
                        let scores: Vec<RerankScore> = arr
                            .iter()
                            .map(|item: &serde_json::Value| RerankScore {
                                index: item
                                    .get("index")
                                    .and_then(|i: &serde_json::Value| i.as_u64())
                                    .unwrap_or(0) as usize,
                                relevance_score: item
                                    .get("relevance_score")
                                    .and_then(|s: &serde_json::Value| s.as_f64())
                                    .unwrap_or(0.0),
                            })
                            .collect();
                        Ok(scores)
                    }
                    None => Err(RetryableError::Transient { status: 200 }),
                }
            }
            Err(e) => {
                if e.is_connect() || e.is_timeout() || e.is_request() {
                    Err(RetryableError::Network)
                } else {
                    Err(RetryableError::Permanent)
                }
            }
        }
    }
}

#[async_trait]
impl Reranker for JinaReranker {
    async fn rerank_search_hits(&self, query: &str, hits: Vec<SearchHit>) -> Vec<SearchHit> {
        if hits.len() < 2 {
            return hits;
        }

        // Build documents from hit excerpts for the Jina API.
        let documents: Vec<String> = hits
            .iter()
            .map(|h| format!("{} {}", h.uri, h.excerpt))
            .collect();
        let top_n = hits.len().min(10);

        let scores = retry_with_backoff(
            self.resilience.max_attempts,
            self.resilience.base_delay,
            self.resilience.max_delay,
            &self.circuit_breaker,
            || async { self.rerank_once(query, &documents, top_n).await },
        )
        .await;

        match scores {
            Some(rerank_scores) => {
                // Build a map from original document index → relevance_score.
                let mut score_map: std::collections::HashMap<usize, f64> =
                    std::collections::HashMap::new();
                for s in &rerank_scores {
                    score_map.insert(s.index, s.relevance_score);
                }

                // Sort hits by their Jina relevance score.
                let mut indexed_hits: Vec<(usize, SearchHit)> =
                    hits.into_iter().enumerate().collect();
                indexed_hits.sort_by(|(left_idx, _left_hit), (right_idx, _right_hit)| {
                    let left_score = score_map.get(left_idx).copied().unwrap_or(0.0);
                    let right_score = score_map.get(right_idx).copied().unwrap_or(0.0);
                    right_score.total_cmp(&left_score)
                });
                indexed_hits.into_iter().map(|(_, hit)| hit).collect()
            }
            None => self.fallback.rerank_search_hits(query, hits).await,
        }
    }

    async fn rerank_ranked_hits(
        &self,
        query: &str,
        hits: Vec<RankedLayeredHit>,
    ) -> Vec<RankedLayeredHit> {
        if hits.len() < 2 {
            return hits;
        }

        let documents: Vec<String> = hits
            .iter()
            .map(|h| format!("{} {}", h.hit.uri, h.hit.excerpt))
            .collect();
        let top_n = hits.len().min(10);

        let scores = retry_with_backoff(
            self.resilience.max_attempts,
            self.resilience.base_delay,
            self.resilience.max_delay,
            &self.circuit_breaker,
            || async { self.rerank_once(query, &documents, top_n).await },
        )
        .await;

        match scores {
            Some(rerank_scores) => {
                let mut score_map: std::collections::HashMap<usize, f64> =
                    std::collections::HashMap::new();
                for s in &rerank_scores {
                    score_map.insert(s.index, s.relevance_score);
                }

                let mut indexed_hits: Vec<(usize, RankedLayeredHit)> =
                    hits.into_iter().enumerate().collect();
                indexed_hits.sort_by(|(left_idx, _left_hit), (right_idx, _right_hit)| {
                    let left_score = score_map.get(left_idx).copied().unwrap_or(0.0);
                    let right_score = score_map.get(right_idx).copied().unwrap_or(0.0);
                    right_score.total_cmp(&left_score)
                });
                indexed_hits.into_iter().map(|(_, hit)| hit).collect()
            }
            None => self.fallback.rerank_ranked_hits(query, hits).await,
        }
    }
}

#[async_trait]
impl RerankerExt for JinaReranker {
    async fn rerank_with_scores(
        &self,
        query: &str,
        documents: &[String],
        top_n: usize,
    ) -> Vec<RerankScore> {
        if let Some(result) = retry_with_backoff(
            self.resilience.max_attempts,
            self.resilience.base_delay,
            self.resilience.max_delay,
            &self.circuit_breaker,
            || async { self.rerank_once(query, documents, top_n).await },
        )
        .await
        {
            return result;
        }
        // Fallback: use deterministic token overlap scoring.
        let mut scores: Vec<RerankScore> = documents
            .iter()
            .enumerate()
            .map(|(i, doc)| RerankScore {
                index: i,
                relevance_score: crate::rerank::score_text(query, doc, ""),
            })
            .collect();
        scores.sort_by(|a, b| b.relevance_score.total_cmp(&a.relevance_score));
        scores.truncate(top_n);
        scores
    }
}

fn env_value(keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        std::env::var(key)
            .ok()
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty())
    })
}
