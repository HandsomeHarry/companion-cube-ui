// macOS activity capture — NSWorkspace notifications + Accessibility API + Vision OCR.
//
// All macOS APIs called via raw objc msg_send! for maximum compatibility.
// No icrate/objc-foundation dependency needed.

use crate::{ActivityCapture, ActivityEvent};
use anyhow::Result;
use block::ConcreteBlock;
use ccube_core::briefing::ActivitySnapshot;
use core_foundation::date::CFAbsoluteTimeGetCurrent;
use core_foundation::runloop::{
    kCFRunLoopDefaultMode, CFRunLoop, CFRunLoopTimer, CFRunLoopTimerCallBack, CFRunLoopTimerContext,
};
use core_foundation_sys::runloop::CFRunLoopTimerRef as CFRunLoopTimerRefRaw;
use objc::runtime::Object;
use objc::{class, msg_send, sel, sel_impl};
use std::cell::RefCell;
use std::ffi::c_void;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc;

/// Idle threshold: 5 minutes in seconds.
const IDLE_THRESHOLD_SECS: f64 = 300.0;

/// A poll-to-poll gap beyond this means the machine was asleep
/// (polls run every 5s; even heavy load won't stall one this long).
const SLEEP_GAP_MS: i64 = 60_000;

/// Poll interval for title/URL/idle: 5 seconds.
const POLL_INTERVAL_SECS: f64 = 5.0;

/// OCR interval: 30 seconds.
const OCR_INTERVAL_SECS: f64 = 30.0;

static SHUTDOWN_REQUESTED: AtomicBool = AtomicBool::new(false);

thread_local! {
    static CAPTURE_STATE: RefCell<Option<CaptureState>> = const { RefCell::new(None) };
}

struct CaptureState {
    tx: mpsc::Sender<ActivityEvent>,
    last_app: String,
    last_title: String,
    last_url: Option<String>,
    idle_active: bool,
    ax_warned: bool,
    /// Wall-clock of the previous poll tick — a large jump means the
    /// machine slept, which the idle check cannot see (timers freeze).
    last_poll_ms: i64,
}

#[derive(Default)]
pub struct MacActivityCapture;

impl MacActivityCapture {
    pub fn new() -> Self {
        Self
    }
}

impl ActivityCapture for MacActivityCapture {
    async fn subscribe(&self) -> mpsc::Receiver<ActivityEvent> {
        let (tx, rx) = mpsc::channel(4096);
        std::thread::Builder::new()
            .name("ccube-macos-capture".to_string())
            .spawn(move || capture_thread_main(tx))
            .expect("failed to spawn macOS capture thread");
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        rx
    }

    async fn snapshot(&self) -> Result<ActivitySnapshot> {
        let app = unsafe { current_frontmost_app() }.unwrap_or_default();
        let title = if !app.is_empty() {
            unsafe { get_frontmost_window_title() }
        } else {
            None
        };
        Ok(ActivitySnapshot {
            app,
            title,
            url: None,
            duration_ms: 0,
        })
    }
}

pub fn request_shutdown() {
    SHUTDOWN_REQUESTED.store(true, Ordering::Release);
    CFRunLoop::get_current().stop();
}

// ---------------------------------------------------------------------------
// Capture thread main
// ---------------------------------------------------------------------------

