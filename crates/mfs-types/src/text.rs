use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

/// Short words that carry critical semantic weight (negation, direction, etc.)
/// and must be retained even though they would otherwise be filtered by the
/// `min_len` gate.  Without this list, "no sugar" and "sugar" would tokenize
/// identically after dropping "no", causing a semantic inversion in dedup.
pub const SEMANTIC_SHORT_WORDS: &[&str] = &[
    "no", "not", "never", "nor", "anti", "non", "nil", "null", "zero", "up", "down", "yes", "ok",
    "go", "do", "don", "off", "on", "out", "bad", "top", "low", "big", "new", "old", "hot", "all",
    "any", "few", "own", "too", "yet",
];

/// Configuration for tokenization behavior.
pub struct TokenizeConfig {
    /// If true: split on whitespace, then trim non-alnum from token edges.
    /// If false: split on any non-alphanumeric character.
    pub trim_edges: bool,
    /// Minimum token length to keep (after lowercasing).
    pub min_len: usize,
    /// If true, keep tokens in [`SEMANTIC_SHORT_WORDS`] regardless of `min_len`.
    pub preserve_semantic_short_words: bool,
}

impl Default for TokenizeConfig {
    fn default() -> Self {
        Self {
            trim_edges: false,
            min_len: 3,
            preserve_semantic_short_words: false,
        }
    }
}

/// Tokenize text into a `HashSet` (dedup, order-independent).
/// Used for similarity comparisons (Jaccard, overlap).
pub fn tokenize_to_set(text: &str, config: &TokenizeConfig) -> HashSet<String> {
    split_and_normalize(text, config.trim_edges)
        .filter(|token| should_keep(token, config))
        .collect()
}

/// Tokenize text into a `Vec` (preserves order and duplicates).
/// Used for ordered scoring and summary tokenization.
pub fn tokenize_to_vec(text: &str, config: &TokenizeConfig) -> Vec<String> {
    split_and_normalize(text, config.trim_edges)
        .filter(|token| should_keep(token, config))
        .collect()
}

/// Case-insensitive word-boundary check for term presence.
/// English ASCII terms use regex `\b` boundaries; non-ASCII uses substring.
/// Compiled regexes are cached in a thread-local map to avoid recompilation.
pub fn contains_any(text: &str, terms: &[&str]) -> bool {
    thread_local! {
        static REGEX_CACHE: RefCell<HashMap<String, regex::Regex>> = RefCell::new(HashMap::new());
    }

    let lower = text.to_lowercase();
    terms.iter().any(|term| {
        if term.chars().all(|c| c.is_ascii_alphabetic()) {
            REGEX_CACHE.with(|cache| {
                let mut cache = cache.borrow_mut();
                let re = cache.entry((*term).to_owned()).or_insert_with(|| {
                    let pattern = format!(r"\b{}\b", regex::escape(term));
                    regex::Regex::new(&pattern).expect("invalid regex pattern")
                });
                re.is_match(&lower)
            })
        } else {
            lower.contains(term)
        }
    })
}

// ── Internal helpers ──────────────────────────────────────────────────

/// Split text and normalize tokens to lowercase.
///
/// When `trim_edges` is true: split on whitespace, then trim non-ASCII-alphanumeric
/// characters from each token's edges (preserving internal punctuation like hyphens).
///
/// When `trim_edges` is false: split on any non-alphanumeric character
/// (Unicode-aware `is_alphanumeric`), producing tokens from alphanumeric runs only.
fn split_and_normalize(text: &str, trim_edges: bool) -> impl Iterator<Item = String> + '_ {
    // Collect into a Vec to avoid returning different iterator types.
    // This is fine for the typical text sizes in this codebase.
    let tokens: Vec<String> = if trim_edges {
        text.split_whitespace()
            .map(|token| {
                token
                    .trim_matches(|ch: char| !ch.is_ascii_alphanumeric())
                    .to_ascii_lowercase()
            })
            .collect()
    } else {
        text.split(|ch: char| !ch.is_alphanumeric())
            .filter(|token| !token.is_empty())
            .map(|token| token.to_ascii_lowercase())
            .collect()
    };
    tokens.into_iter()
}

/// Determine whether a token should be kept based on the config.
fn should_keep(token: &str, config: &TokenizeConfig) -> bool {
    token.len() >= config.min_len
        || (config.preserve_semantic_short_words && SEMANTIC_SHORT_WORDS.contains(&token))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenize_to_set_trim_edges() {
        let config = TokenizeConfig {
            trim_edges: true,
            min_len: 3,
            preserve_semantic_short_words: false,
        };
        let set = tokenize_to_set("(hello) world foo", &config);
        assert!(set.contains("hello"));
        assert!(set.contains("world"));
        assert!(set.contains("foo")); // len 3 >= 3, kept
        // Parentheses are trimmed from edges
        let set2 = tokenize_to_set("(ab) cd", &config);
        assert!(!set2.contains("ab")); // len 2 < 3, filtered
        assert!(!set2.contains("cd")); // len 2 < 3, filtered
    }

    #[test]
    fn tokenize_to_set_split_on_non_alnum() {
        let config = TokenizeConfig {
            trim_edges: false,
            min_len: 3,
            preserve_semantic_short_words: false,
        };
        let set = tokenize_to_set("hello-world test", &config);
        assert!(set.contains("hello"));
        assert!(set.contains("world"));
        assert!(set.contains("test"));
    }

    #[test]
    fn tokenize_to_set_preserves_hyphen_when_trim_edges() {
        let config = TokenizeConfig {
            trim_edges: true,
            min_len: 3,
            preserve_semantic_short_words: false,
        };
        let set = tokenize_to_set("hello-world", &config);
        assert!(set.contains("hello-world"));
        assert!(!set.contains("hello"));
    }

    #[test]
    fn tokenize_to_vec_preserves_order() {
        let config = TokenizeConfig {
            trim_edges: false,
            min_len: 1,
            preserve_semantic_short_words: false,
        };
        let vec = tokenize_to_vec("b a b", &config);
        assert_eq!(vec, vec!["b", "a", "b"]);
    }

    #[test]
    fn semantic_short_words_preserved() {
        let config = TokenizeConfig {
            trim_edges: true,
            min_len: 2,
            preserve_semantic_short_words: true,
        };
        let vec = tokenize_to_vec("no sugar is bad", &config);
        assert!(vec.contains(&"no".to_owned()));
        assert!(vec.contains(&"bad".to_owned()));
        assert!(vec.contains(&"sugar".to_owned()));
    }

    #[test]
    fn semantic_short_words_dropped_without_flag() {
        let config = TokenizeConfig {
            trim_edges: true,
            min_len: 3,
            preserve_semantic_short_words: false,
        };
        let vec = tokenize_to_vec("no sugar is good", &config);
        assert!(!vec.contains(&"no".to_owned())); // "no" len 2 < 3
        assert!(!vec.contains(&"is".to_owned())); // "is" len 2 < 3
        assert!(vec.contains(&"sugar".to_owned()));
        assert!(vec.contains(&"good".to_owned()));
    }

    #[test]
    fn contains_any_word_boundary() {
        assert!(contains_any("I live in Tokyo", &["live"]));
        assert!(!contains_any("liverpool", &["live"]));
    }

    #[test]
    fn contains_any_non_ascii_substring() {
        assert!(contains_any("我在东京住", &["东京"]));
        assert!(!contains_any("我在北京住", &["东京"]));
    }

    #[test]
    fn contains_any_mixed() {
        assert!(contains_any("I live in 东京", &["live", "东京"]));
        assert!(!contains_any("I live in 北京", &["东京"]));
    }
}
