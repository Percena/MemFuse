-- Migration 0020: run_writebacks table
-- Stores agent run evidence and writeback payloads.
-- This enables the POST /runs/writeback endpoint and allows
-- subsequent ticket runs to inherit previous execution results.

CREATE TABLE IF NOT EXISTS run_writebacks (
    repo_id TEXT NOT NULL,
    run_id TEXT NOT NULL,
    tracker TEXT NOT NULL DEFAULT 'github_projects',
    tracker_identifier TEXT NOT NULL DEFAULT '',
    idempotency_key TEXT NOT NULL DEFAULT '',
    payload_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (repo_id, run_id)
);

-- Index for ticket-history queries by tracker_identifier
CREATE INDEX IF NOT EXISTS idx_run_writebacks_tracker
    ON run_writebacks (repo_id, tracker_identifier);