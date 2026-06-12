use ccube_core::agents::{curator, reflector};
use ccube_core::{agents::detector, briefing, db, eval};
use chrono::{Datelike, Timelike};
use serde::Serialize;
use std::path::Path;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

use crate::http::AppState;

/// Run the periodic scheduler. Includes:
/// - Detector loop (focus-change trigger + 5-min heartbeat, 30s debounce)
/// - Curator loop (daily at configurable hour)
/// - Reflector loop (weekly Sunday 3am or patterns.md > 1600 chars)
/// - Hourly event prune
pub async fn run_scheduler(state: Arc<AppState>, cancel: CancellationToken) {
    tracing::info!("scheduler started");

    let detector_cancel = cancel.clone();
    let detector_state = state.clone();
    let detector_handle = tokio::spawn(run_detector_loop(detector_state, detector_cancel));

    let prune_cancel = cancel.clone();
    let prune_state = state.clone();
    let prune_handle = tokio::spawn(run_prune_loop(prune_state, prune_cancel));

    let curator_cancel = cancel.clone();
    let curator_state = state.clone();
    let curator_handle = tokio::spawn(run_curator_loop(curator_state, curator_cancel));

    let reflector_cancel = cancel.clone();
    let reflector_state = state.clone();
    let reflector_handle = tokio::spawn(run_reflector_loop(reflector_state, reflector_cancel));

    let _ = detector_handle.await;
    let _ = prune_handle.await;
    let _ = curator_handle.await;
    let _ = reflector_handle.await;
}

/// Detector loop: fires on focus change (via Notify) or 5-min heartbeat.
/// Debounced to 30s minimum between runs.
async fn run_detector_loop(state: Arc<AppState>, cancel: CancellationToken) {
    tracing::info!("detector loop started");

    let mut last_run_ms: i64 = 0;
    const DEBOUNCE_MS: i64 = 30_000;
    const HEARTBEAT: std::time::Duration = std::time::Duration::from_secs(300);

    loop {
        // Register the notified future *before* we check / run anything,
        // so a notify_one() that fires while run_detector() is executing
        // is not lost.
        let notified = state.detector_trigger.notified();
        tokio::pin!(notified);

        // Check if we should run immediately (dirty flag from a previous wakeup
        // that arrived while we were busy). The first iteration just waits.
        let trigger = tokio::select! {
            () = &mut notified => "focus_change",
            () = tokio::time::sleep(HEARTBEAT) => "heartbeat",
            () = cancel.cancelled() => {
                tracing::info!("detector loop shutting down");
                return;
            }
        };

        // Debounce: skip if <30s since last run
        let now_ms = chrono::Utc::now().timestamp_millis();
        if now_ms - last_run_ms < DEBOUNCE_MS {
            tracing::debug!(trigger, "detector skipped (debounce)");
            continue;
        }

        last_run_ms = now_ms;
        run_detector(&state, trigger).await;
    }
}

