//! System tray icon, running on the main thread.
//!
//! macOS requires a GUI event loop on the main thread, so the daemon runs its
//! tokio runtime on a background thread (see `main.rs`) while this module owns
//! the main-thread `tao` event loop and the `tray-icon` `NSStatusItem`.
//!
//! Shutdown is unified: picking "Quit" cancels the shared `CancellationToken`;
//! the tokio thread observes it, performs cleanup, then sends
//! `UserEvent::Shutdown` back through the event-loop proxy so the process exits
//! only after durable state has been written.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoop, EventLoopBuilder};
use tokio_util::sync::CancellationToken;
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIconBuilder};

/// Focus state mirrored into the tray icon — the Aura concept at its smallest:
/// warm when focused, cool when drifting, gray when idle. Like Mem Reduct's
/// color-coded tray percentage, the icon itself is the status display.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayState {
    Focused,
    Drifting,
    Idle,
}

/// Events delivered to the main-thread event loop.
pub enum UserEvent {
    /// A tray menu item was activated.
    Menu(MenuEvent),
    /// Focus state changed; repaint the icon and tooltip.
    State(TrayState, String),
    /// The tokio runtime finished graceful shutdown; safe to exit the process.
    Shutdown,
}

/// Build the typed event loop. Must be called on the main thread *before* the
/// tokio runtime is spawned so a proxy can be handed to it.
pub fn build_event_loop() -> EventLoop<UserEvent> {
    #[allow(unused_mut)]
    let mut event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();

    // Without an app bundle the default activation policy suppresses menu-bar
    // UI — NSStatusItem creation "succeeds" but nothing appears. Accessory =
    // menu-bar presence without a Dock icon. Must be set before run().
    #[cfg(target_os = "macos")]
    {
        use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
        event_loop.set_activation_policy(ActivationPolicy::Accessory);
    }

    event_loop
}

/// Run the tray event loop on the main thread. Never returns; exits the process
/// once the tokio runtime signals [`UserEvent::Shutdown`].
pub fn run(
    event_loop: EventLoop<UserEvent>,
    cancel: CancellationToken,
    dashboard_url: String,
    snooze_until_ms: Arc<AtomicI64>,
) -> ! {
    // Forward global menu events into the typed event loop so it wakes up.
    let proxy = event_loop.create_proxy();
    MenuEvent::set_event_handler(Some(move |e| {
        let _ = proxy.send_event(UserEvent::Menu(e));
    }));

    let menu = Menu::new();
    let open_item = MenuItem::new("Open Dashboard", true, None);
    let snooze_5 = MenuItem::new("Snooze Nudges for 5 Minutes", true, None);
    let snooze_15 = MenuItem::new("Snooze Nudges for 15 Minutes", true, None);
    let snooze_30 = MenuItem::new("Snooze Nudges for 30 Minutes", true, None);
    let quit_item = MenuItem::new("Quit", true, None);
    if let Err(e) = menu.append_items(&[
        &open_item,
        &PredefinedMenuItem::separator(),
        &snooze_5,
        &snooze_15,
        &snooze_30,
        &PredefinedMenuItem::separator(),
        &quit_item,
    ]) {
        tracing::error!(error = %e, "failed to build tray menu");
    }
    let open_id = open_item.id().clone();
    let snooze_ids = [
        (snooze_5.id().clone(), 5i64),
        (snooze_15.id().clone(), 15i64),
        (snooze_30.id().clone(), 30i64),
    ];
    let quit_id = quit_item.id().clone();

    // The TrayIcon is not Send and must live on the main thread inside the loop.
    let mut tray = None;

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        match event {
            // The tray must be created after the loop is running (macOS).
            Event::NewEvents(StartCause::Init) => {
                match TrayIconBuilder::new()
                    .with_menu(Box::new(menu.clone()))
                    .with_tooltip("Companion Cube")
                    .with_icon(brand_icon())
                    .with_icon_as_template(true)
                    .build()
                {
                    Ok(t) => {
                        tracing::info!("tray icon created");
                        tray = Some(t);
                    }
                    Err(e) => tracing::error!(error = %e, "failed to create tray icon"),
                }

                // Wake the CFRunLoop so the icon renders immediately rather than
                // on the next event.
                #[cfg(target_os = "macos")]
                {
                    use objc2_core_foundation::CFRunLoop;
                    if let Some(rl) = CFRunLoop::main() {
                        rl.wake_up();
                    }
                }
            }
            Event::UserEvent(UserEvent::State(state, tooltip)) => {
                tracing::debug!(?state, %tooltip, "tray: state update received");
                if let Some(ref t) = tray {
                    let (icon, is_template) = state_icon(state);
                    if let Err(e) = t.set_icon(Some(icon)) {
                        tracing::warn!(error = %e, "failed to update tray icon");
                    }
                    // Template rendering must be re-asserted after set_icon.
                    t.set_icon_as_template(is_template);
                    if let Err(e) = t.set_tooltip(Some(&tooltip)) {
                        tracing::warn!(error = %e, "failed to update tray tooltip");
                    }
                }
            }
            Event::UserEvent(UserEvent::Menu(e)) => {
                if e.id == open_id {
                    open_dashboard(&dashboard_url);
                } else if e.id == quit_id {
                    tracing::info!("tray: Quit selected, initiating shutdown");
                    cancel.cancel();
                } else if let Some((_, mins)) =
                    snooze_ids.iter().find(|(id, _)| e.id == *id)
                {
                    let until = chrono::Utc::now().timestamp_millis() + mins * 60_000;
                    snooze_until_ms.store(until, Ordering::Relaxed);
                    tracing::info!(minutes = mins, "tray: nudges snoozed");
                }
            }
            Event::UserEvent(UserEvent::Shutdown) => {
                // tokio cleanup is done; drop the status item and exit cleanly.
                tray.take();
                *control_flow = ControlFlow::Exit;
            }
            _ => {}
        }
    })
}

