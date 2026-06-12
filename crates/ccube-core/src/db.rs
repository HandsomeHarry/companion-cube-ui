use anyhow::Result;
pub use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// A row from the events table, for display purposes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventRow {
    pub id: i64,
    pub ts: i64,
    pub kind: String,
    pub app: Option<String>,
    pub title: Option<String>,
    pub duration_ms: Option<i64>,
    pub mode: Option<String>,
    pub ocr_text: Option<String>,
    pub vision_desc: Option<String>,
    /// Stable session membership (sessions table); None = not yet organized.
    pub session_id: Option<i64>,
    /// Per-event description written by the summarize LLM pass.
    pub llm_desc: Option<String>,
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
    init_summaries_db(data_dir)?;
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

/// Set both ocr_text and mode on a previously inserted event.
/// Used when OCR text arrives after the initial insert and enables re-inferred mode.
pub fn update_event_ocr_and_mode(conn: &Connection, event_id: i64, ocr_text: &str, mode: &str) -> Result<()> {
    let rows = conn.execute(
        "UPDATE events SET ocr_text = ?1, mode = ?2 WHERE id = ?3",
        rusqlite::params![ocr_text, mode, event_id],
    )?;
    if rows == 0 {
        anyhow::bail!("event #{event_id} not found");
    }
    Ok(())
}

/// Set the vision_desc on a previously inserted event (populated by vision model classification).
pub fn update_event_vision(conn: &Connection, event_id: i64, vision_desc: &str) -> Result<()> {
    let rows = conn.execute(
        "UPDATE events SET vision_desc = ?1 WHERE id = ?2",
        rusqlite::params![vision_desc, event_id],
    )?;
    if rows == 0 {
        anyhow::bail!("event #{event_id} not found");
    }
    Ok(())
}

/// Shared SELECT column list and row mapper for EventRow queries.
const EVENT_COLS: &str =
    "id, ts, kind, app, title, duration_ms, mode, ocr_text, vision_desc, session_id, llm_desc";

fn map_event_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<EventRow> {
    Ok(EventRow {
        id: row.get(0)?,
        ts: row.get(1)?,
        kind: row.get(2)?,
        app: row.get(3)?,
        title: row.get(4)?,
        duration_ms: row.get(5)?,
        mode: row.get(6)?,
        ocr_text: row.get(7)?,
        vision_desc: row.get(8)?,
        session_id: row.get(9)?,
        llm_desc: row.get(10)?,
    })
}

/// Query events with ts >= since_ts, ordered by ts ascending.
/// Capped at 10,000 rows as a safety bound.
pub fn query_recent_events(conn: &Connection, since_ts: i64) -> Result<Vec<EventRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {EVENT_COLS} FROM events WHERE ts >= ?1 ORDER BY ts ASC LIMIT 10000",
    ))?;
    let rows = stmt.query_map([since_ts], map_event_row)?;

    let mut results = Vec::new();
    for row in rows {
        results.push(row?);
    }
    Ok(results)
}

/// Return the most recent event of a given kind, or None.
pub fn last_event_of_kind(conn: &Connection, kind: &str) -> Result<Option<EventRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {EVENT_COLS} FROM events WHERE kind = ?1 ORDER BY ts DESC LIMIT 1",
    ))?;
    let mut rows = stmt.query_map([kind], map_event_row)?;
    match rows.next() {
        Some(row) => Ok(Some(row?)),
        None => Ok(None),
    }
}

/// Fetch a single event by ID.
pub fn get_event(conn: &Connection, id: i64) -> Result<Option<EventRow>> {
    let mut stmt =
        conn.prepare(&format!("SELECT {EVENT_COLS} FROM events WHERE id = ?1"))?;
    let mut rows = stmt.query_map([id], map_event_row)?;
    match rows.next() {
        Some(row) => Ok(Some(row?)),
        None => Ok(None),
    }
}

