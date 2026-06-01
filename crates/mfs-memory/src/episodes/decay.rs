//! Episode decay — salience decay and archival of cold episodes.

use std::collections::HashMap;

/// Episode maintenance result.
#[derive(Debug, Clone)]
pub struct EpisodeMaintenanceResult {
    /// Number of episodes whose salience was decayed.
    pub decayed: usize,
    /// Number of cold episodes archived.
    pub archived: usize,
}

/// Compute salience decay for an episode using the Ebbinghaus multiplicative
/// spacing-effect model.
///
/// **Deprecated for Dream-cycle use.** This function uses `days_since_creation`
/// as the time delta, which compounds exponentially when called repeatedly on
/// already-decayed values. The Dream cycle now uses `decay_episode_salience`
/// which applies incremental decay (`days_since_last_decay`) instead.
///
/// This function remains available for one-shot analysis (e.g., computing a
/// theoretical retention curve from scratch) but should NOT be called in any
/// periodic maintenance loop.
///
/// λ_effective = λ_base / (1 + spacing_factor)
/// spacing_factor = σ × Σ(1 / daysSinceAccess_i), capped at max_spacing_factor
/// final_score = current_salience × exp(-λ_effective × days_since_creation)
///
/// Falls back to `DecayConfig::default()` if no config is provided.
#[deprecated(
    note = "Use decay_episode_salience for periodic maintenance; this uses days_since_creation which compounds"
)]
pub fn compute_salience_decay(
    current_salience: f64,
    days_since_creation: f64,
    access_days: &[f64],
    config: Option<&mfs_types::DecayConfig>,
) -> f64 {
    let cfg = config.cloned().unwrap_or_default();
    cfg.compute_retention(
        mfs_types::MemoryType::Episode,
        current_salience,
        days_since_creation,
        access_days,
    )
}

/// Recompute salience for all active episodes using Ebbinghaus decay.
///
/// Uses **incremental decay**: each Dream cycle computes the retention
/// ratio `exp(-λ_effective × days_since_last_decay)` and multiplies the
/// stored salience by this ratio. This avoids the compound-decay bug where
/// `salience × exp(-λ × total_age)` applied on already-decayed values
/// would produce `original × exp(-Nλt)` after N cycles.
///
/// For each episode:
/// 1. Compute days since the last decay cycle (from last_decay_at)
/// 2. Retrieve access history from memory_access_log
/// 3. Compute spacing_factor and λ_effective
/// 4. Apply incremental retention: new_salience = current × exp(-λ_eff × Δt)
/// 5. Update salience_score only (preserve recall_count/last_recalled_at)
/// 6. If salience < survival_threshold → archive the episode
pub fn decay_episode_salience(
    metadata: &mfs_metadata::MetadataStore,
    account_id: &str,
    user_id: &str,
    config: &mfs_types::DecayConfig,
) -> EpisodeMaintenanceResult {
    let episodes = match metadata.get_episodes_by_user(account_id, user_id, None) {
        Ok(eps) => eps,
        Err(e) => {
            tracing::error!(error = %e, "decay_episode_salience: failed to fetch episodes");
            return EpisodeMaintenanceResult {
                decayed: 0,
                archived: 0,
            };
        }
    };

    let mut decayed = 0;
    let mut archived = 0;
    let mut update_failed = 0;
    let mut archive_failed = 0;
    let now = chrono::Utc::now();

    // Batch-retrieve access history for all active episodes (N+1 → single query)
    let active_ids: Vec<String> = episodes
        .iter()
        .filter(|ep| ep.archived_at.is_none())
        .map(|ep| ep.episode_id.clone())
        .collect();
    let access_batch = metadata
        .get_access_days_since_batch(&active_ids, &now)
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "decay: batch access_days fetch failed, using empty per-episode");
            HashMap::new()
        });

    for ep in &episodes {
        if ep.archived_at.is_some() {
            continue;
        }

        // Compute incremental time delta since last decay cycle.
        let last_decay_ts = ep
            .last_decay_at
            .as_ref()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .or_else(|| {
                chrono::DateTime::parse_from_rfc3339(&ep.created_at)
                    .ok()
                    .map(|dt| dt.with_timezone(&chrono::Utc))
            });
        let days_since_last_decay = match last_decay_ts {
            Some(ts) => ((now - ts).num_seconds() as f64 / 86400.0).max(0.0),
            None => {
                tracing::warn!(
                    episode_id = %ep.episode_id,
                    last_decay_at = ?ep.last_decay_at,
                    created_at = %ep.created_at,
                    "skipping episode decay: unparseable timestamps"
                );
                continue;
            }
        };

        // Skip episodes with zero time delta (updated within this second)
        if days_since_last_decay < 0.0001 {
            continue;
        }

        // Use batch-retrieved access history (or empty if batch fetch failed)
        let access_days = access_batch
            .get(&ep.episode_id)
            .cloned()
            .unwrap_or_default();

        // Compute incremental retention ratio
        let lambda_effective = config.lambda_for(mfs_types::MemoryType::Episode)
            / (1.0 + config.compute_spacing_factor(&access_days));
        let retention_ratio = std::f64::consts::E.powf(-lambda_effective * days_since_last_decay);
        let new_salience = ep.salience_score * retention_ratio;

        // Update only salience_score — preserve recall_count and last_recalled_at
        let now_decay_str = now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        if metadata
            .update_episode_salience_only(&ep.episode_id, new_salience, &now_decay_str)
            .is_ok()
        {
            decayed += 1;
        } else {
            update_failed += 1;
        }

        // Archive if below survival threshold
        if new_salience < config.survival_threshold {
            let now_str = now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
            match metadata.archive_episode(&ep.episode_id, &now_str) {
                Ok(()) => archived += 1,
                Err(e) => {
                    tracing::warn!(
                        episode_id = %ep.episode_id,
                        salience = new_salience,
                        error = %e,
                        "decay: failed to archive episode below threshold"
                    );
                    archive_failed += 1;
                }
            }
        }
    }

    if update_failed > 0 {
        tracing::warn!(
            update_failed = update_failed,
            "decay_episode_salience: some salience updates failed"
        );
    }
    if archive_failed > 0 {
        tracing::warn!(
            archive_failed = archive_failed,
            "decay_episode_salience: some archive operations failed"
        );
    }

    EpisodeMaintenanceResult { decayed, archived }
}

#[cfg(test)]
mod tests {
    #[allow(deprecated)]
    use super::*;

    #[test]
    fn salience_decay_no_access() {
        let decayed = compute_salience_decay(0.8, 10.0, &[], None);
        assert!(decayed < 0.8);
        assert!(decayed > 0.1);
    }

    #[test]
    fn salience_decay_with_access() {
        let no_access = compute_salience_decay(0.8, 30.0, &[], None);
        let with_access = compute_salience_decay(0.8, 30.0, &[1.0, 5.0], None);
        assert!(with_access > no_access);
    }
}
