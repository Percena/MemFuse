use std::env;
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use mfs_connectors::{
    ConnectorError, GitConnector, LocalFsConnector, ResourceConnector, ResourceNodeKind, SourceRef,
};
use mfs_uri::{MfsUri, UriError};
use tempfile::TempDir;
use tokio::fs;

use crate::classify::should_skip_path;
use crate::layout::{WorkspaceLayout, WorkspaceLayoutError};
use crate::summaries::{SummaryError, write_layered_summaries};
use mfs_types::IdentityContext;

#[derive(Debug)]
pub struct Materializer {
    layout: WorkspaceLayout,
    source_base: PathBuf,
    workspace_guard: Option<Arc<TempDir>>,
}

impl Materializer {
    pub fn new(workspace_root: impl AsRef<Path>) -> Self {
        Self {
            layout: WorkspaceLayout::new(workspace_root),
            source_base: env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            workspace_guard: None,
        }
    }

    pub fn for_tests() -> Self {
        let workspace_guard =
            Arc::new(tempfile::tempdir().expect("failed to create temporary canonical workspace"));
        let source_base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");

        Self {
            layout: WorkspaceLayout::new(workspace_guard.path()),
            source_base,
            workspace_guard: Some(workspace_guard),
        }
    }

    pub fn with_temp_workspace(workspace_guard: Arc<TempDir>) -> Self {
        Self {
            layout: WorkspaceLayout::new(workspace_guard.path()),
            source_base: env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            workspace_guard: Some(workspace_guard),
        }
    }

    pub async fn reset_projection_root(
        &self,
        identity: &IdentityContext,
        target_uri: &str,
    ) -> Result<PathBuf, MaterializeError> {
        let target_uri = MfsUri::parse(target_uri).map_err(MaterializeError::Uri)?;
        let target_path = self
            .layout
            .materialized_resource_path(identity, &target_uri)
            .map_err(MaterializeError::Layout)?;

        if fs::try_exists(&target_path)
            .await
            .map_err(|source| MaterializeError::io("check projection root", &target_path, source))?
        {
            fs::remove_dir_all(&target_path).await.map_err(|source| {
                MaterializeError::io("remove existing projection root", &target_path, source)
            })?;
        }

        Ok(target_path)
    }

    pub async fn materialize_localfs(
        &self,
        identity: &IdentityContext,
        source_path: &str,
        target_uri: &str,
    ) -> Result<MaterializationResult, MaterializeError> {
        self.materialize_localfs_as(identity, source_path, target_uri, "localfs")
            .await
    }

    pub async fn materialize_localfs_as(
        &self,
        identity: &IdentityContext,
        source_path: &str,
        target_uri: &str,
        provenance_source_kind: &str,
    ) -> Result<MaterializationResult, MaterializeError> {
        self.materialize_with_connector(
            identity,
            source_path,
            target_uri,
            provenance_source_kind,
            "localfs",
            LocalFsConnector::new(),
            None,
            None,
        )
        .await
    }

    pub async fn materialize_git(
        &self,
        identity: &IdentityContext,
        source_path: &str,
        target_uri: &str,
    ) -> Result<MaterializationResult, MaterializeError> {
        self.materialize_git_with_ref(identity, source_path, target_uri, None, None)
            .await
    }

    /// Materialize a git resource with optional branch/revision ref parameters.
    /// When `branch` or `revision` is specified, the connector will resolve the
    /// git ref accordingly instead of using HEAD.
    pub async fn materialize_git_with_ref(
        &self,
        identity: &IdentityContext,
        source_path: &str,
        target_uri: &str,
        branch: Option<&str>,
        revision: Option<&str>,
    ) -> Result<MaterializationResult, MaterializeError> {
        self.materialize_with_connector(
            identity,
            source_path,
            target_uri,
            "git",
            "git",
            GitConnector::new(),
            branch,
            revision,
        )
        .await
    }

