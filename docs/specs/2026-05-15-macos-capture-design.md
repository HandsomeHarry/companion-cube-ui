# macOS Activity Capture — Design Spec

**Date:** 2026-05-15
**Scope:** Implement full macOS activity capture, OCR, and idle detection, matching Windows feature parity.

## Context

ccube has a complete Windows activity capture implementation (`ccube-capture/src/windows.rs`) using `SetWinEventHook`, UI Automation, and Windows OCR. The macOS capture is currently a stub (`macos.rs` contains one comment line). This spec brings macOS to full parity.

## Architecture

Mirror the Windows pattern: a **dedicated native thread** runs a macOS event loop (`CFRunLoop`), bridging events to the async tokio runtime via `mpsc::channel(4096)` with `try_send()`.

```
┌──────────────────────────────────────────────────────┐
│ macOS Capture Thread (CFRunLoop)                     │
│                                                      │
│  NSWorkspaceDidActivateApplicationNotification       │
│       └──► AppFocusChanged (app name)                │
│                                                      │
│  5s CFRunLoopTimer                                   │
│       ├──► poll title via AXUIElement                │
│       │    └──► WindowTitleChanged (if changed)      │
│       ├──► poll URL via AXUIElement (browsers only)  │
│       │    └──► UrlChanged (if changed)              │
│       └──► check idle via CGEventSource              │
│            ├──► IdleStart (idle ≥ 5 min)             │
│            └──► IdleEnd (idle < 5 min, was idle)     │
│                                                      │
│  30s CFRunLoopTimer                                  │
│       └──► screenshot via xcap + Vision OCR          │
│            └──► update ocr_text on latest event       │
│                                                      │
│  All events ──► mpsc::channel(4096) ───────────────► │
└──────────────────────────────────────────────────────┘
                         │
                         ▼
                 tokio async runtime
                 (daemon capture_loop)
```

## Component Details

### 1. App Focus Detection

- Register for `NSWorkspaceDidActivateApplicationNotification` via `NSNotificationCenter`.
- Extract the app name from `NSRunningApplication.executableURL.path.lastPathComponent` (e.g., `"Google Chrome"`, `"Code"`, `"iTerm"`).
- This produces names like `"Google Chrome"` rather than `"chrome.exe"`. The existing `focus_mode::is_browser()` and `focus_mode::infer_focus_mode()` use lowercase `contains()` matching, so `"Google Chrome".contains("chrome")` works — no changes needed in focus_mode.rs.
- Emit `AppFocusChanged` with app name + current title (from Accessibility API).
- Fire an initial snapshot on startup (matching Windows `poll_current_state()`).

### 2. Window Title Polling (5s timer)

- For the frontmost app, get the focused window via `AXUIElementCreateApplication(pid)` → `kAXFocusedWindowAttribute` → `kAXTitleAttribute`.
- If the title changed since last poll, emit `WindowTitleChanged`.
- If the app changed (missed the notification somehow), emit `AppFocusChanged`.
- Debounce: only emit when the value actually changes.

### 3. Browser URL Capture (5s timer, same callback)

- Only attempt for apps identified as browsers by `focus_mode::is_browser()`.
- Walk the AX tree looking for the address bar. Strategy:
  1. Get the focused window (`kAXFocusedWindowAttribute`).
  2. Search descendants for an element with `kAXRole: "AXTextField"` and `kAXDescription` containing "address" or "URL".
  3. Read `kAXValueAttribute` from that element.
- If the URL changed since last poll, emit `UrlChanged`.
- Graceful degradation: if AX permissions aren't granted, URL capture silently fails, logging a warning once.

### 4. Periodic Screenshot + OCR (30s timer)

- Capture screenshot via `xcap::Monitor::capture_image()` (already in Cargo.toml).
- Encode to PNG in memory.
- Pass to `MacOcrEngine::extract_text()` which uses Vision framework:
  - Create `VNImageRequestHandler` from PNG bytes.
  - Create `VNRecognizeTextRequest` with `recognitionLevel = .accurate`.
  - Perform the request, extract `VNRecognizedTextObservation` top candidates.
  - Concatenate into a single string.
