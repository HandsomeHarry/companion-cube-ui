pub mod http;
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
use tray::UserEvent;

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

    // 5. Load frozen memory (spec §15: "Memory never changes mid-session")
    let frozen_profile = memory::read_profile(&root.memory_dir).unwrap_or_default();
    let frozen_patterns = memory::read_patterns(&root.memory_dir).unwrap_or_default();
    let frozen_patterns_hash = memory::patterns_hash(&frozen_patterns);

    tracing::info!(
        profile_chars = frozen_profile.len(),
        patterns_chars = frozen_patterns.len(),
        patterns_hash = %frozen_patterns_hash,
        "frozen memory loaded"
    );

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

    let state = Arc::new(AppState {
        data_root: root,
        start_time: std::time::Instant::now(),
        shutdown_token: cancel.clone(),
        version: env!("CARGO_PKG_VERSION"),
        frozen_profile,
        frozen_patterns,
        frozen_patterns_hash,
        llm: llm_client,
        curator_llm: curator_llm_client,
        detector_trigger: detector_trigger.clone(),
        curator_mutex: Arc::new(tokio::sync::Mutex::new(())),
        curator_schedule_hour,
        cached_summaries,
    });

    // 9. Build the main-thread event loop before spawning the runtime, so we can
    //    hand a proxy to the tokio thread for the shutdown handshake.
    let event_loop = tray::build_event_loop();
    let proxy = event_loop.create_proxy();

    // 10. Spawn the tokio runtime on a background thread. When it finishes its
    //     graceful shutdown it signals the event loop to exit.
    let rt_cancel = cancel.clone();
    let rt_state = state.clone();
    let _runtime_thread = std::thread::Builder::new()
        .name("ccube-tokio".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("failed to build tokio runtime");
            rt.block_on(run_runtime(rt_state, rt_cancel));
            let _ = proxy.send_event(UserEvent::Shutdown);
        })
        .context("failed to spawn tokio runtime thread")?;

    // 11. Run the tray event loop on the main thread (never returns).
    tray::run(event_loop, cancel)
}

/// Drive all async subsystems: capture loop, scheduler, summarize loop, and the
/// HTTP server. Returns only after a graceful shutdown (so the caller can tell
/// the tray event loop to exit the process).
async fn run_runtime(state: Arc<AppState>, cancel: CancellationToken) {
    // Capture loop
    let capture_cancel = cancel.clone();
    let capture_state = state.clone();
    let capture_handle = tokio::spawn(async move {
        if let Err(e) = capture_loop(&capture_state, capture_cancel).await {
            tracing::error!(error = %e, "capture loop failed");
        }
    });

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
                    match http::run_summarize(&summarize_state, None, None).await {
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
    let listener = match TcpListener::bind("127.0.0.1:7431").await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!(error = %e, "failed to bind 127.0.0.1:7431 (port already in use?)");
            cancel.cancel();
            return;
        }
    };
    tracing::info!("HTTP server listening on http://127.0.0.1:7431");

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
                        // Write OCR text to the most recent app_focus event
                        if let Some(&(prev_id, _)) = last_event.get("app_focus") {
                            if let Err(e) = db::update_event_ocr(&conn, prev_id, text) {
                                tracing::warn!(error = %e, "failed to update OCR text");
                            }
                        }
                        continue;
                    }
                };

                let mode = if kind == "app_focus" {
                    let m = focus_mode::infer_focus_mode(app.unwrap_or(""), title, url);
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
                        event_count += 1;

                        // Signal detector on app focus changes
                        if kind == "app_focus" {
                            state.detector_trigger.notify_one();
                        }

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
                            if let Some(&(prev_id, _)) = last_event.get("app_focus") {
                                let _ = db::update_event_ocr(&conn, prev_id, text);
                            }
                            continue;
                        }
                    };
                    let mode = if kind == "app_focus" {
                        let m = focus_mode::infer_focus_mode(app.unwrap_or(""), title, url);
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

/// Capture a screenshot, run OCR, and store the resulting text against a
/// completed event. Uses spawn_blocking because both capture_screenshot and
/// OCR engine are synchronous (and Windows OCR internally creates its own
/// tokio runtime, which cannot run inside an existing async context).
async fn run_ocr_for_event(data_dir: &Path, event_id: i64) -> Result<()> {
    let data_dir = data_dir.to_path_buf();
    let ocr_result = tokio::task::spawn_blocking(move || {
        let png = ccube_capture::capture_screenshot()
            .context("screenshot capture failed")?;

        let engine = ccube_capture::ocr::create_engine()
            .context("no OCR engine available on this platform")?;

        let text = engine.extract_text(&png)?;
        Ok::<_, anyhow::Error>(text)
    })
    .await
    .context("OCR task panicked")??;

    if ocr_result.is_empty() {
        tracing::debug!(event_id, "OCR produced empty text");
        return Ok(());
    }

    let conn = db::open_events_db(&data_dir)?;
    db::update_event_ocr(&conn, event_id, &ocr_result)?;

    tracing::info!(event_id, ocr_len = ocr_result.len(), "OCR stored for event");
    Ok(())
}
