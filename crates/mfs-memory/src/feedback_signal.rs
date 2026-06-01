//! Feedback signal detection for T2H (Trajectory-to-Heuristics) pipeline.
//!
//! Dual-track strategy (roadmap §5.1):
//! 1. **Deterministic rules** (always available): keyword matching for negation,
//!    preference declaration, tradeoff expressions, and implicit rewrite signals.
//! 2. **LLM classification** (available when LlmAssist is active): catches implicit
//!    preference signals that deterministic rules miss.
//!
//! Signal灯塔 philosophy: deterministic rules are the baseline, LLM adds directional
//! signals when available. The system never blocks on an LLM call.

use crate::heuristics::SignalType;
use crate::llm::LlmAssist;
use crate::{ConversationTurn, TurnRole};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

// ─── Privacy stripping (roadmap §10.8) ──────────────────────────────────

/// Strip `<private>...</private>` tags from content before signal detection.
/// Mirrors the TypeScript SDK's `stripPrivate()` function to ensure consistent
/// privacy handling across both SDK and Rust backend (§10.8).
///
/// Content inside `<private>` tags is completely removed — it may contain
/// credentials, API keys, or sensitive business context that should never
/// enter the T2H pipeline.
pub fn strip_private(content: &str) -> String {
    let mut result = String::with_capacity(content.len());
    let mut pos = 0;
    while pos < content.len() {
        if content[pos..].starts_with("<private>") {
            if let Some(end_offset) = content[pos..].find("</private>") {
                pos += end_offset + "</private>".len();
                continue;
            }
            // No closing tag — treat rest as private and stop
            break;
        }
        result.push(content[pos..].chars().next().unwrap_or('\0'));
        pos += content[pos..].chars().next().map_or(1, |c| c.len_utf8());
    }
    result
}

// ─── Deterministic negation keywords (roadmap §5.1) ────────────────────

/// English negation keywords indicating the user is rejecting an agent proposal.
pub const NEGATION_KEYWORDS_EN: &[&str] = &[
    "don't",
    "dont",
    "do not",
    "no",
    "not",
    "never",
    "avoid",
    "instead",
    "stop",
    "don't want",
    "dont want",
    "rather",
    "i prefer",
    "prefer",
    "i'd rather",
    "i would rather",
    "not like",
    "dislike",
];

/// Chinese negation keywords.
/// Note: single-character keywords like 别 are excluded to avoid false matches
/// inside compound words (e.g., 特别 → "别" false-match).
pub const NEGATION_KEYWORDS_ZH: &[&str] = &[
    "不要",
    "不用",
    "不行",
    "不好",
    "避免",
    "换成",
    "别用",
    "停止",
    "不想",
    "更喜欢",
    "偏好",
    "宁愿",
    "不喜欢",
];

/// Preference declaration keywords indicating an explicit preference statement.
pub const PREFERENCE_KEYWORDS_EN: &[&str] = &[
    "always",
    "i always",
    "from now on",
    "i prefer",
    "prefer to",
    "my style",
    "i like",
    "i typically",
    "i usually",
    "standard approach",
    "my approach",
    "my convention",
];

pub const PREFERENCE_KEYWORDS_ZH: &[&str] = &[
    "以后",
    "总是",
    "一直",
    "偏好",
    "习惯",
    "我习惯",
    "i习惯",
    "我的风格",
    "我喜欢",
    "通常",
    "标准做法",
    "我的做法",
];

/// Tradeoff expression keywords indicating the user is making a conscious tradeoff.
const TRADEOFF_KEYWORDS_EN: &[&str] = &[
    "although",
    "even though",
    "trade-off",
    "tradeoff",
    "despite",
    "while",
    "on the other hand",
    "but for",
    "at the cost of",
];

const TRADEOFF_KEYWORDS_ZH: &[&str] = &["虽然", "尽管", "权衡", "代价是", "另一方面"];

/// Correction keywords indicating user is correcting a specific assistant output (§6.2.2).
const CORRECTION_KEYWORDS_EN: &[&str] = &[
    "actually",
    "no, use",
    "change that to",
    "wrong",
    "incorrect",
    "that's not right",
    "fix this",
    "should be",
    "not like that",
    "i meant",
    "let me correct",
    "that's wrong",
];

const CORRECTION_KEYWORDS_ZH: &[&str] = &[
    "不对",
    "改成",
    "应该是",
    "错了",
    "换成",
    "不是这样",
    "修正",
    "纠正",
    "我说的是",
    "不是那个",
];

