pub mod http;
mod notify;
mod scheduler;
mod tray;

use anyhow::{Context, Result};
use ccube_capture::ActivityCapture;
#[cfg(target_os = "windows")]
use ccube_capture::windows::WinActivityCapture;
#[cfg(target_os = "macos")]
use ccube_capture::macos::MacActivityCapture;
use ccube_core::{db, focus_mode, llm, memory, paths::DataRoot};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

use http::AppState;
use tray::{TrayState, UserEvent};

/// Synchronous entry point. macOS requires the tray/event loop on the main
/// thread, so all async work runs on a dedicated tokio runtime thread while the
/// main thread owns the `tao` event loop (see `tray`).
fn main() -> Result<()> {
    dotenvy::dotenv().ok();

    // 1. Resolve paths and init databases
    let root = DataRoot::resolve()?;
    db::init_databases(&root.data_dir)?;

    // 2. Setup logging: JSON to daemon.ndjson + optional stdout.
    // `_guard` must outlive the process; it is held in this never-returning fn.
    let file_appender = tracing_appender::rolling::never(&root.logs_dir, "daemon.ndjson");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
    let json_layer = tracing_subscriber::fmt::layer().json().with_writer(non_blocking);
    let filter = EnvFilter::try_from_env("CCUBE_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    let is_tty = std::io::IsTerminal::is_terminal(&std::io::stdout());
    let stdout_layer = if is_tty {
        Some(
            tracing_subscriber::fmt::layer()
                .compact()
                .with_target(false),
        )
    } else {
        None
    };
    tracing_subscriber::registry()
        .with(filter)
        .with(json_layer)
        .with(stdout_layer)
        .init();

    tracing::info!(version = env!("CARGO_PKG_VERSION"), "ccube-daemon starting");

    // 3. Session fence — recover from previous crash + mark session start
    {
        let conn = db::open_events_db(&root.data_dir)?;
        let now_ms = chrono::Utc::now().timestamp_millis();

        // Check if the previous session ended cleanly (has a daemon_stop after the
        // last daemon_start). If not, the daemon crashed — finalize any open events.
        let last_start = db::last_event_of_kind(&conn, "daemon_start")?;
        let last_stop = db::last_event_of_kind(&conn, "daemon_stop")?;

        let clean_shutdown = match (&last_start, &last_stop) {
            (Some(start), Some(stop)) => stop.ts >= start.ts,
            (None, _) => true,        // first ever run
            (Some(_), None) => false, // started but never stopped
        };

        if !clean_shutdown {
            // Crash recovery: find events with NULL duration and cap them.
            // Use the daemon_start ts as the best estimate of when the daemon died
            // (it's the last known-good timestamp from the previous session).
            let crash_ts = last_start.as_ref().map(|e| e.ts).unwrap_or(now_ms);
            let stale = db::query_recent_events(&conn, crash_ts)?;
            let mut fixed = 0u32;
            for e in &stale {
                if e.duration_ms.is_none() && e.kind == "app_focus" {
                    let capped = (crash_ts - e.ts).max(0);
                    db::update_event_duration(&conn, e.id, capped)?;
                    fixed += 1;
                }
            }
            if fixed > 0 {
                tracing::warn!(
                    fixed,
                    "crash recovery: finalized {fixed} stale events from previous session"
                );
            }
        }

        // Insert daemon_start sentinel
        db::insert_event(&conn, now_ms, "daemon_start", None, None, None)?;
        tracing::info!("session fence: daemon_start sentinel inserted");
    }

    // 4. Write PID file
    let pid_file = root.data_dir.join("daemon.pid");
    std::fs::write(&pid_file, std::process::id().to_string())?;

    // 5. Log the memory state at startup. Memory is no longer frozen here:
    // each agent run loads a fresh snapshot (AppState::memory_snapshot) so
    // curator/reflector commits take effect without a restart.
    match memory::load_snapshot(&root.memory_dir) {
        Ok(snap) => tracing::info!(
            profile_chars = snap.profile.len(),
            patterns_chars = snap.patterns.len(),
            patterns_hash = %snap.patterns_hash,
            "memory loaded (live per-run snapshots)"
        ),
        Err(e) => tracing::warn!(error = %e, "could not read memory at startup"),
    }

    // 6. Create LLM clients (detector: 10s timeout, curator: 120s timeout)
    let llm_client: Arc<dyn ccube_core::llm::LlmBackend> =
        Arc::new(llm::LlamaCppClient::from_env().map_err(|e| anyhow::anyhow!(e))?);
    let curator_llm_client: Arc<dyn ccube_core::llm::LlmBackend> = Arc::new(
        llm::LlamaCppClient::from_env_with_timeout(Duration::from_secs(120))
            .map_err(|e| anyhow::anyhow!(e))?,
    );

    // 7. Read curator schedule config
    let curator_schedule_hour: u32 = std::env::var("CCUBE_CURATOR_HOUR")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(5)
        .min(23);

    // 8. Create shared state
    let cancel = CancellationToken::new();
    let detector_trigger = Arc::new(Notify::new());
    let cached_summaries = Arc::new(tokio::sync::RwLock::new(None));
    let snooze_until_ms = Arc::new(std::sync::atomic::AtomicI64::new(0));

    let state = Arc::new(AppState {
        data_root: root,
        start_time: std::time::Instant::now(),
        shutdown_token: cancel.clone(),
        version: env!("CARGO_PKG_VERSION"),
        llm: llm_client,
        curator_llm: curator_llm_client,
        detector_trigger: detector_trigger.clone(),
        curator_mutex: Arc::new(tokio::sync::Mutex::new(())),
        curator_schedule_hour,
        cached_summaries,
        snooze_until_ms: snooze_until_ms.clone(),
    });

    // Resolve the loopback port (override with CCUBE_PORT; consistent with the
    // other CCUBE_* env conventions). The tray opens this URL in the browser.
    let port: u16 = std::env::var("CCUBE_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(7431);
    let dashboard_url = format!("http://localhost:{port}");

    // 9. Build the main-thread event loop before spawning the runtime, so we can
    //    hand a proxy to the tokio thread for the shutdown handshake.
    let event_loop = tray::build_event_loop();
    let proxy = event_loop.create_proxy();

    // 10. Spawn the tokio runtime on a background thread. When it finishes its
    //     graceful shutdown it signals the event loop to exit.
    let rt_cancel = cancel.clone();
    let rt_state = state.clone();
    let tray_proxy = event_loop.create_proxy();
    let _runtime_thread = std::thread::Builder::new()
        .name("ccube-tokio".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("failed to build tokio runtime");
            rt.block_on(run_runtime(rt_state, rt_cancel, port, tray_proxy));
            let _ = proxy.send_event(UserEvent::Shutdown);
        })
        .context("failed to spawn tokio runtime thread")?;

    // 11. Run the tray event loop on the main thread (never returns).
    tray::run(event_loop, cancel, dashboard_url, snooze_until_ms)
}

/// Drive all async subsystems: capture loop, scheduler, summarize loop, and the
/// HTTP server. Returns only after a graceful shutdown (so the caller can tell
/// the tray event loop to exit the process).
async fn run_runtime(
    state: Arc<AppState>,
    cancel: CancellationToken,
    port: u16,
    tray_proxy: tao::event_loop::EventLoopProxy<UserEvent>,
) {
    // Capture loop
    let capture_cancel = cancel.clone();
    let capture_state = state.clone();
    let capture_handle = tokio::spawn(async move {
        if let Err(e) = capture_loop(&capture_state, capture_cancel).await {
            tracing::error!(error = %e, "capture loop failed");
        }
    });

    // Tray state loop — mirrors focus state into the tray icon
    let tray_state_handle = tokio::spawn(tray_state_loop(
        state.clone(),
        tray_proxy,
        cancel.clone(),
    ));

    // Scheduler
    let scheduler_handle = tokio::spawn(scheduler::run_scheduler(state.clone(), cancel.clone()));

    // Summarize scheduler (every 5 min)
    let summarize_state = state.clone();
    let summarize_cancel = cancel.clone();
    let summarize_handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(300));
        interval.tick().await; // skip the first immediate tick
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    // Incremental pass over today: groups only events that
                    // belong to no session yet. Never rewrites history.
                    match http::run_summarize(&summarize_state, None, None, None, false).await {
                        Ok(result) => {
                            *summarize_state.cached_summaries.write().await = Some(result);
                            tracing::info!("auto-summarization complete");
                        }
                        Err(e) => {
                            tracing::warn!("auto-summarization failed: {:?}", e);
                        }
                    }
                }
                _ = summarize_cancel.cancelled() => break,
            }
        }
    });

    // Bind HTTP server
    let addr = format!("127.0.0.1:{port}");
    let listener = match TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!(error = %e, addr = %addr, "failed to bind (port already in use?)");
            cancel.cancel();
            return;
        }
    };
    tracing::info!(addr = %addr, "HTTP server listening");

    let router = http::router(state.clone());
    let server_cancel = cancel.clone();
    let server_handle = tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, router)
            .with_graceful_shutdown(async move {
                server_cancel.cancelled().await;
            })
            .await
        {
            tracing::error!(error = %e, "HTTP server error");
        }
    });

    // Ctrl-C also triggers shutdown (when running attached to a terminal)
    let ctrl_cancel = cancel.clone();
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        tracing::info!("Ctrl-C received, initiating shutdown");
        ctrl_cancel.cancel();
    });

    // Wait for cancellation, then join tasks with a 2-second timeout.
    cancel.cancelled().await;
    tracing::info!("shutdown initiated, waiting for tasks...");

    let shutdown_result = tokio::time::timeout(Duration::from_secs(2), async {
        let _ = capture_handle.await;
        let _ = scheduler_handle.await;
        let _ = summarize_handle.await;
        let _ = server_handle.await;
        let _ = tray_state_handle.await;
    })
    .await;

    if shutdown_result.is_err() {
        tracing::warn!("shutdown timed out after 2 seconds, exiting anyway");
    }

    // Cleanup — insert daemon_stop sentinel before removing PID
    if let Ok(conn) = db::open_events_db(&state.data_root.data_dir) {
        let stop_ts = chrono::Utc::now().timestamp_millis();
        let _ = db::insert_event(&conn, stop_ts, "daemon_stop", None, None, None);
        tracing::info!("session fence: daemon_stop sentinel inserted");
    }
    let pid_file = state.data_root.data_dir.join("daemon.pid");
    let _ = std::fs::remove_file(&pid_file);
    tracing::info!("ccube-daemon stopped");
}

