use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

use mfs_workspace::WorkspaceFs;

#[test]
fn mcp_lists_tools_and_can_call_find() {
    let workspace = tempfile::tempdir().unwrap();
    let fixture_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/localfs_docs");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let _ = WorkspaceFs::from_localfs_source(
            workspace.path(),
            "acme",
            "alice",
            "coding-agent",
            fixture_root.to_str().unwrap(),
            "mfs://resources/localfs/docs",
        )
        .await
        .unwrap();
        let engine = mfs_session::SessionEngine::open(workspace.path())
            .await
            .unwrap();
        engine
            .new_session_with_id("acme", "alice", "coding-agent", "session-delete")
            .await
            .unwrap();
        engine
            .add_message("session-delete", "user", "turn one")
            .await
            .unwrap();
        let commit = engine.commit("session-delete").await.unwrap();
        for _ in 0..100 {
            if let Some(task) = engine.task_status(commit.task_id.as_deref().unwrap()).await {
                if matches!(task.status, mfs_session::TaskStatus::Completed) {
                    break;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    });

    let mut child = Command::new(assert_cmd::cargo::cargo_bin("mfs-mcp"))
        .args([
            "--workspace-root",
            workspace.path().to_str().unwrap(),
            "--account-id",
            "acme",
            "--user-id",
            "alice",
            "--agent-id",
            "coding-agent",
        ])
        .env("MEMFUSE_SUMMARY_PROVIDER", "deterministic")
        .env_remove("OPENAI_API_KEY")
        .env_remove("OPENAI_BASE_URL")
        .env_remove("OPENAI_COMPATIBLE_MODEL")
        .env_remove("MEMFUSE_OPENAI_API_BASE")
        .env_remove("MEMFUSE_SUMMARY_MODEL")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();

    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut stdout = BufReader::new(stdout);

    send_mcp(
        &mut stdin,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "test", "version": "0.1"}
            }
        }),
    );
    let initialize = read_mcp(&mut stdout);
    assert_eq!(initialize["result"]["serverInfo"]["name"], "mfs-mcp");

    send_mcp(
        &mut stdin,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {}
        }),
    );
    let tools = read_mcp(&mut stdout);
    assert!(
        tools["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool["name"] == "find")
    );
    assert!(
        tools["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool["name"] == "observer_status")
    );

    send_mcp(
        &mut stdin,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "find",
                "arguments": {
                    "query": "authentication",
                    "target": "mfs://resources/localfs/docs"
                }
            }
        }),
    );
    let find = read_mcp(&mut stdout);
    let text = find["result"]["content"][0]["text"].as_str().unwrap();
    let payload: serde_json::Value = serde_json::from_str(text).unwrap();
    assert!(
        payload["resources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["uri"] == "mfs://resources/localfs/docs/auth.md")
    );

    send_mcp(
        &mut stdin,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "observer_status",
                "arguments": {}
            }
        }),
    );
    let observer = read_mcp(&mut stdout);
    let text = observer["result"]["content"][0]["text"].as_str().unwrap();
    let payload: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(payload["runtime"]["summary_provider"], "deterministic");

    send_mcp(
        &mut stdin,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": {
                "name": "session_delete",
                "arguments": { "session_id": "session-delete" }
            }
        }),
    );
    let delete = read_mcp(&mut stdout);
    let text = delete["result"]["content"][0]["text"].as_str().unwrap();
    let payload: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(payload["deleted"], true);

    send_mcp(
        &mut stdin,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 6,
            "method": "tools/call",
            "params": {
                "name": "watch_run_loop",
                "arguments": { "iterations": 1, "sleep_ms": 10 }
            }
        }),
    );
    let loop_call = read_mcp(&mut stdout);
    let text = loop_call["result"]["content"][0]["text"].as_str().unwrap();
    let payload: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(payload["iterations"], 1);

    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn mcp_can_manage_sessions_skills_and_relations() {
    let workspace = tempfile::tempdir().unwrap();
    let skill_dir = tempfile::tempdir().unwrap();
    std::fs::write(
        skill_dir.path().join("SKILL.md"),
        "---\nname: search-web\ndescription: Search the web\n---\n\n# search-web\n",
    )
    .unwrap();
    let fixture_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/localfs_docs");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let _ = WorkspaceFs::from_localfs_source(
            workspace.path(),
            "acme",
            "alice",
            "coding-agent",
            fixture_root.to_str().unwrap(),
            "mfs://resources/localfs/docs",
        )
        .await
        .unwrap();
        let engine = mfs_session::SessionEngine::open(workspace.path())
            .await
            .unwrap();
        engine
            .new_session_with_id("acme", "alice", "coding-agent", "session-ctx")
            .await
            .unwrap();
        engine
            .add_message("session-ctx", "user", "turn one")
            .await
            .unwrap();
        let commit = engine.commit("session-ctx").await.unwrap();
        for _ in 0..100 {
            if let Some(task) = engine.task_status(commit.task_id.as_deref().unwrap()).await {
                if matches!(task.status, mfs_session::TaskStatus::Completed) {
                    break;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    });

    let mut child = Command::new(assert_cmd::cargo::cargo_bin("mfs-mcp"))
        .args([
            "--workspace-root",
            workspace.path().to_str().unwrap(),
            "--account-id",
            "acme",
            "--user-id",
            "alice",
            "--agent-id",
            "coding-agent",
        ])
        .env("MEMFUSE_SUMMARY_PROVIDER", "deterministic")
        .env_remove("OPENAI_API_KEY")
        .env_remove("OPENAI_BASE_URL")
        .env_remove("OPENAI_COMPATIBLE_MODEL")
        .env_remove("MEMFUSE_OPENAI_API_BASE")
        .env_remove("MEMFUSE_SUMMARY_MODEL")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut stdout = BufReader::new(stdout);

    send_mcp(
        &mut stdin,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "test", "version": "0.1"}
            }
        }),
    );
    let _ = read_mcp(&mut stdout);

    send_mcp(
        &mut stdin,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {}
        }),
    );
    let tools = read_mcp(&mut stdout);
    for tool_name in [
        "session_list",
        "session_get",
        "session_context",
        "add_skill",
        "skills_list",
        "relation_link",
        "relations_list",
        "relation_unlink",
    ] {
        assert!(
            tools["result"]["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool["name"] == tool_name),
            "missing tool {tool_name}"
        );
    }

    send_mcp(
        &mut stdin,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "session_list",
                "arguments": {}
            }
        }),
    );
    let sessions = read_mcp(&mut stdout);
    let text = sessions["result"]["content"][0]["text"].as_str().unwrap();
    let payload: serde_json::Value = serde_json::from_str(text).unwrap();
    assert!(
        payload
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["session_id"] == "session-ctx")
    );

    send_mcp(
        &mut stdin,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "add_skill",
                "arguments": { "path": skill_dir.path() }
            }
        }),
    );
    let add_skill = read_mcp(&mut stdout);
    let text = add_skill["result"]["content"][0]["text"].as_str().unwrap();
    let payload: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(payload["skill_uri"], "mfs://agent/skills/search-web");

    send_mcp(
        &mut stdin,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": {
                "name": "skills_list",
                "arguments": {}
            }
        }),
    );
    let skills = read_mcp(&mut stdout);
    let text = skills["result"]["content"][0]["text"].as_str().unwrap();
    let payload: serde_json::Value = serde_json::from_str(text).unwrap();
    assert!(
        payload
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["skill_uri"] == "mfs://agent/skills/search-web")
    );

    send_mcp(
        &mut stdin,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 6,
            "method": "tools/call",
            "params": {
                "name": "relation_link",
                "arguments": {
                    "from_uri": "mfs://resources/localfs/docs/auth.md",
                    "to_uri": "mfs://agent/skills/search-web/SKILL.md",
                    "relation_type": "references"
                }
            }
        }),
    );
    let _ = read_mcp(&mut stdout);

    send_mcp(
        &mut stdin,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "tools/call",
            "params": {
                "name": "relations_list",
                "arguments": { "uri": "mfs://resources/localfs/docs/auth.md" }
            }
        }),
    );
    let relations = read_mcp(&mut stdout);
    let text = relations["result"]["content"][0]["text"].as_str().unwrap();
    let payload: serde_json::Value = serde_json::from_str(text).unwrap();
    assert!(
        payload
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["peer_uri"] == "mfs://agent/skills/search-web/SKILL.md")
    );

    send_mcp(
        &mut stdin,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 8,
            "method": "tools/call",
            "params": {
                "name": "relation_unlink",
                "arguments": {
                    "from_uri": "mfs://resources/localfs/docs/auth.md",
                    "to_uri": "mfs://agent/skills/search-web/SKILL.md",
                    "relation_type": "references"
                }
            }
        }),
    );
    let _ = read_mcp(&mut stdout);

    send_mcp(
        &mut stdin,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 9,
            "method": "tools/call",
            "params": {
                "name": "relations_list",
                "arguments": { "uri": "mfs://resources/localfs/docs/auth.md" }
            }
        }),
    );
    let relations = read_mcp(&mut stdout);
    let text = relations["result"]["content"][0]["text"].as_str().unwrap();
    let payload: serde_json::Value = serde_json::from_str(text).unwrap();
    assert!(
        !payload
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["peer_uri"] == "mfs://agent/skills/search-web/SKILL.md")
    );

    let _ = child.kill();
    let _ = child.wait();
}

fn send_mcp(stdin: &mut impl Write, payload: serde_json::Value) {
    let body = payload.to_string();
    write!(stdin, "Content-Length: {}\r\n\r\n{}", body.len(), body).unwrap();
    stdin.flush().unwrap();
}

fn read_mcp(stdout: &mut impl BufRead) -> serde_json::Value {
    let mut header = String::new();
    loop {
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        header.push_str(&line);
        if header.ends_with("\r\n\r\n") {
            break;
        }
    }
    let length = header
        .lines()
        .find_map(|line| line.strip_prefix("Content-Length: "))
        .unwrap()
        .trim()
        .parse::<usize>()
        .unwrap();
    let mut body = vec![0_u8; length];
    stdout.read_exact(&mut body).unwrap();
    serde_json::from_slice(&body).unwrap()
}
