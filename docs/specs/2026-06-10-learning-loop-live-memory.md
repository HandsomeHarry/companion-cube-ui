# Spec: Close the Learning Loop — Live Memory Snapshots + LLM Setup Hint

## Problem

The learning loop was wired but never closed. The scheduler already runs the
curator daily and the reflector weekly, and both commit to `patterns.md` — but
the detector used `profile.md`/`patterns.md` **frozen into AppState at daemon
startup** (phase-4 decision, spec §15 "Memory never changes mid-session"). So:

- User corrections → curator → `patterns.md` ✅
- `patterns.md` → detector briefing ❌ (stale until daemon restart)

For a daemon users never restart, learning silently never took effect.

A second silent failure: a fresh install without Ollama running (or without
the model downloaded) records activity fine but every LLM call fails with no
indication in the UI.

## Design

### 1. Live memory snapshots (replaces frozen memory)

`memory::load_snapshot(memory_dir) -> MemorySnapshot { profile, patterns,
patterns_hash }` reads both files and hashes patterns in one call.

Every agent run loads a **fresh snapshot at run start**:

| Call site | Before | After |
|---|---|---|
| scheduler detector run | frozen | snapshot per run |
| scheduler curator daily run | frozen | snapshot per run |
| scheduler reflector run | frozen profile + live patterns | snapshot per run |
| HTTP `/briefing`, `/detect` | frozen | snapshot per request |
| HTTP curator/reflector run | frozen / mixed | snapshot per request |

Why this is safe (and why §15 is superseded):

- **Consistency within a run** is preserved — one snapshot per run, memory
  never changes mid-inference.
- **Traceability** is preserved — `patterns_hash` is computed from the
  snapshot and logged with every decision (DB + detector.ndjson), so any
  decision is still attributable to the exact patterns version it saw.
- **Manual edits** (`ccube memory edit`, direct file edits) now also take
  effect on the next run — previously only visible after restart.
- Cost: two small file reads per run (detector runs at most every 30s).

Failure mode: unreadable memory degrades to empty strings with an error log —
agents never crash on I/O (consistent with the phase-4 "fallback is always
Silent" principle).

### 2. LLM setup hint (minimal onboarding)

`GET /api/llm/health` probes the configured backend with a 2s timeout:

- **ollama**: `GET {base}/api/tags` → `reachable` + `model_present`
  (configured model in the local tag list).
- **other providers**: any HTTP response = reachable; only connection
  failures mean down. `model_present: null`.

UI: one quiet line above the content (all views except Settings), only when
something is actually wrong:

- unreachable → "Ollama isn't running — your activity is still recorded, but
  won't be organized." + Settings link
- model missing → "The model "X" isn't downloaded yet — run `ollama pull X`."

Polled every 30s; disappears on its own once fixed. No wizard, no modal, no
first-run state machine — per Dieter Rams: as little design as possible,
honest about what works and what doesn't.

## Out of scope

- In-app model download/progress (would need Ollama pull streaming; revisit
  if the one-liner proves insufficient for non-technical users).
- Aura (smart lights) — deferred indefinitely; the smart-home device matrix
  is too broad to support well in a few iterations.
