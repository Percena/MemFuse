//! Briefs module — cross-thread memory brief generation.
//!
//! Memory briefs are derived summaries that provide cross-thread context:
//! - Resource briefs: episodes from a specific resource across sessions
//! - User briefs: episodes across all threads for a user
//!
//! Briefs are refreshed after consolidation and truncated to 160 chars.

use crate::{BRIEF_SUMMARY_TRUNCATE, EpisodeChunk, MemoryBrief};

/// Build a resource-scoped memory brief from recent episodes.
pub fn build_resource_memory_brief(
    user_id: &str,
    resource_id: &str,
    episodes: &[EpisodeChunk],
) -> Option<MemoryBrief> {
    build_memory_brief(
        user_id,
        "resource",
        resource_id,
        "Cross-thread brief",
        episodes,
    )
}

/// Build a user-scoped memory brief from recent episodes.
pub fn build_user_memory_brief(user_id: &str, episodes: &[EpisodeChunk]) -> Option<MemoryBrief> {
    build_memory_brief(user_id, "user", user_id, "User-memory brief", episodes)
}

/// Build a memory brief from episodes.
///
/// Collects source thread IDs, anchor episode IDs, and up to 3 summary parts.
/// Truncates the combined summary to 160 characters.
fn build_memory_brief(
    user_id: &str,
    scope_type: &str,
    scope_id: &str,
    label: &str,
    episodes: &[EpisodeChunk],
) -> Option<MemoryBrief> {
    if scope_id.is_empty() || episodes.is_empty() {
        return None;
    }

    let (source_thread_ids, anchor_episode_ids, summary_parts) =
        collect_memory_brief_data(episodes);

    if source_thread_ids.is_empty() {
        return None;
    }

    let combined = format!("{}: {}", label, summary_parts.join(" | "));
    let summary = summarize_brief_text(&combined);

    Some(MemoryBrief {
        brief_id: format!("brf_{}", uuid::Uuid::new_v4()),
        user_id: user_id.to_owned(),
        scope_type: scope_type.to_owned(),
        scope_id: scope_id.to_owned(),
        summary,
        source_thread_ids,
        anchor_episode_ids,
        updated_at: None,
    })
}

/// Collect source thread IDs, anchor episode IDs, and summary parts from episodes.
fn collect_memory_brief_data(episodes: &[EpisodeChunk]) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut seen_threads = std::collections::HashSet::new();
    let mut seen_episodes = std::collections::HashSet::new();
    let mut source_thread_ids: Vec<String> = Vec::new();
    let mut anchor_episode_ids: Vec<String> = Vec::new();
    let mut summary_parts: Vec<String> = Vec::new();

    for ep in episodes {
        if !ep.session_id.is_empty() && !seen_threads.contains(&ep.session_id) {
            seen_threads.insert(ep.session_id.clone());
            source_thread_ids.push(ep.session_id.clone());
        }
        if !ep.episode_id.is_empty() && !seen_episodes.contains(&ep.episode_id) {
            seen_episodes.insert(ep.episode_id.clone());
            anchor_episode_ids.push(ep.episode_id.clone());
        }
        if summary_parts.len() < 3 && !ep.summary.trim().is_empty() {
            summary_parts.push(ep.summary.trim().to_owned());
        }
    }

    (source_thread_ids, anchor_episode_ids, summary_parts)
}

/// Truncate brief text to BRIEF_SUMMARY_TRUNCATE (160) characters.
fn summarize_brief_text(text: &str) -> String {
    let text = text.trim();
    if text.len() <= BRIEF_SUMMARY_TRUNCATE {
        text.to_owned()
    } else {
        let boundary = text.floor_char_boundary(BRIEF_SUMMARY_TRUNCATE);
        format!("{}...", &text[..boundary])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_episode(episode_id: &str, session_id: &str, summary: &str) -> EpisodeChunk {
        EpisodeChunk {
            episode_id: episode_id.to_owned(),
            user_id: "u1".to_owned(),
            session_id: session_id.to_owned(),
            resource_id: None,
            summary: summary.to_owned(),
            salience_score: 0.5,
            strength_score: 1.0,
            recall_count: 0,
            last_recalled_at: None,
            source_start_turn_id: "t1".to_owned(),
            source_end_turn_id: "t2".to_owned(),
            created_at: "2026-01-01T10:00:00Z".to_owned(),
            embedding: None,
            emotional_valence: None,
            emotional_intensity: None,
            context_tags_json: None,
        }
    }

    #[test]
    fn build_resource_brief() {
        let episodes = vec![
            make_episode("ep1", "s1", "User discussed auth system"),
            make_episode("ep2", "s2", "User worked on rate limiting"),
            make_episode("ep3", "s1", "User reviewed middleware"),
        ];
        let brief = build_resource_memory_brief("u1", "r1", &episodes).unwrap();
        assert_eq!(brief.scope_type, "resource");
        assert_eq!(brief.scope_id, "r1");
        assert_eq!(brief.source_thread_ids.len(), 2); // s1 and s2 (deduplicated)
        assert_eq!(brief.anchor_episode_ids.len(), 3);
    }

    #[test]
    fn build_user_brief() {
        let episodes = vec![make_episode("ep1", "s1", "Summary 1")];
        let brief = build_user_memory_brief("u1", &episodes).unwrap();
        assert_eq!(brief.scope_type, "user");
        assert_eq!(brief.scope_id, "u1");
    }

    #[test]
    fn empty_episodes_no_brief() {
        let brief = build_resource_memory_brief("u1", "r1", &[]);
        assert!(brief.is_none());
    }

    #[test]
    fn empty_scope_id_no_brief() {
        let episodes = vec![make_episode("ep1", "s1", "Summary")];
        let brief = build_resource_memory_brief("u1", "", &episodes);
        assert!(brief.is_none());
    }

    #[test]
    fn brief_summary_truncated() {
        let long_summary = "A ".repeat(100); // 200 chars
        let episodes = vec![make_episode("ep1", "s1", &long_summary)];
        let brief = build_resource_memory_brief("u1", "r1", &episodes).unwrap();
        assert!(brief.summary.len() <= BRIEF_SUMMARY_TRUNCATE + 3); // 160 + "..."
    }

    #[test]
    fn brief_max_3_summary_parts() {
        let episodes = vec![
            make_episode("ep1", "s1", "Part 1"),
            make_episode("ep2", "s1", "Part 2"),
            make_episode("ep3", "s1", "Part 3"),
            make_episode("ep4", "s1", "Part 4"), // exceeds limit
        ];
        let brief = build_resource_memory_brief("u1", "r1", &episodes).unwrap();
        // Only 3 parts joined in summary
        assert!(brief.summary.contains("Part 1"));
        assert!(brief.summary.contains("Part 2"));
        assert!(brief.summary.contains("Part 3"));
    }
}
