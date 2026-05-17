//! macOS OCR stub — Vision framework not yet implemented.
//! Will use VNRecognizeTextRequest + VNImageRequestHandler via objc.

use anyhow::Result;

use super::OcrEngine;

pub struct MacOcrEngine;

impl OcrEngine for MacOcrEngine {
    fn extract_text(&self, _image_data: &[u8]) -> Result<String> {
        Ok(String::new())
    }
}
