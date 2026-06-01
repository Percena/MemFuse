use std::collections::HashSet;
use std::error::Error;
use std::path::Path;
use std::time::Duration;

use mfs_metadata::MetadataStore;
use mfs_types::IdentityContext;
use serde::Serialize;

use crate::resource::refresh::refresh_registered_resource;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResourceWatchRunResult {
    pub resource_id: String,
    pub refreshed: bool,
    pub root_uri: String,
    pub snapshot_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResourceWatchLoopResult {
    pub iterations: usize,
    pub total_runs: usize,
    pub runs: Vec<ResourceWatchRunResult>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResourceWatchStatus {
    pub resource_id: String,
    pub interval_seconds: u32,
    pub enabled: bool,
    pub due: bool,
    pub last_checked_at: Option<String>,
    pub last_refreshed_at: Option<String>,
}

pub fn register_resource_watch(
    metadata: &MetadataStore,
    identity: &IdentityContext,
    resource_id: &str,
    interval_seconds: u32,
) -> Result<mfs_metadata::StoredResourceWatch, Box<dyn Error>> {
    let source = metadata
        .get_resource_source(resource_id)?
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "resource not found"))?;
    metadata.upsert_resource_watch(&mfs_metadata::ResourceWatchRecord {
        account_id: identity.account_id(),
        user_id: identity.user_id(),
        agent_id: Some(identity.agent_id()),
        resource_id,
        interval_seconds,
        enabled: true,
    })?;
    metadata
        .list_resource_watches(identity.account_id(), identity.user_id(), 100)?
        .into_iter()
        .find(|watch| watch.resource_id == source.resource_id)
        .ok_or_else(|| std::io::Error::other("resource watch was not persisted").into())
}

pub fn list_resource_watches(
    metadata: &MetadataStore,
    account_id: &str,
    user_id: &str,
    limit: usize,
) -> Result<Vec<mfs_metadata::StoredResourceWatch>, Box<dyn Error>> {
    Ok(metadata.list_resource_watches(account_id, user_id, limit)?)
}

pub fn list_resource_watch_statuses(
    metadata: &MetadataStore,
    account_id: &str,
    user_id: &str,
    limit: usize,
) -> Result<Vec<ResourceWatchStatus>, Box<dyn Error>> {
    let due_ids = metadata
        .list_due_resource_watches(account_id, user_id, limit)?
        .into_iter()
        .map(|watch| watch.resource_id)
        .collect::<HashSet<_>>();
    Ok(metadata
        .list_resource_watches(account_id, user_id, limit)?
        .into_iter()
        .map(|watch| ResourceWatchStatus {
            due: due_ids.contains(&watch.resource_id),
            resource_id: watch.resource_id,
            interval_seconds: watch.interval_seconds,
            enabled: watch.enabled,
            last_checked_at: watch.last_checked_at,
            last_refreshed_at: watch.last_refreshed_at,
        })
        .collect())
}

pub fn disable_resource_watch(
    metadata: &MetadataStore,
    account_id: &str,
    user_id: &str,
    resource_id: &str,
) -> Result<mfs_metadata::StoredResourceWatch, Box<dyn Error>> {
    metadata.set_resource_watch_enabled(resource_id, false)?;
    metadata
        .list_resource_watches(account_id, user_id, 100)?
        .into_iter()
        .find(|watch| watch.resource_id == resource_id)
        .ok_or_else(|| std::io::Error::other("resource watch was not updated").into())
}

pub async fn run_resource_watch(
    metadata: &MetadataStore,
    workspace_root: &Path,
    identity: &IdentityContext,
    resource_id: &str,
) -> Result<ResourceWatchRunResult, Box<dyn Error>> {
    let watch_exists = metadata
        .list_resource_watches(identity.account_id(), identity.user_id(), 100)?
        .into_iter()
        .any(|watch| watch.resource_id == resource_id && watch.enabled);
    if !watch_exists {
        return Err(std::io::Error::new(std::io::ErrorKind::NotFound, "watch not found").into());
    }

    let refresh =
        refresh_registered_resource(metadata, workspace_root, identity, resource_id).await?;
    metadata.mark_resource_watch_run(resource_id, true)?;

    Ok(ResourceWatchRunResult {
        resource_id: resource_id.to_owned(),
        refreshed: true,
        root_uri: refresh.root_uri,
        snapshot_id: Some(refresh.snapshot_id),
    })
}

pub async fn run_due_resource_watches(
    metadata: &MetadataStore,
    workspace_root: &Path,
    identity: &IdentityContext,
    limit: usize,
) -> Result<Vec<ResourceWatchRunResult>, Box<dyn Error>> {
    let due =
        metadata.list_due_resource_watches(identity.account_id(), identity.user_id(), limit)?;
    let mut runs = Vec::new();
    for watch in due {
        runs.push(
            run_resource_watch(metadata, workspace_root, identity, &watch.resource_id).await?,
        );
    }
    Ok(runs)
}

pub async fn run_resource_watch_loop(
    metadata: &MetadataStore,
    workspace_root: &Path,
    identity: &IdentityContext,
    iterations: usize,
    sleep_duration: Duration,
    limit: usize,
) -> Result<ResourceWatchLoopResult, Box<dyn Error>> {
    let mut runs = Vec::new();
    for index in 0..iterations {
        runs.extend(run_due_resource_watches(metadata, workspace_root, identity, limit).await?);
        if index + 1 < iterations {
            tokio::time::sleep(sleep_duration).await;
        }
    }
    Ok(ResourceWatchLoopResult {
        iterations,
        total_runs: runs.len(),
        runs,
    })
}
