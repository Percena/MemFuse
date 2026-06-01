-- Heuristic rules table (Hybrid representation: tags + natural language + counter_examples)
CREATE TABLE IF NOT EXISTS heuristic_rules (
    rule_id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    agent_id TEXT,

    -- Structured tag layer (flat key:value set; JSON array for MVP)
    tags_json TEXT NOT NULL DEFAULT '[]',

    -- Natural language layer (for semantic ranking and human readability)
    rule_text TEXT NOT NULL,
    counter_examples_json TEXT NOT NULL DEFAULT '[]',

    -- Lifecycle (applies only to rules)
    lifecycle_stage TEXT NOT NULL DEFAULT 'draft',
    -- draft | candidate | confirmed | archived

    -- Statistics
    evidence_count INTEGER NOT NULL DEFAULT 0,
    aggregate_weight REAL NOT NULL DEFAULT 0.0,
    last_evidence_at TEXT,

    -- Metadata
    source_instance_ids_json TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    promoted_at TEXT,
    archived_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_heuristic_rules_user_stage
    ON heuristic_rules(account_id, user_id, lifecycle_stage);

CREATE INDEX IF NOT EXISTS idx_heuristic_rules_user_active
    ON heuristic_rules(account_id, user_id, lifecycle_stage, aggregate_weight DESC)
    WHERE lifecycle_stage IN ('draft', 'candidate', 'confirmed');

-- Heuristic instances table (rule precursor, few-shot injection source)
CREATE TABLE IF NOT EXISTS heuristic_instances (
    instance_id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    agent_id TEXT,

    -- Interaction context
    context_summary TEXT NOT NULL,
    agent_proposal TEXT,
    user_reaction TEXT NOT NULL,
    outcome TEXT,
    signal_type TEXT NOT NULL,
    -- explicit_negation | implicit_negation | preference_declaration | tradeoff_decision

    -- Structured tags (shared tag space with rules)
    tags_json TEXT NOT NULL DEFAULT '[]',

    -- Associations
    session_id TEXT,
    source_turn_ids_json TEXT,
    derived_rule_id TEXT REFERENCES heuristic_rules(rule_id),
    instance_status TEXT NOT NULL DEFAULT 'open',
    -- open | promoted | dismissed | expired

    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    resolved_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_heuristic_instances_user
    ON heuristic_instances(account_id, user_id, signal_type);

CREATE INDEX IF NOT EXISTS idx_heuristic_instances_status
    ON heuristic_instances(account_id, user_id, instance_status);

-- Heuristic evidence table (Event Sourcing, append-only)
CREATE TABLE IF NOT EXISTS heuristic_evidence (
    evidence_id TEXT PRIMARY KEY,
    rule_id TEXT NOT NULL REFERENCES heuristic_rules(rule_id),
    instance_id TEXT REFERENCES heuristic_instances(instance_id),

    -- Evidence attributes
    evidence_type TEXT NOT NULL DEFAULT 'support',
    -- support | contradict | positive_confirm
    support_weight REAL NOT NULL DEFAULT 1.0,
    session_id TEXT NOT NULL,

    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_heuristic_evidence_rule
    ON heuristic_evidence(rule_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_heuristic_evidence_session
    ON heuristic_evidence(session_id);