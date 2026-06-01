use std::path::Path;
use std::sync::Mutex;

use rusqlite::{Connection, Result, params};
use serde::{Deserialize, Serialize};

use crate::canvas_store::CanvasStoreError;
use crate::schema;
use crate::store_types::{
    AssertionRow, BriefRow, CursorRow, EpisodeRow, StoredHeuristicEvidence,
    StoredHeuristicInstance, StoredHeuristicRule,
};

pub struct MetadataStore {
    conn: Mutex<Connection>,
    /// Independent Canvas DB connection (runtime-configurable).
    /// When `separate_canvas_db` is true, canvas tables live in a separate
    /// canvas.sqlite; canvas methods route to this connection via CanvasStore impl.
    /// When false (default), this is None and canvas methods use `conn`.
    canvas_conn: Option<Mutex<Connection>>,
}

impl MetadataStore {
    /// Acquire the inner Mutex lock, converting poison errors into a rusqlite error
    /// instead of panicking. This prevents cascading crashes when a prior holder panicked.
    pub(crate) fn lock_conn(&self) -> Result<std::sync::MutexGuard<'_, Connection>> {
        self.conn.lock().map_err(|e| {
            rusqlite::Error::InvalidPath(std::path::PathBuf::from(format!(
                "metadata store mutex poisoned: {}",
                e
            )))
        })
    }

    /// Lock the canvas connection. When `canvas_conn` is None (shared DB mode),
    /// returns the same connection as `lock_conn()` (canvas tables are in
    /// metadata.sqlite). When `canvas_conn` is Some (separate DB mode),
    /// returns the independent canvas.sqlite connection.
    pub(crate) fn lock_canvas_conn(&self) -> Result<std::sync::MutexGuard<'_, Connection>> {
        match &self.canvas_conn {
            None => self.lock_conn(),
            Some(canvas_conn) => canvas_conn.lock().map_err(|e| {
                rusqlite::Error::InvalidPath(std::path::PathBuf::from(format!(
                    "canvas store mutex poisoned: {}",
                    e
                )))
            }),
        }
    }
}

impl MetadataStore {
    pub fn open_in_memory(separate_canvas_db: bool) -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        schema::bootstrap(&conn, separate_canvas_db)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")?;

        if separate_canvas_db {
            let canvas_conn = Connection::open_in_memory()?;
            schema::bootstrap_canvas(&canvas_conn)?;
            canvas_conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")?;
            Ok(Self {
                conn: Mutex::new(conn),
                canvas_conn: Some(Mutex::new(canvas_conn)),
            })
        } else {
            Ok(Self {
                conn: Mutex::new(conn),
                canvas_conn: None,
            })
        }
    }

    pub fn open_at(path: impl AsRef<Path>, separate_canvas_db: bool) -> Result<Self> {
        let path_ref = path.as_ref();
        // Create parent directory for both metadata and canvas DBs
        if let Some(parent) = path_ref.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|source| rusqlite::Error::ToSqlConversionFailure(source.into()))?;
        }

        // Derive canvas DB path before moving `path` into Connection::open.
        let canvas_path = if separate_canvas_db {
            Some(path_ref.with_file_name("canvas.sqlite"))
        } else {
            None
        };

        let conn = Connection::open(path)?;
        schema::bootstrap(&conn, separate_canvas_db)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")?;

        if separate_canvas_db {
            let canvas_conn = Connection::open(
                canvas_path
                    .as_ref()
                    .expect("canvas_path set when separate_canvas_db=true"),
            )?;
            schema::bootstrap_canvas(&canvas_conn)?;
            canvas_conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")?;
            Ok(Self {
                conn: Mutex::new(conn),
                canvas_conn: Some(Mutex::new(canvas_conn)),
            })
        } else {
            Ok(Self {
                conn: Mutex::new(conn),
                canvas_conn: None,
            })
        }
    }

    pub fn list_tables(&self) -> Result<Vec<String>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT name
             FROM sqlite_master
             WHERE type = 'table'
               AND name NOT LIKE 'sqlite_%'
             ORDER BY name",
        )?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect()
    }

    pub fn list_columns(&self, table: &str) -> Result<Vec<String>> {
        let sql = format!("PRAGMA table_info({table})");
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
        rows.collect()
    }
}

// ─── New table Row structs ─────────────────────────────────────────────
// (SessionRow, TurnRow, and their from_row helpers moved to store_types.rs)

// ─── Heuristic Rules ──────────────────────────────────────────────────

pub(crate) fn stored_heuristic_rule_from_row(
    row: &rusqlite::Row<'_>,
) -> Result<StoredHeuristicRule> {
    Ok(StoredHeuristicRule {
        rule_id: row.get(0)?,
        account_id: row.get(1)?,
        user_id: row.get(2)?,
        agent_id: row.get(3)?,
        tags_json: row.get(4)?,
        rule_text: row.get(5)?,
        counter_examples_json: row.get(6)?,
        lifecycle_stage: row.get(7)?,
        evidence_count: row.get(8)?,
        aggregate_weight: row.get(9)?,
        last_evidence_at: row.get(10)?,
        source_instance_ids_json: row.get(11)?,
        created_at: row.get(12)?,
        promoted_at: row.get(13)?,
        archived_at: row.get(14)?,
        user_confirmed: row.get::<_, i64>(15)? != 0,
    })
}

// ─── Heuristic Instances ──────────────────────────────────────────────

pub(crate) fn stored_heuristic_instance_from_row(
    row: &rusqlite::Row<'_>,
) -> Result<StoredHeuristicInstance> {
    Ok(StoredHeuristicInstance {
        instance_id: row.get(0)?,
        account_id: row.get(1)?,
        user_id: row.get(2)?,
        agent_id: row.get(3)?,
        context_summary: row.get(4)?,
        agent_proposal: row.get(5)?,
        user_reaction: row.get(6)?,
        outcome: row.get(7)?,
        signal_type: row.get(8)?,
        tags_json: row.get(9)?,
        session_id: row.get(10)?,
        source_turn_ids_json: row.get(11)?,
        derived_rule_id: row.get(12)?,
        instance_status: row.get(13)?,
        created_at: row.get(14)?,
        resolved_at: row.get(15)?,
    })
}

// ─── Heuristic Evidence ──────────────────────────────────────────────

pub(crate) fn stored_heuristic_evidence_from_row(
    row: &rusqlite::Row<'_>,
) -> Result<StoredHeuristicEvidence> {
    Ok(StoredHeuristicEvidence {
        evidence_id: row.get(0)?,
        rule_id: row.get(1)?,
        instance_id: row.get(2)?,
        evidence_type: row.get(3)?,
        support_weight: row.get(4)?,
        session_id: row.get(5)?,
        created_at: row.get(6)?,
    })
}

pub(crate) fn episode_row_from_row(row: &rusqlite::Row<'_>) -> Result<EpisodeRow> {
    Ok(EpisodeRow {
        episode_id: row.get(0)?,
        account_id: row.get(1)?,
        user_id: row.get(2)?,
        agent_id: row.get(3)?,
        session_id: row.get(4)?,
        resource_id: row.get(5)?,
        summary: row.get(6)?,
        detail_ref: row.get(7)?,
        keywords_json: row.get(8)?,
        salience_score: row.get(9)?,
        strength_score: row.get(10)?,
        emotional_valence: row.get(11)?,
        emotional_intensity: row.get(12)?,
        context_tags_json: row.get(13)?,
        recall_count: row.get(14)?,
        last_recalled_at: row.get(15)?,
        source_start_turn_id: row.get(16)?,
        source_end_turn_id: row.get(17)?,
        created_at: row.get(18)?,
        archived_at: row.get(19)?,
        last_decay_at: row.get(20)?,
        embedding_json: row.get(21)?,
    })
}

// fact_assertions
pub(crate) fn assertion_row_from_row(row: &rusqlite::Row<'_>) -> Result<AssertionRow> {
    Ok(AssertionRow {
        assertion_id: row.get(0)?,
        account_id: row.get(1)?,
        user_id: row.get(2)?,
        agent_id: row.get(3)?,
        subject: row.get(4)?,
        predicate: row.get(5)?,
        raw_value_text: row.get(6)?,
        normalized_value_json: row.get(7)?,
        value_type: row.get(8)?,
        operation: row.get(9)?,
        confidence: row.get(10)?,
        valid_from: row.get(11)?,
        valid_to: row.get(12)?,
        source_turn_id: row.get(13)?,
        source_episode_ids_json: row.get(14)?,
        source_resource_id: row.get(15)?,
        source_snapshot_id: row.get(16)?,
        source_uri: row.get(17)?,
        extractor_version: row.get(18)?,
        created_at: row.get(19)?,
    })
}

// memory_consolidation_cursors
pub(crate) fn cursor_row_from_row(row: &rusqlite::Row<'_>) -> Result<CursorRow> {
    Ok(CursorRow {
        cursor_id: row.get(0)?,
        account_id: row.get(1)?,
        user_id: row.get(2)?,
        scope_type: row.get(3)?,
        scope_id: row.get(4)?,
        last_consolidated_turn_id: row.get(5)?,
        last_consolidated_at: row.get(6)?,
        dedupe_key: row.get(7)?,
        lease_owner: row.get(8)?,
        lease_expires_at: row.get(9)?,
        updated_at: row.get(10)?,
    })
}

