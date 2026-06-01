//! Rule-based deterministic memory candidate extraction.

use super::schema::{MemoryCandidate, MemoryCategory};

/// Rule-based extraction used when no LLM is available.
/// Produces minimal but valid `MemoryCandidate` items.
pub fn deterministic_extract(
    messages: &[(String, String)],
    usage: &[(String, String, Option<bool>)],
) -> Vec<MemoryCandidate> {
    let mut candidates = Vec::new();

    for (role, content) in messages {
        let normalized = content.trim();
        if role == "user" {
            if let Some(profile) = extract_profile(normalized) {
                candidates.push(MemoryCandidate::simple(
                    MemoryCategory::Profile,
                    "user_profile",
                    profile,
                    normalized,
                ));
            }
            if let Some(preference) = extract_preference(normalized) {
                candidates.push(MemoryCandidate::simple(
                    MemoryCategory::Preferences,
                    "user_preference",
                    preference,
                    normalized,
                ));
            }
            if let Some(event) = extract_event(normalized) {
                candidates.push(MemoryCandidate::simple(
                    MemoryCategory::Events,
                    "session_event",
                    event,
                    normalized,
                ));
            }
            if let Some((entity_title, entity_content)) = extract_entity(normalized) {
                candidates.push(MemoryCandidate::simple(
                    MemoryCategory::Entities,
                    entity_title,
                    entity_content,
                    normalized,
                ));
            }
        }

        if let Some(case) = extract_case(normalized) {
            candidates.push(MemoryCandidate::simple(
                MemoryCategory::Cases,
                "session_case",
                case,
                normalized,
            ));
        }
        if let Some((pattern_title, pattern_content)) = extract_pattern(normalized) {
            candidates.push(MemoryCandidate::simple(
                MemoryCategory::Patterns,
                pattern_title,
                pattern_content,
                normalized,
            ));
        }
    }

    for (kind, uri, success) in usage {
        let category = match kind.as_str() {
            "skill" => Some(MemoryCategory::Skills),
            "tool" => Some(MemoryCategory::Tools),
            _ => None,
        };
        let Some(category) = category else {
            continue;
        };

        let name = uri
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or(kind)
            .to_owned();
        let content = match success {
            Some(success) => format!("{uri} success={success}"),
            None => uri.clone(),
        };
        let mut candidate = MemoryCandidate::simple(category, &name, content, uri.as_str());
        match category {
            MemoryCategory::Tools => candidate.tool_name = Some(name),
            MemoryCategory::Skills => candidate.skill_name = Some(name),
            _ => {}
        }
        candidates.push(candidate);
    }

    candidates
}

fn extract_preference(content: &str) -> Option<String> {
    let lowered = content.to_ascii_lowercase();
    let marker = "i prefer ";
    let index = lowered.find(marker)?;
    let preference = content[index + marker.len()..]
        .trim()
        .trim_end_matches('.')
        .to_owned();
    if preference.is_empty() {
        None
    } else {
        Some(preference)
    }
}

fn extract_profile(content: &str) -> Option<String> {
    let lowered = content.to_ascii_lowercase();
    let marker = "my name is ";
    let index = lowered.find(marker)?;
    let profile = content[index + marker.len()..]
        .trim()
        .trim_end_matches('.')
        .to_owned();
    if profile.is_empty() {
        None
    } else {
        Some(profile)
    }
}

fn extract_event(content: &str) -> Option<String> {
    let lowered = content.to_ascii_lowercase();
    let marker = "decided to ";
    let index = lowered.find(marker)?;
    let event = content[index + marker.len()..]
        .trim()
        .trim_end_matches('.')
        .to_owned();
    if event.is_empty() { None } else { Some(event) }
}

fn extract_case(content: &str) -> Option<String> {
    let lowered = content.to_ascii_lowercase();
    if lowered.contains("resolved") && (lowered.contains("incident") || lowered.contains("issue")) {
        Some(content.trim().trim_end_matches('.').to_owned())
    } else {
        None
    }
}

fn extract_entity(content: &str) -> Option<(String, String)> {
    let marker = "Project ";
    let index = content.find(marker)?;
    let remainder = &content[index + marker.len()..];
    let name = remainder
        .split_whitespace()
        .next()
        .map(|name| name.trim_matches(|ch: char| !ch.is_ascii_alphanumeric()))
        .unwrap_or_default();
    if name.is_empty() {
        return None;
    }
    Some((
        format!("project-{name}"),
        content.trim().trim_end_matches('.').to_owned(),
    ))
}

