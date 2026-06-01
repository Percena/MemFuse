use std::time::Duration;

use mfs_index::SqliteSemanticIndex;
use mfs_metadata::MetadataStore;
use mfs_session::{SessionEngine, TaskStatus};
use mfs_test_util::env_isolated;
use mfs_types::IdentityContext;
use mfs_uri::{MfsUri, WorkspaceMapper};
use tokio::time::sleep;

#[tokio::test]
async fn commit_archives_messages_and_enqueues_background_memory_work() {
    let engine = SessionEngine::for_tests().await.unwrap();
    let session_id = engine
        .new_session("acme", "alice", "coding-agent")
        .await
        .unwrap();
    engine
        .add_message(&session_id, "user", "remember my auth preference")
        .await
        .unwrap();
    let result = engine.commit(&session_id).await.unwrap();
    assert!(result.archive_uri.contains("/history/"));
    assert!(result.task_id.is_some());
    assert!(result.archive_uri.contains("/coding-agent/"));
    assert!(!result.archive_uri.contains("/alice/coding-agent/"));
}

#[tokio::test]
async fn same_user_id_in_different_accounts_gets_distinct_archive_roots() {
    let engine = SessionEngine::for_tests().await.unwrap();

    let a = engine
        .new_session("acme", "alice", "coding-agent")
        .await
        .unwrap();
    let b = engine
        .new_session("globex", "alice", "coding-agent")
        .await
        .unwrap();

    engine.add_message(&a, "user", "a").await.unwrap();
    engine.add_message(&b, "user", "b").await.unwrap();

    let ra = engine.commit(&a).await.unwrap();
    let rb = engine.commit(&b).await.unwrap();

    let root = engine.workspace_root();
    let acme_archive = root.join("tenants/acme/alice/session/coding-agent");
    let globex_archive = root.join("tenants/globex/alice/session/coding-agent");

    assert!(tokio::fs::try_exists(acme_archive).await.unwrap());
    assert!(tokio::fs::try_exists(globex_archive).await.unwrap());
    assert_ne!(ra.archive_uri, rb.archive_uri);
}

#[tokio::test]
async fn commit_archive_uri_round_trips_into_actual_archive_path() {
    let engine = SessionEngine::for_tests().await.unwrap();
    let session_id = engine
        .new_session("acme", "alice", "coding-agent")
        .await
        .unwrap();
    engine
        .add_message(&session_id, "user", "remember my auth preference")
        .await
        .unwrap();

    let result = engine.commit(&session_id).await.unwrap();
    let identity = IdentityContext::new("acme", "alice", "coding-agent");
    let archive_uri = MfsUri::parse(&result.archive_uri).unwrap();
    let mapped_path = WorkspaceMapper::new(engine.workspace_root())
        .map(&identity, &archive_uri)
        .unwrap();
    let expected_path = engine
        .workspace_root()
        .join("tenants/acme/alice/session/coding-agent")
        .join(&session_id)
        .join("history/archive_001");

    assert_eq!(mapped_path, expected_path);
}

