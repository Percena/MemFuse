//! Jina v4 embedding provider — task-aware, batch-capable embedding via
//! the Jina Cloud API (`https://api.jina.ai/v1/embeddings`).
//!
//! This provider implements both `EmbeddingProvider` and
//! `EmbeddingProviderExt`, leveraging Jina v4's `task` parameter for
//! higher-quality retrieval embeddings and `input: [texts]` for batch
//! efficiency.
//!
//! **Default disabled** — activated only when `MEMFUSE_EMBEDDING_PROVIDER=jina`
//! and `MEMFUSE_JINA_API_KEY` is set.  Falls back to `DeterministicEmbeddingProvider`
//! on any failure.

use std::sync::Arc;

use async_trait::async_trait;

use crate::providers::{DeterministicEmbeddingProvider, EmbeddingProvider, ProcessingMode};
use crate::providers_ext::EmbeddingProviderExt;
use crate::resilience::{
    CircuitBreaker, ResilienceConfig, RetryableError, classify_api_error, retry_with_backoff,
};

/// Supported Jina v4 embedding dimensions.
const SUPPORTED_V4_DIMENSIONS: [usize; 5] = [128, 256, 512, 1024, 2048];

/// Default configuration values.
const DEFAULT_JINA_BASE_URL: &str = "https://api.jina.ai/v1";
const DEFAULT_JINA_EMBEDDING_MODEL: &str = "jina-embeddings-v4";

/// Jina v4 embedding provider backed by the Jina Cloud API.
///
/// Uses `reqwest::Client` (async) for HTTP calls and the shared
/// `CircuitBreaker` + `retry_with_backoff` resilience stack.
#[derive(Debug)]
pub struct JinaEmbeddingProvider {
    api_base: String,
    api_key: String,
    model: String,
    dimensions: usize,
    client: reqwest::Client,
    fallback: DeterministicEmbeddingProvider,
    resilience: Arc<ResilienceConfig>,
    circuit_breaker: Arc<CircuitBreaker>,
}

