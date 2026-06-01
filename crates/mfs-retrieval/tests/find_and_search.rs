use std::time::Duration;

use mfs_metadata::{MetadataStore, PathEntryRecord};
use mfs_retrieval::{RetrievalEngine, RetrievalSettings};
use mfs_semantic::{
    DeterministicEmbeddingProvider, DeterministicSummaryProvider, SemanticPipeline,
    SemanticPipelineConfig,
};
use mfs_session::{SessionEngine, TaskStatus};
use mfs_test_util::env_isolated;
use mfs_types::IdentityContext;
use mfs_workspace::{ResourceCatalog, WorkspaceFs};
use tokio::time::sleep;

#[tokio::test]
async fn search_expands_typed_queries_and_records_trajectory() {
    let engine = RetrievalEngine::for_tests().await.unwrap();
    let result = engine
        .search(
            "help me find auth docs",
            Some("mfs://resources/localfs/docs"),
            Some("recent skill workflow about oauth incident response"),
        )
        .await
        .unwrap();

    assert!(!result.typed_queries.is_empty());
    assert!(
        result
            .typed_queries
            .iter()
            .any(|typed| typed.query != "help me find auth docs")
    );
    assert!(
        result
            .typed_queries
            .iter()
            .any(|typed| typed.query.contains("workflow") || typed.query.contains("incident"))
    );
    assert!(!result.trajectory.steps.is_empty());
    assert!(result.resources.iter().any(|m| m.uri.contains("auth")));
}

#[tokio::test]
async fn search_handles_natural_language_punctuation_without_fts_errors() {
    let retrieval = RetrievalEngine::for_tests().await.unwrap();

    let result = retrieval
        .search("How are refresh tokens used?", None, None)
        .await;

    assert!(
        result.is_ok(),
        "natural-language punctuation should not break FTS query parsing"
    );
}

#[tokio::test]
async fn find_returns_resource_hits_within_scope() {
    let engine = RetrievalEngine::for_tests().await.unwrap();
    let result = engine
        .find("api reference", Some("mfs://resources/localfs/docs"))
        .await
        .unwrap();

    assert!(!result.typed_queries.is_empty());
    assert!(result.memories.is_empty());
    assert!(result.skills.is_empty());
    assert!(
        result
            .resources
            .iter()
            .all(|matched| matched.uri.starts_with("mfs://resources/localfs/docs"))
    );
}

#[tokio::test]
async fn grep_uses_index_backend_for_literal_matches() {
    let engine = RetrievalEngine::for_tests().await.unwrap();
    let result = engine
        .grep("access token", Some("mfs://resources/localfs/docs"), None)
        .await
        .unwrap();

    assert!(result.resources.iter().any(|matched| {
        matched.uri.contains("api.md") && matched.excerpt.to_ascii_lowercase().contains("access")
    }));
}

#[tokio::test]
async fn grep_matches_literal_substrings_with_punctuation() {
    let engine = RetrievalEngine::for_tests().await.unwrap();
    let result = engine
        .grep("access token.", Some("mfs://resources/localfs/docs"), None)
        .await
        .unwrap();

    assert!(
        result
            .resources
            .iter()
            .any(|matched| matched.uri.ends_with("/api.md"))
    );
}

#[tokio::test]
async fn search_prefers_directory_level_hits_before_leaf_hits() {
    let engine = RetrievalEngine::for_tests().await.unwrap();
    let result = engine
        .search(
            "authentication docs",
            Some("mfs://resources/localfs/docs"),
            Some("recent session summary"),
        )
        .await
        .unwrap();

    let first = result
        .resources
        .first()
        .expect("expected at least one resource hit");
    assert_eq!(first.uri, "mfs://resources/localfs/docs");
    assert!(first.level <= 1);
    assert!(
        result
            .resources
            .iter()
            .any(|matched| { matched.uri.ends_with("/auth.md") && matched.level == 2 })
    );
}

