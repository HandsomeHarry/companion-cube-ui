//! Integration test: the platform OCR engine must actually read text.
//! OCR on provided image bytes requires no permissions, so this runs
//! anywhere the framework exists.

#[cfg(target_os = "macos")]
#[test]
fn macos_vision_reads_rendered_text() {
    use ccube_capture::ocr::create_engine;
    let png = include_bytes!("fixtures/ocr-sample.png");
    let engine = create_engine().expect("macOS must have an OCR engine");
    let text = engine.extract_text(png).expect("OCR must not error");
    assert!(
        text.contains("Companion Cube 42"),
        "Vision should read the rendered text, got: {text:?}"
    );
}
