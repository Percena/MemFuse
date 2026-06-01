use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MemoryCategory {
    Profile,
    Preferences,
    Entities,
    Events,
    Cases,
    Patterns,
    Tools,
    Skills,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MemoryOwnership {
    User,
    Agent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MemoryDecision {
    Create,
    Merge,
    Delete,
    Skip,
}

/// A memory candidate extracted from a session.
///
/// Contains L0/L1/L2 three-level content structure matching MemFuse's
/// abstract/overview/content hierarchy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryCandidate {
    pub category: MemoryCategory,
    /// Stable merge key for mergeable categories, e.g. "Python code style: No type hints"
    pub title: String,
    /// L0: one-line abstract / index layer
    pub abstract_text: String,
    /// L1: structured markdown overview
    pub overview_text: String,
    /// L2: full narrative content (legacy `content` field)
    pub content: String,
    /// Source evidence (raw message text that triggered extraction)
    pub evidence: String,
    /// For Tools category: the exact tool name
    pub tool_name: Option<String>,
    /// For Skills category: the exact skill name
    pub skill_name: Option<String>,
}

impl MemoryCandidate {
    /// Build a simple candidate (used by deterministic fallback extractor).
    pub fn simple(
        category: MemoryCategory,
        title: impl Into<String>,
        content: impl Into<String>,
        evidence: impl Into<String>,
    ) -> Self {
        let title = title.into();
        let content = content.into();
        let evidence = evidence.into();
        Self {
            category,
            abstract_text: format!("{}: {}", category.slug(), title),
            overview_text: format!("## {}\n\n{}", category.display_name(), content),
            content: content.clone(),
            title,
            evidence,
            tool_name: None,
            skill_name: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryRecord {
    pub category: MemoryCategory,
    pub uri: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryMergeDecision {
    pub primary: MemoryDecision,
    pub target_uri: Option<String>,
    pub delete_uris: Vec<String>,
}

impl MemoryCategory {
    pub fn all() -> [Self; 8] {
        [
            Self::Profile,
            Self::Preferences,
            Self::Entities,
            Self::Events,
            Self::Cases,
            Self::Patterns,
            Self::Tools,
            Self::Skills,
        ]
    }

    pub fn ownership(self) -> MemoryOwnership {
        match self {
            Self::Profile | Self::Preferences | Self::Entities | Self::Events => {
                MemoryOwnership::User
            }
            Self::Cases | Self::Patterns | Self::Tools | Self::Skills => MemoryOwnership::Agent,
        }
    }

    pub fn is_mergeable(self) -> bool {
        matches!(
            self,
            Self::Profile
                | Self::Preferences
                | Self::Entities
                | Self::Patterns
                | Self::Tools
                | Self::Skills
        )
    }

    pub fn slug(self) -> &'static str {
        match self {
            Self::Profile => "profile",
            Self::Preferences => "preferences",
            Self::Entities => "entities",
            Self::Events => "events",
            Self::Cases => "cases",
            Self::Patterns => "patterns",
            Self::Tools => "tools",
            Self::Skills => "skills",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Profile => "Profile Memory",
            Self::Preferences => "Preferences Memory",
            Self::Entities => "Entity Memory",
            Self::Events => "Event Memory",
            Self::Cases => "Case Memory",
            Self::Patterns => "Pattern Memory",
            Self::Tools => "Tool Memory",
            Self::Skills => "Skill Memory",
        }
    }
}

impl MemoryRecord {
    pub fn for_test(category: MemoryCategory, content: &str) -> Self {
        Self {
            category,
            uri: format!("mfs://memory/{}", category.slug()),
            content: content.to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── MemoryCategory ──────────────────────────────────────────────────

    #[test]
    fn category_all_returns_eight_variants() {
        let all = MemoryCategory::all();
        assert_eq!(all.len(), 8);
        // Verify each variant appears exactly once.
        assert!(all.contains(&MemoryCategory::Profile));
        assert!(all.contains(&MemoryCategory::Preferences));
        assert!(all.contains(&MemoryCategory::Entities));
        assert!(all.contains(&MemoryCategory::Events));
        assert!(all.contains(&MemoryCategory::Cases));
        assert!(all.contains(&MemoryCategory::Patterns));
        assert!(all.contains(&MemoryCategory::Tools));
        assert!(all.contains(&MemoryCategory::Skills));
    }

    #[test]
    fn category_slug_matches_expected_strings() {
        assert_eq!(MemoryCategory::Profile.slug(), "profile");
        assert_eq!(MemoryCategory::Preferences.slug(), "preferences");
        assert_eq!(MemoryCategory::Entities.slug(), "entities");
        assert_eq!(MemoryCategory::Events.slug(), "events");
        assert_eq!(MemoryCategory::Cases.slug(), "cases");
        assert_eq!(MemoryCategory::Patterns.slug(), "patterns");
        assert_eq!(MemoryCategory::Tools.slug(), "tools");
        assert_eq!(MemoryCategory::Skills.slug(), "skills");
    }

    #[test]
    fn category_display_name_matches_expected_strings() {
        assert_eq!(MemoryCategory::Profile.display_name(), "Profile Memory");
        assert_eq!(
            MemoryCategory::Preferences.display_name(),
            "Preferences Memory"
        );
        assert_eq!(MemoryCategory::Entities.display_name(), "Entity Memory");
        assert_eq!(MemoryCategory::Events.display_name(), "Event Memory");
        assert_eq!(MemoryCategory::Cases.display_name(), "Case Memory");
        assert_eq!(MemoryCategory::Patterns.display_name(), "Pattern Memory");
        assert_eq!(MemoryCategory::Tools.display_name(), "Tool Memory");
        assert_eq!(MemoryCategory::Skills.display_name(), "Skill Memory");
    }

    #[test]
    fn category_ownership_user_categories() {
        assert_eq!(MemoryCategory::Profile.ownership(), MemoryOwnership::User);
        assert_eq!(
            MemoryCategory::Preferences.ownership(),
            MemoryOwnership::User
        );
        assert_eq!(MemoryCategory::Entities.ownership(), MemoryOwnership::User);
        assert_eq!(MemoryCategory::Events.ownership(), MemoryOwnership::User);
    }

    #[test]
    fn category_ownership_agent_categories() {
        assert_eq!(MemoryCategory::Cases.ownership(), MemoryOwnership::Agent);
        assert_eq!(MemoryCategory::Patterns.ownership(), MemoryOwnership::Agent);
        assert_eq!(MemoryCategory::Tools.ownership(), MemoryOwnership::Agent);
        assert_eq!(MemoryCategory::Skills.ownership(), MemoryOwnership::Agent);
    }

    #[test]
    fn category_mergeable_true() {
        assert!(MemoryCategory::Profile.is_mergeable());
        assert!(MemoryCategory::Preferences.is_mergeable());
        assert!(MemoryCategory::Entities.is_mergeable());
        assert!(MemoryCategory::Patterns.is_mergeable());
        assert!(MemoryCategory::Tools.is_mergeable());
        assert!(MemoryCategory::Skills.is_mergeable());
    }

    #[test]
    fn category_mergeable_false() {
        assert!(!MemoryCategory::Events.is_mergeable());
        assert!(!MemoryCategory::Cases.is_mergeable());
    }

    #[test]
    fn category_serde_roundtrip() {
        for cat in MemoryCategory::all() {
            let json = serde_json::to_string(&cat).unwrap();
            let back: MemoryCategory = serde_json::from_str(&json).unwrap();
            assert_eq!(cat, back);
        }
    }

    #[test]
    fn category_serde_json_values() {
        // Serde default for these enums (no rename_all) is PascalCase.
        assert_eq!(
            serde_json::to_string(&MemoryCategory::Profile).unwrap(),
            "\"Profile\""
        );
        assert_eq!(
            serde_json::to_string(&MemoryCategory::Preferences).unwrap(),
            "\"Preferences\""
        );
        assert_eq!(
            serde_json::to_string(&MemoryCategory::Entities).unwrap(),
            "\"Entities\""
        );
        assert_eq!(
            serde_json::to_string(&MemoryCategory::Events).unwrap(),
            "\"Events\""
        );
        assert_eq!(
            serde_json::to_string(&MemoryCategory::Cases).unwrap(),
            "\"Cases\""
        );
        assert_eq!(
            serde_json::to_string(&MemoryCategory::Patterns).unwrap(),
            "\"Patterns\""
        );
        assert_eq!(
            serde_json::to_string(&MemoryCategory::Tools).unwrap(),
            "\"Tools\""
        );
        assert_eq!(
            serde_json::to_string(&MemoryCategory::Skills).unwrap(),
            "\"Skills\""
        );
    }

    #[test]
    fn category_serde_deserialize_from_json() {
        let cat: MemoryCategory = serde_json::from_str("\"Profile\"").unwrap();
        assert_eq!(cat, MemoryCategory::Profile);
        let cat: MemoryCategory = serde_json::from_str("\"Skills\"").unwrap();
        assert_eq!(cat, MemoryCategory::Skills);
    }

    // ─── MemoryOwnership ─────────────────────────────────────────────────

    #[test]
    fn ownership_variants_equality() {
        assert_eq!(MemoryOwnership::User, MemoryOwnership::User);
        assert_eq!(MemoryOwnership::Agent, MemoryOwnership::Agent);
        assert_ne!(MemoryOwnership::User, MemoryOwnership::Agent);
    }

    #[test]
    fn ownership_serde_roundtrip() {
        for own in [MemoryOwnership::User, MemoryOwnership::Agent] {
            let json = serde_json::to_string(&own).unwrap();
            let back: MemoryOwnership = serde_json::from_str(&json).unwrap();
            assert_eq!(own, back);
        }
    }

    #[test]
    fn ownership_serde_json_values() {
        assert_eq!(
            serde_json::to_string(&MemoryOwnership::User).unwrap(),
            "\"User\""
        );
        assert_eq!(
            serde_json::to_string(&MemoryOwnership::Agent).unwrap(),
            "\"Agent\""
        );
    }

    // ─── MemoryDecision ──────────────────────────────────────────────────

    #[test]
    fn decision_variants_distinct() {
        assert_ne!(MemoryDecision::Create, MemoryDecision::Merge);
        assert_ne!(MemoryDecision::Merge, MemoryDecision::Delete);
        assert_ne!(MemoryDecision::Delete, MemoryDecision::Skip);
        assert_ne!(MemoryDecision::Create, MemoryDecision::Skip);
    }

    #[test]
    fn decision_serde_roundtrip() {
        for dec in [
            MemoryDecision::Create,
            MemoryDecision::Merge,
            MemoryDecision::Delete,
            MemoryDecision::Skip,
        ] {
            let json = serde_json::to_string(&dec).unwrap();
            let back: MemoryDecision = serde_json::from_str(&json).unwrap();
            assert_eq!(dec, back);
        }
    }

    #[test]
    fn decision_serde_json_values() {
        assert_eq!(
            serde_json::to_string(&MemoryDecision::Create).unwrap(),
            "\"Create\""
        );
        assert_eq!(
            serde_json::to_string(&MemoryDecision::Merge).unwrap(),
            "\"Merge\""
        );
        assert_eq!(
            serde_json::to_string(&MemoryDecision::Delete).unwrap(),
            "\"Delete\""
        );
        assert_eq!(
            serde_json::to_string(&MemoryDecision::Skip).unwrap(),
            "\"Skip\""
        );
    }

    // ─── MemoryCandidate ─────────────────────────────────────────────────

    #[test]
    fn candidate_simple_constructs_basic_fields() {
        let c = MemoryCandidate::simple(
            MemoryCategory::Preferences,
            "user_preference",
            "dark mode over light mode",
            "I prefer dark mode over light mode.",
        );
        assert_eq!(c.category, MemoryCategory::Preferences);
        assert_eq!(c.title, "user_preference");
        assert_eq!(c.content, "dark mode over light mode");
        assert_eq!(c.evidence, "I prefer dark mode over light mode.");
        assert_eq!(c.abstract_text, "preferences: user_preference");
        assert_eq!(
            c.overview_text,
            "## Preferences Memory\n\ndark mode over light mode"
        );
        assert_eq!(c.tool_name, None);
        assert_eq!(c.skill_name, None);
    }

    #[test]
    fn candidate_simple_abstract_text_format() {
        let c = MemoryCandidate::simple(
            MemoryCategory::Profile,
            "user_name",
            "Alice",
            "My name is Alice",
        );
        // abstract_text = "{slug}: {title}"
        assert_eq!(c.abstract_text, "profile: user_name");
    }

    #[test]
    fn candidate_simple_overview_text_format() {
        let c = MemoryCandidate::simple(
            MemoryCategory::Events,
            "session_event",
            "deployed v2",
            "We decided to deploy v2.",
        );
        // overview_text = "## {display_name}\n\n{content}"
        assert_eq!(c.overview_text, "## Event Memory\n\ndeployed v2");
    }

    #[test]
    fn candidate_simple_content_equals_passed_content() {
        let c = MemoryCandidate::simple(
            MemoryCategory::Entities,
            "project-alpha",
            "Alpha is a Rust project",
            "Project Alpha is a Rust project.",
        );
        assert_eq!(c.content, "Alpha is a Rust project");
    }

    #[test]
    fn candidate_serde_roundtrip() {
        let c = MemoryCandidate::simple(
            MemoryCategory::Profile,
            "user_name",
            "Bob",
            "My name is Bob.",
        );
        let json = serde_json::to_string(&c).unwrap();
        let back: MemoryCandidate = serde_json::from_str(&json).unwrap();
        assert_eq!(c, back);
    }

    #[test]
    fn candidate_with_optional_fields_serde_roundtrip() {
        let c = MemoryCandidate {
            category: MemoryCategory::Tools,
            title: "cargo".to_owned(),
            abstract_text: "tools: cargo".to_owned(),
            overview_text: "## Tool Memory\n\ncargo build".to_owned(),
            content: "cargo build".to_owned(),
            evidence: "ran cargo build".to_owned(),
            tool_name: Some("cargo".to_owned()),
            skill_name: None,
        };
        let json = serde_json::to_string(&c).unwrap();
        let back: MemoryCandidate = serde_json::from_str(&json).unwrap();
        assert_eq!(c, back);
        assert_eq!(back.tool_name, Some("cargo".to_owned()));
        assert_eq!(back.skill_name, None);
    }

    #[test]
    fn candidate_simple_tool_and_skill_names_are_none() {
        let c = MemoryCandidate::simple(MemoryCategory::Tools, "test_tool", "content", "evidence");
        assert_eq!(c.tool_name, None);
        assert_eq!(c.skill_name, None);
    }

    // ─── MemoryRecord ────────────────────────────────────────────────────

    #[test]
    fn record_for_test_constructs_uri_from_slug() {
        let r = MemoryRecord::for_test(MemoryCategory::Profile, "Alice");
        assert_eq!(r.category, MemoryCategory::Profile);
        assert_eq!(r.uri, "mfs://memory/profile");
        assert_eq!(r.content, "Alice");
    }

    #[test]
    fn record_for_test_each_category_slug() {
        for cat in MemoryCategory::all() {
            let r = MemoryRecord::for_test(cat, "test content");
            assert_eq!(r.uri, format!("mfs://memory/{}", cat.slug()));
        }
    }

    #[test]
    fn record_serde_roundtrip() {
        let r = MemoryRecord::for_test(MemoryCategory::Preferences, "dark mode");
        let json = serde_json::to_string(&r).unwrap();
        let back: MemoryRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(r, back);
    }

    // ─── MemoryMergeDecision ─────────────────────────────────────────────

    #[test]
    fn merge_decision_fields() {
        let d = MemoryMergeDecision {
            primary: MemoryDecision::Merge,
            target_uri: Some("mfs://memory/profile".to_owned()),
            delete_uris: vec!["mfs://memory/profile-old".to_owned()],
        };
        assert_eq!(d.primary, MemoryDecision::Merge);
        assert_eq!(d.target_uri, Some("mfs://memory/profile".to_owned()));
        assert_eq!(d.delete_uris.len(), 1);
        assert_eq!(d.delete_uris[0], "mfs://memory/profile-old");
    }

    #[test]
    fn merge_decision_create_with_no_target() {
        let d = MemoryMergeDecision {
            primary: MemoryDecision::Create,
            target_uri: None,
            delete_uris: vec![],
        };
        assert_eq!(d.primary, MemoryDecision::Create);
        assert_eq!(d.target_uri, None);
        assert!(d.delete_uris.is_empty());
    }

    #[test]
    fn merge_decision_delete_with_multiple_uris() {
        let d = MemoryMergeDecision {
            primary: MemoryDecision::Delete,
            target_uri: None,
            delete_uris: vec![
                "mfs://memory/old1".to_owned(),
                "mfs://memory/old2".to_owned(),
            ],
        };
        assert_eq!(d.primary, MemoryDecision::Delete);
        assert_eq!(d.delete_uris.len(), 2);
    }

    #[test]
    fn merge_decision_serde_roundtrip() {
        let d = MemoryMergeDecision {
            primary: MemoryDecision::Skip,
            target_uri: None,
            delete_uris: vec!["mfs://memory/dup".to_owned()],
        };
        let json = serde_json::to_string(&d).unwrap();
        let back: MemoryMergeDecision = serde_json::from_str(&json).unwrap();
        assert_eq!(d, back);
    }

    #[test]
    fn merge_decision_skip_variant() {
        let d = MemoryMergeDecision {
            primary: MemoryDecision::Skip,
            target_uri: None,
            delete_uris: vec![],
        };
        assert_eq!(d.primary, MemoryDecision::Skip);
    }
}