/// Build v2 briefing, run two-step detector, handle result (persist + notify + log).
async fn run_detector(state: &AppState, trigger: &str) {
    let start = std::time::Instant::now();
    let now_ms = chrono::Utc::now().timestamp_millis();

    // Open DB, query events (last hour, build_v2 filters to 5 min window)
    let conn = match db::open_events_db(&state.data_root.data_dir) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "detector: failed to open events db");
            return;
        }
    };
    let events = match db::query_recent_events(&conn, now_ms - 3_600_000) {
        Ok(rows) => rows,
        Err(e) => {
            tracing::error!(error = %e, "detector: failed to query events");
            return;
        }
    };

    // Build v2 briefing from a fresh memory snapshot, so curator/reflector
    // commits take effect on the next detector run without a restart.
    let mem = state.memory_snapshot();
    // The open session's label is the user's inferred intention — the
    // reference point that makes "drift" decidable.
    let current_activity = db::get_open_session(&conn, &crate::http::day_range_key(now_ms))
        .ok()
        .flatten()
        .map(|s| s.label);
    let briefing = briefing::build_v2(
        now_ms,
        &events,
        &mem.profile,
        &mem.patterns,
        &[], // vault_today: not implemented until later phases
        current_activity,
    );

    // Run v2 two-step detector agent
    let output = detector::run_v2(&briefing, state.llm.as_ref()).await;
    let duration_ms = start.elapsed().as_millis() as u64;

    // Persist decision to DB
    let decision_str = format!("{:?}", output.decision);
    let nudge_style_str = output.nudge_style.as_ref().map(|s| format!("{:?}", s));
    let briefing_json = serde_json::to_string(&briefing).unwrap_or_else(|e| {
        tracing::error!(error = %e, "detector: failed to serialize briefing");
        String::new()
    });

    let decision_id = match db::insert_decision(
        &conn,
        now_ms,
        trigger,
        &decision_str,
        &output.reasoning,
        nudge_style_str.as_deref(),
        output.nudge_message.as_deref(),
        &briefing_json,
        &mem.patterns_hash,
        detector::PROMPT_VERSION_V2,
        duration_ms as i64,
    ) {
        Ok(id) => {
            tracing::debug!(decision_id = id, "decision persisted");
            Some(id)
        }
        Err(e) => {
            tracing::error!(error = %e, "failed to persist decision");
            None
        }
    };

    tracing::info!(
        agent = "detector",
        trigger,
        prompt_version = detector::PROMPT_VERSION_V2,
        decision = ?output.decision,
        reasoning = %output.reasoning,
        annotations_count = output.annotations.len(),
        ?decision_id,
        duration_ms,
        "detector decision"
    );

    // Log to detector.ndjson
    let log_entry = DetectorLogEntry {
        ts: now_ms,
        agent: "detector",
        trigger,
        prompt_version: detector::PROMPT_VERSION_V2,
        decision: &decision_str,
        reasoning: &output.reasoning,
        nudge_style: nudge_style_str,
        nudge_message: output.nudge_message.as_deref(),
        patterns_cited: &output.patterns_cited,
        patterns_hash: &mem.patterns_hash,
        decision_id,
        duration_ms,
    };

    let log_path = state.data_root.logs_dir.join("detector.ndjson");
    if let Ok(line) = serde_json::to_string(&log_entry) {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
        {
            let _ = writeln!(f, "{}", line);
        }
    }

    // Send notification on Nudge (unless snoozed from the tray — the
    // decision is still recorded above either way)
    if output.decision == briefing::DetectorDecision::Nudge
        && let Some(ref msg) = output.nudge_message
    {
        let snooze_until = state
            .snooze_until_ms
            .load(std::sync::atomic::Ordering::Relaxed);
        if now_ms < snooze_until {
            tracing::info!(
                snooze_remaining_s = (snooze_until - now_ms) / 1000,
                "nudge suppressed (snoozed from tray)"
            );
        } else if let Some(id) = decision_id {
            crate::notify::send_nudge(id, msg);
        } else {
            tracing::warn!("nudge triggered but no decision_id available, skipping notification");
        }
    }
}

/// Hourly event prune loop.
async fn run_prune_loop(state: Arc<AppState>, cancel: CancellationToken) {
    loop {
        tokio::select! {
            () = tokio::time::sleep(std::time::Duration::from_secs(3600)) => {
                run_prune(&state);
            }
            () = cancel.cancelled() => {
                tracing::info!("prune loop shutting down");
                return;
            }
        }
    }
}

fn run_prune(state: &AppState) {
    let now = chrono::Utc::now().timestamp_millis();
    let cutoff = now - (14 * 24 * 3_600_000);

    match db::open_events_db(&state.data_root.data_dir) {
        Ok(conn) => {
            match db::prune_events(&conn, cutoff) {
                Ok(deleted) => {
                    if deleted > 0 {
                        tracing::info!(deleted, "pruned old events");
                    }
                }
                Err(e) => tracing::error!(error = %e, "event prune failed"),
            }
            match db::prune_decisions(&conn, cutoff) {
                Ok(deleted) => {
                    if deleted > 0 {
                        tracing::info!(deleted, "pruned old decisions");
                    }
                }
                Err(e) => tracing::error!(error = %e, "decision prune failed"),
            }
        }
        Err(e) => tracing::error!(error = %e, "could not open events db for prune"),
    }
}

// ---------------------------------------------------------------------------
// Detector log entry — one ndjson line per decision
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct DetectorLogEntry<'a> {
    ts: i64,
    agent: &'a str,
    trigger: &'a str,
    prompt_version: &'a str,
    decision: &'a str,
    reasoning: &'a str,
    nudge_style: Option<String>,
    nudge_message: Option<&'a str>,
    patterns_cited: &'a [usize],
    patterns_hash: &'a str,
    decision_id: Option<i64>,
    duration_ms: u64,
}

// ---------------------------------------------------------------------------
// Curator loop — daily at configurable hour + NDJSON logging
// ---------------------------------------------------------------------------

