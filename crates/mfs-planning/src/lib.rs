//! OverlayService — business logic for Active Overlay API extracted from handler.
//!
//! HTTP handlers own: request validation (overlay_type, conflict_policy constants),
//! request deserialization, and JSON response formatting.
//! This service owns: all state machine logic, conflict detection, overlay CRUD,
//! canonical ref resolution, idempotency, and transition recording.
//!
//! Uses `MfsError` as the error type shared across MemFuse domain services.

use std::sync::Arc;

use mfs_metadata::{
    CanvasRefKind, MetadataStore, OverlayRecord, OverlayTransitionRecord, StoredOverlay,
    local_id_to_canonical_ref, parse_canonical_ref,
};
use mfs_types::MfsError;
use serde_json::{Map, Value, json};

// ─── Pure functions (no HTTP/Axum dependency) ──────────────────────────────

/// State machine guard: returns true if the transition `from → to` is valid.
pub fn is_valid_transition(from: &str, to: &str) -> bool {
    matches!(
        (from, to),
        (
            "proposed",
            "accepted" | "rejected" | "implemented" | "abandoned"
        ) | ("accepted", "implemented" | "abandoned")
            | ("implemented", "merged" | "abandoned")
            | (_, "stale")
            | ("stale", "proposed" | "rejected")
    )
}