fn capture_thread_main(tx: mpsc::Sender<ActivityEvent>) {
    let _pool = unsafe { ns_autorelease_pool_new() };

    let has_ax = unsafe { ax_is_process_trusted() };
    let has_screen = check_screen_recording_permission();

    if !has_ax {
        tracing::warn!(
            "Accessibility permission not granted — prompting. \
             Grant in System Settings → Privacy & Security → Accessibility."
        );
        // Show the system prompt (deep-links to the right Settings pane).
        request_accessibility_permission();
    }
    if !has_screen {
        tracing::warn!(
            "Screen Recording permission not granted — OCR will be disabled. \
             Grant in System Settings → Privacy & Security → Screen Recording."
        );
    }

    CAPTURE_STATE.with(|cell| {
        *cell.borrow_mut() = Some(CaptureState {
            tx,
            last_app: String::new(),
            last_title: String::new(),
            last_url: None,
            idle_active: false,
            ax_warned: !has_ax,
            last_poll_ms: 0,
        });
    });

    register_focus_notification();

    let run_loop = CFRunLoop::get_current();

    let poll_timer = create_timer(POLL_INTERVAL_SECS, POLL_INTERVAL_SECS, poll_timer_callback);
    run_loop.add_timer(&poll_timer, unsafe { kCFRunLoopDefaultMode });

    poll_title_url_and_idle();

    let ocr_timer = create_timer(OCR_INTERVAL_SECS, OCR_INTERVAL_SECS, ocr_timer_callback);
    run_loop.add_timer(&ocr_timer, unsafe { kCFRunLoopDefaultMode });


    CFRunLoop::run_current();

    drop(poll_timer);
    drop(ocr_timer);

    CAPTURE_STATE.with(|cell| {
        *cell.borrow_mut() = None;
    });

    tracing::info!("macOS capture thread exiting");
}

// ---------------------------------------------------------------------------
// NSWorkspace focus notification
// ---------------------------------------------------------------------------

fn register_focus_notification() {
    let block = ConcreteBlock::new(|_notification: *mut Object| {
        handle_focus_change();
    });
    let block = block.copy();
    // Get raw pointer before forgetting — NSNotificationCenter holds this for app lifetime.
    // If we don't leak, Rust drops the block on return and ObjC calls freed memory.
    let block_ptr: *const c_void = &*block as *const _ as *const c_void;
    std::mem::forget(block);

    unsafe {
        let center: *mut Object = msg_send![class!(NSNotificationCenter), defaultCenter];
        let workspace: *mut Object = msg_send![class!(NSWorkspace), sharedWorkspace];
        let name = ns_string("NSWorkspaceDidActivateApplicationNotification");

        let _: () = msg_send![
            center,
            addObserverForName: name
            object: workspace
            queue: ptr::null::<*mut c_void>()
            usingBlock: block_ptr
        ];
    }
}

fn handle_focus_change() {
    let ts = now_ms();
    let app = match unsafe { current_frontmost_app() } {
        Some(a) => a,
        None => return,
    };
    if is_transient_macos_app(&app) {
        return;
    }

    let title = if accessibility_permission_now() {
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
            try_send_event(&state.tx, ActivityEvent::AppFocusChanged {
                app: app.clone(),
                title: title.clone(),
                ts,
            });
            state.last_app = app;
            state.last_title = title.unwrap_or_default();
            state.last_url = None;
        }
    });
}

// ---------------------------------------------------------------------------
// 5-second poll timer
// ---------------------------------------------------------------------------

extern "C" fn poll_timer_callback(
    _timer: CFRunLoopTimerRefRaw,
    _info: *mut c_void,
) {
    if SHUTDOWN_REQUESTED.load(Ordering::Acquire) {
        CFRunLoop::get_current().stop();
        return;
    }
    // Same boundary rule as ocr_timer_callback: panics must not cross
    // extern "C" (they abort the process).
    if std::panic::catch_unwind(poll_title_url_and_idle).is_err() {
        tracing::warn!("poll callback panicked; skipping this cycle");
    }
}

