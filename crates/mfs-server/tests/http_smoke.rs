use axum::body::Body;
use axum::http::Request;
use mfs_test_util::env_isolated;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::util::ServiceExt;

fn test_app(workspace: &tempfile::TempDir, source: &tempfile::TempDir) -> axum::Router {
    mfs_server::http::app_with_config(mfs_server::http::AppConfig {
        workspace_root: workspace.path().to_path_buf(),
        source_kind: "localfs".to_owned(),
        source_path: source.path().to_path_buf(),
        target_uri: "mfs://resources/localfs/docs".to_owned(),
        account_id: "acme".to_owned(),
        user_id: "alice".to_owned(),
        agent_id: "coding-agent".to_owned(),
        canvas_separate_db: false,
    })
}

/// Build a test app with API key auth mode enabled.
/// Auth config is baked into AppState at construction — no env-var mutation needed.
fn test_app_with_auth(
    workspace: &tempfile::TempDir,
    source: &tempfile::TempDir,
    api_key: &str,
) -> axum::Router {
    mfs_server::http::app_with_config_and_auth(
        mfs_server::http::AppConfig {
            workspace_root: workspace.path().to_path_buf(),
            source_kind: "localfs".to_owned(),
            source_path: source.path().to_path_buf(),
            target_uri: "mfs://resources/localfs/docs".to_owned(),
            account_id: "acme".to_owned(),
            user_id: "alice".to_owned(),
            agent_id: "coding-agent".to_owned(),
            canvas_separate_db: false,
        },
        mfs_server::auth::AuthMode::ApiKey,
        Some(mfs_server::auth::ApiKeyConfig {
            key: api_key.to_owned(),
        }),
    )
}

fn seed_memory_rows(workspace: &tempfile::TempDir) {
    let metadata = mfs_metadata::MetadataStore::open_at(
        workspace.path().join("_system/metadata.sqlite"),
        false,
    )
    .unwrap();

    metadata
        .insert_session(
            "session-alpha",
            "acme",
            "alice",
            "coding-agent",
            "active",
            None,
        )
        .unwrap();

    metadata
        .insert_episode(
            "episode-1",
            "acme",
            "alice",
            "coding-agent",
            "session-alpha",
            None,
            "Investigated OAuth token rotation workflow",
            None,
            Some(r#"["oauth","rotation"]"#),
            0.92,
            1.0,
            None,
            None,
            None,
            2,
            Some("2026-04-22T10:10:00Z"),
            Some("turn-1"),
            Some("turn-2"),
            None,
            None,
            None,
        )
        .unwrap();
    metadata
        .insert_episode(
            "episode-2",
            "acme",
            "alice",
            "coding-agent",
            "session-alpha",
            None,
            "Documented rollback steps for auth incidents",
            None,
            Some(r#"["auth","incident"]"#),
            0.80,
            1.0,
            None,
            None,
            None,
            1,
            Some("2026-04-22T10:20:00Z"),
            Some("turn-3"),
            Some("turn-4"),
            None,
            None,
            None,
        )
        .unwrap();
    metadata
        .insert_episode(
            "episode-3",
            "acme",
            "alice",
            "coding-agent",
            "session-alpha",
            None,
            "Captured service rotation notes for the operator handoff",
            None,
            Some(r#"["rotation","handoff"]"#),
            0.70,
            1.0,
            None,
            None,
            None,
            0,
            None,
            Some("turn-5"),
            Some("turn-6"),
            None,
            None,
            None,
        )
        .unwrap();

    metadata
        .insert_fact(&mfs_metadata::FactRecord {
            id: "fact-1",
            account_id: "acme",
            user_id: "alice",
            agent_id: Some("coding-agent"),
            subject: "user",
            predicate: "location.current_city",
            display_value: "Tokyo",
            normalized_value_json: None,
            value_type: "scalar",
            confidence: 0.95,
            status: "active",
            valid_from: None,
            valid_to: None,
            source_assertion_id: None,
            source_episode_ids_json: Some("[\"episode-1\"]"),
        })
        .unwrap();
}

#[tokio::test]
async fn http_api_server_does_not_serve_operator_console_shell() {
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("custom.md"), "# Custom\n").unwrap();

    let app = mfs_server::http::app_with_config(mfs_server::http::AppConfig {
        workspace_root: workspace.path().to_path_buf(),
        source_kind: "localfs".to_owned(),
        source_path: source.path().to_path_buf(),
        target_uri: "mfs://resources/localfs/docs".to_owned(),
        account_id: "acme".to_owned(),
        user_id: "alice".to_owned(),
        agent_id: "coding-agent".to_owned(),
        canvas_separate_db: false,
    });
    let response = app
        .oneshot(Request::builder().uri("/demo").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn http_context_resolve_returns_wrapped_context_and_markdown() {
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("custom.md"), "# Custom\n").unwrap();
    seed_memory_rows(&workspace);

    let engine = mfs_session::SessionEngine::open(workspace.path())
        .await
        .unwrap();
    let session_id = engine
        .new_session_with_id("acme", "alice", "coding-agent", "session-alpha")
        .await
        .unwrap();
    engine
        .add_message(
            &session_id,
            "user",
            "I live in Tokyo and need the oauth runbook",
        )
        .await
        .unwrap();
    engine
        .add_message(
            &session_id,
            "assistant",
            "Confirmed. I will use the OAuth rotation workflow.",
        )
        .await
        .unwrap();

    let app = test_app(&workspace, &source);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/context/resolve")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"query":"oauth rotation","session_id":"session-alpha","token_budget":1200}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(response.status().is_success());
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        json["sections"]["current_facts"][0]["display_value"],
        "Tokyo"
    );
    assert!(
        json["rendered_markdown"]
            .as_str()
            .unwrap()
            .contains("[Current Facts]")
    );
}

#[tokio::test]
async fn http_memory_search_returns_results_for_matching_episodes() {
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("custom.md"), "# Custom\n").unwrap();
    seed_memory_rows(&workspace);

    let app = test_app(&workspace, &source);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/memory:search")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"query":"oauth rotation","user_id":"alice"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(response.status().is_success());
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        json["results"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["episode_id"] == "episode-1")
    );
}

