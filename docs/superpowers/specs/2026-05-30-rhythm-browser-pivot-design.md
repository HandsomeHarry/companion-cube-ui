# Rhythm Feature + Browser-Native Pivot — Design Spec

**Date:** 2026-05-30
**Scope:** Remove Tauri, make the daemon serve the UI directly via browser, add Rhythm (focus analytics) feature.

---

## 1. Architecture Change: Drop Tauri, Daemon Becomes the App

### Current State

```
Tauri app (native window) → WebView → HTTP → daemon at :7431
```

The Tauri wrapper provides one command: `start_daemon` (spawns the daemon binary). Everything else is the WebView making HTTP calls to the daemon.

### New State

```
Browser tab → HTTP → daemon at :7431 (serves UI + API)
```

The daemon becomes a single binary that:
- Captures activity (existing)
- Runs the LLM pipeline (existing)
- Serves the HTTP API (existing, prefixed under `/api/`)
- Serves the built SvelteKit frontend as static files at `/`
- Shows a system tray icon — clicking opens `http://localhost:7431` in the default browser

### What Gets Removed

| Removed | Reason |
|---|---|
| `src-tauri/` entire directory | Tauri is no longer needed |
| `@tauri-apps/api` npm dependency | No Tauri runtime to call |
| `@tauri-apps/cli` npm dependency | No Tauri build step |
| `window.__TAURI__` calls in `+layout.svelte` | Replaced with standard browser APIs |
| `invoke('start_daemon')` button in Settings | Daemon is already running if you see the page |

### What Stays Unchanged

| Kept | Reason |
|---|---|
| All 4 Rust crates in `crates/` | Core logic unchanged |
| `crates/ccube-capture/` platform capture | Independent of UI layer |
| `crates/ccube-daemon/` HTTP API handlers | Gets `/api/` prefix + static serving + tray |
| `crates/ccube-core/` DB, LLM, agents | Untouched |
| `crates/ccube-cli/` CLI commands | Still works for power users |
| Frontend SvelteKit app | Same code, different hosting |

### New Dependencies

**Rust (`ccube-daemon/Cargo.toml`):**
- `tray-icon` — system tray icon (macOS NSStatusItem, Windows Shell_NotifyIcon, Linux AppIndicator)
- `muda` — tray menu items (re-exported by `tray-icon`)
- `tower-http` — `ServeDir` for static file serving
- `include_dir` — embed frontend build output into the daemon binary at compile time

**npm:**
- Remove: `@tauri-apps/api`, `@tauri-apps/cli`
- Add: nothing

### Frontend Changes

- `api.ts`: `BASE` changes from `http://127.0.0.1:7431` to `/api` (relative, same-origin)
- `openUrl()`: `window.open(url, '_blank')` instead of `tauri.shell.open`
- Remove `window.__TAURI__` detection and `invoke('start_daemon')`
- Add theme switching: CSS custom properties + `[data-theme]` attribute on `<html>` + toggle button. Light/dark/auto. Default follows `prefers-color-scheme`.

### API Route Prefix

Existing endpoints move under `/api/` to avoid colliding with frontend routes:

| Old | New |
|---|---|
| `GET /health` | `GET /api/health` |
| `GET /activity` | `GET /api/activity` |
| `GET /config/llm` | `GET /api/config/llm` |
| `PUT /config/llm` | `PUT /api/config/llm` |
| `POST /summarize` | `POST /api/summarize` |
| `GET /summaries` | `GET /api/summaries` |
| `POST /corrections/group` | `POST /api/corrections/group` |
| `POST /detect` | `POST /api/detect` |
| ... | ... |
| *(new)* | `GET /api/rhythm?days=7` |
| *(frontend)* | `GET /*` → serve static files |

### Daemon `main()` Restructuring

macOS requires the tray event loop to run on the main thread. Current code uses `#[tokio::main]` on main. New structure:

```rust
fn main() -> Result<()> {
    // 1. Setup logging, paths, DB, session fence (sync, main thread)
    // 2. Spawn tokio runtime on a background thread:
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let cancel = CancellationToken::new();
    rt.spawn(run_server(state, cancel.clone())); // HTTP + capture loop
    // 3. On main thread: create tray icon, run event loop (blocks)
    run_tray(cancel);
}
```

