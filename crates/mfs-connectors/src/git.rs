use std::path::Path;

use git2::{ObjectType, Repository, Tree};

use crate::connector::{
    ConnectorCapabilities, ConnectorError, ResourceConnector, ResourceNode, SourceRef,
};

#[derive(Debug, Default, Clone, Copy)]
pub struct GitConnector;

impl GitConnector {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl ResourceConnector for GitConnector {
    fn capabilities(&self) -> ConnectorCapabilities {
        ConnectorCapabilities {
            can_enumerate: true,
            can_read_bytes: true,
            can_read_metadata: true,
            supports_snapshot_materialization: true,
        }
    }

    async fn enumerate(&self, source: &SourceRef) -> Result<Vec<ResourceNode>, ConnectorError> {
        if source.source_kind() != "git" {
            return Err(ConnectorError::InvalidSource(format!(
                "git connector cannot enumerate source kind '{}'",
                source.source_kind()
            )));
        }

        let repo = Repository::discover(source.identifier_path()).map_err(|err| {
            ConnectorError::InvalidSource(format!(
                "git source '{}' is not a repository: {err}",
                source.identifier_path().display()
            ))
        })?;
        let git_ref = source.git_ref();
        let tree = ref_tree(&repo, git_ref).map_err(|err| {
            ConnectorError::InvalidSource(format!(
                "git source '{}' has no readable tree at ref '{}': {err}",
                source.identifier_path().display(),
                git_ref,
            ))
        })?;

        let mut nodes = Vec::new();
        collect_tree_entries(&repo, &tree, Path::new(""), &mut nodes).map_err(|err| {
            ConnectorError::InvalidSource(format!(
                "failed to enumerate git tree for '{}' at '{}': {err}",
                source.identifier_path().display(),
                git_ref,
            ))
        })?;
        Ok(nodes)
    }

    async fn read_bytes(
        &self,
        source: &SourceRef,
        node: &ResourceNode,
    ) -> Result<Vec<u8>, ConnectorError> {
        if !node.is_file() {
            return Err(ConnectorError::Unsupported(format!(
                "git connector cannot read directory '{}'",
                node.relative_path().display()
            )));
        }

        let repo = Repository::discover(source.identifier_path()).map_err(|err| {
            ConnectorError::InvalidSource(format!(
                "git source '{}' is not a repository: {err}",
                source.identifier_path().display()
            ))
        })?;
        let git_ref = source.git_ref();
        let object = repo
            .revparse_single(&format!("{}:{}", git_ref, node.relative_path().display()))
            .map_err(|source| {
                ConnectorError::InvalidSource(format!(
                    "failed to resolve git object '{}' at ref '{}': {source}",
                    node.relative_path().display(),
                    git_ref,
                ))
            })?;
        let blob = object.peel_to_blob().map_err(|source| {
            ConnectorError::InvalidSource(format!(
                "git object '{}' is not a blob: {source}",
                node.relative_path().display()
            ))
        })?;
        Ok(blob.content().to_vec())
    }

    async fn snapshot_id(&self, source: &SourceRef) -> Result<String, ConnectorError> {
        if source.source_kind() != "git" {
            return Err(ConnectorError::InvalidSource(format!(
                "git connector cannot snapshot source kind '{}'",
                source.source_kind()
            )));
        }

        let repo = Repository::discover(source.identifier_path()).map_err(|err| {
            ConnectorError::InvalidSource(format!(
                "git source '{}' is not a repository: {err}",
                source.identifier_path().display()
            ))
        })?;
        let git_ref = source.git_ref();
        let object = repo.revparse_single(git_ref).map_err(|err| {
            ConnectorError::InvalidSource(format!(
                "git source '{}' has no readable ref '{}': {err}",
                source.identifier_path().display(),
                git_ref,
            ))
        })?;
        let commit = object.peel_to_commit().map_err(|err| {
            ConnectorError::InvalidSource(format!(
                "git source '{}' ref '{}' is not a commit: {err}",
                source.identifier_path().display(),
                git_ref,
            ))
        })?;

        Ok(commit.id().to_string())
    }
}

fn ref_tree<'a>(repo: &'a Repository, git_ref: &str) -> Result<Tree<'a>, git2::Error> {
    let object = repo.revparse_single(git_ref)?;
    let commit = object.peel_to_commit()?;
    commit.tree()
}

fn collect_tree_entries(
    repo: &Repository,
    tree: &Tree<'_>,
    prefix: &Path,
    nodes: &mut Vec<ResourceNode>,
) -> Result<(), git2::Error> {
    for entry in tree.iter() {
        let name = entry.name().unwrap_or_default();
        let relative = prefix.join(name);

        match entry.kind() {
            Some(ObjectType::Tree) => {
                nodes.push(ResourceNode::directory(relative.clone()));
                let subtree = repo.find_tree(entry.id())?;
                collect_tree_entries(repo, &subtree, &relative, nodes)?;
            }
            Some(ObjectType::Blob) => nodes.push(ResourceNode::file(relative)),
            _ => {}
        }
    }

    Ok(())
}