/// How recently a Nudge/Vault decision must have fired to color the tray as
/// drifting. After this window without a new nudge, the icon warms back up.
const DRIFT_WINDOW_MS: i64 = 10 * 60 * 1000;

/// Poll the local DB every 5s and mirror the focus state into the tray icon
/// (Mem Reduct-style: the icon is the status display). Sends only on change.
async fn tray_state_loop(
    state: Arc<AppState>,
    proxy: tao::event_loop::EventLoopProxy<UserEvent>,
    cancel: CancellationToken,
) {
    let mut last_sent: Option<(TrayState, String)> = None;
    loop {
        tokio::select! {
            () = tokio::time::sleep(Duration::from_secs(5)) => {}
            () = cancel.cancelled() => return,
        }

        let computed = match compute_tray_state(&state) {
            Some(c) => c,
            None => continue, // DB momentarily unavailable; keep last icon
        };

        if last_sent.as_ref() != Some(&computed) {
            let (s, tip) = computed.clone();
            tracing::info!(state = ?s, tooltip = %tip, "tray state changed, sending");
            if proxy.send_event(UserEvent::State(s, tip)).is_err() {
                return; // event loop is gone; we're shutting down
            }
            last_sent = Some(computed);
        }
    }
}

/// Read the latest capture/decision rows and classify the focus state.
fn compute_tray_state(state: &AppState) -> Option<(TrayState, String)> {
    let conn = db::open_events_db(&state.data_root.data_dir).ok()?;
    let now_ms = chrono::Utc::now().timestamp_millis();

    let last_focus = db::last_event_of_kind(&conn, "app_focus").ok()?;
    let last_idle_start = db::last_event_of_kind(&conn, "idle_start").ok()?;
    let last_idle_end = db::last_event_of_kind(&conn, "idle_end").ok()?;
    let last_decision = db::list_decisions(&conn, now_ms - DRIFT_WINDOW_MS, 1)
        .ok()?
        .into_iter()
        .next();

    Some(classify_tray_state(
        last_focus.as_ref().and_then(|e| e.app.as_deref()),
        last_focus.as_ref().map(|e| e.ts),
        last_idle_start.map(|e| e.ts),
        last_idle_end.map(|e| e.ts),
        last_decision.map(|d| d.decision),
    ))
}