/// Workflow keywords indicating user is describing a step-by-step process (§6.2.3).
const WORKFLOW_KEYWORDS_EN: &[&str] = &[
    "first",
    "then",
    "step 1",
    "after that",
    "next step",
    "finally",
    "lastly",
    "how to",
    "workflow",
];

const WORKFLOW_KEYWORDS_ZH: &[&str] = &[
    "先",
    "再",
    "第一步",
    "然后",
    "接下来",
    "最后",
    "流程",
    "怎么做",
    "步骤",
];

/// Minimum content length for WorkflowSignal detection (§6.2.3).
const WORKFLOW_MIN_LENGTH: usize = 100;

/// Code-block pattern for implicit rewrite detection (markdown fenced code blocks).
const CODE_BLOCK_PATTERN: &str = "```";

// ─── Detected feedback signal ──────────────────────────────────────────

/// A detected feedback signal from user-agent interaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedbackSignal {
    /// The type of feedback signal detected.
    pub signal_type: SignalType,
    /// Confidence of the detection (1.0 for deterministic rules, 0.0-1.0 for LLM).
    pub confidence: f64,
    /// Summary of the interaction context.
    pub context_summary: String,
    /// What the user said or did (the signal itself).
    pub user_reaction: String,
    /// What the agent proposed before the user's reaction (if available).
    pub agent_proposal: Option<String>,
    /// Whether this was detected by deterministic rules or LLM classification.
    pub detection_source: DetectionSource,
    /// LLM-extracted tags for this signal (roadmap §10.1).
    /// When non-empty, these are preferred over keyword-derived tags during
    /// instance storage. Empty for deterministic detections; populated by
    /// LLM classification when available.
    #[serde(default)]
    pub llm_tags: Vec<String>,
}

/// How the signal was detected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DetectionSource {
    Deterministic,
    LlmClassified,
}

// ─── Deterministic signal detection ────────────────────────────────────

