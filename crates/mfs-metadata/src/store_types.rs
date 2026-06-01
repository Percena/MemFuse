//! Domain type definitions extracted from store.rs.
//!
//! These types represent the memory subsystem (facts, episodes, heuristic rules,
//! instances, evidence, assertions, consolidation cursors, briefs) AND the
//! infra/utility subsystem (audit, webhooks, snapshots, relations, tasks)
//! that was previously crammed into the 6050-line store.rs monolith.
//!
//! Extraction is the first step in P1-3: splitting store.rs into domain modules.
//! Types are moved here; `impl MetadataStore` methods stay in store.rs for now.

use rusqlite::Result;
use serde::{Deserialize, Serialize};

// ─── Facts ──────────────────────────────────────────────────────────

/// Input record for inserting a fact (borrows from caller).
pub struct FactRecord<'a> {
    pub id: &'a str,
    pub account_id: &'a str,
    pub user_id: &'a str,
    pub agent_id: Option<&'a str>,
    pub subject: &'a str,
    pub predicate: &'a str,
    pub display_value: &'a str,
    pub normalized_value_json: Option<&'a str>,
    pub value_type: &'a str,
    pub confidence: f64,
    pub status: &'a str,
    pub valid_from: Option<&'a str>,
    pub valid_to: Option<&'a str>,
    pub source_assertion_id: Option<&'a str>,
    pub source_episode_ids_json: Option<&'a str>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredFact {
    pub id: String,
    pub account_id: String,
    pub user_id: String,
    pub agent_id: Option<String>,
    pub subject: String,
    pub predicate: String,
    pub display_value: String,
    pub normalized_value_json: Option<String>,
    pub value_type: String,
    pub confidence: f64,
    pub status: String,
    pub valid_from: Option<String>,
    pub valid_to: Option<String>,
    pub source_assertion_id: Option<String>,
    pub source_episode_ids_json: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub superseded_at: Option<String>,
    pub superseded_by: Option<String>,
    pub recall_count: i64,
    pub last_recalled_at: Option<String>,
}

// ─── Episode Chunks ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct EpisodeRow {
    pub episode_id: String,
    pub account_id: String,
    pub user_id: String,
    pub agent_id: String,
    pub session_id: String,
    pub resource_id: Option<String>,
    pub summary: String,
    pub detail_ref: Option<String>,
    pub keywords_json: Option<String>,
    pub salience_score: f64,
    pub strength_score: f64,
    pub emotional_valence: Option<f64>,
    pub emotional_intensity: Option<f64>,
    pub context_tags_json: Option<String>,
    pub recall_count: i64,
    pub last_recalled_at: Option<String>,
    pub source_start_turn_id: Option<String>,
    pub source_end_turn_id: Option<String>,
    pub created_at: String,
    pub archived_at: Option<String>,
    pub last_decay_at: Option<String>,
    pub embedding_json: Option<String>,
}

// ─── Heuristic Rules ──────────────────────────────────────────────────

pub struct HeuristicRuleRecord<'a> {
    pub rule_id: &'a str,
    pub account_id: &'a str,
    pub user_id: &'a str,
    pub agent_id: Option<&'a str>,
    pub tags_json: &'a str,
    pub rule_text: &'a str,
    pub counter_examples_json: &'a str,
    pub lifecycle_stage: &'a str,
    pub evidence_count: i64,
    pub aggregate_weight: f64,
    pub last_evidence_at: Option<&'a str>,
    pub source_instance_ids_json: Option<&'a str>,
    pub promoted_at: Option<&'a str>,
    pub user_confirmed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredHeuristicRule {
    pub rule_id: String,
    pub account_id: String,
    pub user_id: String,
    pub agent_id: Option<String>,
    pub tags_json: String,
    pub rule_text: String,
    pub counter_examples_json: String,
    pub lifecycle_stage: String,
    pub evidence_count: i64,
    pub aggregate_weight: f64,
    pub last_evidence_at: Option<String>,
    pub source_instance_ids_json: Option<String>,
    pub created_at: String,
    pub promoted_at: Option<String>,
    pub archived_at: Option<String>,
    pub user_confirmed: bool,
}

