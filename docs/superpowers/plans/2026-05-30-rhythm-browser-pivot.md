# Rhythm + Browser-Native Pivot Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove the Tauri shell, make the daemon serve the UI directly via browser with a system tray icon, and add the Rhythm focus analytics feature.

**Architecture:** The daemon binary becomes the entire product — it captures activity, runs the LLM pipeline, serves the HTTP API under `/api/`, serves the built SvelteKit frontend as static files at `/`, and shows a system tray icon that opens the browser. Rhythm is a new `compute_rhythm()` pure function in `ccube-core` exposed via `GET /api/rhythm?days=7`.

**Tech Stack:** Rust (axum, tray-icon, include_dir, tower-http), SvelteKit 5, TypeScript, CSS Grid for heatmap.

**Spec:** `docs/superpowers/specs/2026-05-30-rhythm-browser-pivot-design.md`

---

## File Structure

### New Files

| File | Responsibility |
|---|---|
| `crates/ccube-daemon/src/tray.rs` | System tray setup and event loop |
| `crates/ccube-core/src/rhythm.rs` | Pure rhythm analytics computation + types |
| `src/components/Rhythm.svelte` | Rhythm UI: heatmap, focus windows, fingerprint, drift |

### Modified Files

| File | Change |
|---|---|
| `crates/ccube-daemon/Cargo.toml` | Add tray-icon, include_dir, tower-http ServeDir |
| `crates/ccube-daemon/src/main.rs` | Restructure: tokio on background thread, tray on main |
| `crates/ccube-daemon/src/http.rs` | Add `/api/` prefix to all routes, add static serving fallback, add `/api/rhythm` |
| `crates/ccube-core/Cargo.toml` | No changes needed (rhythm uses chrono/serde already present) |
| `crates/ccube-core/src/lib.rs` | Add `pub mod rhythm;` |
| `src/lib/api.ts` | Change BASE to `/api`, add `rhythm()` method |
| `src/lib/stores.ts` | Add `rhythmReport` store, `fetchRhythm()` |
| `src/lib/types.ts` | Add `RhythmReport` and sub-types |
| `src/components/Rail.svelte` | Add Rhythm tab |
| `src/routes/+layout.svelte` | Remove `window.__TAURI__`, add Rhythm view, add theme toggle |
| `src/app.css` | Add `[data-theme="dark"]` variable overrides |
| `package.json` | Remove `@tauri-apps/api` and `@tauri-apps/cli` |

### Deleted

| Path | Reason |
|---|---|
| `src-tauri/` (entire directory) | Tauri no longer needed |

---

## Phase A: Remove Tauri, Restructure Daemon

### Task 1: Delete Tauri and remove npm dependencies

**Files:**
- Delete: `src-tauri/` (entire directory)
- Modify: `package.json`
- Modify: `Cargo.toml` (workspace — remove `exclude = ["src-tauri"]`)

- [ ] **Step 1: Remove `src-tauri/` directory**

```bash
rm -rf src-tauri/
```

- [ ] **Step 2: Remove Tauri npm packages**

```bash
npm uninstall @tauri-apps/api @tauri-apps/cli
```

- [ ] **Step 3: Remove `src-tauri` from workspace `Cargo.toml` exclude list**

In `Cargo.toml`, remove the line `exclude = ["src-tauri"]` so it becomes:

```toml
[workspace]
members = [
    "crates/ccube-core",
    "crates/ccube-capture",
    "crates/ccube-daemon",
    "crates/ccube-cli",
]
resolver = "2"
```

- [ ] **Step 4: Verify Rust builds and tests pass**

Run: `cargo check && cargo test`
Expected: All crates compile, 136 tests pass.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "chore: remove Tauri shell and dependencies"
```

---

### Task 2: Add `/api/` prefix to all HTTP routes

**Files:**
- Modify: `crates/ccube-daemon/src/http.rs`

The goal: every existing route gets `/api` prepended so `/health` becomes `/api/health`, etc. This avoids colliding with frontend static file routes.

- [ ] **Step 1: Add `/api` prefix to every route in `router()`**

In `crates/ccube-daemon/src/http.rs`, change the `router()` function. Wrap all API routes in a nested router with `/api` prefix, then add the static file fallback last:

```rust
pub fn router(state: Arc<AppState>) -> Router {
    let api = Router::new()
        .route("/health", get(health))
        .route("/activity", get(activity))
        .route("/briefing", get(get_briefing))
        .route("/detect", post(detect))
        .route("/memory/profile", get(memory_profile))
        .route("/memory/patterns", get(memory_patterns))
        .route("/memory/patterns/history", get(patterns_history))
        .route("/shutdown", post(shutdown))
        .route("/corrections", get(list_corrections_handler).post(create_correction))
        .route("/corrections/{id}", get(get_correction_handler))
        .route("/corrections/group", post(create_group_correction))
        .route("/decisions", get(list_decisions_handler))
        .route("/agents/curator/run", post(run_curator_handler))
        .route("/agents/reflector/run", post(run_reflector_handler))
        .route("/agents/reflector/pending", get(get_pending_handler))
        .route("/agents/reflector/accept", post(accept_pending_handler))
        .route("/agents/reflector/reject", post(reject_pending_handler))
        .route("/config/llm", get(get_llm_config).put(set_llm_config))
        .route("/summaries", get(get_summaries))
        .route("/summarize", post(run_summarize_handler))
        .with_state(state);

    Router::new()
        .nest("/api", api)
        .layer(CorsLayer::permissive())
}
```

Note: static file serving will be added in Task 4. For now the routes are just prefixed.

- [ ] **Step 2: Verify it compiles**

Run: `cargo check -p ccube-daemon`
Expected: Compiles successfully.

- [ ] **Step 3: Commit**

```bash
git add crates/ccube-daemon/src/http.rs
git commit -m "refactor: prefix all HTTP routes with /api"
```

---

### Task 3: Update frontend to use `/api` base, remove Tauri usage

**Files:**
- Modify: `src/lib/api.ts`
- Modify: `src/routes/+layout.svelte`

- [ ] **Step 1: Update `api.ts` BASE**

In `src/lib/api.ts`, change line 3:

```typescript
const BASE = '/api';
```

Remove the `http://127.0.0.1:7431` absolute URL. The frontend is now served from the same origin as the API.

- [ ] **Step 2: Remove Tauri usage from `+layout.svelte`**

