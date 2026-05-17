//! Linux OCR stub — Tesseract/leptess not yet implemented.
//! Will use leptess crate behind optional feature flag.

use anyhow::Result;

use super::OcrEngine;

pub struct LinuxOcrEngine;

impl OcrEngine for LinuxOcrEngine {
    fn extract_text(&self, _image_data: &[u8]) -> Result<String> {
        Ok(String::new())
    }
}
