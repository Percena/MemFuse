PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS path_entries (
    id INTEGER PRIMARY KEY,
    account_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    agent_id TEXT,
    projection_view_id TEXT NOT NULL,
    canonical_uri TEXT NOT NULL,
    workspace_path TEXT NOT NULL,
    entry_kind TEXT NOT NULL,
    source_kind TEXT,
    source_identifier TEXT,
    source_snapshot_id TEXT,
    content_kind TEXT,
    language TEXT,
    relative_resource_path TEXT,
    repo_root_uri TEXT,
    is_text INTEGER,
    is_generated INTEGER,
    content_digest TEXT,
    metadata_digest TEXT,
    size_bytes INTEGER,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS audit_log (
    id INTEGER PRIMARY KEY,
    account_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    agent_id TEXT,
    projection_view_id TEXT,
    event_type TEXT NOT NULL,
    subject_uri TEXT,
    actor TEXT,
    details_json TEXT,
    recorded_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS snapshots (
    snapshot_id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    agent_id TEXT,
    projection_view_id TEXT NOT NULL,
    root_uri TEXT NOT NULL,
    manifest_digest TEXT,
    created_by TEXT,
    notes TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS resource_sources (
    resource_id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    agent_id TEXT,
    logical_name TEXT NOT NULL,
    source_kind TEXT NOT NULL,
    source_identifier TEXT NOT NULL,
    canonical_root_uri TEXT NOT NULL,
    projection_view_id TEXT NOT NULL,
    resource_kind TEXT NOT NULL DEFAULT 'generic_docs',
    source_host TEXT,
    source_namespace TEXT,
    source_repo TEXT,
    source_ref TEXT,
    canonical_strategy_version TEXT NOT NULL DEFAULT 'v2',
    status TEXT NOT NULL,
    last_snapshot_id TEXT,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS resource_aliases (
    alias_uri TEXT PRIMARY KEY,
    resource_id TEXT NOT NULL REFERENCES resource_sources(resource_id),
    canonical_root_uri TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS tasks (
    id INTEGER PRIMARY KEY,
    task_key TEXT NOT NULL UNIQUE,
    account_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    agent_id TEXT,
    projection_view_id TEXT,
    state TEXT NOT NULL,
    owner_space TEXT,
    summary TEXT,
    last_error TEXT,
    attempt_count INTEGER NOT NULL DEFAULT 0,
    max_attempts INTEGER NOT NULL DEFAULT 1,
    retry_state TEXT NOT NULL DEFAULT 'not_needed',
    processing_mode TEXT,
    scope_type TEXT,
    scope_id TEXT,
    range_start_turn_id TEXT,
    range_end_turn_id TEXT,
    dedupe_key TEXT,
    payload_json TEXT,
    lease_owner TEXT,
    lease_expires_at TEXT,
    scheduled_at TEXT,
    finished_at TEXT,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS relations (
    id INTEGER PRIMARY KEY,
    account_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    agent_id TEXT,
    from_uri TEXT NOT NULL,
    to_uri TEXT NOT NULL,
    relation_type TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS resource_watches (
    id INTEGER PRIMARY KEY,
    account_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    agent_id TEXT,
    resource_id TEXT NOT NULL UNIQUE,
    interval_seconds INTEGER NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    last_checked_at TEXT,
    last_refreshed_at TEXT,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS digest_cache (
    cache_key TEXT NOT NULL,
    account_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    projection_view_id TEXT,
    digest TEXT NOT NULL,
    algorithm TEXT NOT NULL DEFAULT 'sha256',
    size_bytes INTEGER,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_path_entries_workspace_path
    ON path_entries(workspace_path);

CREATE INDEX IF NOT EXISTS idx_path_entries_identity
    ON path_entries(account_id, user_id, projection_view_id);

CREATE UNIQUE INDEX IF NOT EXISTS idx_path_entries_scoped_uri
    ON path_entries(account_id, user_id, projection_view_id, canonical_uri);

CREATE INDEX IF NOT EXISTS idx_audit_log_subject_uri
    ON audit_log(subject_uri);

CREATE INDEX IF NOT EXISTS idx_audit_log_identity
    ON audit_log(account_id, user_id, projection_view_id);

CREATE INDEX IF NOT EXISTS idx_tasks_state
    ON tasks(state);

CREATE INDEX IF NOT EXISTS idx_tasks_identity
    ON tasks(account_id, user_id, projection_view_id);

CREATE UNIQUE INDEX IF NOT EXISTS idx_tasks_dedupe
    ON tasks(dedupe_key) WHERE dedupe_key IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_tasks_scope_status
    ON tasks(scope_type, scope_id, state) WHERE scope_type IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_tasks_lease
    ON tasks(lease_owner, lease_expires_at) WHERE lease_owner IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_relations_identity
    ON relations(account_id, user_id, relation_type);

CREATE INDEX IF NOT EXISTS idx_relations_uri
    ON relations(account_id, user_id, from_uri, to_uri);

CREATE UNIQUE INDEX IF NOT EXISTS idx_relations_unique
    ON relations(account_id, user_id, from_uri, to_uri, relation_type);

CREATE INDEX IF NOT EXISTS idx_resource_watches_identity
    ON resource_watches(account_id, user_id, enabled);

CREATE UNIQUE INDEX IF NOT EXISTS idx_resource_sources_scoped_name
    ON resource_sources(account_id, user_id, projection_view_id, logical_name);

CREATE UNIQUE INDEX IF NOT EXISTS idx_resource_sources_scoped_uri
    ON resource_sources(account_id, user_id, projection_view_id, canonical_root_uri);

CREATE INDEX IF NOT EXISTS idx_resource_sources_identity
    ON resource_sources(account_id, user_id, projection_view_id);

CREATE INDEX IF NOT EXISTS idx_resource_aliases_resource
    ON resource_aliases(resource_id);

CREATE UNIQUE INDEX IF NOT EXISTS idx_digest_cache_scoped_key
    ON digest_cache(projection_view_id, cache_key);

CREATE TABLE IF NOT EXISTS facts (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    agent_id TEXT NOT NULL DEFAULT 'coding-agent',
    subject TEXT NOT NULL,
    predicate TEXT NOT NULL,
    display_value TEXT NOT NULL,
    normalized_value_json TEXT,
    value_type TEXT NOT NULL DEFAULT 'scalar',
    confidence REAL NOT NULL DEFAULT 0.0,
    status TEXT NOT NULL DEFAULT 'active',
    valid_from TEXT,
    valid_to TEXT,
    source_assertion_id TEXT,
    source_episode_id TEXT,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    superseded_at TEXT,
    superseded_by TEXT
);

CREATE INDEX IF NOT EXISTS idx_facts_user_pred_status
    ON facts(account_id, user_id, predicate, status, confidence DESC);

CREATE INDEX IF NOT EXISTS idx_facts_user_active
    ON facts(account_id, user_id, status, confidence DESC);

CREATE INDEX IF NOT EXISTS idx_facts_source_episode
    ON facts(source_episode_id);

CREATE TABLE IF NOT EXISTS code_symbols (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    agent_id TEXT NOT NULL DEFAULT 'coding-agent',
    projection_view_id TEXT NOT NULL,
    canonical_uri TEXT NOT NULL,
    symbol_type TEXT NOT NULL,
    symbol_name TEXT NOT NULL,
    signature TEXT,
    docstring TEXT,
    line_number INTEGER,
    embedding_json TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_code_symbols_name
    ON code_symbols(symbol_name, symbol_type);

CREATE INDEX IF NOT EXISTS idx_code_symbols_uri
    ON code_symbols(projection_view_id, canonical_uri);

CREATE INDEX IF NOT EXISTS idx_code_symbols_identity
    ON code_symbols(account_id, user_id, projection_view_id);

-- MemFuse memory pipeline tables

CREATE TABLE IF NOT EXISTS conversation_sessions (
    session_id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    agent_id TEXT NOT NULL DEFAULT 'coding-agent',
    status TEXT NOT NULL DEFAULT 'active',
    started_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    last_activity_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    metadata_json TEXT
);

CREATE INDEX IF NOT EXISTS idx_sessions_user
    ON conversation_sessions(account_id, user_id, status);

CREATE TABLE IF NOT EXISTS conversation_turns (
    turn_id TEXT PRIMARY KEY,
    turn_seq INTEGER NOT NULL,
    session_id TEXT NOT NULL REFERENCES conversation_sessions(session_id),
    account_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    agent_id TEXT NOT NULL DEFAULT 'coding-agent',
    role TEXT NOT NULL,
    content_text TEXT NOT NULL DEFAULT '',
    content_json TEXT,
    token_count INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    ingested_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_turns_session_created
    ON conversation_turns(session_id, created_at);
CREATE INDEX IF NOT EXISTS idx_turns_session_seq
    ON conversation_turns(session_id, turn_seq);
CREATE UNIQUE INDEX IF NOT EXISTS idx_turns_seq
    ON conversation_turns(session_id, turn_seq);

CREATE TABLE IF NOT EXISTS episode_chunks (
    episode_id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    agent_id TEXT NOT NULL DEFAULT 'coding-agent',
    session_id TEXT NOT NULL REFERENCES conversation_sessions(session_id),
    resource_id TEXT,
    summary TEXT NOT NULL DEFAULT '',
    detail_ref TEXT,
    keywords_json TEXT,
    salience_score REAL NOT NULL DEFAULT 0.5,
    strength_score REAL NOT NULL DEFAULT 1.0,
    emotional_valence REAL,
    emotional_intensity REAL,
    context_tags_json TEXT,
    recall_count INTEGER NOT NULL DEFAULT 0,
    last_recalled_at TEXT,
    source_start_turn_id TEXT,
    source_end_turn_id TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    archived_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_episodes_user_session
    ON episode_chunks(account_id, user_id, session_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_episodes_session_range
    ON episode_chunks(session_id, source_start_turn_id, source_end_turn_id);
CREATE INDEX IF NOT EXISTS idx_episodes_user_resource
    ON episode_chunks(account_id, user_id, resource_id) WHERE resource_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_episodes_salience
    ON episode_chunks(account_id, user_id, salience_score DESC) WHERE archived_at IS NULL;
CREATE INDEX IF NOT EXISTS idx_episodes_last_recalled
    ON episode_chunks(last_recalled_at) WHERE last_recalled_at IS NOT NULL;

CREATE TABLE IF NOT EXISTS fact_assertions (
    assertion_id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    agent_id TEXT NOT NULL DEFAULT 'coding-agent',
    subject TEXT NOT NULL,
    predicate TEXT NOT NULL,
    raw_value_text TEXT NOT NULL,
    normalized_value_json TEXT,
    value_type TEXT NOT NULL DEFAULT 'scalar',
    operation TEXT NOT NULL DEFAULT 'assert',
    confidence REAL NOT NULL DEFAULT 0.8,
    valid_from TEXT,
    valid_to TEXT,
    source_turn_id TEXT,
    source_episode_id TEXT,
    source_resource_id TEXT,
    source_snapshot_id TEXT,
    source_uri TEXT,
    extractor_version TEXT NOT NULL DEFAULT 'v1',
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_assertions_user_pred
    ON fact_assertions(account_id, user_id, predicate);
CREATE INDEX IF NOT EXISTS idx_assertions_source_pred_version
    ON fact_assertions(source_turn_id, predicate, extractor_version)
    WHERE operation IN ('assert', 'update');
CREATE INDEX IF NOT EXISTS idx_assertions_source_resource
    ON fact_assertions(source_resource_id) WHERE source_resource_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS memory_consolidation_cursors (
    cursor_id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    scope_type TEXT NOT NULL DEFAULT 'thread',
    scope_id TEXT NOT NULL,
    last_consolidated_turn_id TEXT,
    last_consolidated_at TEXT,
    dedupe_key TEXT,
    lease_owner TEXT,
    lease_expires_at TEXT,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_cursors_user_scope
    ON memory_consolidation_cursors(account_id, user_id, scope_type, scope_id);

CREATE TABLE IF NOT EXISTS memory_briefs (
    brief_id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    scope_type TEXT NOT NULL DEFAULT 'thread',
    scope_id TEXT NOT NULL,
    summary TEXT NOT NULL DEFAULT '',
    source_thread_ids_json TEXT,
    anchor_episode_ids_json TEXT,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_memory_briefs_scope
    ON memory_briefs(account_id, user_id, scope_type, scope_id);

CREATE TABLE IF NOT EXISTS resource_change_events (
    event_id TEXT PRIMARY KEY,
    resource_id TEXT NOT NULL REFERENCES resource_sources(resource_id),
    account_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    uri TEXT NOT NULL,
    change_type TEXT NOT NULL DEFAULT 'modified',
    content_digest TEXT,
    snapshot_id TEXT REFERENCES snapshots(snapshot_id),
    processed_at TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_change_events_resource_created
    ON resource_change_events(resource_id, created_at);
CREATE INDEX IF NOT EXISTS idx_change_events_resource_processed
    ON resource_change_events(resource_id, processed_at) WHERE processed_at IS NOT NULL;
