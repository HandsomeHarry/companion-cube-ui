// Windows activity capture — Phase 2 implementation.
// Uses SetWinEventHook, GetWindowTextW, UI Automation, GetLastInputInfo.

use crate::ActivityEvent;
use anyhow::Result;
use ccube_core::briefing::ActivitySnapshot;
use std::cell::RefCell;
use std::sync::atomic::{AtomicU32, Ordering};
use tokio::sync::mpsc;
use windows::core::Interface;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows::Win32::System::Threading::{
    GetCurrentThreadId, OpenProcess, PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION,
    QueryFullProcessImageNameW,
};
use windows::Win32::System::Variant::{VARIANT, VT_I4};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, HWINEVENTHOOK, IUIAutomation, IUIAutomationCondition, IUIAutomationElement,
    IUIAutomationValuePattern, SetWinEventHook, TreeScope, UIA_ControlTypePropertyId,
    UIA_EditControlTypeId, UIA_ValuePatternId, UnhookWinEvent,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, EVENT_SYSTEM_FOREGROUND, GetForegroundWindow, GetMessageW, GetWindowTextW,
    GetWindowThreadProcessId, KillTimer, MSG, PostThreadMessageW, SetTimer, TranslateMessage,
    WINEVENT_OUTOFCONTEXT, WM_QUIT,
};

/// Static thread ID for the Win32 message loop thread, used for shutdown signaling.
static WIN32_THREAD_ID: AtomicU32 = AtomicU32::new(0);

/// Idle threshold: 5 minutes in milliseconds.
const IDLE_THRESHOLD_MS: u32 = 300_000;

/// A poll-to-poll gap beyond this means the machine was asleep
/// (polls run every 5s; even heavy load won't stall one this long).
const SLEEP_GAP_MS: i64 = 60_000;

/// Poll interval: 5 seconds.
const POLL_INTERVAL_MS: u32 = 5_000;

/// Thread-local state accessible from Win32 callbacks.
struct CaptureState {
    tx: mpsc::Sender<ActivityEvent>,
    uia: Option<IUIAutomation>,
    last_app: String,
    last_title: String,
    last_url: Option<String>,
    idle_active: bool,
    /// Wall-clock of the previous idle poll — a large jump means the
    /// machine slept, which the idle check cannot see (timers freeze).
    last_poll_ms: i64,
}

thread_local! {
    static CAPTURE_STATE: RefCell<Option<CaptureState>> = const { RefCell::new(None) };
}

/// Windows activity capture implementation.
#[derive(Default)]
pub struct WinActivityCapture;

impl WinActivityCapture {
    pub fn new() -> Self {
        Self
    }
}

impl crate::ActivityCapture for WinActivityCapture {
    async fn subscribe(&self) -> mpsc::Receiver<ActivityEvent> {
        let (tx, rx) = mpsc::channel(4096);
        std::thread::spawn(move || {
            let thread_id = unsafe { GetCurrentThreadId() };
            WIN32_THREAD_ID.store(thread_id, Ordering::Release);
            win32_message_loop(tx);
        });
        // Give the Win32 thread a moment to start and register hooks
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        rx
    }

    async fn snapshot(&self) -> Result<ActivitySnapshot> {
        match active_win_pos_rs::get_active_window() {
            Ok(win) => {
                let app = win
                    .process_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown")
                    .to_string();
                Ok(ActivitySnapshot {
                    app,
                    title: if win.title.is_empty() {
                        None
                    } else {
                        Some(win.title)
                    },
                    url: None,
                    duration_ms: 0,
                })
            }
            Err(_) => Ok(ActivitySnapshot {
                app: "unknown".to_string(),
                title: None,
                url: None,
                duration_ms: 0,
            }),
        }
    }
}

