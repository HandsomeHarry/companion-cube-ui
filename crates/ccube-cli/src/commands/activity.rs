use anyhow::Result;
use ccube_core::db;

use crate::daemon_client;
use crate::paths::DataRoot;

/// Show recent activity events as a readable table.
pub async fn handle_recent(root: &DataRoot, hours: f64) -> Result<()> {
    // Try daemon HTTP first
    let rows =
        match daemon_client::get_json::<Vec<db::EventRow>>(&format!("/activity?hours={hours}"))
            .await
        {
            Ok(rows) => rows,
            Err(_) => {
                // Fallback: direct DB access
                db::init_databases(&root.data_dir)?;
                let conn = db::open_events_db(&root.data_dir)?;
                let now = chrono::Utc::now().timestamp_millis();
                let since_ts = now - (hours * 3_600_000.0) as i64;
                db::query_recent_events(&conn, since_ts)?
            }
        };

    if rows.is_empty() {
        println!("No events in the last {hours} hour(s).");
        return Ok(());
    }

    render_events_table(&rows);

    println!(
        "\nShowing {} events from the last {:.1} hour(s).",
        rows.len(),
        hours
    );

    Ok(())
}

fn render_events_table(rows: &[db::EventRow]) {
    println!(
        "{:<12} {:<14} {:<22} {:<40} Mode",
        "Time", "Kind", "App", "Title"
    );
    println!("{}", "-".repeat(100));

    for row in rows {
        let time_str = format_time_ms(row.ts);
        let kind = &row.kind;
        let app = row.app.as_deref().unwrap_or("");
        let title = row.title.as_deref().unwrap_or("");
        let mode = row.mode.as_deref().unwrap_or("");

        let title_display = truncate(title, 38);

        println!(
            "{:<12} {:<14} {:<22} {:<40} {}",
            time_str,
            kind,
            truncate(app, 20),
            title_display,
            mode
        );
    }
}

/// Delete events older than 14 days.
pub fn handle_prune(root: &DataRoot) -> Result<()> {
    db::init_databases(&root.data_dir)?;
    let conn = db::open_events_db(&root.data_dir)?;

    let now = chrono::Utc::now().timestamp_millis();
    let cutoff = now - (14 * 24 * 3_600_000);

    let deleted = db::prune_events(&conn, cutoff)?;

    if deleted == 0 {
        println!("No events older than 14 days to prune.");
    } else {
        println!("Pruned {deleted} events older than 14 days.");
    }

    Ok(())
}

fn format_time_ms(ts: i64) -> String {
    use chrono::{DateTime, Utc};
    let dt = DateTime::from_timestamp_millis(ts).unwrap_or_else(Utc::now);
    let local = dt.with_timezone(&chrono::Local);
    local.format("%H:%M:%S").to_string()
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() > max {
        let truncated: String = s.chars().take(max - 3).collect();
        format!("{truncated}...")
    } else {
        s.to_string()
    }
}

/// `ccube data sessions` — today's activity sessions, open session first.
/// Direct-DB read (sessions are local rows; no daemon required).
pub async fn handle_sessions(root: &ccube_core::paths::DataRoot) -> anyhow::Result<()> {
    let conn = ccube_core::db::open_events_db(&root.data_dir)?;
    let range_key = format!("day:{}", chrono::Local::now().format("%Y-%m-%d"));
    let sessions = ccube_core::db::list_sessions(&conn, &range_key)?;

    if sessions.is_empty() {
        println!("No sessions yet today. The daemon organizes activity every 5 minutes.");
        return Ok(());
    }

    println!(
        "{:<5} {:<7} {:<13} {:>4}  {}",
        "ID", "STATE", "SPAN", "EV", "LABEL"
    );
    println!("{}", "-".repeat(78));
    for s in sessions {
        let events = ccube_core::db::query_events_by_session(&conn, s.id)?;
        let state = if s.open {
            "open"
        } else if s.pinned {
            "pinned"
        } else {
            "closed"
        };
        let hm = |ts: i64| {
            chrono::DateTime::from_timestamp_millis(ts)
                .map(|t| t.with_timezone(&chrono::Local).format("%H:%M").to_string())
                .unwrap_or_default()
        };
        let span = format!("{}–{}", hm(s.start_ts), hm(s.end_ts));
        println!(
            "{:<5} {:<7} {:<13} {:>4}  {}",
            s.id,
            state,
            span,
            events.len(),
            truncate(&s.label, 46)
        );
    }
    Ok(())
}

/// `ccube data session <id>` — one session's events with descriptions and the
/// screen context that drove grouping. Direct DB read (no daemon needed).
pub fn handle_session_detail(root: &DataRoot, id: i64) -> anyhow::Result<()> {
    let conn = db::open_events_db(&root.data_dir)?;
    let session = match db::get_session(&conn, id)? {
        Some(s) => s,
        None => {
            println!("No session #{id}. List them with: ccube data sessions");
            return Ok(());
        }
    };
    let state = if session.open {
        "open"
    } else if session.pinned {
        "pinned"
    } else {
        "closed"
    };
    println!("Session #{id}  [{state}]  {}", session.label);
    println!("  range {}  ·  created_by {}", session.range_key, session.created_by);
    println!();

    let events = db::query_events_by_session(&conn, id)?;
    println!("{} events:", events.len());
    for e in &events {
        let desc = e
            .llm_desc
            .as_deref()
            .or(e.vision_desc.as_deref())
            .or(e.title.as_deref())
            .unwrap_or("-");
        let app = e.app.as_deref().unwrap_or("-");
        let mins = e.duration_ms.unwrap_or(0) / 60_000;
        println!("  {}  {:<16} {}  ({}m)", format_time_ms(e.ts), truncate(app, 16), truncate(desc, 50), mins);
    }
    Ok(())
}

/// `ccube data organize [--day YYYY-MM-DD]` — trigger a holistic re-group of
/// the day. Requires the daemon (it holds the LLM client).
pub async fn handle_organize(day: Option<String>) -> anyhow::Result<()> {
    if !daemon_client::is_daemon_running().await {
        anyhow::bail!("the daemon must be running to organize (it runs the local LLM)");
    }
    let range_key = match day {
        Some(d) => format!("day:{d}"),
        None => format!("day:{}", chrono::Local::now().format("%Y-%m-%d")),
    };
    println!("Organizing {range_key} … (holistic pass over the day; may take a moment)");
    let body = serde_json::json!({ "range_key": range_key, "full": true });
    let resp: SummariesResp = daemon_client::post_json_timeout(
        "/summarize",
        &body,
        std::time::Duration::from_secs(600),
    )
    .await?;
    println!("{} sessions:", resp.groups.len());
    for g in &resp.groups {
        let state = if g.open { "open" } else { "closed" };
        println!("  [{state:>6}] {}  ({} events)", g.title, g.events.len());
    }
    Ok(())
}

#[derive(serde::Deserialize)]
struct SummariesResp {
    groups: Vec<SessionGroupResp>,
}
#[derive(serde::Deserialize)]
struct SessionGroupResp {
    title: String,
    open: bool,
    events: Vec<serde_json::Value>,
}