- Screenshot bytes are dropped immediately after OCR — never written to disk.
- OCR text is written to the `ocr_text` column of the most recent `app_focus` event in `events.sqlite` via `db::update_event_ocr()`.
- The OCR runs synchronously on the CFRunLoop thread in the 30s timer callback. Since Vision typically completes in <1s and the 5s timer callbacks are queued by CFRunLoop (they won't fire concurrently), there are no re-entrancy concerns. The OCR result is posted through the channel as a new `OcrReady` event, keeping DB writes on the tokio side.

Add a new `ActivityEvent` variant:

```rust
pub enum ActivityEvent {
    // ... existing variants ...
    OcrReady {
        text: String,
        ts: i64,
    },
}
```

The daemon's `capture_loop` handles `OcrReady` by writing `update_event_ocr()` against the most recent `app_focus` event.

### 5. Idle Detection (5s timer, same callback)

- Call `CGEventSourceSecondsSinceLastEventType(.hidSystemState, .any)` to get seconds since last input.
- 5-minute threshold (300s), matching Windows `IDLE_THRESHOLD_MS`.
- Track `idle_active` boolean in thread-local state.
- Emit `IdleStart` when crossing the threshold, `IdleEnd` when returning.

### 6. Shutdown

- Store the CFRunLoop ref in a thread-local or global.
- `request_shutdown()` calls `CFRunLoopStop(run_loop)` from the tokio side.
- The capture thread's `CFRunLoopRun()` returns, cleanup runs, thread exits.
- Channel closes, tokio side sees `None` from `recv()`.

## Struct Layout

```rust
thread_local! {
    static CAPTURE_STATE: RefCell<Option<CaptureState>> = const { RefCell::new(None) };
    static RUN_LOOP: Cell<Option<CFRunLoopRef>> = const { Cell::new(None) };
}

struct CaptureState {
    tx: mpsc::Sender<ActivityEvent>,
    last_app: String,
    last_title: String,
    last_url: Option<String>,
    idle_active: bool,
}

pub struct MacActivityCapture;

impl MacActivityCapture {
    pub fn new() -> Self { Self }
}

impl ActivityCapture for MacActivityCapture {
    async fn subscribe(&self) -> mpsc::Receiver<ActivityEvent>;
    async fn snapshot(&self) -> Result<ActivitySnapshot>;
}
```

## Files to Change

| File | Action | Description |
|------|--------|-------------|
| `ccube-capture/Cargo.toml` | Modify | Add macOS-only dependencies |
| `ccube-capture/src/macos.rs` | Rewrite | Full macOS capture implementation |
| `ccube-capture/src/ocr/macos.rs` | Rewrite | Vision framework OCR implementation |
| `ccube-capture/src/lib.rs` | Modify | Add `OcrReady` event variant |
| `ccube-daemon/src/main.rs` | Modify | Conditional: `MacActivityCapture` on macOS; handle `OcrReady` event in `capture_loop` |
| `ccube-cli/src/commands/capture.rs` | Modify | Conditional: `MacActivityCapture` on macOS; handle `OcrReady` event in capture loop |

## New Dependencies (macOS-only)

In `ccube-capture/Cargo.toml`, under `[target.'cfg(target_os = "macos")'.dependencies]`:

```toml
[target.'cfg(target_os = "macos")'.dependencies]
objc = "0.2"
objc-foundation = "0.1"
objc-appkit = "0.1"
core-graphics = "0.24"
core-foundation = "0.10"
cocoa = "0.26"
block = "0.1"
```

> **Note:** Versions should be validated against the latest crates.io releases before implementation. `objc` 0.2 is the stable branch. `block` 0.1 provides the `ConcreteBlock` type needed for CFRunLoop timer callbacks.

Vision framework is accessed via `objc` calls to `VNImageRequestHandler` / `VNRecognizeTextRequest` — no separate crate needed. We link against `Vision.framework` via `extern_framework!` or manual `msg_send!`.

## macOS Permissions

Two permissions are required. The app must log clear warnings on first launch if not granted, and degrade gracefully.

| Permission | Why | Degradation if missing |
|------------|-----|----------------------|
| Accessibility (System Settings → Privacy & Security → Accessibility) | AXUIElement: window titles, browser URLs | No titles, no URLs. App name still works via NSWorkspace (no permission needed). |
| Screen Recording (System Settings → Privacy & Security → Screen Recording) | xcap screenshots + Vision OCR | No OCR text. Everything else works. |

On first run, detect missing permissions and log:
```
WARN ccube_capture::macos: Accessibility permission not granted — window titles and URLs will not be captured. Grant in System Settings → Privacy & Security → Accessibility.
WARN ccube_capture::macos: Screen Recording permission not granted — OCR will be disabled. Grant in System Settings → Privacy & Security → Screen Recording.
```

### Permission Detection

- **Accessibility:** Call `AXIsProcessTrusted()` — returns `false` if not granted.
- **Screen Recording:** Attempt `xcap::Monitor::all()` or a test screenshot. If it returns an error or a blank image, permission is not granted. Cache the result to avoid spamming.

## Transient Window Filtering

Filter out these macOS-specific transient apps (equivalent to Windows `is_transient_shell_window`):

- `Dock.app` — macOS Dock
- `WindowManager` / `Window Server` — system compositing
- `loginwindow` — login screen
- `ControlCenter` — Control Center popup
- `Notification Center` — notification overlay

## OCR: Vision Framework Integration

```rust
pub struct MacOcrEngine;

impl OcrEngine for MacOcrEngine {
    fn extract_text(&self, image_data: &[u8]) -> Result<String> {
        // 1. Create NSData from PNG bytes
        // 2. Create VNImageRequestHandler with the data
        // 3. Create VNRecognizeTextRequest (recognitionLevel = .accurate)
        // 4. Perform the request
        // 5. Extract top-1 candidate from each VNRecognizedTextObservation
        // 6. Join with newlines
        // 7. Return the concatenated string
    }
}
```

Uses `objc` crate's `msg_send!` macro to call Objective-C Vision APIs. The OCR runs synchronously on the calling thread (the 30s timer callback on the CFRunLoop thread). Since Vision typically completes in <1s for a single screenshot, this won't block the 5s title/URL poll.

If Vision is unavailable (ancient macOS version), return empty string with a logged warning.

## Daemon Integration

The daemon's `capture_loop` already handles all `ActivityEvent` variants generically. The only addition is handling the new `OcrReady` variant:

```rust
ccube_capture::ActivityEvent::OcrReady { text, ts } => {
    // Find the most recent app_focus event and update its ocr_text
    if let Some(&(prev_id, _)) = last_event.get("app_focus") {
        let _ = db::update_event_ocr(&conn, prev_id, &text);
    }
}
```

The conditional compilation in `main.rs` and `capture.rs` switches between `WinActivityCapture` and `MacActivityCapture` based on `target_os`.

## Testing

1. **Unit: idle threshold** — pure function, `should_be_idle(elapsed_secs, threshold) -> bool`.
2. **Unit: transient window filter** — `is_transient_macos_window(app, title) -> bool`.
3. **Integration: subscribe returns channel** — `MacActivityCapture::new().subscribe()` completes without panic.
4. **Unit: OCR engine** — `MacOcrEngine.extract_text(blank_png)` returns empty string.
5. **Unit: Vision framework availability** — verify the objc calls don't crash on a headless CI (guard with `cfg!(test)` or skip if no display).

## Scope Boundaries

**In scope:**
- All five capture components (focus, title, URL, OCR, idle)
- `MacOcrEngine` via Vision framework
- Permission detection + graceful degradation
- Daemon integration (conditional compilation)
- CLI capture command integration

**Out of scope (future work):**
- Linux capture implementation
- Vault feature
- Calendar integration
- Automatic permission grant prompts (just log warnings)
