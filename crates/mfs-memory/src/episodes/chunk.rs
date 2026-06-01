//! Episode chunking — grouping turns into episode-sized chunks.

use chrono::DateTime;

use crate::{ConversationTurn, MAX_EPISODE_TOKENS, TIME_GAP_THRESHOLD_SECS};

/// Chunk turns into episodes based on time gap and token budget.
///
/// Time gap threshold: 15 minutes (900 seconds).
/// Token budget: 1200 tokens per chunk.
pub fn chunk_turns(turns: &[ConversationTurn]) -> Vec<Vec<ConversationTurn>> {
    if turns.is_empty() {
        return Vec::new();
    }

    let mut chunks: Vec<Vec<ConversationTurn>> = Vec::new();
    let mut current: Vec<ConversationTurn> = Vec::new();
    let mut current_tokens: usize = 0;

    for (i, t) in turns.iter().enumerate() {
        // Time gap: split if gap > threshold
        if i > 0 {
            let gap = parse_timestamp_secs(&t.created_at)
                .saturating_sub(parse_timestamp_secs(&turns[i - 1].created_at));
            if gap > TIME_GAP_THRESHOLD_SECS && !current.is_empty() {
                chunks.push(current);
                current = Vec::new();
                current_tokens = 0;
            }
        }

        // Token overflow: split if adding this turn exceeds budget
        if current_tokens + t.token_count > MAX_EPISODE_TOKENS && !current.is_empty() {
            chunks.push(current);
            current = Vec::new();
            current_tokens = 0;
        }

        current.push(t.clone());
        current_tokens += t.token_count;
    }

    if !current.is_empty() {
        chunks.push(current);
    }

    chunks
}

/// Parse a timestamp string to seconds (for time gap calculation).
/// Uses chrono for proper RFC 3339 parsing.
fn parse_timestamp_secs(ts: &str) -> i64 {
    DateTime::parse_from_rfc3339(ts.trim())
        .map(|dt| dt.timestamp())
        .unwrap_or(0)
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
    fn chunk_by_time_gap() {
        let turns = vec![
            make_turn(1, TurnRole::User, "hello", "2026-01-01T10:00:00Z"),
            make_turn(2, TurnRole::User, "hi", "2026-01-01T10:05:00Z"), // 5 min gap < 15 min
            make_turn(3, TurnRole::User, "bye", "2026-01-01T10:30:00Z"), // 25 min gap > 15 min → new chunk
        ];
        let chunks = chunk_turns(&turns);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].len(), 2);
        assert_eq!(chunks[1].len(), 1);
    }

    #[test]
    fn chunk_by_token_overflow() {
        let mut turns = Vec::new();
        for i in 1..=5 {
            // Each turn ~300 tokens (1200 char / 4)
            turns.push(make_turn(
                i,
                TurnRole::User,
                &"x".repeat(1200),
                "2026-01-01T10:00:00Z",
            ));
        }
        let chunks = chunk_turns(&turns);
        // Each turn is ~300 tokens, so 4 turns = 1200 → split at 4th
        assert!(chunks.len() >= 2);
    }

    #[test]
    fn chunk_empty_turns() {
        let chunks = chunk_turns(&[]);
        assert!(chunks.is_empty());
    }

    #[test]
    fn chunk_single_turn() {
        let turns = vec![make_turn(
            1,
            TurnRole::User,
            "hello",
            "2026-01-01T10:00:00Z",
        )];
        let chunks = chunk_turns(&turns);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].len(), 1);
    }
}
