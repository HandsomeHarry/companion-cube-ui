use anyhow::Result;
use ccube_capture::ActivityCapture;
use ccube_capture::windows::WinActivityCapture;
use ccube_core::db;
use ccube_core::focus_mode;
use std::collections::HashMap;

use crate::paths::DataRoot;

/// Run continuous activity capture, writing events to events.sqlite.
/// Press Ctrl+C to stop.
pub async fn handle_capture_run(root: &DataRoot) -> Result<()> {
    db::init_databases(&root.data_dir)?;
    let conn = db::open_events_db(&root.data_dir)?;

    println!("Starting capture... Press Ctrl+C to stop.");

    let capture = WinActivityCapture::new();
    let mut rx = capture.subscribe().await;

    // Track the last event per kind for duration calculation: kind -> (row_id, start_ts)
    let mut last_event: HashMap<String, (i64, i64)> = HashMap::new();
    let mut event_count: u64 = 0;
    let start_ts = chrono::Utc::now().timestamp_millis();

    loop {
        tokio::select! {
            event = rx.recv() => {
                let event = match event {
                    Some(e) => e,
                    None => {
                        println!("Capture channel closed.");
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
                };

                // Infer focus mode for app_focus events
                let mode = if kind == "app_focus" {
                    let mode = focus_mode::infer_focus_mode(
                        app.unwrap_or(""),
                        title,
                        url,
                    );
                    Some(focus_mode::focus_mode_to_str(&mode))
                } else {
                    None
                };

                // Insert event
                match db::insert_event(&conn, ts, kind, app, title, mode) {
                    Ok(row_id) => {
                        // Update duration of previous event of the same kind
                        if let Some((prev_id, prev_ts)) = last_event.get(kind) {
                            let duration = ts - prev_ts;
                            if duration > 0 {
                                let _ = db::update_event_duration(&conn, *prev_id, duration);
                            }
                        }
                        last_event.insert(kind.to_string(), (row_id, ts));
                        event_count += 1;
                    }
                    Err(e) => {
                        tracing::error!("DB write failed: {e}");
                        continue;
                    }
                }

                // Print real-time log line
                let time_str = format_time_ms(ts);
                match kind {
                    "app_focus" => {
                        let app_str = app.unwrap_or("?");
                        let title_str = title.unwrap_or("");
                        let mode_str = mode.unwrap_or("");
                        println!("[{time_str}] app_focus: {app_str} — \"{title_str}\" ({mode_str})");
                    }
                    "window_title" => {
                        let title_str = title.unwrap_or("");
                        println!("[{time_str}] title: \"{title_str}\"");
                    }
                    "url" => {
                        let url_str = title.unwrap_or("");
                        println!("[{time_str}] url: {url_str}");
                    }
                    "idle_start" => println!("[{time_str}] idle_start"),
                    "idle_end" => println!("[{time_str}] idle_end"),
                    _ => {}
                }
            }
            _ = tokio::signal::ctrl_c() => {
                println!("\nShutting down...");
                ccube_capture::windows::request_shutdown();

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
                    };
                    let mode = if kind == "app_focus" {
                        let m = focus_mode::infer_focus_mode(app.unwrap_or(""), title, url);
                        Some(focus_mode::focus_mode_to_str(&m))
                    } else {
                        None
                    };
                    if let Err(e) = db::insert_event(&conn, ts, kind, app, title, mode) {
                        eprintln!("Warning: failed to persist event: {e}");
                    }
                    event_count += 1;
                }

                // Finalize durations for open events
                let now = chrono::Utc::now().timestamp_millis();
                for (prev_id, prev_ts) in last_event.values() {
                    let duration = now - prev_ts;
                    if duration > 0
                        && let Err(e) = db::update_event_duration(&conn, *prev_id, duration)
                    {
                        eprintln!("Warning: failed to update duration: {e}");
                    }
                }

                let elapsed_min = (now - start_ts) as f64 / 60_000.0;
                println!("Captured {event_count} events over {elapsed_min:.1} minutes.");
                break;
            }
        }
    }

    Ok(())
}

fn format_time_ms(ts: i64) -> String {
    use chrono::{DateTime, Utc};
    let dt = DateTime::from_timestamp_millis(ts).unwrap_or_else(Utc::now);
    let local = dt.with_timezone(&chrono::Local);
    local.format("%H:%M:%S").to_string()
}