#[tokio::test]
async fn commit_tracks_usage_and_writes_memory_outputs() {
    let _env_guard = env_isolated();
    let engine = SessionEngine::for_tests().await.unwrap();
    let session_id = engine
        .new_session("acme", "alice", "coding-agent")
        .await
        .unwrap();
    engine
        .add_message(&session_id, "user", "remember my auth preference")
        .await
        .unwrap();
    engine
        .used_context(&session_id, "mfs://resources/localfs/docs/auth.md")
        .await
        .unwrap();
    engine
        .used_skill(
            &session_id,
            "mfs://agent/alice__coding-agent/skills/search",
            true,
        )
        .await
        .unwrap();

    let result = engine.commit(&session_id).await.unwrap();
    let task = wait_for_task(&engine, result.task_id.as_deref().unwrap()).await;

    assert_eq!(task.status, TaskStatus::Completed);
    assert_eq!(task.retry_state.as_deref(), Some("not_needed"));
    assert!(task.processing_mode.is_some());
    assert_eq!(task.used_contexts, 1);
    assert_eq!(task.used_skills, 1);
    assert_eq!(task.memories_extracted.get("skills"), Some(&1));
    assert_eq!(task.artifacts_written.get("user_session"), Some(&1));
    assert_eq!(task.artifacts_written.get("agent_skill"), Some(&1));
    assert_eq!(task.artifacts_written.get("skill_record"), Some(&1));

    let archive_root = engine
        .workspace_root()
        .join("tenants/acme/alice/session/coding-agent")
        .join(&session_id)
        .join("history/archive_001");
    let usage_json = tokio::fs::read_to_string(archive_root.join("usage.json"))
        .await
        .unwrap();
    let done_marker = tokio::fs::read_to_string(archive_root.join(".done"))
        .await
        .unwrap();
    let user_memory = tokio::fs::read_to_string(
        engine
            .workspace_root()
            .join("tenants/acme/alice/user/memories/session/coding-agent")
            .join(&session_id)
            .join("archive_001.md"),
    )
    .await
    .unwrap();
    let agent_memory = tokio::fs::read_to_string(
        engine
            .workspace_root()
            .join("tenants/acme/alice/agent/alice__coding-agent/memories/skills")
            .join(&session_id)
            .join("archive_001.md"),
    )
    .await
    .unwrap();
    let skill_record = tokio::fs::read_to_string(
        engine
            .workspace_root()
            .join("tenants/acme/alice/agent/alice__coding-agent/skills/used")
            .join(&session_id)
            .join("archive_001.md"),
    )
    .await
    .unwrap();

    assert!(usage_json.contains("auth.md"));
    assert_eq!(done_marker, "done\n");
    assert!(user_memory.contains("Highlights"));
    assert!(agent_memory.contains("Skill Usage Memory"));
    assert!(skill_record.contains("Skill Record"));

    let semantic_index =
        SqliteSemanticIndex::open_at(engine.workspace_root().join("_system/semantic.sqlite"))
            .unwrap();
    let memory_hits = semantic_index
        .search_lexical(
            "auth",
            Some(&["tenant:acme:alice:user"]),
            Some("mfs://user/memories"),
            None,
            Some(&["memory"]),
            10,
        )
        .unwrap();
    let skill_hits = semantic_index
        .search_lexical(
            "search",
            Some(&["tenant:acme:alice:agent:alice__coding-agent"]),
            Some("mfs://agent/skills"),
            None,
            Some(&["skill"]),
            10,
        )
        .unwrap();

    assert!(!memory_hits.is_empty());
    assert!(!skill_hits.is_empty());

    let metadata = MetadataStore::open_at(
        engine.workspace_root().join("_system/metadata.sqlite"),
        false,
    )
    .unwrap();
    let user_memory_uri = format!(
        "mfs://user/memories/session/coding-agent/{}/archive_001.md",
        session_id
    );
    let skill_record_uri = format!("mfs://agent/skills/used/{}/archive_001.md", session_id);

    let user_entry = metadata
        .get_path_entry("tenant:acme:alice:user", &user_memory_uri)
        .unwrap()
        .expect("expected user memory path entry");
    let skill_entry = metadata
        .get_path_entry(
            "tenant:acme:alice:agent:alice__coding-agent",
            &skill_record_uri,
        )
        .unwrap()
        .expect("expected skill record path entry");

    assert_eq!(user_entry.source_kind.as_deref(), Some("session"));
    assert!(
        user_entry
            .source_identifier
            .as_deref()
            .unwrap()
            .contains("mfs://session/")
    );
    assert!(user_entry.source_snapshot_id.is_some());
    assert_eq!(skill_entry.source_kind.as_deref(), Some("session"));

    let user_snapshots = metadata
        .list_snapshots("acme", "alice", Some("tenant:acme:alice:user"), 10)
        .unwrap();
    let agent_snapshots = metadata
        .list_snapshots(
            "acme",
            "alice",
            Some("tenant:acme:alice:agent:alice__coding-agent"),
            10,
        )
        .unwrap();
    let audit = metadata.list_audit("acme", "alice", 100).unwrap();

    assert!(!user_snapshots.is_empty());
    assert!(!agent_snapshots.is_empty());
    assert!(
        audit
            .iter()
            .any(|record| record.subject_uri.as_deref() == Some(&user_memory_uri))
    );
    assert!(
        audit
            .iter()
            .any(|record| record.subject_uri.as_deref() == Some(&skill_record_uri))
    );
}

#[tokio::test]
async fn commit_writes_category_aware_memory_outputs() {
    let _env_guard = env_isolated();
    let engine = SessionEngine::for_tests().await.unwrap();
    let session_id = engine
        .new_session("acme", "alice", "coding-agent")
        .await
        .unwrap();
    engine
        .add_message(
            &session_id,
            "user",
            "I prefer short API examples when reading docs",
        )
        .await
        .unwrap();
    engine
        .used_skill(
            &session_id,
            "mfs://agent/alice__coding-agent/skills/search",
            true,
        )
        .await
        .unwrap();

    let result = engine.commit(&session_id).await.unwrap();
    let task = wait_for_task(&engine, result.task_id.as_deref().unwrap()).await;

    assert_eq!(task.memories_extracted.get("preferences"), Some(&1));
    assert_eq!(task.memories_extracted.get("skills"), Some(&1));

    let preference_memory = tokio::fs::read_to_string(
        engine
            .workspace_root()
            .join("tenants/acme/alice/user/memories/preferences/general.md"),
    )
    .await
    .unwrap();
    let skill_memory = tokio::fs::read_to_string(
        engine
            .workspace_root()
            .join("tenants/acme/alice/agent/alice__coding-agent/memories/skills/search.md"),
    )
    .await
    .unwrap();

    assert!(preference_memory.contains("short API examples"));
    assert!(skill_memory.contains("mfs://agent/alice__coding-agent/skills/search"));
}

