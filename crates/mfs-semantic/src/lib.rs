mod config;
mod embedding_util;
mod jina_embedding;
mod pipeline;
mod providers;
mod providers_ext;
mod resilience;

pub use config::{
    SemanticPipelineConfig, SemanticRuntimeConfig, chat_model_from_env, chat_provider_from_env,
    chat_provider_from_env_for_read, current_runtime_config, embedding_model_from_env,
    embedding_provider_from_env, embedding_provider_name_from_env, has_jina_embedding_env,
    has_jina_rerank_env, has_openai_chat_env, has_openai_embedding_env, has_openai_summary_env,
    jina_embedding_provider_from_env, openai_api_base_from_env, openai_api_key_from_env,
    summary_concurrency_from_env, summary_model_from_env, summary_provider_from_env,
    summary_provider_name_from_env,
};
pub use embedding_util::{cosine_similarity, parse_embedding_json, try_embed_query};
pub use jina_embedding::JinaEmbeddingProvider;
pub use pipeline::{SemanticError, SemanticPipeline, SemanticProcessingReport};
pub use providers::{
    ChatProvider, DeterministicChatProvider, DeterministicEmbeddingProvider,
    DeterministicSummaryProvider, EmbeddingProvider, NamedSummary, OpenAiChatProvider,
    OpenAiEmbeddingProvider, OpenAiSummaryProvider, ProcessingMode, SummaryPair, SummaryProvider,
};
pub use providers_ext::EmbeddingProviderExt;
pub use resilience::{
    ApiErrorClass, CircuitBreaker, ResilienceConfig, RetryableError, classify_api_error, env_parse,
    retry_with_backoff,
};
