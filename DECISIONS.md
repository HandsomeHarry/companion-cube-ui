# Decisions

Format: `[YYYY-MM-DD] phase-N: <decision> — <why>`

[2026-04-14] phase-0: Use latest crate versions — spec versions were outdated; axum 0.8, rusqlite 0.39, reqwest 0.13, windows 0.62, thiserror 2, directories 6, active-win-pos-rs 0.10. No migration burden since starting fresh.
[2026-04-14] phase-0: Rust 2024 edition — using edition = "2024" since toolchain is 1.94.1 and it's the current standard.
[2026-04-14] phase-0: GNU toolchain (x86_64-pc-windows-gnu) — MSVC Build Tools installed but VS installer requires reboot to fully provision VC tools. Using MinGW-w64 14.2.0 standalone at C:\Users\Harry\mingw64-toolchain for dlltool/gcc. Revisit MSVC after reboot.
[2026-04-14] phase-0: CARGO_TARGET_DIR — AppLocker policy blocks test execution from D:\playground\ccube\target and Desktop. Use `CARGO_TARGET_DIR=C:\Users\Harry\.cargo\ccube-target` for all builds and test runs.
[2026-04-14] phase-0: Focus mode inference — tiered approach. Default: app name + file extension keyword match with markdown-written guides for the model. Future tiers: screenshot+OCR, multimodal AI. User chooses which tier. This is a core differentiator for ccube.
[2026-04-14] phase-0: LLM endpoint — flexible. Default http://localhost:8080 via CCUBE_LLM_URL env var. Supports both local llama.cpp and remote (e.g. WireGuard to GPU box).
[2026-04-14] phase-0: Notification style — native Windows toast only (via notify-rust). No custom UI for v1.
[2026-04-14] phase-0: URL capture scope — Chrome only for v1. Chromium-based browsers (Edge, Brave, Arc) likely work with the same UI Automation approach but untested until needed.
[2026-04-14] phase-0: Binary name — `ccube` confirmed.
[2026-04-14] phase-1: Core data layer complete — 17 tests passing (12 memory, 5 db), 5 CLI memory commands (show/edit/history/restore/diff), FTS5 validated, atomic write with history rotation, SHA-256 context fencing hash, unified diff via `similar` crate.
[2026-04-14] phase-1: rusqlite features changed to bundled-full — original ["bundled", "vtab"] did not enable FTS5. bundled-full enables all SQLite extensions including FTS5.
[2026-04-14] phase-1: DataRoot path resolution — CCUBE_DATA_DIR env var override, falls back to %APPDATA%\ccube\data via `directories` crate. Three subdirs: memory/, data/, logs/.
[2026-04-14] phase-2: Win32 thread + tokio bridge — dedicated OS thread for SetWinEventHook/COM/UI Automation message loop, bridging to async tokio via bounded mpsc::channel(4096) with try_send() (safe from extern "system" callbacks).
[2026-04-14] phase-2: Bounded channel (4096) — chosen over unbounded to match ActivityCapture trait signature (mpsc::Receiver) and provide backpressure. try_send() drops events silently on overflow rather than blocking the Win32 thread.
[2026-04-14] phase-2: Focus mode tier-1 keyword matching — pure function in ccube-core, no OS deps. URL-based detection highest priority, then app name patterns (IDEs, terminals, video production, writing apps, browsers with title heuristics). 13 tests cover all branches.
[2026-04-14] phase-2: VARIANT construction in Rust 2024 — ManuallyDrop union fields cannot be auto-dereferenced in edition 2024. Must construct VARIANT_0_0 directly and wrap: `VARIANT_0 { Anonymous: ManuallyDrop::new(inner) }`.
[2026-04-14] phase-2: active-win-pos-rs process_path — ActiveWindow.process_path is PathBuf, not String. Use .file_name().and_then(|n| n.to_str()) to extract the executable name.
[2026-04-14] phase-2: Idle detection — GetLastInputInfo + GetTickCount, 5-minute threshold. Checked every 5 seconds via SetTimer callback alongside title/URL polling.
[2026-04-14] phase-2: Phase 2 complete — 34 tests passing (17 phase-1 + 4 event CRUD + 13 focus mode). Live capture verified: detects app focus, persists to events.sqlite, CLI commands (capture run, activity recent, activity prune) all functional.
[2026-04-14] phase-3: DataRoot moved to ccube-core — both daemon and CLI need identical path resolution. Eliminates duplication; CLI re-exports from core.
[2026-04-14] phase-3: HTTP port 7431, loopback only — bound to 127.0.0.1:7431 per spec. No auth required since loopback-only. axum 0.8 with Arc<AppState>.
[2026-04-14] phase-3: CancellationToken for graceful shutdown — tokio-util CancellationToken shared across capture loop, scheduler, and HTTP server. Both POST /shutdown and Ctrl-C trigger it. 2-second timeout on task joins.
[2026-04-14] phase-3: Detached process spawn on Windows — daemon start uses CREATE_NO_WINDOW | DETACHED_PROCESS creation flags via std::os::windows::process::CommandExt. CLI polls /health up to 3 seconds to confirm startup.
[2026-04-14] phase-3: CLI HTTP-first routing — activity recent and memory show try daemon HTTP first (500ms health check timeout, 5s data timeout), fall back to direct DB. memory edit always direct per spec.
[2026-04-14] phase-3: JSON structured logging — tracing-appender writes daemon.ndjson in logs_dir. JSON layer for machine-readable logs, optional stdout layer when running attached. CLI `daemon logs` reads and pretty-prints the ndjson.
[2026-04-14] phase-3: schtasks autostart — ONLOGON trigger with LIMITED run level. Requires admin elevation; install/uninstall give clear error messages on non-elevated terminals.
[2026-04-14] phase-3: Serde on EventRow/CorrectionRow — added Serialize+Deserialize derives so HTTP endpoints can return DB rows as JSON directly.
[2026-04-14] phase-3: Scheduler hourly prune only — Phase 3 scheduler runs prune_events every hour. Agent scheduling deferred to later phases.
[2026-04-14] phase-3: UTF-8 safe truncation — title column display uses .chars().count() and .chars().take() instead of byte slicing. Prevents panics on CJK characters.
[2026-04-14] phase-3: Phase 3 complete — 34 tests passing. Daemon lifecycle verified: start, status, activity via HTTP, stop, direct-DB fallback, logs, install/uninstall. 7 new files, 10 modified.
[2026-04-14] phase-2: RefCell re-entrancy guard — Win32 STA COM calls (IUIAutomation::ElementFromHandle) pump the message queue, which can re-enter win_event_proc while CAPTURE_STATE RefCell is already borrowed. Changed borrow_mut() to try_borrow_mut() in all callback paths; re-entrant calls skip gracefully and the 5s timer catches any missed events.
[2026-04-14] phase-4: Briefing builder as pure function — briefing::build() takes all inputs explicitly (events, profile, patterns, vault). No I/O, fully testable with fixture data. 8 tests cover filtering, aggregation, dedup, and edge cases.
[2026-04-14] phase-4: LLM client via async_trait — LlmBackend trait enables test mocking without network calls. LlamaCppClient targets llama.cpp /completion endpoint with GBNF grammar. 10s timeout per spec.
[2026-04-14] phase-4: GBNF grammar for detector output — hand-written grammar constrains LLM to produce valid DetectorOutput JSON. Avoids the ~2% free-form parse failure rate mentioned in spec §15.
[2026-04-14] phase-4: Detector fallback is always Silent — on any failure (LLM unreachable, bad response, parse error), detector returns Silent with descriptive reasoning. Never crashes the daemon per spec §15.
[2026-04-14] phase-4: Frozen memory at daemon startup — profile.md and patterns.md loaded once into AppState. Detector uses frozen copies throughout session. Per spec §15: "Memory never changes mid-session."
[2026-04-14] phase-4: Detector trigger architecture — capture loop signals tokio::sync::Notify on app_focus events. Scheduler select!s between Notify (focus change) and 300s sleep (heartbeat). 30s debounce between runs.
[2026-04-14] phase-4: PowerShell balloon notifications — notify-rust v4.14 pulls windows v0.61 which has raw-dylib/dlltool issues with GNU toolchain. Replaced with PowerShell System.Windows.Forms.NotifyIcon balloon tip, spawned in background thread. Zero extra dependencies, works on all Windows versions.
[2026-04-14] phase-4: Detector decisions logged to detector.ndjson — each run appends one JSON line with ts, trigger, prompt_version, decision, reasoning, patterns_hash, duration_ms per spec §11.
[2026-04-14] phase-4: CLI detect fallback — ccube briefing and ccube detect try daemon HTTP first, fall back to direct DB + local LLM call when daemon is not running. Consistent with Phase 3 CLI routing pattern.
