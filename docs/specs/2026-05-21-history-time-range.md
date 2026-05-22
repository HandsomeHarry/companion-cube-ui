# Spec: History Time Range — Day / Week / Month Navigation

## Problem
- Date label shows stale date (hardcoded `new Date()` at mount time, never updates)
- Day/Week/Month segmented control is non-functional (no click handlers)
- Arrow buttons (← →) don't navigate between periods
- Backend `/activity?hours=N` only supports hour-based lookback, not date-bounded queries

## Design Spec Reference (§7)
> Left: a date navigator — ← button, label "Today, Dec 27", → button
> Right: Day | Week | Month segmented control
> Day = timeline of one day's events
> Week = weekly aggregation
> Month = monthly aggregation

## Implementation Plan

### 1. Frontend State
```typescript
let viewMode: 'day' | 'week' | 'month' = 'day';
let currentDate: Date = new Date(); // always tracks "today"
let selectedDate: Date = new Date(); // the date being viewed
```

- `selectedDate` changes with ←/→ navigation
- `currentDate` always = today (used to show "Today" vs date)
- `viewMode` switches between day/week/month

### 2. Date Label Logic
```
if same calendar day as today → "Today, May 21"
if same calendar day as yesterday → "Yesterday, May 20"
otherwise → "May 19" (just the date)
```

For week mode: "May 18 – May 24"
For month mode: "May 2026"

### 3. Navigation Arrows
- ← : move `selectedDate` back by 1 day / 1 week / 1 month
- → : move `selectedDate` forward (clamped to today)
- Clicking → when already on today does nothing

### 4. Segmented Control
- Clicking "Day" / "Week" / "Month" changes `viewMode`
- Active segment gets white pill + shadow (existing `.active` style)
- Changing mode resets `selectedDate` to today

### 5. Data Fetching
The backend `GET /activity?hours=N` fetches last N hours from now. For date-specific queries:

**Day mode:** Calculate `hours = hours between selectedDate midnight and now` (or 24 if not today). Pass to existing endpoint. Frontend filters events to only those on `selectedDate`.

**Week mode:** `hours = 168` (7 days). Frontend filters to the week containing `selectedDate`.

**Month mode:** `hours = 720` (30 days). Frontend filters to the month containing `selectedDate`.

**Key insight:** Don't add new backend endpoints. The existing `?hours=N` + frontend date filtering is sufficient for now. The events already have `ts` timestamps.

### 6. Date Formatting Helpers
```typescript
function formatLabel(date: Date, mode: string): string
function isSameDay(a: Date, b: Date): boolean
function addDays(date: Date, n: number): Date
function startOfWeek(date: Date): Date
function startOfMonth(date: Date): Date
function filterEventsForRange(events: EventRow[], start: Date, end: Date): EventRow[]
```

### 7. Auto-date update
- On mount, set a `setInterval` that checks if `currentDate` day has rolled over
- If so, update `currentDate = new Date()` so "Today" label stays correct
- Every 60 seconds is fine (nobody notices a 1-minute staleness)

## Files Changed
- `src/routes/+layout.svelte` — History view template + script
- No backend changes needed

## Out of Scope
- Backend date-bounded queries (future optimization)
- Summarize per-day caching (currently summarize always does last 2 hours)
