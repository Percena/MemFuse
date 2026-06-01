pub mod ingest;
pub mod pack;
pub mod rebuild;
pub mod refresh;

use std::fs;
use std::path::Path;

use mfs_metadata::{MetadataStore, SnapshotRecord};
use mfs_types::IdentityContext;
use mfs_uri::short_hash_hex;
use mfs_workspace::SourceProvenance;
use serde::{Deserialize, Serialize};

use crate::projection_component;

// ---------------------------------------------------------------------------
// Public result types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebuildResult {
    pub indexed_paths: usize,
    pub projection_uri: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefreshResult {
    pub indexed_paths: usize,
    pub projection_uri: String,
    pub projection_root: std::path::PathBuf,
    pub snapshot_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceIngestResult {
    pub task_key: String,
    pub resource_id: String,
    pub logical_name: String,
    pub root_uri: String,
    pub indexed_documents: usize,
    pub mode: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedResourceRefreshResult {
    pub resource_id: String,
    pub root_uri: String,
    pub indexed_documents: usize,
    pub mode: String,
    pub snapshot_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedResourceRebuildResult {
    pub resource_id: String,
    pub root_uri: String,
    pub indexed_documents: usize,
    pub mode: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourcePackManifest {
    pub logical_name: String,
    pub exported_resource_id: String,
    pub canonical_root_uri: String,
    pub source_kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceSemanticCompletion {
    pub indexed_documents: usize,
    pub mode: String,
}

// ---------------------------------------------------------------------------
// Shared helpers used by submodules
// ---------------------------------------------------------------------------

pub(crate) fn snapshot_record(provenance: &SourceProvenance) -> SnapshotRecord<'_> {
    SnapshotRecord {
        snapshot_id: provenance.source_snapshot_id.as_str(),
        account_id: projection_component(&provenance.projection_view_id, 1),
        user_id: projection_component(&provenance.projection_view_id, 2),
        agent_id: None,
        projection_view_id: provenance.projection_view_id.as_str(),
        root_uri: provenance.target_uri.as_str(),
        manifest_digest: Some(provenance.source_snapshot_id.as_str()),
        created_by: Some("refresh"),
        notes: Some("projection refresh"),
    }
}

pub(super) fn remove_file_sidecars(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let abstract_path = path.with_file_name(format!("{file_name}.abstract.md"));
    let overview_path = path.with_file_name(format!("{file_name}.overview.md"));
    if abstract_path.exists() {
        fs::remove_file(abstract_path)?;
    }
    if overview_path.exists() {
        fs::remove_file(overview_path)?;
    }
    Ok(())
}

pub(super) fn join_uri(root_uri: &str, relative: &str) -> String {
    if relative.is_empty() {
        root_uri.to_owned()
    } else {
        format!("{}/{}", root_uri.trim_end_matches('/'), relative)
    }
}

pub(super) fn record_resource_changes(
    metadata: &MetadataStore,
    identity: &IdentityContext,
    resource_id: Option<&str>,
    snapshot_id: &str,
    changes: &[refresh::ProjectionChange],
) -> Result<(), Box<dyn std::error::Error>> {
    let Some(resource_id) = resource_id else {
        return Ok(());
    };
    for change in changes {
        metadata.insert_change_event(
            &format!(
                "{}:{}:{}:{}",
                resource_id,
                snapshot_id,
                change.change_type,
                short_hash_hex(change.uri.as_bytes(), 12)
            ),
            resource_id,
            identity.account_id(),
            identity.user_id(),
            &change.uri,
            change.change_type,
            change.content_digest.as_deref(),
            Some(snapshot_id),
        )?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Re-exports — maintain the same public API surface
// ---------------------------------------------------------------------------

pub(crate) use rebuild::rebuild_metadata_entries_with_provenance;
pub use rebuild::{rebuild_metadata_entries, rebuild_projection, rebuild_registered_resource};

pub use refresh::{refresh_projection, refresh_registered_resource};

pub use ingest::{
    complete_prepared_resource_ingest, complete_registered_resource_ingest, ingest_resource,
    prepare_inline_resource_ingest, prepare_resource_ingest,
};

pub use pack::{export_resource_pack, import_resource_pack};