#[tokio::test]
async fn search_records_recursive_drilldown_for_nested_directory_hits() {
    let engine = RetrievalEngine::for_tests().await.unwrap();
    let result = engine
        .search(
            "oauth flow",
            Some("mfs://resources/localfs/docs"),
            Some("recent session summary"),
        )
        .await
        .unwrap();

    assert!(result.trajectory.steps.iter().any(|step| {
        step.stage == "drill_target" && step.detail == "mfs://resources/localfs/docs/guides"
    }));

    let guides_dir_index = result
        .resources
        .iter()
        .position(|matched| matched.uri == "mfs://resources/localfs/docs/guides")
        .expect("expected guides directory hit");
    let oauth_file_index = result
        .resources
        .iter()
        .position(|matched| matched.uri.ends_with("/guides/oauth.md"))
        .expect("expected oauth file hit");

    assert!(guides_dir_index < oauth_file_index);
}

#[tokio::test]
async fn search_recursively_drills_multiple_directory_levels_without_duplicate_steps() {
    let engine = RetrievalEngine::for_tests().await.unwrap();
    let result = engine
        .search(
            "refresh tokens",
            Some("mfs://resources/localfs/docs"),
            Some("recent session summary"),
        )
        .await
        .unwrap();

    let drill_targets = result
        .trajectory
        .steps
        .iter()
        .filter(|step| step.stage == "drill_target")
        .map(|step| step.detail.clone())
        .collect::<Vec<_>>();

    assert!(drill_targets.contains(&"mfs://resources/localfs/docs/guides".to_owned()));
    assert!(drill_targets.contains(&"mfs://resources/localfs/docs/guides/tokens".to_owned()));

    let unique_targets = drill_targets
        .iter()
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(unique_targets.len(), drill_targets.len());

    let tokens_dir_index = result
        .resources
        .iter()
        .position(|matched| matched.uri == "mfs://resources/localfs/docs/guides/tokens")
        .expect("expected nested tokens directory hit");
    let refresh_file_index = result
        .resources
        .iter()
        .position(|matched| matched.uri.ends_with("/guides/tokens/refresh.md"))
        .expect("expected refresh token file hit");

    assert!(tokens_dir_index < refresh_file_index);
}

#[tokio::test]
async fn search_can_prefer_leaf_hits_with_custom_layer_weights() {
    let engine = RetrievalEngine::for_tests_with_settings(RetrievalSettings {
        level_weights: [200, 100, 0],
        ..Default::default()
    })
    .await
    .unwrap();

    let result = engine
        .search(
            "authentication docs",
            Some("mfs://resources/localfs/docs"),
            Some("recent session summary"),
        )
        .await
        .unwrap();

    let first = result
        .resources
        .first()
        .expect("expected at least one resource hit");
    assert!(first.uri.ends_with("/auth.md"));
    assert_eq!(first.level, 2);
}

#[tokio::test]
async fn search_respects_max_drill_depth_setting() {
    let engine = RetrievalEngine::for_tests_with_settings(RetrievalSettings {
        max_drill_depth: 1,
        ..Default::default()
    })
    .await
    .unwrap();

    let result = engine
        .search(
            "refresh tokens",
            Some("mfs://resources/localfs/docs"),
            Some("recent session summary"),
        )
        .await
        .unwrap();

    let drill_targets = result
        .trajectory
        .steps
        .iter()
        .filter(|step| step.stage == "drill_target")
        .map(|step| step.detail.clone())
        .collect::<Vec<_>>();

    assert!(drill_targets.contains(&"mfs://resources/localfs/docs/guides".to_owned()));
    assert!(!drill_targets.contains(&"mfs://resources/localfs/docs/guides/tokens".to_owned()));
}

#[tokio::test]
async fn search_records_convergence_stop_when_resource_budget_is_reached() {
    let engine = RetrievalEngine::for_tests_with_settings(RetrievalSettings {
        max_total_resources: 2,
        ..Default::default()
    })
    .await
    .unwrap();

    let result = engine
        .search(
            "authentication docs",
            Some("mfs://resources/localfs/docs"),
            Some("recent session summary"),
        )
        .await
        .unwrap();

    assert!(result.resources.len() <= 2);
    assert!(
        result
            .trajectory
            .steps
            .iter()
            .any(|step| { step.stage == "convergence_stop" && step.detail == "resource_budget" })
    );
}

