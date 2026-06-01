use std::sync::Mutex;

use rusqlite::types::Value;
use rusqlite::{Connection, params, params_from_iter};

use crate::{
    IndexError, IndexedDocument, SearchHit, SearchIndex, build_str_in_clause, build_u8_in_clause,
    normalize_target_scope, sanitize_fts_query,
};

pub struct SqliteFtsIndex {
    conn: Mutex<Connection>,
}

impl SqliteFtsIndex {
    pub fn open_in_memory() -> Result<Self, IndexError> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(
            "
            CREATE VIRTUAL TABLE documents USING fts5(
                uri UNINDEXED,
                context_type UNINDEXED,
                level UNINDEXED,
                title,
                body
            );
            ",
        )?;

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }
}

impl SearchIndex for SqliteFtsIndex {
    fn index_document(&self, document: &IndexedDocument) -> Result<(), IndexError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO documents (uri, context_type, level, title, body)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                document.uri,
                document.context_type,
                i64::from(document.level),
                document.title,
                document.body,
            ],
        )?;
        Ok(())
    }

    fn search_with_filters(
        &self,
        query: &str,
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
        let level_clause = build_u8_in_clause("level", levels);

        // Fixed params: ?1=query, ?2=target, ?3=limit; context_type params start at ?4.
        let mut sql_params: Vec<Value> = vec![
            Value::Text(query),
            Value::Text(target.to_string()),
            Value::Integer(limit as i64),
        ];
        let context_type_clause =
            build_str_in_clause("context_type", context_types, &mut sql_params);

        let sql = format!(
            "SELECT uri,
                    context_type,
                    level,
                    bm25(documents) AS rank,
                    snippet(documents, 4, '', '', '…', 8)
             FROM documents
             WHERE documents MATCH ?1
               AND (?2 = '' OR uri = ?2 OR substr(uri, 1, length(?2) + 1) = ?2 || '/')
             {level_clause}
             {context_type_clause}
             ORDER BY rank
             LIMIT ?3"
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

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(IndexError::from)
    }

    fn grep_literal(
        &self,
        pattern: &str,
        target_prefix: Option<&str>,
        levels: Option<&[u8]>,
        context_types: Option<&[&str]>,
        limit: usize,
    ) -> Result<Vec<SearchHit>, IndexError> {
        let conn = self.conn.lock().unwrap();
        let target = normalize_target_scope(target_prefix);
        let level_clause = build_u8_in_clause("level", levels);

        // Fixed params: ?1=pattern, ?2=target, ?3=limit; context_type params start at ?4.
        let mut sql_params: Vec<Value> = vec![
            Value::Text(pattern.to_string()),
            Value::Text(target.to_string()),
            Value::Integer(limit as i64),
        ];
        let context_type_clause =
            build_str_in_clause("context_type", context_types, &mut sql_params);

        let sql = format!(
            "SELECT uri,
                    context_type,
                    level,
                    CAST(instr(body, ?1) AS REAL) AS rank,
                    substr(body,
                           CASE WHEN instr(body, ?1) > 24 THEN instr(body, ?1) - 24 ELSE 1 END,
                           160)
             FROM documents
             WHERE ?1 != ''
               AND instr(body, ?1) > 0
               AND (?2 = '' OR uri = ?2 OR substr(uri, 1, length(?2) + 1) = ?2 || '/')
             {level_clause}
             {context_type_clause}
             ORDER BY rank, uri
             LIMIT ?3"
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

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(IndexError::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_scope_does_not_match_prefix_sharing_siblings() {
        let index = SqliteFtsIndex::open_in_memory().unwrap();
        for uri in [
            "mfs://resources/localfs/project/sdk/src/mcp/server.ts",
            "mfs://resources/localfs/project/sdk/src/mcp/server.tsx",
            "mfs://resources/localfs/project/sdk/src/mcp/server.ts.overview.md",
        ] {
            index
                .index_document(&IndexedDocument {
                    uri: uri.to_owned(),
                    context_type: "resource".to_owned(),
                    level: 2,
                    title: uri.to_owned(),
                    body: "resolve_context".to_owned(),
                })
                .unwrap();
        }

        let target = "mfs://resources/localfs/project/sdk/src/mcp/server.ts";
        let search_hits = index.search("resolve_context", Some(target), 10).unwrap();
        let grep_hits = index
            .grep_literal("resolve_context", Some(target), None, None, 10)
            .unwrap();

        assert_eq!(
            search_hits
                .iter()
                .map(|hit| hit.uri.as_str())
                .collect::<Vec<_>>(),
            [target]
        );
        assert_eq!(
            grep_hits
                .iter()
                .map(|hit| hit.uri.as_str())
                .collect::<Vec<_>>(),
            [target]
        );
    }
}