/// Return the most recent event regardless of kind, or None.
pub fn last_event(conn: &Connection) -> Result<Option<EventRow>> {
    let mut stmt =
        conn.prepare(&format!("SELECT {EVENT_COLS} FROM events ORDER BY ts DESC LIMIT 1"))?;
    let mut rows = stmt.query_map([], map_event_row)?;
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
    // Unpinned sessions whose events were all pruned are dead weight.
    // Pinned ones are kept even when empty — same policy as
    // refresh_session_bounds: the user made them, only the user (or a
    // reorganize they trigger) removes them.
    conn.execute(
        "DELETE FROM sessions WHERE pinned = 0 AND id NOT IN
           (SELECT DISTINCT session_id FROM events WHERE session_id IS NOT NULL)",
        [],
    )?;
    Ok(deleted as u64)
}

// ---------------------------------------------------------------------------
// Sessions — stable activity groups (LLM-proposed, user-correctable)
// ---------------------------------------------------------------------------

/// A session row: a named group of events with stable identity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRow {
    pub id: i64,
    pub range_key: String,
    pub label: String,
    pub start_ts: i64,
    pub end_ts: i64,
    pub distraction: bool,
    pub pinned: bool,
    pub created_by: String,
    /// Still absorbing new events. At most one session per range is open;
    /// solidified (closed) sessions are never auto-modified again.
    pub open: bool,
}

/// Create a session and return its ID.
#[allow(clippy::too_many_arguments)]
pub fn create_session(
    conn: &Connection,
    range_key: &str,
    label: &str,
    start_ts: i64,
    end_ts: i64,
    distraction: bool,
    created_by: &str,
    open: bool,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO sessions (range_key, label, start_ts, end_ts, distraction, pinned, created_by, open)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![
            range_key,
            label,
            start_ts,
            end_ts,
            distraction,
            created_by == "user", // user-created sessions start pinned
            created_by,
            open
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// The session currently absorbing new events for a range, if any.
pub fn get_open_session(conn: &Connection, range_key: &str) -> Result<Option<SessionRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {SESSION_COLS} FROM sessions
         WHERE range_key = ?1 AND open = 1 ORDER BY end_ts DESC LIMIT 1",
    ))?;
    let mut rows = stmt.query_map([range_key], map_session_row)?;
    match rows.next() {
        Some(row) => Ok(Some(row?)),
        None => Ok(None),
    }
}

/// Solidify a session: it stops absorbing events and (unless the user named
/// it — pinned) takes its final context label. Closed sessions are never
/// auto-modified again.
pub fn close_session(conn: &Connection, id: i64, final_label: Option<&str>) -> Result<()> {
    conn.execute("UPDATE sessions SET open = 0 WHERE id = ?1", [id])?;
    if let Some(label) = final_label {
        conn.execute(
            "UPDATE sessions SET label = ?2 WHERE id = ?1 AND pinned = 0",
            rusqlite::params![id, label],
        )?;
    }
    Ok(())
}

/// Refresh an open session's evolving label (skipped for pinned sessions —
/// the user's name is frozen).
pub fn update_open_label(conn: &Connection, id: i64, label: &str) -> Result<()> {
    conn.execute(
        "UPDATE sessions SET label = ?2 WHERE id = ?1 AND pinned = 0 AND open = 1",
        rusqlite::params![id, label],
    )?;
    Ok(())
}

/// Close any open session that belongs to a different range (day rollover):
/// yesterday's open session must not absorb today's events.
pub fn close_stale_open_sessions(conn: &Connection, current_range_key: &str) -> Result<u64> {
    let n = conn.execute(
        "UPDATE sessions SET open = 0 WHERE open = 1 AND range_key != ?1",
        [current_range_key],
    )?;
    Ok(n as u64)
}

const SESSION_COLS: &str =
    "id, range_key, label, start_ts, end_ts, distraction, pinned, created_by, open";

fn map_session_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionRow> {
    Ok(SessionRow {
        id: row.get(0)?,
        range_key: row.get(1)?,
        label: row.get(2)?,
        start_ts: row.get(3)?,
        end_ts: row.get(4)?,
        distraction: row.get(5)?,
        pinned: row.get(6)?,
        created_by: row.get(7)?,
        open: row.get(8)?,
    })
}