#[tokio::test]
async fn commit_merges_similar_preference_memories_into_single_entry() {
    let _env_guard = env_isolated();
    let engine = SessionEngine::for_tests().await.unwrap();

    let first_session = engine
        .new_session("acme", "alice", "coding-agent")
        .await
        .unwrap();
    engine
        .add_message(
            &first_session,
            "user",
            "I prefer short API examples when reading docs",
        )
        .await
        .unwrap();
    let first_commit = engine.commit(&first_session).await.unwrap();
    let _ = wait_for_task(&engine, first_commit.task_id.as_deref().unwrap()).await;

    let second_session = engine
        .new_session("acme", "alice", "coding-agent")
        .await
        .unwrap();
    engine
        .add_message(&second_session, "user", "I prefer short API examples")
        .await
        .unwrap();
    let second_commit = engine.commit(&second_session).await.unwrap();
    let _ = wait_for_task(&engine, second_commit.task_id.as_deref().unwrap()).await;

    let preference_memory = tokio::fs::read_to_string(
        engine
            .workspace_root()
            .join("tenants/acme/alice/user/memories/preferences/general.md"),
    )
    .await
    .unwrap();

    let bullet_count = preference_memory
        .lines()
        .filter(|line| line.trim_start().starts_with("- "))
        .count();
    assert_eq!(bullet_count, 1);
}

#[tokio::test]
async fn commit_writes_profile_memory_outputs() {
    let _env_guard = env_isolated();
    let engine = SessionEngine::for_tests().await.unwrap();
    let session_id = engine
        .new_session("acme", "alice", "coding-agent")
        .await
        .unwrap();
    engine
        .add_message(&session_id, "user", "My name is Alice Example")
        .await
        .unwrap();

    let result = engine.commit(&session_id).await.unwrap();
    let task = wait_for_task(&engine, result.task_id.as_deref().unwrap()).await;

    assert_eq!(task.memories_extracted.get("profile"), Some(&1));

    let profile_memory = tokio::fs::read_to_string(
        engine
            .workspace_root()
            .join("tenants/acme/alice/user/memories/profile.md"),
    )
    .await
    .unwrap();

    assert!(profile_memory.contains("Alice Example"));
}

#[tokio::test]
async fn commit_writes_tool_memory_outputs() {
    let _env_guard = env_isolated();
    let engine = SessionEngine::for_tests().await.unwrap();
    let session_id = engine
        .new_session("acme", "alice", "coding-agent")
        .await
        .unwrap();
    engine
        .used_tool(&session_id, "mfs://agent/tools/read_file", true)
        .await
        .unwrap();

    let result = engine.commit(&session_id).await.unwrap();
    let task = wait_for_task(&engine, result.task_id.as_deref().unwrap()).await;

    assert_eq!(task.memories_extracted.get("tools"), Some(&1));

    let tool_memory = tokio::fs::read_to_string(
        engine
            .workspace_root()
            .join("tenants/acme/alice/agent/alice__coding-agent/memories/tools/read-file.md"),
    )
    .await
    .unwrap();

    assert!(tool_memory.contains("mfs://agent/tools/read_file"));
}

#[tokio::test]
async fn commit_writes_event_and_case_outputs() {
    let _env_guard = env_isolated();
    let engine = SessionEngine::for_tests().await.unwrap();
    let session_id = engine
        .new_session("acme", "alice", "coding-agent")
        .await
        .unwrap();
    engine
        .add_message(
            &session_id,
            "user",
            "We decided to use SQLite for metadata storage",
        )
        .await
        .unwrap();
    engine
        .add_message(
            &session_id,
            "assistant",
            "Resolved oauth incident by rotating refresh tokens",
        )
        .await
        .unwrap();

    let result = engine.commit(&session_id).await.unwrap();
    let task = wait_for_task(&engine, result.task_id.as_deref().unwrap()).await;

    assert_eq!(task.memories_extracted.get("events"), Some(&1));
    assert_eq!(task.memories_extracted.get("cases"), Some(&1));

    let events_dir = engine
        .workspace_root()
        .join("tenants/acme/alice/user/memories/events");
    let cases_dir = engine
        .workspace_root()
        .join("tenants/acme/alice/agent/alice__coding-agent/memories/cases");

    let event_entries = std::fs::read_dir(&events_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(|name| {
                    name.ends_with(".md")
                        && !name.starts_with(".")
                        && !name.ends_with(".abstract.md")
                        && !name.ends_with(".overview.md")
                })
                .unwrap_or(false)
        })
        .collect::<Vec<_>>();
    let case_entries = std::fs::read_dir(&cases_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(|name| {
                    name.ends_with(".md")
                        && !name.starts_with(".")
                        && !name.ends_with(".abstract.md")
                        && !name.ends_with(".overview.md")
                })
                .unwrap_or(false)
        })
        .collect::<Vec<_>>();

    assert_eq!(event_entries.len(), 1);
    assert_eq!(case_entries.len(), 1);

    let event_memory = tokio::fs::read_to_string(&event_entries[0]).await.unwrap();
    let case_memory = tokio::fs::read_to_string(&case_entries[0]).await.unwrap();

    assert!(event_memory.contains("SQLite for metadata storage"));
    assert!(case_memory.contains("Resolved oauth incident"));
}