#[tokio::test]
async fn search_records_rerank_stage_and_reason_when_enabled() {
    let engine = RetrievalEngine::for_tests_with_settings(RetrievalSettings {
        enable_rerank: true,
        ..Default::default()
    })
    .await
    .unwrap();

    let result = engine
        .search(
            "oauth flow",
            Some("mfs://resources/localfs/docs"),
            Some("recent workflow docs about auth incidents"),
        )
        .await
        .unwrap();

    assert!(
        result
            .trajectory
            .steps
            .iter()
            .any(|step| step.stage == "rerank")
    );
    assert!(
        result
            .resources
            .iter()
            .any(|item| item.match_reason.contains("rerank"))
    );
}

#[tokio::test]
async fn search_returns_query_plan_and_match_explanations() {
    let engine = RetrievalEngine::for_tests().await.unwrap();
    let result = engine
        .search(
            "help me find auth docs",
            Some("mfs://resources/localfs/docs"),
            Some("recent skill workflow about oauth incident response"),
        )
        .await
        .unwrap();

    assert_eq!(result.query_plan.mode.as_str(), "search");
    assert!(!result.query_plan.typed_queries.is_empty());
    assert!(
        result
            .resources
            .iter()
            .all(|item| !item.match_reason.is_empty())
    );
    assert!(
        result
            .resources
            .iter()
            .all(|item| !item.retrieval_plane.is_empty())
    );
}

#[tokio::test]
async fn find_keeps_simple_plan_without_search_only_fields() {
    let engine = RetrievalEngine::for_tests().await.unwrap();
    let result = engine
        .find("api reference", Some("mfs://resources/localfs/docs"))
        .await
        .unwrap();

    assert_eq!(result.query_plan.mode.as_str(), "find");
    assert!(result.query_plan.skip_reason.is_none());
}

#[tokio::test]
async fn find_returns_memory_and_skill_hits_from_workspace_state() {
    let _env_guard = env_isolated();
    let workspace = tempfile::tempdir().unwrap();
    let identity = IdentityContext::new("acme", "alice", "coding-agent");
    let fixture_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/localfs_docs");
    let fs = WorkspaceFs::from_localfs_source(
        workspace.path(),
        identity.account_id(),
        identity.user_id(),
        identity.agent_id(),
        fixture_root.to_str().unwrap(),
        "mfs://resources/localfs/docs",
    )
    .await
    .unwrap();
    let session_engine = SessionEngine::open(workspace.path()).await.unwrap();
    let session_id = session_engine
        .new_session(
            identity.account_id(),
            identity.user_id(),
            identity.agent_id(),
        )
        .await
        .unwrap();
    session_engine
        .add_message(
            &session_id,
            "user",
            "remember auth preference and search workflow",
        )
        .await
        .unwrap();
    session_engine
        .used_context(&session_id, "mfs://resources/localfs/docs/auth.md")
        .await
        .unwrap();
    session_engine
        .used_skill(
            &session_id,
            "mfs://agent/alice__coding-agent/skills/search",
            true,
        )
        .await
        .unwrap();
    let commit = session_engine.commit(&session_id).await.unwrap();
    wait_for_session_task(&session_engine, commit.task_id.as_deref().unwrap()).await;

    let retrieval = RetrievalEngine::from_workspace(
        workspace.path(),
        &identity,
        fs.projection_root(),
        fs.projection_uri(),
    )
    .await
    .unwrap();
    let memory_result = retrieval
        .find("workflow", Some("mfs://resources/localfs/docs"))
        .await
        .unwrap();
    let skill_result = retrieval
        .find("search", Some("mfs://resources/localfs/docs"))
        .await
        .unwrap();

    assert!(!memory_result.memories.is_empty());
    let memory_hit = memory_result
        .memories
        .iter()
        .find(|matched| matched.uri.starts_with("mfs://user/memories/"))
        .expect("expected user memory hit");
    let memory_provenance = memory_hit
        .provenance
        .as_ref()
        .expect("expected memory provenance");
    assert_eq!(memory_provenance.source_kind.as_deref(), Some("session"));
    assert!(
        memory_provenance
            .source_identifier
            .as_deref()
            .unwrap_or_default()
            .contains("mfs://session/")
    );
    assert!(memory_provenance.source_snapshot_id.is_some());
    assert!(!memory_provenance.audit_events.is_empty());
    assert!(!skill_result.skills.is_empty());
    let skill_hit = skill_result
        .skills
        .iter()
        .find(|matched| matched.uri.starts_with("mfs://agent/skills/"))
        .expect("expected skill hit");
    let skill_provenance = skill_hit
        .provenance
        .as_ref()
        .expect("expected skill provenance");
    assert_eq!(skill_provenance.source_kind.as_deref(), Some("session"));
    assert!(skill_provenance.source_snapshot_id.is_some());
    assert!(!skill_provenance.audit_events.is_empty());
}

