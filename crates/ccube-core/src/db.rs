use anyhow::Result;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// A row from the events table, for display purposes.
#[derive(Debug, Serialize, Deserialize)]
pub struct EventRow {
    pub id: i64,
    pub ts: i64,
    pub kind: String,
    pub app: Option<String>,
    pub title: Option<String>,
    pub duration_ms: Option<i64>,
    pub mode: Option<String>,
    pub ocr_text: Option<String>,
}

/// A row from the decisions table (detector decisions persisted for correction reference).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionRow {
    pub id: i64,
    pub ts: i64,
    pub trigger: String,
    pub decision: String,
    pub reasoning: String,
    pub nudge_style: Option<String>,
    pub nudge_message: Option<String>,
    pub briefing_json: String,
    pub patterns_hash: String,
    pub prompt_version: String,
    pub duration_ms: i64,
}

/// A row from the corrections table — self-contained with full context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorrectionRow {
    pub id: i64,
    pub ts: i64,
    pub decision_id: i64,
    pub original_decision: String,
    pub user_verdict: String,
    pub ctx_snapshot: String,
    pub patterns_hash: String,
    pub status: String,
}

/// Apply recommended pragmas for concurrent access: WAL mode and busy timeout.
fn apply_pragmas(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA busy_timeout = 5000;",
    )?;
    Ok(())
}

/// Initialize all SQLite databases with their schemas.
pub fn init_databases(data_dir: &Path) -> Result<()> {
    std::fs::create_dir_all(data_dir)?;
    init_events_db(data_dir)?;
    init_corrections_db(data_dir)?;
    init_eval_runs_db(data_dir)?;
    Ok(())
}

/// Open the corrections database (read-only queries).
pub fn open_corrections_db(data_dir: &Path) -> Result<Connection> {
    let conn = Connection::open(data_dir.join("corrections.sqlite"))?;
    apply_pragmas(&conn)?;
    Ok(conn)
}

/// List corrections ordered by timestamp descending.
/// When `pending_only` is true, only corrections with status='pending' are returned.
pub fn list_corrections(
    conn: &Connection,
    limit: i64,
    pending_only: bool,
) -> Result<Vec<CorrectionRow>> {
    let sql = if pending_only {
        "SELECT id, ts, decision_id, original_decision, user_verdict, ctx_snapshot, patterns_hash, status
         FROM corrections WHERE status = 'pending' ORDER BY ts DESC LIMIT ?1"
    } else {
        "SELECT id, ts, decision_id, original_decision, user_verdict, ctx_snapshot, patterns_hash, status
         FROM corrections ORDER BY ts DESC LIMIT ?1"
    };

    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map([limit], |row| {
        Ok(CorrectionRow {
            id: row.get(0)?,
            ts: row.get(1)?,
            decision_id: row.get(2)?,
            original_decision: row.get(3)?,
            user_verdict: row.get(4)?,
            ctx_snapshot: row.get(5)?,
            patterns_hash: row.get(6)?,
            status: row.get(7)?,
        })
    })?;

    let mut results = Vec::new();
    for row in rows {
        results.push(row?);
    }
    Ok(results)
}

/// Get a single correction by ID. Returns None if not found.
pub fn get_correction(conn: &Connection, id: i64) -> Result<Option<CorrectionRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, ts, decision_id, original_decision, user_verdict, ctx_snapshot, patterns_hash, status
         FROM corrections WHERE id = ?1",
    )?;
    let mut rows = stmt.query_map([id], |row| {
        Ok(CorrectionRow {
            id: row.get(0)?,
            ts: row.get(1)?,
            decision_id: row.get(2)?,
            original_decision: row.get(3)?,
            user_verdict: row.get(4)?,
            ctx_snapshot: row.get(5)?,
            patterns_hash: row.get(6)?,
            status: row.get(7)?,
        })
    })?;
    match rows.next() {
        Some(row) => Ok(Some(row?)),
        None => Ok(None),
    }
}

