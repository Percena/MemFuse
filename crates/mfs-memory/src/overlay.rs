//! Overlay module — unconsolidated turn filtering for context injection.
//!
//! Fetches turns after the consolidation cursor, filters by role
//! and confirmation phrases, caps at 6 entries / 350 tokens.

use crate::{
    ConversationTurn, MAX_OVERLAY_ENTRIES, MAX_OVERLAY_TOKENS, OVERLAY_CANDIDATE_LIMIT,
    OverlayEntry, TurnRole, contains_any,
};

/// Confirmation phrases that indicate an assistant turn is a simple
/// acknowledgment worth reflecting in overlay (bilingual).
/// Intentionally conservative — catches clear confirmations without
/// risking inclusion of substantive assistant replies.
const ASSISTANT_CONFIRMATION_PHRASES: &[&str] = &[
    // English
    "got it",
    "understood",
    "noted",
    "i will remember",
    "i'll remember",
    "i will use",
    // Chinese
    "记住",
    "明白",
    "好的",
    "收到",
    "我会记住",
    "我会使用",
];

/// Build overlay entries from a slice of conversation turns.
///
/// Applies V1 filtering rules:
/// - system and tool turns are always excluded
/// - assistant turns only included when they contain a confirmation phrase
/// - overlay is capped at MAX_OVERLAY_ENTRIES entries or MAX_OVERLAY_TOKENS tokens
///
/// Turns are processed from newest to oldest, but output is ordered
/// chronologically (oldest-first) for natural reading.
pub fn build_overlay_entries(turns: &[ConversationTurn]) -> Vec<OverlayEntry> {
    let mut entries: Vec<OverlayEntry> = Vec::new();
    let mut total_tokens: usize = 0;

    // Process from newest to oldest (reverse iteration)
    for i in (0..turns.len()).rev() {
        let t = &turns[i];

        // Exclude system and tool turns
        if t.role == TurnRole::System || t.role == TurnRole::Tool {
            continue;
        }

        // Assistant turns only included with confirmation phrase
        if t.role == TurnRole::Assistant && !is_assistant_confirmation(&t.content_text) {
            continue;
        }

        // Cap at max entries
        if entries.len() >= MAX_OVERLAY_ENTRIES {
            break;
        }

        // Cap at max tokens (allow first entry even if over budget)
        let token_cost = estimate_entry_tokens(&t.content_text);
        if total_tokens + token_cost > MAX_OVERLAY_TOKENS && !entries.is_empty() {
            break;
        }

        // Prepend to maintain chronological order
        entries.insert(
            0,
            OverlayEntry {
                turn_id: t.turn_id.clone(),
                role: t.role,
                content: t.content_text.clone(),
            },
        );
        total_tokens += token_cost;
    }

    entries
}

/// Check if an assistant turn is a simple acknowledgment.
fn is_assistant_confirmation(text: &str) -> bool {
    contains_any(text, ASSISTANT_CONFIRMATION_PHRASES)
}

/// Estimate token count for a text entry (~4 chars per token + 2 overhead).
fn estimate_entry_tokens(content: &str) -> usize {
    (content.len() / 4) + 2
}

/// Select candidate turns for overlay from a pool.
///
/// Returns up to OVERLAY_CANDIDATE_LIMIT turns for further filtering.
pub fn select_overlay_candidates(
    turns: &[ConversationTurn],
    after_seq: i64,
) -> Vec<ConversationTurn> {
    turns
        .iter()
        .filter(|t| t.turn_seq > after_seq)
        .take(OVERLAY_CANDIDATE_LIMIT)
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TurnRole;

    fn make_turn(seq: i64, role: TurnRole, content: &str) -> ConversationTurn {
        ConversationTurn {
            turn_id: format!("turn-{}", seq),
            turn_seq: seq,
            session_id: "s1".to_owned(),
            user_id: "u1".to_owned(),
            role,
            content_text: content.to_owned(),
            token_count: content.len() / 4,
            created_at: "2026-01-01T00:00:00Z".to_owned(),
        }
    }

    #[test]
    fn filters_system_and_tool_turns() {
        let turns = vec![
            make_turn(1, TurnRole::User, "hello"),
            make_turn(2, TurnRole::System, "system message"),
            make_turn(3, TurnRole::Tool, "tool output"),
            make_turn(4, TurnRole::User, "another user message"),
        ];
        let entries = build_overlay_entries(&turns);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].role, TurnRole::User);
        assert_eq!(entries[1].role, TurnRole::User);
    }

    #[test]
    fn assistant_confirmation_included() {
        let turns = vec![
            make_turn(1, TurnRole::User, "I live in Tokyo"),
            make_turn(2, TurnRole::Assistant, "Got it, I'll remember that"),
            make_turn(3, TurnRole::Assistant, "Here's a detailed analysis..."), // no confirmation
        ];
        let entries = build_overlay_entries(&turns);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].role, TurnRole::User);
        assert_eq!(entries[1].role, TurnRole::Assistant);
    }

    #[test]
    fn chinese_confirmation_phrases() {
        assert!(is_assistant_confirmation("明白，我会记住的"));
        assert!(is_assistant_confirmation("收到"));
        assert!(!is_assistant_confirmation("这是一个详细的分析"));
    }

    #[test]
    fn caps_at_max_entries() {
        let mut turns = Vec::new();
        for i in 1..=10 {
            let msg = format!("message {}", i);
            turns.push(make_turn(i, TurnRole::User, &msg));
        }
        let entries = build_overlay_entries(&turns);
        assert!(entries.len() <= MAX_OVERLAY_ENTRIES);
    }

    #[test]
    fn empty_turns_produce_empty_overlay() {
        let entries = build_overlay_entries(&[]);
        assert!(entries.is_empty());
    }

    #[test]
    fn select_overlay_candidates_filters_by_seq() {
        let turns = vec![
            make_turn(1, TurnRole::User, "old"),
            make_turn(5, TurnRole::User, "mid"),
            make_turn(10, TurnRole::User, "new"),
        ];
        let candidates = select_overlay_candidates(&turns, 2);
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].turn_seq, 5);
    }
}
