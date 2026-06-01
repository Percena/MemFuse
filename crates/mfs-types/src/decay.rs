//! Ebbinghaus decay configuration and memory type classification.
//!
//! The multiplicative spacing-effect model (reviewed migration plan):
//!   λ_effective = λ_base / (1 + spacing_factor)
//!   spacing_factor = σ × Σ(1 / daysSinceAccess_i)
//!   final_score = salience × exp(-λ_effective × t)
//!
//! Intervals between recalls reduce the effective decay rate — the core
//! insight from Ebbinghaus's spacing effect: distributed recall strengthens
//! memory traces, not by adding a separate score but by lowering the
//! forgetting rate itself.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Memory type classification for type-specific decay rates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MemoryType {
    /// Heuristic rules — behavioral preferences distilled from interaction trajectory.
    /// Slowest decay (λ=0.02, half-life ~35 days) because validated preferences persist.
    Heuristic,
    /// Episodic memories — what happened in a session.
    /// Fastest decay (λ=0.03, half-life ~23 days) because specific events fade quickly.
    Episode,
    /// Factual knowledge — structured facts about the world/user/project.
    /// Longest-lived (λ=0.01, half-life ~69 days) because facts are reference knowledge.
    Fact,
}

/// Unified configuration for Ebbinghaus decay across all memory types.
///
/// Each memory type has its own base decay rate (λ), reflecting the
/// observation that different kinds of information are forgotten at
/// different speeds — Ebbinghaus himself noted this variability.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecayConfig {
    /// Reinforcement scaling factor (σ) for the spacing-effect boost.
    /// Each recall contributes σ × (1 / daysSinceAccess) to the spacing_factor.
    /// Default: 0.3 (recent access contributes
    /// 0.3, 10-day-old access contributes 0.03).
    pub sigma: f64,

    /// Maximum spacing_factor cap to prevent infinite reinforcement.
    /// If spacing_factor exceeds this, λ_effective approaches zero and
    /// memory never decays. Cap prevents this degenerate case.
    pub max_spacing_factor: f64,

    /// Type-specific base decay rates (λ_base).
    /// Keyed by MemoryType; if a type is missing, falls back to `default_lambda`.
    pub lambda_by_type: HashMap<MemoryType, f64>,

    /// Default λ when a type is not in `lambda_by_type`.
    pub default_lambda: f64,

    /// Tier thresholds for retention classification.
    pub tier_hot: f64,
    pub tier_warm: f64,
    pub tier_cold: f64,

    /// Survival threshold: memories below this score are candidates for archival.
    pub survival_threshold: f64,

    /// Simple expiry threshold for facts: if recall_count < this AND
    /// days since last recall > `fact_expiry_days`, mark as Expired.
    pub fact_expiry_min_recall: i64,
    pub fact_expiry_days: f64,
}

impl Default for DecayConfig {
    fn default() -> Self {
        let mut lambda_by_type = HashMap::new();
        lambda_by_type.insert(MemoryType::Heuristic, 0.02); // half-life ~35d
        lambda_by_type.insert(MemoryType::Episode, 0.03); // half-life ~23d
        lambda_by_type.insert(MemoryType::Fact, 0.01); // half-life ~69d

        Self {
            sigma: 0.3,
            max_spacing_factor: 5.0,
            lambda_by_type,
            default_lambda: 0.02,
            tier_hot: 0.7,
            tier_warm: 0.4,
            tier_cold: 0.15,
            survival_threshold: 0.1,
            fact_expiry_min_recall: 2,
            fact_expiry_days: 90.0,
        }
    }
}