#[tokio::test]
async fn find_enriches_results_with_related_skill_hits() {
    let workspace = tempfile::tempdir().unwrap();
    let identity = IdentityContext::new("acme", "alice", "coding-agent");
    let fixture_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/localfs_docs");
    let fs = WorkspaceFs::from_localfs_source(
        workspace.path(),
        identity.account_id(),
        identity.user_id(),
        identity.agent_id(),
        fixture_root.to_str().unwrap(),
        "mfs://resources/localfs/docs",
    )
    .await
    .unwrap();

    let skill_root = workspace
        .path()
        .join("tenants/acme/alice/agent/alice__coding-agent/skills/search-web");
    std::fs::create_dir_all(&skill_root).unwrap();
    std::fs::write(
        skill_root.join("SKILL.md"),
        "# search-web\nSearch current information from the web.\n",
    )
    .unwrap();

    let metadata = mfs_metadata::MetadataStore::open_at(
        workspace.path().join("_system/metadata.sqlite"),
        false,
    )
    .unwrap();
    metadata
        .upsert_path_entry(&mfs_metadata::PathEntryRecord {
            account_id: "acme",
            user_id: "alice",
            agent_id: Some("coding-agent"),
            projection_view_id: "tenant:acme:alice:resources",
            canonical_uri: "mfs://resources/localfs/docs/auth.md",
            workspace_path: &fs.projection_root().join("auth.md").to_string_lossy(),
            entry_kind: "file",
            source_kind: Some("localfs"),
            source_identifier: Some(fixture_root.to_str().unwrap()),
            source_snapshot_id: Some("fixture"),
            content_kind: None,
            language: None,
            relative_resource_path: None,
            repo_root_uri: None,
            is_text: None,
            is_generated: None,
            content_digest: None,
            metadata_digest: None,
            size_bytes: None,
        })
        .unwrap();
    metadata
        .upsert_path_entry(&mfs_metadata::PathEntryRecord {
            account_id: "acme",
            user_id: "alice",
            agent_id: Some("coding-agent"),
            projection_view_id: "tenant:acme:alice:agent:alice__coding-agent",
            canonical_uri: "mfs://agent/skills/search-web/SKILL.md",
            workspace_path: &skill_root.join("SKILL.md").to_string_lossy(),
            entry_kind: "file",
            source_kind: Some("managed"),
            source_identifier: Some("mfs://agent/skills/search-web"),
            source_snapshot_id: Some("skill"),
            content_kind: None,
            language: None,
            relative_resource_path: None,
            repo_root_uri: None,
            is_text: None,
            is_generated: None,
            content_digest: None,
            metadata_digest: None,
            size_bytes: None,
        })
        .unwrap();
    metadata
        .upsert_relation(&mfs_metadata::RelationRecord {
            account_id: "acme",
            user_id: "alice",
            agent_id: Some("coding-agent"),
            from_uri: "mfs://resources/localfs/docs/auth.md",
            to_uri: "mfs://agent/skills/search-web/SKILL.md",
            relation_type: "references",
        })
        .unwrap();

    let retrieval = RetrievalEngine::from_workspace(
        workspace.path(),
        &identity,
        fs.projection_root(),
        fs.projection_uri(),
    )
    .await
    .unwrap();
    let result = retrieval.find("authentication", None).await.unwrap();

    assert!(
        result
            .resources
            .iter()
            .any(|item| item.uri == "mfs://resources/localfs/docs/auth.md")
    );
    let related_skill = result
        .skills
        .iter()
        .find(|item| item.uri == "mfs://agent/skills/search-web/SKILL.md")
        .expect("expected related skill hit");
    assert_eq!(related_skill.retrieval_plane, "relation");
    assert!(related_skill.match_reason.contains("references"));
}

