//! Periodic cross-session memory consolidation (auto-dream style).
//!
//! Consolidation triggers:
//! - Time gate: hours since last consolidation >= min_hours (default: 24)
//! - Session gate: committed sessions since last consolidation >= min_sessions (default: 5)
//!
//! Uses 4-phase pipeline (§5.3):
//! Phase 1 — Orient: scan current memory state
//! Phase 2 — Gather: consolidate eligible sessions
//! Phase 3 — Consolidate: resolve conflicts, compress stale episodes
//! Phase 4 — Prune: archive decayed items, refresh briefs

use crate::http::AppState;
use chrono::Utc;
use mfs_memory::dream_phases::execute_dream_consolidation;
use mfs_memory::llm::LlmAssist;
use mfs_metadata::{AuditEventRecord, SessionRow};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

/// Spawn the periodic consolidation loop. Returns a JoinHandle.
pub fn spawn_dream_loop(
    state: Arc<AppState>,
    cancel: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    let min_hours: u64 = std::env::var("MEMFUSE_DREAM_MIN_HOURS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(24);
    let min_sessions: usize = std::env::var("MEMFUSE_DREAM_MIN_SESSIONS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(5);
    let poll_secs: u64 = std::env::var("MEMFUSE_DREAM_POLL_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(300);

    tokio::spawn(async move {
        info!(
            "Dream loop started (min_hours={min_hours}, min_sessions={min_sessions}, poll={poll_secs}s)"
        );
        let llm = LlmAssist::from_env();

        loop {
            tokio::select! {
                () = cancel.cancelled() => { info!("Dream loop cancelled"); return; }
                () = tokio::time::sleep(std::time::Duration::from_secs(poll_secs)) => {}
            }

            let metadata = state.metadata.clone();
            let account_id = &state.config.account_id;
            let user_id = &state.config.user_id;
            let agent_id = &state.config.agent_id;

            // Gate 1: Time — parse as proper datetime, not string comparison
            let last_ts = metadata
                .get_latest_audit_by_event_type("dream_consolidation")
                .unwrap_or(None);
            if let Some(ref ts) = last_ts {
                if let Ok(last) = chrono::DateTime::parse_from_rfc3339(ts) {
                    let hours = (Utc::now() - last.with_timezone(&Utc)).num_hours();
                    if hours < min_hours as i64 {
                        debug!("Dream: time gate not met ({hours}h < {min_hours}h)");
                        continue;
                    }
                }
                // If parse fails, treat as "never consolidated" and proceed
            }

            // Gate 2: Session count — use proper datetime comparison
            let sessions = metadata
                .list_sessions_by_user(account_id, user_id, Some("committed"))
                .unwrap_or_default();
            let eligible: Vec<SessionRow> = match &last_ts {
                Some(ts) => {
                    let cutoff = chrono::DateTime::parse_from_rfc3339(ts).ok();
                    sessions
                        .into_iter()
                        .filter(|s| {
                            match (
                                &cutoff,
                                chrono::DateTime::parse_from_rfc3339(&s.last_activity_at).ok(),
                            ) {
                                (Some(c), Some(a)) => a > *c,
                                _ => s.last_activity_at.as_str() > ts.as_str(), // fallback
                            }
                        })
                        .collect()
                }
                None => sessions,
            };

            if eligible.len() < min_sessions {
                debug!(
                    "Dream: session gate not met ({} < {min_sessions})",
                    eligible.len()
                );
                continue;
            }

            info!(
                "Dream: gates met — executing 4-phase consolidation for {} sessions",
                eligible.len()
            );

            match execute_dream_consolidation(
                &metadata, account_id, user_id, agent_id, &eligible, &llm,
            )
            .await
            {
                Ok(report) => {
                    // Write overall dream audit log
                    let details = serde_json::json!({
                        "sessions": report.gather.sessions_processed,
                        "new_episodes": report.gather.new_episodes,
                        "new_facts": report.gather.new_facts,
                        "fact_conflicts": report.gather.fact_conflicts,
                        "facts_resolved": report.consolidate.facts_resolved,
                        "episodes_archived_stale": report.consolidate.episodes_archived_stale,
                        "episodes_archived": report.prune.episodes_archived,
                        "rules_archived": report.prune.rules_archived,
                    })
                    .to_string();
                    let _ = metadata.append_audit(&AuditEventRecord {
                        account_id,
                        user_id,
                        agent_id: Some(agent_id),
                        projection_view_id: None,
                        event_type: "dream_consolidation",
                        subject_uri: None,
                        actor: Some("dream_loop"),
                        details_json: Some(&details),
                    });
                    info!(
                        "Dream: completed 4-phase consolidation for {} sessions",
                        report.gather.sessions_processed
                    );
                }
                Err(e) => {
                    warn!("Dream: 4-phase consolidation failed: {e}");
                }
            }
        }
    })
}