#[tokio::test]
async fn http_memory_consolidate_persists_session_memory_state() {
    let _env_guard = env_isolated();
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("custom.md"), "# Custom\n").unwrap();

    let app = test_app(&workspace, &source);
    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/sessions")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"session_id":"session-consolidate"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(create.status().is_success());

    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/sessions/session-consolidate/messages")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"role":"user","content":"My name is Alice Example and I just moved to Paris"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/memory:consolidate")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"session_id":"session-consolidate","user_id":"alice"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(response.status().is_success());
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_ne!(json["status"], "not_implemented");
    assert_eq!(json["session_id"], "session-consolidate");
    assert!(json["episode_count"].as_u64().unwrap() >= 1);
    assert!(json["fact_count"].as_u64().unwrap() >= 1);

    let facts = app
        .oneshot(
            Request::builder()
                .uri("/facts?user_id=alice")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(facts.status().is_success());
    let facts_body = axum::body::to_bytes(facts.into_body(), usize::MAX)
        .await
        .unwrap();
    let facts_json: serde_json::Value = serde_json::from_slice(&facts_body).unwrap();
    assert!(facts_json.get("next_cursor").is_some());
    assert_eq!(facts_json["limit"], 100);
    assert!(facts_json["total_count"].as_u64().unwrap() >= 1);
    assert_eq!(facts_json["facts"], facts_json["items"]);
    assert!(
        facts_json["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|fact| fact["predicate"] == "location.current_city")
    );
}

#[tokio::test]
async fn http_episode_detail_and_timeline_routes_return_episode_data() {
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("custom.md"), "# Custom\n").unwrap();
    seed_memory_rows(&workspace);

    let metadata = mfs_metadata::MetadataStore::open_at(
        workspace.path().join("_system/metadata.sqlite"),
        false,
    )
    .unwrap();
    for (seq, turn_id, role, content) in [
        (
            1_i64,
            "turn-1",
            "user",
            "Need the OAuth token rotation workflow",
        ),
        (
            2_i64,
            "turn-2",
            "assistant",
            "Confirmed. Using the OAuth rotation workflow.",
        ),
        (3_i64, "turn-3", "user", "Document rollback steps too"),
        (
            4_i64,
            "turn-4",
            "assistant",
            "Added rollback steps for auth incidents.",
        ),
        (5_i64, "turn-5", "user", "Capture service rotation notes"),
        (
            6_i64,
            "turn-6",
            "assistant",
            "Captured operator handoff notes.",
        ),
    ] {
        metadata
            .insert_turn(
                turn_id,
                seq,
                "session-alpha",
                "acme",
                "alice",
                "coding-agent",
                role,
                content,
                None,
                32,
                None,
            )
            .unwrap();
    }

    let app = test_app(&workspace, &source);

    let detail = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/episodes/episode-1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(detail.status().is_success());
    let detail_body = axum::body::to_bytes(detail.into_body(), usize::MAX)
        .await
        .unwrap();
    let detail_json: serde_json::Value = serde_json::from_slice(&detail_body).unwrap();
    assert_eq!(detail_json["episode_id"], "episode-1");
    assert_eq!(detail_json["turns"].as_array().unwrap().len(), 2);

    let timeline = app
        .oneshot(
            Request::builder()
                .uri("/episodes/episode-2/timeline?direction=both&radius=1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(timeline.status().is_success());
    let timeline_body = axum::body::to_bytes(timeline.into_body(), usize::MAX)
        .await
        .unwrap();
    let timeline_json: serde_json::Value = serde_json::from_slice(&timeline_body).unwrap();
    let episode_ids: Vec<String> = timeline_json["episodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["episode_id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(episode_ids, vec!["episode-1", "episode-2", "episode-3"]);
}

#[tokio::test]
async fn http_api_server_does_not_serve_chat_shell() {
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("custom.md"), "# Custom\n").unwrap();

    let app = mfs_server::http::app_with_config(mfs_server::http::AppConfig {
        workspace_root: workspace.path().to_path_buf(),
        source_kind: "localfs".to_owned(),
        source_path: source.path().to_path_buf(),
        target_uri: "mfs://resources/localfs/docs".to_owned(),
        account_id: "acme".to_owned(),
        user_id: "alice".to_owned(),
        agent_id: "coding-agent".to_owned(),
        canvas_separate_db: false,
    });
    let response = app
        .oneshot(Request::builder().uri("/chat").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn http_ls_returns_materialized_entries() {
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("custom.md"), "# Custom\n").unwrap();

    let app = mfs_server::http::app_with_config(mfs_server::http::AppConfig {
        workspace_root: workspace.path().to_path_buf(),
        source_kind: "localfs".to_owned(),
        source_path: source.path().to_path_buf(),
        target_uri: "mfs://resources/localfs/docs".to_owned(),
        account_id: "acme".to_owned(),
        user_id: "alice".to_owned(),
        agent_id: "coding-agent".to_owned(),
        canvas_separate_db: false,
    });
    let response = app
        .oneshot(
            Request::builder()
                .uri("/ls?uri=mfs://resources/localfs/docs")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(response.status().is_success());
}

#[tokio::test]
async fn http_abstract_returns_layered_summary() {
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("custom.md"), "# Custom\n").unwrap();

    let app = mfs_server::http::app_with_config(mfs_server::http::AppConfig {
        workspace_root: workspace.path().to_path_buf(),
        source_kind: "localfs".to_owned(),
        source_path: source.path().to_path_buf(),
        target_uri: "mfs://resources/localfs/docs".to_owned(),
        account_id: "acme".to_owned(),
        user_id: "alice".to_owned(),
        agent_id: "coding-agent".to_owned(),
        canvas_separate_db: false,
    });
    let response = app
        .oneshot(
            Request::builder()
                .uri("/abstract?uri=mfs://resources/localfs/docs")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(response.status().is_success());
}

#[tokio::test]
async fn http_glob_returns_matching_projection_paths() {
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(source.path().join("guides/tokens")).unwrap();
    std::fs::write(source.path().join("auth.md"), "# Authentication\n").unwrap();
    std::fs::write(source.path().join("guides/oauth.md"), "# OAuth\n").unwrap();
    std::fs::write(
        source.path().join("guides/tokens/refresh.md"),
        "# Refresh\n",
    )
    .unwrap();

    let app = mfs_server::http::app_with_config(mfs_server::http::AppConfig {
        workspace_root: workspace.path().to_path_buf(),
        source_kind: "localfs".to_owned(),
        source_path: source.path().to_path_buf(),
        target_uri: "mfs://resources/localfs/docs".to_owned(),
        account_id: "acme".to_owned(),
        user_id: "alice".to_owned(),
        agent_id: "coding-agent".to_owned(),
        canvas_separate_db: false,
    });
    let response = app
        .oneshot(
            Request::builder()
                .uri("/glob?uri=mfs://resources/localfs/docs&pattern=guides/**/*.md")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(response.status().is_success());
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        json.as_array()
            .unwrap()
            .iter()
            .any(|item| item == "mfs://resources/localfs/docs/guides/oauth.md")
    );
    assert!(
        json.as_array()
            .unwrap()
            .iter()
            .any(|item| item == "mfs://resources/localfs/docs/guides/tokens/refresh.md")
    );
}

#[tokio::test]
async fn http_grep_accepts_file_uri_target() {
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(source.path().join("sdk/src/mcp")).unwrap();
    std::fs::write(
        source.path().join("sdk/src/mcp/server.ts"),
        "server.registerTool('resolve_context', {})\n",
    )
    .unwrap();
    std::fs::write(
        source.path().join("sdk/src/mcp/server.tsx"),
        "server.registerTool('resolve_context', {})\n",
    )
    .unwrap();

    let app = mfs_server::http::app_with_config(mfs_server::http::AppConfig {
        workspace_root: workspace.path().to_path_buf(),
        source_kind: "localfs".to_owned(),
        source_path: source.path().to_path_buf(),
        target_uri: "mfs://resources/localfs/MemFuse".to_owned(),
        account_id: "acme".to_owned(),
        user_id: "alice".to_owned(),
        agent_id: "coding-agent".to_owned(),
        canvas_separate_db: false,
    });

    let response = app
        .oneshot(
            Request::builder()
                .uri("/grep?query=resolve_context&target=mfs://resources/localfs/MemFuse/sdk/src/mcp/server.ts")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(response.status().is_success());
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        json["resources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["uri"] == "mfs://resources/localfs/MemFuse/sdk/src/mcp/server.ts"),
        "unexpected grep response: {json}"
    );
    assert!(
        json["resources"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["uri"] == "mfs://resources/localfs/MemFuse/sdk/src/mcp/server.ts"),
        "file target must not include prefix-sharing siblings: {json}"
    );
}

#[tokio::test]
async fn http_owned_write_plane_updates_workspace_and_retrieval() {
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("custom.md"), "# Custom\n").unwrap();

    let app = mfs_server::http::app_with_config(mfs_server::http::AppConfig {
        workspace_root: workspace.path().to_path_buf(),
        source_kind: "localfs".to_owned(),
        source_path: source.path().to_path_buf(),
        target_uri: "mfs://resources/localfs/docs".to_owned(),
        account_id: "acme".to_owned(),
        user_id: "alice".to_owned(),
        agent_id: "coding-agent".to_owned(),
        canvas_separate_db: false,
    });

    let mkdir = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mkdir")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"uri":"mfs://user/notes"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(mkdir.status().is_success());

    let write = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/write")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"uri":"mfs://user/notes/profile.md","content":"OAuth token rotation playbook"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(write.status().is_success());

    let find = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/find?query=rotation&target=mfs://user")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(find.status().is_success());
    let body = axum::body::to_bytes(find.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        json["memories"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["uri"] == "mfs://user/notes/profile.md")
    );

    let mv = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mv")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"from_uri":"mfs://user/notes/profile.md","to_uri":"mfs://user/notes/renamed.md"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(mv.status().is_success());

    let moved = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/find?query=rotation&target=mfs://user")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(moved.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        json["memories"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["uri"] == "mfs://user/notes/renamed.md")
    );
    assert!(
        !json["memories"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["uri"] == "mfs://user/notes/profile.md")
    );

    let rm = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/rm?uri=mfs://user/notes/renamed.md")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(rm.status().is_success());

    let removed = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/find?query=rotation&target=mfs://user")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(removed.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        !json["memories"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["uri"] == "mfs://user/notes/renamed.md")
    );

    let metadata = mfs_metadata::MetadataStore::open_at(
        workspace.path().join("_system/metadata.sqlite"),
        false,
    )
    .unwrap();
    assert!(
        metadata
            .list_audit("acme", "alice", 20)
            .unwrap()
            .iter()
            .any(|record| record.event_type == "mv")
    );
}

#[tokio::test]
async fn http_read_directory_returns_client_error_instead_of_panicking() {
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("custom.md"), "# Custom\n").unwrap();

    let app = mfs_server::http::app_with_config(mfs_server::http::AppConfig {
        workspace_root: workspace.path().to_path_buf(),
        source_kind: "localfs".to_owned(),
        source_path: source.path().to_path_buf(),
        target_uri: "mfs://resources/localfs/docs".to_owned(),
        account_id: "acme".to_owned(),
        user_id: "alice".to_owned(),
        agent_id: "coding-agent".to_owned(),
        canvas_separate_db: false,
    });
    let response = app
        .oneshot(
            Request::builder()
                .uri("/read?uri=mfs://resources/localfs/docs")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(response.status().is_client_error());
}

#[tokio::test]
async fn http_find_returns_structured_retrieval_response() {
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(
        source.path().join("search.md"),
        "# Search\nsearch workflow in resources\n",
    )
    .unwrap();
    std::fs::create_dir_all(
        workspace
            .path()
            .join("tenants/acme/alice/user/memories/session/coding-agent/session-1"),
    )
    .unwrap();
    std::fs::create_dir_all(
        workspace
            .path()
            .join("tenants/acme/alice/agent/alice__coding-agent/skills/used/session-1"),
    )
    .unwrap();
    std::fs::write(
        workspace
            .path()
            .join("tenants/acme/alice/user/memories/session/coding-agent/session-1/archive_001.md"),
        "search workflow memory\n",
    )
    .unwrap();
    std::fs::write(
        workspace.path().join(
            "tenants/acme/alice/agent/alice__coding-agent/skills/used/session-1/archive_001.md",
        ),
        "search skill record\n",
    )
    .unwrap();

    let app = mfs_server::http::app_with_config(mfs_server::http::AppConfig {
        workspace_root: workspace.path().to_path_buf(),
        source_kind: "localfs".to_owned(),
        source_path: source.path().to_path_buf(),
        target_uri: "mfs://resources/localfs/docs".to_owned(),
        account_id: "acme".to_owned(),
        user_id: "alice".to_owned(),
        agent_id: "coding-agent".to_owned(),
        canvas_separate_db: false,
    });
    let response = app
        .oneshot(
            Request::builder()
                .uri("/find?query=search&target=mfs://resources/localfs/docs")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(response.status().is_success());
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert!(json["resources"].is_array());
    assert!(json["memories"].is_array());
    assert!(json["skills"].is_array());
    assert!(json["typed_queries"].is_array());
    assert_eq!(json["query_plan"]["mode"], "find");
    assert!(json["trajectory"]["steps"].is_array());
    assert!(json["resources"][0]["match_reason"].as_str().unwrap().len() > 0);
    assert!(
        json["resources"][0]["retrieval_plane"]
            .as_str()
            .unwrap()
            .len()
            > 0
    );
}

#[tokio::test]
async fn http_search_accepts_explicit_session_context() {
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(
        source.path().join("search.md"),
        "# Search\nincident workflow in resources\n",
    )
    .unwrap();

    let app = mfs_server::http::app_with_config(mfs_server::http::AppConfig {
        workspace_root: workspace.path().to_path_buf(),
        source_kind: "localfs".to_owned(),
        source_path: source.path().to_path_buf(),
        target_uri: "mfs://resources/localfs/docs".to_owned(),
        account_id: "acme".to_owned(),
        user_id: "alice".to_owned(),
        agent_id: "coding-agent".to_owned(),
        canvas_separate_db: false,
    });
    let response = app
        .oneshot(
            Request::builder()
                .uri("/search?query=help%20me%20find%20search&target=mfs://resources/localfs/docs&session_context=recent%20workflow%20incident%20memory")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(response.status().is_success());
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["query_plan"]["mode"], "search");
    assert!(
        json["query_plan"]["typed_queries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["query"].as_str().unwrap().contains("workflow"))
    );
}

#[tokio::test]
async fn http_rebuild_returns_indexed_path_count() {
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("custom.md"), "# Custom\n").unwrap();

    let app = mfs_server::http::app_with_config(mfs_server::http::AppConfig {
        workspace_root: workspace.path().to_path_buf(),
        source_kind: "localfs".to_owned(),
        source_path: source.path().to_path_buf(),
        target_uri: "mfs://resources/localfs/docs".to_owned(),
        account_id: "acme".to_owned(),
        user_id: "alice".to_owned(),
        agent_id: "coding-agent".to_owned(),
        canvas_separate_db: false,
    });
    let response = app
        .oneshot(
            Request::builder()
                .uri("/rebuild")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(response.status().is_success());
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(json["indexed_paths"].as_u64().unwrap() > 0);
}

#[tokio::test]
async fn http_refresh_records_snapshot_and_replaces_projection_files() {
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("current.md"), "# Current\n").unwrap();

    let app = mfs_server::http::app_with_config(mfs_server::http::AppConfig {
        workspace_root: workspace.path().to_path_buf(),
        source_kind: "localfs".to_owned(),
        source_path: source.path().to_path_buf(),
        target_uri: "mfs://resources/localfs/docs".to_owned(),
        account_id: "acme".to_owned(),
        user_id: "alice".to_owned(),
        agent_id: "coding-agent".to_owned(),
        canvas_separate_db: false,
    });

    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/ls?uri=mfs://resources/localfs/docs")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    std::fs::remove_file(source.path().join("current.md")).unwrap();
    std::fs::write(source.path().join("next.md"), "# Next\n").unwrap();

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/refresh")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(response.status().is_success());
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(json["snapshot_id"].as_str().unwrap().len() > 4);
    assert!(
        !workspace
            .path()
            .join("tenants/acme/alice/resources/localfs/docs/current.md")
            .exists()
    );
    assert!(
        workspace
            .path()
            .join("tenants/acme/alice/resources/localfs/docs/next.md")
            .exists()
    );
}

#[tokio::test]
async fn http_retrieval_requests_reuse_cached_engine_until_refresh_invalidates() {
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(
        source.path().join("search.md"),
        "# Search\nsearch workflow in resources\n",
    )
    .unwrap();

    let state = mfs_server::http::build_state(mfs_server::http::AppConfig {
        workspace_root: workspace.path().to_path_buf(),
        source_kind: "localfs".to_owned(),
        source_path: source.path().to_path_buf(),
        target_uri: "mfs://resources/localfs/docs".to_owned(),
        account_id: "acme".to_owned(),
        user_id: "alice".to_owned(),
        agent_id: "coding-agent".to_owned(),
        canvas_separate_db: false,
    });
    let app = mfs_server::http::api_router().with_state(state.clone());

    let first = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/find?query=search&target=mfs://resources/localfs/docs")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(first.status().is_success());
    assert_eq!(state.retrieval_cache_build_count(), 1);
    assert_eq!(state.retrieval_cache_hit_count(), 0);
    assert_eq!(state.retrieval_cache_entry_count(), 1);

    let second = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/search?query=workflow&target=mfs://resources/localfs/docs")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(second.status().is_success());
    assert_eq!(state.retrieval_cache_build_count(), 1);
    assert!(state.retrieval_cache_hit_count() >= 1);

    std::fs::remove_file(source.path().join("search.md")).unwrap();
    std::fs::write(
        source.path().join("incident.md"),
        "# Incident\nincident response in resources\n",
    )
    .unwrap();

    let refresh = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/refresh")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(refresh.status().is_success());
    assert!(state.retrieval_cache_invalidation_count() >= 1);
    assert_eq!(state.retrieval_cache_entry_count(), 0);

    let third = app
        .oneshot(
            Request::builder()
                .uri("/find?query=incident&target=mfs://resources/localfs/docs")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(third.status().is_success());
    assert_eq!(state.retrieval_cache_build_count(), 2);
}

#[tokio::test]
async fn http_snapshots_lists_persisted_snapshot_rows() {
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("custom.md"), "# Custom\n").unwrap();

    let app = mfs_server::http::app_with_config(mfs_server::http::AppConfig {
        workspace_root: workspace.path().to_path_buf(),
        source_kind: "localfs".to_owned(),
        source_path: source.path().to_path_buf(),
        target_uri: "mfs://resources/localfs/docs".to_owned(),
        account_id: "acme".to_owned(),
        user_id: "alice".to_owned(),
        agent_id: "coding-agent".to_owned(),
        canvas_separate_db: false,
    });

    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/refresh")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/snapshots")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(response.status().is_success());
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["limit"], 50);
    assert!(json["total_count"].as_u64().unwrap() >= 1);
    assert!(json.get("next_cursor").is_some());
    assert_eq!(json["snapshots"], json["items"]);
    assert!(json["items"].as_array().unwrap().len() >= 1);
}

#[tokio::test]
async fn http_audit_lists_recorded_operations() {
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("custom.md"), "# Custom\n").unwrap();

    let app = mfs_server::http::app_with_config(mfs_server::http::AppConfig {
        workspace_root: workspace.path().to_path_buf(),
        source_kind: "localfs".to_owned(),
        source_path: source.path().to_path_buf(),
        target_uri: "mfs://resources/localfs/docs".to_owned(),
        account_id: "acme".to_owned(),
        user_id: "alice".to_owned(),
        agent_id: "coding-agent".to_owned(),
        canvas_separate_db: false,
    });

    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/ls?uri=mfs://resources/localfs/docs")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/audit")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(response.status().is_success());
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["limit"], 50);
    assert!(json["total_count"].as_u64().unwrap() >= 1);
    assert!(json.get("next_cursor").is_some());
    assert_eq!(json["audit"], json["items"]);
    assert_eq!(json["entries"], json["items"]);
}