#[tokio::test]
async fn search_prioritizes_explicit_resource_target_over_memory_hits() {
    let _env_guard = env_isolated();
    let workspace = tempfile::tempdir().unwrap();
    let identity = IdentityContext::new("acme", "alice", "coding-agent");
    let fixture_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/localfs_docs");
    let fs = WorkspaceFs::from_localfs_source(
        workspace.path(),
        identity.account_id(),
        identity.user_id(),
        identity.agent_id(),
        fixture_root.to_str().unwrap(),
        "mfs://resources/localfs/docs",
    )
    .await
    .unwrap();

    let session_engine = SessionEngine::open(workspace.path()).await.unwrap();
    let session_id = session_engine
        .new_session(
            identity.account_id(),
            identity.user_id(),
            identity.agent_id(),
        )
        .await
        .unwrap();
    session_engine
        .add_message(
            &session_id,
            "user",
            "refresh tokens memory should not dominate",
        )
        .await
        .unwrap();
    session_engine
        .used_context(
            &session_id,
            "mfs://resources/localfs/docs/guides/tokens/refresh.md",
        )
        .await
        .unwrap();
    let commit = session_engine.commit(&session_id).await.unwrap();
    wait_for_session_task(&session_engine, commit.task_id.as_deref().unwrap()).await;

    let retrieval = RetrievalEngine::from_workspace(
        workspace.path(),
        &identity,
        fs.projection_root(),
        fs.projection_uri(),
    )
    .await
    .unwrap();
    let result = retrieval
        .search("refresh tokens", Some("mfs://resources/localfs/docs"), None)
        .await
        .unwrap();

    assert!(
        result
            .resources
            .iter()
            .any(|matched| matched.uri.ends_with("/guides/tokens/refresh.md")),
        "explicit resource target should still return resource hits"
    );
    assert!(
        result
            .trajectory
            .steps
            .iter()
            .find(|step| step.stage == "typed_query")
            .map(|step| step.detail.as_str())
            == Some("resource:refresh tokens"),
        "resource typed query should run before memory/skill for explicit resource targets"
    );
}

#[tokio::test]
async fn find_can_hit_category_aware_preference_memories_after_session_commit() {
    let _env_guard = env_isolated();
    let workspace = tempfile::tempdir().unwrap();
    let identity = IdentityContext::new("acme", "alice", "coding-agent");
    let fixture_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/localfs_docs");
    let fs = WorkspaceFs::from_localfs_source(
        workspace.path(),
        identity.account_id(),
        identity.user_id(),
        identity.agent_id(),
        fixture_root.to_str().unwrap(),
        "mfs://resources/localfs/docs",
    )
    .await
    .unwrap();

    let session_engine = SessionEngine::open(workspace.path()).await.unwrap();
    let session_id = session_engine
        .new_session(
            identity.account_id(),
            identity.user_id(),
            identity.agent_id(),
        )
        .await
        .unwrap();
    session_engine
        .add_message(
            &session_id,
            "user",
            "I prefer short API examples when reading docs",
        )
        .await
        .unwrap();
    let commit = session_engine.commit(&session_id).await.unwrap();
    wait_for_session_task(&session_engine, commit.task_id.as_deref().unwrap()).await;

    let retrieval = RetrievalEngine::from_workspace(
        workspace.path(),
        &identity,
        fs.projection_root(),
        fs.projection_uri(),
    )
    .await
    .unwrap();
    let result = retrieval.find("short API examples", None).await.unwrap();

    assert!(
        result
            .memories
            .iter()
            .any(|matched| matched.uri == "mfs://user/memories/preferences/general.md")
    );
}

