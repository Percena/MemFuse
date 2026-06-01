//! Memory writeback functions — migrated from mfs-session.
//!
//! These functions build and persist memory content files.  They operate
//! purely on memory-domain types (candidates, categories) and filesystem
//! paths, returning `Result<_, String>` instead of `SessionError` since
//! the session error type is session-domain.

use std::path::Path;

use mfs_metadata::StoredFact;
use mfs_semantic::ProcessingMode;
use mfs_semantic::chat_provider_from_env;
use serde::{Deserialize, Serialize};
use tokio::fs;

use crate::candidates::{
    MemoryCandidate, MemoryCategory, MemoryDecision, MemoryRecord, decide_memory_merge,
    llm_merge_bundle,
};

// ── Domain types ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageRecord {
    pub kind: String,
    pub uri: String,
    pub success: Option<bool>,
}

// ── File write helpers ────────────────────────────────────────────────────

/// Create parent directories and write content to a memory file.
///
/// Changed from `Result<_, SessionError>` to returning `std::io::Error`
/// directly — this crate has no `SessionError` type.
pub async fn write_memory_file(path: &Path, content: &str) -> Result<(), std::io::Error> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await?;
    }
    fs::write(path, content).await
}

// ── Archive content builders ──────────────────────────────────────────────

pub fn build_archive_abstract(
    archive_uri: &str,
    messages: &[(String, String)],
    usage: &[UsageRecord],
) -> String {
    format!(
        "Archive summary for {archive_uri}: {} messages, {} contexts, {} skills.",
        messages.len(),
        usage
            .iter()
            .filter(|record| record.kind == "context")
            .count(),
        usage.iter().filter(|record| record.kind == "skill").count(),
    )
}

pub fn build_archive_overview(
    archive_uri: &str,
    messages: &[(String, String)],
    usage: &[UsageRecord],
) -> String {
    let mut lines = vec![
        "# Archived Session Overview".to_owned(),
        String::new(),
        format!("Archive: `{archive_uri}`"),
        format!("Messages: {}", messages.len()),
        format!(
            "Used contexts: {}",
            usage
                .iter()
                .filter(|record| record.kind == "context")
                .count()
        ),
        format!(
            "Used skills: {}",
            usage.iter().filter(|record| record.kind == "skill").count()
        ),
        String::new(),
        "## Messages".to_owned(),
    ];

    for (role, content) in messages {
        lines.push(format!("- `{role}`: {content}"));
    }

    if !usage.is_empty() {
        lines.push(String::new());
        lines.push("## Usage".to_owned());
        for record in usage {
            match record.success {
                Some(success) => lines.push(format!(
                    "- `{}` {} success={success}",
                    record.kind, record.uri
                )),
                None => lines.push(format!("- `{}` {}", record.kind, record.uri)),
            }
        }
    }

    lines.join("\n")
}

