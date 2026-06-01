use std::fs;
use std::path::Path;

use mfs_ast::extract_skeleton;
use mfs_index::SqliteSemanticIndex;
use mfs_metadata::{CodeSymbolRecord, MetadataStore, PathEntryRecord};
use mfs_semantic::{SemanticPipeline, SemanticPipelineConfig, SemanticProcessingReport};
use mfs_types::IdentityContext;
use mfs_uri::MfsUri;
use mfs_workspace::{
    SourceProvenance, WorkspaceLayout, classify_path, content_digest_for_path,
    directory_metadata_digest, is_summary_sidecar,
};

use crate::projection_view_id_for_uri;

use super::{ManagedResourceRebuildResult, RebuildResult};

// ---------------------------------------------------------------------------
// Public API — rebuild
// ---------------------------------------------------------------------------

pub async fn rebuild_projection(
    metadata: &MetadataStore,
    identity: &IdentityContext,
    projection_root: &Path,
    projection_uri: &str,
) -> Result<mfs_retrieval::RetrievalEngine, Box<dyn std::error::Error>> {
    let _ = rebuild_metadata_entries(metadata, identity, projection_root, projection_uri)?;
    Ok(mfs_retrieval::RetrievalEngine::from_projection(projection_root, projection_uri).await?)
}

pub fn rebuild_metadata_entries(
    metadata: &MetadataStore,
    identity: &IdentityContext,
    projection_root: &Path,
    projection_uri: &str,
) -> Result<RebuildResult, Box<dyn std::error::Error>> {
    rebuild_metadata_entries_with_provenance(
        metadata,
        identity,
        projection_root,
        projection_uri,
        None,
    )
}

pub(crate) fn rebuild_metadata_entries_with_provenance(
    metadata: &MetadataStore,
    identity: &IdentityContext,
    projection_root: &Path,
    projection_uri: &str,
    provenance: Option<&SourceProvenance>,
) -> Result<RebuildResult, Box<dyn std::error::Error>> {
    let projection_view_id = provenance
        .map(|provenance| provenance.projection_view_id.clone())
        .unwrap_or_else(|| projection_view_id_for_uri(identity, projection_uri));
    let repo_root_uri = if projection_uri.starts_with("mfs://resources/") {
        Some(projection_uri.to_owned())
    } else {
        None
    };
    let mut stack = vec![projection_root.to_path_buf()];
    let mut indexed_paths = 0;
    let mut symbol_records = Vec::new();
    let root_metadata_digest = directory_metadata_digest(projection_root).ok();
    let root_size_bytes = fs::metadata(projection_root).ok().map(|item| item.len());

    metadata.upsert_path_entry(&PathEntryRecord {
        account_id: identity.account_id(),
        user_id: identity.user_id(),
        agent_id: Some(identity.agent_id()),
        projection_view_id: &projection_view_id,
        canonical_uri: projection_uri,
        workspace_path: &projection_root.to_string_lossy(),
        entry_kind: "directory",
        source_kind: provenance.map(|item| item.source_kind.as_str()),
        source_identifier: provenance.map(|item| item.source_identifier.as_str()),
        source_snapshot_id: provenance.map(|item| item.source_snapshot_id.as_str()),
        content_kind: Some("directory"),
        language: None,
        relative_resource_path: Some(""),
        repo_root_uri: repo_root_uri.as_deref(),
        is_text: Some(false),
        is_generated: Some(false),
        content_digest: None,
        metadata_digest: root_metadata_digest.as_deref(),
        size_bytes: root_size_bytes,
    })?;
    indexed_paths += 1;

    if projection_uri.starts_with("mfs://resources/") {
        let _ = metadata.delete_code_symbols_for_prefix(&projection_view_id, projection_uri)?;
    }

    while let Some(path) = stack.pop() {
        for entry in std::fs::read_dir(&path)? {
            let entry = entry?;
            let entry_path = entry.path();
            let file_type = entry.file_type()?;
            let relative_path = entry_path
                .strip_prefix(projection_root)?
                .to_string_lossy()
                .replace('\\', "/");
            if file_type.is_dir() {
                let directory_metadata_digest = directory_metadata_digest(&entry_path).ok();
                let directory_size_bytes = fs::metadata(&entry_path).ok().map(|item| item.len());
                let canonical_uri = if relative_path.is_empty() {
                    projection_uri.to_owned()
                } else {
                    format!("{}/{}", projection_uri.trim_end_matches('/'), relative_path)
                };
                metadata.upsert_path_entry(&PathEntryRecord {
                    account_id: identity.account_id(),
                    user_id: identity.user_id(),
                    agent_id: Some(identity.agent_id()),
                    projection_view_id: &projection_view_id,
                    canonical_uri: &canonical_uri,
                    workspace_path: &entry_path.to_string_lossy(),
                    entry_kind: "directory",
                    source_kind: provenance.map(|item| item.source_kind.as_str()),
                    source_identifier: provenance.map(|item| item.source_identifier.as_str()),
                    source_snapshot_id: provenance.map(|item| item.source_snapshot_id.as_str()),
                    content_kind: Some("directory"),
                    language: None,
                    relative_resource_path: Some(&relative_path),
                    repo_root_uri: repo_root_uri.as_deref(),
                    is_text: Some(false),
                    is_generated: Some(false),
                    content_digest: None,
                    metadata_digest: directory_metadata_digest.as_deref(),
                    size_bytes: directory_size_bytes,
                })?;
                indexed_paths += 1;
                stack.push(entry_path);
                continue;
            }

            let canonical_uri = if relative_path.is_empty() {
                projection_uri.to_owned()
            } else {
                format!("{}/{}", projection_uri.trim_end_matches('/'), relative_path)
            };
            let classified = if is_summary_sidecar(&entry_path) {
                GeneratedClassification::summary()
            } else {
                GeneratedClassification::from_path(projection_root, &entry_path)
            };
            metadata.upsert_path_entry(&PathEntryRecord {
                account_id: identity.account_id(),
                user_id: identity.user_id(),
                agent_id: Some(identity.agent_id()),
                projection_view_id: &projection_view_id,
                canonical_uri: &canonical_uri,
                workspace_path: &entry_path.to_string_lossy(),
                entry_kind: "file",
                source_kind: provenance.map(|item| item.source_kind.as_str()),
                source_identifier: provenance.map(|item| item.source_identifier.as_str()),
                source_snapshot_id: provenance.map(|item| item.source_snapshot_id.as_str()),
                content_kind: Some(classified.content_kind.as_str()),
                language: classified.language.as_deref(),
                relative_resource_path: Some(&relative_path),
                repo_root_uri: repo_root_uri.as_deref(),
                is_text: Some(classified.is_text),
                is_generated: Some(classified.is_generated),
                content_digest: classified.content_digest.as_deref(),
                metadata_digest: None,
                size_bytes: fs::metadata(&entry_path).ok().map(|item| item.len()),
            })?;
            if classified.content_kind == "code" {
                symbol_records.extend(build_code_symbol_records(
                    identity,
                    &projection_view_id,
                    &canonical_uri,
                    &entry_path,
                ));
            }
            indexed_paths += 1;
        }
    }

    for record in &symbol_records {
        metadata.insert_code_symbol(&CodeSymbolRecord {
            id: &record.id,
            account_id: &record.account_id,
            user_id: &record.user_id,
            agent_id: Some(&record.agent_id),
            projection_view_id: &record.projection_view_id,
            canonical_uri: &record.canonical_uri,
            symbol_type: &record.symbol_type,
            symbol_name: &record.symbol_name,
            signature: record.signature.as_deref(),
            docstring: record.docstring.as_deref(),
            line_number: record.line_number,
            embedding_json: None,
        })?;
    }

    Ok(RebuildResult {
        indexed_paths,
        projection_uri: projection_uri.to_owned(),
    })
}

