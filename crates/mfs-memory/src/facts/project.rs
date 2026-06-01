//! Fact projection — conflict resolution and formatting.
//!
//! Projection lifecycle: scalar (single active), set (multiple with dedup),
//! temporal (retract supersedes).

use crate::{Fact, FactAssertion, FactOperation, FactStatus};

/// Determine if a predicate belongs to the scalar category (single active fact per predicate).
///
/// Scalar predicates: only one active fact per predicate at a time. Asserting a new value
/// supersedes the previous active fact. Set predicates: multiple active facts allowed.
fn is_scalar_predicate(predicate: &str) -> bool {
    // Location, identity, profile, health, diet, work, language — all scalar by prefix
    predicate.starts_with("location.")
        || predicate.starts_with("identity.")
        || predicate.starts_with("profile.")
        || predicate.starts_with("health.")
        || predicate.starts_with("diet.")
        || predicate.starts_with("work.")
        || predicate.starts_with("language.")
        // Procedural memory predicates (§4.2) — all scalar (single active fact per predicate)
        || predicate.starts_with("procedure.")
        || predicate.starts_with("convention.")
        || predicate.starts_with("environment.")
        // Explicit scalar predicates (not covered by prefix blankets)
        || predicate == "preference.communication_style"
        || predicate == "preference.coding_style"
        || predicate == "entities.architecture_decision"
}

/// Project a fact assertion into the facts table.
///
/// Applies conflict resolution per data-model spec §4:
/// - **Scalar predicates** (location.*, identity.*, profile.*, health.*, diet.*, work.*, language.*,
///   preference.communication_style, preference.coding_style, entities.architecture_decision):
///   For Assert/Update, if an existing active fact with same predicate exists, mark it as
///   Superseded and return both the superseded old fact and the new Active fact.
///   For Retract, mark the existing active fact as Retracted.
/// - **Set predicates** (preference.*, project.*):
///   For Assert, if an existing active fact with same predicate AND same raw_value exists,
///   return empty vec (dedup skip). Otherwise return new Active fact.
///   For Retract, mark existing matching fact as Retracted.
///
/// Returns a Vec of Fact records that need to be written to the metadata store.
pub fn project_assertion(a: &FactAssertion, existing_facts: &[Fact], now: &str) -> Vec<Fact> {
    if is_scalar_predicate(&a.predicate) {
        match a.operation {
            FactOperation::Assert | FactOperation::Update => {
                // Find existing active fact with same predicate
                let existing = existing_facts.iter().find(|f| {
                    f.predicate == a.predicate
                        && f.status == FactStatus::Active
                        && f.user_id == a.user_id
                });
                if let Some(old) = existing {
                    // Supersede old fact + insert new Active fact
                    let superseded = Fact {
                        fact_id: old.fact_id.clone(),
                        user_id: old.user_id.clone(),
                        subject: old.subject.clone(),
                        predicate: old.predicate.clone(),
                        display_value: old.display_value.clone(),
                        confidence: old.confidence,
                        status: FactStatus::Superseded,
                        source_assertion_id: old.source_assertion_id.clone(),
                        valid_from: old.valid_from.clone(),
                        valid_to: Some(now.to_owned()),
                        source_episode_ids: old.source_episode_ids.clone(),
                    };
                    vec![superseded, upsert_fact(a)]
                } else {
                    vec![upsert_fact(a)]
                }
            }
            FactOperation::Retract => {
                // Find existing active fact with same predicate and mark Retracted
                let existing = existing_facts.iter().find(|f| {
                    f.predicate == a.predicate
                        && f.status == FactStatus::Active
                        && f.user_id == a.user_id
                });
                if let Some(old) = existing {
                    vec![Fact {
                        fact_id: old.fact_id.clone(),
                        user_id: old.user_id.clone(),
                        subject: old.subject.clone(),
                        predicate: old.predicate.clone(),
                        display_value: old.display_value.clone(),
                        confidence: old.confidence,
                        status: FactStatus::Retracted,
                        source_assertion_id: old.source_assertion_id.clone(),
                        valid_from: old.valid_from.clone(),
                        valid_to: Some(now.to_owned()),
                        source_episode_ids: old.source_episode_ids.clone(),
                    }]
                } else {
                    vec![] // nothing to retract
                }
            }
        }
    } else {
        // Set predicates (preference.*, project.*)
        match a.operation {
            FactOperation::Assert | FactOperation::Update => {
                // Dedup: if existing active fact with same predicate AND same raw_value, skip
                let already_exists = existing_facts.iter().any(|f| {
                    f.predicate == a.predicate
                        && f.status == FactStatus::Active
                        && f.user_id == a.user_id
                        && f.display_value == format_display_value(a)
                });
                if already_exists {
                    vec![] // dedup skip
                } else {
                    vec![upsert_fact(a)]
                }
            }
            FactOperation::Retract => {
                // Find existing active fact with same predicate AND same raw_value
                let existing = existing_facts.iter().find(|f| {
                    f.predicate == a.predicate
                        && f.status == FactStatus::Active
                        && f.user_id == a.user_id
                        && f.display_value == format_display_value(a)
                });
                if let Some(old) = existing {
                    vec![Fact {
                        fact_id: old.fact_id.clone(),
                        user_id: old.user_id.clone(),
                        subject: old.subject.clone(),
                        predicate: old.predicate.clone(),
                        display_value: old.display_value.clone(),
                        confidence: old.confidence,
                        status: FactStatus::Retracted,
                        source_assertion_id: old.source_assertion_id.clone(),
                        valid_from: old.valid_from.clone(),
                        valid_to: Some(now.to_owned()),
                        source_episode_ids: old.source_episode_ids.clone(),
                    }]
                } else {
                    vec![] // nothing to retract
                }
            }
        }
    }
}

