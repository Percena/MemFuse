//! Shared mathematical utility functions.
//!
//! These are pure functions with no external dependencies, placed in the
//! leaf crate `mfs-types` so they can be reused by all other crates
//! (including `mfs-index` which intentionally avoids heavier deps).

/// Cosine similarity between two `f32` embedding vectors.
///
/// Returns `0.0` for mismatched dimensions, empty inputs, or zero-norm
/// vectors.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f64 {
    if a.is_empty() || b.is_empty() || a.len() != b.len() {
        return 0.0;
    }
    let mut dot = 0.0_f64;
    let mut norm_a = 0.0_f64;
    let mut norm_b = 0.0_f64;
    for (left, right) in a.iter().zip(b.iter()) {
        let left = f64::from(*left);
        let right = f64::from(*right);
        dot += left * right;
        norm_a += left * left;
        norm_b += right * right;
    }
    let denom = norm_a.sqrt() * norm_b.sqrt();
    if denom == 0.0 { 0.0 } else { dot / denom }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_vectors() {
        let v = vec![1.0, 2.0, 3.0];
        assert!((cosine_similarity(&v, &v) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn orthogonal_vectors() {
        assert!(cosine_similarity(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-9);
    }

    #[test]
    fn mismatched_dimensions() {
        assert_eq!(cosine_similarity(&[1.0, 2.0], &[1.0, 2.0, 3.0]), 0.0);
    }

    #[test]
    fn empty_inputs() {
        assert_eq!(cosine_similarity(&[], &[]), 0.0);
    }

    #[test]
    fn zero_norm() {
        assert_eq!(cosine_similarity(&[0.0, 0.0], &[1.0, 2.0]), 0.0);
    }

    #[test]
    fn known_angle() {
        // 45° angle: (1,0) vs (1,1) → cos(45°) = 1/√2 ≈ 0.7071
        let sim = cosine_similarity(&[1.0, 0.0], &[1.0, 1.0]);
        assert!((sim - 1.0 / 2.0_f64.sqrt()).abs() < 1e-6);
    }
}