#[tokio::test]
async fn commit_writes_entity_memory_outputs() {
    let _env_guard = env_isolated();
    let engine = SessionEngine::for_tests().await.unwrap();
    let session_id = engine
        .new_session("acme", "alice", "coding-agent")
        .await
        .unwrap();
    engine
        .add_message(
            &session_id,
            "user",
            "Project Atlas uses Rust for the control plane",
        )
        .await
        .unwrap();

    let result = engine.commit(&session_id).await.unwrap();
    let task = wait_for_task(&engine, result.task_id.as_deref().unwrap()).await;

    assert_eq!(task.memories_extracted.get("entities"), Some(&1));

    let entity_memory = tokio::fs::read_to_string(
        engine
            .workspace_root()
            .join("tenants/acme/alice/user/memories/entities/project-atlas.md"),
    )
    .await
    .unwrap();

    assert!(entity_memory.contains("Project Atlas uses Rust"));
}

#[tokio::test]
async fn commit_merges_similar_entity_memories_into_single_entry() {
    let _env_guard = env_isolated();
    let engine = SessionEngine::for_tests().await.unwrap();

    let first_session = engine
        .new_session("acme", "alice", "coding-agent")
        .await
        .unwrap();
    engine
        .add_message(
            &first_session,
            "user",
            "Project Atlas uses Rust for the control plane",
        )
        .await
        .unwrap();
    let first_commit = engine.commit(&first_session).await.unwrap();
    let _ = wait_for_task(&engine, first_commit.task_id.as_deref().unwrap()).await;

    let second_session = engine
        .new_session("acme", "alice", "coding-agent")
        .await
        .unwrap();
    engine
        .add_message(&second_session, "user", "Project Atlas uses Rust")
        .await
        .unwrap();
    let second_commit = engine.commit(&second_session).await.unwrap();
    let _ = wait_for_task(&engine, second_commit.task_id.as_deref().unwrap()).await;

    let entity_memory = tokio::fs::read_to_string(
        engine
            .workspace_root()
            .join("tenants/acme/alice/user/memories/entities/project-atlas.md"),
    )
    .await
    .unwrap();

    let bullet_count = entity_memory
        .lines()
        .filter(|line| line.trim_start().starts_with("- "))
        .count();
    assert_eq!(bullet_count, 1);
}

#[tokio::test]
async fn commit_writes_pattern_memory_outputs() {
    let _env_guard = env_isolated();
    let engine = SessionEngine::for_tests().await.unwrap();
    let session_id = engine
        .new_session("acme", "alice", "coding-agent")
        .await
        .unwrap();
    engine
        .add_message(
            &session_id,
            "assistant",
            "Use MFS search then inspect overview before grep",
        )
        .await
        .unwrap();

    let result = engine.commit(&session_id).await.unwrap();
    let task = wait_for_task(&engine, result.task_id.as_deref().unwrap()).await;

    assert_eq!(task.memories_extracted.get("patterns"), Some(&1));

    let pattern_memory =
        tokio::fs::read_to_string(engine.workspace_root().join(
            "tenants/acme/alice/agent/alice__coding-agent/memories/patterns/use-mfs-search.md",
        ))
        .await
        .unwrap();

    assert!(pattern_memory.contains("Use MFS search then inspect overview before grep"));
}

#[tokio::test]
async fn commit_merges_similar_pattern_memories_into_single_entry() {
    let _env_guard = env_isolated();
    let engine = SessionEngine::for_tests().await.unwrap();

    let first_session = engine
        .new_session("acme", "alice", "coding-agent")
        .await
        .unwrap();
    engine
        .add_message(
            &first_session,
            "assistant",
            "Use MFS search then inspect overview before grep",
        )
        .await
        .unwrap();
    let first_commit = engine.commit(&first_session).await.unwrap();
    let _ = wait_for_task(&engine, first_commit.task_id.as_deref().unwrap()).await;

    let second_session = engine
        .new_session("acme", "alice", "coding-agent")
        .await
        .unwrap();
    engine
        .add_message(
            &second_session,
            "assistant",
            "Use MFS search then inspect abstract before grep",
        )
        .await
        .unwrap();
    let second_commit = engine.commit(&second_session).await.unwrap();
    let _ = wait_for_task(&engine, second_commit.task_id.as_deref().unwrap()).await;

    let pattern_memory =
        tokio::fs::read_to_string(engine.workspace_root().join(
            "tenants/acme/alice/agent/alice__coding-agent/memories/patterns/use-mfs-search.md",
        ))
        .await
        .unwrap();

    let bullet_count = pattern_memory
        .lines()
        .filter(|line| line.trim_start().starts_with("- "))
        .count();
    assert_eq!(bullet_count, 1);
}

