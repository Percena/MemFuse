use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use mfs_metadata::MetadataStore;
use mfs_types::IdentityContext;
use mfs_uri::MfsUri;
use mfs_workspace::{Materializer, WorkspaceLayout, content_digest_for_path, is_summary_sidecar};

use super::rebuild::{rebuild_metadata_entries_with_provenance, reprocess_semantic_root};
use super::{
    ManagedResourceRefreshResult, RefreshResult, join_uri, record_resource_changes,
    remove_file_sidecars, snapshot_record,
};

// ---------------------------------------------------------------------------
// Public API — refresh
// ---------------------------------------------------------------------------

pub async fn refresh_projection(
    metadata: &MetadataStore,
    workspace_root: &Path,
    identity: &IdentityContext,
    source_kind: &str,
    source_path: &str,
    target_uri: &str,
) -> Result<RefreshResult, Box<dyn std::error::Error>> {
    let materialized = stage_refresh_materialization(
        workspace_root,
        identity,
        source_kind,
        source_path,
        target_uri,
    )
    .await?;
    let live_target =
        WorkspaceLayout::new(workspace_root).path_for_uri(identity, &MfsUri::parse(target_uri)?)?;
    let changes = sync_staged_resource_root(
        &materialized.target_path,
        &live_target,
        &materialized.provenance.target_uri,
    )?;
    metadata.clear_refresh_scope(
        &materialized.provenance.projection_view_id,
        &materialized.provenance.target_uri,
    )?;
    reprocess_semantic_root(
        workspace_root,
        &materialized.provenance.projection_view_id,
        &live_target,
        &materialized.provenance.target_uri,
        "resource",
    )
    .await?;
    let rebuild = rebuild_metadata_entries_with_provenance(
        metadata,
        identity,
        &live_target,
        &materialized.provenance.target_uri,
        Some(&materialized.provenance),
    )?;
    metadata.append_snapshot(&snapshot_record(&materialized.provenance))?;
    record_resource_changes(
        metadata,
        identity,
        None,
        &materialized.provenance.source_snapshot_id,
        &changes,
    )?;

    Ok(RefreshResult {
        indexed_paths: rebuild.indexed_paths,
        projection_uri: rebuild.projection_uri,
        projection_root: live_target,
        snapshot_id: materialized.provenance.source_snapshot_id,
    })
}

