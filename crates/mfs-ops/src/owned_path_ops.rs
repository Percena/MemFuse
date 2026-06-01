use std::error::Error;
use std::path::Path;
use std::time::Duration;

use mfs_metadata::MetadataStore;
use mfs_types::IdentityContext;
use mfs_uri::MfsUri;
use mfs_workspace::{WorkspaceFs, WorkspaceLayout};

use crate::projection_view_id_for_uri;
use crate::resource::rebuild::{rebuild_metadata_entries_with_provenance, reprocess_semantic_root};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedPathMutationResult {
    pub primary_uri: String,
    pub indexed_paths: usize,
    pub scopes_reindexed: Vec<String>,
}

pub async fn mkdir_owned_path(
    metadata: &MetadataStore,
    workspace_root: &Path,
    identity: &IdentityContext,
    uri: &str,
) -> Result<OwnedPathMutationResult, Box<dyn Error>> {
    let _lock = acquire_owned_write_lock(workspace_root, identity).await?;
    let fs = WorkspaceFs::open_existing_for_uri(
        workspace_root,
        identity.account_id(),
        identity.user_id(),
        identity.agent_id(),
        Some(uri),
    )?;
    fs.mkdir(uri).await?;
    let (indexed_paths, scopes_reindexed) =
        sync_owned_scopes(metadata, workspace_root, identity, &[uri]).await?;
    Ok(OwnedPathMutationResult {
        primary_uri: uri.to_owned(),
        indexed_paths,
        scopes_reindexed,
    })
}

pub async fn write_owned_path(
    metadata: &MetadataStore,
    workspace_root: &Path,
    identity: &IdentityContext,
    uri: &str,
    content: &str,
) -> Result<OwnedPathMutationResult, Box<dyn Error>> {
    let _lock = acquire_owned_write_lock(workspace_root, identity).await?;
    let fs = WorkspaceFs::open_existing_for_uri(
        workspace_root,
        identity.account_id(),
        identity.user_id(),
        identity.agent_id(),
        Some(uri),
    )?;
    fs.write_text(uri, content).await?;
    let (indexed_paths, scopes_reindexed) =
        sync_owned_scopes(metadata, workspace_root, identity, &[uri]).await?;
    Ok(OwnedPathMutationResult {
        primary_uri: uri.to_owned(),
        indexed_paths,
        scopes_reindexed,
    })
}

pub async fn move_owned_path(
    metadata: &MetadataStore,
    workspace_root: &Path,
    identity: &IdentityContext,
    from_uri: &str,
    to_uri: &str,
) -> Result<OwnedPathMutationResult, Box<dyn Error>> {
    let _lock = acquire_owned_write_lock(workspace_root, identity).await?;
    let fs = WorkspaceFs::open_existing_for_uri(
        workspace_root,
        identity.account_id(),
        identity.user_id(),
        identity.agent_id(),
        Some(from_uri),
    )?;
    fs.move_path(from_uri, to_uri).await?;
    let (indexed_paths, scopes_reindexed) =
        sync_owned_scopes(metadata, workspace_root, identity, &[from_uri, to_uri]).await?;
    Ok(OwnedPathMutationResult {
        primary_uri: to_uri.to_owned(),
        indexed_paths,
        scopes_reindexed,
    })
}

pub async fn remove_owned_path(
    metadata: &MetadataStore,
    workspace_root: &Path,
    identity: &IdentityContext,
    uri: &str,
) -> Result<OwnedPathMutationResult, Box<dyn Error>> {
    let _lock = acquire_owned_write_lock(workspace_root, identity).await?;
    let fs = WorkspaceFs::open_existing_for_uri(
        workspace_root,
        identity.account_id(),
        identity.user_id(),
        identity.agent_id(),
        Some(uri),
    )?;
    fs.remove_path(uri).await?;
    let (indexed_paths, scopes_reindexed) =
        sync_owned_scopes(metadata, workspace_root, identity, &[uri]).await?;
    Ok(OwnedPathMutationResult {
        primary_uri: uri.to_owned(),
        indexed_paths,
        scopes_reindexed,
    })
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
struct OwnedScope {
    uri: String,
    context_type: &'static str,
}

struct OwnedWriteLock {
    lock_path: std::path::PathBuf,
}

impl Drop for OwnedWriteLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.lock_path);
    }
}

