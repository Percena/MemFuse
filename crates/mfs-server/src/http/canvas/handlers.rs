//! HTTP handler functions for the Canvas API.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Query, State};
use serde_json::{Value, json};

use mfs_metadata::{
    CanvasEdgeRecord, CanvasNodeRecord, CanvasStoreError, ResolveResult, local_id_to_canonical_ref,
};
use mfs_types::MfsError;

use super::conflict::detect_canvas_conflicts;
use super::generator::generate_elixir_canvas;
use super::{
    AppState, CanvasQueryParams, CanvasRefreshRequest, CanvasSnapshotRequest, HandlerResult,
};

/// Map CanvasStoreError to MfsError for handler responses.
fn canvas_err(e: CanvasStoreError) -> MfsError {
    MfsError::Internal {
        message: e.to_string(),
    }
}

pub async fn query_canvas(
    State(state): State<Arc<AppState>>,
    Query(query): Query<CanvasQueryParams>,
) -> HandlerResult<Json<Value>> {
    let metadata = state.metadata.clone();
    let canvas_store = state.canvas_store.clone();
    let repo_id = &query.repo_id;

    // Verify manifest exists for this repo (manifest → metadata, not canvas_store)
    let identity = metadata
        .get_manifest_identity(repo_id)
        .map_err(|e| MfsError::Internal {
            message: e.to_string(),
        })?;
    let Some(_identity) = identity else {
        return Err(MfsError::NotFound {
            resource: format!("manifest:{}", repo_id),
        }
        .into());
    };

    let canvas_type = query.canvas_type.as_deref().unwrap_or("structural");
    if !["structural", "contracts", "status"].contains(&canvas_type) {
        return Err(MfsError::InvalidArgument {
            field: "type".into(),
            reason: "must be one of structural, contracts, status".into(),
        }
        .into());
    }

    // Fetch nodes (via canvas_store trait)
    let mut nodes = canvas_store
        .list_canvas_nodes(
            repo_id,
            query.node_type.as_deref(),
            query.component.as_deref(),
        )
        .map_err(canvas_err)?;

    // Determine which node IDs are relevant
    let node_ids: Vec<String> = nodes.iter().map(|n| n.id.clone()).collect();

    // Fetch edges connected to those nodes
    let has_node_filter = query.node_type.is_some() || query.component.is_some();
    let mut edges = if !node_ids.is_empty() {
        canvas_store
            .list_canvas_edges_by_nodes(repo_id, &node_ids)
            .map_err(canvas_err)?
    } else if has_node_filter {
        Vec::new()
    } else {
        canvas_store
            .list_canvas_edges_by_repo(repo_id)
            .map_err(canvas_err)?
    };
    if query.component.is_some() {
        let mut known_ids: BTreeSet<String> = nodes.iter().map(|node| node.id.clone()).collect();
        for edge in &edges {
            for id in [&edge.source_node_id, &edge.target_node_id] {
                if known_ids.contains(id) {
                    continue;
                }
                if let Some(node) = canvas_store.get_canvas_node(id).map_err(canvas_err)? {
                    known_ids.insert(node.id.clone());
                    nodes.push(node);
                }
            }
        }
    }

    match canvas_type {
        "structural" => {
            edges.retain(|edge| edge.edge_type != "contract");
        }
        "contracts" => {
            edges.retain(|edge| edge.edge_type == "contract");
            let connected_ids: BTreeSet<String> = edges
                .iter()
                .flat_map(|edge| [edge.source_node_id.clone(), edge.target_node_id.clone()])
                .collect();
            nodes.retain(|node| connected_ids.contains(&node.id));
        }
        "status" => {
            edges.clear();
        }
        _ => {}
    }

    // Fetch overlays
    let overlays = metadata
        .list_active_overlays(repo_id, query.status.as_deref())
        .map_err(|e| MfsError::Internal {
            message: e.to_string(),
        })?;

    // Detect conflicts among overlays
    let conflicts = detect_canvas_conflicts(&overlays, &node_ids);

    // Build version_hash from latest node/edge
    let version_hash = nodes
        .iter()
        .map(|n| n.version_hash.clone())
        .max()
        .or_else(|| edges.iter().map(|e| e.version_hash.clone()).max())
        .unwrap_or_default();

    // Build overlay data with canonical refs and collect unresolved refs (T0.4)
    let mut unresolved_refs: Vec<Value> = Vec::new();
    let overlay_data: Vec<Value> = overlays
        .iter()
        .map(|o| {
            // Parse affected_node_refs/affected_edge_refs from StoredOverlay
            let node_refs: Vec<String> =
                serde_json::from_str(o.affected_node_refs.as_deref().unwrap_or("[]"))
                    .unwrap_or_default();
            let edge_refs: Vec<String> =
                serde_json::from_str(o.affected_edge_refs.as_deref().unwrap_or("[]"))
                    .unwrap_or_default();

            // Check each ref against local Canvas data
            for ref_str in node_refs.iter().chain(edge_refs.iter()) {
                let exists_in_canvas =
                    node_ids
                        .iter()
                        .any(|nid| match local_id_to_canonical_ref(nid) {
                            ResolveResult::Resolved {
                                local_id: canonical,
                            } => canonical == *ref_str,
                            _ => false,
                        })
                        || edges
                            .iter()
                            .any(|e| match local_id_to_canonical_ref(&e.id) {
                                ResolveResult::Resolved {
                                    local_id: canonical,
                                } => canonical == *ref_str,
                                _ => false,
                            });
                if !exists_in_canvas {
                    unresolved_refs.push(json!({
                        "ref": ref_str,
                        "overlay_id": o.id,
                        "reason": "ref not found in current canvas data",
                    }));
                }
            }

            json!({
                "id": o.id,
                "overlay_type": o.overlay_type,
                "tracker": o.tracker,
                "tracker_identifier": o.tracker_identifier,
                "tracker_project_item_id": o.tracker_project_item_id,
                "status": o.status,
                "affected_nodes": o.affected_nodes,
                "affected_edges": o.affected_edges,
                "affected_node_refs": node_refs,
                "affected_edge_refs": edge_refs,
            })
        })
        .collect::<Vec<Value>>();

    let data = json!({
        "version_hash": version_hash,
        "nodes": nodes,
        "edges": edges,
        "overlays": overlay_data,
        "conflicts": conflicts,
        "unresolved_refs": unresolved_refs,
    });

    // Compute freshness: "current" if no manifest_cache or canvas version matches cloud;
    // "stale" if canvas version_hash diverges from cloud_version_hash in manifest_cache.
    let manifest_cache = metadata.get_manifest_cache(repo_id)?;
    let (freshness, last_synced_at) = match manifest_cache {
        Some(cache) => {
            let fresh = match cache.cloud_version_hash {
                Some(ref cloud_hash) if cloud_hash == &version_hash => "current",
                Some(_) => "stale", // cloud hash differs from local
                None => "current",  // no cloud sync yet, treat as current
            };
            (fresh, Some(cache.last_synced_at))
        }
        None => ("current", None), // pure-local mode, no cloud sync data
    };

    Ok(Json(json!({
        "status": "ok",
        "data": data,
        "version_hash": version_hash,
        "freshness": freshness,
        "last_synced_at": last_synced_at,
        "hint": null,
    })))
}