/// Insert a correction. Returns the new correction ID.
/// Timestamp is set to current UTC time; status defaults to "pending".
pub fn insert_correction(
    conn: &Connection,
    decision_id: i64,
    original_decision: &str,
    user_verdict: &str,
    ctx_snapshot: &str,
    patterns_hash: &str,
) -> Result<i64> {
    let ts = chrono::Utc::now().timestamp_millis();
    conn.execute(
        "INSERT INTO corrections (ts, decision_id, original_decision, user_verdict, ctx_snapshot, patterns_hash)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![ts, decision_id, original_decision, user_verdict, ctx_snapshot, patterns_hash],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Open the events database for reading/writing.
pub fn open_events_db(data_dir: &Path) -> Result<Connection> {
    let conn = Connection::open(data_dir.join("events.sqlite"))?;
    apply_pragmas(&conn)?;
    Ok(conn)
}

/// Insert a new event row. Returns the row ID.
pub fn insert_event(
    conn: &Connection,
    ts: i64,
    kind: &str,
    app: Option<&str>,
    title: Option<&str>,
    mode: Option<&str>,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO events (ts, kind, app, title, mode) VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![ts, kind, app, title, mode],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Set the duration_ms on a previously inserted event.
pub fn update_event_duration(conn: &Connection, event_id: i64, duration_ms: i64) -> Result<()> {
    let rows = conn.execute(
        "UPDATE events SET duration_ms = ?1 WHERE id = ?2",
        rusqlite::params![duration_ms, event_id],
    )?;
    if rows == 0 {
        anyhow::bail!("event #{event_id} not found");
    }
    Ok(())
}

/// Set the ocr_text on a previously inserted event (populated by background OCR task).
pub fn update_event_ocr(conn: &Connection, event_id: i64, ocr_text: &str) -> Result<()> {
    let rows = conn.execute(
        "UPDATE events SET ocr_text = ?1 WHERE id = ?2",
        rusqlite::params![ocr_text, event_id],
    )?;
    if rows == 0 {
        anyhow::bail!("event #{event_id} not found");
    }
    Ok(())
}

/// Query events with ts >= since_ts, ordered by ts ascending.
/// Capped at 10,000 rows as a safety bound.
pub fn query_recent_events(conn: &Connection, since_ts: i64) -> Result<Vec<EventRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, ts, kind, app, title, duration_ms, mode, ocr_text
         FROM events WHERE ts >= ?1 ORDER BY ts ASC LIMIT 10000",
    )?;

    let rows = stmt.query_map([since_ts], |row| {
        Ok(EventRow {
            id: row.get(0)?,
            ts: row.get(1)?,
            kind: row.get(2)?,
            app: row.get(3)?,
            title: row.get(4)?,
            duration_ms: row.get(5)?,
            mode: row.get(6)?,
            ocr_text: row.get(7)?,
        })
    })?;

    let mut results = Vec::new();
    for row in rows {
        results.push(row?);
    }
    Ok(results)
}

/// Return the most recent event of a given kind, or None.
pub fn last_event_of_kind(conn: &Connection, kind: &str) -> Result<Option<EventRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, ts, kind, app, title, duration_ms, mode, ocr_text
         FROM events WHERE kind = ?1 ORDER BY ts DESC LIMIT 1",
    )?;
    let mut rows = stmt.query_map([kind], |row| {
        Ok(EventRow {
            id: row.get(0)?,
            ts: row.get(1)?,
            kind: row.get(2)?,
            app: row.get(3)?,
            title: row.get(4)?,
            duration_ms: row.get(5)?,
            mode: row.get(6)?,
            ocr_text: row.get(7)?,
        })
    })?;
    match rows.next() {
        Some(row) => Ok(Some(row?)),
        None => Ok(None),
    }
}

/// Return the most recent event regardless of kind, or None.
pub fn last_event(conn: &Connection) -> Result<Option<EventRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, ts, kind, app, title, duration_ms, mode, ocr_text
         FROM events ORDER BY ts DESC LIMIT 1",
    )?;
    let mut rows = stmt.query_map([], |row| {
        Ok(EventRow {
            id: row.get(0)?,
            ts: row.get(1)?,
            kind: row.get(2)?,
            app: row.get(3)?,
            title: row.get(4)?,
            duration_ms: row.get(5)?,
            mode: row.get(6)?,
            ocr_text: row.get(7)?,
        })
    })?;
    match rows.next() {
        Some(row) => Ok(Some(row?)),
        None => Ok(None),
    }
}

/// Delete events older than before_ts. Returns count of deleted rows.
pub fn prune_events(conn: &Connection, before_ts: i64) -> Result<u64> {
    let deleted = conn.execute(
        "DELETE FROM events WHERE ts < ?1",
        rusqlite::params![before_ts],
    )?;
    Ok(deleted as u64)
}

// ---------------------------------------------------------------------------
// Decisions (Phase 5) — detector decisions persisted with integer IDs
// ---------------------------------------------------------------------------

/// Insert a detector decision. Returns the new decision ID.
#[allow(clippy::too_many_arguments)]
pub fn insert_decision(
    conn: &Connection,
    ts: i64,
    trigger: &str,
    decision: &str,
    reasoning: &str,
    nudge_style: Option<&str>,
    nudge_message: Option<&str>,
    briefing_json: &str,
    patterns_hash: &str,
    prompt_version: &str,
    duration_ms: i64,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO decisions (ts, trigger, decision, reasoning, nudge_style, nudge_message, briefing_json, patterns_hash, prompt_version, duration_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        rusqlite::params![ts, trigger, decision, reasoning, nudge_style, nudge_message, briefing_json, patterns_hash, prompt_version, duration_ms],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Get a single decision by ID. Returns None if not found.
pub fn get_decision(conn: &Connection, id: i64) -> Result<Option<DecisionRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, ts, trigger, decision, reasoning, nudge_style, nudge_message, briefing_json, patterns_hash, prompt_version, duration_ms
         FROM decisions WHERE id = ?1",
    )?;
    let mut rows = stmt.query_map([id], |row| {
        Ok(DecisionRow {
            id: row.get(0)?,
            ts: row.get(1)?,
            trigger: row.get(2)?,
            decision: row.get(3)?,
            reasoning: row.get(4)?,
            nudge_style: row.get(5)?,
            nudge_message: row.get(6)?,
            briefing_json: row.get(7)?,
            patterns_hash: row.get(8)?,
            prompt_version: row.get(9)?,
            duration_ms: row.get(10)?,
        })
    })?;
    match rows.next() {
        Some(row) => Ok(Some(row?)),
        None => Ok(None),
    }
}

