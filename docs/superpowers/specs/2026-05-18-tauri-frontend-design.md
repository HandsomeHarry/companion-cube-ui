# Tauri 2 + Svelte 5 Frontend Design

**Date:** 2026-05-18  
**Scope:** Window shell, Vault, History (vertical slice B)  
**Deferred:** Floating Cube, Nudges, Rhythm modal, Aura, drag-to-regroup, vault search

---

## Context

ccube v0.2.1 has a working Rust backend: native macOS capture, daemon with HTTP API on `127.0.0.1:7431`, SQLite storage, CLI. The frontend is being redesigned from scratch following a Dieter Rams visual spec (warm paper aesthetic, serif headings, no scores, no judgment).

## Tech Stack

- **Tauri 2** — app shell, native window, future tray/menubar
- **Svelte 5** — UI framework (runes, derived stores)
- **TypeScript** — type-safe API layer
- No component library — custom CSS using design tokens from the visual spec

## Architecture

```
┌─────────────────────────────────────────────┐
│  Tauri Window                                │
│  ┌──────┬────────────────────────────────┐  │
│  │ Rail │  Content Area                  │  │
│  │      │  (Vault or History)            │  │
│  │ Hist │                                │  │
│  │ Vault│  Svelte components             │  │
│  │      │       │                        │  │
│  │      │       ▼                        │  │
│  │ ⚙️   │  api.ts ──fetch──▶ daemon      │  │
│  │      │              127.0.0.1:7431    │  │
│  └──────┴────────────────────────────────┘  │
└─────────────────────────────────────────────┘
```

All data flows through the daemon HTTP API. No direct SQLite access from the frontend.

## Daemon API Endpoints Used

| Endpoint | Method | Used by |
|----------|--------|---------|
| `/health` | GET | App startup — check daemon is running |
| `/activity/recent?limit=N` | GET | History timeline |
| `/vault` | GET | Vault table |
| `/vault` | POST | Save new vault entry |
| `/vault/:id` | DELETE | Remove vault entry |
| `/vault/:id/favorite` | PUT | Toggle favorite |

(Endpoints that don't exist yet will be stubbed in the frontend with mock data until daemon support is added.)

## File Structure

```
src-tauri/
  src/
    main.rs          # Tauri app builder, window config
    lib.rs           # Tauri commands (if needed for native bridges)
  Cargo.toml
  tauri.conf.json

src/
  app.html
  app.css            # Design tokens as CSS custom properties
  lib/
    api.ts           # fetch wrapper for daemon HTTP
    stores.ts        # Svelte 5 runes: vault store, history store, daemon status
  components/
    Rail.svelte           # Left icon rail (History, Vault, Settings)
    VaultTable.svelte     # Full vault table with column headers
    VaultRow.svelte       # Single vault row (idea, items, added, star, kebab)
    Timeline.svelte       # Full history timeline
    TimelineGroup.svelte  # One grouped session (time dot, title, items)
    TimelineItem.svelte   # Single activity row (app – item)
    EmptyState.svelte     # Shown when no data (daemon off, empty vault)
  routes/
    +layout.svelte        # Window shell: titlebar area + rail + content slot
    history/+page.svelte  # History view
    vault/+page.svelte    # Vault view
```

## Design Tokens (from visual spec)

```css
:root {
  --brand-orange: #F16A01;
  --brand-orange-hov: #D85F02;
  --brand-orange-deep: #C8480E;
  --gold-star: #E8A41E;

  --paper: #FEFCF4;
  --paper-panel: #FBF4E6;
  --card-white: #FFFFFF;
  --titlebar: #FDFDFD;

  --ink: #2A2622;
  --ink-soft: #6B6359;
  --ink-faint: #A39B8E;
  --on-orange: #FFFFFF;

  --divider: #ECE4D3;
  --row-hover: #FBEFE0;

  --r-cube-icon: 14px;
  --r-card: 16px;
  --r-panel: 12px;
  --r-control: 10px;
  --r-pill: 999px;

  --shadow-float: 0 12px 32px rgba(60,40,20,0.18);
  --shadow-window: 0 24px 64px rgba(30,20,10,0.28);
  --shadow-rest: 0 2px 8px rgba(60,40,20,0.08);

  --ease: cubic-bezier(0.22, 0.61, 0.36, 1);
  --t-fast: 140ms;
  --t-normal: 260ms;
  --t-calm: 420ms;

  --font-display: "Source Serif 4", "PT Serif", Charter, Georgia, serif;
  --font-ui: "Inter", -apple-system, "Segoe UI", system-ui, sans-serif;
}
```

## Component Details

### Rail (left icon nav)
- 56px wide, `--paper` background
- History icon (clock), Vault icon (box), top section
- Settings gear, bottom-pinned
- Active item: `--brand-orange` rounded square behind white glyph
- Click switches the content area view (no URL routing needed, Svelte state)

### Vault Table
- Header row: IDEA | ITEMS | ADDED (12px uppercase, `--ink-soft`, `--divider` underline)
- Rows: 44-48px tall, `--divider` separators
- Row content: optional gold star + idea name | items count | relative date
- Hover: `--row-hover` background, star + kebab fade in
- Kebab menu: Edit / Delete
- Toolbar: right-aligned search field (240px, `--r-control`) + orange "+" button (32x32)

### History Timeline
- Header strip: date navigator (← Today, Dec 27 →) + segmented Day|Week|Month control
- Vertical list of grouped sessions
- Each group: time gutter (64px, orange dot, `--ink-soft` time) + group title (16px bold) + pencil icon
- Items under each group: bullet + "App – item" text, drag handle on hover (deferred)
- Click item → open app/page (deferred to later phase)

### Empty State
- Shown when daemon is unreachable or no data
- Simple message + illustration placeholder
- "Start daemon" call-to-action if daemon is off

## Window Configuration

- Title: "Companion Cube", centered, 13px `--ink-soft`
- Size: 900×650 default, resizable, min 600×400
- No native title bar — custom titlebar with OS controls (Tauri `decorations: true` for v1 simplicity, can switch to custom later)
- Background: `--paper`

## State Management

Svelte 5 runes, no external state library:

```typescript
// stores.ts
let daemonOnline = $state(false);
let activeView = $state<'history' | 'vault'>('history');
let vaultItems = $state<VaultItem[]>([]);
let historyGroups = $state<HistoryGroup[]>([]);
```

API polling: check `/health` every 10s. Refetch vault/history when view switches or on a 30s interval.

## Future Additions (not in scope)

- **Drag-to-regroup in History** — `svelte-dnd-action`, drop emits correction event to daemon
- **Vault search** — client-side filter via Svelte derived store
- **Floating Cube** — Tauri transparent overlay window, always-on-top
- **Nudge cards** — Tauri notification window, 3s hold gesture
- **Rhythm modal** — overlay card over History
- **Aura** — Tauri commands calling HomeKit/Home Assistant APIs
