//! Emotional annotation — valence, intensity, and context tag estimation.

use crate::{ConversationTurn, TurnRole};

/// Positive emotional keywords (bilingual).
/// English keywords use word-boundary matching via `contains_any`;
/// Chinese keywords use substring matching (no spaces in Chinese text).
const POSITIVE_KEYWORDS_EN: &[&str] = &[
    "great",
    "excellent",
    "perfect",
    "awesome",
    "works",
    "success",
    "solved",
    "fixed",
    "love",
    "happy",
    "thanks",
    "thank you",
];

const POSITIVE_KEYWORDS_ZH: &[&str] = &["成功了", "解决了", "完美", "很好", "太好了", "谢谢"];

/// Negative emotional keywords (bilingual).
const NEGATIVE_KEYWORDS_EN: &[&str] = &[
    "error",
    "bug",
    "broken",
    "fail",
    "failed",
    "wrong",
    "incorrect",
    "frustrating",
    "annoying",
    "hate",
    "dislike",
    "crash",
];

const NEGATIVE_KEYWORDS_ZH: &[&str] = &["错误", "失败", "崩溃", "坏了", "不对", "烦"];

/// High-intensity emotional keywords (bilingual).
const HIGH_INTENSITY_KEYWORDS_EN: &[&str] =
    &["critical", "urgent", "emergency", "must", "immediately"];

const HIGH_INTENSITY_KEYWORDS_ZH: &[&str] = &["至关重要", "紧急", "必须", "立刻"];

/// Context tag keyword mappings (bilingual).
/// English keywords use word-boundary matching; Chinese keywords use substring.
const CONTEXT_TAG_MAP: &[(&str, &str)] = &[
    ("error", "error_encountered"),
    ("bug", "bug_found"),
    ("fail", "failure"),
    ("crash", "crash"),
    ("错误", "error_encountered"),
    ("崩溃", "crash"),
    ("fixed", "fix_applied"),
    ("solved", "problem_solved"),
    ("修复", "fix_applied"),
    ("解决", "problem_solved"),
    ("deploy", "deployment"),
    ("部署", "deployment"),
    ("test", "testing"),
    ("测试", "testing"),
    ("debug", "debugging"),
    ("调试", "debugging"),
    ("refactor", "refactoring"),
    ("重构", "refactoring"),
];

