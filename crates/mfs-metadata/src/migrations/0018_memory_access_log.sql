-- Migration 0018: memory_access_log table
-- Tracks per-memory access timestamps for Ebbinghaus spacing-effect computation.
-- Replaces simple recall_count with a richer history that enables the reinforcement
-- boost formula: spacing_factor = σ × Σ(1/daysSinceAccess_i).
-- Each row is one access event; pruning removes rows older than a configurable window.

CREATE TABLE IF NOT EXISTS memory_access_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    memory_id TEXT NOT NULL,
    memory_type TEXT NOT NULL,   -- 'episode' | 'fact' | 'heuristic'
    accessed_at TEXT NOT NULL,   -- ISO 8601 timestamp
    account_id TEXT NOT NULL,
    user_id TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_access_log_memory
    ON memory_access_log (memory_id, memory_type);

CREATE INDEX IF NOT EXISTS idx_access_log_time
    ON memory_access_log (accessed_at);

CREATE INDEX IF NOT EXISTS idx_access_log_identity
    ON memory_access_log (account_id, user_id);