impl DecayConfig {
    /// Construct DecayConfig from environment variables, falling back to defaults.
    ///
    /// Env vars: MEMFUSE_DECAY_SIGMA, MEMFUSE_DECAY_MAX_SPACING,
    /// MEMFUSE_DECAY_LAMBDA_HEURISTIC, MEMFUSE_DECAY_LAMBDA_EPISODE,
    /// MEMFUSE_DECAY_LAMBDA_FACT, MEMFUSE_DECAY_DEFAULT_LAMBDA,
    /// MEMFUSE_DECAY_TIER_HOT, MEMFUSE_DECAY_TIER_WARM,
    /// MEMFUSE_DECAY_TIER_COLD, MEMFUSE_DECAY_SURVIVAL_THRESHOLD,
    /// MEMFUSE_DECAY_FACT_EXPIRY_MIN_RECALL, MEMFUSE_DECAY_FACT_EXPIRY_DAYS
    pub fn from_env() -> Self {
        let mut cfg = Self::default();

        if let Ok(v) = std::env::var("MEMFUSE_DECAY_SIGMA") {
            cfg.sigma = v.parse().unwrap_or(cfg.sigma);
        }
        if let Ok(v) = std::env::var("MEMFUSE_DECAY_MAX_SPACING") {
            cfg.max_spacing_factor = v.parse().unwrap_or(cfg.max_spacing_factor);
        }
        if let Ok(v) = std::env::var("MEMFUSE_DECAY_LAMBDA_HEURISTIC") {
            cfg.lambda_by_type
                .insert(MemoryType::Heuristic, v.parse().unwrap_or(0.02));
        }
        if let Ok(v) = std::env::var("MEMFUSE_DECAY_LAMBDA_EPISODE") {
            cfg.lambda_by_type
                .insert(MemoryType::Episode, v.parse().unwrap_or(0.03));
        }
        if let Ok(v) = std::env::var("MEMFUSE_DECAY_LAMBDA_FACT") {
            cfg.lambda_by_type
                .insert(MemoryType::Fact, v.parse().unwrap_or(0.01));
        }
        if let Ok(v) = std::env::var("MEMFUSE_DECAY_DEFAULT_LAMBDA") {
            cfg.default_lambda = v.parse().unwrap_or(cfg.default_lambda);
        }
        if let Ok(v) = std::env::var("MEMFUSE_DECAY_TIER_HOT") {
            cfg.tier_hot = v.parse().unwrap_or(cfg.tier_hot);
        }
        if let Ok(v) = std::env::var("MEMFUSE_DECAY_TIER_WARM") {
            cfg.tier_warm = v.parse().unwrap_or(cfg.tier_warm);
        }
        if let Ok(v) = std::env::var("MEMFUSE_DECAY_TIER_COLD") {
            cfg.tier_cold = v.parse().unwrap_or(cfg.tier_cold);
        }
        if let Ok(v) = std::env::var("MEMFUSE_DECAY_SURVIVAL_THRESHOLD") {
            cfg.survival_threshold = v.parse().unwrap_or(cfg.survival_threshold);
        }
        if let Ok(v) = std::env::var("MEMFUSE_DECAY_FACT_EXPIRY_MIN_RECALL") {
            cfg.fact_expiry_min_recall = v.parse().unwrap_or(cfg.fact_expiry_min_recall);
        }
        if let Ok(v) = std::env::var("MEMFUSE_DECAY_FACT_EXPIRY_DAYS") {
            cfg.fact_expiry_days = v.parse().unwrap_or(cfg.fact_expiry_days);
        }

        cfg
    }

    /// Get the base decay rate (λ) for a given memory type.
    pub fn lambda_for(&self, mt: MemoryType) -> f64 {
        self.lambda_by_type
            .get(&mt)
            .copied()
            .unwrap_or(self.default_lambda)
    }

    /// Compute the Ebbinghaus retention score using the multiplicative spacing-effect model.
    ///
    /// λ_effective = λ_base / (1 + spacing_factor)
    /// spacing_factor = σ × Σ(1 / daysSinceAccess_i), capped at max_spacing_factor
    /// final_score = salience × exp(-λ_effective × days_since_creation)
    ///
    /// When there are no access records, spacing_factor = 0 and the formula
    /// reduces to the classic exponential forgetting curve:
    ///   final_score = salience × exp(-λ_base × t)
    pub fn compute_retention(
        &self,
        mt: MemoryType,
        salience: f64,
        days_since_creation: f64,
        access_days: &[f64], // days_since_access for each recall event
    ) -> f64 {
        let lambda_base = self.lambda_for(mt);

        // Guard: clamp negative days_since_creation to 0.0.
        // Clock skew or injected future timestamps would produce exp(+k) > 1.0,
        // violating the model's constraint that retention ≤ salience.
        let days_since_creation = days_since_creation.max(0.0);

        // Compute spacing_factor with cap
        let spacing_factor = self.compute_spacing_factor(access_days);

        // Effective decay rate: distributed recall reduces forgetting speed
        let lambda_effective = lambda_base / (1.0 + spacing_factor);

        // Ebbinghaus forgetting curve with adjusted rate
        salience * std::f64::consts::E.powf(-lambda_effective * days_since_creation)
    }

