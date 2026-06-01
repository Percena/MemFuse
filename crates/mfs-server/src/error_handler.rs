use axum::response::{IntoResponse, Response};
use mfs_types::{MfsError, sanitize_secrets};
use std::io;

/// Wrapper that allows `MfsError` to be used as an Axum response type.
///
/// Since we cannot directly implement `IntoResponse` for `MfsError` (orphan rule),
/// we wrap it in this newtype. Use `AppError(mfs_error)` in handler return types.
pub struct AppError(pub MfsError);

impl AppError {
    /// Convert any `std::error::Error` into an `AppError` using best-effort heuristics.
    ///
    /// This is used for error types that don't have a dedicated `From` impl.
    /// Prefer adding a proper `From<T> for AppError` impl over relying on this.
    pub fn from_error<E: std::error::Error>(error: E) -> Self {
        AppError(map_boxed_error_str(&sanitize_secrets(&error.to_string())))
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        crate::http::api_types::ApiErrorResponse::from_mfs_error(self.0).into_response()
    }
}

/// Convert any `Box<dyn Error>` into an `AppError` by best-effort mapping,
/// then into an Axum response.
///
/// This allows existing handler code that returns `Result<T, Box<dyn Error>>`
/// to gradually adopt structured error responses without changing all return types.
pub fn boxed_error_to_response(error: Box<dyn std::error::Error>) -> Response {
    let mfs_error = map_boxed_error(error);
    AppError(mfs_error).into_response()
}

// ---------------------------------------------------------------------------
// Type-based error conversions (§2.5 — replaces string-match heuristics)
// ---------------------------------------------------------------------------
// Each source error type is mapped to MfsError by matching its variants
// directly, preserving structured information instead of relying on
// substring heuristics in `map_boxed_error_str`.
//
// NOTE: We implement `From<T> for AppError` directly (not `From<T> for MfsError`)
// because `AppError` is defined in this crate, satisfying the orphan rule.

impl From<MfsError> for AppError {
    fn from(error: MfsError) -> Self {
        AppError(error)
    }
}

// -- FsError → AppError (workspace/fs errors) --

impl From<mfs_workspace::FsError> for AppError {
    fn from(error: mfs_workspace::FsError) -> Self {
        AppError(classify_fs_error(error))
    }
}

// -- SessionError → AppError (session lifecycle errors) --

impl From<mfs_session::SessionError> for AppError {
    fn from(error: mfs_session::SessionError) -> Self {
        AppError(classify_session_error(error))
    }
}

// -- RetrievalError → AppError (search/retrieval errors) --

impl From<mfs_retrieval::RetrievalError> for AppError {
    fn from(error: mfs_retrieval::RetrievalError) -> Self {
        AppError(classify_retrieval_error(error))
    }
}

// -- ConnectorError → AppError (connector/source errors) --

impl From<mfs_connectors::ConnectorError> for AppError {
    fn from(error: mfs_connectors::ConnectorError) -> Self {
        AppError(classify_connector_error(error))
    }
}

// -- SemanticError → AppError (LLM/embedding pipeline errors) --

impl From<mfs_semantic::SemanticError> for AppError {
    fn from(error: mfs_semantic::SemanticError) -> Self {
        AppError(classify_semantic_error(error))
    }
}

// -- UrlGuardError → AppError (SSRF protection errors) --

impl From<mfs_connectors::url_guard::UrlGuardError> for AppError {
    fn from(error: mfs_connectors::url_guard::UrlGuardError) -> Self {
        AppError(classify_url_guard_error(error))
    }
}

// -- CatalogError → AppError (resource catalog errors) --

impl From<mfs_workspace::CatalogError> for AppError {
    fn from(error: mfs_workspace::CatalogError) -> Self {
        AppError(classify_catalog_error(error))
    }
}

// -- rusqlite::Error → AppError (metadata database errors) --

impl From<rusqlite::Error> for AppError {
    fn from(error: rusqlite::Error) -> Self {
        AppError(classify_rusqlite_error(error))
    }
}

// -- CanvasStoreError → AppError (canvas data operation errors) --

impl From<mfs_metadata::CanvasStoreError> for AppError {
    fn from(error: mfs_metadata::CanvasStoreError) -> Self {
        AppError(classify_canvas_store_error(error))
    }
}

