use rusqlite::{Connection, Result};

const CREATE_MIGRATIONS_TABLE: &str = "
CREATE TABLE IF NOT EXISTS _schema_migrations (
    version TEXT PRIMARY KEY,
    applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
";

const CANVAS_SNAPSHOT_IMMUTABILITY_TRIGGERS: &str = "
CREATE TRIGGER IF NOT EXISTS reject_immutable_canvas_snapshot_update
BEFORE UPDATE ON canvas_snapshots
FOR EACH ROW
WHEN OLD.immutable = 1
BEGIN
    SELECT RAISE(ABORT, 'canvas snapshot is immutable and cannot be updated');
END;

CREATE TRIGGER IF NOT EXISTS reject_immutable_canvas_snapshot_delete
BEFORE DELETE ON canvas_snapshots
FOR EACH ROW
WHEN OLD.immutable = 1
BEGIN
    SELECT RAISE(ABORT, 'canvas snapshot is immutable and cannot be deleted');
END;
";

pub(crate) fn bootstrap(conn: &Connection, separate_canvas_db: bool) -> Result<()> {
    conn.execute_batch(CREATE_MIGRATIONS_TABLE)?;
    let applied = get_applied_versions(conn)?;

    if !applied.iter().any(|v| v == "0001") {
        conn.execute_batch(include_str!("migrations/0001_initial.sql"))?;
        mark_applied(conn, "0001")?;
    }

    if !applied.iter().any(|v| v == "0002") {
        migration_0002(conn)?;
        mark_applied(conn, "0002")?;
    }

    if !applied.iter().any(|v| v == "0003") {
        conn.execute_batch(include_str!("migrations/0003_heuristics.sql"))?;
        mark_applied(conn, "0003")?;
    }

    if !applied.iter().any(|v| v == "0004") {
        // Migration 0004 (evaluation tables) has been removed — the
        // evaluation framework was excised as marginal (§10.6 never
        // produced real datasets). The version slot is kept as a
        // no-op so that existing databases that already applied 0004
        // don't diverge from fresh databases on future migrations.
        mark_applied(conn, "0004")?;
    }

    if !applied.iter().any(|v| v == "0005") {
        conn.execute_batch(include_str!("migrations/0005_user_confirmed.sql"))?;
        mark_applied(conn, "0005")?;
    }

    if !applied.iter().any(|v| v == "0006") {
        migration_0006(conn)?;
        mark_applied(conn, "0006")?;
    }

    if !applied.iter().any(|v| v == "0007") {
        migration_0007(conn)?;
        mark_applied(conn, "0007")?;
    }

    if !applied.iter().any(|v| v == "0008") {
        conn.execute_batch(include_str!("migrations/0008_fts5.sql"))?;
        mark_applied(conn, "0008")?;
    }

    if !applied.iter().any(|v| v == "0009") {
        conn.execute_batch(include_str!("migrations/0009_fts5_content_mode.sql"))?;
        mark_applied(conn, "0009")?;
    }

    if !applied.iter().any(|v| v == "0010") {
        conn.execute_batch(include_str!("migrations/0010_source_episode_ids_json.sql"))?;
        mark_applied(conn, "0010")?;
    }

    if !applied.iter().any(|v| v == "0011") {
        conn.execute_batch(include_str!("migrations/0011_webhooks.sql"))?;
        mark_applied(conn, "0011")?;
    }

    if !applied.iter().any(|v| v == "0012") {
        conn.execute_batch(include_str!("migrations/0012_manifest.sql"))?;
        mark_applied(conn, "0012")?;
    }

    // Migration 0013 (canvas/overlay tables): runtime routing.
    // When separate_canvas_db=false, all canvas tables live in metadata.sqlite.
    // When separate_canvas_db=true, canvas tables are created in canvas.sqlite
    // via bootstrap_canvas(), and 0013 is marked as applied here for version tracking.
    if !applied.iter().any(|v| v == "0013") {
        if separate_canvas_db {
            mark_applied(conn, "0013")?;
        } else {
            conn.execute_batch(include_str!("migrations/0013_canvas_overlay.sql"))?;
            mark_applied(conn, "0013")?;
        }
    }

    // Migration 0014 (PRD alignment): runtime routing.
    // When separate_canvas_db=false, all ALTERs run on conn.
    // When separate_canvas_db=true, only manifest_repo_identity ALTERs run
    // on conn; canvas/overlay ALTERs run in canvas.sqlite via bootstrap_canvas().
    if !applied.iter().any(|v| v == "0014") {
        if separate_canvas_db {
            conn.execute_batch(include_str!("migrations/0014_prd_alignment_metadata.sql"))?;
            mark_applied(conn, "0014")?;
        } else {
            conn.execute_batch(include_str!("migrations/0014_prd_alignment.sql"))?;
            mark_applied(conn, "0014")?;
        }
    }

    // Migration 0015 (overlay canvas refs): runtime routing.
    // When separate_canvas_db=false, overlay table columns are in metadata.sqlite.
    // When separate_canvas_db=true, they're in canvas.sqlite via bootstrap_canvas().
    if !applied.iter().any(|v| v == "0015") {
        if separate_canvas_db {
            mark_applied(conn, "0015")?;
        } else {
            conn.execute_batch(include_str!("migrations/0015_overlay_canvas_refs.sql"))?;
            mark_applied(conn, "0015")?;
        }
    }

    if !applied.iter().any(|v| v == "0016") {
        conn.execute_batch(include_str!("migrations/0016_manifest_yaml_nullable.sql"))?;
        mark_applied(conn, "0016")?;
    }

    // Migration 0017: overlay_refs + manifest_cache tables — runtime routing.
    // When separate_canvas_db=false, both tables go in metadata.sqlite.
    // When separate_canvas_db=true, both go in canvas.sqlite via bootstrap_canvas().
    if !applied.iter().any(|v| v == "0017") {
        if separate_canvas_db {
            mark_applied(conn, "0017")?;
        } else {
            conn.execute_batch(include_str!(
                "migrations/0017_overlay_refs_manifest_cache.sql"
            ))?;
            mark_applied(conn, "0017")?;
        }
    }

    // Immutability triggers on canvas_snapshots: only install in the DB
    // that owns the canvas_snapshots table.
    if !separate_canvas_db {
        conn.execute_batch(CANVAS_SNAPSHOT_IMMUTABILITY_TRIGGERS)?;
    }

    // Migration 0018: memory_access_log table for Ebbinghaus spacing-effect.
    // Stores per-memory access timestamps; replaces simple recall_count with
    // a richer history enabling reinforcement boost computation.
    if !applied.iter().any(|v| v == "0018") {
        conn.execute_batch(include_str!("migrations/0018_memory_access_log.sql"))?;
        mark_applied(conn, "0018")?;
    }

    // Migration 0019: temporal fields on relations (valid_from, valid_to,
    // tcommit, is_latest, superseded_by) — mirrors facts supersession pattern.
    if !applied.iter().any(|v| v == "0019") {
        migration_0019(conn)?;
        mark_applied(conn, "0019")?;
    }

    // Migration 0020: run_writebacks table for agent run
    // evidence persistence (POST /runs/writeback endpoint).
    if !applied.iter().any(|v| v == "0020") {
        conn.execute_batch(include_str!("migrations/0020_run_writebacks.sql"))?;
        mark_applied(conn, "0020")?;
    }

    // Migration 0021: add last_decay_at to episode_chunks for incremental Ebbinghaus decay.
    if !applied.iter().any(|v| v == "0021") {
        migration_0021(conn)?;
        mark_applied(conn, "0021")?;
    }

    // Migration 0022: orchestrator-facing resource business metadata.
    if !applied.iter().any(|v| v == "0022") {
        conn.execute_batch(include_str!(
            "migrations/0022_resource_business_metadata.sql"
        ))?;
        mark_applied(conn, "0022")?;
    }

    Ok(())
}

/// Bootstrap Canvas-specific tables into an independent SQLite connection.
///
/// Called when `separate_canvas_db` is true at runtime. Creates the canvas/
/// overlay tables (0013) and overlay canvas ref columns (0015) in a separate
/// `canvas.sqlite` database, along with immutability triggers.
///
/// The metadata.sqlite connection tracks 0013/0015 as "applied" (virtual)
/// via `bootstrap()` above, so future upgrades won't double-apply them.
pub(crate) fn bootstrap_canvas(conn: &Connection) -> Result<()> {
    conn.execute_batch(CREATE_MIGRATIONS_TABLE)?;
    let applied = get_applied_versions(conn)?;

    if !applied.iter().any(|v| v == "0013") {
        // Use variant migration without cross-DB FK references to manifest_repo_identity
        conn.execute_batch(include_str!("migrations/0013_canvas_overlay_separate.sql"))?;
        mark_applied(conn, "0013")?;
    }

    // Migration 0014: canvas/overlay ALTER statements run in canvas DB.
    // (manifest_repo_identity ALTERs run in metadata DB via bootstrap().)
    if !applied.iter().any(|v| v == "0014") {
        conn.execute_batch(include_str!("migrations/0014_prd_alignment_canvas.sql"))?;
        mark_applied(conn, "0014")?;
    }

    if !applied.iter().any(|v| v == "0015") {
        conn.execute_batch(include_str!("migrations/0015_overlay_canvas_refs.sql"))?;
        mark_applied(conn, "0015")?;
    }

    if !applied.iter().any(|v| v == "0017") {
        conn.execute_batch(include_str!(
            "migrations/0017_overlay_refs_manifest_cache_canvas.sql"
        ))?;
        mark_applied(conn, "0017")?;
    }

    conn.execute_batch(CANVAS_SNAPSHOT_IMMUTABILITY_TRIGGERS)?;
    Ok(())
}

fn migration_0002(conn: &Connection) -> Result<()> {
    // path_entries: add columns for enhanced file metadata
    ensure_column(conn, "path_entries", "content_kind", "TEXT")?;
    ensure_column(conn, "path_entries", "language", "TEXT")?;
    ensure_column(conn, "path_entries", "relative_resource_path", "TEXT")?;
    ensure_column(conn, "path_entries", "repo_root_uri", "TEXT")?;
    ensure_column(conn, "path_entries", "is_text", "INTEGER")?;
    ensure_column(conn, "path_entries", "is_generated", "INTEGER")?;

    // resource_sources: add columns for git/forge provenance
    ensure_column(
        conn,
        "resource_sources",
        "resource_kind",
        "TEXT NOT NULL DEFAULT 'generic_docs'",
    )?;
    ensure_column(conn, "resource_sources", "source_host", "TEXT")?;
    ensure_column(conn, "resource_sources", "source_namespace", "TEXT")?;
    ensure_column(conn, "resource_sources", "source_repo", "TEXT")?;
    ensure_column(conn, "resource_sources", "source_ref", "TEXT")?;
    ensure_column(
        conn,
        "resource_sources",
        "canonical_strategy_version",
        "TEXT NOT NULL DEFAULT 'v2'",
    )?;

    // tasks: add columns for memory pipeline
    ensure_column(conn, "tasks", "attempt_count", "INTEGER NOT NULL DEFAULT 0")?;
    ensure_column(conn, "tasks", "max_attempts", "INTEGER NOT NULL DEFAULT 1")?;
    ensure_column(
        conn,
        "tasks",
        "retry_state",
        "TEXT NOT NULL DEFAULT 'not_needed'",
    )?;
    ensure_column(conn, "tasks", "processing_mode", "TEXT")?;
    ensure_column(conn, "tasks", "scope_type", "TEXT")?;
    ensure_column(conn, "tasks", "scope_id", "TEXT")?;
    ensure_column(conn, "tasks", "range_start_turn_id", "TEXT")?;
    ensure_column(conn, "tasks", "range_end_turn_id", "TEXT")?;
    ensure_column(conn, "tasks", "dedupe_key", "TEXT")?;
    ensure_column(conn, "tasks", "payload_json", "TEXT")?;
    ensure_column(conn, "tasks", "lease_owner", "TEXT")?;
    ensure_column(conn, "tasks", "lease_expires_at", "TEXT")?;
    ensure_column(conn, "tasks", "scheduled_at", "TEXT")?;
    ensure_column(conn, "tasks", "finished_at", "TEXT")?;

    // facts: add columns for memory pipeline
    ensure_column(conn, "facts", "normalized_value_json", "TEXT")?;
    ensure_column(
        conn,
        "facts",
        "value_type",
        "TEXT NOT NULL DEFAULT 'scalar'",
    )?;
    ensure_column(conn, "facts", "valid_from", "TEXT")?;
    ensure_column(conn, "facts", "valid_to", "TEXT")?;
    ensure_column(conn, "facts", "source_assertion_id", "TEXT")?;
    ensure_column(
        conn,
        "facts",
        "updated_at",
        "TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP",
    )?;

    Ok(())
}

fn migration_0006(conn: &Connection) -> Result<()> {
    ensure_column(conn, "facts", "recall_count", "INTEGER NOT NULL DEFAULT 0")?;
    ensure_column(conn, "facts", "last_recalled_at", "TEXT")?;
    Ok(())
}

fn migration_0007(conn: &Connection) -> Result<()> {
    ensure_column(conn, "episode_chunks", "embedding_json", "TEXT")?;
    Ok(())
}

fn get_applied_versions(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT version FROM _schema_migrations")?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    rows.collect()
}

fn mark_applied(conn: &Connection, version: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO _schema_migrations (version) VALUES (?1)",
        [version],
    )?;
    Ok(())
}