    /// Compute the spacing factor from access history.
    ///
    /// σ × Σ(1 / daysSinceAccess_i), capped at max_spacing_factor.
    /// More recent accesses contribute more (1/d is larger when d is small).
    pub fn compute_spacing_factor(&self, access_days: &[f64]) -> f64 {
        let raw = self.sigma
            * access_days
                .iter()
                .filter(|d| **d > 0.0) // skip zero/negative (clock skew or same-instant)
                .map(|d| 1.0 / d)
                .sum::<f64>();
        raw.min(self.max_spacing_factor)
    }

    /// Classify a retention score into a tier.
    pub fn classify_tier(&self, score: f64) -> &'static str {
        if score >= self.tier_hot {
            "hot"
        } else if score >= self.tier_warm {
            "warm"
        } else if score >= self.tier_cold {
            "cold"
        } else {
            "evictable"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_lambda_values() {
        let cfg = DecayConfig::default();
        assert_eq!(cfg.lambda_for(MemoryType::Heuristic), 0.02);
        assert_eq!(cfg.lambda_for(MemoryType::Episode), 0.03);
        assert_eq!(cfg.lambda_for(MemoryType::Fact), 0.01);
    }

    #[test]
    fn classic_forgetting_curve_no_access() {
        let cfg = DecayConfig::default();
        // No access history → pure exponential decay
        let score = cfg.compute_retention(MemoryType::Heuristic, 1.0, 34.7, &[]);
        // With λ=0.02, half-life ~34.7 days; score should be ~0.5
        assert!((score - 0.5).abs() < 0.05);
    }

    #[test]
    fn spacing_effect_reduces_decay() {
        let cfg = DecayConfig::default();
        let score_no_access = cfg.compute_retention(MemoryType::Episode, 1.0, 30.0, &[]);
        let score_with_access =
            cfg.compute_retention(MemoryType::Episode, 1.0, 30.0, &[1.0, 5.0, 10.0]);
        // Access history should reduce forgetting → higher score
        assert!(score_with_access > score_no_access);
    }

    #[test]
    fn spacing_factor_capped() {
        let cfg = DecayConfig::default();
        // Many recent accesses → spacing_factor should hit cap
        let many_accesses: Vec<f64> = (1..100).map(|i| 0.1 + i as f64 * 0.01).collect();
        let score = cfg.compute_retention(MemoryType::Episode, 1.0, 100.0, &many_accesses);
        // Even with massive access history, score should not approach 1.0 at t=100
        assert!(score < 1.0);
    }

    #[test]
    fn tier_classification() {
        let cfg = DecayConfig::default();
        assert_eq!(cfg.classify_tier(0.8), "hot");
        assert_eq!(cfg.classify_tier(0.5), "warm");
        assert_eq!(cfg.classify_tier(0.2), "cold");
        assert_eq!(cfg.classify_tier(0.1), "evictable");
    }

    #[test]
    fn multiplicative_model_does_not_exceed_salience() {
        let cfg = DecayConfig::default();
        // Unlike the additive model, multiplicative model never produces
        // final_score > salience (even with infinite reinforcement)
        let score = cfg.compute_retention(MemoryType::Fact, 0.5, 1.0, &[1.0]);
        assert!(score <= 0.5 + 0.001); // slight margin for float
    }

    #[test]
    fn fact_expiry_threshold() {
        let cfg = DecayConfig::default();
        assert_eq!(cfg.fact_expiry_min_recall, 2);
        assert_eq!(cfg.fact_expiry_days, 90.0);
    }

    #[test]
    fn spacing_factor_ignores_zero_and_negative_days() {
        let cfg = DecayConfig::default();
        // Zero and negative days should be filtered out (clock skew / same-instant)
        let score_with_zero =
            cfg.compute_retention(MemoryType::Episode, 0.8, 30.0, &[0.0, 1.0, 5.0]);
        let score_without_zero = cfg.compute_retention(MemoryType::Episode, 0.8, 30.0, &[1.0, 5.0]);
        assert!(
            (score_with_zero - score_without_zero).abs() < 0.001,
            "zero-valued access days should be ignored"
        );

        let score_with_negative =
            cfg.compute_retention(MemoryType::Episode, 0.8, 30.0, &[-1.0, 1.0, 5.0]);
        let score_without_negative =
            cfg.compute_retention(MemoryType::Episode, 0.8, 30.0, &[1.0, 5.0]);
        assert!(
            (score_with_negative - score_without_negative).abs() < 0.001,
            "negative access days (future timestamps) should be ignored"
        );
    }
}