// ─── Heuristic Instances ──────────────────────────────────────────────

pub struct HeuristicInstanceRecord<'a> {
    pub instance_id: &'a str,
    pub account_id: &'a str,
    pub user_id: &'a str,
    pub agent_id: Option<&'a str>,
    pub context_summary: &'a str,
    pub agent_proposal: Option<&'a str>,
    pub user_reaction: &'a str,
    pub outcome: Option<&'a str>,
    pub signal_type: &'a str,
    pub tags_json: &'a str,
    pub session_id: Option<&'a str>,
    pub source_turn_ids_json: Option<&'a str>,
    pub derived_rule_id: Option<&'a str>,
    pub instance_status: &'a str,
    pub resolved_at: Option<&'a str>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredHeuristicInstance {
    pub instance_id: String,
    pub account_id: String,
    pub user_id: String,
    pub agent_id: Option<String>,
    pub context_summary: String,
    pub agent_proposal: Option<String>,
    pub user_reaction: String,
    pub outcome: Option<String>,
    pub signal_type: String,
    pub tags_json: String,
    pub session_id: Option<String>,
    pub source_turn_ids_json: Option<String>,
    pub derived_rule_id: Option<String>,
    pub instance_status: String,
    pub created_at: String,
    pub resolved_at: Option<String>,
}

// ─── Heuristic Evidence ──────────────────────────────────────────────

pub struct HeuristicEvidenceRecord<'a> {
    pub evidence_id: &'a str,
    pub rule_id: &'a str,
    pub instance_id: Option<&'a str>,
    pub evidence_type: &'a str,
    pub support_weight: f64,
    pub session_id: &'a str,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredHeuristicEvidence {
    pub evidence_id: String,
    pub rule_id: String,
    pub instance_id: Option<String>,
    pub evidence_type: String,
    pub support_weight: f64,
    pub session_id: String,
    pub created_at: String,
}

// ─── Fact Assertions ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct AssertionRow {
    pub assertion_id: String,
    pub account_id: String,
    pub user_id: String,
    pub agent_id: String,
    pub subject: String,
    pub predicate: String,
    pub raw_value_text: String,
    pub normalized_value_json: Option<String>,
    pub value_type: String,
    pub operation: String,
    pub confidence: f64,
    pub valid_from: Option<String>,
    pub valid_to: Option<String>,
    pub source_turn_id: Option<String>,
    pub source_episode_ids_json: Option<String>,
    pub source_resource_id: Option<String>,
    pub source_snapshot_id: Option<String>,
    pub source_uri: Option<String>,
    pub extractor_version: String,
    pub created_at: String,
}

// ─── Consolidation Cursors ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct CursorRow {
    pub cursor_id: String,
    pub account_id: String,
    pub user_id: String,
    pub scope_type: String,
    pub scope_id: String,
    pub last_consolidated_turn_id: Option<String>,
    pub last_consolidated_at: Option<String>,
    pub dedupe_key: Option<String>,
    pub lease_owner: Option<String>,
    pub lease_expires_at: Option<String>,
    pub updated_at: String,
}

// ─── Memory Briefs ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct BriefRow {
    pub brief_id: String,
    pub account_id: String,
    pub user_id: String,
    pub scope_type: String,
    pub scope_id: String,
    pub summary: String,
    pub source_thread_ids_json: Option<String>,
    pub anchor_episode_ids_json: Option<String>,
    pub updated_at: String,
}

// ─── Session & Turn Rows ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct SessionRow {
    pub session_id: String,
    pub account_id: String,
    pub user_id: String,
    pub agent_id: String,
    pub status: String,
    pub started_at: String,
    pub last_activity_at: String,
    pub metadata_json: Option<String>,
}

