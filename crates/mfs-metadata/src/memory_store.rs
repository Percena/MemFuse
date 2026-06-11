// Memory-domain methods for MetadataStore.
// Extracted from store.rs into a separate module for clarity.
// All methods operate on the Memory subdomain: facts, episodes,
// FTS5 search, access logs, assertions, cursors, briefs, and heuristics.

use std::collections::HashMap;

use rusqlite::{Result, params};

use crate::store::MetadataStore;
use crate::store::{
    assertion_row_from_row, brief_row_from_row, cursor_row_from_row, episode_row_from_row,
    stored_heuristic_evidence_from_row, stored_heuristic_instance_from_row,
    stored_heuristic_rule_from_row,
};
use crate::store_types::{
    AssertionRow, BriefRow, CursorRow, EpisodeRow, FactRecord, HeuristicEvidenceRecord,
    HeuristicInstanceRecord, HeuristicRuleRecord, StoredFact, StoredHeuristicEvidence,
    StoredHeuristicInstance, StoredHeuristicRule,
};

// ─── Helper functions ──────────────────────────────────────────────

fn stored_fact_from_row(row: &rusqlite::Row<'_>) -> Result<StoredFact> {
    Ok(StoredFact {
        id: row.get(0)?,
        account_id: row.get(1)?,
        user_id: row.get(2)?,
        agent_id: row.get(3)?,
        subject: row.get(4)?,
        predicate: row.get(5)?,
        display_value: row.get(6)?,
        normalized_value_json: row.get(7)?,
        value_type: row.get(8)?,
        confidence: row.get(9)?,
        status: row.get(10)?,
        valid_from: row.get(11)?,
        valid_to: row.get(12)?,
        source_assertion_id: row.get(13)?,
        source_episode_ids_json: row.get(14)?,
        created_at: row.get(15)?,
        updated_at: row.get(16)?,
        superseded_at: row.get(17)?,
        superseded_by: row.get(18)?,
        recall_count: row.get(19)?,
        last_recalled_at: row.get(20)?,
    })
}

impl MetadataStore {
    // ─── Facts ──────────────────────────────────────────────────────────

