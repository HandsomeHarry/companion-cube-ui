//! Windows OCR implementation using Windows.Media.Ocr (Win10+ built-in).
//! Screenshots via xcap crate (cross-platform window capture).

use anyhow::{Context, Result};

use super::OcrEngine;

#[derive(Default)]
pub struct WinOcrEngine;

impl WinOcrEngine {
    pub fn new() -> Self {
        Self
    }
}

impl OcrEngine for WinOcrEngine {
    fn extract_text(&self, image_data: &[u8]) -> Result<String> {
        ocr_from_png(image_data)
    }
}

/// Run OCR on PNG image bytes using Windows.Media.Ocr.
/// Uses a small tokio runtime to bridge Windows async APIs in a sync function.
fn ocr_from_png(image_data: &[u8]) -> Result<String> {
    use windows::Graphics::Imaging::BitmapDecoder;
    use windows::Media::Ocr::OcrEngine;
    use windows::Storage::Streams::{DataWriter, InMemoryRandomAccessStream};

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        // Write PNG bytes into an in-memory stream
        let stream = InMemoryRandomAccessStream::new()?;
        let writer = DataWriter::CreateDataWriter(&stream)?;
        writer.WriteBytes(image_data)?;
        writer.StoreAsync()?.await?;
        writer.DetachStream()?;
        drop(writer);

        // Rewind the stream
        stream.Seek(0)?;

        // Decode the image and run OCR
        let decoder = BitmapDecoder::CreateAsync(&stream)?.await?;
        let bitmap = decoder.GetSoftwareBitmapAsync()?.await?;
        let engine = OcrEngine::TryCreateFromUserProfileLanguages()
            .context("failed to create Windows OCR engine")?;
        let result = engine.RecognizeAsync(&bitmap)?.await?;
        Ok::<_, anyhow::Error>(result.Text()?.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ocr_empty_image() {
        // Minimal valid PNG: 1x1 transparent pixel
        let png = &[
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, // PNG header
            0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, // IHDR chunk
            0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, // 1x1
            0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4, 0x89, //
            0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, // IDAT chunk
            0x78, 0x9C, 0x62, 0x00, 0x00, 0x00, 0x02, 0x00, 0x01, //
            0xE2, 0x21, 0xBC, 0x33, 0x00, 0x00, 0x00, 0x00, // IEND
            0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82, //
        ];
        let engine = WinOcrEngine::new();
        let result = engine.extract_text(png);
        // Empty 1x1 image should return empty string
        assert!(result.is_ok());
    }
}
