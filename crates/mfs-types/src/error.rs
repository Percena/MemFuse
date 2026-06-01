/// Unified error type for the MemFuse system.
///
/// Every crate should convert its local errors into `MfsError` before
/// returning them through the HTTP API layer. This ensures consistent
/// error responses and proper HTTP status code mapping.
#[derive(Debug, thiserror::Error)]
pub enum MfsError {
    #[error("not found: {resource}")]
    NotFound { resource: String },

    #[error("permission denied: {reason}")]
    PermissionDenied { reason: String },

    #[error("conflict on {resource}: {reason}")]
    Conflict { resource: String, reason: String },

    #[error("invalid argument '{field}': {reason}")]
    InvalidArgument { field: String, reason: String },

    #[error("unavailable: {subsystem} — {reason}")]
    Unavailable { subsystem: String, reason: String },

    #[error("failed precondition '{precondition}': {reason}")]
    FailedPrecondition {
        precondition: String,
        reason: String,
    },

    #[error("internal error: {message}")]
    Internal { message: String },
}

impl MfsError {
    /// Map this error to an HTTP status code.
    pub fn http_status(&self) -> u16 {
        match self {
            Self::NotFound { .. } => 404,
            Self::PermissionDenied { .. } => 403,
            Self::Conflict { .. } => 409,
            Self::InvalidArgument { .. } => 400,
            Self::Unavailable { .. } => 503,
            Self::FailedPrecondition { .. } => 412,
            Self::Internal { .. } => 500,
        }
    }

    /// The error category name, suitable for use as a JSON `error` field.
    pub fn category(&self) -> &'static str {
        match self {
            Self::NotFound { .. } => "NotFound",
            Self::PermissionDenied { .. } => "PermissionDenied",
            Self::Conflict { .. } => "Conflict",
            Self::InvalidArgument { .. } => "InvalidArgument",
            Self::Unavailable { .. } => "Unavailable",
            Self::FailedPrecondition { .. } => "FailedPrecondition",
            Self::Internal { .. } => "Internal",
        }
    }

    /// Whether the client can retry the request and potentially succeed.
    ///
    /// - `Unavailable` and `Conflict` are retryable (transient failures).
    /// - All other variants are not retryable (client must change something).
    pub fn retryable(&self) -> bool {
        matches!(self, Self::Unavailable { .. } | Self::Conflict { .. })
    }
}

/// Sanitize a string by replacing known secret patterns with `[REDACTED]`.
///
/// This should be applied to any text before it is included in:
/// - HTTP error responses
/// - Audit log entries
/// - Log output
///
/// Patterns detected:
/// - `sk-...` (OpenAI API keys)
/// - `Bearer ...` (HTTP auth tokens)
/// - `ghp_...` (GitHub personal access tokens)
/// - `cr_...` (various cloud credentials)
pub fn sanitize_secrets(text: &str) -> String {
    let mut result = text.to_owned();

    // OpenAI API keys: sk-proj-xxx, sk-xxx
    let sk_pattern = regex_lazy("sk-[A-Za-z0-9_-]{8,}");
    result = sk_pattern
        .replace_all(&result, "sk-[REDACTED]")
        .into_owned();

    // Bearer tokens
    let bearer_pattern = regex_lazy("Bearer [A-Za-z0-9_.-]{8,}");
    result = bearer_pattern
        .replace_all(&result, "Bearer [REDACTED]")
        .into_owned();

    // GitHub personal access tokens
    let ghp_pattern = regex_lazy("ghp_[A-Za-z0-9]{8,}");
    result = ghp_pattern
        .replace_all(&result, "ghp_[REDACTED]")
        .into_owned();

    // Cloud credentials
    let cr_pattern = regex_lazy("cr_[A-Za-z0-9]{8,}");
    result = cr_pattern
        .replace_all(&result, "cr_[REDACTED]")
        .into_owned();

    result
}

fn regex_lazy(pattern: &str) -> regex::Regex {
    regex::Regex::new(pattern).expect("invalid secret pattern regex")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_status_mapping() {
        assert_eq!(
            MfsError::NotFound {
                resource: "x".into()
            }
            .http_status(),
            404
        );
        assert_eq!(
            MfsError::PermissionDenied { reason: "x".into() }.http_status(),
            403
        );
        assert_eq!(
            MfsError::Conflict {
                resource: "x".into(),
                reason: "x".into()
            }
            .http_status(),
            409
        );
        assert_eq!(
            MfsError::InvalidArgument {
                field: "x".into(),
                reason: "x".into()
            }
            .http_status(),
            400
        );
        assert_eq!(
            MfsError::Unavailable {
                subsystem: "x".into(),
                reason: "x".into()
            }
            .http_status(),
            503
        );
        assert_eq!(
            MfsError::FailedPrecondition {
                precondition: "x".into(),
                reason: "x".into()
            }
            .http_status(),
            412
        );
        assert_eq!(
            MfsError::Internal {
                message: "x".into()
            }
            .http_status(),
            500
        );
    }

    #[test]
    fn category_names() {
        assert_eq!(
            MfsError::NotFound {
                resource: "x".into()
            }
            .category(),
            "NotFound"
        );
        assert_eq!(
            MfsError::Internal {
                message: "x".into()
            }
            .category(),
            "Internal"
        );
    }

    #[test]
    fn sanitize_openai_key() {
        let input = "Error with key sk-proj-abc123def456ghi789";
        assert_eq!(sanitize_secrets(input), "Error with key sk-[REDACTED]");
    }

    #[test]
    fn sanitize_bearer_token() {
        let input = "Auth failed: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9";
        assert_eq!(sanitize_secrets(input), "Auth failed: Bearer [REDACTED]");
    }

    #[test]
    fn sanitize_github_token() {
        let input = "Push failed with ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZ";
        assert_eq!(sanitize_secrets(input), "Push failed with ghp_[REDACTED]");
    }

    #[test]
    fn sanitize_no_secrets() {
        let input = "Normal error message without secrets";
        assert_eq!(sanitize_secrets(input), input);
    }

    #[test]
    fn sanitize_multiple_secrets() {
        let input = "sk-proj-abc123 and Bearer token123abc";
        assert_eq!(
            sanitize_secrets(input),
            "sk-[REDACTED] and Bearer [REDACTED]"
        );
    }
}