/// Send a shutdown signal to the Win32 message loop thread.
pub fn request_shutdown() {
    let thread_id = WIN32_THREAD_ID.load(Ordering::Acquire);
    if thread_id != 0 {
        unsafe {
            let _ = PostThreadMessageW(thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
        }
    }
}

// ---------------------------------------------------------------------------
// Win32 message loop
// ---------------------------------------------------------------------------

fn win32_message_loop(tx: mpsc::Sender<ActivityEvent>) {
    // Initialize COM for UI Automation
    let mut com_initialized = false;
    let uia: Option<IUIAutomation> = unsafe {
        let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        if hr.is_err() {
            tracing::warn!("COM init failed (HRESULT {}), URL capture disabled", hr.0);
            None
        } else {
            com_initialized = true;
            match CoCreateInstance::<_, IUIAutomation>(&CUIAutomation, None, CLSCTX_ALL) {
                Ok(u) => Some(u),
                Err(e) => {
                    tracing::warn!("IUIAutomation creation failed: {e}, URL capture disabled");
                    None
                }
            }
        }
    };

    // Initialize thread-local state
    CAPTURE_STATE.with(|cell| {
        *cell.borrow_mut() = Some(CaptureState {
            tx,
            uia,
            last_app: String::new(),
            last_title: String::new(),
            last_url: None,
            idle_active: false,
            last_poll_ms: 0,
        });
    });

    // Install the WinEvent hook for foreground changes
    let hook = unsafe {
        SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            None,
            Some(win_event_proc),
            0,
            0,
            WINEVENT_OUTOFCONTEXT,
        )
    };
    if hook.is_invalid() {
        tracing::error!("SetWinEventHook failed — only timer-based polling will work");
    }

    // Install the 5-second polling timer
    let timer_id = unsafe { SetTimer(None, 1, POLL_INTERVAL_MS, Some(timer_proc)) };
    if timer_id == 0 {
        tracing::error!("SetTimer failed — no periodic polling");
    }

    // Fire an initial snapshot of the current foreground window
    poll_current_state();

    // Enter message loop
    tracing::info!("Win32 capture thread started");
    unsafe {
        let mut msg = MSG::default();
        loop {
            let ret = GetMessageW(&mut msg, None, 0, 0);
            if ret.0 == 0 {
                // WM_QUIT received
                break;
            }
            if ret.0 == -1 {
                // GetMessageW error — log and exit the loop
                tracing::error!("GetMessageW returned -1, exiting capture loop");
                break;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    // Cleanup
    if timer_id != 0 {
        let _ = unsafe { KillTimer(None, timer_id) };
    }
    if !hook.is_invalid() {
        let _ = unsafe { UnhookWinEvent(hook) };
    }
    // Drop UIA references before CoUninitialize
    CAPTURE_STATE.with(|cell| {
        *cell.borrow_mut() = None;
    });
    if com_initialized {
        unsafe { CoUninitialize() };
    }
    tracing::info!("Win32 capture thread exiting");
}

// ---------------------------------------------------------------------------
// WinEvent callback — fires on EVENT_SYSTEM_FOREGROUND
// ---------------------------------------------------------------------------

unsafe extern "system" fn win_event_proc(
    _hwineventhook: HWINEVENTHOOK,
    _event: u32,
    hwnd: HWND,
    _idobject: i32,
    _idchild: i32,
    _ideventthread: u32,
    _dwmseventtime: u32,
) {
    if hwnd.is_invalid() {
        return;
    }
    handle_focus_change(hwnd);
}

// ---------------------------------------------------------------------------
// Timer callback — fires every 5 seconds
// ---------------------------------------------------------------------------

unsafe extern "system" fn timer_proc(_hwnd: HWND, _msg: u32, _id: usize, _time: u32) {
    poll_current_state();
    check_idle();
}

// ---------------------------------------------------------------------------
// Core logic functions
// ---------------------------------------------------------------------------

/// Send an event via try_send, logging a warning if the channel is full.
fn try_send_event(tx: &mpsc::Sender<ActivityEvent>, event: ActivityEvent) {
    if let Err(e) = tx.try_send(event) {
        tracing::warn!("capture event dropped: {e}");
    }
}

fn handle_focus_change(hwnd: HWND) {
    let ts = now_ms();
    let title = get_window_title(hwnd);
    let app = get_process_name(hwnd);

    // Filter out transient shell windows (Alt+Tab switcher, desktop frame).
    if is_transient_shell_window(&app, &title) {
        return;
    }

    CAPTURE_STATE.with(|cell| {
        // Use try_borrow_mut to handle re-entrancy: COM/UIA calls inside this
        // closure can pump the Win32 message queue, which may fire another
        // win_event_proc callback on the same thread. If the RefCell is already
        // borrowed, we skip the re-entrant call — the timer will catch it.
        let Ok(mut borrow) = cell.try_borrow_mut() else {
            return;
        };
        let state = match borrow.as_mut() {
            Some(s) => s,
            None => return,
        };

        if app != state.last_app || title != state.last_title {
            try_send_event(
                &state.tx,
                ActivityEvent::AppFocusChanged {
                    app: app.clone(),
                    title: if title.is_empty() {
                        None
                    } else {
                        Some(title.clone())
                    },
                    ts,
                },
            );

            // Attempt URL capture for browsers
            let app_lower = app.to_lowercase();
            #[allow(clippy::collapsible_if)] // collapsing changes else-branch semantics
            if ccube_core::focus_mode::is_browser(&app_lower) {
                if let Some(ref uia) = state.uia {
                    let url = try_get_browser_url(uia, hwnd);
                    if url != state.last_url {
                        if let Some(ref u) = url {
                            try_send_event(
                                &state.tx,
                                ActivityEvent::UrlChanged { url: u.clone(), ts },
                            );
                        }
                        state.last_url = url;
                    }
                }
            } else {
                state.last_url = None;
            }

            state.last_app = app;
            state.last_title = title;
        }
    });
}

fn poll_current_state() {
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.is_invalid() {
        return;
    }

    let ts = now_ms();
    let title = get_window_title(hwnd);
    let app = get_process_name(hwnd);

    // Filter out transient shell windows (Alt+Tab switcher, desktop frame).
    if is_transient_shell_window(&app, &title) {
        return;
    }

    CAPTURE_STATE.with(|cell| {
        let Ok(mut borrow) = cell.try_borrow_mut() else {
            return;
        };
        let state = match borrow.as_mut() {
            Some(s) => s,
            None => return,
        };

        // If app changed, treat as a focus change we might have missed
        if app != state.last_app {
            try_send_event(
                &state.tx,
                ActivityEvent::AppFocusChanged {
                    app: app.clone(),
                    title: if title.is_empty() {
                        None
                    } else {
                        Some(title.clone())
                    },
                    ts,
                },
            );
            state.last_app = app.clone();
            state.last_title = title.clone();
        } else if title != state.last_title {
            // Same app, title changed
            try_send_event(
                &state.tx,
                ActivityEvent::WindowTitleChanged {
                    title: title.clone(),
                    ts,
                },
            );
            state.last_title = title.clone();
        }

        // URL poll for browsers
        let app_lower = app.to_lowercase();
        if ccube_core::focus_mode::is_browser(&app_lower)
            && let Some(ref uia) = state.uia
        {
            let url = try_get_browser_url(uia, hwnd);
            if url != state.last_url {
                if let Some(ref u) = url {
                    try_send_event(&state.tx, ActivityEvent::UrlChanged { url: u.clone(), ts });
                }
                state.last_url = url;
            }
        }
    });
}

fn check_idle() {
    let ts = now_ms();
    let mut info = LASTINPUTINFO {
        cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };

    let ok = unsafe { GetLastInputInfo(&mut info) };
    if !ok.as_bool() {
        return;
    }

    let tick_count = unsafe { windows::Win32::System::SystemInformation::GetTickCount() };
    let idle_ms = tick_count.wrapping_sub(info.dwTime);

    CAPTURE_STATE.with(|cell| {
        let Ok(mut borrow) = cell.try_borrow_mut() else {
            return;
        };
        let state = match borrow.as_mut() {
            Some(s) => s,
            None => return,
        };

        // Sleep detection: timers freeze during system sleep, so the idle
        // check never fires across a suspend — emit a synthetic away period
        // covering the wall-clock jump.
        if state.last_poll_ms > 0 && !state.idle_active {
            let gap = ts - state.last_poll_ms;
            if gap > SLEEP_GAP_MS {
                try_send_event(&state.tx, ActivityEvent::IdleStart { ts: state.last_poll_ms });
                try_send_event(&state.tx, ActivityEvent::IdleEnd { ts });
                state.last_app.clear(); // re-emit focus, fresh clock
            }
        }
        state.last_poll_ms = ts;

        if idle_ms >= IDLE_THRESHOLD_MS && !state.idle_active {
            // The user left when input stopped, not when we noticed.
            let onset_ts = ts - idle_ms as i64;
            try_send_event(&state.tx, ActivityEvent::IdleStart { ts: onset_ts });
            state.idle_active = true;
        } else if idle_ms < IDLE_THRESHOLD_MS && state.idle_active {
            try_send_event(&state.tx, ActivityEvent::IdleEnd { ts });
            state.idle_active = false;
        }
    });
}

// ---------------------------------------------------------------------------
// Win32 helper functions
// ---------------------------------------------------------------------------

/// Returns true for transient shell windows that should be filtered out:
/// explorer.exe with no title (desktop frame) or the Alt+Tab task switcher
/// (whose title varies by locale: "Task Switching", "任务切换", etc.).
fn is_transient_shell_window(app: &str, title: &str) -> bool {
    let app_lower = app.to_lowercase();
    if app_lower == "explorer.exe" {
        // Desktop frame has no title; the Alt+Tab switcher title is locale-
        // dependent so we check for known strings across major locales.
        return title.is_empty()
            || title == "\u{4efb}\u{52a1}\u{5207}\u{6362}" // Chinese 任务切换
            || title.eq_ignore_ascii_case("Task Switching")
            || title.eq_ignore_ascii_case("Programmumschalter")  // German
            || title.eq_ignore_ascii_case("Cambio de tareas")    // Spanish
            || title.eq_ignore_ascii_case("Changement de t\u{00e2}ches"); // French
    }
    // ShellExperienceHost.exe handles notification popups and other transient UI
    if app_lower == "shellexperiencehost.exe" {
        return true;
    }
    false
}

fn get_window_title(hwnd: HWND) -> String {
    let mut buf = [0u16; 512];
    let len = unsafe { GetWindowTextW(hwnd, &mut buf) };
    if len <= 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buf[..len as usize])
}

fn get_process_name(hwnd: HWND) -> String {
    unsafe {
        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return "unknown".to_string();
        }

        let handle = match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
            Ok(h) => h,
            Err(_) => return "unknown".to_string(),
        };

        let mut buf = [0u16; 260];
        let mut size = buf.len() as u32;
        let result = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_FORMAT(0),
            windows::core::PWSTR(buf.as_mut_ptr()),
            &mut size,
        );
        let _ = windows::Win32::Foundation::CloseHandle(handle);

        match result {
            Ok(()) => {
                let path = String::from_utf16_lossy(&buf[..size as usize]);
                // Extract just the filename from the full path
                path.rsplit('\\').next().unwrap_or("unknown").to_string()
            }
            Err(_) => "unknown".to_string(),
        }
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

// ---------------------------------------------------------------------------
// UI Automation URL capture
// ---------------------------------------------------------------------------

fn try_get_browser_url(uia: &IUIAutomation, hwnd: HWND) -> Option<String> {
    try_get_browser_url_inner(uia, hwnd).unwrap_or_else(|e| {
        tracing::debug!("URL capture failed: {e}");
        None
    })
}

fn try_get_browser_url_inner(
    uia: &IUIAutomation,
    hwnd: HWND,
) -> std::result::Result<Option<String>, Box<dyn std::error::Error>> {
    unsafe {
        // Get root element for the window
        let root: IUIAutomationElement = uia.ElementFromHandle(hwnd)?;

        // Build a VARIANT with VT_I4 = UIA_EditControlTypeId (50004)
        let variant: VARIANT = {
            use windows::Win32::System::Variant::{VARIANT_0, VARIANT_0_0, VARIANT_0_0_0};
            let mut v = VARIANT::default();
            let inner = VARIANT_0_0 {
                vt: VT_I4,
                wReserved1: 0,
                wReserved2: 0,
                wReserved3: 0,
                Anonymous: VARIANT_0_0_0 {
                    lVal: UIA_EditControlTypeId.0,
                },
            };
            v.Anonymous = VARIANT_0 {
                Anonymous: std::mem::ManuallyDrop::new(inner),
            };
            v
        };

        // Create a property condition: ControlType == Edit
        let condition: IUIAutomationCondition =
            uia.CreatePropertyCondition(UIA_ControlTypePropertyId, &variant)?;

        // Find the first Edit control (the address bar).
        // FindFirst may return S_OK with a null pointer if no element matches
        // (e.g., fullscreen browser, non-Chromium UI tree). We must check.
        let edit: IUIAutomationElement = root.FindFirst(TreeScope(4), &condition)?; // TreeScope_Descendants = 4
        if edit.as_raw().is_null() {
            return Ok(None);
        }

        // Get the ValuePattern from the edit control
        let pattern: IUIAutomationValuePattern =
            edit.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)?;

        // Read the current value (the URL)
        let value = pattern.CurrentValue()?;
        let url = value.to_string();

        if url.is_empty() {
            Ok(None)
        } else {
            Ok(Some(url))
        }
    }
}
