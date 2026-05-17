use anyhow::Result;
use ccube_core::briefing::{BriefingV2, DetectorV2Output};
use ccube_core::paths::DataRoot;
use serde::{Deserialize, Serialize};

use crate::daemon_client;

/// Mirrors the daemon's DetectResponse so we can deserialize decision_id + flattened output.
#[derive(Serialize, Deserialize)]
struct DetectResponse {
    decision_id: i64,
    #[serde(flatten)]
    output: DetectorV2Output,
}

/// ccube briefing — show the current v2 briefing the detector would see.
pub async fn handle_briefing(root: &DataRoot, json: bool) -> Result<()> {
    if daemon_client::is_daemon_running().await {
        let briefing: BriefingV2 = daemon_client::get_json("/briefing").await?;
        if json {
            println!("{}", serde_json::to_string_pretty(&briefing)?);
        } else {
            print_briefing(&briefing);
        }
    } else {
        // Fallback: build briefing directly from local DB + files
        let conn = ccube_core::db::open_events_db(&root.data_dir)?;
        let now_ms = chrono::Utc::now().timestamp_millis();
        let since_ms = now_ms - 3_600_000;
        let events = ccube_core::db::query_recent_events(&conn, since_ms)?;
        let profile = ccube_core::memory::read_profile(&root.memory_dir)?;
        let patterns = ccube_core::memory::read_patterns(&root.memory_dir)?;
        let briefing = ccube_core::briefing::build_v2(now_ms, &events, &profile, &patterns, &[]);

        if json {
            println!("{}", serde_json::to_string_pretty(&briefing)?);
        } else {
            print_briefing(&briefing);
        }
    }
    Ok(())
}

/// ccube detect [--dry-run] — run the v2 two-step detector once.
pub async fn handle_detect(root: &DataRoot, dry_run: bool, json: bool) -> Result<()> {
    let (decision_id, output): (Option<i64>, DetectorV2Output) =
        if daemon_client::is_daemon_running().await {
            let path = if dry_run {
                "/detect?dry_run=true"
            } else {
                "/detect"
            };
            let resp: DetectResponse =
                daemon_client::post_empty_timeout(path, std::time::Duration::from_secs(30))
                    .await?;
            (Some(resp.decision_id), resp.output)
        } else {
            // Fallback: run v2 detector directly
            let conn = ccube_core::db::open_events_db(&root.data_dir)?;
            let now_ms = chrono::Utc::now().timestamp_millis();
            let since_ms = now_ms - 3_600_000;
            let events = ccube_core::db::query_recent_events(&conn, since_ms)?;
            let profile = ccube_core::memory::read_profile(&root.memory_dir)?;
            let patterns = ccube_core::memory::read_patterns(&root.memory_dir)?;
            let briefing =
                ccube_core::briefing::build_v2(now_ms, &events, &profile, &patterns, &[]);

            let llm =
                ccube_core::llm::LlamaCppClient::from_env().map_err(|e| anyhow::anyhow!(e))?;
            let start = std::time::Instant::now();
            let mut det_output =
                ccube_core::agents::detector::run_v2(&briefing, &llm).await;
            let duration_ms = start.elapsed().as_millis() as i64;

            if dry_run {
                det_output.nudge_message = None;
            }

            // Persist decision so it gets an ID even without the daemon
            let decision_str = format!("{:?}", det_output.decision);
            let nudge_style_str = det_output.nudge_style.as_ref().map(|s| format!("{:?}", s));
            let briefing_json = serde_json::to_string(&briefing)?;

            let did = ccube_core::db::insert_decision(
                &conn,
                now_ms,
                "manual",
                &decision_str,
                &det_output.reasoning,
                nudge_style_str.as_deref(),
                det_output.nudge_message.as_deref(),
                &briefing_json,
                &briefing.memory.patterns_hash,
                ccube_core::agents::detector::PROMPT_VERSION_V2,
                duration_ms,
            )?;

            (Some(did), det_output)
        };

    if json {
        let val = serde_json::json!({
            "decision_id": decision_id,
            "decision": output.decision,
            "reasoning": output.reasoning,
            "nudge_style": output.nudge_style,
            "nudge_message": output.nudge_message,
            "vault_category": output.vault_category,
            "patterns_cited": output.patterns_cited,
            "annotations": output.annotations,
            "rhythm_notes": output.rhythm_notes,
        });
        println!("{}", serde_json::to_string_pretty(&val)?);
    } else {
        print_detect_output(&output, dry_run, decision_id);
    }

    Ok(())
}