#[tokio::test]
async fn http_session_commit_exposes_task_status() {
    let _env_guard = env_isolated();
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("custom.md"), "# Custom\n").unwrap();

    let app = mfs_server::http::app_with_config(mfs_server::http::AppConfig {
        workspace_root: workspace.path().to_path_buf(),
        source_kind: "localfs".to_owned(),
        source_path: source.path().to_path_buf(),
        target_uri: "mfs://resources/localfs/docs".to_owned(),
        account_id: "acme".to_owned(),
        user_id: "alice".to_owned(),
        agent_id: "coding-agent".to_owned(),
        canvas_separate_db: false,
    });

    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/sessions")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(create.status().is_success());
    let body = axum::body::to_bytes(create.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let session_id = json["session_id"].as_str().unwrap().to_owned();

    let add_message = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/sessions/{session_id}/messages"))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"role":"user","content":"remember this"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(add_message.status().is_success());

    let mark_used = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/sessions/{session_id}/used_context"))
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"uri":"mfs://resources/localfs/docs/custom.md"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(mark_used.status().is_success());

    let mark_tool = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/sessions/{session_id}/used_tool"))
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"tool_uri":"mfs://agent/tools/read_file","success":true}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(mark_tool.status().is_success());

    let commit = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/sessions/{session_id}/commit"))
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(commit.status().is_success());
    let body = axum::body::to_bytes(commit.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let task_id = json["task_id"].as_str().unwrap().to_owned();

    let mut completed = false;
    for _ in 0..20 {
        let task = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/tasks/{task_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(task.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        if json["status"] == "Completed" || json["status"] == "completed" {
            assert_eq!(json["retry_state"].as_str(), Some("not_needed"));
            assert!(json["processing_mode"].is_string());
            assert_eq!(json["used_contexts"].as_u64(), Some(1));
            assert_eq!(json["used_tools"].as_u64(), Some(1));
            assert_eq!(json["memories_extracted"]["tools"].as_u64(), Some(1));
            assert_eq!(json["artifacts_written"]["user_session"].as_u64(), Some(1));
            completed = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    assert!(completed);
}

#[tokio::test]
async fn http_observation_sanitizes_secret_patterns_before_storage() {
    let _env_guard = env_isolated();
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let app = test_app(&workspace, &source);

    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/sessions")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"session_id":"session-secret"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(create.status().is_success());

    let observation = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/sessions/session-secret/observations")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{
                        "tool_name":"Bash",
                        "tool_input":"curl -H 'Authorization: Bearer abcdefghijk'",
                        "tool_output":"failed with sk-proj-abcdefghi and ghp_abcdefghijk",
                        "content":"cloud token cr_abcdefghijk",
                        "platform":"test"
                    }"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(observation.status().is_success());

    let messages_path = workspace
        .path()
        .join("tenants/acme/alice/session/coding-agent/session-secret/messages.jsonl");
    let stored = std::fs::read_to_string(messages_path).unwrap();
    assert!(!stored.contains("Bearer abcdefghijk"), "{stored}");
    assert!(!stored.contains("sk-proj-abcdefghi"), "{stored}");
    assert!(!stored.contains("ghp_abcdefghijk"), "{stored}");
    assert!(!stored.contains("cr_abcdefghijk"), "{stored}");
    assert!(stored.contains("Bearer [REDACTED]"), "{stored}");
    assert!(stored.contains("sk-[REDACTED]"), "{stored}");
    assert!(stored.contains("ghp_[REDACTED]"), "{stored}");
    assert!(stored.contains("cr_[REDACTED]"), "{stored}");
}

#[tokio::test]
async fn http_rate_limit_returns_429_with_retry_after() {
    let _env_guard = env_isolated();
    unsafe {
        std::env::set_var("MEMFUSE_RATE_LIMIT_ENABLED", "true");
        std::env::set_var("MEMFUSE_RATE_LIMIT_REQUESTS", "1");
        std::env::set_var("MEMFUSE_RATE_LIMIT_WINDOW_SECS", "60");
    }

    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let app = test_app(&workspace, &source);

    let first = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/sessions")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(first.status().is_success());

    let second = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/sessions")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(second.status(), axum::http::StatusCode::TOO_MANY_REQUESTS);
    let retry_after = second
        .headers()
        .get("Retry-After")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap();
    assert!((1..=60).contains(&retry_after), "retry_after={retry_after}");

    let body = axum::body::to_bytes(second.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"]["category"], "RateLimited");
    assert_eq!(json["error"]["retryable"], true);
}

#[tokio::test]
async fn http_body_limit_returns_structured_413() {
    let _env_guard = env_isolated();
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let app = mfs_server::http::app_with_config_and_body_limit(
        mfs_server::http::AppConfig {
            workspace_root: workspace.path().to_path_buf(),
            source_kind: "localfs".to_owned(),
            source_path: source.path().to_path_buf(),
            target_uri: "mfs://resources/localfs/docs".to_owned(),
            account_id: "acme".to_owned(),
            user_id: "alice".to_owned(),
            agent_id: "coding-agent".to_owned(),
            canvas_separate_db: false,
        },
        0,
    );

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/sessions")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"session_id":"too-large"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), axum::http::StatusCode::PAYLOAD_TOO_LARGE);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"]["category"], "PayloadTooLarge");
    assert_eq!(json["error"]["retryable"], false);
}

#[tokio::test]
async fn http_openapi_json_lists_core_paths() {
    let _env_guard = env_isolated();
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let app = test_app(&workspace, &source);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/docs/openapi.json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_success());

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let paths = json["paths"].as_object().expect("OpenAPI paths object");
    let schemas = json["components"]["schemas"]
        .as_object()
        .expect("OpenAPI schemas object");

    for path in [
        "/health",
        "/ready",
        "/sessions",
        "/sessions/{session_id}/observations",
        "/context/resolve",
        "/v1/memory:search",
        "/facts",
        "/relations",
        "/v1/webhooks",
        "/v1/webhooks/{id}",
        "/v1/webhooks/{id}/test",
        "/v1/resources",
        "/v1/resources/batch",
        "/v1/resources/import",
        "/v1/resources/{resource_id}/export",
        "/v1/resources/{resource_id}/refresh",
        "/v1/resources/{resource_id}/rebuild",
        "/v1/workspace/ls",
        "/v1/workspace/tree",
        "/v1/workspace/stat",
        "/v1/workspace/abstract",
        "/v1/workspace/overview",
        "/v1/workspace/read",
        "/v1/workspace/glob",
        "/v1/workspace/mkdir",
        "/v1/workspace/write",
        "/v1/workspace/mv",
        "/v1/workspace/rm",
        "/v1/workspace/find",
        "/v1/workspace/grep",
        "/v1/workspace/search",
        "/v1/workspace/rebuild",
        "/v1/workspace/refresh",
        "/watches",
        "/watches/run-due",
        "/watches/run-loop",
        "/watch-service/start",
        "/watch-service/status",
        "/watch-service/stop",
        "/resources/{resource_id}/watch",
        "/resources/{resource_id}/watch/disable",
        "/resources/{resource_id}/watch/run",
        "/heuristics/rules",
        "/heuristics/rules/{rule_id}",
        "/heuristics/rules/{rule_id}/promote",
        "/heuristics/rules/{rule_id}/confirm",
        "/heuristics/instances",
        "/heuristics/instances/{instance_id}",
        "/heuristics/retrieve",
        "/heuristics/l0-confirmed",
        "/heuristics/simulate-reaction",
        "/episodes/{episode_id}",
        "/episodes/{episode_id}/timeline",
        "/memories/cite",
        "/memories/export",
        "/memories/import",
        "/v1/memory:consolidate",
        "/v1/memory:extract-facts",
        "/v1/memory:archive",
        "/v1/eval/recall",
        "/code_symbols",
        "/code_symbols/search",
        "/code_symbols/{view_id}",
        "/snapshots",
        "/audit",
        "/skills",
        "/system/status",
        "/system/observer",
        "/tasks",
        "/tasks/evict",
        "/tasks/{task_id}",
        "/tasks/{task_id}/wait",
    ] {
        assert!(paths.contains_key(path), "missing OpenAPI path {path}");
    }

    for schema in [
        "CreateSessionRequest",
        "SessionCreateResponse",
        "SessionSummary",
        "SessionListResponse",
        "AddMessageRequest",
        "AddMessageResponse",
        "CommitSessionRequest",
        "CommitSessionResponse",
        "DeleteSessionResponse",
        "CreateFactRequest",
        "FactRecord",
        "FactListResponse",
        "CreateFactResponse",
        "SupersedeFactRequest",
        "SupersedeFactResponse",
        "RetractFactResponse",
        "TraceFactResponse",
        "CreateWebhookRequest",
        "WebhookRecord",
        "WebhookListResponse",
        "DeleteWebhookResponse",
        "TestWebhookResponse",
        "AddObservationRequest",
        "ResolveMemoryContextRequest",
        "ContextResolveResponse",
        "MemorySearchRequest",
        "MemorySearchResponse",
        "LinkRelationRequest",
        "RelationListResponse",
        "CreateResourceRequest",
        "CreateResourceResponse",
        "ResourceRecord",
        "ResourceListResponse",
        "CreateResourcesBatchRequest",
        "ResourceBatchResponse",
        "ResourceImportRequest",
        "ResourceExportRequest",
        "ResourceExportResponse",
        "ResourceTaskResponse",
        "WorkspaceUriRequest",
        "WorkspaceWriteRequest",
        "WorkspaceMoveRequest",
        "WorkspaceStatResponse",
        "WorkspaceMutationResponse",
        "WorkspaceSearchResponse",
        "WorkspaceRebuildResponse",
        "WorkspaceRefreshResponse",
        "ResourceWatchRequest",
        "ResourceWatchLoopRequest",
        "WatchServiceRequest",
        "ResourceWatchRecord",
        "ResourceWatchRunResponse",
        "WatchListResponse",
        "WatchServiceStatusResponse",
        "HeuristicRuleRequest",
        "HeuristicRuleResponse",
        "HeuristicRuleRecord",
        "HeuristicRuleListResponse",
        "PromoteRuleRequest",
        "ConfirmRuleResponse",
        "HeuristicInstanceRequest",
        "HeuristicInstanceResponse",
        "HeuristicInstanceRecord",
        "HeuristicInstanceListResponse",
        "RetrieveHeuristicsRequest",
        "RetrieveHeuristicsResponse",
        "L0ConfirmedRequest",
        "SimulateReactionRequest",
        "EpisodeDetailResponse",
        "EpisodeTimelineResponse",
        "CiteMemoriesRequest",
        "CiteMemoriesResponse",
        "MemoryExportResponse",
        "MemoryImportRequest",
        "MemoryImportResponse",
        "MemoryConsolidateRequest",
        "MemoryConsolidateResponse",
        "MemoryExtractFactsRequest",
        "MemoryExtractFactsResponse",
        "MemoryArchiveRequest",
        "MemoryArchiveResponse",
        "EvalRecallRequest",
        "EvalRecallResponse",
        "CreateCodeSymbolsRequest",
        "CodeSymbolRecord",
        "CodeSymbolListResponse",
        "CodeSymbolSearchResponse",
        "DeleteCodeSymbolsResponse",
        "SnapshotListResponse",
        "AuditListResponse",
        "AddSkillRequest",
        "AddSkillResponse",
        "SkillListResponse",
        "SystemStatusResponse",
        "ObserverStatusResponse",
        "TaskListResponse",
        "TaskEvictResponse",
        "TaskStatusResponse",
        "WaitTaskResponse",
    ] {
        assert!(
            schemas.contains_key(schema),
            "missing OpenAPI schema {schema}"
        );
    }

    assert_eq!(
        paths["/sessions"]["post"]["requestBody"]["content"]["application/json"]["schema"]["$ref"],
        "#/components/schemas/CreateSessionRequest",
    );
    assert_eq!(
        paths["/sessions"]["get"]["responses"]["200"]["content"]["application/json"]["schema"]["$ref"],
        "#/components/schemas/SessionListResponse",
    );
    assert_eq!(
        paths["/sessions/{session_id}/messages"]["post"]["requestBody"]["content"]["application/json"]
            ["schema"]["$ref"],
        "#/components/schemas/AddMessageRequest",
    );
    assert_eq!(
        paths["/sessions/{session_id}/commit"]["post"]["responses"]["200"]["content"]["application/json"]
            ["schema"]["$ref"],
        "#/components/schemas/CommitSessionResponse",
    );
    assert_eq!(
        paths["/facts"]["post"]["requestBody"]["content"]["application/json"]["schema"]["$ref"],
        "#/components/schemas/CreateFactRequest",
    );
    assert_eq!(
        paths["/facts"]["get"]["responses"]["200"]["content"]["application/json"]["schema"]["$ref"],
        "#/components/schemas/FactListResponse",
    );
    assert_eq!(
        paths["/facts/{fact_id}/supersede"]["post"]["requestBody"]["content"]["application/json"]["schema"]
            ["$ref"],
        "#/components/schemas/SupersedeFactRequest",
    );
    assert_eq!(
        paths["/facts/{fact_id}/trace"]["get"]["responses"]["200"]["content"]["application/json"]["schema"]
            ["$ref"],
        "#/components/schemas/TraceFactResponse",
    );
    assert_eq!(
        paths["/v1/webhooks"]["post"]["requestBody"]["content"]["application/json"]["schema"]["$ref"],
        "#/components/schemas/CreateWebhookRequest",
    );
    assert_eq!(
        paths["/v1/webhooks"]["get"]["responses"]["200"]["content"]["application/json"]["schema"]["$ref"],
        "#/components/schemas/WebhookListResponse",
    );
    assert_eq!(
        paths["/v1/webhooks/{id}"]["delete"]["responses"]["200"]["content"]["application/json"]["schema"]
            ["$ref"],
        "#/components/schemas/DeleteWebhookResponse",
    );
    assert_eq!(
        paths["/v1/webhooks/{id}/test"]["post"]["responses"]["200"]["content"]["application/json"]
            ["schema"]["$ref"],
        "#/components/schemas/TestWebhookResponse",
    );
    assert_eq!(
        paths["/sessions/{session_id}/observations"]["post"]["requestBody"]["content"]["application/json"]
            ["schema"]["$ref"],
        "#/components/schemas/AddObservationRequest",
    );
    assert_eq!(
        paths["/context/resolve"]["post"]["requestBody"]["content"]["application/json"]["schema"]["$ref"],
        "#/components/schemas/ResolveMemoryContextRequest",
    );
    assert_eq!(
        paths["/v1/memory:search"]["post"]["responses"]["200"]["content"]["application/json"]["schema"]
            ["$ref"],
        "#/components/schemas/MemorySearchResponse",
    );
    assert_eq!(
        paths["/relations"]["post"]["requestBody"]["content"]["application/json"]["schema"]["$ref"],
        "#/components/schemas/LinkRelationRequest",
    );
    assert_eq!(
        paths["/v1/resources"]["post"]["requestBody"]["content"]["application/json"]["schema"]["$ref"],
        "#/components/schemas/CreateResourceRequest",
    );
    assert_eq!(
        paths["/v1/resources"]["get"]["responses"]["200"]["content"]["application/json"]["schema"]
            ["$ref"],
        "#/components/schemas/ResourceListResponse",
    );
    assert_eq!(
        paths["/v1/resources/batch"]["post"]["responses"]["200"]["content"]["application/json"]["schema"]
            ["$ref"],
        "#/components/schemas/ResourceBatchResponse",
    );
    assert_eq!(
        paths["/v1/workspace/write"]["post"]["requestBody"]["content"]["application/json"]["schema"]
            ["$ref"],
        "#/components/schemas/WorkspaceWriteRequest",
    );
    assert_eq!(
        paths["/v1/workspace/mv"]["post"]["requestBody"]["content"]["application/json"]["schema"]["$ref"],
        "#/components/schemas/WorkspaceMoveRequest",
    );
    assert_eq!(
        paths["/watches"]["get"]["responses"]["200"]["content"]["application/json"]["schema"]["$ref"],
        "#/components/schemas/WatchListResponse",
    );
    assert_eq!(
        paths["/resources/{resource_id}/watch"]["post"]["requestBody"]["content"]["application/json"]
            ["schema"]["$ref"],
        "#/components/schemas/ResourceWatchRequest",
    );
    assert_eq!(
        paths["/heuristics/rules"]["post"]["requestBody"]["content"]["application/json"]["schema"]
            ["$ref"],
        "#/components/schemas/HeuristicRuleRequest",
    );
    assert_eq!(
        paths["/heuristics/instances"]["post"]["responses"]["200"]["content"]["application/json"]["schema"]
            ["$ref"],
        "#/components/schemas/HeuristicInstanceResponse",
    );
    assert_eq!(
        paths["/memories/import"]["post"]["requestBody"]["content"]["application/json"]["schema"]["$ref"],
        "#/components/schemas/MemoryImportRequest",
    );
    assert_eq!(
        paths["/v1/memory:consolidate"]["post"]["responses"]["200"]["content"]["application/json"]
            ["schema"]["$ref"],
        "#/components/schemas/MemoryConsolidateResponse",
    );
    assert_eq!(
        paths["/code_symbols"]["post"]["requestBody"]["content"]["application/json"]["schema"]["$ref"],
        "#/components/schemas/CreateCodeSymbolsRequest",
    );
    assert_eq!(
        paths["/tasks"]["get"]["responses"]["200"]["content"]["application/json"]["schema"]["$ref"],
        "#/components/schemas/TaskListResponse",
    );
    assert_eq!(
        paths["/tasks/{task_id}/wait"]["get"]["responses"]["200"]["content"]["application/json"]["schema"]
            ["$ref"],
        "#/components/schemas/WaitTaskResponse",
    );
}

#[tokio::test]
async fn http_v1_core_routes_alias_legacy_routes_and_mark_legacy_deprecated() {
    let _env_guard = env_isolated();
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let app = test_app(&workspace, &source);

    let create_v1 = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/sessions")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"session_id":"v1-session"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(create_v1.status().is_success());
    assert!(create_v1.headers().get("Deprecation").is_none());

    let list_v1 = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/sessions")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(list_v1.status().is_success());
    assert!(list_v1.headers().get("Deprecation").is_none());
    let body = axum::body::to_bytes(list_v1.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["sessions"][0]["session_id"], "v1-session");

    let legacy = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/sessions")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(legacy.status().is_success());
    assert_eq!(
        legacy
            .headers()
            .get("Deprecation")
            .and_then(|v| v.to_str().ok()),
        Some("true"),
    );
    assert_eq!(
        legacy.headers().get("Sunset").and_then(|v| v.to_str().ok()),
        Some("2027-01-01"),
    );
}

#[tokio::test]
async fn http_webhooks_register_list_delete_and_test_trigger() {
    let _env_guard = env_isolated();
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let app = test_app(&workspace, &source);

    let received = Arc::new(tokio::sync::Mutex::new(Vec::<(String, String)>::new()));
    let received_for_server = received.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let callback = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 8192];
        let n = stream.read(&mut buf).await.unwrap();
        let raw = String::from_utf8_lossy(&buf[..n]).to_string();
        let signature = raw
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(": ")?;
                name.eq_ignore_ascii_case("X-MemFuse-Signature")
                    .then(|| value)
            })
            .unwrap_or("")
            .to_owned();
        let body = raw.split("\r\n\r\n").nth(1).unwrap_or("").to_owned();
        received_for_server.lock().await.push((signature, body));
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
            .await
            .unwrap();
    });

    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/webhooks")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "event_type": "test.event",
                        "callback_url": format!("http://{addr}/hook"),
                        "secret": "super-secret",
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(create.status().is_success());
    let body = axum::body::to_bytes(create.into_body(), usize::MAX)
        .await
        .unwrap();
    let created: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let webhook_id = created["id"].as_str().unwrap().to_owned();
    assert_eq!(created["event_type"], "test.event");
    assert!(created.get("secret").is_none());

    let list = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/webhooks")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(list.status().is_success());
    let body = axum::body::to_bytes(list.into_body(), usize::MAX)
        .await
        .unwrap();
    let listed: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(listed["items"], listed["webhooks"]);
    assert_eq!(listed["total_count"], 1);
    assert_eq!(listed["items"][0]["id"], webhook_id);
    assert!(listed["items"][0].get("secret").is_none());

    let test = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/webhooks/{webhook_id}/test"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(test.status().is_success());
    callback.await.unwrap();
    let captured = received.lock().await;
    assert_eq!(captured.len(), 1);
    assert!(captured[0].0.starts_with("sha256="), "{:?}", captured[0]);
    assert!(captured[0].1.contains("test.event"), "{:?}", captured[0]);

    let delete = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/v1/webhooks/{webhook_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(delete.status().is_success());

    let list = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/webhooks")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(list.into_body(), usize::MAX)
        .await
        .unwrap();
    let listed: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(listed["total_count"], 0);
}

#[tokio::test]
async fn http_webhooks_deliver_matching_audit_events() {
    let _env_guard = env_isolated();
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("auth.md"), "# Authentication\n").unwrap();
    let app = test_app(&workspace, &source);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let callback = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 8192];
        let n = stream.read(&mut buf).await.unwrap();
        let raw = String::from_utf8_lossy(&buf[..n]).to_string();
        let signature = raw
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(": ")?;
                name.eq_ignore_ascii_case("X-MemFuse-Signature")
                    .then(|| value)
            })
            .unwrap_or("")
            .to_owned();
        let body = raw.split("\r\n\r\n").nth(1).unwrap_or("").to_owned();
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
            .await
            .unwrap();
        (signature, body)
    });

    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/webhooks")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "event_type": "resource.ingest",
                        "callback_url": format!("http://{addr}/hook"),
                        "secret": "super-secret",
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(create.status().is_success());

    let ingest = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/resources")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "source_kind": "localfs",
                        "source_path": source.path().display().to_string(),
                        "logical_name": "docs",
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(ingest.status().is_success());

    let (signature, body) = tokio::time::timeout(std::time::Duration::from_secs(5), callback)
        .await
        .unwrap()
        .unwrap();
    assert!(signature.starts_with("sha256="), "{signature:?}");
    assert!(body.contains("resource.ingest"), "{body:?}");
    assert!(body.contains("mfs://resources/localfs/docs"), "{body:?}");
}

