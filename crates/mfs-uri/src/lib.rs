use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::path::{Path, PathBuf};

use mfs_types::IdentityContext;

const MFS_SCHEME: &str = "mfs://";
const VALID_ROOTS: [&str; 4] = ["resources", "user", "agent", "session"];
const MAX_COMPONENT_BYTES: usize = 96;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MfsUri {
    root: String,
    canonical_path: String,
}

impl MfsUri {
    pub fn parse(raw: &str) -> Result<Self, UriError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(UriError::Empty);
        }

        let logical_path = if let Some(path) = trimmed.strip_prefix(MFS_SCHEME) {
            path
        } else if trimmed.contains("://") {
            return Err(UriError::InvalidScheme(trimmed.to_owned()));
        } else {
            trimmed.trim_start_matches('/')
        };

        let mut segments = logical_path
            .split('/')
            .filter(|segment| !segment.is_empty());
        let root = segments.next().ok_or(UriError::MissingRoot)?;

        if !VALID_ROOTS.contains(&root) {
            return Err(UriError::InvalidRoot(root.to_owned()));
        }

        let mut canonical_segments = Vec::new();
        for segment in segments {
            Self::validate_segment(segment)?;
            canonical_segments.push(segment);
        }

        Ok(Self {
            root: root.to_owned(),
            canonical_path: canonical_segments.join("/"),
        })
    }

    pub fn root(&self) -> &str {
        &self.root
    }

    pub fn canonical_path(&self) -> &str {
        &self.canonical_path
    }

    fn canonical_segments(&self) -> impl Iterator<Item = &str> {
        self.canonical_path
            .split('/')
            .filter(|segment| !segment.is_empty())
    }

    fn validate_segment(segment: &str) -> Result<(), UriError> {
        if matches!(segment, "." | "..") || segment.contains('\\') || segment.contains('\0') {
            return Err(UriError::InvalidSegment(segment.to_owned()));
        }

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UriError {
    Empty,
    MissingRoot,
    InvalidScheme(String),
    InvalidRoot(String),
    InvalidSegment(String),
}

impl Display for UriError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("URI cannot be empty"),
            Self::MissingRoot => f.write_str("URI must include a logical root"),
            Self::InvalidScheme(raw) => write!(
                f,
                "URI must start with '{MFS_SCHEME}' or a short MFS path: {raw}"
            ),
            Self::InvalidRoot(root) => write!(
                f,
                "URI root '{root}' is not supported; expected one of {}",
                VALID_ROOTS.join(", ")
            ),
            Self::InvalidSegment(segment) => {
                write!(f, "URI path segment '{segment}' is not allowed")
            }
        }
    }
}

impl Error for UriError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceMapper {
    workspace_root: PathBuf,
}

impl WorkspaceMapper {
    pub fn new(workspace_root: impl AsRef<Path>) -> Self {
        Self {
            workspace_root: workspace_root.as_ref().to_path_buf(),
        }
    }

    pub fn map(
        &self,
        identity: &IdentityContext,
        uri: &MfsUri,
    ) -> Result<PathBuf, WorkspaceMapError> {
        let mut path = self.workspace_root.clone();
        path.push("tenants");
        path.push(identity.account_id());
        path.push(identity.user_id());

        match uri.root.as_str() {
            "resources" => path.push("resources"),
            "user" => {
                path.push("user");
            }
            "agent" => {
                path.push("agent");
                path.push(identity.agent_space_name());
            }
            "session" => {
                path.push("session");
            }
            root => return Err(WorkspaceMapError::UnsupportedRoot(root.to_owned())),
        }

        for segment in uri.canonical_segments() {
            path.push(shorten_component(segment));
        }

        Ok(path)
    }
}

pub fn shorten_component(component: &str) -> String {
    if component.len() <= MAX_COMPONENT_BYTES {
        return component.to_owned();
    }

    let suffix = short_hash_hex(component.as_bytes(), 8);
    let target_bytes = MAX_COMPONENT_BYTES.saturating_sub(suffix.len() + 1);
    let mut prefix = component.to_owned();
    while prefix.len() > target_bytes {
        prefix.pop();
    }
    format!("{prefix}-{suffix}")
}

pub fn short_hash_hex(bytes: &[u8], digits: usize) -> String {
    let mut hash = 14_695_981_039_346_656_037_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(1_099_511_628_211);
    }
    format!("{hash:016x}")
        .chars()
        .take(digits)
        .collect::<String>()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceMapError {
    UnsupportedRoot(String),
}

impl Display for WorkspaceMapError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedRoot(root) => {
                write!(f, "workspace mapping does not support URI root '{root}'")
            }
        }
    }
}

impl Error for WorkspaceMapError {}
