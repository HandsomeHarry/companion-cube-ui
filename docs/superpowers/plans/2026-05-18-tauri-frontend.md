# Tauri 2 + Svelte 5 Frontend Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a Tauri 2 + Svelte 5 desktop app with window shell, Vault view, and History view that talks to the ccube daemon HTTP API.

**Architecture:** Single Tauri window with a left icon rail (History, Vault, Settings) and a content area. Svelte 5 runes for state. All data via fetch to `127.0.0.1:7431`. CSS design tokens from the visual spec.

**Tech Stack:** Tauri 2, Svelte 5 (with TypeScript), SvelteKit (Tauri template default). No component library.

**Spec:** `docs/superpowers/specs/2026-05-18-tauri-frontend-design.md`

---

## File Map

### New files (frontend)

| File | Purpose |
|------|---------|
| `src/app.css` | Design tokens as CSS custom properties |
| `src/app.html` | SvelteKit HTML shell |
| `src/lib/api.ts` | fetch wrapper for daemon HTTP API |
| `src/lib/stores.ts` | Svelte 5 runes: daemon status, active view, vault items, history events |
| `src/lib/types.ts` | TypeScript interfaces matching daemon JSON shapes |
| `src/components/Rail.svelte` | Left icon navigation rail |
| `src/components/VaultTable.svelte` | Vault table with headers |
| `src/components/VaultRow.svelte` | Single vault row |
| `src/components/Timeline.svelte` | History timeline container |
| `src/components/TimelineGroup.svelte` | One grouped session |
| `src/components/TimelineItem.svelte` | Single activity row |
| `src/components/EmptyState.svelte` | Daemon-off or empty-data state |
| `src/routes/+layout.svelte` | Window shell: rail + content slot |
| `src/routes/history/+page.svelte` | History view page |
| `src/routes/vault/+page.svelte` | Vault view page |

### New files (Tauri)

| File | Purpose |
|------|---------|
| `src-tauri/tauri.conf.json` | Window config, app metadata |
| `src-tauri/src/main.rs` | Tauri app entry point |
| `src-tauri/src/lib.rs` | Tauri commands/invokes (if needed) |
| `src-tauri/Cargo.toml` | Tauri dependencies |
| `package.json` | npm deps (svelte, sveltekit, tauri-api) |
| `svelte.config.js` | SvelteKit config for Tauri |
| `vite.config.ts` | Vite config for Tauri |
| `tsconfig.json` | TypeScript config |

### Existing files modified

None. This is a new frontend alongside the existing Rust crates.

---

## Task 1: Scaffold Tauri 2 + Svelte 5 + TypeScript project

**Files:**
- Create: `package.json`, `src-tauri/Cargo.toml`, `src-tauri/tauri.conf.json`, `src-tauri/src/main.rs`, `src-tauri/src/lib.rs`, `svelte.config.js`, `vite.config.ts`, `tsconfig.json`, `src/app.html`, `src/app.css`, `src/routes/+layout.svelte`, `src/routes/+page.svelte`

- [ ] **Step 1: Scaffold with create-tauri-app**

```bash
cd /Users/harryyu/Desktop
npx create-tauri-app@latest ccube-ui-temp --template svelte-ts --manager npm
```

Then copy the generated files into the existing project:

```bash
cd /Users/harryyu/Desktop
cp -r ccube-ui-temp/src-tauri/ ccube-ui/src-tauri/
cp -r ccube-ui-temp/src/ ccube-ui/src/
cp ccube-ui-temp/package.json ccube-ui/
cp ccube-ui-temp/svelte.config.js ccube-ui/
cp ccube-ui-temp/vite.config.ts ccube-ui/
cp ccube-ui-temp/tsconfig.json ccube-ui/
rm -rf ccube-ui-temp
```

- [ ] **Step 2: Update tauri.conf.json for ccube branding**

In `src-tauri/tauri.conf.json`, set:
- `identifier` to `"com.ccube.app"`
- `productName` to `"Companion Cube"`
- `title` to `"Companion Cube"`
- window `width` to `900`, `height` to `650`, `minWidth` to `600`, `minHeight` to `400`
- window `decorations` to `true`

- [ ] **Step 3: Install dependencies**

```bash
cd /Users/harryyu/Desktop/ccube-ui
npm install
```

- [ ] **Step 4: Verify dev server starts**

