//! Shared embedding utility functions used across crates.
//!
//! Centralises embedding JSON parsing and query embedding helpers.
//! Cosine similarity lives in `mfs_types::math` (leaf crate, no
//! serde/async deps) so it can be reused by `mfs-index` without
//! pulling in `mfs-semantic`.

use crate::providers::{EmbeddingProvider, ProcessingMode};

// Re-export cosine_similarity from mfs_types::math for convenience:
// callers importing mfs_semantic get it without also needing mfs_types::math.
pub use mfs_types::math::cosine_similarity;

/// Parse an embedding JSON string (`serde_json`-serialised `Vec<f32>`)
/// back into a `Vec<f32>`.  Returns `None` for missing or malformed
/// values.
pub fn parse_embedding_json(json: &Option<String>) -> Option<Vec<f32>> {
    let s = json.as_ref()?;
    serde_json::from_str(s).ok()
}

/// Attempt to embed a query string using the given provider.
///
/// Returns `None` if:
/// - the query is empty,
/// - the provider is in [`ProcessingMode::Degraded`],
/// - the embedding vector is empty or all-zero.
pub async fn try_embed_query(query: &str, provider: &dyn EmbeddingProvider) -> Option<Vec<f32>> {
    if query.is_empty() {
        return None;
    }
    if provider.mode() == ProcessingMode::Degraded {
        return None;
    }
    let embedding = provider.embed_text(query).await;
    if embedding.is_empty() || embedding.iter().all(|v| *v == 0.0) {
        None
    } else {
        Some(embedding)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_similarity_identical_vectors() {
        let v = vec![1.0, 2.0, 3.0];
        assert!((cosine_similarity(&v, &v) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn cosine_similarity_orthogonal_vectors() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        assert!(cosine_similarity(&a, &b).abs() < 1e-9);
    }

    #[test]
    fn cosine_similarity_mismatched_dimensions() {
        let a = vec![1.0, 2.0];
        let b = vec![1.0, 2.0, 3.0];
        assert_eq!(cosine_similarity(&a, &b), 0.0);
    }

    #[test]
    fn cosine_similarity_empty_vectors() {
        assert_eq!(cosine_similarity(&[], &[]), 0.0);
    }

    #[test]
    fn cosine_similarity_zero_norm() {
        let a = vec![0.0, 0.0];
        let b = vec![1.0, 2.0];
        assert_eq!(cosine_similarity(&a, &b), 0.0);
    }

    #[test]
    fn parse_embedding_json_valid() {
        let json = Some(serde_json::to_string(&vec![1.0_f32, 2.0, 3.0]).unwrap());
        let parsed = parse_embedding_json(&json);
        assert_eq!(parsed, Some(vec![1.0, 2.0, 3.0]));
    }

    #[test]
    fn parse_embedding_json_none() {
        assert_eq!(parse_embedding_json(&None), None);
    }

    #[test]
    fn parse_embedding_json_malformed() {
        assert_eq!(parse_embedding_json(&Some("not json".to_owned())), None);
    }
}
