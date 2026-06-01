CREATE TABLE IF NOT EXISTS webhooks (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    agent_id TEXT,
    event_type TEXT NOT NULL,
    callback_url TEXT NOT NULL,
    secret TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_webhooks_account_user_event_enabled
    ON webhooks(account_id, user_id, event_type, enabled);
