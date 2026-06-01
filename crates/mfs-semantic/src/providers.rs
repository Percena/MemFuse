use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use mfs_ast::detect_language;

use serde_json::json;

use crate::config::{
    chat_model_from_env, embedding_model_from_env, openai_api_base_from_env,
    openai_api_key_from_env, summary_model_from_env,
};
use crate::resilience::{
    CircuitBreaker, ResilienceConfig, RetryableError, classify_api_error, retry_with_backoff,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessingMode {
    Full,
    Degraded,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SummaryPair {
    pub abstract_text: String,
    pub overview_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedSummary {
    pub name: String,
    pub abstract_text: String,
    pub overview_text: String,
}

#[async_trait]
pub trait SummaryProvider: Send + Sync {
    fn mode(&self) -> ProcessingMode;
    async fn summarize_file(&self, path: &Path, content: &str) -> SummaryPair;
    async fn summarize_directory(
        &self,
        uri: &str,
        files: &[NamedSummary],
        children: &[NamedSummary],
    ) -> SummaryPair;
}

#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    fn mode(&self) -> ProcessingMode;
    fn dimension(&self) -> usize;
    async fn embed_text(&self, text: &str) -> Vec<f32>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DeterministicSummaryProvider;

impl DeterministicSummaryProvider {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl SummaryProvider for DeterministicSummaryProvider {
    fn mode(&self) -> ProcessingMode {
        ProcessingMode::Degraded
    }

    async fn summarize_file(&self, path: &Path, content: &str) -> SummaryPair {
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("untitled");
        let normalized = normalize_whitespace(content);
        let excerpt = truncate_tokens(&normalized, 32);
        SummaryPair {
            abstract_text: truncate_tokens(
                &format!("Deterministic semantic abstract for {file_name}: {excerpt}"),
                24,
            ),
            overview_text: format!(
                "# Overview\n\nFile: `{file_name}`\n\nSummary: {excerpt}\n\nLength: {} tokens",
                normalized.split_whitespace().count()
            ),
        }
    }

    async fn summarize_directory(
        &self,
        uri: &str,
        files: &[NamedSummary],
        children: &[NamedSummary],
    ) -> SummaryPair {
        let file_names = files
            .iter()
            .map(|item| item.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let child_names = children
            .iter()
            .map(|item| item.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let abstract_text = truncate_tokens(
            &format!(
                "Deterministic semantic abstract for {uri}: files [{file_names}] children [{child_names}]"
            ),
            32,
        );
        let mut overview = vec![
            "# Overview".to_owned(),
            String::new(),
            format!("Directory: `{uri}`"),
            format!("Files: {}", files.len()),
            format!("Subdirectories: {}", children.len()),
            String::new(),
            "## File Summaries".to_owned(),
        ];
        if files.is_empty() {
            overview.push("- None".to_owned());
        } else {
            for item in files {
                overview.push(format!("- `{}`: {}", item.name, item.abstract_text));
            }
        }
        overview.push(String::new());
        overview.push("## Child Directories".to_owned());
        if children.is_empty() {
            overview.push("- None".to_owned());
        } else {
            for item in children {
                overview.push(format!("- `{}`: {}", item.name, item.abstract_text));
            }
        }
        SummaryPair {
            abstract_text,
            overview_text: overview.join("\n"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct DeterministicEmbeddingProvider {
    dimension: usize,
}

impl DeterministicEmbeddingProvider {
    pub fn new(dimension: usize) -> Self {
        Self { dimension }
    }
}

#[async_trait]
impl EmbeddingProvider for DeterministicEmbeddingProvider {
    fn mode(&self) -> ProcessingMode {
        ProcessingMode::Degraded
    }

    fn dimension(&self) -> usize {
        self.dimension
    }

    async fn embed_text(&self, text: &str) -> Vec<f32> {
        let mut vector = vec![0.0_f32; self.dimension];
        if self.dimension == 0 {
            return vector;
        }
        for token in text.split_whitespace() {
            let mut hasher = DefaultHasher::new();
            token.to_ascii_lowercase().hash(&mut hasher);
            let bucket = (hasher.finish() as usize) % self.dimension;
            vector[bucket] += 1.0;
        }
        if vector.iter().all(|value| *value == 0.0) {
            vector[0] = 1.0;
        }
        vector
    }
}

#[derive(Debug)]
pub struct OpenAiSummaryProvider {
    api_base: String,
    api_key: Option<String>,
    model: String,
    client: reqwest::Client,
    fallback: DeterministicSummaryProvider,
    resilience: Arc<ResilienceConfig>,
    circuit_breaker: Arc<CircuitBreaker>,
}

impl OpenAiSummaryProvider {
    pub fn from_env() -> Self {
        let resilience = Arc::new(ResilienceConfig::from_env());
        let circuit_breaker = Arc::new(CircuitBreaker::new(
            resilience.cb_failure_threshold,
            resilience.cb_reset_timeout,
        ));
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap_or_else(|e| {
                tracing::warn!(
                    "OpenAiSummaryProvider client builder failed: {e}, using default (no timeout)"
                );
                reqwest::Client::new()
            });
        Self {
            api_base: openai_api_base_from_env(),
            api_key: openai_api_key_from_env(),
            model: summary_model_from_env(),
            client,
            fallback: DeterministicSummaryProvider::new(),
            resilience,
            circuit_breaker,
        }
    }

    async fn complete(&self, prompt: &str) -> Option<String> {
        retry_with_backoff(
            self.resilience.max_attempts,
            self.resilience.base_delay,
            self.resilience.max_delay,
            &self.circuit_breaker,
            || async { self.complete_once(prompt).await },
        )
        .await
    }

    async fn complete_once(&self, prompt: &str) -> Result<String, RetryableError> {
        let mut request = self
            .client
            .post(format!(
                "{}/chat/completions",
                self.api_base.trim_end_matches('/')
            ))
            .json(&json!({
                "model": self.model,
                "temperature": 0.0,
                "messages": [{"role": "user", "content": prompt}],
            }));
        if let Some(api_key) = &self.api_key {
            request = request.bearer_auth(api_key);
        }

        let response = match request.send().await {
            Ok(response) => response,
            Err(e) => {
                if e.is_connect() || e.is_timeout() || e.is_request() {
                    return Err(RetryableError::Network);
                }
                return Err(RetryableError::Permanent);
            }
        };

        let status = response.status().as_u16();
        if status >= 400 {
            return match classify_api_error(status) {
                crate::resilience::ApiErrorClass::Permanent => Err(RetryableError::Permanent),
                crate::resilience::ApiErrorClass::Transient => {
                    Err(RetryableError::Transient { status })
                }
            };
        }

        let json = match response.json::<serde_json::Value>().await {
            Ok(json) => json,
            Err(_) => {
                return Err(RetryableError::Transient { status: 200 });
            }
        };

        match json["choices"][0]["message"]["content"]
            .as_str()
            .map(str::trim)
            .map(str::to_owned)
        {
            Some(content) => Ok(content),
            None => Err(RetryableError::Transient { status: 200 }),
        }
    }
}

#[async_trait]
impl SummaryProvider for OpenAiSummaryProvider {
    fn mode(&self) -> ProcessingMode {
        ProcessingMode::Full
    }

    async fn summarize_file(&self, path: &Path, content: &str) -> SummaryPair {
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("untitled");
        let lang = detect_language(file_name);
        let prompt = if lang.is_some() {
            // Code-aware prompt
            let lang_name = lang.map(|l| l.name()).unwrap_or("unknown");
            format!(
                "Analyze this {lang_name} code file `{file_name}`.\nProvide:\n1. A one-line abstract describing what this module does\n2. A short markdown overview listing key classes/functions and their purposes, dependencies, and design patterns\nReturn plain markdown only.\n\nContent:\n{content}"
            )
        } else {
            // Generic prompt
            format!(
                "Summarize the file `{file_name}` into a one-line abstract and a short markdown overview.\nReturn plain markdown only.\n\nContent:\n{content}"
            )
        };
        if let Some(overview_text) = self.complete(&prompt).await {
            return SummaryPair {
                abstract_text: overview_text.lines().next().unwrap_or_default().to_owned(),
                overview_text,
            };
        }
        self.fallback.summarize_file(path, content).await
    }

    async fn summarize_directory(
        &self,
        uri: &str,
        files: &[NamedSummary],
        children: &[NamedSummary],
    ) -> SummaryPair {
        let file_summaries = files
            .iter()
            .map(|item| format!("- file `{}`: {}", item.name, item.abstract_text))
            .collect::<Vec<_>>()
            .join("\n");
        let child_summaries = children
            .iter()
            .map(|item| format!("- dir `{}`: {}", item.name, item.abstract_text))
            .collect::<Vec<_>>()
            .join("\n");
        let prompt = format!(
            "Summarize the directory `{uri}` into a one-line abstract and a short markdown overview.\nReturn plain markdown only.\n\nFiles:\n{file_summaries}\n\nChildren:\n{child_summaries}"
        );
        if let Some(overview_text) = self.complete(&prompt).await {
            return SummaryPair {
                abstract_text: overview_text.lines().next().unwrap_or_default().to_owned(),
                overview_text,
            };
        }
        self.fallback
            .summarize_directory(uri, files, children)
            .await
    }
}

#[derive(Debug)]
pub struct OpenAiEmbeddingProvider {
    api_base: String,
    api_key: Option<String>,
    model: String,
    dimension: usize,
    client: reqwest::Client,
    fallback: DeterministicEmbeddingProvider,
    resilience: Arc<ResilienceConfig>,
    circuit_breaker: Arc<CircuitBreaker>,
}

impl OpenAiEmbeddingProvider {
    pub fn from_env(default_dimension: usize) -> Self {
        let resilience = Arc::new(ResilienceConfig::from_env());
        let circuit_breaker = Arc::new(CircuitBreaker::new(
            resilience.cb_failure_threshold,
            resilience.cb_reset_timeout,
        ));
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap_or_else(|e| {
                tracing::warn!(
                    "OpenAiEmbeddingProvider client builder failed: {e}, using default (no timeout)"
                );
                reqwest::Client::new()
            });
        Self {
            api_base: openai_api_base_from_env(),
            api_key: openai_api_key_from_env(),
            model: embedding_model_from_env(),
            dimension: default_dimension,
            client,
            fallback: DeterministicEmbeddingProvider::new(default_dimension),
            resilience,
            circuit_breaker,
        }
    }

    async fn embed_once(&self, text: &str) -> Result<Vec<f32>, RetryableError> {
        let mut request = self
            .client
            .post(format!(
                "{}/embeddings",
                self.api_base.trim_end_matches('/')
            ))
            .json(&json!({
                "model": self.model,
                "input": text,
            }));
        if let Some(api_key) = &self.api_key {
            request = request.bearer_auth(api_key);
        }

        let response = match request.send().await {
            Ok(response) => response,
            Err(e) => {
                if e.is_connect() || e.is_timeout() || e.is_request() {
                    return Err(RetryableError::Network);
                }
                return Err(RetryableError::Permanent);
            }
        };

        let status = response.status().as_u16();
        if status >= 400 {
            return match classify_api_error(status) {
                crate::resilience::ApiErrorClass::Permanent => Err(RetryableError::Permanent),
                crate::resilience::ApiErrorClass::Transient => {
                    Err(RetryableError::Transient { status })
                }
            };
        }

        let json = match response.json::<serde_json::Value>().await {
            Ok(json) => json,
            Err(_) => {
                return Err(RetryableError::Transient { status: 200 });
            }
        };

        let Some(values) = json["data"][0]["embedding"].as_array() else {
            return Err(RetryableError::Transient { status: 200 });
        };

        let mut vector = values
            .iter()
            .filter_map(|value| value.as_f64().map(|value| value as f32))
            .collect::<Vec<_>>();
        if vector.is_empty() {
            return Err(RetryableError::Transient { status: 200 });
        }
        if self.dimension > 0 && vector.len() != self.dimension {
            vector.truncate(self.dimension.min(vector.len()));
            while vector.len() < self.dimension {
                vector.push(0.0);
            }
        }
        Ok(vector)
    }
}

#[async_trait]
impl EmbeddingProvider for OpenAiEmbeddingProvider {
    fn mode(&self) -> ProcessingMode {
        ProcessingMode::Full
    }

    fn dimension(&self) -> usize {
        self.dimension
    }

    async fn embed_text(&self, text: &str) -> Vec<f32> {
        if let Some(result) = retry_with_backoff(
            self.resilience.max_attempts,
            self.resilience.base_delay,
            self.resilience.max_delay,
            &self.circuit_breaker,
            || async { self.embed_once(text).await },
        )
        .await
        {
            return result;
        }
        self.fallback.embed_text(text).await
    }
}

fn normalize_whitespace(content: &str) -> String {
    content.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate_tokens(content: &str, limit: usize) -> String {
    let tokens = content.split_whitespace().take(limit).collect::<Vec<_>>();
    tokens.join(" ")
}

// ─── Chat Provider ────────────────────────────────────────────────────────────

/// A minimal async chat completion provider used by the session memory
/// pipeline.  The interface is intentionally narrow: callers pass a single
/// user-turn prompt and receive the assistant reply as a plain `String`.
///
/// The trait is object-safe (via `#[async_trait]`) so it can be stored as
/// `Box<dyn ChatProvider>`.
#[async_trait]
pub trait ChatProvider: Send + Sync {
    fn mode(&self) -> ProcessingMode;
    /// Send `prompt` as a single user message and return the assistant reply.
    /// Returns `None` when the provider is unavailable or the call fails after
    /// all retries.
    async fn complete(&self, prompt: &str) -> Option<String>;
}

/// Deterministic (no-op) chat provider used when no LLM is configured.
/// Always returns `None` so callers fall back to rule-based extraction.
#[derive(Debug, Default, Clone, Copy)]
pub struct DeterministicChatProvider;

#[async_trait]
impl ChatProvider for DeterministicChatProvider {
    fn mode(&self) -> ProcessingMode {
        ProcessingMode::Degraded
    }

    async fn complete(&self, _prompt: &str) -> Option<String> {
        None
    }
}

/// OpenAI-compatible chat provider backed by the same resilience stack used
/// by `OpenAiSummaryProvider`.
#[derive(Debug)]
pub struct OpenAiChatProvider {
    api_base: String,
    api_key: Option<String>,
    model: String,
    client: reqwest::Client,
    resilience: Arc<ResilienceConfig>,
    circuit_breaker: Arc<CircuitBreaker>,
}

impl OpenAiChatProvider {
    pub fn from_env() -> Self {
        Self::from_env_with(
            Arc::new(ResilienceConfig::from_env()),
            std::time::Duration::from_secs(120),
            std::time::Duration::from_secs(10),
        )
    }

    pub fn from_env_for_read() -> Self {
        Self::from_env_with(
            Arc::new(ResilienceConfig::from_env_for_read()),
            std::time::Duration::from_millis(crate::resilience::env_parse(
                "MEMFUSE_READ_TIMEOUT_MS",
                1500,
            )),
            std::time::Duration::from_millis(crate::resilience::env_parse(
                "MEMFUSE_READ_CONNECT_TIMEOUT_MS",
                500,
            )),
        )
    }

    fn from_env_with(
        resilience: Arc<ResilienceConfig>,
        timeout: std::time::Duration,
        connect_timeout: std::time::Duration,
    ) -> Self {
        let circuit_breaker = Arc::new(CircuitBreaker::new(
            resilience.cb_failure_threshold,
            resilience.cb_reset_timeout,
        ));
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .connect_timeout(connect_timeout)
            .build()
            .unwrap_or_else(|e| {
                tracing::warn!(
                    "OpenAiChatProvider client builder failed: {e}, using default (no timeout)"
                );
                reqwest::Client::new()
            });
        Self {
            api_base: openai_api_base_from_env(),
            api_key: openai_api_key_from_env(),
            model: chat_model_from_env(),
            client,
            resilience,
            circuit_breaker,
        }
    }

    async fn complete_once(&self, prompt: &str) -> Result<String, RetryableError> {
        let mut request = self
            .client
            .post(format!(
                "{}/chat/completions",
                self.api_base.trim_end_matches('/')
            ))
            .json(&json!({
                "model": self.model,
                "temperature": 0.0,
                "messages": [{"role": "user", "content": prompt}],
            }));
        if let Some(api_key) = &self.api_key {
            request = request.bearer_auth(api_key);
        }

        let response = match request.send().await {
            Ok(r) => r,
            Err(e) => {
                if e.is_connect() || e.is_timeout() || e.is_request() {
                    return Err(RetryableError::Network);
                }
                return Err(RetryableError::Permanent);
            }
        };

        let status = response.status().as_u16();
        if status >= 400 {
            return match classify_api_error(status) {
                crate::resilience::ApiErrorClass::Permanent => Err(RetryableError::Permanent),
                crate::resilience::ApiErrorClass::Transient => {
                    Err(RetryableError::Transient { status })
                }
            };
        }

        let json = match response.json::<serde_json::Value>().await {
            Ok(j) => j,
            Err(_) => return Err(RetryableError::Transient { status: 200 }),
        };

        match json["choices"][0]["message"]["content"]
            .as_str()
            .map(str::trim)
            .map(str::to_owned)
        {
            Some(content) => Ok(content),
            None => Err(RetryableError::Transient { status: 200 }),
        }
    }
}

#[async_trait]
impl ChatProvider for OpenAiChatProvider {
    fn mode(&self) -> ProcessingMode {
        ProcessingMode::Full
    }

    async fn complete(&self, prompt: &str) -> Option<String> {
        retry_with_backoff(
            self.resilience.max_attempts,
            self.resilience.base_delay,
            self.resilience.max_delay,
            &self.circuit_breaker,
            || async { self.complete_once(prompt).await },
        )
        .await
    }
}