In `src/routes/+layout.svelte`, find the `openUrl` function (~line 307-314). Replace the entire function body:

```typescript
function openUrl(url: string) {
    window.open(url, '_blank');
}
```

Remove the old `window.__TAURI__` detection and `tauri.shell.open` call.

- [ ] **Step 3: Remove the Tauri `start_daemon` invoke from Settings**

In `src/routes/+layout.svelte`, find the `handleStartDaemon` function (~line 355-384). Replace it with a version that doesn't use Tauri invoke:

```typescript
async function handleStartDaemon() {
    daemonStarting = true;
    daemonMsg = '';
    try {
        daemonMsg = 'Daemon should already be running. Checking...';
        const res = await fetch('/api/health');
        if (res.ok) {
            daemonOnline.set(true);
            daemonMsg = 'Daemon is running ✓';
            await refreshDaemonInfo();
            await loadLlmSettings();
        } else {
            daemonMsg = 'Daemon is not responding. Start it manually: ccube-daemon';
        }
    } catch {
        daemonMsg = 'Cannot reach daemon. Start it manually: ccube-daemon';
    } finally {
        daemonStarting = false;
    }
}
```

Remove the `(window as any).__TAURI__` and `invoke` references.

- [ ] **Step 4: Verify svelte-check passes**

Run: `npx svelte-check --tsconfig ./tsconfig.json`
Expected: 0 errors.

- [ ] **Step 5: Commit**

```bash
git add src/lib/api.ts src/routes/+layout.svelte
git commit -m "refactor: use relative /api base, remove Tauri window calls"
```

---

### Task 4: Add static file serving and tray dependencies

**Files:**
- Modify: `crates/ccube-daemon/Cargo.toml`
- Modify: `crates/ccube-daemon/src/http.rs`

- [ ] **Step 1: Add new dependencies to `ccube-daemon/Cargo.toml`**

Append to the `[dependencies]` section:

```toml
tray-icon = "0.21"
muda = "0.17"
include_dir = "0.7"
tower-http = { version = "0.6", features = ["cors", "fs"] }
```

Note: `tower-http` already exists with just `cors` feature — replace that line with the new one that adds `fs`.

- [ ] **Step 2: Add static file serving to the router**

In `crates/ccube-daemon/src/http.rs`, at the top add the include_dir macro:

```rust
use include_dir::include_dir;
use include_dir::Dir;

static UI_DIR: Dir = include_dir!("$CARGO_MANIFEST_DIR/../../build");
```

Then in the `router()` function, add the static file fallback after the API nest:

```rust
pub fn router(state: Arc<AppState>) -> Router {
    let api = Router::new()
        // ... all existing routes ...
        .with_state(state);

    Router::new()
        .nest("/api", api)
        .fallback(serve_frontend)
        .layer(CorsLayer::permissive())
}

async fn serve_frontend(req: axum::extract::Request) -> axum::response::Response {
    let path = req.uri().path();
    // Try exact file first
    let file_path = if path == "/" { "index.html" } else { path.trim_start_matches('/') };
    if let Some(file) = UI_DIR.get_file(file_path) {
        let content_type = match file_path.rsplit('.').next() {
            Some("html") => "text/html",
            Some("js") => "application/javascript",
            Some("css") => "text/css",
            Some("svg") => "image/svg+xml",
            Some("png") => "image/png",
            Some("ico") => "image/x-icon",
            Some("json") => "application/json",
            Some("wasm") => "application/wasm",
            _ => "application/octet-stream",
        };
        return axum::response::IntoResponse::into_response(
            ([(axum::http::header::CONTENT_TYPE, content_type)], file.contents())
        );
    }
    // SPA fallback: serve index.html for any unmatched route
    if let Some(index) = UI_DIR.get_file("index.html") {
        return axum::response::IntoResponse::into_response(
            ([(axum::http::header::CONTENT_TYPE, "text/html")], index.contents())
        );
    }
    axum::http::StatusCode::NOT_FOUND.into_response()
}
```

- [ ] **Step 3: Verify it compiles**

Run: `npm run build && cargo check -p ccube-daemon`
Expected: Compiles. (The `include_dir!` macro needs the `build/` directory to exist from a prior frontend build.)

- [ ] **Step 4: Commit**

```bash
git add crates/ccube-daemon/Cargo.toml crates/ccube-daemon/src/http.rs
git commit -m "feat: add static file serving with include_dir"
```

---

### Task 5: Restructure daemon main() — tray on main thread, tokio on background

**Files:**
- Create: `crates/ccube-daemon/src/tray.rs`
- Modify: `crates/ccube-daemon/src/main.rs`

This is the most complex task. The current `main()` uses `#[tokio::main]` (tokio on the main thread). We need to flip this: tokio runs on a background thread, the main thread runs the tray event loop (required by macOS).

- [ ] **Step 1: Create `crates/ccube-daemon/src/tray.rs`**

```rust
//! System tray icon and menu.
//! On macOS the tray event loop MUST run on the main thread.

use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use tray_icon::{
    TrayIconBuilder, menu::{Menu, MenuEvent, MenuItem},
};
use muda::ContextMenu;

pub fn run_tray(cancel: CancellationToken) {
    // Create menu
    let menu = Menu::new();
    let open_item = MenuItem::new("Open Dashboard", true, None);
    let quit_item = MenuItem::new("Quit", true, None);
    menu.append_items(&[&open_item, &quit_item]).ok();

    // Create tray icon
    let _tray = TrayIconBuilder::new()
        .icon(get_icon())
        .tooltip("Companion Cube")
        .menu(&menu)
        .build()
        .expect("failed to create tray icon");

    // Listen for menu events
    MenuEvent::set_event_handler(Some(move |event| {
        if event.id == open_item.id() {
            open_browser("http://localhost:7431");
        } else if event.id == quit_item.id() {
            cancel.cancel();
        }
    }));

    // Run the event loop — blocks until process exits
    // On macOS this is CFRunLoop, on Windows it's a GetMessage loop
    let mut line = String::new();
    // Simple blocking read — the tray events are handled via the callback above.
    // When cancel fires, the tokio runtime shuts down, and the process exits.
    loop {
        line.clear();
        if std::io::stdin().read_line(&mut line).is_err() {
            break;
        }
        if cancel.is_cancelled() {
            break;
        }
    }
}

fn open_browser(url: &str) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(url).spawn();

    #[cfg(target_os = "linux")]
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();

    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("cmd")
        .args(["/c", "start", url])
        .spawn();
}

fn get_icon() -> tray_icon::icon::Icon {
    // Create a simple 32x32 orange icon programmatically
    // The tray-icon crate expects RGBA data
    let size = 32u32;
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    let brand_r = 0xF1u8;
    let brand_g = 0x6Au8;
    let brand_b = 0x01u8;
    for y in 0..size {
        for x in 0..size {
            // Circle mask: distance from center
            let dx = (x as i32) - 16;
            let dy = (y as i32) - 16;
            let dist = ((dx * dx + dy * dy) as f64).sqrt();
            if dist < 12.0 {
                rgba.push(brand_r);
                rgba.push(brand_g);
                rgba.push(brand_b);
                rgba.push(255);
            } else {
                rgba.push(0);
                rgba.push(0);
                rgba.push(0);
                rgba.push(0);
            }
        }
    }
    tray_icon::icon::Icon::from_rgba(rgba, size, size).expect("failed to create icon")
}
```