```bash
cd /Users/harryyu/Desktop/ccube-ui
npx tauri dev
```

Expected: A native window opens with the default SvelteKit welcome page. Kill it after confirming (Ctrl+C).

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: scaffold Tauri 2 + Svelte 5 + TypeScript project"
```

Wait for user approval before pushing.

---

## Task 2: Design tokens + global styles

**Files:**
- Create: `src/app.css` (replace default)
- Modify: `src/routes/+layout.svelte` (import app.css, set paper background)

- [ ] **Step 1: Write the design tokens CSS**

Replace `src/app.css` with:

```css
@import url('https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600;700&family=Source+Serif+4:wght@700&display=swap');

:root {
  /* Brand */
  --brand-orange: #F16A01;
  --brand-orange-hov: #D85F02;
  --brand-orange-deep: #C8480E;
  --gold-star: #E8A41E;

  /* Surfaces */
  --paper: #FEFCF4;
  --paper-panel: #FBF4E6;
  --card-white: #FFFFFF;
  --titlebar: #FDFDFD;

  /* Text */
  --ink: #2A2622;
  --ink-soft: #6B6359;
  --ink-faint: #A39B8E;
  --on-orange: #FFFFFF;

  /* Lines */
  --divider: #ECE4D3;
  --row-hover: #FBEFE0;

  /* Radii */
  --r-card: 16px;
  --r-panel: 12px;
  --r-control: 10px;
  --r-pill: 999px;

  /* Elevation */
  --shadow-float: 0 12px 32px rgba(60, 40, 20, 0.18);
  --shadow-window: 0 24px 64px rgba(30, 20, 10, 0.28);
  --shadow-rest: 0 2px 8px rgba(60, 40, 20, 0.08);

  /* Motion */
  --ease: cubic-bezier(0.22, 0.61, 0.36, 1);
  --t-fast: 140ms;
  --t-normal: 260ms;
  --t-calm: 420ms;

  /* Typography */
  --font-display: "Source Serif 4", "PT Serif", Charter, Georgia, serif;
  --font-ui: "Inter", -apple-system, "Segoe UI", system-ui, sans-serif;
}

* {
  margin: 0;
  padding: 0;
  box-sizing: border-box;
}

html, body {
  font-family: var(--font-ui);
  font-size: 14px;
  line-height: 1.5;
  color: var(--ink);
  background: var(--paper);
  -webkit-font-smoothing: antialiased;
}

button {
  font-family: var(--font-ui);
  cursor: pointer;
  border: none;
  background: none;
}

button:focus-visible {
  outline: 2px solid var(--brand-orange);
  outline-offset: 2px;
}
```

- [ ] **Step 2: Write the root layout**

Replace `src/routes/+layout.svelte` with:

```svelte
<script lang="ts">
  import '../app.css';
  let { children } = $props();
</script>

{@render children()}
```

- [ ] **Step 3: Verify styles apply**

```bash
npx tauri dev
```

Expected: Window opens with warm cream `#FEFCF4` background, Inter font. Kill after confirming.

- [ ] **Step 4: Commit**

```bash
git add src/app.css src/routes/+layout.svelte
git commit -m "feat: add design tokens and global styles"
```

---

## Task 3: TypeScript types + API layer

**Files:**
- Create: `src/lib/types.ts`
- Create: `src/lib/api.ts`

- [ ] **Step 1: Create types matching daemon JSON**

`src/lib/types.ts`:

```typescript
export interface EventRow {
  id: number;
  ts: number;
  kind: string;
  app: string | null;
  title: string | null;
  duration_ms: number | null;
  mode: string | null;
  ocr_text: string | null;
}

export interface DecisionRow {
  id: number;
  ts: number;
  trigger: string;
  decision: string;
  reasoning: string;
  nudge_style: string | null;
  nudge_message: string | null;
}

export interface CorrectionRow {
  id: number;
  ts: number;
  decision_id: number;
  original_decision: string;
  user_verdict: string;
  ctx_snapshot: string;
  patterns_hash: string;
  status: string;
}

export interface VaultItem {
  id: number;
  ts: number;
  idea: string;
  items: string;
  favorited: boolean;
}
```

- [ ] **Step 2: Create API fetch wrapper**

`src/lib/api.ts`:

