-- Migration 0006: Citation feedback loop
-- Adds recall_count and last_recalled_at to facts table for usage-based ranking.
-- Mirrors the existing episode_chunks recall tracking (OV-P1-6).

ALTER TABLE facts ADD COLUMN recall_count INTEGER NOT NULL DEFAULT 0;
ALTER TABLE facts ADD COLUMN last_recalled_at TEXT;