/// List decisions with ts >= since_ts, ordered by ts descending.
pub fn list_decisions(conn: &Connection, since_ts: i64, limit: i64) -> Result<Vec<DecisionRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, ts, trigger, decision, reasoning, nudge_style, nudge_message, briefing_json, patterns_hash, prompt_version, duration_ms
         FROM decisions WHERE ts >= ?1 ORDER BY ts DESC LIMIT ?2",
    )?;
    let rows = stmt.query_map(rusqlite::params![since_ts, limit], |row| {
        Ok(DecisionRow {
            id: row.get(0)?,
            ts: row.get(1)?,
            trigger: row.get(2)?,
            decision: row.get(3)?,
            reasoning: row.get(4)?,
            nudge_style: row.get(5)?,
            nudge_message: row.get(6)?,
            briefing_json: row.get(7)?,
            patterns_hash: row.get(8)?,
            prompt_version: row.get(9)?,
            duration_ms: row.get(10)?,
        })
    })?;

    let mut results = Vec::new();
    for row in rows {
        results.push(row?);
    }
    Ok(results)
}

/// Delete decisions older than before_ts. Returns count of deleted rows.
pub fn prune_decisions(conn: &Connection, before_ts: i64) -> Result<u64> {
    let deleted = conn.execute(
        "DELETE FROM decisions WHERE ts < ?1",
        rusqlite::params![before_ts],
    )?;
    Ok(deleted as u64)
}

// ---------------------------------------------------------------------------
// Corrections — status updates + counting (Phase 6)
// ---------------------------------------------------------------------------

/// Update a correction's status. Valid values: "pending", "retained", "discarded", "deferred".
pub fn update_correction_status(conn: &Connection, id: i64, status: &str) -> Result<()> {
    let rows = conn.execute(
        "UPDATE corrections SET status = ?1 WHERE id = ?2",
        rusqlite::params![status, id],
    )?;
    if rows == 0 {
        anyhow::bail!("correction #{id} not found");
    }
    Ok(())
}

/// Count corrections with status='pending'.
pub fn count_pending_corrections(conn: &Connection) -> Result<i64> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM corrections WHERE status = 'pending'",
        [],
        |row| row.get(0),
    )?;
    Ok(count)
}

/// List corrections with status='retained' and ts >= since_ts (for reflector context).
pub fn list_retained_corrections(
    conn: &Connection,
    since_ts: i64,
    limit: i64,
) -> Result<Vec<CorrectionRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, ts, decision_id, original_decision, user_verdict, ctx_snapshot, patterns_hash, status
         FROM corrections WHERE status = 'retained' AND ts >= ?1 ORDER BY ts DESC LIMIT ?2",
    )?;
    let rows = stmt.query_map(rusqlite::params![since_ts, limit], |row| {
        Ok(CorrectionRow {
            id: row.get(0)?,
            ts: row.get(1)?,
            decision_id: row.get(2)?,
            original_decision: row.get(3)?,
            user_verdict: row.get(4)?,
            ctx_snapshot: row.get(5)?,
            patterns_hash: row.get(6)?,
            status: row.get(7)?,
        })
    })?;

    let mut results = Vec::new();
    for row in rows {
        results.push(row?);
    }
    Ok(results)
}

// ---------------------------------------------------------------------------
// Eval runs (Phase 6) — audit trail for curator/reflector eval gate
// ---------------------------------------------------------------------------

/// A row from the eval_runs table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalRunRow {
    pub id: i64,
    pub ts: i64,
    pub triggered_by: String,
    pub patterns_before: String,
    pub patterns_after: String,
    pub events_replayed: i64,
    pub decisions_changed: i64,
    pub regressions: i64,
    pub passed: bool,
    pub rationale: Option<String>,
}

/// Open the eval_runs database for reading/writing.
pub fn open_eval_runs_db(data_dir: &Path) -> Result<Connection> {
    let conn = Connection::open(data_dir.join("eval_runs.sqlite"))?;
    apply_pragmas(&conn)?;
    Ok(conn)
}

/// Insert an eval run. Returns the new row ID.
#[allow(clippy::too_many_arguments)]
pub fn insert_eval_run(
    conn: &Connection,
    ts: i64,
    triggered_by: &str,
    patterns_before: &str,
    patterns_after: &str,
    events_replayed: i64,
    decisions_changed: i64,
    regressions: i64,
    passed: bool,
    rationale: Option<&str>,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO eval_runs (ts, triggered_by, patterns_before, patterns_after, events_replayed, decisions_changed, regressions, passed, rationale)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        rusqlite::params![ts, triggered_by, patterns_before, patterns_after, events_replayed, decisions_changed, regressions, passed as i64, rationale],
    )?;
    Ok(conn.last_insert_rowid())
}

