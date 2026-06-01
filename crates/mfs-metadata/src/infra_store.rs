//! Infra / Utility-domain `impl MetadataStore` methods.
//!
//! Contains the audit, webhooks, snapshots, relations, tasks,
//! refresh-scope, and task-pipeline methods extracted from the
//! 3292-line store.rs monolith.

use rusqlite::{Result, params};

use crate::store::MetadataStore;
use crate::store_types::{
    AuditEventRecord, AuditRecord, RelationRecord, SnapshotRecord, StoredRelation, StoredSnapshot,
    StoredTask, StoredWebhook, StoredWebhookWithSecret, TaskRecord, WebhookRecord,
    stored_audit_from_row, stored_snapshot_from_row, stored_task_from_row, stored_webhook_from_row,
    stored_webhook_with_secret_from_row,
};

impl MetadataStore {
    // ── Audit ──────────────────────────────────────────────────────

    pub fn append_audit(&self, record: &AuditEventRecord<'_>) -> Result<usize> {
        self.lock_conn()?.execute(
            "INSERT INTO audit_log (
                account_id,
                user_id,
                agent_id,
                projection_view_id,
                event_type,
                subject_uri,
                actor,
                details_json
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                record.account_id,
                record.user_id,
                record.agent_id,
                record.projection_view_id,
                record.event_type,
                record.subject_uri,
                record.actor,
                record.details_json,
            ],
        )
    }

    pub fn list_audit(
        &self,
        account_id: &str,
        user_id: &str,
        limit: usize,
    ) -> Result<Vec<AuditRecord>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT id,
                    account_id,
                    user_id,
                    agent_id,
                    projection_view_id,
                    event_type,
                    subject_uri,
                    actor,
                    details_json,
                    recorded_at
             FROM audit_log
             WHERE account_id = ?1
               AND user_id = ?2
             ORDER BY id DESC
             LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![account_id, user_id, limit as i64], |row| {
            Ok(AuditRecord {
                id: row.get(0)?,
                account_id: row.get(1)?,
                user_id: row.get(2)?,
                agent_id: row.get(3)?,
                projection_view_id: row.get(4)?,
                event_type: row.get(5)?,
                subject_uri: row.get(6)?,
                actor: row.get(7)?,
                details_json: row.get(8)?,
                recorded_at: row.get(9)?,
            })
        })?;
        rows.collect()
    }

    pub fn list_audit_for_subject(
        &self,
        account_id: &str,
        user_id: &str,
        subject_uri: &str,
        limit: usize,
    ) -> Result<Vec<AuditRecord>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT id,
                    account_id,
                    user_id,
                    agent_id,
                    projection_view_id,
                    event_type,
                    subject_uri,
                    actor,
                    details_json,
                    recorded_at
             FROM audit_log
             WHERE account_id = ?1
               AND user_id = ?2
               AND subject_uri = ?3
             ORDER BY recorded_at DESC, id DESC
             LIMIT ?4",
        )?;
        let rows = stmt.query_map(
            params![account_id, user_id, subject_uri, limit as i64],
            stored_audit_from_row,
        )?;
        rows.collect()
    }

    /// Get the recorded_at timestamp of the most recent audit event of a given type.
    pub fn get_latest_audit_by_event_type(&self, event_type: &str) -> Result<Option<String>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT recorded_at FROM audit_log WHERE event_type = ?1 ORDER BY id DESC LIMIT 1",
        )?;
        let mut rows = stmt.query(params![event_type])?;
        match rows.next()? {
            Some(row) => Ok(Some(row.get(0)?)),
            None => Ok(None),
        }
    }

    // ── Webhooks ──────────────────────────────────────────────────────

    pub fn upsert_webhook(&self, record: &WebhookRecord<'_>) -> Result<usize> {
        self.lock_conn()?.execute(
            "INSERT INTO webhooks (
                id,
                account_id,
                user_id,
                agent_id,
                event_type,
                callback_url,
                secret,
                enabled
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(id)
             DO UPDATE SET
                account_id = excluded.account_id,
                user_id = excluded.user_id,
                agent_id = excluded.agent_id,
                event_type = excluded.event_type,
                callback_url = excluded.callback_url,
                secret = excluded.secret,
                enabled = excluded.enabled",
            params![
                record.id,
                record.account_id,
                record.user_id,
                record.agent_id,
                record.event_type,
                record.callback_url,
                record.secret,
                if record.enabled { 1_i64 } else { 0_i64 },
            ],
        )
    }

    pub fn list_webhooks(
        &self,
        account_id: &str,
        user_id: &str,
        limit: usize,
    ) -> Result<Vec<StoredWebhook>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT id,
                    account_id,
                    user_id,
                    agent_id,
                    event_type,
                    callback_url,
                    enabled,
                    created_at
             FROM webhooks
             WHERE account_id = ?1
               AND user_id = ?2
             ORDER BY created_at DESC, id DESC
             LIMIT ?3",
        )?;
        let rows = stmt.query_map(
            params![account_id, user_id, limit as i64],
            stored_webhook_from_row,
        )?;
        rows.collect()
    }

    pub fn delete_webhook(&self, account_id: &str, user_id: &str, id: &str) -> Result<usize> {
        self.lock_conn()?.execute(
            "DELETE FROM webhooks
             WHERE account_id = ?1
               AND user_id = ?2
               AND id = ?3",
            params![account_id, user_id, id],
        )
    }

    pub fn get_webhook_with_secret(
        &self,
        account_id: &str,
        user_id: &str,
        id: &str,
    ) -> Result<Option<StoredWebhookWithSecret>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT id,
                    account_id,
                    user_id,
                    agent_id,
                    event_type,
                    callback_url,
                    secret,
                    enabled,
                    created_at
             FROM webhooks
             WHERE account_id = ?1
               AND user_id = ?2
               AND id = ?3
             LIMIT 1",
        )?;
        let mut rows = stmt.query(params![account_id, user_id, id])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        Ok(Some(stored_webhook_with_secret_from_row(row)?))
    }

    pub fn list_enabled_webhooks_for_event(
        &self,
        account_id: &str,
        user_id: &str,
        event_type: &str,
    ) -> Result<Vec<StoredWebhookWithSecret>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT id,
                    account_id,
                    user_id,
                    agent_id,
                    event_type,
                    callback_url,
                    secret,
                    enabled,
                    created_at
             FROM webhooks
             WHERE account_id = ?1
               AND user_id = ?2
               AND event_type = ?3
               AND enabled = 1
             ORDER BY created_at ASC, id ASC",
        )?;
        let rows = stmt.query_map(
            params![account_id, user_id, event_type],
            stored_webhook_with_secret_from_row,
        )?;
        rows.collect()
    }

    // ── Snapshots ──────────────────────────────────────────────────────

    pub fn append_snapshot(&self, record: &SnapshotRecord<'_>) -> Result<usize> {
        self.lock_conn()?.execute(
            "INSERT INTO snapshots (
                snapshot_id,
                account_id,
                user_id,
                agent_id,
                projection_view_id,
                root_uri,
                manifest_digest,
                created_by,
                notes
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            ON CONFLICT(snapshot_id)
            DO UPDATE SET
                account_id = excluded.account_id,
                user_id = excluded.user_id,
                agent_id = excluded.agent_id,
                projection_view_id = excluded.projection_view_id,
                root_uri = excluded.root_uri,
                manifest_digest = excluded.manifest_digest,
                created_by = excluded.created_by,
                notes = excluded.notes,
                created_at = CURRENT_TIMESTAMP",
            params![
                record.snapshot_id,
                record.account_id,
                record.user_id,
                record.agent_id,
                record.projection_view_id,
                record.root_uri,
                record.manifest_digest,
                record.created_by,
                record.notes,
            ],
        )
    }

    pub fn list_snapshots(
        &self,
        account_id: &str,
        user_id: &str,
        projection_view_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<StoredSnapshot>> {
        if let Some(projection_view_id) = projection_view_id {
            let conn = self.lock_conn()?;
            let mut stmt = conn.prepare(
                "SELECT snapshot_id,
                        account_id,
                        user_id,
                        agent_id,
                        projection_view_id,
                        root_uri,
                        manifest_digest,
                        created_by,
                        notes,
                        created_at
                 FROM snapshots
                 WHERE account_id = ?1
                   AND user_id = ?2
                   AND projection_view_id = ?3
                 ORDER BY created_at DESC, snapshot_id DESC
                 LIMIT ?4",
            )?;
            let rows = stmt.query_map(
                params![account_id, user_id, projection_view_id, limit as i64],
                stored_snapshot_from_row,
            )?;
            rows.collect()
        } else {
            let conn = self.lock_conn()?;
            let mut stmt = conn.prepare(
                "SELECT snapshot_id,
                        account_id,
                        user_id,
                        agent_id,
                        projection_view_id,
                        root_uri,
                        manifest_digest,
                        created_by,
                        notes,
                        created_at
                 FROM snapshots
                 WHERE account_id = ?1
                   AND user_id = ?2
                 ORDER BY created_at DESC, snapshot_id DESC
                 LIMIT ?3",
            )?;
            let rows = stmt.query_map(
                params![account_id, user_id, limit as i64],
                stored_snapshot_from_row,
            )?;
            rows.collect()
        }
    }

    pub fn count_snapshots(
        &self,
        account_id: &str,
        user_id: &str,
        projection_view_id: Option<&str>,
    ) -> Result<usize> {
        if let Some(projection_view_id) = projection_view_id {
            self.lock_conn()?.query_row(
                "SELECT COUNT(*)
                 FROM snapshots
                 WHERE account_id = ?1
                   AND user_id = ?2
                   AND projection_view_id = ?3",
                params![account_id, user_id, projection_view_id],
                |row| row.get(0),
            )
        } else {
            self.lock_conn()?.query_row(
                "SELECT COUNT(*)
                 FROM snapshots
                 WHERE account_id = ?1
                   AND user_id = ?2",
                params![account_id, user_id],
                |row| row.get(0),
            )
        }
    }

    // ── Relations ──────────────────────────────────────────────────────

    pub fn upsert_relation(&self, record: &RelationRecord<'_>) -> Result<usize> {
        let conn = self.lock_conn()?;
        let tx = conn.unchecked_transaction()?;
        let now_ts = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

        // Insert the new version of this edge with temporal fields.
        tx.execute(
            "INSERT INTO relations (
                account_id,
                user_id,
                agent_id,
                from_uri,
                to_uri,
                relation_type,
                valid_from,
                valid_to,
                tcommit,
                is_latest,
                superseded_by
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, ?7, 1, NULL)",
            params![
                record.account_id,
                record.user_id,
                record.agent_id,
                record.from_uri,
                record.to_uri,
                record.relation_type,
                now_ts,
            ],
        )?;
        let new_id = conn.last_insert_rowid();

        // Supersede any existing is_latest=1 rows with the same edge key:
        // mark them is_latest=0, valid_to=now, superseded_by=new_id.
        // Uses i64 for id comparison (INTEGER column) instead of String
        // to avoid SQLite affinity casting issues.
        tx.execute(
            "UPDATE relations SET
                is_latest = 0,
                valid_to = ?1,
                superseded_by = ?2
             WHERE account_id = ?3
               AND user_id = ?4
               AND from_uri = ?5
               AND to_uri = ?6
               AND relation_type = ?7
               AND is_latest = 1
               AND id != ?2",
            params![
                now_ts,
                new_id, // i64 — matches INTEGER id column directly
                record.account_id,
                record.user_id,
                record.from_uri,
                record.to_uri,
                record.relation_type,
            ],
        )?;

        tx.commit()?;
        Ok(1)
    }

    pub fn remove_relation(
        &self,
        account_id: &str,
        user_id: &str,
        from_uri: &str,
        to_uri: &str,
        relation_type: &str,
    ) -> Result<usize> {
        // Close the current version of this edge: set is_latest=0, valid_to=now,
        // and superseded_by='unlink' sentinel so temporal history distinguishes
        // unlink vs supersession. Historical versions remain for AS OF queries.
        // Returns the number of rows that were closed (should be 0 or 1).
        let now_ts = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        self.lock_conn()?.execute(
            "UPDATE relations SET
                is_latest = 0,
                valid_to = ?1,
                superseded_by = 'unlink'
             WHERE account_id = ?2
               AND user_id = ?3
               AND from_uri = ?4
               AND to_uri = ?5
               AND relation_type = ?6
               AND is_latest = 1",
            params![now_ts, account_id, user_id, from_uri, to_uri, relation_type],
        )
    }

    pub fn list_relations(
        &self,
        account_id: &str,
        user_id: &str,
        uri: &str,
        limit: usize,
    ) -> Result<Vec<StoredRelation>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT id,
                    account_id,
                    user_id,
                    agent_id,
                    from_uri,
                    to_uri,
                    relation_type,
                    updated_at,
                    valid_from,
                    valid_to,
                    tcommit,
                    is_latest,
                    superseded_by
             FROM relations
             WHERE account_id = ?1
               AND user_id = ?2
               AND (from_uri = ?3 OR to_uri = ?3)
               AND is_latest = 1
             ORDER BY updated_at DESC, id DESC
             LIMIT ?4",
        )?;
        let rows = stmt.query_map(params![account_id, user_id, uri, limit as i64], |row| {
            Ok(StoredRelation {
                id: row.get(0)?,
                account_id: row.get(1)?,
                user_id: row.get(2)?,
                agent_id: row.get(3)?,
                from_uri: row.get(4)?,
                to_uri: row.get(5)?,
                relation_type: row.get(6)?,
                updated_at: row.get(7)?,
                valid_from: row.get(8)?,
                valid_to: row.get(9)?,
                tcommit: row.get(10)?,
                is_latest: row.get::<_, i64>(11)?,
                superseded_by: row.get(12)?,
            })
        })?;
        rows.collect()
    }

    /// Get relations with AS OF temporal filtering.
    ///
    /// When `as_of` is provided, returns relations that were valid at that point in time:
    ///   - tcommit <= as_of (not committed after query point)
    ///   - valid_from <= as_of (real-world start before query point)
    ///   - valid_to IS NULL OR valid_to >= as_of (still valid or ended after query point)
    ///   - deduplicate by (from_uri, to_uri, relation_type), keeping latest version
    ///
    /// When `as_of` is None, returns currently valid relations (is_latest = 1).
    /// Optional filters (relation_type, from_uri_prefix) are applied in Rust
    /// to keep the SQL simple and avoid dynamic param construction.
    pub fn get_temporal_relations(
        &self,
        account_id: &str,
        user_id: &str,
        as_of: Option<&str>,
        relation_type: Option<&str>,
        from_uri_prefix: Option<&str>,
    ) -> Result<Vec<StoredRelation>> {
        let conn = self.lock_conn()?;

        let (sql, params): (String, Vec<Box<dyn rusqlite::types::ToSql>>) =
            if let Some(as_of_ts) = as_of {
                (
                    "SELECT id, account_id, user_id, agent_id, from_uri, to_uri,
                        relation_type, updated_at, valid_from, valid_to,
                        tcommit, is_latest, superseded_by
                 FROM relations
                 WHERE account_id = ?1 AND user_id = ?2
                   AND tcommit <= ?3
                   AND (valid_from IS NULL OR valid_from <= ?3)
                   AND (valid_to IS NULL OR valid_to > ?3)
                 ORDER BY tcommit DESC, id DESC"
                        .to_string(),
                    vec![Box::new(account_id), Box::new(user_id), Box::new(as_of_ts)],
                )
            } else {
                (
                    "SELECT id, account_id, user_id, agent_id, from_uri, to_uri,
                        relation_type, updated_at, valid_from, valid_to,
                        tcommit, is_latest, superseded_by
                 FROM relations
                 WHERE account_id = ?1 AND user_id = ?2 AND is_latest = 1
                 ORDER BY updated_at DESC, id DESC"
                        .to_string(),
                    vec![Box::new(account_id), Box::new(user_id)],
                )
            };

        let mut stmt = conn.prepare(&sql)?;
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();
        let rows = stmt.query_map(param_refs.as_slice(), |row| {
            Ok(StoredRelation {
                id: row.get(0)?,
                account_id: row.get(1)?,
                user_id: row.get(2)?,
                agent_id: row.get(3)?,
                from_uri: row.get(4)?,
                to_uri: row.get(5)?,
                relation_type: row.get(6)?,
                updated_at: row.get(7)?,
                valid_from: row.get(8)?,
                valid_to: row.get(9)?,
                tcommit: row.get(10)?,
                is_latest: row.get::<_, i64>(11)?,
                superseded_by: row.get(12)?,
            })
        })?;
        let mut relations: Vec<StoredRelation> = rows.collect::<Result<Vec<_>, _>>()?;

        // For AS OF queries: deduplicate by edge key, keeping latest tcommit
        if as_of.is_some() {
            let mut seen: std::collections::HashSet<(String, String, String)> =
                std::collections::HashSet::new();
            relations.retain(|r| {
                let key = (
                    r.from_uri.clone(),
                    r.to_uri.clone(),
                    r.relation_type.clone(),
                );
                seen.insert(key)
            });
        }

        // Apply optional filters in Rust
        if let Some(rt) = relation_type {
            relations.retain(|r| r.relation_type == rt);
        }
        if let Some(fp) = from_uri_prefix {
            relations.retain(|r| r.from_uri.starts_with(fp));
        }

        Ok(relations)
    }

    // ── Tasks ──────────────────────────────────────────────────────

    pub fn upsert_task(&self, record: &TaskRecord<'_>) -> Result<usize> {
        self.lock_conn()?.execute(
            "INSERT INTO tasks (
                task_key,
                account_id,
                user_id,
                agent_id,
                projection_view_id,
                state,
                owner_space,
                summary,
                last_error
                ,
                attempt_count,
                max_attempts,
                retry_state,
                processing_mode
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
             ON CONFLICT(task_key)
             DO UPDATE SET
                state = excluded.state,
                owner_space = excluded.owner_space,
                summary = excluded.summary,
                last_error = excluded.last_error,
                attempt_count = excluded.attempt_count,
                max_attempts = excluded.max_attempts,
                retry_state = excluded.retry_state,
                processing_mode = excluded.processing_mode,
                updated_at = CURRENT_TIMESTAMP",
            params![
                record.task_key,
                record.account_id,
                record.user_id,
                record.agent_id,
                record.projection_view_id,
                record.state,
                record.owner_space,
                record.summary,
                record.last_error,
                record.attempt_count,
                record.max_attempts,
                record.retry_state,
                record.processing_mode,
            ],
        )
    }

    pub fn get_task(&self, task_key: &str) -> Result<Option<StoredTask>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT task_key,
                    account_id,
                    user_id,
                    agent_id,
                    projection_view_id,
                    state,
                    owner_space,
                    summary,
                    last_error,
                    attempt_count,
                    max_attempts,
                    retry_state,
                    processing_mode,
                    scope_type,
                    scope_id,
                    range_start_turn_id,
                    range_end_turn_id,
                    dedupe_key,
                    payload_json,
                    lease_owner,
                    lease_expires_at,
                    scheduled_at,
                    finished_at,
                    updated_at
             FROM tasks
             WHERE task_key = ?1
             LIMIT 1",
        )?;
        let mut rows = stmt.query(params![task_key])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        Ok(Some(stored_task_from_row(row)?))
    }

    pub fn list_tasks(
        &self,
        account_id: &str,
        user_id: &str,
        limit: usize,
    ) -> Result<Vec<StoredTask>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT task_key,
                    account_id,
                    user_id,
                    agent_id,
                    projection_view_id,
                    state,
                    owner_space,
                    summary,
                    last_error,
                    attempt_count,
                    max_attempts,
                    retry_state,
                    processing_mode,
                    scope_type,
                    scope_id,
                    range_start_turn_id,
                    range_end_turn_id,
                    dedupe_key,
                    payload_json,
                    lease_owner,
                    lease_expires_at,
                    scheduled_at,
                    finished_at,
                    updated_at
             FROM tasks
             WHERE account_id = ?1
               AND user_id = ?2
             ORDER BY updated_at DESC, task_key DESC
             LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![account_id, user_id, limit as i64], |row| {
            stored_task_from_row(row)
        })?;
        rows.collect()
    }

    pub fn count_tasks(
        &self,
        account_id: &str,
        user_id: &str,
        state: Option<&str>,
    ) -> Result<usize> {
        if let Some(state) = state {
            self.lock_conn()?.query_row(
                "SELECT COUNT(*)
                 FROM tasks
                 WHERE account_id = ?1
                   AND user_id = ?2
                   AND state = ?3",
                params![account_id, user_id, state],
                |row| row.get(0),
            )
        } else {
            self.lock_conn()?.query_row(
                "SELECT COUNT(*)
                 FROM tasks
                 WHERE account_id = ?1
                   AND user_id = ?2",
                params![account_id, user_id],
                |row| row.get(0),
            )
        }
    }

    /// Evict completed tasks older than `completed_ttl_hours` and failed tasks
    /// older than `failed_ttl_hours`.
    ///
    /// This prevents the tasks table from growing unboundedly while preserving
    /// recent task records for observability.
    pub fn evict_expired_tasks(
        &self,
        completed_ttl_hours: u32,
        failed_ttl_hours: u32,
    ) -> Result<usize> {
        // NOTE: The TTL values are u32 integers from trusted internal config, not
        // user input, so embedding them directly in the SQL string is safe here.
        // We cannot use them as bound parameters because SQLite would treat the
        // datetime() expression as a literal string rather than evaluating it.
        let sql = format!(
            "DELETE FROM tasks
             WHERE (state = 'completed' AND updated_at < datetime('now', '-{completed_ttl_hours} hours'))
                OR (state = 'failed' AND updated_at < datetime('now', '-{failed_ttl_hours} hours'))"
        );
        self.lock_conn()?.execute(&sql, [])
    }

    /// Evict the oldest tasks when total count exceeds `max_tasks`,
    /// using FIFO (first-in-first-out) ordering by updated_at.
    ///
    /// Only completed or failed tasks are evicted; running/pending tasks are preserved.
    pub fn evict_oldest_tasks_fifo(&self, max_tasks: usize) -> Result<usize> {
        let conn = self.lock_conn()?;
        let total = conn.query_row(
            "SELECT COUNT(*) FROM tasks",
            [],
            |row: &rusqlite::Row<'_>| row.get::<_, i64>(0),
        )? as usize;

        if total <= max_tasks {
            return Ok(0);
        }

        let excess = total - max_tasks;
        self.lock_conn()?.execute(
            "DELETE FROM tasks
             WHERE id IN (
                 SELECT id FROM tasks
                 WHERE state IN ('completed', 'failed')
                 ORDER BY updated_at ASC
                 LIMIT ?1
             )",
            params![excess as i64],
        )
    }

    // ── Refresh scopes ──────────────────────────────────────────────

    pub fn invalidate_refresh_scope(
        &self,
        projection_view_id: &str,
        canonical_uri: &str,
    ) -> Result<usize> {
        let scope_root = canonical_uri.trim_end_matches('/');
        let descendant_pattern = format!("{scope_root}/%");
        let summary_cache_key = format!("summary:{scope_root}");
        let summary_cache_pattern = format!("{summary_cache_key}/%");

        let updated_entries = self.lock_conn()?.execute(
            "UPDATE path_entries
             SET metadata_digest = NULL,
                 updated_at = CURRENT_TIMESTAMP
             WHERE projection_view_id = ?1
               AND (canonical_uri = ?2 OR canonical_uri LIKE ?3)",
            params![projection_view_id, scope_root, descendant_pattern],
        )?;
        let removed_cache_entries = self.lock_conn()?.execute(
            "DELETE FROM digest_cache
             WHERE projection_view_id = ?1
               AND (
                    cache_key = ?2
                 OR cache_key LIKE ?3
                 OR cache_key = ?4
                 OR cache_key LIKE ?5
               )",
            params![
                projection_view_id,
                scope_root,
                descendant_pattern,
                summary_cache_key,
                summary_cache_pattern
            ],
        )?;

        Ok(updated_entries + removed_cache_entries)
    }

    pub fn clear_refresh_scope(
        &self,
        projection_view_id: &str,
        canonical_uri: &str,
    ) -> Result<usize> {
        let scope_root = canonical_uri.trim_end_matches('/');
        let descendant_pattern = format!("{scope_root}/%");
        let summary_cache_key = format!("summary:{scope_root}");
        let summary_cache_pattern = format!("{summary_cache_key}/%");

        let removed_entries = self.lock_conn()?.execute(
            "DELETE FROM path_entries
             WHERE projection_view_id = ?1
               AND (canonical_uri = ?2 OR canonical_uri LIKE ?3)",
            params![projection_view_id, scope_root, descendant_pattern],
        )?;
        let removed_cache_entries = self.lock_conn()?.execute(
            "DELETE FROM digest_cache
             WHERE projection_view_id = ?1
               AND (
                    cache_key = ?2
                 OR cache_key LIKE ?3
                 OR cache_key = ?4
                 OR cache_key LIKE ?5
               )",
            params![
                projection_view_id,
                scope_root,
                descendant_pattern,
                summary_cache_key,
                summary_cache_pattern
            ],
        )?;

        Ok(removed_entries + removed_cache_entries)
    }

    // ── Task pipeline (memory jobs) ──────────────────────────────────

    pub fn enqueue_memory_job(
        &self,
        task_key: &str,
        account_id: &str,
        user_id: &str,
        agent_id: Option<&str>,
        scope_type: &str,
        scope_id: &str,
        range_start_turn_id: Option<&str>,
        range_end_turn_id: Option<&str>,
        dedupe_key: Option<&str>,
        payload_json: Option<&str>,
    ) -> Result<()> {
        self.lock_conn()?.execute(
            "INSERT INTO tasks (
                task_key, account_id, user_id, agent_id,
                state, scope_type, scope_id,
                range_start_turn_id, range_end_turn_id,
                dedupe_key, payload_json
             ) VALUES (
                ?1, ?2, ?3, ?4,
                'queued', ?5, ?6,
                ?7, ?8,
                ?9, ?10
             )
             ON CONFLICT(dedupe_key) WHERE dedupe_key IS NOT NULL
             DO NOTHING",
            params![
                task_key,
                account_id,
                user_id,
                agent_id,
                scope_type,
                scope_id,
                range_start_turn_id,
                range_end_turn_id,
                dedupe_key,
                payload_json,
            ],
        )?;
        Ok(())
    }

    pub fn lease_task(
        &self,
        task_key: &str,
        lease_owner: &str,
        lease_expires_at: &str,
    ) -> Result<()> {
        self.lock_conn()?.execute(
            "UPDATE tasks
             SET state = 'running',
                 lease_owner = ?2,
                 lease_expires_at = ?3,
                 updated_at = CURRENT_TIMESTAMP
             WHERE task_key = ?1
               AND state = 'queued'",
            params![task_key, lease_owner, lease_expires_at],
        )?;
        Ok(())
    }

    pub fn complete_task(&self, task_key: &str, finished_at: &str) -> Result<()> {
        self.lock_conn()?.execute(
            "UPDATE tasks
             SET state = 'completed',
                 finished_at = ?2,
                 lease_owner = NULL,
                 lease_expires_at = NULL,
                 updated_at = CURRENT_TIMESTAMP
             WHERE task_key = ?1",
            params![task_key, finished_at],
        )?;
        Ok(())
    }

    pub fn fail_task(&self, task_key: &str, error_text: &str) -> Result<()> {
        self.lock_conn()?.execute(
            "UPDATE tasks
             SET state = 'failed',
                 last_error = ?2,
                 attempt_count = attempt_count + 1,
                 lease_owner = NULL,
                 lease_expires_at = NULL,
                 updated_at = CURRENT_TIMESTAMP
             WHERE task_key = ?1",
            params![task_key, error_text],
        )?;
        Ok(())
    }

    pub fn get_tasks_by_scope(
        &self,
        scope_type: &str,
        scope_id: &str,
        state: &str,
    ) -> Result<Vec<StoredTask>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT task_key,
                    account_id,
                    user_id,
                    agent_id,
                    projection_view_id,
                    state,
                    owner_space,
                    summary,
                    last_error,
                    attempt_count,
                    max_attempts,
                    retry_state,
                    processing_mode,
                    scope_type,
                    scope_id,
                    range_start_turn_id,
                    range_end_turn_id,
                    dedupe_key,
                    payload_json,
                    lease_owner,
                    lease_expires_at,
                    scheduled_at,
                    finished_at,
                    updated_at
             FROM tasks
             WHERE scope_type = ?1
               AND scope_id = ?2
               AND state = ?3
             ORDER BY updated_at ASC",
        )?;
        let rows = stmt.query_map(params![scope_type, scope_id, state], |row| {
            stored_task_from_row(row)
        })?;
        rows.collect()
    }
}