pub async fn refresh_canvas(
    State(state): State<Arc<AppState>>,
    Json(request): Json<CanvasRefreshRequest>,
) -> HandlerResult<Json<Value>> {
    let metadata = state.metadata.clone();
    let canvas_store = state.canvas_store.clone();
    let repo_id = &request.repo_id;
    let generator = request
        .generator
        .unwrap_or_else(|| "regex-deterministic".into());

    // Verify manifest exists
    let identity = metadata
        .get_manifest_identity(repo_id)
        .map_err(|e| MfsError::Internal {
            message: e.to_string(),
        })?;
    let Some(_identity) = identity else {
        return Err(MfsError::NotFound {
            resource: format!("manifest:{}", repo_id),
        }
        .into());
    };

    // For P0, only regex-deterministic generator is available
    if generator != "regex-deterministic" {
        return Err(MfsError::InvalidArgument {
            field: "generator".into(),
            reason: format!(
                "only 'regex-deterministic' generator is supported in P0; '{}' is not available",
                generator
            ),
        }
        .into());
    }

    let source_path = resolve_canvas_source_path(&state, repo_id)?;
    let generated = generate_elixir_canvas(&source_path, repo_id, &generator).map_err(|e| {
        MfsError::Internal {
            message: e.to_string(),
        }
    })?;

    // Delete existing canvas data for this repo
    canvas_store
        .delete_canvas_edges_by_repo(repo_id)
        .map_err(canvas_err)?;
    canvas_store
        .delete_canvas_nodes_by_repo(repo_id)
        .map_err(canvas_err)?;

    let now = chrono::Utc::now().to_rfc3339();

    for node in &generated.nodes {
        canvas_store
            .upsert_canvas_node(&CanvasNodeRecord {
                id: &node.id,
                repo_id,
                node_type: &node.node_type,
                name: &node.name,
                path: node.path.as_deref(),
                language: Some("elixir"),
                purpose: Some(&node.purpose),
                confidence: "deterministic",
                generator: &generator,
                generated_at: &now,
                version_hash: &generated.version_hash,
                source: node.source.as_deref(),
                manifest_id: Some(repo_id),
                created_at: &now,
                updated_at: &now,
            })
            .map_err(canvas_err)?;
    }

    for edge in &generated.edges {
        canvas_store
            .upsert_canvas_edge(&CanvasEdgeRecord {
                id: &edge.id,
                repo_id,
                edge_type: &edge.edge_type,
                source_node_id: &edge.source_node_id,
                target_node_id: &edge.target_node_id,
                contract_spec: None,
                confidence: "deterministic",
                generator: &generator,
                generated_at: &now,
                version_hash: &generated.version_hash,
                manifest_id: Some(repo_id),
                created_at: &now,
                updated_at: &now,
            })
            .map_err(canvas_err)?;
    }

    let data = json!({
        "version_hash": generated.version_hash,
        "nodes_count": generated.nodes.len(),
        "edges_count": generated.edges.len(),
        "generator": generator,
        "source_path": source_path.to_string_lossy(),
    });

    Ok(Json(json!({
        "status": "ok",
        "data": data,
        "version_hash": generated.version_hash,
        "freshness": "current",
        "hint": null,
    })))
}

