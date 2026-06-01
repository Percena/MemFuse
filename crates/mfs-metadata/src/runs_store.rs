//! Run writeback store methods for MetadataStore.
//!
//! Stores run evidence and ticket trace data so external orchestrators
//! can writeback execution results and subsequent ticket runs can read the history.

use rusqlite::{Result, params};
use serde_json::Value;

use crate::store::{MetadataStore, StoredOverlay};

impl MetadataStore {
    // ── Run Writeback CRUD ──

    pub fn insert_run_writeback(
        &self,
        repo_id: &str,
        run_id: &str,
        tracker: &str,
        tracker_identifier: &str,
        idempotency_key: &str,
        payload_json: &str,
        created_at: &str,
    ) -> Result<()> {
        self.lock_conn()?.execute(
            "INSERT OR REPLACE INTO run_writebacks (
                repo_id, run_id, tracker, tracker_identifier,
                idempotency_key, payload_json, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                repo_id,
                run_id,
                tracker,
                tracker_identifier,
                idempotency_key,
                payload_json,
                created_at
            ],
        )?;
        Ok(())
    }

    pub fn list_run_writebacks_by_tracker(
        &self,
        repo_id: &str,
        tracker_identifier: Option<&str>,
    ) -> Result<Vec<Value>> {
        let conn = self.lock_conn()?;
        let mut stmt = if tracker_identifier.is_some() {
            conn.prepare(
                "SELECT payload_json FROM run_writebacks WHERE repo_id = ?1 AND tracker_identifier = ?2 ORDER BY created_at DESC"
            )?
        } else {
            conn.prepare(
                "SELECT payload_json FROM run_writebacks WHERE repo_id = ?1 ORDER BY created_at DESC"
            )?
        };

        let rows: Vec<String> = match tracker_identifier {
            Some(s) => stmt
                .query_map(params![repo_id, s], extract_payload_json)?
                .filter_map(Result::ok)
                .collect(),
            None => stmt
                .query_map(params![repo_id], extract_payload_json)?
                .filter_map(Result::ok)
                .collect(),
        };

        Ok(rows
            .iter()
            .filter_map(|s| serde_json::from_str::<Value>(s).ok())
            .collect())
    }

    // ── Overlay by tracker_identifier ──

    pub fn list_overlays_by_tracker(
        &self,
        repo_id: &str,
        tracker_identifier: Option<&str>,
    ) -> Result<Vec<StoredOverlay>> {
        let conn = self.lock_canvas_conn()?;
        let mut stmt = if tracker_identifier.is_some() {
            conn.prepare(
                "SELECT id, repo_id, overlay_type, tracker, tracker_content_id,
                        tracker_project_item_id, tracker_identifier, issue_number,
                        branch, pr_url, agent_session_id, author, status,
                        content_json, affected_nodes, affected_edges,
                        affected_node_refs, affected_edge_refs,
                        created_at, updated_at, superseded_by, manifest_id,
                        accepted_at, implemented_at, merged_at, stale_at, abandoned_at
                 FROM active_overlays WHERE repo_id = ?1 AND tracker_identifier = ?2 ORDER BY created_at DESC"
            )?
        } else {
            conn.prepare(
                "SELECT id, repo_id, overlay_type, tracker, tracker_content_id,
                        tracker_project_item_id, tracker_identifier, issue_number,
                        branch, pr_url, agent_session_id, author, status,
                        content_json, affected_nodes, affected_edges,
                        affected_node_refs, affected_edge_refs,
                        created_at, updated_at, superseded_by, manifest_id,
                        accepted_at, implemented_at, merged_at, stale_at, abandoned_at
                 FROM active_overlays WHERE repo_id = ?1 ORDER BY created_at DESC",
            )?
        };

        match tracker_identifier {
            Some(s) => {
                let rows = stmt
                    .query_map(params![repo_id, s], stored_overlay_from_row)?
                    .filter_map(Result::ok)
                    .collect();
                Ok(rows)
            }
            None => {
                let rows = stmt
                    .query_map(params![repo_id], stored_overlay_from_row)?
                    .filter_map(Result::ok)
                    .collect();
                Ok(rows)
            }
        }
    }
}

fn stored_overlay_from_row(row: &rusqlite::Row<'_>) -> Result<StoredOverlay> {
    Ok(StoredOverlay {
        id: row.get::<_, String>(0)?,
        repo_id: row.get::<_, String>(1)?,
        overlay_type: row.get::<_, String>(2)?,
        tracker: row.get::<_, String>(3)?,
        tracker_content_id: row.get::<_, String>(4)?,
        tracker_project_item_id: row.get::<_, Option<String>>(5)?,
        tracker_identifier: row.get::<_, String>(6)?,
        issue_number: row.get::<_, Option<i64>>(7)?,
        branch: row.get::<_, Option<String>>(8)?,
        pr_url: row.get::<_, Option<String>>(9)?,
        agent_session_id: row.get::<_, Option<String>>(10)?,
        author: row.get::<_, String>(11)?,
        status: row.get::<_, String>(12)?,
        content_json: row.get::<_, String>(13)?,
        affected_nodes: row.get::<_, Option<String>>(14)?,
        affected_edges: row.get::<_, Option<String>>(15)?,
        affected_node_refs: row.get::<_, Option<String>>(16)?,
        affected_edge_refs: row.get::<_, Option<String>>(17)?,
        created_at: row.get::<_, String>(18)?,
        updated_at: row.get::<_, String>(19)?,
        superseded_by: row.get::<_, Option<String>>(20)?,
        manifest_id: row.get::<_, Option<String>>(21)?,
        accepted_at: row.get::<_, Option<String>>(22)?,
        implemented_at: row.get::<_, Option<String>>(23)?,
        merged_at: row.get::<_, Option<String>>(24)?,
        stale_at: row.get::<_, Option<String>>(25)?,
        abandoned_at: row.get::<_, Option<String>>(26)?,
    })
}

fn extract_payload_json(row: &rusqlite::Row<'_>) -> Result<String> {
    row.get::<_, String>(0)
}