/// Merge keys: combines local IDs and canonical refs into a single set for
/// conflict overlap detection.
pub fn overlay_conflict_keys(local_ids: &[String], canonical_refs: &[String]) -> Vec<String> {
    let mut keys = local_ids.to_vec();
    keys.extend(canonical_refs.iter().cloned());
    keys
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OverlayTargetKeys {
    nodes: Vec<String>,
    edges: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OverlayOverlap {
    has_conflict: bool,
    nodes: Vec<String>,
    edges: Vec<String>,
}

fn overlay_target_keys(
    affected_nodes: &[String],
    affected_edges: &[String],
    affected_node_refs: &[String],
    affected_edge_refs: &[String],
) -> OverlayTargetKeys {
    OverlayTargetKeys {
        nodes: overlay_conflict_keys(affected_nodes, affected_node_refs),
        edges: overlay_conflict_keys(affected_edges, affected_edge_refs),
    }
}

fn has_overlay_targets(
    affected_nodes: &[String],
    affected_edges: &[String],
    affected_node_refs: &[String],
    affected_edge_refs: &[String],
) -> bool {
    !affected_nodes.is_empty()
        || !affected_edges.is_empty()
        || !affected_node_refs.is_empty()
        || !affected_edge_refs.is_empty()
}

fn overlap_values(left: &[String], right: &[String]) -> Vec<String> {
    let mut overlap = Vec::new();
    for value in left {
        if right.contains(value) && !overlap.contains(value) {
            overlap.push(value.clone());
        }
    }
    overlap
}

fn overlay_overlap(left: &OverlayTargetKeys, right: &OverlayTargetKeys) -> OverlayOverlap {
    let nodes = overlap_values(&left.nodes, &right.nodes);
    let edges = overlap_values(&left.edges, &right.edges);
    OverlayOverlap {
        has_conflict: !nodes.is_empty() || !edges.is_empty(),
        nodes,
        edges,
    }
}

fn validate_canonical_refs_for_repo(
    repo_id: &str,
    field: &str,
    refs: &[String],
    expected_kind: CanvasRefKind,
) -> Result<(), MfsError> {
    for ref_str in refs {
        let components =
            parse_canonical_ref(ref_str).map_err(|reason| MfsError::InvalidArgument {
                field: field.into(),
                reason: format!("invalid canonical ref '{}': {}", ref_str, reason),
            })?;
        if components.repo_id != repo_id {
            return Err(MfsError::InvalidArgument {
                field: field.into(),
                reason: format!(
                    "canonical ref '{}' belongs to repo '{}', expected '{}'",
                    ref_str, components.repo_id, repo_id
                ),
            });
        }
        if components.kind != expected_kind {
            return Err(MfsError::InvalidArgument {
                field: field.into(),
                reason: format!(
                    "canonical ref '{}' has kind '{:?}', expected '{:?}'",
                    ref_str, components.kind, expected_kind
                ),
            });
        }
    }
    Ok(())
}

/// Extract idempotency key from an overlay's content_json `_memfuse.idempotency_key` field.
pub fn overlay_idempotency_key(overlay: &StoredOverlay) -> Option<String> {
    serde_json::from_str::<Value>(&overlay.content_json)
        .ok()
        .and_then(|content| {
            content
                .pointer("/_memfuse/idempotency_key")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
}

/// Inject idempotency key into content_json under `_memfuse.idempotency_key`.
/// If no key is provided, returns content_json unchanged.
pub fn content_json_with_idempotency(content_json: Value, idempotency_key: Option<&str>) -> Value {
    let Some(idempotency_key) = idempotency_key else {
        return content_json;
    };

    match content_json {
        Value::Object(mut object) => {
            let mut metadata = object
                .remove("_memfuse")
                .and_then(|value| value.as_object().cloned())
                .unwrap_or_else(Map::new);
            metadata.insert(
                "idempotency_key".into(),
                Value::String(idempotency_key.to_owned()),
            );
            object.insert("_memfuse".into(), Value::Object(metadata));
            Value::Object(object)
        }
        value => json!({
            "value": value,
            "_memfuse": {
                "idempotency_key": idempotency_key,
            },
        }),
    }
}

/// Parse a JSON array string (stored in overlay's affected_nodes/edges fields)
/// into a Vec<String>. Returns empty vec on parse failure or None input.
pub fn parse_overlay_refs(raw: Option<&str>) -> Vec<String> {
    raw.and_then(|value| serde_json::from_str::<Vec<String>>(value).ok())
        .unwrap_or_default()
}

/// Parse comma-separated status filter string into a Vec<String>.
/// Returns empty vec for None or empty input.
pub fn parse_status_filter(status: Option<&str>) -> Vec<String> {
    status
        .unwrap_or("")
        .split(',')
        .map(str::trim)
        .filter(|status| !status.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

// ─── Input / Output structs ────────────────────────────────────────────────

/// Input for `propose`.
pub struct ProposeInput {
    pub repo_id: String,
    pub overlay_type: String,
    pub tracker: Option<String>,
    pub tracker_content_id: String,
    pub tracker_project_item_id: Option<String>,
    pub tracker_identifier: String,
    pub issue_number: Option<i64>,
    pub affected_nodes: Vec<String>,
    pub affected_edges: Vec<String>,
    pub affected_node_refs: Option<Vec<String>>,
    pub affected_edge_refs: Option<Vec<String>>,
    pub content_json: Value,
    pub author: String,
    pub branch: Option<String>,
    pub pr_url: Option<String>,
    pub agent_session_id: Option<String>,
    pub idempotency_key: Option<String>,
    pub conflict_policy: Option<String>,
}

/// Output for `propose`.
pub struct ProposeOutput {
    pub overlay_id: String,
    pub overlay_type: String,
    pub status: String, // "ok" or "conflict"
    pub version_hash: String,
    pub idempotent_replay: bool,
    pub data: Value,
    /// When status == "conflict", contains a hint message.
    pub conflict_hint: Option<String>,
}

/// Output for `report_conflict`.
pub struct ConflictAnalysis {
    pub conflict_id: String,
    pub has_conflict: bool,
    pub requires_human_review: bool,
    pub overlap_nodes: Vec<String>,
    pub overlap_edges: Vec<String>,
    pub description: Option<String>,
    pub details: Vec<ConflictDetail>,
}

/// Per-overlay conflict detail for `report_conflict`.
pub struct ConflictDetail {
    pub overlay_id: String,
    pub overlay_type: String,
    pub status: String,
    pub overlap_nodes: Vec<String>,
    pub overlap_edges: Vec<String>,
}

/// Input for `record_conflict`.
pub struct RecordConflictInput {
    pub repo_id: String,
    pub run_id: Option<String>,
    pub tracker: Option<String>,
    pub tracker_identifier: Option<String>,
    pub conflict_summary: String,
    pub severity: Option<String>,
    pub evidence: Option<Value>,
    pub author: Option<String>,
}

/// Output for `consolidate`.
pub struct ConsolidateOutput {
    pub merged_count: usize,
    pub snapshot_id: String,
}

// ─── OverlayService ────────────────────────────────────────────────────────

pub struct OverlayService {
    metadata: Arc<MetadataStore>,
}

impl OverlayService {
    pub fn new(metadata: Arc<MetadataStore>) -> Self {
        Self { metadata }
    }

    // ─── Propose ───────────────────────────────────────────────────────

    /// Propose a new overlay. Returns ProposeOutput with status "ok" on success,
    /// or "conflict" when fail_on_conflict detects overlapping active overlays.
    ///
    /// The handler is responsible for validating overlay_type and conflict_policy
    /// (HTTP-level validation). This method handles all business logic:
    /// manifest existence check, idempotency replay, affected ref validation,
    /// canonical ref resolution, conflict detection, and recording.
    pub fn propose(&self, input: ProposeInput) -> Result<ProposeOutput, MfsError> {
        let repo_id = &input.repo_id;

        // Verify manifest exists for this repo
        let identity = self
            .metadata
            .get_manifest_identity(repo_id)
            .map_err(meta_err)?;
        if identity.is_none() {
            return Err(MfsError::NotFound {
                resource: format!("manifest:{}", repo_id),
            });
        }

        let id = format!("overlay_{}", uuid::Uuid::new_v4());
        let now = chrono::Utc::now().to_rfc3339();
        let tracker = input.tracker.unwrap_or_else(|| "github_projects".into());

        let fail_on_conflict = input.conflict_policy.as_deref() == Some("fail_on_conflict");
        let idempotency_key = input
            .idempotency_key
            .as_deref()
            .map(str::trim)
            .filter(|key| !key.is_empty())
            .map(str::to_owned);

        // Idempotency replay check
        if let Some(key) = idempotency_key.as_deref() {
            if let Some(existing) = self.find_overlay_by_idempotency_key(repo_id, key)? {
                let data = json!({
                    "overlay_id": existing.id,
                    "status": existing.status,
                    "affected_node_refs": parse_overlay_refs(existing.affected_node_refs.as_deref()),
                    "affected_edge_refs": parse_overlay_refs(existing.affected_edge_refs.as_deref()),
                    "idempotent_replay": true,
                });

                return Ok(ProposeOutput {
                    overlay_id: existing.id,
                    overlay_type: existing.overlay_type,
                    status: "ok".into(),
                    version_hash: existing.updated_at,
                    idempotent_replay: true,
                    data,
                    conflict_hint: None,
                });
            }
        }

        let affected_nodes = input.affected_nodes;
        let affected_edges = input.affected_edges;
        // Validate and normalize canonical refs before the target-presence guard,
        // because a ref-only proposal is a valid canonical Canvas request.
        let affected_node_refs: Vec<String> = match &input.affected_node_refs {
            Some(refs) => {
                validate_canonical_refs_for_repo(
                    repo_id,
                    "affected_node_refs",
                    refs,
                    CanvasRefKind::Node,
                )?;
                refs.clone()
            }
            None => affected_nodes
                .iter()
                .filter_map(|id| match local_id_to_canonical_ref(id) {
                    mfs_metadata::ResolveResult::Resolved {
                        local_id: canonical,
                    } => Some(canonical),
                    mfs_metadata::ResolveResult::Unresolved { reason } => {
                        tracing::warn!(
                            "Cannot resolve local ID '{}' to canonical ref: {}",
                            id,
                            reason
                        );
                        None
                    }
                    mfs_metadata::ResolveResult::InvalidRef { reason } => {
                        tracing::warn!("Invalid local ID '{}' for canonical ref: {}", id, reason);
                        None
                    }
                })
                .collect(),
        };
        let affected_edge_refs: Vec<String> = match &input.affected_edge_refs {
            Some(refs) => {
                validate_canonical_refs_for_repo(
                    repo_id,
                    "affected_edge_refs",
                    refs,
                    CanvasRefKind::Edge,
                )?;
                refs.clone()
            }
            None => affected_edges
                .iter()
                .filter_map(|id| match local_id_to_canonical_ref(id) {
                    mfs_metadata::ResolveResult::Resolved {
                        local_id: canonical,
                    } => Some(canonical),
                    mfs_metadata::ResolveResult::Unresolved { reason } => {
                        tracing::warn!(
                            "Cannot resolve local ID '{}' to canonical ref: {}",
                            id,
                            reason
                        );
                        None
                    }
                    mfs_metadata::ResolveResult::InvalidRef { reason } => {
                        tracing::warn!("Invalid local ID '{}' for canonical ref: {}", id, reason);
                        None
                    }
                })
                .collect(),
        };

        if fail_on_conflict
            && !has_overlay_targets(
                &affected_nodes,
                &affected_edges,
                &affected_node_refs,
                &affected_edge_refs,
            )
        {
            return Err(MfsError::InvalidArgument {
                field: "affected_nodes/affected_edges/affected_node_refs/affected_edge_refs"
                    .into(),
                reason: "fail_on_conflict requires at least one affected local id or canonical ref to detect conflicts against".into(),
            });
        }

        // Validate that affected node/edge IDs exist in the repo
        self.validate_affected_refs(repo_id, &affected_nodes, &affected_edges)?;

        let affected_nodes_json =
            serde_json::to_string(&affected_nodes).unwrap_or_else(|_| "[]".into());
        let affected_edges_json =
            serde_json::to_string(&affected_edges).unwrap_or_else(|_| "[]".into());
        let affected_node_refs_json =
            serde_json::to_string(&affected_node_refs).unwrap_or_else(|_| "[]".into());
        let affected_edge_refs_json =
            serde_json::to_string(&affected_edge_refs).unwrap_or_else(|_| "[]".into());
        let content_json =
            content_json_with_idempotency(input.content_json, idempotency_key.as_deref());
        let content_json_str =
            serde_json::to_string(&content_json).map_err(|e| MfsError::InvalidArgument {
                field: "content_json".into(),
                reason: e.to_string(),
            })?;

        let record = OverlayRecord {
            id: &id,
            repo_id,
            overlay_type: &input.overlay_type,
            tracker: &tracker,
            tracker_content_id: &input.tracker_content_id,
            tracker_project_item_id: input.tracker_project_item_id.as_deref(),
            tracker_identifier: &input.tracker_identifier,
            issue_number: input.issue_number,
            branch: input.branch.as_deref(),
            pr_url: input.pr_url.as_deref(),
            agent_session_id: input.agent_session_id.as_deref(),
            author: &input.author,
            status: "proposed",
            content_json: &content_json_str,
            affected_nodes: Some(&affected_nodes_json),
            affected_edges: Some(&affected_edges_json),
            affected_node_refs: Some(&affected_node_refs_json),
            affected_edge_refs: Some(&affected_edge_refs_json),
            created_at: &now,
            updated_at: &now,
            superseded_by: None,
            manifest_id: Some(repo_id),
            accepted_at: None,
            implemented_at: None,
            merged_at: None,
            stale_at: None,
            abandoned_at: None,
        };

        // Record transition: (none) → proposed
        let transition_id = format!("trans_{}", uuid::Uuid::new_v4());
        let transition = OverlayTransitionRecord {
            id: &transition_id,
            overlay_id: &id,
            from_status: "(none)",
            to_status: "proposed",
            triggered_by: "agent",
            reason: None,
            created_at: &now,
        };

        if fail_on_conflict {
            let affected_node_keys = overlay_conflict_keys(&affected_nodes, &affected_node_refs);
            let affected_edge_keys = overlay_conflict_keys(&affected_edges, &affected_edge_refs);
            let conflicts = self
                .metadata
                .insert_overlay_unless_active_conflict(
                    &record,
                    &transition,
                    &affected_node_keys,
                    &affected_edge_keys,
                )
                .map_err(meta_err)?;

            if !conflicts.is_empty() {
                let data =
                    self.build_conflict_data(&affected_node_keys, &affected_edge_keys, &conflicts);

                return Ok(ProposeOutput {
                    overlay_id: String::new(), // conflict → no overlay created
                    overlay_type: input.overlay_type,
                    status: "conflict".into(),
                    version_hash: now,
                    idempotent_replay: false,
                    data,
                    conflict_hint: Some(
                        "Resolve or accept the conflicting overlay before proposing this overlay."
                            .into(),
                    ),
                });
            }
        } else {
            self.metadata
                .insert_overlay_with_transition(&record, &transition)
                .map_err(meta_err)?;
        }

        let data = json!({
            "overlay_id": id,
            "status": "proposed",
            "affected_node_refs": affected_node_refs,
            "affected_edge_refs": affected_edge_refs,
            "idempotent_replay": false,
        });

        Ok(ProposeOutput {
            overlay_id: id,
            overlay_type: input.overlay_type,
            status: "ok".into(),
            version_hash: now,
            idempotent_replay: false,
            data,
            conflict_hint: None,
        })
    }

    // ─── Accept ────────────────────────────────────────────────────────

    /// Accept an overlay. Only human can accept (enforced by handler validation).
    /// Transitions overlay from "proposed" to "accepted" and records transition.
    pub fn accept(&self, overlay_id: &str, acceptor: &str) -> Result<StoredOverlay, MfsError> {
        let overlay = self.metadata.get_overlay(overlay_id).map_err(meta_err)?;

        let overlay = overlay.ok_or_else(|| MfsError::NotFound {
            resource: format!("overlay:{}", overlay_id),
        })?;

        if !is_valid_transition(&overlay.status, "accepted") {
            return Err(MfsError::InvalidArgument {
                field: "status".into(),
                reason: format!(
                    "cannot transition overlay from '{}' to 'accepted'",
                    overlay.status
                ),
            });
        }

        let from_status = overlay.status.clone();
        let now = chrono::Utc::now().to_rfc3339();

        self.metadata
            .update_overlay_status(overlay_id, "accepted", &now)
            .map_err(meta_err)?;

        self.metadata
            .set_overlay_status_timestamp(overlay_id, "accepted_at", &now)
            .map_err(meta_err)?;

        let transition = OverlayTransitionRecord {
            id: &format!("trans_{}", uuid::Uuid::new_v4()),
            overlay_id,
            from_status: &from_status,
            to_status: "accepted",
            triggered_by: acceptor,
            reason: None,
            created_at: &now,
        };
        self.metadata
            .insert_overlay_transition(&transition)
            .map_err(meta_err)?;

        Ok(overlay)
    }

    // ─── Mark Implemented ──────────────────────────────────────────────

    /// Mark an overlay as implemented. Transitions overlay from its current status
    /// to "implemented" and records transition.
    pub fn mark_implemented(
        &self,
        overlay_id: &str,
        session_id: Option<&str>,
    ) -> Result<StoredOverlay, MfsError> {
        let overlay = self.metadata.get_overlay(overlay_id).map_err(meta_err)?;

        let overlay = overlay.ok_or_else(|| MfsError::NotFound {
            resource: format!("overlay:{}", overlay_id),
        })?;

        if !is_valid_transition(&overlay.status, "implemented") {
            return Err(MfsError::InvalidArgument {
                field: "status".into(),
                reason: format!(
                    "cannot transition overlay from '{}' to 'implemented'",
                    overlay.status
                ),
            });
        }

        let from_status = overlay.status.clone();
        let now = chrono::Utc::now().to_rfc3339();

        self.metadata
            .update_overlay_status(overlay_id, "implemented", &now)
            .map_err(meta_err)?;

        self.metadata
            .set_overlay_status_timestamp(overlay_id, "implemented_at", &now)
            .map_err(meta_err)?;

        let transition = OverlayTransitionRecord {
            id: &format!("trans_{}", uuid::Uuid::new_v4()),
            overlay_id,
            from_status: &from_status,
            to_status: "implemented",
            triggered_by: "agent",
            reason: session_id,
            created_at: &now,
        };
        self.metadata
            .insert_overlay_transition(&transition)
            .map_err(meta_err)?;

        Ok(overlay)
    }

    // ─── Abandon ───────────────────────────────────────────────────────

    /// Abandon an overlay. Enforces that:
    /// - triggered_by must be "human" or "agent"
    /// - implemented overlays can only be abandoned by "human"
    /// - status transition must be valid
    pub fn abandon(
        &self,
        overlay_id: &str,
        reason: Option<&str>,
        actor: Option<&str>,
    ) -> Result<StoredOverlay, MfsError> {
        let overlay = self.metadata.get_overlay(overlay_id).map_err(meta_err)?;

        let overlay = overlay.ok_or_else(|| MfsError::NotFound {
            resource: format!("overlay:{}", overlay_id),
        })?;

        if !is_valid_transition(&overlay.status, "abandoned") {
            return Err(MfsError::InvalidArgument {
                field: "status".into(),
                reason: format!(
                    "cannot transition overlay from '{}' to 'abandoned'",
                    overlay.status
                ),
            });
        }

        let from_status = overlay.status.clone();
        let triggered_by = actor.unwrap_or("agent");
        if !["human", "agent"].contains(&triggered_by) {
            return Err(MfsError::InvalidArgument {
                field: "abandoner".into(),
                reason: "must be one of human, agent".into(),
            });
        }
        if from_status == "implemented" && triggered_by != "human" {
            return Err(MfsError::InvalidArgument {
                field: "abandoner".into(),
                reason: "implemented overlays can only be abandoned by human".into(),
            });
        }

        let now = chrono::Utc::now().to_rfc3339();

        self.metadata
            .update_overlay_status(overlay_id, "abandoned", &now)
            .map_err(meta_err)?;

        self.metadata
            .set_overlay_status_timestamp(overlay_id, "abandoned_at", &now)
            .map_err(meta_err)?;

        let transition = OverlayTransitionRecord {
            id: &format!("trans_{}", uuid::Uuid::new_v4()),
            overlay_id,
            from_status: &from_status,
            to_status: "abandoned",
            triggered_by,
            reason,
            created_at: &now,
        };
        self.metadata
            .insert_overlay_transition(&transition)
            .map_err(meta_err)?;

        Ok(overlay)
    }

    // ─── Report Conflict ───────────────────────────────────────────────

    /// Analyze whether two overlays conflict (overlap on nodes/edges).
    /// Returns ConflictAnalysis with overlap details.
    pub fn report_conflict(
        &self,
        repo_id: &str,
        id1: &str,
        id2: &str,
        desc: Option<&str>,
    ) -> Result<ConflictAnalysis, MfsError> {
        let overlay_a = self.metadata.get_overlay(id1).map_err(meta_err)?;
        let overlay_b = self.metadata.get_overlay(id2).map_err(meta_err)?;

        let a = overlay_a.ok_or_else(|| MfsError::NotFound {
            resource: format!("overlay:{}", id1),
        })?;
        let b = overlay_b.ok_or_else(|| MfsError::NotFound {
            resource: format!("overlay:{}", id2),
        })?;

        if a.repo_id != repo_id || b.repo_id != repo_id {
            return Err(MfsError::InvalidArgument {
                field: "repo_id".into(),
                reason: "both overlays must belong to the requested repo_id".into(),
            });
        }

        let a_nodes = parse_overlay_refs(a.affected_nodes.as_deref());
        let b_nodes = parse_overlay_refs(b.affected_nodes.as_deref());
        let a_edges = parse_overlay_refs(a.affected_edges.as_deref());
        let b_edges = parse_overlay_refs(b.affected_edges.as_deref());
        let a_node_refs = parse_overlay_refs(a.affected_node_refs.as_deref());
        let b_node_refs = parse_overlay_refs(b.affected_node_refs.as_deref());
        let a_edge_refs = parse_overlay_refs(a.affected_edge_refs.as_deref());
        let b_edge_refs = parse_overlay_refs(b.affected_edge_refs.as_deref());

        let a_keys = overlay_target_keys(&a_nodes, &a_edges, &a_node_refs, &a_edge_refs);
        let b_keys = overlay_target_keys(&b_nodes, &b_edges, &b_node_refs, &b_edge_refs);
        let overlap = overlay_overlap(&a_keys, &b_keys);
        let has_conflict = overlap.has_conflict;
        let active_statuses = ["accepted", "implemented"];
        let requires_human_review = has_conflict
            && (active_statuses.contains(&a.status.as_str())
                || active_statuses.contains(&b.status.as_str()));

        let conflict_id = format!("conflict_{}", uuid::Uuid::new_v4());

        // Build per-overlay conflict details
        let details = vec![
            ConflictDetail {
                overlay_id: a.id,
                overlay_type: a.overlay_type,
                status: a.status,
                overlap_nodes: overlap.nodes.clone(),
                overlap_edges: overlap.edges.clone(),
            },
            ConflictDetail {
                overlay_id: b.id,
                overlay_type: b.overlay_type,
                status: b.status,
                overlap_nodes: overlap.nodes.clone(),
                overlap_edges: overlap.edges.clone(),
            },
        ];

        Ok(ConflictAnalysis {
            conflict_id,
            has_conflict,
            requires_human_review,
            overlap_nodes: overlap.nodes,
            overlap_edges: overlap.edges,
            description: desc.map(str::to_owned),
            details,
        })
    }

    // ─── Record Conflict ───────────────────────────────────────────────

    /// Record a conflict as a conflict_declaration overlay.
    pub fn record_conflict(&self, input: RecordConflictInput) -> Result<StoredOverlay, MfsError> {
        if input.conflict_summary.trim().is_empty() {
            return Err(MfsError::InvalidArgument {
                field: "conflict_summary".into(),
                reason: "conflict_summary must be non-empty".into(),
            });
        }

        let repo_id = &input.repo_id;

        let identity = self
            .metadata
            .get_manifest_identity(repo_id)
            .map_err(meta_err)?;
        if identity.is_none() {
            return Err(MfsError::NotFound {
                resource: format!("manifest:{}", repo_id),
            });
        }

        let conflict_id = format!("conflict_{}", uuid::Uuid::new_v4());
        let now = chrono::Utc::now().to_rfc3339();
        let tracker = input.tracker.unwrap_or_else(|| "github_projects".into());
        let tracker_identifier = input
            .tracker_identifier
            .clone()
            .or_else(|| input.run_id.clone())
            .unwrap_or_else(|| conflict_id.clone());
        let tracker_content_id = input
            .run_id
            .as_deref()
            .map(|run_id| format!("run:{run_id}"))
            .unwrap_or_else(|| conflict_id.clone());
        let severity = input.severity.unwrap_or_else(|| "blocking".into());
        let author = input.author.unwrap_or_else(|| "agent".into());

        let content_json = json!({
            "conflict_id": conflict_id,
            "run_id": input.run_id,
            "tracker_identifier": tracker_identifier,
            "conflict_summary": input.conflict_summary,
            "severity": severity,
            "evidence": input.evidence.unwrap_or_else(|| json!({})),
        });
        let content_json_str =
            serde_json::to_string(&content_json).map_err(|e| MfsError::InvalidArgument {
                field: "content_json".into(),
                reason: e.to_string(),
            })?;

        let empty_refs = "[]";
        let record = OverlayRecord {
            id: &conflict_id,
            repo_id,
            overlay_type: "conflict_declaration",
            tracker: &tracker,
            tracker_content_id: &tracker_content_id,
            tracker_project_item_id: None,
            tracker_identifier: &tracker_identifier,
            issue_number: None,
            branch: None,
            pr_url: None,
            agent_session_id: input.run_id.as_deref(),
            author: &author,
            status: "proposed",
            content_json: &content_json_str,
            affected_nodes: Some(empty_refs),
            affected_edges: Some(empty_refs),
            affected_node_refs: Some(empty_refs),
            affected_edge_refs: Some(empty_refs),
            created_at: &now,
            updated_at: &now,
            superseded_by: None,
            manifest_id: Some(repo_id),
            accepted_at: None,
            implemented_at: None,
            merged_at: None,
            stale_at: None,
            abandoned_at: None,
        };
        self.metadata.insert_overlay(&record).map_err(meta_err)?;

        let transition = OverlayTransitionRecord {
            id: &format!("trans_{}", uuid::Uuid::new_v4()),
            overlay_id: &conflict_id,
            from_status: "(none)",
            to_status: "proposed",
            triggered_by: &author,
            reason: Some("run-level conflict"),
            created_at: &now,
        };
        self.metadata
            .insert_overlay_transition(&transition)
            .map_err(meta_err)?;

        // Return a synthetic StoredOverlay-like result for the handler to format.
        // We don't have the full StoredOverlay from the insert, so we construct one.
        let stored = StoredOverlay {
            id: conflict_id.clone(),
            repo_id: repo_id.clone(),
            overlay_type: "conflict_declaration".into(),
            tracker: tracker.clone(),
            tracker_content_id: tracker_content_id.clone(),
            tracker_project_item_id: None,
            tracker_identifier: tracker_identifier.clone(),
            issue_number: None,
            branch: None,
            pr_url: None,
            agent_session_id: input.run_id.clone(),
            author: author.clone(),
            status: "proposed".into(),
            content_json: content_json_str,
            affected_nodes: Some("[]".into()),
            affected_edges: Some("[]".into()),
            affected_node_refs: Some("[]".into()),
            affected_edge_refs: Some("[]".into()),
            created_at: now.clone(),
            updated_at: now.clone(),
            superseded_by: None,
            manifest_id: Some(repo_id.clone()),
            accepted_at: None,
            implemented_at: None,
            merged_at: None,
            stale_at: None,
            abandoned_at: None,
        };

        Ok(stored)
    }

    // ─── Consolidate ───────────────────────────────────────────────────

    /// Consolidate overlays by transitioning them to "merged" and creating a canvas snapshot.
    pub fn consolidate(
        &self,
        repo_id: &str,
        overlay_ids: Option<Vec<String>>,
        merge_commit: &str,
    ) -> Result<ConsolidateOutput, MfsError> {
        // Get overlays to consolidate — if none specified, consolidate all 'implemented' overlays
        let overlays_to_merge: Vec<String> = match overlay_ids {
            Some(ids) => ids,
            None => self
                .metadata
                .list_active_overlays(repo_id, Some("implemented"))
                .map_err(meta_err)?
                .iter()
                .map(|o| o.id.clone())
                .collect(),
        };

        let now = chrono::Utc::now().to_rfc3339();
        let mut merged_count = 0;

        for overlay_id in &overlays_to_merge {
            let overlay = match self.metadata.get_overlay(overlay_id).map_err(meta_err)? {
                Some(o) => o,
                None => {
                    tracing::warn!(overlay_id = %overlay_id, "consolidate: overlay not found, skipping");
                    continue;
                }
            };
            if !is_valid_transition(&overlay.status, "merged") {
                tracing::warn!(
                    overlay_id = %overlay_id,
                    current_status = %overlay.status,
                    "consolidate: invalid transition to 'merged', skipping"
                );
                continue;
            }
            let from_status = overlay.status.clone();
            self.metadata
                .update_overlay_status(overlay_id, "merged", &now)
                .map_err(meta_err)?;

            self.metadata
                .set_overlay_status_timestamp(overlay_id, "merged_at", &now)
                .map_err(meta_err)?;

            let transition = OverlayTransitionRecord {
                id: &format!("trans_{}", uuid::Uuid::new_v4()),
                overlay_id,
                from_status: &from_status,
                to_status: "merged",
                triggered_by: "consolidation",
                reason: Some(&format!("merge commit: {}", merge_commit)),
                created_at: &now,
            };
            self.metadata
                .insert_overlay_transition(&transition)
                .map_err(meta_err)?;
            merged_count += 1;
        }

        // Create snapshot
        let snapshot_id = format!("snapshot_{}", uuid::Uuid::new_v4());
        let nodes = self
            .metadata
            .list_canvas_nodes(repo_id, None, None)
            .map_err(meta_err)?;
        let edges = self
            .metadata
            .list_canvas_edges_by_repo(repo_id)
            .map_err(meta_err)?;
        let snapshot_json = serde_json::to_string(&json!({"nodes": nodes, "edges": edges}))
            .map_err(|e| MfsError::Internal {
                message: e.to_string(),
            })?;

        let snapshot_record = mfs_metadata::CanvasSnapshotRecord {
            id: &snapshot_id,
            repo_id,
            merge_commit,
            snapshot_type: "full",
            snapshot_json: &snapshot_json,
            created_at: &now,
            immutable: true,
        };
        self.metadata
            .insert_canvas_snapshot(&snapshot_record)
            .map_err(meta_err)?;

        Ok(ConsolidateOutput {
            merged_count,
            snapshot_id,
        })
    }

    // ─── List ──────────────────────────────────────────────────────────

    /// List overlays for a repo, optionally filtered by status.
    pub fn list(
        &self,
        repo_id: &str,
        status_filter: Option<&str>,
    ) -> Result<Vec<StoredOverlay>, MfsError> {
        self.metadata
            .list_active_overlays(repo_id, status_filter)
            .map_err(meta_err)
    }

    // ─── Private helpers ───────────────────────────────────────────────

    /// Find an overlay by idempotency key among active overlays for a repo.
    fn find_overlay_by_idempotency_key(
        &self,
        repo_id: &str,
        idempotency_key: &str,
    ) -> Result<Option<StoredOverlay>, MfsError> {
        let overlays = self
            .metadata
            .list_active_overlays(repo_id, None)
            .map_err(meta_err)?;

        Ok(overlays
            .into_iter()
            .find(|overlay| overlay_idempotency_key(overlay).as_deref() == Some(idempotency_key)))
    }

    /// Validate that affected node/edge IDs exist and belong to the given repo.
    fn validate_affected_refs(
        &self,
        repo_id: &str,
        node_ids: &[String],
        edge_ids: &[String],
    ) -> Result<(), MfsError> {
        for node_id in node_ids {
            let node = self.metadata.get_canvas_node(node_id).map_err(meta_err)?;
            match node {
                Some(node) if node.repo_id == repo_id => {}
                Some(_) => {
                    return Err(MfsError::InvalidArgument {
                        field: "affected_nodes".into(),
                        reason: format!("node '{}' belongs to a different repo", node_id),
                    });
                }
                None => {
                    return Err(MfsError::InvalidArgument {
                        field: "affected_nodes".into(),
                        reason: format!("node '{}' does not exist", node_id),
                    });
                }
            }
        }
        for edge_id in edge_ids {
            let edge = self.metadata.get_canvas_edge(edge_id).map_err(meta_err)?;
            match edge {
                Some(edge) if edge.repo_id == repo_id => {}
                Some(_) => {
                    return Err(MfsError::InvalidArgument {
                        field: "affected_edges".into(),
                        reason: format!("edge '{}' belongs to a different repo", edge_id),
                    });
                }
                None => {
                    return Err(MfsError::InvalidArgument {
                        field: "affected_edges".into(),
                        reason: format!("edge '{}' does not exist", edge_id),
                    });
                }
            }
        }
        Ok(())
    }

    /// Build conflict data JSON from overlapping overlays.
    fn build_conflict_data(
        &self,
        affected_node_keys: &[String],
        affected_edge_keys: &[String],
        conflicts: &[StoredOverlay],
    ) -> Value {
        let conflict_details: Vec<Value> = conflicts
            .iter()
            .map(|overlay| {
                let overlay_node_keys = overlay_conflict_keys(
                    &parse_overlay_refs(overlay.affected_nodes.as_deref()),
                    &parse_overlay_refs(overlay.affected_node_refs.as_deref()),
                );
                let overlay_edge_keys = overlay_conflict_keys(
                    &parse_overlay_refs(overlay.affected_edges.as_deref()),
                    &parse_overlay_refs(overlay.affected_edge_refs.as_deref()),
                );
                let overlap_nodes: Vec<String> = affected_node_keys
                    .iter()
                    .filter(|node| overlay_node_keys.contains(*node))
                    .cloned()
                    .collect();
                let overlap_edges: Vec<String> = affected_edge_keys
                    .iter()
                    .filter(|edge| overlay_edge_keys.contains(*edge))
                    .cloned()
                    .collect();

                json!({
                    "overlay_id": overlay.id,
                    "tracker_identifier": overlay.tracker_identifier,
                    "status": overlay.status,
                    "overlap_nodes": overlap_nodes,
                    "overlap_edges": overlap_edges,
                    "requires_human_review": true,
                })
            })
            .collect();

        json!({
            "overlay_id": null,
            "status": "conflict",
            "conflicts": conflict_details,
        })
    }
}

// ─── Error helper ───────────────────────────────────────────────────────

fn meta_err<T: std::fmt::Display>(e: T) -> MfsError {
    MfsError::Internal {
        message: e.to_string(),
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_valid_transition_happy_paths() {
        assert!(is_valid_transition("proposed", "accepted"));
        assert!(is_valid_transition("proposed", "implemented"));
        assert!(is_valid_transition("accepted", "implemented"));
        assert!(is_valid_transition("implemented", "merged"));
        assert!(is_valid_transition("accepted", "abandoned"));
        assert!(is_valid_transition("implemented", "abandoned"));
        assert!(is_valid_transition("anything", "stale"));
        assert!(is_valid_transition("stale", "proposed"));
    }

    #[test]
    fn is_valid_transition_invalid_paths() {
        assert!(!is_valid_transition("merged", "proposed"));
        assert!(!is_valid_transition("abandoned", "accepted"));
        assert!(!is_valid_transition("accepted", "proposed"));
        assert!(!is_valid_transition("stale", "accepted"));
    }

    #[test]
    fn overlay_conflict_keys_merges_both() {
        let local = vec!["node_1".into()];
        let refs = vec!["canvas://nodes/node_1".into()];
        let keys = overlay_conflict_keys(&local, &refs);
        assert_eq!(keys.len(), 2);
        assert!(keys.contains(&"node_1".into()));
        assert!(keys.contains(&"canvas://nodes/node_1".into()));
    }

    #[test]
    fn overlay_overlap_includes_canonical_refs_without_local_ids() {
        let left = overlay_target_keys(
            &[],
            &[],
            &["canvas://repo-a/node/module/App.Router".into()],
            &[],
        );
        let right = overlay_target_keys(
            &[],
            &[],
            &["canvas://repo-a/node/module/App.Router".into()],
            &[],
        );

        let overlap = overlay_overlap(&left, &right);

        assert!(overlap.has_conflict);
        assert_eq!(
            overlap.nodes,
            vec!["canvas://repo-a/node/module/App.Router".to_owned()]
        );
        assert!(overlap.edges.is_empty());
    }

    #[test]
    fn overlay_targets_are_present_when_only_canonical_refs_are_provided() {
        assert!(has_overlay_targets(
            &[],
            &[],
            &["canvas://repo-a/node/module/App.Router".into()],
            &[]
        ));
        assert!(has_overlay_targets(
            &[],
            &[],
            &[],
            &["canvas://repo-a/edge/call/App.Router->App.Controller".into()]
        ));
    }

    #[test]
    fn canonical_refs_must_belong_to_requested_repo() {
        let result = validate_canonical_refs_for_repo(
            "repo-a",
            "affected_node_refs",
            &["canvas://repo-b/node/module/App.Router".into()],
            CanvasRefKind::Node,
        );

        assert!(matches!(
            result,
            Err(MfsError::InvalidArgument { ref field, .. }) if field == "affected_node_refs"
        ));
    }

    #[test]
    fn canonical_node_refs_reject_edge_refs() {
        let result = validate_canonical_refs_for_repo(
            "repo-a",
            "affected_node_refs",
            &["canvas://repo-a/edge/call/App.Router->App.Controller".into()],
            CanvasRefKind::Node,
        );

        assert!(matches!(
            result,
            Err(MfsError::InvalidArgument { ref field, .. }) if field == "affected_node_refs"
        ));
    }

    #[test]
    fn canonical_edge_refs_reject_node_refs() {
        let result = validate_canonical_refs_for_repo(
            "repo-a",
            "affected_edge_refs",
            &["canvas://repo-a/node/module/App.Router".into()],
            CanvasRefKind::Edge,
        );

        assert!(matches!(
            result,
            Err(MfsError::InvalidArgument { ref field, .. }) if field == "affected_edge_refs"
        ));
    }

    #[test]
    fn overlay_idempotency_key_extracts_from_memfuse() {
        let overlay = StoredOverlay {
            content_json: serde_json::to_string(&json!({
                "_memfuse": { "idempotency_key": "abc123" },
                "data": "hello"
            }))
            .unwrap(),
            ..default_stored_overlay()
        };
        assert_eq!(overlay_idempotency_key(&overlay), Some("abc123".into()));
    }

    #[test]
    fn overlay_idempotency_key_returns_none_when_absent() {
        let overlay = StoredOverlay {
            content_json: serde_json::to_string(&json!({"data": "hello"})).unwrap(),
            ..default_stored_overlay()
        };
        assert!(overlay_idempotency_key(&overlay).is_none());
    }

    #[test]
    fn content_json_with_idempotency_injects_key() {
        let input = json!({"data": "hello"});
        let result = content_json_with_idempotency(input, Some("my-key"));
        assert_eq!(
            result.pointer("/_memfuse/idempotency_key"),
            Some(&Value::String("my-key".into()))
        );
    }

    #[test]
    fn content_json_with_idempotency_no_key_returns_unchanged() {
        let input = json!({"data": "hello"});
        let result = content_json_with_idempotency(input.clone(), None);
        assert_eq!(result, input);
    }

    #[test]
    fn content_json_with_idempotency_preserves_existing_memfuse() {
        let input = json!({
            "_memfuse": { "existing_field": "val" },
            "data": "hello"
        });
        let result = content_json_with_idempotency(input, Some("my-key"));
        let memfuse = result.pointer("/_memfuse").unwrap();
        assert_eq!(
            memfuse.get("existing_field"),
            Some(&Value::String("val".into()))
        );
        assert_eq!(
            memfuse.get("idempotency_key"),
            Some(&Value::String("my-key".into()))
        );
    }

    #[test]
    fn content_json_with_idempotency_wraps_non_object() {
        let input = Value::String("just a string".into());
        let result = content_json_with_idempotency(input, Some("my-key"));
        assert_eq!(
            result.get("value"),
            Some(&Value::String("just a string".into()))
        );
        assert_eq!(
            result.pointer("/_memfuse/idempotency_key"),
            Some(&Value::String("my-key".into()))
        );
    }

    #[test]
    fn parse_overlay_refs_parses_json_array() {
        let raw = Some("[\"n1\",\"n2\"]");
        let refs = parse_overlay_refs(raw);
        assert_eq!(refs, vec!["n1", "n2"]);
    }

    #[test]
    fn parse_overlay_refs_returns_empty_on_none() {
        assert!(parse_overlay_refs(None).is_empty());
    }

    #[test]
    fn parse_overlay_refs_returns_empty_on_invalid() {
        assert!(parse_overlay_refs(Some("not json")).is_empty());
    }

    #[test]
    fn parse_status_filter_comma_separated() {
        let result = parse_status_filter(Some("proposed,accepted"));
        assert_eq!(result, vec!["proposed", "accepted"]);
    }

    #[test]
    fn parse_status_filter_none_returns_empty() {
        assert!(parse_status_filter(None).is_empty());
    }

    #[test]
    fn parse_status_filter_empty_string_returns_empty() {
        assert!(parse_status_filter(Some("")).is_empty());
    }

    fn default_stored_overlay() -> StoredOverlay {
        StoredOverlay {
            id: "overlay_test".into(),
            repo_id: "repo_test".into(),
            overlay_type: "planned_change".into(),
            tracker: "github_projects".into(),
            tracker_content_id: "tc_1".into(),
            tracker_project_item_id: None,
            tracker_identifier: "issue#1".into(),
            issue_number: None,
            branch: None,
            pr_url: None,
            agent_session_id: None,
            author: "agent".into(),
            status: "proposed".into(),
            content_json: "{}".into(),
            affected_nodes: None,
            affected_edges: None,
            affected_node_refs: None,
            affected_edge_refs: None,
            created_at: "2026-01-01T00:00:00Z".into(),
            updated_at: "2026-01-01T00:00:00Z".into(),
            superseded_by: None,
            manifest_id: None,
            accepted_at: None,
            implemented_at: None,
            merged_at: None,
            stale_at: None,
            abandoned_at: None,
        }
    }
}
