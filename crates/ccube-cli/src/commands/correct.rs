use anyhow::Result;
use ccube_core::db::{self, CorrectionRow};
use ccube_core::paths::DataRoot;
use serde::Serialize;

use crate::daemon_client;

#[derive(Serialize)]
struct CreateCorrectionBody {
    decision_id: i64,
    verdict: String,
}

/// ccube correct <decision-id> <verdict> — record a correction.
pub async fn handle_correct(root: &DataRoot, decision_id: i64, verdict: &str) -> Result<()> {
    let row: CorrectionRow = if daemon_client::is_daemon_running().await {
        let body = CreateCorrectionBody {
            decision_id,
            verdict: verdict.to_string(),
        };
        daemon_client::post_json("/corrections", &body).await?
    } else {
        // Fallback: direct DB access
        let events_conn = db::open_events_db(&root.data_dir)?;
        let decision = db::get_decision(&events_conn, decision_id)?
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "decision #{decision_id} not found (is the daemon running? decision may have been pruned)"
                )
            })?;

        let corr_conn = db::open_corrections_db(&root.data_dir)?;
        let corr_id = db::insert_correction(
            &corr_conn,
            decision.id,
            &decision.decision,
            verdict,
            &decision.briefing_json,
            &decision.patterns_hash,
        )?;

        db::get_correction(&corr_conn, corr_id)?
            .ok_or_else(|| anyhow::anyhow!("failed to read back correction"))?
    };

    println!(
        "Correction #{} recorded for decision #{}",
        row.id, row.decision_id
    );
    println!("  Original: {}", row.original_decision);
    println!("  Verdict:  {}", row.user_verdict);
    println!("  Status:   {}", row.status);

    Ok(())
}

/// ccube corrections list [--pending] [--limit N]
pub async fn handle_corrections_list(
    root: &DataRoot,
    pending: bool,
    limit: i64,
) -> Result<()> {
    let rows: Vec<CorrectionRow> = if daemon_client::is_daemon_running().await {
        let status_param = if pending { "&status=pending" } else { "" };
        let path = format!("/corrections?limit={limit}{status_param}");
        daemon_client::get_json(&path).await?
    } else {
        let conn = db::open_corrections_db(&root.data_dir)?;
        db::list_corrections(&conn, limit, pending)?
    };

    if rows.is_empty() {
        println!("No corrections found.");
        return Ok(());
    }

    println!(
        "{:<6} {:<20} {:<10} {:<10} {:<}",
        "ID", "Time", "Decision", "Status", "Verdict"
    );
    println!("{}", "-".repeat(76));

    for row in &rows {
        let time = chrono::DateTime::from_timestamp_millis(row.ts)
            .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_else(|| row.ts.to_string());

        // Truncate verdict for table display (char-safe)
        let verdict_short = if row.user_verdict.chars().count() > 30 {
            let truncated: String = row.user_verdict.chars().take(27).collect();
            format!("{truncated}...")
        } else {
            row.user_verdict.clone()
        };

        println!(
            "{:<6} {:<20} {:<10} {:<10} {}",
            row.id, time, row.original_decision, row.status, verdict_short
        );
    }

    Ok(())
}

/// ccube corrections show <id> — show full correction detail.
pub async fn handle_corrections_show(root: &DataRoot, id: i64) -> Result<()> {
    let row: CorrectionRow = if daemon_client::is_daemon_running().await {
        daemon_client::get_json(&format!("/corrections/{id}")).await?
    } else {
        let conn = db::open_corrections_db(&root.data_dir)?;
        db::get_correction(&conn, id)?
            .ok_or_else(|| anyhow::anyhow!("correction #{id} not found"))?
    };

    let time = chrono::DateTime::from_timestamp_millis(row.ts)
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        .unwrap_or_else(|| row.ts.to_string());

    println!("=== Correction #{} ===", row.id);
    println!();
    println!("  Time:          {}", time);
    println!("  Decision ID:   #{}", row.decision_id);
    println!("  Original:      {}", row.original_decision);
    println!("  Verdict:       {}", row.user_verdict);
    println!("  Status:        {}", row.status);
    println!("  Patterns hash: {}...", &row.patterns_hash[..12.min(row.patterns_hash.len())]);
    println!();

    // Show context snapshot summary (first few lines of prettified JSON)
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(&row.ctx_snapshot)
        && let Ok(pretty) = serde_json::to_string_pretty(&val)
    {
        println!("  Context snapshot:");
        for (i, line) in pretty.lines().enumerate() {
            if i >= 20 {
                println!("    ... ({} more lines)", pretty.lines().count() - 20);
                break;
            }
            println!("    {line}");
        }
    }

    Ok(())
}