- [ ] **Step 2: Restructure `main.rs`**

Replace the current `#[tokio::main] async fn main()` with:

```rust
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
use tokio_util::sync::CancellationToken;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

use http::AppState;

fn main() -> Result<()> {
    dotenvy::dotenv().ok();

    // 1. Resolve paths and init databases (sync, main thread)
    let root = DataRoot::resolve()?;
    db::init_databases(&root.data_dir)?;

    // 2. Setup logging
    let file_appender = tracing_appender::rolling::never(&root.logs_dir, "daemon.ndjson");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
    let json_layer = tracing_subscriber::fmt::layer()
        .json()
        .with_writer(non_blocking);
    let filter = EnvFilter::try_from_env("CCUBE_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    let is_tty = std::io::IsTerminal::is_terminal(&std::io::stdout());
    let stdout_layer = if is_tty {
        Some(tracing_subscriber::fmt::layer().compact().with_target(false))
    } else {
        None
    };
    tracing_subscriber::registry()
        .with(filter)
        .with(json_layer)
        .with(stdout_layer)
        .init();
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "ccube-daemon starting");

    // 3. Session fence (same as before)
    {
        let conn = db::open_events_db(&root.data_dir)?;
        let now_ms = chrono::Utc::now().timestamp_millis();
        let last_start = db::last_event_of_kind(&conn, "daemon_start")?;
        let last_stop = db::last_event_of_kind(&conn, "daemon_stop")?;
        let clean_shutdown = match (&last_start, &last_stop) {
            (Some(start), Some(stop)) => stop.ts >= start.ts,
            (None, _) => true,
            (Some(_), None) => false,
        };
        if !clean_shutdown {
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
                tracing::warn!(fixed, "crash recovery: finalized {fixed} stale events from previous session");
            }
        }
        db::insert_event(&conn, now_ms, "daemon_start", None, None, None)?;
        tracing::info!("session fence: daemon_start sentinel inserted");
    }

    // 4. Write PID file
    let pid_file = root.data_dir.join("daemon.pid");
    std::fs::write(&pid_file, std::process::id().to_string())?;

    // 5. Load frozen memory
    let frozen_profile = memory::read_profile(&root.memory_dir).unwrap_or_default();
    let frozen_patterns = memory::read_patterns(&root.memory_dir).unwrap_or_default();
    let frozen_patterns_hash = memory::patterns_hash(&frozen_patterns);
    tracing::info!(
        profile_chars = frozen_profile.len(),
        patterns_chars = frozen_patterns.len(),
        patterns_hash = %frozen_patterns_hash,
        "frozen memory loaded"
    );

    // 6. Create LLM clients
    let llm_client: Arc<dyn ccube_core::llm::LlmBackend> =
        Arc::new(llm::LlamaCppClient::from_env().map_err(|e| anyhow::anyhow!(e))?);
    let curator_llm_client: Arc<dyn ccube_core::llm::LlmBackend> = Arc::new(
        llm::LlamaCppClient::from_env_with_timeout(Duration::from_secs(120))
            .map_err(|e| anyhow::anyhow!(e))?,
    );

    // 7. Config
    let curator_schedule_hour: u32 = std::env::var("CCUBE_CURATOR_HOUR")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(5)
        .min(23);

    // 8. Create shared state
    let cancel = CancellationToken::new();
    let detector_trigger = Arc::new(tokio::sync::Notify::new());
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

    // 9. Spawn tokio runtime on background thread
    let tokio_cancel = cancel.clone();
    let tokio_state = state.clone();
    let _guard = _guard; // move log guard into this scope

    let tokio_handle = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("failed to build tokio runtime");
        rt.block_on(async move {
            // Spawn capture loop
            let capture_cancel = tokio_cancel.clone();
            let capture_state = tokio_state.clone();
            let capture_handle = tokio::spawn(async move {
                if let Err(e) = capture_loop(&capture_state, capture_cancel).await {
                    tracing::error!(error = %e, "capture loop failed");
                }
            });

            // Spawn scheduler
            let scheduler_cancel = tokio_cancel.clone();
            let scheduler_state = tokio_state.clone();
            let scheduler_handle =
                tokio::spawn(scheduler::run_scheduler(scheduler_state, scheduler_cancel));

            // Spawn summarize scheduler
            let summarize_state = tokio_state.clone();
            let summarize_cancel = tokio_cancel.clone();
            let summarize_handle = tokio::spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_secs(300));
                interval.tick().await;
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
            let listener = tokio::net::TcpListener::bind("127.0.0.1:7431").await?;
            tracing::info!("HTTP server listening on 127.0.0.1:7431");
            let router = http::router(tokio_state.clone());
            let server_cancel = tokio_cancel.clone();
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

            // Ctrl-C handler
            let ctrl_cancel = tokio_cancel.clone();
            tokio::spawn(async move {
                let _ = tokio::signal::ctrl_c().await;
                tracing::info!("Ctrl-C received, initiating shutdown");
                ctrl_cancel.cancel();
            });

            // Wait for cancellation
            tokio_cancel.cancelled().await;
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

            // Cleanup
            if let Ok(conn) = db::open_events_db(&tokio_state.data_root.data_dir) {
                let stop_ts = chrono::Utc::now().timestamp_millis();
                let _ = db::insert_event(&conn, stop_ts, "daemon_stop", None, None, None);
                tracing::info!("session fence: daemon_stop sentinel inserted");
            }
            let pid_path = tokio_state.data_root.data_dir.join("daemon.pid");
            let _ = std::fs::remove_file(&pid_path);
            tracing::info!("ccube-daemon stopped");

            Ok::<(), anyhow::Error>(())
        })
    });

    // 10. Run tray on main thread (blocks until cancel)
    tray::run_tray(cancel);

    // Wait for tokio thread to finish
    let _ = tokio_handle.join();

    Ok(())
}
```