/// Detect feedback signals from conversation turns using deterministic rules.
///
/// Scans user messages for negation keywords, preference declarations,
/// tradeoff expressions, and implicit rewrite signals (code blocks within
/// 2 turns after an assistant message).
///
/// Returns all detected signals with confidence = 1.0.
pub fn detect_signals_deterministic(turns: &[ConversationTurn]) -> Vec<FeedbackSignal> {
    let mut signals = Vec::new();

    for (i, turn) in turns.iter().enumerate() {
        if turn.role != TurnRole::User {
            continue;
        }

        // Strip <private> tags before signal detection (roadmap §10.8)
        let stripped = strip_private(&turn.content_text);
        let text = stripped.to_lowercase();

        // Check negation keywords
        if contains_any_keyword(&text, NEGATION_KEYWORDS_EN, NEGATION_KEYWORDS_ZH) {
            // Determine if this is a preference declaration or explicit negation
            // Preference declarations that also contain negation framing
            if contains_any_keyword(&text, PREFERENCE_KEYWORDS_EN, PREFERENCE_KEYWORDS_ZH) {
                signals.push(FeedbackSignal {
                    signal_type: SignalType::PreferenceDeclaration,
                    confidence: 1.0,
                    context_summary: extract_context(turns, i),
                    user_reaction: truncate_reaction(&stripped, 200),
                    agent_proposal: find_agent_proposal(turns, i),
                    detection_source: DetectionSource::Deterministic,
                    llm_tags: Vec::new(),
                });
            } else if contains_any_keyword(&text, TRADEOFF_KEYWORDS_EN, TRADEOFF_KEYWORDS_ZH) {
                signals.push(FeedbackSignal {
                    signal_type: SignalType::TradeoffDecision,
                    confidence: 1.0,
                    context_summary: extract_context(turns, i),
                    user_reaction: truncate_reaction(&stripped, 200),
                    agent_proposal: find_agent_proposal(turns, i),
                    detection_source: DetectionSource::Deterministic,
                    llm_tags: Vec::new(),
                });
            } else {
                signals.push(FeedbackSignal {
                    signal_type: SignalType::ExplicitNegation,
                    confidence: 1.0,
                    context_summary: extract_context(turns, i),
                    user_reaction: truncate_reaction(&stripped, 200),
                    agent_proposal: find_agent_proposal(turns, i),
                    detection_source: DetectionSource::Deterministic,
                    llm_tags: Vec::new(),
                });
            }
            continue; // Don't double-detect the same turn
        }

        // Check standalone preference declarations (no negation framing)
        if contains_any_keyword(&text, PREFERENCE_KEYWORDS_EN, PREFERENCE_KEYWORDS_ZH) {
            signals.push(FeedbackSignal {
                signal_type: SignalType::PreferenceDeclaration,
                confidence: 1.0,
                context_summary: extract_context(turns, i),
                user_reaction: truncate_reaction(&stripped, 200),
                agent_proposal: find_agent_proposal(turns, i),
                detection_source: DetectionSource::Deterministic,
                llm_tags: Vec::new(),
            });
            continue;
        }

        // Check tradeoff expressions
        if contains_any_keyword(&text, TRADEOFF_KEYWORDS_EN, TRADEOFF_KEYWORDS_ZH) {
            signals.push(FeedbackSignal {
                signal_type: SignalType::TradeoffDecision,
                confidence: 1.0,
                context_summary: extract_context(turns, i),
                user_reaction: truncate_reaction(&stripped, 200),
                agent_proposal: find_agent_proposal(turns, i),
                detection_source: DetectionSource::Deterministic,
                llm_tags: Vec::new(),
            });
            continue;
        }

        // Check implicit rewrite: user sent code block within 2 turns after assistant
        if contains_code_block(&turn.content_text) {
            if let Some(agent_text) = find_recent_assistant_proposal(turns, i, 2) {
                // Only flag as implicit negation if the user's code block is substantial
                // (more than just a one-liner confirmation)
                if code_block_line_count(&turn.content_text) >= 3 {
                    signals.push(FeedbackSignal {
                        signal_type: SignalType::ImplicitNegation,
                        confidence: 0.7, // Lower confidence — implicit signals are less certain
                        context_summary: extract_context(turns, i),
                        user_reaction: truncate_reaction(&stripped, 200),
                        agent_proposal: Some(truncate_reaction(&agent_text, 200)),
                        detection_source: DetectionSource::Deterministic,
                        llm_tags: Vec::new(),
                    });
                }
            }
        }

        // ── Correction signal (§6.2.2): user corrects a specific assistant output ──
        // Only detected within 1 turn after assistant (tighter window than implicit negation)
        if contains_any_keyword(&text, CORRECTION_KEYWORDS_EN, CORRECTION_KEYWORDS_ZH)
            && find_recent_assistant_proposal(turns, i, 1).is_some()
        {
            signals.push(FeedbackSignal {
                signal_type: SignalType::CorrectionSignal,
                confidence: 0.90,
                context_summary: extract_context(turns, i),
                user_reaction: truncate_reaction(&stripped, 200),
                agent_proposal: find_agent_proposal(turns, i),
                detection_source: DetectionSource::Deterministic,
                llm_tags: Vec::new(),
            });
        }

        // ── Workflow signal (§6.2.3): user describes a step-by-step process ──
        if contains_any_keyword(&text, WORKFLOW_KEYWORDS_EN, WORKFLOW_KEYWORDS_ZH) {
            // Only detect workflow signals for substantial content (>100 chars)
            if stripped.len() >= WORKFLOW_MIN_LENGTH {
                signals.push(FeedbackSignal {
                    signal_type: SignalType::WorkflowSignal,
                    confidence: 0.75,
                    context_summary: extract_context(turns, i),
                    user_reaction: truncate_reaction(&stripped, 200),
                    agent_proposal: find_agent_proposal(turns, i),
                    detection_source: DetectionSource::Deterministic,
                    llm_tags: Vec::new(),
                });
            }
        }
    }

    signals
}

// ─── LLM-based signal classification ──────────────────────────────────

/// LLM classification result for implicit preference signals.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct LlmSignalClassification {
    turn_id: String,
    signal_type: String,
    context: String,
    reaction: String,
    confidence: f64,
    /// LLM-extracted tags using allowed keys (roadmap §10.1).
    #[serde(default)]
    tags: Vec<String>,
}

/// Detect feedback signals using LLM classification for turns that
/// deterministic rules did not flag.
///
/// Per roadmap §5.1:
/// - Only processes user messages that deterministic rules missed
/// - Confidence < 0.6 signals are discarded (noise filter)
/// - Implicit signals (implicit_negation) don't count as formal evidence
///   unless later validated by explicit feedback
pub async fn detect_signals_llm(
    turns: &[ConversationTurn],
    deterministic_signals: &[FeedbackSignal],
    llm: &LlmAssist,
) -> Vec<FeedbackSignal> {
    if !llm.is_available() {
        return Vec::new();
    }

    // Collect user turns that weren't already flagged by deterministic rules
    let flagged_turn_ids: std::collections::HashSet<String> = deterministic_signals
        .iter()
        .map(|s| s.user_reaction.clone()) // Match by user_reaction content, not turn_id
        .collect();

    let unflagged_user_turns: Vec<(usize, &ConversationTurn)> = turns
        .iter()
        .enumerate()
        .filter(|(_, t)| {
            t.role == TurnRole::User
                && !flagged_turn_ids.contains(&t.content_text)
                && t.content_text.len() >= 20 // Skip very short messages
        })
        .collect();

    // Build prompt text from the batch (we need the turns, not references)
    let batch_turns: Vec<ConversationTurn> = unflagged_user_turns
        .iter()
        .take(5)
        .map(|(_, t)| (*t).clone())
        .collect();

    if batch_turns.is_empty() {
        return Vec::new();
    }

    let prompt = build_signal_classification_prompt(&batch_turns);
    let response = llm.complete(&prompt).await;

    match response {
        Some(text) => parse_llm_signal_classifications(&text, turns)
            .into_iter()
            .filter(|s| s.confidence >= 0.6)
            .collect(),
        None => Vec::new(), // LLM unavailable — deterministic fallback only
    }
}