fn resolve_canvas_source_path(state: &AppState, repo_id: &str) -> Result<PathBuf, MfsError> {
    let resources = state
        .metadata
        .list_resource_sources(&state.config.account_id, &state.config.user_id, 512, None)
        .map_err(|e| MfsError::Internal {
            message: e.to_string(),
        })?;

    let matched_resource = resources.into_iter().find(|resource| {
        resource.resource_id == repo_id
            || resource.logical_name == repo_id
            || resource.source_repo.as_deref() == Some(repo_id)
    });

    let source_path = match matched_resource {
        Some(resource) => {
            let path = PathBuf::from(&resource.source_identifier);
            if path.exists() {
                path
            } else {
                return Err(MfsError::FailedPrecondition {
                    precondition: "registered resource source path exists".into(),
                    reason: format!(
                        "registered resource source path does not exist for repo_id '{}': {}",
                        repo_id, resource.source_identifier
                    ),
                });
            }
        }
        None => state.config.source_path.clone(),
    };

    Ok(source_path)
}

pub async fn create_snapshot(
    State(state): State<Arc<AppState>>,
    Json(request): Json<CanvasSnapshotRequest>,
) -> HandlerResult<Json<Value>> {
    let _metadata = state.metadata.clone();
    let canvas_store = state.canvas_store.clone();
    let repo_id = &request.repo_id;
    let snapshot_type = request.snapshot_type.unwrap_or_else(|| "full".into());

    let id = format!("snapshot_{}", uuid::Uuid::new_v4());
    let now = chrono::Utc::now().to_rfc3339();

    // Collect current canvas state as snapshot JSON
    let nodes = canvas_store
        .list_canvas_nodes(repo_id, None, None)
        .map_err(canvas_err)?;
    let edges = canvas_store
        .list_canvas_edges_by_repo(repo_id)
        .map_err(canvas_err)?;
    let snapshot_json = serde_json::to_string(&json!({
        "nodes": nodes,
        "edges": edges,
    }))
    .map_err(|e| MfsError::Internal {
        message: e.to_string(),
    })?;

    let record = mfs_metadata::CanvasSnapshotRecord {
        id: &id,
        repo_id,
        merge_commit: &request.merge_commit,
        snapshot_type: &snapshot_type,
        snapshot_json: &snapshot_json,
        created_at: &now,
        immutable: true,
    };

    canvas_store
        .insert_canvas_snapshot(&record)
        .map_err(canvas_err)?;

    let data = json!({
        "snapshot_id": id,
        "immutable": true,
    });

    Ok(Json(json!({
        "status": "ok",
        "data": data,
        "version_hash": request.merge_commit,
        "freshness": "current",
        "hint": null,
    })))
}