/// Curator loop: checks every 60s whether it's time to run the daily curator.
async fn run_curator_loop(state: Arc<AppState>, cancel: CancellationToken) {
    tracing::info!(
        schedule_hour = state.curator_schedule_hour,
        "curator loop started"
    );

    let mut last_run_date: Option<chrono::NaiveDate> = None;

    loop {
        tokio::select! {
            () = tokio::time::sleep(std::time::Duration::from_secs(60)) => {}
            () = cancel.cancelled() => {
                tracing::info!("curator loop shutting down");
                return;
            }
        }

        let now = chrono::Local::now();
        let today = now.date_naive();
        let hour = now.hour();

        // Already ran today? Skip.
        if last_run_date == Some(today) {
            continue;
        }

        // Not the scheduled hour? Skip.
        if hour != state.curator_schedule_hour {
            continue;
        }

        // Any pending corrections?
        let pending = match db::open_corrections_db(&state.data_root.data_dir) {
            Ok(conn) => db::count_pending_corrections(&conn).unwrap_or(0),
            Err(e) => {
                tracing::error!(error = %e, "curator: failed to open corrections db");
                continue;
            }
        };

        if pending == 0 {
            tracing::debug!("curator: no pending corrections, skipping daily run");
            last_run_date = Some(today);
            continue;
        }

        // Try to acquire mutex (non-blocking). If a manual run is in progress, skip.
        let guard = match state.curator_mutex.try_lock() {
            Ok(g) => g,
            Err(_) => {
                tracing::info!("curator: already running (manual?), skipping scheduled run");
                continue;
            }
        };

        tracing::info!(pending, "curator: starting scheduled daily run");
        let start = std::time::Instant::now();

        let mem = state.memory_snapshot();
        match curator::run_curator(
            &state.data_root.data_dir,
            &state.data_root.memory_dir,
            &mem.profile,
            &mem.patterns,
            state.curator_llm.as_ref(),
            state.llm.as_ref(),
            false, // not dry_run
        )
        .await
        {
            Ok(result) => {
                let duration_ms = start.elapsed().as_millis() as u64;
                tracing::info!(
                    corrections = result.corrections_processed,
                    committed = result.committed,
                    eval_passed = result.eval_result.as_ref().map(|e| e.passed),
                    duration_ms,
                    "curator: scheduled run complete"
                );
                log_curator_run(
                    &state.data_root.logs_dir,
                    "daily_schedule",
                    &result,
                    duration_ms,
                );
            }
            Err(e) => {
                tracing::error!(error = %e, "curator: scheduled run failed");
            }
        }

        drop(guard);
        last_run_date = Some(today);
    }
}

/// Write a curator run to `curator.ndjson`. Called from both scheduler and HTTP handler.
pub(crate) fn log_curator_run(
    logs_dir: &Path,
    trigger: &str,
    result: &curator::CuratorRunResult,
    duration_ms: u64,
) {
    let retained = result
        .output
        .correction_verdicts
        .iter()
        .filter(|v| v.verdict == "retain")
        .count();
    let discarded = result
        .output
        .correction_verdicts
        .iter()
        .filter(|v| v.verdict == "discard")
        .count();
    let deferred = result
        .output
        .correction_verdicts
        .iter()
        .filter(|v| v.verdict == "defer")
        .count();

    let entry = CuratorLogEntry {
        ts: chrono::Utc::now().timestamp_millis(),
        agent: "curator",
        trigger,
        prompt_version: curator::PROMPT_VERSION,
        corrections_processed: result.corrections_processed,
        retained,
        discarded,
        deferred,
        eval_passed: result.eval_result.as_ref().map(|e| e.passed),
        patterns_chars_before: result
            .candidate_patterns
            .len()
            .saturating_sub(result.output.proposed_adds.iter().map(|a| a.text.len() + 1).sum()),
        patterns_chars_after: result.candidate_patterns.len(),
        committed: result.committed,
        dry_run: result.dry_run,
        duration_ms,
    };

    let log_path = logs_dir.join("curator.ndjson");
    if let Ok(line) = serde_json::to_string(&entry) {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
        {
            let _ = writeln!(f, "{}", line);
        }
    }
}

#[derive(Serialize)]
struct CuratorLogEntry<'a> {
    ts: i64,
    agent: &'a str,
    trigger: &'a str,
    prompt_version: &'a str,
    corrections_processed: usize,
    retained: usize,
    discarded: usize,
    deferred: usize,
    eval_passed: Option<bool>,
    patterns_chars_before: usize,
    patterns_chars_after: usize,
    committed: bool,
    dry_run: bool,
    duration_ms: u64,
}

// ---------------------------------------------------------------------------
// Reflector loop — weekly (Sunday 3am) or when patterns.md > 1600 chars
// ---------------------------------------------------------------------------

