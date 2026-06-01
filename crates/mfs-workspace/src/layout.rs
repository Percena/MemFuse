use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::path::{Path, PathBuf};

use mfs_types::IdentityContext;
use mfs_uri::{MfsUri, shorten_component};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceLayout {
    workspace_root: PathBuf,
}

impl WorkspaceLayout {
    pub fn new(workspace_root: impl AsRef<Path>) -> Self {
        Self {
            workspace_root: workspace_root.as_ref().to_path_buf(),
        }
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    pub fn materialized_resource_path(
        &self,
        identity: &IdentityContext,
        target_uri: &MfsUri,
    ) -> Result<PathBuf, WorkspaceLayoutError> {
        self.path_for_uri(identity, target_uri)
    }

    pub fn path_for_uri(
        &self,
        identity: &IdentityContext,
        uri: &MfsUri,
    ) -> Result<PathBuf, WorkspaceLayoutError> {
        let mut path = self.workspace_root.clone();

        path.push("tenants");
        path.push(identity.account_id());
        path.push(identity.user_id());

        match uri.root() {
            "resources" => path.push("resources"),
            "user" => path.push("user"),
            "agent" => {
                path.push("agent");
                path.push(identity.agent_space_name());
            }
            "session" => path.push("session"),
            root => return Err(WorkspaceLayoutError::UnsupportedRoot(root.to_owned())),
        }

        for segment in uri
            .canonical_path()
            .split('/')
            .filter(|segment| !segment.is_empty())
        {
            path.push(shorten_component(segment));
        }

        Ok(path)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceLayoutError {
    UnsupportedRoot(String),
}

impl Display for WorkspaceLayoutError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedRoot(root) => write!(
                f,
                "workspace materialization does not support MFS root '{root}'"
            ),
        }
    }
}

impl Error for WorkspaceLayoutError {}
