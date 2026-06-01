//! Canvas Store trait — B2 (SaaS §7.2)
//!
//! Defines the Canvas data persistence interface (nodes, edges, snapshots).
//! This trait isolates the Canvas domain from the monolithic MetadataStore,
//! enabling future substitution with a PostgreSQL-backed Canvas Store
//! for SaaS degraded mode (stale cloud snapshots).
//!
//! The current Sqlite-backed implementation is provided by MetadataStore
//! via the `impl CanvasStore for MetadataStore` block in store.rs.

use crate::store::{
    CanvasEdgeRecord, CanvasNodeRecord, CanvasSnapshotRecord, StoredCanvasEdge, StoredCanvasNode,
    StoredCanvasSnapshot,
};

/// Canvas data persistence trait.
///
/// All methods use domain types only (no rusqlite in the trait interface).
/// The `Result` type maps rusqlite errors to a generic error for trait decoupling.
/// Requires Send + Sync for thread-safe Arc<dyn CanvasStore> usage in AppState.
pub trait CanvasStore: Send + Sync {
    // ── Nodes ──────────────────────────────────────────────────────────

    /// Insert or update a canvas node. Returns affected row count.
    fn upsert_canvas_node(&self, record: &CanvasNodeRecord<'_>) -> Result<usize, CanvasStoreError>;

    /// Get a single canvas node by its local ID.
    fn get_canvas_node(&self, id: &str) -> Result<Option<StoredCanvasNode>, CanvasStoreError>;

    /// List canvas nodes for a repo, optionally filtered by node type or name.
    fn list_canvas_nodes(
        &self,
        repo_id: &str,
        node_type: Option<&str>,
        name_filter: Option<&str>,
    ) -> Result<Vec<StoredCanvasNode>, CanvasStoreError>;

    /// Delete all canvas nodes for a repo. Returns deleted row count.
    fn delete_canvas_nodes_by_repo(&self, repo_id: &str) -> Result<usize, CanvasStoreError>;

    // ── Edges ──────────────────────────────────────────────────────────

    /// Insert or update a canvas edge. Returns affected row count.
    fn upsert_canvas_edge(&self, record: &CanvasEdgeRecord<'_>) -> Result<usize, CanvasStoreError>;

    /// List canvas edges connected to a set of node IDs.
    fn list_canvas_edges_by_nodes(
        &self,
        repo_id: &str,
        node_ids: &[String],
    ) -> Result<Vec<StoredCanvasEdge>, CanvasStoreError>;

    /// Get a single canvas edge by its ID.
    fn get_canvas_edge(&self, id: &str) -> Result<Option<StoredCanvasEdge>, CanvasStoreError>;

    /// List all canvas edges for a repo.
    fn list_canvas_edges_by_repo(
        &self,
        repo_id: &str,
    ) -> Result<Vec<StoredCanvasEdge>, CanvasStoreError>;

    /// Delete all canvas edges for a repo. Returns deleted row count.
    fn delete_canvas_edges_by_repo(&self, repo_id: &str) -> Result<usize, CanvasStoreError>;

    // ── Snapshots ──────────────────────────────────────────────────────

    /// Insert a new canvas snapshot (immutable after creation).
    fn insert_canvas_snapshot(
        &self,
        record: &CanvasSnapshotRecord<'_>,
    ) -> Result<(), CanvasStoreError>;

    /// Get a canvas snapshot by its ID.
    fn get_canvas_snapshot(
        &self,
        snapshot_id: &str,
    ) -> Result<Option<StoredCanvasSnapshot>, CanvasStoreError>;
}

/// Error type for CanvasStore operations.
///
/// Wraps rusqlite errors for the SQLite impl; will wrap sqlx/postgres errors
/// for the future PostgreSQL impl. Provides a stable interface across backends.
#[derive(Debug)]
pub enum CanvasStoreError {
    /// SQLite backend error (rusqlite).
    Sqlite(String),
    /// PostgreSQL backend error (future).
    Postgres(String),
    /// General / fallback error.
    Other(String),
}

impl std::fmt::Display for CanvasStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CanvasStoreError::Sqlite(msg) => write!(f, "CanvasStore sqlite error: {}", msg),
            CanvasStoreError::Postgres(msg) => write!(f, "CanvasStore postgres error: {}", msg),
            CanvasStoreError::Other(msg) => write!(f, "CanvasStore error: {}", msg),
        }
    }
}

impl std::error::Error for CanvasStoreError {}