/// Minimum time between reflector runs (23 hours). Prevents re-triggering on the
/// size condition right after a run completes within the same day.
const REFLECTOR_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(23 * 3600);

/// Reflector loop: checks every 60s whether trigger conditions are met.
///
/// Triggers:
/// - **weekly**: Sunday at 3am local time (once per week)
/// - **size**: `patterns.md` exceeds 1600 chars (once, then cooldown)
async fn run_reflector_loop(state: Arc<AppState>, cancel: CancellationToken) {
    tracing::info!("reflector loop started");

    let mut last_run: Option<std::time::Instant> = None;

    loop {
        tokio::select! {
            () = tokio::time::sleep(std::time::Duration::from_secs(60)) => {}
            () = cancel.cancelled() => {
                tracing::info!("reflector loop shutting down");
                return;
            }
        }

        // Cooldown check
        if let Some(prev) = last_run
            && prev.elapsed() < REFLECTOR_COOLDOWN
        {
            continue;
        }

        // Fresh snapshot: curator may have updated memory since the last tick.
        let mem = state.memory_snapshot();

        // Determine trigger
        let now = chrono::Local::now();
        let is_weekly =
            now.weekday() == chrono::Weekday::Sun && now.hour() == 3;
        let is_size = mem.patterns.len() > 1600;

        let trigger = if is_weekly {
            "weekly"
        } else if is_size {
            "size"
        } else {
            continue;
        };

        // Try to acquire curator mutex (non-blocking). Skip if curator is running.
        let guard = match state.curator_mutex.try_lock() {
            Ok(g) => g,
            Err(_) => {
                tracing::info!("reflector: curator mutex held, skipping scheduled run");
                continue;
            }
        };

        tracing::info!(
            trigger,
            patterns_len = mem.patterns.len(),
            "reflector: starting scheduled run"
        );
        let start = std::time::Instant::now();

        match reflector::run_reflector(
            &state.data_root.data_dir,
            &state.data_root.memory_dir,
            &mem.profile,
            &mem.patterns,
            state.curator_llm.as_ref(),
            state.llm.as_ref(), // eval uses detector LLM (faster)
            false,              // not dry_run
        )
        .await
        {
            Ok(result) => {
                let duration_ms = start.elapsed().as_millis() as u64;
                tracing::info!(
                    trigger,
                    committed = result.committed,
                    pending = result.pending,
                    chars_before = result.chars_before,
                    chars_after = result.chars_after,
                    eval_outcome = ?result.eval_outcome,
                    duration_ms,
                    "reflector: scheduled run complete"
                );
                log_reflector_run(
                    &state.data_root.logs_dir,
                    trigger,
                    &result,
                    duration_ms,
                );
            }
            Err(e) => {
                tracing::error!(error = %e, "reflector: scheduled run failed");
            }
        }

        drop(guard);
        last_run = Some(std::time::Instant::now());
    }
}

/// Write a reflector run to `reflector.ndjson`. Called from both scheduler and HTTP handler.
pub(crate) fn log_reflector_run(
    logs_dir: &Path,
    trigger: &str,
    result: &reflector::ReflectorRunResult,
    duration_ms: u64,
) {
    let eval_outcome_str = result.eval_outcome.map(|o| match o {
        eval::ReflectorEvalOutcome::Pass => "pass",
        eval::ReflectorEvalOutcome::Borderline => "borderline",
        eval::ReflectorEvalOutcome::Fail => "fail",
    });

    let entry = ReflectorLogEntry {
        ts: chrono::Utc::now().timestamp_millis(),
        agent: "reflector",
        trigger,
        prompt_version: reflector::PROMPT_VERSION,
        chars_before: result.chars_before,
        chars_after: result.chars_after,
        retained_corrections_count: result.retained_corrections_count,
        eval_outcome: eval_outcome_str,
        committed: result.committed,
        pending: result.pending,
        dry_run: result.dry_run,
        duration_ms,
    };

    let log_path = logs_dir.join("reflector.ndjson");
    if let Ok(line) = serde_json::to_string(&entry) {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
        {
            let _ = writeln!(f, "{}", line);
        }
    }
}

#[derive(Serialize)]
struct ReflectorLogEntry<'a> {
    ts: i64,
    agent: &'a str,
    trigger: &'a str,
    prompt_version: &'a str,
    chars_before: usize,
    chars_after: usize,
    retained_corrections_count: usize,
    eval_outcome: Option<&'a str>,
    committed: bool,
    pending: bool,
    dry_run: bool,
    duration_ms: u64,
}