/// Build a Fact from an assertion for upsert.
fn upsert_fact(a: &FactAssertion) -> Fact {
    Fact {
        fact_id: fact_id_from_assertion(&a.assertion_id),
        user_id: a.user_id.clone(),
        subject: a.subject.clone(),
        predicate: a.predicate.clone(),
        display_value: format_display_value(a),
        confidence: a.confidence,
        status: FactStatus::Active,
        source_assertion_id: a.assertion_id.clone(),
        valid_from: a.valid_from.clone(),
        valid_to: None,
        source_episode_ids: a.source_episode_ids.clone(),
    }
}

/// Generate a fact ID from an assertion ID.
fn fact_id_from_assertion(assertion_id: &str) -> String {
    if let Some(stripped) = assertion_id.strip_prefix("ast_") {
        format!("fact_{}", stripped)
    } else {
        format!("fact_{}", assertion_id)
    }
}

/// Format display value based on predicate.
/// Matches Go `formatDisplayValue()` with all 21 predicate formats.
pub fn format_display_value(a: &FactAssertion) -> String {
    match a.predicate.as_str() {
        // identity / profile
        "identity.name" | "profile.name" => format!("User's name is {}", a.raw_value_text),
        "identity.pronouns" => format!("User's pronouns are {}", a.raw_value_text),
        "profile.role" | "work.current_role" => format!("User works as {}", a.raw_value_text),
        "profile.location" | "location.current_city" => {
            format!("User currently lives in {}", a.raw_value_text)
        }
        "location.current_country" => {
            format!("User's current country is {}", a.raw_value_text)
        }
        "work.current_company" => format!("User works at {}", a.raw_value_text),
        // health / diet
        "health.allergy" => format!("User is allergic to {}", a.raw_value_text),
        "health.constraint" => {
            format!("User has health-related constraint: {}", a.raw_value_text)
        }
        "diet.spicy_preference" => "User prefers spicy food".to_owned(),
        // preferences
        "preference.food" => format!("User likes eating {}", a.raw_value_text),
        "preference.communication_style" => {
            format!("User prefers communication style: {}", a.raw_value_text)
        }
        "preference.coding_style" => format!("Prefers coding with {}", a.raw_value_text),
        // language
        "language.spoken" => format!("User speaks {}", a.raw_value_text),
        // entities / projects
        "project.active" | "entities.project" => {
            format!("User is working on {}", a.raw_value_text)
        }
        "entities.architecture_decision" => {
            format!("Current architecture decision: {}", a.raw_value_text)
        }
        // Procedural memory (§4.2)
        "procedure.build_command" => format!("Build command: {}", a.raw_value_text),
        "procedure.test_command" => format!("Test command: {}", a.raw_value_text),
        "procedure.deploy_step" => format!("Deploy step: {}", a.raw_value_text),
        "convention.tool" => format!("Project uses {}", a.raw_value_text),
        "convention.naming" => format!("Naming convention: {}", a.raw_value_text),
        "environment.ci" => format!("CI/CD platform: {}", a.raw_value_text),
        "environment.runtime" => format!("Runtime requirement: {}", a.raw_value_text),
        // cases / events — use raw value without predicate prefix
        p if p.starts_with("cases.") || p.starts_with("events.") => a.raw_value_text.clone(),
        // All other unrecognized predicates — preserve predicate for context
        _ => format!("{}: {}", a.predicate, a.raw_value_text),
    }
}