// ---------------------------------------------------------------------------
// Public API — rebuild registered resource
// ---------------------------------------------------------------------------

pub async fn rebuild_registered_resource(
    metadata: &MetadataStore,
    workspace_root: &Path,
    identity: &IdentityContext,
    resource_id: &str,
) -> Result<ManagedResourceRebuildResult, Box<dyn std::error::Error>> {
    let source = metadata
        .get_resource_source(resource_id)?
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "resource not found"))?;
    let target_uri = MfsUri::parse(&source.canonical_root_uri)?;
    let target_path = WorkspaceLayout::new(workspace_root).path_for_uri(identity, &target_uri)?;
    let report = reprocess_semantic_root(
        workspace_root,
        &source.projection_view_id,
        &target_path,
        &source.canonical_root_uri,
        "resource",
    )
    .await?;
    let provenance = SourceProvenance {
        source_kind: source.source_kind.clone(),
        source_identifier: source.source_identifier.clone(),
        source_snapshot_id: source
            .last_snapshot_id
            .clone()
            .unwrap_or_else(|| "rebuild".to_owned()),
        projection_view_id: source.projection_view_id.clone(),
        materialization_mode: "managed".to_owned(),
        target_uri: source.canonical_root_uri.clone(),
    };
    let _ = rebuild_metadata_entries_with_provenance(
        metadata,
        identity,
        &target_path,
        &source.canonical_root_uri,
        Some(&provenance),
    )?;

    Ok(ManagedResourceRebuildResult {
        resource_id: source.resource_id,
        root_uri: source.canonical_root_uri,
        indexed_documents: report.indexed_documents,
        mode: format!("{:?}", report.mode).to_ascii_lowercase(),
    })
}

// ---------------------------------------------------------------------------
// pub(crate) — used by watch_ops and owned_path_ops
// ---------------------------------------------------------------------------

