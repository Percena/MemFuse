//! Fact expiry — Ebbinghaus-based fact decay and expiry.

use serde::{Deserialize, Serialize};

/// Result of fact expiry maintenance pass.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FactExpiryResult {
    /// Number of facts checked for expiry.
    pub checked: usize,
    /// Number of facts expired (transitioned from Active to Expired).
    pub expired: usize,
}

/// Expire stale active facts using the Ebbinghaus retention model.
///
/// Two-phase approach:
/// 1. Simple rule first: if recall_count < threshold AND days since last recall > expiry_days → Expired
/// 2. Ebbinghaus fine-grained: compute retention score for remaining facts, expire if below survival_threshold
///
/// This activates the previously unused `FactStatus::Expired` state,
/// giving facts a time-based lifecycle beyond just supersession/retraction.
pub fn expire_stale_facts(
    metadata: &mfs_metadata::MetadataStore,
    account_id: &str,
    user_id: &str,
    config: &mfs_types::DecayConfig,
) -> FactExpiryResult {
    let facts = metadata
        .get_active_facts(account_id, user_id)
        .unwrap_or_default();

    let now = chrono::Utc::now();
    let mut expired = 0;

    for fact in &facts {
        // Skip facts that are already superseded/retracted/expired
        if fact.status != "active" {
            continue;
        }

        // ── Phase 1: Simple expiry rule ──
        // Facts with very low recall and long inactivity → immediate expiry.
        // When last_recalled_at is NULL (never recalled), use created_at
        // instead of a sentinel value — a newly-created fact shouldn't be
        // expired just because it hasn't been recalled yet.
        let days_since_staleness = fact
            .last_recalled_at
            .as_ref()
            .and_then(|ts| chrono::DateTime::parse_from_rfc3339(ts).ok())
            .map(|dt| {
                let utc_dt = dt.with_timezone(&chrono::Utc);
                (now - utc_dt).num_seconds() as f64 / 86400.0
            })
            .or_else(|| {
                // Never recalled: fall back to days since creation
                chrono::DateTime::parse_from_rfc3339(&fact.created_at)
                    .ok()
                    .map(|dt| (now - dt.with_timezone(&chrono::Utc)).num_seconds() as f64 / 86400.0)
            })
            .unwrap_or(0.0); // both timestamps unparseable → skip expiry (safe default)

        if fact.recall_count < config.fact_expiry_min_recall
            && days_since_staleness > config.fact_expiry_days
        {
            metadata.expire_fact(&fact.id).ok();
            expired += 1;
            continue;
        }

        // ── Phase 2: Ebbinghaus fine-grained expiry ──
        // Compute retention score for facts with some recall history.
        // Skip facts with unparseable timestamps (safe default: don't expire).
        let days_since_creation = match chrono::DateTime::parse_from_rfc3339(&fact.created_at)
            .ok()
            .map(|dt| (now - dt.with_timezone(&chrono::Utc)).num_seconds() as f64 / 86400.0)
        {
            Some(d) => d,
            None => {
                tracing::warn!(
                    fact_id = %fact.id,
                    created_at = %fact.created_at,
                    "skipping fact expiry: unparseable created_at"
                );
                continue;
            }
        };

        let access_days = metadata
            .get_access_days_since(&fact.id, &now)
            .unwrap_or_default();

        let retention_score = config.compute_retention(
            mfs_types::MemoryType::Fact,
            fact.confidence, // use confidence as salience for facts
            days_since_creation,
            &access_days,
        );

        if retention_score < config.survival_threshold {
            metadata.expire_fact(&fact.id).ok();
            expired += 1;
        }
    }

    FactExpiryResult {
        checked: facts.len(),
        expired,
    }
}
