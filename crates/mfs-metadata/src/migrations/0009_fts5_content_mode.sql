-- Fix FTS5 tables: use content= mode so triggers can safely write to FTS5
-- SQLite blocks "unsafe use of virtual table" when writing to regular FTS5
-- tables from within triggers on base tables. content= mode is the proper fix.

-- ── Facts FTS5 ──────────────────────────────────────────────────────────

-- Drop old triggers
DROP TRIGGER IF EXISTS facts_fts_insert;
DROP TRIGGER IF EXISTS facts_fts_delete;
DROP TRIGGER IF EXISTS facts_fts_update;

-- Drop old FTS5 table
DROP TABLE IF EXISTS facts_fts;

-- Re-create with content= mode (reads content from facts table)
CREATE VIRTUAL TABLE facts_fts USING fts5(
    display_value,
    predicate,
    content=facts,
    content_rowid=rowid
);

-- Populate index for existing active facts
INSERT INTO facts_fts(rowid, display_value, predicate)
    SELECT rowid, display_value, predicate FROM facts WHERE status = 'active';

-- Triggers to keep facts_fts index in sync (now safe with content= mode)
CREATE TRIGGER facts_fts_insert AFTER INSERT ON facts BEGIN
    INSERT INTO facts_fts(rowid, display_value, predicate)
        VALUES (new.rowid, new.display_value, new.predicate);
END;

CREATE TRIGGER facts_fts_delete AFTER DELETE ON facts BEGIN
    INSERT INTO facts_fts(facts_fts, rowid, display_value, predicate)
        VALUES ('delete', old.rowid, old.display_value, old.predicate);
END;

CREATE TRIGGER facts_fts_update AFTER UPDATE ON facts BEGIN
    INSERT INTO facts_fts(facts_fts, rowid, display_value, predicate)
        VALUES ('delete', old.rowid, old.display_value, old.predicate);
    INSERT INTO facts_fts(rowid, display_value, predicate)
        VALUES (new.rowid, new.display_value, new.predicate);
END;

-- ── Episodes FTS5 ───────────────────────────────────────────────────────

-- Drop old triggers
DROP TRIGGER IF EXISTS episodes_fts_insert;
DROP TRIGGER IF EXISTS episodes_fts_delete;
DROP TRIGGER IF EXISTS episodes_fts_update;

-- Drop old FTS5 table
DROP TABLE IF EXISTS episodes_fts;

-- Re-create with content= mode (reads content from episode_chunks table)
-- Note: column name changed from "keywords" to "keywords_json" to match
-- the content table column name (FTS5 content= requires exact name match)
CREATE VIRTUAL TABLE episodes_fts USING fts5(
    summary,
    keywords_json,
    content=episode_chunks,
    content_rowid=rowid
);

-- Populate index for existing non-archived episodes
INSERT INTO episodes_fts(rowid, summary, keywords_json)
    SELECT rowid, summary, keywords_json FROM episode_chunks WHERE archived_at IS NULL;

-- Triggers to keep episodes_fts index in sync (now safe with content= mode)
CREATE TRIGGER episodes_fts_insert AFTER INSERT ON episode_chunks BEGIN
    INSERT INTO episodes_fts(rowid, summary, keywords_json)
        VALUES (new.rowid, new.summary, COALESCE(new.keywords_json, ''));
END;

CREATE TRIGGER episodes_fts_delete AFTER DELETE ON episode_chunks BEGIN
    INSERT INTO episodes_fts(episodes_fts, rowid, summary, keywords_json)
        VALUES ('delete', old.rowid, old.summary, COALESCE(old.keywords_json, ''));
END;

CREATE TRIGGER episodes_fts_update AFTER UPDATE ON episode_chunks BEGIN
    INSERT INTO episodes_fts(episodes_fts, rowid, summary, keywords_json)
        VALUES ('delete', old.rowid, old.summary, COALESCE(old.keywords_json, ''));
    INSERT INTO episodes_fts(rowid, summary, keywords_json)
        VALUES (new.rowid, new.summary, COALESCE(new.keywords_json, ''));
END;