`run_tray()` creates the tray icon with a menu:
- **Open Dashboard** → opens `http://localhost:7431` via `open` command (macOS) / `xdg-open` (Linux) / `start` (Windows)
- **Quit** → triggers `CancellationToken`, tokio tasks shut down gracefully


### Build & Install Flow

Two-step build (frontend first, then daemon):

```bash
# 1. Build frontend
npm run build          # → build/ directory

# 2. Build daemon (links the built frontend into its resource dir)
cargo build --release -p ccube-daemon
```

The daemon uses a `build.rs` script that:
1. Checks if `../build/` exists (the frontend build output)
2. If yes, copies it into `OUT_DIR/ui/`
3. The daemon binary includes the UI files via `include_dir!` or the Rust `include_dir` crate

At runtime, the daemon serves static files from the embedded UI directory.

If the UI files are missing (e.g., building daemon without frontend), the daemon logs a warning and serves API-only mode.

### Error Handling

- **Port conflict**: Daemon logs error, tray shows "Port in use" tooltip. Exit with clear message.
- **UI files missing**: Daemon serves API only, tray still works. Logs a warning.
- **Browser fails to open**: `open`/`xdg-open` failure is logged, not fatal. User can navigate manually.

---

## 2. Rhythm Feature — Focus Analytics

### Overview

Rhythm provides four analytics views of your activity data, computed from the existing `events` table. No new data collection, no LLM calls — pure aggregation over the last 7 days.

### Backend: `GET /api/rhythm?days=7`

**New module**: `ccube-core/src/rhythm.rs`

```rust
pub struct RhythmReport {
    pub focus_windows: Vec<FocusWindow>,
    pub fingerprint: Vec<AppCluster>,
    pub drift_origins: Vec<DriftOrigin>,
    pub heatmap: HeatmapData,
}

pub struct FocusWindow {
    pub hour_start: u8,        // 0–23
    pub hour_end: u8,          // 0–23
    pub total_focus_ms: i64,
    pub label: String,         // "9–11 AM"
}

pub struct AppCluster {
    pub apps: Vec<String>,     // ["VS Code", "Terminal"]
    pub session_count: u32,
}

pub struct DriftOrigin {
    pub app: String,           // "YouTube"
    pub from_app: String,      // "VS Code"
    pub count: u32,
}

pub struct HeatmapData {
    pub cells: Vec<u32>,       // 168 values (24h × 7d), activity duration in minutes
    pub max_value: u32,        // for normalization
    pub day_labels: Vec<String>,  // ["Mon", "Tue", ...]
    pub hour_labels: Vec<String>, // ["12AM", "1AM", ...]
}
```

**Core function signature:**

```rust
pub fn compute_rhythm(events: &[EventRow]) -> RhythmReport
```

Pure function. Takes a slice of events from the DB, returns the computed report. Fully testable with fixture data.

### Computation Algorithms

| Feature | Algorithm |
|---|---|
| **Best Focus Window** | Bin all focus-mode events into 1-hour buckets by `ts` hour. Sum `duration_ms` per bucket. Find contiguous ranges of 2+ hours with highest combined focus duration. Return top 3. Label with hour range (e.g., "9–11 AM"). |
| **Focus Fingerprint** | For focus-mode events, group consecutive same-app events into sessions (gap < 5min = same session). Within each 2-hour window, collect the set of distinct apps. Count co-occurrence of app pairs/triples across windows. Return top 5 combinations with session counts. |
| **Drift Origins** | Sort events by `ts`. For each event, check if the previous event (within 5 minutes) was in a focus mode and the current event is not. Record the transition as `(from_app → to_app)`. Aggregate by `to_app`, return top 5 with the most common `from_app` for each. |
| **Heatmap** | Create a 24×7 grid (hour-of-day × day-of-week). For each event, add its `duration_ms` to the corresponding cell. Convert to minutes. Return as flat array row-major (Mon 12AM, Mon 1AM, ..., Sun 11PM). Include `max_value` for frontend normalization. |

### HTTP Handler

```rust
async fn get_rhythm(
    State(state): State<Arc<AppState>>,
    Query(params): Query<RhythmQuery>,
) -> Result<Json<RhythmReport>, ApiError>
```

