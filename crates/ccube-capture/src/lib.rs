#![allow(clippy::missing_const_for_thread_local)]

use anyhow::{Context, Result};
use ccube_core::briefing::ActivitySnapshot;

/// Capture events emitted by the platform layer.
pub enum ActivityEvent {
    AppFocusChanged {
        app: String,
        title: Option<String>,
        ts: i64,
    },
    WindowTitleChanged {
        title: String,
        ts: i64,
    },
    UrlChanged {
        url: String,
        ts: i64,
    },
    IdleStart {
        ts: i64,
    },
    IdleEnd {
        ts: i64,
    },
    OcrReady {
        text: String,
        ts: i64,
    },
}

/// Platform-agnostic activity capture trait.
#[allow(async_fn_in_trait)]
pub trait ActivityCapture: Send + Sync {
    async fn subscribe(&self) -> tokio::sync::mpsc::Receiver<ActivityEvent>;
    async fn snapshot(&self) -> Result<ActivitySnapshot>;
}

#[cfg(target_os = "windows")]
pub mod windows;

#[cfg(target_os = "macos")]
pub mod macos;

#[cfg(target_os = "linux")]
pub mod linux;

pub mod ocr;

/// Capture a screenshot of the primary monitor as PNG bytes.
/// Uses xcap under the hood (cross-platform: Windows/macOS/Linux).
/// Returns an error if no monitors are available (headless system).
pub fn capture_screenshot() -> Result<Vec<u8>> {
    // macOS: the capture APIs throw NSExceptions in permission edge cases
    // (notably: permission granted while this process was already running).
    // A foreign exception crossing into Rust aborts the whole daemon, so it
    // must be caught at the ObjC level and converted into an Err.
    #[cfg(target_os = "macos")]
    {
        objc2::exception::catch(capture_screenshot_inner)
            .unwrap_or_else(|e| Err(anyhow::anyhow!("screen capture threw: {e:?}")))
    }
    #[cfg(not(target_os = "macos"))]
    {
        capture_screenshot_inner()
    }
}

fn capture_screenshot_inner() -> Result<Vec<u8>> {
    use xcap::Monitor;

    let monitors = Monitor::all()?;
    let primary = monitors
        .into_iter()
        .find(|m| m.is_primary())
        .or_else(|| Monitor::all().ok()?.into_iter().next())
        .context("no monitors found or available for screenshot")?;

    let image = primary.capture_image()?;
    let mut buf = std::io::Cursor::new(Vec::new());
    image.write_to(&mut buf, image::ImageFormat::Png)?;
    Ok(buf.into_inner())
}