// ─── Combined detection ───────────────────────────────────────────────

/// Detect feedback signals using both deterministic rules and LLM classification.
///
/// Deterministic rules are always applied. LLM classification is applied only
/// to turns that deterministic rules missed, and only when LlmAssist is available.
///
/// Per roadmap §5.1 Phase 2 scope:
/// Only explicit_negation, preference_declaration, and tradeoff_decision
/// count as formal evidence. implicit_negation is recorded as instance
/// but not as formal evidence unless later validated.
pub async fn detect_feedback_signals(
    turns: &[ConversationTurn],
    llm: &LlmAssist,
) -> Vec<FeedbackSignal> {
    let deterministic = detect_signals_deterministic(turns);
    let llm_signals = detect_signals_llm(turns, &deterministic, llm).await;
    let mut combined = deterministic;
    combined.extend(llm_signals);
    combined
}

// ─── Helper functions ──────────────────────────────────────────────────

/// Check if text contains any of the given keyword lists (bilingual).
/// English keywords use word-boundary matching via `contains_any`;
/// Chinese keywords use plain substring matching (no spaces in Chinese text).
fn contains_any_keyword(text: &str, en_keywords: &[&str], zh_keywords: &[&str]) -> bool {
    // Word-boundary matching for English keywords (prevents "first" matching "firstly")
    if crate::contains_any(text, en_keywords) {
        return true;
    }
    // Chinese keywords don't need word boundaries (no spaces in Chinese)
    let lower = text.to_lowercase();
    for kw in zh_keywords {
        if lower.contains(kw) {
            return true;
        }
    }
    false
}

/// Check if text contains a markdown fenced code block.
fn contains_code_block(text: &str) -> bool {
    text.contains(CODE_BLOCK_PATTERN)
}

/// Count lines in code blocks within text.
fn code_block_line_count(text: &str) -> usize {
    let mut in_block = false;
    let mut count = 0;
    for line in text.lines() {
        if line.starts_with(CODE_BLOCK_PATTERN) {
            in_block = !in_block;
            continue;
        }
        if in_block {
            count += 1;
        }
    }
    count
}

/// Find the assistant message immediately preceding a user turn at index `i`.
fn find_agent_proposal(turns: &[ConversationTurn], i: usize) -> Option<String> {
    if i > 0 && turns[i - 1].role == TurnRole::Assistant {
        Some(truncate_reaction(&turns[i - 1].content_text, 150))
    } else {
        None
    }
}

/// Find the most recent assistant message within `window` turns before index `i`.
fn find_recent_assistant_proposal(
    turns: &[ConversationTurn],
    i: usize,
    window: usize,
) -> Option<String> {
    let start = i.saturating_sub(window);
    for j in (start..i).rev() {
        if turns[j].role == TurnRole::Assistant {
            return Some(turns[j].content_text.clone());
        }
    }
    None
}

/// Extract a brief context summary from surrounding turns.
fn extract_context(turns: &[ConversationTurn], i: usize) -> String {
    let start = i.saturating_sub(2);
    let end = (i + 2).min(turns.len());
    let context_lines: Vec<String> = turns[start..end]
        .iter()
        .map(|t| {
            format!(
                "{}: {}",
                t.role.as_str(),
                truncate_content(&t.content_text, 80)
            )
        })
        .collect();
    context_lines.join("\n")
}

/// Truncate a reaction string to `max_chars` characters (UTF-8 safe).
///
/// Note: the limit is in **character count** (not byte count). For CJK text,
/// this allows up to ~3× the byte budget compared to a byte-based limit.
/// Ensure downstream storage/API fields can accommodate the resulting byte length.
fn truncate_reaction(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        text.to_owned()
    } else {
        let truncated: String = text.chars().take(max_chars).collect();
        // Find last complete word boundary
        if let Some(last_space) = truncated.rfind(' ') {
            format!("{}…", &truncated[..last_space])
        } else {
            format!("{}…", truncated)
        }
    }
}

