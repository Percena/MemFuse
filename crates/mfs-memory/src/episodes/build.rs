//! Episode building — constructing EpisodeChunks from turns.

use chrono::Utc;

use super::annotate::{
    estimate_context_tags, estimate_emotional_intensity, estimate_emotional_valence,
};
use crate::llm::{LlmAssist, LlmEpisodeSummary, build_episode_summary_prompt, parse_llm_json};
use crate::{ConversationTurn, EpisodeChunk, TurnRole};

/// Build an EpisodeChunk from a slice of turns.
/// Uses LLM summary when available, falls back to simple_summary.
/// Salience defaults to 0.5 (or LLM salience_hint when available).
pub async fn build_episode(
    turns: &[ConversationTurn],
    user_id: &str,
    session_id: &str,
    resource_id: Option<&str>,
    llm: &LlmAssist,
) -> EpisodeChunk {
    assert!(!turns.is_empty(), "episode requires at least one turn");

    let (summary, salience_score) = try_llm_episode_summary(turns, llm)
        .await
        .unwrap_or_else(|| (build_simple_summary(turns), 0.5));

    let start_turn_id = turns[0].turn_id.clone();
    let end_turn_id = turns[turns.len() - 1].turn_id.clone();

    EpisodeChunk {
        episode_id: format!("ep_{}", uuid::Uuid::new_v4()),
        user_id: user_id.to_owned(),
        session_id: session_id.to_owned(),
        resource_id: resource_id.map(|s| s.to_owned()),
        summary,
        salience_score,
        strength_score: 1.0,
        recall_count: 0,
        last_recalled_at: None,
        source_start_turn_id: start_turn_id,
        source_end_turn_id: end_turn_id,
        created_at: now_timestamp(),
        embedding: None,
        emotional_valence: estimate_emotional_valence(turns),
        emotional_intensity: estimate_emotional_intensity(turns),
        context_tags_json: estimate_context_tags(turns),
    }
}

/// Try LLM episode summary generation.
/// Returns (summary_text, salience_hint) when LLM is available and responds.
/// The LLM produces L0 (one-line abstract) + L1 (structured overview) + metadata.
async fn try_llm_episode_summary(
    turns: &[ConversationTurn],
    llm: &LlmAssist,
) -> Option<(String, f64)> {
    if !llm.is_available() {
        return None;
    }

    let turns_text = turns
        .iter()
        .filter(|t| t.role != TurnRole::System && t.role != TurnRole::Tool)
        .map(|t| format!("[{}]: {}", t.role.as_str(), t.content_text))
        .collect::<Vec<_>>()
        .join("\n");

    if turns_text.is_empty() {
        return None;
    }

    let prompt = build_episode_summary_prompt(&turns_text);
    let response = llm.complete(&prompt).await?;

    let parsed: LlmEpisodeSummary = parse_llm_json(&response)?;

    // Combine abstract + overview as the summary field
    let summary = if parsed.overview_text.is_empty() {
        parsed.abstract_text.clone()
    } else {
        format!("{}\n\n{}", parsed.abstract_text, parsed.overview_text)
    };

    // Use LLM salience hint if provided, otherwise default 0.5
    let salience = parsed.salience_hint.unwrap_or(0.5).clamp(0.1, 1.0);

    Some((summary, salience))
}

/// Build an EpisodeChunk from a slice of turns with a pre-provided summary.
/// Used by callers that generate summaries externally.
/// Salience defaults to 0.5, strength defaults to 1.0.
pub fn build_episode_with_summary(
    turns: &[ConversationTurn],
    user_id: &str,
    session_id: &str,
    resource_id: Option<&str>,
    summary: &str,
) -> EpisodeChunk {
    assert!(!turns.is_empty(), "episode requires at least one turn");

    let start_turn_id = turns[0].turn_id.clone();
    let end_turn_id = turns[turns.len() - 1].turn_id.clone();

    EpisodeChunk {
        episode_id: format!("ep_{}", uuid::Uuid::new_v4()),
        user_id: user_id.to_owned(),
        session_id: session_id.to_owned(),
        resource_id: resource_id.map(|s| s.to_owned()),
        summary: summary.to_owned(),
        salience_score: 0.5,
        strength_score: 1.0,
        recall_count: 0,
        last_recalled_at: None,
        source_start_turn_id: start_turn_id,
        source_end_turn_id: end_turn_id,
        created_at: now_timestamp(),
        embedding: None,
        emotional_valence: estimate_emotional_valence(turns),
        emotional_intensity: estimate_emotional_intensity(turns),
        context_tags_json: estimate_context_tags(turns),
    }
}

/// Build a simple summary from turns (fallback when no LLM summarizer).
/// Concatenates role:content pairs, truncated per line (200 chars) and total (500 chars).
pub fn build_simple_summary(turns: &[ConversationTurn]) -> String {
    let mut summary = String::new();

    for t in turns {
        if t.role == TurnRole::System || t.role == TurnRole::Tool {
            continue;
        }
        let mut line = format!("{}: {}", t.role.as_str(), t.content_text);
        if line.len() > 200 {
            let boundary = line.floor_char_boundary(200);
            line = format!("{}...", &line[..boundary]);
        }
        if !summary.is_empty() {
            summary.push_str(" | ");
        }
        summary.push_str(&line);
        if summary.len() > 500 {
            let boundary = summary.floor_char_boundary(500);
            summary = format!("{}...", &summary[..boundary]);
            break;
        }
    }

    summary
}

fn now_timestamp() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TurnRole;

    fn make_turn(seq: i64, role: TurnRole, content: &str, ts: &str) -> ConversationTurn {
        ConversationTurn {
            turn_id: format!("turn-{}", seq),
            turn_seq: seq,
            session_id: "s1".to_owned(),
            user_id: "u1".to_owned(),
            role,
            content_text: content.to_owned(),
            token_count: content.len() / 4,
            created_at: ts.to_owned(),
        }
    }

    #[test]
    fn build_episode() {
        let turns = vec![
            make_turn(1, TurnRole::User, "hello", "2026-01-01T10:00:00Z"),
            make_turn(2, TurnRole::User, "world", "2026-01-01T10:05:00Z"),
        ];
        let ep = build_episode_with_summary(&turns, "u1", "s1", None, "User said hello and world");
        assert_eq!(ep.user_id, "u1");
        assert_eq!(ep.session_id, "s1");
        assert_eq!(ep.salience_score, 0.5);
        assert_eq!(ep.strength_score, 1.0);
        assert_eq!(ep.source_start_turn_id, "turn-1");
        assert_eq!(ep.source_end_turn_id, "turn-2");
    }

    #[test]
    fn build_simple_summary_truncates() {
        let turns = vec![
            make_turn(1, TurnRole::User, &"x".repeat(300), "2026-01-01T10:00:00Z"),
            make_turn(2, TurnRole::System, "system msg", "2026-01-01T10:01:00Z"), // filtered
            make_turn(3, TurnRole::User, &"y".repeat(300), "2026-01-01T10:02:00Z"),
        ];
        let summary = build_simple_summary(&turns);
        assert!(summary.len() <= 510); // 500 + "..."
        assert!(!summary.contains("system")); // system turns filtered
    }
}
