//! HTTP handlers for Repo Knowledge Manifest API.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Json;
use axum::extract::{Query, State};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use mfs_metadata::{ManifestIdentityRecord, MetadataStore};
use mfs_types::MfsError;

use super::{AppState, HandlerResult};

// ─── Query / Request structs ──────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(super) struct GetManifestQuery {
    pub repo_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct UpdateManifestRequest {
    pub repo_id: String,
    /// Path to MANIFEST.yaml on local filesystem (pure-local mode).
    /// In SaaS mode (content or manifest_json provided), this field is optional;
    /// if absent, no local YAML file is read.
    pub manifest_yaml_path: Option<String>,
    pub updater: String,
    pub resource_uri: Option<String>,
    pub default_branch: Option<String>,
    pub primary_languages: Option<Value>,
    /// Raw YAML content string (SaaS mode upload — replaces manifest_yaml_path).
    /// When provided, manifest_yaml_path is ignored for content reading.
    pub content: Option<String>,
    /// Structured manifest JSON (SaaS mode upload — already parsed).
    /// When provided, content and manifest_yaml_path are both ignored.
    pub manifest_json: Option<Value>,
}

// ─── Handlers ─────────────────────────────────────────────────────────

pub(super) async fn get_manifest(
    State(state): State<Arc<AppState>>,
    Query(query): Query<GetManifestQuery>,
) -> HandlerResult<Json<Value>> {
    let repo_id = query
        .repo_id
        .unwrap_or_else(|| state.config.account_id.clone());
    let metadata = state.metadata.clone();

    let identity = metadata
        .get_manifest_identity(&repo_id)
        .map_err(|e| MfsError::Internal {
            message: e.to_string(),
        })?;

    let Some(identity) = identity else {
        return Err(MfsError::NotFound {
            resource: format!("manifest:{}", repo_id),
        }
        .into());
    };

    let Some(ref yaml_path) = identity.manifest_yaml_path else {
        // SaaS mode: manifest_yaml_path is not set (cloud manifest).
        // Return identity data directly without reading local YAML file.
        let mut manifest = json!({});
        merge_sqlite_manifest_metadata(&mut manifest, &identity, &metadata)?;
        return Ok(Json(json!({
            "status": "ok",
            "data": manifest,
            "version_hash": identity.last_verified_at,
            "hint": null,
        })));
    };
    let manifest_path = PathBuf::from(&yaml_path);
    let mut manifest = read_manifest_json(&manifest_path)?;
    merge_sqlite_manifest_metadata(&mut manifest, &identity, &metadata)?;

    Ok(Json(json!({
        "status": "ok",
        "data": manifest,
        "version_hash": identity.last_verified_at,
        "hint": null,
    })))
}

