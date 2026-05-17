use anyhow::Result;
use ccube_core::paths::DataRoot;
use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::daemon_client;

/// Mirrors the daemon's ReflectorRunResponse for deserialization.
#[derive(Serialize, Deserialize)]
struct ReflectorRunResponse {
    trigger: String,
    #[serde(default)]
    patterns_after: String,
    rationale: String,
    eval_passed: Option<bool>,
    eval_outcome: Option<String>,
    committed: bool,
    pending: bool,
    dry_run: bool,
    chars_before: usize,
    chars_after: usize,
    duration_ms: u64,
}

/// Mirrors the daemon's PendingResponse.
#[derive(Serialize, Deserialize)]
struct PendingResponse {
    exists: bool,
    content: Option<String>,
    chars: Option<usize>,
}

/// Mirrors the daemon's PendingActionResponse.
#[derive(Serialize, Deserialize)]
struct PendingActionResponse {
    status: String,
}

/// ccube reflect run [--dry-run] [--json]
pub async fn handle_reflect(root: &DataRoot, dry_run: bool, json: bool) -> Result<()> {
    let resp = if daemon_client::is_daemon_running().await {
        let path = if dry_run {
            "/agents/reflector/run?dry_run=true"
        } else {
            "/agents/reflector/run"
        };
        daemon_client::post_empty_timeout::<ReflectorRunResponse>(path, Duration::from_secs(180))
            .await?
    } else {
        // Local fallback: run reflector directly
        let profile = ccube_core::memory::read_profile(&root.memory_dir)?;
        let patterns = ccube_core::memory::read_patterns(&root.memory_dir)?;

        let llm =
            ccube_core::llm::LlamaCppClient::from_env_with_timeout(Duration::from_secs(120))
                .map_err(|e| anyhow::anyhow!(e))?;

        let start = std::time::Instant::now();
        let result = ccube_core::agents::reflector::run_reflector(
            &root.data_dir,
            &root.memory_dir,
            &profile,
            &patterns,
            &llm,
            &llm,
            dry_run,
        )
        .await?;
        let duration_ms = start.elapsed().as_millis() as u64;

        let eval_outcome = result.eval_outcome.map(|o| {
            use ccube_core::eval::ReflectorEvalOutcome;
            match o {
                ReflectorEvalOutcome::Pass => "pass".to_string(),
                ReflectorEvalOutcome::Borderline => "borderline".to_string(),
                ReflectorEvalOutcome::Fail => "fail".to_string(),
            }
        });

        ReflectorRunResponse {
            trigger: "manual".to_string(),
            patterns_after: result.patterns_after,
            rationale: result.rationale,
            eval_passed: result.eval_result.as_ref().map(|e| e.passed),
            eval_outcome,
            committed: result.committed,
            pending: result.pending,
            dry_run: result.dry_run,
            chars_before: result.chars_before,
            chars_after: result.chars_after,
            duration_ms,
        }
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&resp)?);
    } else {
        print_reflect_output(&resp);
    }

    Ok(())
}

/// ccube reflect accept
pub async fn handle_accept(root: &DataRoot) -> Result<()> {
    if daemon_client::is_daemon_running().await {
        let resp =
            daemon_client::post_empty::<PendingActionResponse>("/agents/reflector/accept").await?;
        if resp.status == "accepted" {
            println!("Pending patterns accepted and committed to patterns.md.");
        } else {
            eprintln!("Accept failed (daemon returned status={}).", resp.status);
        }
    } else {
        ccube_core::agents::reflector::accept_pending(&root.memory_dir)?;
        println!("Pending patterns accepted and committed to patterns.md.");
    }
    Ok(())
}

/// ccube reflect reject
pub async fn handle_reject(root: &DataRoot) -> Result<()> {
    if daemon_client::is_daemon_running().await {
        let resp =
            daemon_client::post_empty::<PendingActionResponse>("/agents/reflector/reject").await?;
        if resp.status == "rejected" {
            println!("Pending patterns rejected and discarded.");
        } else {
            eprintln!("Reject failed (daemon returned status={}).", resp.status);
        }
    } else {
        ccube_core::agents::reflector::reject_pending(&root.memory_dir)?;
        println!("Pending patterns rejected and discarded.");
    }
    Ok(())
}

/// ccube reflect show
pub async fn handle_show_pending(root: &DataRoot, json: bool) -> Result<()> {
    let (exists, content) = if daemon_client::is_daemon_running().await {
        let resp =
            daemon_client::get_json::<PendingResponse>("/agents/reflector/pending").await?;
        (resp.exists, resp.content)
    } else {
        let content = ccube_core::agents::reflector::read_pending(&root.memory_dir)?;
        (content.is_some(), content)
    };

    if json {
        let resp = PendingResponse {
            exists,
            content,
            chars: None,
        };
        println!("{}", serde_json::to_string_pretty(&resp)?);
    } else if let Some(ref text) = content {
        println!("=== Pending Reflector Output ===");
        println!();
        println!("{text}");
        println!();
        println!("Run `ccube reflect accept` to commit or `ccube reflect reject` to discard.");
    } else {
        println!("No pending reflector output.");
    }

    Ok(())
}

fn print_reflect_output(resp: &ReflectorRunResponse) {
    if resp.dry_run {
        println!("=== Reflector Run (dry run) ===");
    } else {
        println!("=== Reflector Run ===");
    }
    println!();

    println!(
        "  Patterns: {} chars -> {} chars",
        resp.chars_before, resp.chars_after
    );
    println!("  Duration: {:.1}s", resp.duration_ms as f64 / 1000.0);

    if !resp.rationale.is_empty() {
        println!();
        println!("  Rationale: {}", resp.rationale);
    }

    // Eval outcome
    println!();
    match resp.eval_outcome.as_deref() {
        Some("pass") => println!("  Eval: PASSED"),
        Some("borderline") => println!("  Eval: BORDERLINE -- saved as pending"),
        Some("fail") => println!("  Eval: FAILED -- changes discarded"),
        Some(other) => println!("  Eval: {other}"),
        None => {
            if resp.dry_run {
                println!("  Eval: skipped (dry run)");
            } else {
                println!("  Eval: skipped");
            }
        }
    }

    // Commit status
    if resp.committed {
        println!("  Status: Changes committed to patterns.md");
    } else if resp.pending {
        println!("  Status: Saved as pending -- run `ccube reflect show` to review");
    } else if resp.dry_run {
        println!("  Status: (dry run -- no changes committed)");
    } else if resp.eval_outcome.as_deref() == Some("fail") {
        println!("  Status: Changes rejected by eval gate");
    } else {
        println!("  Status: No changes to commit");
    }
}