#[tokio::test]
async fn find_can_hit_profile_memory_after_session_commit() {
    let _env_guard = env_isolated();
    let workspace = tempfile::tempdir().unwrap();
    let identity = IdentityContext::new("acme", "alice", "coding-agent");
    let fixture_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/localfs_docs");
    let fs = WorkspaceFs::from_localfs_source(
        workspace.path(),
        identity.account_id(),
        identity.user_id(),
        identity.agent_id(),
        fixture_root.to_str().unwrap(),
        "mfs://resources/localfs/docs",
    )
    .await
    .unwrap();

    let session_engine = SessionEngine::open(workspace.path()).await.unwrap();
    let session_id = session_engine
        .new_session(
            identity.account_id(),
            identity.user_id(),
            identity.agent_id(),
        )
        .await
        .unwrap();
    session_engine
        .add_message(&session_id, "user", "My name is Alice Example")
        .await
        .unwrap();
    let commit = session_engine.commit(&session_id).await.unwrap();
    wait_for_session_task(&session_engine, commit.task_id.as_deref().unwrap()).await;

    let retrieval = RetrievalEngine::from_workspace(
        workspace.path(),
        &identity,
        fs.projection_root(),
        fs.projection_uri(),
    )
    .await
    .unwrap();
    let result = retrieval.find("Alice Example", None).await.unwrap();

    assert!(
        result
            .memories
            .iter()
            .any(|matched| matched.uri == "mfs://user/memories/profile.md")
    );
}

#[tokio::test]
async fn find_can_hit_tool_memory_after_session_commit() {
    let _env_guard = env_isolated();
    let workspace = tempfile::tempdir().unwrap();
    let identity = IdentityContext::new("acme", "alice", "coding-agent");
    let fixture_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/localfs_docs");
    let fs = WorkspaceFs::from_localfs_source(
        workspace.path(),
        identity.account_id(),
        identity.user_id(),
        identity.agent_id(),
        fixture_root.to_str().unwrap(),
        "mfs://resources/localfs/docs",
    )
    .await
    .unwrap();

    let session_engine = SessionEngine::open(workspace.path()).await.unwrap();
    let session_id = session_engine
        .new_session(
            identity.account_id(),
            identity.user_id(),
            identity.agent_id(),
        )
        .await
        .unwrap();
    session_engine
        .used_tool(&session_id, "mfs://agent/tools/read_file", true)
        .await
        .unwrap();
    let commit = session_engine.commit(&session_id).await.unwrap();
    wait_for_session_task(&session_engine, commit.task_id.as_deref().unwrap()).await;

    let retrieval = RetrievalEngine::from_workspace(
        workspace.path(),
        &identity,
        fs.projection_root(),
        fs.projection_uri(),
    )
    .await
    .unwrap();
    let result = retrieval.find("read_file", None).await.unwrap();

    assert!(
        result
            .memories
            .iter()
            .any(|matched| matched.uri == "mfs://agent/memories/tools/read-file.md")
    );
}