/// Pure classification: idle wins if the newest idle_start is strictly after
/// both the newest activity and idle_end (with a 1s margin — capture emits an
/// idle probe at startup with the same timestamp as the first app_focus);
/// otherwise a recent Nudge/Vault decision means drifting; otherwise focused.
fn classify_tray_state(
    focus_app: Option<&str>,
    focus_ts: Option<i64>,
    idle_start_ts: Option<i64>,
    idle_end_ts: Option<i64>,
    recent_decision: Option<String>,
) -> (TrayState, String) {
    let active_ts = focus_ts.unwrap_or(0).max(idle_end_ts.unwrap_or(0));
    if idle_start_ts.unwrap_or(0) > active_ts + 1000 {
        return (TrayState::Idle, "Companion Cube — idle".to_string());
    }

    let app = focus_app.unwrap_or("").to_string();
    let suffix = if app.is_empty() {
        String::new()
    } else {
        format!(" · {app}")
    };

    match recent_decision.as_deref() {
        Some("Nudge") | Some("Vault") => {
            (TrayState::Drifting, format!("Companion Cube — drifting{suffix}"))
        }
        _ => (TrayState::Focused, format!("Companion Cube — focused{suffix}")),
    }
}

/// Run the continuous capture loop, writing events to the database.
async fn capture_loop(state: &AppState, cancel: CancellationToken) -> Result<()> {
    tracing::info!("capture loop starting");

    #[cfg(target_os = "windows")]
    let capture = WinActivityCapture::new();
    #[cfg(target_os = "macos")]
    let capture = MacActivityCapture::new();
    let mut rx = capture.subscribe().await;

    let conn = db::open_events_db(&state.data_root.data_dir)?;
    let mut last_event: HashMap<String, (i64, i64)> = HashMap::new();
    // Track context of the last app_focus event for OCR-based re-inference
    let mut last_focus_context: Option<(i64, String, Option<String>, Option<String>)> = None;
    let mut event_count: u64 = 0;

    loop {
        tokio::select! {
            event = rx.recv() => {
                let event = match event {
                    Some(e) => e,
                    None => {
                        tracing::warn!("capture channel closed");
                        break;
                    }
                };

                let (kind, ts, app, title, url) = match &event {
                    ccube_capture::ActivityEvent::AppFocusChanged { app, title, ts } => {
                        ("app_focus", *ts, Some(app.as_str()), title.as_deref(), None)
                    }
                    ccube_capture::ActivityEvent::WindowTitleChanged { title, ts } => {
                        ("window_title", *ts, None, Some(title.as_str()), None)
                    }
                    ccube_capture::ActivityEvent::UrlChanged { url, ts } => {
                        ("url", *ts, None, Some(url.as_str()), Some(url.as_str()))
                    }
                    ccube_capture::ActivityEvent::IdleStart { ts } => {
                        ("idle_start", *ts, None, None, None)
                    }
                    ccube_capture::ActivityEvent::IdleEnd { ts } => {
                        ("idle_end", *ts, None, None, None)
                    }
                    ccube_capture::ActivityEvent::OcrReady { text, ts: _ } => {
                        // Write OCR text + re-inferred mode to the most recent app_focus event
                        if let Some((prev_id, ref app, ref title, ref url)) = last_focus_context {
                            let m = focus_mode::infer_focus_mode(
                                app,
                                title.as_deref(),
                                url.as_deref(),
                                Some(text),
                            );
                            let mode_str = focus_mode::focus_mode_to_str(&m);
                            if let Err(e) = db::update_event_ocr_and_mode(&conn, prev_id, text, mode_str) {
                                tracing::warn!(error = %e, "failed to update OCR text + mode");
                            }
                        } else if let Some(&(prev_id, _)) = last_event.get("app_focus") {
                            // Fallback: no app context stored, just write OCR text
                            if let Err(e) = db::update_event_ocr(&conn, prev_id, text) {
                                tracing::warn!(error = %e, "failed to update OCR text");
                            }
                        }
                        continue;
                    }
                };

                let mode = if kind == "app_focus" {
                    let m = focus_mode::infer_focus_mode(app.unwrap_or(""), title, url, None);
                    Some(focus_mode::focus_mode_to_str(&m))
                } else {
                    None
                };

                match db::insert_event(&conn, ts, kind, app, title, mode) {
                    Ok(row_id) => {
                        if let Some(&(prev_id, prev_ts)) = last_event.get(kind) {
                            let duration = ts - prev_ts;
                            if duration > 0 {
                                let _ = db::update_event_duration(&conn, prev_id, duration);

                                // OCR gate: on app_focus switch with >5s session
                                if kind == "app_focus" && duration > 5_000 {
                                    let data_dir = state.data_root.data_dir.clone();
                                    tokio::spawn(async move {
                                        if let Err(e) = run_ocr_for_event(&data_dir, prev_id).await {
                                            tracing::warn!(error = %e, event_id = prev_id, "OCR failed");
                                        }
                                    });
                                }
                            }
                        }
                        last_event.insert(kind.to_string(), (row_id, ts));

                        // Store app context for OCR-based mode re-inference
                        if kind == "app_focus" {
                            last_focus_context = Some((
                                row_id,
                                app.unwrap_or("").to_string(),
                                title.map(String::from),
                                url.map(String::from),
                            ));
                            state.detector_trigger.notify_one();
                        }
                        event_count += 1;

                        tracing::debug!(
                            kind,
                            app = app.unwrap_or(""),
                            title = title.unwrap_or(""),
                            mode = mode.unwrap_or(""),
                            "event captured"
                        );
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "DB write failed");
                    }
                }
            }
            () = cancel.cancelled() => {
                tracing::info!("capture loop shutting down");
                #[cfg(target_os = "windows")]
                ccube_capture::windows::request_shutdown();
                #[cfg(target_os = "macos")]
                ccube_capture::macos::request_shutdown();

                // Drain remaining events
                while let Ok(event) = rx.try_recv() {
                    let (kind, ts, app, title, url) = match &event {
                        ccube_capture::ActivityEvent::AppFocusChanged { app, title, ts } => {
                            ("app_focus", *ts, Some(app.as_str()), title.as_deref(), None)
                        }
                        ccube_capture::ActivityEvent::WindowTitleChanged { title, ts } => {
                            ("window_title", *ts, None, Some(title.as_str()), None)
                        }
                        ccube_capture::ActivityEvent::UrlChanged { url, ts } => {
                            ("url", *ts, None, Some(url.as_str()), Some(url.as_str()))
                        }
                        ccube_capture::ActivityEvent::IdleStart { ts } => {
                            ("idle_start", *ts, None, None, None)
                        }
                        ccube_capture::ActivityEvent::IdleEnd { ts } => {
                            ("idle_end", *ts, None, None, None)
                        }
                        ccube_capture::ActivityEvent::OcrReady { text, ts: _ } => {
                            if let Some((prev_id, ref app, ref title, ref url)) = last_focus_context {
                                let m = focus_mode::infer_focus_mode(
                                    app, title.as_deref(), url.as_deref(), Some(text),
                                );
                                let mode_str = focus_mode::focus_mode_to_str(&m);
                                let _ = db::update_event_ocr_and_mode(&conn, prev_id, text, mode_str);
                            } else if let Some(&(prev_id, _)) = last_event.get("app_focus") {
                                let _ = db::update_event_ocr(&conn, prev_id, text);
                            }
                            continue;
                        }
                    };
                    let mode = if kind == "app_focus" {
                        let m = focus_mode::infer_focus_mode(app.unwrap_or(""), title, url, None);
                        Some(focus_mode::focus_mode_to_str(&m))
                    } else {
                        None
                    };
                    if let Ok(row_id) = db::insert_event(&conn, ts, kind, app, title, mode) {
                        if let Some((prev_id, prev_ts)) = last_event.get(kind) {
                            let duration = ts - prev_ts;
                            if duration > 0
                                && let Err(e) = db::update_event_duration(&conn, *prev_id, duration)
                            {
                                tracing::warn!(error = %e, "failed to update duration during drain");
                            }
                        }
                        last_event.insert(kind.to_string(), (row_id, ts));
                        if kind == "app_focus" {
                            last_focus_context = Some((
                                row_id,
                                app.unwrap_or("").to_string(),
                                title.map(String::from),
                                url.map(String::from),
                            ));
                        }
                    } else {
                        tracing::warn!("failed to persist event during drain");
                    }
                    event_count += 1;
                }

                // Finalize durations
                let now = chrono::Utc::now().timestamp_millis();
                for (prev_id, prev_ts) in last_event.values() {
                    let duration = now - prev_ts;
                    if duration > 0
                        && let Err(e) = db::update_event_duration(&conn, *prev_id, duration)
                    {
                        tracing::warn!(error = %e, "failed to finalize duration during drain");
                    }
                }

                tracing::info!(event_count, "capture loop stopped");
                break;
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_focused_when_active_no_recent_nudge() {
        let (s, tip) = classify_tray_state(Some("iTerm2"), Some(2000), Some(1000), None, None);
        assert_eq!(s, TrayState::Focused);
        assert_eq!(tip, "Companion Cube — focused · iTerm2");
    }

    #[test]
    fn tray_drifting_on_recent_nudge() {
        let (s, _) = classify_tray_state(
            Some("Brave Browser"),
            Some(2000),
            None,
            None,
            Some("Nudge".to_string()),
        );
        assert_eq!(s, TrayState::Drifting);
    }

    #[test]
    fn tray_idle_when_idle_start_newest() {
        let (s, tip) = classify_tray_state(Some("iTerm2"), Some(1000), Some(600_000), Some(500), None);
        assert_eq!(s, TrayState::Idle);
        assert_eq!(tip, "Companion Cube — idle");
    }

    #[test]
    fn tray_not_idle_on_startup_probe_same_ts() {
        // capture emits idle_start with the same ts as the first app_focus
        let (s, _) = classify_tray_state(Some("iTerm2"), Some(1000), Some(1000), None, None);
        assert_eq!(s, TrayState::Focused);
    }

    #[test]
    fn tray_idle_end_returns_to_focused() {
        let (s, _) = classify_tray_state(Some("iTerm2"), Some(1000), Some(5000), Some(9000), None);
        assert_eq!(s, TrayState::Focused);
    }
}

/// Capture a screenshot, run OCR + vision classification, and store results
/// against a completed event. Uses spawn_blocking because both capture_screenshot,
/// OCR engine, and vision model inference are synchronous.
async fn run_ocr_for_event(data_dir: &Path, event_id: i64) -> Result<()> {
    let data_dir = data_dir.to_path_buf();
    let (ocr_result, vision_result) = tokio::task::spawn_blocking(move || {
        // Without permission the capture throws an uncatchable ObjC
        // exception that would abort the whole daemon — preflight first.
        #[cfg(target_os = "macos")]
        if !ccube_capture::macos::screen_permission_now() {
            anyhow::bail!("screen recording permission not granted");
        }
        let png = ccube_capture::capture_screenshot()
            .context("screenshot capture failed")?;

        // OCR via platform-native engine
        let ocr_text = match ccube_capture::ocr::create_engine() {
            Some(engine) => engine.extract_text(&png).unwrap_or_default(),
            None => String::new(),
        };

        // Vision classification via Ollama (best-effort, non-blocking on failure)
        let vision_desc = match llm::vision_classify(&png) {
            Ok(desc) => {
                tracing::info!(event_id, desc_len = desc.len(), "vision classified");
                Some(desc)
            }
            Err(e) => {
                tracing::debug!(event_id, error = %e, "vision classify skipped");
                None
            }
        };

        Ok::<_, anyhow::Error>((ocr_text, vision_desc))
    })
    .await
    .context("OCR+vision task panicked")??;

    let conn = db::open_events_db(&data_dir)?;

    // Store OCR text if non-empty
    if !ocr_result.is_empty() {
        db::update_event_ocr(&conn, event_id, &ocr_result)?;
        tracing::info!(event_id, ocr_len = ocr_result.len(), "OCR stored for event");
    }

    // Store vision description if available
    if let Some(ref desc) = vision_result {
        db::update_event_vision(&conn, event_id, desc)?;
        tracing::info!(event_id, desc_len = desc.len(), "vision stored for event");
    }

    Ok(())
}