/// List sessions for a range key, newest first.
pub fn list_sessions(conn: &Connection, range_key: &str) -> Result<Vec<SessionRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {SESSION_COLS} FROM sessions WHERE range_key = ?1 ORDER BY end_ts DESC",
    ))?;
    let rows = stmt.query_map([range_key], map_session_row)?;
    let mut results = Vec::new();
    for row in rows {
        results.push(row?);
    }
    Ok(results)
}

/// Fetch a single session by ID.
pub fn get_session(conn: &Connection, id: i64) -> Result<Option<SessionRow>> {
    let mut stmt =
        conn.prepare(&format!("SELECT {SESSION_COLS} FROM sessions WHERE id = ?1"))?;
    let mut rows = stmt.query_map([id], map_session_row)?;
    match rows.next() {
        Some(row) => Ok(Some(row?)),
        None => Ok(None),
    }
}

/// Rename a session. A rename is a user decision, so the session pins.
pub fn rename_session(conn: &Connection, id: i64, label: &str) -> Result<bool> {
    let n = conn.execute(
        "UPDATE sessions SET label = ?2, pinned = 1 WHERE id = ?1",
        rusqlite::params![id, label],
    )?;
    Ok(n > 0)
}

/// Pin or unpin a session.
pub fn set_session_pinned(conn: &Connection, id: i64, pinned: bool) -> Result<()> {
    conn.execute(
        "UPDATE sessions SET pinned = ?2 WHERE id = ?1",
        rusqlite::params![id, pinned],
    )?;
    Ok(())
}

/// Assign an event to a session (None detaches it back to ungrouped) and
/// refresh both affected sessions' time bounds from their member events.
/// Empty unpinned sessions left behind are deleted; empty pinned sessions
/// are kept (the user made them, the user can delete them via reorganize).
pub fn assign_event_session(
    conn: &Connection,
    event_id: i64,
    session_id: Option<i64>,
) -> Result<()> {
    let prev: Option<i64> = conn
        .query_row(
            "SELECT session_id FROM events WHERE id = ?1",
            [event_id],
            |r| r.get(0),
        )
        .unwrap_or(None);

    conn.execute(
        "UPDATE events SET session_id = ?2 WHERE id = ?1",
        rusqlite::params![event_id, session_id],
    )?;

    for sid in [prev, session_id].into_iter().flatten() {
        refresh_session_bounds(conn, sid)?;
    }
    Ok(())
}

/// Recompute a session's start/end from its member events; delete it if it
/// has no members and is not pinned.
fn refresh_session_bounds(conn: &Connection, session_id: i64) -> Result<()> {
    let bounds: Option<(i64, i64)> = conn
        .query_row(
            "SELECT MIN(ts), MAX(ts) FROM events WHERE session_id = ?1",
            [session_id],
            |r| {
                let lo: Option<i64> = r.get(0)?;
                let hi: Option<i64> = r.get(1)?;
                Ok(lo.zip(hi))
            },
        )
        .unwrap_or(None);

    match bounds {
        Some((lo, hi)) => {
            conn.execute(
                "UPDATE sessions SET start_ts = ?2, end_ts = ?3 WHERE id = ?1",
                rusqlite::params![session_id, lo, hi],
            )?;
        }
        None => {
            conn.execute(
                "DELETE FROM sessions WHERE id = ?1 AND pinned = 0",
                [session_id],
            )?;
        }
    }
    Ok(())
}

/// Events belonging to a session, oldest first.
pub fn query_events_by_session(conn: &Connection, session_id: i64) -> Result<Vec<EventRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {EVENT_COLS} FROM events WHERE session_id = ?1 ORDER BY ts ASC",
    ))?;
    let rows = stmt.query_map([session_id], map_event_row)?;
    let mut results = Vec::new();
    for row in rows {
        results.push(row?);
    }
    Ok(results)
}

/// Store the summarize LLM's per-event description.
pub fn update_event_llm_desc(conn: &Connection, event_id: i64, desc: &str) -> Result<()> {
    conn.execute(
        "UPDATE events SET llm_desc = ?2 WHERE id = ?1",
        rusqlite::params![event_id, desc],
    )?;
    Ok(())
}