Note: The `capture_loop` and `run_ocr_for_event` functions remain unchanged — they are just called from within the tokio runtime block_on instead of directly from `#[tokio::main]`. Copy them exactly as they are from the current `main.rs`.

- [ ] **Step 3: Verify it compiles**

Run: `npm run build && cargo check -p ccube-daemon`
Expected: Compiles successfully. Warnings about unused variables in the restructured code are OK.

- [ ] **Step 4: Commit**

```bash
git add crates/ccube-daemon/src/main.rs crates/ccube-daemon/src/tray.rs
git commit -m "feat: restructure daemon — tray on main thread, tokio on background"
```

---

### Task 6: Add theme switching (light/dark)

**Files:**
- Modify: `src/app.css`
- Modify: `src/routes/+layout.svelte`
- Modify: `src/components/Rail.svelte`

- [ ] **Step 1: Add dark theme CSS variables to `app.css`**

Append after the existing `::root` block:

```css
[data-theme="dark"] {
  --brand-orange: #F16A01;
  --brand-orange-hov: #FF8A33;
  --brand-orange-deep: #FF9E55;
  --gold-star: #E8A41E;

  --paper: #1A1814;
  --paper-panel: #242018;
  --card-white: #2A2520;
  --titlebar: #1E1A16;

  --ink: #E8E0D4;
  --ink-soft: #A39B8E;
  --ink-faint: #6B6359;
  --on-orange: #FFFFFF;

  --divider: #3A342A;
  --row-hover: #342E24;

  --shadow-float: 0 12px 32px rgba(0, 0, 0, 0.4);
  --shadow-window: 0 24px 64px rgba(0, 0, 0, 0.6);
  --shadow-rest: 0 2px 8px rgba(0, 0, 0, 0.2);
}
```

- [ ] **Step 2: Add theme toggle and initialization to `+layout.svelte`**

In the `<script>` section of `+layout.svelte`, add after the existing imports:

```typescript
// Theme
let theme: 'light' | 'dark' = 'light';

function initTheme() {
    const stored = localStorage.getItem('ccube-theme');
    if (stored === 'dark' || stored === 'light') {
        theme = stored;
    } else if (window.matchMedia('(prefers-color-scheme: dark)').matches) {
        theme = 'dark';
    }
    document.documentElement.setAttribute('data-theme', theme);
}

function toggleTheme() {
    theme = theme === 'dark' ? 'light' : 'dark';
    document.documentElement.setAttribute('data-theme', theme);
    localStorage.setItem('ccube-theme', theme);
}
```

In `onMount`, call `initTheme()` at the top:

```typescript
onMount(() => {
    initTheme();
    // ... rest of existing onMount code
});
```

- [ ] **Step 3: Add theme toggle button to `Rail.svelte`**

In `Rail.svelte`, add a theme toggle button in `rail__bottom` above the settings button:

```svelte
<div class="rail__bottom">
    <button
      class="rail__btn"
      on:click={() => {
        const current = document.documentElement.getAttribute('data-theme');
        const next = current === 'dark' ? 'light' : 'dark';
        document.documentElement.setAttribute('data-theme', next);
        localStorage.setItem('ccube-theme', next);
      }}
      aria-label="Toggle theme"
      title="Toggle theme"
    >
      <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <circle cx="12" cy="12" r="5"/>
        <line x1="12" y1="1" x2="12" y2="3"/>
        <line x1="12" y1="21" x2="12" y2="23"/>
        <line x1="4.22" y1="4.22" x2="5.64" y2="5.64"/>
        <line x1="18.36" y1="18.36" x2="19.78" y2="19.78"/>
        <line x1="1" y1="12" x2="3" y2="12"/>
        <line x1="21" y1="12" x2="23" y2="12"/>
        <line x1="4.22" y1="19.78" x2="5.64" y2="18.36"/>
        <line x1="18.36" y1="5.64" x2="19.78" y2="4.22"/>
      </svg>
    </button>

    <!-- existing settings button -->
```

- [ ] **Step 4: Verify svelte-check passes**

Run: `npx svelte-check --tsconfig ./tsconfig.json`
Expected: 0 errors.

- [ ] **Step 5: Commit**

```bash
git add src/app.css src/routes/+layout.svelte src/components/Rail.svelte
git commit -m "feat: add light/dark theme switching with CSS variables"
```

---

### Task 7: End-to-end smoke test of Phase A

- [ ] **Step 1: Build frontend**

Run: `npm run build`
Expected: Build completes, `build/` directory exists.

- [ ] **Step 2: Build daemon**

Run: `cargo build -p ccube-daemon`
Expected: Compiles. The `include_dir!` macro embeds the frontend.

- [ ] **Step 3: Run daemon and verify tray + HTTP**

```bash
# In one terminal:
cargo run -p ccube-daemon

# In another terminal:
curl http://localhost:7431/api/health
# Expected: {"status":"ok",...}

curl http://localhost:7431/
# Expected: HTML of the SvelteKit frontend
```

- [ ] **Step 4: Run full test suite**

Run: `cargo test && npx svelte-check --tsconfig ./tsconfig.json`
Expected: 136+ tests pass, 0 type errors.

---

## Phase B: Rhythm Backend

### Task 8: Create `rhythm.rs` with types and `compute_rhythm()`

**Files:**
- Create: `crates/ccube-core/src/rhythm.rs`
- Modify: `crates/ccube-core/src/lib.rs`

- [ ] **Step 1: Create `crates/ccube-core/src/rhythm.rs`**