fn ensure_column(conn: &Connection, table: &str, column: &str, definition: &str) -> Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    let columns = rows.collect::<Result<Vec<_>, _>>()?;
    if columns.iter().any(|existing| existing == column) {
        return Ok(());
    }

    conn.execute(
        &format!("ALTER TABLE {table} ADD COLUMN {column} {definition}"),
        [],
    )?;
    Ok(())
}

/// Migration 0019: add temporal fields to relations table.
///
/// Adds valid_from, valid_to, tcommit, is_latest, superseded_by columns
/// mirroring the same-table supersession pattern already used by facts.
/// Uses ensure_column for idempotency; backfills existing rows with
/// valid_from = updated_at, valid_to = NULL (open-ended), is_latest = 1.
fn migration_0019(conn: &Connection) -> Result<()> {
    ensure_column(conn, "relations", "valid_from", "TEXT")?;
    ensure_column(conn, "relations", "valid_to", "TEXT")?;
    ensure_column(conn, "relations", "tcommit", "TEXT")?;
    ensure_column(conn, "relations", "is_latest", "INTEGER NOT NULL DEFAULT 1")?;
    ensure_column(conn, "relations", "superseded_by", "TEXT")?;

    // Backfill existing rows: assume valid from creation (updated_at) until now (open-ended).
    conn.execute(
        "UPDATE relations SET
            valid_from = updated_at,
            valid_to = NULL,
            tcommit = updated_at,
            is_latest = 1,
            superseded_by = NULL
         WHERE valid_from IS NULL",
        [],
    )?;

    // Drop the UNIQUE constraint on (account_id, user_id, from_uri, to_uri, relation_type).
    // Same-table supersession requires multiple rows per edge key (v1 superseded + v2 current),
    // so the edge key must no longer be unique.
    conn.execute("DROP INDEX IF EXISTS idx_relations_unique", [])?;

    // Create a non-unique index on the edge key for efficient lookup.
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_relations_edge_key
         ON relations (account_id, user_id, from_uri, to_uri, relation_type)",
        [],
    )?;

    // Create index for AS OF temporal queries on latest edges.
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_relations_temporal_latest
         ON relations (account_id, user_id, is_latest, valid_from, valid_to)",
        [],
    )?;
    Ok(())
}

/// Migration 0020: add last_decay_at to episode_chunks for incremental decay.
///
/// Tracks when the last Ebbinghaus decay cycle wrote to this episode's
/// salience_score, enabling incremental Δt calculation instead of
/// compound decay on total age.
fn migration_0021(conn: &Connection) -> Result<()> {
    ensure_column(conn, "episode_chunks", "last_decay_at", "TEXT")?;
    // Backfill: NULL means "never decayed" → first cycle uses created_at as baseline.
    Ok(())
}