async fn acquire_owned_write_lock(
    workspace_root: &Path,
    identity: &IdentityContext,
) -> Result<OwnedWriteLock, Box<dyn Error>> {
    let lock_path = workspace_root.join("_system").join("locks").join(format!(
        "{}-{}-owned-write.lock",
        identity.account_id(),
        identity.user_id()
    ));
    if let Some(parent) = lock_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let started = std::time::Instant::now();
    loop {
        match tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
            .await
        {
            Ok(_) => return Ok(OwnedWriteLock { lock_path }),
            Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {
                if started.elapsed() >= Duration::from_secs(5) {
                    return Err(std::io::Error::other(format!(
                        "timed out waiting for owned write lock '{}'",
                        lock_path.display()
                    ))
                    .into());
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            Err(source) => return Err(source.into()),
        }
    }
}

async fn sync_owned_scopes(
    metadata: &MetadataStore,
    workspace_root: &Path,
    identity: &IdentityContext,
    uris: &[&str],
) -> Result<(usize, Vec<String>), Box<dyn Error>> {
    let mut scopes = uris
        .iter()
        .map(|uri| owned_scope_for_uri(uri))
        .collect::<Result<Vec<_>, _>>()?;
    scopes.sort_by(|left, right| left.uri.cmp(&right.uri));
    scopes.dedup_by(|left, right| left.uri == right.uri);

    let mut indexed_paths = 0;
    let mut scope_uris = Vec::new();
    for scope in scopes {
        indexed_paths += sync_owned_scope(metadata, workspace_root, identity, &scope).await?;
        scope_uris.push(scope.uri);
    }
    Ok((indexed_paths, scope_uris))
}

async fn sync_owned_scope(
    metadata: &MetadataStore,
    workspace_root: &Path,
    identity: &IdentityContext,
    scope: &OwnedScope,
) -> Result<usize, Box<dyn Error>> {
    let projection_view_id = projection_view_id_for_uri(identity, &scope.uri);
    metadata.clear_refresh_scope(&projection_view_id, &scope.uri)?;

    let scope_path =
        WorkspaceLayout::new(workspace_root).path_for_uri(identity, &MfsUri::parse(&scope.uri)?)?;
    if !tokio::fs::try_exists(&scope_path).await? {
        return Ok(0);
    }

    reprocess_semantic_root(
        workspace_root,
        &projection_view_id,
        &scope_path,
        &scope.uri,
        scope.context_type,
    )
    .await?;
    let rebuild = rebuild_metadata_entries_with_provenance(
        metadata,
        identity,
        &scope_path,
        &scope.uri,
        None,
    )?;
    Ok(rebuild.indexed_paths)
}

fn owned_scope_for_uri(uri: &str) -> Result<OwnedScope, Box<dyn Error>> {
    let parsed = MfsUri::parse(uri)?;
    let first_segment = parsed
        .canonical_path()
        .split('/')
        .find(|segment| !segment.is_empty());
    match parsed.root() {
        "user" => Ok(OwnedScope {
            uri: first_segment
                .map(|segment| format!("mfs://user/{segment}"))
                .unwrap_or_else(|| "mfs://user".to_owned()),
            context_type: "memory",
        }),
        "agent" => Ok(OwnedScope {
            uri: first_segment
                .map(|segment| format!("mfs://agent/{segment}"))
                .unwrap_or_else(|| "mfs://agent".to_owned()),
            context_type: if first_segment == Some("skills") {
                "skill"
            } else {
                "resource"
            },
        }),
        root => Err(std::io::Error::other(format!(
            "MemFuse-owned write plane does not support root '{root}'"
        ))
        .into()),
    }
}