pub(crate) fn session_row_from_row(row: &rusqlite::Row<'_>) -> Result<SessionRow> {
    Ok(SessionRow {
        session_id: row.get(0)?,
        account_id: row.get(1)?,
        user_id: row.get(2)?,
        agent_id: row.get(3)?,
        status: row.get(4)?,
        started_at: row.get(5)?,
        last_activity_at: row.get(6)?,
        metadata_json: row.get(7)?,
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct TurnRow {
    pub turn_id: String,
    pub turn_seq: i64,
    pub session_id: String,
    pub account_id: String,
    pub user_id: String,
    pub agent_id: String,
    pub role: String,
    pub content_text: String,
    pub content_json: Option<String>,
    pub token_count: i64,
    pub created_at: String,
    pub ingested_at: Option<String>,
}

pub(crate) fn turn_row_from_row(row: &rusqlite::Row<'_>) -> Result<TurnRow> {
    Ok(TurnRow {
        turn_id: row.get(0)?,
        turn_seq: row.get(1)?,
        session_id: row.get(2)?,
        account_id: row.get(3)?,
        user_id: row.get(4)?,
        agent_id: row.get(5)?,
        role: row.get(6)?,
        content_text: row.get(7)?,
        content_json: row.get(8)?,
        token_count: row.get(9)?,
        created_at: row.get(10)?,
        ingested_at: row.get(11)?,
    })
}

// ─── Path Entries ──────────────────────────────────────────────────────

pub struct PathEntryRecord<'a> {
    pub account_id: &'a str,
    pub user_id: &'a str,
    pub agent_id: Option<&'a str>,
    pub projection_view_id: &'a str,
    pub canonical_uri: &'a str,
    pub workspace_path: &'a str,
    pub entry_kind: &'a str,
    pub source_kind: Option<&'a str>,
    pub source_identifier: Option<&'a str>,
    pub source_snapshot_id: Option<&'a str>,
    pub content_kind: Option<&'a str>,
    pub language: Option<&'a str>,
    pub relative_resource_path: Option<&'a str>,
    pub repo_root_uri: Option<&'a str>,
    pub is_text: Option<bool>,
    pub is_generated: Option<bool>,
    pub content_digest: Option<&'a str>,
    pub metadata_digest: Option<&'a str>,
    pub size_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredPathEntry {
    pub account_id: String,
    pub user_id: String,
    pub agent_id: Option<String>,
    pub projection_view_id: String,
    pub canonical_uri: String,
    pub workspace_path: String,
    pub entry_kind: String,
    pub source_kind: Option<String>,
    pub source_identifier: Option<String>,
    pub source_snapshot_id: Option<String>,
    pub content_kind: Option<String>,
    pub language: Option<String>,
    pub relative_resource_path: Option<String>,
    pub repo_root_uri: Option<String>,
    pub is_text: Option<bool>,
    pub is_generated: Option<bool>,
    pub content_digest: Option<String>,
    pub metadata_digest: Option<String>,
    pub size_bytes: Option<u64>,
}

// ─── Resource Sources ──────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ResourceSourceRecord<'a> {
    pub resource_id: &'a str,
    pub account_id: &'a str,
    pub user_id: &'a str,
    pub agent_id: Option<&'a str>,
    pub logical_name: &'a str,
    pub source_kind: &'a str,
    pub source_identifier: &'a str,
    pub canonical_root_uri: &'a str,
    pub projection_view_id: &'a str,
    pub resource_kind: &'a str,
    pub source_host: Option<&'a str>,
    pub source_namespace: Option<&'a str>,
    pub source_repo: Option<&'a str>,
    pub source_ref: Option<&'a str>,
    pub canonical_strategy_version: &'a str,
    pub status: &'a str,
    pub last_snapshot_id: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredResourceSource {
    pub resource_id: String,
    pub account_id: String,
    pub user_id: String,
    pub agent_id: Option<String>,
    pub logical_name: String,
    pub source_kind: String,
    pub source_identifier: String,
    pub canonical_root_uri: String,
    pub projection_view_id: String,
    pub resource_kind: String,
    pub source_host: Option<String>,
    pub source_namespace: Option<String>,
    pub source_repo: Option<String>,
    pub source_ref: Option<String>,
    pub repo_id: Option<String>,
    pub tracker: Option<String>,
    pub tracker_project_identifier: Option<String>,
    pub canonical_strategy_version: String,
    pub status: String,
    pub last_snapshot_id: Option<String>,
    pub updated_at: String,
}

pub(crate) fn stored_resource_source_from_row(
    row: &rusqlite::Row<'_>,
) -> Result<StoredResourceSource> {
    Ok(StoredResourceSource {
        resource_id: row.get(0)?,
        account_id: row.get(1)?,
        user_id: row.get(2)?,
        agent_id: row.get(3)?,
        logical_name: row.get(4)?,
        source_kind: row.get(5)?,
        source_identifier: row.get(6)?,
        canonical_root_uri: row.get(7)?,
        projection_view_id: row.get(8)?,
        resource_kind: row.get(9)?,
        source_host: row.get(10)?,
        source_namespace: row.get(11)?,
        source_repo: row.get(12)?,
        source_ref: row.get(13)?,
        repo_id: row.get(14)?,
        tracker: row.get(15)?,
        tracker_project_identifier: row.get(16)?,
        canonical_strategy_version: row.get(17)?,
        status: row.get(18)?,
        last_snapshot_id: row.get(19)?,
        updated_at: row.get(20)?,
    })
}

// ─── Resource Aliases ──────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ResourceAliasRecord<'a> {
    pub alias_uri: &'a str,
    pub resource_id: &'a str,
    pub canonical_root_uri: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredResourceAlias {
    pub alias_uri: String,
    pub resource_id: String,
    pub canonical_root_uri: String,
    pub created_at: String,
}

pub(crate) fn stored_resource_alias_from_row(
    row: &rusqlite::Row<'_>,
) -> Result<StoredResourceAlias> {
    Ok(StoredResourceAlias {
        alias_uri: row.get(0)?,
        resource_id: row.get(1)?,
        canonical_root_uri: row.get(2)?,
        created_at: row.get(3)?,
    })
}

// ─── Resource Watches ──────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ResourceWatchRecord<'a> {
    pub account_id: &'a str,
    pub user_id: &'a str,
    pub agent_id: Option<&'a str>,
    pub resource_id: &'a str,
    pub interval_seconds: u32,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredResourceWatch {
    pub account_id: String,
    pub user_id: String,
    pub agent_id: Option<String>,
    pub resource_id: String,
    pub interval_seconds: u32,
    pub enabled: bool,
    pub last_checked_at: Option<String>,
    pub last_refreshed_at: Option<String>,
    pub updated_at: String,
}

// ─── Resource Change Events ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct ChangeEventRow {
    pub event_id: String,
    pub resource_id: String,
    pub account_id: String,
    pub user_id: String,
    pub uri: String,
    pub change_type: String,
    pub content_digest: Option<String>,
    pub snapshot_id: Option<String>,
    pub processed_at: Option<String>,
    pub created_at: String,
}

pub(crate) fn change_event_row_from_row(row: &rusqlite::Row<'_>) -> Result<ChangeEventRow> {
    Ok(ChangeEventRow {
        event_id: row.get(0)?,
        resource_id: row.get(1)?,
        account_id: row.get(2)?,
        user_id: row.get(3)?,
        uri: row.get(4)?,
        change_type: row.get(5)?,
        content_digest: row.get(6)?,
        snapshot_id: row.get(7)?,
        processed_at: row.get(8)?,
        created_at: row.get(9)?,
    })
}

// ─── Infra / Utility Domain ────────────────────────────────────────────
// Audit, webhooks, snapshots, relations, tasks, refresh scopes,
// and task pipeline types. Extracted from store.rs into infra_store.rs.

// ─── Audit ──────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct AuditEventRecord<'a> {
    pub account_id: &'a str,
    pub user_id: &'a str,
    pub agent_id: Option<&'a str>,
    pub projection_view_id: Option<&'a str>,
    pub event_type: &'a str,
    pub subject_uri: Option<&'a str>,
    pub actor: Option<&'a str>,
    pub details_json: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditRecord {
    pub id: i64,
    pub account_id: String,
    pub user_id: String,
    pub agent_id: Option<String>,
    pub projection_view_id: Option<String>,
    pub event_type: String,
    pub subject_uri: Option<String>,
    pub actor: Option<String>,
    pub details_json: Option<String>,
    pub recorded_at: String,
}

pub(crate) fn stored_audit_from_row(row: &rusqlite::Row<'_>) -> Result<AuditRecord> {
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
}

// ─── Webhooks ──────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct WebhookRecord<'a> {
    pub id: &'a str,
    pub account_id: &'a str,
    pub user_id: &'a str,
    pub agent_id: Option<&'a str>,
    pub event_type: &'a str,
    pub callback_url: &'a str,
    pub secret: &'a str,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredWebhook {
    pub id: String,
    pub account_id: String,
    pub user_id: String,
    pub agent_id: Option<String>,
    pub event_type: String,
    pub callback_url: String,
    pub enabled: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredWebhookWithSecret {
    pub id: String,
    pub account_id: String,
    pub user_id: String,
    pub agent_id: Option<String>,
    pub event_type: String,
    pub callback_url: String,
    pub secret: String,
    pub enabled: bool,
    pub created_at: String,
}

pub(crate) fn stored_webhook_from_row(row: &rusqlite::Row<'_>) -> Result<StoredWebhook> {
    Ok(StoredWebhook {
        id: row.get(0)?,
        account_id: row.get(1)?,
        user_id: row.get(2)?,
        agent_id: row.get(3)?,
        event_type: row.get(4)?,
        callback_url: row.get(5)?,
        enabled: row.get::<_, i64>(6)? != 0,
        created_at: row.get(7)?,
    })
}

pub(crate) fn stored_webhook_with_secret_from_row(
    row: &rusqlite::Row<'_>,
) -> Result<StoredWebhookWithSecret> {
    Ok(StoredWebhookWithSecret {
        id: row.get(0)?,
        account_id: row.get(1)?,
        user_id: row.get(2)?,
        agent_id: row.get(3)?,
        event_type: row.get(4)?,
        callback_url: row.get(5)?,
        secret: row.get(6)?,
        enabled: row.get::<_, i64>(7)? != 0,
        created_at: row.get(8)?,
    })
}

// ─── Snapshots ──────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct SnapshotRecord<'a> {
    pub snapshot_id: &'a str,
    pub account_id: &'a str,
    pub user_id: &'a str,
    pub agent_id: Option<&'a str>,
    pub projection_view_id: &'a str,
    pub root_uri: &'a str,
    pub manifest_digest: Option<&'a str>,
    pub created_by: Option<&'a str>,
    pub notes: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredSnapshot {
    pub snapshot_id: String,
    pub account_id: String,
    pub user_id: String,
    pub agent_id: Option<String>,
    pub projection_view_id: String,
    pub root_uri: String,
    pub manifest_digest: Option<String>,
    pub created_by: Option<String>,
    pub notes: Option<String>,
    pub created_at: String,
}

pub(crate) fn stored_snapshot_from_row(row: &rusqlite::Row<'_>) -> Result<StoredSnapshot> {
    Ok(StoredSnapshot {
        snapshot_id: row.get(0)?,
        account_id: row.get(1)?,
        user_id: row.get(2)?,
        agent_id: row.get(3)?,
        projection_view_id: row.get(4)?,
        root_uri: row.get(5)?,
        manifest_digest: row.get(6)?,
        created_by: row.get(7)?,
        notes: row.get(8)?,
        created_at: row.get(9)?,
    })
}

// ─── Relations ──────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct RelationRecord<'a> {
    pub account_id: &'a str,
    pub user_id: &'a str,
    pub agent_id: Option<&'a str>,
    pub from_uri: &'a str,
    pub to_uri: &'a str,
    pub relation_type: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredRelation {
    pub id: i64,
    pub account_id: String,
    pub user_id: String,
    pub agent_id: Option<String>,
    pub from_uri: String,
    pub to_uri: String,
    pub relation_type: String,
    pub updated_at: String,
    /// Temporal validity start (migration 0019).
    pub valid_from: Option<String>,
    /// Temporal validity end (NULL = open-ended / currently valid).
    pub valid_to: Option<String>,
    /// System commit time — when this edge version was stored.
    pub tcommit: Option<String>,
    /// Whether this is the latest version of this edge key.
    pub is_latest: i64,
    /// ID of the relation that superseded this one (same-table supersession pattern).
    pub superseded_by: Option<String>,
}

// ─── Tasks ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct TaskRecord<'a> {
    pub task_key: &'a str,
    pub account_id: &'a str,
    pub user_id: &'a str,
    pub agent_id: Option<&'a str>,
    pub projection_view_id: Option<&'a str>,
    pub state: &'a str,
    pub owner_space: Option<&'a str>,
    pub summary: Option<&'a str>,
    pub last_error: Option<&'a str>,
    pub attempt_count: u32,
    pub max_attempts: u32,
    pub retry_state: &'a str,
    pub processing_mode: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredTask {
    pub task_key: String,
    pub account_id: String,
    pub user_id: String,
    pub agent_id: Option<String>,
    pub projection_view_id: Option<String>,
    pub state: String,
    pub owner_space: Option<String>,
    pub summary: Option<String>,
    pub last_error: Option<String>,
    pub attempt_count: u32,
    pub max_attempts: u32,
    pub retry_state: String,
    pub processing_mode: Option<String>,
    pub scope_type: Option<String>,
    pub scope_id: Option<String>,
    pub range_start_turn_id: Option<String>,
    pub range_end_turn_id: Option<String>,
    pub dedupe_key: Option<String>,
    pub payload_json: Option<String>,
    pub lease_owner: Option<String>,
    pub lease_expires_at: Option<String>,
    pub scheduled_at: Option<String>,
    pub finished_at: Option<String>,
    pub updated_at: String,
}

pub(crate) fn stored_task_from_row(row: &rusqlite::Row<'_>) -> Result<StoredTask> {
    Ok(StoredTask {
        task_key: row.get(0)?,
        account_id: row.get(1)?,
        user_id: row.get(2)?,
        agent_id: row.get(3)?,
        projection_view_id: row.get(4)?,
        state: row.get(5)?,
        owner_space: row.get(6)?,
        summary: row.get(7)?,
        last_error: row.get(8)?,
        attempt_count: row.get::<_, i64>(9)? as u32,
        max_attempts: row.get::<_, i64>(10)? as u32,
        retry_state: row.get(11)?,
        processing_mode: row.get(12)?,
        scope_type: row.get(13)?,
        scope_id: row.get(14)?,
        range_start_turn_id: row.get(15)?,
        range_end_turn_id: row.get(16)?,
        dedupe_key: row.get(17)?,
        payload_json: row.get(18)?,
        lease_owner: row.get(19)?,
        lease_expires_at: row.get(20)?,
        scheduled_at: row.get(21)?,
        finished_at: row.get(22)?,
        updated_at: row.get(23)?,
    })
}