```typescript
import type { EventRow } from './types';

const BASE = 'http://127.0.0.1:7431';

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(`${BASE}${path}`, {
    headers: { 'Accept': 'application/json' },
    ...init,
  });
  if (!res.ok) {
    throw new Error(`daemon ${res.status}: ${res.statusText}`);
  }
  return res.json();
}

export const api = {
  health: () =>
    request<{ status: string; uptime_s: number; daemon_version: string }>('/health'),

  activity: (hours?: number) =>
    request<EventRow[]>(`/activity${hours ? `?hours=${hours}` : ''}`),

  recent: (limit = 50) =>
    request<EventRow[]>(`/activity?hours=${24}`),
};
```

- [ ] **Step 3: Verify TypeScript compiles**

```bash
npx svelte-check --tsconfig ./tsconfig.json
```

Expected: No type errors (warnings OK).

- [ ] **Step 4: Commit**

```bash
git add src/lib/types.ts src/lib/api.ts
git commit -m "feat: add TypeScript types and daemon API layer"
```

---

## Task 4: Svelte stores (state management)

**Files:**
- Create: `src/lib/stores.ts`

- [ ] **Step 1: Create stores using Svelte 5 runes**

`src/lib/stores.ts`:

```typescript
import type { EventRow, VaultItem } from './types';
import { api } from './api';

// Active view: 'history' or 'vault'
export let activeView = $state<'history' | 'vault'>('history');

// Daemon online status
export let daemonOnline = $state(false);

// Data
export let historyEvents = $state<EventRow[]>([]);
export let vaultItems = $state<VaultItem[]>([]);
export let loading = $state(false);
export let error = $state<string | null>(null);

// Check daemon health
export async function checkDaemon() {
  try {
    await api.health();
    daemonOnline = true;
  } catch {
    daemonOnline = false;
  }
}

// Fetch history events
export async function fetchHistory() {
  loading = true;
  error = null;
  try {
    historyEvents = await api.recent();
    daemonOnline = true;
  } catch (e) {
    error = e instanceof Error ? e.message : 'Failed to fetch history';
    daemonOnline = false;
  } finally {
    loading = false;
  }
}

// Fetch vault items (stubbed until daemon has vault endpoints)
export async function fetchVault() {
  loading = true;
  error = null;
  try {
    // TODO: replace with api.vault() when daemon endpoint exists
    vaultItems = [];
    daemonOnline = true;
  } catch (e) {
    error = e instanceof Error ? e.message : 'Failed to fetch vault';
    daemonOnline = false;
  } finally {
    loading = false;
  }
}

// Start periodic health check (every 10s)
export function startHealthPolling() {
  checkDaemon();
  return setInterval(checkDaemon, 10_000);
}
```

- [ ] **Step 2: Verify TypeScript compiles**

```bash
npx svelte-check --tsconfig ./tsconfig.json
```

- [ ] **Step 3: Commit**

```bash
git add src/lib/stores.ts
git commit -m "feat: add Svelte 5 state stores and daemon polling"
```

---

## Task 5: Rail component (left icon navigation)

**Files:**
- Create: `src/components/Rail.svelte`
- Modify: `src/routes/+layout.svelte`

- [ ] **Step 1: Build the Rail component**

`src/components/Rail.svelte`:

```svelte
<script lang="ts">
  import { activeView } from '$lib/stores';

  type View = 'history' | 'vault';
  let { onViewChange }: { onViewChange: (view: View) => void } = $props();
</script>

<nav class="rail">
  <div class="rail__top">
    <button
      class="rail__btn"
      class:active={$activeView === 'history'}
      onclick={() => onViewChange('history')}
      aria-label="History"
      title="History"
    >
      <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <circle cx="12" cy="12" r="10"/>
        <polyline points="12 6 12 12 16 14"/>
      </svg>
    </button>

    <button
      class="rail__btn"
      class:active={$activeView === 'vault'}
      onclick={() => onViewChange('vault')}
      aria-label="Vault"
      title="Vault"
    >
      <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <path d="M21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16z"/>
        <polyline points="3.27 6.96 12 12.01 20.73 6.96"/>
        <line x1="12" y1="22.08" x2="12" y2="12"/>
      </svg>
    </button>
  </div>

  <div class="rail__bottom">
    <button class="rail__btn" aria-label="Settings" title="Settings">
      <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <circle cx="12" cy="12" r="3"/>
        <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z"/>
      </svg>
    </button>
  </div>
</nav>

<style>
  .rail {
    width: 56px;
    min-height: 100%;
    background: var(--paper);
    display: flex;
    flex-direction: column;
    align-items: center;
    padding: 16px 0;
    border-right: 1px solid var(--divider);
  }

  .rail__top {
    display: flex;
    flex-direction: column;
    gap: 14px;
  }

  .rail__bottom {
    margin-top: auto;
  }

  .rail__btn {
    width: 36px;
    height: 36px;
    border-radius: var(--r-panel);
    display: grid;
    place-items: center;
    color: var(--ink-soft);
    transition: all var(--t-normal) var(--ease);
  }

  .rail__btn:hover {
    background: var(--row-hover);
  }

  .rail__btn.active {
    background: var(--brand-orange);
    color: var(--on-orange);
  }
</style>
```