// memory_briefs
pub(crate) fn brief_row_from_row(row: &rusqlite::Row<'_>) -> Result<BriefRow> {
    Ok(BriefRow {
        brief_id: row.get(0)?,
        account_id: row.get(1)?,
        user_id: row.get(2)?,
        scope_type: row.get(3)?,
        scope_id: row.get(4)?,
        summary: row.get(5)?,
        source_thread_ids_json: row.get(6)?,
        anchor_episode_ids_json: row.get(7)?,
        updated_at: row.get(8)?,
    })
}

#[cfg(test)]
mod tests {
    use super::{CanvasSnapshotRecord, ManifestIdentityRecord, MetadataStore};
    use crate::store_types::RelationRecord;
    use rusqlite::params;

    #[test]
    fn invalidates_metadata_digests_and_summary_cache_for_refresh_scope() {
        let store = MetadataStore::open_in_memory(false).unwrap();
        insert_path_entry(
            &store,
            "tenant:acme:alice:resources",
            "mfs://resources/localfs/docs",
            Some("root-metadata-digest"),
        );
        insert_path_entry(
            &store,
            "tenant:acme:alice:resources",
            "mfs://resources/localfs/docs/api.md",
            Some("child-metadata-digest"),
        );
        insert_path_entry(
            &store,
            "tenant:acme:alice:resources",
            "mfs://resources/elsewhere",
            Some("other-metadata-digest"),
        );
        insert_path_entry(
            &store,
            "tenant:acme:bob:resources",
            "mfs://resources/localfs/docs",
            Some("bob-root-digest"),
        );

        insert_digest_cache(
            &store,
            "tenant:acme:alice:resources",
            "summary:mfs://resources/localfs/docs",
        );
        insert_digest_cache(
            &store,
            "tenant:acme:alice:resources",
            "summary:mfs://resources/localfs/docs/api.md",
        );
        insert_digest_cache(
            &store,
            "tenant:acme:alice:resources",
            "summary:mfs://resources/elsewhere",
        );
        insert_digest_cache(
            &store,
            "tenant:acme:bob:resources",
            "summary:mfs://resources/localfs/docs",
        );

        let affected = store
            .invalidate_refresh_scope(
                "tenant:acme:alice:resources",
                "mfs://resources/localfs/docs",
            )
            .unwrap();

        assert_eq!(affected, 4);
        assert_eq!(
            metadata_digest(
                &store,
                "tenant:acme:alice:resources",
                "mfs://resources/localfs/docs"
            ),
            None
        );
        assert_eq!(
            metadata_digest(
                &store,
                "tenant:acme:alice:resources",
                "mfs://resources/localfs/docs/api.md"
            ),
            None
        );
        assert_eq!(
            metadata_digest(
                &store,
                "tenant:acme:alice:resources",
                "mfs://resources/elsewhere"
            ),
            Some("other-metadata-digest".to_owned())
        );
        assert_eq!(
            metadata_digest(
                &store,
                "tenant:acme:bob:resources",
                "mfs://resources/localfs/docs"
            ),
            Some("bob-root-digest".to_owned())
        );
        assert_eq!(
            digest_cache_count(
                &store,
                "tenant:acme:alice:resources",
                "summary:mfs://resources/localfs/docs"
            ),
            0
        );
        assert_eq!(
            digest_cache_count(
                &store,
                "tenant:acme:alice:resources",
                "summary:mfs://resources/localfs/docs/api.md"
            ),
            0
        );
        assert_eq!(
            digest_cache_count(
                &store,
                "tenant:acme:alice:resources",
                "summary:mfs://resources/elsewhere"
            ),
            1
        );
        assert_eq!(
            digest_cache_count(
                &store,
                "tenant:acme:bob:resources",
                "summary:mfs://resources/localfs/docs"
            ),
            1
        );
    }

