use mfs_session::{
    MemoryCategory, MemoryDecision, MemoryOwnership, MemoryRecord, deterministic_extract,
    deterministic_merge,
};

#[test]
fn memory_taxonomy_covers_all_eight_categories() {
    let categories = MemoryCategory::all();
    assert_eq!(categories.len(), 8);
    assert!(categories.contains(&MemoryCategory::Profile));
    assert!(categories.contains(&MemoryCategory::Preferences));
    assert!(categories.contains(&MemoryCategory::Entities));
    assert!(categories.contains(&MemoryCategory::Events));
    assert!(categories.contains(&MemoryCategory::Cases));
    assert!(categories.contains(&MemoryCategory::Patterns));
    assert!(categories.contains(&MemoryCategory::Tools));
    assert!(categories.contains(&MemoryCategory::Skills));
}

#[test]
fn each_category_has_stable_ownership_and_merge_policy() {
    assert_eq!(MemoryCategory::Profile.ownership(), MemoryOwnership::User);
    assert!(MemoryCategory::Profile.is_mergeable());
    assert_eq!(MemoryCategory::Cases.ownership(), MemoryOwnership::Agent);
    assert!(!MemoryCategory::Cases.is_mergeable());
}

#[test]
fn deterministic_extraction_can_emit_user_preference_and_skill_candidates() {
    let candidates = deterministic_extract(
        &[(
            "user".to_owned(),
            "I prefer short API examples when reading docs".to_owned(),
        )],
        &[(
            "skill".to_owned(),
            "mfs://agent/skills/search".to_owned(),
            Some(true),
        )],
    );

    assert!(
        candidates
            .iter()
            .any(|candidate| candidate.category == MemoryCategory::Preferences)
    );
    assert!(
        candidates
            .iter()
            .any(|candidate| candidate.category == MemoryCategory::Skills)
    );
}

#[test]
fn deterministic_extraction_can_emit_user_profile_candidates() {
    let candidates = deterministic_extract(
        &[("user".to_owned(), "My name is Alice Example".to_owned())],
        &[],
    );

    assert!(
        candidates
            .iter()
            .any(|candidate| candidate.category == MemoryCategory::Profile)
    );
}

#[test]
fn deterministic_extraction_can_emit_event_and_case_candidates() {
    let candidates = deterministic_extract(
        &[
            (
                "user".to_owned(),
                "We decided to use SQLite for metadata storage".to_owned(),
            ),
            (
                "assistant".to_owned(),
                "Resolved oauth incident by rotating refresh tokens".to_owned(),
            ),
        ],
        &[],
    );

    assert!(
        candidates
            .iter()
            .any(|candidate| candidate.category == MemoryCategory::Events)
    );
    assert!(
        candidates
            .iter()
            .any(|candidate| candidate.category == MemoryCategory::Cases)
    );
}

#[test]
fn deterministic_extraction_can_emit_entity_candidates() {
    let candidates = deterministic_extract(
        &[(
            "user".to_owned(),
            "Project Atlas uses Rust for the control plane".to_owned(),
        )],
        &[],
    );

    assert!(
        candidates
            .iter()
            .any(|candidate| candidate.category == MemoryCategory::Entities)
    );
}

#[test]
fn deterministic_extraction_can_emit_pattern_candidates() {
    let candidates = deterministic_extract(
        &[(
            "assistant".to_owned(),
            "Use MFS search then inspect overview before grep".to_owned(),
        )],
        &[],
    );

    assert!(
        candidates
            .iter()
            .any(|candidate| candidate.category == MemoryCategory::Patterns)
    );
}

#[test]
fn deterministic_mergeable_categories_prefer_merge_over_duplicate_create() {
    let existing = vec![MemoryRecord::for_test(
        MemoryCategory::Preferences,
        "short API examples",
    )];
    let decision = deterministic_merge(
        MemoryCategory::Preferences,
        "I prefer short API examples",
        &existing,
    );

    assert_eq!(decision.primary, MemoryDecision::Merge);
}

#[test]
fn non_mergeable_categories_keep_new_case_records() {
    let existing = vec![MemoryRecord::for_test(
        MemoryCategory::Cases,
        "oauth incident recovery",
    )];
    let decision = deterministic_merge(
        MemoryCategory::Cases,
        "oauth rollout issue resolved",
        &existing,
    );

    assert_eq!(decision.primary, MemoryDecision::Create);
}

#[test]
fn deterministic_skill_decisions_merge_same_skill_even_when_status_changes() {
    let existing = vec![MemoryRecord::for_test(
        MemoryCategory::Skills,
        "mfs://agent/skills/search success=true",
    )];
    let decision = deterministic_merge(
        MemoryCategory::Skills,
        "mfs://agent/skills/search success=false",
        &existing,
    );

    assert_eq!(decision.primary, MemoryDecision::Merge);
}