- [ ] **Step 2: Update root layout with Rail + content**

`src/routes/+layout.svelte`:

```svelte
<script lang="ts">
  import '../app.css';
  import Rail from '$components/Rail.svelte';
  import { activeView, startHealthPolling, fetchHistory, fetchVault } from '$lib/stores';

  let { children } = $props();

  function handleViewChange(view: 'history' | 'vault') {
    $activeView = view;
    if (view === 'history') fetchHistory();
    if (view === 'vault') fetchVault();
  }

  // Start polling on mount
  const interval = startHealthPolling();

  // Fetch initial data
  fetchHistory();

  // Cleanup on unmount
  $effect(() => {
    return () => clearInterval(interval);
  });
</script>

<div class="app">
  <Rail onViewChange={handleViewChange} />
  <main class="content">
    {@render children()}
  </main>
</div>

<style>
  .app {
    display: flex;
    height: 100vh;
    overflow: hidden;
  }

  .content {
    flex: 1;
    padding: 30px;
    overflow-y: auto;
    background: var(--paper);
  }
</style>
```

- [ ] **Step 3: Verify Rail renders**

```bash
npx tauri dev
```

Expected: Window with left icon rail (clock, box, gear). Active item has orange background. Clicking switches orange highlight. Kill after confirming.

- [ ] **Step 4: Commit**

```bash
git add src/components/Rail.svelte src/routes/+layout.svelte
git commit -m "feat: add Rail navigation and app shell layout"
```

---

## Task 6: EmptyState component

**Files:**
- Create: `src/components/EmptyState.svelte`

- [ ] **Step 1: Build the EmptyState component**

`src/components/EmptyState.svelte`:

```svelte
<script lang="ts">
  let { message, icon = '📭' }: { message: string; icon?: string } = $props();
</script>

<div class="empty">
  <span class="empty__icon">{icon}</span>
  <p class="empty__msg">{message}</p>
</div>

<style>
  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    padding: 80px 20px;
    text-align: center;
  }

  .empty__icon {
    font-size: 48px;
    margin-bottom: 16px;
  }

  .empty__msg {
    font-size: 15px;
    color: var(--ink-soft);
    max-width: 320px;
    line-height: 1.6;
  }
</style>
```

- [ ] **Step 2: Commit**

```bash
git add src/components/EmptyState.svelte
git commit -m "feat: add EmptyState component"
```

---

## Task 7: History view (Timeline + TimelineGroup + TimelineItem)

**Files:**
- Create: `src/components/Timeline.svelte`
- Create: `src/components/TimelineGroup.svelte`
- Create: `src/components/TimelineItem.svelte`
- Create: `src/routes/history/+page.svelte`

- [ ] **Step 1: Build TimelineItem**

`src/components/TimelineItem.svelte`:

```svelte
<script lang="ts">
  import type { EventRow } from '$lib/types';

  let { event }: { event: EventRow } = $props();

  const app = $derived(event.app ?? 'Unknown');
  const title = $derived(event.title ?? 'No title');
</script>

<div class="item">
  <span class="item__dot">·</span>
  <span class="item__text">{app} – {title}</span>
  <span class="item__handle">≡</span>
</div>

<style>
  .item {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 8px;
    border-radius: 8px;
    cursor: pointer;
    transition: background var(--t-fast) var(--ease);
  }

  .item:hover {
    background: var(--row-hover);
  }

  .item__dot {
    color: var(--ink-faint);
    flex-shrink: 0;
  }

  .item__text {
    flex: 1;
    font-size: 14px;
    color: var(--ink);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .item__handle {
    color: var(--ink-faint);
    opacity: 0;
    cursor: grab;
    transition: opacity var(--t-fast) var(--ease);
    flex-shrink: 0;
  }

  .item:hover .item__handle {
    opacity: 1;
  }
</style>
```