/// Filter facts below minimum confidence threshold.
pub fn filter_facts_for_injection(facts: &[crate::FactEntry]) -> Vec<crate::FactEntry> {
    facts
        .iter()
        .filter(|f| f.confidence >= crate::MIN_INJECTED_FACT_CONFIDENCE)
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FactOperation;

    #[test]
    fn project_assertion_creates_fact() {
        let assertion = FactAssertion {
            assertion_id: "ast_test123".to_owned(),
            user_id: "u1".to_owned(),
            subject: "user".to_owned(),
            predicate: "location.current_city".to_owned(),
            raw_value_text: "Tokyo".to_owned(),
            value_type: "scalar".to_owned(),
            operation: FactOperation::Assert,
            confidence: 0.75,
            valid_from: None,
            valid_to: None,
            source_turn_id: Some("t1".to_owned()),
            source_episode_ids: None,
            extractor_version: "v1-rules".to_owned(),
        };
        let facts = project_assertion(&assertion, &[], "2026-05-02T12:00:00Z");
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].predicate, "location.current_city");
        assert_eq!(facts[0].display_value, "User currently lives in Tokyo");
        assert_eq!(facts[0].status, crate::FactStatus::Active);
    }

    #[test]
    fn project_retract_creates_retracted_fact() {
        // Create an existing active fact to retract
        let existing = Fact {
            fact_id: "fact_ret123".to_owned(),
            user_id: "u1".to_owned(),
            subject: "user".to_owned(),
            predicate: "diet.spicy_preference".to_owned(),
            display_value: "User prefers spicy food".to_owned(),
            confidence: 0.85,
            status: FactStatus::Active,
            source_assertion_id: "ast_old".to_owned(),
            valid_from: None,
            valid_to: None,
            source_episode_ids: None,
        };
        let assertion = FactAssertion {
            assertion_id: "ast_ret123".to_owned(),
            user_id: "u1".to_owned(),
            subject: "user".to_owned(),
            predicate: "diet.spicy_preference".to_owned(),
            raw_value_text: "retracted".to_owned(),
            value_type: "temporal".to_owned(),
            operation: FactOperation::Retract,
            confidence: 0.85,
            valid_from: Some("1234567890".to_owned()),
            valid_to: None,
            source_turn_id: Some("t2".to_owned()),
            source_episode_ids: None,
            extractor_version: "v1-rules".to_owned(),
        };
        let facts = project_assertion(&assertion, &[existing], "2026-05-02T12:00:00Z");
        assert!(!facts.is_empty());
        assert_eq!(facts[0].status, crate::FactStatus::Retracted);
    }

    #[test]
    fn format_display_value_all_predicates() {
        let test_cases = vec![
            (
                "location.current_city",
                "Tokyo",
                "User currently lives in Tokyo",
            ),
            ("identity.name", "Alice", "User's name is Alice"),
            ("work.current_role", "engineer", "User works as engineer"),
            ("work.current_company", "Acme", "User works at Acme"),
            ("health.allergy", "peanuts", "User is allergic to peanuts"),
            (
                "health.constraint",
                "gluten",
                "User has health-related constraint: gluten",
            ),
            (
                "diet.spicy_preference",
                "retracted",
                "User prefers spicy food",
            ),
            ("preference.food", "pizza", "User likes eating pizza"),
            (
                "preference.communication_style",
                "concise",
                "User prefers communication style: concise",
            ),
            (
                "identity.pronouns",
                "she/her",
                "User's pronouns are she/her",
            ),
            ("language.spoken", "Japanese", "User speaks Japanese"),
            ("project.active", "web app", "User is working on web app"),
        ];

        for (predicate, value, expected) in test_cases {
            let a = FactAssertion {
                assertion_id: "ast_test".to_owned(),
                user_id: "u1".to_owned(),
                subject: "user".to_owned(),
                predicate: predicate.to_owned(),
                raw_value_text: value.to_owned(),
                value_type: "scalar".to_owned(),
                operation: FactOperation::Assert,
                confidence: 0.8,
                valid_from: None,
                valid_to: None,
                source_turn_id: None,
                source_episode_ids: None,
                extractor_version: "v1-rules".to_owned(),
            };
            assert_eq!(format_display_value(&a), expected);
        }
    }

    #[test]
    fn filter_facts_by_confidence() {
        let facts = vec![
            crate::FactEntry {
                fact_id: "f1".to_owned(),
                predicate: "test".to_owned(),
                display_value: "low".to_owned(),
                confidence: 0.3,
                staleness_note: None,
                valid_from: None,
            },
            crate::FactEntry {
                fact_id: "f2".to_owned(),
                predicate: "test".to_owned(),
                display_value: "high".to_owned(),
                confidence: 0.9,
                staleness_note: None,
                valid_from: None,
            },
        ];
        let filtered = filter_facts_for_injection(&facts);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].confidence, 0.9);
    }

    #[test]
    fn entities_architecture_decision_is_scalar() {
        // Scalar predicates
        assert!(is_scalar_predicate("entities.architecture_decision"));
        assert!(is_scalar_predicate("preference.coding_style"));
        assert!(is_scalar_predicate("preference.communication_style"));
        assert!(is_scalar_predicate("location.current_city"));
        assert!(is_scalar_predicate("identity.name"));
        assert!(is_scalar_predicate("profile.name"));
        assert!(is_scalar_predicate("profile.role"));
        // Set predicates are NOT scalar
        assert!(!is_scalar_predicate("project.active"));
        assert!(!is_scalar_predicate("entities.project"));
        assert!(!is_scalar_predicate("preference.food"));
        assert!(!is_scalar_predicate("cases.bug_fix"));
    }

    #[test]
    fn format_display_value_new_predicates() {
        let test_cases = vec![
            (
                "entities.architecture_decision",
                "tRPC",
                "Current architecture decision: tRPC",
            ),
            (
                "preference.coding_style",
                "Rust",
                "Prefers coding with Rust",
            ),
        ];

        for (predicate, value, expected) in test_cases {
            let a = FactAssertion {
                assertion_id: "ast_test".to_owned(),
                user_id: "u1".to_owned(),
                subject: "user".to_owned(),
                predicate: predicate.to_owned(),
                raw_value_text: value.to_owned(),
                value_type: "scalar".to_owned(),
                operation: FactOperation::Assert,
                confidence: 0.8,
                valid_from: None,
                valid_to: None,
                source_turn_id: None,
                source_episode_ids: None,
                extractor_version: "v1-rules".to_owned(),
            };
            assert_eq!(format_display_value(&a), expected);
        }
    }

    #[test]
    fn project_assertion_supersedes_architecture_decision() {
        let existing = Fact {
            fact_id: "f1".to_owned(),
            user_id: "u1".to_owned(),
            subject: "user".to_owned(),
            predicate: "entities.architecture_decision".to_owned(),
            display_value: "Current architecture decision: REST".to_owned(),
            confidence: 0.85,
            status: FactStatus::Active,
            source_assertion_id: "ast_old".to_owned(),
            valid_from: None,
            valid_to: None,
            source_episode_ids: None,
        };
        let assertion = FactAssertion {
            assertion_id: "ast_new".to_owned(),
            user_id: "u1".to_owned(),
            subject: "user".to_owned(),
            predicate: "entities.architecture_decision".to_owned(),
            raw_value_text: "GraphQL".to_owned(),
            value_type: "scalar".to_owned(),
            operation: FactOperation::Update,
            confidence: 0.90,
            valid_from: Some("1234567890".to_owned()),
            valid_to: None,
            source_turn_id: Some("t2".to_owned()),
            source_episode_ids: None,
            extractor_version: "v1-rules".to_owned(),
        };
        let facts = project_assertion(&assertion, &[existing], "2026-05-02T12:00:00Z");
        assert_eq!(facts.len(), 2);
        assert_eq!(facts[0].status, FactStatus::Superseded);
        assert_eq!(facts[1].predicate, "entities.architecture_decision");
        assert_eq!(
            facts[1].display_value,
            "Current architecture decision: GraphQL"
        );
        assert_eq!(facts[1].status, FactStatus::Active);
    }

    #[test]
    fn entities_project_is_set_not_scalar() {
        let first = FactAssertion {
            assertion_id: "ast_1".to_owned(),
            user_id: "u1".to_owned(),
            subject: "user".to_owned(),
            predicate: "entities.project".to_owned(),
            raw_value_text: "web-app".to_owned(),
            value_type: "set".to_owned(),
            operation: FactOperation::Assert,
            confidence: 0.70,
            valid_from: None,
            valid_to: None,
            source_turn_id: None,
            source_episode_ids: None,
            extractor_version: "v1-rules".to_owned(),
        };
        let second = FactAssertion {
            assertion_id: "ast_2".to_owned(),
            user_id: "u1".to_owned(),
            subject: "user".to_owned(),
            predicate: "entities.project".to_owned(),
            raw_value_text: "mobile-app".to_owned(),
            value_type: "set".to_owned(),
            operation: FactOperation::Assert,
            confidence: 0.70,
            valid_from: None,
            valid_to: None,
            source_turn_id: None,
            source_episode_ids: None,
            extractor_version: "v1-rules".to_owned(),
        };
        let first_result = project_assertion(&first, &[], "2026-05-02T12:00:00Z");
        assert_eq!(first_result.len(), 1);
        assert_eq!(first_result[0].status, FactStatus::Active);
        let second_result = project_assertion(&second, &first_result, "2026-05-02T12:00:00Z");
        assert_eq!(second_result.len(), 1);
        assert_eq!(
            second_result[0].display_value,
            "User is working on mobile-app"
        );
        assert_eq!(second_result[0].status, FactStatus::Active);
    }

    #[test]
    fn procedural_predicates_are_scalar() {
        assert!(is_scalar_predicate("procedure.build_command"));
        assert!(is_scalar_predicate("procedure.test_command"));
        assert!(is_scalar_predicate("procedure.deploy_step"));
        assert!(is_scalar_predicate("convention.tool"));
        assert!(is_scalar_predicate("convention.naming"));
        assert!(is_scalar_predicate("environment.ci"));
        assert!(is_scalar_predicate("environment.runtime"));
    }

    #[test]
    fn format_display_value_procedural_predicates() {
        let test_cases = vec![
            (
                "procedure.build_command",
                "cargo build",
                "Build command: cargo build",
            ),
            (
                "procedure.test_command",
                "cargo test",
                "Test command: cargo test",
            ),
            (
                "procedure.deploy_step",
                "docker push then deploy",
                "Deploy step: docker push then deploy",
            ),
            ("convention.tool", "React", "Project uses React"),
            (
                "convention.naming",
                "snake_case",
                "Naming convention: snake_case",
            ),
            (
                "environment.ci",
                "GitHub Actions",
                "CI/CD platform: GitHub Actions",
            ),
            (
                "environment.runtime",
                "Node 22",
                "Runtime requirement: Node 22",
            ),
        ];

        for (predicate, value, expected) in test_cases {
            let a = FactAssertion {
                assertion_id: "ast_test".to_owned(),
                user_id: "u1".to_owned(),
                subject: "user".to_owned(),
                predicate: predicate.to_owned(),
                raw_value_text: value.to_owned(),
                value_type: "scalar".to_owned(),
                operation: FactOperation::Assert,
                confidence: 0.8,
                valid_from: None,
                valid_to: None,
                source_turn_id: None,
                source_episode_ids: None,
                extractor_version: "v1-rules".to_owned(),
            };
            assert_eq!(format_display_value(&a), expected);
        }
    }

    #[test]
    fn project_assertion_supersedes_procedural_predicate() {
        let existing = Fact {
            fact_id: "f1".to_owned(),
            user_id: "u1".to_owned(),
            subject: "user".to_owned(),
            predicate: "procedure.build_command".to_owned(),
            display_value: "Build command: make".to_owned(),
            confidence: 0.82,
            status: FactStatus::Active,
            source_assertion_id: "ast_old".to_owned(),
            valid_from: None,
            valid_to: None,
            source_episode_ids: None,
        };
        let assertion = FactAssertion {
            assertion_id: "ast_new".to_owned(),
            user_id: "u1".to_owned(),
            subject: "user".to_owned(),
            predicate: "procedure.build_command".to_owned(),
            raw_value_text: "cargo build".to_owned(),
            value_type: "scalar".to_owned(),
            operation: FactOperation::Assert,
            confidence: 0.82,
            valid_from: Some("1234567890".to_owned()),
            valid_to: None,
            source_turn_id: Some("t2".to_owned()),
            source_episode_ids: None,
            extractor_version: "v1-rules".to_owned(),
        };
        let facts = project_assertion(&assertion, &[existing], "2026-05-02T12:00:00Z");
        assert_eq!(facts.len(), 2);
        assert_eq!(facts[0].status, FactStatus::Superseded);
        assert_eq!(facts[1].predicate, "procedure.build_command");
        assert_eq!(facts[1].display_value, "Build command: cargo build");
        assert_eq!(facts[1].status, FactStatus::Active);
    }

    #[test]
    fn project_assertion_assert_fills_valid_from_and_source_episode_ids() {
        let assertion = FactAssertion {
            assertion_id: "ast_temp123".to_owned(),
            user_id: "u1".to_owned(),
            subject: "user".to_owned(),
            predicate: "location.current_city".to_owned(),
            raw_value_text: "Tokyo".to_owned(),
            value_type: "scalar".to_owned(),
            operation: FactOperation::Assert,
            confidence: 0.75,
            valid_from: Some("2026-05-02T12:00:00Z".to_owned()),
            valid_to: None,
            source_turn_id: Some("t1".to_owned()),
            source_episode_ids: Some(vec!["ep-42".to_owned()]),
            extractor_version: "v1-rules".to_owned(),
        };
        let facts = project_assertion(&assertion, &[], "2026-05-02T12:00:00Z");
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].valid_from, Some("2026-05-02T12:00:00Z".to_owned()));
        assert_eq!(facts[0].source_episode_ids, Some(vec!["ep-42".to_owned()]));
        assert_eq!(facts[0].valid_to, None);
    }

    #[test]
    fn project_assertion_supersede_fills_valid_to() {
        let existing = Fact {
            fact_id: "f_old".to_owned(),
            user_id: "u1".to_owned(),
            subject: "user".to_owned(),
            predicate: "location.current_city".to_owned(),
            display_value: "User currently lives in Paris".to_owned(),
            confidence: 0.85,
            status: FactStatus::Active,
            source_assertion_id: "ast_old".to_owned(),
            valid_from: Some("2026-01-01T00:00:00Z".to_owned()),
            valid_to: None,
            source_episode_ids: Some(vec!["ep-old".to_owned()]),
        };
        let assertion = FactAssertion {
            assertion_id: "ast_new".to_owned(),
            user_id: "u1".to_owned(),
            subject: "user".to_owned(),
            predicate: "location.current_city".to_owned(),
            raw_value_text: "Tokyo".to_owned(),
            value_type: "scalar".to_owned(),
            operation: FactOperation::Update,
            confidence: 0.90,
            valid_from: Some("2026-05-02T12:00:00Z".to_owned()),
            valid_to: None,
            source_turn_id: Some("t2".to_owned()),
            source_episode_ids: Some(vec!["ep-42".to_owned()]),
            extractor_version: "v1-rules".to_owned(),
        };
        let facts = project_assertion(&assertion, &[existing], "2026-05-02T12:00:00Z");
        assert_eq!(facts.len(), 2);
        assert_eq!(facts[0].status, FactStatus::Superseded);
        assert_eq!(facts[0].valid_to, Some("2026-05-02T12:00:00Z".to_owned()));
        assert_eq!(facts[0].source_episode_ids, Some(vec!["ep-old".to_owned()]));
        assert_eq!(facts[1].status, FactStatus::Active);
        assert_eq!(facts[1].valid_from, Some("2026-05-02T12:00:00Z".to_owned()));
        assert_eq!(facts[1].valid_to, None);
        assert_eq!(facts[1].source_episode_ids, Some(vec!["ep-42".to_owned()]));
    }

    #[test]
    fn project_assertion_retract_fills_valid_to() {
        let existing = Fact {
            fact_id: "f_ret".to_owned(),
            user_id: "u1".to_owned(),
            subject: "user".to_owned(),
            predicate: "diet.spicy_preference".to_owned(),
            display_value: "User prefers spicy food".to_owned(),
            confidence: 0.85,
            status: FactStatus::Active,
            source_assertion_id: "ast_old".to_owned(),
            valid_from: Some("2026-01-01T00:00:00Z".to_owned()),
            valid_to: None,
            source_episode_ids: Some(vec!["ep-old".to_owned()]),
        };
        let assertion = FactAssertion {
            assertion_id: "ast_ret".to_owned(),
            user_id: "u1".to_owned(),
            subject: "user".to_owned(),
            predicate: "diet.spicy_preference".to_owned(),
            raw_value_text: "retracted".to_owned(),
            value_type: "temporal".to_owned(),
            operation: FactOperation::Retract,
            confidence: 0.85,
            valid_from: Some("2026-05-02T12:00:00Z".to_owned()),
            valid_to: None,
            source_turn_id: Some("t2".to_owned()),
            source_episode_ids: Some(vec!["ep-42".to_owned()]),
            extractor_version: "v1-rules".to_owned(),
        };
        let facts = project_assertion(&assertion, &[existing], "2026-05-02T12:00:00Z");
        assert!(!facts.is_empty());
        assert_eq!(facts[0].status, FactStatus::Retracted);
        assert_eq!(facts[0].valid_to, Some("2026-05-02T12:00:00Z".to_owned()));
        assert_eq!(facts[0].source_episode_ids, Some(vec!["ep-old".to_owned()]));
    }
}
