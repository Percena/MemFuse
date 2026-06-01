-- FTS5 full-text search for facts and episodes (§10.2.1)
-- Enhances lexical fallback from naive token-matching to BM25-ranked search.

-- Facts FTS5 virtual table
CREATE VIRTUAL TABLE facts_fts USING fts5(
    display_value,
    predicate
);

-- Populate with existing active facts
INSERT INTO facts_fts(rowid, display_value, predicate)
    SELECT rowid, display_value, predicate FROM facts WHERE status = 'active';

-- Triggers to keep facts_fts in sync
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

-- Episodes FTS5 virtual table
CREATE VIRTUAL TABLE episodes_fts USING fts5(
    summary,
    keywords
);

-- Populate with existing episodes
INSERT INTO episodes_fts(rowid, summary, keywords)
    SELECT rowid, summary, COALESCE(keywords_json, '') FROM episode_chunks WHERE archived_at IS NULL;

-- Triggers to keep episodes_fts in sync
CREATE TRIGGER episodes_fts_insert AFTER INSERT ON episode_chunks BEGIN
    INSERT INTO episodes_fts(rowid, summary, keywords)
        VALUES (new.rowid, new.summary, COALESCE(new.keywords_json, ''));
END;

CREATE TRIGGER episodes_fts_delete AFTER DELETE ON episode_chunks BEGIN
    INSERT INTO episodes_fts(episodes_fts, rowid, summary, keywords)
        VALUES ('delete', old.rowid, old.summary, COALESCE(old.keywords_json, ''));
END;

CREATE TRIGGER episodes_fts_update AFTER UPDATE ON episode_chunks BEGIN
    INSERT INTO episodes_fts(episodes_fts, rowid, summary, keywords)
        VALUES ('delete', old.rowid, old.summary, COALESCE(old.keywords_json, ''));
    INSERT INTO episodes_fts(rowid, summary, keywords)
        VALUES (new.rowid, new.summary, COALESCE(new.keywords_json, ''));
END;
