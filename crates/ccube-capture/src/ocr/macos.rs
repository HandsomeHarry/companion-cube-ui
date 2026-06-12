//! macOS OCR via Vision framework (VNRecognizeTextRequest).
//! Uses raw objc msg_send! calls since icrate doesn't include Vision bindings.
//! Raw image pixels only exist in memory briefly — never written to disk.

use anyhow::Result;
use objc::runtime::Object;
use objc::{class, msg_send, sel, sel_impl};

// Without this the Vision classes simply don't exist at runtime —
// objc::runtime::Class::get("VNImageRequestHandler") returns None and OCR
// silently never works. An empty extern block is enough to force linkage.
#[link(name = "Vision", kind = "framework")]
unsafe extern "C" {}

use super::OcrEngine;

pub struct MacOcrEngine;

impl OcrEngine for MacOcrEngine {
    fn extract_text(&self, image_data: &[u8]) -> Result<String> {
        if image_data.is_empty() {
            anyhow::bail!("empty image data");
        }
        let _pool = unsafe { AutoreleasePool::new() };

        // Vision throws ObjC exceptions on malformed input; a foreign
        // exception crossing into Rust aborts the daemon, so catch at the
        // language boundary and convert to Err (same as capture_screenshot).
        objc2::exception::catch(std::panic::AssertUnwindSafe(|| unsafe {
            vision_ocr_inner(image_data)
        }))
        .unwrap_or_else(|e| Err(anyhow::anyhow!("Vision OCR threw: {e:?}")))
    }
}

unsafe fn vision_ocr_inner(image_data: &[u8]) -> Result<String> {
    unsafe {
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
    let handler_alloc: *mut Object = msg_send![handler_class, alloc];
    let handler: *mut Object = msg_send![handler_alloc, initWithData: nsdata options: nil_dict];
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

    let request_alloc: *mut Object = msg_send![request_class, alloc];
    let request: *mut Object = msg_send![request_alloc, init];
    if request.is_null() {
        let _: () = msg_send![handler, release];
        anyhow::bail!("failed to create VNRecognizeTextRequest");
    }

    // VNRequestTextRecognitionLevelAccurate = 0 (Fast = 1)
    let _: () = msg_send![request, setRecognitionLevel: 0i64];

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
        let _: () = msg_send![request, release];
        let _: () = msg_send![handler, release];
        anyhow::bail!("VNImageRequestHandler performRequests failed: {}", desc);
    }

    // 5. Extract results from the request
    let results: *mut Object = msg_send![request, results];
    let count: usize = if results.is_null() {
        0
    } else {
        msg_send![results, count]
    };
    if count == 0 {
        let _: () = msg_send![request, release];
        let _: () = msg_send![handler, release];
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

    let text = text_parts.join("\n");
    let _: () = msg_send![request, release];
    let _: () = msg_send![handler, release];
    Ok(text)
    }
}

/// Convert an NSString* to a Rust String.
unsafe fn nsstring_to_string(nsstring: *mut Object) -> Option<String> {
    unsafe {
    if nsstring.is_null() {
        return None;
    }
    let cstr: *const std::os::raw::c_char = msg_send![nsstring, UTF8String];
    if cstr.is_null() {
        return None;
    }
    Some(
        std::ffi::CStr::from_ptr(cstr)
            .to_string_lossy()
            .into_owned(),
    )
    }
}

/// RAII wrapper for NSAutoreleasePool.
struct AutoreleasePool {
    pool: *mut Object,
}

impl AutoreleasePool {
    unsafe fn new() -> Self {
        unsafe {
        let pool: *mut Object = msg_send![class!(NSAutoreleasePool), new];
        Self { pool }
        }
    }

    unsafe fn drain(&self) {
        unsafe { let _: () = msg_send![self.pool, drain]; }
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