```rust
//! Rhythm analytics — pure computation over activity events.
//! No I/O, no LLM calls. Fully testable with fixture data.

use crate::db::EventRow;
use chrono::{DateTime, Datelike, Timelike, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ---- Types ----

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RhythmReport {
    pub focus_windows: Vec<FocusWindow>,
    pub fingerprint: Vec<AppCluster>,
    pub drift_origins: Vec<DriftOrigin>,
    pub heatmap: HeatmapData,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FocusWindow {
    pub hour_start: u8,
    pub hour_end: u8,
    pub total_focus_ms: i64,
    pub label: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AppCluster {
    pub apps: Vec<String>,
    pub session_count: u32,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DriftOrigin {
    pub app: String,
    pub from_app: String,
    pub count: u32,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct HeatmapData {
    pub cells: Vec<u32>,
    pub max_value: u32,
    pub day_labels: Vec<String>,
    pub hour_labels: Vec<String>,
}

// ---- Computation ----

pub fn compute_rhythm(events: &[EventRow]) -> RhythmReport {
    RhythmReport {
        focus_windows: compute_focus_windows(events),
        fingerprint: compute_fingerprint(events),
        drift_origins: compute_drift_origins(events),
        heatmap: compute_heatmap(events),
    }
}

fn is_focus_mode(mode: Option<&str>) -> bool {
    mode.map_or(false, |m| m != "distraction" && m != "idle" && m != "unknown")
}

fn format_hour(h: u8) -> String {
    match h {
        0 => "12AM".to_string(),
        12 => "12PM".to_string(),
        1..=11 => format!("{}AM", h),
        13..=23 => format!("{}PM", h - 12),
        _ => "?".to_string(),
    }
}

fn compute_focus_windows(events: &[EventRow]) -> Vec<FocusWindow> {
    let mut buckets: [i64; 24] = [0; 24];

    for e in events {
        if !is_focus_mode(e.mode.as_deref()) {
            continue;
        }
        let dt = DateTime::<Utc>::from_timestamp_millis(e.ts);
        if let Some(dt) = dt {
            let hour = dt.hour() as usize;
            if hour < 24 {
                buckets[hour] += e.duration_ms.unwrap_or(0);
            }
        }
    }

    // Find contiguous ranges of 2+ hours with highest combined focus
    let mut windows: Vec<FocusWindow> = Vec::new();
    for start in 0..24 {
        for len in (2..=6).rev() {
            if start + len > 24 {
                continue;
            }
            let total: i64 = buckets[start..start + len].iter().sum();
            if total > 0 {
                windows.push(FocusWindow {
                    hour_start: start as u8,
                    hour_end: (start + len) as u8,
                    total_focus_ms: total,
                    label: format!("{} – {}", format_hour(start as u8), format_hour((start + len) as u8)),
                });
            }
        }
    }

    windows.sort_by(|a, b| b.total_focus_ms.cmp(&a.total_focus_ms));
    windows.truncate(3);
    windows
}

fn compute_fingerprint(events: &[EventRow]) -> Vec<AppCluster> {
    // Group consecutive same-app focus events into sessions (gap < 5min)
    let focus_events: Vec<&EventRow> = events
        .iter()
        .filter(|e| is_focus_mode(e.mode.as_deref()))
        .collect();

    if focus_events.is_empty() {
        return Vec::new();
    }

    // Group into 2-hour windows and collect app sets
    let mut window_apps: Vec<Vec<String>> = Vec::new();
    let mut current_window_start: i64 = focus_events[0].ts;
    let mut current_apps: Vec<String> = Vec::new();

    let two_hours_ms: i64 = 2 * 60 * 60 * 1000;

    for e in &focus_events {
        if e.ts - current_window_start > two_hours_ms {
            if !current_apps.is_empty() {
                current_apps.sort();
                current_apps.dedup();
                if current_apps.len() >= 2 {
                    window_apps.push(current_apps.clone());
                }
            }
            current_window_start = e.ts;
            current_apps.clear();
        }
        if let Some(app) = &e.app {
            current_apps.push(app.clone());
        }
    }
    // Flush last window
    if !current_apps.is_empty() {
        current_apps.sort();
        current_apps.dedup();
        if current_apps.len() >= 2 {
            window_apps.push(current_apps);
        }
    }

    // Count co-occurrences
    let mut counts: HashMap<Vec<String>, u32> = HashMap::new();
    for apps in &window_apps {
        *counts.entry(apps.clone()).or_insert(0) += 1;
    }

    let mut clusters: Vec<AppCluster> = counts
        .into_iter()
        .map(|(apps, count)| AppCluster { apps, session_count: count })
        .collect();
    clusters.sort_by(|a, b| b.session_count.cmp(&a.session_count));
    clusters.truncate(5);
    clusters
}

fn compute_drift_origins(events: &[EventRow]) -> Vec<DriftOrigin> {
    let mut transitions: HashMap<String, HashMap<String, u32>> = HashMap::new();
    let five_min_ms: i64 = 5 * 60 * 1000;

    for i in 1..events.len() {
        let prev = &events[i - 1];
        let curr = &events[i];

        // Previous was focus, current is not, within 5 minutes
        if curr.ts - prev.ts > five_min_ms {
            continue;
        }
        if !is_focus_mode(prev.mode.as_deref()) {
            continue;
        }
        if is_focus_mode(curr.mode.as_deref()) {
            continue;
        }

        let to_app = curr.app.clone().unwrap_or_else(|| curr.kind.clone());
        let from_app = prev.app.clone().unwrap_or_else(|| prev.kind.clone());

        transitions
            .entry(to_app)
            .or_default()
            .entry(from_app)
            .and_modify(|c| *c += 1)
            .or_insert(1);
    }

    let mut origins: Vec<DriftOrigin> = transitions
        .into_iter()
        .map(|(app, froms)| {
            let total: u32 = froms.values().sum();
            let top_from = froms
                .into_iter()
                .max_by_key(|(_, c)| *c)
                .map(|(f, _)| f)
                .unwrap_or_default();
            DriftOrigin { app, from_app: top_from, count: total }
        })
        .collect();

    origins.sort_by(|a, b| b.count.cmp(&a.count));
    origins.truncate(5);
    origins
}

fn compute_heatmap(events: &[EventRow]) -> HeatmapData {
    // 24 hours × 7 days = 168 cells, row-major (Mon 12AM, Mon 1AM, ..., Sun 11PM)
    let mut cells: [u32; 168] = [0; 168];

    for e in events {
        let dt = DateTime::<Utc>::from_timestamp_millis(e.ts);
        if let Some(dt) = dt {
            let hour = dt.hour() as usize;
            let day = dt.weekday().num_days_from_monday() as usize; // Mon=0
            let idx = day * 24 + hour;
            if idx < 168 {
                let dur_min = (e.duration_ms.unwrap_or(0) / 60_000) as u32;
                cells[idx] += dur_min;
            }
        }
    }

    let max_value = *cells.iter().max().unwrap_or(&0).max(&1);

    HeatmapData {
        cells: cells.to_vec(),
        max_value,
        day_labels: vec!["Mon".into(), "Tue".into(), "Wed".into(), "Thu".into(), "Fri".into(), "Sat".into(), "Sun".into()],
        hour_labels: (0..24).map(|h| format_hour(h as u8)).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_event(id: i64, ts: i64, app: &str, mode: Option<&str>, duration_ms: Option<i64>) -> EventRow {
        EventRow {
            id,
            ts,
            kind: "app_focus".to_string(),
            app: Some(app.to_string()),
            title: None,
            duration_ms,
            mode: mode.map(|m| m.to_string()),
            ocr_text: None,
        }
    }

    #[test]
    fn test_empty_events() {
        let report = compute_rhythm(&[]);
        assert!(report.focus_windows.is_empty());
        assert!(report.fingerprint.is_empty());
        assert!(report.drift_origins.is_empty());
        assert_eq!(report.heatmap.cells.len(), 168);
        assert_eq!(report.heatmap.max_value, 1); // min 1 for normalization
    }

    #[test]
    fn test_heatmap_values() {
        // Monday 9AM, 30 min focus
        let monday_9am = chrono::NaiveDate::from_ymd_opt(2026, 5, 25).unwrap()
            .and_hms_opt(9, 0, 0).unwrap()
            .and_utc().timestamp_millis();
        let events = vec![make_event(1, monday_9am, "VS Code", Some("deep_focus"), Some(30 * 60 * 1000))];
        let report = compute_rhythm(&events);
        // Monday (day 0) * 24 + hour 9 = cell 9
        assert_eq!(report.heatmap.cells[9], 30);
        assert_eq!(report.heatmap.max_value, 30);
    }

    #[test]
    fn test_focus_windows_ranks_highest() {
        // Create events at 9AM and 10AM (high focus) and 2PM (low focus)
        let base = chrono::NaiveDate::from_ymd_opt(2026, 5, 25).unwrap()
            .and_hms_opt(0, 0, 0).unwrap()
            .and_utc().timestamp_millis();
        let hour_ms = 3_600_000i64;

        let events = vec![
            make_event(1, base + 9 * hour_ms, "VS Code", Some("deep_focus"), Some(60 * 60 * 1000)),
            make_event(2, base + 10 * hour_ms, "VS Code", Some("deep_focus"), Some(60 * 60 * 1000)),
            make_event(3, base + 14 * hour_ms, "Safari", Some("deep_focus"), Some(15 * 60 * 1000)),
        ];

        let report = compute_rhythm(&events);
        assert!(!report.focus_windows.is_empty());
        // 9-11 AM window should rank first
        assert_eq!(report.focus_windows[0].hour_start, 9);
    }

    #[test]
    fn test_drift_origins() {
        let base = chrono::NaiveDate::from_ymd_opt(2026, 5, 25).unwrap()
            .and_hms_opt(10, 0, 0).unwrap()
            .and_utc().timestamp_millis();

        let events = vec![
            make_event(1, base, "VS Code", Some("deep_focus"), Some(60 * 1000)),
            make_event(2, base + 30_000, "YouTube", Some("distraction"), Some(60 * 1000)),
            make_event(3, base + 120_000, "VS Code", Some("deep_focus"), Some(60 * 1000)),
            make_event(4, base + 150_000, "YouTube", Some("distraction"), Some(60 * 1000)),
        ];

        let report = compute_rhythm(&events);
        assert!(!report.drift_origins.is_empty());
        assert_eq!(report.drift_origins[0].app, "YouTube");
        assert_eq!(report.drift_origins[0].from_app, "VS Code");
        assert_eq!(report.drift_origins[0].count, 2);
    }

    #[test]
    fn test_fingerprint_clusters() {
        let base = chrono::NaiveDate::from_ymd_opt(2026, 5, 25).unwrap()
            .and_hms_opt(9, 0, 0).unwrap()
            .and_utc().timestamp_millis();

        let mut events = Vec::new();
        for i in 0..4 {
            let offset = (i * 60 * 60 * 1000) as i64; // 1 hour apart
            events.push(make_event(i * 2 + 1, base + offset, "VS Code", Some("deep_focus"), Some(30 * 60 * 1000)));
            events.push(make_event(i * 2 + 2, base + offset + 60_000, "Terminal", Some("deep_focus"), Some(10 * 60 * 1000)));
        }

        let report = compute_rhythm(&events);
        assert!(!report.fingerprint.is_empty());
        let top = &report.fingerprint[0];
        assert!(top.apps.contains(&"VS Code".to_string()));
        assert!(top.apps.contains(&"Terminal".to_string()));
    }
}
```

