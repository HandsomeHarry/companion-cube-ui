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

use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoop, EventLoopBuilder};
use tokio_util::sync::CancellationToken;
use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{Icon, TrayIconBuilder};

/// The dashboard URL the tray opens in the default browser.
const DASHBOARD_URL: &str = "http://localhost:7431";

/// Events delivered to the main-thread event loop.
pub enum UserEvent {
    /// A tray menu item was activated.
    Menu(MenuEvent),
    /// The tokio runtime finished graceful shutdown; safe to exit the process.
    Shutdown,
}

/// Build the typed event loop. Must be called on the main thread *before* the
/// tokio runtime is spawned so a proxy can be handed to it.
pub fn build_event_loop() -> EventLoop<UserEvent> {
    EventLoopBuilder::<UserEvent>::with_user_event().build()
}

/// Run the tray event loop on the main thread. Never returns; exits the process
/// once the tokio runtime signals [`UserEvent::Shutdown`].
pub fn run(event_loop: EventLoop<UserEvent>, cancel: CancellationToken) -> ! {
    // Forward global menu events into the typed event loop so it wakes up.
    let proxy = event_loop.create_proxy();
    MenuEvent::set_event_handler(Some(move |e| {
        let _ = proxy.send_event(UserEvent::Menu(e));
    }));

    let menu = Menu::new();
    let open_item = MenuItem::new("Open Dashboard", true, None);
    let quit_item = MenuItem::new("Quit", true, None);
    if let Err(e) = menu.append_items(&[&open_item, &quit_item]) {
        tracing::error!(error = %e, "failed to build tray menu");
    }
    let open_id = open_item.id().clone();
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
                    .build()
                {
                    Ok(t) => tray = Some(t),
                    Err(e) => tracing::error!(error = %e, "failed to create tray icon"),
                }

                // Wake the CFRunLoop so the icon renders immediately rather than
                // on the next event.
                #[cfg(target_os = "macos")]
                unsafe {
                    use objc2_core_foundation::{CFRunLoopGetMain, CFRunLoopWakeUp};
                    if let Some(rl) = CFRunLoopGetMain() {
                        CFRunLoopWakeUp(&rl);
                    }
                }
            }
            Event::UserEvent(UserEvent::Menu(e)) => {
                if e.id == open_id {
                    open_dashboard();
                } else if e.id == quit_id {
                    tracing::info!("tray: Quit selected, initiating shutdown");
                    cancel.cancel();
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
fn open_dashboard() {
    #[cfg(target_os = "macos")]
    let (program, args): (&str, &[&str]) = ("open", &[DASHBOARD_URL]);
    #[cfg(target_os = "linux")]
    let (program, args): (&str, &[&str]) = ("xdg-open", &[DASHBOARD_URL]);
    #[cfg(target_os = "windows")]
    let (program, args): (&str, &[&str]) = ("cmd", &["/C", "start", "", DASHBOARD_URL]);

    match std::process::Command::new(program).args(args).spawn() {
        Ok(_) => tracing::info!(url = DASHBOARD_URL, "opened dashboard in browser"),
        Err(e) => tracing::warn!(error = %e, "failed to open dashboard in browser"),
    }
}

/// A 32x32 burnt-orange filled circle matching the ccube brand (`#F16A01`).
fn brand_icon() -> Icon {
    const SIZE: u32 = 32;
    let mut rgba = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    let center = (SIZE as f32 - 1.0) / 2.0;
    let radius = SIZE as f32 / 2.0 - 1.0;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let dx = x as f32 - center;
            let dy = y as f32 - center;
            if dx * dx + dy * dy <= radius * radius {
                rgba.extend_from_slice(&[0xF1, 0x6A, 0x01, 0xFF]);
            } else {
                rgba.extend_from_slice(&[0, 0, 0, 0]);
            }
        }
    }
    Icon::from_rgba(rgba, SIZE, SIZE).expect("valid 32x32 RGBA tray icon")
}
