use anyhow::Result;
use ccube_core::briefing::{CorrectionVerdict, PatternAdd, PatternReplace};
use ccube_core::paths::DataRoot;
use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::daemon_client;

/// Mirrors the daemon's CuratorRunResponse for deserialization.
#[derive(Serialize, Deserialize)]
struct CuratorRunResponse {
    trigger: String,
    corrections_processed: usize,
    correction_verdicts: Vec<CorrectionVerdict>,
    proposed_adds: Vec<PatternAdd>,
    proposed_replaces: Vec<PatternReplace>,
    candidate_patterns: String,
    eval_passed: Option<bool>,
    committed: bool,
    dry_run: bool,
    duration_ms: u64,
}

/// ccube curate [--dry-run] [--json]
pub async fn handle_curate(root: &DataRoot, dry_run: bool, json: bool) -> Result<()> {
    let resp = if daemon_client::is_daemon_running().await {
        // Daemon-first: POST to /agents/curator/run with 180s timeout
        let path = if dry_run {
            "/agents/curator/run?dry_run=true"
        } else {
            "/agents/curator/run"
        };
        daemon_client::post_empty_timeout::<CuratorRunResponse>(path, Duration::from_secs(180))
            .await?
    } else {
        // Local fallback: run curator directly
        let profile = ccube_core::memory::read_profile(&root.memory_dir)?;
        let patterns = ccube_core::memory::read_patterns(&root.memory_dir)?;

        let curator_llm = ccube_core::llm::LlamaCppClient::from_env_with_timeout(
            Duration::from_secs(120),
        )
        .map_err(|e| anyhow::anyhow!(e))?;
        let eval_llm =
            ccube_core::llm::LlamaCppClient::from_env().map_err(|e| anyhow::anyhow!(e))?;

        let start = std::time::Instant::now();
        let result = ccube_core::agents::curator::run_curator(
            &root.data_dir,
            &root.memory_dir,
            &profile,
            &patterns,
            &curator_llm,
            &eval_llm,
            dry_run,
        )
        .await?;
        let duration_ms = start.elapsed().as_millis() as u64;

        CuratorRunResponse {
            trigger: "manual".to_string(),
            corrections_processed: result.corrections_processed,
            correction_verdicts: result.output.correction_verdicts,
            proposed_adds: result.output.proposed_adds,
            proposed_replaces: result.output.proposed_replaces,
            candidate_patterns: result.candidate_patterns,
            eval_passed: result.eval_result.as_ref().map(|e| e.passed),
            committed: result.committed,
            dry_run: result.dry_run,
            duration_ms,
        }
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&resp)?);
    } else {
        print_curate_output(&resp);
    }

    Ok(())
}

fn print_curate_output(resp: &CuratorRunResponse) {
    if resp.dry_run {
        println!("=== Curator Run (dry run) ===");
    } else {
        println!("=== Curator Run ===");
    }
    println!();

    if resp.corrections_processed == 0 {
        println!("  No pending corrections to process.");
        return;
    }

    println!(
        "  Corrections processed: {}",
        resp.corrections_processed
    );
    println!(
        "  Duration: {:.1}s",
        resp.duration_ms as f64 / 1000.0
    );

    // Verdicts
    if !resp.correction_verdicts.is_empty() {
        println!();
        println!("  Verdicts:");
        for v in &resp.correction_verdicts {
            println!(
                "    #{:<4} {:<8} -- \"{}\"",
                v.correction_id, v.verdict, v.rationale
            );
        }
    }

    // Proposed additions
    if !resp.proposed_adds.is_empty() {
        println!();
        println!("  Proposed additions:");
        for a in &resp.proposed_adds {
            let ids: Vec<String> = a.supporting_correction_ids.iter().map(|i| format!("#{i}")).collect();
            println!("    + \"{}\" (from {})", a.text, ids.join(", "));
        }
    }

    // Proposed replacements
    if !resp.proposed_replaces.is_empty() {
        println!();
        println!("  Proposed replacements:");
        for r in &resp.proposed_replaces {
            println!("    - \"{}\"", r.old_text);
            println!("    + \"{}\"", r.new_text);
            println!("      (reason: {})", r.rationale);
        }
    }

    // Eval result
    println!();
    match resp.eval_passed {
        Some(true) => println!("  Eval: PASSED (0 regressions)"),
        Some(false) => println!("  Eval: FAILED -- changes NOT committed"),
        None => {
            if resp.dry_run {
                println!("  Eval: skipped (dry run)");
            } else {
                println!("  Eval: skipped (no changes proposed)");
            }
        }
    }

    // Commit status
    if resp.committed {
        println!("  Status: Changes committed to patterns.md");
    } else if resp.dry_run {
        println!("  Status: (dry run -- no changes committed)");
    } else if resp.eval_passed == Some(false) {
        println!("  Status: Changes rejected by eval gate");
    } else {
        println!("  Status: No changes to commit");
    }
}