- [ ] **Step 2: Build TimelineGroup**

`src/components/TimelineGroup.svelte`:

```svelte
<script lang="ts">
  import type { EventRow } from '$lib/types';
  import TimelineItem from './TimelineItem.svelte';

  let { time, title, events }: { time: string; title: string; events: EventRow[] } = $props();
</script>

<div class="group">
  <div class="group__time">
    <span class="group__dot"></span>
    {time}
  </div>
  <div class="group__body">
    <div class="group__title">
      {title}
      <button class="group__edit" aria-label="Rename group" title="Rename">
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M17 3a2.828 2.828 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5L17 3z"/>
        </svg>
      </button>
    </div>
    <div class="group__items">
      {#each events as event (event.id)}
        <TimelineItem {event} />
      {/each}
    </div>
  </div>
</div>

<style>
  .group {
    display: grid;
    grid-template-columns: 64px 1fr;
    margin-bottom: 22px;
  }

  .group__time {
    position: relative;
    font-size: 13px;
    color: var(--ink-soft);
    padding-right: 12px;
    text-align: right;
    padding-top: 2px;
  }

  .group__dot {
    position: absolute;
    right: -4px;
    top: 5px;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--brand-orange);
  }

  .group__body {
    min-width: 0;
  }

  .group__title {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 16px;
    font-weight: 700;
    color: var(--ink);
    margin-bottom: 4px;
  }

  .group__edit {
    color: var(--ink-faint);
    display: grid;
    place-items: center;
    padding: 2px;
    border-radius: 4px;
    opacity: 0;
    transition: opacity var(--t-fast) var(--ease);
  }

  .group__title:hover .group__edit {
    opacity: 1;
  }

  .group__items {
    display: flex;
    flex-direction: column;
  }
</style>
```

- [ ] **Step 3: Build Timeline**

`src/components/Timeline.svelte`:

```svelte
<script lang="ts">
  import type { EventRow } from '$lib/types';
  import TimelineGroup from './TimelineGroup.svelte';

  let { events }: { events: EventRow[] } = $props();

  // Group consecutive events by app (naive grouping)
  const groups = $derived(() => {
    if (events.length === 0) return [];

    const result: { time: string; title: string; events: EventRow[] }[] = [];
    let currentGroup: { time: string; title: string; events: EventRow[] } | null = null;

    for (const event of events) {
      const time = new Date(event.ts).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
      const app = event.app ?? 'Unknown';

      if (!currentGroup || currentGroup.title !== app) {
        currentGroup = { time, title: app, events: [event] };
        result.push(currentGroup);
      } else {
        currentGroup.events.push(event);
      }
    }

    return result;
  });
</script>

<div class="timeline">
  {#each $derived(groups()) as group (group.time + group.title)}
    <TimelineGroup time={group.time} title={group.title} events={group.events} />
  {/each}
</div>

<style>
  .timeline {
    padding-top: 8px;
  }
</style>
```

- [ ] **Step 4: Build History page**

`src/routes/history/+page.svelte`:

