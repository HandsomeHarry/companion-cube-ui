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
    has_ax_permission: bool,
    has_screen_permission: bool,
    ax_warned: bool,
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

    let title = if CAPTURE_STATE.with(|c| c.borrow().as_ref().map(|s| s.has_ax_permission).unwrap_or(false)) {
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
    poll_title_url_and_idle();
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

        if app != state.last_app {
            let title = if state.has_ax_permission {
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
        } else if state.has_ax_permission {
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

        // Idle check
        let idle_secs = unsafe { get_idle_seconds() };
        if idle_secs >= IDLE_THRESHOLD_SECS && !state.idle_active {
            try_send_event(&state.tx, ActivityEvent::IdleStart { ts });
            state.idle_active = true;
        } else if idle_secs < IDLE_THRESHOLD_SECS && state.idle_active {
            try_send_event(&state.tx, ActivityEvent::IdleEnd { ts });
            state.idle_active = false;
        }
    });
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

    let should = CAPTURE_STATE.with(|c| c.borrow().as_ref().map(|s| s.has_screen_permission).unwrap_or(false));
    if !should {
        return;
    }

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

unsafe fn get_frontmost_window_title() -> Option<String> {
    unsafe {
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
    }
}

unsafe fn get_browser_url() -> Option<String> {
    unsafe {
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
    }
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
        let children = ax_copy_attribute_array(element, "AXChildren");
        for child in &children {
            let role = ax_copy_string_attribute(*child, "AXRole").unwrap_or_default();
            let desc = ax_copy_string_attribute(*child, "AXDescription").unwrap_or_default();
            let rl = role.to_lowercase();
            let dl = desc.to_lowercase();

            if rl == "axtextfield" || rl == "axcombobox" || dl.contains("address") || dl.contains("url bar") {
                if let Some(value) = ax_copy_string_attribute(*child, "AXValue") {
                    if value.starts_with("http") || value.contains("://") {
                        return Some(value);
                    }
                }
            }
            if let Some(url) = search_ax_for_url(*child, max_depth - 1) {
                return Some(url);
            }
        }
        None
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
    fn AXUIElementCreateApplication(pid: i32) -> *mut Object;
    fn AXUIElementCopyAttributeValue(element: *mut Object, attribute: *mut Object, value: *mut *mut Object) -> i32;
    fn CFRelease(cf: *mut c_void);
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

fn check_screen_recording_permission() -> bool {
    match crate::capture_screenshot() {
        Ok(data) if !data.is_empty() => true,
        _ => false,
    }
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