impl From<rusqlite::Error> for CanvasStoreError {
    fn from(err: rusqlite::Error) -> Self {
        CanvasStoreError::Sqlite(err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::{CanvasStore, CanvasStoreError};
    use crate::store::{
        CanvasEdgeRecord, CanvasNodeRecord, CanvasSnapshotRecord, ManifestIdentityRecord,
        MetadataStore,
    };
    use std::sync::Arc;

    /// Create an in-memory MetadataStore with a manifest for REPO,
    /// wrapped as Arc<dyn CanvasStore>.
    fn trait_store() -> Arc<dyn CanvasStore> {
        let meta = MetadataStore::open_in_memory(false).unwrap();
        meta.upsert_manifest_identity(&ManifestIdentityRecord {
            repo_id: REPO,
            resource_uri: "mfs://resources/localfs/test-repo/MANIFEST.yaml",
            default_branch: "main",
            primary_languages: r#"["elixir"]"#,
            created_at: NOW,
            last_verified_at: NOW,
            manifest_yaml_path: Some("/workspace/MANIFEST.yaml"),
            repo_name: None,
            repo_path: None,
            last_commit_hash: None,
            last_commit_date: None,
            manifest_version: "1",
            yaml_hash: None,
            source_roots_json: "[]",
            quality_gates_json: "{}",
            updated_at: NOW,
        })
        .unwrap();
        Arc::new(meta) as Arc<dyn CanvasStore>
    }

    const REPO: &str = "test-repo";
    const NOW: &str = "2025-01-01T00:00:00Z";

    // ── Nodes ──────────────────────────────────────────────────────────

    #[test]
    fn upsert_and_get_canvas_node() {
        let store = trait_store();
        let rec = CanvasNodeRecord {
            id: "n1",
            repo_id: REPO,
            node_type: "module",
            name: "MyModule",
            path: Some("src/my_module.ex"),
            language: Some("elixir"),
            purpose: Some("business logic"),
            confidence: "deterministic",
            generator: "regex-deterministic",
            generated_at: NOW,
            version_hash: "v1-content:abc123",
            source: None,
            manifest_id: Some(REPO),
            created_at: NOW,
            updated_at: NOW,
        };
        assert_eq!(store.upsert_canvas_node(&rec).unwrap(), 1);
        let node = store.get_canvas_node("n1").unwrap().unwrap();
        assert_eq!(node.id, "n1");
        assert_eq!(node.node_type, "module");
    }

    #[test]
    fn list_canvas_nodes_filters_by_type() {
        let store = trait_store();
        for (id, ntype) in [("n1", "module"), ("n2", "function")] {
            store
                .upsert_canvas_node(&CanvasNodeRecord {
                    id,
                    repo_id: REPO,
                    node_type: ntype,
                    name: id,
                    path: None,
                    language: None,
                    purpose: None,
                    confidence: "deterministic",
                    generator: "regex-deterministic",
                    generated_at: NOW,
                    version_hash: "v1-content:abc",
                    source: None,
                    manifest_id: Some(REPO),
                    created_at: NOW,
                    updated_at: NOW,
                })
                .unwrap();
        }
        let modules = store.list_canvas_nodes(REPO, Some("module"), None).unwrap();
        assert_eq!(modules.len(), 1);
        assert_eq!(modules[0].node_type, "module");
    }

    #[test]
    fn delete_canvas_nodes_by_repo() {
        let store = trait_store();
        store
            .upsert_canvas_node(&CanvasNodeRecord {
                id: "n1",
                repo_id: REPO,
                node_type: "module",
                name: "M",
                path: None,
                language: None,
                purpose: None,
                confidence: "deterministic",
                generator: "regex-deterministic",
                generated_at: NOW,
                version_hash: "v1-content:abc",
                source: None,
                manifest_id: Some(REPO),
                created_at: NOW,
                updated_at: NOW,
            })
            .unwrap();
        assert_eq!(store.delete_canvas_nodes_by_repo(REPO).unwrap(), 1);
        assert!(store.get_canvas_node("n1").unwrap().is_none());
    }

    // ── Edges ──────────────────────────────────────────────────────────

    #[test]
    fn upsert_and_get_canvas_edge() {
        let store = trait_store();
        // Create source/target nodes first
        for id in ["src_n", "tgt_n"] {
            store
                .upsert_canvas_node(&CanvasNodeRecord {
                    id,
                    repo_id: REPO,
                    node_type: "module",
                    name: id,
                    path: None,
                    language: None,
                    purpose: None,
                    confidence: "deterministic",
                    generator: "regex-deterministic",
                    generated_at: NOW,
                    version_hash: "v1-content:abc",
                    source: None,
                    manifest_id: Some(REPO),
                    created_at: NOW,
                    updated_at: NOW,
                })
                .unwrap();
        }
        let rec = CanvasEdgeRecord {
            id: "e1",
            repo_id: REPO,
            edge_type: "call",
            source_node_id: "src_n",
            target_node_id: "tgt_n",
            contract_spec: None,
            confidence: "deterministic",
            generator: "regex-deterministic",
            generated_at: NOW,
            version_hash: "v1-content:abc",
            manifest_id: Some(REPO),
            created_at: NOW,
            updated_at: NOW,
        };
        assert_eq!(store.upsert_canvas_edge(&rec).unwrap(), 1);
        let edge = store.get_canvas_edge("e1").unwrap().unwrap();
        assert_eq!(edge.edge_type, "call");
    }

    #[test]
    fn list_canvas_edges_by_nodes_and_repo() {
        let store = trait_store();
        for id in ["src_n", "tgt_n", "other_n"] {
            store
                .upsert_canvas_node(&CanvasNodeRecord {
                    id,
                    repo_id: REPO,
                    node_type: "module",
                    name: id,
                    path: None,
                    language: None,
                    purpose: None,
                    confidence: "deterministic",
                    generator: "regex-deterministic",
                    generated_at: NOW,
                    version_hash: "v1-content:abc",
                    source: None,
                    manifest_id: Some(REPO),
                    created_at: NOW,
                    updated_at: NOW,
                })
                .unwrap();
        }
        store
            .upsert_canvas_edge(&CanvasEdgeRecord {
                id: "e1",
                repo_id: REPO,
                edge_type: "call",
                source_node_id: "src_n",
                target_node_id: "tgt_n",
                contract_spec: None,
                confidence: "deterministic",
                generator: "regex-deterministic",
                generated_at: NOW,
                version_hash: "v1-content:abc",
                manifest_id: Some(REPO),
                created_at: NOW,
                updated_at: NOW,
            })
            .unwrap();
        // By specific nodes
        let by_nodes = store
            .list_canvas_edges_by_nodes(REPO, &["src_n".into(), "tgt_n".into()])
            .unwrap();
        assert_eq!(by_nodes.len(), 1);
        // By repo
        let by_repo = store.list_canvas_edges_by_repo(REPO).unwrap();
        assert_eq!(by_repo.len(), 1);
    }

    #[test]
    fn delete_canvas_edges_by_repo() {
        let store = trait_store();
        for id in ["src_n", "tgt_n"] {
            store
                .upsert_canvas_node(&CanvasNodeRecord {
                    id,
                    repo_id: REPO,
                    node_type: "module",
                    name: id,
                    path: None,
                    language: None,
                    purpose: None,
                    confidence: "deterministic",
                    generator: "regex-deterministic",
                    generated_at: NOW,
                    version_hash: "v1-content:abc",
                    source: None,
                    manifest_id: Some(REPO),
                    created_at: NOW,
                    updated_at: NOW,
                })
                .unwrap();
        }
        store
            .upsert_canvas_edge(&CanvasEdgeRecord {
                id: "e1",
                repo_id: REPO,
                edge_type: "call",
                source_node_id: "src_n",
                target_node_id: "tgt_n",
                contract_spec: None,
                confidence: "deterministic",
                generator: "regex-deterministic",
                generated_at: NOW,
                version_hash: "v1-content:abc",
                manifest_id: Some(REPO),
                created_at: NOW,
                updated_at: NOW,
            })
            .unwrap();
        assert_eq!(store.delete_canvas_edges_by_repo(REPO).unwrap(), 1);
        assert!(store.get_canvas_edge("e1").unwrap().is_none());
    }

    // ── Snapshots ──────────────────────────────────────────────────────

    #[test]
    fn insert_and_get_canvas_snapshot() {
        let store = trait_store();
        let rec = CanvasSnapshotRecord {
            id: "snap1",
            repo_id: REPO,
            merge_commit: "v1-content:abc123",
            snapshot_type: "full",
            snapshot_json: "{\"nodes\":[]}",
            created_at: NOW,
            immutable: true,
        };
        store.insert_canvas_snapshot(&rec).unwrap();
        let snap = store.get_canvas_snapshot("snap1").unwrap().unwrap();
        assert_eq!(snap.snapshot_type, "full");
        assert!(snap.immutable);
    }

    // ── Arc<dyn CanvasStore> is Send + Sync ────────────────────────────

    #[test]
    fn arc_dyn_canvas_store_is_send_sync() {
        let store: Arc<dyn CanvasStore> = trait_store();
        // Verify the Arc<dyn CanvasStore> can be sent across threads
        let _send: &dyn Send = &*store;
        let _sync: &dyn Sync = &*store;
    }

    // ── Error mapping ──────────────────────────────────────────────────

    #[test]
    fn canvas_store_error_from_rusqlite() {
        let err = CanvasStoreError::from(rusqlite::Error::InvalidColumnIndex(99));
        assert!(matches!(err, CanvasStoreError::Sqlite(_)));
    }

    #[test]
    fn canvas_store_error_display() {
        let err = CanvasStoreError::Postgres("connection refused".into());
        assert!(err.to_string().contains("postgres"));
        let err = CanvasStoreError::Other("custom".into());
        assert!(err.to_string().contains("custom"));
    }
}