/// List eval runs ordered by timestamp descending.
pub fn list_eval_runs(conn: &Connection, limit: i64) -> Result<Vec<EvalRunRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, ts, triggered_by, patterns_before, patterns_after, events_replayed, decisions_changed, regressions, passed, rationale
         FROM eval_runs ORDER BY ts DESC LIMIT ?1",
    )?;
    let rows = stmt.query_map([limit], |row| {
        let passed_int: i64 = row.get(8)?;
        Ok(EvalRunRow {
            id: row.get(0)?,
            ts: row.get(1)?,
            triggered_by: row.get(2)?,
            patterns_before: row.get(3)?,
            patterns_after: row.get(4)?,
            events_replayed: row.get(5)?,
            decisions_changed: row.get(6)?,
            regressions: row.get(7)?,
            passed: passed_int != 0,
            rationale: row.get(9)?,
        })
    })?;

    let mut results = Vec::new();
    for row in rows {
        results.push(row?);
    }
    Ok(results)
}

fn init_events_db(data_dir: &Path) -> Result<()> {
    let conn = Connection::open(data_dir.join("events.sqlite"))?;
    apply_pragmas(&conn)?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS events (
            id           INTEGER PRIMARY KEY AUTOINCREMENT,
            ts           INTEGER NOT NULL,
            kind         TEXT NOT NULL,
            app          TEXT,
            title        TEXT,
            duration_ms  INTEGER,
            mode         TEXT,
            ocr_text     TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_events_ts ON events(ts);
        CREATE INDEX IF NOT EXISTS idx_events_kind_ts ON events(kind, ts);
        CREATE TABLE IF NOT EXISTS decisions (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            ts              INTEGER NOT NULL,
            trigger         TEXT NOT NULL,
            decision        TEXT NOT NULL,
            reasoning       TEXT NOT NULL,
            nudge_style     TEXT,
            nudge_message   TEXT,
            briefing_json   TEXT NOT NULL,
            patterns_hash   TEXT NOT NULL,
            prompt_version  TEXT NOT NULL,
            duration_ms     INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_decisions_ts ON decisions(ts);",
    )?;
    // Migration: add ocr_text column to existing databases
    conn.execute_batch(
        "ALTER TABLE events ADD COLUMN ocr_text TEXT;",
    ).ok(); // ok() — column already exists on fresh databases
    Ok(())
}

fn init_corrections_db(data_dir: &Path) -> Result<()> {
    let conn = Connection::open(data_dir.join("corrections.sqlite"))?;
    apply_pragmas(&conn)?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS corrections (
            id                 INTEGER PRIMARY KEY AUTOINCREMENT,
            ts                 INTEGER NOT NULL,
            decision_id        INTEGER NOT NULL,
            original_decision  TEXT NOT NULL,
            user_verdict       TEXT NOT NULL,
            ctx_snapshot       TEXT NOT NULL,
            patterns_hash      TEXT NOT NULL,
            status             TEXT NOT NULL DEFAULT 'pending'
        );
        CREATE INDEX IF NOT EXISTS idx_corrections_ts ON corrections(ts);
        CREATE INDEX IF NOT EXISTS idx_corrections_status_ts ON corrections(status, ts);",
    )?;
    // FTS5 virtual table for full-text search on corrections
    conn.execute_batch(
        "CREATE VIRTUAL TABLE IF NOT EXISTS corrections_fts USING fts5(
            user_verdict, ctx_snapshot, content='corrections', content_rowid='id'
        );",
    )?;
    // Triggers to keep FTS5 index in sync with the corrections table
    conn.execute_batch(
        "CREATE TRIGGER IF NOT EXISTS corrections_ai AFTER INSERT ON corrections BEGIN
            INSERT INTO corrections_fts(rowid, user_verdict, ctx_snapshot)
            VALUES (new.id, new.user_verdict, new.ctx_snapshot);
        END;
        CREATE TRIGGER IF NOT EXISTS corrections_ad AFTER DELETE ON corrections BEGIN
            INSERT INTO corrections_fts(corrections_fts, rowid, user_verdict, ctx_snapshot)
            VALUES ('delete', old.id, old.user_verdict, old.ctx_snapshot);
        END;
        CREATE TRIGGER IF NOT EXISTS corrections_au AFTER UPDATE ON corrections BEGIN
            INSERT INTO corrections_fts(corrections_fts, rowid, user_verdict, ctx_snapshot)
            VALUES ('delete', old.id, old.user_verdict, old.ctx_snapshot);
            INSERT INTO corrections_fts(rowid, user_verdict, ctx_snapshot)
            VALUES (new.id, new.user_verdict, new.ctx_snapshot);
        END;",
    )?;
    Ok(())
}

