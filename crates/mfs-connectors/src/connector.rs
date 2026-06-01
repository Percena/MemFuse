use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConnectorCapabilities {
    pub can_enumerate: bool,
    pub can_read_bytes: bool,
    pub can_read_metadata: bool,
    pub supports_snapshot_materialization: bool,
}

#[async_trait::async_trait]
pub trait ResourceConnector {
    fn capabilities(&self) -> ConnectorCapabilities;
    async fn enumerate(&self, source: &SourceRef) -> Result<Vec<ResourceNode>, ConnectorError>;
    async fn read_bytes(
        &self,
        source: &SourceRef,
        node: &ResourceNode,
    ) -> Result<Vec<u8>, ConnectorError>;
    async fn snapshot_id(&self, source: &SourceRef) -> Result<String, ConnectorError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRef {
    source_kind: String,
    source_identifier: String,
    branch: Option<String>,
    revision: Option<String>,
}

impl SourceRef {
    pub fn new(source_kind: impl Into<String>, source_identifier: impl Into<String>) -> Self {
        Self {
            source_kind: source_kind.into(),
            source_identifier: source_identifier.into(),
            branch: None,
            revision: None,
        }
    }

    pub fn with_branch(mut self, branch: impl Into<String>) -> Self {
        self.branch = Some(branch.into());
        self
    }

    pub fn with_revision(mut self, revision: impl Into<String>) -> Self {
        self.revision = Some(revision.into());
        self
    }

    pub fn source_kind(&self) -> &str {
        &self.source_kind
    }

    pub fn source_identifier(&self) -> &str {
        &self.source_identifier
    }

    pub fn identifier_path(&self) -> &Path {
        Path::new(&self.source_identifier)
    }

    pub fn branch(&self) -> Option<&str> {
        self.branch.as_deref()
    }

    pub fn revision(&self) -> Option<&str> {
        self.revision.as_deref()
    }

    /// Resolve the git ref to use: revision overrides branch, branch overrides HEAD.
    /// Returns the refspec string for git2 operations.
    pub fn git_ref(&self) -> &str {
        self.revision
            .as_deref()
            .or(self.branch.as_deref())
            .unwrap_or("HEAD")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceNodeKind {
    Directory,
    File,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceNode {
    relative_path: PathBuf,
    kind: ResourceNodeKind,
}

impl ResourceNode {
    pub fn directory(relative_path: impl Into<PathBuf>) -> Self {
        Self {
            relative_path: relative_path.into(),
            kind: ResourceNodeKind::Directory,
        }
    }

    pub fn file(relative_path: impl Into<PathBuf>) -> Self {
        Self {
            relative_path: relative_path.into(),
            kind: ResourceNodeKind::File,
        }
    }

    pub fn relative_path(&self) -> &Path {
        &self.relative_path
    }

    pub fn kind(&self) -> ResourceNodeKind {
        self.kind
    }

    pub fn is_file(&self) -> bool {
        self.kind == ResourceNodeKind::File
    }
}

#[derive(Debug)]
pub enum ConnectorError {
    InvalidSource(String),
    Unsupported(String),
    Io {
        action: &'static str,
        path: PathBuf,
        source: io::Error,
    },
}

impl ConnectorError {
    pub fn io(action: &'static str, path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::Io {
            action,
            path: path.into(),
            source,
        }
    }
}

impl Display for ConnectorError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSource(message) => f.write_str(message),
            Self::Unsupported(message) => f.write_str(message),
            Self::Io {
                action,
                path,
                source,
            } => write!(
                f,
                "connector failed to {action} '{}': {source}",
                path.display()
            ),
        }
    }
}

impl Error for ConnectorError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::InvalidSource(_) | Self::Unsupported(_) => None,
        }
    }
}
