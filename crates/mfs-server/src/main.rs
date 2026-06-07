use mfs_server::PidLockGuard;
use mfs_server::http::build_state;
use mfs_server::runtime_config::{RuntimeConfig, RuntimeOverrides, render_usage};
use std::io::ErrorKind;
use tracing::{error, info, warn};

#[tokio::main]
async fn main() {
    let overrides =
        RuntimeOverrides::from_args(std::env::args_os().skip(1)).unwrap_or_else(|err| {
            eprintln!("MemFuse server argument error: {err}\n\n{}", render_usage());
            std::process::exit(2);
        });
    if overrides.help {
        println!("{}", render_usage());
        return;
    }
    if let Some(env_file) = overrides.env_file.as_deref() {
        mfs_server::load_runtime_env_from(env_file);
    }
    let runtime_config = RuntimeConfig::load(overrides).unwrap_or_else(|err| {
        eprintln!("MemFuse runtime config error: {err}");
        std::process::exit(2);
    });
    runtime_config.apply_to_process_env();
    if runtime_config.print_config {
        println!("{}", runtime_config.render_summary());
        return;
    }

    // Initialize structured logging (RUST_LOG controls level filtering)
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    // Validate configuration safety before starting
    validate_config_safety(&runtime_config.bind_addr);

    let workspace_root = runtime_config.app.workspace_root.clone();

    // Acquire PID-based data dir lock to prevent multiple instances
    let _pid_lock = PidLockGuard::acquire(&workspace_root).unwrap_or_else(|err| {
        eprintln!(
            "MemFuse server failed to acquire workspace lock for {}: {err}",
            workspace_root.display()
        );
        std::process::exit(1);
    });

    let bind_addr = runtime_config.bind_addr.clone();
    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .unwrap_or_else(|err| {
            if err.kind() == ErrorKind::AddrInUse {
                eprintln!(
                    "MemFuse server failed to bind {bind_addr}: address is already in use.\n\
                     Stop the process using that port or set MEMFUSE_BIND_ADDR to a free address."
                );
            } else {
                eprintln!("MemFuse server failed to bind {bind_addr}: {err}");
            }
            std::process::exit(1);
        });

    info!("Server listening on {}", bind_addr);
    info!("Workspace root: {}", workspace_root.display());
    let providers = mfs_server::startup_runtime_provider_names();
    info!("Semantic provider: {}", providers.summary_provider);
    info!("Embedding provider: {}", providers.embedding_provider);

    // Spawn a watchdog that force-exits after the graceful shutdown timeout
    let shutdown_timeout_ms: u64 = std::env::var("MEMFUSE_SHUTDOWN_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(30_000);

    tokio::spawn(async move {
        shutdown_signal_watchdog().await;
        tokio::time::sleep(std::time::Duration::from_millis(shutdown_timeout_ms)).await;
        error!(
            "Shutdown timeout ({}ms) exceeded, forcing exit",
            shutdown_timeout_ms
        );
        // Exit with a non-zero code so that process supervisors (systemd,
        // Docker on-failure restart policies, etc.) can distinguish a hung
        // shutdown from a clean one and take the appropriate action.
        std::process::exit(1);
    });

    let config = runtime_config.app;
    let state = build_state(config);
    let router = mfs_server::http::app_with_state(state.clone());

    // Spawn periodic cross-session consolidation (auto-dream)
    let dream_handle =
        mfs_server::dream::spawn_dream_loop(state.clone(), state.shutdown_token.child_token());
    state.background_handles.lock().unwrap().push(dream_handle);

    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("serve app");

    // After Axum has stopped accepting new requests, drain background tasks.
    info!("Cancelling background tasks...");
    state.shutdown_token.cancel();

    // Wait for all background tasks to finish, with a shared deadline.
    // Using a deadline (rather than per-handle timeout) ensures the total
    // drain time is bounded by a single MEMFUSE_SHUTDOWN_TIMEOUT_MS window,
    // regardless of how many background tasks are running.
    let drain_timeout_ms: u64 = std::env::var("MEMFUSE_SHUTDOWN_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(30_000);
    let drain_deadline =
        tokio::time::Instant::now() + std::time::Duration::from_millis(drain_timeout_ms);
    let handles: Vec<_> = {
        let mut guard = state.background_handles.lock().unwrap();
        guard.drain(..).collect()
    };
    info!("Waiting for {} background tasks to drain...", handles.len());
    for handle in handles {
        let remaining = drain_deadline.saturating_duration_since(tokio::time::Instant::now());
        let _ = tokio::time::timeout(remaining, handle).await;
    }

    info!("Server shutdown complete");
}

/// Validate configuration safety at startup.
///
/// Checks for insecure configuration combinations and either warns or refuses to start.
fn validate_config_safety(bind_addr: &str) {
    let auth_mode = std::env::var("MEMFUSE_AUTH_MODE").unwrap_or_else(|_| "dev".to_owned());
    let providers = mfs_server::startup_runtime_provider_names();
    let summary_provider = providers.summary_provider;
    let allow_insecure = std::env::var("MEMFUSE_ALLOW_INSECURE_BIND")
        .ok()
        .map(|v| v == "true")
        .unwrap_or(false);

    // Check 1: Non-localhost binding without authentication
    let is_localhost = bind_addr.starts_with("127.")
        || bind_addr.starts_with("localhost:")
        || bind_addr == "localhost"
        || bind_addr.contains("[::1]");
    if !is_localhost && auth_mode == "dev" && !allow_insecure {
        warn!(
            "Binding to {} with auth mode '{}' (no authentication)",
            bind_addr, auth_mode
        );
        warn!("This exposes the server to network access without any authentication.");
        warn!(
            "Set MEMFUSE_AUTH_MODE=api_key or MEMFUSE_ALLOW_INSECURE_BIND=true to suppress this warning."
        );
        warn!("Continuing startup — proceed with caution.");
    }

    // Check 2: OpenAI provider without API key (using project's canonical env-priority order)
    if summary_provider == "openai" && !mfs_semantic::has_openai_summary_env() {
        error!(
            "MEMFUSE_SUMMARY_PROVIDER=openai but no OpenAI-compatible API key found (checked MEMFUSE_OPENAI_API_KEY then OPENAI_API_KEY)."
        );
        error!("Either set a valid API key or switch to MEMFUSE_SUMMARY_PROVIDER=deterministic.");
        std::process::exit(1);
    }

    let embedding_provider = providers.embedding_provider;
    if embedding_provider == "openai" && mfs_semantic::openai_api_key_from_env().is_none() {
        error!(
            "MEMFUSE_EMBEDDING_PROVIDER=openai but no OpenAI-compatible API key found (checked MEMFUSE_OPENAI_API_KEY then OPENAI_API_KEY)."
        );
        std::process::exit(1);
    }
}

async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();
    let term = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler")
            .recv()
            .await;
    };

    tokio::select! {
        _ = ctrl_c => { info!("Received Ctrl+C, shutting down gracefully"); },
        () = term => { info!("Received SIGTERM, shutting down gracefully"); },
    }
}

/// A second independent signal listener used by the shutdown watchdog.
/// Mirrors `shutdown_signal` but does not print — the main handler already logs.
async fn shutdown_signal_watchdog() {
    let ctrl_c = tokio::signal::ctrl_c();
    let term = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler (watchdog)")
            .recv()
            .await;
    };

    tokio::select! {
        _ = ctrl_c => {},
        () = term => {},
    }
}