#[tokio::test]
async fn find_can_hit_event_and_case_memories_after_session_commit() {
    let _env_guard = env_isolated();
    let workspace = tempfile::tempdir().unwrap();
    let identity = IdentityContext::new("acme", "alice", "coding-agent");
    let fixture_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/localfs_docs");
    let fs = WorkspaceFs::from_localfs_source(
        workspace.path(),
        identity.account_id(),
        identity.user_id(),
        identity.agent_id(),
        fixture_root.to_str().unwrap(),
        "mfs://resources/localfs/docs",
    )
    .await
    .unwrap();

    let session_engine = SessionEngine::open(workspace.path()).await.unwrap();
    let session_id = session_engine
        .new_session(
            identity.account_id(),
            identity.user_id(),
            identity.agent_id(),
        )
        .await
        .unwrap();
    session_engine
        .add_message(
            &session_id,
            "user",
            "We decided to use SQLite for metadata storage",
        )
        .await
        .unwrap();
    session_engine
        .add_message(
            &session_id,
            "assistant",
            "Resolved oauth incident by rotating refresh tokens",
        )
        .await
        .unwrap();
    let commit = session_engine.commit(&session_id).await.unwrap();
    wait_for_session_task(&session_engine, commit.task_id.as_deref().unwrap()).await;

    let retrieval = RetrievalEngine::from_workspace(
        workspace.path(),
        &identity,
        fs.projection_root(),
        fs.projection_uri(),
    )
    .await
    .unwrap();
    let event_result = retrieval.find("SQLite metadata", None).await.unwrap();
    let case_result = retrieval.find("oauth incident", None).await.unwrap();

    assert!(
        event_result
            .memories
            .iter()
            .any(|matched| { matched.uri.starts_with("mfs://user/memories/events/") })
    );
    assert!(
        case_result
            .memories
            .iter()
            .any(|matched| { matched.uri.starts_with("mfs://agent/memories/cases/") })
    );
}

#[tokio::test]
async fn find_can_hit_entity_memory_after_session_commit() {
    let _env_guard = env_isolated();
    let workspace = tempfile::tempdir().unwrap();
    let identity = IdentityContext::new("acme", "alice", "coding-agent");
    let fixture_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/localfs_docs");
    let fs = WorkspaceFs::from_localfs_source(
        workspace.path(),
        identity.account_id(),
        identity.user_id(),
        identity.agent_id(),
        fixture_root.to_str().unwrap(),
        "mfs://resources/localfs/docs",
    )
    .await
    .unwrap();

    let session_engine = SessionEngine::open(workspace.path()).await.unwrap();
    let session_id = session_engine
        .new_session(
            identity.account_id(),
            identity.user_id(),
            identity.agent_id(),
        )
        .await
        .unwrap();
    session_engine
        .add_message(
            &session_id,
            "user",
            "Project Atlas uses Rust for the control plane",
        )
        .await
        .unwrap();
    let commit = session_engine.commit(&session_id).await.unwrap();
    wait_for_session_task(&session_engine, commit.task_id.as_deref().unwrap()).await;

    let retrieval = RetrievalEngine::from_workspace(
        workspace.path(),
        &identity,
        fs.projection_root(),
        fs.projection_uri(),
    )
    .await
    .unwrap();
    let result = retrieval.find("Atlas Rust", None).await.unwrap();

    assert!(
        result
            .memories
            .iter()
            .any(|matched| matched.uri == "mfs://user/memories/entities/project-atlas.md")
    );
}

#[tokio::test]
async fn find_can_hit_pattern_memory_after_session_commit() {
    let _env_guard = env_isolated();
    let workspace = tempfile::tempdir().unwrap();
    let identity = IdentityContext::new("acme", "alice", "coding-agent");
    let fixture_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/localfs_docs");
    let fs = WorkspaceFs::from_localfs_source(
        workspace.path(),
        identity.account_id(),
        identity.user_id(),
        identity.agent_id(),
        fixture_root.to_str().unwrap(),
        "mfs://resources/localfs/docs",
    )
    .await
    .unwrap();

    let session_engine = SessionEngine::open(workspace.path()).await.unwrap();
    let session_id = session_engine
        .new_session(
            identity.account_id(),
            identity.user_id(),
            identity.agent_id(),
        )
        .await
        .unwrap();
    session_engine
        .add_message(
            &session_id,
            "assistant",
            "Use MemFuse search then inspect overview before grep",
        )
        .await
        .unwrap();
    let commit = session_engine.commit(&session_id).await.unwrap();
    wait_for_session_task(&session_engine, commit.task_id.as_deref().unwrap()).await;

    let retrieval = RetrievalEngine::from_workspace(
        workspace.path(),
        &identity,
        fs.projection_root(),
        fs.projection_uri(),
    )
    .await
    .unwrap();
    let result = retrieval.find("search overview grep", None).await.unwrap();

    assert!(
        result
            .memories
            .iter()
            .any(|matched| { matched.uri.starts_with("mfs://agent/memories/patterns/") })
    );
}

