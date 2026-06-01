use std::path::PathBuf;
use std::time::UNIX_EPOCH;

use tokio::fs;

use crate::connector::{
    ConnectorCapabilities, ConnectorError, ResourceConnector, ResourceNode, ResourceNodeKind,
    SourceRef,
};

#[derive(Debug, Default, Clone, Copy)]
pub struct LocalFsConnector;

impl LocalFsConnector {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl ResourceConnector for LocalFsConnector {
    fn capabilities(&self) -> ConnectorCapabilities {
        ConnectorCapabilities {
            can_enumerate: true,
            can_read_bytes: true,
            can_read_metadata: true,
            supports_snapshot_materialization: true,
        }
    }

    async fn enumerate(&self, source: &SourceRef) -> Result<Vec<ResourceNode>, ConnectorError> {
        if source.source_kind() != "localfs" {
            return Err(ConnectorError::InvalidSource(format!(
                "localfs connector cannot enumerate source kind '{}'",
                source.source_kind()
            )));
        }

        let root = source.identifier_path().to_path_buf();
        let metadata = fs::metadata(&root)
            .await
            .map_err(|source| ConnectorError::io("stat source", &root, source))?;

        if metadata.is_file() {
            let file_name = root.file_name().ok_or_else(|| {
                ConnectorError::InvalidSource(format!(
                    "localfs file source '{}' is missing a file name",
                    root.display()
                ))
            })?;
            return Ok(vec![ResourceNode::file(PathBuf::from(file_name))]);
        }

        if !metadata.is_dir() {
            return Err(ConnectorError::InvalidSource(format!(
                "localfs source '{}' is not a directory or file",
                root.display()
            )));
        }

        let mut nodes = Vec::new();
        let mut pending = vec![root.clone()];

        while let Some(dir) = pending.pop() {
            let mut entries = fs::read_dir(&dir)
                .await
                .map_err(|source| ConnectorError::io("read directory", &dir, source))?;

            while let Some(entry) = entries
                .next_entry()
                .await
                .map_err(|source| ConnectorError::io("advance directory entry", &dir, source))?
            {
                let entry_path = entry.path();
                let relative_path = entry_path
                    .strip_prefix(&root)
                    .map_err(|_| {
                        ConnectorError::InvalidSource(format!(
                            "entry '{}' escaped localfs source '{}'",
                            entry_path.display(),
                            root.display()
                        ))
                    })?
                    .to_path_buf();
                let file_type = entry.file_type().await.map_err(|source| {
                    ConnectorError::io("inspect entry type", &entry_path, source)
                })?;

                if file_type.is_dir() {
                    pending.push(entry_path);
                    nodes.push(ResourceNode::directory(relative_path));
                } else if file_type.is_file() {
                    nodes.push(ResourceNode::file(relative_path));
                }
            }
        }

        nodes.sort_by(|left, right| {
            left.relative_path()
                .cmp(right.relative_path())
                .then_with(|| match (left.kind(), right.kind()) {
                    (ResourceNodeKind::Directory, ResourceNodeKind::File) => {
                        std::cmp::Ordering::Less
                    }
                    (ResourceNodeKind::File, ResourceNodeKind::Directory) => {
                        std::cmp::Ordering::Greater
                    }
                    _ => std::cmp::Ordering::Equal,
                })
        });

        Ok(nodes)
    }

    async fn read_bytes(
        &self,
        source: &SourceRef,
        node: &ResourceNode,
    ) -> Result<Vec<u8>, ConnectorError> {
        if !node.is_file() {
            return Err(ConnectorError::Unsupported(format!(
                "localfs connector cannot read directory '{}'",
                node.relative_path().display()
            )));
        }

        let full_path = resolve_node_path(source, node);
        fs::read(&full_path)
            .await
            .map_err(|source| ConnectorError::io("read file bytes", full_path, source))
    }

    async fn snapshot_id(&self, source: &SourceRef) -> Result<String, ConnectorError> {
        if source.source_kind() != "localfs" {
            return Err(ConnectorError::InvalidSource(format!(
                "localfs connector cannot snapshot source kind '{}'",
                source.source_kind()
            )));
        }

        let root = source.identifier_path().to_path_buf();
        let metadata = fs::metadata(&root)
            .await
            .map_err(|source| ConnectorError::io("stat source", &root, source))?;
        if metadata.is_file() {
            let modified = metadata
                .modified()
                .ok()
                .and_then(|mtime| mtime.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_nanos())
                .unwrap_or_default();
            let mut hash = 14695981039346656037_u64;
            update_hash(&mut hash, root.to_string_lossy().as_bytes());
            update_hash(&mut hash, &[0]);
            update_hash(&mut hash, &metadata.len().to_le_bytes());
            update_hash(&mut hash, &modified.to_le_bytes());
            return Ok(format!("localfs-{hash:016x}"));
        }

        if !metadata.is_dir() {
            return Err(ConnectorError::InvalidSource(format!(
                "localfs source '{}' is not a directory or file",
                root.display()
            )));
        }

        let mut stack = vec![root.clone()];
        let mut signatures = Vec::new();
        while let Some(dir) = stack.pop() {
            let mut entries = fs::read_dir(&dir)
                .await
                .map_err(|source| ConnectorError::io("read directory", &dir, source))?;
            while let Some(entry) = entries
                .next_entry()
                .await
                .map_err(|source| ConnectorError::io("advance directory entry", &dir, source))?
            {
                let entry_path = entry.path();
                let relative_path = entry_path
                    .strip_prefix(&root)
                    .map_err(|_| {
                        ConnectorError::InvalidSource(format!(
                            "entry '{}' escaped localfs source '{}'",
                            entry_path.display(),
                            root.display()
                        ))
                    })?
                    .to_path_buf();
                let metadata = entry.metadata().await.map_err(|source| {
                    ConnectorError::io("read entry metadata", &entry_path, source)
                })?;
                let is_dir = metadata.is_dir();
                if is_dir {
                    stack.push(entry_path.clone());
                }
                let modified = metadata
                    .modified()
                    .ok()
                    .and_then(|mtime| mtime.duration_since(UNIX_EPOCH).ok())
                    .map(|duration| duration.as_nanos())
                    .unwrap_or_default();
                signatures.push((
                    relative_path.to_string_lossy().replace('\\', "/"),
                    is_dir,
                    metadata.len(),
                    modified,
                ));
            }
        }

        signatures.sort_by(|left, right| left.0.cmp(&right.0));
        let mut hash = 14695981039346656037_u64;
        for (path, is_dir, len, modified) in signatures {
            update_hash(&mut hash, path.as_bytes());
            update_hash(&mut hash, &[u8::from(is_dir)]);
            update_hash(&mut hash, &len.to_le_bytes());
            update_hash(&mut hash, &modified.to_le_bytes());
        }

        Ok(format!("localfs-{hash:016x}"))
    }
}

fn resolve_node_path(source: &SourceRef, node: &ResourceNode) -> PathBuf {
    let identifier = source.identifier_path();
    if identifier.is_file() {
        identifier.to_path_buf()
    } else {
        identifier.join(node.relative_path())
    }
}

fn update_hash(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(1099511628211);
    }
}
