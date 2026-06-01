use std::path::Path;
use std::sync::Mutex;

use rusqlite::types::Value;
use rusqlite::{Connection, params, params_from_iter};

use crate::{
    IndexError, SearchHit, build_str_in_clause, build_u8_in_clause, normalize_target_scope,
    sanitize_fts_query,
};

/// Levels 0-1 are "high-level" (abstracts, overviews) — low update frequency,
/// suitable for future cloud sync.
/// Level 2+ is "detail" (full content, AST data) — high update frequency, local-only.
pub const HIGH_LEVEL_MAX: u8 = 1;

struct TablePair {
    main: &'static str,
    fts: &'static str,
}

const HIGH: TablePair = TablePair {
    main: "semantic_docs_high",
    fts: "semantic_docs_high_fts",
};

const DETAIL: TablePair = TablePair {
    main: "semantic_docs_detail",
    fts: "semantic_docs_detail_fts",
};

const ALL_PAIRS: [&TablePair; 2] = [&HIGH, &DETAIL];

fn is_high_level(level: u8) -> bool {
    level <= HIGH_LEVEL_MAX
}

fn pair_for_level(level: u8) -> &'static TablePair {
    if is_high_level(level) { &HIGH } else { &DETAIL }
}

fn pairs_needed(levels: Option<&[u8]>) -> Vec<&'static TablePair> {
    match levels {
        None | Some(&[]) => ALL_PAIRS.to_vec(),
        Some(lvls) => {
            let mut result = Vec::with_capacity(2);
            if lvls.iter().any(|&l| is_high_level(l)) {
                result.push(&HIGH);
            }
            if lvls.iter().any(|&l| !is_high_level(l)) {
                result.push(&DETAIL);
            }
            result
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SemanticDocument {
    pub projection_view_id: String,
    pub uri: String,
    pub context_type: String,
    pub resource_id: Option<String>,
    pub content_kind: Option<String>,
    pub language: Option<String>,
    pub level: u8,
    pub title: String,
    pub body: String,
    pub embedding: Vec<f32>,
}

pub struct SqliteSemanticIndex {
    conn: Mutex<Connection>,
}

impl SqliteSemanticIndex {
    pub fn open_in_memory() -> Result<Self, IndexError> {
        let conn = Connection::open_in_memory()?;
        bootstrap(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn open_at(path: impl AsRef<Path>) -> Result<Self, IndexError> {
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent)
                .map_err(|source| rusqlite::Error::ToSqlConversionFailure(source.into()))?;
        }
        let conn = Connection::open(path)?;
        bootstrap(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn upsert_document(&self, document: &SemanticDocument) -> Result<(), IndexError> {
        let conn = self.conn.lock().unwrap();
        let embedding = serde_json::to_string(&document.embedding)
            .map_err(|source| rusqlite::Error::ToSqlConversionFailure(source.into()))?;
        let document_id = document_identity(document);
        let pair = pair_for_level(document.level);

        conn.execute_batch("SAVEPOINT upsert_doc")?;
        let result = (|| {
            conn.execute(
                &format!(
                    "INSERT INTO {} (
                        document_id,
                        projection_view_id,
                        canonical_uri,
                        context_type,
                        resource_id,
                        content_kind,
                        language,
                        level,
                        title,
                        body,
                        embedding_json
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                     ON CONFLICT(projection_view_id, canonical_uri, context_type, level)
                     DO UPDATE SET
                        document_id = excluded.document_id,
                        context_type = excluded.context_type,
                        resource_id = excluded.resource_id,
                        content_kind = excluded.content_kind,
                        language = excluded.language,
                        level = excluded.level,
                        title = excluded.title,
                        body = excluded.body,
                        embedding_json = excluded.embedding_json,
                        updated_at = CURRENT_TIMESTAMP",
                    pair.main,
                ),
                params![
                    document_id,
                    document.projection_view_id,
                    document.uri,
                    document.context_type,
                    document.resource_id,
                    document.content_kind,
                    document.language,
                    i64::from(document.level),
                    document.title,
                    document.body,
                    embedding,
                ],
            )?;
            conn.execute(
                &format!("DELETE FROM {} WHERE document_id = ?1", pair.fts),
                params![document_id],
            )?;
            conn.execute(
                &format!(
                    "INSERT INTO {} (
                        document_id,
                        projection_view_id,
                        canonical_uri,
                        context_type,
                        resource_id,
                        content_kind,
                        language,
                        level,
                        title,
                        body
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    pair.fts,
                ),
                params![
                    document_id,
                    document.projection_view_id,
                    document.uri,
                    document.context_type,
                    document.resource_id,
                    document.content_kind,
                    document.language,
                    i64::from(document.level),
                    document.title,
                    document.body,
                ],
            )?;
            Ok(())
        })();
        if result.is_ok() {
            conn.execute_batch("RELEASE upsert_doc")?;
        } else {
            conn.execute_batch("ROLLBACK TO upsert_doc")?;
            conn.execute_batch("RELEASE upsert_doc")?;
        }
        result
    }

    pub fn embedding_dimension(&self) -> Result<Option<usize>, IndexError> {
        let conn = self.conn.lock().unwrap();
        for pair in ALL_PAIRS {
            let mut stmt =
                conn.prepare(&format!("SELECT embedding_json FROM {} LIMIT 1", pair.main))?;
            let mut rows = stmt.query([])?;
            if let Some(row) = rows.next()? {
                let embedding_json: String = row.get(0)?;
                let embedding: Vec<f32> =
                    serde_json::from_str(&embedding_json).map_err(to_sqlite_json_error)?;
                return Ok(Some(embedding.len()));
            }
        }
        Ok(None)
    }

    pub fn count_documents(&self) -> Result<usize, IndexError> {
        let conn = self.conn.lock().unwrap();
        let mut total: usize = 0;
        for pair in ALL_PAIRS {
            total += conn.query_row(&format!("SELECT COUNT(*) FROM {}", pair.main), [], |row| {
                row.get::<_, usize>(0)
            })?;
        }
        Ok(total)
    }

    pub fn count_documents_by_tier(&self) -> Result<(usize, usize), IndexError> {
        let conn = self.conn.lock().unwrap();
        let high = conn.query_row(&format!("SELECT COUNT(*) FROM {}", HIGH.main), [], |row| {
            row.get::<_, usize>(0)
        })?;
        let detail = conn.query_row(
            &format!("SELECT COUNT(*) FROM {}", DETAIL.main),
            [],
            |row| row.get::<_, usize>(0),
        )?;
        Ok((high, detail))
    }

    pub fn count_documents_by_context_type(&self, context_type: &str) -> Result<usize, IndexError> {
        let conn = self.conn.lock().unwrap();
        let mut total: usize = 0;
        for pair in ALL_PAIRS {
            total += conn.query_row(
                &format!("SELECT COUNT(*) FROM {} WHERE context_type = ?1", pair.main),
                params![context_type],
                |row| row.get::<_, usize>(0),
            )?;
        }
        Ok(total)
    }

    pub fn delete_prefix(&self, target_prefix: Option<&str>) -> Result<usize, IndexError> {
        self.delete_prefix_in_projection(None, target_prefix)
    }

    pub fn delete_prefix_in_projection(
        &self,
        projection_view_id: Option<&str>,
        target_prefix: Option<&str>,
    ) -> Result<usize, IndexError> {
        let conn = self.conn.lock().unwrap();
        let projection = projection_view_id.unwrap_or("");
        let target = target_prefix.unwrap_or("").trim_end_matches('/');
        let mut total_deleted = 0;

        for pair in ALL_PAIRS {
            if projection.is_empty() && target.is_empty() {
                total_deleted += conn.execute(&format!("DELETE FROM {}", pair.main), [])?;
                conn.execute(&format!("DELETE FROM {}", pair.fts), [])?;
            } else {
                let slash_prefix = format!("{target}/");
                total_deleted += conn.execute(
                    &format!(
                        "DELETE FROM {}
                         WHERE (?1 = '' OR projection_view_id = ?1)
                           AND (?2 = '' OR canonical_uri = ?2
                                OR substr(canonical_uri, 1, length(?3)) = ?3)",
                        pair.main,
                    ),
                    params![projection, target, slash_prefix],
                )?;
                conn.execute(
                    &format!(
                        "DELETE FROM {}
                         WHERE (?1 = '' OR projection_view_id = ?1)
                           AND (?2 = '' OR canonical_uri = ?2
                                OR substr(canonical_uri, 1, length(?3)) = ?3)",
                        pair.fts,
                    ),
                    params![projection, target, slash_prefix],
                )?;
            }
        }
        Ok(total_deleted)
    }

    pub fn search_lexical(
        &self,
        query: &str,
        projection_view_ids: Option<&[&str]>,
        target_prefix: Option<&str>,
        levels: Option<&[u8]>,
        context_types: Option<&[&str]>,
        limit: usize,
    ) -> Result<Vec<SearchHit>, IndexError> {
        let query = sanitize_fts_query(query);
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.conn.lock().unwrap();
        let target = normalize_target_scope(target_prefix);

        let mut all_hits = Vec::new();
        for pair in pairs_needed(levels) {
            let mut sql_params: Vec<Value> = vec![
                Value::Text(query.clone()),
                Value::Text(target.to_string()),
                Value::Integer(limit as i64),
            ];
            let projection_clause =
                build_str_in_clause("projection_view_id", projection_view_ids, &mut sql_params);
            let level_clause = build_u8_in_clause("level", levels);
            let context_type_clause =
                build_str_in_clause("context_type", context_types, &mut sql_params);

            let sql = format!(
                "SELECT canonical_uri,
                        context_type,
                        level,
                        bm25({fts}) AS rank,
                        snippet({fts}, 9, '', '', '…', 8)
                 FROM {fts}
                 WHERE {fts} MATCH ?1
                   AND (?2 = '' OR canonical_uri = ?2 OR substr(canonical_uri, 1, length(?2) + 1) = ?2 || '/')
                 {projection_clause}
                 {level_clause}
                 {context_type_clause}
                 ORDER BY rank
                 LIMIT ?3",
                fts = pair.fts,
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(params_from_iter(sql_params), |row| {
                Ok(SearchHit {
                    uri: row.get(0)?,
                    context_type: row.get(1)?,
                    level: row.get::<_, i64>(2)? as u8,
                    score: row.get(3)?,
                    excerpt: row.get(4)?,
                })
            })?;
            for row in rows {
                all_hits.push(row?);
            }
        }

        all_hits.sort_by(|a, b| a.score.total_cmp(&b.score).then_with(|| a.uri.cmp(&b.uri)));
        all_hits.truncate(limit);
        Ok(all_hits)
    }

    pub fn semantic_search(
        &self,
        query_embedding: &[f32],
        query_hint: &str,
        projection_view_ids: Option<&[&str]>,
        target_prefix: Option<&str>,
        levels: Option<&[u8]>,
        context_types: Option<&[&str]>,
        limit: usize,
    ) -> Result<Vec<SearchHit>, IndexError> {
        let conn = self.conn.lock().unwrap();
        let target = normalize_target_scope(target_prefix);

        let mut all_hits = Vec::new();
        for pair in pairs_needed(levels) {
            let mut sql_params: Vec<Value> = vec![Value::Text(target.to_string())];
            let projection_clause =
                build_str_in_clause("projection_view_id", projection_view_ids, &mut sql_params);
            let level_clause = build_u8_in_clause("level", levels);
            let context_type_clause =
                build_str_in_clause("context_type", context_types, &mut sql_params);

            let sql = format!(
                "SELECT canonical_uri,
                        context_type,
                        level,
                        body,
                        embedding_json
                 FROM {main}
                 WHERE (?1 = '' OR canonical_uri = ?1 OR substr(canonical_uri, 1, length(?1) + 1) = ?1 || '/')
                 {projection_clause}
                 {level_clause}
                 {context_type_clause}",
                main = pair.main,
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(params_from_iter(sql_params), |row| {
                let body: String = row.get(3)?;
                let embedding_json: String = row.get(4)?;
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)? as u8,
                    body,
                    embedding_json,
                ))
            })?;

            for row in rows {
                let (uri, context_type, level, body, embedding_json) = row?;
                let embedding: Vec<f32> =
                    serde_json::from_str(&embedding_json).map_err(to_sqlite_json_error)?;
                let score = cosine_similarity(query_embedding, &embedding);
                if !score.is_finite() {
                    continue;
                }
                all_hits.push(SearchHit {
                    excerpt: excerpt_for_query(&body, query_hint),
                    uri,
                    context_type,
                    level,
                    score,
                });
            }
        }

        all_hits.sort_by(|left, right| {
            right
                .score
                .total_cmp(&left.score)
                .then_with(|| left.uri.cmp(&right.uri))
        });
        all_hits.truncate(limit);
        Ok(all_hits)
    }
}

// ─── Schema bootstrap & migration ──────────────────────────────────────────

fn bootstrap(conn: &Connection) -> Result<(), IndexError> {
    let old_exists = table_exists(conn, "semantic_documents");
    let new_exists = table_exists(conn, "semantic_docs_high");

    if new_exists && new_schema_requires_reset(conn)? {
        drop_table_pair(conn, &HIGH)?;
        drop_table_pair(conn, &DETAIL)?;
        create_table_pair(conn, &HIGH)?;
        create_table_pair(conn, &DETAIL)?;
    } else if new_exists {
        // Already up to date.
    } else {
        create_table_pair(conn, &HIGH)?;
        create_table_pair(conn, &DETAIL)?;
        if old_exists {
            migrate_old_to_new(conn)?;
        }
    }

    if old_exists {
        conn.execute_batch(
            "DROP TABLE IF EXISTS semantic_documents_fts;
             DROP TABLE IF EXISTS semantic_documents;",
        )?;
    }

    Ok(())
}

fn table_exists(conn: &Connection, table: &str) -> bool {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1",
        params![table],
        |_| Ok(()),
    )
    .is_ok()
}

fn create_table_pair(conn: &Connection, pair: &TablePair) -> Result<(), IndexError> {
    conn.execute_batch(&format!(
        "
        CREATE TABLE IF NOT EXISTS {main} (
            document_id TEXT PRIMARY KEY,
            projection_view_id TEXT NOT NULL,
            canonical_uri TEXT NOT NULL,
            context_type TEXT NOT NULL,
            resource_id TEXT,
            content_kind TEXT,
            language TEXT,
            level INTEGER NOT NULL,
            title TEXT NOT NULL,
            body TEXT NOT NULL,
            embedding_json TEXT NOT NULL,
            updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );

        CREATE UNIQUE INDEX IF NOT EXISTS idx_{main}_identity
            ON {main}(projection_view_id, canonical_uri, context_type, level);

        CREATE VIRTUAL TABLE IF NOT EXISTS {fts} USING fts5(
            document_id UNINDEXED,
            projection_view_id UNINDEXED,
            canonical_uri UNINDEXED,
            context_type UNINDEXED,
            resource_id UNINDEXED,
            content_kind UNINDEXED,
            language UNINDEXED,
            level UNINDEXED,
            title,
            body
        );
        ",
        main = pair.main,
        fts = pair.fts,
    ))?;
    Ok(())
}

fn drop_table_pair(conn: &Connection, pair: &TablePair) -> Result<(), IndexError> {
    conn.execute_batch(&format!(
        "DROP TABLE IF EXISTS {fts}; DROP TABLE IF EXISTS {main};",
        main = pair.main,
        fts = pair.fts,
    ))?;
    Ok(())
}

fn migrate_old_to_new(conn: &Connection) -> Result<(), IndexError> {
    conn.execute_batch(&format!(
        "INSERT OR IGNORE INTO {high_main}
             (document_id, projection_view_id, canonical_uri, context_type,
              resource_id, content_kind, language, level, title, body,
              embedding_json, updated_at)
         SELECT document_id, projection_view_id, canonical_uri, context_type,
                resource_id, content_kind, language, level, title, body,
                embedding_json, updated_at
         FROM semantic_documents
         WHERE level <= {threshold};

         INSERT OR IGNORE INTO {detail_main}
             (document_id, projection_view_id, canonical_uri, context_type,
              resource_id, content_kind, language, level, title, body,
              embedding_json, updated_at)
         SELECT document_id, projection_view_id, canonical_uri, context_type,
                resource_id, content_kind, language, level, title, body,
                embedding_json, updated_at
         FROM semantic_documents
         WHERE level > {threshold};

         INSERT INTO {high_fts}
             (document_id, projection_view_id, canonical_uri, context_type,
              resource_id, content_kind, language, level, title, body)
         SELECT document_id, projection_view_id, canonical_uri, context_type,
                resource_id, content_kind, language, level, title, body
         FROM {high_main};

         INSERT INTO {detail_fts}
             (document_id, projection_view_id, canonical_uri, context_type,
              resource_id, content_kind, language, level, title, body)
         SELECT document_id, projection_view_id, canonical_uri, context_type,
                resource_id, content_kind, language, level, title, body
         FROM {detail_main};",
        high_main = HIGH.main,
        high_fts = HIGH.fts,
        detail_main = DETAIL.main,
        detail_fts = DETAIL.fts,
        threshold = HIGH_LEVEL_MAX,
    ))?;
    Ok(())
}

const REQUIRED_COLUMNS: &[&str] = &[
    "document_id",
    "projection_view_id",
    "canonical_uri",
    "resource_id",
    "content_kind",
    "language",
];

fn new_schema_requires_reset(conn: &Connection) -> Result<bool, IndexError> {
    for pair in ALL_PAIRS {
        let main_cols = table_columns(conn, pair.main)?;
        if main_cols.is_empty() {
            continue;
        }
        let fts_cols = table_columns(conn, pair.fts)?;
        let has_all = |cols: &[String]| {
            REQUIRED_COLUMNS
                .iter()
                .all(|required| cols.iter().any(|existing| existing == required))
        };
        if !has_all(&main_cols) || !has_all(&fts_cols) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn table_columns(conn: &Connection, table: &str) -> Result<Vec<String>, IndexError> {
    let sql = format!("PRAGMA table_info({table})");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(IndexError::from)
}

// ─── Helpers ────────────────────────────────────────────────────────────────

fn document_identity(document: &SemanticDocument) -> String {
    format!(
        "{}|{}|{}|{}",
        document.projection_view_id, document.context_type, document.level, document.uri
    )
}

fn excerpt_for_query(body: &str, query_hint: &str) -> String {
    let words: Vec<&str> = body.split_whitespace().collect();
    if words.is_empty() {
        return body.chars().take(96).collect();
    }
    const WINDOW: usize = 16;
    let start = if query_hint.is_empty() {
        0
    } else {
        let query_tokens: Vec<String> = query_hint
            .split_whitespace()
            .map(str::to_ascii_lowercase)
            .collect();
        (0..words.len())
            .map(|i| {
                let window_end = (i + WINDOW).min(words.len());
                let matches = words[i..window_end]
                    .iter()
                    .filter(|w| {
                        let lw = w.to_ascii_lowercase();
                        query_tokens.iter().any(|qt| lw.contains(qt.as_str()))
                    })
                    .count();
                (matches, i)
            })
            .max_by_key(|(matches, _)| *matches)
            .map_or(0, |(_, i)| i)
    };
    words[start..(start + WINDOW).min(words.len())].join(" ")
}

fn cosine_similarity(query: &[f32], candidate: &[f32]) -> f64 {
    mfs_types::math::cosine_similarity(query, candidate)
}

fn to_sqlite_json_error(source: serde_json::Error) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(source.into())
}
