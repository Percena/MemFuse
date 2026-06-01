//! Unified API response types and OpenAPI schema definitions.
//!
//! All HTTP responses should use these strong-typed structs instead of
//! `serde_json::json!({...})`, ensuring consistent field names, null handling,
//! and automatic OpenAPI spec generation.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use mfs_types::{MfsError, sanitize_secrets};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

// ---------------------------------------------------------------------------
// Search strategy enum (API-level, with OpenAPI schema)
// ---------------------------------------------------------------------------

/// Search strategy for memory context resolution and search.
/// Mirrors `mfs_memory::SearchStrategy` but includes OpenAPI schema generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, Default)]
#[serde(rename_all = "lowercase")]
pub enum SearchStrategy {
    /// Pure relevance sorting — current behavior unchanged. (default)
    #[default]
    Precision,
    /// Relevance + MMR post-processing (lambda=0.7).
    Diverse,
    /// Enhanced recency boost: 24h 2.0×, 7d 1.3×.
    Recent,
    /// Budget ×2 for maximum recall.
    Comprehensive,
}

impl From<SearchStrategy> for mfs_memory::SearchStrategy {
    fn from(s: SearchStrategy) -> Self {
        match s {
            SearchStrategy::Precision => mfs_memory::SearchStrategy::Precision,
            SearchStrategy::Diverse => mfs_memory::SearchStrategy::Diverse,
            SearchStrategy::Recent => mfs_memory::SearchStrategy::Recent,
            SearchStrategy::Comprehensive => mfs_memory::SearchStrategy::Comprehensive,
        }
    }
}

// ---------------------------------------------------------------------------
// Turn role enum (API-level, with OpenAPI schema)
// ---------------------------------------------------------------------------

/// Turn role for message and episode turn records.
/// Mirrors `mfs_memory::TurnRole` but includes OpenAPI schema generation.
/// Serializes as lowercase strings: "user", "assistant", "system", "tool".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, Default)]
#[serde(rename_all = "lowercase")]
pub enum TurnRole {
    #[default]
    User,
    Assistant,
    System,
    Tool,
}

impl TurnRole {
    /// Returns the lowercase string representation of this role.
    pub fn as_str(&self) -> &'static str {
        match self {
            TurnRole::User => "user",
            TurnRole::Assistant => "assistant",
            TurnRole::System => "system",
            TurnRole::Tool => "tool",
        }
    }
}

impl From<TurnRole> for mfs_memory::TurnRole {
    fn from(r: TurnRole) -> Self {
        match r {
            TurnRole::User => mfs_memory::TurnRole::User,
            TurnRole::Assistant => mfs_memory::TurnRole::Assistant,
            TurnRole::System => mfs_memory::TurnRole::System,
            TurnRole::Tool => mfs_memory::TurnRole::Tool,
        }
    }
}

// ---------------------------------------------------------------------------
// Error responses — unified nested format
// ---------------------------------------------------------------------------

/// Structured error detail used in all API error responses.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ApiError {
    /// Error category — machine-readable string (e.g. "NotFound", "InvalidArgument").
    pub category: String,
    /// Human-readable error message (secrets sanitized).
    pub message: String,
    /// Whether the client can retry the request and potentially succeed.
    pub retryable: bool,
}

/// Unified error response envelope used by all API endpoints.
///
/// Both auth failures and application errors return this format:
/// ```json
/// { "error": { "category": "NotFound", "message": "...", "retryable": false } }
/// ```
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ApiErrorResponse {
    pub error: ApiError,
}

impl ApiErrorResponse {
    /// Construct from an MfsError (the system's canonical error enum).
    pub fn from_mfs_error(err: MfsError) -> Self {
        let sanitized_message = sanitize_secrets(&err.to_string());
        Self {
            error: ApiError {
                category: err.category().to_string(),
                message: sanitized_message,
                retryable: err.retryable(),
            },
        }
    }

    /// Construct an Unauthorized error (used by auth middleware).
    pub fn unauthorized(message: &str) -> Self {
        Self {
            error: ApiError {
                category: "Unauthorized".to_string(),
                message: message.to_string(),
                retryable: false,
            },
        }
    }

    /// Construct a rate-limited error (429).
    pub fn rate_limited(message: &str, retry_after_secs: u64) -> RateLimitedResponse {
        RateLimitedResponse {
            body: Self {
                error: ApiError {
                    category: "RateLimited".to_string(),
                    message: message.to_string(),
                    retryable: true,
                },
            },
            retry_after_secs,
        }
    }

    /// Construct a payload-too-large error (413).
    pub fn payload_too_large(message: &str) -> Self {
        Self {
            error: ApiError {
                category: "PayloadTooLarge".to_string(),
                message: message.to_string(),
                retryable: false,
            },
        }
    }
}

impl IntoResponse for ApiErrorResponse {
    fn into_response(self) -> Response {
        let status = match self.error.category.as_str() {
            "NotFound" => StatusCode::NOT_FOUND,
            "PermissionDenied" => StatusCode::FORBIDDEN,
            "Conflict" => StatusCode::CONFLICT,
            "InvalidArgument" => StatusCode::BAD_REQUEST,
            "Unavailable" => StatusCode::SERVICE_UNAVAILABLE,
            "FailedPrecondition" => StatusCode::PRECONDITION_FAILED,
            "Internal" => StatusCode::INTERNAL_SERVER_ERROR,
            "Unauthorized" => StatusCode::UNAUTHORIZED,
            "RateLimited" => StatusCode::TOO_MANY_REQUESTS,
            "PayloadTooLarge" => StatusCode::PAYLOAD_TOO_LARGE,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, axum::Json(self)).into_response()
    }
}

/// Rate-limited response with Retry-After header.
pub struct RateLimitedResponse {
    pub body: ApiErrorResponse,
    pub retry_after_secs: u64,
}

impl IntoResponse for RateLimitedResponse {
    fn into_response(self) -> Response {
        let mut response = axum::Json(self.body).into_response();
        *response.status_mut() = StatusCode::TOO_MANY_REQUESTS;
        response.headers_mut().insert(
            "Retry-After",
            axum::http::HeaderValue::from(self.retry_after_secs),
        );
        response
    }
}

// ---------------------------------------------------------------------------
// Success responses — typed envelopes
// ---------------------------------------------------------------------------

#[allow(dead_code)]
/// Generic success response envelope for single-resource endpoints.
#[derive(Debug, Serialize, ToSchema)]
pub struct ApiResponse<T: ToSchema> {
    pub data: T,
}

#[allow(dead_code)]
/// Paginated list response with cursor-based pagination.
#[derive(Debug, Serialize, ToSchema)]
pub struct PaginatedResponse<T: ToSchema> {
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
    #[schema(value_type = u64)]
    pub total_count: usize,
}