    fn insert_path_entry(
        store: &MetadataStore,
        projection_view_id: &str,
        canonical_uri: &str,
        metadata_digest: Option<&str>,
    ) {
        store
            .conn
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO path_entries (
                    account_id,
                    user_id,
                    agent_id,
                    projection_view_id,
                    canonical_uri,
                    workspace_path,
                    entry_kind,
                    metadata_digest,
                    content_digest
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    "acme",
                    "alice",
                    "coding-agent",
                    projection_view_id,
                    canonical_uri,
                    canonical_uri.replace("mfs://", "/workspace/"),
                    "file",
                    metadata_digest,
                    "content-digest"
                ],
            )
            .unwrap();
    }

    fn insert_digest_cache(store: &MetadataStore, projection_view_id: &str, cache_key: &str) {
        store
            .conn
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO digest_cache (
                    cache_key,
                    account_id,
                    user_id,
                    projection_view_id,
                    digest,
                    size_bytes
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    cache_key,
                    "acme",
                    "alice",
                    projection_view_id,
                    "digest",
                    128_i64
                ],
            )
            .unwrap();
    }

    fn metadata_digest(
        store: &MetadataStore,
        projection_view_id: &str,
        canonical_uri: &str,
    ) -> Option<String> {
        store
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT metadata_digest
                 FROM path_entries
                 WHERE projection_view_id = ?1
                   AND canonical_uri = ?2",
                params![projection_view_id, canonical_uri],
                |row| row.get(0),
            )
            .unwrap()
    }

    fn digest_cache_count(
        store: &MetadataStore,
        projection_view_id: &str,
        cache_key: &str,
    ) -> usize {
        store
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*)
                 FROM digest_cache
                 WHERE projection_view_id = ?1
                   AND cache_key = ?2",
                params![projection_view_id, cache_key],
                |row| row.get::<_, usize>(0),
            )
            .unwrap()
    }

    // ─── confirm_heuristic_rule tests ──────────────────────────────────

    fn insert_test_rule(
        store: &MetadataStore,
        rule_id: &str,
        account_id: &str,
        user_id: &str,
        lifecycle_stage: &str,
    ) {
        use crate::store_types::HeuristicRuleRecord;
        let rule = HeuristicRuleRecord {
            rule_id,
            account_id,
            user_id,
            agent_id: Some("test-agent"),
            tags_json: "[\"domain:test\"]",
            rule_text: "Test rule",
            counter_examples_json: "[]",
            lifecycle_stage,
            evidence_count: 1,
            aggregate_weight: 1.0,
            last_evidence_at: Some("2026-01-01T00:00:00Z"),
            source_instance_ids_json: Some("[\"i1\"]"),
            promoted_at: None,
            user_confirmed: false,
        };
        store.insert_heuristic_rule(&rule).unwrap();
    }

    #[test]
    fn confirm_heuristic_rule_scopes_to_account_and_user() {
        let store = MetadataStore::open_in_memory(false).unwrap();
        insert_test_rule(&store, "r1", "acme", "alice", "candidate");
        insert_test_rule(&store, "r2", "acme", "bob", "candidate");
        insert_test_rule(&store, "r3", "other", "alice", "candidate");

        // Only acme/alice's rule should be confirmed
        let ok = store.confirm_heuristic_rule("r1", "acme", "alice");
        assert!(ok, "should confirm own rule");

        // Cross-account: should not confirm other's rule
        let cross_account = store.confirm_heuristic_rule("r3", "acme", "alice");
        assert!(
            !cross_account,
            "should NOT confirm rule from different account"
        );

        // Cross-user: should not confirm another user's rule in same account
        let cross_user = store.confirm_heuristic_rule("r2", "acme", "alice");
        assert!(!cross_user, "should NOT confirm rule from different user");
    }

    #[test]
    fn confirm_heuristic_rule_returns_false_for_nonexistent() {
        let store = MetadataStore::open_in_memory(false).unwrap();
        let ok = store.confirm_heuristic_rule("nonexistent", "acme", "alice");
        assert!(!ok, "should return false for nonexistent rule_id");
    }

    // ─── increment_rule_evidence_stats tests ────────────────────────────

    #[test]
    fn increment_rule_evidence_stats_adds_weight_and_count() {
        let store = MetadataStore::open_in_memory(false).unwrap();
        insert_test_rule(&store, "r1", "acme", "alice", "candidate");

        // Initial state: evidence_count=1, aggregate_weight=1.0
        store
            .increment_rule_evidence_stats("r1", 0.8, "2026-04-30T00:00:00Z")
            .unwrap();

        let rules = store
            .get_active_heuristic_rules("acme", "alice", &["candidate"])
            .unwrap();
        let rule = &rules[0];
        assert_eq!(
            rule.evidence_count, 2,
            "evidence_count should be incremented by 1"
        );
        // aggregate_weight should be 1.0 + 0.8 = 1.8 (preserves prior decay-adjusted value)
        assert!(
            (rule.aggregate_weight - 1.8).abs() < 0.01,
            "aggregate_weight should be 1.8, got {}",
            rule.aggregate_weight
        );
        assert_eq!(
            rule.last_evidence_at,
            Some("2026-04-30T00:00:00Z".to_owned())
        );
    }

    #[test]
    fn increment_rule_evidence_stats_preserves_decayed_weight() {
        let store = MetadataStore::open_in_memory(false).unwrap();
        insert_test_rule(&store, "r1", "acme", "alice", "candidate");

        // Simulate prior decay: manually set aggregate_weight to a decayed value (0.6)
        store
            .conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE heuristic_rules SET aggregate_weight = 0.6 WHERE rule_id = 'r1'",
                params![],
            )
            .unwrap();

        // Incremental update: should ADD 0.5 to the decayed 0.6 = 1.1
        store
            .increment_rule_evidence_stats("r1", 0.5, "2026-04-30T00:00:00Z")
            .unwrap();

        let rules = store
            .get_active_heuristic_rules("acme", "alice", &["candidate"])
            .unwrap();
        let rule = &rules[0];
        // aggregate_weight should be 0.6 + 0.5 = 1.1, NOT 1.0+0.5=1.5 (raw sum would overwrite decay)
        assert!(
            (rule.aggregate_weight - 1.1).abs() < 0.01,
            "incremental update preserves decayed weight: expected 1.1, got {}",
            rule.aggregate_weight
        );
    }

    #[test]
    fn metadata_store_open_in_memory_succeeds_shared_db() {
        // Verify open_in_memory works with shared DB (separate_canvas_db=false)
        let store = MetadataStore::open_in_memory(false).expect("open_in_memory must succeed");
        // Basic sanity: metadata tables should exist
        let tables = store.list_tables().unwrap();
        assert!(
            tables.contains(&"path_entries".to_string()),
            "path_entries must exist in metadata DB"
        );
        // Shared DB mode: canvas tables live in same DB
        assert!(
            tables.contains(&"canvas_nodes".to_string()),
            "canvas_nodes must exist in shared DB mode"
        );
    }

    #[test]
    fn metadata_store_open_in_memory_succeeds_separate_db() {
        // Verify open_in_memory works with separate canvas DB (separate_canvas_db=true)
        let store = MetadataStore::open_in_memory(true).expect("open_in_memory must succeed");
        // Basic sanity: metadata tables should exist
        let tables = store.list_tables().unwrap();
        assert!(
            tables.contains(&"path_entries".to_string()),
            "path_entries must exist in metadata DB"
        );
        // Separate DB mode: canvas tables NOT in metadata DB
        assert!(
            !tables.contains(&"canvas_nodes".to_string()),
            "canvas_nodes must NOT exist in metadata DB when separated"
        );
    }

    #[test]
    fn immutable_canvas_snapshots_reject_update_and_delete() {
        let store = MetadataStore::open_in_memory(false).unwrap();
        store
            .upsert_manifest_identity(&ManifestIdentityRecord {
                repo_id: "symphony-gh",
                resource_uri: "mfs://resources/localfs/symphony-gh/MANIFEST.yaml",
                default_branch: "main",
                primary_languages: r#"["elixir"]"#,
                created_at: "2026-05-11T00:00:00Z",
                last_verified_at: "2026-05-11T00:00:00Z",
                manifest_yaml_path: Some("/workspace/MANIFEST.yaml"),
                repo_name: None,
                repo_path: None,
                last_commit_hash: None,
                last_commit_date: None,
                manifest_version: "1",
                yaml_hash: None,
                source_roots_json: "[]",
                quality_gates_json: "{}",
                updated_at: "2026-05-11T00:00:00Z",
            })
            .unwrap();
        store
            .insert_canvas_snapshot(&CanvasSnapshotRecord {
                id: "snapshot-immutable",
                repo_id: "symphony-gh",
                merge_commit: "abc123",
                snapshot_type: "full",
                snapshot_json: r#"{"nodes":[],"edges":[]}"#,
                created_at: "2026-05-11T00:00:00Z",
                immutable: true,
            })
            .unwrap();

        let update = store.conn.lock().unwrap().execute(
            "UPDATE canvas_snapshots SET snapshot_json = ?1 WHERE id = ?2",
            params![r#"{"nodes":["changed"],"edges":[]}"#, "snapshot-immutable"],
        );
        assert!(
            update.is_err(),
            "immutable snapshot update must be rejected"
        );

        let delete = store.conn.lock().unwrap().execute(
            "DELETE FROM canvas_snapshots WHERE id = ?1",
            params!["snapshot-immutable"],
        );
        assert!(
            delete.is_err(),
            "immutable snapshot delete must be rejected"
        );

        let snapshot = store
            .get_canvas_snapshot("snapshot-immutable")
            .unwrap()
            .expect("snapshot should remain after rejected writes");
        assert_eq!(snapshot.snapshot_json, r#"{"nodes":[],"edges":[]}"#);
        assert!(snapshot.immutable);
    }

    // ─── access_log tests ────────────────────────────────────────────

    #[test]
    fn access_log_append_and_retrieve() {
        let store = MetadataStore::open_in_memory(false).unwrap();
        let now = chrono::Utc::now();

        store
            .append_access_log("ep-1", "episode", "2026-03-10T12:00:00Z", "acme", "alice")
            .unwrap();
        store
            .append_access_log("ep-1", "episode", "2026-03-15T08:00:00Z", "acme", "alice")
            .unwrap();
        store
            .append_access_log("ep-1", "episode", "2026-03-20T16:00:00Z", "acme", "alice")
            .unwrap();

        let days = store.get_access_days_since("ep-1", &now).unwrap();
        assert_eq!(days.len(), 3, "should return 3 access entries");
        // Days should be sorted ascending (oldest first)
        assert!(
            days[0] > days[1],
            "older accesses have larger days-since values"
        );
        assert!(
            days[1] > days[2],
            "ascending order: days[0] > days[1] > days[2]"
        );
    }

    #[test]
    fn access_log_is_scoped_to_memory_id() {
        let store = MetadataStore::open_in_memory(false).unwrap();
        let now = chrono::Utc::now();

        store
            .append_access_log("ep-1", "episode", "2026-03-10T12:00:00Z", "acme", "alice")
            .unwrap();
        store
            .append_access_log("fact-1", "fact", "2026-03-12T12:00:00Z", "acme", "alice")
            .unwrap();

        let days_ep = store.get_access_days_since("ep-1", &now).unwrap();
        let days_fact = store.get_access_days_since("fact-1", &now).unwrap();
        assert_eq!(days_ep.len(), 1, "ep-1 should have 1 entry");
        assert_eq!(days_fact.len(), 1, "fact-1 should have 1 entry");
        assert!(
            days_ep[0] > days_fact[0],
            "ep-1 accessed earlier, more days since"
        );
    }

    #[test]
    fn access_log_batch_retrieves_multiple_ids() {
        let store = MetadataStore::open_in_memory(false).unwrap();
        let now = chrono::Utc::now();

        store
            .append_access_log("ep-1", "episode", "2026-03-10T12:00:00Z", "acme", "alice")
            .unwrap();
        store
            .append_access_log("ep-1", "episode", "2026-03-20T12:00:00Z", "acme", "alice")
            .unwrap();
        store
            .append_access_log("ep-2", "episode", "2026-03-15T12:00:00Z", "acme", "alice")
            .unwrap();

        let batch = store
            .get_access_days_since_batch(&["ep-1".to_string(), "ep-2".to_string()], &now)
            .unwrap();
        assert_eq!(batch.len(), 2, "batch should return 2 keys");
        assert_eq!(batch["ep-1"].len(), 2, "ep-1 has 2 accesses");
        assert_eq!(batch["ep-2"].len(), 1, "ep-2 has 1 access");
    }

    #[test]
    fn access_log_prune_removes_old_entries() {
        let store = MetadataStore::open_in_memory(false).unwrap();
        let now = chrono::Utc::now();
        // Old entry: 200 days ago (will be pruned)
        let old_ts =
            (now - chrono::Duration::days(200)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        // Recent entry: 5 days ago (will survive 90-day cutoff)
        let recent_ts =
            (now - chrono::Duration::days(5)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

        store
            .append_access_log("ep-1", "episode", &old_ts, "acme", "alice")
            .unwrap();
        store
            .append_access_log("ep-1", "episode", &recent_ts, "acme", "alice")
            .unwrap();

        let pruned = store.prune_access_log(90.0).unwrap();
        assert_eq!(pruned, 1, "should prune 1 entry older than 90 days");

        let remaining = store.get_access_days_since("ep-1", &now).unwrap();
        assert_eq!(remaining.len(), 1, "only recent entry should remain");
        assert!(
            remaining[0] >= 4.0 && remaining[0] <= 6.0,
            "recent entry ~5 days since"
        );
    }

    #[test]
    fn access_log_returns_empty_for_unknown_id() {
        let store = MetadataStore::open_in_memory(false).unwrap();
        let days = store
            .get_access_days_since("nonexistent", &chrono::Utc::now())
            .unwrap();
        assert!(days.is_empty(), "unknown id should return empty vec");
    }

    // ─── temporal relations tests ─────────────────────────────────────

    #[test]
    fn temporal_relations_as_of_query_filters_by_validity_window() {
        let store = MetadataStore::open_in_memory(false).unwrap();

        // Insert two versions of the same edge:
        // v1: valid 2026-01-01 to 2026-03-01 (superseded)
        // v2: valid 2026-03-01 onward (current)
        store.conn.lock().unwrap().execute_batch(
            "INSERT INTO relations (id, account_id, user_id, from_uri, to_uri, relation_type, updated_at, valid_from, valid_to, tcommit, is_latest, superseded_by)
             VALUES (100, 'acme', 'alice', 'mfs://sessions/s1', 'mfs://resources/docs', 'accessed', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', '2026-03-01T00:00:00Z', '2026-01-01T00:00:00Z', 0, '200');
             INSERT INTO relations (id, account_id, user_id, from_uri, to_uri, relation_type, updated_at, valid_from, valid_to, tcommit, is_latest, superseded_by)
             VALUES (200, 'acme', 'alice', 'mfs://sessions/s1', 'mfs://resources/docs', 'accessed', '2026-03-01T00:00:00Z', '2026-03-01T00:00:00Z', NULL, '2026-03-01T00:00:00Z', 1, NULL);",
        ).unwrap();

        // AS OF 2026-02-01: only v1 is valid
        let feb = store
            .get_temporal_relations("acme", "alice", Some("2026-02-01T00:00:00Z"), None, None)
            .unwrap();
        assert_eq!(feb.len(), 1, "AS OF Feb should return v1 only");
        assert_eq!(feb[0].id, 100, "should be the old version");

        // AS OF 2026-04-01: only v2 is valid
        let apr = store
            .get_temporal_relations("acme", "alice", Some("2026-04-01T00:00:00Z"), None, None)
            .unwrap();
        assert_eq!(apr.len(), 1, "AS OF Apr should return v2 only");
        assert_eq!(apr[0].id, 200, "should be the new version");

        // No as_of: returns is_latest=1 only (v2)
        let current = store
            .get_temporal_relations("acme", "alice", None, None, None)
            .unwrap();
        assert_eq!(current.len(), 1, "current query returns latest only");
        assert_eq!(current[0].is_latest, 1);
    }

    #[test]
    fn temporal_relations_as_of_deduplicates_by_edge_key() {
        let store = MetadataStore::open_in_memory(false).unwrap();

        // Two edges with same key but overlapping validity (unlikely in practice, but tests dedup)
        store.conn.lock().unwrap().execute_batch(
            "INSERT INTO relations (id, account_id, user_id, from_uri, to_uri, relation_type, updated_at, valid_from, valid_to, tcommit, is_latest, superseded_by)
             VALUES (10, 'acme', 'alice', 'mfs://sessions/s1', 'mfs://resources/docs', 'accessed', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', '2026-06-01T00:00:00Z', '2026-01-01T00:00:00Z', 0, NULL);
             INSERT INTO relations (id, account_id, user_id, from_uri, to_uri, relation_type, updated_at, valid_from, valid_to, tcommit, is_latest, superseded_by)
             VALUES (20, 'acme', 'alice', 'mfs://sessions/s1', 'mfs://resources/docs', 'accessed', '2026-02-01T00:00:00Z', '2026-02-01T00:00:00Z', '2026-06-01T00:00:00Z', '2026-02-01T00:00:00Z', 1, NULL);",
        ).unwrap();

        // AS OF 2026-03-01: both valid, but dedup keeps latest tcommit (id=20)
        let results = store
            .get_temporal_relations("acme", "alice", Some("2026-03-01T00:00:00Z"), None, None)
            .unwrap();
        assert_eq!(results.len(), 1, "dedup should keep only 1 edge per key");
        assert_eq!(results[0].id, 20, "should keep latest tcommit version");
    }

    #[test]
    fn temporal_relations_filters_by_type_and_prefix() {
        let store = MetadataStore::open_in_memory(false).unwrap();

        store.conn.lock().unwrap().execute_batch(
            "INSERT INTO relations (id, account_id, user_id, from_uri, to_uri, relation_type, updated_at, valid_from, valid_to, tcommit, is_latest, superseded_by)
             VALUES (1, 'acme', 'alice', 'mfs://sessions/s1', 'mfs://resources/docs', 'accessed', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', NULL, '2026-01-01T00:00:00Z', 1, NULL);
             INSERT INTO relations (id, account_id, user_id, from_uri, to_uri, relation_type, updated_at, valid_from, valid_to, tcommit, is_latest, superseded_by)
             VALUES (2, 'acme', 'alice', 'mfs://sessions/s1', 'mfs://skills/code', 'invoked', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', NULL, '2026-01-01T00:00:00Z', 1, NULL);
             INSERT INTO relations (id, account_id, user_id, from_uri, to_uri, relation_type, updated_at, valid_from, valid_to, tcommit, is_latest, superseded_by)
             VALUES (3, 'acme', 'alice', 'mfs://canvas/c1', 'mfs://resources/docs', 'accessed', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', NULL, '2026-01-01T00:00:00Z', 1, NULL);",
        ).unwrap();

        let by_type = store
            .get_temporal_relations("acme", "alice", None, Some("invoked"), None)
            .unwrap();
        assert_eq!(by_type.len(), 1, "type filter should match 1");
        assert_eq!(by_type[0].relation_type, "invoked");

        let by_prefix = store
            .get_temporal_relations("acme", "alice", None, None, Some("mfs://sessions/"))
            .unwrap();
        assert_eq!(
            by_prefix.len(),
            2,
            "prefix filter should match 2 sessions edges"
        );
    }

    #[test]
    fn temporal_relations_scoped_to_account_and_user() {
        let store = MetadataStore::open_in_memory(false).unwrap();

        store.conn.lock().unwrap().execute_batch(
            "INSERT INTO relations (id, account_id, user_id, from_uri, to_uri, relation_type, updated_at, valid_from, valid_to, tcommit, is_latest, superseded_by)
             VALUES (1, 'acme', 'alice', 'mfs://sessions/s1', 'mfs://resources/docs', 'accessed', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', NULL, '2026-01-01T00:00:00Z', 1, NULL);
             INSERT INTO relations (id, account_id, user_id, from_uri, to_uri, relation_type, updated_at, valid_from, valid_to, tcommit, is_latest, superseded_by)
             VALUES (2, 'acme', 'bob', 'mfs://sessions/s2', 'mfs://resources/docs', 'accessed', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', NULL, '2026-01-01T00:00:00Z', 1, NULL);
             INSERT INTO relations (id, account_id, user_id, from_uri, to_uri, relation_type, updated_at, valid_from, valid_to, tcommit, is_latest, superseded_by)
             VALUES (3, 'other', 'alice', 'mfs://sessions/s3', 'mfs://resources/docs', 'accessed', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', NULL, '2026-01-01T00:00:00Z', 1, NULL);",
        ).unwrap();

        let alice = store
            .get_temporal_relations("acme", "alice", None, None, None)
            .unwrap();
        assert_eq!(alice.len(), 1, "alice should see only her own edge");
        assert_eq!(alice[0].user_id, "alice");
    }

    #[test]
    fn list_relations_returns_only_latest_edges() {
        let store = MetadataStore::open_in_memory(false).unwrap();

        // v1: superseded (is_latest=0)
        // v2: current (is_latest=1)
        store.conn.lock().unwrap().execute_batch(
            "INSERT INTO relations (id, account_id, user_id, from_uri, to_uri, relation_type, updated_at, valid_from, valid_to, tcommit, is_latest, superseded_by)
             VALUES (10, 'acme', 'alice', 'mfs://sessions/s1', 'mfs://resources/docs', 'accessed', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', '2026-03-01T00:00:00Z', '2026-01-01T00:00:00Z', 0, '20');
             INSERT INTO relations (id, account_id, user_id, from_uri, to_uri, relation_type, updated_at, valid_from, valid_to, tcommit, is_latest, superseded_by)
             VALUES (20, 'acme', 'alice', 'mfs://sessions/s1', 'mfs://resources/docs', 'accessed', '2026-03-01T00:00:00Z', '2026-03-01T00:00:00Z', NULL, '2026-03-01T00:00:00Z', 1, NULL);",
        ).unwrap();

        let relations = store
            .list_relations("acme", "alice", "mfs://sessions/s1", 10)
            .unwrap();
        assert_eq!(
            relations.len(),
            1,
            "list_relations should return only is_latest=1"
        );
        assert_eq!(relations[0].id, 20);
        assert_eq!(relations[0].is_latest, 1);
        assert_eq!(relations[0].superseded_by, None);
    }

    #[test]
    fn upsert_relation_supersedes_existing_edge() {
        let store = MetadataStore::open_in_memory(false).unwrap();

        // Insert first version of the edge
        let rec1 = RelationRecord {
            account_id: "acme",
            user_id: "alice",
            agent_id: Some("agent-v1"),
            from_uri: "mfs://sessions/s1",
            to_uri: "mfs://resources/docs",
            relation_type: "accessed",
        };
        store.upsert_relation(&rec1).unwrap();

        let v1_relations = store
            .list_relations("acme", "alice", "mfs://sessions/s1", 10)
            .unwrap();
        assert_eq!(v1_relations.len(), 1, "first insert: 1 edge");
        let v1_id = v1_relations[0].id;
        assert_eq!(v1_relations[0].is_latest, 1);
        assert_eq!(v1_relations[0].agent_id, Some("agent-v1".to_string()));

        // Insert second version of the same edge (supersedes v1)
        let rec2 = RelationRecord {
            account_id: "acme",
            user_id: "alice",
            agent_id: Some("agent-v2"),
            from_uri: "mfs://sessions/s1",
            to_uri: "mfs://resources/docs",
            relation_type: "accessed",
        };
        store.upsert_relation(&rec2).unwrap();

        // list_relations should return only v2 (is_latest=1)
        let v2_relations = store
            .list_relations("acme", "alice", "mfs://sessions/s1", 10)
            .unwrap();
        assert_eq!(
            v2_relations.len(),
            1,
            "after supersession: only 1 latest edge"
        );
        assert_eq!(v2_relations[0].is_latest, 1);
        assert_eq!(v2_relations[0].agent_id, Some("agent-v2".to_string()));
        assert_ne!(v2_relations[0].id, v1_id, "v2 should have a different id");

        // Verify v1 was superseded: check via direct SQL
        let v1_row: (i64, Option<String>, Option<String>) = store
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT is_latest, valid_to, superseded_by FROM relations WHERE id = ?1",
                params![v1_id],
                |row| Ok((row.get::<_, i64>(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(v1_row.0, 0, "v1 is_latest should be 0");
        assert!(v1_row.1.is_some(), "v1 valid_to should be set");
        assert!(v1_row.2.is_some(), "v1 superseded_by should point to v2 id");
    }

    #[test]
    fn upsert_relation_no_supersession_for_new_edge() {
        let store = MetadataStore::open_in_memory(false).unwrap();

        // Insert a brand-new edge (no existing version to supersede)
        let rec = RelationRecord {
            account_id: "acme",
            user_id: "alice",
            agent_id: Some("agent-1"),
            from_uri: "mfs://sessions/s1",
            to_uri: "mfs://resources/docs",
            relation_type: "accessed",
        };
        store.upsert_relation(&rec).unwrap();

        let relations = store
            .list_relations("acme", "alice", "mfs://sessions/s1", 10)
            .unwrap();
        assert_eq!(relations.len(), 1, "new edge should appear");
        assert_eq!(relations[0].is_latest, 1);
        assert!(
            relations[0].valid_from.is_some(),
            "new edge should have valid_from"
        );
        assert_eq!(
            relations[0].valid_to, None,
            "new edge should have open-ended valid_to"
        );
        assert_eq!(
            relations[0].superseded_by, None,
            "new edge should not be superseded"
        );
    }
}

// ── CodeSymbol ──────────────────────────────────────────────────────────

/// Input record for inserting a code symbol.
#[derive(Debug, Clone)]
pub struct CodeSymbolRecord<'a> {
    pub id: &'a str,
    pub account_id: &'a str,
    pub user_id: &'a str,
    pub agent_id: Option<&'a str>,
    pub projection_view_id: &'a str,
    pub canonical_uri: &'a str,
    pub symbol_type: &'a str,
    pub symbol_name: &'a str,
    pub signature: Option<&'a str>,
    pub docstring: Option<&'a str>,
    pub line_number: Option<i64>,
    pub embedding_json: Option<&'a str>,
}

/// Stored code symbol (all fields owned).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredCodeSymbol {
    pub id: String,
    pub account_id: String,
    pub user_id: String,
    pub agent_id: Option<String>,
    pub projection_view_id: String,
    pub canonical_uri: String,
    pub symbol_type: String,
    pub symbol_name: String,
    pub signature: Option<String>,
    pub docstring: Option<String>,
    pub line_number: Option<i64>,
    pub embedding_json: Option<String>,
    pub created_at: String,
}

fn stored_code_symbol_from_row(row: &rusqlite::Row<'_>) -> Result<StoredCodeSymbol> {
    Ok(StoredCodeSymbol {
        id: row.get(0)?,
        account_id: row.get(1)?,
        user_id: row.get(2)?,
        agent_id: row.get(3)?,
        projection_view_id: row.get(4)?,
        canonical_uri: row.get(5)?,
        symbol_type: row.get(6)?,
        symbol_name: row.get(7)?,
        signature: row.get(8)?,
        docstring: row.get(9)?,
        line_number: row.get(10)?,
        embedding_json: row.get(11)?,
        created_at: row.get(12)?,
    })
}

impl MetadataStore {
    /// Insert a code symbol record.
    pub fn insert_code_symbol(&self, rec: &CodeSymbolRecord<'_>) -> Result<()> {
        let agent_id = rec.agent_id.unwrap_or("coding-agent");
        self.lock_conn()?.execute(
            "INSERT INTO code_symbols (
                id, account_id, user_id, agent_id,
                projection_view_id, canonical_uri,
                symbol_type, symbol_name, signature,
                docstring, line_number, embedding_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                rec.id,
                rec.account_id,
                rec.user_id,
                agent_id,
                rec.projection_view_id,
                rec.canonical_uri,
                rec.symbol_type,
                rec.symbol_name,
                rec.signature,
                rec.docstring,
                rec.line_number,
                rec.embedding_json,
            ],
        )?;
        Ok(())
    }

    /// Insert multiple code symbols in a single transaction.
    pub fn insert_code_symbols_batch(&self, records: &[CodeSymbolRecord<'_>]) -> Result<()> {
        let conn = self.lock_conn()?;
        let tx = conn.unchecked_transaction()?;
        for rec in records {
            let agent_id = rec.agent_id.unwrap_or("coding-agent");
            tx.execute(
                "INSERT INTO code_symbols (
                    id, account_id, user_id, agent_id,
                    projection_view_id, canonical_uri,
                    symbol_type, symbol_name, signature,
                    docstring, line_number, embedding_json
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    rec.id,
                    rec.account_id,
                    rec.user_id,
                    agent_id,
                    rec.projection_view_id,
                    rec.canonical_uri,
                    rec.symbol_type,
                    rec.symbol_name,
                    rec.signature,
                    rec.docstring,
                    rec.line_number,
                    rec.embedding_json,
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Get code symbols by projection_view_id and optional canonical_uri.
    pub fn get_code_symbols(
        &self,
        projection_view_id: &str,
        canonical_uri: Option<&str>,
    ) -> Result<Vec<StoredCodeSymbol>> {
        if let Some(uri) = canonical_uri {
            let conn = self.lock_conn()?;
            let mut stmt = conn.prepare(
                "SELECT * FROM code_symbols
                 WHERE projection_view_id = ?1 AND canonical_uri = ?2
                 ORDER BY line_number",
            )?;
            let mut rows = stmt.query(params![projection_view_id, uri])?;
            let mut result = Vec::new();
            while let Some(row) = rows.next()? {
                result.push(stored_code_symbol_from_row(row)?);
            }
            Ok(result)
        } else {
            let conn = self.lock_conn()?;
            let mut stmt = conn.prepare(
                "SELECT * FROM code_symbols
                 WHERE projection_view_id = ?1
                 ORDER BY canonical_uri, line_number",
            )?;
            let mut rows = stmt.query(params![projection_view_id])?;
            let mut result = Vec::new();
            while let Some(row) = rows.next()? {
                result.push(stored_code_symbol_from_row(row)?);
            }
            Ok(result)
        }
    }

    /// Search code symbols by name prefix (case-insensitive).
    pub fn search_code_symbols(
        &self,
        projection_view_id: &str,
        name_prefix: &str,
    ) -> Result<Vec<StoredCodeSymbol>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT * FROM code_symbols
             WHERE projection_view_id = ?1
               AND symbol_name LIKE ?2
             ORDER BY symbol_name, canonical_uri
             LIMIT 100",
        )?;
        let pattern = format!("{}%", name_prefix);
        let mut rows = stmt.query(params![projection_view_id, pattern])?;
        let mut result = Vec::new();
        while let Some(row) = rows.next()? {
            result.push(stored_code_symbol_from_row(row)?);
        }
        Ok(result)
    }

    /// Delete all code symbols for a projection_view_id.
    pub fn delete_code_symbols_for_view(&self, projection_view_id: &str) -> Result<usize> {
        self.lock_conn()?.execute(
            "DELETE FROM code_symbols WHERE projection_view_id = ?1",
            params![projection_view_id],
        )
    }

    pub fn delete_code_symbols_for_prefix(
        &self,
        projection_view_id: &str,
        canonical_uri_prefix: &str,
    ) -> Result<usize> {
        let like = format!("{}%", canonical_uri_prefix.trim_end_matches('/'));
        self.lock_conn()?.execute(
            "DELETE FROM code_symbols
             WHERE projection_view_id = ?1
               AND canonical_uri LIKE ?2",
            params![projection_view_id, like],
        )
    }

    /// Count code symbols for a projection_view_id.
    pub fn count_code_symbols(&self, projection_view_id: &str) -> Result<usize> {
        self.lock_conn()?.query_row(
            "SELECT COUNT(*) FROM code_symbols WHERE projection_view_id = ?1",
            params![projection_view_id],
            |row| row.get::<_, usize>(0),
        )
    }

    // ─── Manifest Repo Identity ───────────────────────────────────────

    pub fn upsert_manifest_identity(&self, record: &ManifestIdentityRecord<'_>) -> Result<usize> {
        self.lock_conn()?.execute(
            "INSERT INTO manifest_repo_identity (
                repo_id, resource_uri, default_branch,
                primary_languages, created_at, last_verified_at, manifest_yaml_path,
                repo_name, repo_path, last_commit_hash, last_commit_date,
                manifest_version, yaml_hash, source_roots_json, quality_gates_json, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
             ON CONFLICT(repo_id)
             DO UPDATE SET
                resource_uri = excluded.resource_uri,
                default_branch = excluded.default_branch,
                primary_languages = excluded.primary_languages,
                last_verified_at = excluded.last_verified_at,
                manifest_yaml_path = excluded.manifest_yaml_path,
                repo_name = excluded.repo_name,
                repo_path = excluded.repo_path,
                last_commit_hash = excluded.last_commit_hash,
                last_commit_date = excluded.last_commit_date,
                manifest_version = excluded.manifest_version,
                yaml_hash = excluded.yaml_hash,
                source_roots_json = excluded.source_roots_json,
                quality_gates_json = excluded.quality_gates_json,
                updated_at = excluded.updated_at",
            params![
                record.repo_id,
                record.resource_uri,
                record.default_branch,
                record.primary_languages,
                record.created_at,
                record.last_verified_at,
                record.manifest_yaml_path,
                record.repo_name,
                record.repo_path,
                record.last_commit_hash,
                record.last_commit_date,
                record.manifest_version,
                record.yaml_hash,
                record.source_roots_json,
                record.quality_gates_json,
                record.updated_at,
            ],
        )
    }

    pub fn get_manifest_identity(&self, repo_id: &str) -> Result<Option<StoredManifestIdentity>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT repo_id, resource_uri, default_branch,
                    primary_languages, created_at, last_verified_at, manifest_yaml_path,
                    repo_name, repo_path, last_commit_hash, last_commit_date,
                    manifest_version, yaml_hash, source_roots_json, quality_gates_json, updated_at
             FROM manifest_repo_identity
             WHERE repo_id = ?1
             LIMIT 1",
        )?;
        let mut rows = stmt.query(params![repo_id])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        Ok(Some(stored_manifest_identity_from_row(row)?))
    }

    pub fn list_manifest_identities(&self) -> Result<Vec<StoredManifestIdentity>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT repo_id, resource_uri, default_branch,
                    primary_languages, created_at, last_verified_at, manifest_yaml_path,
                    repo_name, repo_path, last_commit_hash, last_commit_date,
                    manifest_version, yaml_hash, source_roots_json, quality_gates_json, updated_at
             FROM manifest_repo_identity
             ORDER BY repo_id",
        )?;
        let rows = stmt.query_map([], stored_manifest_identity_from_row)?;
        rows.collect()
    }

    // ─── Canvas Nodes ──────────────────────────────────────────────────

    pub fn upsert_canvas_node(&self, record: &CanvasNodeRecord<'_>) -> Result<usize> {
        self.lock_canvas_conn()?.execute(
            "INSERT INTO canvas_nodes (
                id, repo_id, node_type, name, path, language, purpose,
                confidence, generator, generated_at, version_hash, source,
                manifest_id, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
             ON CONFLICT(id)
             DO UPDATE SET
                node_type = excluded.node_type,
                name = excluded.name,
                path = excluded.path,
                language = excluded.language,
                purpose = excluded.purpose,
                confidence = excluded.confidence,
                generator = excluded.generator,
                generated_at = excluded.generated_at,
                version_hash = excluded.version_hash,
                source = excluded.source,
                manifest_id = excluded.manifest_id,
                created_at = excluded.created_at,
                updated_at = excluded.updated_at",
            params![
                record.id,
                record.repo_id,
                record.node_type,
                record.name,
                record.path,
                record.language,
                record.purpose,
                record.confidence,
                record.generator,
                record.generated_at,
                record.version_hash,
                record.source,
                record.manifest_id,
                record.created_at,
                record.updated_at,
            ],
        )
    }

    pub fn get_canvas_node(&self, id: &str) -> Result<Option<StoredCanvasNode>> {
        let conn = self.lock_canvas_conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, repo_id, node_type, name, path, language, purpose,
                    confidence, generator, generated_at, version_hash, source,
                    manifest_id, created_at, updated_at
             FROM canvas_nodes WHERE id = ?1 LIMIT 1",
        )?;
        let mut rows = stmt.query(params![id])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        Ok(Some(stored_canvas_node_from_row(row)?))
    }

    pub fn list_canvas_nodes(
        &self,
        repo_id: &str,
        node_type: Option<&str>,
        name_filter: Option<&str>,
    ) -> Result<Vec<StoredCanvasNode>> {
        let conn = self.lock_canvas_conn()?;
        let sql = match (node_type, name_filter) {
            (Some(_), Some(_)) => {
                "SELECT id, repo_id, node_type, name, path, language, purpose, confidence, generator, generated_at, version_hash, source, manifest_id, created_at, updated_at FROM canvas_nodes WHERE repo_id = ?1 AND node_type = ?2 AND name LIKE ?3"
            }
            (Some(_), None) => {
                "SELECT id, repo_id, node_type, name, path, language, purpose, confidence, generator, generated_at, version_hash, source, manifest_id, created_at, updated_at FROM canvas_nodes WHERE repo_id = ?1 AND node_type = ?2"
            }
            (None, Some(_)) => {
                "SELECT id, repo_id, node_type, name, path, language, purpose, confidence, generator, generated_at, version_hash, source, manifest_id, created_at, updated_at FROM canvas_nodes WHERE repo_id = ?1 AND name LIKE ?2"
            }
            (None, None) => {
                "SELECT id, repo_id, node_type, name, path, language, purpose, confidence, generator, generated_at, version_hash, source, manifest_id, created_at, updated_at FROM canvas_nodes WHERE repo_id = ?1"
            }
        };
        let mut stmt = conn.prepare(sql)?;
        let rows = match (node_type, name_filter) {
            (Some(nt), Some(nf)) => stmt.query_map(
                params![repo_id, nt, format!("%{}%", nf)],
                stored_canvas_node_from_row,
            )?,
            (Some(nt), None) => {
                stmt.query_map(params![repo_id, nt], stored_canvas_node_from_row)?
            }
            (None, Some(nf)) => stmt.query_map(
                params![repo_id, format!("%{}%", nf)],
                stored_canvas_node_from_row,
            )?,
            (None, None) => stmt.query_map(params![repo_id], stored_canvas_node_from_row)?,
        };
        rows.collect()
    }

    pub fn delete_canvas_nodes_by_repo(&self, repo_id: &str) -> Result<usize> {
        self.lock_canvas_conn()?.execute(
            "DELETE FROM canvas_nodes WHERE repo_id = ?1",
            params![repo_id],
        )
    }

    // ─── Canvas Edges ──────────────────────────────────────────────────

    pub fn upsert_canvas_edge(&self, record: &CanvasEdgeRecord<'_>) -> Result<usize> {
        self.lock_canvas_conn()?.execute(
            "INSERT INTO canvas_edges (
                id, repo_id, edge_type, source_node_id, target_node_id,
                contract_spec, confidence, generator, generated_at, version_hash,
                manifest_id, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
             ON CONFLICT(id)
             DO UPDATE SET
                edge_type = excluded.edge_type,
                source_node_id = excluded.source_node_id,
                target_node_id = excluded.target_node_id,
                contract_spec = excluded.contract_spec,
                confidence = excluded.confidence,
                generator = excluded.generator,
                generated_at = excluded.generated_at,
                version_hash = excluded.version_hash,
                manifest_id = excluded.manifest_id,
                created_at = excluded.created_at,
                updated_at = excluded.updated_at",
            params![
                record.id,
                record.repo_id,
                record.edge_type,
                record.source_node_id,
                record.target_node_id,
                record.contract_spec,
                record.confidence,
                record.generator,
                record.generated_at,
                record.version_hash,
                record.manifest_id,
                record.created_at,
                record.updated_at,
            ],
        )
    }

    pub fn list_canvas_edges_by_nodes(
        &self,
        repo_id: &str,
        node_ids: &[String],
    ) -> Result<Vec<StoredCanvasEdge>> {
        if node_ids.is_empty() {
            return Ok(vec![]);
        }
        let conn = self.lock_canvas_conn()?;
        let placeholders = node_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!(
            "SELECT id, repo_id, edge_type, source_node_id, target_node_id,
                    contract_spec, confidence, generator, generated_at, version_hash,
                    manifest_id, created_at, updated_at
             FROM canvas_edges
             WHERE repo_id = ?1 AND (source_node_id IN ({placeholders}) OR target_node_id IN ({placeholders2}))",
            placeholders2 = placeholders,
        );
        let mut stmt = conn.prepare(&sql)?;
        let params_vec: Vec<&dyn rusqlite::types::ToSql> =
            std::iter::once(&repo_id as &dyn rusqlite::types::ToSql)
                .chain(node_ids.iter().map(|id| id as &dyn rusqlite::types::ToSql))
                .chain(node_ids.iter().map(|id| id as &dyn rusqlite::types::ToSql))
                .collect();
        let rows = stmt.query_map(params_vec.as_slice(), stored_canvas_edge_from_row)?;
        rows.collect()
    }

    pub fn get_canvas_edge(&self, id: &str) -> Result<Option<StoredCanvasEdge>> {
        let conn = self.lock_canvas_conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, repo_id, edge_type, source_node_id, target_node_id,
                    contract_spec, confidence, generator, generated_at, version_hash,
                    manifest_id, created_at, updated_at
             FROM canvas_edges WHERE id = ?1 LIMIT 1",
        )?;
        let mut rows = stmt.query(params![id])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        Ok(Some(stored_canvas_edge_from_row(row)?))
    }

    pub fn list_canvas_edges_by_repo(&self, repo_id: &str) -> Result<Vec<StoredCanvasEdge>> {
        let conn = self.lock_canvas_conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, repo_id, edge_type, source_node_id, target_node_id,
                    contract_spec, confidence, generator, generated_at, version_hash,
                    manifest_id, created_at, updated_at
             FROM canvas_edges WHERE repo_id = ?1",
        )?;
        let rows = stmt.query_map(params![repo_id], stored_canvas_edge_from_row)?;
        rows.collect()
    }

    pub fn delete_canvas_edges_by_repo(&self, repo_id: &str) -> Result<usize> {
        self.lock_canvas_conn()?.execute(
            "DELETE FROM canvas_edges WHERE repo_id = ?1",
            params![repo_id],
        )
    }

    // ─── Canvas Snapshots ──────────────────────────────────────────────

    pub fn insert_canvas_snapshot(&self, record: &CanvasSnapshotRecord<'_>) -> Result<()> {
        self.lock_canvas_conn()?.execute(
            "INSERT INTO canvas_snapshots (
                id, repo_id, merge_commit, snapshot_type,
                snapshot_json, created_at, immutable
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                record.id,
                record.repo_id,
                record.merge_commit,
                record.snapshot_type,
                record.snapshot_json,
                record.created_at,
                record.immutable,
            ],
        )?;
        Ok(())
    }

    pub fn get_canvas_snapshot(&self, snapshot_id: &str) -> Result<Option<StoredCanvasSnapshot>> {
        let conn = self.lock_canvas_conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, repo_id, merge_commit, snapshot_type,
                    snapshot_json, created_at, immutable
             FROM canvas_snapshots WHERE id = ?1 LIMIT 1",
        )?;
        let mut rows = stmt.query(params![snapshot_id])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        Ok(Some(stored_canvas_snapshot_from_row(row)?))
    }

    // ─── Overlay Refs CRUD ──────────────────────────────────────────────

    pub fn upsert_overlay_ref(&self, rec: &OverlayRefRecord) -> Result<usize> {
        let resolved_int = if rec.resolved { 1 } else { 0 };
        let conn = self.lock_canvas_conn()?;
        conn.execute(
            "INSERT OR REPLACE INTO overlay_refs
             (overlay_id, repo_id, ref_kind, canonical_ref,
              local_node_id, local_edge_id, resolved, unresolved_reason, synced_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                rec.overlay_id,
                rec.repo_id,
                rec.ref_kind,
                rec.canonical_ref,
                rec.local_node_id,
                rec.local_edge_id,
                resolved_int,
                rec.unresolved_reason,
                rec.synced_at,
            ],
        )
    }

    pub fn get_overlay_refs_by_overlay(&self, overlay_id: &str) -> Result<Vec<StoredOverlayRef>> {
        let conn = self.lock_canvas_conn()?;
        let mut stmt = conn.prepare(
            "SELECT overlay_id, repo_id, ref_kind, canonical_ref,
                    local_node_id, local_edge_id, resolved, unresolved_reason, synced_at
             FROM overlay_refs WHERE overlay_id = ?1",
        )?;
        let rows = stmt.query_map(params![overlay_id], |row| {
            Ok(StoredOverlayRef {
                overlay_id: row.get(0)?,
                repo_id: row.get(1)?,
                ref_kind: row.get(2)?,
                canonical_ref: row.get(3)?,
                local_node_id: row.get(4)?,
                local_edge_id: row.get(5)?,
                resolved: row.get::<_, i32>(6)? != 0,
                unresolved_reason: row.get(7)?,
                synced_at: row.get(8)?,
            })
        })?;
        rows.collect()
    }

    pub fn list_unresolved_overlay_refs(&self, repo_id: &str) -> Result<Vec<StoredOverlayRef>> {
        let conn = self.lock_canvas_conn()?;
        let mut stmt = conn.prepare(
            "SELECT overlay_id, repo_id, ref_kind, canonical_ref,
                    local_node_id, local_edge_id, resolved, unresolved_reason, synced_at
             FROM overlay_refs WHERE repo_id = ?1 AND resolved = 0",
        )?;
        let rows = stmt.query_map(params![repo_id], |row| {
            Ok(StoredOverlayRef {
                overlay_id: row.get(0)?,
                repo_id: row.get(1)?,
                ref_kind: row.get(2)?,
                canonical_ref: row.get(3)?,
                local_node_id: row.get(4)?,
                local_edge_id: row.get(5)?,
                resolved: row.get::<_, i32>(6)? != 0,
                unresolved_reason: row.get(7)?,
                synced_at: row.get(8)?,
            })
        })?;
        rows.collect()
    }

    pub fn delete_overlay_refs_by_overlay(&self, overlay_id: &str) -> Result<usize> {
        let conn = self.lock_canvas_conn()?;
        conn.execute(
            "DELETE FROM overlay_refs WHERE overlay_id = ?1",
            params![overlay_id],
        )
    }

    // ─── Manifest Cache CRUD ────────────────────────────────────────────

    pub fn upsert_manifest_cache(&self, rec: &ManifestCacheRecord) -> Result<usize> {
        let conn = self.lock_canvas_conn()?;
        conn.execute(
            "INSERT OR REPLACE INTO manifest_cache
             (repo_id, default_branch, primary_languages,
              source_roots_json, last_synced_at, cloud_version_hash)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                rec.repo_id,
                rec.default_branch,
                rec.primary_languages,
                rec.source_roots_json,
                rec.last_synced_at,
                rec.cloud_version_hash,
            ],
        )
    }

    pub fn get_manifest_cache(&self, repo_id: &str) -> Result<Option<StoredManifestCache>> {
        let conn = self.lock_canvas_conn()?;
        let mut stmt = conn.prepare(
            "SELECT repo_id, default_branch, primary_languages,
                    source_roots_json, last_synced_at, cloud_version_hash
             FROM manifest_cache WHERE repo_id = ?1 LIMIT 1",
        )?;
        let mut rows = stmt.query(params![repo_id])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        Ok(Some(StoredManifestCache {
            repo_id: row.get(0)?,
            default_branch: row.get(1)?,
            primary_languages: row.get(2)?,
            source_roots_json: row.get(3)?,
            last_synced_at: row.get(4)?,
            cloud_version_hash: row.get(5)?,
        }))
    }
}

// ─── Manifest Types ────────────────────────────────────────────────────

pub struct ManifestIdentityRecord<'a> {
    pub repo_id: &'a str,
    pub resource_uri: &'a str,
    pub default_branch: &'a str,
    pub primary_languages: &'a str,
    pub created_at: &'a str,
    pub last_verified_at: &'a str,
    pub manifest_yaml_path: Option<&'a str>,
    pub repo_name: Option<&'a str>,
    pub repo_path: Option<&'a str>,
    pub last_commit_hash: Option<&'a str>,
    pub last_commit_date: Option<&'a str>,
    pub manifest_version: &'a str,
    pub yaml_hash: Option<&'a str>,
    pub source_roots_json: &'a str,
    pub quality_gates_json: &'a str,
    pub updated_at: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredManifestIdentity {
    pub repo_id: String,
    pub resource_uri: String,
    pub default_branch: String,
    pub primary_languages: String,
    pub created_at: String,
    pub last_verified_at: String,
    pub manifest_yaml_path: Option<String>,
    pub repo_name: Option<String>,
    pub repo_path: Option<String>,
    pub last_commit_hash: Option<String>,
    pub last_commit_date: Option<String>,
    pub manifest_version: String,
    pub yaml_hash: Option<String>,
    pub source_roots_json: String,
    pub quality_gates_json: String,
    pub updated_at: String,
}

// ─── Canvas Node Types ─────────────────────────────────────────────────

pub struct CanvasNodeRecord<'a> {
    pub id: &'a str,
    pub repo_id: &'a str,
    pub node_type: &'a str,
    pub name: &'a str,
    pub path: Option<&'a str>,
    pub language: Option<&'a str>,
    pub purpose: Option<&'a str>,
    pub confidence: &'a str,
    pub generator: &'a str,
    pub generated_at: &'a str,
    pub version_hash: &'a str,
    pub source: Option<&'a str>,
    pub manifest_id: Option<&'a str>,
    pub created_at: &'a str,
    pub updated_at: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredCanvasNode {
    pub id: String,
    pub repo_id: String,
    pub node_type: String,
    pub name: String,
    pub path: Option<String>,
    pub language: Option<String>,
    pub purpose: Option<String>,
    pub confidence: String,
    pub generator: String,
    pub generated_at: String,
    pub version_hash: String,
    pub source: Option<String>,
    pub manifest_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

// ─── Canvas Edge Types ─────────────────────────────────────────────────

pub struct CanvasEdgeRecord<'a> {
    pub id: &'a str,
    pub repo_id: &'a str,
    pub edge_type: &'a str,
    pub source_node_id: &'a str,
    pub target_node_id: &'a str,
    pub contract_spec: Option<&'a str>,
    pub confidence: &'a str,
    pub generator: &'a str,
    pub generated_at: &'a str,
    pub version_hash: &'a str,
    pub manifest_id: Option<&'a str>,
    pub created_at: &'a str,
    pub updated_at: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredCanvasEdge {
    pub id: String,
    pub repo_id: String,
    pub edge_type: String,
    pub source_node_id: String,
    pub target_node_id: String,
    pub contract_spec: Option<String>,
    pub confidence: String,
    pub generator: String,
    pub generated_at: String,
    pub version_hash: String,
    pub manifest_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

// ─── Overlay Types ─────────────────────────────────────────────────────

pub struct OverlayRecord<'a> {
    pub id: &'a str,
    pub repo_id: &'a str,
    pub overlay_type: &'a str,
    pub tracker: &'a str,
    pub tracker_content_id: &'a str,
    pub tracker_project_item_id: Option<&'a str>,
    pub tracker_identifier: &'a str,
    pub issue_number: Option<i64>,
    pub branch: Option<&'a str>,
    pub pr_url: Option<&'a str>,
    pub agent_session_id: Option<&'a str>,
    pub author: &'a str,
    pub status: &'a str,
    pub content_json: &'a str,
    pub affected_nodes: Option<&'a str>,
    pub affected_edges: Option<&'a str>,
    pub affected_node_refs: Option<&'a str>,
    pub affected_edge_refs: Option<&'a str>,
    pub created_at: &'a str,
    pub updated_at: &'a str,
    pub superseded_by: Option<&'a str>,
    pub manifest_id: Option<&'a str>,
    pub accepted_at: Option<&'a str>,
    pub implemented_at: Option<&'a str>,
    pub merged_at: Option<&'a str>,
    pub stale_at: Option<&'a str>,
    pub abandoned_at: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredOverlay {
    pub id: String,
    pub repo_id: String,
    pub overlay_type: String,
    pub tracker: String,
    pub tracker_content_id: String,
    pub tracker_project_item_id: Option<String>,
    pub tracker_identifier: String,
    pub issue_number: Option<i64>,
    pub branch: Option<String>,
    pub pr_url: Option<String>,
    pub agent_session_id: Option<String>,
    pub author: String,
    pub status: String,
    pub content_json: String,
    pub affected_nodes: Option<String>,
    pub affected_edges: Option<String>,
    pub affected_node_refs: Option<String>,
    pub affected_edge_refs: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub superseded_by: Option<String>,
    pub manifest_id: Option<String>,
    pub accepted_at: Option<String>,
    pub implemented_at: Option<String>,
    pub merged_at: Option<String>,
    pub stale_at: Option<String>,
    pub abandoned_at: Option<String>,
}

// ─── Overlay Transition Types ──────────────────────────────────────────

pub struct OverlayTransitionRecord<'a> {
    pub id: &'a str,
    pub overlay_id: &'a str,
    pub from_status: &'a str,
    pub to_status: &'a str,
    pub triggered_by: &'a str,
    pub reason: Option<&'a str>,
    pub created_at: &'a str,
}

// ─── Canvas Snapshot Types ─────────────────────────────────────────────

pub struct CanvasSnapshotRecord<'a> {
    pub id: &'a str,
    pub repo_id: &'a str,
    pub merge_commit: &'a str,
    pub snapshot_type: &'a str,
    pub snapshot_json: &'a str,
    pub created_at: &'a str,
    pub immutable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredCanvasSnapshot {
    pub id: String,
    pub repo_id: String,
    pub merge_commit: String,
    pub snapshot_type: String,
    pub snapshot_json: String,
    pub created_at: String,
    pub immutable: bool,
}

// ─── Overlay Refs (cloud overlay ID → local canvas ID mapping) ───────────

pub struct OverlayRefRecord<'a> {
    pub overlay_id: &'a str,
    pub repo_id: &'a str,
    pub ref_kind: &'a str,      // "node" or "edge"
    pub canonical_ref: &'a str, // canvas://... canonical ref
    pub local_node_id: Option<&'a str>,
    pub local_edge_id: Option<&'a str>,
    pub resolved: bool,
    pub unresolved_reason: Option<&'a str>,
    pub synced_at: &'a str,
}

pub struct StoredOverlayRef {
    pub overlay_id: String,
    pub repo_id: String,
    pub ref_kind: String,
    pub canonical_ref: String,
    pub local_node_id: Option<String>,
    pub local_edge_id: Option<String>,
    pub resolved: bool,
    pub unresolved_reason: Option<String>,
    pub synced_at: String,
}

// ─── Manifest Cache (local sync of cloud manifest metadata) ──────────────

pub struct ManifestCacheRecord<'a> {
    pub repo_id: &'a str,
    pub default_branch: &'a str,
    pub primary_languages: &'a str,
    pub source_roots_json: &'a str,
    pub last_synced_at: &'a str,
    pub cloud_version_hash: Option<&'a str>,
}

pub struct StoredManifestCache {
    pub repo_id: String,
    pub default_branch: String,
    pub primary_languages: String,
    pub source_roots_json: String,
    pub last_synced_at: String,
    pub cloud_version_hash: Option<String>,
}

// ─── Row → Stored helper functions ─────────────────────────────────────

fn stored_manifest_identity_from_row(row: &rusqlite::Row<'_>) -> Result<StoredManifestIdentity> {
    Ok(StoredManifestIdentity {
        repo_id: row.get(0)?,
        resource_uri: row.get(1)?,
        default_branch: row.get(2)?,
        primary_languages: row.get(3)?,
        created_at: row.get(4)?,
        last_verified_at: row.get(5)?,
        manifest_yaml_path: row.get(6)?,
        repo_name: row.get(7)?,
        repo_path: row.get(8)?,
        last_commit_hash: row.get(9)?,
        last_commit_date: row.get(10)?,
        manifest_version: row.get(11)?,
        yaml_hash: row.get(12)?,
        source_roots_json: row.get(13)?,
        quality_gates_json: row.get(14)?,
        updated_at: row.get(15)?,
    })
}

fn stored_canvas_node_from_row(row: &rusqlite::Row<'_>) -> Result<StoredCanvasNode> {
    Ok(StoredCanvasNode {
        id: row.get(0)?,
        repo_id: row.get(1)?,
        node_type: row.get(2)?,
        name: row.get(3)?,
        path: row.get(4)?,
        language: row.get(5)?,
        purpose: row.get(6)?,
        confidence: row.get(7)?,
        generator: row.get(8)?,
        generated_at: row.get(9)?,
        version_hash: row.get(10)?,
        source: row.get(11)?,
        manifest_id: row.get(12)?,
        created_at: row.get(13)?,
        updated_at: row.get(14)?,
    })
}

fn stored_canvas_edge_from_row(row: &rusqlite::Row<'_>) -> Result<StoredCanvasEdge> {
    Ok(StoredCanvasEdge {
        id: row.get(0)?,
        repo_id: row.get(1)?,
        edge_type: row.get(2)?,
        source_node_id: row.get(3)?,
        target_node_id: row.get(4)?,
        contract_spec: row.get(5)?,
        confidence: row.get(6)?,
        generator: row.get(7)?,
        generated_at: row.get(8)?,
        version_hash: row.get(9)?,
        manifest_id: row.get(10)?,
        created_at: row.get(11)?,
        updated_at: row.get(12)?,
    })
}

fn stored_canvas_snapshot_from_row(row: &rusqlite::Row<'_>) -> Result<StoredCanvasSnapshot> {
    Ok(StoredCanvasSnapshot {
        id: row.get(0)?,
        repo_id: row.get(1)?,
        merge_commit: row.get(2)?,
        snapshot_type: row.get(3)?,
        snapshot_json: row.get(4)?,
        created_at: row.get(5)?,
        immutable: row.get::<_, i64>(6)? != 0,
    })
}

// ─── CanvasStore trait impl ─────────────────────────────────────────────

use crate::canvas_store::CanvasStore;

impl CanvasStore for MetadataStore {
    fn upsert_canvas_node(&self, record: &CanvasNodeRecord<'_>) -> Result<usize, CanvasStoreError> {
        self.upsert_canvas_node(record)
            .map_err(CanvasStoreError::from)
    }
    fn get_canvas_node(&self, id: &str) -> Result<Option<StoredCanvasNode>, CanvasStoreError> {
        self.get_canvas_node(id).map_err(CanvasStoreError::from)
    }
    fn list_canvas_nodes(
        &self,
        repo_id: &str,
        node_type: Option<&str>,
        name_filter: Option<&str>,
    ) -> Result<Vec<StoredCanvasNode>, CanvasStoreError> {
        self.list_canvas_nodes(repo_id, node_type, name_filter)
            .map_err(CanvasStoreError::from)
    }
    fn delete_canvas_nodes_by_repo(&self, repo_id: &str) -> Result<usize, CanvasStoreError> {
        self.delete_canvas_nodes_by_repo(repo_id)
            .map_err(CanvasStoreError::from)
    }
    fn upsert_canvas_edge(&self, record: &CanvasEdgeRecord<'_>) -> Result<usize, CanvasStoreError> {
        self.upsert_canvas_edge(record)
            .map_err(CanvasStoreError::from)
    }
    fn list_canvas_edges_by_nodes(
        &self,
        repo_id: &str,
        node_ids: &[String],
    ) -> Result<Vec<StoredCanvasEdge>, CanvasStoreError> {
        self.list_canvas_edges_by_nodes(repo_id, node_ids)
            .map_err(CanvasStoreError::from)
    }
    fn get_canvas_edge(&self, id: &str) -> Result<Option<StoredCanvasEdge>, CanvasStoreError> {
        self.get_canvas_edge(id).map_err(CanvasStoreError::from)
    }
    fn list_canvas_edges_by_repo(
        &self,
        repo_id: &str,
    ) -> Result<Vec<StoredCanvasEdge>, CanvasStoreError> {
        self.list_canvas_edges_by_repo(repo_id)
            .map_err(CanvasStoreError::from)
    }
    fn delete_canvas_edges_by_repo(&self, repo_id: &str) -> Result<usize, CanvasStoreError> {
        self.delete_canvas_edges_by_repo(repo_id)
            .map_err(CanvasStoreError::from)
    }
    fn insert_canvas_snapshot(
        &self,
        record: &CanvasSnapshotRecord<'_>,
    ) -> Result<(), CanvasStoreError> {
        self.insert_canvas_snapshot(record)
            .map_err(CanvasStoreError::from)
    }
    fn get_canvas_snapshot(
        &self,
        snapshot_id: &str,
    ) -> Result<Option<StoredCanvasSnapshot>, CanvasStoreError> {
        self.get_canvas_snapshot(snapshot_id)
            .map_err(CanvasStoreError::from)
    }
}

/// Stored run writeback record — read model for run evidence persisted
/// by an external orchestrator after an agent ticket execution completes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredRunWriteback {
    pub repo_id: String,
    pub run_id: String,
    pub tracker: String,
    pub tracker_identifier: String,
    pub idempotency_key: String,
    pub payload_json: String,
    pub created_at: String,
}