pub(crate) async fn reprocess_semantic_root(
    workspace_root: &Path,
    projection_view_id: &str,
    root: &Path,
    root_uri: &str,
    context_type: &str,
) -> Result<SemanticProcessingReport, Box<dyn std::error::Error>> {
    let workspace_root = workspace_root.to_path_buf();
    let projection_view_id = projection_view_id.to_owned();
    let root = root.to_path_buf();
    let root_uri = root_uri.to_owned();
    let context_type = context_type.to_owned();
    let pv_for_blocking = projection_view_id.clone();
    let ru_for_blocking = root_uri.clone();
    let ws_for_blocking = workspace_root.clone();
    let semantic_index =
        tokio::task::spawn_blocking(move || -> Result<SqliteSemanticIndex, String> {
            let idx = SqliteSemanticIndex::open_at(
                ws_for_blocking.join("_system").join("semantic.sqlite"),
            )
            .map_err(|e| e.to_string())?;
            idx.delete_prefix_in_projection(Some(&pv_for_blocking), Some(&ru_for_blocking))
                .map_err(|e| e.to_string())?;
            Ok(idx)
        })
        .await
        .map_err(|e| -> Box<dyn std::error::Error> { format!("spawn_blocking: {e}").into() })?
        .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
    let pipeline = SemanticPipeline::new(SemanticPipelineConfig::from_env(8));
    pipeline
        .process_root(
            &root,
            &projection_view_id,
            &root_uri,
            &context_type,
            None,
            &semantic_index,
        )
        .await
        .map_err(|e| -> Box<dyn std::error::Error> { e.into() })
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct OwnedCodeSymbolRecord {
    id: String,
    account_id: String,
    user_id: String,
    agent_id: String,
    projection_view_id: String,
    canonical_uri: String,
    symbol_type: String,
    symbol_name: String,
    signature: Option<String>,
    docstring: Option<String>,
    line_number: Option<i64>,
}

#[derive(Debug, Clone)]
struct GeneratedClassification {
    content_kind: String,
    language: Option<String>,
    is_text: bool,
    is_generated: bool,
    content_digest: Option<String>,
}

impl GeneratedClassification {
    fn summary() -> Self {
        Self {
            content_kind: "generated".to_owned(),
            language: Some("markdown".to_owned()),
            is_text: true,
            is_generated: true,
            content_digest: None,
        }
    }

    fn from_path(projection_root: &Path, path: &Path) -> Self {
        let relative = path.strip_prefix(projection_root).unwrap_or(path);
        let classified = classify_path(relative, false);
        Self {
            content_kind: classified.content_kind,
            language: classified.language,
            is_text: classified.is_text,
            is_generated: classified.is_generated,
            content_digest: content_digest_for_path(path).ok(),
        }
    }
}

fn build_code_symbol_records(
    identity: &IdentityContext,
    projection_view_id: &str,
    canonical_uri: &str,
    path: &Path,
) -> Vec<OwnedCodeSymbolRecord> {
    let Ok(content) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let Ok(skeleton) = extract_skeleton(file_name, &content) else {
        return Vec::new();
    };

    let mut records = Vec::new();
    for class in skeleton.classes {
        records.push(OwnedCodeSymbolRecord {
            id: format!("{canonical_uri}#class:{}", class.name),
            account_id: identity.account_id().to_owned(),
            user_id: identity.user_id().to_owned(),
            agent_id: identity.agent_id().to_owned(),
            projection_view_id: projection_view_id.to_owned(),
            canonical_uri: canonical_uri.to_owned(),
            symbol_type: "class".to_owned(),
            symbol_name: class.name.clone(),
            signature: if class.bases.is_empty() {
                None
            } else {
                Some(format!("extends {}", class.bases.join(", ")))
            },
            docstring: class.docstring.clone(),
            line_number: None,
        });
        for method in class.methods {
            records.push(OwnedCodeSymbolRecord {
                id: format!("{canonical_uri}#method:{}:{}", class.name, method.name),
                account_id: identity.account_id().to_owned(),
                user_id: identity.user_id().to_owned(),
                agent_id: identity.agent_id().to_owned(),
                projection_view_id: projection_view_id.to_owned(),
                canonical_uri: canonical_uri.to_owned(),
                symbol_type: "method".to_owned(),
                symbol_name: format!("{}.{}", class.name, method.name),
                signature: Some(format!("{}({})", method.name, method.params.join(", "))),
                docstring: method.docstring.clone(),
                line_number: None,
            });
        }
    }

    for function in skeleton.functions {
        records.push(OwnedCodeSymbolRecord {
            id: format!("{canonical_uri}#fn:{}", function.name),
            account_id: identity.account_id().to_owned(),
            user_id: identity.user_id().to_owned(),
            agent_id: identity.agent_id().to_owned(),
            projection_view_id: projection_view_id.to_owned(),
            canonical_uri: canonical_uri.to_owned(),
            symbol_type: "function".to_owned(),
            symbol_name: function.name.clone(),
            signature: Some(format!("{}({})", function.name, function.params.join(", "))),
            docstring: function.docstring.clone(),
            line_number: None,
        });
    }

    records
}
