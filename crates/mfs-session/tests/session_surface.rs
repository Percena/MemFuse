use mfs_session::SessionEngine;
use mfs_test_util::env_isolated;
use std::time::Duration;
use tokio::time::sleep;

#[tokio::test]
async fn list_sessions_returns_known_session_ids() {
    let engine = SessionEngine::for_tests().await.unwrap();
    engine
        .new_session_with_id("acme", "alice", "coding-agent", "session-a")
        .await
        .unwrap();
    engine
        .new_session_with_id("acme", "alice", "coding-agent", "session-b")
        .await
        .unwrap();

    let sessions = engine
        .list_sessions("acme", "alice", "coding-agent")
        .await
        .unwrap();

    assert_eq!(sessions.len(), 2);
    assert!(
        sessions
            .iter()
            .any(|session| session.session_id == "session-a")
    );
    assert!(
        sessions
            .iter()
            .any(|session| session.session_id == "session-b")
    );
}

#[tokio::test]
async fn get_session_reports_message_and_commit_counts() {
    let engine = SessionEngine::for_tests().await.unwrap();
    let session_id = engine
        .new_session_with_id("acme", "alice", "coding-agent", "session-a")
        .await
        .unwrap();
    engine
        .add_message(&session_id, "user", "hello")
        .await
        .unwrap();

    let session = engine.get_session(&session_id).await.unwrap();

    assert_eq!(session.session_id, session_id);
    assert_eq!(session.message_count, 1);
    assert_eq!(session.commit_count, 0);
}

#[tokio::test]
async fn recreating_existing_session_preserves_live_messages_from_disk() {
    let workspace = tempfile::tempdir().unwrap();
    let engine = SessionEngine::open(workspace.path()).await.unwrap();
    let session_id = engine
        .new_session_with_id("acme", "alice", "coding-agent", "session-a")
        .await
        .unwrap();
    engine
        .add_message(&session_id, "user", "message before restart")
        .await
        .unwrap();
    drop(engine);

    let restarted = SessionEngine::open(workspace.path()).await.unwrap();
    restarted
        .new_session_with_id("acme", "alice", "coding-agent", "session-a")
        .await
        .unwrap();
    restarted
        .add_message(&session_id, "assistant", "message after restart")
        .await
        .unwrap();

    let context = restarted
        .get_session_context(&session_id, 128_000)
        .await
        .unwrap();
    let contents = context
        .messages
        .iter()
        .map(|message| message.content.as_str())
        .collect::<Vec<_>>();

    assert_eq!(
        contents,
        vec!["message before restart", "message after restart"]
    );
}

#[tokio::test]
async fn get_session_context_returns_latest_overview_and_messages() {
    let _env_guard = env_isolated();
    let engine = SessionEngine::for_tests().await.unwrap();
    let session_id = engine
        .new_session_with_id("acme", "alice", "coding-agent", "session-a")
        .await
        .unwrap();
    engine
        .add_message(&session_id, "user", "turn one")
        .await
        .unwrap();
    let commit = engine.commit(&session_id).await.unwrap();
    wait_for_task(&engine, commit.task_id.as_deref().unwrap()).await;
    engine
        .add_message(&session_id, "user", "active message")
        .await
        .unwrap();

    let context = engine
        .get_session_context(&session_id, 128_000)
        .await
        .unwrap();

    assert!(context.latest_archive_overview.contains("Archive:"));
    assert!(!context.messages.is_empty());
    assert_eq!(context.messages[0].content, "active message");
}

#[tokio::test]
async fn get_session_archive_returns_messages_and_summaries() {
    let _env_guard = env_isolated();
    let engine = SessionEngine::for_tests().await.unwrap();
    let session_id = engine
        .new_session_with_id("acme", "alice", "coding-agent", "session-a")
        .await
        .unwrap();
    engine
        .add_message(&session_id, "user", "turn one")
        .await
        .unwrap();
    let commit = engine.commit(&session_id).await.unwrap();
    wait_for_task(&engine, commit.task_id.as_deref().unwrap()).await;

    let archive = engine
        .get_session_archive(&session_id, "archive_001")
        .await
        .unwrap();

    assert_eq!(archive.archive_id, "archive_001");
    assert!(archive.abstract_text.len() > 0);
    assert!(archive.overview_text.len() > 0);
    assert!(!archive.messages.is_empty());
}

#[tokio::test]
async fn delete_session_removes_it_from_read_surfaces() {
    let _env_guard = env_isolated();
    let engine = SessionEngine::for_tests().await.unwrap();
    let session_id = engine
        .new_session_with_id("acme", "alice", "coding-agent", "session-delete")
        .await
        .unwrap();
    engine
        .add_message(&session_id, "user", "turn one")
        .await
        .unwrap();
    let commit = engine.commit(&session_id).await.unwrap();
    wait_for_task(&engine, commit.task_id.as_deref().unwrap()).await;

    engine.delete_session(&session_id).await.unwrap();

    let sessions = engine
        .list_sessions("acme", "alice", "coding-agent")
        .await
        .unwrap();
    assert!(
        !sessions
            .iter()
            .any(|session| session.session_id == session_id)
    );
    assert!(engine.get_session(&session_id).await.is_err());
    assert!(
        engine
            .get_session_context(&session_id, 128_000)
            .await
            .is_err()
    );
    assert!(
        engine
            .get_session_archive(&session_id, "archive_001")
            .await
            .is_err()
    );
}

async fn wait_for_task(engine: &SessionEngine, task_id: &str) {
    for _ in 0..100 {
        if let Some(task) = engine.task_status(task_id).await {
            if matches!(task.status, mfs_session::TaskStatus::Completed) {
                return;
            }
        }
        sleep(Duration::from_millis(10)).await;
    }

    panic!("task {task_id} did not complete");
}