#[tokio::test]
async fn from_workspace_prefers_semantic_index_and_attaches_provenance() {
    let _env_guard = env_isolated();
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(
        source.path().join("auth.md"),
        "# Authentication\nOAuth login flow and token refresh\n",
    )
    .unwrap();

    let identity = IdentityContext::new("acme", "alice", "coding-agent");
    let catalog = ResourceCatalog::open(workspace.path()).unwrap();
    let resource = catalog
        .register_localfs(
            identity.account_id(),
            identity.user_id(),
            identity.agent_id(),
            source.path().to_str().unwrap(),
            Some("docs"),
        )
        .await
        .unwrap();
    let projection_root = resource.target_path.clone();
    let semantic_index_path = workspace.path().join("_system/semantic.sqlite");
    let semantic_index = mfs_index::SqliteSemanticIndex::open_at(&semantic_index_path).unwrap();
    let pipeline = SemanticPipeline::new(SemanticPipelineConfig {
        summary_provider: std::sync::Arc::new(DeterministicSummaryProvider::new()),
        embedding_provider: Box::new(DeterministicEmbeddingProvider::new(8)),
    });
    let report = pipeline
        .process_resource_root(
            &projection_root,
            "tenant:acme:alice:resources",
            &resource.root_uri,
            Some(&resource.resource_id),
            &semantic_index,
        )
        .await
        .unwrap();
    assert!(report.indexed_documents >= 3);

    let metadata =
        MetadataStore::open_at(workspace.path().join("_system/metadata.sqlite"), false).unwrap();
    let workspace_path = projection_root.join("auth.md");
    let workspace_path_string = workspace_path.to_string_lossy().into_owned();
    metadata
        .upsert_path_entry(&PathEntryRecord {
            account_id: identity.account_id(),
            user_id: identity.user_id(),
            agent_id: Some(identity.agent_id()),
            projection_view_id: "tenant:acme:alice:resources",
            canonical_uri: "mfs://resources/localfs/docs/auth.md",
            workspace_path: &workspace_path_string,
            entry_kind: "file",
            source_kind: Some("localfs"),
            source_identifier: Some(source.path().to_str().unwrap()),
            source_snapshot_id: Some("snap-docs"),
            content_kind: None,
            language: None,
            relative_resource_path: None,
            repo_root_uri: None,
            is_text: None,
            is_generated: None,
            content_digest: None,
            metadata_digest: None,
            size_bytes: None,
        })
        .unwrap();

    let fs = WorkspaceFs::open_existing(
        workspace.path(),
        identity.account_id(),
        identity.user_id(),
        identity.agent_id(),
    )
    .unwrap();
    let retrieval = RetrievalEngine::from_workspace(
        workspace.path(),
        &identity,
        fs.projection_root(),
        fs.projection_uri(),
    )
    .await
    .unwrap();

    let result = retrieval
        .find("authentication", Some("mfs://resources/localfs/docs"))
        .await
        .unwrap();

    assert!(
        result
            .trajectory
            .steps
            .iter()
            .any(|step| step.stage == "retrieval_plane" && step.detail == "semantic")
    );
    let hit = result
        .resources
        .iter()
        .find(|item| item.uri.ends_with("/auth.md"))
        .expect("expected semantic auth hit");
    let provenance = hit.provenance.as_ref().expect("expected provenance");
    assert_eq!(provenance.source_kind.as_deref(), Some("localfs"));
    assert_eq!(provenance.source_snapshot_id.as_deref(), Some("snap-docs"));
}

async fn wait_for_session_task(engine: &SessionEngine, task_id: &str) {
    for _ in 0..50 {
        if let Some(task) = engine.task_status(task_id).await {
            if matches!(task.status, TaskStatus::Completed | TaskStatus::Failed) {
                assert_eq!(task.status, TaskStatus::Completed);
                return;
            }
        }
        sleep(Duration::from_millis(20)).await;
    }

    panic!("task {task_id} did not finish in time");
}
