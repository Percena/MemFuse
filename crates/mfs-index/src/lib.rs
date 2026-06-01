mod fts;
mod semantic;

use std::error::Error;
use std::fmt::{self, Display, Formatter};

use rusqlite::types::Value;

pub use fts::SqliteFtsIndex;
pub use semantic::{HIGH_LEVEL_MAX, SemanticDocument, SqliteSemanticIndex};

pub(crate) fn sanitize_fts_query(query: &str) -> String {
    // Preserve Unicode letters/digits (covers CJK, Arabic, etc.) and ASCII whitespace.
    // Replace everything else (FTS5 operators: " + - * ^ ( ) ) with spaces.
    query
        .chars()
        .map(|ch| {
            if ch.is_alphanumeric() || ch.is_ascii_whitespace() {
                ch
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn normalize_target_scope(target: Option<&str>) -> &str {
    target.unwrap_or("").trim_end_matches('/')
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedDocument {
    pub uri: String,
    pub context_type: String,
    pub level: u8,
    pub title: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SearchHit {
    pub uri: String,
    pub context_type: String,
    pub level: u8,
    pub score: f64,
    pub excerpt: String,
}

pub trait SearchIndex {
    fn index_document(&self, document: &IndexedDocument) -> Result<(), IndexError>;
    fn search(
        &self,
        query: &str,
        target_prefix: Option<&str>,
        limit: usize,
    ) -> Result<Vec<SearchHit>, IndexError> {
        self.search_with_filters(query, target_prefix, None, None, limit)
    }

    fn search_with_levels(
        &self,
        query: &str,
        target_prefix: Option<&str>,
        levels: Option<&[u8]>,
        limit: usize,
    ) -> Result<Vec<SearchHit>, IndexError> {
        self.search_with_filters(query, target_prefix, levels, None, limit)
    }

    fn search_with_filters(
        &self,
        query: &str,
        target_prefix: Option<&str>,
        levels: Option<&[u8]>,
        context_types: Option<&[&str]>,
        limit: usize,
    ) -> Result<Vec<SearchHit>, IndexError>;

    fn grep_literal(
        &self,
        pattern: &str,
        target_prefix: Option<&str>,
        levels: Option<&[u8]>,
        context_types: Option<&[&str]>,
        limit: usize,
    ) -> Result<Vec<SearchHit>, IndexError>;
}

#[derive(Debug)]
pub enum IndexError {
    Sqlite(rusqlite::Error),
}

impl Display for IndexError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(source) => write!(f, "sqlite index error: {source}"),
        }
    }
}

impl Error for IndexError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Sqlite(source) => Some(source),
        }
    }
}

impl From<rusqlite::Error> for IndexError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Sqlite(value)
    }
}

/// Build a SQL `IN` clause for `u8` level values.
///
/// `u8` is bounded 0-255, so numeric `to_string()` interpolation is safe here.
/// `column` must be a known-safe identifier.
pub(crate) fn build_u8_in_clause(column: &str, values: Option<&[u8]>) -> String {
    debug_assert!(
        column
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_'),
        "column name must be alphanumeric/underscore only, got: {column}"
    );
    match values {
        Some(values) if !values.is_empty() => format!(
            " AND {column} IN ({})",
            values
                .iter()
                .map(|v| v.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        _ => String::new(),
    }
}

/// Build a parameterized SQL `IN` clause for string values.
///
/// Appends each value to `params` as `Value::Text` and returns the SQL fragment
/// using `?N` numbered placeholders (1-based, starting after the existing params).
/// `column` must be a known-safe identifier — never pass user-controlled input.
pub(crate) fn build_str_in_clause(
    column: &str,
    values: Option<&[&str]>,
    params: &mut Vec<Value>,
) -> String {
    debug_assert!(
        column
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_'),
        "column name must be alphanumeric/underscore only, got: {column}"
    );
    match values {
        Some(values) if !values.is_empty() => {
            let offset = params.len() + 1;
            let placeholders = (offset..offset + values.len())
                .map(|i| format!("?{i}"))
                .collect::<Vec<_>>()
                .join(", ");
            for s in values {
                params.push(Value::Text(s.to_string()));
            }
            format!(" AND {column} IN ({placeholders})")
        }
        _ => String::new(),
    }
}