// ---------------------------------------------------------------------------
// Session response types (first batch)
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, ToSchema)]
pub struct SessionCreateResponse {
    pub session_id: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CreateSessionRequest {
    pub session_id: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SessionSummary {
    pub session_id: String,
    pub account_id: String,
    pub user_id: String,
    pub agent_id: String,
    pub message_count: usize,
    pub commit_count: u32,
    pub last_commit_archive_id: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SessionListResponse {
    pub items: Vec<SessionSummary>,
    pub sessions: Vec<SessionSummary>,
    pub next_cursor: Option<String>,
    #[schema(value_type = u64)]
    pub total_count: usize,
    #[schema(value_type = u64)]
    pub limit: usize,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct AddMessageRequest {
    pub role: TurnRole,
    pub content: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AddMessageResponse {
    pub ok: bool,
    pub session_id: String,
    pub auto_committed: bool,
    pub archive_uri: Option<String>,
    pub task_id: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CommitSessionRequest {
    pub user_id: Option<String>,
    pub thread_id: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CommitSessionResponse {
    pub archive_uri: String,
    pub task_id: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DeleteSessionResponse {
    pub deleted: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CreateFactRequest {
    pub id: String,
    pub subject: String,
    pub predicate: String,
    pub display_value: String,
    pub confidence: Option<f64>,
    pub agent_id: Option<String>,
    pub value_type: Option<String>,
    pub source_assertion_id: Option<String>,
    pub source_episode_ids_json: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct FactRecord {
    pub id: String,
    pub account_id: String,
    pub user_id: String,
    pub agent_id: Option<String>,
    pub subject: String,
    pub predicate: String,
    pub display_value: String,
    pub normalized_value_json: Option<String>,
    pub value_type: String,
    pub confidence: f64,
    pub status: String,
    pub valid_from: Option<String>,
    pub valid_to: Option<String>,
    pub source_assertion_id: Option<String>,
    pub source_episode_ids_json: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub superseded_at: Option<String>,
    pub superseded_by: Option<String>,
    pub recall_count: u32,
    pub last_recalled_at: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct FactListResponse {
    pub items: Vec<FactRecord>,
    pub facts: Vec<FactRecord>,
    pub next_cursor: Option<String>,
    #[schema(value_type = u64)]
    pub total_count: usize,
    #[schema(value_type = u64)]
    pub count: usize,
    #[schema(value_type = u64)]
    pub limit: usize,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CreateFactResponse {
    pub ok: bool,
    pub id: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SupersedeFactRequest {
    pub new_fact_id: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SupersedeFactResponse {
    pub ok: bool,
    pub superseded: String,
    pub superseded_by: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RetractFactResponse {
    pub ok: bool,
    pub retracted: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TraceFactItem {
    pub id: String,
    pub subject: String,
    pub predicate: String,
    pub display_value: String,
    pub confidence: f64,
    pub status: String,
    pub valid_from: Option<String>,
    pub valid_to: Option<String>,
    pub source_episode_ids_json: Option<String>,
    pub source_assertion_id: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TraceSourceEpisode {
    pub episode_id: String,
    pub session_id: String,
    pub summary: String,
    pub created_at: String,
    pub salience_score: f64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TraceSourceAssertion {
    pub assertion_id: String,
    pub subject: String,
    pub predicate: String,
    pub operation: String,
    pub confidence: f64,
    pub created_at: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TraceFactResponse {
    pub fact: TraceFactItem,
    pub source_episodes: Vec<TraceSourceEpisode>,
    pub source_assertions: Vec<TraceSourceAssertion>,
    #[schema(value_type = u64)]
    pub assertion_count: usize,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CreateWebhookRequest {
    pub event_type: String,
    pub callback_url: String,
    pub secret: String,
    pub enabled: Option<bool>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct WebhookRecord {
    pub id: String,
    pub account_id: String,
    pub user_id: String,
    pub agent_id: Option<String>,
    pub event_type: String,
    pub callback_url: String,
    pub enabled: bool,
    pub created_at: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct WebhookListResponse {
    pub items: Vec<WebhookRecord>,
    pub webhooks: Vec<WebhookRecord>,
    pub next_cursor: Option<String>,
    #[schema(value_type = u64)]
    pub total_count: usize,
    #[schema(value_type = u64)]
    pub count: usize,
    #[schema(value_type = u64)]
    pub limit: usize,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DeleteWebhookResponse {
    pub deleted: bool,
    pub id: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TestWebhookResponse {
    pub ok: bool,
    pub id: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ContextResolveResponse {
    pub formatted_context: String,
    pub budget_used: usize,
    pub budget_total: usize,
    pub sections: ContextSections,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ContextSections {
    pub facts_count: usize,
    pub episodes_count: usize,
    pub heuristics_count: usize,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AddObservationRequest {
    pub tool_name: String,
    pub tool_input: String,
    pub tool_output: String,
    pub content: String,
    pub platform: Option<String>,
    pub source_trust: Option<String>,
    pub metadata: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AddObservationResponse {
    pub ok: bool,
    pub session_id: String,
    pub turn_id: String,
    pub auto_committed: bool,
    pub archive_uri: Option<String>,
    pub task_id: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ResolveMemoryContextRequest {
    pub query: String,
    pub session_id: Option<String>,
    #[schema(value_type = u64)]
    pub token_budget: Option<usize>,
    pub user_id: Option<String>,
    pub resource_id: Option<String>,
    pub strategy: Option<SearchStrategy>,
    pub at_time: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MemorySearchRequest {
    pub query: String,
    #[schema(value_type = u64)]
    pub top_k: Option<usize>,
    #[schema(value_type = u64)]
    pub limit: Option<usize>,
    pub user_id: Option<String>,
    pub session_id: Option<String>,
    pub thread_id: Option<String>,
    pub strategy: Option<SearchStrategy>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MemorySearchHit {
    pub episode_id: String,
    pub session_id: String,
    pub summary: String,
    pub salience_score: f64,
    pub score: f64,
    pub created_at: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MemorySearchResponse {
    pub results: Vec<MemorySearchHit>,
    pub query: String,
    #[schema(value_type = u64)]
    pub total: usize,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct LinkRelationRequest {
    pub from_uri: String,
    pub to_uri: String,
    pub relation_type: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RelationLinkResponse {
    pub ok: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RelationRecord {
    pub relation_type: String,
    pub direction: String,
    pub peer_uri: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RelationListResponse {
    pub items: Vec<RelationRecord>,
    pub relations: Vec<RelationRecord>,
    pub next_cursor: Option<String>,
    #[schema(value_type = u64)]
    pub total_count: usize,
    #[schema(value_type = u64)]
    pub count: usize,
    #[schema(value_type = u64)]
    pub limit: usize,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CreateResourceRequest {
    pub source_kind: Option<String>,
    pub source_path: Option<String>,
    pub logical_name: Option<String>,
    pub branch: Option<String>,
    pub revision: Option<String>,
    pub repo_id: Option<String>,
    pub tracker: Option<String>,
    pub tracker_project_identifier: Option<String>,
    pub file_name: Option<String>,
    pub content: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CreateResourceResponse {
    pub task_key: String,
    pub resource_id: String,
    pub logical_name: String,
    pub root_uri: String,
    pub repo_id: Option<String>,
    pub tracker: Option<String>,
    pub tracker_project_identifier: Option<String>,
    pub state: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ResourceRecord {
    pub resource_id: String,
    pub logical_name: String,
    pub source_kind: String,
    pub source_identifier: String,
    pub root_uri: String,
    pub source_host: Option<String>,
    pub source_namespace: Option<String>,
    pub source_repo: Option<String>,
    pub source_ref: Option<String>,
    pub repo_id: Option<String>,
    pub tracker: Option<String>,
    pub tracker_project_identifier: Option<String>,
    pub status: String,
    pub last_snapshot_id: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ResourceListResponse {
    pub items: Vec<ResourceRecord>,
    pub resources: Vec<ResourceRecord>,
    pub next_cursor: Option<String>,
    #[schema(value_type = u64)]
    pub total_count: usize,
    #[schema(value_type = u64)]
    pub limit: usize,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CreateResourcesBatchRequest {
    pub resources: Vec<CreateResourceRequest>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ResourceBatchItem {
    #[schema(value_type = u64)]
    pub index: usize,
    pub task_key: Option<String>,
    pub resource_id: Option<String>,
    pub logical_name: Option<String>,
    pub root_uri: Option<String>,
    pub state: String,
    pub error: Option<ApiError>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ResourceBatchResponse {
    pub results: Vec<ResourceBatchItem>,
    #[schema(value_type = u64)]
    pub count: usize,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ResourceImportRequest {
    pub pack_path: String,
    pub logical_name: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ResourceExportRequest {
    pub output_path: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ResourceExportResponse {
    pub output_path: String,
    pub logical_name: String,
    pub root_uri: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ResourceTaskResponse {
    pub task_key: String,
    pub resource_id: String,
    pub root_uri: String,
    pub state: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct WorkspaceUriRequest {
    pub uri: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct WorkspaceWriteRequest {
    pub uri: String,
    pub content: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct WorkspaceMoveRequest {
    pub from_uri: String,
    pub to_uri: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct WorkspaceDirEntry {
    pub name: String,
    pub is_dir: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct WorkspaceStatResponse {
    pub path: String,
    pub is_dir: bool,
    #[schema(value_type = u64)]
    pub size_bytes: u64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct WorkspaceMutationResponse {
    pub uri: String,
    pub indexed_paths: Vec<String>,
    pub scopes_reindexed: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct WorkspaceSearchResponse {
    pub query_plan: String,
    pub typed_queries: Vec<String>,
    pub trajectory: String,
    pub resources: Vec<String>,
    pub memories: Vec<String>,
    pub skills: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct WorkspaceRebuildResponse {
    pub indexed_paths: Vec<String>,
    pub projection_uri: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct WorkspaceRefreshResponse {
    pub snapshot_id: String,
    pub indexed_paths: Vec<String>,
    pub projection_uri: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ResourceWatchRequest {
    pub interval_seconds: u32,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ResourceWatchLoopRequest {
    #[schema(value_type = u64)]
    pub iterations: usize,
    pub sleep_ms: u64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct WatchServiceRequest {
    pub poll_ms: u64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ResourceWatchRecord {
    pub account_id: Option<String>,
    pub user_id: Option<String>,
    pub agent_id: Option<String>,
    pub resource_id: String,
    pub interval_seconds: u32,
    pub enabled: bool,
    pub due: Option<bool>,
    pub last_checked_at: Option<String>,
    pub last_refreshed_at: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ResourceWatchRunResponse {
    pub resource_id: String,
    pub refreshed: bool,
    pub root_uri: String,
    pub snapshot_id: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct WatchRunDueResponse {
    pub runs: Vec<ResourceWatchRunResponse>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct WatchLoopResponse {
    #[schema(value_type = u64)]
    pub iterations: usize,
    #[schema(value_type = u64)]
    pub total_runs: usize,
    pub runs: Vec<ResourceWatchRunResponse>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct WatchListResponse {
    pub items: Vec<ResourceWatchRecord>,
    pub watches: Vec<ResourceWatchRecord>,
    pub next_cursor: Option<String>,
    #[schema(value_type = u64)]
    pub total_count: usize,
    #[schema(value_type = u64)]
    pub count: usize,
    #[schema(value_type = u64)]
    pub limit: usize,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct WatchServiceStatusResponse {
    pub running: bool,
    pub poll_ms: u64,
    pub started_at_ms: Option<u64>,
    pub stopped_at_ms: Option<u64>,
    pub last_tick_at_ms: Option<u64>,
    pub total_ticks: u64,
    pub total_runs: u64,
    pub last_run_count: u64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct HeuristicRuleRequest {
    pub rule_text: String,
    pub tags: Vec<String>,
    pub counter_examples: Option<Vec<String>>,
    pub lifecycle_stage: Option<String>,
    pub user_id: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct HeuristicRuleResponse {
    pub rule_id: String,
    pub lifecycle_stage: String,
}

/// Heuristic rule record — mirrors mfs_memory::heuristics::HeuristicEntry
/// with OpenAPI schema generation. Use `From<HeuristicEntry>` for conversion.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct HeuristicRuleRecord {
    pub rule_id: String,
    pub rule_text: String,
    pub tags: Vec<String>,
    pub counter_examples: Vec<String>,
    pub lifecycle_stage: String,
    pub evidence_count: i64,
    pub aggregate_weight: f64,
    pub user_confirmed: bool,
    pub created_at: Option<String>,
}

impl From<mfs_memory::heuristics::HeuristicEntry> for HeuristicRuleRecord {
    fn from(e: mfs_memory::heuristics::HeuristicEntry) -> Self {
        Self {
            rule_id: e.rule_id,
            rule_text: e.rule_text,
            tags: e.tags,
            counter_examples: e.counter_examples,
            lifecycle_stage: e.lifecycle_stage,
            evidence_count: e.evidence_count,
            aggregate_weight: e.aggregate_weight,
            user_confirmed: e.user_confirmed,
            created_at: e.created_at,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct HeuristicRuleListResponse {
    pub items: Vec<HeuristicRuleRecord>,
    pub rules: Vec<HeuristicRuleRecord>,
    pub next_cursor: Option<String>,
    #[schema(value_type = u64)]
    pub total_count: usize,
    #[schema(value_type = u64)]
    pub total: usize,
    #[schema(value_type = u64)]
    pub limit: usize,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PromoteRuleRequest {
    pub new_stage: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PromoteRuleResponse {
    pub rule_id: String,
    pub new_stage: String,
    pub status: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ConfirmRuleResponse {
    pub rule_id: String,
    pub user_confirmed: bool,
    pub status: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct HeuristicInstanceRequest {
    pub context_summary: String,
    pub user_reaction: String,
    pub signal_type: String,
    pub tags: Option<Vec<String>>,
    pub agent_proposal: Option<String>,
    pub outcome: Option<String>,
    pub session_id: Option<String>,
    pub user_id: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct HeuristicInstanceResponse {
    pub instance_id: String,
    pub signal_type: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct HeuristicInstanceRecord {
    pub instance_id: String,
    pub context_summary: String,
    pub agent_proposal: Option<String>,
    pub user_reaction: String,
    pub outcome: Option<String>,
    pub signal_type: String,
    pub tags: Vec<String>,
    pub session_id: Option<String>,
    pub instance_status: String,
    pub derived_rule_id: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct HeuristicInstanceListResponse {
    pub items: Vec<HeuristicInstanceRecord>,
    pub instances: Vec<HeuristicInstanceRecord>,
    pub next_cursor: Option<String>,
    #[schema(value_type = u64)]
    pub total_count: usize,
    #[schema(value_type = u64)]
    pub total: usize,
    #[schema(value_type = u64)]
    pub limit: usize,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RetrieveHeuristicsRequest {
    pub query: String,
    pub tags: Option<Vec<String>>,
    #[schema(value_type = u64)]
    pub top_k: Option<usize>,
    pub user_id: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RetrieveHeuristicsResponse {
    pub heuristics: Vec<HeuristicRuleRecord>,
    #[schema(value_type = u64)]
    pub total: usize,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct L0ConfirmedRequest {
    #[schema(value_type = u64)]
    pub max_rules: Option<usize>,
    pub user_id: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SimulateReactionRequest {
    pub scenario: String,
    pub tags: Option<Vec<String>>,
    pub user_id: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SimulateReactionResponse {
    pub scenario: String,
    pub relevant_rules: Vec<HeuristicRuleRecord>,
    pub rules_summary: String,
    pub prediction: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct EpisodeTurnRecord {
    pub turn_id: String,
    pub turn_seq: i64,
    pub role: TurnRole,
    pub content: String,
    pub created_at: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct EpisodeDetailResponse {
    pub episode_id: String,
    pub session_id: String,
    pub resource_id: Option<String>,
    pub summary: String,
    pub salience_score: f64,
    pub strength_score: f64,
    pub emotional_valence: Option<f64>,
    pub emotional_intensity: Option<f64>,
    pub context_tags_json: Option<String>,
    pub created_at: String,
    pub facts: Vec<FactRecord>,
    pub turns: Vec<EpisodeTurnRecord>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct EpisodeTimelineItem {
    pub episode_id: String,
    pub session_id: String,
    pub summary: String,
    pub salience_score: f64,
    pub emotional_valence: Option<f64>,
    pub emotional_intensity: Option<f64>,
    pub created_at: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct EpisodeTimelineResponse {
    pub anchor_episode_id: String,
    pub direction: String,
    #[schema(value_type = u64)]
    pub radius: usize,
    pub episodes: Vec<EpisodeTimelineItem>,
    #[schema(value_type = u64)]
    pub count: usize,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CiteMemoriesRequest {
    pub episode_ids: Option<Vec<String>>,
    pub fact_ids: Option<Vec<String>>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CiteMemoriesResponse {
    pub cited_episodes: u32,
    pub cited_facts: u32,
    pub warning: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MemoryExportResponse {
    pub markdown: String,
    #[schema(value_type = u64)]
    pub fact_count: usize,
    #[schema(value_type = u64)]
    pub rule_count: usize,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MemoryImportRequest {
    pub markdown: String,
    pub user_id: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MemoryImportResponse {
    pub updated_facts: u32,
    pub retracted_facts: u32,
    pub skipped_rows: u32,
    pub errors: u32,
    #[schema(value_type = u64)]
    pub total_imported: usize,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MemoryConsolidateRequest {
    pub session_id: String,
    pub user_id: String,
    pub resource_id: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MemoryConsolidateResponse {
    pub session_id: String,
    pub user_id: String,
    pub status: String,
    pub range_start_turn_id: Option<String>,
    pub range_end_turn_id: Option<String>,
    #[schema(value_type = u64)]
    pub episode_count: usize,
    #[schema(value_type = u64)]
    pub assertion_count: usize,
    #[schema(value_type = u64)]
    pub fact_count: usize,
    #[schema(value_type = u64)]
    pub turn_count: usize,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MemoryExtractFactsRequest {
    pub texts: Vec<String>,
    pub user_id: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MemoryFactAssertion {
    pub subject: String,
    pub predicate: String,
    pub value: String,
    pub confidence: f64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MemoryExtractFactsResponse {
    pub assertions: Vec<MemoryFactAssertion>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MemoryArchiveRequest {
    pub hotness_threshold: Option<f64>,
    pub min_age_days: Option<f64>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MemoryArchiveResponse {
    #[schema(value_type = u64)]
    pub archived_episodes: usize,
    pub user_id: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct EvalRecallRequest {
    pub query: String,
    pub expected_facts: Vec<String>,
    #[schema(value_type = u64)]
    pub k: Option<usize>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct EvalRecallResponse {
    pub recall_at_k: f64,
    #[schema(value_type = u64)]
    pub retrieved_count: usize,
    #[schema(value_type = u64)]
    pub expected_count: usize,
    #[schema(value_type = u64)]
    pub matched_count: usize,
    pub missing_facts: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CreateCodeSymbolsRequest {
    pub symbols: Vec<CreateCodeSymbolItem>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CreateCodeSymbolItem {
    pub id: String,
    pub projection_view_id: String,
    pub canonical_uri: String,
    pub symbol_type: String,
    pub symbol_name: String,
    pub signature: Option<String>,
    pub docstring: Option<String>,
    pub line_number: Option<i64>,
    pub agent_id: Option<String>,
    pub embedding_json: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CodeSymbolsCreateResponse {
    pub ok: bool,
    #[schema(value_type = u64)]
    pub count: usize,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CodeSymbolRecord {
    pub id: String,
    pub account_id: String,
    pub user_id: String,
    pub agent_id: Option<String>,
    pub projection_view_id: String,
    pub canonical_uri: String,
    pub symbol_type: String,
    pub symbol_name: String,
    pub signature: Option<String>,
    pub docstring: Option<String>,
    pub line_number: Option<i64>,
    pub embedding_json: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CodeSymbolListResponse {
    pub items: Vec<CodeSymbolRecord>,
    pub symbols: Vec<CodeSymbolRecord>,
    pub next_cursor: Option<String>,
    #[schema(value_type = u64)]
    pub total_count: usize,
    #[schema(value_type = u64)]
    pub count: usize,
    #[schema(value_type = u64)]
    pub limit: usize,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CodeSymbolSearchResponse {
    pub items: Vec<CodeSymbolRecord>,
    pub symbols: Vec<CodeSymbolRecord>,
    pub results: Vec<CodeSymbolRecord>,
    pub next_cursor: Option<String>,
    #[schema(value_type = u64)]
    pub total_count: usize,
    #[schema(value_type = u64)]
    pub count: usize,
    #[schema(value_type = u64)]
    pub limit: usize,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DeleteCodeSymbolsResponse {
    pub ok: bool,
    #[schema(value_type = u64)]
    pub deleted: usize,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SnapshotRecord {
    pub snapshot_id: String,
    pub account_id: String,
    pub user_id: String,
    pub agent_id: Option<String>,
    pub projection_view_id: String,
    pub root_uri: String,
    pub manifest_digest: Option<String>,
    pub created_by: Option<String>,
    pub notes: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SnapshotListResponse {
    pub items: Vec<SnapshotRecord>,
    pub snapshots: Vec<SnapshotRecord>,
    pub next_cursor: Option<String>,
    #[schema(value_type = u64)]
    pub total_count: usize,
    #[schema(value_type = u64)]
    pub limit: usize,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AuditRecord {
    pub id: i64,
    pub account_id: String,
    pub user_id: String,
    pub agent_id: Option<String>,
    pub projection_view_id: Option<String>,
    pub event_type: String,
    pub subject_uri: Option<String>,
    pub actor: Option<String>,
    pub details_json: Option<String>,
    pub recorded_at: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AuditListResponse {
    pub items: Vec<AuditRecord>,
    pub audit: Vec<AuditRecord>,
    pub entries: Vec<AuditRecord>,
    pub next_cursor: Option<String>,
    #[schema(value_type = u64)]
    pub total_count: usize,
    #[schema(value_type = u64)]
    pub limit: usize,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AddSkillRequest {
    pub path: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AddSkillResponse {
    pub skill_name: String,
    pub skill_uri: String,
    pub indexed_documents: Vec<String>,
    pub mode: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SkillRecord {
    pub skill_name: String,
    pub skill_uri: String,
    pub path: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SkillListResponse {
    pub items: Vec<SkillRecord>,
    pub skills: Vec<SkillRecord>,
    pub next_cursor: Option<String>,
    #[schema(value_type = u64)]
    pub total_count: usize,
    #[schema(value_type = u64)]
    pub count: usize,
    #[schema(value_type = u64)]
    pub limit: usize,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RuntimeCacheStatus {
    #[schema(value_type = u64)]
    pub entries: usize,
    pub builds: u64,
    pub hits: u64,
    pub invalidations: u64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SystemRuntimeStatus {
    pub status: String,
    pub retrieval_cache: RuntimeCacheStatus,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SystemStatusResponse {
    pub runtime: SystemRuntimeStatus,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ObserverStatusResponse {
    pub runtime: SystemRuntimeStatus,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TaskRecord {
    pub kind: String,
    pub task_id: String,
    pub status: String,
    pub summary: Option<String>,
    pub archive_uri: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TaskListResponse {
    pub items: Vec<TaskRecord>,
    pub tasks: Vec<TaskRecord>,
    pub next_cursor: Option<String>,
    #[schema(value_type = u64)]
    pub total_count: usize,
    #[schema(value_type = u64)]
    pub limit: usize,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TaskEvictResponse {
    #[schema(value_type = u64)]
    pub ttl_evicted: usize,
    #[schema(value_type = u64)]
    pub fifo_evicted: usize,
    pub completed_ttl_hours: u32,
    pub failed_ttl_hours: u32,
    #[schema(value_type = u64)]
    pub max_tasks: usize,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TaskStatusResponse {
    pub task_key: Option<String>,
    pub task_id: Option<String>,
    pub state: Option<String>,
    pub status: Option<String>,
    pub summary: Option<String>,
    pub last_error: Option<String>,
    pub attempt_count: Option<u32>,
    pub max_attempts: Option<u32>,
    pub retry_state: Option<String>,
    pub processing_mode: Option<String>,
    pub archive_uri: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct WaitTaskResponse {
    pub task_key: Option<String>,
    pub task_id: Option<String>,
    pub state: Option<String>,
    pub status: Option<String>,
    pub summary: Option<String>,
    pub last_error: Option<String>,
    pub archive_uri: Option<String>,
}

// ---------------------------------------------------------------------------
// OpenAPI document
// ---------------------------------------------------------------------------

use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    paths(
        openapi_health,
        openapi_ready,
        openapi_create_session,
        openapi_list_sessions,
        openapi_get_session,
        openapi_delete_session,
        openapi_add_message,
        openapi_commit_session,
        openapi_add_observation,
        openapi_resolve_context,
        openapi_search_memories,
        openapi_create_fact,
        openapi_list_facts,
        openapi_supersede_fact,
        openapi_retract_fact,
        openapi_trace_fact,
        openapi_link_relation,
        openapi_list_relations,
        openapi_unlink_relation,
        openapi_create_webhook,
        openapi_list_webhooks,
        openapi_delete_webhook,
        openapi_test_webhook,
        openapi_create_resource,
        openapi_list_resources,
        openapi_create_resources_batch,
        openapi_import_resource,
        openapi_export_resource,
        openapi_refresh_resource,
        openapi_rebuild_resource,
        openapi_workspace_ls,
        openapi_workspace_tree,
        openapi_workspace_stat,
        openapi_workspace_abstract,
        openapi_workspace_overview,
        openapi_workspace_read,
        openapi_workspace_glob,
        openapi_workspace_mkdir,
        openapi_workspace_write,
        openapi_workspace_mv,
        openapi_workspace_rm,
        openapi_workspace_find,
        openapi_workspace_grep,
        openapi_workspace_search,
        openapi_workspace_rebuild,
        openapi_workspace_refresh,
        openapi_list_watches,
        openapi_run_due_watches,
        openapi_run_watch_loop,
        openapi_start_watch_service,
        openapi_watch_service_status,
        openapi_stop_watch_service,
        openapi_register_watch,
        openapi_disable_watch,
        openapi_run_watch,
        openapi_create_heuristic_rule,
        openapi_list_heuristic_rules,
        openapi_get_heuristic_rule,
        openapi_promote_heuristic_rule,
        openapi_confirm_heuristic_rule,
        openapi_create_heuristic_instance,
        openapi_list_heuristic_instances,
        openapi_get_heuristic_instance,
        openapi_retrieve_heuristics,
        openapi_l0_confirmed_rules,
        openapi_simulate_reaction,
        openapi_get_episode,
        openapi_get_episode_timeline,
        openapi_cite_memories,
        openapi_export_memories,
        openapi_import_memories,
        openapi_consolidate_memories,
        openapi_extract_facts,
        openapi_archive_memories,
        openapi_eval_recall,
        openapi_create_code_symbols,
        openapi_list_code_symbols,
        openapi_search_code_symbols,
        openapi_delete_code_symbols,
        openapi_snapshots,
        openapi_audit,
        openapi_add_skill,
        openapi_list_skills,
        openapi_system_status,
        openapi_observer_status,
        openapi_list_tasks,
        openapi_evict_tasks,
        openapi_task_status,
        openapi_wait_task,
    ),
    info(
        title = "MemFuse API",
        version = "0.1.0",
        description = "MemFuse persistent memory system REST API for agents and traditional applications."
    ),
    modifiers(&SecurityAddon),
    tags(
       (name = "sessions", description = "Session lifecycle management"),
        (name = "context", description = "Memory context resolution"),
        (name = "facts", description = "Fact CRUD operations"),
        (name = "memory", description = "Memory search and management"),
        (name = "resources", description = "Resource registration and management"),
        (name = "workspace", description = "Workspace file operations"),
        (name = "watches", description = "Resource watch management"),
        (name = "heuristics", description = "Behavioral heuristic rules"),
        (name = "episodes", description = "Episodic memory details"),
        (name = "code_symbols", description = "Code symbol index management"),
        (name = "tasks", description = "Background task status and control"),
        (name = "skills", description = "Skill ingestion and discovery"),
        (name = "webhooks", description = "Webhook registration and event delivery"),
        (name = "system", description = "System status and health"),
    )
)]
pub struct ApiDoc;

#[allow(dead_code)]
#[utoipa::path(
    get,
    path = "/health",
    tag = "system",
    responses((status = 200, description = "Server health status"))
)]
fn openapi_health() {}

#[allow(dead_code)]
#[utoipa::path(
    get,
    path = "/ready",
    tag = "system",
    responses(
        (status = 200, description = "Server is ready"),
        (status = 503, description = "Server is not ready", body = ApiErrorResponse)
    )
)]
fn openapi_ready() {}

#[allow(dead_code)]
#[utoipa::path(
    post,
    path = "/sessions",
    tag = "sessions",
    request_body = CreateSessionRequest,
    responses(
        (status = 200, description = "Session created", body = SessionCreateResponse),
        (status = 429, description = "Rate limited", body = ApiErrorResponse)
    )
)]
fn openapi_create_session() {}

#[allow(dead_code)]
#[utoipa::path(
    get,
    path = "/sessions",
    tag = "sessions",
    params(("limit" = Option<usize>, Query, description = "Maximum number of sessions")),
    responses((status = 200, description = "List sessions", body = SessionListResponse))
)]
fn openapi_list_sessions() {}

#[allow(dead_code)]
#[utoipa::path(
    get,
    path = "/sessions/{session_id}",
    tag = "sessions",
    params(("session_id" = String, Path, description = "Session ID")),
    responses(
        (status = 200, description = "Get session", body = SessionSummary),
        (status = 404, description = "Session not found", body = ApiErrorResponse)
    )
)]
fn openapi_get_session() {}

#[allow(dead_code)]
#[utoipa::path(
    delete,
    path = "/sessions/{session_id}",
    tag = "sessions",
    params(("session_id" = String, Path, description = "Session ID")),
    responses(
        (status = 200, description = "Delete session", body = DeleteSessionResponse),
        (status = 404, description = "Session not found", body = ApiErrorResponse)
    )
)]
fn openapi_delete_session() {}

#[allow(dead_code)]
#[utoipa::path(
    post,
    path = "/sessions/{session_id}/messages",
    tag = "sessions",
    params(("session_id" = String, Path, description = "Session ID")),
    request_body = AddMessageRequest,
    responses(
        (status = 200, description = "Message added", body = AddMessageResponse),
        (status = 404, description = "Session not found", body = ApiErrorResponse),
        (status = 429, description = "Rate limited", body = ApiErrorResponse)
    )
)]
fn openapi_add_message() {}

#[allow(dead_code)]
#[utoipa::path(
    post,
    path = "/sessions/{session_id}/commit",
    tag = "sessions",
    params(("session_id" = String, Path, description = "Session ID")),
    request_body = CommitSessionRequest,
    responses(
        (status = 200, description = "Session committed", body = CommitSessionResponse),
        (status = 404, description = "Session not found", body = ApiErrorResponse),
        (status = 429, description = "Rate limited", body = ApiErrorResponse)
    )
)]
fn openapi_commit_session() {}

#[allow(dead_code)]
#[utoipa::path(
    post,
    path = "/sessions/{session_id}/observations",
    tag = "sessions",
    params(("session_id" = String, Path, description = "Session ID")),
    request_body = AddObservationRequest,
    responses(
        (status = 200, description = "Observation stored", body = AddObservationResponse),
        (status = 429, description = "Rate limited", body = ApiErrorResponse)
    )
)]
fn openapi_add_observation() {}

#[allow(dead_code)]
#[utoipa::path(
    post,
    path = "/context/resolve",
    tag = "context",
    request_body = ResolveMemoryContextRequest,
    responses(
        (status = 200, description = "Resolved memory context", body = ContextResolveResponse),
        (status = 429, description = "Rate limited", body = ApiErrorResponse)
    )
)]
fn openapi_resolve_context() {}

#[allow(dead_code)]
#[utoipa::path(
    post,
    path = "/v1/memory:search",
    tag = "memory",
    request_body = MemorySearchRequest,
    responses(
        (status = 200, description = "Search memory results", body = MemorySearchResponse),
        (status = 429, description = "Rate limited", body = ApiErrorResponse)
    )
)]
fn openapi_search_memories() {}

#[allow(dead_code)]
#[utoipa::path(
    post,
    path = "/facts",
    tag = "facts",
    request_body = CreateFactRequest,
    responses(
        (status = 200, description = "Fact created", body = CreateFactResponse),
        (status = 429, description = "Rate limited", body = ApiErrorResponse)
    )
)]
fn openapi_create_fact() {}

#[allow(dead_code)]
#[utoipa::path(
    get,
    path = "/facts",
    tag = "facts",
    params(
        ("account_id" = Option<String>, Query, description = "Account ID override"),
        ("user_id" = Option<String>, Query, description = "User ID override"),
        ("limit" = Option<usize>, Query, description = "Maximum number of facts")
    ),
    responses((status = 200, description = "List active facts", body = FactListResponse))
)]
fn openapi_list_facts() {}

#[allow(dead_code)]
#[utoipa::path(
    post,
    path = "/facts/{fact_id}/supersede",
    tag = "facts",
    params(("fact_id" = String, Path, description = "Fact ID")),
    request_body = SupersedeFactRequest,
    responses(
        (status = 200, description = "Fact superseded", body = SupersedeFactResponse),
        (status = 404, description = "Fact not found", body = ApiErrorResponse),
        (status = 429, description = "Rate limited", body = ApiErrorResponse)
    )
)]
fn openapi_supersede_fact() {}

#[allow(dead_code)]
#[utoipa::path(
    post,
    path = "/facts/{fact_id}/retract",
    tag = "facts",
    params(("fact_id" = String, Path, description = "Fact ID")),
    responses(
        (status = 200, description = "Fact retracted", body = RetractFactResponse),
        (status = 404, description = "Fact not found", body = ApiErrorResponse),
        (status = 429, description = "Rate limited", body = ApiErrorResponse)
    )
)]
fn openapi_retract_fact() {}

#[allow(dead_code)]
#[utoipa::path(
    get,
    path = "/facts/{fact_id}/trace",
    tag = "facts",
    params(("fact_id" = String, Path, description = "Fact ID")),
    responses(
        (status = 200, description = "Trace fact provenance", body = TraceFactResponse),
        (status = 404, description = "Fact not found", body = ApiErrorResponse),
        (status = 429, description = "Rate limited", body = ApiErrorResponse)
    )
)]
fn openapi_trace_fact() {}

#[allow(dead_code)]
#[utoipa::path(
    post,
    path = "/relations",
    tag = "relations",
    request_body = LinkRelationRequest,
    responses(
        (status = 200, description = "Relation linked", body = RelationLinkResponse),
        (status = 429, description = "Rate limited", body = ApiErrorResponse)
    )
)]
fn openapi_link_relation() {}

#[allow(dead_code)]
#[utoipa::path(
    get,
    path = "/relations",
    tag = "relations",
    params(
        ("uri" = String, Query, description = "MemFuse URI"),
        ("limit" = Option<usize>, Query, description = "Maximum number of relations")
    ),
    responses((status = 200, description = "List URI relations", body = RelationListResponse))
)]
fn openapi_list_relations() {}

#[allow(dead_code)]
#[utoipa::path(
    delete,
    path = "/relations",
    tag = "relations",
    params(
        ("from_uri" = String, Query, description = "Source MemFuse URI"),
        ("to_uri" = String, Query, description = "Target MemFuse URI"),
        ("relation_type" = String, Query, description = "Relation type")
    ),
    responses(
        (status = 200, description = "Relation unlinked", body = RelationLinkResponse),
        (status = 429, description = "Rate limited", body = ApiErrorResponse)
    )
)]
fn openapi_unlink_relation() {}

#[allow(dead_code)]
#[utoipa::path(
    post,
    path = "/v1/webhooks",
    tag = "webhooks",
    request_body = CreateWebhookRequest,
    responses(
        (status = 200, description = "Webhook registered", body = WebhookRecord),
        (status = 429, description = "Rate limited", body = ApiErrorResponse)
    )
)]
fn openapi_create_webhook() {}

#[allow(dead_code)]
#[utoipa::path(
    get,
    path = "/v1/webhooks",
    tag = "webhooks",
    params(("limit" = Option<usize>, Query, description = "Maximum number of webhooks")),
    responses((status = 200, description = "List registered webhooks", body = WebhookListResponse))
)]
fn openapi_list_webhooks() {}

#[allow(dead_code)]
#[utoipa::path(
    delete,
    path = "/v1/webhooks/{id}",
    tag = "webhooks",
    params(("id" = String, Path, description = "Webhook ID")),
    responses(
        (status = 200, description = "Webhook deleted", body = DeleteWebhookResponse),
        (status = 404, description = "Webhook not found", body = ApiErrorResponse),
        (status = 429, description = "Rate limited", body = ApiErrorResponse)
    )
)]
fn openapi_delete_webhook() {}

#[allow(dead_code)]
#[utoipa::path(
    post,
    path = "/v1/webhooks/{id}/test",
    tag = "webhooks",
    params(("id" = String, Path, description = "Webhook ID")),
    responses(
        (status = 200, description = "Webhook test event delivered", body = TestWebhookResponse),
        (status = 404, description = "Webhook not found", body = ApiErrorResponse),
        (status = 429, description = "Rate limited", body = ApiErrorResponse)
    )
)]
fn openapi_test_webhook() {}

#[allow(dead_code)]
#[utoipa::path(post, path = "/v1/resources", tag = "resources", request_body = CreateResourceRequest, responses((status = 200, description = "Resource ingest scheduled", body = CreateResourceResponse), (status = 429, description = "Rate limited", body = ApiErrorResponse)))]
fn openapi_create_resource() {}

#[allow(dead_code)]
#[utoipa::path(get, path = "/v1/resources", tag = "resources", params(("limit" = Option<usize>, Query, description = "Maximum number of resources")), responses((status = 200, description = "List resources", body = ResourceListResponse)))]
fn openapi_list_resources() {}

#[allow(dead_code)]
#[utoipa::path(post, path = "/v1/resources/batch", tag = "resources", request_body = CreateResourcesBatchRequest, responses((status = 200, description = "Batch resource ingest scheduled", body = ResourceBatchResponse), (status = 429, description = "Rate limited", body = ApiErrorResponse)))]
fn openapi_create_resources_batch() {}

#[allow(dead_code)]
#[utoipa::path(post, path = "/v1/resources/import", tag = "resources", request_body = ResourceImportRequest, responses((status = 200, description = "Resource pack imported", body = CreateResourceResponse), (status = 429, description = "Rate limited", body = ApiErrorResponse)))]
fn openapi_import_resource() {}

#[allow(dead_code)]
#[utoipa::path(post, path = "/v1/resources/{resource_id}/export", tag = "resources", params(("resource_id" = String, Path, description = "Resource ID")), request_body = ResourceExportRequest, responses((status = 200, description = "Resource pack exported", body = ResourceExportResponse), (status = 404, description = "Resource not found", body = ApiErrorResponse)))]
fn openapi_export_resource() {}

#[allow(dead_code)]
#[utoipa::path(post, path = "/v1/resources/{resource_id}/refresh", tag = "resources", params(("resource_id" = String, Path, description = "Resource ID")), responses((status = 200, description = "Resource refresh scheduled", body = ResourceTaskResponse), (status = 404, description = "Resource not found", body = ApiErrorResponse)))]
fn openapi_refresh_resource() {}

#[allow(dead_code)]
#[utoipa::path(post, path = "/v1/resources/{resource_id}/rebuild", tag = "resources", params(("resource_id" = String, Path, description = "Resource ID")), responses((status = 200, description = "Resource rebuild scheduled", body = ResourceTaskResponse), (status = 404, description = "Resource not found", body = ApiErrorResponse)))]
fn openapi_rebuild_resource() {}

#[allow(dead_code)]
#[utoipa::path(get, path = "/v1/workspace/ls", tag = "workspace", params(("uri" = String, Query, description = "Workspace URI")), responses((status = 200, description = "List workspace URI", body = Vec<WorkspaceDirEntry>)))]
fn openapi_workspace_ls() {}

#[allow(dead_code)]
#[utoipa::path(get, path = "/v1/workspace/tree", tag = "workspace", params(("uri" = String, Query, description = "Workspace URI"), ("depth" = Option<usize>, Query, description = "Maximum tree depth")), responses((status = 200, description = "Render workspace tree", body = String)))]
fn openapi_workspace_tree() {}

#[allow(dead_code)]
#[utoipa::path(get, path = "/v1/workspace/stat", tag = "workspace", params(("uri" = String, Query, description = "Workspace URI")), responses((status = 200, description = "Get workspace URI metadata", body = WorkspaceStatResponse)))]
fn openapi_workspace_stat() {}

#[allow(dead_code)]
#[utoipa::path(get, path = "/v1/workspace/abstract", tag = "workspace", params(("uri" = String, Query, description = "Workspace URI")), responses((status = 200, description = "Read abstract for workspace URI", body = String)))]
fn openapi_workspace_abstract() {}

#[allow(dead_code)]
#[utoipa::path(get, path = "/v1/workspace/overview", tag = "workspace", params(("uri" = String, Query, description = "Workspace URI")), responses((status = 200, description = "Read overview for workspace URI", body = String)))]
fn openapi_workspace_overview() {}

#[allow(dead_code)]
#[utoipa::path(get, path = "/v1/workspace/read", tag = "workspace", params(("uri" = String, Query, description = "Workspace URI")), responses((status = 200, description = "Read workspace URI content", body = String)))]
fn openapi_workspace_read() {}

#[allow(dead_code)]
#[utoipa::path(get, path = "/v1/workspace/glob", tag = "workspace", params(("uri" = String, Query, description = "Workspace URI"), ("pattern" = String, Query, description = "Glob pattern")), responses((status = 200, description = "Glob workspace resources", body = Vec<String>)))]
fn openapi_workspace_glob() {}

#[allow(dead_code)]
#[utoipa::path(post, path = "/v1/workspace/mkdir", tag = "workspace", request_body = WorkspaceUriRequest, responses((status = 200, description = "Create workspace directory", body = WorkspaceMutationResponse)))]
fn openapi_workspace_mkdir() {}

#[allow(dead_code)]
#[utoipa::path(post, path = "/v1/workspace/write", tag = "workspace", request_body = WorkspaceWriteRequest, responses((status = 200, description = "Write workspace file", body = WorkspaceMutationResponse)))]
fn openapi_workspace_write() {}

#[allow(dead_code)]
#[utoipa::path(post, path = "/v1/workspace/mv", tag = "workspace", request_body = WorkspaceMoveRequest, responses((status = 200, description = "Move workspace file", body = WorkspaceMutationResponse)))]
fn openapi_workspace_mv() {}

#[allow(dead_code)]
#[utoipa::path(delete, path = "/v1/workspace/rm", tag = "workspace", params(("uri" = String, Query, description = "Workspace URI")), responses((status = 200, description = "Remove workspace path", body = WorkspaceMutationResponse)))]
fn openapi_workspace_rm() {}

#[allow(dead_code)]
#[utoipa::path(get, path = "/v1/workspace/find", tag = "workspace", params(("query" = String, Query, description = "Search query"), ("target" = Option<String>, Query, description = "Target URI"), ("limit" = Option<usize>, Query, description = "Maximum number of results")), responses((status = 200, description = "Find workspace resources", body = WorkspaceSearchResponse)))]
fn openapi_workspace_find() {}

#[allow(dead_code)]
#[utoipa::path(get, path = "/v1/workspace/grep", tag = "workspace", params(("query" = String, Query, description = "Search query"), ("target" = Option<String>, Query, description = "Target URI"), ("limit" = Option<usize>, Query, description = "Maximum number of results")), responses((status = 200, description = "Grep workspace resources", body = WorkspaceSearchResponse)))]
fn openapi_workspace_grep() {}

#[allow(dead_code)]
#[utoipa::path(get, path = "/v1/workspace/search", tag = "workspace", params(("query" = String, Query, description = "Search query"), ("target" = Option<String>, Query, description = "Target URI"), ("session_context" = Option<String>, Query, description = "Session context"), ("limit" = Option<usize>, Query, description = "Maximum number of results")), responses((status = 200, description = "Search workspace resources", body = WorkspaceSearchResponse)))]
fn openapi_workspace_search() {}

#[allow(dead_code)]
#[utoipa::path(get, path = "/v1/workspace/rebuild", tag = "workspace", responses((status = 200, description = "Rebuild workspace projection", body = WorkspaceRebuildResponse)))]
fn openapi_workspace_rebuild() {}

#[allow(dead_code)]
#[utoipa::path(post, path = "/v1/workspace/refresh", tag = "workspace", responses((status = 200, description = "Refresh workspace projection", body = WorkspaceRefreshResponse)))]
fn openapi_workspace_refresh() {}

#[allow(dead_code)]
#[utoipa::path(get, path = "/watches", tag = "watches", params(("limit" = Option<usize>, Query, description = "Maximum number of watches")), responses((status = 200, description = "List resource watches", body = WatchListResponse)))]
fn openapi_list_watches() {}

#[allow(dead_code)]
#[utoipa::path(post, path = "/watches/run-due", tag = "watches", responses((status = 200, description = "Run due watches", body = WatchRunDueResponse)))]
fn openapi_run_due_watches() {}

#[allow(dead_code)]
#[utoipa::path(post, path = "/watches/run-loop", tag = "watches", request_body = ResourceWatchLoopRequest, responses((status = 200, description = "Run watch loop", body = WatchLoopResponse)))]
fn openapi_run_watch_loop() {}

#[allow(dead_code)]
#[utoipa::path(post, path = "/watch-service/start", tag = "watches", request_body = WatchServiceRequest, responses((status = 200, description = "Start watch service", body = WatchServiceStatusResponse)))]
fn openapi_start_watch_service() {}

#[allow(dead_code)]
#[utoipa::path(get, path = "/watch-service/status", tag = "watches", responses((status = 200, description = "Watch service status", body = WatchServiceStatusResponse)))]
fn openapi_watch_service_status() {}

#[allow(dead_code)]
#[utoipa::path(post, path = "/watch-service/stop", tag = "watches", responses((status = 200, description = "Stop watch service", body = WatchServiceStatusResponse)))]
fn openapi_stop_watch_service() {}

#[allow(dead_code)]
#[utoipa::path(post, path = "/resources/{resource_id}/watch", tag = "watches", params(("resource_id" = String, Path, description = "Resource ID")), request_body = ResourceWatchRequest, responses((status = 200, description = "Register resource watch", body = ResourceWatchRecord)))]
fn openapi_register_watch() {}

#[allow(dead_code)]
#[utoipa::path(post, path = "/resources/{resource_id}/watch/disable", tag = "watches", params(("resource_id" = String, Path, description = "Resource ID")), responses((status = 200, description = "Disable resource watch", body = ResourceWatchRecord)))]
fn openapi_disable_watch() {}

#[allow(dead_code)]
#[utoipa::path(post, path = "/resources/{resource_id}/watch/run", tag = "watches", params(("resource_id" = String, Path, description = "Resource ID")), responses((status = 200, description = "Run resource watch", body = ResourceWatchRunResponse)))]
fn openapi_run_watch() {}

#[allow(dead_code)]
#[utoipa::path(post, path = "/heuristics/rules", tag = "heuristics", request_body = HeuristicRuleRequest, responses((status = 200, description = "Create heuristic rule", body = HeuristicRuleResponse)))]
fn openapi_create_heuristic_rule() {}

#[allow(dead_code)]
#[utoipa::path(get, path = "/heuristics/rules", tag = "heuristics", params(("limit" = Option<usize>, Query, description = "Maximum number of rules")), responses((status = 200, description = "List heuristic rules", body = HeuristicRuleListResponse)))]
fn openapi_list_heuristic_rules() {}

#[allow(dead_code)]
#[utoipa::path(get, path = "/heuristics/rules/{rule_id}", tag = "heuristics", params(("rule_id" = String, Path, description = "Rule ID")), responses((status = 200, description = "Get heuristic rule", body = HeuristicRuleRecord), (status = 404, description = "Rule not found", body = ApiErrorResponse)))]
fn openapi_get_heuristic_rule() {}

#[allow(dead_code)]
#[utoipa::path(post, path = "/heuristics/rules/{rule_id}/promote", tag = "heuristics", params(("rule_id" = String, Path, description = "Rule ID")), request_body = PromoteRuleRequest, responses((status = 200, description = "Promote heuristic rule", body = PromoteRuleResponse)))]
fn openapi_promote_heuristic_rule() {}

#[allow(dead_code)]
#[utoipa::path(post, path = "/heuristics/rules/{rule_id}/confirm", tag = "heuristics", params(("rule_id" = String, Path, description = "Rule ID")), responses((status = 200, description = "Confirm heuristic rule", body = ConfirmRuleResponse)))]
fn openapi_confirm_heuristic_rule() {}

#[allow(dead_code)]
#[utoipa::path(post, path = "/heuristics/instances", tag = "heuristics", request_body = HeuristicInstanceRequest, responses((status = 200, description = "Create heuristic instance", body = HeuristicInstanceResponse)))]
fn openapi_create_heuristic_instance() {}

#[allow(dead_code)]
#[utoipa::path(get, path = "/heuristics/instances", tag = "heuristics", params(("limit" = Option<usize>, Query, description = "Maximum number of instances")), responses((status = 200, description = "List heuristic instances", body = HeuristicInstanceListResponse)))]
fn openapi_list_heuristic_instances() {}

#[allow(dead_code)]
#[utoipa::path(get, path = "/heuristics/instances/{instance_id}", tag = "heuristics", params(("instance_id" = String, Path, description = "Instance ID")), responses((status = 200, description = "Get heuristic instance", body = HeuristicInstanceRecord), (status = 404, description = "Instance not found", body = ApiErrorResponse)))]
fn openapi_get_heuristic_instance() {}

#[allow(dead_code)]
#[utoipa::path(post, path = "/heuristics/retrieve", tag = "heuristics", request_body = RetrieveHeuristicsRequest, responses((status = 200, description = "Retrieve heuristics", body = RetrieveHeuristicsResponse)))]
fn openapi_retrieve_heuristics() {}

#[allow(dead_code)]
#[utoipa::path(post, path = "/heuristics/l0-confirmed", tag = "heuristics", request_body = L0ConfirmedRequest, responses((status = 200, description = "Retrieve confirmed L0 rules", body = RetrieveHeuristicsResponse)))]
fn openapi_l0_confirmed_rules() {}

#[allow(dead_code)]
#[utoipa::path(post, path = "/heuristics/simulate-reaction", tag = "heuristics", request_body = SimulateReactionRequest, responses((status = 200, description = "Simulate heuristic reaction", body = SimulateReactionResponse)))]
fn openapi_simulate_reaction() {}

#[allow(dead_code)]
#[utoipa::path(get, path = "/episodes/{episode_id}", tag = "episodes", params(("episode_id" = String, Path, description = "Episode ID")), responses((status = 200, description = "Get episode detail", body = EpisodeDetailResponse), (status = 404, description = "Episode not found", body = ApiErrorResponse)))]
fn openapi_get_episode() {}

#[allow(dead_code)]
#[utoipa::path(get, path = "/episodes/{episode_id}/timeline", tag = "episodes", params(("episode_id" = String, Path, description = "Episode ID"), ("direction" = Option<String>, Query, description = "Timeline direction"), ("radius" = Option<usize>, Query, description = "Timeline radius")), responses((status = 200, description = "Get episode timeline", body = EpisodeTimelineResponse), (status = 404, description = "Episode not found", body = ApiErrorResponse)))]
fn openapi_get_episode_timeline() {}

#[allow(dead_code)]
#[utoipa::path(post, path = "/memories/cite", tag = "memory", request_body = CiteMemoriesRequest, responses((status = 200, description = "Record memory citation feedback", body = CiteMemoriesResponse)))]
fn openapi_cite_memories() {}

#[allow(dead_code)]
#[utoipa::path(get, path = "/memories/export", tag = "memory", params(("user_id" = Option<String>, Query, description = "User ID override")), responses((status = 200, description = "Export memories", body = MemoryExportResponse)))]
fn openapi_export_memories() {}

#[allow(dead_code)]
#[utoipa::path(post, path = "/memories/import", tag = "memory", request_body = MemoryImportRequest, responses((status = 200, description = "Import memories", body = MemoryImportResponse)))]
fn openapi_import_memories() {}

#[allow(dead_code)]
#[utoipa::path(post, path = "/v1/memory:consolidate", tag = "memory", request_body = MemoryConsolidateRequest, responses((status = 200, description = "Consolidate memory", body = MemoryConsolidateResponse)))]
fn openapi_consolidate_memories() {}

#[allow(dead_code)]
#[utoipa::path(post, path = "/v1/memory:extract-facts", tag = "memory", request_body = MemoryExtractFactsRequest, responses((status = 200, description = "Extract facts from messages", body = MemoryExtractFactsResponse)))]
fn openapi_extract_facts() {}

#[allow(dead_code)]
#[utoipa::path(post, path = "/v1/memory:archive", tag = "memory", request_body = MemoryArchiveRequest, responses((status = 200, description = "Archive cold episodes", body = MemoryArchiveResponse)))]
fn openapi_archive_memories() {}

#[allow(dead_code)]
#[utoipa::path(post, path = "/v1/eval/recall", tag = "memory", request_body = EvalRecallRequest, responses((status = 200, description = "Evaluate recall", body = EvalRecallResponse)))]
fn openapi_eval_recall() {}

#[allow(dead_code)]
#[utoipa::path(post, path = "/code_symbols", tag = "code_symbols", request_body = CreateCodeSymbolsRequest, responses((status = 200, description = "Create code symbols", body = CodeSymbolsCreateResponse)))]
fn openapi_create_code_symbols() {}

#[allow(dead_code)]
#[utoipa::path(get, path = "/code_symbols", tag = "code_symbols", params(("projection_view_id" = Option<String>, Query, description = "Projection view ID"), ("canonical_uri" = Option<String>, Query, description = "Canonical URI"), ("limit" = Option<usize>, Query, description = "Maximum number of symbols")), responses((status = 200, description = "List code symbols", body = CodeSymbolListResponse)))]
fn openapi_list_code_symbols() {}

#[allow(dead_code)]
#[utoipa::path(get, path = "/code_symbols/search", tag = "code_symbols", params(("projection_view_id" = String, Query, description = "Projection view ID"), ("q" = String, Query, description = "Search query"), ("limit" = Option<usize>, Query, description = "Maximum number of symbols")), responses((status = 200, description = "Search code symbols", body = CodeSymbolSearchResponse)))]
fn openapi_search_code_symbols() {}

#[allow(dead_code)]
#[utoipa::path(delete, path = "/code_symbols/{view_id}", tag = "code_symbols", params(("view_id" = String, Path, description = "Projection view ID")), responses((status = 200, description = "Delete code symbols", body = DeleteCodeSymbolsResponse)))]
fn openapi_delete_code_symbols() {}

#[allow(dead_code)]
#[utoipa::path(get, path = "/snapshots", tag = "resources", params(("limit" = Option<usize>, Query, description = "Maximum number of snapshots")), responses((status = 200, description = "List snapshots", body = SnapshotListResponse)))]
fn openapi_snapshots() {}

#[allow(dead_code)]
#[utoipa::path(get, path = "/audit", tag = "system", params(("limit" = Option<usize>, Query, description = "Maximum number of audit records")), responses((status = 200, description = "List audit records", body = AuditListResponse)))]
fn openapi_audit() {}

#[allow(dead_code)]
#[utoipa::path(post, path = "/skills", tag = "skills", request_body = AddSkillRequest, responses((status = 200, description = "Add skill", body = AddSkillResponse)))]
fn openapi_add_skill() {}

#[allow(dead_code)]
#[utoipa::path(get, path = "/skills", tag = "skills", params(("limit" = Option<usize>, Query, description = "Maximum number of skills")), responses((status = 200, description = "List skills", body = SkillListResponse)))]
fn openapi_list_skills() {}

#[allow(dead_code)]
#[utoipa::path(get, path = "/system/status", tag = "system", responses((status = 200, description = "System status", body = SystemStatusResponse)))]
fn openapi_system_status() {}

#[allow(dead_code)]
#[utoipa::path(get, path = "/system/observer", tag = "system", responses((status = 200, description = "Observer status", body = ObserverStatusResponse)))]
fn openapi_observer_status() {}

#[allow(dead_code)]
#[utoipa::path(get, path = "/tasks", tag = "tasks", params(("limit" = Option<usize>, Query, description = "Maximum number of tasks")), responses((status = 200, description = "List tasks", body = TaskListResponse)))]
fn openapi_list_tasks() {}

#[allow(dead_code)]
#[utoipa::path(post, path = "/tasks/evict", tag = "tasks", responses((status = 200, description = "Evict tasks", body = TaskEvictResponse)))]
fn openapi_evict_tasks() {}

#[allow(dead_code)]
#[utoipa::path(get, path = "/tasks/{task_id}", tag = "tasks", params(("task_id" = String, Path, description = "Task ID")), responses((status = 200, description = "Get task status", body = TaskStatusResponse), (status = 404, description = "Task not found", body = ApiErrorResponse)))]
fn openapi_task_status() {}

#[allow(dead_code)]
#[utoipa::path(get, path = "/tasks/{task_id}/wait", tag = "tasks", params(("task_id" = String, Path, description = "Task ID"), ("timeout_ms" = Option<u64>, Query, description = "Wait timeout in milliseconds"), ("poll_ms" = Option<u64>, Query, description = "Polling interval in milliseconds")), responses((status = 200, description = "Wait for task completion", body = WaitTaskResponse), (status = 404, description = "Task not found", body = ApiErrorResponse)))]
fn openapi_wait_task() {}

struct SecurityAddon;

impl utoipa::Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "bearer_auth",
            utoipa::openapi::security::SecurityScheme::Http(utoipa::openapi::security::Http::new(
                utoipa::openapi::security::HttpAuthScheme::Bearer,
            )),
        );
    }
}