```svelte
<script lang="ts">
  import { historyEvents, loading, daemonOnline } from '$lib/stores';
  import Timeline from '$components/Timeline.svelte';
  import EmptyState from '$components/EmptyState.svelte';
</script>

<svelte:head>
  <title>History – Companion Cube</title>
</svelte:head>

<h1 class="heading">History</h1>

<div class="bar">
  <div class="datenav">
    <button class="datenav__btn" aria-label="Previous day">←</button>
    <span class="datenav__label">Today, {new Date().toLocaleDateString(undefined, { month: 'short', day: 'numeric' })}</span>
    <button class="datenav__btn" aria-label="Next day">→</button>
  </div>
  <div class="seg">
    <button class="seg__opt active">Day</button>
    <button class="seg__opt">Week</button>
    <button class="seg__opt">Month</button>
  </div>
</div>

{#if !$daemonOnline}
  <EmptyState message="Daemon is not running. Start it with: ccube daemon start" icon="🔌" />
{:else if $loading}
  <EmptyState message="Loading..." icon="⏳" />
{:else if $historyEvents.length === 0}
  <EmptyState message="No activity recorded yet. Start capturing with: ccube daemon capture" icon="📭" />
{:else}
  <Timeline events={$historyEvents} />
{/if}

<style>
  .heading {
    font-family: var(--font-display);
    font-size: 32px;
    font-weight: 700;
    color: var(--brand-orange-deep);
    margin-bottom: 8px;
  }

  .bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 16px;
  }

  .datenav {
    display: flex;
    align-items: center;
    gap: 10px;
    font-size: 14px;
  }

  .datenav__btn {
    width: 28px;
    height: 28px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    font-size: 14px;
    color: var(--ink-soft);
    transition: background var(--t-fast) var(--ease);
  }

  .datenav__btn:hover {
    background: var(--row-hover);
  }

  .datenav__label {
    color: var(--ink);
  }

  .seg {
    display: inline-flex;
    background: var(--paper-panel);
    border-radius: var(--r-pill);
    padding: 3px;
  }

  .seg__opt {
    padding: 6px 14px;
    border-radius: var(--r-pill);
    font-size: 13px;
    font-weight: 600;
    color: var(--ink-soft);
    transition: all var(--t-normal) var(--ease);
  }

  .seg__opt.active {
    background: var(--card-white);
    color: var(--ink);
    box-shadow: var(--shadow-rest);
  }

  .seg__opt:hover:not(.active) {
    color: var(--ink);
  }
</style>
```

- [ ] **Step 5: Update the index route to redirect to history**

Replace `src/routes/+page.svelte`:

```svelte
<script lang="ts">
  import { goto } from '$app/navigation';
  import { onMount } from 'svelte';

  onMount(() => {
    goto('/history');
  });
</script>
```

- [ ] **Step 6: Verify History renders**

```bash
npx tauri dev
```

Expected: Window opens showing History view. If daemon is running, shows timeline of events. If not, shows "Daemon is not running" empty state. Kill after confirming.

- [ ] **Step 7: Commit**

```bash
git add src/components/Timeline*.svelte src/routes/history/+page.svelte src/routes/+page.svelte
git commit -m "feat: add History view with timeline, groups, and items"
```

---

## Task 8: Vault view (VaultTable + VaultRow)

**Files:**
- Create: `src/components/VaultTable.svelte`
- Create: `src/components/VaultRow.svelte`
- Create: `src/routes/vault/+page.svelte`

- [ ] **Step 1: Build VaultRow**

`src/components/VaultRow.svelte`:

```svelte
<script lang="ts">
  import type { VaultItem } from '$lib/types';

  let { item }: { item: VaultItem } = $props();

  const relativeDate = $derived(() => {
    const now = new Date();
    const d = new Date(item.ts);
    const diffDays = Math.floor((now.getTime() - d.getTime()) / (1000 * 60 * 60 * 24));

    if (diffDays === 0) return `Today, ${d.toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' })}`;
    if (diffDays === 1) return 'Yesterday';
    return d.toLocaleDateString();
  });
</script>

<tr class="row">
  <td class="row__idea">
    {#if item.favorited}
      <span class="row__star filled">★</span>
    {/if}
    {item.idea}
  </td>
  <td class="row__items">{item.items}</td>
  <td class="row__added">{$derived(relativeDate())}</td>
  <td class="row__actions">
    <button
      class="row__star-btn"
      class:filled={item.favorited}
      aria-label={item.favorited ? 'Unfavorite' : 'Favorite'}
    >
      {item.favorited ? '★' : '☆'}
    </button>
    <button class="row__kebab" aria-label="More options">⋮</button>
  </td>
</tr>

<style>
  .row td {
    padding: 12px;
    border-bottom: 1px solid var(--divider);
    font-size: 14px;
    vertical-align: middle;
  }

  .row:hover td {
    background: var(--row-hover);
  }

  .row__idea {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--ink);
    font-weight: 400;
  }

  .row__star {
    color: var(--gold-star);
    font-size: 14px;
  }

  .row__items {
    color: var(--ink-soft);
    font-size: 13px;
  }

  .row__added {
    color: var(--ink-faint);
    font-size: 13px;
  }

  .row__actions {
    display: flex;
    align-items: center;
    gap: 4px;
    justify-content: flex-end;
  }

  .row__star-btn,
  .row__kebab {
    width: 28px;
    height: 28px;
    border-radius: 6px;
    display: grid;
    place-items: center;
    font-size: 14px;
    color: var(--ink-faint);
    opacity: 0;
    transition: all var(--t-fast) var(--ease);
  }

  .row:hover .row__star-btn,
  .row:hover .row__kebab,
  .row__star-btn.filled {
    opacity: 1;
  }

  .row__star-btn.filled {
    color: var(--gold-star);
  }

  .row__star-btn:hover,
  .row__kebab:hover {
    background: var(--paper-panel);
  }