- [ ] **Step 2: Register the module in `lib.rs`**

Add `pub mod rhythm;` to `crates/ccube-core/src/lib.rs`.

- [ ] **Step 3: Run tests**

Run: `cargo test -p ccube-core rhythm`
Expected: 5 new tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/ccube-core/src/rhythm.rs crates/ccube-core/src/lib.rs
git commit -m "feat: add rhythm analytics computation (focus windows, fingerprint, drift, heatmap)"
```

---

### Task 9: Add `GET /api/rhythm` endpoint to daemon

**Files:**
- Modify: `crates/ccube-daemon/src/http.rs`

- [ ] **Step 1: Add the rhythm handler and route**

In `crates/ccube-daemon/src/http.rs`, add a query struct and handler:

```rust
// Add import at top:
use ccube_core::rhythm;

// Add query struct near the other query structs:
#[derive(Deserialize)]
struct RhythmQuery {
    days: Option<u64>,
}

// Add handler:
async fn get_rhythm(
    State(state): State<Arc<AppState>>,
    Query(params): Query<RhythmQuery>,
) -> Result<Json<rhythm::RhythmReport>, ApiError> {
    let days = params.days.unwrap_or(7);
    let since_ms = chrono::Utc::now().timestamp_millis() - (days as i64 * 24 * 60 * 60 * 1000);
    let conn = db::open_events_db(&state.data_root.data_dir)
        .map_err(|e| ApiError::internal(&format!("db error: {e}")))?;
    let events = db::query_events_since(&conn, since_ms)
        .map_err(|e| ApiError::internal(&format!("db error: {e}")))?;
    let report = rhythm::compute_rhythm(&events);
    Ok(Json(report))
}
```

Add the route to the `router()` function's API nest:

```rust
.route("/rhythm", get(get_rhythm))
```

- [ ] **Step 2: Check if `db::query_events_since` exists**

Search `crates/ccube-core/src/db.rs` for `query_events_since`. If it doesn't exist, check what similar function does exist (likely `query_recent_events` which takes a ts parameter). Use that function instead, adjusting the call accordingly.

- [ ] **Step 3: Verify it compiles**

Run: `cargo check -p ccube-daemon`
Expected: Compiles.

- [ ] **Step 4: Commit**

```bash
git add crates/ccube-daemon/src/http.rs
git commit -m "feat: add GET /api/rhythm?days=7 endpoint"
```

---

## Phase C: Rhythm Frontend

### Task 10: Add Rhythm types, API method, and store

**Files:**
- Modify: `src/lib/types.ts`
- Modify: `src/lib/api.ts`
- Modify: `src/lib/stores.ts`

- [ ] **Step 1: Add Rhythm types to `types.ts`**

Append to `src/lib/types.ts`:

```typescript
export interface FocusWindow {
  hour_start: number;
  hour_end: number;
  total_focus_ms: number;
  label: string;
}