fn init_eval_runs_db(data_dir: &Path) -> Result<()> {
    let conn = Connection::open(data_dir.join("eval_runs.sqlite"))?;
    apply_pragmas(&conn)?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS eval_runs (
            id                 INTEGER PRIMARY KEY AUTOINCREMENT,
            ts                 INTEGER NOT NULL,
            triggered_by       TEXT NOT NULL,
            patterns_before    TEXT NOT NULL,
            patterns_after     TEXT NOT NULL,
            events_replayed    INTEGER NOT NULL,
            decisions_changed  INTEGER NOT NULL,
            regressions        INTEGER NOT NULL,
            passed             INTEGER NOT NULL,
            rationale          TEXT
        );",
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_init_creates_files() {
        let dir = TempDir::new().unwrap();
        init_databases(dir.path()).unwrap();
        assert!(dir.path().join("events.sqlite").exists());
        assert!(dir.path().join("corrections.sqlite").exists());
        assert!(dir.path().join("eval_runs.sqlite").exists());
    }

    #[test]
    fn test_init_idempotent() {
        let dir = TempDir::new().unwrap();
        init_databases(dir.path()).unwrap();
        init_databases(dir.path()).unwrap(); // second call should not error
    }

    #[test]
    fn test_fts5_works() {
        let dir = TempDir::new().unwrap();
        init_databases(dir.path()).unwrap();

        let conn = open_corrections_db(dir.path()).unwrap();
        conn.execute(
            "INSERT INTO corrections (ts, decision_id, original_decision, user_verdict, ctx_snapshot, patterns_hash, status)
             VALUES (1000, 1, 'nudge', 'was not drift', '{}', 'abc123', 'pending')",
            [],
        )
        .unwrap();

        // FTS5 trigger should auto-sync — no manual insert needed

        // Query FTS5
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM corrections_fts WHERE user_verdict MATCH 'drift'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_list_corrections_empty() {
        let dir = TempDir::new().unwrap();
        init_databases(dir.path()).unwrap();
        let conn = open_corrections_db(dir.path()).unwrap();
        let rows = list_corrections(&conn, 20, false).unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn test_list_corrections_returns_rows() {
        let dir = TempDir::new().unwrap();
        init_databases(dir.path()).unwrap();
        let conn = open_corrections_db(dir.path()).unwrap();

        conn.execute(
            "INSERT INTO corrections (ts, decision_id, original_decision, user_verdict, ctx_snapshot, patterns_hash, status)
             VALUES (1000, 1, 'nudge', 'was fine', '{\"ts\":1000}', 'hash1', 'pending')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO corrections (ts, decision_id, original_decision, user_verdict, ctx_snapshot, patterns_hash, status)
             VALUES (2000, 2, 'silent', 'should nudge', '{\"ts\":2000}', 'hash2', 'pending')",
            [],
        )
        .unwrap();

        let rows = list_corrections(&conn, 20, false).unwrap();
        assert_eq!(rows.len(), 2);
        // Ordered by ts DESC, so newest first
        assert_eq!(rows[0].ts, 2000);
        assert_eq!(rows[1].ts, 1000);
        assert_eq!(rows[0].original_decision, "silent");
        assert_eq!(rows[1].user_verdict, "was fine");
        // Verify expanded fields
        assert_eq!(rows[0].decision_id, 2);
        assert_eq!(rows[0].patterns_hash, "hash2");
    }

    #[test]
    fn test_insert_and_query_events() {
        let dir = TempDir::new().unwrap();
        init_databases(dir.path()).unwrap();
        let conn = open_events_db(dir.path()).unwrap();

        let id1 = insert_event(
            &conn,
            1000,
            "app_focus",
            Some("code.exe"),
            Some("main.rs"),
            Some("Coding"),
        )
        .unwrap();
        let id2 = insert_event(
            &conn,
            2000,
            "window_title",
            Some("code.exe"),
            Some("lib.rs"),
            None,
        )
        .unwrap();
        assert!(id1 > 0);
        assert!(id2 > id1);

        let rows = query_recent_events(&conn, 0).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].ts, 1000);
        assert_eq!(rows[0].kind, "app_focus");
        assert_eq!(rows[0].app.as_deref(), Some("code.exe"));
        assert_eq!(rows[0].title.as_deref(), Some("main.rs"));
        assert_eq!(rows[0].mode.as_deref(), Some("Coding"));
        assert!(rows[0].duration_ms.is_none());
        assert_eq!(rows[1].ts, 2000);
    }

    #[test]
    fn test_query_events_respects_since_ts() {
        let dir = TempDir::new().unwrap();
        init_databases(dir.path()).unwrap();
        let conn = open_events_db(dir.path()).unwrap();

        insert_event(&conn, 1000, "app_focus", Some("a"), None, None).unwrap();
        insert_event(&conn, 2000, "app_focus", Some("b"), None, None).unwrap();
        insert_event(&conn, 3000, "app_focus", Some("c"), None, None).unwrap();

        let rows = query_recent_events(&conn, 2000).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].app.as_deref(), Some("b"));
        assert_eq!(rows[1].app.as_deref(), Some("c"));
    }

    #[test]
    fn test_update_event_duration() {
        let dir = TempDir::new().unwrap();
        init_databases(dir.path()).unwrap();
        let conn = open_events_db(dir.path()).unwrap();

        let id = insert_event(&conn, 1000, "app_focus", Some("code.exe"), None, None).unwrap();
        assert!(
            query_recent_events(&conn, 0).unwrap()[0]
                .duration_ms
                .is_none()
        );

        update_event_duration(&conn, id, 5000).unwrap();
        let rows = query_recent_events(&conn, 0).unwrap();
        assert_eq!(rows[0].duration_ms, Some(5000));
    }

    #[test]
    fn test_prune_events() {
        let dir = TempDir::new().unwrap();
        init_databases(dir.path()).unwrap();
        let conn = open_events_db(dir.path()).unwrap();

        insert_event(&conn, 1000, "app_focus", Some("old"), None, None).unwrap();
        insert_event(&conn, 2000, "app_focus", Some("old2"), None, None).unwrap();
        insert_event(&conn, 5000, "app_focus", Some("new"), None, None).unwrap();

        let deleted = prune_events(&conn, 3000).unwrap();
        assert_eq!(deleted, 2);

        let remaining = query_recent_events(&conn, 0).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].app.as_deref(), Some("new"));
    }

    // -----------------------------------------------------------------------
    // Phase 5: Decision + correction CRUD tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_insert_and_get_decision() {
        let dir = TempDir::new().unwrap();
        init_databases(dir.path()).unwrap();
        let conn = open_events_db(dir.path()).unwrap();

        let id = insert_decision(
            &conn,
            5000,
            "focus_change",
            "Nudge",
            "user browsing twitter",
            Some("Gentle"),
            Some("Consider refocusing"),
            r#"{"ts":5000}"#,
            "abc123hash",
            "detector.v1",
            847,
        )
        .unwrap();
        assert!(id > 0);

        let d = get_decision(&conn, id).unwrap().expect("decision not found");
        assert_eq!(d.id, id);
        assert_eq!(d.ts, 5000);
        assert_eq!(d.trigger, "focus_change");
        assert_eq!(d.decision, "Nudge");
        assert_eq!(d.reasoning, "user browsing twitter");
        assert_eq!(d.nudge_style.as_deref(), Some("Gentle"));
        assert_eq!(d.nudge_message.as_deref(), Some("Consider refocusing"));
        assert_eq!(d.briefing_json, r#"{"ts":5000}"#);
        assert_eq!(d.patterns_hash, "abc123hash");
        assert_eq!(d.prompt_version, "detector.v1");
        assert_eq!(d.duration_ms, 847);
    }

    #[test]
    fn test_get_decision_not_found() {
        let dir = TempDir::new().unwrap();
        init_databases(dir.path()).unwrap();
        let conn = open_events_db(dir.path()).unwrap();
        assert!(get_decision(&conn, 99999).unwrap().is_none());
    }

    #[test]
    fn test_list_decisions_since() {
        let dir = TempDir::new().unwrap();
        init_databases(dir.path()).unwrap();
        let conn = open_events_db(dir.path()).unwrap();

        insert_decision(&conn, 1000, "heartbeat", "Silent", "ok", None, None, "{}", "h1", "detector.v1", 100).unwrap();
        insert_decision(&conn, 2000, "focus_change", "Nudge", "drift", Some("Gentle"), Some("hey"), "{}", "h2", "detector.v1", 200).unwrap();
        insert_decision(&conn, 3000, "heartbeat", "Silent", "fine", None, None, "{}", "h3", "detector.v1", 150).unwrap();

        // All since ts=0
        let all = list_decisions(&conn, 0, 100).unwrap();
        assert_eq!(all.len(), 3);
        // DESC order
        assert_eq!(all[0].ts, 3000);
        assert_eq!(all[2].ts, 1000);

        // Since ts=2000
        let recent = list_decisions(&conn, 2000, 100).unwrap();
        assert_eq!(recent.len(), 2);

        // Limit
        let limited = list_decisions(&conn, 0, 1).unwrap();
        assert_eq!(limited.len(), 1);
        assert_eq!(limited[0].ts, 3000);
    }

    #[test]
    fn test_insert_correction_full() {
        let dir = TempDir::new().unwrap();
        init_databases(dir.path()).unwrap();

        let corr_conn = open_corrections_db(dir.path()).unwrap();
        let briefing = r#"{"ts":5000,"right_now":{"app":"chrome.exe"}}"#;

        let corr_id = insert_correction(
            &corr_conn,
            42,
            "Nudge",
            "wasn't drift, was researching",
            briefing,
            "abc123hash",
        )
        .unwrap();
        assert!(corr_id > 0);

        let c = get_correction(&corr_conn, corr_id).unwrap().expect("correction not found");
        assert_eq!(c.id, corr_id);
        assert_eq!(c.decision_id, 42);
        assert_eq!(c.original_decision, "Nudge");
        assert_eq!(c.user_verdict, "wasn't drift, was researching");
        assert_eq!(c.ctx_snapshot, briefing);
        assert_eq!(c.patterns_hash, "abc123hash");
        assert_eq!(c.status, "pending");
        assert!(c.ts > 0); // auto-set
    }

    #[test]
    fn test_correction_fts_via_insert_fn() {
        let dir = TempDir::new().unwrap();
        init_databases(dir.path()).unwrap();
        let conn = open_corrections_db(dir.path()).unwrap();

        insert_correction(
            &conn,
            1,
            "Nudge",
            "was not drift, I was researching quantum computing",
            r#"{"ts":1000}"#,
            "hash_abc",
        )
        .unwrap();

        // FTS5 triggers should have auto-synced
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM corrections_fts WHERE user_verdict MATCH 'quantum'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_list_corrections_pending_filter() {
        let dir = TempDir::new().unwrap();
        init_databases(dir.path()).unwrap();
        let conn = open_corrections_db(dir.path()).unwrap();

        conn.execute(
            "INSERT INTO corrections (ts, decision_id, original_decision, user_verdict, ctx_snapshot, patterns_hash, status)
             VALUES (1000, 1, 'nudge', 'fine', '{}', 'h1', 'pending')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO corrections (ts, decision_id, original_decision, user_verdict, ctx_snapshot, patterns_hash, status)
             VALUES (2000, 2, 'nudge', 'wrong', '{}', 'h2', 'retained')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO corrections (ts, decision_id, original_decision, user_verdict, ctx_snapshot, patterns_hash, status)
             VALUES (3000, 3, 'silent', 'should nudge', '{}', 'h3', 'pending')",
            [],
        ).unwrap();

        let all = list_corrections(&conn, 50, false).unwrap();
        assert_eq!(all.len(), 3);

        let pending = list_corrections(&conn, 50, true).unwrap();
        assert_eq!(pending.len(), 2);
        assert!(pending.iter().all(|c| c.status == "pending"));
    }

    #[test]
    fn test_get_correction_not_found() {
        let dir = TempDir::new().unwrap();
        init_databases(dir.path()).unwrap();
        let conn = open_corrections_db(dir.path()).unwrap();
        assert!(get_correction(&conn, 99999).unwrap().is_none());
    }

    #[test]
    fn test_prune_decisions() {
        let dir = TempDir::new().unwrap();
        init_databases(dir.path()).unwrap();
        let conn = open_events_db(dir.path()).unwrap();

        insert_decision(&conn, 1000, "heartbeat", "Silent", "ok", None, None, "{}", "h1", "detector.v1", 100).unwrap();
        insert_decision(&conn, 2000, "heartbeat", "Silent", "ok", None, None, "{}", "h2", "detector.v1", 100).unwrap();
        insert_decision(&conn, 5000, "heartbeat", "Silent", "ok", None, None, "{}", "h3", "detector.v1", 100).unwrap();

        let deleted = prune_decisions(&conn, 3000).unwrap();
        assert_eq!(deleted, 2);

        let remaining = list_decisions(&conn, 0, 100).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].ts, 5000);
    }

    #[test]
    fn test_last_event_of_kind() {
        let dir = TempDir::new().unwrap();
        init_databases(dir.path()).unwrap();
        let conn = open_events_db(dir.path()).unwrap();

        // Empty DB
        assert!(last_event_of_kind(&conn, "daemon_start").unwrap().is_none());

        insert_event(&conn, 1000, "app_focus", Some("Code.exe"), Some("main.rs"), None).unwrap();
        insert_event(&conn, 2000, "daemon_start", None, None, None).unwrap();
        insert_event(&conn, 3000, "app_focus", Some("chrome.exe"), Some("Google"), None).unwrap();
        insert_event(&conn, 4000, "daemon_stop", None, None, None).unwrap();

        let ds = last_event_of_kind(&conn, "daemon_start").unwrap().unwrap();
        assert_eq!(ds.ts, 2000);
        assert_eq!(ds.kind, "daemon_start");

        let af = last_event_of_kind(&conn, "app_focus").unwrap().unwrap();
        assert_eq!(af.ts, 3000);
        assert_eq!(af.app.as_deref(), Some("chrome.exe"));
    }

    #[test]
    fn test_last_event() {
        let dir = TempDir::new().unwrap();
        init_databases(dir.path()).unwrap();
        let conn = open_events_db(dir.path()).unwrap();

        assert!(last_event(&conn).unwrap().is_none());

        insert_event(&conn, 1000, "app_focus", Some("Code.exe"), None, None).unwrap();
        insert_event(&conn, 2000, "daemon_stop", None, None, None).unwrap();

        let le = last_event(&conn).unwrap().unwrap();
        assert_eq!(le.ts, 2000);
        assert_eq!(le.kind, "daemon_stop");
    }

    // -----------------------------------------------------------------------
    // Phase 6: update_correction_status, count_pending, eval_runs CRUD
    // -----------------------------------------------------------------------

    #[test]
    fn test_update_correction_status() {
        let dir = TempDir::new().unwrap();
        init_databases(dir.path()).unwrap();
        let conn = open_corrections_db(dir.path()).unwrap();

        let id = insert_correction(&conn, 1, "Nudge", "was fine", "{}", "h1").unwrap();
        let c = get_correction(&conn, id).unwrap().unwrap();
        assert_eq!(c.status, "pending");

        update_correction_status(&conn, id, "retained").unwrap();
        let c = get_correction(&conn, id).unwrap().unwrap();
        assert_eq!(c.status, "retained");

        update_correction_status(&conn, id, "discarded").unwrap();
        let c = get_correction(&conn, id).unwrap().unwrap();
        assert_eq!(c.status, "discarded");
    }

    #[test]
    fn test_update_correction_status_not_found() {
        let dir = TempDir::new().unwrap();
        init_databases(dir.path()).unwrap();
        let conn = open_corrections_db(dir.path()).unwrap();

        let result = update_correction_status(&conn, 99999, "retained");
        assert!(result.is_err());
    }

    #[test]
    fn test_count_pending_corrections() {
        let dir = TempDir::new().unwrap();
        init_databases(dir.path()).unwrap();
        let conn = open_corrections_db(dir.path()).unwrap();

        assert_eq!(count_pending_corrections(&conn).unwrap(), 0);

        insert_correction(&conn, 1, "Nudge", "wrong", "{}", "h1").unwrap();
        insert_correction(&conn, 2, "Silent", "should nudge", "{}", "h2").unwrap();
        assert_eq!(count_pending_corrections(&conn).unwrap(), 2);

        // Mark one as retained — count should drop
        let rows = list_corrections(&conn, 10, false).unwrap();
        update_correction_status(&conn, rows[0].id, "retained").unwrap();
        assert_eq!(count_pending_corrections(&conn).unwrap(), 1);
    }

    #[test]
    fn test_list_retained_corrections() {
        let dir = TempDir::new().unwrap();
        init_databases(dir.path()).unwrap();
        let conn = open_corrections_db(dir.path()).unwrap();

        // Insert corrections with various statuses via raw SQL to control ts
        conn.execute(
            "INSERT INTO corrections (ts, decision_id, original_decision, user_verdict, ctx_snapshot, patterns_hash, status)
             VALUES (1000, 1, 'Nudge', 'fine', '{}', 'h1', 'pending')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO corrections (ts, decision_id, original_decision, user_verdict, ctx_snapshot, patterns_hash, status)
             VALUES (2000, 2, 'Nudge', 'was researching', '{}', 'h2', 'retained')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO corrections (ts, decision_id, original_decision, user_verdict, ctx_snapshot, patterns_hash, status)
             VALUES (3000, 3, 'Silent', 'should nudge', '{}', 'h3', 'retained')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO corrections (ts, decision_id, original_decision, user_verdict, ctx_snapshot, patterns_hash, status)
             VALUES (4000, 4, 'Nudge', 'ok', '{}', 'h4', 'discarded')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO corrections (ts, decision_id, original_decision, user_verdict, ctx_snapshot, patterns_hash, status)
             VALUES (500, 5, 'Nudge', 'old retained', '{}', 'h5', 'retained')",
            [],
        ).unwrap();

        // All retained: should get 3 (ids 2, 3, 5)
        let all = list_retained_corrections(&conn, 0, 100).unwrap();
        assert_eq!(all.len(), 3);
        assert!(all.iter().all(|c| c.status == "retained"));

        // Retained since ts=1500: should get 2 (ids 2, 3), not id 5 (ts=500)
        let recent = list_retained_corrections(&conn, 1500, 100).unwrap();
        assert_eq!(recent.len(), 2);
        // DESC order: ts 3000 first
        assert_eq!(recent[0].ts, 3000);
        assert_eq!(recent[1].ts, 2000);

        // Limit
        let limited = list_retained_corrections(&conn, 0, 1).unwrap();
        assert_eq!(limited.len(), 1);
    }

    #[test]
    fn test_insert_and_list_eval_runs() {
        let dir = TempDir::new().unwrap();
        init_databases(dir.path()).unwrap();
        let conn = open_eval_runs_db(dir.path()).unwrap();

        let id1 = insert_eval_run(
            &conn, 1000, "curator", "old patterns", "new patterns",
            50, 3, 0, true, Some("all good"),
        ).unwrap();
        let id2 = insert_eval_run(
            &conn, 2000, "curator", "patterns v2", "patterns v3",
            80, 5, 2, false, Some("2 regressions found"),
        ).unwrap();
        assert!(id1 > 0);
        assert!(id2 > id1);

        let runs = list_eval_runs(&conn, 10).unwrap();
        assert_eq!(runs.len(), 2);
        // DESC order
        assert_eq!(runs[0].ts, 2000);
        assert_eq!(runs[0].triggered_by, "curator");
        assert_eq!(runs[0].events_replayed, 80);
        assert_eq!(runs[0].decisions_changed, 5);
        assert_eq!(runs[0].regressions, 2);
        assert!(!runs[0].passed);
        assert_eq!(runs[0].rationale.as_deref(), Some("2 regressions found"));

        assert_eq!(runs[1].ts, 1000);
        assert!(runs[1].passed);
    }

    #[test]
    fn test_list_eval_runs_respects_limit() {
        let dir = TempDir::new().unwrap();
        init_databases(dir.path()).unwrap();
        let conn = open_eval_runs_db(dir.path()).unwrap();

        for i in 0..5 {
            insert_eval_run(
                &conn, 1000 + i, "curator", "a", "b", 10, 1, 0, true, None,
            ).unwrap();
        }

        let runs = list_eval_runs(&conn, 2).unwrap();
        assert_eq!(runs.len(), 2);
    }
}