/// Delete unpinned sessions in a range and detach their events, so a full
/// organize pass can regroup them. Pinned sessions and their events are
/// untouched. Returns the number of sessions deleted.
pub fn clear_unpinned_sessions(conn: &Connection, range_key: &str) -> Result<u64> {
    conn.execute(
        "UPDATE events SET session_id = NULL WHERE session_id IN
           (SELECT id FROM sessions WHERE range_key = ?1 AND pinned = 0)",
        [range_key],
    )?;
    let n = conn.execute(
        "DELETE FROM sessions WHERE range_key = ?1 AND pinned = 0",
        [range_key],
    )?;
    Ok(n as u64)
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

// ---------------------------------------------------------------------------
// Session summaries — persisted LLM grouping results keyed by date range
// ---------------------------------------------------------------------------

/// Open the summaries database for reading/writing.
pub fn open_summaries_db(data_dir: &Path) -> Result<Connection> {
    let conn = Connection::open(data_dir.join("summaries.sqlite"))?;
    apply_pragmas(&conn)?;
    Ok(conn)
}

/// Upsert a summary for a given range_key. Replaces any existing entry.
pub fn upsert_summary(
    conn: &Connection,
    range_key: &str,
    since_ms: i64,
    until_ms: i64,
    generated_at: i64,
    groups_json: &str,
) -> Result<()> {
    conn.execute(
        "INSERT INTO summaries (range_key, since_ms, until_ms, generated_at, groups_json)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(range_key) DO UPDATE SET
           since_ms = excluded.since_ms,
           until_ms = excluded.until_ms,
           generated_at = excluded.generated_at,
           groups_json = excluded.groups_json",
        rusqlite::params![range_key, since_ms, until_ms, generated_at, groups_json],
    )?;
    Ok(())
}

/// Get a summary by range_key. Returns None if not found.
pub fn get_summary(conn: &Connection, range_key: &str) -> Result<Option<(i64, String)>> {
    let mut stmt = conn.prepare(
        "SELECT generated_at, groups_json FROM summaries WHERE range_key = ?1",
    )?;
    let mut rows = stmt.query_map([range_key], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
    })?;
    match rows.next() {
        Some(row) => Ok(Some(row?)),
        None => Ok(None),
    }
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
    // Sessions: LLM- or user-created activity groups with stable identity.
    // `pinned` marks sessions the user has touched — organize passes must
    // never modify them. Lives in events.sqlite so events.session_id is local.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS sessions (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            range_key   TEXT NOT NULL,
            label       TEXT NOT NULL,
            start_ts    INTEGER NOT NULL,
            end_ts      INTEGER NOT NULL,
            distraction INTEGER NOT NULL DEFAULT 0,
            pinned      INTEGER NOT NULL DEFAULT 0,
            created_by  TEXT NOT NULL DEFAULT 'llm'
        );
        CREATE INDEX IF NOT EXISTS idx_sessions_range ON sessions(range_key);",
    )?;
    // Migration: add ocr_text column to existing databases
    conn.execute_batch(
        "ALTER TABLE events ADD COLUMN ocr_text TEXT;",
    ).ok(); // ok() — column already exists on fresh databases
    // Migration: add vision_desc column (safe to run repeatedly)
    let _ = conn.execute("ALTER TABLE events ADD COLUMN vision_desc TEXT", []);
    // Migration: session membership + per-event LLM description
    let _ = conn.execute("ALTER TABLE events ADD COLUMN session_id INTEGER", []);
    let _ = conn.execute("ALTER TABLE events ADD COLUMN llm_desc TEXT", []);
    // Migration: open-session model — the newest session of a day stays open
    // (absorbing new events) until a break or topic change solidifies it.
    let _ = conn.execute("ALTER TABLE sessions ADD COLUMN open INTEGER NOT NULL DEFAULT 0", []);
    // After the ALTER (the column must exist before it can be indexed):
    // query_events_by_session and refresh_session_bounds hit this on every
    // summaries poll and every assignment.
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_events_session ON events(session_id)",
        [],
    )?;
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

