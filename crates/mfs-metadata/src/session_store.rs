//! Session-domain `impl MetadataStore` methods extracted from store.rs.
//!
//! Covers `conversation_sessions` and `conversation_turns` CRUD operations.
//! The corresponding `SessionRow` / `TurnRow` types live in `store_types.rs`.

use rusqlite::{Result, params};

use crate::store::MetadataStore;
use crate::store_types::{SessionRow, TurnRow, session_row_from_row, turn_row_from_row};

impl MetadataStore {
    // ── conversation_sessions ──────────────────────────────────────

    pub fn insert_session(
        &self,
        session_id: &str,
        account_id: &str,
        user_id: &str,
        agent_id: &str,
        status: &str,
        metadata_json: Option<&str>,
    ) -> Result<()> {
        self.lock_conn()?.execute(
            "INSERT INTO conversation_sessions (
                session_id, account_id, user_id, agent_id,
                status, metadata_json
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                session_id,
                account_id,
                user_id,
                agent_id,
                status,
                metadata_json
            ],
        )?;
        Ok(())
    }

    pub fn get_session(&self, session_id: &str) -> Result<Option<SessionRow>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT session_id, account_id, user_id, agent_id,
                    status, started_at, last_activity_at, metadata_json
             FROM conversation_sessions
             WHERE session_id = ?1
             LIMIT 1",
        )?;
        let mut rows = stmt.query(params![session_id])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        Ok(Some(session_row_from_row(row)?))
    }

    pub fn update_session_activity(&self, session_id: &str) -> Result<()> {
        self.lock_conn()?.execute(
            "UPDATE conversation_sessions
             SET last_activity_at = CURRENT_TIMESTAMP
             WHERE session_id = ?1",
            params![session_id],
        )?;
        Ok(())
    }

    pub fn list_sessions_by_user(
        &self,
        account_id: &str,
        user_id: &str,
        status: Option<&str>,
    ) -> Result<Vec<SessionRow>> {
        if let Some(status) = status {
            let conn = self.lock_conn()?;
            let mut stmt = conn.prepare(
                "SELECT session_id, account_id, user_id, agent_id,
                        status, started_at, last_activity_at, metadata_json
                 FROM conversation_sessions
                 WHERE account_id = ?1 AND user_id = ?2 AND status = ?3
                 ORDER BY last_activity_at DESC",
            )?;
            let rows =
                stmt.query_map(params![account_id, user_id, status], session_row_from_row)?;
            rows.collect()
        } else {
            let conn = self.lock_conn()?;
            let mut stmt = conn.prepare(
                "SELECT session_id, account_id, user_id, agent_id,
                        status, started_at, last_activity_at, metadata_json
                 FROM conversation_sessions
                 WHERE account_id = ?1 AND user_id = ?2
                 ORDER BY last_activity_at DESC",
            )?;
            let rows = stmt.query_map(params![account_id, user_id], session_row_from_row)?;
            rows.collect()
        }
    }

    // ── conversation_turns ─────────────────────────────────────────

    pub fn insert_turn(
        &self,
        turn_id: &str,
        turn_seq: i64,
        session_id: &str,
        account_id: &str,
        user_id: &str,
        agent_id: &str,
        role: &str,
        content_text: &str,
        content_json: Option<&str>,
        token_count: i64,
        ingested_at: Option<&str>,
    ) -> Result<()> {
        self.lock_conn()?.execute(
            "INSERT INTO conversation_turns (
                turn_id, turn_seq, session_id,
                account_id, user_id, agent_id,
                role, content_text, content_json,
                token_count, ingested_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                turn_id,
                turn_seq,
                session_id,
                account_id,
                user_id,
                agent_id,
                role,
                content_text,
                content_json,
                token_count,
                ingested_at,
            ],
        )?;
        Ok(())
    }

    pub fn get_turns_by_session(&self, session_id: &str) -> Result<Vec<TurnRow>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT turn_id, turn_seq, session_id,
                    account_id, user_id, agent_id,
                    role, content_text, content_json,
                    token_count, created_at, ingested_at
             FROM conversation_turns
             WHERE session_id = ?1
             ORDER BY turn_seq ASC",
        )?;
        let rows = stmt.query_map(params![session_id], turn_row_from_row)?;
        rows.collect()
    }

    pub fn get_turn_by_id(&self, turn_id: &str) -> Result<Option<TurnRow>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT turn_id, turn_seq, session_id,
                    account_id, user_id, agent_id,
                    role, content_text, content_json,
                    token_count, created_at, ingested_at
             FROM conversation_turns
             WHERE turn_id = ?1
             LIMIT 1",
        )?;
        let mut rows = stmt.query(params![turn_id])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        Ok(Some(turn_row_from_row(row)?))
    }

    /// Get recent user turns from other sessions for the same user, excluding the current session.
    /// Used for repetition detection across sessions.
    pub fn get_recent_session_turns(
        &self,
        account_id: &str,
        user_id: &str,
        current_session_id: &str,
        limit: i64,
    ) -> Result<Vec<TurnRow>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT turn_id, turn_seq, session_id,
                    account_id, user_id, agent_id,
                    role, content_text, content_json,
                    token_count, created_at, ingested_at
             FROM conversation_turns
             WHERE account_id = ?1 AND user_id = ?2
               AND session_id != ?3
               AND role = 'user'
             ORDER BY created_at DESC
             LIMIT ?4",
        )?;
        let rows = stmt.query_map(
            params![account_id, user_id, current_session_id, limit],
            turn_row_from_row,
        )?;
        rows.collect()
    }
}