</style>
```

- [ ] **Step 2: Build VaultTable**

`src/components/VaultTable.svelte`:

```svelte
<script lang="ts">
  import type { VaultItem } from '$lib/types';
  import VaultRow from './VaultRow.svelte';

  let { items }: { items: VaultItem[] } = $props();
</script>

<div class="toolbar">
  <div class="search">
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
      <circle cx="11" cy="11" r="8"/>
      <line x1="21" y1="21" x2="16.65" y2="16.65"/>
    </svg>
    <input type="text" placeholder="Search ideas..." />
  </div>
  <button class="add-btn" aria-label="Add idea">+</button>
</div>

<table class="vault-table">
  <thead>
    <tr>
      <th>Idea</th>
      <th>Items</th>
      <th>Added</th>
      <th></th>
    </tr>
  </thead>
  <tbody>
    {#each items as item (item.id)}
      <VaultRow {item} />
    {/each}
  </tbody>
</table>

<style>
  .toolbar {
    display: flex;
    justify-content: flex-end;
    gap: 10px;
    margin: 8px 0 18px;
  }

  .search {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 240px;
    background: var(--card-white);
    border: 1px solid var(--divider);
    border-radius: var(--r-control);
    padding: 7px 12px;
    transition: border-color var(--t-fast) var(--ease);
  }

  .search:focus-within {
    border-color: var(--brand-orange);
  }

  .search svg {
    color: var(--ink-faint);
    flex-shrink: 0;
  }

  .search input {
    border: none;
    outline: none;
    font-size: 13px;
    color: var(--ink);
    background: transparent;
    width: 100%;
    font-family: var(--font-ui);
  }

  .search input::placeholder {
    color: var(--ink-faint);
  }

  .add-btn {
    width: 32px;
    height: 32px;
    border-radius: var(--r-control);
    background: var(--brand-orange);
    color: var(--on-orange);
    font-size: 18px;
    display: grid;
    place-items: center;
    transition: background var(--t-fast) var(--ease);
  }

  .add-btn:hover {
    background: var(--brand-orange-hov);
  }

  .vault-table {
    width: 100%;
    border-collapse: collapse;
  }

  .vault-table th {
    font-size: 12px;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--ink-soft);
    text-align: left;
    padding: 8px 12px;
    border-bottom: 1px solid var(--divider);
  }

  .vault-table th:last-child {
    width: 80px;
  }
</style>
```

- [ ] **Step 3: Build Vault page**

`src/routes/vault/+page.svelte`:

```svelte
<script lang="ts">
  import { vaultItems, loading, daemonOnline } from '$lib/stores';
  import VaultTable from '$components/VaultTable.svelte';
  import EmptyState from '$components/EmptyState.svelte';
</script>

<svelte:head>
  <title>Vault – Companion Cube</title>
</svelte:head>

<h1 class="heading">Vault</h1>

