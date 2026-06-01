//! Maintenance CLI commands for heuristic rule lifecycle management (roadmap §8).
//!
//! Provides CLI entry points for:
//! - Decay: recomputing aggregate_weight with exponential decay + archival
//! - Sleep consolidation: contrast-pair discovery + rule merging
//! - Scheduled maintenance: periodic decay + consolidation (roadmap §5.5)

use crate::helpers::CliState;

use mfs_memory::consolidation_sleep::run_sleep_consolidation;
use mfs_memory::heuristics::{
    DEFAULT_DECAY_LAMBDA, DEFAULT_SURVIVAL_THRESHOLD, decay_heuristic_weights,
};
use mfs_memory::llm::LlmAssist;

/// Run heuristic decay maintenance pass.
/// Recomputes aggregate_weight for all active rules using exponential decay.
/// User-confirmed rules and recently active rules are exempt (roadmap §5.4).
#[allow(clippy::unnecessary_wraps)]
pub fn handle_heuristic_decay(state: &CliState) -> Result<(), Box<dyn std::error::Error>> {
    let result = decay_heuristic_weights(
        &state.metadata,
        &state.cli.account_id,
        &state.cli.user_id,
        DEFAULT_DECAY_LAMBDA,
        DEFAULT_SURVIVAL_THRESHOLD,
    );

    println!("decayed={}", result.decayed);
    println!("skipped_confirmed={}", result.skipped_confirmed);
    println!(
        "skipped_active_protected={}",
        result.skipped_active_protected
    );
    println!("archived={}", result.archived);

    Ok(())
}

/// Run sleep consolidation pass.
/// Discover contrast pairs, extract latent variables via LLM, merge rules.
pub async fn handle_heuristic_consolidate(
    state: &CliState,
) -> Result<(), Box<dyn std::error::Error>> {
    let llm = LlmAssist::from_env();
    let result = run_sleep_consolidation(
        &state.metadata,
        &state.cli.account_id,
        &state.cli.user_id,
        &state.cli.agent_id,
        &llm,
    )
    .await;

    println!("pairs_discovered={}", result.pairs_discovered);
    println!("latent_variables_found={}", result.latent_variables_found);
    println!("rules_merged={}", result.rules_merged);
    println!("rules_archived={}", result.rules_archived);
    println!("pending_pairs={}", result.pending_pairs);

    Ok(())
}

/// Run scheduled maintenance: decay + sleep consolidation in sequence.
/// This is the recommended periodic maintenance task (roadmap §5.5).
/// Runs decay first (cheap, deterministic), then consolidation (LLM-enhanced).
///
/// When run with `--schedule <interval_secs>`, this loops forever at the
/// given interval. Otherwise it runs a single pass.
pub async fn handle_heuristic_scheduled(
    state: &CliState,
    interval_secs: Option<u64>,
) -> Result<(), Box<dyn std::error::Error>> {
    let interval = interval_secs.unwrap_or(0);
    let llm = LlmAssist::from_env();

    loop {
        let now = chrono::Utc::now();
        println!("=== Scheduled maintenance pass at {} ===", now.to_rfc3339());

        // Step 1: Decay (cheap, always runs)
        let decay_result = decay_heuristic_weights(
            &state.metadata,
            &state.cli.account_id,
            &state.cli.user_id,
            DEFAULT_DECAY_LAMBDA,
            DEFAULT_SURVIVAL_THRESHOLD,
        );
        println!(
            "  decay: decayed={}, skipped_confirmed={}, skipped_active_protected={}, archived={}",
            decay_result.decayed,
            decay_result.skipped_confirmed,
            decay_result.skipped_active_protected,
            decay_result.archived
        );

        // Step 2: Sleep consolidation (LLM-enhanced, may degrade to deterministic)
        let consolidation_result = run_sleep_consolidation(
            &state.metadata,
            &state.cli.account_id,
            &state.cli.user_id,
            &state.cli.agent_id,
            &llm,
        )
        .await;
        println!(
            "  consolidation: pairs={}, latent_vars={}, merged={}, archived={}, pending={}",
            consolidation_result.pairs_discovered,
            consolidation_result.latent_variables_found,
            consolidation_result.rules_merged,
            consolidation_result.rules_archived,
            consolidation_result.pending_pairs
        );

        if interval == 0 {
            break; // Single pass mode
        }

        println!("  next pass in {}s (press Ctrl+C to stop)", interval);
        tokio::select! {
            () = tokio::time::sleep(tokio::time::Duration::from_secs(interval)) => {},
            _ = tokio::signal::ctrl_c() => {
                println!("  received Ctrl+C, exiting scheduled maintenance");
                break;
            }
        }
    }

    Ok(())
}
