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
