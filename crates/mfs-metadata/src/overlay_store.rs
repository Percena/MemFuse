//! Overlay store methods — extracted from the monolithic MetadataStore impl.
//!
//! Active overlays and overlay state transitions belong to the Canvas connection
//! domain (they use `self.lock_canvas_conn()`).  These methods are separated here
//! for maintainability, following the same pattern as `canvas_store.rs`.

use rusqlite::{Result, params};

use crate::store::{MetadataStore, OverlayRecord, OverlayTransitionRecord, StoredOverlay};

// ─── Active Overlays ───────────────────────────────────────────────

impl MetadataStore {
    pub fn insert_overlay(&self, record: &OverlayRecord<'_>) -> Result<()> {
        self.lock_canvas_conn()?.execute(
            "INSERT INTO active_overlays (
                id, repo_id, overlay_type, tracker, tracker_content_id,
                tracker_project_item_id, tracker_identifier, issue_number,
                branch, pr_url, agent_session_id, author, status,
                content_json, affected_nodes, affected_edges,
                affected_node_refs, affected_edge_refs,
                created_at, updated_at, superseded_by,
                manifest_id, accepted_at, implemented_at, merged_at, stale_at, abandoned_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27)",
            params![
                record.id,
                record.repo_id,
                record.overlay_type,
                record.tracker,
                record.tracker_content_id,
                record.tracker_project_item_id,
                record.tracker_identifier,
                record.issue_number,
                record.branch,
                record.pr_url,
                record.agent_session_id,
                record.author,
                record.status,
                record.content_json,
                record.affected_nodes,
                record.affected_edges,
                record.affected_node_refs,
                record.affected_edge_refs,
                record.created_at,
                record.updated_at,
                record.superseded_by,
                record.manifest_id,
                record.accepted_at,
                record.implemented_at,
                record.merged_at,
                record.stale_at,
                record.abandoned_at,
            ],
        )?;
        Ok(())
    }

    pub fn update_overlay_status(
        &self,
        overlay_id: &str,
        new_status: &str,
        updated_at: &str,
    ) -> Result<usize> {
        self.lock_canvas_conn()?.execute(
            "UPDATE active_overlays SET status = ?1, updated_at = ?2 WHERE id = ?3",
            params![new_status, updated_at, overlay_id],
        )
    }

    pub fn set_overlay_status_timestamp(
        &self,
        overlay_id: &str,
        column: &str,
        value: &str,
    ) -> Result<usize> {
        let sql = match column {
            "accepted_at" => "UPDATE active_overlays SET accepted_at = ?1 WHERE id = ?2",
            "implemented_at" => "UPDATE active_overlays SET implemented_at = ?1 WHERE id = ?2",
            "merged_at" => "UPDATE active_overlays SET merged_at = ?1 WHERE id = ?2",
            "stale_at" => "UPDATE active_overlays SET stale_at = ?1 WHERE id = ?2",
            "abandoned_at" => "UPDATE active_overlays SET abandoned_at = ?1 WHERE id = ?2",
            _ => return Err(rusqlite::Error::InvalidParameterName(column.into())),
        };
        self.lock_canvas_conn()?
            .execute(sql, params![value, overlay_id])
    }

    pub fn get_overlay(&self, overlay_id: &str) -> Result<Option<StoredOverlay>> {
        let conn = self.lock_canvas_conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, repo_id, overlay_type, tracker, tracker_content_id,
                    tracker_project_item_id, tracker_identifier, issue_number,
                    branch, pr_url, agent_session_id, author, status,
                    content_json, affected_nodes, affected_edges,
                    affected_node_refs, affected_edge_refs,
                    created_at, updated_at, superseded_by,
                    manifest_id, accepted_at, implemented_at, merged_at, stale_at, abandoned_at
             FROM active_overlays WHERE id = ?1 LIMIT 1",
        )?;
        let mut rows = stmt.query(params![overlay_id])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        Ok(Some(stored_overlay_from_row(row)?))
    }

    pub fn list_active_overlays(
        &self,
        repo_id: &str,
        status_filter: Option<&str>,
    ) -> Result<Vec<StoredOverlay>> {
        let conn = self.lock_canvas_conn()?;
        let sql = match status_filter {
            Some(_) => {
                "SELECT id, repo_id, overlay_type, tracker, tracker_content_id, tracker_project_item_id, tracker_identifier, issue_number, branch, pr_url, agent_session_id, author, status, content_json, affected_nodes, affected_edges, affected_node_refs, affected_edge_refs, created_at, updated_at, superseded_by, manifest_id, accepted_at, implemented_at, merged_at, stale_at, abandoned_at FROM active_overlays WHERE repo_id = ?1 AND status = ?2 ORDER BY created_at DESC"
            }
            None => {
                "SELECT id, repo_id, overlay_type, tracker, tracker_content_id, tracker_project_item_id, tracker_identifier, issue_number, branch, pr_url, agent_session_id, author, status, content_json, affected_nodes, affected_edges, affected_node_refs, affected_edge_refs, created_at, updated_at, superseded_by, manifest_id, accepted_at, implemented_at, merged_at, stale_at, abandoned_at FROM active_overlays WHERE repo_id = ?1 ORDER BY created_at DESC"
            }
        };
        let mut stmt = conn.prepare(sql)?;
        let rows = match status_filter {
            Some(s) => stmt.query_map(params![repo_id, s], stored_overlay_from_row)?,
            None => stmt.query_map(params![repo_id], stored_overlay_from_row)?,
        };
        rows.collect()
    }

    pub fn find_overlapping_overlays(
        &self,
        repo_id: &str,
        node_ids: &[String],
        edge_ids: &[String],
        exclude_overlay_id: Option<&str>,
    ) -> Result<Vec<StoredOverlay>> {
        // Find overlays whose affected_nodes or affected_edges overlap with given sets,
        // and whose status is 'accepted' or 'implemented'
        let conn = self.lock_canvas_conn()?;
        let mut overlays = Vec::new();
        let all_overlays: Vec<StoredOverlay> = {
            let mut stmt = conn.prepare(
                "SELECT id, repo_id, overlay_type, tracker, tracker_content_id,
                        tracker_project_item_id, tracker_identifier, issue_number,
                        branch, pr_url, agent_session_id, author, status,
                        content_json, affected_nodes, affected_edges,
                        affected_node_refs, affected_edge_refs,
                        created_at, updated_at, superseded_by,
                        manifest_id, accepted_at, implemented_at, merged_at, stale_at, abandoned_at
                 FROM active_overlays
                 WHERE repo_id = ?1 AND status IN ('accepted','implemented')",
            )?;
            let rows = stmt.query_map(params![repo_id], stored_overlay_from_row)?;
            rows.collect::<Result<Vec<_>, _>>()?
        };

        for overlay in all_overlays {
            if let Some(exclude) = exclude_overlay_id {
                if overlay.id == exclude {
                    continue;
                }
            }
            let overlay_nodes: Vec<String> =
                serde_json::from_str(&overlay.affected_nodes.clone().unwrap_or_default())
                    .unwrap_or_default();
            let overlay_edges: Vec<String> =
                serde_json::from_str(&overlay.affected_edges.clone().unwrap_or_default())
                    .unwrap_or_default();
            let nodes_overlap =
                !node_ids.is_empty() && overlay_nodes.iter().any(|n| node_ids.contains(n));
            let edges_overlap =
                !edge_ids.is_empty() && overlay_edges.iter().any(|e| edge_ids.contains(e));
            if nodes_overlap || edges_overlap {
                overlays.push(overlay);
            }
        }
        Ok(overlays)
    }

    pub fn insert_overlay_unless_active_conflict(
        &self,
        record: &OverlayRecord<'_>,
        transition: &OverlayTransitionRecord<'_>,
        affected_node_keys: &[String],
        affected_edge_keys: &[String],
    ) -> Result<Vec<StoredOverlay>> {
        let mut conn = self.lock_canvas_conn()?;
        let tx = conn.transaction()?;
        let conflicts: Vec<StoredOverlay> = {
            let mut stmt = tx.prepare(
                "SELECT id, repo_id, overlay_type, tracker, tracker_content_id,
                        tracker_project_item_id, tracker_identifier, issue_number,
                        branch, pr_url, agent_session_id, author, status,
                        content_json, affected_nodes, affected_edges,
                        affected_node_refs, affected_edge_refs,
                        created_at, updated_at, superseded_by,
                        manifest_id, accepted_at, implemented_at, merged_at, stale_at, abandoned_at
                 FROM active_overlays
                 WHERE repo_id = ?1 AND status IN ('proposed','accepted','implemented')",
            )?;
            let rows = stmt.query_map(params![record.repo_id], stored_overlay_from_row)?;
            rows.collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .filter(|overlay| overlay_overlaps(overlay, affected_node_keys, affected_edge_keys))
                .collect()
        };

        if !conflicts.is_empty() {
            drop(tx); // explicit rollback — no writes were made
            return Ok(conflicts);
        }

        tx.execute(
            "INSERT INTO active_overlays (
                id, repo_id, overlay_type, tracker, tracker_content_id,
                tracker_project_item_id, tracker_identifier, issue_number,
                branch, pr_url, agent_session_id, author, status,
                content_json, affected_nodes, affected_edges,
                affected_node_refs, affected_edge_refs,
                created_at, updated_at, superseded_by,
                manifest_id, accepted_at, implemented_at, merged_at, stale_at, abandoned_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27)",
            params![
                record.id,
                record.repo_id,
                record.overlay_type,
                record.tracker,
                record.tracker_content_id,
                record.tracker_project_item_id,
                record.tracker_identifier,
                record.issue_number,
                record.branch,
                record.pr_url,
                record.agent_session_id,
                record.author,
                record.status,
                record.content_json,
                record.affected_nodes,
                record.affected_edges,
                record.affected_node_refs,
                record.affected_edge_refs,
                record.created_at,
                record.updated_at,
                record.superseded_by,
                record.manifest_id,
                record.accepted_at,
                record.implemented_at,
                record.merged_at,
                record.stale_at,
                record.abandoned_at,
            ],
        )?;
        tx.execute(
            "INSERT INTO overlay_state_transitions (
                id, overlay_id, from_status, to_status,
                triggered_by, reason, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                transition.id,
                transition.overlay_id,
                transition.from_status,
                transition.to_status,
                transition.triggered_by,
                transition.reason,
                transition.created_at,
            ],
        )?;
        tx.commit()?;
        Ok(Vec::new())
    }

    /// Insert an overlay and its initial transition in a single transaction,
    /// so that a transition-insert failure does not leave an orphan overlay.
    pub fn insert_overlay_with_transition(
        &self,
        record: &OverlayRecord<'_>,
        transition: &OverlayTransitionRecord<'_>,
    ) -> Result<()> {
        let mut conn = self.lock_canvas_conn()?;
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO active_overlays (
                id, repo_id, overlay_type, tracker, tracker_content_id,
                tracker_project_item_id, tracker_identifier, issue_number,
                branch, pr_url, agent_session_id, author, status,
                content_json, affected_nodes, affected_edges,
                affected_node_refs, affected_edge_refs,
                created_at, updated_at, superseded_by,
                manifest_id, accepted_at, implemented_at, merged_at, stale_at, abandoned_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27)",
            params![
                record.id,
                record.repo_id,
                record.overlay_type,
                record.tracker,
                record.tracker_content_id,
                record.tracker_project_item_id,
                record.tracker_identifier,
                record.issue_number,
                record.branch,
                record.pr_url,
                record.agent_session_id,
                record.author,
                record.status,
                record.content_json,
                record.affected_nodes,
                record.affected_edges,
                record.affected_node_refs,
                record.affected_edge_refs,
                record.created_at,
                record.updated_at,
                record.superseded_by,
                record.manifest_id,
                record.accepted_at,
                record.implemented_at,
                record.merged_at,
                record.stale_at,
                record.abandoned_at,
            ],
        )?;
        tx.execute(
            "INSERT INTO overlay_state_transitions (
                id, overlay_id, from_status, to_status,
                triggered_by, reason, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                transition.id,
                transition.overlay_id,
                transition.from_status,
                transition.to_status,
                transition.triggered_by,
                transition.reason,
                transition.created_at,
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    // ─── Overlay State Transitions ─────────────────────────────────────

    pub fn insert_overlay_transition(&self, record: &OverlayTransitionRecord<'_>) -> Result<()> {
        self.lock_canvas_conn()?.execute(
            "INSERT INTO overlay_state_transitions (
                id, overlay_id, from_status, to_status,
                triggered_by, reason, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                record.id,
                record.overlay_id,
                record.from_status,
                record.to_status,
                record.triggered_by,
                record.reason,
                record.created_at,
            ],
        )?;
        Ok(())
    }
}