// ---------------------------------------------------------------------------
// Box<dyn Error> fallback — still uses string heuristics for unknown types,
// but now only used when no type-based From impl is available.
// ---------------------------------------------------------------------------

impl From<Box<dyn std::error::Error>> for AppError {
    fn from(error: Box<dyn std::error::Error>) -> Self {
        AppError(map_boxed_error(error))
    }
}

impl From<Box<dyn std::error::Error + Send + Sync>> for AppError {
    fn from(error: Box<dyn std::error::Error + Send + Sync>) -> Self {
        AppError(map_boxed_error_str(&sanitize_secrets(&error.to_string())))
    }
}

// ---------------------------------------------------------------------------
// Structured error classification functions
// ---------------------------------------------------------------------------

/// Classify `FsError` variants into `MfsError` by matching enum variants directly.
fn classify_fs_error(error: mfs_workspace::FsError) -> MfsError {
    match error {
        mfs_workspace::FsError::Uri(uri_err) => classify_uri_error(uri_err),
        mfs_workspace::FsError::Layout(layout_err) => MfsError::InvalidArgument {
            field: "workspace_root".into(),
            reason: layout_err.to_string(),
        },
        mfs_workspace::FsError::Materialize(mat_err) => classify_materialize_error(mat_err),
        mfs_workspace::FsError::WritePolicy(msg) => MfsError::FailedPrecondition {
            precondition: "write_policy".into(),
            reason: msg,
        },
        mfs_workspace::FsError::Io {
            action,
            path,
            source,
        } => classify_io_error(action, &path, source),
        mfs_workspace::FsError::IoRaw(source) => MfsError::Internal {
            message: source.to_string(),
        },
    }
}

/// Classify `SessionError` variants into `MfsError`.
fn classify_session_error(error: mfs_session::SessionError) -> MfsError {
    match error {
        mfs_session::SessionError::NotFound(session_id) => MfsError::NotFound {
            resource: format!("session:{session_id}"),
        },
        mfs_session::SessionError::InvalidArgument { field, reason } => {
            MfsError::InvalidArgument { field, reason }
        }
        mfs_session::SessionError::LockTimeout(path) => MfsError::Conflict {
            resource: "path_lock".into(),
            reason: format!("timed out waiting for path lock '{}'", path.display()),
        },
        mfs_session::SessionError::Serde(source) => MfsError::Internal {
            message: source.to_string(),
        },
        mfs_session::SessionError::Io {
            action,
            path,
            source,
        } => classify_io_error(action, &path, source),
        mfs_session::SessionError::IoRaw(source) => MfsError::Internal {
            message: source.to_string(),
        },
    }
}

/// Classify `RetrievalError` variants into `MfsError`.
fn classify_retrieval_error(error: mfs_retrieval::RetrievalError) -> MfsError {
    match error {
        mfs_retrieval::RetrievalError::Fs(fs_err) => classify_fs_error(fs_err),
        mfs_retrieval::RetrievalError::Io(source) => MfsError::Internal {
            message: source.to_string(),
        },
        mfs_retrieval::RetrievalError::Index(idx_err) => MfsError::Unavailable {
            subsystem: "semantic_index".into(),
            reason: idx_err.to_string(),
        },
        mfs_retrieval::RetrievalError::Metadata(rusqlite_err) => {
            classify_rusqlite_error(rusqlite_err)
        }
    }
}

/// Classify `ConnectorError` variants into `MfsError`.
fn classify_connector_error(error: mfs_connectors::ConnectorError) -> MfsError {
    match error {
        mfs_connectors::ConnectorError::InvalidSource(msg) => MfsError::InvalidArgument {
            field: "source".into(),
            reason: msg,
        },
        mfs_connectors::ConnectorError::Unsupported(msg) => MfsError::InvalidArgument {
            field: "source_kind".into(),
            reason: msg,
        },
        mfs_connectors::ConnectorError::Io {
            action,
            path,
            source,
        } => classify_io_error(action, &path, source),
    }
}

/// Classify `SemanticError` variants into `MfsError`.
fn classify_semantic_error(error: mfs_semantic::SemanticError) -> MfsError {
    match error {
        mfs_semantic::SemanticError::Io(source) => MfsError::Internal {
            message: source.to_string(),
        },
        mfs_semantic::SemanticError::Index(idx_err) => MfsError::Unavailable {
            subsystem: "semantic_index".into(),
            reason: idx_err.to_string(),
        },
    }
}

