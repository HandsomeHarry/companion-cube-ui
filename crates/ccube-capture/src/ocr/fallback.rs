//! Noop OCR engine for unsupported platforms.

use anyhow::Result;

use super::OcrEngine;

pub struct NoopOcrEngine;

impl OcrEngine for NoopOcrEngine {
    fn extract_text(&self, _image_data: &[u8]) -> Result<String> {
        Ok(String::new())
    }
}
