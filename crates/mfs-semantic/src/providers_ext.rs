//! Extended trait interfaces for Jina v4 task-aware embedding.
//!
//! These traits extend the base `EmbeddingProvider` trait with optional
//! methods whose default implementations delegate to the base trait,
//! so existing providers (Deterministic, OpenAI) require no changes
//! beyond a trivial `impl EmbeddingProviderExt for X {}`.

use async_trait::async_trait;

use crate::providers::{
    DeterministicEmbeddingProvider, EmbeddingProvider, OpenAiEmbeddingProvider,
};

// ─── Embedding Provider Extension ─────────────────────────────────────────

/// Extended embedding provider for Jina v4 task-aware and batch embedding.
///
/// The default implementations delegate to the base `EmbeddingProvider`
/// methods.  Existing providers satisfy this trait with an empty impl
/// block that inherits all defaults.
#[async_trait]
pub trait EmbeddingProviderExt: EmbeddingProvider {
    /// Embed a single text with a task hint (e.g. `"retrieval.query"`,
    /// `"retrieval.passage"`).
    ///
    /// Providers that don't support task hints should ignore the
    /// `task` parameter and call `embed_text` directly.
    async fn embed_text_with_task(&self, text: &str, task: &str) -> Vec<f32> {
        let _ = task;
        self.embed_text(text).await
    }

    /// Embed multiple texts in a single API call (batch embedding).
    ///
    /// Providers that don't support batching should call `embed_text`
    /// for each item sequentially.
    async fn embed_batch(&self, texts: &[&str]) -> Vec<Vec<f32>> {
        if texts.is_empty() {
            return Vec::new();
        }
        let mut results = Vec::with_capacity(texts.len());
        for t in texts {
            results.push(self.embed_text(t).await);
        }
        results
    }

    /// Embed multiple texts with a shared task hint.
    ///
    /// Providers that support both batching and task hints can
    /// combine them into a single HTTP call (Jina v4 supports
    /// `input: [texts]` + `task: "retrieval.passage"`).
    async fn embed_batch_with_task(&self, texts: &[&str], task: &str) -> Vec<Vec<f32>> {
        if texts.is_empty() {
            return Vec::new();
        }
        let mut results = Vec::with_capacity(texts.len());
        for t in texts {
            results.push(self.embed_text_with_task(t, task).await);
        }
        results
    }
}

// ─── Default impls for existing providers ──────────────────────────────────

#[async_trait]
impl EmbeddingProviderExt for DeterministicEmbeddingProvider {}
#[async_trait]
impl EmbeddingProviderExt for OpenAiEmbeddingProvider {}