fn poll_title_url_and_idle() {
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

        let has_ax = accessibility_permission_now();
        if app != state.last_app {
            let title = if has_ax {
                unsafe { get_frontmost_window_title() }
            } else {
                None
            };
            try_send_event(&state.tx, ActivityEvent::AppFocusChanged {
                app: app.clone(),
                title: title.clone(),
                ts,
            });
            state.last_app = app;
            state.last_title = title.unwrap_or_default();
            state.last_url = None;
        } else if has_ax {
            let title = unsafe { get_frontmost_window_title() }.unwrap_or_default();
            if title != state.last_title {
                try_send_event(&state.tx, ActivityEvent::WindowTitleChanged {
                    title: title.clone(),
                    ts,
                });
                state.last_title = title;
            }

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
        } else if !state.ax_warned {
            tracing::debug!("macOS: no Accessibility permission — skipping title/URL poll");
            state.ax_warned = true;
        }

        // Sleep detection: timers freeze during system sleep, so the idle
        // check never fires across a closed lid — a night becomes app time.
        // A poll-to-poll wall-clock jump is the sleep signal: emit a
        // synthetic away period covering it.
        if state.last_poll_ms > 0 && !state.idle_active {
            let gap = ts - state.last_poll_ms;
            if gap > SLEEP_GAP_MS {
                try_send_event(&state.tx, ActivityEvent::IdleStart { ts: state.last_poll_ms });
                try_send_event(&state.tx, ActivityEvent::IdleEnd { ts });
                state.last_app.clear(); // re-emit focus, fresh clock
            }
        }
        state.last_poll_ms = ts;

        // Idle check
        let idle_secs = unsafe { get_idle_seconds() };
        if idle_secs >= IDLE_THRESHOLD_SECS && !state.idle_active {
            // Watching a video produces no input but is not AFK — and it's
            // exactly the activity drift detection must keep seeing. Media
            // players hold display-sleep assertions; respect them.
            if !display_sleep_prevented() {
                // The user left when input stopped, not when we noticed.
                let onset_ts = ts - (idle_secs * 1000.0) as i64;
                try_send_event(&state.tx, ActivityEvent::IdleStart { ts: onset_ts });
                state.idle_active = true;
            }
        } else if idle_secs < IDLE_THRESHOLD_SECS && state.idle_active {
            try_send_event(&state.tx, ActivityEvent::IdleEnd { ts });
            state.idle_active = false;
            // Force the next poll to re-emit AppFocusChanged, opening a
            // fresh focus event so post-break time starts a clean clock.
            state.last_app.clear();
        }
    });
}

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOPMCopyAssertionsStatus(
        assertions_status: *mut core_foundation_sys::dictionary::CFDictionaryRef,
    ) -> i32;
}

/// True while any process holds a display-sleep-prevention assertion —
/// the standard "media is playing" signal (video players, presentations).
/// Reads assertion *counts* only; no process inspection, no content.
fn display_sleep_prevented() -> bool {
    use core_foundation::base::TCFType;
    use core_foundation::dictionary::CFDictionary;
    use core_foundation::number::CFNumber;
    use core_foundation::string::CFString;
    use core_foundation_sys::dictionary::CFDictionaryRef;

    let mut dict_ref: CFDictionaryRef = std::ptr::null();
    let kr = unsafe { IOPMCopyAssertionsStatus(&mut dict_ref) };
    if kr != 0 || dict_ref.is_null() {
        return false;
    }
    let dict: CFDictionary<CFString, CFNumber> =
        unsafe { CFDictionary::wrap_under_create_rule(dict_ref as _) };

    ["PreventUserIdleDisplaySleep", "NoDisplaySleepAssertion"]
        .iter()
        .any(|key| {
            dict.find(CFString::new(key))
                .and_then(|n| n.to_i64())
                .unwrap_or(0)
                > 0
        })
}

// ---------------------------------------------------------------------------
// 30-second OCR timer
// ---------------------------------------------------------------------------

extern "C" fn ocr_timer_callback(
    _timer: CFRunLoopTimerRefRaw,
    _info: *mut c_void,
) {
    if SHUTDOWN_REQUESTED.load(Ordering::Acquire) {
        return;
    }
    let ts = now_ms();

    // Live permission check each cycle — never the startup flag: the user
    // may grant Screen Recording after launch (or revoke it mid-session),
    // and OCR must follow that immediately in both directions.
    if !screen_permission_now() {
        return;
    }

    // catch_unwind: a panic must not cross this extern "C" boundary (it
    // aborts the process). xcap is known to panic internally when screen
    // recording permission is missing or revoked mid-session.
    let ocr_text = match std::panic::catch_unwind(run_ocr) {
        Ok(Ok(text)) if !text.is_empty() => Some(text),
        Ok(Ok(_)) => None,
        Ok(Err(e)) => {
            tracing::debug!("macOS OCR failed: {e}");
            None
        }
        Err(_) => {
            tracing::warn!("macOS OCR panicked (screen permission?); skipping this cycle");
            None
        }
    };

    if let Some(text) = ocr_text {
        CAPTURE_STATE.with(|cell| {
            let Ok(mut borrow) = cell.try_borrow_mut() else { return };
            if let Some(state) = borrow.as_mut() {
                try_send_event(&state.tx, ActivityEvent::OcrReady { text, ts });
            }
        });
    }
}