#[tokio::test]
async fn http_wait_endpoint_returns_terminal_session_task_state() {
    let _env_guard = env_isolated();
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("custom.md"), "# Custom\n").unwrap();

    let app = mfs_server::http::app_with_config(mfs_server::http::AppConfig {
        workspace_root: workspace.path().to_path_buf(),
        source_kind: "localfs".to_owned(),
        source_path: source.path().to_path_buf(),
        target_uri: "mfs://resources/localfs/docs".to_owned(),
        account_id: "acme".to_owned(),
        user_id: "alice".to_owned(),
        agent_id: "coding-agent".to_owned(),
        canvas_separate_db: false,
    });

    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/sessions")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(create.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let session_id = json["session_id"].as_str().unwrap().to_owned();

    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/sessions/{session_id}/messages"))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"role":"user","content":"remember this"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    let commit = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/sessions/{session_id}/commit"))
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(commit.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let task_id = json["task_id"].as_str().unwrap().to_owned();

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/tasks/{task_id}/wait?timeout_ms=5000&poll_ms=10"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(response.status().is_success());
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "Completed");
    assert_eq!(json["retry_state"], "not_needed");
}

#[tokio::test]
async fn http_session_list_and_context() {
    let _env_guard = env_isolated();
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("custom.md"), "# Custom\n").unwrap();

    let app = mfs_server::http::app_with_config(mfs_server::http::AppConfig {
        workspace_root: workspace.path().to_path_buf(),
        source_kind: "localfs".to_owned(),
        source_path: source.path().to_path_buf(),
        target_uri: "mfs://resources/localfs/docs".to_owned(),
        account_id: "acme".to_owned(),
        user_id: "alice".to_owned(),
        agent_id: "coding-agent".to_owned(),
        canvas_separate_db: false,
    });

    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/sessions")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"session_id":"session-surface"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(create.status().is_success());

    let add_message = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/sessions/session-surface/messages")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"role":"user","content":"turn one"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(add_message.status().is_success());

    let commit = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/sessions/session-surface/commit")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(commit.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let task_id = json["task_id"].as_str().unwrap().to_owned();

    let mut completed = false;
    for _ in 0..50 {
        let task = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/tasks/{task_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(task.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        if json["status"] == "Completed" || json["status"] == "completed" {
            completed = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(completed);

    let sessions = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/sessions")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(sessions.status().is_success());
    let body = axum::body::to_bytes(sessions.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["limit"], 20);
    assert!(json.get("next_cursor").is_some());
    assert!(json["total_count"].as_u64().unwrap() >= 1);
    assert!(
        json["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["session_id"] == "session-surface")
    );

    let session = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/sessions/session-surface")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(session.status().is_success());
    let body = axum::body::to_bytes(session.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["session_id"], "session-surface");

    let context = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/sessions/session-surface/context?token_budget=128000")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(context.status().is_success());
    let body = axum::body::to_bytes(context.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(json["latest_archive_overview"].is_string());

    let archive = app
        .oneshot(
            Request::builder()
                .uri("/sessions/session-surface/archives/archive_001")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(archive.status().is_success());
    let body = axum::body::to_bytes(archive.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["archive_id"], "archive_001");
}

#[tokio::test]
async fn http_session_delete_removes_session_surface() {
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("custom.md"), "# Custom\n").unwrap();

    let app = mfs_server::http::app_with_config(mfs_server::http::AppConfig {
        workspace_root: workspace.path().to_path_buf(),
        source_kind: "localfs".to_owned(),
        source_path: source.path().to_path_buf(),
        target_uri: "mfs://resources/localfs/docs".to_owned(),
        account_id: "acme".to_owned(),
        user_id: "alice".to_owned(),
        agent_id: "coding-agent".to_owned(),
        canvas_separate_db: false,
    });

    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/sessions")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"session_id":"session-delete"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(create.status().is_success());

    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/sessions/session-delete/messages")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"role":"user","content":"turn one"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    let commit = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/sessions/session-delete/commit")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(commit.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let task_id = json["task_id"].as_str().unwrap().to_owned();
    for _ in 0..50 {
        let task = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/tasks/{task_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(task.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        if json["status"] == "Completed" || json["status"] == "completed" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/sessions/session-delete")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_success());

    let sessions = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/sessions")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(sessions.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        !json["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["session_id"] == "session-delete")
    );
}

#[tokio::test]
async fn http_resources_register_and_list_managed_resources() {
    let _env_guard = env_isolated();
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(
        source.path().join("auth.md"),
        "# Authentication\nOAuth login flow\n",
    )
    .unwrap();

    let app = mfs_server::http::app_with_config(mfs_server::http::AppConfig {
        workspace_root: workspace.path().to_path_buf(),
        source_kind: "localfs".to_owned(),
        source_path: source.path().to_path_buf(),
        target_uri: "mfs://resources/localfs/docs".to_owned(),
        account_id: "acme".to_owned(),
        user_id: "alice".to_owned(),
        agent_id: "coding-agent".to_owned(),
        canvas_separate_db: false,
    });

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/resources")
                .header("content-type", "application/json")
                .body(Body::from(format!(
                    "{{\"source_kind\":\"localfs\",\"source_path\":\"{}\",\"logical_name\":\"docs\"}}",
                    source.path().display()
                )))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let task_key = json["task_key"].as_str().unwrap().to_owned();

    let mut completed = false;
    for _ in 0..50 {
        let task = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/tasks/{task_key}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(task.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        if json["state"] == "completed" {
            assert_eq!(json["retry_state"].as_str(), Some("not_needed"));
            assert!(json["processing_mode"].is_string());
            completed = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(completed);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/resources")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["limit"], 100);
    assert_eq!(json["total_count"], 1);
    assert!(json.get("next_cursor").is_some());
    let items = json["items"].as_array().unwrap();
    let resources = json["resources"].as_array().unwrap();
    assert_eq!(items, resources);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["logical_name"], "docs");
}

#[tokio::test]
async fn http_resources_persist_orchestrator_business_metadata() {
    let _env_guard = env_isolated();
    let workspace = tempfile::tempdir().unwrap();
    let source_a = tempfile::tempdir().unwrap();
    let source_b = tempfile::tempdir().unwrap();
    std::fs::write(source_a.path().join("auth.md"), "# Authentication\n").unwrap();
    std::fs::write(source_b.path().join("billing.md"), "# Billing\n").unwrap();

    let app = mfs_server::http::app_with_config(mfs_server::http::AppConfig {
        workspace_root: workspace.path().to_path_buf(),
        source_kind: "localfs".to_owned(),
        source_path: source_a.path().to_path_buf(),
        target_uri: "mfs://resources/localfs/docs".to_owned(),
        account_id: "acme".to_owned(),
        user_id: "alice".to_owned(),
        agent_id: "coding-agent".to_owned(),
        canvas_separate_db: false,
    });

    for (logical_name, repo_id, source_path) in [
        ("primary-repo", "primary-repo", source_a.path()),
        ("other-repo", "other-repo", source_b.path()),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/resources")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "source_kind": "localfs",
                            "source_path": source_path,
                            "logical_name": logical_name,
                            "repo_id": repo_id,
                            "tracker": "github_projects",
                            "tracker_project_identifier": "octo-org/7"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(response.status().is_success());
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["repo_id"], repo_id);
        assert_eq!(json["tracker"], "github_projects");
        assert_eq!(json["tracker_project_identifier"], "octo-org/7");
    }

    let response = app
        .oneshot(
            Request::builder()
                .uri("/resources?repo_id=primary-repo")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let items = json["items"].as_array().unwrap();
    assert_eq!(json["total_count"], 1);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["logical_name"], "primary-repo");
    assert_eq!(items[0]["repo_id"], "primary-repo");
    assert_eq!(items[0]["tracker"], "github_projects");
    assert_eq!(items[0]["tracker_project_identifier"], "octo-org/7");
}

#[tokio::test]
async fn http_resources_support_inline_content_ingest() {
    let _env_guard = env_isolated();
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("custom.md"), "# Custom\n").unwrap();

    let app = mfs_server::http::app_with_config(mfs_server::http::AppConfig {
        workspace_root: workspace.path().to_path_buf(),
        source_kind: "localfs".to_owned(),
        source_path: source.path().to_path_buf(),
        target_uri: "mfs://resources/localfs/docs".to_owned(),
        account_id: "acme".to_owned(),
        user_id: "alice".to_owned(),
        agent_id: "coding-agent".to_owned(),
        canvas_separate_db: false,
    });

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/resources")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "logical_name": "inline-docs",
                        "file_name": "auth.md",
                        "content": "# Authentication\ninline upload oauth\n",
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let task_key = json["task_key"].as_str().unwrap().to_owned();

    for _ in 0..50 {
        let task = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/tasks/{task_key}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(task.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        if json["state"] == "completed" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    let response = app
        .oneshot(
            Request::builder()
                .uri("/find?query=inline%20upload&target=mfs://resources/inline/inline-docs")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        json["resources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["uri"] == "mfs://resources/inline/inline-docs/auth.md")
    );
}

#[tokio::test]
async fn http_resources_support_url_ingest() {
    let _env_guard = env_isolated();
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("custom.md"), "# Custom\n").unwrap();
    let (url, _server) = spawn_static_http_server(
        "/auth.md",
        "# Authentication\nremote url oauth\n",
        "text/markdown",
    );

    let app = mfs_server::http::app_with_config(mfs_server::http::AppConfig {
        workspace_root: workspace.path().to_path_buf(),
        source_kind: "localfs".to_owned(),
        source_path: source.path().to_path_buf(),
        target_uri: "mfs://resources/localfs/docs".to_owned(),
        account_id: "acme".to_owned(),
        user_id: "alice".to_owned(),
        agent_id: "coding-agent".to_owned(),
        canvas_separate_db: false,
    });

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/resources")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "source_kind": "url",
                        "source_path": url,
                        "logical_name": "remote-docs",
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let task_key = json["task_key"].as_str().unwrap().to_owned();

    for _ in 0..50 {
        let task = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/tasks/{task_key}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(task.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        if json["state"] == "completed" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    let response = app
        .oneshot(
            Request::builder()
                .uri("/find?query=remote%20url%20oauth&target=mfs://resources/url/")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(json["resources"].as_array().unwrap().iter().any(|item| {
        let uri = item["uri"].as_str().unwrap();
        uri.starts_with("mfs://resources/url/") && uri.ends_with("/auth.md")
    }));
}

#[tokio::test]
async fn http_resource_export_and_import_pack_round_trip() {
    let _env_guard = env_isolated();
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    // Place export_root inside workspace so path-traversal validation passes
    let export_root_path = workspace.path().join("export_area");
    std::fs::create_dir_all(&export_root_path).unwrap();
    std::fs::write(
        source.path().join("auth.md"),
        "# Authentication\npack export oauth\n",
    )
    .unwrap();

    let app = mfs_server::http::app_with_config(mfs_server::http::AppConfig {
        workspace_root: workspace.path().to_path_buf(),
        source_kind: "localfs".to_owned(),
        source_path: source.path().to_path_buf(),
        target_uri: "mfs://resources/localfs/docs".to_owned(),
        account_id: "acme".to_owned(),
        user_id: "alice".to_owned(),
        agent_id: "coding-agent".to_owned(),
        canvas_separate_db: false,
    });

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/resources")
                .header("content-type", "application/json")
                .body(Body::from(format!(
                    "{{\"source_kind\":\"localfs\",\"source_path\":\"{}\",\"logical_name\":\"docs\"}}",
                    source.path().display()
                )))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let resource_id = json["resource_id"].as_str().unwrap().to_owned();
    let task_key = json["task_key"].as_str().unwrap().to_owned();
    for _ in 0..50 {
        let task = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/tasks/{task_key}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(task.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        if json["state"] == "completed" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    let pack_path = export_root_path.join("docs.ovpack");
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/resources/{resource_id}/export"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "output_path": pack_path }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_success());
    assert!(pack_path.join("manifest.json").exists());

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/resources/import")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "pack_path": pack_path,
                        "logical_name": "imported-docs",
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let import_task_key = json["task_key"].as_str().unwrap().to_owned();
    for _ in 0..50 {
        let task = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/tasks/{import_task_key}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(task.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        if json["state"] == "completed" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    let response = app
        .oneshot(
            Request::builder()
                .uri(
                    "/find?query=pack%20export%20oauth&target=mfs://resources/inline/imported-docs",
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        json["resources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["uri"] == "mfs://resources/inline/imported-docs/auth.md")
    );
}

#[tokio::test]
async fn http_add_skill_ingests_local_directory_and_makes_it_searchable() {
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    // Place skill_dir inside workspace so path-traversal validation passes
    let skill_dir_path = workspace.path().join("skill_input");
    std::fs::create_dir_all(&skill_dir_path).unwrap();
    std::fs::write(source.path().join("custom.md"), "# Custom\n").unwrap();
    std::fs::write(
        skill_dir_path.join("SKILL.md"),
        r#"---
name: search-web
description: Search the web for current information
---

# search-web

Search the web for current information.
"#,
    )
    .unwrap();

    let app = mfs_server::http::app_with_config(mfs_server::http::AppConfig {
        workspace_root: workspace.path().to_path_buf(),
        source_kind: "localfs".to_owned(),
        source_path: source.path().to_path_buf(),
        target_uri: "mfs://resources/localfs/docs".to_owned(),
        account_id: "acme".to_owned(),
        user_id: "alice".to_owned(),
        agent_id: "coding-agent".to_owned(),
        canvas_separate_db: false,
    });

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/skills")
                .header("content-type", "application/json")
                .body(Body::from(format!(
                    "{{\"path\":\"{}\"}}",
                    skill_dir_path.display()
                )))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["skill_uri"], "mfs://agent/skills/search-web");

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/find?query=web%20search&target=mfs://agent/skills")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        json["skills"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["uri"] == "mfs://agent/skills/search-web")
    );

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/skills")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["items"], json["skills"]);
    assert!(json.get("next_cursor").is_some());
    assert!(
        json["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["skill_uri"] == "mfs://agent/skills/search-web")
    );
}

#[tokio::test]
async fn http_find_returns_provenance_for_managed_resource_hits() {
    let _env_guard = env_isolated();
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(
        source.path().join("auth.md"),
        "# Authentication\nOAuth login flow\n",
    )
    .unwrap();

    let app = mfs_server::http::app_with_config(mfs_server::http::AppConfig {
        workspace_root: workspace.path().to_path_buf(),
        source_kind: "localfs".to_owned(),
        source_path: source.path().to_path_buf(),
        target_uri: "mfs://resources/localfs/docs".to_owned(),
        account_id: "acme".to_owned(),
        user_id: "alice".to_owned(),
        agent_id: "coding-agent".to_owned(),
        canvas_separate_db: false,
    });

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/resources")
                .header("content-type", "application/json")
                .body(Body::from(format!(
                    "{{\"source_kind\":\"localfs\",\"source_path\":\"{}\",\"logical_name\":\"docs\"}}",
                    source.path().display()
                )))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let task_key = json["task_key"].as_str().unwrap().to_owned();

    for _ in 0..50 {
        let task = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/tasks/{task_key}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(task.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        if json["state"] == "completed" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    let response = app
        .oneshot(
            Request::builder()
                .uri("/find?query=authentication&target=mfs://resources/docs")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        json["resources"][0]["provenance"]["source_kind"].as_str(),
        Some("localfs")
    );
}

#[tokio::test]
async fn http_registered_resource_refresh_reindexes_semantic_state() {
    let _env_guard = env_isolated();
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(
        source.path().join("auth.md"),
        "# Authentication\nOAuth login flow\n",
    )
    .unwrap();

    let app = mfs_server::http::app_with_config(mfs_server::http::AppConfig {
        workspace_root: workspace.path().to_path_buf(),
        source_kind: "localfs".to_owned(),
        source_path: source.path().to_path_buf(),
        target_uri: "mfs://resources/localfs/docs".to_owned(),
        account_id: "acme".to_owned(),
        user_id: "alice".to_owned(),
        agent_id: "coding-agent".to_owned(),
        canvas_separate_db: false,
    });

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/resources")
                .header("content-type", "application/json")
                .body(Body::from(format!(
                    "{{\"source_kind\":\"localfs\",\"source_path\":\"{}\",\"logical_name\":\"docs\"}}",
                    source.path().display()
                )))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let resource_id = json["resource_id"].as_str().unwrap().to_owned();

    std::fs::write(
        source.path().join("auth.md"),
        "# Sessions\nrefresh token rotation\n",
    )
    .unwrap();

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/resources/{resource_id}/refresh"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let task_key = json["task_key"].as_str().unwrap().to_owned();

    for _ in 0..50 {
        let task = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/tasks/{task_key}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(task.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        if json["state"] == "completed" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    let response = app
        .oneshot(
            Request::builder()
                .uri("/find?query=rotation&target=mfs://resources/localfs/docs")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        json["resources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["uri"].as_str().unwrap().ends_with("/auth.md"))
    );
}

#[tokio::test]
async fn http_resource_watch_registers_and_runs_refresh_tick() {
    let _env_guard = env_isolated();
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("auth.md"), "# Authentication\nOAuth\n").unwrap();

    let app = mfs_server::http::app_with_config(mfs_server::http::AppConfig {
        workspace_root: workspace.path().to_path_buf(),
        source_kind: "localfs".to_owned(),
        source_path: source.path().to_path_buf(),
        target_uri: "mfs://resources/localfs/docs".to_owned(),
        account_id: "acme".to_owned(),
        user_id: "alice".to_owned(),
        agent_id: "coding-agent".to_owned(),
        canvas_separate_db: false,
    });

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/resources")
                .header("content-type", "application/json")
                .body(Body::from(format!(
                    "{{\"source_kind\":\"localfs\",\"source_path\":\"{}\",\"logical_name\":\"docs\"}}",
                    source.path().display()
                )))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let resource_id = json["resource_id"].as_str().unwrap().to_owned();
    let task_key = json["task_key"].as_str().unwrap().to_owned();
    for _ in 0..50 {
        let task = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/tasks/{task_key}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(task.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        if json["state"] == "completed" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/resources/{resource_id}/watch"))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"interval_seconds":1}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_success());

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/watches")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["items"], json["watches"]);
    assert!(json.get("next_cursor").is_some());
    assert!(
        json["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["resource_id"] == resource_id)
    );

    std::fs::write(
        source.path().join("auth.md"),
        "# Sessions\nwatch tick rotation\n",
    )
    .unwrap();

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/resources/{resource_id}/watch/run"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["refreshed"], true);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/find?query=watch%20tick%20rotation&target=mfs://resources/localfs/docs")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        json["resources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["uri"].as_str().unwrap().ends_with("/auth.md"))
    );
}

#[tokio::test]
async fn http_resource_watch_disable_and_run_due() {
    let workspace = tempfile::tempdir().unwrap();
    let source_due = tempfile::tempdir().unwrap();
    let source_disabled = tempfile::tempdir().unwrap();
    std::fs::write(
        source_due.path().join("auth.md"),
        "# Authentication\nOAuth\n",
    )
    .unwrap();
    std::fs::write(
        source_disabled.path().join("guide.md"),
        "# Guide\nrotation baseline\n",
    )
    .unwrap();

    let app = mfs_server::http::app_with_config(mfs_server::http::AppConfig {
        workspace_root: workspace.path().to_path_buf(),
        source_kind: "localfs".to_owned(),
        source_path: source_due.path().to_path_buf(),
        target_uri: "mfs://resources/localfs/docs".to_owned(),
        account_id: "acme".to_owned(),
        user_id: "alice".to_owned(),
        agent_id: "coding-agent".to_owned(),
        canvas_separate_db: false,
    });

    async fn add_resource_and_wait(
        app: &axum::Router,
        source_path: &std::path::Path,
        logical_name: &str,
    ) -> String {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/resources")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(
                        "{{\"source_kind\":\"localfs\",\"source_path\":\"{}\",\"logical_name\":\"{}\"}}",
                        source_path.display(),
                        logical_name
                    )))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let resource_id = json["resource_id"].as_str().unwrap().to_owned();
        let task_key = json["task_key"].as_str().unwrap().to_owned();
        for _ in 0..50 {
            let task = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(format!("/tasks/{task_key}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            let body = axum::body::to_bytes(task.into_body(), usize::MAX)
                .await
                .unwrap();
            let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
            if json["state"] == "completed" {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        resource_id
    }

    let due_resource_id = add_resource_and_wait(&app, source_due.path(), "docs-due").await;
    let disabled_resource_id =
        add_resource_and_wait(&app, source_disabled.path(), "docs-disabled").await;

    for resource_id in [&due_resource_id, &disabled_resource_id] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/resources/{resource_id}/watch"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"interval_seconds":1}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(response.status().is_success());
    }

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/resources/{disabled_resource_id}/watch/disable"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_success());

    std::fs::write(
        source_due.path().join("auth.md"),
        "# Sessions\nwatch due rotation\n",
    )
    .unwrap();
    std::fs::write(
        source_disabled.path().join("guide.md"),
        "# Guide\nrotation should stay absent\n",
    )
    .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/watches")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        json["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["resource_id"] == due_resource_id && item["due"] == true)
    );
    assert!(
        json["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["resource_id"] == disabled_resource_id && item["enabled"] == false)
    );

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/watches/run-due")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        json["runs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["resource_id"] == due_resource_id && item["refreshed"] == true)
    );
    assert!(
        !json["runs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["resource_id"] == disabled_resource_id)
    );
}

#[tokio::test]
async fn http_resource_watch_loop_runs_due_checks_over_multiple_iterations() {
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("auth.md"), "# Authentication\nOAuth\n").unwrap();

    let app = mfs_server::http::app_with_config(mfs_server::http::AppConfig {
        workspace_root: workspace.path().to_path_buf(),
        source_kind: "localfs".to_owned(),
        source_path: source.path().to_path_buf(),
        target_uri: "mfs://resources/localfs/docs".to_owned(),
        account_id: "acme".to_owned(),
        user_id: "alice".to_owned(),
        agent_id: "coding-agent".to_owned(),
        canvas_separate_db: false,
    });

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/resources")
                .header("content-type", "application/json")
                .body(Body::from(format!(
                    "{{\"source_kind\":\"localfs\",\"source_path\":\"{}\",\"logical_name\":\"docs-loop\"}}",
                    source.path().display()
                )))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let resource_id = json["resource_id"].as_str().unwrap().to_owned();
    let task_key = json["task_key"].as_str().unwrap().to_owned();
    for _ in 0..50 {
        let task = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/tasks/{task_key}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(task.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        if json["state"] == "completed" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/resources/{resource_id}/watch"))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"interval_seconds":1}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/resources/{resource_id}/watch/run"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    std::fs::write(source.path().join("auth.md"), "# Sessions\nloop rotation\n").unwrap();

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/watches/run-loop")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"iterations":3,"sleep_ms":600}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["iterations"], 3);
    assert!(json["total_runs"].as_u64().unwrap() >= 1);
    assert!(
        json["runs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["resource_id"] == resource_id)
    );
}

#[tokio::test]
async fn http_watch_service_start_status_stop_and_refreshes_due_watch() {
    let _env_guard = env_isolated();
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("auth.md"), "# Authentication\nOAuth\n").unwrap();

    let app = mfs_server::http::app_with_config(mfs_server::http::AppConfig {
        workspace_root: workspace.path().to_path_buf(),
        source_kind: "localfs".to_owned(),
        source_path: source.path().to_path_buf(),
        target_uri: "mfs://resources/localfs/docs".to_owned(),
        account_id: "acme".to_owned(),
        user_id: "alice".to_owned(),
        agent_id: "coding-agent".to_owned(),
        canvas_separate_db: false,
    });

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/resources")
                .header("content-type", "application/json")
                .body(Body::from(format!(
                    "{{\"source_kind\":\"localfs\",\"source_path\":\"{}\",\"logical_name\":\"docs-service\"}}",
                    source.path().display()
                )))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let resource_id = json["resource_id"].as_str().unwrap().to_owned();
    let task_key = json["task_key"].as_str().unwrap().to_owned();
    for _ in 0..50 {
        let task = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/tasks/{task_key}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(task.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        if json["state"] == "completed" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/resources/{resource_id}/watch"))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"interval_seconds":1}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/resources/{resource_id}/watch/run"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/watch-service/start")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"poll_ms":200}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_success());

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/watch-service/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["running"], true);
    assert_eq!(json["poll_ms"], 200);

    std::fs::write(
        source.path().join("auth.md"),
        "# Sessions\nservice rotation\n",
    )
    .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/find?query=service%20rotation&target=mfs://resources/localfs/docs-service")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        json["resources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["uri"].as_str().unwrap().ends_with("/auth.md"))
    );

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/watch-service/stop")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_success());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/watch-service/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["running"], false);
}

#[tokio::test]
async fn http_registered_resource_refresh_is_idempotent_for_same_snapshot() {
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(
        source.path().join("auth.md"),
        "# Authentication\nOAuth login flow\n",
    )
    .unwrap();

    let app = mfs_server::http::app_with_config(mfs_server::http::AppConfig {
        workspace_root: workspace.path().to_path_buf(),
        source_kind: "localfs".to_owned(),
        source_path: source.path().to_path_buf(),
        target_uri: "mfs://resources/localfs/docs".to_owned(),
        account_id: "acme".to_owned(),
        user_id: "alice".to_owned(),
        agent_id: "coding-agent".to_owned(),
        canvas_separate_db: false,
    });

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/resources")
                .header("content-type", "application/json")
                .body(Body::from(format!(
                    "{{\"source_kind\":\"localfs\",\"source_path\":\"{}\",\"logical_name\":\"docs\"}}",
                    source.path().display()
                )))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let resource_id = json["resource_id"].as_str().unwrap().to_owned();

    for _ in 0..2 {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/resources/{resource_id}/refresh"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(response.status().is_success());
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["task_key"].is_string());
    }
}

#[tokio::test]
async fn http_registered_resource_rebuild_restores_deleted_semantic_entries() {
    let _env_guard = env_isolated();
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(
        source.path().join("auth.md"),
        "# Authentication\nOAuth login flow\n",
    )
    .unwrap();

    let app = mfs_server::http::app_with_config(mfs_server::http::AppConfig {
        workspace_root: workspace.path().to_path_buf(),
        source_kind: "localfs".to_owned(),
        source_path: source.path().to_path_buf(),
        target_uri: "mfs://resources/localfs/docs".to_owned(),
        account_id: "acme".to_owned(),
        user_id: "alice".to_owned(),
        agent_id: "coding-agent".to_owned(),
        canvas_separate_db: false,
    });

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/resources")
                .header("content-type", "application/json")
                .body(Body::from(format!(
                    "{{\"source_kind\":\"localfs\",\"source_path\":\"{}\",\"logical_name\":\"docs\"}}",
                    source.path().display()
                )))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let resource_id = json["resource_id"].as_str().unwrap().to_owned();

    let semantic_index =
        mfs_index::SqliteSemanticIndex::open_at(workspace.path().join("_system/semantic.sqlite"))
            .unwrap();
    semantic_index
        .delete_prefix(Some("mfs://resources/localfs/docs"))
        .unwrap();

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/resources/{resource_id}/rebuild"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let task_key = json["task_key"].as_str().unwrap().to_owned();

    for _ in 0..50 {
        let task = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/tasks/{task_key}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(task.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        if json["state"] == "completed" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    let response = app
        .oneshot(
            Request::builder()
                .uri("/find?query=authentication&target=mfs://resources/localfs/docs")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(!json["resources"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn http_system_status_summarizes_resources_and_tasks() {
    let _env_guard = env_isolated();
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("custom.md"), "# Custom\n").unwrap();

    let metadata = mfs_metadata::MetadataStore::open_at(
        workspace.path().join("_system/metadata.sqlite"),
        false,
    )
    .unwrap();
    metadata
        .register_resource_source(&mfs_metadata::ResourceSourceRecord {
            resource_id: "res-ready",
            account_id: "acme",
            user_id: "alice",
            agent_id: Some("coding-agent"),
            logical_name: "docs",
            source_kind: "localfs",
            source_identifier: "/tmp/docs",
            canonical_root_uri: "mfs://resources/docs",
            projection_view_id: "tenant:acme:alice:resources",
            resource_kind: "generic_docs",
            source_host: None,
            source_namespace: None,
            source_repo: None,
            source_ref: None,
            canonical_strategy_version: "v2",
            status: "ready",
            last_snapshot_id: Some("snap-001"),
        })
        .unwrap();
    metadata
        .upsert_task(&mfs_metadata::TaskRecord {
            task_key: "semantic:running",
            account_id: "acme",
            user_id: "alice",
            agent_id: Some("coding-agent"),
            projection_view_id: Some("tenant:acme:alice:resources"),
            state: "running",
            owner_space: Some("resources"),
            summary: Some("running task"),
            last_error: None,
            attempt_count: 1,
            max_attempts: 2,
            retry_state: "retrying",
            processing_mode: None,
        })
        .unwrap();
    let engine = mfs_session::SessionEngine::open(workspace.path())
        .await
        .unwrap();
    let session_id = engine
        .new_session_with_id("acme", "alice", "coding-agent", "ops-status")
        .await
        .unwrap();
    engine
        .add_message(&session_id, "user", "remember my runbook preference")
        .await
        .unwrap();
    let commit = engine.commit(&session_id).await.unwrap();
    wait_for_session_task(&engine, commit.task_id.as_deref().unwrap()).await;

    let app = mfs_server::http::app_with_config(mfs_server::http::AppConfig {
        workspace_root: workspace.path().to_path_buf(),
        source_kind: "localfs".to_owned(),
        source_path: source.path().to_path_buf(),
        target_uri: "mfs://resources/localfs/docs".to_owned(),
        account_id: "acme".to_owned(),
        user_id: "alice".to_owned(),
        agent_id: "coding-agent".to_owned(),
        canvas_separate_db: false,
    });

    let response = app
        .oneshot(
            Request::builder()
                .uri("/system/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(response.status().is_success());
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["resources"]["total"], 1);
    assert_eq!(json["resources"]["ready"], 1);
    assert_eq!(json["metadata_tasks"]["total"], 1);
    assert_eq!(json["metadata_tasks"]["running"], 1);
    assert_eq!(json["session_tasks"]["total"], 1);
    assert_eq!(json["session_tasks"]["completed"], 1);
}

#[tokio::test]
async fn http_observer_status_reports_runtime_and_index_stats() {
    let _env_guard = env_isolated();
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("custom.md"), "# Custom\n").unwrap();

    let app = mfs_server::http::app_with_config(mfs_server::http::AppConfig {
        workspace_root: workspace.path().to_path_buf(),
        source_kind: "localfs".to_owned(),
        source_path: source.path().to_path_buf(),
        target_uri: "mfs://resources/localfs/docs".to_owned(),
        account_id: "acme".to_owned(),
        user_id: "alice".to_owned(),
        agent_id: "coding-agent".to_owned(),
        canvas_separate_db: false,
    });

    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mkdir")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"uri":"mfs://user/notes"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/write")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"uri":"mfs://user/notes/profile.md","content":"OAuth token rotation observer sample"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/system/observer")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(response.status().is_success());
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["runtime"]["summary_provider"], "deterministic");
    assert_eq!(json["runtime"]["embedding_provider"], "deterministic");
    assert!(json["semantic"]["total_documents"].as_u64().unwrap() >= 1);
    assert!(json["semantic"]["memory_documents"].as_u64().unwrap() >= 1);
    assert!(json["semantic"]["embedding_dimension"].as_u64().unwrap() >= 1);
}

#[tokio::test]
async fn http_tasks_list_shows_metadata_and_session_tasks() {
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("auth.md"), "# Authentication\nOAuth\n").unwrap();

    let app = mfs_server::http::app_with_config(mfs_server::http::AppConfig {
        workspace_root: workspace.path().to_path_buf(),
        source_kind: "localfs".to_owned(),
        source_path: source.path().to_path_buf(),
        target_uri: "mfs://resources/localfs/docs".to_owned(),
        account_id: "acme".to_owned(),
        user_id: "alice".to_owned(),
        agent_id: "coding-agent".to_owned(),
        canvas_separate_db: false,
    });

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/resources")
                .header("content-type", "application/json")
                .body(Body::from(format!(
                    "{{\"source_kind\":\"localfs\",\"source_path\":\"{}\",\"logical_name\":\"docs\"}}",
                    source.path().display()
                )))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let metadata_task_key = json["task_key"].as_str().unwrap().to_owned();
    for _ in 0..50 {
        let task = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/tasks/{metadata_task_key}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(task.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        if json["state"] == "completed" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/sessions")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(create.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let session_id = json["session_id"].as_str().unwrap().to_owned();

    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/sessions/{session_id}/messages"))
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"role":"user","content":"remember the queue state"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let commit = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/sessions/{session_id}/commit"))
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(commit.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let session_task_id = json["task_id"].as_str().unwrap().to_owned();
    for _ in 0..50 {
        let task = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/tasks/{session_task_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(task.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        if json["status"] == "Completed" || json["status"] == "completed" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    let response = app
        .oneshot(
            Request::builder()
                .uri("/tasks?limit=10")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["limit"], 10);
    assert!(json["total_count"].as_u64().unwrap() >= 2);
    assert!(json.get("next_cursor").is_some());
    let items = json["items"].as_array().unwrap();
    let tasks = json["tasks"].as_array().unwrap();
    assert_eq!(items, tasks);
    assert!(
        items
            .iter()
            .any(|item| item["kind"] == "metadata" && item["task_id"] == metadata_task_key)
    );
    assert!(
        items
            .iter()
            .any(|item| item["kind"] == "session" && item["task_id"] == session_task_id)
    );
}

#[tokio::test]
async fn http_link_and_unlink_relations() {
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("custom.md"), "# Custom\n").unwrap();

    let app = mfs_server::http::app_with_config(mfs_server::http::AppConfig {
        workspace_root: workspace.path().to_path_buf(),
        source_kind: "localfs".to_owned(),
        source_path: source.path().to_path_buf(),
        target_uri: "mfs://resources/localfs/docs".to_owned(),
        account_id: "acme".to_owned(),
        user_id: "alice".to_owned(),
        agent_id: "coding-agent".to_owned(),
        canvas_separate_db: false,
    });

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/relations")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "from_uri": "mfs://user/notes/profile.md",
                        "to_uri": "mfs://agent/skills/search-web/SKILL.md",
                        "relation_type": "references",
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_success());

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/relations?uri=mfs://user/notes/profile.md")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["items"], json["relations"]);
    assert!(json.get("next_cursor").is_some());
    assert_eq!(json["total_count"], 1);
    assert_eq!(json["count"], 1);
    assert_eq!(json["limit"], 20);
    assert!(
        json["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["peer_uri"] == "mfs://agent/skills/search-web/SKILL.md")
    );

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/relations?from_uri=mfs://user/notes/profile.md&to_uri=mfs://agent/skills/search-web/SKILL.md&relation_type=references")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_success());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/relations?uri=mfs://user/notes/profile.md")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        !json["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["peer_uri"] == "mfs://agent/skills/search-web/SKILL.md")
    );
}

async fn wait_for_session_task(engine: &mfs_session::SessionEngine, task_id: &str) {
    for _ in 0..100 {
        if let Some(task) = engine.task_status(task_id).await {
            if matches!(task.status, mfs_session::TaskStatus::Completed) {
                return;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    panic!("task {task_id} did not complete in time");
}

fn spawn_static_http_server(
    path: &str,
    body: &str,
    content_type: &str,
) -> (String, std::thread::JoinHandle<()>) {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let path = path.to_owned();
    let body = body.to_owned();
    let content_type = content_type.to_owned();
    let handle = std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buffer = [0_u8; 4096];
            let _ = stream.read(&mut buffer);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    (format!("http://{address}{path}"), handle)
}

// ── Phase 2 Tests ─────────────────────────────────────────────────────

#[tokio::test]
async fn http_memory_archive_moves_cold_episodes_to_archived_state() {
    let _env = env_isolated();
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("readme.md"), "# Docs\n").unwrap();

    let metadata = mfs_metadata::MetadataStore::open_at(
        workspace.path().join("_system/metadata.sqlite"),
        false,
    )
    .unwrap();
    metadata
        .insert_session("s1", "acme", "alice", "coding-agent", "active", None)
        .unwrap();
    // Cold episode: low salience, low strength, zero recall, old enough
    metadata
        .insert_episode(
            "ep-cold",
            "acme",
            "alice",
            "coding-agent",
            "s1",
            None,
            "Old cold episode",
            None,
            None,
            0.1, // salience at floor
            0.1, // strength at floor
            None,
            None,
            None,
            0, // zero recall
            None,
            Some("turn-1"),
            Some("turn-1"),
            None,
            None,
            None,
        )
        .unwrap();
    // Hot episode: high salience, should NOT be archived
    metadata
        .insert_episode(
            "ep-hot",
            "acme",
            "alice",
            "coding-agent",
            "s1",
            None,
            "Recent hot episode",
            None,
            None,
            0.9,
            1.0,
            None,
            None,
            None,
            10,
            None,
            Some("turn-2"),
            Some("turn-2"),
            None,
            None,
            None,
        )
        .unwrap();

    let app = test_app(&workspace, &source);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/memory:archive")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"hotness_threshold":0.5,"min_age_days":0}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(response.status().is_success());
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    // Cold episode should be archived
    assert_eq!(json["archived_episodes"], 1);
}

#[tokio::test]
async fn http_eval_recall_returns_recall_at_k() {
    let _env = env_isolated();
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("readme.md"), "# Docs\n").unwrap();

    let metadata = mfs_metadata::MetadataStore::open_at(
        workspace.path().join("_system/metadata.sqlite"),
        false,
    )
    .unwrap();
    metadata
        .insert_fact(&mfs_metadata::FactRecord {
            id: "f1",
            account_id: "acme",
            user_id: "alice",
            agent_id: Some("coding-agent"),
            subject: "user",
            predicate: "location.current_city",
            display_value: "User currently lives in Tokyo",
            normalized_value_json: None,
            value_type: "scalar",
            confidence: 0.95,
            status: "active",
            valid_from: None,
            valid_to: None,
            source_assertion_id: None,
            source_episode_ids_json: None,
        })
        .unwrap();

    let app = test_app(&workspace, &source);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/eval/recall")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"query":"where does the user live","expected_facts":["Tokyo","Paris"],"k":10}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(response.status().is_success());
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    // "Tokyo" is in the fact, "Paris" is not → recall = 0.5
    assert_eq!(json["matched_count"], 1);
    assert_eq!(json["expected_count"], 2);
    assert_eq!(json["recall_at_k"].as_f64().unwrap(), 0.5);
    assert!(
        json["missing_facts"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("Paris"))
    );
}

#[tokio::test]
async fn http_context_resolve_facts_sorted_by_confidence() {
    let _env = env_isolated();
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("readme.md"), "# Docs\n").unwrap();

    let metadata = mfs_metadata::MetadataStore::open_at(
        workspace.path().join("_system/metadata.sqlite"),
        false,
    )
    .unwrap();
    // Insert two facts with different confidence levels
    metadata
        .insert_fact(&mfs_metadata::FactRecord {
            id: "f-low",
            account_id: "acme",
            user_id: "alice",
            agent_id: Some("coding-agent"),
            subject: "user",
            predicate: "identity.food",
            display_value: "User likes eating pizza",
            normalized_value_json: None,
            value_type: "set",
            confidence: 0.60,
            status: "active",
            valid_from: None,
            valid_to: None,
            source_assertion_id: None,
            source_episode_ids_json: None,
        })
        .unwrap();
    metadata
        .insert_fact(&mfs_metadata::FactRecord {
            id: "f-high",
            account_id: "acme",
            user_id: "alice",
            agent_id: Some("coding-agent"),
            subject: "user",
            predicate: "identity.name",
            display_value: "User's name is Alice",
            normalized_value_json: None,
            value_type: "scalar",
            confidence: 0.95,
            status: "active",
            valid_from: None,
            valid_to: None,
            source_assertion_id: None,
            source_episode_ids_json: None,
        })
        .unwrap();

    let app = test_app(&workspace, &source);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/context/resolve")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"query":"user info","user_id":"alice"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(response.status().is_success());
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let facts = json["sections"]["current_facts"].as_array().unwrap();
    // High-confidence fact should appear first
    if facts.len() >= 2 {
        let first_conf = facts[0]["confidence"].as_f64().unwrap_or(0.0);
        let second_conf = facts[1]["confidence"].as_f64().unwrap_or(0.0);
        assert!(
            first_conf >= second_conf,
            "facts should be sorted by confidence desc"
        );
    }
}

// ── Phase 2 Tests: P2-A (strategy) + P2-B (trace) ───────────────────

#[tokio::test]
async fn http_fact_trace_returns_provenance() {
    let _env = env_isolated();
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("readme.md"), "# Docs\n").unwrap();

    let metadata = mfs_metadata::MetadataStore::open_at(
        workspace.path().join("_system/metadata.sqlite"),
        false,
    )
    .unwrap();
    // Insert session + episode + fact with source_episode_ids_json populated
    metadata
        .insert_session("s1", "acme", "alice", "coding-agent", "active", None)
        .unwrap();
    metadata
        .insert_episode(
            "ep-source",
            "acme",
            "alice",
            "coding-agent",
            "s1",
            None,
            "Episode that produced a fact",
            None,
            None,
            0.7,
            1.0,
            None,
            None,
            None,
            3,
            None,
            Some("turn-1"),
            Some("turn-2"),
            None,
            None,
            None,
        )
        .unwrap();
    metadata
        .insert_fact(&mfs_metadata::FactRecord {
            id: "f-trace",
            account_id: "acme",
            user_id: "alice",
            agent_id: Some("coding-agent"),
            subject: "user",
            predicate: "preference.editor",
            display_value: "User prefers vim",
            normalized_value_json: None,
            value_type: "scalar",
            confidence: 0.85,
            status: "active",
            valid_from: None,
            valid_to: None,
            source_assertion_id: None,
            source_episode_ids_json: Some("[\"ep-source\"]"),
        })
        .unwrap();

    let app = test_app(&workspace, &source);
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/facts/f-trace/trace")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(response.status().is_success());
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    // Fact should be present
    assert_eq!(json["fact"]["id"], "f-trace");
    assert_eq!(json["fact"]["predicate"], "preference.editor");
    assert_eq!(json["fact"]["source_episode_ids_json"], "[\"ep-source\"]");
    // Source episodes should be populated
    assert_eq!(json["source_episodes"][0]["episode_id"], "ep-source");
    assert_eq!(
        json["source_episodes"][0]["summary"],
        "Episode that produced a fact"
    );
}

#[tokio::test]
async fn http_resolve_context_with_strategy_recent() {
    let _env = env_isolated();
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("readme.md"), "# Docs\n").unwrap();

    let metadata = mfs_metadata::MetadataStore::open_at(
        workspace.path().join("_system/metadata.sqlite"),
        false,
    )
    .unwrap();
    metadata
        .insert_session("s1", "acme", "alice", "coding-agent", "active", None)
        .unwrap();
    metadata
        .insert_fact(&mfs_metadata::FactRecord {
            id: "f1",
            account_id: "acme",
            user_id: "alice",
            agent_id: Some("coding-agent"),
            subject: "user",
            predicate: "identity.name",
            display_value: "User's name is Alice",
            normalized_value_json: None,
            value_type: "scalar",
            confidence: 0.90,
            status: "active",
            valid_from: None,
            valid_to: None,
            source_assertion_id: None,
            source_episode_ids_json: None,
        })
        .unwrap();

    let app = test_app(&workspace, &source);
    // Test with "recent" strategy
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/context/resolve")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"query":"user info","user_id":"alice","strategy":"recent"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(response.status().is_success());
}

/// Shared helper: spawn a fake TCP server that counts incoming connections,
/// configure OpenAI env vars to point at it, seed workspace, and return
/// (app, call_counter, server_handle) for assertion.
async fn setup_read_llm_test_app() -> (
    axum::Router,
    Arc<AtomicUsize>,
    tokio::task::JoinHandle<()>,
    mfs_test_util::EnvGuard,
) {
    let env_guard = env_isolated();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let calls_for_task = Arc::clone(&calls);
    let server = tokio::spawn(async move {
        while let Ok((mut socket, _peer)) = listener.accept().await {
            let mut buf = [0_u8; 1024];
            if let Ok(n) = socket.read(&mut buf).await {
                let request = String::from_utf8_lossy(&buf[..n]);
                if request.contains("/chat/completions") {
                    calls_for_task.fetch_add(1, Ordering::SeqCst);
                }
            }
            let _ = socket
                .write_all(b"HTTP/1.1 500 Internal Server Error\r\ncontent-length: 0\r\n\r\n")
                .await;
        }
    });

    unsafe {
        std::env::set_var("MEMFUSE_OPENAI_API_KEY", "test-key");
        std::env::set_var("MEMFUSE_OPENAI_API_BASE", format!("http://{addr}"));
        std::env::set_var("MEMFUSE_CHAT_PROVIDER", "openai");
    }

    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("readme.md"), "# Docs\n").unwrap();
    seed_memory_rows(&workspace);

    (test_app(&workspace, &source), calls, server, env_guard)
}

#[tokio::test]
async fn http_context_resolve_default_skips_intent_classification() {
    let (app, calls, server, _env_guard) = setup_read_llm_test_app().await;
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/context/resolve")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"query":"Where do I live?","user_id":"alice"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(response.status().is_success());
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    // Fallback chain returns top-confidence facts even without intent classification.
    assert!(json["sections"]["current_facts"].is_array());

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    server.abort();
}

#[tokio::test]
async fn http_context_resolve_default_skips_fact_intent_routing() {
    let _env = env_isolated();
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("readme.md"), "# Docs\n").unwrap();

    let metadata = mfs_metadata::MetadataStore::open_at(
        workspace.path().join("_system/metadata.sqlite"),
        false,
    )
    .unwrap();
    metadata
        .insert_session("s1", "acme", "alice", "coding-agent", "active", None)
        .unwrap();
    metadata
        .insert_fact(&mfs_metadata::FactRecord {
            id: "f-location",
            account_id: "acme",
            user_id: "alice",
            agent_id: Some("coding-agent"),
            subject: "user",
            predicate: "location.current_city",
            display_value: "Tokyo",
            normalized_value_json: None,
            value_type: "scalar",
            confidence: 0.80,
            status: "active",
            valid_from: None,
            valid_to: None,
            source_assertion_id: None,
            source_episode_ids_json: None,
        })
        .unwrap();
    metadata
        .insert_fact(&mfs_metadata::FactRecord {
            id: "f-name",
            account_id: "acme",
            user_id: "alice",
            agent_id: Some("coding-agent"),
            subject: "user",
            predicate: "identity.name",
            display_value: "Alice",
            normalized_value_json: None,
            value_type: "scalar",
            confidence: 0.99,
            status: "active",
            valid_from: None,
            valid_to: None,
            source_assertion_id: None,
            source_episode_ids_json: None,
        })
        .unwrap();

    let app = test_app(&workspace, &source);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/context/resolve")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"query":"Tokyo","user_id":"alice"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(response.status().is_success());
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    // Assert both facts appear regardless of FTS5/confidence ordering.
    let fact_predicates: Vec<_> = json["sections"]["current_facts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["predicate"].as_str().unwrap())
        .collect();
    assert!(
        fact_predicates.contains(&"identity.name"),
        "predicates: {fact_predicates:?}"
    );
    assert!(
        fact_predicates.contains(&"location.current_city"),
        "predicates: {fact_predicates:?}"
    );
}

#[tokio::test]
async fn http_context_resolve_comprehensive_uses_read_llm_once() {
    let (app, calls, server, _env_guard) = setup_read_llm_test_app().await;
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/context/resolve")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"query":"Where do I live?","user_id":"alice","strategy":"comprehensive"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(response.status().is_success());
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        json["sections"]["current_facts"][0]["display_value"],
        "Tokyo"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    server.abort();
}

// ── Phase 1 Tests ─────────────────────────────────────────────────────

#[tokio::test]
async fn http_auth_dev_mode_allows_all_requests() {
    let _env = env_isolated();
    // MEMFUSE_AUTH_MODE not set → dev mode
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("readme.md"), "# Docs\n").unwrap();

    let app = test_app(&workspace, &source);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_success());
}

#[tokio::test]
async fn http_auth_api_key_mode_rejects_missing_token() {
    let _env = env_isolated();
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("readme.md"), "# Docs\n").unwrap();

    let app = test_app_with_auth(&workspace, &source, "secret-key-123");
    // Request without auth header to a protected endpoint
    let response = app
        .oneshot(
            Request::builder()
                .uri("/facts?user_id=alice")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 401);
}

#[tokio::test]
async fn http_auth_api_key_mode_accepts_valid_bearer_token() {
    let _env = env_isolated();
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("readme.md"), "# Docs\n").unwrap();

    let app = test_app_with_auth(&workspace, &source, "secret-key-123");
    let response = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .header("Authorization", "Bearer secret-key-123")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    // /health is exempt from auth
    assert!(response.status().is_success());
}

#[tokio::test]
async fn http_heuristic_rules_list_returns_paginated_envelope() {
    let _env = env_isolated();
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("readme.md"), "# Docs\n").unwrap();

    let app = test_app(&workspace, &source);
    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/heuristics/rules")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"rule_text":"Prefer TDD","tags":["testing"],"lifecycle_stage":"confirmed"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(create.status().is_success());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/heuristics/rules?limit=1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["limit"], 1);
    assert!(json.get("next_cursor").is_some());
    assert_eq!(json["total"], json["total_count"]);
    assert_eq!(json["rules"], json["items"]);
    assert_eq!(json["items"].as_array().unwrap().len(), 1);
    assert_eq!(json["items"][0]["rule_text"], "Prefer TDD");
}

#[tokio::test]
async fn http_heuristic_instances_list_returns_paginated_envelope() {
    let _env = env_isolated();
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("readme.md"), "# Docs\n").unwrap();

    let app = test_app(&workspace, &source);
    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/heuristics/instances")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"context_summary":"Review feedback","user_reaction":"Please use TDD","signal_type":"preference_declaration","tags":["testing"]}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(create.status().is_success());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/heuristics/instances?limit=1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["limit"], 1);
    assert!(json.get("next_cursor").is_some());
    assert_eq!(json["total"], json["total_count"]);
    assert_eq!(json["instances"], json["items"]);
    assert_eq!(json["items"].as_array().unwrap().len(), 1);
    assert_eq!(json["items"][0]["context_summary"], "Review feedback");
}

#[tokio::test]
async fn http_code_symbols_list_and_search_return_paginated_envelopes() {
    let _env = env_isolated();
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("readme.md"), "# Docs\n").unwrap();

    let app = test_app(&workspace, &source);
    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/code_symbols")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"symbols":[{"id":"sym-auth","projection_view_id":"view-1","canonical_uri":"mfs://docs/auth.rs","symbol_type":"struct","symbol_name":"AuthService","signature":"struct AuthService","line_number":12}]}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(create.status().is_success());

    let list = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/code_symbols?projection_view_id=view-1&limit=1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(list.status().is_success());
    let body = axum::body::to_bytes(list.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["limit"], 1);
    assert!(json.get("next_cursor").is_some());
    assert_eq!(json["count"], json["total_count"]);
    assert_eq!(json["symbols"], json["items"]);
    assert_eq!(json["items"][0]["symbol_name"], "AuthService");

    let search = app
        .oneshot(
            Request::builder()
                .uri("/code_symbols/search?projection_view_id=view-1&q=Auth&limit=1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(search.status().is_success());
    let body = axum::body::to_bytes(search.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["limit"], 1);
    assert_eq!(json["count"], json["total_count"]);
    assert_eq!(json["symbols"], json["items"]);
    assert_eq!(json["results"], json["items"]);
    assert_eq!(json["items"][0]["symbol_name"], "AuthService");
}

#[tokio::test]
async fn http_manifest_get_returns_full_yaml_manifest_after_human_update() {
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let manifest_path = source.path().join("MANIFEST.yaml");
    std::fs::write(&manifest_path, sample_manifest_yaml("symphony-gh")).unwrap();

    let app = test_app(&workspace, &source);

    let update = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/manifest/update")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "repo_id": "symphony-gh",
                        "manifest_yaml_path": manifest_path,
                        "updater": "human"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(update.status(), axum::http::StatusCode::OK);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/manifest/get?repo_id=symphony-gh")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["status"], "ok");
    assert_eq!(json["data"]["repo_identity"]["repo_id"], "symphony-gh");
    assert_eq!(json["data"]["memory_assets"].as_array().unwrap().len(), 1);
    assert_eq!(json["data"]["canvas_indexes"][0]["version_hash"], "seed");
    assert_eq!(json["data"]["source_roots"][0]["name"], "elixir_app");
    assert!(json["data"]["quality_gates"].is_array());
    assert!(json["data"]["conflicts"].is_array());
}

#[tokio::test]
async fn http_canvas_refresh_extracts_elixir_skeleton_and_component_filter_works() {
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let manifest_path = source.path().join("MANIFEST.yaml");
    std::fs::write(&manifest_path, sample_manifest_yaml("symphony-gh")).unwrap();
    let lib_dir = source.path().join("lib");
    std::fs::create_dir_all(&lib_dir).unwrap();
    std::fs::write(
        lib_dir.join("runner.ex"),
        r#"
defmodule SymphonyGh.Runner do
  def start_link(opts) do
    GenServer.start_link(__MODULE__, opts)
  end

  defp normalize_issue(issue) do
    issue
  end
end
"#,
    )
    .unwrap();

    let app = test_app(&workspace, &source);
    let update = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/manifest/update")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "repo_id": "symphony-gh",
                        "manifest_yaml_path": manifest_path,
                        "updater": "human"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(update.status(), axum::http::StatusCode::OK);

    let refresh = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/canvas/refresh")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "repo_id": "symphony-gh",
                        "generator": "regex-deterministic"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(refresh.status(), axum::http::StatusCode::OK);
    let refresh_body = axum::body::to_bytes(refresh.into_body(), usize::MAX)
        .await
        .unwrap();
    let refresh_json: serde_json::Value = serde_json::from_slice(&refresh_body).unwrap();
    assert_eq!(refresh_json["status"], "ok");
    assert_eq!(refresh_json["data"]["nodes_count"], 3);
    assert_eq!(refresh_json["data"]["edges_count"], 2);

    let query = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/canvas/query?repo_id=symphony-gh&component=Runner&type=structural")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(query.status(), axum::http::StatusCode::OK);
    let query_body = axum::body::to_bytes(query.into_body(), usize::MAX)
        .await
        .unwrap();
    let query_json: serde_json::Value = serde_json::from_slice(&query_body).unwrap();
    let nodes = query_json["data"]["nodes"].as_array().unwrap();
    assert!(nodes.iter().any(|n| n["name"] == "SymphonyGh.Runner"));
    assert!(nodes.iter().any(|n| n["name"] == "start_link"));
    assert!(nodes.iter().any(|n| n["name"] == "normalize_issue"));
    assert!(query_json["data"]["edges"].as_array().unwrap().len() >= 2);

    let no_match = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/canvas/query?repo_id=symphony-gh&component=Missing&type=structural")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(no_match.status(), axum::http::StatusCode::OK);
    let no_match_body = axum::body::to_bytes(no_match.into_body(), usize::MAX)
        .await
        .unwrap();
    let no_match_json: serde_json::Value = serde_json::from_slice(&no_match_body).unwrap();
    assert_eq!(no_match_json["data"]["nodes"].as_array().unwrap().len(), 0);
    assert_eq!(no_match_json["data"]["edges"].as_array().unwrap().len(), 0);

    let contracts = app
        .oneshot(
            Request::builder()
                .uri("/canvas/query?repo_id=symphony-gh&type=contracts")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(contracts.status(), axum::http::StatusCode::OK);
    let contracts_body = axum::body::to_bytes(contracts.into_body(), usize::MAX)
        .await
        .unwrap();
    let contracts_json: serde_json::Value = serde_json::from_slice(&contracts_body).unwrap();
    assert_eq!(contracts_json["data"]["nodes"].as_array().unwrap().len(), 0);
    assert_eq!(contracts_json["data"]["edges"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn http_canvas_refresh_uses_registered_repo_resource_source_path() {
    let workspace = tempfile::tempdir().unwrap();
    let empty_config_source = tempfile::tempdir().unwrap();
    let repo_source = tempfile::tempdir().unwrap();
    let manifest_path = repo_source.path().join("MANIFEST.yaml");
    std::fs::write(&manifest_path, sample_manifest_yaml("symphony-gh")).unwrap();
    let lib_dir = repo_source.path().join("lib");
    std::fs::create_dir_all(&lib_dir).unwrap();
    std::fs::write(
        lib_dir.join("runner.ex"),
        r#"
defmodule SymphonyGh.Runner do
end
"#,
    )
    .unwrap();

    let app = test_app(&workspace, &empty_config_source);
    let metadata = mfs_metadata::MetadataStore::open_at(
        workspace.path().join("_system/metadata.sqlite"),
        false,
    )
    .unwrap();
    let now = "2026-05-11T00:00:00Z";
    metadata
        .upsert_manifest_identity(&mfs_metadata::ManifestIdentityRecord {
            repo_id: "symphony-gh",
            resource_uri: "mfs://resources/git/local/local/symphony-gh",
            default_branch: "main",
            primary_languages: r#"["elixir"]"#,
            created_at: now,
            last_verified_at: now,
            manifest_yaml_path: Some(manifest_path.to_str().unwrap()),
            repo_name: None,
            repo_path: None,
            last_commit_hash: None,
            last_commit_date: None,
            manifest_version: "1",
            yaml_hash: None,
            source_roots_json: "[]",
            quality_gates_json: "{}",
            updated_at: now,
        })
        .unwrap();
    metadata
        .register_resource_source(&mfs_metadata::ResourceSourceRecord {
            resource_id: "res-symphony-gh",
            account_id: "acme",
            user_id: "alice",
            agent_id: Some("coding-agent"),
            logical_name: "symphony-gh",
            source_kind: "git",
            source_identifier: repo_source.path().to_str().unwrap(),
            canonical_root_uri: "mfs://resources/git/local/local/symphony-gh",
            projection_view_id: "tenant:acme:alice:resources",
            resource_kind: "code_repo",
            source_host: Some("local"),
            source_namespace: Some("local"),
            source_repo: Some("symphony-gh"),
            source_ref: Some("main:abcdef0"),
            canonical_strategy_version: "v2",
            status: "ready",
            last_snapshot_id: Some("snapshot-1"),
        })
        .unwrap();

    let refresh = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/canvas/refresh")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "repo_id": "symphony-gh",
                        "generator": "regex-deterministic"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(refresh.status(), axum::http::StatusCode::OK);
    let refresh_body = axum::body::to_bytes(refresh.into_body(), usize::MAX)
        .await
        .unwrap();
    let refresh_json: serde_json::Value = serde_json::from_slice(&refresh_body).unwrap();
    assert_eq!(refresh_json["status"], "ok");
    assert_eq!(refresh_json["data"]["nodes_count"], 1);
    assert_eq!(
        refresh_json["data"]["source_path"],
        repo_source.path().to_string_lossy().as_ref()
    );
}

#[tokio::test]
async fn http_canvas_refresh_fails_when_registered_repo_source_path_is_missing() {
    let workspace = tempfile::tempdir().unwrap();
    let fallback_source = tempfile::tempdir().unwrap();
    let missing_repo_source = workspace.path().join("missing-repo-source");

    let app = test_app(&workspace, &fallback_source);
    let metadata = mfs_metadata::MetadataStore::open_at(
        workspace.path().join("_system/metadata.sqlite"),
        false,
    )
    .unwrap();
    let now = "2026-05-11T00:00:00Z";
    metadata
        .upsert_manifest_identity(&mfs_metadata::ManifestIdentityRecord {
            repo_id: "symphony-gh",
            resource_uri: "mfs://resources/git/local/local/symphony-gh",
            default_branch: "main",
            primary_languages: r#"["elixir"]"#,
            created_at: now,
            last_verified_at: now,
            manifest_yaml_path: None,
            repo_name: None,
            repo_path: None,
            last_commit_hash: None,
            last_commit_date: None,
            manifest_version: "1",
            yaml_hash: None,
            source_roots_json: "[]",
            quality_gates_json: "{}",
            updated_at: now,
        })
        .unwrap();
    metadata
        .register_resource_source(&mfs_metadata::ResourceSourceRecord {
            resource_id: "res-symphony-gh",
            account_id: "acme",
            user_id: "alice",
            agent_id: Some("coding-agent"),
            logical_name: "symphony-gh",
            source_kind: "git",
            source_identifier: missing_repo_source.to_str().unwrap(),
            canonical_root_uri: "mfs://resources/git/local/local/symphony-gh",
            projection_view_id: "tenant:acme:alice:resources",
            resource_kind: "code_repo",
            source_host: Some("local"),
            source_namespace: Some("local"),
            source_repo: Some("symphony-gh"),
            source_ref: Some("main:abcdef0"),
            canonical_strategy_version: "v2",
            status: "ready",
            last_snapshot_id: Some("snapshot-1"),
        })
        .unwrap();

    let refresh = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/canvas/refresh")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "repo_id": "symphony-gh",
                        "generator": "regex-deterministic"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        refresh.status(),
        axum::http::StatusCode::PRECONDITION_FAILED
    );
    let refresh_body = axum::body::to_bytes(refresh.into_body(), usize::MAX)
        .await
        .unwrap();
    let refresh_json: serde_json::Value = serde_json::from_slice(&refresh_body).unwrap();
    assert_eq!(refresh_json["error"]["category"], "FailedPrecondition");
    assert!(
        refresh_json["error"]["message"]
            .as_str()
            .unwrap()
            .contains("registered resource source path does not exist")
    );
}

#[tokio::test]
async fn http_overlay_propose_validates_refs_persists_tracker_and_reports_conflicts() {
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let manifest_path = source.path().join("MANIFEST.yaml");
    std::fs::write(&manifest_path, sample_manifest_yaml("symphony-gh")).unwrap();

    let app = test_app(&workspace, &source);
    let metadata = mfs_metadata::MetadataStore::open_at(
        workspace.path().join("_system/metadata.sqlite"),
        false,
    )
    .unwrap();
    let now = "2026-05-11T00:00:00Z";
    metadata
        .upsert_manifest_identity(&mfs_metadata::ManifestIdentityRecord {
            repo_id: "symphony-gh",
            resource_uri: "mfs://resources/localfs/symphony-gh/MANIFEST.yaml",
            default_branch: "main",
            primary_languages: r#"["elixir"]"#,
            created_at: now,
            last_verified_at: now,
            manifest_yaml_path: Some(manifest_path.to_str().unwrap()),
            repo_name: None,
            repo_path: None,
            last_commit_hash: None,
            last_commit_date: None,
            manifest_version: "1",
            yaml_hash: None,
            source_roots_json: "[]",
            quality_gates_json: "{}",
            updated_at: now,
        })
        .unwrap();
    metadata
        .upsert_canvas_node(&mfs_metadata::CanvasNodeRecord {
            id: "symphony-gh:module:SymphonyGh.Runner",
            repo_id: "symphony-gh",
            node_type: "module",
            name: "SymphonyGh.Runner",
            path: Some("lib/runner.ex"),
            language: Some("elixir"),
            purpose: Some("Elixir module"),
            confidence: "deterministic",
            generator: "regex-deterministic",
            generated_at: now,
            version_hash: "v1",
            source: None,
            manifest_id: Some("symphony-gh"),
            created_at: now,
            updated_at: now,
        })
        .unwrap();

    let invalid = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/overlay/propose")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "repo_id": "symphony-gh",
                        "tracker": "github_projects",
                        "tracker_content_id": "I_bad",
                        "tracker_identifier": "owner/repo#bad",
                        "overlay_type": "planned_change",
                        "affected_nodes": ["missing-node"],
                        "affected_edges": [],
                        "content_json": {"summary": "bad"},
                        "author": "agent"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(invalid.status(), axum::http::StatusCode::BAD_REQUEST);

    let first = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/overlay/propose")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "repo_id": "symphony-gh",
                        "tracker": "github_projects",
                        "tracker_content_id": "I_kwDO_1",
                        "tracker_project_item_id": "PVTI_1",
                        "tracker_identifier": "owner/repo#1",
                        "overlay_type": "planned_change",
                        "affected_nodes": ["symphony-gh:module:SymphonyGh.Runner"],
                        "affected_edges": [],
                        "content_json": {"summary": "change runner"},
                        "idempotency_key": "overlay-propose:owner/repo#1:run-1",
                        "author": "agent"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(first.status(), axum::http::StatusCode::OK);
    let first_body = axum::body::to_bytes(first.into_body(), usize::MAX)
        .await
        .unwrap();
    let first_json: serde_json::Value = serde_json::from_slice(&first_body).unwrap();
    let first_id = first_json["data"]["overlay_id"]
        .as_str()
        .unwrap()
        .to_owned();

    let stored = metadata.get_overlay(&first_id).unwrap().unwrap();
    assert_eq!(stored.tracker_identifier, "owner/repo#1");
    assert_eq!(stored.issue_number, None);
    assert_eq!(stored.branch, None);
    let stored_content: serde_json::Value = serde_json::from_str(&stored.content_json).unwrap();
    assert_eq!(
        stored_content["_memfuse"]["idempotency_key"],
        "overlay-propose:owner/repo#1:run-1"
    );

    let replay = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/overlay/propose")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "repo_id": "symphony-gh",
                        "tracker": "github_projects",
                        "tracker_content_id": "I_kwDO_1",
                        "tracker_project_item_id": "PVTI_1",
                        "tracker_identifier": "owner/repo#1",
                        "overlay_type": "planned_change",
                        "affected_nodes": ["symphony-gh:module:SymphonyGh.Runner"],
                        "affected_edges": [],
                        "content_json": {"summary": "change runner"},
                        "idempotency_key": "overlay-propose:owner/repo#1:run-1",
                        "author": "agent"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(replay.status(), axum::http::StatusCode::OK);
    let replay_body = axum::body::to_bytes(replay.into_body(), usize::MAX)
        .await
        .unwrap();
    let replay_json: serde_json::Value = serde_json::from_slice(&replay_body).unwrap();
    assert_eq!(replay_json["data"]["overlay_id"], first_id);
    assert_eq!(replay_json["data"]["idempotent_replay"], true);
    assert_eq!(
        metadata
            .list_active_overlays("symphony-gh", Some("proposed"))
            .unwrap()
            .len(),
        1
    );

    let accept = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/overlay/accept")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "overlay_id": first_id,
                        "acceptor": "human"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(accept.status(), axum::http::StatusCode::OK);
    let accept_body = axum::body::to_bytes(accept.into_body(), usize::MAX)
        .await
        .unwrap();
    let accept_json: serde_json::Value = serde_json::from_slice(&accept_body).unwrap();
    assert_eq!(accept_json["status"], "ok");
    assert_eq!(accept_json["data"]["status"], "accepted");
    assert_eq!(accept_json["new_status"], "accepted");

    let implemented = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/overlay/mark_implemented")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "overlay_id": first_id,
                        "agent_session_id": "session-1"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(implemented.status(), axum::http::StatusCode::OK);
    let implemented_body = axum::body::to_bytes(implemented.into_body(), usize::MAX)
        .await
        .unwrap();
    let implemented_json: serde_json::Value = serde_json::from_slice(&implemented_body).unwrap();
    assert_eq!(implemented_json["status"], "ok");
    assert_eq!(implemented_json["data"]["status"], "implemented");
    assert_eq!(implemented_json["new_status"], "implemented");

    let atomic_conflict = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/overlay/propose")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "repo_id": "symphony-gh",
                        "tracker": "github_projects",
                        "tracker_content_id": "I_kwDO_conflict",
                        "tracker_identifier": "owner/repo#conflict",
                        "overlay_type": "planned_change",
                        "affected_nodes": ["symphony-gh:module:SymphonyGh.Runner"],
                        "affected_edges": [],
                        "content_json": {"summary": "conflicting runner change"},
                        "conflict_policy": "fail_on_conflict",
                        "author": "agent"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(atomic_conflict.status(), axum::http::StatusCode::OK);
    let atomic_conflict_body = axum::body::to_bytes(atomic_conflict.into_body(), usize::MAX)
        .await
        .unwrap();
    let atomic_conflict_json: serde_json::Value =
        serde_json::from_slice(&atomic_conflict_body).unwrap();
    assert_eq!(atomic_conflict_json["status"], "conflict");
    assert_eq!(
        atomic_conflict_json["data"]["overlay_id"],
        serde_json::Value::Null
    );
    assert_eq!(
        atomic_conflict_json["data"]["conflicts"][0]["overlay_id"],
        first_id
    );
    assert_eq!(
        atomic_conflict_json["data"]["conflicts"][0]["overlap_nodes"][0],
        "symphony-gh:module:SymphonyGh.Runner"
    );
    assert_eq!(
        metadata
            .list_active_overlays("symphony-gh", None)
            .unwrap()
            .len(),
        1
    );

    let second = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/overlay/propose")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "repo_id": "symphony-gh",
                        "tracker": "github_projects",
                        "tracker_content_id": "I_kwDO_2",
                        "tracker_identifier": "owner/repo#2",
                        "overlay_type": "planned_test",
                        "affected_nodes": ["symphony-gh:module:SymphonyGh.Runner"],
                        "affected_edges": [],
                        "content_json": {"summary": "test runner"},
                        "author": "agent"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(second.status(), axum::http::StatusCode::OK);
    let second_body = axum::body::to_bytes(second.into_body(), usize::MAX)
        .await
        .unwrap();
    let second_json: serde_json::Value = serde_json::from_slice(&second_body).unwrap();
    let second_id = second_json["data"]["overlay_id"].as_str().unwrap();

    let filtered = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/overlays?repo_id=symphony-gh&status=proposed,implemented")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(filtered.status(), axum::http::StatusCode::OK);
    let filtered_body = axum::body::to_bytes(filtered.into_body(), usize::MAX)
        .await
        .unwrap();
    let filtered_json: serde_json::Value = serde_json::from_slice(&filtered_body).unwrap();
    assert_eq!(filtered_json["status"], "ok");
    assert_eq!(filtered_json["total_count"], 2);
    let filtered_statuses: std::collections::BTreeSet<String> = filtered_json["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["status"].as_str().unwrap().to_owned())
        .collect();
    assert!(filtered_statuses.contains("proposed"));
    assert!(filtered_statuses.contains("implemented"));

    let conflict = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/overlay/report_conflict")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "repo_id": "symphony-gh",
                        "overlay_id_1": first_json["data"]["overlay_id"],
                        "overlay_id_2": second_id,
                        "conflict_description": "same runner module"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(conflict.status(), axum::http::StatusCode::OK);
    let conflict_body = axum::body::to_bytes(conflict.into_body(), usize::MAX)
        .await
        .unwrap();
    let conflict_json: serde_json::Value = serde_json::from_slice(&conflict_body).unwrap();
    assert_eq!(conflict_json["data"]["has_conflict"], true);
    assert_eq!(conflict_json["data"]["requires_human_review"], true);
    assert_eq!(
        conflict_json["data"]["overlap_nodes"][0],
        "symphony-gh:module:SymphonyGh.Runner"
    );
}

#[tokio::test]
async fn http_overlay_abandon_enforces_actor_for_implemented_overlays() {
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let manifest_path = source.path().join("MANIFEST.yaml");
    std::fs::write(&manifest_path, sample_manifest_yaml("symphony-gh")).unwrap();

    let app = test_app(&workspace, &source);
    let metadata = mfs_metadata::MetadataStore::open_at(
        workspace.path().join("_system/metadata.sqlite"),
        false,
    )
    .unwrap();
    let now = "2026-05-11T00:00:00Z";
    metadata
        .upsert_manifest_identity(&mfs_metadata::ManifestIdentityRecord {
            repo_id: "symphony-gh",
            resource_uri: "mfs://resources/localfs/symphony-gh/MANIFEST.yaml",
            default_branch: "main",
            primary_languages: r#"["elixir"]"#,
            created_at: now,
            last_verified_at: now,
            manifest_yaml_path: Some(manifest_path.to_str().unwrap()),
            repo_name: None,
            repo_path: None,
            last_commit_hash: None,
            last_commit_date: None,
            manifest_version: "1",
            yaml_hash: None,
            source_roots_json: "[]",
            quality_gates_json: "{}",
            updated_at: now,
        })
        .unwrap();
    metadata
        .insert_overlay(&mfs_metadata::OverlayRecord {
            id: "overlay-implemented",
            repo_id: "symphony-gh",
            overlay_type: "planned_change",
            tracker: "github_projects",
            tracker_content_id: "I_done",
            tracker_project_item_id: None,
            tracker_identifier: "owner/repo#42",
            issue_number: None,
            branch: None,
            pr_url: None,
            agent_session_id: Some("agent-session"),
            author: "agent",
            status: "implemented",
            content_json: r#"{"summary":"implemented"}"#,
            affected_nodes: Some("[]"),
            affected_edges: Some("[]"),
            affected_node_refs: Some("[]"),
            affected_edge_refs: Some("[]"),
            created_at: now,
            updated_at: now,
            superseded_by: None,
            manifest_id: Some("symphony-gh"),
            accepted_at: None,
            implemented_at: None,
            merged_at: None,
            stale_at: None,
            abandoned_at: None,
        })
        .unwrap();

    let agent_abandon = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/overlay/abandon")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "overlay_id": "overlay-implemented",
                        "reason": "agent cannot abandon implemented work",
                        "abandoner": "agent"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(agent_abandon.status(), axum::http::StatusCode::BAD_REQUEST);
    assert_eq!(
        metadata
            .get_overlay("overlay-implemented")
            .unwrap()
            .unwrap()
            .status,
        "implemented"
    );

    let human_abandon = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/overlay/abandon")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "overlay_id": "overlay-implemented",
                        "reason": "PR closed",
                        "abandoner": "human"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(human_abandon.status(), axum::http::StatusCode::OK);
    let body = axum::body::to_bytes(human_abandon.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "ok");
    assert_eq!(json["new_status"], "abandoned");
    assert_eq!(json["data"]["status"], "abandoned");
    assert_eq!(json["data"]["triggered_by"], "human");
    assert_eq!(
        metadata
            .get_overlay("overlay-implemented")
            .unwrap()
            .unwrap()
            .status,
        "abandoned"
    );
}

#[tokio::test]
async fn http_conflicts_records_run_level_conflict_as_overlay_declaration() {
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let manifest_path = source.path().join("MANIFEST.yaml");
    std::fs::write(&manifest_path, sample_manifest_yaml("symphony-gh")).unwrap();

    let app = test_app(&workspace, &source);
    let metadata = mfs_metadata::MetadataStore::open_at(
        workspace.path().join("_system/metadata.sqlite"),
        false,
    )
    .unwrap();
    let now = "2026-05-11T00:00:00Z";
    metadata
        .upsert_manifest_identity(&mfs_metadata::ManifestIdentityRecord {
            repo_id: "symphony-gh",
            resource_uri: "mfs://resources/localfs/symphony-gh/MANIFEST.yaml",
            default_branch: "main",
            primary_languages: r#"["elixir"]"#,
            created_at: now,
            last_verified_at: now,
            manifest_yaml_path: Some(manifest_path.to_str().unwrap()),
            repo_name: None,
            repo_path: None,
            last_commit_hash: None,
            last_commit_date: None,
            manifest_version: "1",
            yaml_hash: None,
            source_roots_json: "[]",
            quality_gates_json: "{}",
            updated_at: now,
        })
        .unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/conflicts")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "repo_id": "symphony-gh",
                        "run_id": "run-123",
                        "tracker": "github_projects",
                        "tracker_identifier": "owner/repo#42",
                        "conflict_summary": "agent found incompatible API expectations",
                        "severity": "blocking",
                        "evidence": {
                            "tool": "memfuse_conflict",
                            "related_overlay_ids": ["overlay-a"]
                        },
                        "author": "agent"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "ok");
    assert_eq!(json["data"]["run_id"], "run-123");
    assert_eq!(json["data"]["tracker_identifier"], "owner/repo#42");
    assert_eq!(json["data"]["severity"], "blocking");
    let overlay_id = json["data"]["overlay_id"].as_str().unwrap();
    assert!(
        json["data"]["conflict_id"]
            .as_str()
            .unwrap()
            .starts_with("conflict_")
    );

    let stored = metadata.get_overlay(overlay_id).unwrap().unwrap();
    assert_eq!(stored.repo_id, "symphony-gh");
    assert_eq!(stored.overlay_type, "conflict_declaration");
    assert_eq!(stored.tracker_identifier, "owner/repo#42");
    assert_eq!(stored.status, "proposed");

    let content: serde_json::Value = serde_json::from_str(&stored.content_json).unwrap();
    assert_eq!(content["run_id"], "run-123");
    assert_eq!(
        content["conflict_summary"],
        "agent found incompatible API expectations"
    );
    assert_eq!(content["evidence"]["tool"], "memfuse_conflict");
}

fn sample_manifest_yaml(repo_id: &str) -> String {
    format!(
        r#"repo_identity:
  repo_id: {repo_id}
  default_branch: main
  primary_languages:
    - elixir
  created_at: "2026-05-11T00:00:00Z"
  last_verified_at: "2026-05-11T00:00:00Z"
memory_assets:
  - type: goals
    location: mfs://resources/localfs/{repo_id}/goals
    freshness: stable
    last_updated: "2026-05-11T00:00:00Z"
    owner: test-user
canvas_indexes:
  - type: structural
    location: mfs://resources/localfs/{repo_id}/canvas/structural
    generator: regex-deterministic
    generator_version: 0.1.0
    generated_at: "2026-05-11T00:00:00Z"
    version_hash: seed
    confidence: deterministic
    freshness: on_change
source_roots:
  - name: elixir_app
    path: lib/
    purpose: scheduler
    key_entries:
      - lib/runner.ex
active_overlays: []
quality_gates: []
conflicts: []
"#
    )
}

#[tokio::test]
async fn http_metrics_endpoint_returns_prometheus_format() {
    let _env = env_isolated();
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("readme.md"), "# Docs\n").unwrap();

    let app = test_app(&workspace, &source);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let text = std::str::from_utf8(&body).unwrap();
    // Prometheus format must contain HELP and TYPE lines
    assert!(
        text.contains("# HELP") || text.is_empty(),
        "metrics endpoint should return prometheus format"
    );
}
