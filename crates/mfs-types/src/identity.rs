use crate::MfsError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OwnerSpace {
    User(String),
    Agent(String),
}

/// Validate that a string is safe for use as a filesystem path segment.
/// Allows only alphanumeric characters, hyphens, underscores, and dots
/// (but not leading dots). Rejects empty strings, `..`, and anything
/// containing `/` or `\`.
pub fn sanitize_path_segment<'a>(value: &'a str, field: &str) -> Result<&'a str, MfsError> {
    if value.is_empty() {
        return Err(MfsError::InvalidArgument {
            field: field.to_owned(),
            reason: "must not be empty".to_owned(),
        });
    }
    if value == "." || value == ".." {
        return Err(MfsError::InvalidArgument {
            field: field.to_owned(),
            reason: "must not be '.' or '..'".to_owned(),
        });
    }
    if value.starts_with('.') {
        return Err(MfsError::InvalidArgument {
            field: field.to_owned(),
            reason: "must not start with '.'".to_owned(),
        });
    }
    let valid = value
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.');
    if !valid {
        return Err(MfsError::InvalidArgument {
            field: field.to_owned(),
            reason: format!(
                "contains invalid characters (allowed: alphanumeric, '-', '_', '.'): {value}"
            ),
        });
    }
    Ok(value)
}

#[derive(Debug, Clone)]
pub struct IdentityContext {
    account_id: String,
    user_id: String,
    agent_id: String,
}

impl IdentityContext {
    pub fn new(account_id: &str, user_id: &str, agent_id: &str) -> Self {
        Self {
            account_id: account_id.to_owned(),
            user_id: user_id.to_owned(),
            agent_id: agent_id.to_owned(),
        }
    }

    pub fn account_id(&self) -> &str {
        &self.account_id
    }

    pub fn user_id(&self) -> &str {
        &self.user_id
    }

    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }

    pub fn agent_space_name(&self) -> String {
        format!("{}__{}", self.user_id, self.agent_id)
    }

    pub fn user_space(&self) -> OwnerSpace {
        OwnerSpace::User(self.user_id.clone())
    }

    pub fn agent_space(&self) -> OwnerSpace {
        OwnerSpace::Agent(self.agent_space_name())
    }
}