pub(super) async fn update_manifest(
    State(state): State<Arc<AppState>>,
    Json(request): Json<UpdateManifestRequest>,
) -> HandlerResult<Json<Value>> {
    // Only human can update Manifest
    if request.updater != "human" {
        return Err(MfsError::PermissionDenied {
            reason:
                "manifest.update requires updater='human'; agents cannot directly modify Manifest"
                    .into(),
        }
        .into());
    }

    // Determine manifest content source:
    //   1. manifest_json (structured JSON) — highest priority, SaaS upload
    //   2. content (raw YAML string) — SaaS upload
    //   3. manifest_yaml_path (local file) — pure-local mode (backward compatible)
    let manifest: Value;
    let manifest_path_for_record: Option<String>;
    let source_label: &'static str;

    if let Some(json_value) = request.manifest_json {
        // SaaS: structured JSON provided directly — skip YAML parsing
        manifest = json_value;
        manifest_path_for_record = request.manifest_yaml_path.clone();
        source_label = "json";
    } else if let Some(yaml_content) = request.content {
        // SaaS: raw YAML content string — parse it
        manifest = serde_yaml::from_str::<Value>(&yaml_content).map_err(|error| {
            MfsError::InvalidArgument {
                field: "content".into(),
                reason: format!("invalid Manifest YAML content: {}", error),
            }
        })?;
        manifest_path_for_record = request.manifest_yaml_path.clone();
        source_label = "content";
    } else if let Some(ref yaml_path) = request.manifest_yaml_path {
        // Pure-local mode: read from filesystem (backward compatible)
        let manifest_path = resolve_manifest_path(&state, yaml_path);
        manifest = read_manifest_json(&manifest_path)?;
        manifest_path_for_record = Some(manifest_path.to_string_lossy().to_string());
        source_label = "file";
    } else {
        return Err(MfsError::InvalidArgument {
            field: "manifest_yaml_path".into(),
            reason: "one of manifest_yaml_path, content, or manifest_json must be provided".into(),
        }
        .into());
    }

    validate_manifest(&manifest, &request.repo_id)?;

    let metadata = state.metadata.clone();
    let now = chrono::Utc::now().to_rfc3339();
    let repo_identity = manifest
        .get("repo_identity")
        .and_then(Value::as_object)
        .ok_or_else(|| MfsError::InvalidArgument {
            field: "repo_identity".into(),
            reason: "repo_identity must be an object".into(),
        })?;
    let default_branch = request
        .default_branch
        .clone()
        .or_else(|| string_field(repo_identity, "default_branch"))
        .unwrap_or_else(|| "main".into());
    let primary_languages = request
        .primary_languages
        .clone()
        .or_else(|| repo_identity.get("primary_languages").cloned())
        .unwrap_or_else(|| json!([]));
    let languages_str = serde_json::to_string(&primary_languages).unwrap_or_else(|_| "[]".into());
    let created_at = string_field(repo_identity, "created_at").unwrap_or_else(|| now.clone());
    let resource_uri = request
        .resource_uri
        .unwrap_or_else(|| format!("mfs://resources/localfs/{}/MANIFEST.yaml", request.repo_id));
    // manifest_yaml_path: None when SaaS mode (no local file), Some when pure-local mode
    let yaml_path_str = manifest_path_for_record.as_deref();

    let record = ManifestIdentityRecord {
        repo_id: &request.repo_id,
        resource_uri: &resource_uri,
        default_branch: &default_branch,
        primary_languages: &languages_str,
        created_at: &created_at,
        last_verified_at: &now,
        manifest_yaml_path: yaml_path_str,
        repo_name: None,
        repo_path: None,
        last_commit_hash: None,
        last_commit_date: None,
        manifest_version: "1",
        yaml_hash: None,
        source_roots_json: "[]",
        quality_gates_json: "{}",
        updated_at: &now,
    };

    metadata
        .upsert_manifest_identity(&record)
        .map_err(|e| MfsError::Internal {
            message: e.to_string(),
        })?;

    Ok(Json(json!({
        "status": "ok",
        "data": {
            "repo_id": request.repo_id,
            "manifest_yaml_path": manifest_path_for_record,
            "source": source_label,
        },
        "version_hash": now,
        "hint": null,
    })))
}

fn resolve_manifest_path(state: &AppState, raw_path: &str) -> PathBuf {
    let path = Path::new(raw_path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        state.config.source_path.join(path)
    }
}

fn read_manifest_json(path: &Path) -> Result<Value, MfsError> {
    let raw = std::fs::read_to_string(path).map_err(|error| MfsError::InvalidArgument {
        field: "manifest_yaml_path".into(),
        reason: format!("cannot read Manifest YAML '{}': {}", path.display(), error),
    })?;
    serde_yaml::from_str::<Value>(&raw).map_err(|error| MfsError::InvalidArgument {
        field: "manifest_yaml_path".into(),
        reason: format!("invalid Manifest YAML '{}': {}", path.display(), error),
    })
}