fn print_briefing(b: &BriefingV2) {
    println!("=== Briefing (v2) ===");
    println!();

    // Timeline
    if b.events.is_empty() {
        println!("  Timeline: no activity this window");
    } else {
        println!("  Timeline (last 5 min):");
        for e in &b.events {
            let ts_hms = {
                let secs = e.ts / 1000;
                let h = (secs / 3600) % 24;
                let m = (secs / 60) % 60;
                let s = secs % 60;
                format!("{h:02}:{m:02}:{s:02}")
            };
            let dur_s = e.duration_ms / 1000;
            let title = e.title.as_deref().unwrap_or("(no title)");
            let ocr_preview = e.ocr_text.as_ref().map(|t| {
                let short = t.lines().next().unwrap_or("");
                format!(" | ocr: \"{}\"", short)
            });
            println!(
                "    [{ts_hms}] {} | {} | {}s | {} {}",
                e.app,
                title,
                dur_s,
                e.mode,
                ocr_preview.as_deref().unwrap_or("")
            );
        }
    }

    // Metrics
    println!();
    println!("  Metrics:");
    println!("    Switches:      {}", b.metrics.switch_count);
    println!("    Avg session:   {}ms", b.metrics.avg_session_duration_ms);
    println!(
        "    AFK:           {}",
        if b.metrics.is_currently_afk {
            "yes"
        } else {
            "no"
        }
    );
    println!(
        "    AFK->Active:   {}",
        if b.metrics.transitioned_afk_to_active {
            "yes"
        } else {
            "no"
        }
    );

    // Memory
    println!();
    let profile_preview = b.memory.profile.lines().next().unwrap_or("(empty)");
    println!("  Profile: {}", profile_preview);
    println!("  Patterns hash: {}", b.memory.patterns_hash);
}

fn print_detect_output(
    output: &DetectorV2Output,
    dry_run: bool,
    decision_id: Option<i64>,
) {
    if dry_run {
        println!("=== Detect (dry run) v2 ===");
    } else {
        println!("=== Detect v2 ===");
    }
    println!();
    if let Some(id) = decision_id {
        println!("  Decision #{}: {:?}", id, output.decision);
    } else {
        println!("  Decision:  {:?}", output.decision);
    }
    println!("  Reasoning: {}", output.reasoning);

    if let Some(ref style) = output.nudge_style {
        println!("  Style:     {:?}", style);
    }
    if let Some(ref msg) = output.nudge_message {
        println!("  Message:   {}", msg);
    }
    if let Some(ref cat) = output.vault_category {
        println!("  Vault cat: {}", cat);
    }
    if !output.patterns_cited.is_empty() {
        println!("  Patterns:  {:?}", output.patterns_cited);
    }

    // Show rhythm notes
    if let Some(ref notes) = output.rhythm_notes {
        println!("  Rhythm:    {}", notes);
    }

    // Show annotation summary
    if !output.annotations.is_empty() {
        println!();
        println!("  Annotations:");
        for a in &output.annotations {
            let reason = a
                .intent_reasoning
                .as_deref()
                .map(|r| format!(" ({r})"))
                .unwrap_or_default();
            println!("    [{}] {}{}", a.event_ts, a.intent, reason);
        }
    }

    if let Some(id) = decision_id {
        println!();
        println!("  To correct: ccube correct {} \"<your verdict>\"", id);
    }
}