pub async fn refresh_registered_resource(
    metadata: &MetadataStore,
    workspace_root: &Path,
    identity: &IdentityContext,
    resource_id: &str,
) -> Result<ManagedResourceRefreshResult, Box<dyn std::error::Error>> {
    let source = metadata
        .get_resource_source(resource_id)?
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "resource not found"))?;

    let materialized = stage_refresh_materialization(
        workspace_root,
        identity,
        &source.source_kind,
        &source.source_identifier,
        &source.canonical_root_uri,
    )
    .await?;
    let target_path = WorkspaceLayout::new(workspace_root)
        .path_for_uri(identity, &MfsUri::parse(&source.canonical_root_uri)?)?;
    let changes = sync_staged_resource_root(
        &materialized.target_path,
        &target_path,
        &source.canonical_root_uri,
    )?;
    metadata.clear_refresh_scope(
        &materialized.provenance.projection_view_id,
        &materialized.provenance.target_uri,
    )?;
    let report = reprocess_semantic_root(
        workspace_root,
        &materialized.provenance.projection_view_id,
        &target_path,
        &materialized.provenance.target_uri,
        "resource",
    )
    .await?;
    let _ = rebuild_metadata_entries_with_provenance(
        metadata,
        identity,
        &target_path,
        &materialized.provenance.target_uri,
        Some(&materialized.provenance),
    )?;
    metadata.append_snapshot(&snapshot_record(&materialized.provenance))?;
    record_resource_changes(
        metadata,
        identity,
        Some(resource_id),
        &materialized.provenance.source_snapshot_id,
        &changes,
    )?;
    metadata.register_resource_source(&mfs_metadata::ResourceSourceRecord {
        resource_id: &source.resource_id,
        account_id: &source.account_id,
        user_id: &source.user_id,
        agent_id: source.agent_id.as_deref(),
        logical_name: &source.logical_name,
        source_kind: &source.source_kind,
        source_identifier: &source.source_identifier,
        canonical_root_uri: &source.canonical_root_uri,
        projection_view_id: &source.projection_view_id,
        resource_kind: &source.resource_kind,
        source_host: source.source_host.as_deref(),
        source_namespace: source.source_namespace.as_deref(),
        source_repo: source.source_repo.as_deref(),
        source_ref: source.source_ref.as_deref(),
        canonical_strategy_version: &source.canonical_strategy_version,
        status: "ready",
        last_snapshot_id: Some(&materialized.provenance.source_snapshot_id),
    })?;

    Ok(ManagedResourceRefreshResult {
        resource_id: source.resource_id,
        root_uri: materialized.provenance.target_uri,
        indexed_documents: report.indexed_documents,
        mode: format!("{:?}", report.mode).to_ascii_lowercase(),
        snapshot_id: materialized.provenance.source_snapshot_id,
    })
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub(super) struct ProjectionSnapshotEntry {
    is_dir: bool,
    digest: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ProjectionChange {
    pub(super) uri: String,
    pub(super) change_type: &'static str,
    pub(super) content_digest: Option<String>,
}

async fn stage_refresh_materialization(
    workspace_root: &Path,
    identity: &IdentityContext,
    source_kind: &str,
    source_path: &str,
    target_uri: &str,
) -> Result<mfs_workspace::MaterializationResult, Box<dyn std::error::Error>> {
    let staging = std::sync::Arc::new(tempfile::tempdir()?);
    let materializer = Materializer::with_temp_workspace(staging);
    let staged = match source_kind {
        "localfs" | "inline" | "import" => {
            materializer
                .materialize_localfs_as(identity, source_path, target_uri, source_kind)
                .await?
        }
        "git" => {
            materializer
                .materialize_git(identity, source_path, target_uri)
                .await?
        }
        "git_url" => {
            let staged_git_url =
                super::ingest::stage_git_url_source(workspace_root, source_path, None).await?;
            materializer
                .materialize_localfs_as(
                    identity,
                    staged_git_url.to_str().expect("staged git_url path utf-8"),
                    target_uri,
                    "git_url",
                )
                .await?
        }
        "url" => {
            let staged_url = super::ingest::stage_url_source(workspace_root, source_path).await?;
            materializer
                .materialize_localfs_as(
                    identity,
                    staged_url.to_str().expect("staged url path utf-8"),
                    target_uri,
                    "url",
                )
                .await?
        }
        other => {
            return Err(std::io::Error::other(format!(
                "refresh does not support source kind '{other}'"
            ))
            .into());
        }
    };
    Ok(staged)
}

fn sync_staged_resource_root(
    staged_root: &Path,
    live_root: &Path,
    root_uri: &str,
) -> Result<Vec<ProjectionChange>, Box<dyn std::error::Error>> {
    let staged = collect_projection_snapshot(staged_root)?;
    let live = collect_projection_snapshot(live_root)?;
    fs::create_dir_all(live_root)?;

    let mut changes = Vec::new();
    let mut removed = live
        .keys()
        .filter(|key| !staged.contains_key(*key))
        .cloned()
        .collect::<Vec<_>>();
    removed.sort_by_key(|item| std::cmp::Reverse(item.matches('/').count()));
    for relative in removed {
        let live_path = live_root.join(&relative);
        if live.get(&relative).map(|item| item.is_dir).unwrap_or(false) {
            if live_path.exists() {
                fs::remove_dir_all(&live_path)?;
            }
            continue;
        }
        if live_path.exists() {
            fs::remove_file(&live_path)?;
        }
        remove_file_sidecars(&live_path)?;
        changes.push(ProjectionChange {
            uri: join_uri(root_uri, &relative),
            change_type: "deleted",
            content_digest: None,
        });
    }

    let mut added_or_updated = staged.keys().cloned().collect::<Vec<_>>();
    added_or_updated.sort_by_key(|item| (item.matches('/').count(), item.clone()));
    for relative in added_or_updated {
        let staged_entry = staged.get(&relative).expect("staged entry exists");
        let live_entry = live.get(&relative);
        let staged_path = staged_root.join(&relative);
        let live_path = live_root.join(&relative);

        if staged_entry.is_dir {
            if live_entry.map(|entry| !entry.is_dir).unwrap_or(false) {
                if live_path.exists() {
                    fs::remove_file(&live_path)?;
                }
                remove_file_sidecars(&live_path)?;
            }
            fs::create_dir_all(&live_path)?;
            continue;
        }

        if live_entry.map(|entry| entry.is_dir).unwrap_or(false) && live_path.exists() {
            fs::remove_dir_all(&live_path)?;
        }

        let changed =
            live_entry.map(|entry| entry.digest.as_ref()) != Some(staged_entry.digest.as_ref());
        if !changed {
            continue;
        }
        if let Some(parent) = live_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(&staged_path, &live_path)?;
        changes.push(ProjectionChange {
            uri: join_uri(root_uri, &relative),
            change_type: if live_entry.is_some() {
                "modified"
            } else {
                "added"
            },
            content_digest: staged_entry.digest.clone(),
        });
    }

    Ok(changes)
}

fn collect_projection_snapshot(
    root: &Path,
) -> Result<BTreeMap<String, ProjectionSnapshotEntry>, Box<dyn std::error::Error>> {
    let mut snapshot = BTreeMap::new();
    if !root.exists() {
        return Ok(snapshot);
    }
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        for entry in fs::read_dir(&path)? {
            let entry = entry?;
            let entry_path = entry.path();
            let relative = entry_path
                .strip_prefix(root)?
                .to_string_lossy()
                .replace('\\', "/");
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                snapshot.insert(
                    relative.clone(),
                    ProjectionSnapshotEntry {
                        is_dir: true,
                        digest: None,
                    },
                );
                stack.push(entry_path);
                continue;
            }
            if is_summary_sidecar(&entry_path) {
                continue;
            }
            snapshot.insert(
                relative,
                ProjectionSnapshotEntry {
                    is_dir: false,
                    digest: content_digest_for_path(&entry_path).ok(),
                },
            );
        }
    }
    Ok(snapshot)
}