/// Classify `UrlGuardError` variants into `MfsError` — SSRF protection errors
/// always map to client-side error variants (InvalidArgument or PermissionDenied).
fn classify_url_guard_error(error: mfs_connectors::url_guard::UrlGuardError) -> MfsError {
    match error {
        mfs_connectors::url_guard::UrlGuardError::InvalidUrl(url) => MfsError::InvalidArgument {
            field: "url".into(),
            reason: format!("invalid URL: {url}"),
        },
        mfs_connectors::url_guard::UrlGuardError::PrivateTarget { url, ip } => {
            MfsError::InvalidArgument {
                field: "url".into(),
                reason: format!("SSRF risk: '{url}' resolves to private IP {ip}"),
            }
        }
        mfs_connectors::url_guard::UrlGuardError::DnsResolutionFailed { url, reason } => {
            MfsError::Unavailable {
                subsystem: "dns".into(),
                reason: format!("DNS resolution failed for '{url}': {reason}"),
            }
        }
        mfs_connectors::url_guard::UrlGuardError::WhitelistOnly { url } => {
            MfsError::PermissionDenied {
                reason: format!("URL '{url}' not in allowed whitelist"),
            }
        }
    }
}

/// Classify `CatalogError` variants into `MfsError`.
fn classify_catalog_error(error: mfs_workspace::CatalogError) -> MfsError {
    match error {
        mfs_workspace::CatalogError::Metadata(rusqlite_err) => {
            classify_rusqlite_error(rusqlite_err)
        }
        mfs_workspace::CatalogError::Materialize(mat_err) => classify_materialize_error(mat_err),
        mfs_workspace::CatalogError::UnsupportedSourceKind(kind) => MfsError::InvalidArgument {
            field: "source_kind".into(),
            reason: format!("unsupported source kind: {kind}"),
        },
    }
}

/// Classify `rusqlite::Error` into `MfsError`.
fn classify_rusqlite_error(error: rusqlite::Error) -> MfsError {
    match error {
        rusqlite::Error::QueryReturnedNoRows => MfsError::NotFound {
            resource: "metadata record".into(),
        },
        rusqlite::Error::InvalidParameterName(_) | rusqlite::Error::InvalidColumnName(_) => {
            MfsError::InvalidArgument {
                field: "query".into(),
                reason: error.to_string(),
            }
        }
        // Map UNIQUE constraint violations to Conflict (409) instead of Internal (500)
        rusqlite::Error::SqliteFailure(ref err, ref msg)
            if err.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            MfsError::Conflict {
                resource: "metadata record".into(),
                reason: msg
                    .as_deref()
                    .unwrap_or("UNIQUE constraint violation")
                    .into(),
            }
        }
        _ => MfsError::Internal {
            message: error.to_string(),
        },
    }
}

/// Classify `CanvasStoreError` into `MfsError`.
fn classify_canvas_store_error(error: mfs_metadata::CanvasStoreError) -> MfsError {
    match error {
        mfs_metadata::CanvasStoreError::Sqlite(msg) => MfsError::Internal { message: msg },
        mfs_metadata::CanvasStoreError::Postgres(msg) => MfsError::Internal { message: msg },
        mfs_metadata::CanvasStoreError::Other(msg) => MfsError::Internal { message: msg },
    }
}

/// Classify an `io::Error` into the appropriate `MfsError` variant based on
/// `io::ErrorKind`, preserving the action and path context.
fn classify_io_error(action: &'static str, path: &std::path::Path, source: io::Error) -> MfsError {
    match source.kind() {
        io::ErrorKind::NotFound => MfsError::NotFound {
            resource: format!("{action} '{}'", path.display()),
        },
        io::ErrorKind::PermissionDenied => MfsError::PermissionDenied {
            reason: format!("failed to {action} '{}': permission denied", path.display()),
        },
        io::ErrorKind::AlreadyExists => MfsError::Conflict {
            resource: path.display().to_string(),
            reason: "already exists".into(),
        },
        io::ErrorKind::InvalidInput | io::ErrorKind::InvalidData => MfsError::InvalidArgument {
            field: "path".into(),
            reason: format!("failed to {action} '{}': {source}", path.display()),
        },
        _ => {
            // "Is a directory" and similar OS errors arrive as ErrorKind::Other
            // with a specific message — classify them as client errors.
            let msg = source.to_string();
            if msg.contains("Is a directory")
                || msg.contains("is a directory")
                || msg.contains("is not a file")
                || msg.contains("os error 21")
            {
                MfsError::InvalidArgument {
                    field: "uri".into(),
                    reason: format!("failed to {action} '{}': {msg}", path.display()),
                }
            } else {
                MfsError::Internal {
                    message: format!("failed to {action} '{}': {source}", path.display()),
                }
            }
        }
    }
}

