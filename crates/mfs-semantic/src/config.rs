use std::sync::Arc;

use crate::jina_embedding::JinaEmbeddingProvider;
use crate::providers::{
    ChatProvider, DeterministicChatProvider, DeterministicEmbeddingProvider,
    DeterministicSummaryProvider, EmbeddingProvider, OpenAiChatProvider, OpenAiEmbeddingProvider,
    OpenAiSummaryProvider, SummaryProvider,
};
use serde::Serialize;

pub struct SemanticPipelineConfig {
    pub summary_provider: Arc<dyn SummaryProvider>,
    pub embedding_provider: Box<dyn EmbeddingProvider>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SemanticRuntimeConfig {
    pub summary_provider: String,
    pub embedding_provider: String,
    pub summary_model: String,
    pub chat_model: String,
    pub embedding_model: String,
    pub summary_concurrency: usize,
    pub openai_compatible_env: bool,
    pub jina_enabled: bool,
    pub jina_rerank_enabled: bool,
}

impl SemanticPipelineConfig {
    pub fn from_env(default_dimension: usize) -> Self {
        Self {
            summary_provider: summary_provider_from_env(),
            embedding_provider: embedding_provider_from_env(default_dimension),
        }
    }
}

pub fn current_runtime_config() -> SemanticRuntimeConfig {
    let summary_provider = summary_provider_name_from_env();
    let embedding_provider = embedding_provider_name_from_env();
    let chat_provider = chat_provider_name_from_env();
    SemanticRuntimeConfig {
        summary_provider: summary_provider.clone(),
        embedding_provider: embedding_provider.clone(),
        summary_model: if summary_provider == "openai" {
            summary_model_from_env()
        } else {
            "n/a".to_owned()
        },
        chat_model: if chat_provider == "openai" {
            chat_model_from_env()
        } else {
            "n/a".to_owned()
        },
        embedding_model: if embedding_provider == "openai" {
            embedding_model_from_env()
        } else if embedding_provider == "jina" {
            env_value(&["MEMFUSE_JINA_EMBEDDING_MODEL"])
                .unwrap_or_else(|| "jina-embeddings-v4".to_owned())
        } else {
            "n/a".to_owned()
        },
        summary_concurrency: summary_concurrency_from_env(),
        openai_compatible_env: summary_provider == "openai"
            || embedding_provider == "openai"
            || has_openai_chat_env(),
        jina_enabled: embedding_provider == "jina",
        jina_rerank_enabled: has_jina_rerank_env(),
    }
}

pub fn summary_provider_from_env() -> Arc<dyn SummaryProvider> {
    match summary_provider_name_from_env().as_str() {
        "openai" => Arc::new(OpenAiSummaryProvider::from_env()),
        _ => Arc::new(DeterministicSummaryProvider::new()),
    }
}

pub fn embedding_provider_from_env(default_dimension: usize) -> Box<dyn EmbeddingProvider> {
    match embedding_provider_name_from_env().as_str() {
        "openai" => Box::new(OpenAiEmbeddingProvider::from_env(default_dimension)),
        "jina" => jina_embedding_provider_from_env(default_dimension)
            .map(|p| Box::new(p) as Box<dyn EmbeddingProvider>)
            .unwrap_or_else(|| Box::new(DeterministicEmbeddingProvider::new(default_dimension))),
        _ => Box::new(DeterministicEmbeddingProvider::new(default_dimension)),
    }
}

/// Create a `JinaEmbeddingProvider` from environment variables.
/// Returns `None` (and falls back to Deterministic) if `MEMFUSE_JINA_API_KEY`
/// is missing or dimensions are invalid.
pub fn jina_embedding_provider_from_env(default_dimension: usize) -> Option<JinaEmbeddingProvider> {
    JinaEmbeddingProvider::from_env(default_dimension)
}

pub fn chat_provider_from_env() -> Box<dyn ChatProvider> {
    match chat_provider_name_from_env().as_str() {
        "openai" => Box::new(OpenAiChatProvider::from_env()),
        _ => Box::new(DeterministicChatProvider),
    }
}

pub fn chat_provider_from_env_for_read() -> Box<dyn ChatProvider> {
    match chat_provider_name_from_env().as_str() {
        "openai" => Box::new(OpenAiChatProvider::from_env_for_read()),
        other => {
            tracing::info!(
                "read-path chat provider '{other}' has no latency-bounded variant, falling back to DeterministicChatProvider"
            );
            Box::new(DeterministicChatProvider)
        }
    }
}

pub fn chat_provider_name_from_env() -> String {
    env_value(&["MEMFUSE_CHAT_PROVIDER"])
        .unwrap_or_else(|| {
            if has_openai_chat_env() {
                "openai".to_owned()
            } else {
                "deterministic".to_owned()
            }
        })
        .to_ascii_lowercase()
}

pub fn summary_provider_name_from_env() -> String {
    env_value(&["MEMFUSE_SUMMARY_PROVIDER"])
        .unwrap_or_else(|| {
            if has_openai_summary_env() {
                "openai".to_owned()
            } else {
                "deterministic".to_owned()
            }
        })
        .to_ascii_lowercase()
}

pub fn embedding_provider_name_from_env() -> String {
    env_value(&["MEMFUSE_EMBEDDING_PROVIDER"])
        .unwrap_or_else(|| {
            if has_jina_embedding_env() {
                "jina".to_owned()
            } else if has_openai_embedding_env() {
                "openai".to_owned()
            } else {
                "deterministic".to_owned()
            }
        })
        .to_ascii_lowercase()
}

pub fn openai_api_base_from_env() -> String {
    env_value(&["MEMFUSE_OPENAI_API_BASE", "OPENAI_BASE_URL"])
        .unwrap_or_else(|| "https://api.openai.com/v1".to_owned())
}

pub fn openai_api_key_from_env() -> Option<String> {
    env_value(&["MEMFUSE_OPENAI_API_KEY", "OPENAI_API_KEY"])
}

pub fn summary_model_from_env() -> String {
    env_value(&["MEMFUSE_SUMMARY_MODEL", "OPENAI_COMPATIBLE_MODEL"])
        .unwrap_or_else(|| "gpt-4.1-mini".to_owned())
}

pub fn chat_model_from_env() -> String {
    env_value(&["MEMFUSE_CHAT_MODEL"]).unwrap_or_else(summary_model_from_env)
}

pub fn embedding_model_from_env() -> String {
    env_value(&["MEMFUSE_EMBEDDING_MODEL", "OPENAI_EMBEDDING_MODEL"])
        .unwrap_or_else(|| "text-embedding-3-small".to_owned())
}

pub fn summary_concurrency_from_env() -> usize {
    env_value(&["MEMFUSE_SUMMARY_CONCURRENCY"])
        .and_then(|value| value.parse::<usize>().ok())
        .map(|value| value.clamp(1, 8))
        .unwrap_or(4)
}

pub fn has_openai_summary_env() -> bool {
    openai_api_key_from_env().is_some()
        || env_value(&["MEMFUSE_OPENAI_API_BASE", "OPENAI_BASE_URL"]).is_some()
        || env_value(&["MEMFUSE_SUMMARY_MODEL", "OPENAI_COMPATIBLE_MODEL"]).is_some()
}

pub fn has_openai_embedding_env() -> bool {
    env_value(&["MEMFUSE_EMBEDDING_MODEL", "OPENAI_EMBEDDING_MODEL"]).is_some()
        && (openai_api_key_from_env().is_some()
            || env_value(&["MEMFUSE_OPENAI_API_BASE", "OPENAI_BASE_URL"]).is_some())
}

/// Check if Jina embedding environment is configured.
/// Returns `true` when `MEMFUSE_JINA_API_KEY` is set and non-empty.
pub fn has_jina_embedding_env() -> bool {
    env_value(&["MEMFUSE_JINA_API_KEY"]).is_some()
}

/// Check if Jina reranker environment is configured.
/// Returns `true` when `MEMFUSE_RERANK_PROVIDER=jina` is explicitly set,
/// or when `MEMFUSE_JINA_API_KEY` is available (auto-detect).
pub fn has_jina_rerank_env() -> bool {
    let explicit = env_value(&["MEMFUSE_RERANK_PROVIDER"]);
    match explicit.as_deref() {
        Some("jina") => true,
        Some("deterministic") | Some(_) => false,
        None => has_jina_embedding_env(), // auto-detect
    }
}

pub fn has_openai_chat_env() -> bool {
    openai_api_key_from_env().is_some()
        || env_value(&["MEMFUSE_OPENAI_API_BASE", "OPENAI_BASE_URL"]).is_some()
        || env_value(&["MEMFUSE_CHAT_MODEL"]).is_some()
}

fn env_value(keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        std::env::var(key)
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::ProcessingMode;

    struct EnvGuard {
        saved: Vec<(&'static str, Option<String>)>,
    }

    impl EnvGuard {
        fn new(keys: &[&'static str]) -> Self {
            let saved = keys
                .iter()
                .map(|key| (*key, std::env::var(key).ok()))
                .collect::<Vec<_>>();
            for key in keys {
                unsafe {
                    std::env::remove_var(key);
                }
            }
            Self { saved }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (key, value) in &self.saved {
                if let Some(value) = value {
                    unsafe {
                        std::env::set_var(key, value);
                    }
                } else {
                    unsafe {
                        std::env::remove_var(key);
                    }
                }
            }
        }
    }

    #[test]
    fn chat_provider_can_be_forced_to_deterministic() {
        let _guard = EnvGuard::new(&[
            "MEMFUSE_CHAT_PROVIDER",
            "MEMFUSE_OPENAI_API_KEY",
            "OPENAI_API_KEY",
        ]);
        unsafe {
            std::env::set_var("MEMFUSE_OPENAI_API_KEY", "test-key");
            std::env::set_var("MEMFUSE_CHAT_PROVIDER", "deterministic");
        }

        let provider = chat_provider_from_env();

        assert_eq!(provider.mode(), ProcessingMode::Degraded);
    }
}
