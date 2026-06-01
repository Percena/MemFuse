//! Conflict detection for canvas overlays.

use mfs_metadata::StoredOverlay;
use serde_json::{Value, json};

pub(super) fn detect_canvas_conflicts(
    overlays: &[StoredOverlay],
    _node_ids: &[String],
) -> Vec<Value> {
    let active_statuses = ["accepted", "implemented"];
    let mut conflicts = Vec::new();
    let active_overlays: Vec<&StoredOverlay> = overlays
        .iter()
        .filter(|o| active_statuses.contains(&o.status.as_str()))
        .collect();

    for i in 0..active_overlays.len() {
        for j in (i + 1)..active_overlays.len() {
            let a = active_overlays[i];
            let b = active_overlays[j];
            let a_nodes: Vec<String> =
                serde_json::from_str(&a.affected_nodes.clone().unwrap_or_default())
                    .unwrap_or_default();
            let b_nodes: Vec<String> =
                serde_json::from_str(&b.affected_nodes.clone().unwrap_or_default())
                    .unwrap_or_default();
            let a_edges: Vec<String> =
                serde_json::from_str(&a.affected_edges.clone().unwrap_or_default())
                    .unwrap_or_default();
            let b_edges: Vec<String> =
                serde_json::from_str(&b.affected_edges.clone().unwrap_or_default())
                    .unwrap_or_default();

            let nodes_overlap = a_nodes.iter().any(|n| b_nodes.contains(n));
            let edges_overlap = a_edges.iter().any(|e| b_edges.contains(e));

            if nodes_overlap || edges_overlap {
                conflicts.push(json!({
                    "overlay_id_1": a.id,
                    "overlay_id_2": b.id,
                    "requires_human_review": true,
                }));
            }
        }
    }
    conflicts
}
