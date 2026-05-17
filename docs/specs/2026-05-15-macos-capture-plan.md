# macOS Activity Capture — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement full macOS activity capture with app focus detection, window title polling, browser URL capture via Accessibility API, periodic screenshot+OCR via Vision framework, and idle detection.

**Architecture:** A dedicated native thread runs a `CFRunLoop` with `NSWorkspace` notifications for focus changes and two `CFRunLoopTimer`s (5s for title/URL/idle, 30s for OCR). Events bridge to tokio via `mpsc::channel(4096)`. The struct implements the existing `ActivityCapture` trait, matching the Windows implementation pattern exactly.

**Tech Stack:** Rust 2024 edition, `objc` + `icrate` (AppKit/Foundation/Accessibility bindings), `core-graphics` (idle detection), `xcap` + Vision framework (OCR via raw `msg_send!`), `block` (CFRunLoop timer callbacks).

**Spec:** `docs/specs/2026-05-15-macos-capture-design.md`

---

## File Structure

| File | Action | Responsibility |
|------|--------|---------------|
| `ccube-capture/Cargo.toml` | Modify | Add macOS-only deps |
| `ccube-capture/src/lib.rs` | Modify | Add `OcrReady` event variant |
| `ccube-capture/src/macos.rs` | Rewrite | Full macOS capture: NSWorkspace, AX, idle, OCR trigger |
| `ccube-capture/src/ocr/macos.rs` | Rewrite | Vision framework OCR engine |
| `ccube-daemon/src/main.rs` | Modify | Conditional `MacActivityCapture` + `OcrReady` handler |
| `ccube-cli/src/commands/capture.rs` | Modify | Conditional `MacActivityCapture` + `OcrReady` handler |

---

### Task 1: Add macOS Dependencies

**Files:**
- Modify: `crates/ccube-capture/Cargo.toml`

- [ ] **Step 1: Add macOS-only dependencies to Cargo.toml**

Append the following to `crates/ccube-capture/Cargo.toml` after the existing `[target.'cfg(windows)'.dependencies]` block:

```toml
[target.'cfg(target_os = "macos")'.dependencies]
objc = "0.2"
block = "0.1"
icrate = { version = "0.1", features = [
    "Foundation",
    "Foundation_NSNotificationCenter",
    "Foundation_NSRunLoop",
    "Foundation_NSAutoreleasePool",
    "Foundation_NSData",
    "Foundation_NSString",
    "Foundation_NSURL",
    "Foundation_NSArray",
    "AppKit",
    "AppKit_NSWorkspace",
    "AppKit_NSRunningApplication",
] }
core-graphics = "0.25"
core-foundation = "0.10"
```

- [ ] **Step 2: Verify Cargo.toml parses**