export interface AppCluster {
  apps: string[];
  session_count: number;
}

export interface DriftOrigin {
  app: string;
  from_app: string;
  count: number;
}

export interface HeatmapData {
  cells: number[];
  max_value: number;
  day_labels: string[];
  hour_labels: string[];
}

export interface RhythmReport {
  focus_windows: FocusWindow[];
  fingerprint: AppCluster[];
  drift_origins: DriftOrigin[];
  heatmap: HeatmapData;
}
```

- [ ] **Step 2: Add API method to `api.ts`**

In `src/lib/api.ts`, add to the `api` object:

```typescript
rhythm: (days?: number) =>
    request<RhythmReport>(`/rhythm${days ? `?days=${days}` : ''}`),
```

Import `RhythmReport` in the import at the top of `api.ts`:

```typescript
import type { EventRow, SummariesResponse, RhythmReport } from './types';
```

- [ ] **Step 3: Add store and fetch function to `stores.ts`**

In `src/lib/stores.ts`, add:

```typescript
import type { ..., RhythmReport } from './types';

export const rhythmReport = writable<RhythmReport | null>(null);

export async function fetchRhythm(days = 7) {
    try {
        const data = await api.rhythm(days);
        rhythmReport.set(data);
        return data;
    } catch {
        return null;
    }
}
```

- [ ] **Step 4: Verify svelte-check**

Run: `npx svelte-check --tsconfig ./tsconfig.json`
Expected: 0 errors.

- [ ] **Step 5: Commit**

```bash
git add src/lib/types.ts src/lib/api.ts src/lib/stores.ts
git commit -m "feat: add Rhythm types, API method, and store"
```

---

### Task 11: Create `Rhythm.svelte` component

**Files:**
- Create: `src/components/Rhythm.svelte`

- [ ] **Step 1: Create the Rhythm component**

Create `src/components/Rhythm.svelte`:

```svelte
<script lang="ts">
  import type { RhythmReport } from '$lib/types';

  export let report: RhythmReport;

  function formatDuration(ms: number): string {
    const totalMin = Math.round(ms / 60_000);
    if (totalMin < 60) return `${totalMin}m`;
    const h = Math.floor(totalMin / 60);
    const m = totalMin % 60;
    return m > 0 ? `${h}h ${m}m` : `${h}h`;
  }
</script>