/// Returns local–cloud synchronization status for a repo's canvas data.
///
/// Per §5.4 of the SaaS architecture doc, `/canvas/sync-status` exposes:
/// - whether manifest_cache is present and its cloud_version_hash
/// - unresolved overlay_refs count
/// - last sync timestamps
pub async fn sync_status(
    State(state): State<Arc<AppState>>,
    Query(params): Query<CanvasQueryParams>,
) -> HandlerResult<Json<Value>> {
    let metadata = state.metadata.clone();
    let repo_id = &params.repo_id;

    // 1. Manifest cache status
    let manifest_cache = metadata.get_manifest_cache(repo_id)?;
    let manifest_status = match &manifest_cache {
        Some(cache) => json!({
            "synced": true,
            "cloud_version_hash": cache.cloud_version_hash,
            "last_synced_at": cache.last_synced_at,
            "default_branch": cache.default_branch,
            "primary_languages": cache.primary_languages,
        }),
        None => json!({
            "synced": false,
            "cloud_version_hash": null,
            "last_synced_at": null,
        }),
    };

    // 2. Overlay refs resolution status
    let unresolved_refs = metadata.list_unresolved_overlay_refs(repo_id)?;
    let unresolved_count = unresolved_refs.len();

    Ok(Json(json!({
        "status": "ok",
        "data": {
            "repo_id": repo_id,
            "manifest_cache": manifest_status,
            "overlay_refs": {
                "unresolved_count": unresolved_count,
                "unresolved_refs": unresolved_refs.iter().map(|r| json!({
                    "overlay_id": r.overlay_id,
                    "canonical_ref": r.canonical_ref,
                    "ref_kind": r.ref_kind,
                    "unresolved_reason": r.unresolved_reason,
                })).collect::<Vec<Value>>(),
            },
        },
        "hint": null,
    })))
}

/// Returns stale canvas data for SDK offline degradation fallback.
///
/// Per §5.5: when Canvas Daemon is offline, SDK degrades to reading the latest
/// canvas state from the cloud via `/canvas/snapshot/latest`. This endpoint
/// returns current canvas data tagged `freshness: "stale"`.
pub async fn get_latest_snapshot(
    State(state): State<Arc<AppState>>,
    Query(params): Query<CanvasQueryParams>,
) -> HandlerResult<Json<Value>> {
    let canvas_store = state.canvas_store.clone();
    let metadata = state.metadata.clone();
    let repo_id = &params.repo_id;

    let nodes = canvas_store.list_canvas_nodes(repo_id, None, None)?;
    let edges = canvas_store.list_canvas_edges_by_repo(repo_id)?;

    let version_hash = nodes
        .iter()
        .map(|n| &n.version_hash)
        .collect::<BTreeSet<&String>>()
        .iter()
        .fold(String::new(), |acc, h| {
            if acc.is_empty() {
                (*h).clone()
            } else {
                format!("{},{}", acc, h)
            }
        });

    let manifest_cache = metadata.get_manifest_cache(repo_id)?;
    let last_synced_at = manifest_cache.as_ref().map(|c| c.last_synced_at.clone());

    Ok(Json(json!({
        "status": "ok",
        "data": {
            "nodes_count": nodes.len(),
            "edges_count": edges.len(),
            "version_hash": version_hash,
        },
        "version_hash": version_hash,
        "freshness": "stale",
        "last_synced_at": last_synced_at,
        "hint": "Canvas Daemon offline — stale data, not current workspace.",
    })))
}

/// Returns the current version_hash for a repo's canvas data.
///
/// Per §5.4 Canvas Daemon API: `/canvas/version-hash` returns the aggregate
/// version hash. This is a write/real-time endpoint — returns unavailable
/// when daemon offline (no stale fallback per §5.5 degradation constraint).
pub async fn get_version_hash(
    State(state): State<Arc<AppState>>,
    Query(params): Query<CanvasQueryParams>,
) -> HandlerResult<Json<Value>> {
    let canvas_store = state.canvas_store.clone();
    let repo_id = &params.repo_id;

    let nodes = canvas_store.list_canvas_nodes(repo_id, None, None)?;

    let version_hash = nodes
        .iter()
        .map(|n| &n.version_hash)
        .collect::<BTreeSet<&String>>()
        .iter()
        .fold(String::new(), |acc, h| {
            if acc.is_empty() {
                (*h).clone()
            } else {
                format!("{},{}", acc, h)
            }
        });

    Ok(Json(json!({
        "status": "ok",
        "data": {
            "repo_id": repo_id,
            "version_hash": version_hash,
            "nodes_count": nodes.len(),
        },
        "version_hash": version_hash,
        "freshness": "current",
        "hint": null,
    })))
}
