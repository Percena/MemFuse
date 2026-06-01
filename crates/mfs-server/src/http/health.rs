use super::*;

pub(super) async fn health_handler() -> Json<serde_json::Value> {
    let runtime = current_runtime_config();
    Json(serde_json::json!({
        "status": "alive",
        "version": env!("CARGO_PKG_VERSION"),
        "summary_provider": runtime.summary_provider,
        "embedding_provider": runtime.embedding_provider,
    }))
}

pub(super) async fn metrics_handler() -> String {
    crate::metrics::render_metrics()
}

pub(super) async fn ready_handler(
    State(state): State<Arc<AppState>>,
) -> (StatusCode, Json<serde_json::Value>) {
    let mut checks = serde_json::Map::new();
    let mut all_ok = true;

    // Check 1: Workspace directory exists
    if state.config.workspace_root.exists() {
        checks.insert("workspace".into(), serde_json::Value::String("ok".into()));
    } else {
        checks.insert(
            "workspace".into(),
            serde_json::json!({"status": "failed", "reason": "workspace root does not exist"}),
        );
        all_ok = false;
    }

    // Check 2: SQLite metadata store reachable
    if state.metadata.list_tables().is_ok() {
        checks.insert("storage".into(), serde_json::Value::String("ok".into()));
    } else {
        checks.insert(
            "storage".into(),
            serde_json::json!({"status": "failed", "reason": "metadata store unreachable"}),
        );
        all_ok = false;
    }

    // Check 3: Semantic index reachable
    let semantic_path = state
        .config
        .workspace_root
        .join("_system")
        .join("semantic.sqlite");
    if semantic_path.exists() {
        if SqliteSemanticIndex::open_at(&semantic_path).is_ok() {
            checks.insert(
                "semantic_index".into(),
                serde_json::Value::String("ok".into()),
            );
        } else {
            checks.insert(
                "semantic_index".into(),
                serde_json::json!({"status": "failed", "reason": "cannot open semantic.sqlite"}),
            );
            all_ok = false;
        }
    } else {
        checks.insert(
            "semantic_index".into(),
            serde_json::json!({"status": "warn", "reason": "semantic.sqlite not yet created"}),
        );
    }

    // Check 4: Semantic provider configuration valid
    let runtime = current_runtime_config();
    if runtime.summary_provider == "openai" && !mfs_semantic::has_openai_summary_env() {
        checks.insert("semantic_provider".into(), serde_json::json!({"status": "failed", "reason": "No OpenAI-compatible API key found (checked MEMFUSE_OPENAI_API_KEY then OPENAI_API_KEY) but summary_provider=openai"}));
        all_ok = false;
    } else {
        checks.insert(
            "semantic_provider".into(),
            serde_json::Value::String("ok".into()),
        );
    }

    let status_str = if all_ok { "ready" } else { "not_ready" };
    let status_code = if all_ok {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (
        status_code,
        Json(serde_json::json!({
            "status": status_str,
            "checks": checks,
        })),
    )
}
