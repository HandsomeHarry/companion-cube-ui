# Spec: Open-Session Grouping (implemented)

## Problem

The chunk-based summarizer grouped events well enough to demo, badly enough
to live with: labels were app-category mush ("Technical research and system
configuration"), every pass re-judged the world, and the model had no notion
of "this is still happening." Grouping quality is the core of the product —
History, the detector's drift judgment, and Rhythm all stand on it.

## Model

At most **one open session per day** — the activity that is still happening.
Everything else is **solidified** and never auto-touched again.

```
new events ──▶ membership pass ──▶ belong?  ──▶ append to open session
                    │                              (label may refresh)
                    └──▶ topic changed at k ──▶ solidify open session
                                                (final context label)
                                                new open session from k+1
AFK / sleep / >15 min gap ──▶ solidify immediately (the activity is over)
day rollover              ──▶ yesterday's open session closes
```

- **Membership pass** (every 5 min, batches ≤40): the LLM sees the open
  session's label + its last 5 events, then the new events, and answers one
  question: *how many of these still belong?* Quick chat checks, reference
  lookups, and videos serving the same purpose stay; the session ends only
  on a clear move to something unrelated.
- **Solidify** (on close): one more LLM call names the finished activity
  from everything in it. Labels are purpose-with-context, enforced by
  examples in the prompt: "terminal — working on companion cube",
  "browsing dieter rams design rules", "working on history essay while
  watching history videos and reading papers".
- **Breaks are structural, not judged**: idle/sleep markers and >15-min
  gaps split segments before the LLM ever sees them.

## User control (unchanged guarantees, refined semantics)

- Rename: pins the label (LLM never rewrites it) but the session **stays
  open** and keeps absorbing — naming the present doesn't end it.
- Drag in/out: pins both ends; solidified sessions stay untouchable.
- ⚡ Organize (Day view only): clears unpinned sessions and replays the day
  through the same state machine; today's last session ends up open.

## Detector synergy

The open session's label is the user's **inferred intention** — the missing
reference point that made "drift" undecidable. Both detector steps now
receive it ("What they are currently working on") and judge drift relative
to it: activity serving the current session is on-task by definition.

## Surfaces

- **UI**: the open session is the live head — gray dot, quiet NOW pill,
  not-yet-absorbed events fade into it at 55% opacity. The "Just now"
  pseudo-group appears only when no session is open yet.
- **CLI**: `ccube data sessions` — ID, state (open/pinned/closed), span,
  event count, label. Direct DB read; works without the daemon.

## Failure posture

Same as everywhere: the LLM unreachable or unparseable → the batch is
absorbed silently into the open session (never re-sent, never lost), and a
close falls back to the working label. Lenient JSON parsing throughout.

## Known limits

- Label quality is capped by capture signal: without Accessibility
  permission there are no window titles/URLs, so "browsing dieter rams
  design rules" requires the vision/OCR path or titles to see the words.
- One activity at a time: true parallel contexts (essay + reference videos)
  are expressible in the label ("X while Y") but not as parallel sessions.