<div class="rhythm">
  <!-- Heatmap -->
  <section class="rhythm__section">
    <h2 class="rhythm__heading">Weekly Activity</h2>
    <div class="heatmap">
      <!-- Day labels column + hour columns -->
      <div class="heatmap__row heatmap__header">
        <div class="heatmap__day-label"></div>
        {#each report.heatmap.hour_labels as label, i}
          {#if i % 3 === 0}
            <div class="heatmap__hour-label">{label}</div>
          {:else}
            <div class="heatmap__hour-label"></div>
          {/if}
        {/each}
      </div>
      {#each report.heatmap.day_labels as day, dayIdx}
        <div class="heatmap__row">
          <div class="heatmap__day-label">{day}</div>
          {#each report.heatmap.hour_labels as _, hourIdx}
            {@const cellIdx = dayIdx * 24 + hourIdx}
            {@const value = report.heatmap.cells[cellIdx]}
            {@const opacity = report.heatmap.max_value > 0 ? value / report.heatmap.max_value : 0}
            <div
              class="heatmap__cell"
              style="opacity: {Math.max(opacity, 0.05)}"
              title="{day} {report.heatmap.hour_labels[hourIdx]}: {value}m"
            ></div>
          {/each}
        </div>
      {/each}
    </div>
  </section>

  <!-- Focus Windows -->
  <section class="rhythm__section">
    <h2 class="rhythm__heading">Best Focus Windows</h2>
    {#if report.focus_windows.length > 0}
      <div class="cards">
        {#each report.focus_windows as fw}
          <div class="card card--focus">
            <div class="card__label">{fw.label}</div>
            <div class="card__value">{formatDuration(fw.total_focus_ms)}</div>
          </div>
        {/each}
      </div>
    {:else}
      <p class="rhythm__empty">Not enough data yet. Keep using Companion Cube!</p>
    {/if}
  </section>

  <!-- Fingerprint -->
  <section class="rhythm__section">
    <h2 class="rhythm__heading">Focus Fingerprint</h2>
    {#if report.fingerprint.length > 0}
      <ul class="list">
        {#each report.fingerprint as cluster}
          <li class="list__item">
            <span class="list__bullet">●</span>
            <span class="list__text">{cluster.apps.join(' + ')}</span>
            <span class="list__count">{cluster.session_count} sessions</span>
          </li>
        {/each}
      </ul>
    {:else}
      <p class="rhythm__empty">Not enough data yet.</p>
    {/if}
  </section>

  <!-- Drift Origins -->
  <section class="rhythm__section">
    <h2 class="rhythm__heading">Drift Origins</h2>
    {#if report.drift_origins.length > 0}
      <ul class="list">
        {#each report.drift_origins as drift}
          <li class="list__item">
            <span class="list__bullet">●</span>
            <span class="list__text">{drift.app}</span>
            <span class="list__meta">← from {drift.from_app} ({drift.count})</span>
          </li>
        {/each}
      </ul>
    {:else}
      <p class="rhythm__empty">No drift detected. Nice focus!</p>
    {/if}
  </section>
</div>

<style>
  .rhythm {
    display: flex;
    flex-direction: column;
    gap: 28px;
    padding: 0 0 40px 0;
  }

  .rhythm__section {
    /* section spacing handled by parent gap */
  }

  .rhythm__heading {
    font-size: 16px;
    font-weight: 600;
    color: var(--ink);
    margin-bottom: 12px;
  }

  .rhythm__empty {
    color: var(--ink-faint);
    font-size: 13px;
  }

  /* Heatmap */
  .heatmap {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .heatmap__row {
    display: flex;
    gap: 2px;
    align-items: center;
  }

  .heatmap__header {
    margin-bottom: 2px;
  }

  .heatmap__day-label {
    width: 32px;
    font-size: 11px;
    color: var(--ink-faint);
    text-align: right;
    padding-right: 6px;
    flex-shrink: 0;
  }

  .heatmap__hour-label {
    width: 14px;
    height: 14px;
    font-size: 8px;
    color: var(--ink-faint);
    text-align: center;
    line-height: 14px;
    flex-shrink: 0;
  }

  .heatmap__cell {
    width: 14px;
    height: 14px;
    background: var(--brand-orange);
    border-radius: 3px;
    flex-shrink: 0;
    transition: transform 0.1s ease;
    cursor: default;
  }

  .heatmap__cell:hover {
    transform: scale(1.4);
    z-index: 1;
  }

  /* Focus Window Cards */
  .cards {
    display: flex;
    gap: 12px;
    flex-wrap: wrap;
  }

  .card--focus {
    background: var(--card-white);
    border: 1px solid var(--divider);
    border-radius: var(--r-panel);
    padding: 16px 20px;
    min-width: 120px;
  }

  .card--focus .card__label {
    font-size: 13px;
    color: var(--ink-soft);
    margin-bottom: 4px;
  }

  .card--focus .card__value {
    font-size: 20px;
    font-weight: 700;
    color: var(--brand-orange-deep);
  }

  /* List (fingerprint + drift) */
  .list {
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .list__item {
    display: flex;
    align-items: baseline;
    gap: 8px;
    font-size: 13px;
  }

  .list__bullet {
    color: var(--brand-orange);
    font-size: 10px;
    line-height: 1;
  }

  .list__text {
    color: var(--ink);
    font-weight: 500;
  }

  .list__count,
  .list__meta {
    color: var(--ink-faint);
    font-size: 12px;
  }
</style>
```

- [ ] **Step 2: Commit**

```bash
git add src/components/Rhythm.svelte
git commit -m "feat: add Rhythm.svelte component (heatmap, focus windows, fingerprint, drift)"
```

---

### Task 12: Wire Rhythm into Rail and layout

**Files:**
- Modify: `src/components/Rail.svelte`
- Modify: `src/routes/+layout.svelte`

- [ ] **Step 1: Add Rhythm tab to Rail**

In `src/components/Rail.svelte`:

1. Change the View type: `type View = 'history' | 'vault' | 'rhythm' | 'settings';`

2. Add a Rhythm button in `rail__top` after the Vault button (before the closing `</div>` of `rail__top`):

```svelte
<button
  class="rail__btn"
  class:active={$activeView === 'rhythm'}
  on:click={() => onViewChange('rhythm')}
  aria-label="Rhythm"
  title="Rhythm"
>
  <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
    <polyline points="22 12 18 12 15 21 9 3 6 12 2 12"/>
  </svg>
</button>
```

- [ ] **Step 2: Update `activeView` store type**

In `src/lib/stores.ts`, change the activeView type:

```typescript
export const activeView = writable<'history' | 'vault' | 'rhythm' | 'settings'>('history');
```

- [ ] **Step 3: Add Rhythm view to `+layout.svelte`**

In `src/routes/+layout.svelte`:

1. Add imports:
```typescript
import Rhythm from '$components/Rhythm.svelte';
import { rhythmReport, fetchRhythm } from '$lib/stores';
```

2. Add a Rhythm section between Vault and Settings in the template:

```svelte
{:else if $activeView === 'rhythm'}
  <!-- RHYTHM VIEW -->
  <h1 class="heading">Rhythm</h1>
  {#if !$daemonOnline}
    <p style="color:var(--ink-soft)">🔌 Daemon is not running.</p>
  {:else if $rhythmReport}
    <Rhythm report={$rhythmReport} />
  {:else}
    <p class="hint">Loading rhythm data...</p>
  {/if}
```

3. In the `onMount`, add rhythm fetching:
```typescript
fetchRhythm(7);
```

4. In the `refreshHistory` interval, also refresh rhythm when on that view:
```typescript
if ($activeView === 'rhythm') {
    fetchRhythm(7);
}
```

- [ ] **Step 4: Verify svelte-check**

Run: `npx svelte-check --tsconfig ./tsconfig.json`
Expected: 0 errors.

- [ ] **Step 5: Commit**

```bash
git add src/components/Rail.svelte src/lib/stores.ts src/routes/+layout.svelte
git commit -m "feat: wire Rhythm view into Rail and layout"
```

---

### Task 13: Final verification

- [ ] **Step 1: Run full Rust gate**

Run: `cargo check && cargo test`
Expected: All crates compile, all tests pass (136+ existing + 5 new rhythm tests).

- [ ] **Step 2: Run frontend type check**

Run: `npx svelte-check --tsconfig ./tsconfig.json`
Expected: 0 errors.

- [ ] **Step 3: Build frontend and daemon**

Run: `npm run build && cargo build -p ccube-daemon`
Expected: Both succeed.

- [ ] **Step 4: Final commit (if any remaining changes)**

```bash
git add -A
git commit -m "chore: final cleanup after Rhythm + browser pivot"
```