fn validate_manifest(manifest: &Value, expected_repo_id: &str) -> Result<(), MfsError> {
    let object = manifest
        .as_object()
        .ok_or_else(|| MfsError::InvalidArgument {
            field: "manifest".into(),
            reason: "Manifest YAML must be a mapping".into(),
        })?;
    for field in [
        "repo_identity",
        "memory_assets",
        "canvas_indexes",
        "source_roots",
        "active_overlays",
        "quality_gates",
        "conflicts",
    ] {
        if !object.contains_key(field) {
            return Err(MfsError::InvalidArgument {
                field: field.into(),
                reason: "required Manifest section is missing".into(),
            });
        }
    }

    let repo_identity = object
        .get("repo_identity")
        .and_then(Value::as_object)
        .ok_or_else(|| MfsError::InvalidArgument {
            field: "repo_identity".into(),
            reason: "repo_identity must be an object".into(),
        })?;
    let repo_id =
        string_field(repo_identity, "repo_id").ok_or_else(|| MfsError::InvalidArgument {
            field: "repo_identity.repo_id".into(),
            reason: "repo_id is required".into(),
        })?;
    if repo_id != expected_repo_id {
        return Err(MfsError::InvalidArgument {
            field: "repo_identity.repo_id".into(),
            reason: format!(
                "repo_id '{}' does not match request repo_id '{}'",
                repo_id, expected_repo_id
            ),
        });
    }
    if string_field(repo_identity, "default_branch").is_none() {
        return Err(MfsError::InvalidArgument {
            field: "repo_identity.default_branch".into(),
            reason: "default_branch is required".into(),
        });
    }
    if repo_identity
        .get("primary_languages")
        .and_then(Value::as_array)
        .is_none_or(|items| items.is_empty())
    {
        return Err(MfsError::InvalidArgument {
            field: "repo_identity.primary_languages".into(),
            reason: "primary_languages must be a non-empty array".into(),
        });
    }
    for array_field in [
        "memory_assets",
        "canvas_indexes",
        "source_roots",
        "active_overlays",
        "quality_gates",
        "conflicts",
    ] {
        if object.get(array_field).and_then(Value::as_array).is_none() {
            return Err(MfsError::InvalidArgument {
                field: array_field.into(),
                reason: "section must be an array".into(),
            });
        }
    }
    Ok(())
}

fn merge_sqlite_manifest_metadata(
    manifest: &mut Value,
    identity: &mfs_metadata::StoredManifestIdentity,
    metadata: &MetadataStore,
) -> Result<(), MfsError> {
    let object = manifest
        .as_object_mut()
        .ok_or_else(|| MfsError::InvalidArgument {
            field: "manifest".into(),
            reason: "Manifest YAML must be a mapping".into(),
        })?;
    let repo_identity = object
        .entry("repo_identity")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| MfsError::InvalidArgument {
            field: "repo_identity".into(),
            reason: "repo_identity must be an object".into(),
        })?;
    repo_identity.insert("repo_id".into(), json!(identity.repo_id));
    repo_identity.insert("resource_uri".into(), json!(identity.resource_uri));
    repo_identity.insert("default_branch".into(), json!(identity.default_branch));
    let languages =
        serde_json::from_str::<Value>(&identity.primary_languages).unwrap_or_else(|_| json!([]));
    repo_identity.insert("primary_languages".into(), languages);
    repo_identity.insert("created_at".into(), json!(identity.created_at));
    repo_identity.insert("last_verified_at".into(), json!(identity.last_verified_at));
    object.insert(
        "manifest_yaml_path".into(),
        json!(identity.manifest_yaml_path),
    );

    let overlays = metadata
        .list_active_overlays(&identity.repo_id, None)
        .map_err(|error| MfsError::Internal {
            message: error.to_string(),
        })?;
    object.insert(
        "active_overlays".into(),
        serde_json::to_value(overlays).unwrap_or_else(|_| json!([])),
    );
    Ok(())
}

fn string_field(object: &Map<String, Value>, field: &str) -> Option<String> {
    object.get(field).and_then(Value::as_str).map(str::to_owned)
}
