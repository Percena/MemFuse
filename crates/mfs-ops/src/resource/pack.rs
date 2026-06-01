use std::path::Path;

use mfs_metadata::MetadataStore;
use mfs_types::IdentityContext;
use mfs_uri::MfsUri;
use mfs_workspace::{ManagedResource, WorkspaceLayout};

use super::ResourcePackManifest;
use super::ingest::prepare_resource_ingest;

// ---------------------------------------------------------------------------
// Public API — export / import
// ---------------------------------------------------------------------------

pub async fn import_resource_pack(
    metadata: &MetadataStore,
    workspace_root: &Path,
    identity: &IdentityContext,
    pack_path: &Path,
    logical_name: Option<&str>,
) -> Result<ManagedResource, Box<dyn std::error::Error>> {
    let manifest_path = pack_path.join("manifest.json");
    let files_path = pack_path.join("files");
    let manifest =
        serde_json::from_slice::<ResourcePackManifest>(&tokio::fs::read(&manifest_path).await?)?;
    prepare_resource_ingest(
        metadata,
        workspace_root,
        identity,
        "import",
        files_path.to_str().expect("pack files path utf-8"),
        logical_name.or(Some(manifest.logical_name.as_str())),
        None,
        None,
    )
    .await
}

pub async fn export_resource_pack(
    metadata: &MetadataStore,
    workspace_root: &Path,
    identity: &IdentityContext,
    resource_id: &str,
    output_path: &Path,
) -> Result<ResourcePackManifest, Box<dyn std::error::Error>> {
    let source = metadata
        .get_resource_source(resource_id)?
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "resource not found"))?;
    let root_uri = MfsUri::parse(&source.canonical_root_uri)?;
    let source_root = WorkspaceLayout::new(workspace_root).path_for_uri(identity, &root_uri)?;
    if tokio::fs::try_exists(output_path).await? {
        tokio::fs::remove_dir_all(output_path).await?;
    }
    tokio::fs::create_dir_all(output_path.join("files")).await?;
    copy_pack_files(&source_root, &output_path.join("files"))?;
    let manifest = ResourcePackManifest {
        logical_name: source.logical_name,
        exported_resource_id: source.resource_id,
        canonical_root_uri: source.canonical_root_uri,
        source_kind: source.source_kind,
    };
    tokio::fs::write(
        output_path.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )
    .await?;
    Ok(manifest)
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

fn copy_pack_files(source: &Path, destination: &Path) -> Result<(), Box<dyn std::error::Error>> {
    if source.is_file() {
        let file_name = source
            .file_name()
            .ok_or_else(|| std::io::Error::other("source file missing file name"))?;
        std::fs::copy(source, destination.join(file_name))?;
        return Ok(());
    }

    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            std::fs::create_dir_all(&destination_path)?;
            copy_pack_files(&path, &destination_path)?;
        } else {
            std::fs::copy(&path, &destination_path)?;
        }
    }
    Ok(())
}
