//! Resource-domain `impl MetadataStore` methods.
//!
//! Extracted from the former store.rs monolith as part of P1-3 (splitting
//! store.rs into domain modules). This file owns all resource sub-domain
//! methods: path entries, resource sources, resource aliases, resource
//! watches, and resource change events.

use rusqlite::{Result, params};

use crate::store::MetadataStore;
use crate::store_types::{
    ChangeEventRow, PathEntryRecord, ResourceAliasRecord, ResourceSourceRecord,
    ResourceWatchRecord, StoredPathEntry, StoredResourceAlias, StoredResourceSource,
    StoredResourceWatch, change_event_row_from_row, stored_resource_alias_from_row,
    stored_resource_source_from_row,
};

// ─── Path Entries ──────────────────────────────────────────────────────

impl MetadataStore {
    pub fn upsert_path_entry(&self, record: &PathEntryRecord<'_>) -> Result<usize> {
        self.lock_conn()?.execute(
            "INSERT INTO path_entries (
                account_id,
                user_id,
                agent_id,
                projection_view_id,
                canonical_uri,
                workspace_path,
                entry_kind,
                source_kind,
                source_identifier,
                source_snapshot_id,
                content_kind,
                language,
                relative_resource_path,
                repo_root_uri,
                is_text,
                is_generated,
                content_digest,
                metadata_digest,
                size_bytes
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)
             ON CONFLICT(account_id, user_id, projection_view_id, canonical_uri)
             DO UPDATE SET
                workspace_path = excluded.workspace_path,
                entry_kind = excluded.entry_kind,
                source_kind = excluded.source_kind,
                source_identifier = excluded.source_identifier,
                source_snapshot_id = excluded.source_snapshot_id,
                content_kind = excluded.content_kind,
                language = excluded.language,
                relative_resource_path = excluded.relative_resource_path,
                repo_root_uri = excluded.repo_root_uri,
                is_text = excluded.is_text,
                is_generated = excluded.is_generated,
                content_digest = excluded.content_digest,
                metadata_digest = excluded.metadata_digest,
                size_bytes = excluded.size_bytes,
                updated_at = CURRENT_TIMESTAMP",
            params![
                record.account_id,
                record.user_id,
                record.agent_id,
                record.projection_view_id,
                record.canonical_uri,
                record.workspace_path,
                record.entry_kind,
                record.source_kind,
                record.source_identifier,
                record.source_snapshot_id,
                record.content_kind,
                record.language,
                record.relative_resource_path,
                record.repo_root_uri,
                record.is_text.map(i64::from),
                record.is_generated.map(i64::from),
                record.content_digest,
                record.metadata_digest,
                record.size_bytes.map(|value| value as i64),
            ],
        )
    }

    pub fn get_path_entry(
        &self,
        projection_view_id: &str,
        canonical_uri: &str,
    ) -> Result<Option<StoredPathEntry>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT account_id,
                    user_id,
                    agent_id,
                    projection_view_id,
                    canonical_uri,
                    workspace_path,
                    entry_kind,
                    source_kind,
                    source_identifier,
                    source_snapshot_id,
                    content_kind,
                    language,
                    relative_resource_path,
                    repo_root_uri,
                    is_text,
                    is_generated,
                    content_digest,
                    metadata_digest,
                    size_bytes
             FROM path_entries
             WHERE projection_view_id = ?1
               AND canonical_uri = ?2
             LIMIT 1",
        )?;
        let mut rows = stmt.query(params![projection_view_id, canonical_uri])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };

        Ok(Some(StoredPathEntry {
            account_id: row.get(0)?,
            user_id: row.get(1)?,
            agent_id: row.get(2)?,
            projection_view_id: row.get(3)?,
            canonical_uri: row.get(4)?,
            workspace_path: row.get(5)?,
            entry_kind: row.get(6)?,
            source_kind: row.get(7)?,
            source_identifier: row.get(8)?,
            source_snapshot_id: row.get(9)?,
            content_kind: row.get(10)?,
            language: row.get(11)?,
            relative_resource_path: row.get(12)?,
            repo_root_uri: row.get(13)?,
            is_text: row.get::<_, Option<i64>>(14)?.map(|value| value != 0),
            is_generated: row.get::<_, Option<i64>>(15)?.map(|value| value != 0),
            content_digest: row.get(16)?,
            metadata_digest: row.get(17)?,
            size_bytes: row.get::<_, Option<i64>>(18)?.map(|value| value as u64),
        }))
    }

    pub fn count_path_entries(&self) -> Result<usize> {
        self.lock_conn()?
            .query_row("SELECT COUNT(*) FROM path_entries", [], |row| {
                row.get::<_, usize>(0)
            })
    }

    // ── Resource Sources ───────────────────────────────────────────

    pub fn register_resource_source(&self, record: &ResourceSourceRecord<'_>) -> Result<usize> {
        self.lock_conn()?.execute(
            "INSERT INTO resource_sources (
                resource_id,
                account_id,
                user_id,
                agent_id,
                logical_name,
                source_kind,
                source_identifier,
                canonical_root_uri,
                projection_view_id,
                resource_kind,
                source_host,
                source_namespace,
                source_repo,
                source_ref,
                canonical_strategy_version,
                status,
                last_snapshot_id
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
             ON CONFLICT(resource_id)
             DO UPDATE SET
                logical_name = excluded.logical_name,
                source_kind = excluded.source_kind,
                source_identifier = excluded.source_identifier,
                canonical_root_uri = excluded.canonical_root_uri,
                projection_view_id = excluded.projection_view_id,
                resource_kind = excluded.resource_kind,
                source_host = excluded.source_host,
                source_namespace = excluded.source_namespace,
                source_repo = excluded.source_repo,
                source_ref = excluded.source_ref,
                canonical_strategy_version = excluded.canonical_strategy_version,
                status = excluded.status,
                last_snapshot_id = excluded.last_snapshot_id,
                updated_at = CURRENT_TIMESTAMP",
            params![
                record.resource_id,
                record.account_id,
                record.user_id,
                record.agent_id,
                record.logical_name,
                record.source_kind,
                record.source_identifier,
                record.canonical_root_uri,
                record.projection_view_id,
                record.resource_kind,
                record.source_host,
                record.source_namespace,
                record.source_repo,
                record.source_ref,
                record.canonical_strategy_version,
                record.status,
                record.last_snapshot_id,
            ],
        )
    }

    pub fn get_resource_source(&self, resource_id: &str) -> Result<Option<StoredResourceSource>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT resource_id,
                    account_id,
                    user_id,
                    agent_id,
                    logical_name,
                    source_kind,
                    source_identifier,
                    canonical_root_uri,
                    projection_view_id,
                    resource_kind,
                    source_host,
                    source_namespace,
                    source_repo,
                    source_ref,
                    repo_id,
                    tracker,
                    tracker_project_identifier,
                    canonical_strategy_version,
                    status,
                    last_snapshot_id,
                    updated_at
             FROM resource_sources
             WHERE resource_id = ?1
             LIMIT 1",
        )?;
        let mut rows = stmt.query(params![resource_id])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        Ok(Some(stored_resource_source_from_row(row)?))
    }

    pub fn get_resource_source_by_root_uri(
        &self,
        account_id: &str,
        user_id: &str,
        canonical_root_uri: &str,
    ) -> Result<Option<StoredResourceSource>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT resource_id,
                    account_id,
                    user_id,
                    agent_id,
                    logical_name,
                    source_kind,
                    source_identifier,
                    canonical_root_uri,
                    projection_view_id,
                    resource_kind,
                    source_host,
                    source_namespace,
                    source_repo,
                    source_ref,
                    repo_id,
                    tracker,
                    tracker_project_identifier,
                    canonical_strategy_version,
                    status,
                    last_snapshot_id,
                    updated_at
             FROM resource_sources
             WHERE account_id = ?1
               AND user_id = ?2
               AND canonical_root_uri = ?3
             LIMIT 1",
        )?;
        let mut rows = stmt.query(params![account_id, user_id, canonical_root_uri])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        Ok(Some(stored_resource_source_from_row(row)?))
    }

    pub fn list_resource_sources(
        &self,
        account_id: &str,
        user_id: &str,
        limit: usize,
        repo_id: Option<&str>,
    ) -> Result<Vec<StoredResourceSource>> {
        let conn = self.lock_conn()?;
        if let Some(repo_id) = repo_id {
            let mut stmt = conn.prepare(
                "SELECT resource_id,
                    account_id,
                    user_id,
                    agent_id,
                    logical_name,
                    source_kind,
                    source_identifier,
                    canonical_root_uri,
                    projection_view_id,
                    resource_kind,
                    source_host,
                    source_namespace,
                    source_repo,
                    source_ref,
                    repo_id,
                    tracker,
                    tracker_project_identifier,
                    canonical_strategy_version,
                    status,
                    last_snapshot_id,
                    updated_at
             FROM resource_sources
             WHERE account_id = ?1
               AND user_id = ?2
               AND repo_id = ?3
             ORDER BY updated_at DESC, resource_id DESC
             LIMIT ?4",
            )?;
            let rows = stmt
                .query_map(params![account_id, user_id, repo_id, limit as i64], |row| {
                    stored_resource_source_from_row(row)
                })?;
            rows.collect()
        } else {
            let mut stmt = conn.prepare(
                "SELECT resource_id,
                    account_id,
                    user_id,
                    agent_id,
                    logical_name,
                    source_kind,
                    source_identifier,
                    canonical_root_uri,
                    projection_view_id,
                    resource_kind,
                    source_host,
                    source_namespace,
                    source_repo,
                    source_ref,
                    repo_id,
                    tracker,
                    tracker_project_identifier,
                    canonical_strategy_version,
                    status,
                    last_snapshot_id,
                    updated_at
             FROM resource_sources
             WHERE account_id = ?1
               AND user_id = ?2
             ORDER BY updated_at DESC, resource_id DESC
             LIMIT ?3",
            )?;
            let rows = stmt.query_map(params![account_id, user_id, limit as i64], |row| {
                stored_resource_source_from_row(row)
            })?;
            rows.collect()
        }
    }

    pub fn update_resource_business_metadata(
        &self,
        resource_id: &str,
        repo_id: Option<&str>,
        tracker: Option<&str>,
        tracker_project_identifier: Option<&str>,
    ) -> Result<usize> {
        self.lock_conn()?.execute(
            "UPDATE resource_sources
             SET repo_id = ?2,
                 tracker = ?3,
                 tracker_project_identifier = ?4,
                 updated_at = CURRENT_TIMESTAMP
             WHERE resource_id = ?1",
            params![resource_id, repo_id, tracker, tracker_project_identifier],
        )
    }

    pub fn count_resource_sources(
        &self,
        account_id: &str,
        user_id: &str,
        status: Option<&str>,
    ) -> Result<usize> {
        if let Some(status) = status {
            self.lock_conn()?.query_row(
                "SELECT COUNT(*)
                 FROM resource_sources
                 WHERE account_id = ?1
                   AND user_id = ?2
                   AND status = ?3",
                params![account_id, user_id, status],
                |row| row.get(0),
            )
        } else {
            self.lock_conn()?.query_row(
                "SELECT COUNT(*)
                 FROM resource_sources
                 WHERE account_id = ?1
                   AND user_id = ?2",
                params![account_id, user_id],
                |row| row.get(0),
            )
        }
    }

    // ── Resource Aliases ───────────────────────────────────────────

    pub fn upsert_resource_alias(&self, record: &ResourceAliasRecord<'_>) -> Result<usize> {
        self.lock_conn()?.execute(
            "INSERT INTO resource_aliases (
                alias_uri,
                resource_id,
                canonical_root_uri
             ) VALUES (?1, ?2, ?3)
             ON CONFLICT(alias_uri)
             DO UPDATE SET
                resource_id = excluded.resource_id,
                canonical_root_uri = excluded.canonical_root_uri",
            params![
                record.alias_uri,
                record.resource_id,
                record.canonical_root_uri,
            ],
        )
    }

    pub fn get_resource_alias(&self, alias_uri: &str) -> Result<Option<StoredResourceAlias>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT alias_uri,
                    resource_id,
                    canonical_root_uri,
                    created_at
             FROM resource_aliases
             WHERE alias_uri = ?1
             LIMIT 1",
        )?;
        let mut rows = stmt.query(params![alias_uri])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        Ok(Some(stored_resource_alias_from_row(row)?))
    }

    pub fn list_resource_aliases(&self, resource_id: &str) -> Result<Vec<StoredResourceAlias>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT alias_uri,
                    resource_id,
                    canonical_root_uri,
                    created_at
             FROM resource_aliases
             WHERE resource_id = ?1
             ORDER BY alias_uri",
        )?;
        let rows = stmt.query_map(params![resource_id], stored_resource_alias_from_row)?;
        rows.collect()
    }

    // ── Resource Watches ───────────────────────────────────────────

    pub fn upsert_resource_watch(&self, record: &ResourceWatchRecord<'_>) -> Result<usize> {
        self.lock_conn()?.execute(
            "INSERT INTO resource_watches (
                account_id,
                user_id,
                agent_id,
                resource_id,
                interval_seconds,
                enabled
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(resource_id)
             DO UPDATE SET
                interval_seconds = excluded.interval_seconds,
                enabled = excluded.enabled,
                updated_at = CURRENT_TIMESTAMP",
            params![
                record.account_id,
                record.user_id,
                record.agent_id,
                record.resource_id,
                i64::from(record.interval_seconds),
                if record.enabled { 1 } else { 0 },
            ],
        )
    }

    pub fn list_resource_watches(
        &self,
        account_id: &str,
        user_id: &str,
        limit: usize,
    ) -> Result<Vec<StoredResourceWatch>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT account_id,
                    user_id,
                    agent_id,
                    resource_id,
                    interval_seconds,
                    enabled,
                    last_checked_at,
                    last_refreshed_at,
                    updated_at
             FROM resource_watches
             WHERE account_id = ?1
               AND user_id = ?2
             ORDER BY updated_at DESC, resource_id DESC
             LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![account_id, user_id, limit as i64], |row| {
            Ok(StoredResourceWatch {
                account_id: row.get(0)?,
                user_id: row.get(1)?,
                agent_id: row.get(2)?,
                resource_id: row.get(3)?,
                interval_seconds: row.get::<_, i64>(4)? as u32,
                enabled: row.get::<_, i64>(5)? != 0,
                last_checked_at: row.get(6)?,
                last_refreshed_at: row.get(7)?,
                updated_at: row.get(8)?,
            })
        })?;
        rows.collect()
    }

    pub fn mark_resource_watch_run(&self, resource_id: &str, refreshed: bool) -> Result<usize> {
        if refreshed {
            self.lock_conn()?.execute(
                "UPDATE resource_watches
                 SET last_checked_at = CURRENT_TIMESTAMP,
                     last_refreshed_at = CURRENT_TIMESTAMP,
                     updated_at = CURRENT_TIMESTAMP
                 WHERE resource_id = ?1",
                params![resource_id],
            )
        } else {
            self.lock_conn()?.execute(
                "UPDATE resource_watches
                 SET last_checked_at = CURRENT_TIMESTAMP,
                     updated_at = CURRENT_TIMESTAMP
                 WHERE resource_id = ?1",
                params![resource_id],
            )
        }
    }

    pub fn set_resource_watch_enabled(&self, resource_id: &str, enabled: bool) -> Result<usize> {
        self.lock_conn()?.execute(
            "UPDATE resource_watches
             SET enabled = ?2,
                 updated_at = CURRENT_TIMESTAMP
             WHERE resource_id = ?1",
            params![resource_id, if enabled { 1 } else { 0 }],
        )
    }

    pub fn list_due_resource_watches(
        &self,
        account_id: &str,
        user_id: &str,
        limit: usize,
    ) -> Result<Vec<StoredResourceWatch>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT account_id,
                    user_id,
                    agent_id,
                    resource_id,
                    interval_seconds,
                    enabled,
                    last_checked_at,
                    last_refreshed_at,
                    updated_at
             FROM resource_watches
             WHERE account_id = ?1
               AND user_id = ?2
               AND enabled = 1
               AND (
                    last_checked_at IS NULL
                    OR (strftime('%s','now') - strftime('%s', last_checked_at)) >= interval_seconds
               )
             ORDER BY updated_at DESC, resource_id DESC
             LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![account_id, user_id, limit as i64], |row| {
            Ok(StoredResourceWatch {
                account_id: row.get(0)?,
                user_id: row.get(1)?,
                agent_id: row.get(2)?,
                resource_id: row.get(3)?,
                interval_seconds: row.get::<_, i64>(4)? as u32,
                enabled: row.get::<_, i64>(5)? != 0,
                last_checked_at: row.get(6)?,
                last_refreshed_at: row.get(7)?,
                updated_at: row.get(8)?,
            })
        })?;
        rows.collect()
    }

    // ── Resource Change Events ─────────────────────────────────────

    pub fn insert_change_event(
        &self,
        event_id: &str,
        resource_id: &str,
        account_id: &str,
        user_id: &str,
        uri: &str,
        change_type: &str,
        content_digest: Option<&str>,
        snapshot_id: Option<&str>,
    ) -> Result<()> {
        self.lock_conn()?.execute(
            "INSERT INTO resource_change_events (
                event_id, resource_id, account_id, user_id,
                uri, change_type, content_digest, snapshot_id
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                event_id,
                resource_id,
                account_id,
                user_id,
                uri,
                change_type,
                content_digest,
                snapshot_id,
            ],
        )?;
        Ok(())
    }

    pub fn list_change_events_by_resource(
        &self,
        resource_id: &str,
        limit: usize,
    ) -> Result<Vec<ChangeEventRow>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT event_id, resource_id, account_id, user_id,
                    uri, change_type, content_digest, snapshot_id,
                    processed_at, created_at
             FROM resource_change_events
             WHERE resource_id = ?1
             ORDER BY created_at DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(
            params![resource_id, limit as i64],
            change_event_row_from_row,
        )?;
        rows.collect()
    }

    pub fn mark_change_event_processed(&self, event_id: &str, processed_at: &str) -> Result<()> {
        self.lock_conn()?.execute(
            "UPDATE resource_change_events
             SET processed_at = ?2
             WHERE event_id = ?1",
            params![event_id, processed_at],
        )?;
        Ok(())
    }
}