    async fn materialize_with_connector<C>(
        &self,
        identity: &IdentityContext,
        source_path: &str,
        target_uri: &str,
        provenance_source_kind: &str,
        connector_source_kind: &str,
        connector: C,
        branch: Option<&str>,
        revision: Option<&str>,
    ) -> Result<MaterializationResult, MaterializeError>
    where
        C: ResourceConnector,
    {
        let target_uri = MfsUri::parse(target_uri).map_err(MaterializeError::Uri)?;
        let target_path = self
            .layout
            .materialized_resource_path(identity, &target_uri)
            .map_err(MaterializeError::Layout)?;
        let resolved_source = self.resolve_source_path(source_path);

        // Guard against source being the workspace root itself — prevents
        // infinite recursion when LocalFsConnector enumerates the entire
        // workspace including tenants/ and the target projection directory.
        let workspace_root = self.layout.workspace_root();
        if resolved_source == workspace_root {
            return Err(MaterializeError::Overlap {
                source: resolved_source,
                workspace: workspace_root.to_path_buf(),
            });
        }
        let mut source = SourceRef::new(
            connector_source_kind,
            resolved_source.to_string_lossy().into_owned(),
        );
        if let Some(branch) = branch {
            source = source.with_branch(branch);
        }
        if let Some(revision) = revision {
            source = source.with_revision(revision);
        }
        let source_snapshot_id = connector
            .snapshot_id(&source)
            .await
            .map_err(MaterializeError::Connector)?;
        let nodes = connector
            .enumerate(&source)
            .await
            .map_err(MaterializeError::Connector)?;
        let canonical_uri = format!(
            "mfs://{}/{}",
            target_uri.root(),
            target_uri.canonical_path()
        )
        .trim_end_matches('/')
        .to_owned();

        fs::create_dir_all(&target_path).await.map_err(|source| {
            MaterializeError::io("create target directory", &target_path, source)
        })?;

        for node in nodes {
            if should_skip_path(
                node.relative_path(),
                node.kind() == ResourceNodeKind::Directory,
            ) {
                continue;
            }
            let destination = target_path.join(node.relative_path());

            match node.kind() {
                ResourceNodeKind::Directory => {
                    fs::create_dir_all(&destination).await.map_err(|source| {
                        MaterializeError::io("create materialized directory", &destination, source)
                    })?;
                }
                ResourceNodeKind::File => {
                    if let Some(parent) = destination.parent() {
                        fs::create_dir_all(parent).await.map_err(|source| {
                            MaterializeError::io("create file parent directory", parent, source)
                        })?;
                    }

                    let bytes = connector
                        .read_bytes(&source, &node)
                        .await
                        .map_err(MaterializeError::Connector)?;
                    fs::write(&destination, bytes).await.map_err(|source| {
                        MaterializeError::io("write materialized file", &destination, source)
                    })?;
                }
            }
        }

        for directory in collect_directories(&target_path).await.map_err(|source| {
            MaterializeError::io("collect materialized directories", &target_path, source)
        })? {
            let directory_uri = if directory == target_path {
                canonical_uri.clone()
            } else {
                let relative = directory
                    .strip_prefix(&target_path)
                    .unwrap_or(&directory)
                    .to_string_lossy()
                    .replace('\\', "/");
                format!("{}/{}", canonical_uri.trim_end_matches('/'), relative)
            };
            write_layered_summaries(&directory, &directory_uri)
                .map_err(MaterializeError::Summary)?;
        }

        Ok(MaterializationResult {
            workspace_root: self.layout.workspace_root().to_path_buf(),
            target_path,
            provenance: SourceProvenance {
                source_kind: provenance_source_kind.to_owned(),
                source_identifier: resolved_source.to_string_lossy().into_owned(),
                source_snapshot_id,
                projection_view_id: format!(
                    "tenant:{}:{}:{}",
                    identity.account_id(),
                    identity.user_id(),
                    target_uri.root()
                ),
                materialization_mode: "snapshot".to_owned(),
                target_uri: canonical_uri,
            },
            _workspace_guard: self.workspace_guard.clone(),
        })
    }

    fn resolve_source_path(&self, source_path: &str) -> PathBuf {
        let source_path = PathBuf::from(source_path);

        if source_path.is_absolute() {
            source_path
        } else {
            self.source_base.join(source_path)
        }
    }
}

async fn collect_directories(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut stack = vec![root.to_path_buf()];
    let mut directories = Vec::new();

    while let Some(path) = stack.pop() {
        directories.push(path.clone());
        let mut entries = fs::read_dir(&path).await?;
        while let Some(entry) = entries.next_entry().await? {
            if entry.file_type().await?.is_dir() {
                stack.push(entry.path());
            }
        }
    }

    directories.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    Ok(directories)
}

#[derive(Debug, Clone)]
pub struct SourceProvenance {
    pub source_kind: String,
    pub source_identifier: String,
    pub source_snapshot_id: String,
    pub projection_view_id: String,
    pub materialization_mode: String,
    pub target_uri: String,
}

#[derive(Debug)]
pub struct MaterializationResult {
    pub workspace_root: PathBuf,
    pub target_path: PathBuf,
    pub provenance: SourceProvenance,
    _workspace_guard: Option<Arc<TempDir>>,
}

impl MaterializationResult {
    pub(crate) fn managed(
        workspace_root: PathBuf,
        target_path: PathBuf,
        provenance: SourceProvenance,
    ) -> Self {
        Self {
            workspace_root,
            target_path,
            provenance,
            _workspace_guard: None,
        }
    }
}

#[derive(Debug)]
pub enum MaterializeError {
    Uri(UriError),
    Layout(WorkspaceLayoutError),
    Connector(ConnectorError),
    Summary(SummaryError),
    Overlap {
        source: PathBuf,
        workspace: PathBuf,
    },
    Io {
        action: &'static str,
        path: PathBuf,
        source: io::Error,
    },
}

impl MaterializeError {
    fn io(action: &'static str, path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::Io {
            action,
            path: path.into(),
            source,
        }
    }
}

impl Display for MaterializeError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Uri(source) => write!(f, "invalid MFS target URI: {source}"),
            Self::Layout(source) => write!(f, "invalid workspace target: {source}"),
            Self::Connector(source) => write!(f, "connector materialization failed: {source}"),
            Self::Summary(source) => write!(f, "summary generation failed: {source}"),
            Self::Overlap { source, workspace } => write!(
                f,
                "source path '{}' overlaps with workspace root '{}' — this causes infinite recursion; use a separate directory outside the workspace",
                source.display(),
                workspace.display()
            ),
            Self::Io {
                action,
                path,
                source,
            } => write!(
                f,
                "materializer failed to {action} '{}': {source}",
                path.display()
            ),
        }
    }
}

impl Error for MaterializeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Uri(source) => Some(source),
            Self::Layout(source) => Some(source),
            Self::Connector(source) => Some(source),
            Self::Summary(source) => Some(source),
            Self::Overlap { .. } => None,
            Self::Io { source, .. } => Some(source),
        }
    }
}