/// Truncate content for context display.
fn truncate_content(text: &str, max_len: usize) -> String {
    truncate_reaction(text, max_len)
}

// ─── LLM prompt templates ─────────────────────────────────────────────

/// Build the LLM prompt for classifying implicit preference signals
/// in user messages that deterministic rules missed.
pub fn build_signal_classification_prompt(turns: &[ConversationTurn]) -> String {
    let turns_text = turns
        .iter()
        .map(|t| {
            format!(
                "{} [{}]: {}",
                t.role.as_str(),
                t.turn_id,
                truncate_content(&t.content_text, 150)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        r#"Analyze these user messages for implicit preference signals that keyword matching would miss.

Context messages:
{turns_text}

For each message that contains an implicit preference signal (not a simple factual statement),
output a classification. Skip messages that are purely factual, informational, or neutral.

Signal types:
- explicit_negation — user clearly rejects an approach ("don't use X", "avoid Y")
- implicit_negation — user subtly redirects without explicit rejection (rewrites code, changes approach)
- preference_declaration — user states how they like things done ("I prefer X", "always use Y")
- tradeoff_decision — user makes a deliberate choice with acknowledged tradeoffs ("although X is faster, I choose Y for readability")
- correction_signal — user corrects a specific assistant output ("actually, use X instead", "that's wrong")
- workflow_signal — user describes a step-by-step process ("first build, then test, finally deploy")
- repetition_signal — user repeats a similar instruction across sessions, suggesting the memory system didn't capture a key preference

Output JSON array only:
[
  {{
    "turn_id": "the turn_id from the input",
    "signal_type": "explicit_negation|implicit_negation|preference_declaration|tradeoff_decision|correction_signal|workflow_signal|repetition_signal",
    "context": "brief description of what was happening",
    "reaction": "what the user expressed",
    "confidence": 0.0-1.0,
    "tags": ["key:value tags using allowed keys: domain, phase, pressure, language, topic"]
  }}
]

If no messages contain preference signals, output: []

Confidence guidelines:
- 0.9-1.0: Very clear signal (explicit rejection, strong preference statement)
- 0.7-0.8: Reasonably clear (implicit preference visible from context)
- 0.5-0.6: Weak signal (slight hint, might be noise)
- Below 0.5: Not a preference signal, do not output"#
    )
}

/// Parse LLM signal classification response into FeedbackSignal structs.
fn parse_llm_signal_classifications(
    response: &str,
    turns: &[ConversationTurn],
) -> Vec<FeedbackSignal> {
    let cleaned = crate::llm::strip_code_fences(response);
    let parsed: Vec<LlmSignalClassification> = match serde_json::from_str(cleaned) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    let turn_map: std::collections::HashMap<&str, &ConversationTurn> =
        turns.iter().map(|t| (t.turn_id.as_str(), t)).collect();

    parsed
        .into_iter()
        .filter_map(|cls| {
            let signal_type = match cls.signal_type.as_str() {
                "explicit_negation" => SignalType::ExplicitNegation,
                "implicit_negation" => SignalType::ImplicitNegation,
                "preference_declaration" => SignalType::PreferenceDeclaration,
                "tradeoff_decision" => SignalType::TradeoffDecision,
                "correction_signal" => SignalType::CorrectionSignal,
                "workflow_signal" => SignalType::WorkflowSignal,
                "repetition_signal" => SignalType::RepetitionSignal,
                _ => return None,
            };

            let _turn = turn_map.get(cls.turn_id.as_str())?;
            let turn_idx = turns.iter().position(|t| t.turn_id == cls.turn_id)?;

            Some(FeedbackSignal {
                signal_type,
                confidence: cls.confidence,
                context_summary: cls.context.clone(),
                user_reaction: cls.reaction.clone(),
                agent_proposal: find_agent_proposal(turns, turn_idx),
                detection_source: DetectionSource::LlmClassified,
                llm_tags: cls.tags.clone(),
            })
        })
        .collect()
}

// ─── Repetition detection (cross-session) ───────────────────────────────

/// Jaccard similarity between two sets of words.
/// Returns 0.0 if both sets are empty.
pub fn jaccard_similarity(a: &str, b: &str) -> f64 {
    let set_a: HashSet<&str> = a.split_whitespace().collect();
    let set_b: HashSet<&str> = b.split_whitespace().collect();

    if set_a.is_empty() && set_b.is_empty() {
        return 0.0;
    }

    let intersection = set_a.intersection(&set_b).count();
    let union = set_a.union(&set_b).count();

    if union == 0 {
        0.0
    } else {
        intersection as f64 / union as f64
    }
}

/// Jaccard similarity threshold for repetition detection.
const REPETITION_JACCARD_THRESHOLD: f64 = 0.7;

/// Detect repetition signals between current session turns and recent turns from other sessions.
///
/// For each user turn in `current_turns`, checks if any turn in `recent_turns`
/// has Jaccard similarity above 0.7. Returns one signal per matching current turn.
pub fn detect_repetition_signals(
    current_turns: &[ConversationTurn],
    recent_turns: &[ConversationTurn],
) -> Vec<FeedbackSignal> {
    if recent_turns.is_empty() {
        return Vec::new();
    }

    let mut signals = Vec::new();

    for (i, current) in current_turns.iter().enumerate() {
        if current.role != TurnRole::User {
            continue;
        }

        let current_lower = current.content_text.to_lowercase();

        for recent in recent_turns {
            let recent_lower = recent.content_text.to_lowercase();
            let similarity = jaccard_similarity(&current_lower, &recent_lower);

            if similarity >= REPETITION_JACCARD_THRESHOLD {
                signals.push(FeedbackSignal {
                    signal_type: SignalType::RepetitionSignal,
                    confidence: 0.85,
                    context_summary: format!(
                        "Repetition detected: current turn '{}' similar to recent turn '{}' (Jaccard: {:.2})",
                        truncate_content(&current.content_text, 50),
                        truncate_content(&recent.content_text, 50),
                        similarity,
                    ),
                    user_reaction: truncate_reaction(&current.content_text, 200),
                    agent_proposal: find_agent_proposal(current_turns, i),
                    detection_source: DetectionSource::Deterministic,
                    llm_tags: Vec::new(),
                });
                break; // One signal per current turn
            }
        }
    }

    signals
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_turn(id: &str, seq: i64, role: TurnRole, content: &str) -> ConversationTurn {
        ConversationTurn {
            turn_id: id.to_owned(),
            turn_seq: seq,
            session_id: "sess_test".to_owned(),
            user_id: "u1".to_owned(),
            role,
            content_text: content.to_owned(),
            token_count: content.len(),
            created_at: "2026-04-29T10:00:00Z".to_owned(),
        }
    }

    #[test]
    fn detect_explicit_negation_en() {
        let turns = [
            make_turn(
                "t1",
                1,
                TurnRole::Assistant,
                "I'll use anyhow for error handling",
            ),
            make_turn(
                "t2",
                2,
                TurnRole::User,
                "Don't use anyhow, I want custom error enums",
            ),
        ];
        let signals = detect_signals_deterministic(&turns);
        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0].signal_type, SignalType::ExplicitNegation);
        assert_eq!(signals[0].confidence, 1.0);
        assert_eq!(signals[0].detection_source, DetectionSource::Deterministic);
        assert!(signals[0].agent_proposal.is_some());
    }

    #[test]
    fn detect_explicit_negation_zh() {
        let turns = [
            make_turn("t1", 1, TurnRole::Assistant, "我会用 anyhow 处理错误"),
            make_turn(
                "t2",
                2,
                TurnRole::User,
                "不要用 anyhow，我想要自定义错误枚举",
            ),
        ];
        let signals = detect_signals_deterministic(&turns);
        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0].signal_type, SignalType::ExplicitNegation);
    }

    #[test]
    fn detect_preference_declaration() {
        let turns = [make_turn(
            "t1",
            1,
            TurnRole::User,
            "I always prefer concrete types over trait objects in Rust",
        )];
        let signals = detect_signals_deterministic(&turns);
        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0].signal_type, SignalType::PreferenceDeclaration);
    }

    #[test]
    fn detect_tradeoff_decision() {
        let turns = [
            make_turn(
                "t1",
                1,
                TurnRole::Assistant,
                "I suggest using regex for parsing — it's faster",
            ),
            make_turn(
                "t2",
                2,
                TurnRole::User,
                "Although regex is faster, I choose manual parsing for readability",
            ),
        ];
        let signals = detect_signals_deterministic(&turns);
        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0].signal_type, SignalType::TradeoffDecision);
    }

    #[test]
    fn detect_implicit_rewrite() {
        let turns = [
            make_turn(
                "t1",
                1,
                TurnRole::Assistant,
                "Here's the function:\n```rust\nfn process(data: &str) -> Result<()> {\n  anyhow::Ok(())\n}\n```",
            ),
            make_turn(
                "t2",
                2,
                TurnRole::User,
                "I rewrote it:\n```rust\nfn process(data: &str) -> Result<(), AppError> {\n  Ok(data.parse()?)\n}\n```",
            ),
        ];
        let signals = detect_signals_deterministic(&turns);
        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0].signal_type, SignalType::ImplicitNegation);
        assert_eq!(signals[0].confidence, 0.7); // Lower confidence for implicit signals
    }

    #[test]
    fn skip_short_code_block() {
        // One-liner code block should not trigger implicit negation
        let turns = [
            make_turn(
                "t1",
                1,
                TurnRole::Assistant,
                "Use this: `println!(\"hello\")`",
            ),
            make_turn("t2", 2, TurnRole::User, "```rust\nprintln!(\"world\")\n```"),
        ];
        let signals = detect_signals_deterministic(&turns);
        assert!(
            signals.is_empty(),
            "One-liner code block should not trigger implicit negation"
        );
    }

    #[test]
    fn no_signal_for_neutral_message() {
        let turns = [make_turn(
            "t1",
            1,
            TurnRole::User,
            "Can you explain how this works?",
        )];
        let signals = detect_signals_deterministic(&turns);
        assert!(signals.is_empty());
    }

    #[test]
    fn preference_over_negation_when_both() {
        // "I prefer X instead of Y" — should be preference_declaration, not explicit_negation
        let turns = [
            make_turn(
                "t1",
                1,
                TurnRole::Assistant,
                "I'll use interfaces for the abstraction",
            ),
            make_turn(
                "t2",
                2,
                TurnRole::User,
                "I prefer concrete types instead of interfaces",
            ),
        ];
        let signals = detect_signals_deterministic(&turns);
        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0].signal_type, SignalType::PreferenceDeclaration);
    }

    #[test]
    fn code_block_line_count() {
        let text = "```rust\nfn main() {\n  println!(\"hi\");\n}\n```";
        assert_eq!(super::code_block_line_count(text), 3);
    }

    #[test]
    fn contains_keyword_bilingual() {
        assert!(super::contains_any_keyword(
            "don't use this",
            super::NEGATION_KEYWORDS_EN,
            super::NEGATION_KEYWORDS_ZH
        ));
        assert!(super::contains_any_keyword(
            "不要用这个",
            super::NEGATION_KEYWORDS_EN,
            super::NEGATION_KEYWORDS_ZH
        ));
        assert!(!super::contains_any_keyword(
            "this is fine",
            super::NEGATION_KEYWORDS_EN,
            super::NEGATION_KEYWORDS_ZH
        ));
    }

    #[test]
    fn strip_private_removes_private_blocks() {
        let input = "I prefer <private>secret key=abc123</private> Rust for CLI tools";
        let stripped = super::strip_private(input);
        assert_eq!(stripped, "I prefer  Rust for CLI tools");
        // No sensitive content leaked
        assert!(!stripped.contains("secret"));
        assert!(!stripped.contains("abc123"));
    }

    #[test]
    fn strip_private_no_tags_unchanged() {
        let input = "I prefer Rust for CLI tools";
        assert_eq!(super::strip_private(input), input);
    }

    #[test]
    fn strip_private_unclosed_tag_stops() {
        let input = "Hello <private>this is sensitive and never closed";
        let stripped = super::strip_private(input);
        assert_eq!(stripped, "Hello ");
    }

    // ── CorrectionSignal tests (§6.2.2) ──

    #[test]
    fn detect_correction_signal_english() {
        let turns = [
            make_turn(
                "t1",
                1,
                TurnRole::Assistant,
                "The code uses anyhow for error handling",
            ),
            make_turn(
                "t2",
                2,
                TurnRole::User,
                "Actually, change that to use custom error types",
            ),
        ];
        let signals = detect_signals_deterministic(&turns);
        assert!(
            signals
                .iter()
                .any(|s| s.signal_type == SignalType::CorrectionSignal),
            "Expected CorrectionSignal"
        );
        let correction = signals
            .iter()
            .find(|s| s.signal_type == SignalType::CorrectionSignal)
            .unwrap();
        assert_eq!(correction.confidence, 0.90);
        assert!(correction.agent_proposal.is_some());
    }

    #[test]
    fn detect_correction_signal_chinese() {
        let turns = [
            make_turn("t1", 1, TurnRole::Assistant, "代码用了 anyhow 处理错误"),
            make_turn("t2", 2, TurnRole::User, "不对，改成自定义错误枚举"),
        ];
        let signals = detect_signals_deterministic(&turns);
        assert!(
            signals
                .iter()
                .any(|s| s.signal_type == SignalType::CorrectionSignal),
            "Expected CorrectionSignal"
        );
    }

    #[test]
    fn correction_without_assistant_not_detected() {
        // Correction keywords without recent assistant proposal should not trigger
        let turns = [make_turn(
            "t1",
            1,
            TurnRole::User,
            "Actually, that's not how it works",
        )];
        let signals = detect_signals_deterministic(&turns);
        // "Actually" is a correction keyword, but no assistant within 1 turn
        // "that's not right" / "actually" could also match negation keywords
        // Only CorrectionSignal should not appear since there's no assistant proposal
        assert!(
            !signals
                .iter()
                .any(|s| s.signal_type == SignalType::CorrectionSignal)
        );
    }

    // ── WorkflowSignal tests (§6.2.3) ──

    #[test]
    fn detect_workflow_signal_english() {
        let long_workflow = "First, you need to set up the environment, then install the dependencies, and after that you can run the build command to compile the project. Finally, deploy to staging.";
        assert!(long_workflow.len() >= super::WORKFLOW_MIN_LENGTH);
        let turns = [make_turn("t1", 1, TurnRole::User, long_workflow)];
        let signals = detect_signals_deterministic(&turns);
        assert!(
            signals
                .iter()
                .any(|s| s.signal_type == SignalType::WorkflowSignal),
            "Expected WorkflowSignal"
        );
        let workflow = signals
            .iter()
            .find(|s| s.signal_type == SignalType::WorkflowSignal)
            .unwrap();
        assert_eq!(workflow.confidence, 0.75);
    }

    #[test]
    fn detect_workflow_signal_chinese() {
        let long_workflow = "先搭建开发环境，再安装所有依赖包，然后执行构建命令编译项目代码。接下来配置 CI 流水线，最后部署到生产环境。这个流程需要特别注意版本兼容性。";
        assert!(long_workflow.len() >= super::WORKFLOW_MIN_LENGTH);
        let turns = [make_turn("t1", 1, TurnRole::User, long_workflow)];
        let signals = detect_signals_deterministic(&turns);
        assert!(
            signals
                .iter()
                .any(|s| s.signal_type == SignalType::WorkflowSignal),
            "Expected WorkflowSignal"
        );
    }

    #[test]
    fn short_workflow_not_detected() {
        // Short workflow expressions (<100 chars) should not trigger WorkflowSignal
        let short_workflow = "First build, then test";
        assert!(short_workflow.len() < super::WORKFLOW_MIN_LENGTH);
        let turns = [make_turn("t1", 1, TurnRole::User, short_workflow)];
        let signals = detect_signals_deterministic(&turns);
        assert!(
            !signals
                .iter()
                .any(|s| s.signal_type == SignalType::WorkflowSignal)
        );
    }

    #[test]
    fn signal_detection_strips_private_content() {
        let turns = [
            make_turn("t1", 1, TurnRole::Assistant, "Suggested approach"),
            make_turn(
                "t2",
                2,
                TurnRole::User,
                "Don't use <private>internal-api-key</private> that approach",
            ),
        ];
        let signals = detect_signals_deterministic(&turns);
        assert_eq!(signals.len(), 1);
        // Private content should not appear in user_reaction
        assert!(!signals[0].user_reaction.contains("internal-api-key"));
    }

    // ── RepetitionSignal tests ──

    #[test]
    fn detect_repetition_similar_content() {
        let current = [
            make_turn(
                "c1",
                1,
                TurnRole::User,
                "Please use Rust for all backend services and always prefer async",
            ),
            make_turn("c2", 2, TurnRole::Assistant, "Got it"),
        ];
        let recent = [make_turn(
            "r1",
            1,
            TurnRole::User,
            "Please use Rust for all backend services and always prefer async",
        )];
        let signals = detect_repetition_signals(&current, &recent);
        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0].signal_type, SignalType::RepetitionSignal);
        assert_eq!(signals[0].confidence, 0.85);
    }

    #[test]
    fn no_repetition_different_content() {
        let current = [make_turn(
            "c1",
            1,
            TurnRole::User,
            "Help me write a Python script for data analysis",
        )];
        let recent = [make_turn(
            "r1",
            1,
            TurnRole::User,
            "Please configure the nginx reverse proxy",
        )];
        let signals = detect_repetition_signals(&current, &recent);
        assert!(signals.is_empty());
    }

    #[test]
    fn no_repetition_empty_recent() {
        let current = [make_turn("c1", 1, TurnRole::User, "Some content")];
        let recent: Vec<ConversationTurn> = vec![];
        let signals = detect_repetition_signals(&current, &recent);
        assert!(signals.is_empty());
    }
}