/// Open the dashboard in the user's default browser.
fn open_dashboard(url: &str) {
    #[cfg(target_os = "macos")]
    let (program, args): (&str, &[&str]) = ("open", &[url]);
    #[cfg(target_os = "linux")]
    let (program, args): (&str, &[&str]) = ("xdg-open", &[url]);
    #[cfg(target_os = "windows")]
    let (program, args): (&str, &[&str]) = ("cmd", &["/C", "start", "", url]);

    match std::process::Command::new(program).args(args).spawn() {
        Ok(_) => tracing::info!(url, "opened dashboard in browser"),
        Err(e) => tracing::warn!(error = %e, "failed to open dashboard in browser"),
    }
}

/// Initial icon before the first state update arrives: unobtrusive template.
fn brand_icon() -> Icon {
    circle_icon([0x00, 0x00, 0x00], 0xFF)
}

/// Icon + template flag for a focus state.
///
/// Default (focused) is a *template* image: macOS draws it monochrome like
/// every other status item — unobtrusive per Dieter Rams; on modern macOS a
/// colored menu-bar item reads as an alert (mic/screen access). Color appears
/// only when the state carries information: cool blue while drifting (the
/// Aura palette), and idle dims to a faint template mark.
fn state_icon(state: TrayState) -> (Icon, bool) {
    match state {
        TrayState::Focused => (circle_icon([0x00, 0x00, 0x00], 0xFF), true),
        TrayState::Drifting => (circle_icon([0x4A, 0x90, 0xD8], 0xFF), false),
        TrayState::Idle => (circle_icon([0x00, 0x00, 0x00], 0x59), true),
    }
}

/// A 32x32 filled circle. For template icons macOS uses only the alpha
/// channel, so `alpha` controls how dimmed the mark renders.
fn circle_icon(rgb: [u8; 3], alpha: u8) -> Icon {
    const SIZE: u32 = 32;
    let mut rgba = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    let center = (SIZE as f32 - 1.0) / 2.0;
    let radius = SIZE as f32 / 2.0 - 1.0;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let dx = x as f32 - center;
            let dy = y as f32 - center;
            if dx * dx + dy * dy <= radius * radius {
                rgba.extend_from_slice(&[rgb[0], rgb[1], rgb[2], alpha]);
            } else {
                rgba.extend_from_slice(&[0, 0, 0, 0]);
            }
        }
    }
    Icon::from_rgba(rgba, SIZE, SIZE).expect("valid 32x32 RGBA tray icon")
}
