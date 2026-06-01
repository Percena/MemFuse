use axum::{
    extract::{Request, State},
    middleware::Next,
    response::{IntoResponse, Response},
};

use crate::http::AppState;

/// Authentication mode — cached at server startup, not read per-request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthMode {
    /// Development mode — all requests pass through (no authentication).
    Dev,
    /// API key mode — requires a valid `Authorization: Bearer <key>` header.
    ApiKey,
}

impl AuthMode {
    /// Read the authentication mode from `MEMFUSE_AUTH_MODE` (default: `dev`).
    ///
    /// Called once at server startup; the result is stored in [`AppState`].
    pub fn from_env() -> Self {
        match std::env::var("MEMFUSE_AUTH_MODE")
            .unwrap_or_else(|_| "dev".to_owned())
            .as_str()
        {
            "api_key" | "api-key" => AuthMode::ApiKey,
            _ => AuthMode::Dev,
        }
    }
}

/// API key configuration — cached at server startup, not read per-request.
#[derive(Debug, Clone)]
pub struct ApiKeyConfig {
    pub key: String,
}

impl ApiKeyConfig {
    /// Read the API key from `MEMFUSE_API_KEY`.
    ///
    /// Returns `None` if the variable is not set or empty.
    /// Called once at server startup; the result is stored in [`AppState`].
    pub fn from_env() -> Option<Self> {
        std::env::var("MEMFUSE_API_KEY")
            .ok()
            .filter(|k| !k.is_empty())
            .map(|key| ApiKeyConfig { key })
    }
}

/// Axum middleware that enforces authentication based on [`AppState`] fields.
///
/// - In `Dev` mode: all requests pass through unconditionally.
/// - In `ApiKey` mode: requests must carry `Authorization: Bearer <key>` where
///   `<key>` matches the configured API key.  Health/readiness/metrics
///   endpoints (`/health`, `/ready`, `/metrics`) are exempt.
///
/// Auth config is read from the environment once at server startup and stored
/// in `AppState`, so env-var changes mid-flight do NOT affect auth behaviour
/// without a server restart.  This avoids both timing inconsistencies and
/// cross-test env pollution.
///
/// On authentication failure the middleware returns `401 Unauthorized` with a
/// JSON body describing the error.
pub async fn auth_middleware(
    State(state): State<std::sync::Arc<AppState>>,
    req: Request,
    next: Next,
) -> Response {
    // In dev mode, skip all checks.
    if state.auth_mode == AuthMode::Dev {
        return next.run(req).await;
    }

    // Exempt observability endpoints from authentication.
    let path = req.uri().path();
    if matches!(path, "/health" | "/ready" | "/metrics") {
        return next.run(req).await;
    }

    // In api_key mode, validate the Bearer token.
    let api_key = match &state.api_key_config {
        Some(config) => &config.key,
        None => {
            // Misconfiguration: api_key mode without MEMFUSE_API_KEY set.
            tracing::error!("MEMFUSE_AUTH_MODE=api_key but MEMFUSE_API_KEY is not set");
            return unauthorized_response("server misconfigured: API key not set");
        }
    };

    let auth_header = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok());

    let token = match auth_header {
        Some(header) if header.starts_with("Bearer ") => &header[7..],
        _ => {
            return unauthorized_response("missing or invalid Authorization header");
        }
    };

    // Constant-time comparison to prevent timing attacks.
    if !constant_time_eq(token.as_bytes(), api_key.as_bytes()) {
        return unauthorized_response("invalid API key");
    }

    next.run(req).await
}

/// Byte-by-byte constant-time equality check.
///
/// Always scans both slices in full regardless of where the first mismatch
/// occurs, preventing timing side-channel attacks on secret comparison.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    // Include length mismatch in the accumulator so the result is always
    // false when lengths differ, while still scanning max(a.len(), b.len())
    // bytes to avoid leaking length via timing.
    let len_eq = a.len() == b.len();
    let max_len = a.len().max(b.len());
    let mut acc = if len_eq { 0u8 } else { 1u8 };
    for i in 0..max_len {
        let ba = if i < a.len() { a[i] } else { 0 };
        let bb = if i < b.len() { b[i] } else { 0 };
        acc |= ba ^ bb;
    }
    acc == 0
}

fn unauthorized_response(message: &str) -> Response {
    crate::http::api_types::ApiErrorResponse::unauthorized(message).into_response()
}
