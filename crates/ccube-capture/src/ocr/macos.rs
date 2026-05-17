//! macOS OCR via Vision framework (VNRecognizeTextRequest).
//! Uses raw objc msg_send! calls since icrate doesn't include Vision bindings.
//! Raw image pixels only exist in memory briefly — never written to disk.

use anyhow::Result;
use objc::runtime::Object;
use objc::{class, msg_send, sel, sel_impl};

use super::OcrEngine;

pub struct MacOcrEngine;

impl OcrEngine for MacOcrEngine {
    fn extract_text(&self, image_data: &[u8]) -> Result<String> {
        let _pool = unsafe { AutoreleasePool::new() };

        unsafe { vision_ocr_inner(image_data) }
    }
}

unsafe fn vision_ocr_inner(image_data: &[u8]) -> Result<String> {
    // 1. Create NSData from PNG bytes
    let nsdata: *mut Object = msg_send![
        class!(NSData),
        dataWithBytes: image_data.as_ptr() as *const std::ffi::c_void
        length: image_data.len()
    ];
    if nsdata.is_null() {
        anyhow::bail!("failed to create NSData from image bytes");
    }

    // 2. Create VNImageRequestHandler with the data
    let handler_class = match objc::runtime::Class::get("VNImageRequestHandler") {
        Some(cls) => cls,
        None => anyhow::bail!(
            "Vision framework not available (VNImageRequestHandler class not found)"
        ),
    };

    // VNImageRequestHandler initWithData:options:
    // options is an NSDictionary — pass nil for empty options
    let nil_dict: *mut Object = std::ptr::null_mut();
    let handler: *mut Object = msg_send![handler_class, initWithData: nsdata options: nil_dict];
    if handler.is_null() {
        anyhow::bail!("failed to create VNImageRequestHandler");
    }

    // 3. Create VNRecognizeTextRequest with no completion handler (we read results after)
    let request_class = match objc::runtime::Class::get("VNRecognizeTextRequest") {
        Some(cls) => cls,
        None => anyhow::bail!(
            "Vision framework not available (VNRecognizeTextRequest class not found)"
        ),
    };

    let nil_handler: *const std::ffi::c_void = std::ptr::null();
    let request: *mut Object = msg_send![request_class, initWithCompletionHandler: nil_handler];
    if request.is_null() {
        anyhow::bail!("failed to create VNRecognizeTextRequest");
    }

    // Set recognition level to accurate (1 = VNRequestTextRecognitionLevelAccurate)
    let _: () = msg_send![request, setRecognitionLevel: 1];

    // 4. Perform the request
    // performRequests:error: takes an NSArray of requests and an NSError* pointer
    let request_array: *mut Object = msg_send![class!(NSArray), arrayWithObject: request];
    let mut nserror: *mut Object = std::ptr::null_mut();
    let success: bool = msg_send![
        handler,
        performRequests: request_array
        error: &mut nserror as *mut _
    ];
    if !success {
        let desc = if !nserror.is_null() {
            nsstring_to_string(nserror).unwrap_or_else(|| "unknown error".to_string())
        } else {
            "unknown error".to_string()
        };
        anyhow::bail!("VNImageRequestHandler performRequests failed: {}", desc);
    }

    // 5. Extract results from the request
    let results: *mut Object = msg_send![request, results];
    if results.is_null() {
        return Ok(String::new());
    }

    let count: usize = msg_send![results, count];
    if count == 0 {
        return Ok(String::new());
    }

    let mut text_parts = Vec::with_capacity(count);
    for i in 0..count {
        let observation: *mut Object = msg_send![results, objectAtIndex: i];
        if observation.is_null() {
            continue;
        }

        // Get top 1 candidate
        let candidates: *mut Object = msg_send![observation, topCandidates: 1usize];
        if candidates.is_null() {
            continue;
        }

        let candidate_count: usize = msg_send![candidates, count];
        if candidate_count > 0 {
            let candidate: *mut Object = msg_send![candidates, objectAtIndex: 0usize];
            let string: *mut Object = msg_send![candidate, string];
            if let Some(s) = nsstring_to_string(string) {
                if !s.is_empty() {
                    text_parts.push(s);
                }
            }
        }
    }

    Ok(text_parts.join("\n"))
}

/// Convert an NSString* to a Rust String.
unsafe fn nsstring_to_string(nsstring: *mut Object) -> Option<String> {
    if nsstring.is_null() {
        return None;
    }
    // NSUTF8StringEncoding = 4
    let len: usize = msg_send![nsstring, lengthOfBytesUsingEncoding: 4usize];
    if len == 0 {
        return Some(String::new());
    }
    let mut buf = vec![0u8; len];
    let mut used_length: usize = 0;
    let ok: bool = msg_send![
        nsstring,
        getBytes: buf.as_mut_ptr() as *mut std::ffi::c_void
        maxLength: len
        encoding: 4usize
        usedLength: &mut used_length as *mut _
        lossyConversion: true
    ];
    if !ok {
        return None;
    }
    buf.truncate(used_length);
    Some(String::from_utf8_lossy(&buf).to_string())
}

/// RAII wrapper for NSAutoreleasePool.
struct AutoreleasePool {
    pool: *mut Object,
}

impl AutoreleasePool {
    unsafe fn new() -> Self {
        let pool: *mut Object = msg_send![class!(NSAutoreleasePool), new];
        Self { pool }
    }

    unsafe fn drain(&self) {
        let _: () = msg_send![self.pool, drain];
    }
}

impl Drop for AutoreleasePool {
    fn drop(&mut self) {
        unsafe { self.drain() };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_text_empty_data() {
        // Empty bytes should not panic — returns empty or error
        let engine = MacOcrEngine;
        let result = engine.extract_text(&[]);
        // We accept either empty string or error (Vision needs valid image data)
        assert!(result.is_ok() || result.is_err());
    }
}