fn overlay_overlaps(
    overlay: &StoredOverlay,
    affected_node_keys: &[String],
    affected_edge_keys: &[String],
) -> bool {
    let overlay_node_keys = overlay_keys(
        overlay.affected_nodes.as_deref(),
        overlay.affected_node_refs.as_deref(),
    );
    let overlay_edge_keys = overlay_keys(
        overlay.affected_edges.as_deref(),
        overlay.affected_edge_refs.as_deref(),
    );

    (!affected_node_keys.is_empty()
        && overlay_node_keys
            .iter()
            .any(|node| affected_node_keys.contains(node)))
        || (!affected_edge_keys.is_empty()
            && overlay_edge_keys
                .iter()
                .any(|edge| affected_edge_keys.contains(edge)))
}

fn overlay_keys(local_ids: Option<&str>, canonical_refs: Option<&str>) -> Vec<String> {
    let mut keys = parse_overlay_json_array(local_ids);
    keys.extend(parse_overlay_json_array(canonical_refs));
    keys
}

fn parse_overlay_json_array(raw: Option<&str>) -> Vec<String> {
    raw.and_then(|value| serde_json::from_str::<Vec<String>>(value).ok())
        .unwrap_or_default()
}

// ─── Row helpers ───────────────────────────────────────────────────────

fn stored_overlay_from_row(row: &rusqlite::Row<'_>) -> Result<StoredOverlay> {
    Ok(StoredOverlay {
        id: row.get(0)?,
        repo_id: row.get(1)?,
        overlay_type: row.get(2)?,
        tracker: row.get(3)?,
        tracker_content_id: row.get(4)?,
        tracker_project_item_id: row.get(5)?,
        tracker_identifier: row.get(6)?,
        issue_number: row.get(7)?,
        branch: row.get(8)?,
        pr_url: row.get(9)?,
        agent_session_id: row.get(10)?,
        author: row.get(11)?,
        status: row.get(12)?,
        content_json: row.get(13)?,
        affected_nodes: row.get(14)?,
        affected_edges: row.get(15)?,
        affected_node_refs: row.get(16)?,
        affected_edge_refs: row.get(17)?,
        created_at: row.get(18)?,
        updated_at: row.get(19)?,
        superseded_by: row.get(20)?,
        manifest_id: row.get(21)?,
        accepted_at: row.get(22)?,
        implemented_at: row.get(23)?,
        merged_at: row.get(24)?,
        stale_at: row.get(25)?,
        abandoned_at: row.get(26)?,
    })
}
