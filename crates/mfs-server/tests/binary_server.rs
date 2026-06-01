use std::process::{Child, Command, Stdio};
use std::time::Duration;

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn free_local_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    listener.local_addr().expect("local addr").port()
}

async fn wait_for_health(base_url: &str) {
    let client = reqwest::Client::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        if let Ok(response) = client.get(format!("{base_url}/health")).send().await {
            if response.status().is_success() {
                return;
            }
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "server did not become healthy"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test]
async fn binary_server_enables_production_http_layers() {
    let workspace = tempfile::tempdir().unwrap();
    let port = free_local_port();
    let bind_addr = format!("127.0.0.1:{port}");
    let base_url = format!("http://{bind_addr}");
    let binary = env!("CARGO_BIN_EXE_mfs-server");

    let child = Command::new(binary)
        .env("MEMFUSE_WORKSPACE_ROOT", workspace.path())
        .env("MEMFUSE_SOURCE_KIND", "managed")
        .env("MEMFUSE_BIND_ADDR", &bind_addr)
        .env("MEMFUSE_AUTH_MODE", "api_key")
        .env("MEMFUSE_API_KEY", "test-secret")
        .env("MEMFUSE_SUMMARY_PROVIDER", "deterministic")
        .env("MEMFUSE_EMBEDDING_PROVIDER", "deterministic")
        .env("MEMFUSE_OPENAI_API_KEY", "")
        .env("OPENAI_API_KEY", "")
        .env("MEMFUSE_JINA_API_KEY", "")
        .env("RUST_LOG", "error")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn mfs-server");
    let _guard = ChildGuard(child);

    wait_for_health(&base_url).await;
    let client = reqwest::Client::new();

    let unauthenticated_write = client
        .post(format!("{base_url}/sessions"))
        .json(&serde_json::json!({"session_id": "blocked"}))
        .send()
        .await
        .expect("post sessions");
    assert_eq!(
        unauthenticated_write.status(),
        reqwest::StatusCode::UNAUTHORIZED
    );

    let openapi = client
        .get(format!("{base_url}/docs/openapi.json"))
        .send()
        .await
        .expect("get openapi");
    assert!(openapi.status().is_success());

    let preflight = client
        .request(reqwest::Method::OPTIONS, format!("{base_url}/sessions"))
        .header(reqwest::header::ORIGIN, "http://example.com")
        .header(reqwest::header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
        .send()
        .await
        .expect("cors preflight");
    assert!(preflight.status().is_success());
    assert_eq!(
        preflight
            .headers()
            .get(reqwest::header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .and_then(|value| value.to_str().ok()),
        Some("*")
    );
}