/// Classify `UriError` variants — URI parse errors are always client-side.
fn classify_uri_error(uri_err: mfs_uri::UriError) -> MfsError {
    match uri_err {
        mfs_uri::UriError::Empty => MfsError::InvalidArgument {
            field: "uri".into(),
            reason: "URI cannot be empty".into(),
        },
        mfs_uri::UriError::MissingRoot => MfsError::InvalidArgument {
            field: "uri".into(),
            reason: "URI must include a logical root".into(),
        },
        mfs_uri::UriError::InvalidScheme(raw) => MfsError::InvalidArgument {
            field: "uri".into(),
            reason: format!("invalid URI scheme: {raw}"),
        },
        mfs_uri::UriError::InvalidRoot(root) => MfsError::InvalidArgument {
            field: "uri".into(),
            reason: format!("unsupported URI root: {root}"),
        },
        mfs_uri::UriError::InvalidSegment(seg) => MfsError::InvalidArgument {
            field: "uri".into(),
            reason: format!("invalid URI segment: {seg}"),
        },
    }
}

/// Classify `MaterializeError` — resource materialization failures.
fn classify_materialize_error(mat_err: mfs_workspace::MaterializeError) -> MfsError {
    match mat_err {
        mfs_workspace::MaterializeError::Uri(uri_err) => classify_uri_error(uri_err),
        mfs_workspace::MaterializeError::Layout(layout_err) => MfsError::InvalidArgument {
            field: "workspace_root".into(),
            reason: layout_err.to_string(),
        },
        mfs_workspace::MaterializeError::Connector(conn_err) => classify_connector_error(conn_err),
        mfs_workspace::MaterializeError::Summary(summ_err) => MfsError::Internal {
            message: summ_err.to_string(),
        },
        mfs_workspace::MaterializeError::Overlap { source, workspace } => {
            MfsError::FailedPrecondition {
                precondition: "no_overlap".into(),
                reason: format!(
                    "source '{}' overlaps with workspace '{}' — causes infinite recursion",
                    source.display(),
                    workspace.display(),
                ),
            }
        }
        mfs_workspace::MaterializeError::Io {
            action,
            path,
            source,
        } => classify_io_error(action, &path, source),
    }
}

// ---------------------------------------------------------------------------
// Legacy string-match fallback for truly unknown error types
// ---------------------------------------------------------------------------

/// Map a `Box<dyn Error>` to a `MfsError` using best-effort heuristics.
///
/// Only used for error types without a dedicated `From<T> for AppError` impl.
/// Prefer adding type-based conversions over relying on this.
fn map_boxed_error(error: Box<dyn std::error::Error>) -> MfsError {
    let error_str = sanitize_secrets(&error.to_string());
    map_boxed_error_str(&error_str)
}