fn run_ocr() -> anyhow::Result<String> {
    let png = crate::capture_screenshot()?;
    let engine = crate::ocr::create_engine()
        .ok_or_else(|| anyhow::anyhow!("no OCR engine available"))?;
    engine.extract_text(&png)
}

// ---------------------------------------------------------------------------
// Idle detection
// ---------------------------------------------------------------------------

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventSourceSecondsSinceLastEventType(state_id: i32, event_type: u32) -> f64;
    /// True if the process already has Screen Recording permission. Cheap,
    /// never prompts, never throws.
    fn CGPreflightScreenCaptureAccess() -> bool;
    /// Triggers the system Screen Recording prompt (once per identity).
    fn CGRequestScreenCaptureAccess() -> bool;
}

unsafe fn get_idle_seconds() -> f64 {
    unsafe { CGEventSourceSecondsSinceLastEventType(0, 0) }
}

// ---------------------------------------------------------------------------
// Accessibility API
// ---------------------------------------------------------------------------

unsafe fn ax_is_process_trusted() -> bool {
    unsafe { AXIsProcessTrusted() }
}

unsafe fn current_frontmost_app() -> Option<String> {
    // NSWorkspace must be called from the main thread.
    // Use NSProcessInfo + ActiveApp via C API to avoid thread issues.
    // Actually, use a simple osascript subprocess — thread-safe and reliable.
    let output = std::process::Command::new("osascript")
        .args(["-e", "tell application \"System Events\" to get name of first process whose frontmost is true"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if name.is_empty() { None } else { Some(name) }
}

unsafe fn get_frontmost_pid() -> Option<i32> {
    // NSWorkspace.frontmostApplication is main-thread-only.
    // Use osascript subprocess — thread-safe.
    let output = std::process::Command::new("osascript")
        .args(["-e", "tell application \"System Events\" to get unix id of first process whose frontmost is true"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let pid_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
    pid_str.parse::<i32>().ok()
}

/// Run an ObjC/AX closure, turning any thrown NSException into None. The AX
/// tree belongs to *other* apps, some of which return nodes that make the
/// Accessibility APIs throw; a foreign exception crossing into Rust aborts
/// the whole daemon, so every AX query must pass through here. (These paths
/// only began executing once Accessibility permission actually persisted —
/// see scripts/dev-signing-identity.sh — which is why the throws surfaced.)
fn ax_catch<T>(f: impl FnOnce() -> Option<T> + std::panic::UnwindSafe) -> Option<T> {
    objc2::exception::catch(std::panic::AssertUnwindSafe(f))
        .unwrap_or_else(|e| {
            tracing::debug!("AX query threw an ObjC exception: {e:?}");
            None
        })
}

unsafe fn get_frontmost_window_title() -> Option<String> {
    ax_catch(|| unsafe {
        let pid = get_frontmost_pid()?;
        let ax_app = ax_create_application(pid);
        if ax_app.is_null() {
            return None;
        }
        let win = ax_copy_attribute(ax_app, "AXFocusedWindow");
        if win.is_null() {
            cf_release(ax_app);
            return None;
        }
        let title = ax_copy_string_attribute(win, "AXTitle");
        cf_release(win);
        cf_release(ax_app);
        title
    })
}

unsafe fn get_browser_url() -> Option<String> {
    ax_catch(|| unsafe {
        let pid = get_frontmost_pid()?;
        let ax_app = ax_create_application(pid);
        if ax_app.is_null() {
            return None;
        }
        let win = ax_copy_attribute(ax_app, "AXFocusedWindow");
        if win.is_null() {
            cf_release(ax_app);
            return None;
        }
        let url = search_ax_for_url(win, 5);
        cf_release(win);
        cf_release(ax_app);
        url
    })
}

unsafe fn ax_create_application(pid: i32) -> *mut Object {
    unsafe { AXUIElementCreateApplication(pid) }
}

unsafe fn ax_copy_attribute(element: *mut Object, attr: &str) -> *mut Object {
    unsafe {
        let attr_ns = ns_string(attr);
        let mut value: *mut Object = ptr::null_mut();
        let err = AXUIElementCopyAttributeValue(element, attr_ns as *mut Object, &mut value);
        if err != 0 { ptr::null_mut() } else { value }
    }
}

unsafe fn ax_copy_string_attribute(element: *mut Object, attr: &str) -> Option<String> {
    unsafe {
        let attr_ns = ns_string(attr);
        let mut value: *mut Object = ptr::null_mut();
        let err = AXUIElementCopyAttributeValue(element, attr_ns as *mut Object, &mut value);
        if err != 0 || value.is_null() {
            return None;
        }
        let result = nsstring_to_string(value);
        CFRelease(value as *mut c_void);
        result
    }
}

unsafe fn ax_copy_attribute_array(element: *mut Object, attr: &str) -> Vec<*mut Object> {
    unsafe {
        let attr_ns = ns_string(attr);
        let mut value: *mut Object = ptr::null_mut();
        let err = AXUIElementCopyAttributeValue(element, attr_ns as *mut Object, &mut value);
        if err != 0 || value.is_null() {
            return Vec::new();
        }
        let count: usize = msg_send![value, count];
        let mut items = Vec::with_capacity(count.min(50));
        for i in 0..count.min(50) {
            let item: *mut Object = msg_send![value, objectAtIndex: i];
            if !item.is_null() {
                // The items are owned by `value`; releasing the array below
                // would deallocate them and leave us walking freed pointers
                // (a use-after-free that traps deep in AXUIElementValidate).
                // Retain each so we own it; the caller releases when done.
                CFRetain(item as *const c_void);
                items.push(item);
            }
        }
        CFRelease(value as *mut c_void);
        items
    }
}

unsafe fn search_ax_for_url(element: *mut Object, max_depth: usize) -> Option<String> {
    unsafe {
        if max_depth == 0 {
            return None;
        }
        // Each child is a retained reference we own (see
        // ax_copy_attribute_array); release every one before returning.
        let children = ax_copy_attribute_array(element, "AXChildren");
        let mut found = None;
        for &child in &children {
            if found.is_none() {
                let role = ax_copy_string_attribute(child, "AXRole").unwrap_or_default();
                let desc = ax_copy_string_attribute(child, "AXDescription").unwrap_or_default();
                let rl = role.to_lowercase();
                let dl = desc.to_lowercase();

                if rl == "axtextfield" || rl == "axcombobox" || dl.contains("address") || dl.contains("url bar") {
                    if let Some(value) = ax_copy_string_attribute(child, "AXValue") {
                        if value.starts_with("http") || value.contains("://") {
                            found = Some(value);
                        }
                    }
                }
                if found.is_none() {
                    found = search_ax_for_url(child, max_depth - 1);
                }
            }
            cf_release(child);
        }
        found
    }
}

// ---------------------------------------------------------------------------
// ObjC helper functions
// ---------------------------------------------------------------------------

/// Create an NSString from a Rust &str.
unsafe fn ns_string(s: &str) -> *mut Object {
    unsafe {
        let ns: *mut Object = msg_send![class!(NSString), alloc];
        let bytes = s.as_ptr() as *const c_void;
        let len = s.len();
        msg_send![ns, initWithBytes: bytes length: len encoding: 4usize] // NSUTF8StringEncoding = 4
    }
}

/// Convert an NSString* to a Rust String.
unsafe fn nsstring_to_string(obj: *mut Object) -> Option<String> {
    unsafe {
        if obj.is_null() {
            return None;
        }
        let len: usize = msg_send![obj, lengthOfBytesUsingEncoding: 4usize];
        if len == 0 {
            return Some(String::new());
        }
        let mut buf = vec![0u8; len];
        let mut used: usize = 0;
        let ok: bool = msg_send![
            obj,
            getBytes: buf.as_mut_ptr() as *mut c_void
            maxLength: len
            encoding: 4usize
            usedLength: &mut used as *mut _
            lossyConversion: true
        ];
        if !ok { return None; }
        buf.truncate(used);
        Some(String::from_utf8_lossy(&buf).to_string())
    }
}

/// Create an NSAutoreleasePool.
unsafe fn ns_autorelease_pool_new() -> *mut Object {
    unsafe { msg_send![class!(NSAutoreleasePool), new] }
}

// Accessibility framework functions — AXUIElement is a CF type, not an ObjC class.
#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXIsProcessTrustedWithOptions(options: core_foundation_sys::dictionary::CFDictionaryRef) -> bool;
    fn AXUIElementCreateApplication(pid: i32) -> *mut Object;
    fn AXUIElementCopyAttributeValue(element: *mut Object, attribute: *mut Object, value: *mut *mut Object) -> i32;
    fn CFRelease(cf: *mut c_void);
    fn CFRetain(cf: *const c_void) -> *const c_void;
}

fn cf_release(obj: *mut Object) {
    if !obj.is_null() {
        unsafe { CFRelease(obj as *mut c_void) };
    }
}

// ---------------------------------------------------------------------------
// CFRunLoop Timer
// ---------------------------------------------------------------------------

fn create_timer(interval: f64, initial_fire: f64, callback: CFRunLoopTimerCallBack) -> CFRunLoopTimer {
    let fire_date = unsafe { CFAbsoluteTimeGetCurrent() } + initial_fire;
    let context = CFRunLoopTimerContext {
        version: 0,
        info: ptr::null_mut(),
        retain: None,
        release: None,
        copyDescription: None,
    };
    CFRunLoopTimer::new(fire_date, interval, 0, 0isize, callback, ptr::addr_of!(context) as *mut _)
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

fn is_transient_macos_app(app: &str) -> bool {
    let a = app.to_lowercase();
    matches!(
        a.as_str(),
        "dock" | "loginwindow" | "windowserver" | "controlcenter"
            | "notificationcenter" | "systemuiserver" | "spotlight" | "mission control"
    )
}

/// Ask the OS, not a trial screenshot: a probe capture can succeed during a
/// permission grace period and then throw an uncatchable ObjC exception on
/// a later capture. Preflight is authoritative; the request triggers the
/// System Settings prompt for first-run bundles.
fn check_screen_recording_permission() -> bool {
    if unsafe { CGPreflightScreenCaptureAccess() } {
        return true;
    }
    unsafe { CGRequestScreenCaptureAccess() }
}

/// Live Accessibility permission check — without it there are no window
/// titles or URLs, and grouping quality drops to app names.
pub fn accessibility_permission_now() -> bool {
    unsafe { ax_is_process_trusted() }
}

/// Ask macOS for Accessibility access, showing the system prompt that
/// deep-links to System Settings → Privacy → Accessibility. Plain
/// AXIsProcessTrusted() never prompts, so without this the app is invisible
/// to the user as something to grant. Returns the current grant state.
fn request_accessibility_permission() -> bool {
    use core_foundation::base::TCFType;
    use core_foundation::boolean::CFBoolean;
    use core_foundation::dictionary::CFDictionary;
    use core_foundation::string::CFString;
    // The literal underlying value of kAXTrustedCheckOptionPrompt.
    let key = CFString::from_static_string("AXTrustedCheckOptionPrompt");
    let opts = CFDictionary::from_CFType_pairs(&[(
        key.as_CFType(),
        CFBoolean::true_value().as_CFType(),
    )]);
    unsafe { AXIsProcessTrustedWithOptions(opts.as_concrete_TypeRef()) }
}

/// Cheap re-check, so revocation mid-session disables OCR/vision instead of
/// crashing: a capture without permission throws an uncatchable ObjC
/// exception. Callers must gate every capture_screenshot() on this.
pub fn screen_permission_now() -> bool {
    unsafe { CGPreflightScreenCaptureAccess() }
}

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
        assert!(is_transient_macos_app("dock"));
        assert!(is_transient_macos_app("DOCK"));
        assert!(!is_transient_macos_app("Google Chrome"));
        assert!(!is_transient_macos_app("Code"));
        assert!(!is_transient_macos_app("iTerm"));
        assert!(!is_transient_macos_app("Safari"));
    }

    #[test]
    fn test_idle_threshold() {
        assert_eq!(IDLE_THRESHOLD_SECS, 300.0);
    }
}