{#if !$daemonOnline}
  <EmptyState message="Daemon is not running. Start it with: ccube daemon start" icon="🔌" />
{:else if $loading}
  <EmptyState message="Loading..." icon="⏳" />
{:else if $vaultItems.length === 0}
  <EmptyState message="Your vault is empty. Save distractions here when nudged, or add ideas manually." icon="🏦" />
{:else}
  <VaultTable items={$vaultItems} />
{/if}

<style>
  .heading {
    font-family: var(--font-display);
    font-size: 32px;
    font-weight: 700;
    color: var(--brand-orange-deep);
    margin-bottom: 8px;
  }
</style>
```

- [ ] **Step 4: Verify Vault renders**

```bash
npx tauri dev
```

Expected: Click Vault icon in rail → shows Vault page with "Your vault is empty" empty state. Table headers and search bar visible in layout. Kill after confirming.

- [ ] **Step 5: Commit**

```bash
git add src/components/Vault*.svelte src/routes/vault/+page.svelte
git commit -m "feat: add Vault view with table, search, and row components"
```

---

## Task 9: Wire navigation + auto-refresh

**Files:**
- Modify: `src/routes/+layout.svelte`
- Modify: `src/routes/history/+page.svelte`
- Modify: `src/routes/vault/+page.svelte`

- [ ] **Step 1: Add auto-refresh on view switch**

Update `src/routes/+layout.svelte` to re-fetch data when switching views:

```svelte
<script lang="ts">
  import '../app.css';
  import Rail from '$components/Rail.svelte';
  import { activeView, startHealthPolling, fetchHistory, fetchVault, daemonOnline } from '$lib/stores';

  let { children } = $props();

  function handleViewChange(view: 'history' | 'vault') {
    $activeView = view;
    if (view === 'history') fetchHistory();
    if (view === 'vault') fetchVault();
  }

  const interval = startHealthPolling();
  fetchHistory();

  // Auto-refresh current view every 30s
  const refreshInterval = setInterval(() => {
    if ($activeView === 'history') fetchHistory();
    else fetchVault();
  }, 30_000);

  $effect(() => {
    return () => {
      clearInterval(interval);
      clearInterval(refreshInterval);
    };
  });
</script>

<div class="app">
  <Rail onViewChange={handleViewChange} />
  <main class="content">
    {@render children()}
  </main>
</div>

<style>
  .app {
    display: flex;
    height: 100vh;
    overflow: hidden;
  }

  .content {
    flex: 1;
    padding: 30px;
    overflow-y: auto;
    background: var(--paper);
  }
</style>
```

- [ ] **Step 2: Verify full flow**

```bash
npx tauri dev
```

Expected:
1. Opens on History view (default)
2. Click Vault icon → switches to Vault view
3. Click History icon → switches back
4. If daemon running: events load in timeline
5. If daemon not running: empty state shown

Kill after confirming.

- [ ] **Step 3: Commit**

```bash
git add src/routes/+layout.svelte src/routes/history/+page.svelte src/routes/vault/+page.svelte
git commit -m "feat: wire view switching and auto-refresh"
```

---

## Task 10: Add SvelteKit path aliases

**Files:**
- Modify: `vite.config.ts` (or `svelte.config.js`)

- [ ] **Step 1: Configure `$components` alias**

Update `vite.config.ts` to add the `$components` alias:

```typescript
import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';

export default defineConfig({
  plugins: [sveltekit()],
  resolve: {
    alias: {
      $components: './src/components',
    },
  },
});
```

Also update `tsconfig.json` paths:

```json
{
  "compilerOptions": {
    "paths": {
      "$lib": ["./src/lib"],
      "$lib/*": ["./src/lib/*"],
      "$components": ["./src/components"],
      "$components/*": ["./src/components/*"]
    }
  }
}
```

- [ ] **Step 2: Verify all imports resolve**

```bash
npx svelte-check --tsconfig ./tsconfig.json
```

Expected: No unresolved import errors.

- [ ] **Step 3: Commit**

```bash
git add vite.config.ts tsconfig.json
git commit -m "feat: add $components path alias"
```

---

## Task 11: Final build + verify

**Files:** None (verification only)

- [ ] **Step 1: Run type check**

```bash
npx svelte-check --tsconfig ./tsconfig.json
```

Expected: No errors.

- [ ] **Step 2: Run production build**

```bash
npx tauri build
```

Expected: Produces a `.dmg` or `.app` in `src-tauri/target/release/bundle/`. Build succeeds without errors.

- [ ] **Step 3: Commit any final fixes**

```bash
git add -A
git commit -m "chore: final build fixes"
```

---

## Self-Review Checklist

**1. Spec coverage:**
- ✅ Design tokens (§1) → Task 2
- ✅ Window shell + rail (§5) → Tasks 5, 9
- ✅ Vault table + search (§6) → Task 8
- ✅ History timeline + groups (§7) → Task 7
- ✅ API layer → Task 3
- ✅ State management → Task 4
- ✅ Empty states → Task 6
- ⏳ Drag-to-regroup → deferred (spec approved)
- ⏳ Vault search functional → deferred (spec approved)

**2. Placeholder scan:** No TBDs or "implement later" in task steps. Vault fetch is stubbed with clear TODO comment.

**3. Type consistency:** `EventRow`, `VaultItem` types defined in Task 3, used consistently in Tasks 7, 8. `activeView` typed as `'history' | 'vault'` throughout.