impl JinaEmbeddingProvider {
    /// Create a new provider from explicit configuration.
    ///
    /// Returns `None` if `api_key` is empty or `dimensions` is not
    /// a supported Jina v4 value (128, 256, 512, 1024, 2048).
    pub fn new(
        api_key: String,
        api_base: String,
        model: String,
        dimensions: usize,
    ) -> Option<Self> {
        if api_key.is_empty() {
            return None;
        }
        if !SUPPORTED_V4_DIMENSIONS.contains(&dimensions) {
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
                    "JinaEmbeddingProvider client builder failed: {e}, using default (no timeout)"
                );
                reqwest::Client::new()
            });
        Some(Self {
            api_base,
            api_key,
            model,
            dimensions,
            client,
            fallback: DeterministicEmbeddingProvider::new(dimensions),
            resilience,
            circuit_breaker,
        })
    }

    /// Create from environment variables.
    ///
    /// Reads `MEMFUSE_JINA_API_KEY`, `MEMFUSE_JINA_BASE_URL`,
    /// `MEMFUSE_JINA_EMBEDDING_MODEL`, `MEMFUSE_JINA_EMBEDDING_DIMENSIONS`.
    /// Returns `None` if API key is missing or dimensions invalid.
    pub fn from_env(default_dimension: usize) -> Option<Self> {
        let api_key = env_value(&["MEMFUSE_JINA_API_KEY"]).filter(|k| !k.is_empty())?;
        let api_base = env_value(&["MEMFUSE_JINA_BASE_URL"])
            .unwrap_or_else(|| DEFAULT_JINA_BASE_URL.to_owned());
        let model = env_value(&["MEMFUSE_JINA_EMBEDDING_MODEL"])
            .unwrap_or_else(|| DEFAULT_JINA_EMBEDDING_MODEL.to_owned());
        // When no explicit dimension is configured, derive a sensible default
        // from the model name rather than relying on the caller's generic
        // default (which is 8 — not a valid Jina v4 dimension).
        // jina-embeddings-v4 natively supports 2048 dimensions.
        let model_default_dimension = if model == DEFAULT_JINA_EMBEDDING_MODEL {
            2048
        } else {
            default_dimension
        };
        let dimensions = env_value(&["MEMFUSE_JINA_EMBEDDING_DIMENSIONS"])
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(model_default_dimension);
        Self::new(api_key, api_base, model, dimensions)
    }

    /// Embed a single text with a task hint via Jina API.
    async fn embed_once_with_task(
        &self,
        text: &str,
        task: &str,
    ) -> Result<Vec<f32>, RetryableError> {
        let request_body = serde_json::json!({
            "model": self.model,
            "task": task,
            "dimensions": self.dimensions,
            "normalized": true,
            "input": [text],
        });

        let response = self
            .client
            .post(format!(
                "{}/embeddings",
                self.api_base.trim_end_matches('/')
            ))
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
                        crate::resilience::ApiErrorClass::Permanent => RetryableError::Permanent,
                        crate::resilience::ApiErrorClass::Transient => {
                            RetryableError::Transient { status }
                        }
                    });
                }
                let json: serde_json::Value = resp
                    .json()
                    .await
                    .map_err(|_| RetryableError::Transient { status: 200 })?;

                let data = json.get("data").and_then(|d| d.as_array());
                match data {
                    Some(arr) if !arr.is_empty() => {
                        let embedding_values = arr[0].get("embedding").and_then(|e| e.as_array());
                        match embedding_values {
                            Some(values) => {
                                let vector: Vec<f32> = values
                                    .iter()
                                    .filter_map(|v| v.as_f64().map(|f| f as f32))
                                    .collect();
                                if vector.is_empty() {
                                    return Err(RetryableError::Transient { status: 200 });
                                }
                                Ok(vector)
                            }
                            None => Err(RetryableError::Transient { status: 200 }),
                        }
                    }
                    _ => Err(RetryableError::Transient { status: 200 }),
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

    /// Embed multiple texts in a single Jina API call (batch).
    async fn embed_batch_once(
        &self,
        texts: &[&str],
        task: &str,
    ) -> Result<Vec<Vec<f32>>, RetryableError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        let input: Vec<&str> = texts.to_vec();
        let request_body = serde_json::json!({
            "model": self.model,
            "task": task,
            "dimensions": self.dimensions,
            "normalized": true,
            "input": input,
        });

        let response = self
            .client
            .post(format!(
                "{}/embeddings",
                self.api_base.trim_end_matches('/')
            ))
            .timeout(std::time::Duration::from_secs(60))
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
                        crate::resilience::ApiErrorClass::Permanent => RetryableError::Permanent,
                        crate::resilience::ApiErrorClass::Transient => {
                            RetryableError::Transient { status }
                        }
                    });
                }
                let json: serde_json::Value = resp
                    .json()
                    .await
                    .map_err(|_| RetryableError::Transient { status: 200 })?;

                let data = json.get("data").and_then(|d| d.as_array());
                match data {
                    Some(arr) => {
                        let mut results: Vec<(usize, Vec<f32>)> = Vec::with_capacity(arr.len());
                        for item in arr {
                            let index =
                                item.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
                            let embedding_values = item.get("embedding").and_then(|e| e.as_array());
                            match embedding_values {
                                Some(values) => {
                                    let vector: Vec<f32> = values
                                        .iter()
                                        .filter_map(|v| v.as_f64().map(|f| f as f32))
                                        .collect();
                                    if vector.is_empty() {
                                        return Err(RetryableError::Transient { status: 200 });
                                    }
                                    results.push((index, vector));
                                }
                                None => return Err(RetryableError::Transient { status: 200 }),
                            }
                        }
                        // Sort by index and validate completeness.
                        results.sort_by_key(|(i, _)| *i);
                        if results.len() != texts.len() {
                            return Err(RetryableError::Transient { status: 200 });
                        }
                        Ok(results.into_iter().map(|(_, v)| v).collect())
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
impl EmbeddingProvider for JinaEmbeddingProvider {
    fn mode(&self) -> ProcessingMode {
        ProcessingMode::Full
    }

    fn dimension(&self) -> usize {
        self.dimensions
    }

    async fn embed_text(&self, text: &str) -> Vec<f32> {
        // Use "retrieval.passage" as the default task for storing passages.
        if let Some(result) = retry_with_backoff(
            self.resilience.max_attempts,
            self.resilience.base_delay,
            self.resilience.max_delay,
            &self.circuit_breaker,
            || async { self.embed_once_with_task(text, "retrieval.passage").await },
        )
        .await
        {
            return result;
        }
        self.fallback.embed_text(text).await
    }
}

#[async_trait]
impl EmbeddingProviderExt for JinaEmbeddingProvider {
    async fn embed_text_with_task(&self, text: &str, task: &str) -> Vec<f32> {
        if let Some(result) = retry_with_backoff(
            self.resilience.max_attempts,
            self.resilience.base_delay,
            self.resilience.max_delay,
            &self.circuit_breaker,
            || async { self.embed_once_with_task(text, task).await },
        )
        .await
        {
            return result;
        }
        self.fallback.embed_text(text).await
    }

    async fn embed_batch(&self, texts: &[&str]) -> Vec<Vec<f32>> {
        if let Some(result) = retry_with_backoff(
            self.resilience.max_attempts,
            self.resilience.base_delay,
            self.resilience.max_delay,
            &self.circuit_breaker,
            || async { self.embed_batch_once(texts, "retrieval.passage").await },
        )
        .await
        {
            return result;
        }
        // Fallback: sequential deterministic embedding
        let mut results = Vec::with_capacity(texts.len());
        for t in texts {
            results.push(self.fallback.embed_text(t).await);
        }
        results
    }

    async fn embed_batch_with_task(&self, texts: &[&str], task: &str) -> Vec<Vec<f32>> {
        if let Some(result) = retry_with_backoff(
            self.resilience.max_attempts,
            self.resilience.base_delay,
            self.resilience.max_delay,
            &self.circuit_breaker,
            || async { self.embed_batch_once(texts, task).await },
        )
        .await
        {
            return result;
        }
        // Fallback: sequential deterministic embedding, preserving the task hint
        // so that callers relying on task-aware embeddings get consistent behaviour
        // if the fallback provider is ever upgraded to support tasks.
        let mut results = Vec::with_capacity(texts.len());
        for t in texts {
            results.push(self.fallback.embed_text_with_task(t, task).await);
        }
        results
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