pub fn build_user_memory_content(
    archive_uri: &str,
    messages: &[(String, String)],
    usage: &[UsageRecord],
) -> String {
    let contexts = usage
        .iter()
        .filter(|record| record.kind == "context")
        .map(|record| record.uri.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let highlights = messages
        .iter()
        .take(3)
        .map(|(role, content)| format!("{role}: {content}"))
        .collect::<Vec<_>>()
        .join(" | ");

    format!(
        "# Session Memory\n\nArchive: `{archive_uri}`\n\nHighlights: {highlights}\n\nContexts: {contexts}\n"
    )
}

pub fn build_agent_memory_content(archive_uri: &str, usage: &[UsageRecord]) -> String {
    let skills = usage
        .iter()
        .filter(|record| record.kind == "skill")
        .map(|record| match record.success {
            Some(success) => format!("{} success={success}", record.uri),
            None => record.uri.clone(),
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!("# Skill Usage Memory\n\nArchive: `{archive_uri}`\n\n{skills}\n")
}

pub fn build_agent_skill_record(archive_uri: &str, usage: &[UsageRecord]) -> String {
    let skills = usage
        .iter()
        .filter(|record| record.kind == "skill")
        .map(|record| match record.success {
            Some(success) => format!("- {} success={success}", record.uri),
            None => format!("- {}", record.uri),
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!("# Skill Record\n\nArchive: `{archive_uri}`\n\n{skills}\n")
}

// ── Mergeable / profile / append-only builders ────────────────────────────

pub async fn build_mergeable_category_memory_content(
    path: &Path,
    category: MemoryCategory,
    title: &str,
    candidates: &[MemoryCandidate],
) -> Result<String, String> {
    // Read existing content if the file already exists.
    let existing_raw = if fs::try_exists(path)
        .await
        .map_err(|e| format!("check mergeable memory {}: {}", path.display(), e))?
    {
        fs::read_to_string(path)
            .await
            .map_err(|e| format!("read mergeable memory {}: {}", path.display(), e))?
    } else {
        String::new()
    };

    // Detect whether the existing file uses bullet-list format (deterministic)
    // or rich markdown format (LLM-merged).  We preserve the format of the
    // existing file so tests that count `- ` lines keep working.
    let existing_is_bullet = existing_raw
        .lines()
        .any(|line| line.trim_start().starts_with("- "));

    // Build existing records for the dedup/merge decision.
    let existing_records: Vec<MemoryRecord> = existing_raw
        .lines()
        .filter(|line| line.trim_start().starts_with("- "))
        .map(|line| MemoryRecord {
            category,
            uri: path.to_string_lossy().into_owned(),
            content: line.trim_start_matches("- ").to_owned(),
        })
        .collect();

    // ── LLM path ──────────────────────────────────────────────────────────
    // Try LLM merge bundle for the first candidate when LLM is available.
    // If it succeeds we switch to rich-markdown format for the whole file.
    {
        let provider = chat_provider_from_env();
        if provider.mode() == ProcessingMode::Full && !candidates.is_empty() {
            let initial_content = if existing_is_bullet {
                existing_records
                    .iter()
                    .map(|r| r.content.as_str())
                    .collect::<Vec<_>>()
                    .join("\n\n")
            } else {
                existing_raw.clone()
            };
            let mut current_content = initial_content;
            let mut current_abstract = String::new();
            let mut current_overview = String::new();
            let mut llm_used = false;

            for candidate in candidates {
                let decision = decide_memory_merge(category, candidate, &existing_records).await;
                if matches!(decision.primary, MemoryDecision::Skip) {
                    continue;
                }
                if let Some((merged_abstract, merged_overview, merged_content)) = llm_merge_bundle(
                    candidate,
                    &current_content,
                    &current_abstract,
                    &current_overview,
                )
                .await
                {
                    current_abstract = merged_abstract;
                    current_overview = merged_overview;
                    current_content = merged_content;
                    llm_used = true;
                } else if current_content.is_empty() {
                    current_content = candidate.content.clone();
                    current_abstract = candidate.abstract_text.clone();
                    current_overview = candidate.overview_text.clone();
                } else {
                    current_content =
                        format!("{}\n\n{}", current_content.trim_end(), candidate.content);
                }
            }

            let llm_result = if llm_used && !current_content.is_empty() {
                if !current_abstract.is_empty() {
                    let abstract_path = path.with_extension("md.abstract.md");
                    let _ = tokio::fs::write(&abstract_path, &current_abstract).await;
                }
                if !current_overview.is_empty() {
                    let overview_path = path.with_extension("md.overview.md");
                    let _ = tokio::fs::write(&overview_path, &current_overview).await;
                }
                Some(format!("# {title}\n\n{current_content}\n"))
            } else {
                None
            };

            if let Some(merged) = llm_result {
                return Ok(merged);
            }
        }
    }

    // ── Deterministic fallback: bullet-list format ─────────────────────────
    let mut lines: Vec<String> = existing_records.iter().map(|r| r.content.clone()).collect();

    for candidate in candidates {
        let decision = decide_memory_merge(category, candidate, &existing_records).await;
        match decision.primary {
            MemoryDecision::Merge | MemoryDecision::Skip => {}
            MemoryDecision::Create | MemoryDecision::Delete => {
                lines.push(candidate.content.clone());
            }
        }
    }

    lines.sort();
    lines.dedup();
    let body = lines
        .into_iter()
        .map(|line| format!("- {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    Ok(format!("# {title}\n\n{body}\n"))
}

pub fn sanitize_memory_slug(raw: &str) -> String {
    let mut slug = raw
        .to_ascii_lowercase()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect::<String>();
    while slug.contains("--") {
        slug = slug.replace("--", "-");
    }
    let slug = slug.trim_matches('-');
    // Limit to 200 characters to stay well under the 255-byte OS filename
    // limit.  Using .chars().take() ensures safe truncation regardless of
    // UTF-8 byte boundaries — even if the normalization logic changes in
    // the future to preserve non-ASCII characters.
    slug.chars()
        .take(200)
        .collect::<String>()
        .trim_end_matches('-')
        .to_owned()
}

pub fn build_append_only_category_memory_content(
    title: &str,
    archive_uri: &str,
    candidate: &MemoryCandidate,
) -> String {
    // Use L1 overview if available, otherwise fall back to content.
    let body = if candidate.overview_text.is_empty() {
        candidate.content.clone()
    } else {
        candidate.overview_text.clone()
    };
    format!(
        "# {title}\n\nArchive: `{archive_uri}`\n\n{body}\n\n---\n\n*Abstract: {}*\n",
        candidate.abstract_text
    )
}

pub async fn build_profile_memory_content(
    path: &Path,
    candidates: &[MemoryCandidate],
) -> Result<String, String> {
    let existing_content = if fs::try_exists(path)
        .await
        .map_err(|e| format!("check profile memory {}: {}", path.display(), e))?
    {
        fs::read_to_string(path)
            .await
            .map_err(|e| format!("read profile memory {}: {}", path.display(), e))?
    } else {
        String::new()
    };

    let mut cc = existing_content.clone();
    let mut ca = String::new();
    let mut co = String::new();

    for candidate in candidates {
        if cc.is_empty() {
            ca = candidate.abstract_text.clone();
            co = candidate.overview_text.clone();
            cc = candidate.content.clone();
        } else if let Some((merged_abstract, merged_overview, merged_content)) =
            llm_merge_bundle(candidate, &cc, &ca, &co).await
        {
            ca = merged_abstract;
            co = merged_overview;
            cc = merged_content;
        } else {
            if !cc.contains(&candidate.content) {
                cc = format!("{}\n\n{}", cc.trim_end(), candidate.content);
            }
            if ca.is_empty() {
                ca = candidate.abstract_text.clone();
            }
            if co.is_empty() {
                co = candidate.overview_text.clone();
            }
        }
    }

    let llm_result = if cc.is_empty() {
        None
    } else {
        if !ca.is_empty() {
            let abstract_path = path.with_extension("md.abstract.md");
            let _ = tokio::fs::write(&abstract_path, &ca).await;
        }
        if !co.is_empty() {
            let overview_path = path.with_extension("md.overview.md");
            let _ = tokio::fs::write(&overview_path, &co).await;
        }
        Some(format!("# Profile\n\n{cc}\n"))
    };

    if let Some(merged) = llm_result {
        return Ok(merged);
    }

    // Deterministic fallback: return existing content unchanged.
    Ok(existing_content)
}

// ── Fact-backed content builder ────────────────────────────────────────────

pub fn build_fact_backed_memory_content(
    title: &str,
    facts: &[StoredFact],
    extra_lines: Vec<String>,
) -> String {
    let mut lines = facts
        .iter()
        .map(|fact| fact.display_value.clone())
        .collect::<Vec<_>>();
    lines.extend(
        extra_lines
            .into_iter()
            .filter(|line| !line.trim().is_empty()),
    );
    lines.sort();
    lines.dedup();
    let body = lines
        .into_iter()
        .map(|line| format!("- {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    format!("# {title}\n\n{body}\n")
}

// ── Fact classification helpers ────────────────────────────────────────────

pub fn is_profile_fact(fact: &StoredFact) -> bool {
    matches!(
        fact.predicate.as_str(),
        "identity.name"
            | "identity.pronouns"
            | "profile.name"
            | "profile.role"
            | "profile.location"
            | "location.current_city"
            | "location.current_country"
            | "work.current_role"
            | "work.current_company"
            | "language.spoken"
    )
}

pub fn is_preference_fact(fact: &StoredFact) -> bool {
    fact.predicate.starts_with("preference.")
        || fact.predicate.starts_with("health.")
        || fact.predicate.starts_with("diet.")
}

pub fn is_entity_fact(fact: &StoredFact) -> bool {
    fact.predicate.starts_with("entities.") || fact.predicate == "project.active"
}

pub fn entity_slug_from_fact(fact: &StoredFact) -> String {
    let title = fact
        .display_value
        .strip_prefix("User is working on ")
        .or_else(|| {
            fact.display_value
                .strip_prefix("Current architecture decision: ")
        })
        .unwrap_or(fact.display_value.as_str());
    sanitize_memory_slug(title)
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── sanitize_memory_slug tests ──

    #[test]
    fn slug_basic_ascii() {
        assert_eq!(sanitize_memory_slug("Hello World"), "hello-world");
    }

    #[test]
    fn slug_dedup_dashes() {
        assert_eq!(sanitize_memory_slug("a  --  b"), "a-b");
    }

    #[test]
    fn slug_truncate_at_200_chars() {
        let long = "a".repeat(300);
        let result = sanitize_memory_slug(&long);
        assert_eq!(result.len(), 200);
        assert_eq!(result, "a".repeat(200));
    }

    #[test]
    fn slug_truncate_does_not_end_with_dash() {
        // A slug like "a-b-c-d-..." truncated at 200 where char 200 is '-'
        // should be trimmed.
        let input = "ab-".repeat(150); // 300 chars, alternating ab-
        let result = sanitize_memory_slug(&input);
        assert!(!result.ends_with('-'));
        assert!(result.len() <= 200);
    }

    #[test]
    fn slug_short_input_unchanged() {
        assert_eq!(sanitize_memory_slug("short"), "short");
    }

    #[test]
    fn slug_unicode_replaced_with_dashes() {
        // Current normalization replaces non-ASCII-alphanumeric with dashes.
        // 4 CJK chars → 4 dashes → dedup → 1 dash → trim → empty string
        assert_eq!(sanitize_memory_slug("你好世界"), "");
        // 2 CJK chars → 2 dashes → dedup → 1 dash → trim → empty string
        assert_eq!(sanitize_memory_slug("你好"), "");
        // Mixed: "a你好b" → "a--b" → dedup → "a-b"
        assert_eq!(sanitize_memory_slug("a你好b"), "a-b");
    }
}