    pub fn insert_fact(&self, record: &FactRecord<'_>) -> Result<()> {
        let agent_id = record.agent_id.unwrap_or("coding-agent");
        self.lock_conn()?.execute(
            "INSERT INTO facts (
                id, account_id, user_id, agent_id,
                subject, predicate, display_value,
                normalized_value_json, value_type,
                confidence, status,
                valid_from, valid_to,
                source_assertion_id, source_episode_ids_json,
                created_at, superseded_at, superseded_by
             ) VALUES (
                ?1, ?2, ?3, ?4,
                ?5, ?6, ?7,
                ?8, ?9,
                ?10, ?11,
                ?12, ?13,
                ?14, ?15,
                CURRENT_TIMESTAMP, NULL, NULL
             )",
            params![
                record.id,
                record.account_id,
                record.user_id,
                agent_id,
                record.subject,
                record.predicate,
                record.display_value,
                record.normalized_value_json,
                record.value_type,
                record.confidence,
                record.status,
                record.valid_from,
                record.valid_to,
                record.source_assertion_id,
                record.source_episode_ids_json,
            ],
        )?;
        Ok(())
    }

    /// Get currently-active facts (status='active' AND valid_to IS NULL).
    /// By default, only return facts that are currently effective —
    /// valid_to must be NULL (not yet superseded/retracted)
    /// OR valid_to must be in the future.
    pub fn get_active_facts(&self, account_id: &str, user_id: &str) -> Result<Vec<StoredFact>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, account_id, user_id, agent_id,
                    subject, predicate, display_value,
                    normalized_value_json, value_type,
                    confidence, status,
                    valid_from, valid_to,
                    source_assertion_id, source_episode_ids_json,
                    created_at, updated_at,
                    superseded_at, superseded_by,
                    recall_count, last_recalled_at
             FROM facts
             WHERE account_id = ?1 AND user_id = ?2 AND status = 'active'
               AND (valid_to IS NULL OR valid_to > strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
             ORDER BY confidence DESC",
        )?;
        let rows = stmt.query_map(params![account_id, user_id], stored_fact_from_row)?;
        rows.collect()
    }

    /// Point-in-time fact query (§2.1): return facts that were effective
    /// at the given timestamp. A fact is effective at `at_time` when:
    ///   (valid_from IS NULL OR valid_from <= at_time)
    ///   AND (valid_to IS NULL OR valid_to > at_time)
    ///
    /// Uses pure temporal filtering rather than `status = 'active'`,
    /// because status reflects the *current* DB state, not the historical
    /// state at `at_time`. A fact superseded at T2 was active at T1 < T2
    /// and should appear in an AS OF T1 query.
    ///
    /// Deduplicates by predicate to return only the latest version at
    /// the query time (latest tcommit among facts with the same predicate
    /// that satisfy the temporal window).
    pub fn get_facts_at_time(
        &self,
        account_id: &str,
        user_id: &str,
        at_time: &str,
    ) -> Result<Vec<StoredFact>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, account_id, user_id, agent_id,
                    subject, predicate, display_value,
                    normalized_value_json, value_type,
                    confidence, status,
                    valid_from, valid_to,
                    source_assertion_id, source_episode_ids_json,
                    created_at, updated_at,
                    superseded_at, superseded_by,
                    recall_count, last_recalled_at
             FROM facts
             WHERE account_id = ?1 AND user_id = ?2
               AND (valid_from IS NULL OR valid_from <= ?3)
               AND (valid_to IS NULL OR valid_to > ?3)
             ORDER BY updated_at DESC, id DESC",
        )?;
        let rows = stmt.query_map(params![account_id, user_id, at_time], stored_fact_from_row)?;

        // Deduplicate by predicate: keep only the latest version at at_time.
        // This ensures superseded facts (status != 'active') that were valid
        // at the query point are included, with only the then-active version
        // per predicate retained.
        let mut facts: Vec<StoredFact> = rows.collect::<Result<Vec<_>, _>>()?;
        let mut seen_predicates: std::collections::HashSet<(String, String)> =
            std::collections::HashSet::new();
        facts.retain(|f| {
            let key = (f.subject.clone(), f.predicate.clone());
            seen_predicates.insert(key)
        });

        Ok(facts)
    }

    /// Get facts by status (e.g., "superseded", "retracted", "expired").
    /// Used by dream consolidation Phase 3 to count superseded facts.
    pub fn get_facts_by_status(
        &self,
        account_id: &str,
        user_id: &str,
        status: &str,
    ) -> Result<Vec<StoredFact>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, account_id, user_id, agent_id,
                    subject, predicate, display_value,
                    normalized_value_json, value_type,
                    confidence, status,
                    valid_from, valid_to,
                    source_assertion_id, source_episode_ids_json,
                    created_at, updated_at,
                    superseded_at, superseded_by,
                    recall_count, last_recalled_at
             FROM facts
             WHERE account_id = ?1 AND user_id = ?2 AND status = ?3
             ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map(params![account_id, user_id, status], stored_fact_from_row)?;
        rows.collect()
    }

    pub fn get_active_facts_by_predicate(
        &self,
        account_id: &str,
        user_id: &str,
        predicate: &str,
    ) -> Result<Vec<StoredFact>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, account_id, user_id, agent_id,
                    subject, predicate, display_value,
                    normalized_value_json, value_type,
                    confidence, status,
                    valid_from, valid_to,
                    source_assertion_id, source_episode_ids_json,
                    created_at, updated_at,
                    superseded_at, superseded_by,
                    recall_count, last_recalled_at
             FROM facts
             WHERE account_id = ?1 AND user_id = ?2 AND predicate = ?3 AND status = 'active'
             ORDER BY confidence DESC",
        )?;
        let rows = stmt.query_map(
            params![account_id, user_id, predicate],
            stored_fact_from_row,
        )?;
        rows.collect()
    }

    pub fn supersede_fact(&self, fact_id: &str, new_fact_id: &str, valid_to: &str) -> Result<()> {
        self.lock_conn()?.execute(
            "UPDATE facts
             SET status = 'superseded',
                 superseded_at = CURRENT_TIMESTAMP,
                 superseded_by = ?2,
                 valid_to = ?3
             WHERE id = ?1",
            params![fact_id, new_fact_id, valid_to],
        )?;
        Ok(())
    }

    pub fn retract_fact(&self, fact_id: &str, valid_to: &str) -> Result<()> {
        self.lock_conn()?.execute(
            "UPDATE facts
             SET status = 'retracted',
                 superseded_at = CURRENT_TIMESTAMP,
                 valid_to = ?2
             WHERE id = ?1",
            params![fact_id, valid_to],
        )?;
        Ok(())
    }

    /// Mark a fact as expired (time-based lifecycle, Ebbinghaus decay).
    /// Sets status to 'expired', valid_to to current timestamp.
    /// This activates the previously unused FactStatus::Expired state.
    pub fn expire_fact(&self, fact_id: &str) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        self.lock_conn()?.execute(
            "UPDATE facts
             SET status = 'expired',
                 valid_to = ?2,
                 superseded_at = CURRENT_TIMESTAMP
             WHERE id = ?1 AND status = 'active'",
            params![fact_id, now],
        )?;
        Ok(())
    }

    /// Update a fact's display_value and confidence (for memory import).
    /// Only updates active facts — retracted/superseded facts are not modified.
    pub fn update_fact_value(
        &self,
        fact_id: &str,
        display_value: &str,
        confidence: f64,
    ) -> Result<usize> {
        let rows = self.lock_conn()?.execute(
            "UPDATE facts
             SET display_value = ?2,
                 confidence = ?3,
                 updated_at = CURRENT_TIMESTAMP
             WHERE id = ?1 AND status = 'active'",
            params![fact_id, display_value, confidence],
        )?;
        Ok(rows)
    }

    pub fn get_fact(&self, fact_id: &str) -> Result<Option<StoredFact>> {
        let result = self.lock_conn()?.query_row(
            "SELECT id, account_id, user_id, agent_id,
                    subject, predicate, display_value,
                    normalized_value_json, value_type,
                    confidence, status,
                    valid_from, valid_to,
                    source_assertion_id, source_episode_ids_json,
                    created_at, updated_at,
                    superseded_at, superseded_by,
                    recall_count, last_recalled_at
             FROM facts
             WHERE id = ?1",
            params![fact_id],
            stored_fact_from_row,
        );
        match result {
            Ok(fact) => Ok(Some(fact)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn count_active_facts(&self, account_id: &str, user_id: &str) -> Result<usize> {
        self.lock_conn()?.query_row(
            "SELECT COUNT(*) FROM facts WHERE account_id = ?1 AND user_id = ?2 AND status = 'active'",
            params![account_id, user_id],
            |row| row.get::<_, usize>(0),
        )
    }
    // ── FTS5 full-text search (§10.2.1) ─────────────────────────────

    /// Sanitize a raw query string for FTS5: strip special chars,
    /// collapse whitespace, and return space-delimited terms.
    fn sanitize_fts5_query(query: &str) -> String {
        query
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || ch.is_ascii_whitespace() {
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

    /// Full-text search active facts using BM25 ranking.
    /// Falls back to empty vec if the FTS5 table doesn't exist (e.g., older DB).
    pub fn search_facts_fts(
        &self,
        account_id: &str,
        user_id: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<StoredFact>> {
        let sanitized = Self::sanitize_fts5_query(query);
        if sanitized.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.lock_conn()?;
        let sql = "
            SELECT f.id, f.account_id, f.user_id, f.agent_id,
                   f.subject, f.predicate, f.display_value,
                   f.normalized_value_json, f.value_type,
                   f.confidence, f.status,
                   f.valid_from, f.valid_to,
                   f.source_assertion_id, f.source_episode_ids_json,
                   f.created_at, f.updated_at,
                   f.superseded_at, f.superseded_by,
                   f.recall_count, f.last_recalled_at
            FROM facts f
            INNER JOIN facts_fts ft ON f.rowid = ft.rowid
            WHERE facts_fts MATCH ?1
              AND f.account_id = ?2 AND f.user_id = ?3 AND f.status = 'active'
            ORDER BY bm25(facts_fts)
            LIMIT ?4
        ";
        let mut stmt = match conn.prepare(sql) {
            Ok(s) => s,
            Err(_) => return Ok(Vec::new()), // FTS table missing — non-fatal
        };
        let rows = stmt.query_map(
            params![sanitized, account_id, user_id, limit as i64],
            stored_fact_from_row,
        )?;
        rows.collect()
    }

    /// Full-text search episodes using BM25 ranking.
    /// Falls back to empty vec if the FTS5 table doesn't exist.
    pub fn search_episodes_fts(
        &self,
        account_id: &str,
        user_id: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<EpisodeRow>> {
        let sanitized = Self::sanitize_fts5_query(query);
        if sanitized.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.lock_conn()?;
        let sql = "
            SELECT e.episode_id, e.account_id, e.user_id, e.agent_id,
                   e.session_id, e.resource_id,
                   e.summary, e.detail_ref,
                   e.keywords_json,
                   e.salience_score, e.strength_score,
                   e.emotional_valence, e.emotional_intensity,
                   e.context_tags_json,
                   e.recall_count, e.last_recalled_at,
                   e.source_start_turn_id, e.source_end_turn_id,
                   e.created_at, e.archived_at, e.last_decay_at,
                   e.embedding_json
            FROM episode_chunks e
            INNER JOIN episodes_fts ef ON e.rowid = ef.rowid
            WHERE episodes_fts MATCH ?1
              AND e.account_id = ?2 AND e.user_id = ?3 AND e.archived_at IS NULL
            ORDER BY bm25(episodes_fts)
            LIMIT ?4
        ";
        let mut stmt = match conn.prepare(sql) {
            Ok(s) => s,
            Err(_) => return Ok(Vec::new()), // FTS table missing — non-fatal
        };
        let rows = stmt.query_map(
            params![sanitized, account_id, user_id, limit as i64],
            episode_row_from_row,
        )?;
        rows.collect()
    }

    // ── episode_chunks ─────────────────────────────────────────────

    pub fn insert_episode(
        &self,
        episode_id: &str,
        account_id: &str,
        user_id: &str,
        agent_id: &str,
        session_id: &str,
        resource_id: Option<&str>,
        summary: &str,
        detail_ref: Option<&str>,
        keywords_json: Option<&str>,
        salience_score: f64,
        strength_score: f64,
        emotional_valence: Option<f64>,
        emotional_intensity: Option<f64>,
        context_tags_json: Option<&str>,
        recall_count: i64,
        last_recalled_at: Option<&str>,
        source_start_turn_id: Option<&str>,
        source_end_turn_id: Option<&str>,
        archived_at: Option<&str>,
        last_decay_at: Option<&str>,
        embedding_json: Option<&str>,
    ) -> Result<()> {
        self.lock_conn()?.execute(
            "INSERT INTO episode_chunks (
                episode_id, account_id, user_id, agent_id,
                session_id, resource_id,
                summary, detail_ref, keywords_json,
                salience_score, strength_score,
                emotional_valence, emotional_intensity,
                context_tags_json, recall_count, last_recalled_at,
                source_start_turn_id, source_end_turn_id,
                archived_at, last_decay_at, embedding_json
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6,
                ?7, ?8, ?9, ?10, ?11,
                ?12, ?13, ?14, ?15, ?16,
                ?17, ?18, ?19, ?20, ?21
             )",
            params![
                episode_id,
                account_id,
                user_id,
                agent_id,
                session_id,
                resource_id,
                summary,
                detail_ref,
                keywords_json,
                salience_score,
                strength_score,
                emotional_valence,
                emotional_intensity,
                context_tags_json,
                recall_count,
                last_recalled_at,
                source_start_turn_id,
                source_end_turn_id,
                archived_at,
                last_decay_at,
                embedding_json,
            ],
        )?;
        Ok(())
    }

    pub fn get_episodes_by_user(
        &self,
        account_id: &str,
        user_id: &str,
        resource_id: Option<&str>,
    ) -> Result<Vec<EpisodeRow>> {
        if let Some(resource_id) = resource_id {
            let conn = self.lock_conn()?;
            let mut stmt = conn.prepare(
                "SELECT episode_id, account_id, user_id, agent_id,
                        session_id, resource_id,
                        summary, detail_ref, keywords_json,
                        salience_score, strength_score,
                        emotional_valence, emotional_intensity,
                        context_tags_json, recall_count, last_recalled_at,
                        source_start_turn_id, source_end_turn_id,
                        created_at, archived_at, last_decay_at, embedding_json
                 FROM episode_chunks
                 WHERE account_id = ?1 AND user_id = ?2 AND resource_id = ?3
                   AND archived_at IS NULL
                 ORDER BY salience_score DESC",
            )?;
            let rows = stmt.query_map(
                params![account_id, user_id, resource_id],
                episode_row_from_row,
            )?;
            rows.collect()
        } else {
            let conn = self.lock_conn()?;
            let mut stmt = conn.prepare(
                "SELECT episode_id, account_id, user_id, agent_id,
                        session_id, resource_id,
                        summary, detail_ref, keywords_json,
                        salience_score, strength_score,
                        emotional_valence, emotional_intensity,
                        context_tags_json, recall_count, last_recalled_at,
                        source_start_turn_id, source_end_turn_id,
                        created_at, archived_at, last_decay_at, embedding_json
                 FROM episode_chunks
                 WHERE account_id = ?1 AND user_id = ?2
                   AND archived_at IS NULL
                 ORDER BY salience_score DESC",
            )?;
            let rows = stmt.query_map(params![account_id, user_id], episode_row_from_row)?;
            rows.collect()
        }
    }

    /// Get a bounded candidate set for online context resolution.
    ///
    /// `resolve_context` only needs a small reranking pool, not every historical
    /// episode. Keep this query bounded at the storage layer so prompt-time
    /// recall cost does not grow linearly with a user's full episode history.
    pub fn get_episode_candidates_for_recall(
        &self,
        account_id: &str,
        user_id: &str,
        resource_id: Option<&str>,
        limit: usize,
        since: Option<&str>,
    ) -> Result<Vec<EpisodeRow>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let conn = self.lock_conn()?;
        if let Some(resource_id) = resource_id {
            let mut stmt = conn.prepare(
                "SELECT episode_id, account_id, user_id, agent_id,
                        session_id, resource_id,
                        summary, detail_ref, keywords_json,
                        salience_score, strength_score,
                        emotional_valence, emotional_intensity,
                        context_tags_json, recall_count, last_recalled_at,
                        source_start_turn_id, source_end_turn_id,
                        created_at, archived_at, last_decay_at, embedding_json
                 FROM episode_chunks
                 WHERE account_id = ?1 AND user_id = ?2 AND resource_id = ?3
                   AND archived_at IS NULL
                   AND (?4 IS NULL OR created_at >= ?4)
                 ORDER BY salience_score DESC, created_at DESC
                 LIMIT ?5",
            )?;
            let rows = stmt.query_map(
                params![account_id, user_id, resource_id, since, limit as i64],
                episode_row_from_row,
            )?;
            rows.collect()
        } else {
            let mut stmt = conn.prepare(
                "SELECT episode_id, account_id, user_id, agent_id,
                        session_id, resource_id,
                        summary, detail_ref, keywords_json,
                        salience_score, strength_score,
                        emotional_valence, emotional_intensity,
                        context_tags_json, recall_count, last_recalled_at,
                        source_start_turn_id, source_end_turn_id,
                        created_at, archived_at, last_decay_at, embedding_json
                 FROM episode_chunks
                 WHERE account_id = ?1 AND user_id = ?2
                   AND archived_at IS NULL
                   AND (?3 IS NULL OR created_at >= ?3)
                 ORDER BY salience_score DESC, created_at DESC
                 LIMIT ?4",
            )?;
            let rows = stmt.query_map(
                params![account_id, user_id, since, limit as i64],
                episode_row_from_row,
            )?;
            rows.collect()
        }
    }

    /// Get recent high-salience episodes.
    /// Returns the most recent N episodes with salience >= min_salience,
    /// optionally limited to those created after a specific timestamp.
    /// Used by Dream Phase 2 (Gather) for incremental context retrieval.
    pub fn get_recent_episodes(
        &self,
        account_id: &str,
        user_id: &str,
        limit: usize,
        min_salience: f64,
        since: Option<&str>,
    ) -> Result<Vec<EpisodeRow>> {
        let conn = self.lock_conn()?;
        let sql = if since.is_some() {
            "SELECT episode_id, account_id, user_id, agent_id,
                    session_id, resource_id,
                    summary, detail_ref, keywords_json,
                    salience_score, strength_score,
                    emotional_valence, emotional_intensity,
                    context_tags_json, recall_count, last_recalled_at,
                    source_start_turn_id, source_end_turn_id,
                    created_at, archived_at, last_decay_at, embedding_json
             FROM episode_chunks
             WHERE account_id = ?1 AND user_id = ?2
               AND archived_at IS NULL
               AND salience_score >= ?3
               AND created_at > ?4
             ORDER BY created_at DESC
             LIMIT ?5"
        } else {
            "SELECT episode_id, account_id, user_id, agent_id,
                    session_id, resource_id,
                    summary, detail_ref, keywords_json,
                    salience_score, strength_score,
                    emotional_valence, emotional_intensity,
                    context_tags_json, recall_count, last_recalled_at,
                    source_start_turn_id, source_end_turn_id,
                    created_at, archived_at, last_decay_at, embedding_json
             FROM episode_chunks
             WHERE account_id = ?1 AND user_id = ?2
               AND archived_at IS NULL
               AND salience_score >= ?3
             ORDER BY created_at DESC
             LIMIT ?5"
        };
        let mut stmt = conn.prepare(sql)?;
        let rows = if let Some(since_ts) = since {
            stmt.query_map(
                params![account_id, user_id, min_salience, since_ts, limit as i64],
                episode_row_from_row,
            )?
        } else {
            stmt.query_map(
                params![account_id, user_id, min_salience, limit as i64],
                episode_row_from_row,
            )?
        };
        rows.collect()
    }

    pub fn get_episode(&self, episode_id: &str) -> Result<Option<EpisodeRow>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT episode_id, account_id, user_id, agent_id,
                    session_id, resource_id,
                    summary, detail_ref, keywords_json,
                    salience_score, strength_score,
                    emotional_valence, emotional_intensity,
                    context_tags_json, recall_count, last_recalled_at,
                    source_start_turn_id, source_end_turn_id,
                    created_at, archived_at, last_decay_at, embedding_json
             FROM episode_chunks
             WHERE episode_id = ?1
             LIMIT 1",
        )?;
        let mut rows = stmt.query(params![episode_id])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        Ok(Some(episode_row_from_row(row)?))
    }

    pub fn update_episode_embedding(&self, episode_id: &str, embedding_json: &str) -> Result<()> {
        self.lock_conn()?.execute(
            "UPDATE episode_chunks
             SET embedding_json = ?2
             WHERE episode_id = ?1",
            params![episode_id, embedding_json],
        )?;
        Ok(())
    }

    pub fn update_episode_salience(
        &self,
        episode_id: &str,
        salience_score: f64,
        strength_score: f64,
        recall_count: i64,
        last_recalled_at: &str,
    ) -> Result<()> {
        self.lock_conn()?.execute(
            "UPDATE episode_chunks
             SET salience_score = ?2,
                 strength_score = ?3,
                 recall_count = ?4,
                 last_recalled_at = ?5
             WHERE episode_id = ?1",
            params![
                episode_id,
                salience_score,
                strength_score,
                recall_count,
                last_recalled_at
            ],
        )?;
        Ok(())
    }

    /// Update only the salience_score field, preserving recall_count and
    /// last_recalled_at. Used by Ebbinghaus decay to avoid overwriting
    /// concurrent atomic increments from resolve_memory_context/cite_memories.
    /// Also writes last_decay_at = now to track the decay cycle timestamp
    /// for incremental Δt computation on the next Dream cycle.
    pub fn update_episode_salience_only(
        &self,
        episode_id: &str,
        salience_score: f64,
        last_decay_at: &str,
    ) -> Result<()> {
        self.lock_conn()?.execute(
            "UPDATE episode_chunks
             SET salience_score = ?2, last_decay_at = ?3
             WHERE episode_id = ?1",
            params![episode_id, salience_score, last_decay_at],
        )?;
        Ok(())
    }

    pub fn archive_episode(&self, episode_id: &str, archived_at: &str) -> Result<()> {
        self.lock_conn()?.execute(
            "UPDATE episode_chunks
             SET archived_at = ?2
             WHERE episode_id = ?1",
            params![episode_id, archived_at],
        )?;
        Ok(())
    }

    /// Increment recall_count by 1 and set last_recalled_at to the current timestamp (OV-P1-6).
    /// This tracks how often an episode is retrieved for hotness-based scoring.
    pub fn increment_episode_recall(&self, episode_id: &str, recalled_at: &str) -> Result<()> {
        self.lock_conn()?.execute(
            "UPDATE episode_chunks
             SET recall_count = recall_count + 1,
                 last_recalled_at = ?2
             WHERE episode_id = ?1",
            params![episode_id, recalled_at],
        )?;
        Ok(())
    }

    /// Increment recall_count for a fact and set last_recalled_at (citation feedback loop).
    pub fn increment_fact_recall(&self, fact_id: &str, recalled_at: &str) -> Result<()> {
        self.lock_conn()?.execute(
            "UPDATE facts
             SET recall_count = recall_count + 1,
                 last_recalled_at = ?2
             WHERE id = ?1",
            params![fact_id, recalled_at],
        )?;
        Ok(())
    }

    // ── memory_access_log ──────────────────────────────────────────

    /// Append an access event to the memory_access_log table.
    /// Called every time a memory (episode/fact/heuristic) is retrieved,
    /// enabling Ebbinghaus spacing-effect computation from access history.
    pub fn append_access_log(
        &self,
        memory_id: &str,
        memory_type: &str,
        accessed_at: &str,
        account_id: &str,
        user_id: &str,
    ) -> Result<()> {
        self.lock_conn()?.execute(
            "INSERT INTO memory_access_log (memory_id, memory_type, accessed_at, account_id, user_id)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![memory_id, memory_type, accessed_at, account_id, user_id],
        )?;
        Ok(())
    }

    /// Increment recall counters and append access-log rows in one transaction.
    ///
    /// Prompt-time context resolution can touch several facts and episodes.
    /// Doing this as a single transaction avoids per-memory SQLite lock/commit
    /// churn on the read path.
    pub fn record_recall_access_batch(
        &self,
        fact_ids: &[&str],
        episode_ids: &[&str],
        recalled_at: &str,
        account_id: &str,
        user_id: &str,
    ) -> Result<()> {
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction()?;
        for fact_id in fact_ids {
            tx.execute(
                "UPDATE facts
                 SET recall_count = recall_count + 1,
                     last_recalled_at = ?2
                 WHERE id = ?1",
                params![fact_id, recalled_at],
            )?;
            tx.execute(
                "INSERT INTO memory_access_log (memory_id, memory_type, accessed_at, account_id, user_id)
                 VALUES (?1, 'fact', ?2, ?3, ?4)",
                params![fact_id, recalled_at, account_id, user_id],
            )?;
        }
        for episode_id in episode_ids {
            tx.execute(
                "UPDATE episode_chunks
                 SET recall_count = recall_count + 1,
                     last_recalled_at = ?2
                 WHERE episode_id = ?1",
                params![episode_id, recalled_at],
            )?;
            tx.execute(
                "INSERT INTO memory_access_log (memory_id, memory_type, accessed_at, account_id, user_id)
                 VALUES (?1, 'episode', ?2, ?3, ?4)",
                params![episode_id, recalled_at, account_id, user_id],
            )?;
        }
        tx.commit()
    }

    /// Prune access log entries older than `cutoff_days` days.
    /// Keeps the log size bounded; older entries contribute diminishingly
    /// to spacing_factor and are no longer needed for accurate computation.
    pub fn prune_access_log(&self, cutoff_days: f64) -> Result<usize> {
        let cutoff_ts = chrono::Utc::now() - chrono::Duration::days(cutoff_days as i64);
        let cutoff_str = cutoff_ts.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        self.lock_conn()?.execute(
            "DELETE FROM memory_access_log WHERE accessed_at < ?1",
            params![cutoff_str],
        )
    }

    /// Retrieve access timestamps for a memory, sorted ascending.
    /// Returns days-since-access for each entry (relative to `now`),
    /// suitable for spacing_factor Σ(1/d_i) computation.
    pub fn get_access_days_since(
        &self,
        memory_id: &str,
        now: &chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<f64>> {
        let batch = self.get_access_days_since_batch(&[memory_id.to_string()], now)?;
        Ok(batch.get(memory_id).cloned().unwrap_or_default())
    }

    /// Batch-retrieve access days-since for multiple memories.
    /// Uses a single SQL query with IN clause instead of N+1 per-id queries.
    pub fn get_access_days_since_batch(
        &self,
        memory_ids: &[String],
        now: &chrono::DateTime<chrono::Utc>,
    ) -> Result<HashMap<String, Vec<f64>>> {
        if memory_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let conn = self.lock_conn()?;
        // Build dynamic IN clause: WHERE memory_id IN (?1, ?2, ...)
        let placeholders: Vec<String> = memory_ids
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", i + 1))
            .collect();
        let sql = format!(
            "SELECT memory_id, accessed_at FROM memory_access_log
             WHERE memory_id IN ({}) ORDER BY accessed_at ASC",
            placeholders.join(",")
        );
        let mut stmt = conn.prepare(&sql)?;
        let param_refs: Vec<&dyn rusqlite::types::ToSql> = memory_ids
            .iter()
            .map(|id| id as &dyn rusqlite::types::ToSql)
            .collect();
        let rows = stmt.query_map(param_refs.as_slice(), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut result: HashMap<String, Vec<f64>> = HashMap::new();
        for row in rows {
            let (id, ts_str) = row?;
            let accessed = chrono::DateTime::parse_from_rfc3339(&ts_str)
                .ok()
                .map(|dt| dt.with_timezone(&chrono::Utc));
            if let Some(accessed) = accessed {
                let d = (*now - accessed).num_seconds() as f64 / 86400.0;
                if d >= 0.001 {
                    result.entry(id).or_default().push(d);
                }
            }
        }
        Ok(result)
    }

    pub fn delete_episodes_by_session(&self, session_id: &str) -> Result<usize> {
        self.lock_conn()?.execute(
            "DELETE FROM episode_chunks WHERE session_id = ?1",
            params![session_id],
        )
    }
    // ── fact_assertions ────────────────────────────────────────────

    pub fn insert_fact_assertion(
        &self,
        assertion_id: &str,
        account_id: &str,
        user_id: &str,
        agent_id: &str,
        subject: &str,
        predicate: &str,
        raw_value_text: &str,
        normalized_value_json: Option<&str>,
        value_type: &str,
        operation: &str,
        confidence: f64,
        valid_from: Option<&str>,
        valid_to: Option<&str>,
        source_turn_id: Option<&str>,
        source_episode_ids_json: Option<&str>,
        source_resource_id: Option<&str>,
        source_snapshot_id: Option<&str>,
        source_uri: Option<&str>,
        extractor_version: &str,
    ) -> Result<()> {
        self.lock_conn()?.execute(
            "INSERT INTO fact_assertions (
                assertion_id, account_id, user_id, agent_id,
                subject, predicate, raw_value_text,
                normalized_value_json, value_type, operation,
                confidence, valid_from, valid_to,
                source_turn_id, source_episode_ids_json,
                source_resource_id, source_snapshot_id,
                source_uri, extractor_version
             ) VALUES (
                ?1, ?2, ?3, ?4,
                ?5, ?6, ?7,
                ?8, ?9, ?10,
                ?11, ?12, ?13,
                ?14, ?15,
                ?16, ?17,
                ?18, ?19
             )",
            params![
                assertion_id,
                account_id,
                user_id,
                agent_id,
                subject,
                predicate,
                raw_value_text,
                normalized_value_json,
                value_type,
                operation,
                confidence,
                valid_from,
                valid_to,
                source_turn_id,
                source_episode_ids_json,
                source_resource_id,
                source_snapshot_id,
                source_uri,
                extractor_version,
            ],
        )?;
        Ok(())
    }

    pub fn get_assertions_by_source(
        &self,
        source_turn_id: Option<&str>,
        source_episode_id: Option<&str>,
    ) -> Result<Vec<AssertionRow>> {
        // §2.4: source_episode_ids_json stores a JSON array of episode IDs.
        // When querying by a single episode ID, use json_each to check containment
        // rather than exact string equality.
        if let (Some(turn_id), Some(ep_id)) = (source_turn_id, source_episode_id) {
            let conn = self.lock_conn()?;
            let mut stmt = conn.prepare(
                "SELECT assertion_id, account_id, user_id, agent_id,
                        subject, predicate, raw_value_text,
                        normalized_value_json, value_type, operation,
                        confidence, valid_from, valid_to,
                        source_turn_id, source_episode_ids_json,
                        source_resource_id, source_snapshot_id,
                        source_uri, extractor_version, created_at
                 FROM fact_assertions
                 WHERE source_turn_id = ?1
                   AND EXISTS (SELECT 1 FROM json_each(source_episode_ids_json) WHERE value = ?2)",
            )?;
            let rows = stmt.query_map(params![turn_id, ep_id], assertion_row_from_row)?;
            rows.collect()
        } else if let Some(turn_id) = source_turn_id {
            let conn = self.lock_conn()?;
            let mut stmt = conn.prepare(
                "SELECT assertion_id, account_id, user_id, agent_id,
                        subject, predicate, raw_value_text,
                        normalized_value_json, value_type, operation,
                        confidence, valid_from, valid_to,
                        source_turn_id, source_episode_ids_json,
                        source_resource_id, source_snapshot_id,
                        source_uri, extractor_version, created_at
                 FROM fact_assertions
                 WHERE source_turn_id = ?1",
            )?;
            let rows = stmt.query_map(params![turn_id], assertion_row_from_row)?;
            rows.collect()
        } else if let Some(ep_id) = source_episode_id {
            let conn = self.lock_conn()?;
            let mut stmt = conn.prepare(
                "SELECT assertion_id, account_id, user_id, agent_id,
                        subject, predicate, raw_value_text,
                        normalized_value_json, value_type, operation,
                        confidence, valid_from, valid_to,
                        source_turn_id, source_episode_ids_json,
                        source_resource_id, source_snapshot_id,
                        source_uri, extractor_version, created_at
                 FROM fact_assertions
                 WHERE EXISTS (SELECT 1 FROM json_each(source_episode_ids_json) WHERE value = ?1)",
            )?;
            let rows = stmt.query_map(params![ep_id], assertion_row_from_row)?;
            rows.collect()
        } else {
            Ok(Vec::new())
        }
    }
    // ── memory_consolidation_cursors ───────────────────────────────

    pub fn insert_cursor(
        &self,
        cursor_id: &str,
        account_id: &str,
        user_id: &str,
        scope_type: &str,
        scope_id: &str,
        last_consolidated_turn_id: Option<&str>,
        last_consolidated_at: Option<&str>,
    ) -> Result<()> {
        self.lock_conn()?.execute(
            "INSERT INTO memory_consolidation_cursors (
                cursor_id, account_id, user_id,
                scope_type, scope_id,
                last_consolidated_turn_id, last_consolidated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                cursor_id,
                account_id,
                user_id,
                scope_type,
                scope_id,
                last_consolidated_turn_id,
                last_consolidated_at,
            ],
        )?;
        Ok(())
    }

    pub fn get_cursor(
        &self,
        account_id: &str,
        user_id: &str,
        scope_type: &str,
        scope_id: &str,
    ) -> Result<Option<CursorRow>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT cursor_id, account_id, user_id,
                    scope_type, scope_id,
                    last_consolidated_turn_id, last_consolidated_at,
                    dedupe_key, lease_owner, lease_expires_at,
                    updated_at
             FROM memory_consolidation_cursors
             WHERE account_id = ?1 AND user_id = ?2
               AND scope_type = ?3 AND scope_id = ?4
             LIMIT 1",
        )?;
        let mut rows = stmt.query(params![account_id, user_id, scope_type, scope_id])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        Ok(Some(cursor_row_from_row(row)?))
    }

    pub fn advance_cursor(
        &self,
        cursor_id: &str,
        last_consolidated_turn_id: &str,
        last_consolidated_at: &str,
    ) -> Result<()> {
        self.lock_conn()?.execute(
            "UPDATE memory_consolidation_cursors
             SET last_consolidated_turn_id = ?2,
                 last_consolidated_at = ?3,
                 updated_at = CURRENT_TIMESTAMP
             WHERE cursor_id = ?1",
            params![cursor_id, last_consolidated_turn_id, last_consolidated_at],
        )?;
        Ok(())
    }

    pub fn lease_cursor(
        &self,
        cursor_id: &str,
        lease_owner: &str,
        lease_expires_at: &str,
    ) -> Result<()> {
        self.lock_conn()?.execute(
            "UPDATE memory_consolidation_cursors
             SET lease_owner = ?2,
                 lease_expires_at = ?3,
                 updated_at = CURRENT_TIMESTAMP
             WHERE cursor_id = ?1",
            params![cursor_id, lease_owner, lease_expires_at],
        )?;
        Ok(())
    }

    pub fn release_cursor_lease(&self, cursor_id: &str) -> Result<()> {
        self.lock_conn()?.execute(
            "UPDATE memory_consolidation_cursors
             SET lease_owner = NULL,
                 lease_expires_at = NULL,
                 updated_at = CURRENT_TIMESTAMP
             WHERE cursor_id = ?1",
            params![cursor_id],
        )?;
        Ok(())
    }

    // ── memory_briefs ──────────────────────────────────────────────

    pub fn insert_brief(
        &self,
        brief_id: &str,
        account_id: &str,
        user_id: &str,
        scope_type: &str,
        scope_id: &str,
        summary: &str,
        source_thread_ids_json: Option<&str>,
        anchor_episode_ids_json: Option<&str>,
    ) -> Result<()> {
        self.lock_conn()?.execute(
            "INSERT INTO memory_briefs (
                brief_id, account_id, user_id,
                scope_type, scope_id,
                summary, source_thread_ids_json, anchor_episode_ids_json
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                brief_id,
                account_id,
                user_id,
                scope_type,
                scope_id,
                summary,
                source_thread_ids_json,
                anchor_episode_ids_json,
            ],
        )?;
        Ok(())
    }

    pub fn get_brief(
        &self,
        account_id: &str,
        user_id: &str,
        scope_type: &str,
        scope_id: &str,
    ) -> Result<Option<BriefRow>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT brief_id, account_id, user_id,
                    scope_type, scope_id,
                    summary, source_thread_ids_json, anchor_episode_ids_json,
                    updated_at
             FROM memory_briefs
             WHERE account_id = ?1 AND user_id = ?2
               AND scope_type = ?3 AND scope_id = ?4
             LIMIT 1",
        )?;
        let mut rows = stmt.query(params![account_id, user_id, scope_type, scope_id])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        Ok(Some(brief_row_from_row(row)?))
    }

    pub fn upsert_brief(
        &self,
        brief_id: &str,
        account_id: &str,
        user_id: &str,
        scope_type: &str,
        scope_id: &str,
        summary: &str,
        source_thread_ids_json: Option<&str>,
        anchor_episode_ids_json: Option<&str>,
    ) -> Result<()> {
        self.lock_conn()?.execute(
            "INSERT OR REPLACE INTO memory_briefs (
                brief_id, account_id, user_id,
                scope_type, scope_id,
                summary, source_thread_ids_json, anchor_episode_ids_json
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                brief_id,
                account_id,
                user_id,
                scope_type,
                scope_id,
                summary,
                source_thread_ids_json,
                anchor_episode_ids_json,
            ],
        )?;
        Ok(())
    }

    // ─── Heuristic Rules ─────────────────────────────────────────────

    pub fn insert_heuristic_rule(&self, record: &HeuristicRuleRecord<'_>) -> Result<()> {
        let agent_id = record.agent_id.unwrap_or("coding-agent");
        let user_confirmed_int = record.user_confirmed as i64;
        self.lock_conn()?.execute(
            "INSERT INTO heuristic_rules (
                rule_id, account_id, user_id, agent_id,
                tags_json, rule_text, counter_examples_json,
                lifecycle_stage, evidence_count, aggregate_weight,
                last_evidence_at, source_instance_ids_json,
                promoted_at, user_confirmed
             ) VALUES (
                ?1, ?2, ?3, ?4,
                ?5, ?6, ?7,
                ?8, ?9, ?10,
                ?11, ?12,
                ?13, ?14
             )",
            params![
                record.rule_id,
                record.account_id,
                record.user_id,
                agent_id,
                record.tags_json,
                record.rule_text,
                record.counter_examples_json,
                record.lifecycle_stage,
                record.evidence_count,
                record.aggregate_weight,
                record.last_evidence_at,
                record.source_instance_ids_json,
                record.promoted_at,
                user_confirmed_int,
            ],
        )?;
        Ok(())
    }

    pub fn get_heuristic_rule(&self, rule_id: &str) -> Result<Option<StoredHeuristicRule>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT rule_id, account_id, user_id, agent_id,
                    tags_json, rule_text, counter_examples_json,
                    lifecycle_stage, evidence_count, aggregate_weight,
                    last_evidence_at, source_instance_ids_json,
                    created_at, promoted_at, archived_at, user_confirmed
             FROM heuristic_rules WHERE rule_id = ?1",
        )?;
        let mut rows = stmt.query(params![rule_id])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        Ok(Some(stored_heuristic_rule_from_row(row)?))
    }

    pub fn get_active_heuristic_rules(
        &self,
        account_id: &str,
        user_id: &str,
        stages: &[&str],
    ) -> Result<Vec<StoredHeuristicRule>> {
        let stages_json = serde_json::to_string(stages).map_err(|source: serde_json::Error| {
            rusqlite::Error::ToSqlConversionFailure(source.into())
        })?;
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT rule_id, account_id, user_id, agent_id,
                    tags_json, rule_text, counter_examples_json,
                    lifecycle_stage, evidence_count, aggregate_weight,
                    last_evidence_at, source_instance_ids_json,
                    created_at, promoted_at, archived_at, user_confirmed
             FROM heuristic_rules
             WHERE account_id = ?1 AND user_id = ?2
               AND lifecycle_stage IN (SELECT value FROM json_each(?3))
             ORDER BY aggregate_weight DESC",
        )?;
        let rows = stmt.query_map(
            params![account_id, user_id, stages_json],
            stored_heuristic_rule_from_row,
        )?;
        rows.collect()
    }

    pub fn list_heuristic_rules(
        &self,
        account_id: &str,
        user_id: &str,
    ) -> Result<Vec<StoredHeuristicRule>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT rule_id, account_id, user_id, agent_id,
                    tags_json, rule_text, counter_examples_json,
                    lifecycle_stage, evidence_count, aggregate_weight,
                    last_evidence_at, source_instance_ids_json,
                    created_at, promoted_at, archived_at, user_confirmed
             FROM heuristic_rules
             WHERE account_id = ?1 AND user_id = ?2
             ORDER BY lifecycle_stage, aggregate_weight DESC",
        )?;
        let rows = stmt.query_map(params![account_id, user_id], stored_heuristic_rule_from_row)?;
        rows.collect()
    }

    pub fn update_rule_lifecycle(&self, rule_id: &str, new_stage: &str) -> Result<()> {
        let now_sql = "CURRENT_TIMESTAMP";
        let archived_update = if new_stage == "archived" {
            format!(", archived_at = {now_sql}")
        } else {
            String::new()
        };
        let promoted_update = if new_stage == "candidate" || new_stage == "confirmed" {
            format!(", promoted_at = {now_sql}")
        } else {
            String::new()
        };
        self.lock_conn()?.execute(
            &format!(
                "UPDATE heuristic_rules SET lifecycle_stage = ?1 {archived_update} {promoted_update} WHERE rule_id = ?2"
            ),
            params![new_stage, rule_id],
        )?;
        Ok(())
    }

    /// Mark a heuristic rule as user-confirmed (roadmap §5.4).
    /// User-confirmed rules are exempt from automatic decay, distinct from
    /// lifecycle_stage 'confirmed' which is reached via auto-promotion.
    /// Scopes the update to account_id + user_id to prevent cross-tenant mutation.
    /// Returns true if a row was updated, false if no matching rule found.
    /// Returns false on mutex poison (graceful degradation instead of panic).
    pub fn confirm_heuristic_rule(&self, rule_id: &str, account_id: &str, user_id: &str) -> bool {
        let Ok(conn) = self.lock_conn() else {
            return false;
        };
        conn.execute(
            "UPDATE heuristic_rules SET user_confirmed = 1 WHERE rule_id = ?1 AND account_id = ?2 AND user_id = ?3",
            params![rule_id, account_id, user_id],
        ).unwrap_or(0) > 0
    }

    pub fn update_rule_evidence_stats(
        &self,
        rule_id: &str,
        evidence_count: i64,
        aggregate_weight: f64,
        last_evidence_at: Option<&str>,
    ) -> Result<()> {
        self.lock_conn()?.execute(
            "UPDATE heuristic_rules
             SET evidence_count = ?1, aggregate_weight = ?2, last_evidence_at = ?3
             WHERE rule_id = ?4",
            params![evidence_count, aggregate_weight, last_evidence_at, rule_id],
        )?;
        Ok(())
    }

    /// Incrementally add one evidence record's weight to the rule's aggregate_weight
    /// and bump evidence_count by 1, preserving previously applied decay values.
    /// This avoids the crash-resilience gap where a full recomputation (raw sum)
    /// would overwrite decayed weights between the inline update and the next decay pass.
    pub fn increment_rule_evidence_stats(
        &self,
        rule_id: &str,
        new_weight: f64,
        last_evidence_at: &str,
    ) -> Result<()> {
        self.lock_conn()?.execute(
            "UPDATE heuristic_rules
             SET evidence_count = evidence_count + 1,
                 aggregate_weight = aggregate_weight + ?1,
                 last_evidence_at = ?2
             WHERE rule_id = ?3",
            params![new_weight, last_evidence_at, rule_id],
        )?;
        Ok(())
    }

    pub fn count_heuristic_rules(&self, account_id: &str, user_id: &str) -> Result<usize> {
        self.lock_conn()?.query_row(
            "SELECT COUNT(*) FROM heuristic_rules WHERE account_id = ?1 AND user_id = ?2",
            params![account_id, user_id],
            |row| row.get::<_, usize>(0),
        )
    }

    // ─── Heuristic Instances ──────────────────────────────────────────

    pub fn insert_heuristic_instance(&self, record: &HeuristicInstanceRecord<'_>) -> Result<()> {
        let agent_id = record.agent_id.unwrap_or("coding-agent");
        self.lock_conn()?.execute(
            "INSERT INTO heuristic_instances (
                instance_id, account_id, user_id, agent_id,
                context_summary, agent_proposal, user_reaction, outcome,
                signal_type, tags_json,
                session_id, source_turn_ids_json,
                derived_rule_id, instance_status, resolved_at
             ) VALUES (
                ?1, ?2, ?3, ?4,
                ?5, ?6, ?7, ?8,
                ?9, ?10,
                ?11, ?12,
                ?13, ?14, ?15
             )",
            params![
                record.instance_id,
                record.account_id,
                record.user_id,
                agent_id,
                record.context_summary,
                record.agent_proposal,
                record.user_reaction,
                record.outcome,
                record.signal_type,
                record.tags_json,
                record.session_id,
                record.source_turn_ids_json,
                record.derived_rule_id,
                record.instance_status,
                record.resolved_at,
            ],
        )?;
        Ok(())
    }

    pub fn get_heuristic_instance(
        &self,
        instance_id: &str,
    ) -> Result<Option<StoredHeuristicInstance>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT instance_id, account_id, user_id, agent_id,
                    context_summary, agent_proposal, user_reaction, outcome,
                    signal_type, tags_json,
                    session_id, source_turn_ids_json,
                    derived_rule_id, instance_status,
                    created_at, resolved_at
             FROM heuristic_instances WHERE instance_id = ?1",
        )?;
        let mut rows = stmt.query(params![instance_id])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        Ok(Some(stored_heuristic_instance_from_row(row)?))
    }

    pub fn list_heuristic_instances(
        &self,
        account_id: &str,
        user_id: &str,
        status_filter: Option<&str>,
    ) -> Result<Vec<StoredHeuristicInstance>> {
        let conn = self.lock_conn()?;
        let sql = if status_filter.is_some() {
            "SELECT instance_id, account_id, user_id, agent_id,
                    context_summary, agent_proposal, user_reaction, outcome,
                    signal_type, tags_json,
                    session_id, source_turn_ids_json,
                    derived_rule_id, instance_status,
                    created_at, resolved_at
             FROM heuristic_instances
             WHERE account_id = ?1 AND user_id = ?2 AND instance_status = ?3
             ORDER BY created_at DESC"
        } else {
            "SELECT instance_id, account_id, user_id, agent_id,
                    context_summary, agent_proposal, user_reaction, outcome,
                    signal_type, tags_json,
                    session_id, source_turn_ids_json,
                    derived_rule_id, instance_status,
                    created_at, resolved_at
             FROM heuristic_instances
             WHERE account_id = ?1 AND user_id = ?2
             ORDER BY created_at DESC"
        };
        let mut stmt = conn.prepare(sql)?;
        let rows = if let Some(status) = status_filter {
            stmt.query_map(
                params![account_id, user_id, status],
                stored_heuristic_instance_from_row,
            )?
        } else {
            stmt.query_map(
                params![account_id, user_id],
                stored_heuristic_instance_from_row,
            )?
        };
        rows.collect()
    }

    pub fn update_instance_status(
        &self,
        instance_id: &str,
        new_status: &str,
        derived_rule_id: Option<&str>,
    ) -> Result<()> {
        let resolved_at =
            if new_status == "promoted" || new_status == "dismissed" || new_status == "expired" {
                "CURRENT_TIMESTAMP"
            } else {
                "resolved_at"
            };
        self.lock_conn()?.execute(
            &format!(
                "UPDATE heuristic_instances
                 SET instance_status = ?1, derived_rule_id = COALESCE(?2, derived_rule_id), resolved_at = {resolved_at}
                 WHERE instance_id = ?3"
            ),
            params![new_status, derived_rule_id, instance_id],
        )?;
        Ok(())
    }

    // ─── Heuristic Evidence ───────────────────────────────────────────

    pub fn insert_heuristic_evidence(&self, record: &HeuristicEvidenceRecord<'_>) -> Result<()> {
        self.lock_conn()?.execute(
            "INSERT INTO heuristic_evidence (
                evidence_id, rule_id, instance_id,
                evidence_type, support_weight, session_id
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                record.evidence_id,
                record.rule_id,
                record.instance_id,
                record.evidence_type,
                record.support_weight,
                record.session_id,
            ],
        )?;
        Ok(())
    }

    pub fn list_evidence_for_rule(&self, rule_id: &str) -> Result<Vec<StoredHeuristicEvidence>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT evidence_id, rule_id, instance_id,
                    evidence_type, support_weight, session_id,
                    created_at
             FROM heuristic_evidence
             WHERE rule_id = ?1
             ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map(params![rule_id], stored_heuristic_evidence_from_row)?;
        rows.collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn insert_test_episode(
        store: &MetadataStore,
        episode_id: &str,
        user_id: &str,
        resource_id: Option<&str>,
        salience_score: f64,
        archived_at: Option<&str>,
    ) {
        let _ = store.insert_session("session-1", "acme", user_id, "coding-agent", "active", None);
        store
            .insert_episode(
                episode_id,
                "acme",
                user_id,
                "coding-agent",
                "session-1",
                resource_id,
                episode_id,
                None,
                None,
                salience_score,
                0.5,
                None,
                None,
                None,
                0,
                None,
                None,
                None,
                archived_at,
                None,
                None,
            )
            .unwrap();
    }

    fn insert_test_fact(store: &MetadataStore, fact_id: &str) {
        store
            .insert_fact(&FactRecord {
                id: fact_id,
                account_id: "acme",
                user_id: "alice",
                agent_id: Some("coding-agent"),
                subject: "alice",
                predicate: "prefers_editor",
                display_value: "zed",
                normalized_value_json: None,
                value_type: "text",
                confidence: 0.9,
                status: "active",
                valid_from: None,
                valid_to: None,
                source_assertion_id: None,
                source_episode_ids_json: None,
            })
            .unwrap();
    }

    fn set_episode_created_at(store: &MetadataStore, episode_id: &str, created_at: &str) {
        store
            .lock_conn()
            .unwrap()
            .execute(
                "UPDATE episode_chunks SET created_at = ?2 WHERE episode_id = ?1",
                params![episode_id, created_at],
            )
            .unwrap();
    }

    fn sqlite_index_names(store: &MetadataStore) -> Vec<String> {
        let conn = store.lock_conn().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT name
                 FROM sqlite_master
                 WHERE type = 'index'
                 ORDER BY name",
            )
            .unwrap();
        stmt.query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>>>()
            .unwrap()
    }

    #[test]
    fn episode_recall_candidate_indexes_are_bootstrapped() {
        let store = MetadataStore::open_in_memory(false).unwrap();
        let indexes = sqlite_index_names(&store);

        assert!(
            indexes.contains(&"idx_episode_recall_candidates_by_resource".to_string()),
            "resource-scoped recall candidate index missing: {indexes:?}"
        );
        assert!(
            indexes.contains(&"idx_episode_recall_candidates_all_resources".to_string()),
            "all-resource recall candidate index missing: {indexes:?}"
        );
    }

    #[test]
    fn episode_candidates_for_recall_are_bounded_ranked_and_filtered() {
        let store = MetadataStore::open_in_memory(false).unwrap();
        insert_test_episode(&store, "ep-low", "alice", Some("repo-1"), 0.1, None);
        insert_test_episode(&store, "ep-high", "alice", Some("repo-1"), 0.9, None);
        insert_test_episode(&store, "ep-mid", "alice", Some("repo-1"), 0.5, None);
        insert_test_episode(
            &store,
            "ep-archived",
            "alice",
            Some("repo-1"),
            1.0,
            Some("2026-01-01T00:00:00Z"),
        );
        insert_test_episode(
            &store,
            "ep-other-resource",
            "alice",
            Some("repo-2"),
            0.95,
            None,
        );
        insert_test_episode(&store, "ep-other-user", "bob", Some("repo-1"), 0.99, None);

        let rows = store
            .get_episode_candidates_for_recall("acme", "alice", Some("repo-1"), 2, None)
            .unwrap();
        let ids: Vec<&str> = rows.iter().map(|row| row.episode_id.as_str()).collect();
        assert_eq!(ids, vec!["ep-high", "ep-mid"]);

        let all_resource_rows = store
            .get_episode_candidates_for_recall("acme", "alice", None, 3, None)
            .unwrap();
        let all_resource_ids: Vec<&str> = all_resource_rows
            .iter()
            .map(|row| row.episode_id.as_str())
            .collect();
        assert_eq!(
            all_resource_ids,
            vec!["ep-other-resource", "ep-high", "ep-mid"]
        );

        assert!(
            store
                .get_episode_candidates_for_recall("acme", "alice", None, 0, None)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn episode_candidates_for_recall_can_prefilter_by_time_window() {
        let store = MetadataStore::open_in_memory(false).unwrap();
        insert_test_episode(&store, "ep-old", "alice", Some("repo-1"), 0.95, None);
        insert_test_episode(&store, "ep-new", "alice", Some("repo-1"), 0.5, None);
        set_episode_created_at(&store, "ep-old", "2025-01-01 00:00:00");
        set_episode_created_at(&store, "ep-new", "2026-06-01 00:00:00");

        let rows = store
            .get_episode_candidates_for_recall(
                "acme",
                "alice",
                Some("repo-1"),
                10,
                Some("2026-01-01 00:00:00"),
            )
            .unwrap();
        let ids: Vec<&str> = rows.iter().map(|row| row.episode_id.as_str()).collect();
        assert_eq!(ids, vec!["ep-new"]);
    }

    #[test]
    fn record_recall_access_batch_updates_counts_and_logs_once() {
        let store = MetadataStore::open_in_memory(false).unwrap();
        insert_test_fact(&store, "fact-1");
        insert_test_episode(&store, "ep-1", "alice", Some("repo-1"), 0.5, None);

        store
            .record_recall_access_batch(
                &["fact-1"],
                &["ep-1"],
                "2026-06-11T12:00:00Z",
                "acme",
                "alice",
            )
            .unwrap();

        let fact = store.get_fact("fact-1").unwrap().unwrap();
        assert_eq!(fact.recall_count, 1);
        assert_eq!(
            fact.last_recalled_at.as_deref(),
            Some("2026-06-11T12:00:00Z")
        );

        let episode = store.get_episode("ep-1").unwrap().unwrap();
        assert_eq!(episode.recall_count, 1);
        assert_eq!(
            episode.last_recalled_at.as_deref(),
            Some("2026-06-11T12:00:00Z")
        );

        let conn = store.lock_conn().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT memory_id, memory_type, accessed_at, account_id, user_id
                 FROM memory_access_log
                 ORDER BY memory_type, memory_id",
            )
            .unwrap();
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })
            .unwrap()
            .collect::<Result<Vec<_>>>()
            .unwrap();

        assert_eq!(
            rows,
            vec![
                (
                    "ep-1".to_string(),
                    "episode".to_string(),
                    "2026-06-11T12:00:00Z".to_string(),
                    "acme".to_string(),
                    "alice".to_string(),
                ),
                (
                    "fact-1".to_string(),
                    "fact".to_string(),
                    "2026-06-11T12:00:00Z".to_string(),
                    "acme".to_string(),
                    "alice".to_string(),
                ),
            ]
        );
    }
}