fn init_summaries_db(data_dir: &Path) -> Result<()> {
    let conn = Connection::open(data_dir.join("summaries.sqlite"))?;
    apply_pragmas(&conn)?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS summaries (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            range_key       TEXT NOT NULL UNIQUE,
            since_ms        INTEGER NOT NULL,
            until_ms        INTEGER NOT NULL,
            generated_at    INTEGER NOT NULL,
            groups_json     TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_summaries_range ON summaries(range_key);",
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
    fn test_session_crud_and_pinning() {
        let dir = TempDir::new().unwrap();
        init_databases(dir.path()).unwrap();
        let conn = open_events_db(dir.path()).unwrap();

        let e1 = insert_event(&conn, 1000, "app_focus", Some("iTerm2"), None, None).unwrap();
        let e2 = insert_event(&conn, 2000, "app_focus", Some("Brave"), None, None).unwrap();

        let sid = create_session(&conn, "day:2026-06-11", "Coding", 1000, 2000, false, "llm", false).unwrap();
        assign_event_session(&conn, e1, Some(sid)).unwrap();
        assign_event_session(&conn, e2, Some(sid)).unwrap();

        let sessions = list_sessions(&conn, "day:2026-06-11").unwrap();
        assert_eq!(sessions.len(), 1);
        assert!(!sessions[0].pinned);
        assert_eq!(query_events_by_session(&conn, sid).unwrap().len(), 2);

        // Rename pins
        assert!(rename_session(&conn, sid, "Terminal work").unwrap());
        let s = get_session(&conn, sid).unwrap().unwrap();
        assert!(s.pinned);
        assert_eq!(s.label, "Terminal work");

        // Pinned sessions survive clear_unpinned_sessions
        assert_eq!(clear_unpinned_sessions(&conn, "day:2026-06-11").unwrap(), 0);
        assert!(get_session(&conn, sid).unwrap().is_some());
    }

    #[test]
    fn test_detach_deletes_empty_unpinned_session() {
        let dir = TempDir::new().unwrap();
        init_databases(dir.path()).unwrap();
        let conn = open_events_db(dir.path()).unwrap();

        let e1 = insert_event(&conn, 1000, "app_focus", Some("iTerm2"), None, None).unwrap();
        let sid = create_session(&conn, "day:2026-06-11", "Coding", 1000, 1000, false, "llm", false).unwrap();
        assign_event_session(&conn, e1, Some(sid)).unwrap();

        // Detach the only event — unpinned session should be auto-deleted
        assign_event_session(&conn, e1, None).unwrap();
        assert!(get_session(&conn, sid).unwrap().is_none());

        let ev = query_recent_events(&conn, 0).unwrap();
        assert_eq!(ev[0].session_id, None);
    }

    #[test]
    fn test_bounds_refresh_on_move() {
        let dir = TempDir::new().unwrap();
        init_databases(dir.path()).unwrap();
        let conn = open_events_db(dir.path()).unwrap();

        let e1 = insert_event(&conn, 1000, "app_focus", Some("A"), None, None).unwrap();
        let e2 = insert_event(&conn, 9000, "app_focus", Some("B"), None, None).unwrap();
        let s1 = create_session(&conn, "day:x", "One", 0, 0, false, "llm", false).unwrap();
        let s2 = create_session(&conn, "day:x", "Two", 0, 0, false, "user", false).unwrap();
        assign_event_session(&conn, e1, Some(s1)).unwrap();
        assign_event_session(&conn, e2, Some(s1)).unwrap();
        assert_eq!(get_session(&conn, s1).unwrap().unwrap().end_ts, 9000);

        // user-created session is born pinned
        assert!(get_session(&conn, s2).unwrap().unwrap().pinned);

        // Move the later event over; s1's bounds shrink, s2's grow
        assign_event_session(&conn, e2, Some(s2)).unwrap();
        assert_eq!(get_session(&conn, s1).unwrap().unwrap().end_ts, 1000);
        assert_eq!(get_session(&conn, s2).unwrap().unwrap().start_ts, 9000);
    }

    #[test]
    fn test_open_session_lifecycle() {
        let dir = TempDir::new().unwrap();
        init_databases(dir.path()).unwrap();
        let conn = open_events_db(dir.path()).unwrap();

        let sid = create_session(&conn, "day:a", "working", 1000, 2000, false, "llm", true).unwrap();
        assert_eq!(get_open_session(&conn, "day:a").unwrap().unwrap().id, sid);
        assert!(get_open_session(&conn, "day:b").unwrap().is_none());

        // Evolving label refresh works while open + unpinned
        update_open_label(&conn, sid, "terminal — working on companion cube").unwrap();
        assert_eq!(
            get_session(&conn, sid).unwrap().unwrap().label,
            "terminal — working on companion cube"
        );

        // Day rollover closes it
        assert_eq!(close_stale_open_sessions(&conn, "day:b").unwrap(), 1);
        assert!(get_open_session(&conn, "day:a").unwrap().is_none());
    }

    #[test]
    fn test_close_respects_pinned_label() {
        let dir = TempDir::new().unwrap();
        init_databases(dir.path()).unwrap();
        let conn = open_events_db(dir.path()).unwrap();

        let sid = create_session(&conn, "day:a", "llm label", 1000, 2000, false, "llm", true).unwrap();
        rename_session(&conn, sid, "my name").unwrap(); // pins, stays open

        // Label refresh and final label must both respect the user's name
        update_open_label(&conn, sid, "llm rewrite").unwrap();
        close_session(&conn, sid, Some("llm final")).unwrap();
        let s = get_session(&conn, sid).unwrap().unwrap();
        assert_eq!(s.label, "my name");
        assert!(!s.open);
    }

    #[test]
    fn test_prune_keeps_pinned_sessions() {
        let dir = TempDir::new().unwrap();
        init_databases(dir.path()).unwrap();
        let conn = open_events_db(dir.path()).unwrap();

        // Old event in a pinned session, old event in an unpinned one
        let e1 = insert_event(&conn, 1000, "app_focus", Some("A"), None, None).unwrap();
        let e2 = insert_event(&conn, 2000, "app_focus", Some("B"), None, None).unwrap();
        let pinned = create_session(&conn, "day:x", "Mine", 1000, 1000, false, "user", false).unwrap();
        let unpinned = create_session(&conn, "day:x", "LLM", 2000, 2000, false, "llm", false).unwrap();
        assign_event_session(&conn, e1, Some(pinned)).unwrap();
        assign_event_session(&conn, e2, Some(unpinned)).unwrap();

        // Prune everything — both sessions are now empty
        assert_eq!(prune_events(&conn, 10_000).unwrap(), 2);
        // The user's session survives; the LLM's is dead weight and goes
        assert!(get_session(&conn, pinned).unwrap().is_some());
        assert!(get_session(&conn, unpinned).unwrap().is_none());
    }

    #[test]
    fn test_get_event_by_id() {
        let dir = TempDir::new().unwrap();
        init_databases(dir.path()).unwrap();
        let conn = open_events_db(dir.path()).unwrap();
        let id = insert_event(&conn, 1000, "app_focus", Some("iTerm2"), None, None).unwrap();
        assert_eq!(get_event(&conn, id).unwrap().unwrap().app.as_deref(), Some("iTerm2"));
        assert!(get_event(&conn, id + 999).unwrap().is_none());
    }

    #[test]
    fn test_clear_unpinned_detaches_events() {
        let dir = TempDir::new().unwrap();
        init_databases(dir.path()).unwrap();
        let conn = open_events_db(dir.path()).unwrap();

        let e1 = insert_event(&conn, 1000, "app_focus", Some("A"), None, None).unwrap();
        let sid = create_session(&conn, "day:x", "One", 1000, 1000, false, "llm", false).unwrap();
        assign_event_session(&conn, e1, Some(sid)).unwrap();
        update_event_llm_desc(&conn, e1, "writing code").unwrap();

        assert_eq!(clear_unpinned_sessions(&conn, "day:x").unwrap(), 1);
        let ev = query_recent_events(&conn, 0).unwrap();
        assert_eq!(ev[0].session_id, None);
        // description survives reorganization
        assert_eq!(ev[0].llm_desc.as_deref(), Some("writing code"));
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
