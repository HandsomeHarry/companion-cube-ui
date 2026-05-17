//! OCR (Optical Character Recognition) for extracting text from screenshots.
//!
//! Platform-native implementations extract text locally — raw images are
//! never stored on disk or transmitted to any LLM.

use anyhow::Result;

/// Extract text from a screenshot image buffer (PNG bytes).
/// Returns an empty string if no text is recognized.
pub trait OcrEngine: Send + Sync {
    fn extract_text(&self, image_data: &[u8]) -> Result<String>;
}

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use windows::WinOcrEngine;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::MacOcrEngine;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::LinuxOcrEngine;

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
mod fallback;
#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
pub use fallback::NoopOcrEngine;

/// Create the platform-appropriate OCR engine.
pub fn create_engine() -> Option<Box<dyn OcrEngine>> {
    #[cfg(target_os = "windows")]
    {
        Some(Box::new(WinOcrEngine::new()))
    }
    #[cfg(target_os = "macos")]
    {
        Some(Box::new(MacOcrEngine))
    }
    #[cfg(target_os = "linux")]
    {
        Some(Box::new(LinuxOcrEngine))
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        None
    }
}