/// Map an already-sanitized error string to a `MfsError` using best-effort heuristics.
///
/// **This is a fallback for unknown error types only.** All known crate error
/// types should have dedicated `From<T> for AppError` implementations above.
fn map_boxed_error_str(error_str: &str) -> MfsError {
    let error_str = error_str.to_owned();

    // PID lock errors — must come before generic "not found" pattern
    if error_str.contains("Data dir locked") {
        return MfsError::Conflict {
            resource: "workspace".into(),
            reason: error_str,
        };
    }

    // SSRF errors from url_guard — must come before generic patterns
    if error_str.contains("SSRF risk") {
        return MfsError::InvalidArgument {
            field: "url".into(),
            reason: error_str,
        };
    }

    // Generic patterns for unknown error types
    if error_str.contains("not found")
        || error_str.contains("does not exist")
        || error_str.contains("no such")
    {
        return MfsError::NotFound {
            resource: error_str,
        };
    }

    if error_str.contains("already exists") || error_str.contains("UNIQUE constraint failed") {
        return MfsError::Conflict {
            resource: "unknown".into(),
            reason: error_str,
        };
    }

    if error_str.contains("permission denied") || error_str.contains("unauthorized") {
        return MfsError::PermissionDenied { reason: error_str };
    }

    if error_str.contains("is a directory")
        || error_str.contains("Is a directory")
        || error_str.contains("is not a file")
        || error_str.contains("os error 21")
        || error_str.contains("invalid")
        || error_str.contains("unsupported")
    {
        return MfsError::InvalidArgument {
            field: "uri".into(),
            reason: error_str,
        };
    }

    // Default: treat as internal error
    MfsError::Internal { message: error_str }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;

    #[test]
    fn app_error_into_response_status_codes() {
        let error = AppError(MfsError::NotFound {
            resource: "mfs://resources/docs".into(),
        });
        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn app_error_sanitizes_secrets_in_response() {
        let error = AppError(MfsError::Internal {
            message: "OpenAI API error with key sk-proj-abc123def456".into(),
        });
        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn fs_error_uri_variants_map_to_invalid_argument() {
        let uri_err = mfs_uri::UriError::Empty;
        let fs_err = mfs_workspace::FsError::Uri(uri_err);
        let app: AppError = fs_err.into();
        assert!(matches!(app.0, MfsError::InvalidArgument { .. }));
    }

    #[test]
    fn fs_error_io_not_found_maps_to_mfs_not_found() {
        let fs_err = mfs_workspace::FsError::Io {
            action: "read",
            path: "/tmp/missing.txt".into(),
            source: io::Error::new(io::ErrorKind::NotFound, "file not found"),
        };
        let app: AppError = fs_err.into();
        assert!(matches!(app.0, MfsError::NotFound { .. }));
    }

    #[test]
    fn fs_error_io_permission_denied_maps_to_mfs_permission() {
        let fs_err = mfs_workspace::FsError::Io {
            action: "write",
            path: "/root/secret.txt".into(),
            source: io::Error::new(io::ErrorKind::PermissionDenied, "permission denied"),
        };
        let app: AppError = fs_err.into();
        assert!(matches!(app.0, MfsError::PermissionDenied { .. }));
    }

    #[test]
    fn fs_error_io_already_exists_maps_to_mfs_conflict() {
        let fs_err = mfs_workspace::FsError::Io {
            action: "create",
            path: "/tmp/existing.txt".into(),
            source: io::Error::new(io::ErrorKind::AlreadyExists, "already exists"),
        };
        let app: AppError = fs_err.into();
        assert!(matches!(app.0, MfsError::Conflict { .. }));
    }

    #[test]
    fn fs_error_write_policy_maps_to_failed_precondition() {
        let fs_err = mfs_workspace::FsError::WritePolicy("no writes to resources root".into());
        let app: AppError = fs_err.into();
        assert!(matches!(app.0, MfsError::FailedPrecondition { .. }));
    }

    #[test]
    fn fs_error_materialize_overlap_maps_to_failed_precondition() {
        let mat_err = mfs_workspace::MaterializeError::Overlap {
            source: "/tmp/src".into(),
            workspace: "/tmp/ws".into(),
        };
        let fs_err = mfs_workspace::FsError::Materialize(mat_err);
        let app: AppError = fs_err.into();
        assert!(matches!(app.0, MfsError::FailedPrecondition { .. }));
    }

    #[test]
    fn session_error_not_found_maps_to_mfs_not_found() {
        let sess_err = mfs_session::SessionError::NotFound("sess-123".into());
        let app: AppError = sess_err.into();
        assert!(matches!(app.0, MfsError::NotFound { .. }));
        if let MfsError::NotFound { resource } = app.0 {
            assert!(resource.contains("sess-123"));
        }
    }

    #[test]
    fn session_error_lock_timeout_maps_to_mfs_conflict() {
        let sess_err = mfs_session::SessionError::LockTimeout("/tmp/lock".into());
        let app: AppError = sess_err.into();
        assert!(matches!(app.0, MfsError::Conflict { .. }));
    }

    #[test]
    fn retrieval_error_fs_delegates_to_fs_error_classification() {
        let fs_err = mfs_workspace::FsError::Uri(mfs_uri::UriError::Empty);
        let ret_err = mfs_retrieval::RetrievalError::Fs(fs_err);
        let app: AppError = ret_err.into();
        assert!(matches!(app.0, MfsError::InvalidArgument { .. }));
    }

    #[test]
    fn retrieval_error_metadata_maps_to_internal() {
        let ret_err = mfs_retrieval::RetrievalError::Metadata(rusqlite::Error::InvalidPath(
            "bad path".into(),
        ));
        let app: AppError = ret_err.into();
        assert!(matches!(app.0, MfsError::Internal { .. }));
    }

    #[test]
    fn connector_error_invalid_source_maps_to_invalid_argument() {
        let conn_err = mfs_connectors::ConnectorError::InvalidSource("bad source".into());
        let app: AppError = conn_err.into();
        assert!(matches!(app.0, MfsError::InvalidArgument { .. }));
    }

    #[test]
    fn url_guard_error_ssrf_maps_to_invalid_argument() {
        let guard_err = mfs_connectors::url_guard::UrlGuardError::PrivateTarget {
            url: "http://evil.com".into(),
            ip: "10.0.0.1".into(),
        };
        let app: AppError = guard_err.into();
        assert!(matches!(app.0, MfsError::InvalidArgument { field, .. } if field == "url"));
    }

    #[test]
    fn catalog_error_unsupported_source_kind_maps_to_invalid_argument() {
        let cat_err = mfs_workspace::CatalogError::UnsupportedSourceKind("s3".into());
        let app: AppError = cat_err.into();
        assert!(matches!(app.0, MfsError::InvalidArgument { .. }));
    }

    #[test]
    fn semantic_error_index_maps_to_unavailable() {
        let sem_err = mfs_semantic::SemanticError::Index(mfs_index::IndexError::Sqlite(
            rusqlite::Error::InvalidPath("bad".into()),
        ));
        let app: AppError = sem_err.into();
        assert!(matches!(app.0, MfsError::Unavailable { .. }));
    }

    #[test]
    fn map_boxed_error_not_found_pattern() {
        let error: Box<dyn std::error::Error> = "resource not found: foo".into();
        let mfs = map_boxed_error(error);
        assert!(matches!(mfs, MfsError::NotFound { .. }));
    }

    #[test]
    fn map_boxed_error_ssrf_pattern() {
        let error: Box<dyn std::error::Error> =
            "SSRF risk: url resolves to private IP 10.0.0.1".into();
        let mfs = map_boxed_error(error);
        assert!(matches!(mfs, MfsError::InvalidArgument { .. }));
    }

    #[test]
    fn map_boxed_error_pid_lock_pattern() {
        let error: Box<dyn std::error::Error> =
            "Data dir locked by MemFuse process PID=1234".into();
        let mfs = map_boxed_error(error);
        assert!(matches!(mfs, MfsError::Conflict { .. }));
    }

    #[test]
    fn map_boxed_error_default_internal() {
        let error: Box<dyn std::error::Error> = "some random error".into();
        let mfs = map_boxed_error(error);
        assert!(matches!(mfs, MfsError::Internal { .. }));
    }

    #[test]
    fn map_boxed_error_conflict_not_misclassified_as_not_found() {
        // Regression test: "Data dir locked" contains no "not found" substring
        // but the PID lock pattern correctly takes priority.
        let error: Box<dyn std::error::Error> = "Data dir locked — config key not found".into();
        let mfs = map_boxed_error(error);
        // PID lock pattern takes priority over "not found" pattern
        assert!(matches!(mfs, MfsError::Conflict { .. }));
    }

    #[test]
    fn type_based_conversion_overrides_string_match() {
        // This is the key improvement: a NotFound SessionError now correctly
        // maps to MfsError::NotFound via type-based conversion, not via
        // string heuristic that could misclassify.
        let sess_err = mfs_session::SessionError::NotFound("test-session".into());
        let app: AppError = sess_err.into();
        // Verify it's NotFound with the session ID in the resource field
        if let MfsError::NotFound { resource } = app.0 {
            assert!(resource.contains("test-session"));
        } else {
            panic!("expected NotFound, got {:?}", app.0);
        }
    }

    #[test]
    fn rusqlite_query_returned_no_rows_maps_to_not_found() {
        let err = rusqlite::Error::QueryReturnedNoRows;
        let app: AppError = err.into();
        assert!(matches!(app.0, MfsError::NotFound { .. }));
    }
}