#[tokio::test]
async fn recover_pending_redo_replays_memory_pipeline() {
    let workspace = tempfile::tempdir().unwrap();
    let archive_root = workspace
        .path()
        .join("tenants/acme/alice/session/coding-agent/session-1/history/archive_001");
    tokio::fs::create_dir_all(&archive_root).await.unwrap();
    tokio::fs::write(
        archive_root.join("messages.jsonl"),
        "{\"role\":\"user\",\"content\":\"redo this\"}\n",
    )
    .await
    .unwrap();
    tokio::fs::write(
        archive_root.join("usage.json"),
        "[{\"kind\":\"context\",\"uri\":\"mfs://resources/localfs/docs/api.md\",\"success\":null}]",
    )
    .await
    .unwrap();

    let redo_dir = workspace.path().join("_system/redo");
    tokio::fs::create_dir_all(&redo_dir).await.unwrap();
    tokio::fs::write(
        redo_dir.join("task-1.json"),
        r#"{
  "task_id": "task-1",
  "archive_uri": "mfs://session/coding-agent/session-1/history/archive_001",
  "archive_path": "ARCHIVE_PATH",
  "account_id": "acme",
  "user_id": "alice",
  "agent_id": "coding-agent",
  "session_id": "session-1"
}"#
        .replace("ARCHIVE_PATH", &archive_root.to_string_lossy()),
    )
    .await
    .unwrap();

    let engine = SessionEngine::open(workspace.path()).await.unwrap();
    let recovered = engine.recover_pending_redo().await.unwrap();

    assert_eq!(recovered, 1);
    assert!(
        tokio::fs::try_exists(archive_root.join(".done"))
            .await
            .unwrap()
    );
    assert!(
        tokio::fs::try_exists(workspace.path().join(
            "tenants/acme/alice/user/memories/session/coding-agent/session-1/archive_001.md"
        ))
        .await
        .unwrap()
    );
    assert!(
        !tokio::fs::try_exists(redo_dir.join("task-1.json"))
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn concurrent_empty_commit_does_not_skip_archive_index() {
    let engine = SessionEngine::for_tests().await.unwrap();
    let session_id = engine
        .new_session("acme", "alice", "coding-agent")
        .await
        .unwrap();
    engine
        .add_message(&session_id, "user", "first")
        .await
        .unwrap();

    let (first, second) = tokio::join!(engine.commit(&session_id), engine.commit(&session_id));
    let first = first.unwrap();
    let second = second.unwrap();
    let results = [first.archive_uri, second.archive_uri];

    assert!(results.iter().any(|uri| uri.ends_with("archive_001")));
    assert!(results.iter().any(|uri| uri.is_empty()));

    engine
        .add_message(&session_id, "user", "second")
        .await
        .unwrap();
    let third = engine.commit(&session_id).await.unwrap();

    assert!(third.archive_uri.ends_with("archive_002"));
}

#[tokio::test]
async fn concurrent_commits_from_distinct_engines_allocate_distinct_archives() {
    let workspace = tempfile::tempdir().unwrap();
    let engine_a = SessionEngine::open(workspace.path()).await.unwrap();
    let engine_b = SessionEngine::open(workspace.path()).await.unwrap();
    let session_id = "shared-session";

    engine_a
        .new_session_with_id("acme", "alice", "coding-agent", session_id)
        .await
        .unwrap();
    engine_b
        .new_session_with_id("acme", "alice", "coding-agent", session_id)
        .await
        .unwrap();
    engine_a
        .add_message(session_id, "user", "first engine")
        .await
        .unwrap();
    engine_b
        .add_message(session_id, "user", "second engine")
        .await
        .unwrap();

    let (left, right) = tokio::join!(engine_a.commit(session_id), engine_b.commit(session_id));
    let left = left.unwrap();
    let right = right.unwrap();
    let archives = [left.archive_uri, right.archive_uri];

    assert!(archives.iter().any(|uri| uri.ends_with("archive_001")));
    assert!(archives.iter().any(|uri| uri.ends_with("archive_002")));
}

#[tokio::test]
async fn metadata_memory_pipeline_persists_turns_facts_episodes_cursor_and_brief() {
    let _env_guard = env_isolated();
    let engine = SessionEngine::for_tests().await.unwrap();
    let session_id = engine
        .new_session_with_id("acme", "alice", "coding-agent", "session-memory-1")
        .await
        .unwrap();
    engine
        .add_message(
            &session_id,
            "user",
            "My name is Alice Example and I live in Tokyo",
        )
        .await
        .unwrap();
    engine
        .add_message(
            &session_id,
            "assistant",
            "Confirmed. You are Alice Example in Tokyo.",
        )
        .await
        .unwrap();

    let result = engine.commit(&session_id).await.unwrap();
    let task = wait_for_task(&engine, result.task_id.as_deref().unwrap()).await;
    assert_eq!(task.status, TaskStatus::Completed);

    let metadata = MetadataStore::open_at(
        engine.workspace_root().join("_system/metadata.sqlite"),
        false,
    )
    .unwrap();

    let session_row = metadata.get_session(&session_id).unwrap();
    assert!(session_row.is_some());

    let turns = metadata.get_turns_by_session(&session_id).unwrap();
    assert_eq!(turns.len(), 2);

    let active_facts = metadata.get_active_facts("acme", "alice").unwrap();
    assert!(
        active_facts
            .iter()
            .any(|fact| fact.predicate == "identity.name" && fact.display_value.contains("Alice"))
    );
    assert!(
        active_facts
            .iter()
            .any(|fact| fact.predicate == "location.current_city"
                && fact.display_value.contains("Tokyo"))
    );

    let episodes = metadata
        .get_episodes_by_user("acme", "alice", None)
        .unwrap();
    assert_eq!(episodes.len(), 1);

    let assertions = metadata
        .get_assertions_by_source(None, Some(&episodes[0].episode_id))
        .unwrap();
    assert!(!assertions.is_empty());

    let cursor = metadata
        .get_cursor("acme", "alice", "thread", &session_id)
        .unwrap();
    assert!(cursor.is_some());
    assert_eq!(
        cursor.unwrap().last_consolidated_turn_id.as_deref(),
        Some(turns.last().unwrap().turn_id.as_str())
    );

    let brief = metadata
        .get_brief("acme", "alice", "user", "alice")
        .unwrap();
    assert!(brief.is_some());
}

#[tokio::test]
async fn metadata_memory_pipeline_supersedes_scalar_facts_across_sessions() {
    let _env_guard = env_isolated();
    let engine = SessionEngine::for_tests().await.unwrap();

    let first_session = engine
        .new_session_with_id("acme", "alice", "coding-agent", "session-memory-a")
        .await
        .unwrap();
    engine
        .add_message(&first_session, "user", "I live in Tokyo")
        .await
        .unwrap();
    let first_commit = engine.commit(&first_session).await.unwrap();
    let first_task = wait_for_task(&engine, first_commit.task_id.as_deref().unwrap()).await;
    assert_eq!(first_task.status, TaskStatus::Completed);

    let second_session = engine
        .new_session_with_id("acme", "alice", "coding-agent", "session-memory-b")
        .await
        .unwrap();
    engine
        .add_message(&second_session, "user", "I just moved to Paris")
        .await
        .unwrap();
    let second_commit = engine.commit(&second_session).await.unwrap();
    let second_task = wait_for_task(&engine, second_commit.task_id.as_deref().unwrap()).await;
    assert_eq!(second_task.status, TaskStatus::Completed);

    let metadata = MetadataStore::open_at(
        engine.workspace_root().join("_system/metadata.sqlite"),
        false,
    )
    .unwrap();
    let active_facts = metadata.get_active_facts("acme", "alice").unwrap();
    let location_facts: Vec<_> = active_facts
        .iter()
        .filter(|fact| fact.predicate == "location.current_city")
        .collect();
    assert_eq!(location_facts.len(), 1);
    assert!(location_facts[0].display_value.contains("Paris"));

    let episodes = metadata
        .get_episodes_by_user("acme", "alice", None)
        .unwrap();
    assert_eq!(episodes.len(), 2);

    let brief = metadata
        .get_brief("acme", "alice", "user", "alice")
        .unwrap()
        .expect("expected user brief");
    let source_threads = brief.source_thread_ids_json.unwrap_or_default();
    assert!(source_threads.contains("session-memory-a"));
    assert!(source_threads.contains("session-memory-b"));
}

#[tokio::test]
async fn profile_memory_tracks_latest_location_from_canonical_facts() {
    let _env_guard = env_isolated();
    let engine = SessionEngine::for_tests().await.unwrap();

    let first_session = engine
        .new_session_with_id("acme", "alice", "coding-agent", "session-profile-a")
        .await
        .unwrap();
    engine
        .add_message(&first_session, "user", "I live in Tokyo")
        .await
        .unwrap();
    let first_commit = engine.commit(&first_session).await.unwrap();
    let first_task = wait_for_task(&engine, first_commit.task_id.as_deref().unwrap()).await;
    assert_eq!(first_task.status, TaskStatus::Completed);

    let second_session = engine
        .new_session_with_id("acme", "alice", "coding-agent", "session-profile-b")
        .await
        .unwrap();
    engine
        .add_message(&second_session, "user", "I just moved to Paris")
        .await
        .unwrap();
    let second_commit = engine.commit(&second_session).await.unwrap();
    let second_task = wait_for_task(&engine, second_commit.task_id.as_deref().unwrap()).await;
    assert_eq!(second_task.status, TaskStatus::Completed);

    let profile_memory = tokio::fs::read_to_string(
        engine
            .workspace_root()
            .join("tenants/acme/alice/user/memories/profile.md"),
    )
    .await
    .unwrap();

    assert!(profile_memory.contains("Paris"));
    assert!(!profile_memory.contains("Tokyo"));
}

#[tokio::test]
async fn preferences_memory_uses_canonical_communication_style_facts() {
    let _env_guard = env_isolated();
    let engine = SessionEngine::for_tests().await.unwrap();
    let session_id = engine
        .new_session_with_id("acme", "alice", "coding-agent", "session-preferences-style")
        .await
        .unwrap();
    engine
        .add_message(
            &session_id,
            "user",
            "Please communicate more concise with me",
        )
        .await
        .unwrap();

    let result = engine.commit(&session_id).await.unwrap();
    let task = wait_for_task(&engine, result.task_id.as_deref().unwrap()).await;
    assert_eq!(task.status, TaskStatus::Completed);

    let preference_memory = tokio::fs::read_to_string(
        engine
            .workspace_root()
            .join("tenants/acme/alice/user/memories/preferences/general.md"),
    )
    .await
    .unwrap();

    assert!(preference_memory.to_ascii_lowercase().contains("concise"));
}

#[tokio::test]
async fn entities_memory_uses_canonical_project_facts() {
    let _env_guard = env_isolated();
    let engine = SessionEngine::for_tests().await.unwrap();
    let session_id = engine
        .new_session_with_id("acme", "alice", "coding-agent", "session-entity-project")
        .await
        .unwrap();
    engine
        .add_message(&session_id, "user", "I'm working on Atlas")
        .await
        .unwrap();

    let result = engine.commit(&session_id).await.unwrap();
    let task = wait_for_task(&engine, result.task_id.as_deref().unwrap()).await;
    assert_eq!(task.status, TaskStatus::Completed);

    let entity_memory = tokio::fs::read_to_string(
        engine
            .workspace_root()
            .join("tenants/acme/alice/user/memories/entities/atlas.md"),
    )
    .await
    .unwrap();

    assert!(entity_memory.contains("Atlas"));
    assert!(entity_memory.contains("User is working on Atlas"));
}

#[tokio::test]
async fn commit_auto_links_session_to_consumed_resources_and_skills() {
    let _env_guard = env_isolated();
    let engine = SessionEngine::for_tests().await.unwrap();
    let session_id = engine
        .new_session("acme", "alice", "coding-agent")
        .await
        .unwrap();
    engine
        .add_message(&session_id, "user", "I need auth docs")
        .await
        .unwrap();
    engine
        .used_context(&session_id, "mfs://resources/localfs/docs/auth.md")
        .await
        .unwrap();
    engine
        .used_skill(
            &session_id,
            "mfs://agent/alice__coding-agent/skills/search",
            true,
        )
        .await
        .unwrap();

    let result = engine.commit(&session_id).await.unwrap();
    let task = wait_for_task(&engine, result.task_id.as_deref().unwrap()).await;
    assert_eq!(task.status, TaskStatus::Completed);

    let metadata = MetadataStore::open_at(
        engine.workspace_root().join("_system/metadata.sqlite"),
        false,
    )
    .unwrap();
    let session_uri = format!("mfs://session/coding-agent/{session_id}");
    let relations = metadata
        .list_relations("acme", "alice", &session_uri, 10)
        .unwrap();

    assert_eq!(relations.len(), 2);

    let resource_link = relations
        .iter()
        .find(|r| r.relation_type == "accessed" && r.to_uri.contains("resources"))
        .expect("expected session-resource link");
    assert_eq!(resource_link.from_uri, session_uri);
    assert_eq!(resource_link.to_uri, "mfs://resources/localfs/docs/auth.md");

    let skill_link = relations
        .iter()
        .find(|r| r.relation_type == "accessed" && r.to_uri.contains("skills"))
        .expect("expected session-skill link");
    assert_eq!(skill_link.from_uri, session_uri);
    assert_eq!(
        skill_link.to_uri,
        "mfs://agent/alice__coding-agent/skills/search"
    );
}

#[tokio::test]
async fn commit_rebuilds_layered_summaries_after_memory_writeback() {
    let _env_guard = env_isolated();
    let engine = SessionEngine::for_tests().await.unwrap();
    let session_id = engine
        .new_session("acme", "alice", "coding-agent")
        .await
        .unwrap();
    engine
        .add_message(&session_id, "user", "I prefer short API examples")
        .await
        .unwrap();

    let memories_root = engine
        .workspace_root()
        .join("tenants/acme/alice/user/memories");
    let abstract_path = memories_root.join(".abstract.md");
    let overview_path = memories_root.join(".overview.md");

    assert!(!tokio::fs::try_exists(&abstract_path).await.unwrap());
    assert!(!tokio::fs::try_exists(&overview_path).await.unwrap());

    let result = engine.commit(&session_id).await.unwrap();
    let task = wait_for_task(&engine, result.task_id.as_deref().unwrap()).await;
    assert_eq!(task.status, TaskStatus::Completed);

    assert!(tokio::fs::try_exists(&abstract_path).await.unwrap());
    assert!(tokio::fs::try_exists(&overview_path).await.unwrap());

    let overview_content = tokio::fs::read_to_string(&overview_path).await.unwrap();
    assert!(overview_content.contains("user/memories"));
    assert!(overview_content.contains("preferences"));
}

#[tokio::test]
async fn commit_rebuilds_agent_memory_summaries_after_pattern_extraction() {
    let _env_guard = env_isolated();
    let engine = SessionEngine::for_tests().await.unwrap();
    let session_id = engine
        .new_session("acme", "alice", "coding-agent")
        .await
        .unwrap();
    engine
        .add_message(
            &session_id,
            "assistant",
            "Use MFS search then inspect overview before grep",
        )
        .await
        .unwrap();

    let agent_memories_root = engine
        .workspace_root()
        .join("tenants/acme/alice/agent/alice__coding-agent/memories");
    let abstract_path = agent_memories_root.join(".abstract.md");

    let result = engine.commit(&session_id).await.unwrap();
    let task = wait_for_task(&engine, result.task_id.as_deref().unwrap()).await;
    assert_eq!(task.status, TaskStatus::Completed);

    assert!(tokio::fs::try_exists(&abstract_path).await.unwrap());
    let abstract_content = tokio::fs::read_to_string(&abstract_path).await.unwrap();
    assert!(abstract_content.contains("agent/memories"));
}

async fn wait_for_task(engine: &SessionEngine, task_id: &str) -> mfs_session::TaskRecord {
    for _ in 0..50 {
        if let Some(task) = engine.task_status(task_id).await {
            if matches!(task.status, TaskStatus::Completed | TaskStatus::Failed) {
                return task;
            }
        }
        sleep(Duration::from_millis(20)).await;
    }

    panic!("task {task_id} did not finish in time");
}

#[tokio::test]
async fn auto_commit_triggers_when_estimated_tokens_exceed_threshold() {
    let _env_guard = env_isolated();
    let engine = SessionEngine::for_tests_with_threshold(5).await.unwrap();
    let session_id = engine
        .new_session("acme", "alice", "coding-agent")
        .await
        .unwrap();

    // Send a message that exceeds the threshold (20 chars / 4 = 5 tokens >= 5 threshold).
    let result = engine
        .add_message(&session_id, "user", "This is twenty chars!!")
        .await
        .unwrap();
    assert!(
        result.auto_committed,
        "expected auto_committed=true when tokens exceed threshold"
    );
    assert!(
        result.archive_uri.is_some(),
        "expected archive_uri when auto-committed"
    );
    assert!(
        result.task_id.is_some(),
        "expected task_id when auto-committed"
    );

    // Verify the session was drained (no messages left).
    let session = engine.get_session(&session_id).await.unwrap();
    assert_eq!(
        session.message_count, 0,
        "session should be empty after auto-commit"
    );
}

#[tokio::test]
async fn auto_commit_disabled_when_threshold_is_zero() {
    let _env_guard = env_isolated();
    let engine = SessionEngine::for_tests_with_threshold(0).await.unwrap();
    let session_id = engine
        .new_session("acme", "alice", "coding-agent")
        .await
        .unwrap();

    // Send several messages — none should trigger auto-commit.
    let result1 = engine
        .add_message(&session_id, "user", "first message with some content")
        .await
        .unwrap();
    assert!(
        !result1.auto_committed,
        "expected auto_committed=false when threshold=0"
    );

    let result2 = engine
        .add_message(
            &session_id,
            "user",
            "second message with more content here too",
        )
        .await
        .unwrap();
    assert!(
        !result2.auto_committed,
        "expected auto_committed=false when threshold=0"
    );

    // Messages should still be in the session.
    let session = engine.get_session(&session_id).await.unwrap();
    assert_eq!(
        session.message_count, 2,
        "session should retain messages when auto-commit disabled"
    );
}

#[tokio::test]
async fn auto_commit_resets_estimated_tokens_after_commit() {
    let _env_guard = env_isolated();
    let engine = SessionEngine::for_tests_with_threshold(5).await.unwrap();
    let session_id = engine
        .new_session("acme", "alice", "coding-agent")
        .await
        .unwrap();

    // First message triggers auto-commit (20 chars / 4 = 5 tokens >= 5 threshold).
    let result1 = engine
        .add_message(&session_id, "user", "This is twenty chars!!")
        .await
        .unwrap();
    assert!(result1.auto_committed);

    // After auto-commit, session is reset. Next short message should NOT auto-commit.
    let result2 = engine.add_message(&session_id, "user", "x").await.unwrap();
    assert!(
        !result2.auto_committed,
        "expected auto_committed=false after reset — short message (1 token < 5 threshold)"
    );
}
