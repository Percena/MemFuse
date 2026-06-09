// ---------------------------------------------------------------------------
// Sub-module declarations
// ---------------------------------------------------------------------------
pub(crate) mod api_types;
mod canvas;
mod code_symbols;
mod episodes;
mod facts;
mod health;
mod heuristics;
mod manifest;
mod memory;
mod overlay;
mod relations;
mod resources;
mod runs;
mod search;
mod sessions;
mod skills;
mod snapshots;
mod tasks;
mod tickets;
mod watches;
mod webhooks;
mod workspace;

// ---------------------------------------------------------------------------
// Shared imports
// ---------------------------------------------------------------------------
use axum::{
    Json, Router,
    extract::{MatchedPath, Path, Query, State},
    http::{HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock as AsyncRwLock;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tower_http::cors::{Any, CorsLayer};
use utoipa::OpenApi;

use crate::error_handler::AppError;

/// Convenience alias so every handler can return `HandlerResult<T>` instead of
/// spelling out the full `Result<T, AppError>` type.
pub(super) type HandlerResult<T> = Result<T, AppError>;

use mfs_index::SqliteSemanticIndex;
use mfs_memory::{
    ConversationTurn, DEFAULT_INJECTION_BUDGET, MemoryContextArtifacts, MemoryContextSections,
    TurnRole,
    heuristics::{retrieve_heuristics, retrieve_l0_confirmed},
    llm::LlmAssist,
};
use mfs_metadata::{AuditEventRecord, CanvasStore, MetadataStore};
use mfs_retrieval::{RetrievalEngine, SearchResult};
use mfs_semantic::current_runtime_config;
use mfs_session::SessionEngine;
use mfs_types::{IdentityContext, MfsError};
use mfs_workspace::WorkspaceFs;
use tracing;

use crate::{
    WaitTaskOutcome, complete_prepared_resource_ingest, disable_resource_watch,
    export_resource_pack, import_resource_pack, ingest_skill, list_resource_watch_statuses,
    list_skills, mkdir_owned_path, move_owned_path, observer_status,
    prepare_inline_resource_ingest, prepare_resource_ingest, rebuild_metadata_entries,
    rebuild_registered_resource, refresh_projection, refresh_registered_resource,
    register_resource_watch, remove_owned_path, run_due_resource_watches, run_resource_watch,
    run_resource_watch_loop, system_status, wait_for_task_completion, write_owned_path,
};

// ---------------------------------------------------------------------------
// Path traversal validation
// ---------------------------------------------------------------------------

/// Validate that a user-provided filesystem path does not escape the workspace root.
/// Canonicalizes both paths and checks that the resolved path starts with the workspace root.
/// Returns the canonicalized path on success, or an InvalidArgument error on failure.
pub(super) fn validate_path_within_workspace(
    workspace_root: &std::path::Path,
    user_path: &str,
) -> Result<std::path::PathBuf, AppError> {
    let canonical_root = workspace_root.canonicalize().map_err(|e| {
        AppError(MfsError::Internal {
            message: format!("cannot canonicalize workspace root: {}", e),
        })
    })?;
    let user_path_buf = std::path::Path::new(user_path);
    // If the path is relative, resolve it against the workspace root first
    let resolved = if user_path_buf.is_relative() {
        canonical_root.join(user_path_buf)
    } else {
        user_path_buf.to_path_buf()
    };
    // For paths that don't yet exist (e.g. export output_path), canonicalize
    // the existing parent and append the remaining non-existent components.
    let canonical_path = if resolved.exists() {
        resolved.canonicalize().map_err(|e| {
            AppError(MfsError::InvalidArgument {
                field: "path".into(),
                reason: format!(
                    "path '{}' does not exist or cannot be resolved: {}",
                    user_path, e
                ),
            })
        })?
    } else {
        // Walk up to find the deepest existing ancestor, canonicalize it,
        // then append the remaining non-existent components.
        let mut existing = resolved.clone();
        let mut suffix = std::path::PathBuf::new();
        while !existing.exists() {
            if let Some(name) = existing.file_name() {
                suffix = suffix.join(name);
            }
            if !existing.pop() {
                return Err(AppError(MfsError::InvalidArgument {
                    field: "path".into(),
                    reason: format!(
                        "path '{}' has no existing ancestor for traversal check",
                        user_path
                    ),
                }));
            }
        }
        let canonical_existing = existing.canonicalize().map_err(|e| {
            AppError(MfsError::InvalidArgument {
                field: "path".into(),
                reason: format!("path '{}' cannot canonicalize ancestor: {}", user_path, e),
            })
        })?;
        canonical_existing.join(suffix)
    };
    if !canonical_path.starts_with(&canonical_root) {
        return Err(AppError(MfsError::InvalidArgument {
            field: "path".into(),
            reason: format!(
                "path '{}' resolves outside the workspace root (path traversal blocked)",
                user_path
            ),
        }));
    }
    Ok(canonical_path)
}

// ---------------------------------------------------------------------------
// Shared query parameter structs
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct UriQuery {
    pub uri: String,
}

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    pub query: String,
    pub target: Option<String>,
    pub session_context: Option<String>,
    pub limit: Option<usize>,
}

impl SearchQuery {
    /// Validate that the query string is not excessively long.
    /// Returns an error if the query exceeds the maximum allowed length.
    pub fn validate_query_length(&self, max_len: usize) -> Result<(), AppError> {
        if self.query.len() > max_len {
            return Err(AppError(MfsError::InvalidArgument {
                field: "query".into(),
                reason: format!(
                    "query exceeds maximum length of {} characters (got {})",
                    max_len,
                    self.query.len()
                ),
            }));
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
pub struct GlobQuery {
    pub uri: String,
    pub pattern: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SessionContextQuery {
    pub token_budget: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct TreeQuery {
    pub uri: String,
    pub depth: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct AuditQuery {
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WaitQuery {
    pub timeout_ms: Option<u64>,
    pub poll_ms: Option<u64>,
}

// ---------------------------------------------------------------------------
// AppConfig, AppState, and supporting types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub workspace_root: PathBuf,
    pub source_kind: String,
    pub source_path: PathBuf,
    pub target_uri: String,
    pub account_id: String,
    pub user_id: String,
    pub agent_id: String,
    pub canvas_separate_db: bool,
}

pub struct AppState {
    pub config: AppConfig,
    pub auth_mode: crate::auth::AuthMode,
    pub api_key_config: Option<crate::auth::ApiKeyConfig>,
    pub session_engine: Arc<SessionEngine>,
    pub metadata: Arc<MetadataStore>,
    /// Canvas data store — trait object for future PG backend substitution.
    /// Currently backed by the same MetadataStore via CanvasStore impl.
    pub canvas_store: Arc<dyn CanvasStore>,
    /// Cached embedding provider — created once at startup, reused across requests.
    pub embedding_provider: Box<dyn mfs_semantic::EmbeddingProvider>,
    /// Cached LLM assistant for the latency-bounded read/context path.
    /// Shares a CircuitBreaker across requests so it can accumulate failure state.
    pub read_llm: LlmAssist,
    /// Whether LLM intent classification is enabled for non-Comprehensive
    /// read paths (baked at startup from MEMFUSE_READ_LLM_ENABLED).
    pub read_llm_enabled: bool,
    /// Embedding timeout for non-Comprehensive read paths in ms
    /// (baked at startup from MEMFUSE_READ_EMBED_TIMEOUT_MS; 0 = no timeout).
    pub read_embed_timeout_ms: u64,
    pub(super) retrieval_cache:
        AsyncRwLock<HashMap<RetrievalCacheKey, Arc<tokio::sync::Mutex<RetrievalEngine>>>>,
    retrieval_cache_entries: AtomicU64,
    retrieval_cache_builds: AtomicU64,
    retrieval_cache_hits: AtomicU64,
    retrieval_cache_invalidations: AtomicU64,
    pub(super) watch_service: Mutex<WatchServiceState>,
    pub(super) rate_limiter: RateLimiter,
    /// Token cancelled on server shutdown; child tokens derived for spawned tasks.
    pub shutdown_token: CancellationToken,
    /// Join handles for background tasks so we can drain them on shutdown.
    pub background_handles: Mutex<Vec<JoinHandle<()>>>,
}

pub(super) struct RateLimiter {
    config: RateLimitConfig,
    buckets: Mutex<HashMap<String, RateLimitBucket>>,
}

#[derive(Clone, Copy)]
struct RateLimitConfig {
    enabled: bool,
    max_requests: u64,
    window: Duration,
}

struct RateLimitBucket {
    window_started: Instant,
    count: u64,
}

impl RateLimiter {
    fn from_env() -> Self {
        let enabled = std::env::var("MEMFUSE_RATE_LIMIT_ENABLED")
            .ok()
            .map(|v| {
                matches!(
                    v.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            })
            .unwrap_or(false);
        let max_requests = std::env::var("MEMFUSE_RATE_LIMIT_REQUESTS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(120);
        let window_secs = std::env::var("MEMFUSE_RATE_LIMIT_WINDOW_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(60);

        Self {
            config: RateLimitConfig {
                enabled: enabled && max_requests > 0 && window_secs > 0,
                max_requests,
                window: Duration::from_secs(window_secs),
            },
            buckets: Mutex::new(HashMap::new()),
        }
    }

    fn check(&self, key: &str) -> Result<(), u64> {
        if !self.config.enabled {
            return Ok(());
        }

        let now = Instant::now();
        let mut buckets = self.buckets.lock().unwrap_or_else(|e| e.into_inner());
        buckets.retain(|_, bucket| now.duration_since(bucket.window_started) < self.config.window);

        let bucket = buckets.entry(key.to_owned()).or_insert(RateLimitBucket {
            window_started: now,
            count: 0,
        });
        if now.duration_since(bucket.window_started) >= self.config.window {
            bucket.window_started = now;
            bucket.count = 0;
        }

        if bucket.count >= self.config.max_requests {
            let elapsed = now.duration_since(bucket.window_started);
            let retry_after = self.config.window.saturating_sub(elapsed).as_secs().max(1);
            return Err(retry_after);
        }

        bucket.count += 1;
        Ok(())
    }
}

impl AppState {
    /// Push a background task handle, converting Mutex poison to a graceful error
    /// instead of panicking. Also evicts completed handles to prevent unbounded growth.
    pub(super) fn push_background_handle(&self, handle: JoinHandle<()>) -> Result<(), AppError> {
        let mut guard = self.background_handles.lock().map_err(|e| {
            AppError(MfsError::Internal {
                message: format!("background_handles mutex poisoned: {}", e),
            })
        })?;
        // Evict completed handles before pushing the new one
        guard.retain(|h| !h.is_finished());
        guard.push(handle);
        Ok(())
    }
}

pub(super) struct WatchServiceState {
    pub(super) status: Arc<Mutex<WatchServiceStatus>>,
    pub(super) stop: Option<CancellationToken>,
    pub(super) handle: Option<JoinHandle<()>>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub(super) struct WatchServiceStatus {
    pub(super) running: bool,
    pub(super) poll_ms: u64,
    pub(super) started_at_ms: Option<u128>,
    pub(super) stopped_at_ms: Option<u128>,
    pub(super) last_tick_at_ms: Option<u128>,
    pub(super) total_ticks: u64,
    pub(super) total_runs: u64,
    pub(super) last_run_count: u64,
}

#[derive(Debug, Clone, Eq)]
pub(super) struct RetrievalCacheKey {
    workspace_root: PathBuf,
    projection_root: PathBuf,
    projection_uri: String,
    account_id: String,
    user_id: String,
    agent_space_name: String,
}

impl RetrievalCacheKey {
    pub(super) fn from_parts(
        workspace_root: &std::path::Path,
        identity: &IdentityContext,
        projection_root: &std::path::Path,
        projection_uri: &str,
    ) -> Self {
        Self {
            workspace_root: workspace_root.to_path_buf(),
            projection_root: projection_root.to_path_buf(),
            projection_uri: projection_uri.to_owned(),
            account_id: identity.account_id().to_owned(),
            user_id: identity.user_id().to_owned(),
            agent_space_name: identity.agent_space_name(),
        }
    }
}

impl PartialEq for RetrievalCacheKey {
    fn eq(&self, other: &Self) -> bool {
        self.workspace_root == other.workspace_root
            && self.projection_root == other.projection_root
            && self.projection_uri == other.projection_uri
            && self.account_id == other.account_id
            && self.user_id == other.user_id
            && self.agent_space_name == other.agent_space_name
    }
}

impl Hash for RetrievalCacheKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.workspace_root.hash(state);
        self.projection_root.hash(state);
        self.projection_uri.hash(state);
        self.account_id.hash(state);
        self.user_id.hash(state);
        self.agent_space_name.hash(state);
    }
}

impl AppState {
    pub fn retrieval_cache_entry_count(&self) -> usize {
        self.retrieval_cache_entries.load(Ordering::Relaxed) as usize
    }

    pub fn retrieval_cache_build_count(&self) -> u64 {
        self.retrieval_cache_builds.load(Ordering::Relaxed)
    }

    pub fn retrieval_cache_hit_count(&self) -> u64 {
        self.retrieval_cache_hits.load(Ordering::Relaxed)
    }

    pub fn retrieval_cache_invalidation_count(&self) -> u64 {
        self.retrieval_cache_invalidations.load(Ordering::Relaxed)
    }
}

impl AppConfig {
    pub fn from_env() -> Result<Self, std::env::VarError> {
        let source_kind =
            std::env::var("MEMFUSE_SOURCE_KIND").unwrap_or_else(|_| "managed".to_owned());
        let source_path = match source_kind.as_str() {
            "localfs" | "git" => mfs_types::expand_tilde(&std::env::var("MEMFUSE_SOURCE_PATH")?),
            _ => mfs_types::expand_tilde(&std::env::var("MEMFUSE_SOURCE_PATH").unwrap_or_default()),
        };

        Ok(Self {
            workspace_root: mfs_types::expand_tilde(&std::env::var("MEMFUSE_WORKSPACE_ROOT")?),
            source_kind,
            source_path,
            target_uri: std::env::var("MEMFUSE_TARGET_URI")
                .unwrap_or_else(|_| "mfs://resources/localfs/docs".to_owned()),
            account_id: std::env::var("MEMFUSE_ACCOUNT_ID")
                .unwrap_or_else(|_| "default".to_owned()),
            user_id: std::env::var("MEMFUSE_USER_ID").unwrap_or_else(|_| "default".to_owned()),
            agent_id: std::env::var("MEMFUSE_AGENT_ID").unwrap_or_else(|_| "default".to_owned()),
            canvas_separate_db: std::env::var("MEMFUSE_CANVAS_SEPARATE_DB")
                .ok()
                .map(|v| {
                    matches!(
                        v.trim().to_ascii_lowercase().as_str(),
                        "1" | "true" | "yes" | "on"
                    )
                })
                .unwrap_or(false),
        })
    }
}

pub fn build_state(config: AppConfig) -> Arc<AppState> {
    let auth_mode = crate::auth::AuthMode::from_env();
    let api_key_config = crate::auth::ApiKeyConfig::from_env();
    let separate_canvas_db = config.canvas_separate_db;
    let metadata = Arc::new(
        MetadataStore::open_at(
            config
                .workspace_root
                .join("_system")
                .join("metadata.sqlite"),
            separate_canvas_db,
        )
        .expect("Failed to open metadata database"),
    );
    let canvas_store: Arc<dyn CanvasStore> = metadata.clone() as Arc<dyn CanvasStore>;
    let auto_commit_threshold = std::env::var("MEMFUSE_AUTO_COMMIT_THRESHOLD")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(mfs_session::AUTO_COMMIT_THRESHOLD_DEFAULT);
    let session_engine = Arc::new(SessionEngine::from_workspace_root_with_threshold(
        config.workspace_root.clone(),
        auto_commit_threshold,
    ));
    Arc::new(AppState {
        config,
        auth_mode,
        api_key_config,
        session_engine,
        metadata,
        canvas_store,
        embedding_provider: mfs_semantic::embedding_provider_from_env(256),
        read_llm: LlmAssist::from_env_for_read(),
        read_llm_enabled: std::env::var("MEMFUSE_READ_LLM_ENABLED")
            .ok()
            .map(|v| {
                matches!(
                    v.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            })
            .unwrap_or(false),
        read_embed_timeout_ms: mfs_semantic::env_parse("MEMFUSE_READ_EMBED_TIMEOUT_MS", 1500),
        retrieval_cache: AsyncRwLock::new(HashMap::new()),
        retrieval_cache_entries: AtomicU64::new(0),
        retrieval_cache_builds: AtomicU64::new(0),
        retrieval_cache_hits: AtomicU64::new(0),
        retrieval_cache_invalidations: AtomicU64::new(0),
        watch_service: Mutex::new(WatchServiceState {
            status: Arc::new(Mutex::new(WatchServiceStatus {
                running: false,
                poll_ms: 0,
                started_at_ms: None,
                stopped_at_ms: None,
                last_tick_at_ms: None,
                total_ticks: 0,
                total_runs: 0,
                last_run_count: 0,
            })),
            stop: None,
            handle: None,
        }),
        rate_limiter: RateLimiter::from_env(),
        shutdown_token: CancellationToken::new(),
        background_handles: Mutex::new(Vec::new()),
    })
}

/// Build AppState with explicit auth configuration (for tests).
pub fn build_state_with_auth(
    config: AppConfig,
    auth_mode: crate::auth::AuthMode,
    api_key_config: Option<crate::auth::ApiKeyConfig>,
) -> Arc<AppState> {
    let separate_canvas_db = config.canvas_separate_db;
    let metadata = Arc::new(
        MetadataStore::open_at(
            config
                .workspace_root
                .join("_system")
                .join("metadata.sqlite"),
            separate_canvas_db,
        )
        .expect("Failed to open metadata database"),
    );
    let canvas_store: Arc<dyn CanvasStore> = metadata.clone() as Arc<dyn CanvasStore>;
    let auto_commit_threshold = std::env::var("MEMFUSE_AUTO_COMMIT_THRESHOLD")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(mfs_session::AUTO_COMMIT_THRESHOLD_DEFAULT);
    let session_engine = Arc::new(SessionEngine::from_workspace_root_with_threshold(
        config.workspace_root.clone(),
        auto_commit_threshold,
    ));
    Arc::new(AppState {
        config,
        auth_mode,
        api_key_config,
        session_engine,
        metadata,
        canvas_store,
        embedding_provider: mfs_semantic::embedding_provider_from_env(256),
        read_llm: LlmAssist::from_env_for_read(),
        read_llm_enabled: std::env::var("MEMFUSE_READ_LLM_ENABLED")
            .ok()
            .map(|v| {
                matches!(
                    v.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            })
            .unwrap_or(false),
        read_embed_timeout_ms: mfs_semantic::env_parse("MEMFUSE_READ_EMBED_TIMEOUT_MS", 1500),
        retrieval_cache: AsyncRwLock::new(HashMap::new()),
        retrieval_cache_entries: AtomicU64::new(0),
        retrieval_cache_builds: AtomicU64::new(0),
        retrieval_cache_hits: AtomicU64::new(0),
        retrieval_cache_invalidations: AtomicU64::new(0),
        watch_service: Mutex::new(WatchServiceState {
            status: Arc::new(Mutex::new(WatchServiceStatus {
                running: false,
                poll_ms: 0,
                started_at_ms: None,
                stopped_at_ms: None,
                last_tick_at_ms: None,
                total_ticks: 0,
                total_runs: 0,
                last_run_count: 0,
            })),
            stop: None,
            handle: None,
        }),
        rate_limiter: RateLimiter::from_env(),
        shutdown_token: CancellationToken::new(),
        background_handles: Mutex::new(Vec::new()),
    })
}

// ---------------------------------------------------------------------------
// Router assembly
// ---------------------------------------------------------------------------

pub fn api_router() -> Router<Arc<AppState>> {
    Router::new()
        .merge(v1_core_router())
        // Health & readiness
        .route("/health", get(health::health_handler))
        .route("/ready", get(health::ready_handler))
        .route("/metrics", get(health::metrics_handler))
        // Workspace
        .route("/ls", get(workspace::ls))
        .route("/tree", get(workspace::tree))
        .route("/stat", get(workspace::stat))
        .route("/abstract", get(workspace::abstract_text))
        .route("/overview", get(workspace::overview_text))
        .route("/read", get(workspace::read))
        .route("/glob", get(workspace::glob))
        .route("/mkdir", post(workspace::mkdir))
        .route("/write", post(workspace::write))
        .route("/mv", post(workspace::mv))
        .route("/rm", delete(workspace::rm))
        // Search
        .route("/find", get(search::find))
        .route("/grep", get(search::grep))
        .route("/search", get(search::search))
        .route("/rebuild", get(search::rebuild))
        .route("/refresh", post(search::refresh))
        // Resources
        .route("/resources", post(resources::create_resource))
        .route("/resources/batch", post(resources::create_resources_batch))
        .route("/resources", get(resources::list_resources))
        .route("/resources/import", post(resources::import_resource))
        .route(
            "/resources/{resource_id}/export",
            post(resources::export_resource),
        )
        .route(
            "/resources/{resource_id}/refresh",
            post(resources::refresh_resource),
        )
        .route(
            "/resources/{resource_id}/rebuild",
            post(resources::rebuild_resource),
        )
        // Watches
        .route("/watches", get(watches::list_watches))
        .route("/watches/run-due", post(watches::run_due_watches))
        .route("/watches/run-loop", post(watches::run_watch_loop_handler))
        .route("/watch-service/start", post(watches::start_watch_service))
        .route("/watch-service/status", get(watches::watch_service_status))
        .route("/watch-service/stop", post(watches::stop_watch_service))
        .route(
            "/resources/{resource_id}/watch",
            post(watches::register_watch),
        )
        .route(
            "/resources/{resource_id}/watch/disable",
            post(watches::disable_watch),
        )
        .route(
            "/resources/{resource_id}/watch/run",
            post(watches::run_watch),
        )
        // Skills
        .route("/skills", post(skills::add_skill))
        .route("/skills", get(skills::list_skills_handler))
        // Relations
        .route("/relations", post(relations::link_relation))
        .route("/relations", get(relations::list_relations))
        .route("/relations", delete(relations::unlink_relation))
        // Snapshots & audit
        .route("/snapshots", get(snapshots::snapshots))
        .route("/audit", get(snapshots::audit))
        // Sessions
        .route("/sessions", post(sessions::create_session))
        .route("/sessions", get(sessions::list_sessions))
        .route("/sessions/{session_id}", get(sessions::get_session))
        .route("/sessions/{session_id}", delete(sessions::delete_session))
        .route(
            "/sessions/{session_id}/messages",
            post(sessions::add_message),
        )
        .route(
            "/sessions/{session_id}/context",
            get(sessions::get_session_context),
        )
        .route(
            "/sessions/{session_id}/archives/{archive_id}",
            get(sessions::get_session_archive),
        )
        .route(
            "/sessions/{session_id}/used_context",
            post(sessions::used_context),
        )
        .route(
            "/sessions/{session_id}/used_skill",
            post(sessions::used_skill),
        )
        .route(
            "/sessions/{session_id}/used_tool",
            post(sessions::used_tool),
        )
        .route(
            "/sessions/{session_id}/commit",
            post(sessions::commit_session),
        )
        .route(
            "/sessions/{session_id}/observations",
            post(sessions::add_observation),
        )
        .route(
            "/sessions/{session_id}/timeline",
            get(sessions::session_timeline),
        )
        // Episodes
        .route("/episodes/{episode_id}", get(episodes::get_episode_detail))
        .route(
            "/episodes/{episode_id}/timeline",
            get(episodes::get_episode_timeline),
        )
        // Citation feedback
        .route("/memories/cite", post(episodes::cite_memories))
        // Memory export/import
        .route("/memories/export", get(memory::memory_export))
        .route("/memories/import", post(memory::memory_import))
        // Ebbinghaus retention scores
        .route("/memories/retention-scores", get(memory::retention_scores))
        // Temporal Graph: AS OF queries on relations
        .route(
            "/relations/temporal-query",
            post(memory::relations_temporal_query),
        )
        // Facts
        .route("/facts", get(facts::list_facts))
        .route("/facts", post(facts::create_fact))
        .route("/facts/{fact_id}/supersede", post(facts::supersede_fact))
        .route("/facts/{fact_id}/retract", post(facts::retract_fact))
        // Fact provenance trace (P2-B)
        .route("/facts/{fact_id}/trace", get(facts::trace_fact))
        // Memory context
        .route("/context/resolve", post(memory::resolve_memory_context))
        // Memory-specific routes (MemFuse)
        .route("/v1/memory:search", post(memory::memory_search))
        .route("/v1/memory:consolidate", post(memory::memory_consolidate))
        .route(
            "/v1/memory:extract-facts",
            post(memory::memory_extract_facts),
        )
        // Phase 2-3: archive cold episodes
        .route("/v1/memory:archive", post(memory::memory_archive))
        // Phase 2-4: recall evaluation
        .route("/v1/eval/recall", post(memory::eval_recall))
        // Heuristic rules (T2H Phase 1)
        .route("/heuristics/rules", post(heuristics::create_heuristic_rule))
        .route("/heuristics/rules", get(heuristics::list_heuristic_rules))
        .route(
            "/heuristics/rules/{rule_id}",
            get(heuristics::get_heuristic_rule),
        )
        .route(
            "/heuristics/rules/{rule_id}/promote",
            post(heuristics::promote_heuristic_rule),
        )
        .route(
            "/heuristics/rules/{rule_id}/confirm",
            post(heuristics::confirm_heuristic_rule),
        )
        .route(
            "/heuristics/instances",
            post(heuristics::create_heuristic_instance),
        )
        .route(
            "/heuristics/instances",
            get(heuristics::list_heuristic_instances),
        )
        .route(
            "/heuristics/instances/{instance_id}",
            get(heuristics::get_heuristic_instance),
        )
        .route(
            "/heuristics/retrieve",
            post(heuristics::retrieve_heuristics_handler),
        )
        .route(
            "/heuristics/l0-confirmed",
            post(heuristics::l0_confirmed_rules),
        )
        .route(
            "/heuristics/simulate-reaction",
            post(heuristics::simulate_reaction_handler),
        )
        // Code symbols
        .route("/code_symbols", get(code_symbols::list_code_symbols))
        .route("/code_symbols", post(code_symbols::create_code_symbols))
        .route(
            "/code_symbols/search",
            get(code_symbols::search_code_symbols),
        )
        .route(
            "/code_symbols/{view_id}",
            delete(code_symbols::delete_code_symbols),
        )
        // System status
        .route("/system/status", get(workspace::system_status_handler))
        .route("/system/observer", get(workspace::observer_status_handler))
        // Tasks
        .route("/tasks", get(tasks::list_tasks))
        .route("/tasks/evict", post(tasks::evict_tasks))
        .route("/tasks/{task_id}", get(tasks::task_status))
        .route("/tasks/{task_id}/wait", get(tasks::wait_task))
        // Manifest
        .route("/manifest/get", get(manifest::get_manifest))
        .route("/manifest/update", post(manifest::update_manifest))
        // Canvas
        .route("/canvas/query", get(canvas::query_canvas))
        .route("/canvas/refresh", post(canvas::refresh_canvas))
        .route("/canvas/snapshot", post(canvas::create_snapshot))
        .route("/canvas/sync-status", get(canvas::sync_status))
        .route("/canvas/snapshot/latest", get(canvas::get_latest_snapshot))
        .route("/canvas/version-hash", get(canvas::get_version_hash))
        .route("/overlay/propose", post(overlay::propose_overlay))
        .route("/overlay/accept", post(overlay::accept_overlay))
        .route("/overlay/mark_implemented", post(overlay::mark_implemented))
        .route("/overlay/abandon", post(overlay::abandon_overlay))
        .route("/overlay/report_conflict", post(overlay::report_conflict))
        .route("/overlay/consolidate", post(overlay::consolidate))
        .route("/overlays", get(overlay::list_overlays))
        .route("/conflicts", post(overlay::record_conflict))
        .route("/runs/writeback", post(runs::writeback_run))
        .route("/tickets/history", get(tickets::ticket_history))
        .layer(middleware::from_fn(legacy_api_version_middleware))
        .layer(middleware::from_fn(metrics_middleware))
}

fn v1_core_router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/v1/workspace/ls", get(workspace::ls))
        .route("/v1/workspace/tree", get(workspace::tree))
        .route("/v1/workspace/stat", get(workspace::stat))
        .route("/v1/workspace/abstract", get(workspace::abstract_text))
        .route("/v1/workspace/overview", get(workspace::overview_text))
        .route("/v1/workspace/read", get(workspace::read))
        .route("/v1/workspace/glob", get(workspace::glob))
        .route("/v1/workspace/mkdir", post(workspace::mkdir))
        .route("/v1/workspace/write", post(workspace::write))
        .route("/v1/workspace/mv", post(workspace::mv))
        .route("/v1/workspace/rm", delete(workspace::rm))
        .route("/v1/workspace/find", get(search::find))
        .route("/v1/workspace/grep", get(search::grep))
        .route("/v1/workspace/search", get(search::search))
        .route("/v1/workspace/rebuild", get(search::rebuild))
        .route("/v1/workspace/refresh", post(search::refresh))
        .route("/v1/resources", post(resources::create_resource))
        .route(
            "/v1/resources/batch",
            post(resources::create_resources_batch),
        )
        .route("/v1/resources", get(resources::list_resources))
        .route("/v1/resources/import", post(resources::import_resource))
        .route(
            "/v1/resources/{resource_id}/export",
            post(resources::export_resource),
        )
        .route(
            "/v1/resources/{resource_id}/refresh",
            post(resources::refresh_resource),
        )
        .route(
            "/v1/resources/{resource_id}/rebuild",
            post(resources::rebuild_resource),
        )
        .route("/v1/sessions", post(sessions::create_session))
        .route("/v1/sessions", get(sessions::list_sessions))
        .route("/v1/sessions/{session_id}", get(sessions::get_session))
        .route(
            "/v1/sessions/{session_id}",
            delete(sessions::delete_session),
        )
        .route(
            "/v1/sessions/{session_id}/messages",
            post(sessions::add_message),
        )
        .route(
            "/v1/sessions/{session_id}/context",
            get(sessions::get_session_context),
        )
        .route(
            "/v1/sessions/{session_id}/archives/{archive_id}",
            get(sessions::get_session_archive),
        )
        .route(
            "/v1/sessions/{session_id}/used_context",
            post(sessions::used_context),
        )
        .route(
            "/v1/sessions/{session_id}/used_skill",
            post(sessions::used_skill),
        )
        .route(
            "/v1/sessions/{session_id}/used_tool",
            post(sessions::used_tool),
        )
        .route(
            "/v1/sessions/{session_id}/commit",
            post(sessions::commit_session),
        )
        .route(
            "/v1/sessions/{session_id}/observations",
            post(sessions::add_observation),
        )
        .route(
            "/v1/sessions/{session_id}/timeline",
            get(sessions::session_timeline),
        )
        .route("/v1/facts", get(facts::list_facts))
        .route("/v1/facts", post(facts::create_fact))
        .route("/v1/facts/{fact_id}/supersede", post(facts::supersede_fact))
        .route("/v1/facts/{fact_id}/retract", post(facts::retract_fact))
        .route("/v1/facts/{fact_id}/trace", get(facts::trace_fact))
        .route("/v1/context/resolve", post(memory::resolve_memory_context))
        .route("/v1/webhooks", post(webhooks::create_webhook))
        .route("/v1/webhooks", get(webhooks::list_webhooks))
        .route("/v1/webhooks/{id}", delete(webhooks::delete_webhook))
        .route("/v1/webhooks/{id}/test", post(webhooks::test_webhook))
        // Manifest
        .route("/v1/manifest/get", get(manifest::get_manifest))
        .route("/v1/manifest/update", post(manifest::update_manifest))
        // Canvas
        .route("/v1/canvas/query", get(canvas::query_canvas))
        .route("/v1/canvas/refresh", post(canvas::refresh_canvas))
        .route("/v1/canvas/snapshot", post(canvas::create_snapshot))
        .route("/v1/canvas/sync-status", get(canvas::sync_status))
        .route(
            "/v1/canvas/snapshot/latest",
            get(canvas::get_latest_snapshot),
        )
        .route("/v1/canvas/version-hash", get(canvas::get_version_hash))
        .route("/v1/overlay/propose", post(overlay::propose_overlay))
        .route("/v1/overlay/accept", post(overlay::accept_overlay))
        .route(
            "/v1/overlay/mark_implemented",
            post(overlay::mark_implemented),
        )
        .route("/v1/overlay/abandon", post(overlay::abandon_overlay))
        .route(
            "/v1/overlay/report_conflict",
            post(overlay::report_conflict),
        )
        .route("/v1/overlay/consolidate", post(overlay::consolidate))
        .route("/v1/overlays", get(overlay::list_overlays))
        .route("/v1/conflicts", post(overlay::record_conflict))
        .route("/v1/runs/writeback", post(runs::writeback_run))
        .route("/v1/tickets/history", get(tickets::ticket_history))
}

async fn legacy_api_version_middleware(
    matched_path: Option<MatchedPath>,
    req: axum::http::Request<axum::body::Body>,
    next: Next,
) -> Response {
    let mark_deprecated = std::env::var("MEMFUSE_API_VERSION_HEADER")
        .ok()
        .map(|value| value != "false" && value != "0")
        .unwrap_or(true)
        && matched_path
            .as_ref()
            .map(|path| is_legacy_api_path(path.as_str()))
            .unwrap_or(false);

    let mut response = next.run(req).await;
    if mark_deprecated {
        response
            .headers_mut()
            .insert("Deprecation", HeaderValue::from_static("true"));
        response
            .headers_mut()
            .insert("Sunset", HeaderValue::from_static("2027-01-01"));
    }
    response
}

fn is_legacy_api_path(path: &str) -> bool {
    !matches!(path, "/health" | "/ready" | "/metrics")
        && !path.starts_with("/docs")
        && !path.starts_with("/v1/")
}

/// Axum middleware that records Prometheus metrics and emits a tracing span
/// for every HTTP request.
///
/// Observability endpoints (`/health`, `/ready`, `/metrics`) are excluded to
/// avoid self-observation noise.
///
/// When `MatchedPath` is absent (i.e. the request did not match any registered
/// route), the path label is set to the constant `"<unmatched>"` rather than
/// the raw URI.  Using the raw URI would create one time-series per unique
/// path seen by the server — bots and scanners alone can generate thousands of
/// distinct paths, causing unbounded cardinality growth that bloats memory and
/// can break `/metrics`.
async fn metrics_middleware(
    matched_path: Option<MatchedPath>,
    req: axum::http::Request<axum::body::Body>,
    next: Next,
) -> Response {
    let method = req.method().to_string();
    let path = matched_path
        .map(|mp| mp.as_str().to_owned())
        .unwrap_or_else(|| "<unmatched>".to_owned());

    if matches!(path.as_str(), "/health" | "/ready" | "/metrics") {
        return next.run(req).await;
    }

    let span = tracing::info_span!(
        "http_request",
        method = %method,
        path = %path,
    );
    let _enter = span.enter();

    crate::metrics::inflight_request_enter(&method);
    let start = std::time::Instant::now();
    let response = next.run(req).await;
    let duration = start.elapsed().as_secs_f64();
    let status = response.status().as_u16();
    crate::metrics::inflight_request_exit(&method);
    crate::metrics::record_http_request(&method, &path, status, duration);

    tracing::debug!(
        status = status,
        duration_ms = (duration * 1000.0) as u64,
        "request complete"
    );

    response
}

async fn rate_limit_middleware(
    State(state): State<Arc<AppState>>,
    matched_path: Option<MatchedPath>,
    req: axum::http::Request<axum::body::Body>,
    next: Next,
) -> Response {
    let path = matched_path
        .map(|mp| mp.as_str().to_owned())
        .unwrap_or_else(|| req.uri().path().to_owned());

    if matches!(path.as_str(), "/health" | "/ready" | "/metrics") {
        return next.run(req).await;
    }

    let key = format!("{} {}", req.method(), path);
    match state.rate_limiter.check(&key) {
        Ok(()) => next.run(req).await,
        Err(retry_after) => {
            api_types::ApiErrorResponse::rate_limited("rate limit exceeded", retry_after)
                .into_response()
        }
    }
}

async fn payload_too_large_middleware(
    req: axum::http::Request<axum::body::Body>,
    next: Next,
) -> Response {
    let response = next.run(req).await;
    if response.status() == StatusCode::PAYLOAD_TOO_LARGE {
        return api_types::ApiErrorResponse::payload_too_large("request body too large")
            .into_response();
    }
    response
}

/// Build a CORS layer from environment variables.
///
/// - `MEMFUSE_CORS_ENABLED` (default `true`): controls whether CORS headers are added.
/// - `MEMFUSE_CORS_ORIGINS` (default `*` in dev mode): comma-separated list of allowed origins.
///   When empty or unset, all origins are allowed (suitable for dev/localhost).
///   When set, only those specific origins are allowed.
fn cors_layer_from_env() -> CorsLayer {
    let enabled = std::env::var("MEMFUSE_CORS_ENABLED")
        .ok()
        .map(|v| v != "false" && v != "0")
        .unwrap_or(true);

    if !enabled {
        // Return a no-op layer that doesn't add any CORS headers.
        return CorsLayer::new()
            .allow_origin([])
            .allow_methods([])
            .allow_headers([]);
    }

    let origins_str = std::env::var("MEMFUSE_CORS_ORIGINS").ok();

    match origins_str {
        Some(s) if !s.is_empty() && s != "*" => {
            // Specific origins listed — parse each one.
            let origins: Vec<_> = s.split(',').filter_map(|o| o.trim().parse().ok()).collect();
            if origins.is_empty() {
                CorsLayer::new()
                    .allow_origin(Any)
                    .allow_methods(Any)
                    .allow_headers(Any)
            } else {
                CorsLayer::new()
                    .allow_origin(origins)
                    .allow_methods(Any)
                    .allow_headers(Any)
            }
        }
        _ => {
            // Dev mode: allow all origins.
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any)
        }
    }
}

/// Get the max request body size from `MEMFUSE_MAX_BODY_SIZE_MB` (default 10MB).
fn max_body_size_bytes() -> usize {
    let mb = std::env::var("MEMFUSE_MAX_BODY_SIZE_MB")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(10);
    mb * 1024 * 1024
}

pub fn app_with_state(state: Arc<AppState>) -> Router {
    app_with_state_and_body_limit(state, max_body_size_bytes())
}

fn app_with_state_and_body_limit(state: Arc<AppState>, body_limit: usize) -> Router {
    let cors_enabled = std::env::var("MEMFUSE_CORS_ENABLED")
        .ok()
        .map(|v| v != "false" && v != "0")
        .unwrap_or(true);

    let router = api_router()
        .with_state(state.clone())
        .layer(middleware::from_fn_with_state(
            state.clone(),
            crate::auth::auth_middleware,
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            rate_limit_middleware,
        ))
        .layer(axum::extract::DefaultBodyLimit::max(body_limit))
        .layer(middleware::from_fn(payload_too_large_middleware));

    let router = if cors_enabled {
        router.layer(cors_layer_from_env())
    } else {
        router
    };

    router.merge(
        utoipa_swagger_ui::SwaggerUi::new("/docs")
            .url("/docs/openapi.json", api_types::ApiDoc::openapi()),
    )
}

pub fn app_with_config(config: AppConfig) -> Router {
    app_with_state_and_body_limit(build_state(config), max_body_size_bytes())
}

pub fn app_with_config_and_body_limit(config: AppConfig, body_limit: usize) -> Router {
    app_with_state_and_body_limit(build_state(config), body_limit)
}

/// Build a router with explicit auth configuration, bypassing env-var reads.
///
/// This is the test-friendly entry point — callers supply auth mode and key
/// directly, so no unsafe env-var mutation is needed in tests.
pub fn app_with_config_and_auth(
    config: AppConfig,
    auth_mode: crate::auth::AuthMode,
    api_key_config: Option<crate::auth::ApiKeyConfig>,
) -> Router {
    let state = build_state_with_auth(config, auth_mode, api_key_config);
    app_with_state_and_body_limit(state, max_body_size_bytes())
}

pub fn app() -> Router {
    app_with_config(AppConfig::from_env().expect("read MEMFUSE_* environment"))
}

// ---------------------------------------------------------------------------
// Shared utility functions (used by multiple sub-modules)
// ---------------------------------------------------------------------------

pub(super) async fn configured_fs(
    config: &AppConfig,
) -> Result<WorkspaceFs, Box<dyn std::error::Error + Send + Sync>> {
    match config.source_kind.as_str() {
        "localfs" | "git" => {
            let source_path = config.source_path.to_str().unwrap_or("");
            if source_path.is_empty() {
                return Err(std::io::Error::other(
                    "MEMFUSE_SOURCE_PATH is required when MEMFUSE_SOURCE_KIND is 'localfs' or 'git'",
                )
                .into());
            }
        }
        _ => {}
    }

    Ok(match config.source_kind.as_str() {
        "localfs" => {
            WorkspaceFs::from_localfs_source(
                &config.workspace_root,
                &config.account_id,
                &config.user_id,
                &config.agent_id,
                config.source_path.to_str().expect("source path utf-8"),
                &config.target_uri,
            )
            .await?
        }
        "git" => {
            WorkspaceFs::from_git_source(
                &config.workspace_root,
                &config.account_id,
                &config.user_id,
                &config.agent_id,
                config.source_path.to_str().expect("source path utf-8"),
                &config.target_uri,
            )
            .await?
        }
        other => {
            return Err(std::io::Error::other(format!("unsupported source kind '{other}'")).into());
        }
    })
}

pub(super) fn resolve_alias_target(
    metadata: &MetadataStore,
    target: Option<&str>,
) -> Option<String> {
    let target = target?;
    let alias = metadata.get_resource_alias(target).ok()?;
    alias.map(|a| a.canonical_root_uri)
}

pub(super) async fn resolved_fs(
    config: &AppConfig,
    uri: Option<&str>,
) -> Result<WorkspaceFs, Box<dyn std::error::Error + Send + Sync>> {
    if let Some(uri) = uri {
        let scoped_existing = WorkspaceFs::open_existing_for_uri(
            &config.workspace_root,
            &config.account_id,
            &config.user_id,
            &config.agent_id,
            Some(uri),
        )?;
        if let Ok(stat) = scoped_existing.stat(uri).await {
            if stat.is_dir {
                return Ok(scoped_existing);
            }
            if let Some(parent_uri) = parent_mfs_uri(uri) {
                let parent_existing = WorkspaceFs::open_existing_for_uri(
                    &config.workspace_root,
                    &config.account_id,
                    &config.user_id,
                    &config.agent_id,
                    Some(&parent_uri),
                )?;
                if parent_existing.stat(uri).await.is_ok() {
                    return Ok(parent_existing);
                }
            }
            return Ok(scoped_existing);
        }
    }
    let existing = WorkspaceFs::open_existing(
        &config.workspace_root,
        &config.account_id,
        &config.user_id,
        &config.agent_id,
    )?;
    if let Some(uri) = uri {
        if existing.stat(uri).await.is_ok() {
            return Ok(existing);
        }
    }
    configured_fs(config).await
}

fn parent_mfs_uri(uri: &str) -> Option<String> {
    let parsed = mfs_uri::MfsUri::parse(uri).ok()?;
    let path = parsed.canonical_path();
    if path.is_empty() {
        return None;
    }
    let parent_path = path
        .rsplit_once('/')
        .map(|(parent, _)| parent)
        .unwrap_or("");
    if parent_path.is_empty() {
        Some(format!("mfs://{}", parsed.root()))
    } else {
        Some(format!("mfs://{}/{}", parsed.root(), parent_path))
    }
}

pub(super) async fn retrieval_engine_for_fs(
    state: &AppState,
    fs: &WorkspaceFs,
) -> Result<Arc<tokio::sync::Mutex<RetrievalEngine>>, Box<dyn std::error::Error + Send + Sync>> {
    let identity = configured_identity(&state.config);
    let key = RetrievalCacheKey::from_parts(
        &state.config.workspace_root,
        &identity,
        fs.projection_root(),
        fs.projection_uri(),
    );

    if let Some(engine) = state.retrieval_cache.read().await.get(&key).cloned() {
        state.retrieval_cache_hits.fetch_add(1, Ordering::Relaxed);
        return Ok(engine);
    }

    let engine = Arc::new(tokio::sync::Mutex::new(
        RetrievalEngine::from_workspace(
            &state.config.workspace_root,
            &identity,
            fs.projection_root(),
            fs.projection_uri(),
        )
        .await?,
    ));

    let mut cache = state.retrieval_cache.write().await;
    if let Some(existing) = cache.get(&key).cloned() {
        state.retrieval_cache_hits.fetch_add(1, Ordering::Relaxed);
        return Ok(existing);
    }

    cache.insert(key, Arc::clone(&engine));
    state.retrieval_cache_builds.fetch_add(1, Ordering::Relaxed);
    state
        .retrieval_cache_entries
        .fetch_add(1, Ordering::Relaxed);
    Ok(engine)
}

pub(super) async fn invalidate_retrieval_cache(state: &AppState) {
    let mut cache = state.retrieval_cache.write().await;
    if cache.is_empty() {
        return;
    }
    cache.clear();
    state.retrieval_cache_entries.store(0, Ordering::Relaxed);
    state
        .retrieval_cache_invalidations
        .fetch_add(1, Ordering::Relaxed);
}

pub(super) fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

pub(super) fn configured_identity(config: &AppConfig) -> IdentityContext {
    IdentityContext::new(&config.account_id, &config.user_id, &config.agent_id)
}

pub(super) fn resource_projection_view_id(config: &AppConfig) -> String {
    format!("tenant:{}:{}:resources", config.account_id, config.user_id)
}

pub(super) fn render_tree(node: &mfs_workspace::TreeNode, depth: usize) -> String {
    let indent = "  ".repeat(depth);
    let mut output = format!("{indent}{}\n", node.name);
    for child in &node.children {
        output.push_str(&render_tree(child, depth + 1));
    }
    output
}

pub(super) fn append_audit(
    state: &AppState,
    event_type: &str,
    subject_uri: Option<&str>,
    details_json: Option<&str>,
) {
    let projection_view_id = resource_projection_view_id(&state.config);
    let metadata = state.metadata.clone();
    let _ = metadata.append_audit(&AuditEventRecord {
        account_id: &state.config.account_id,
        user_id: &state.config.user_id,
        agent_id: Some(&state.config.agent_id),
        projection_view_id: Some(&projection_view_id),
        event_type,
        subject_uri,
        actor: Some("http"),
        details_json,
    });
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        let metadata = metadata.clone();
        let account_id = state.config.account_id.clone();
        let user_id = state.config.user_id.clone();
        let event_type = event_type.to_owned();
        let subject_uri = subject_uri.map(ToOwned::to_owned);
        let details_json = details_json.map(ToOwned::to_owned);
        handle.spawn(async move {
            webhooks::trigger_event(
                metadata,
                account_id,
                user_id,
                event_type,
                subject_uri,
                details_json,
            )
            .await;
        });
    }
}

pub(super) fn semantic_task_key(operation: &str, identifier: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("semantic:{operation}:{identifier}:{nanos}")
}

pub(super) fn upsert_semantic_task(
    metadata: &MetadataStore,
    config: &AppConfig,
    task_key: &str,
    state: &str,
    owner_space: Option<&str>,
    summary: Option<&str>,
    last_error: Option<&str>,
    attempt_count: u32,
    max_attempts: u32,
    retry_state: &str,
    processing_mode: Option<&str>,
) {
    let _ = metadata.upsert_task(&mfs_metadata::TaskRecord {
        task_key,
        account_id: &config.account_id,
        user_id: &config.user_id,
        agent_id: Some(&config.agent_id),
        projection_view_id: Some(&resource_projection_view_id(config)),
        state,
        owner_space,
        summary,
        last_error,
        attempt_count,
        max_attempts,
        retry_state,
        processing_mode,
    });
}

pub(super) async fn run_resource_task<F, Fut>(
    state: Arc<AppState>,
    task_key: String,
    resource_id: Option<String>,
    event_type: &'static str,
    worker: F,
) where
    F: Fn() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<(Option<String>, Option<String>), String>> + Send,
{
    let max_attempts = 2_u32;
    for attempt in 1..=max_attempts {
        let metadata = state.metadata.clone();
        let retry_state = if attempt < max_attempts {
            "retrying"
        } else {
            "not_needed"
        };
        upsert_semantic_task(
            &metadata,
            &state.config,
            &task_key,
            "running",
            Some("resources"),
            resource_id.as_deref(),
            None,
            attempt,
            max_attempts,
            retry_state,
            None,
        );

        match worker().await {
            Ok((subject_uri, processing_mode)) => {
                let metadata = state.metadata.clone();
                upsert_semantic_task(
                    &metadata,
                    &state.config,
                    &task_key,
                    "completed",
                    Some("resources"),
                    resource_id.as_deref(),
                    None,
                    attempt,
                    max_attempts,
                    "not_needed",
                    processing_mode.as_deref(),
                );
                if let Some(resource_id) = resource_id.as_deref() {
                    update_resource_status(&metadata, resource_id, "ready");
                }
                invalidate_retrieval_cache(&state).await;
                append_audit(
                    &state,
                    event_type,
                    subject_uri.as_deref(),
                    Some("{\"result\":\"ok\"}"),
                );
                return;
            }
            Err(error) => {
                let metadata = state.metadata.clone();
                let retry_state = if attempt < max_attempts {
                    "retryable"
                } else {
                    "exhausted"
                };
                upsert_semantic_task(
                    &metadata,
                    &state.config,
                    &task_key,
                    if attempt < max_attempts {
                        "retrying"
                    } else {
                        "failed"
                    },
                    Some("resources"),
                    resource_id.as_deref(),
                    Some(&error.clone()),
                    attempt,
                    max_attempts,
                    retry_state,
                    None,
                );
                if attempt >= max_attempts {
                    if let Some(resource_id) = resource_id.as_deref() {
                        update_resource_status(&metadata, resource_id, "failed");
                    }
                }
                if attempt < max_attempts {
                    tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                    continue;
                }
                return;
            }
        }
    }
}

pub(super) fn update_resource_status(metadata: &MetadataStore, resource_id: &str, status: &str) {
    let Ok(Some(resource)) = metadata.get_resource_source(resource_id) else {
        return;
    };

    let _ = metadata.register_resource_source(&mfs_metadata::ResourceSourceRecord {
        resource_id: &resource.resource_id,
        account_id: &resource.account_id,
        user_id: &resource.user_id,
        agent_id: resource.agent_id.as_deref(),
        logical_name: &resource.logical_name,
        source_kind: &resource.source_kind,
        source_identifier: &resource.source_identifier,
        canonical_root_uri: &resource.canonical_root_uri,
        projection_view_id: &resource.projection_view_id,
        resource_kind: &resource.resource_kind,
        source_host: resource.source_host.as_deref(),
        source_namespace: resource.source_namespace.as_deref(),
        source_repo: resource.source_repo.as_deref(),
        source_ref: resource.source_ref.as_deref(),
        canonical_strategy_version: &resource.canonical_strategy_version,
        status,
        last_snapshot_id: resource.last_snapshot_id.as_deref(),
    });
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::AppConfig;

    #[test]
    fn app_config_from_env_requires_source_path_for_legacy_source_kinds() {
        let _env = mfs_test_util::env_with_vars(&[
            ("MEMFUSE_WORKSPACE_ROOT", Some("/tmp/workspace")),
            ("MEMFUSE_SOURCE_KIND", Some("localfs")),
            ("MEMFUSE_SOURCE_PATH", None),
            ("MEMFUSE_TARGET_URI", None),
            ("MEMFUSE_ACCOUNT_ID", None),
            ("MEMFUSE_USER_ID", None),
            ("MEMFUSE_AGENT_ID", None),
        ]);

        let error = AppConfig::from_env().unwrap_err();
        assert_eq!(error, std::env::VarError::NotPresent);
    }

    #[test]
    fn app_config_from_env_defaults_to_managed_source_kind() {
        let _env = mfs_test_util::env_with_vars(&[
            ("MEMFUSE_WORKSPACE_ROOT", Some("/tmp/workspace")),
            ("MEMFUSE_SOURCE_KIND", None),
            ("MEMFUSE_SOURCE_PATH", None),
            ("MEMFUSE_TARGET_URI", None),
            ("MEMFUSE_ACCOUNT_ID", None),
            ("MEMFUSE_USER_ID", None),
            ("MEMFUSE_AGENT_ID", None),
        ]);

        let config = AppConfig::from_env().unwrap();
        assert_eq!(config.source_kind, "managed");
        assert!(config.source_path.as_os_str().is_empty());
    }

    #[test]
    fn app_config_from_env_allows_missing_source_path_for_managed_mode() {
        let _env = mfs_test_util::env_with_vars(&[
            ("MEMFUSE_WORKSPACE_ROOT", Some("/tmp/workspace")),
            ("MEMFUSE_SOURCE_KIND", Some("managed")),
            ("MEMFUSE_SOURCE_PATH", None),
            ("MEMFUSE_TARGET_URI", None),
            ("MEMFUSE_ACCOUNT_ID", None),
            ("MEMFUSE_USER_ID", None),
            ("MEMFUSE_AGENT_ID", None),
        ]);

        let config = AppConfig::from_env().unwrap();
        assert_eq!(config.source_kind, "managed");
        assert!(config.source_path.as_os_str().is_empty());
    }

    #[test]
    fn docker_image_defaults_to_managed_source_kind() {
        let dockerfile =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../Dockerfile"))
                .unwrap();

        assert!(
            dockerfile.contains("ENV MEMFUSE_SOURCE_KIND=managed"),
            "Docker image must start without MEMFUSE_SOURCE_PATH"
        );
    }
}