Run: `cd /Users/harryyu/Downloads/0515-1837 && cargo check -p ccube-capture --target x86_64-apple-darwin 2>&1 | tail -5`
Expected: Compilation errors about missing `MacActivityCapture` (expected — we haven't written it yet) but no Cargo.toml parse errors.

- [ ] **Step 3: Commit**

```bash
cd /Users/harryyu/Downloads/0515-1837
git add crates/ccube-capture/Cargo.toml
git commit -m "deps: add macOS-only capture dependencies (objc, icrate, core-graphics)"
```

---

### Task 2: Add `OcrReady` Event Variant

**Files:**
- Modify: `crates/ccube-capture/src/lib.rs`

- [ ] **Step 1: Add the `OcrReady` variant to `ActivityEvent`**

In `crates/ccube-capture/src/lib.rs`, add the new variant to the `ActivityEvent` enum after `IdleEnd`:

```rust
    IdleEnd {
        ts: i64,
    },
    OcrReady {
        text: String,
        ts: i64,
    },
```

- [ ] **Step 2: Verify compilation**

Run: `cd /Users/harryyu/Downloads/0515-1837 && cargo check -p ccube-capture --target x86_64-apple-darwin 2>&1 | tail -5`
Expected: Compilation errors in daemon/cli about non-exhaustive match (the new variant isn't handled yet). The capture crate itself should compile.

- [ ] **Step 3: Commit**

```bash
cd /Users/harryyu/Downloads/0515-1837
git add crates/ccube-capture/src/lib.rs
git commit -m "capture: add OcrReady event variant for periodic OCR results"
```

---

### Task 3: Implement Vision Framework OCR Engine

**Files:**
- Rewrite: `crates/ccube-capture/src/ocr/macos.rs`

- [ ] **Step 1: Write the Vision OCR implementation**

Replace the entire contents of `crates/ccube-capture/src/ocr/macos.rs` with:

```rust
//! macOS OCR via Vision framework (VNRecognizeTextRequest).
//! Uses raw objc msg_send! calls since icrate doesn't include Vision bindings.

use anyhow::Result;
use objc::runtime::{Class, Object};
use objc::{class, msg_send, sel, sel_impl};

use super::OcrEngine;

pub struct MacOcrEngine;

impl OcrEngine for MacOcrEngine {
    fn extract_text(&self, image_data: &[u8]) -> Result<String> {
        let pool = unsafe { NSAutoreleasePool::new() };

        let result = unsafe { vision_ocr_inner(image_data) };

        unsafe { pool.drain() };
        result
    }
}

unsafe fn vision_ocr_inner(image_data: &[u8]) -> Result<String> {
    // 1. Create NSData from PNG bytes
    let nsdata: *mut Object = msg_send![
        class!(NSData),
        dataWithBytes: image_data.as_ptr() as *const std::ffi::c_void
        length: image_data.len()
    ];
    if nsdata.is_null() {
        anyhow::bail!("failed to create NSData from image bytes");
    }

    // 2. Create VNImageRequestHandler with the data
    let vn_image_request_handler_class = match Class::get("VNImageRequestHandler") {
        Some(cls) => cls,
        None => anyhow::bail!("Vision framework not available (VNImageRequestHandler class not found)"),
    };

    let handler: *mut Object = msg_send![
        vn_image_request_handler_class,
        initWithData: nsdata
        options: std::ptr::null::<*mut Object>()
    ];
    if handler.is_null() {
        anyhow::bail!("failed to create VNImageRequestHandler");
    }

    // 3. Create VNRecognizeTextRequest
    let vn_request_class = match Class::get("VNRecognizeTextRequest") {
        Some(cls) => cls,
        None => anyhow::bail!("Vision framework not available (VNRecognizeTextRequest class not found)"),
    };

    let request: *mut Object = msg_send![
        vn_request_class,
        initWithCompletionHandler: std::ptr::null::<*mut std::ffi::c_void>()
    ];
    if request.is_null() {
        anyhow::bail!("failed to create VNRecognizeTextRequest");
    }

    // Set recognition level to accurate (1 = VNRequestTextRecognitionLevelAccurate)
    let _: () = msg_send![request, setRecognitionLevel: 1];

    // 4. Perform the request
    let nserror: *mut Object = std::ptr::null_mut();
    let success: bool = msg_send![handler, performRequests: &[request][..] error: &nserror as *const _ as *mut _];
    if !success {
        let error_msg: *mut Object = msg_send![nserror, localizedDescription];
        let desc = nsstring_to_string(error_msg).unwrap_or_else(|| "unknown error".to_string());
        anyhow::bail!("VNImageRequestHandler performRequests failed: {}", desc);
    }

    // 5. Extract results
    let results: *mut Object = msg_send![request, results];
    if results.is_null() {
        return Ok(String::new());
    }

    let count: usize = msg_send![results, count];
    if count == 0 {
        return Ok(String::new());
    }

    let mut text_parts = Vec::with_capacity(count);
    for i in 0..count {
        let observation: *mut Object = msg_send![results, objectAtIndex: i];
        if observation.is_null() {
            continue;
        }

        // Get top candidates
        let candidates: *mut Object = msg_send![observation, topCandidates: 1];
        if candidates.is_null() {
            continue;
        }

        let candidate_count: usize = msg_send![candidates, count];
        if candidate_count > 0 {
            let candidate: *mut Object = msg_send![candidates, objectAtIndex: 0];
            let string: *mut Object = msg_send![candidate, string];
            if let Some(s) = nsstring_to_string(string) {
                if !s.is_empty() {
                    text_parts.push(s);
                }
            }
        }
    }

    Ok(text_parts.join("\n"))
}

/// Convert an NSString* to a Rust String.
unsafe fn nsstring_to_string(nsstring: *mut Object) -> Option<String> {
    if nsstring.is_null() {
        return None;
    }
    let len: usize = msg_send![nsstring, lengthOfBytesUsingEncoding: 4]; // NSUTF8StringEncoding = 4
    if len == 0 {
        return Some(String::new());
    }
    let mut buf = vec![0u8; len];
    let _: bool = msg_send![nsstring, getBytes: buf.as_mut_ptr() as *mut std::ffi::c_void maxLength: len encoding: 4 usedLength: std::ptr::null_mut() lossyConversion: true];
    Some(String::from_utf8_lossy(&buf).to_string())
}

/// Wrapper for NSAutoreleasePool.
struct NSAutoreleasePool {
    pool: *mut Object,
}

impl NSAutoreleasePool {
    unsafe fn new() -> Self {
        let pool: *mut Object = msg_send![class!(NSAutoreleasePool), new];
        Self { pool }
    }

    unsafe fn drain(&self) {
        let _: () = msg_send![self.pool, drain];
    }
}

impl Drop for NSAutoreleasePool {
    fn drop(&mut self) {
        unsafe { self.drain() };
    }
}
```

- [ ] **Step 2: Verify the OCR module compiles**

Run: `cd /Users/harryyu/Downloads/0515-1837 && cargo check -p ccube-capture --target x86_64-apple-darwin 2>&1 | tail -10`
Expected: The ocr/macos.rs compiles. May still have errors in macos.rs (stub) or other crates.

- [ ] **Step 3: Commit**

```bash
cd /Users/harryyu/Downloads/0515-1837
git add crates/ccube-capture/src/ocr/macos.rs
git commit -m "capture: implement Vision framework OCR engine for macOS"
```

---

### Task 4: Implement macOS Activity Capture — Core Structure

**Files:**
- Rewrite: `crates/ccube-capture/src/macos.rs`

This is the largest task. The file is split into logical sections. Write the entire file in one step.

- [ ] **Step 1: Write the complete macos.rs implementation**

Replace the entire contents of `crates/ccube-capture/src/macos.rs` with:

```rust
// macOS activity capture — NSWorkspace notifications + Accessibility API + Vision OCR.
//
// Architecture mirrors the Windows implementation:
// - Dedicated thread with CFRunLoop
// - NSWorkspace notification for app focus changes
// - 5s timer for title/URL/idle polling
// - 30s timer for screenshot + OCR
// - Bridge to tokio via mpsc::channel(4096)

use crate::ocr::OcrEngine;
use crate::{ActivityCapture, ActivityEvent};
use anyhow::Result;
use ccube_core::briefing::ActivitySnapshot;
use core_foundation::runloop::{kCFRunLoopDefaultMode, CFRunLoopRef, CFRunLoopTimerRef};
use icrate::Foundation::{
    NSAutoreleasePool, NSData, NSDate, NSNotification, NSNotificationCenter, NSObject,
    NSRunLoop, NSString, NSURL,
};
use icrate::AppKit::{NSRunningApplication, NSWorkspace};
use objc::runtime::Object;
use objc::{class, msg_send, sel, sel_impl};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use tokio::sync::mpsc;

/// Static CFRunLoop ref for shutdown signaling from the tokio side.
static CAPTURE_RUN_LOOP: Cell<Option<CFRunLoopRef>> = Cell::new(None);

/// Idle threshold: 5 minutes in milliseconds.
const IDLE_THRESHOLD_MS: u64 = 300_000;

/// Poll interval for title/URL/idle: 5 seconds.
const POLL_INTERVAL_SECS: f64 = 5.0;

/// OCR interval: 30 seconds.
const OCR_INTERVAL_SECS: f64 = 30.0;

thread_local! {
    static CAPTURE_STATE: RefCell<Option<CaptureState>> = const { RefCell::new(None) };
}

struct CaptureState {
    tx: mpsc::Sender<ActivityEvent>,
    last_app: String,
    last_title: String,
    last_url: Option<String>,
    idle_active: bool,
    has_ax_permission: bool,
    has_screen_permission: bool,
    ax_warned: bool,
    screen_warned: bool,
}

/// macOS activity capture implementation.
pub struct MacActivityCapture;

impl MacActivityCapture {
    pub fn new() -> Self {
        Self
    }
}

impl ActivityCapture for MacActivityCapture {
    async fn subscribe(&self) -> mpsc::Receiver<ActivityEvent> {
        let (tx, rx) = mpsc::channel(4096);
        std::thread::spawn(move || {
            capture_thread_main(tx);
        });
        // Give the capture thread a moment to start and register notifications
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        rx
    }

    async fn snapshot(&self) -> Result<ActivitySnapshot> {
        let app = unsafe { current_frontmost_app() };
        let title = if !app.is_empty() {
            unsafe { get_frontmost_window_title() }
        } else {
            None
        };
        Ok(ActivitySnapshot {
            app: app.unwrap_or_default(),
            title,
            url: None,
            duration_ms: 0,
        })
    }
}

/// Send a shutdown signal to the capture thread.
pub fn request_shutdown() {
    let rl = CAPTURE_RUN_LOOP.take();
    if let Some(run_loop) = rl {
        unsafe {
            core_foundation::runloop::CFRunLoopStop(run_loop);
        }
    }
}

// ---------------------------------------------------------------------------
// Capture thread main
// ---------------------------------------------------------------------------

fn capture_thread_main(tx: mpsc::Sender<ActivityEvent>) {
    let _pool = unsafe { NSAutoreleasePool::new() };

    // Check permissions
    let has_ax = unsafe { ax_is_process_trusted() };
    let has_screen = check_screen_recording_permission();

    if !has_ax {
        tracing::warn!(
            "Accessibility permission not granted — window titles and URLs will not be captured. \
             Grant in System Settings → Privacy & Security → Accessibility."
        );
    }
    if !has_screen {
        tracing::warn!(
            "Screen Recording permission not granted — OCR will be disabled. \
             Grant in System Settings → Privacy & Security → Screen Recording."
        );
    }

    // Initialize thread-local state
    CAPTURE_STATE.with(|cell| {
        *cell.borrow_mut() = Some(CaptureState {
            tx,
            last_app: String::new(),
            last_title: String::new(),
            last_url: None,
            idle_active: false,
            has_ax_permission: has_ax,
            has_screen_permission: has_screen,
            ax_warned: !has_ax,
            screen_warned: !has_screen,
        });
    });

    // Store the CFRunLoop ref for shutdown
    let cf_run_loop = unsafe { core_foundation::runloop::CFRunLoopGetMain() };
    CAPTURE_RUN_LOOP.set(Some(cf_run_loop));

    // Register for NSWorkspace app activation notifications
    register_focus_notification();

    // Install 5s polling timer (title, URL, idle)
    let poll_timer = create_cf_timer(POLL_INTERVAL_SECS, POLL_INTERVAL_SECS, poll_timer_callback);
    // Fire an initial snapshot
    poll_current_state();

    // Install 30s OCR timer
    let ocr_timer = create_cf_timer(OCR_INTERVAL_SECS, OCR_INTERVAL_SECS, ocr_timer_callback);

    tracing::info!("macOS capture thread started");

    // Run the CFRunLoop — blocks until CFRunLoopStop is called
    unsafe {
        let ns_run_loop = NSRunLoop::currentRunLoop();
        let mode = NSString::from_str("kCFRunLoopDefaultMode");
        let _ = msg_send![&ns_run_loop, run];
    }

    // Cleanup
    remove_cf_timer(poll_timer);
    remove_cf_timer(ocr_timer);

    CAPTURE_STATE.with(|cell| {
        *cell.borrow_mut() = None;
    });

    tracing::info!("macOS capture thread exiting");
}

// ---------------------------------------------------------------------------
// NSWorkspace focus notification
// ---------------------------------------------------------------------------

/// Callback type for NSWorkspace notification.
type NotificationCallback = unsafe fn(*mut Object, *mut Object);

fn register_focus_notification() {
    unsafe {
        let center = NSNotificationCenter::defaultCenter();
        let workspace = NSWorkspace::sharedWorkspace();

        // We need to observe NSWorkspaceDidActivateApplicationNotification
        let notification_name =
            NSString::from_str("NSWorkspaceDidActivateApplicationNotification");

        let _: () = msg_send![
            &center,
            addObserverForName: &*notification_name
            object: &*workspace
            queue: std::ptr::null::<*mut Object>()
            usingBlock: &focus_change_block as *const _ as *const std::ffi::c_void
        ];
    }
}

/// Called when the frontmost application changes.
static focus_change_block: objc::runtime::Block = unsafe {
    // We use a static block stub — the real block is created at runtime
    // in register_focus_notification via the ConcreteBlock wrapper
    objc::runtime::Block::from_raw(std::ptr::null_mut())
};

// Since we can't easily create an ObjC block in Rust that captures state,
// we'll use a notification observer pattern instead.

fn register_focus_notification() {
    use block::ConcreteBlock;

    unsafe {
        let center = NSNotificationCenter::defaultCenter();
        let workspace = NSWorkspace::sharedWorkspace();
        let notification_name =
            NSString::from_str("NSWorkspaceDidActivateApplicationNotification");

        let block = ConcreteBlock::new(|_notification: &NSObject| {
            handle_focus_change();
        });
        let block = block.copy();

        let _: () = msg_send![
            &center,
            addObserverForName: &*notification_name
            object: &*workspace
            queue: std::ptr::null::<*mut Object>()
            usingBlock: &*block as *const _ as *const std::ffi::c_void
        ];
    }
}

fn handle_focus_change() {
    let ts = now_ms();

    let app = match unsafe { current_frontmost_app() } {
        Some(a) => a,
        None => return,
    };

    // Filter transient macOS windows
    if is_transient_macos_app(&app) {
        return;
    }

    let title = if CAPTURE_STATE.with(|cell| {
        cell.borrow()
            .as_ref()
            .map(|s| s.has_ax_permission)
            .unwrap_or(false)
    }) {
        unsafe { get_frontmost_window_title() }
    } else {
        None
    };

    CAPTURE_STATE.with(|cell| {
        let Ok(mut borrow) = cell.try_borrow_mut() else { return };
        let state = match borrow.as_mut() {
            Some(s) => s,
            None => return,
        };

        if app != state.last_app {
            try_send_event(
                &state.tx,
                ActivityEvent::AppFocusChanged {
                    app: app.clone(),
                    title: title.clone(),
                    ts,
                },
            );
            state.last_app = app;
            state.last_title = title.unwrap_or_default();
        }
    });
}

// ---------------------------------------------------------------------------
// 5-second poll timer (title, URL, idle)
// ---------------------------------------------------------------------------

fn poll_timer_callback(_timer: CFRunLoopTimerRef) {
    poll_title_and_url();
    check_idle();
}

fn poll_title_and_url() {
    let ts = now_ms();

    let app = match unsafe { current_frontmost_app() } {
        Some(a) => a,
        None => return,
    };

    if is_transient_macos_app(&app) {
        return;
    }

    CAPTURE_STATE.with(|cell| {
        let Ok(mut borrow) = cell.try_borrow_mut() else { return };
        let state = match borrow.as_mut() {
            Some(s) => s,
            None => return,
        };

        // Check if app changed (missed notification)
        if app != state.last_app {
            let title = if state.has_ax_permission {
                unsafe { get_frontmost_window_title() }
            } else {
                None
            };
            try_send_event(
                &state.tx,
                ActivityEvent::AppFocusChanged {
                    app: app.clone(),
                    title: title.clone(),
                    ts,
                },
            );
            state.last_app = app;
            state.last_title = title.unwrap_or_default();
            state.last_url = None;
            return;
        }

        if !state.has_ax_permission {
            if !state.ax_warned {
                tracing::warn!("macOS: no Accessibility permission — skipping title/URL poll");
                state.ax_warned = true;
            }
            return;
        }

        // Poll window title
        let title = unsafe { get_frontmost_window_title() }.unwrap_or_default();
        if title != state.last_title {
            try_send_event(
                &state.tx,
                ActivityEvent::WindowTitleChanged {
                    title: title.clone(),
                    ts,
                },
            );
            state.last_title = title;
        }

        // Poll browser URL
        let app_lower = app.to_lowercase();
        if ccube_core::focus_mode::is_browser(&app_lower) {
            let url = unsafe { get_browser_url() };
            if url != state.last_url {
                if let Some(ref u) = url {
                    try_send_event(&state.tx, ActivityEvent::UrlChanged { url: u.clone(), ts });
                }
                state.last_url = url;
            }
        } else {
            state.last_url = None;
        }
    });
}

fn poll_current_state() {
    poll_title_and_url();
}

// ---------------------------------------------------------------------------
// 30-second OCR timer
// ---------------------------------------------------------------------------

fn ocr_timer_callback(_timer: CFRunLoopTimerRef) {
    let ts = now_ms();

    let should_ocr = CAPTURE_STATE.with(|cell| {
        cell.borrow()
            .as_ref()
            .map(|s| s.has_screen_permission)
            .unwrap_or(false)
    });

    if !should_ocr {
        return;
    }

    // Capture screenshot + OCR on this thread (CFRunLoop timer callbacks are
    // serialized, so this won't conflict with the 5s poll timer)
    let ocr_text = match run_ocr() {
        Ok(text) if !text.is_empty() => Some(text),
        Ok(_) => None,
        Err(e) => {
            tracing::debug!("macOS OCR failed: {e}");
            None
        }
    };

    if let Some(text) = ocr_text {
        CAPTURE_STATE.with(|cell| {
            let Ok(mut borrow) = cell.try_borrow_mut() else { return };
            let state = match borrow.as_mut() {
                Some(s) => s,
                None => return,
            };
            try_send_event(&state.tx, ActivityEvent::OcrReady { text, ts });
        });
    }
}

fn run_ocr() -> Result<String> {
    let png = crate::capture_screenshot()?;
    let engine = crate::ocr::MacOcrEngine;
    engine.extract_text(&png)
}

// ---------------------------------------------------------------------------
// Idle detection
// ---------------------------------------------------------------------------

fn check_idle() {
    let ts = now_ms();
    let idle_secs = unsafe { get_idle_seconds() };
    let idle_ms = (idle_secs * 1000.0) as u64;

    CAPTURE_STATE.with(|cell| {
        let Ok(mut borrow) = cell.try_borrow_mut() else { return };
        let state = match borrow.as_mut() {
            Some(s) => s,
            None => return,
        };

        if idle_ms >= IDLE_THRESHOLD_MS && !state.idle_active {
            try_send_event(&state.tx, ActivityEvent::IdleStart { ts });
            state.idle_active = true;
        } else if idle_ms < IDLE_THRESHOLD_MS && state.idle_active {
            try_send_event(&state.tx, ActivityEvent::IdleEnd { ts });
            state.idle_active = false;
        }
    });
}

/// Get seconds since last user input (keyboard/mouse) via CoreGraphics.
unsafe fn get_idle_seconds() -> f64 {
    let source = core_graphics::event::CGEventSource::new(
        core_graphics::event::CGEventSourceStateID::CombinedSessionState,
    );
    let elapsed = core_graphics::event::CGEventSource::seconds_since_last_event(
        &source.unwrap_or_else(|| {
            core_graphics::event::CGEventSource::new(
                core_graphics::event::CGEventSourceStateID::CombinedSessionState,
            )
            .unwrap()
        }),
    );
    elapsed
}

// ---------------------------------------------------------------------------
// macOS Accessibility API helpers
// ---------------------------------------------------------------------------

/// Check if the process has Accessibility permission.
unsafe fn ax_is_process_trusted() -> bool {
    let api = core_foundation::string::CFString::new("AXIsProcessTrusted");
    // Use AXIsProcessTrusted via CoreFoundation
    let trusted: bool = {
        let cls = class!(AXUIElement);
        let result: i8 = msg_send![cls, isProcessTrusted];
        result != 0
    };
    trusted
}

/// Get the frontmost app's executable name (e.g., "Google Chrome", "Code").
unsafe fn current_frontmost_app() -> Option<String> {
    let workspace = NSWorkspace::sharedWorkspace();
    let app: Option<&NSRunningApplication> = msg_send![&workspace, frontmostApplication];
    app.and_then(|running_app| {
        let url: Option<&NSURL> = msg_send![running_app, executableURL];
        url.and_then(|u| {
            let path: Option<&NSString> = msg_send![u, lastPathComponent];
            path.map(|s| s.to_string())
        })
    })
}

/// Get the focused window title via Accessibility API.
unsafe fn get_frontmost_window_title() -> Option<String> {
    let workspace = NSWorkspace::sharedWorkspace();
    let app: Option<&NSRunningApplication> = msg_send![&workspace, frontmostApplication];
    let running_app = app?;

    let pid: i32 = msg_send![running_app, processIdentifier];

    // Create AXUIElement for the application
    let ax_app = {
        let cls = class!(AXUIElement);
        let element: *mut Object = msg_send![cls, CreateApplication: pid];
        element
    };
    if ax_app.is_null() {
        return None;
    }

    // Get the focused window
    let focused_window = get_ax_attribute(ax_app, "AXFocusedWindow");
    if focused_window.is_null() {
        release_ax(ax_app);
        return None;
    }

    let title = get_ax_string_attribute(focused_window, "AXTitle");

    release_ax(focused_window);
    release_ax(ax_app);

    title
}

/// Get the browser URL from the address bar via Accessibility API.
unsafe fn get_browser_url() -> Option<String> {
    let workspace = NSWorkspace::sharedWorkspace();
    let app: Option<&NSRunningApplication> = msg_send![&workspace, frontmostApplication];
    let running_app = app?;

    let pid: i32 = msg_send![running_app, processIdentifier];

    let ax_app = {
        let cls = class!(AXUIElement);
        let element: *mut Object = msg_send![cls, CreateApplication: pid];
        element
    };
    if ax_app.is_null() {
        return None;
    }

    let focused_window = get_ax_attribute(ax_app, "AXFocusedWindow");
    if focused_window.is_null() {
        release_ax(ax_app);
        return None;
    }

    // Search for the address bar: look for AXTextField with AXDescription containing "address" or "URL"
    let url = search_ax_url(focused_window);

    release_ax(focused_window);
    release_ax(ax_app);

    url
}

/// Search the AX tree under `element` for a URL/address bar field.
unsafe fn search_ax_url(element: *mut Object) -> Option<String> {
    // First try: direct AXURL attribute on the window
    if let Some(url) = get_ax_string_attribute(element, "AXURL") {
        if url.starts_with("http") {
            return Some(url);
        }
    }

    // Second try: search children for AXTextField with address/URL description
    let children = get_ax_attribute_array(element, "AXChildren");
    for child in &children {
        let role = get_ax_string_attribute(*child, "AXRole").unwrap_or_default();
        let desc = get_ax_string_attribute(*child, "AXDescription").unwrap_or_default();
        let role_lower = role.to_lowercase();
        let desc_lower = desc.to_lowercase();

        if role_lower == "axtextfield"
            || role_lower == "axcombobox"
            || desc_lower.contains("address")
            || desc_lower.contains("url")
            || desc_lower.contains("search")
        {
            // Check if this element has a value that looks like a URL
            if let Some(value) = get_ax_string_attribute(*child, "AXValue") {
                if value.starts_with("http") || value.contains("://") {
                    return Some(value);
                }
            }
        }

        // Recurse into children (limit depth to avoid infinite loops)
        if let Some(url) = search_ax_url_depth(*child, 3) {
            return Some(url);
        }
    }

    None
}

unsafe fn search_ax_url_depth(element: *mut Object, max_depth: usize) -> Option<String> {
    if max_depth == 0 {
        return None;
    }

    let children = get_ax_attribute_array(element, "AXChildren");
    for child in &children {
        let role = get_ax_string_attribute(*child, "AXRole").unwrap_or_default();
        let desc = get_ax_string_attribute(*child, "AXDescription").unwrap_or_default();
        let role_lower = role.to_lowercase();
        let desc_lower = desc.to_lowercase();

        if role_lower == "axtextfield"
            || role_lower == "axcombobox"
            || desc_lower.contains("address")
            || desc_lower.contains("url")
        {
            if let Some(value) = get_ax_string_attribute(*child, "AXValue") {
                if value.starts_with("http") || value.contains("://") {
                    return Some(value);
                }
            }
        }

        if let Some(url) = search_ax_url_depth(*child, max_depth - 1) {
            return Some(url);
        }
    }

    None
}

/// Get an AX attribute as a raw AXUIElement ref.
unsafe fn get_ax_attribute(element: *mut Object, attr: &str) -> *mut Object {
    let attr_str = NSString::from_str(attr);
    let mut value: *mut Object = std::ptr::null_mut();
    let result: i32 = msg_send![element, CopyAttributeValue: &*attr_str as *const _ as *const std::ffi::c_void attribute: &mut value as *mut _];
    if result != 0 {
        std::ptr::null_mut()
    } else {
        value
    }
}

/// Get an AX attribute as a String.
unsafe fn get_ax_string_attribute(element: *mut Object, attr: &str) -> Option<String> {
    let attr_str = NSString::from_str(attr);
    let mut value: *mut Object = std::ptr::null_mut();
    let result: i32 = msg_send![element, CopyAttributeValue: &*attr_str as *const _ as *const std::ffi::c_void attribute: &mut value as *mut _];
    if result != 0 || value.is_null() {
        return None;
    }
    // Check if it's an NSString
    let string_value: Option<&NSString> = msg_send![value, isKindOfClass: class!(NSString)];
    if let Some(s) = string_value {
        // Re-try with proper cast
        let s: &NSString = &*(value as *const NSString);
        Some(s.to_string())
    } else {
        None
    }
}

/// Get an AX attribute as an array of AXUIElement refs.
unsafe fn get_ax_attribute_array(element: *mut Object, attr: &str) -> Vec<*mut Object> {
    let attr_str = NSString::from_str(attr);
    let mut value: *mut Object = std::ptr::null_mut();
    let result: i32 = msg_send![element, CopyAttributeValue: &*attr_str as *const _ as *const std::ffi::c_void attribute: &mut value as *mut _];
    if result != 0 || value.is_null() {
        return Vec::new();
    }
    let count: usize = msg_send![value, count];
    let mut items = Vec::with_capacity(count);
    for i in 0..count {
        let item: *mut Object = msg_send![value, objectAtIndex: i];
        if !item.is_null() {
            items.push(item);
        }
    }
    items
}

/// Release an AXUIElement (CFRelease).
unsafe fn release_ax(element: *mut Object) {
    if !element.is_null() {
        let _: () = msg_send![class!(NSObject), release];
    }
}

// ---------------------------------------------------------------------------
// CFRunLoop Timer helpers
// ---------------------------------------------------------------------------

fn create_cf_timer(
    interval: f64,
    initial_fire: f64,
    callback: extern "C" fn(CFRunLoopTimerRef),
) -> CFRunLoopTimerRef {
    unsafe {
        let run_loop = core_foundation::runloop::CFRunLoopGetMain();
        let mode = core_foundation::runloop::kCFRunLoopDefaultMode;

        let fire_date = core_foundation::date::CFAbsoluteTimeGetCurrent() + initial_fire;

        let context = core_foundation::runloop::CFRunLoopTimerContext {
            version: 0,
            info: std::ptr::null_mut(),
            retain: None,
            release: None,
            copyDescription: None,
        };

        let timer = core_foundation::runloop::CFRunLoopTimerCreate(
            std::ptr::null_mut(),
            fire_date,
            interval,
            0,
            0,
            callback,
            &context,
        );

        core_foundation::runloop::CFRunLoopAddTimer(run_loop, timer, mode);
        timer
    }
}

fn remove_cf_timer(timer: CFRunLoopTimerRef) {
    unsafe {
        let run_loop = core_foundation::runloop::CFRunLoopGetMain();
        let mode = core_foundation::runloop::kCFRunLoopDefaultMode;
        core_foundation::runloop::CFRunLoopRemoveTimer(run_loop, timer, mode);
        core_foundation::CFRelease(timer as *mut _);
    }
}

// ---------------------------------------------------------------------------
// Utility
// ---------------------------------------------------------------------------

fn try_send_event(tx: &mpsc::Sender<ActivityEvent>, event: ActivityEvent) {
    if let Err(e) = tx.try_send(event) {
        tracing::warn!("macOS capture event dropped: {e}");
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Check if the app is a transient macOS system UI element that should be filtered.
fn is_transient_macos_app(app: &str) -> bool {
    let app_lower = app.to_lowercase();
    matches!(
        app_lower.as_str(),
        "dock"
            | "loginwindow"
            | "windowserver"
            | "controlcenter"
            | "notificationcenter"
            | "systemuiserver"
            | "spotlight"
            | "mission control"
    )
}

/// Check if Screen Recording permission is granted by attempting a screenshot.
fn check_screen_recording_permission() -> bool {
    match crate::capture_screenshot() {
        Ok(data) if !data.is_empty() => true,
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_transient_macos_app() {
        assert!(is_transient_macos_app("Dock"));
        assert!(is_transient_macos_app("loginwindow"));
        assert!(is_transient_macos_app("WindowServer"));
        assert!(is_transient_macos_app("ControlCenter"));
        assert!(is_transient_macos_app("NotificationCenter"));
        assert!(is_transient_macos_app("SystemUIServer"));
        assert!(is_transient_macos_app("Spotlight"));
        assert!(is_transient_macos_app("Mission Control"));
        // Case insensitive
        assert!(is_transient_macos_app("dock"));
        assert!(is_transient_macos_app("DOCK"));
        // Not transient
        assert!(!is_transient_macos_app("Google Chrome"));
        assert!(!is_transient_macos_app("Code"));
        assert!(!is_transient_macos_app("iTerm"));
        assert!(!is_transient_macos_app("Safari"));
    }

    #[test]
    fn test_idle_threshold() {
        // 5 minutes = 300,000 ms
        assert_eq!(IDLE_THRESHOLD_MS, 300_000);
    }
}
```

- [ ] **Step 2: Fix the duplicate `register_focus_notification` function**

The file above has two definitions of `register_focus_notification()`. Remove the first one (the one with the `focus_change_block` static). Keep only the second version that uses `ConcreteBlock`.

- [ ] **Step 3: Verify the module compiles**

Run: `cd /Users/harryyu/Downloads/0515-1837 && cargo check -p ccube-capture --target x86_64-apple-darwin 2>&1 | tail -20`
Expected: `ccube-capture` compiles. The daemon and CLI will still fail on the `OcrReady` match arm.

- [ ] **Step 4: Commit**

```bash
cd /Users/harryyu/Downloads/0515-1837
git add crates/ccube-capture/src/macos.rs
git commit -m "capture: implement macOS activity capture (NSWorkspace, AX, OCR, idle)"
```

---

### Task 5: Wire Daemon capture_loop for macOS + OcrReady

**Files:**
- Modify: `crates/ccube-daemon/src/main.rs`

- [ ] **Step 1: Add macOS conditional import**

In `crates/ccube-daemon/src/main.rs`, find the line:
```rust
use ccube_capture::ActivityCapture;
```

After it, add the conditional import block:

```rust
#[cfg(target_os = "windows")]
use ccube_capture::windows::WinActivityCapture;
#[cfg(target_os = "macos")]
use ccube_capture::macos::MacActivityCapture;
```

- [ ] **Step 2: Add OcrReady handling in capture_loop**

In the `capture_loop` function, find the match arm for `IdleEnd` (the last variant currently handled). After it, add:

```rust
                    ccube_capture::ActivityEvent::OcrReady { text, ts: _ } => {
                        // Write OCR text to the most recent app_focus event
                        if let Some(&(prev_id, _)) = last_event.get("app_focus") {
                            if let Err(e) = db::update_event_ocr(&conn, prev_id, &text) {
                                tracing::warn!(error = %e, "failed to update OCR text");
                            }
                        }
                    }
```

Also add the same handler in the drain loop (the `while let Ok(event) = rx.try_recv()` block during shutdown). Find the drain match arm for `IdleEnd` and add after it:

```rust
                        ccube_capture::ActivityEvent::OcrReady { text, ts: _ } => {
                            if let Some(&(prev_id, _)) = last_event.get("app_focus") {
                                let _ = db::update_event_ocr(&conn, *prev_id, &text);
                            }
                        }
```

- [ ] **Step 3: Switch capture creation to conditional**

In `capture_loop`, find:
```rust
    let capture = ccube_capture::windows::WinActivityCapture::new();
```

Replace with:
```rust
    #[cfg(target_os = "windows")]
    let capture = WinActivityCapture::new();
    #[cfg(target_os = "macos")]
    let capture = MacActivityCapture::new();
```

- [ ] **Step 4: Switch the shutdown call to conditional**

In `capture_loop`, find:
```rust
                ccube_capture::windows::request_shutdown();
```

Replace with:
```rust
                #[cfg(target_os = "windows")]
                ccube_capture::windows::request_shutdown();
                #[cfg(target_os = "macos")]
                ccube_capture::macos::request_shutdown();
```

- [ ] **Step 5: Verify compilation**

Run: `cd /Users/harryyu/Downloads/0515-1837 && cargo check -p ccube-daemon --target x86_64-apple-darwin 2>&1 | tail -10`
Expected: Compiles successfully.

- [ ] **Step 6: Commit**

```bash
cd /Users/harryyu/Downloads/0515-1837
git add crates/ccube-daemon/src/main.rs
git commit -m "daemon: wire macOS capture + OcrReady event handling"
```

---

### Task 6: Wire CLI capture command for macOS + OcrReady

**Files:**
- Modify: `crates/ccube-cli/src/commands/capture.rs`

- [ ] **Step 1: Add macOS conditional import**

At the top of `crates/ccube-cli/src/commands/capture.rs`, add after the existing imports:

```rust
#[cfg(target_os = "windows")]
use ccube_capture::windows::WinActivityCapture;
#[cfg(target_os = "macos")]
use ccube_capture::macos::MacActivityCapture;
```

- [ ] **Step 2: Switch capture creation to conditional**

In `handle_capture_run`, find:
```rust
    let capture = WinActivityCapture::new();
```

Replace with:
```rust
    #[cfg(target_os = "windows")]
    let capture = WinActivityCapture::new();
    #[cfg(target_os = "macos")]
    let capture = MacActivityCapture::new();
```

- [ ] **Step 3: Add OcrReady match arm in both match blocks**

In the main capture loop match block, after the `IdleEnd` arm, add:

```rust
                    ccube_capture::ActivityEvent::OcrReady { text, ts } => {
                        // Write OCR text to the most recent app_focus event
                        if let Some(&(prev_id, _)) = last_event.get("app_focus") {
                            let _ = db::update_event_ocr(&conn, *prev_id, &text);
                        }
                        event_count += 1;
                    }
```

In the drain loop match block (during Ctrl+C shutdown), after the `IdleEnd` arm, add:

```rust
                        ccube_capture::ActivityEvent::OcrReady { text, ts: _ } => {
                            if let Some(&(prev_id, _)) = last_event.get("app_focus") {
                                let _ = db::update_event_ocr(&conn, *prev_id, &text);
                            }
                            event_count += 1;
                        }
```

- [ ] **Step 4: Switch the shutdown call to conditional**

Find:
```rust
                ccube_capture::windows::request_shutdown();
```

Replace with:
```rust
                #[cfg(target_os = "windows")]
                ccube_capture::windows::request_shutdown();
                #[cfg(target_os = "macos")]
                ccube_capture::macos::request_shutdown();
```

- [ ] **Step 5: Add print log for OcrReady**

In the main loop's print section, after the `"idle_end"` match arm, add:

```rust
                    "ocr_ready" => {
                        println!("[{time_str}] ocr: {} chars", text.len());
                    }
```

Note: This won't match on `kind` — we need to add it to the print section. Find the `match kind {` block after the `"idle_end"` arm and add before the `_ => {}`:

```rust
                    _ => {
                        // OcrReady and other events don't have a kind string
                    }
```

Actually, since `OcrReady` doesn't have a `kind` field, it's handled outside the `(kind, ts, app, title, url)` destructuring. Add a print line in the `OcrReady` match arm:

```rust
                    ccube_capture::ActivityEvent::OcrReady { text, ts } => {
                        if let Some(&(prev_id, _)) = last_event.get("app_focus") {
                            let _ = db::update_event_ocr(&conn, *prev_id, &text);
                        }
                        let time_str = format_time_ms(*ts);
                        println!("[{time_str}] ocr: {} chars", text.len());
                        event_count += 1;
                    }
```

- [ ] **Step 6: Verify compilation**

Run: `cd /Users/harryyu/Downloads/0515-1837 && cargo check -p ccube-cli --target x86_64-apple-darwin 2>&1 | tail -10`
Expected: Compiles successfully.

- [ ] **Step 7: Commit**

```bash
cd /Users/harryyu/Downloads/0515-1837
git add crates/ccube-cli/src/commands/capture.rs
git commit -m "cli: wire macOS capture + OcrReady event handling"
```

---

### Task 7: Build Verification + Integration Test

**Files:**
- None (verification only)

- [ ] **Step 1: Full workspace build for macOS**

Run: `cd /Users/harryyu/Downloads/0515-1837 && cargo build --target x86_64-apple-darwin 2>&1 | tail -20`
Expected: All 4 crates compile successfully.

- [ ] **Step 2: Run existing tests (macOS)**

Run: `cd /Users/harryyu/Downloads/0515-1837 && cargo test --target x86_64-apple-darwin 2>&1 | tail -30`
Expected: All existing tests pass. New tests (`test_is_transient_macos_app`, `test_idle_threshold`) pass.

- [ ] **Step 3: Run the CLI capture command briefly**

Run: `cd /Users/harryyu/Downloads/0515-1837 && timeout 10 cargo run --bin ccube --target x86_64-apple-darwin -- daemon capture 2>&1 || true`
Expected: Prints "Starting capture... Press Ctrl+C to stop." and starts logging events. May warn about permissions if not granted.

- [ ] **Step 4: Commit any fixes**

If any fixes were needed during verification:
```bash
cd /Users/harryyu/Downloads/0515-1837
git add -A
git commit -m "fix: address compilation/test issues from macOS capture integration"
```

---

### Task 8: Update DECISIONS.md

**Files:**
- Modify: `DECISIONS.md`

- [ ] **Step 1: Add decisions for macOS capture**

Append to `DECISIONS.md`:

```
[2026-05-15] phase-macos: macOS capture — hybrid NSWorkspace notifications + 5s AX poll + 30s Vision OCR. Mirrors Windows architecture (dedicated thread, CFRunLoop, mpsc bridge).
[2026-05-15] phase-macos: Browser URL via Accessibility API — AXUIElement tree walk for address bar field. Requires Accessibility permission. Graceful degradation to no URLs if ungranted.
[2026-05-15] phase-macos: Vision framework OCR via raw objc msg_send! — icrate doesn't include Vision bindings. Uses VNRecognizeTextRequest with .accurate recognition level.
[2026-05-15] phase-macos: Idle detection via CGEventSourceSecondsSinceLastEventType — CoreGraphics, no special permissions. 5-minute threshold matching Windows.
[2026-05-15] phase-macos: OcrReady event variant — OCR text posted through channel to tokio side for DB writes. Screenshot bytes never touch disk.
[2026-05-15] phase-macos: icrate for NSWorkspace/AppKit/Foundation — modern Apple framework bindings replacing objc-appkit (which doesn't exist on crates.io).
```

- [ ] **Step 2: Commit**

```bash
cd /Users/harryyu/Downloads/0515-1837
git add DECISIONS.md
git commit -m "docs: record macOS capture implementation decisions"
```
