# Spec: History Organization Rework + macOS App Bundle

Status: proposal (nothing here is implemented yet)

## Part 1 — Custom notification identity needs an .app bundle

`osascript display notification` always posts under **Script Editor's**
identity; no flag changes the icon. macOS grants notification identity
(name, icon, action buttons) only to a real `.app` bundle posting through
`UNUserNotificationCenter`, and the bundle must be signed (ad-hoc is fine
for local builds).

One package fixes four things:

| Problem today | Fixed by bundle |
|---|---|
| Banner shows Script Editor icon | Our name + cube icon on every nudge |
| No action buttons on nudges | `UNNotificationAction` → "Save to Vault" / "Snooze" directly on the banner (design spec §4, the part we deferred) |
| No login-item autostart | `SMAppService` needs a bundle |
| Raw binary triggers Gatekeeper warnings | Signed bundle + eventual notarization |

Plan:
1. `Companion Cube.app/Contents/{MacOS/ccube-daemon, Info.plist, Resources/ccube.icns}`
   built by a small `make bundle` script (iconutil renders the icns from the
   brand circle; later the real cube art).
2. Ad-hoc `codesign` in the script; notarization when distribution matters.
3. Notifications via `UNUserNotificationCenter` (objc2-user-notifications,
   already in the objc2 family we link). Keep the osascript path as fallback
   when running un-bundled (dev builds, `cargo run`).
4. Banner actions: "Save to Vault…" and "Snooze 5 min" — the design-doc
   nudge card mapped onto native notification actions. Hold-to-snooze
   friction can't exist in a banner; the snooze action is the lightweight
   stand-in. (UNUserNotificationCenter from a bundle is also how we'd later
   render a true custom card window if ever wanted.)

## Part 2 — History view: grouping, dragging, organizing, updating

### How it works today (code reality)

- The LLM re-summarizes the whole lookback window every 5 minutes (and on
  ⚡ Organize). The response **wholesale replaces** `localGroups`
  (`+layout.svelte`: `generated_at` change → rebuild, `expandedGroups`
  reset to all-expanded).
- Group identity is the **title string**. Drag corrections post
  `from_group`/`to_group` as titles; renames and duplicate titles make
  corrections ambiguous. A rename lives only until the next summarize pass.
- Events newer than the last summarize render as a **flat tail at the
  bottom**, while groups sort newest-first at the top — two temporal
  directions in one view.
- Drag exists (pointer-based, ≡ handle) but only moves an event into
  another *existing* group; no "make new group", no multi-select, no undo.
  A 5-minute auto-summarize can land mid-drag and stomp local state.
- User moves are recorded as corrections (good — they feed the curator) but
  the *display* isn't pinned: the next LLM pass is free to regroup
  everything differently. Manual effort visibly evaporates.
- `Timeline.svelte` / `TimelineGroup.svelte` / `TimelineItem.svelte` are
  dead code — the real rendering is inline in `+layout.svelte`.
- Updates are 30s full refetches + the 5-min summarize; no push channel.

### Design principles for the rework

The product promise is "your corrections teach it." That requires the UI to
*honor* corrections immediately and durably. The LLM proposes; the user
disposes; the system must never visibly undo the user.

### Proposed changes

**1. Sessions become rows, not strings.**
New `sessions` table: `id, range_key, label, start_ts, end_ts, distraction,
pinned, created_by (llm|user)`; events get a nullable `session_id`. Group
corrections reference session IDs. Renames update the row — permanent by
construction. The summaries JSON cache remains only as the LLM I/O format.

**2. Organize becomes incremental and pin-respecting.**
- Auto-pass every 5 min summarizes **only events after the last session
  boundary** — it appends or extends, never rewrites history.
- Any session the user touched (rename, drag in/out) sets `pinned`;
  LLM passes never modify pinned sessions or their members.
- ⚡ Organize = full re-pass over the visible range, still skipping pinned
  sessions. (Escape hatch: "Reset grouping" clears pins for the range.)

**3. One temporal direction, with a live head.**
Single newest-first column. The ungrouped tail becomes a **"Just now"
pseudo-group at the top** (gray dot, no LLM label yet). New events appear
there live and visibly *crystallize* into a named session on the next
organize pass — the update model becomes legible instead of mysterious.

**4. Drag that matches the promise.**
Keep the ≡ pointer drag, add: drop on "Just now"→new session, drop between
groups → "New session" target, Esc cancels, and a 5s undo toast after a
move. Local drag state survives background refreshes (ID-keyed, summaries
merge instead of replace).

**5. Update transport.**
`GET /api/events/stream` (SSE) pushes new events + session changes; the
30s polling remains the fallback. Eliminates the refetch flicker and makes
"Just now" feel live.

**6. Delete dead components.**
Remove the unused `Timeline*.svelte` triplet; extract the real inline
markup into `Session.svelte` + `LiveTail.svelte` once behavior settles.

### Sequencing

1. Sessions table + ID-based corrections (backend, breaking-free)
2. Pinning + incremental organize (backend + minor UI)
3. "Just now" head + merge-don't-replace updates (UI)
4. SSE stream (backend + UI)
5. Bundle + UNUserNotificationCenter (Part 1, independent track)