/// Collect lowercased user content from turns (shared helper for emotional annotation).
/// Computes the joined user-content string once, avoiding triple redundant construction.
fn collect_user_content(turns: &[ConversationTurn]) -> String {
    turns
        .iter()
        .filter(|t| t.role == TurnRole::User)
        .map(|t| t.content_text.to_lowercase())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Bilingual keyword check: English uses word-boundary matching (`contains_any`),
/// Chinese uses plain substring matching.
fn contains_bilingual(text: &str, en_keywords: &[&str], zh_keywords: &[&str]) -> bool {
    if crate::contains_any(text, en_keywords) {
        return true;
    }
    let lower = text.to_lowercase();
    for kw in zh_keywords {
        if lower.contains(kw) {
            return true;
        }
    }
    false
}

/// Count bilingual keyword hits with word-boundary matching for English.
pub(crate) fn count_bilingual_hits(
    text: &str,
    en_keywords: &[&str],
    zh_keywords: &[&str],
) -> usize {
    let mut count = 0;
    // English: word-boundary check via contains_any (counts each matched keyword)
    let lower = text.to_lowercase();
    for kw in en_keywords {
        if crate::contains_any(text, &[*kw]) {
            count += 1;
        }
    }
    // Chinese: substring check
    for kw in zh_keywords {
        if lower.contains(kw) {
            count += 1;
        }
    }
    count
}

/// Estimate emotional valence from user turns.
/// Returns: positive (0.3-0.8), neutral (None), negative (-0.3 to -0.8).
pub(crate) fn estimate_emotional_valence(turns: &[ConversationTurn]) -> Option<f64> {
    let user_content = collect_user_content(turns);

    if user_content.is_empty() {
        return None;
    }

    let positive_count =
        count_bilingual_hits(&user_content, POSITIVE_KEYWORDS_EN, POSITIVE_KEYWORDS_ZH);
    let negative_count =
        count_bilingual_hits(&user_content, NEGATIVE_KEYWORDS_EN, NEGATIVE_KEYWORDS_ZH);

    if positive_count == 0 && negative_count == 0 {
        return None; // Neutral — no annotation needed
    }

    if positive_count > negative_count {
        Some(0.3 + 0.1 * (positive_count - negative_count).min(5) as f64)
    } else if negative_count > positive_count {
        Some(-0.3 - 0.1 * (negative_count - positive_count).min(5) as f64)
    } else {
        Some(0.0) // Mixed signals — annotate as neutral-ish
    }
}

/// Estimate emotional intensity from user turns.
/// Returns: high (0.7-0.9) when urgency/critical keywords detected,
///          moderate (0.5) for positive/negative without urgency,
///          None for neutral.
pub(crate) fn estimate_emotional_intensity(turns: &[ConversationTurn]) -> Option<f64> {
    let user_content = collect_user_content(turns);

    if user_content.is_empty() {
        return None;
    }

    // Check for high-intensity keywords
    let high_count = count_bilingual_hits(
        &user_content,
        HIGH_INTENSITY_KEYWORDS_EN,
        HIGH_INTENSITY_KEYWORDS_ZH,
    );

    if high_count > 0 {
        return Some(0.7 + 0.1 * high_count.min(2) as f64);
    }

    // Moderate intensity if we have any emotional signal
    let has_emotional =
        contains_bilingual(&user_content, POSITIVE_KEYWORDS_EN, POSITIVE_KEYWORDS_ZH)
            || contains_bilingual(&user_content, NEGATIVE_KEYWORDS_EN, NEGATIVE_KEYWORDS_ZH);

    if has_emotional { Some(0.5) } else { None }
}

/// Estimate context tags from user turns.
/// Returns JSON array string like ["error_encountered", "fix_applied"].
pub(crate) fn estimate_context_tags(turns: &[ConversationTurn]) -> Option<String> {
    let user_content = collect_user_content(turns);

    if user_content.is_empty() {
        return None;
    }

    let tags: Vec<&str> = CONTEXT_TAG_MAP
        .iter()
        .filter(|(kw, _)| {
            // English keywords: word-boundary matching
            if kw.chars().all(|c| c.is_ascii_alphabetic()) {
                crate::contains_any(&user_content, &[*kw])
            } else {
                // Chinese keywords: substring matching
                user_content.to_lowercase().contains(*kw)
            }
        })
        .map(|(_, tag)| *tag)
        .collect::<Vec<_>>();

    // Deduplicate
    let mut unique_tags: Vec<&str> = tags;
    unique_tags.sort_unstable();
    unique_tags.dedup();

    if unique_tags.is_empty() {
        None
    } else {
        serde_json::to_string(&unique_tags).ok()
    }
}

// ─── Query Valence Estimation (§4.3) ──────────────────────────────────

/// Estimate query polarity from keywords for valence-weighted retrieval.
/// Returns negative float for negation/frustration queries, positive for
/// preference/affirmative queries, 0.0 for neutral queries.
///
/// Uses a dedicated set of query-polarity keywords rather than importing
/// feedback_signal::NEGATION_KEYWORDS, because those contain preference-flip
/// words like "prefer" which serve a different semantic role in feedback
/// detection ("I prefer X instead" = negation of current proposal) but
/// would be wrong for query polarity ("I always prefer X" = positive).
pub(crate) fn estimate_query_valence(query: &str) -> f64 {
    let lower = query.to_lowercase();

    // Query negation keywords — pure rejection/negation words only.
    const QUERY_NEGATION_EN: &[&str] = &[
        "don't",
        "dont",
        "do not",
        "no",
        "not",
        "never",
        "avoid",
        "instead",
        "stop",
        "rather",
        "not like",
        "dislike",
        "don't want",
        "dont want",
        "i'd rather",
        "i would rather",
    ];
    const QUERY_NEGATION_ZH: &[&str] = &[
        "不要",
        "不用",
        "不行",
        "不好",
        "避免",
        "换成",
        "别用",
        "停止",
        "不想",
        "不喜欢",
        "宁愿",
    ];

    // Preference keywords — reused from feedback_signal (§5.2).
    let pref_hits = count_bilingual_hits(
        &lower,
        crate::feedback_signal::PREFERENCE_KEYWORDS_EN,
        crate::feedback_signal::PREFERENCE_KEYWORDS_ZH,
    );

    let neg_hits = count_bilingual_hits(&lower, QUERY_NEGATION_EN, QUERY_NEGATION_ZH);

    // Frustration keywords — only emotional terms, NOT factual technical terms.
    const QUERY_FRUSTRATION_EN: &[&str] = &[
        "frustrating",
        "annoying",
        "hate",
        "awful",
        "terrible",
        "horrible",
        "sucks",
        "worst",
        "disappointed",
    ];
    const QUERY_FRUSTRATION_ZH: &[&str] = &["烦", "讨厌", "糟糕", "差劲", "失望", "气死"];
    let frustration_hits = count_bilingual_hits(&lower, QUERY_FRUSTRATION_EN, QUERY_FRUSTRATION_ZH);

    // §4.3 polarity logic: negation flips preference direction.
    let effective_neg = neg_hits + frustration_hits + if neg_hits > 0 { pref_hits } else { 0 };
    let effective_pos = if neg_hits > 0 { 0 } else { pref_hits };

    if effective_neg > effective_pos {
        let strength = 0.3 + 0.1 * (effective_neg - effective_pos).min(3) as f64;
        -strength.min(0.8)
    } else if effective_pos > effective_neg {
        let strength = 0.3 + 0.1 * (effective_pos - effective_neg).min(3) as f64;
        strength.min(0.8)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_query_valence_negation() {
        assert!(estimate_query_valence("don't use this") < 0.0);
        assert!(estimate_query_valence("避免使用") < 0.0);
    }

    #[test]
    fn estimate_query_valence_affirmative() {
        assert!(estimate_query_valence("I always prefer this") > 0.0);
        assert!(estimate_query_valence("习惯用这个") > 0.0);
    }

    #[test]
    fn estimate_query_valence_neutral() {
        assert_eq!(estimate_query_valence("explain the architecture"), 0.0);
        assert_eq!(estimate_query_valence("explain the bug"), 0.0);
        assert_eq!(estimate_query_valence("what caused the error"), 0.0);
    }

    #[test]
    fn estimate_query_valence_negation_plus_preference_is_negative() {
        assert!(estimate_query_valence("I don't like this approach") < 0.0);
        assert!(estimate_query_valence("I don't want that") < 0.0);
        assert!(estimate_query_valence("不喜欢这种做法") < 0.0);
    }

    #[test]
    fn estimate_query_valence_frustration_is_negative() {
        assert!(estimate_query_valence("this is frustrating") < 0.0);
        assert!(estimate_query_valence("讨厌这种做法") < 0.0);
    }
}