fn extract_pattern(content: &str) -> Option<(String, String)> {
    let lowered = content.to_ascii_lowercase();
    if lowered.starts_with("use ") && lowered.contains(" then ") {
        let tokens = content
            .split(|ch: char| !ch.is_ascii_alphanumeric())
            .filter(|token| !token.is_empty())
            .map(|token| token.to_ascii_lowercase())
            .collect::<Vec<_>>();
        let stable_title = tokens.iter().take(3).cloned().collect::<Vec<_>>().join("-");
        Some((
            stable_title,
            content.trim().trim_end_matches('.').to_owned(),
        ))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── extract_preference ──────────────────────────────────────────────

    #[test]
    fn preference_basic_match() {
        let result = extract_preference("I prefer dark mode over light mode");
        assert_eq!(result, Some("dark mode over light mode".to_owned()));
    }

    #[test]
    fn preference_case_insensitive_match() {
        let result = extract_preference("i PREFER running in the morning");
        assert_eq!(result, Some("running in the morning".to_owned()));
    }

    #[test]
    fn preference_trailing_period_stripped() {
        let result = extract_preference("I prefer tea over coffee.");
        assert_eq!(result, Some("tea over coffee".to_owned()));
    }

    #[test]
    fn preference_in_middle_of_sentence() {
        let result = extract_preference("Well, I prefer Python for scripting, actually");
        assert_eq!(result, Some("Python for scripting, actually".to_owned()));
    }

    #[test]
    fn preference_no_marker_returns_none() {
        assert_eq!(extract_preference("I like dark mode"), None);
    }

    #[test]
    fn preference_empty_after_marker_returns_none() {
        assert_eq!(extract_preference("I prefer "), None);
    }

    #[test]
    fn preference_marker_only_returns_none() {
        assert_eq!(extract_preference("I prefer."), None);
    }

    #[test]
    fn preference_marker_with_trailing_period_empty_returns_none() {
        assert_eq!(extract_preference("I prefer ."), None);
    }

    #[test]
    fn preference_preserves_original_case_in_extraction() {
        let result = extract_preference("I prefer macOS for development");
        assert_eq!(result, Some("macOS for development".to_owned()));
    }

    #[test]
    fn preference_multiple_markers_uses_first() {
        let result = extract_preference("I prefer A and then I prefer B");
        assert_eq!(result, Some("A and then I prefer B".to_owned()));
    }

    // ─── extract_profile ────────────────────────────────────────────────

    #[test]
    fn profile_basic_match() {
        let result = extract_profile("My name is Alice");
        assert_eq!(result, Some("Alice".to_owned()));
    }

    #[test]
    fn profile_case_insensitive_match() {
        let result = extract_profile("MY NAME IS Bob");
        assert_eq!(result, Some("Bob".to_owned()));
    }

    #[test]
    fn profile_trailing_period_stripped() {
        let result = extract_profile("My name is Carol.");
        assert_eq!(result, Some("Carol".to_owned()));
    }

    #[test]
    fn profile_in_middle_of_sentence() {
        let result = extract_profile("Hello, my name is Dave, nice to meet you");
        assert_eq!(result, Some("Dave, nice to meet you".to_owned()));
    }

    #[test]
    fn profile_no_marker_returns_none() {
        assert_eq!(extract_profile("I'm called Alice"), None);
    }

    #[test]
    fn profile_empty_after_marker_returns_none() {
        assert_eq!(extract_profile("My name is "), None);
    }

    #[test]
    fn profile_preserves_original_case() {
        let result = extract_profile("My name is Dr. Smith");
        assert_eq!(result, Some("Dr. Smith".to_owned()));
    }

    #[test]
    fn profile_multiple_markers_uses_first() {
        let result = extract_profile("My name is Alice and my name is also Bob");
        assert_eq!(result, Some("Alice and my name is also Bob".to_owned()));
    }

    // ─── extract_event ──────────────────────────────────────────────────

    #[test]
    fn event_basic_match() {
        let result = extract_event("We decided to deploy the new version");
        assert_eq!(result, Some("deploy the new version".to_owned()));
    }

    #[test]
    fn event_case_insensitive_match() {
        let result = extract_event("DECIDED TO migrate the database");
        assert_eq!(result, Some("migrate the database".to_owned()));
    }

    #[test]
    fn event_trailing_period_stripped() {
        let result = extract_event("I decided to switch to Rust.");
        assert_eq!(result, Some("switch to Rust".to_owned()));
    }

    #[test]
    fn event_in_middle_of_sentence() {
        let result = extract_event("After discussion we decided to cancel the project entirely");
        assert_eq!(result, Some("cancel the project entirely".to_owned()));
    }

    #[test]
    fn event_no_marker_returns_none() {
        assert_eq!(extract_event("We will deploy tomorrow"), None);
    }

    #[test]
    fn event_empty_after_marker_returns_none() {
        assert_eq!(extract_event("decided to "), None);
    }

    #[test]
    fn event_preserves_original_case() {
        let result = extract_event("We decided to adopt Kubernetes");
        assert_eq!(result, Some("adopt Kubernetes".to_owned()));
    }

    #[test]
    fn event_multiple_markers_uses_first() {
        let result = extract_event("We decided to go left, then decided to go right");
        assert_eq!(result, Some("go left, then decided to go right".to_owned()));
    }

    // ─── extract_case ───────────────────────────────────────────────────

    #[test]
    fn case_resolved_incident_match() {
        let result = extract_case("We resolved the incident successfully");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "We resolved the incident successfully");
    }

    #[test]
    fn case_resolved_issue_match() {
        let result = extract_case("The team resolved the issue with database");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "The team resolved the issue with database");
    }

    #[test]
    fn case_case_insensitive_match() {
        let result = extract_case("RESOLVED the INCIDENT quickly");
        assert!(result.is_some());
    }

    #[test]
    fn case_resolved_without_incident_or_issue_returns_none() {
        assert_eq!(extract_case("We resolved the conflict"), None);
    }

    #[test]
    fn case_incident_without_resolved_returns_none() {
        assert_eq!(extract_case("There was an incident yesterday"), None);
    }

    #[test]
    fn case_issue_without_resolved_returns_none() {
        assert_eq!(extract_case("The issue is still open"), None);
    }

    #[test]
    fn case_trailing_period_stripped() {
        let result = extract_case("resolved the incident.");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "resolved the incident");
    }

    #[test]
    fn case_no_relevant_keywords_returns_none() {
        assert_eq!(extract_case("Everything is fine"), None);
    }

    #[test]
    fn case_resolved_incident_and_issue_both_present() {
        let result = extract_case("resolved the incident issue");
        assert!(result.is_some());
    }

    // ─── extract_entity ─────────────────────────────────────────────────

    #[test]
    fn entity_basic_match() {
        let result = extract_entity("Project Alpha is a Rust project");
        assert!(result.is_some());
        let (title, content) = result.unwrap();
        assert_eq!(title, "project-Alpha");
        assert_eq!(content, "Project Alpha is a Rust project");
    }

    #[test]
    fn entity_case_sensitive_marker() {
        let result = extract_entity("project Alpha is a Rust project");
        assert!(result.is_none());
    }

    #[test]
    fn entity_trailing_period_stripped_in_content() {
        let result = extract_entity("Project Beta is cool.");
        assert!(result.is_some());
        let (_, content) = result.unwrap();
        assert_eq!(content, "Project Beta is cool");
    }

    #[test]
    fn entity_no_marker_returns_none() {
        assert_eq!(extract_entity("Alpha is a Rust project"), None);
    }

    #[test]
    fn entity_name_is_first_word_after_project() {
        let result = extract_entity("Project MyApp uses Vue.js");
        assert!(result.is_some());
        let (title, _) = result.unwrap();
        assert_eq!(title, "project-MyApp");
    }

    #[test]
    fn entity_name_with_non_alpha_chars_trimmed() {
        let result = extract_entity("Project \"Cloud\" is deployed");
        assert!(result.is_some());
        let (title, _) = result.unwrap();
        assert_eq!(title, "project-Cloud");
    }

    #[test]
    fn entity_empty_after_project_returns_none() {
        assert_eq!(extract_entity("Project "), None);
    }

    #[test]
    fn entity_in_middle_of_sentence() {
        let result = extract_entity("As for Project Gamma, it was cancelled");
        assert!(result.is_some());
        let (title, _) = result.unwrap();
        assert_eq!(title, "project-Gamma");
    }

    // ─── extract_pattern ────────────────────────────────────────────────

    #[test]
    fn pattern_basic_match() {
        let result = extract_pattern("Use docker then build the image");
        assert!(result.is_some());
        let (_title, content) = result.unwrap();
        assert_eq!(content, "Use docker then build the image");
    }

    #[test]
    fn pattern_title_from_first_three_tokens() {
        let result = extract_pattern("Use docker then build the image");
        assert!(result.is_some());
        let (title, _) = result.unwrap();
        assert_eq!(title, "use-docker-then");
    }

    #[test]
    fn pattern_case_insensitive_startswith_use() {
        let result = extract_pattern("USE git then commit");
        assert!(result.is_some());
    }

    #[test]
    fn pattern_no_then_returns_none() {
        assert_eq!(extract_pattern("Use docker for building"), None);
    }

    #[test]
    fn pattern_does_not_start_with_use_returns_none() {
        assert_eq!(extract_pattern("First use docker then build"), None);
    }

    #[test]
    fn pattern_trailing_period_stripped() {
        let result = extract_pattern("Use cargo then test.");
        assert!(result.is_some());
        let (_, content) = result.unwrap();
        assert_eq!(content, "Use cargo then test");
    }

    #[test]
    fn pattern_multiple_then_still_matches() {
        let result = extract_pattern("Use ssh then connect then run");
        assert!(result.is_some());
        let (title, _) = result.unwrap();
        assert_eq!(title, "use-ssh-then");
    }

    #[test]
    fn pattern_empty_string_returns_none() {
        assert_eq!(extract_pattern(""), None);
    }

    #[test]
    fn pattern_only_use_returns_none() {
        assert_eq!(extract_pattern("Use "), None);
    }

    #[test]
    fn pattern_tokens_from_split_on_non_alphanumeric() {
        let result = extract_pattern("Use A/B then C");
        assert!(result.is_some());
        let (title, _) = result.unwrap();
        assert_eq!(title, "use-a-b");
    }

    // ─── deterministic_extract (public entry point) ─────────────────────

    #[test]
    fn deterministic_extract_empty_messages_empty_usage() {
        let messages: Vec<(String, String)> = vec![];
        let usage: Vec<(String, String, Option<bool>)> = vec![];
        let result = deterministic_extract(&messages, &usage);
        assert!(result.is_empty());
    }

    #[test]
    fn deterministic_extract_no_matching_patterns() {
        let messages = vec![
            ("user".to_owned(), "Hello there!".to_owned()),
            ("assistant".to_owned(), "Hi, how can I help?".to_owned()),
        ];
        let usage: Vec<(String, String, Option<bool>)> = vec![];
        let result = deterministic_extract(&messages, &usage);
        assert!(result.is_empty());
    }

    #[test]
    fn deterministic_extract_preference_from_user_message() {
        let messages = vec![("user".to_owned(), "I prefer dark mode".to_owned())];
        let usage: Vec<(String, String, Option<bool>)> = vec![];
        let result = deterministic_extract(&messages, &usage);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].category, MemoryCategory::Preferences);
        assert_eq!(result[0].title, "user_preference");
        assert_eq!(result[0].content, "dark mode");
    }

    #[test]
    fn deterministic_extract_profile_from_user_message() {
        let messages = vec![("user".to_owned(), "My name is Alice".to_owned())];
        let usage: Vec<(String, String, Option<bool>)> = vec![];
        let result = deterministic_extract(&messages, &usage);
        assert!(result.iter().any(|c| c.category == MemoryCategory::Profile));
    }

    #[test]
    fn deterministic_extract_event_from_user_message() {
        let messages = vec![(
            "user".to_owned(),
            "We decided to migrate the database".to_owned(),
        )];
        let usage: Vec<(String, String, Option<bool>)> = vec![];
        let result = deterministic_extract(&messages, &usage);
        assert!(result.iter().any(|c| c.category == MemoryCategory::Events));
    }

    #[test]
    fn deterministic_extract_case_from_any_role() {
        let messages = vec![(
            "assistant".to_owned(),
            "We resolved the incident successfully".to_owned(),
        )];
        let usage: Vec<(String, String, Option<bool>)> = vec![];
        let result = deterministic_extract(&messages, &usage);
        assert!(result.iter().any(|c| c.category == MemoryCategory::Cases));
    }

    #[test]
    fn deterministic_extract_pattern_from_any_role() {
        let messages = vec![(
            "assistant".to_owned(),
            "Use docker then build the image".to_owned(),
        )];
        let usage: Vec<(String, String, Option<bool>)> = vec![];
        let result = deterministic_extract(&messages, &usage);
        assert!(
            result
                .iter()
                .any(|c| c.category == MemoryCategory::Patterns)
        );
    }

    #[test]
    fn deterministic_extract_entity_from_user_only() {
        let messages = vec![(
            "assistant".to_owned(),
            "Project Alpha is a Rust project".to_owned(),
        )];
        let usage: Vec<(String, String, Option<bool>)> = vec![];
        let result = deterministic_extract(&messages, &usage);
        assert!(
            !result
                .iter()
                .any(|c| c.category == MemoryCategory::Entities)
        );
    }

    #[test]
    fn deterministic_extract_multiple_candidates_from_one_message() {
        let messages = vec![(
            "user".to_owned(),
            "My name is Alice and I prefer dark mode. We decided to deploy v2.".to_owned(),
        )];
        let usage: Vec<(String, String, Option<bool>)> = vec![];
        let result = deterministic_extract(&messages, &usage);
        assert!(result.iter().any(|c| c.category == MemoryCategory::Profile));
        assert!(
            result
                .iter()
                .any(|c| c.category == MemoryCategory::Preferences)
        );
        assert!(result.iter().any(|c| c.category == MemoryCategory::Events));
        assert!(result.len() >= 3);
    }

    #[test]
    fn deterministic_extract_preference_and_profile_in_same_message() {
        let messages = vec![(
            "user".to_owned(),
            "My name is Bob. I prefer Rust over Python.".to_owned(),
        )];
        let usage: Vec<(String, String, Option<bool>)> = vec![];
        let result = deterministic_extract(&messages, &usage);
        let prefs = result
            .iter()
            .filter(|c| c.category == MemoryCategory::Preferences)
            .count();
        let profiles = result
            .iter()
            .filter(|c| c.category == MemoryCategory::Profile)
            .count();
        assert_eq!(prefs, 1);
        assert_eq!(profiles, 1);
    }

    #[test]
    fn deterministic_extract_case_and_pattern_from_assistant() {
        let messages = vec![
            (
                "assistant".to_owned(),
                "We resolved the incident by restarting the service.".to_owned(),
            ),
            (
                "assistant".to_owned(),
                "Use docker then rebuild the image".to_owned(),
            ),
        ];
        let usage: Vec<(String, String, Option<bool>)> = vec![];
        let result = deterministic_extract(&messages, &usage);
        assert!(result.iter().any(|c| c.category == MemoryCategory::Cases));
        assert!(
            result
                .iter()
                .any(|c| c.category == MemoryCategory::Patterns)
        );
    }

    // ─── Usage records ──────────────────────────────────────────────────

    #[test]
    fn deterministic_extract_tool_usage() {
        let messages: Vec<(String, String)> = vec![];
        let usage = vec![("tool".to_owned(), "mfs://tool/cargo".to_owned(), Some(true))];
        let result = deterministic_extract(&messages, &usage);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].category, MemoryCategory::Tools);
        assert_eq!(result[0].tool_name, Some("cargo".to_owned()));
        assert_eq!(result[0].skill_name, None);
    }

    #[test]
    fn deterministic_extract_skill_usage() {
        let messages: Vec<(String, String)> = vec![];
        let usage = vec![(
            "skill".to_owned(),
            "mfs://skill/review".to_owned(),
            Some(true),
        )];
        let result = deterministic_extract(&messages, &usage);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].category, MemoryCategory::Skills);
        assert_eq!(result[0].skill_name, Some("review".to_owned()));
        assert_eq!(result[0].tool_name, None);
    }

    #[test]
    fn deterministic_extract_usage_name_extraction_from_uri() {
        let messages: Vec<(String, String)> = vec![];
        let usage = vec![(
            "tool".to_owned(),
            "mfs://tool/cargo-build//".to_owned(),
            Some(true),
        )];
        let result = deterministic_extract(&messages, &usage);
        assert_eq!(result[0].tool_name.as_deref(), Some("cargo-build"));
    }

    #[test]
    fn deterministic_extract_usage_with_none_success() {
        let messages: Vec<(String, String)> = vec![];
        let usage = vec![("tool".to_owned(), "mfs://tool/git".to_owned(), None)];
        let result = deterministic_extract(&messages, &usage);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].content, "mfs://tool/git");
    }

    #[test]
    fn deterministic_extract_usage_with_success_true() {
        let messages: Vec<(String, String)> = vec![];
        let usage = vec![("tool".to_owned(), "mfs://tool/cargo".to_owned(), Some(true))];
        let result = deterministic_extract(&messages, &usage);
        assert_eq!(result[0].content, "mfs://tool/cargo success=true");
    }

    #[test]
    fn deterministic_extract_usage_with_success_false() {
        let messages: Vec<(String, String)> = vec![];
        let usage = vec![(
            "tool".to_owned(),
            "mfs://tool/cargo".to_owned(),
            Some(false),
        )];
        let result = deterministic_extract(&messages, &usage);
        assert_eq!(result[0].content, "mfs://tool/cargo success=false");
    }

    #[test]
    fn deterministic_extract_unknown_usage_kind_skipped() {
        let messages: Vec<(String, String)> = vec![];
        let usage = vec![(
            "unknown".to_owned(),
            "mfs://unknown/foo".to_owned(),
            Some(true),
        )];
        let result = deterministic_extract(&messages, &usage);
        assert!(result.is_empty());
    }

    #[test]
    fn deterministic_extract_multiple_usage_records() {
        let messages: Vec<(String, String)> = vec![];
        let usage = vec![
            ("tool".to_owned(), "mfs://tool/cargo".to_owned(), Some(true)),
            (
                "skill".to_owned(),
                "mfs://skill/review".to_owned(),
                Some(true),
            ),
            ("tool".to_owned(), "mfs://tool/git".to_owned(), Some(false)),
        ];
        let result = deterministic_extract(&messages, &usage);
        assert_eq!(result.len(), 3);
        let tools = result
            .iter()
            .filter(|c| c.category == MemoryCategory::Tools)
            .count();
        let skills = result
            .iter()
            .filter(|c| c.category == MemoryCategory::Skills)
            .count();
        assert_eq!(tools, 2);
        assert_eq!(skills, 1);
    }

    // ─── Combined messages + usage ───────────────────────────────────────

    #[test]
    fn deterministic_extract_messages_and_usage_combined() {
        let messages = vec![("user".to_owned(), "I prefer dark mode".to_owned())];
        let usage = vec![("tool".to_owned(), "mfs://tool/cargo".to_owned(), Some(true))];
        let result = deterministic_extract(&messages, &usage);
        assert_eq!(result.len(), 2);
        assert!(
            result
                .iter()
                .any(|c| c.category == MemoryCategory::Preferences)
        );
        assert!(result.iter().any(|c| c.category == MemoryCategory::Tools));
    }

    #[test]
    fn deterministic_extract_evidence_matches_original_content() {
        let messages = vec![(
            "user".to_owned(),
            "I prefer dark mode over light mode".to_owned(),
        )];
        let usage: Vec<(String, String, Option<bool>)> = vec![];
        let result = deterministic_extract(&messages, &usage);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].evidence, "I prefer dark mode over light mode");
    }

    #[test]
    fn deterministic_extract_whitespace_only_message_returns_no_candidates() {
        let messages = vec![("user".to_owned(), "   ".to_owned())];
        let usage: Vec<(String, String, Option<bool>)> = vec![];
        let result = deterministic_extract(&messages, &usage);
        assert!(result.is_empty());
    }

    #[test]
    fn deterministic_extract_multiple_messages_independent_extraction() {
        let messages = vec![
            ("user".to_owned(), "My name is Alice".to_owned()),
            ("user".to_owned(), "I prefer Rust".to_owned()),
            ("assistant".to_owned(), "Use docker then build".to_owned()),
        ];
        let usage: Vec<(String, String, Option<bool>)> = vec![];
        let result = deterministic_extract(&messages, &usage);
        assert!(result.iter().any(|c| c.category == MemoryCategory::Profile));
        assert!(
            result
                .iter()
                .any(|c| c.category == MemoryCategory::Preferences)
        );
        assert!(
            result
                .iter()
                .any(|c| c.category == MemoryCategory::Patterns)
        );
    }
}