- Fetches events from DB: `SELECT * FROM events WHERE ts > (now - days*24h)`
- Calls `compute_rhythm(&events)`
- Returns JSON

No caching needed — the query is fast (~1ms on a week of events). If it becomes slow later, add an in-memory cache with TTL.

### Frontend: Rhythm View

**New component**: `src/components/Rhythm.svelte`
**New store**: `rhythmReport` writable in `stores.ts`
**New API method**: `api.rhythm(days?: number)`
**Rail update**: Add "Rhythm" tab (History → Vault → Rhythm → Settings)

**Layout** — single scrollable page:

```
┌─────────────────────────────────────┐
│ Rhythm              [Light/Dark ◐]  │
│                                     │
│ Weekly Activity                     │
│ ┌─────────────────────────────────┐ │
│ │ Heatmap (24×7 grid, CSS-only)  │ │
│ │ Hover: "Tue 3PM: 47m active"   │ │
│ └─────────────────────────────────┘ │
│                                     │
│ Best Focus Windows                  │
│ ┌──────────┐ ┌──────────┐          │
│ │ 9–11 AM  │ │ 2–4 PM   │          │
│ │ 2h 15m   │ │ 1h 40m   │          │
│ └──────────┘ └──────────┘          │
│                                     │
│ Focus Fingerprint                   │
│ ● VS Code + Terminal    14 sessions│
│ ● Figma + Safari         8 sessions│
│ ● Docs + Chrome          5 sessions│
│                                     │
│ Drift Origins                       │
│ ● YouTube       ← from VS Code (12)│
│ ● Twitter/X     ← from Terminal (8)│
│ ● Reddit        ← from Chrome (6)  │
└─────────────────────────────────────┘
```

**Heatmap implementation:**
- CSS grid: 7 columns (Mon–Sun) × 24 rows (hours), or transposed (24 columns × 7 rows) depending on what looks better on screen
- Each cell: `<div>` with `background-color: var(--brand-orange)` and `opacity` scaled to `cell_value / max_value`
- Tooltip: `title` attribute on each cell, e.g., "Tue 3PM: 47m"
- No canvas, no charting library, pure CSS

**Theme switching:**
- Toggle button in the header or Rail
- Sets `data-theme="light"` or `data-theme="dark"` on `<html>`
- CSS variables in `app.css` define two sets under `[data-theme="dark"]` and the default (light)
- `prefers-color-scheme` media query sets initial theme
- Store preference in `localStorage`

### Data Flow

```
Browser                     Daemon
  │                           │
  │ GET /api/rhythm?days=7   │
  │ ────────────────────────>│
  │                          │ SELECT * FROM events
  │                          │ WHERE ts > (now - 7d)
  │                          │
  │                          │ compute_rhythm(events)
  │                          │
  │   { focus_windows: [...],│
  │     fingerprint: [...],  │
  │     drift_origins: [...],│
  │     heatmap: {...} }     │
  │ <────────────────────────│
  │                           │
  │ Render Rhythm.svelte      │
```

---

## 3. Implementation Phases

### Phase A: Remove Tauri, Restructure Daemon

1. Remove `src-tauri/`, remove `@tauri-apps/*` npm deps
2. Add `/api/` prefix to all existing HTTP routes
3. Add static file serving for frontend at `/`
4. Restructure daemon `main()`: tokio on background thread, tray on main thread
5. Update frontend `api.ts` to use relative `/api` base
6. Remove `window.__TAURI__` usage from `+layout.svelte`
7. Add `build.rs` to copy frontend into daemon's resource dir
8. Add theme switching (CSS variables + toggle)
9. Verify everything works end-to-end: `cargo build` → `npm run build` → open browser

### Phase B: Rhythm Backend

1. Create `ccube-core/src/rhythm.rs` with types and `compute_rhythm()` function
2. Add `GET /api/rhythm?days=7` endpoint to daemon HTTP router
3. Write unit tests for each computation algorithm with fixture data

### Phase C: Rhythm Frontend

1. Add `api.rhythm()` method and `rhythmReport` store
2. Create `Rhythm.svelte` component (heatmap + stats)
3. Add "Rhythm" tab to `Rail.svelte`
4. Integrate into `+layout.svelte` routing
