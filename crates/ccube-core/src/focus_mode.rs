use crate::briefing::FocusMode;

/// Tier-1 focus mode inference via keyword matching.
///
/// Checks app name, window title, URL, and OCR screen text to determine what the user
/// is likely doing. Returns `Unspecified` when no pattern matches.
pub fn infer_focus_mode(app: &str, title: Option<&str>, url: Option<&str>, ocr_text: Option<&str>) -> FocusMode {
    let app_lower = app.to_lowercase();

    // URL-based inference (highest priority when available)
    if let Some(u) = url {
        let u_lower = u.to_lowercase();
        if u_lower.contains("docs.google.com")
            || u_lower.contains("notion.so")
            || u_lower.contains("medium.com/p/")
        {
            return FocusMode::Writing;
        }
        if u_lower.contains("github.com") || u_lower.contains("gitlab.com") {
            return FocusMode::Coding;
        }
        if u_lower.contains("stackoverflow.com") || u_lower.contains("crates.io") {
            return FocusMode::Coding;
        }
    }

    // Video production apps
    if app_lower.contains("davinci")
        || app_lower.contains("resolve")
        || app_lower.contains("premiere")
        || app_lower.contains("after effects")
        || app_lower.contains("afterfx")
        || app_lower.contains("final cut")
        || app_lower.contains("kdenlive")
        || app_lower == "obs64.exe"
        || app_lower == "obs32.exe"
        || app_lower == "obs.exe"
        || app_lower.contains("obs studio")
    {
        return FocusMode::VideoProduction;
    }

    // Writing apps
    if app_lower.contains("winword")
        || app_lower.contains("word")
            && !app_lower.contains("code")
            && !app_lower.contains("wordpad")
        || app_lower.contains("notion")
        || app_lower.contains("obsidian")
        || app_lower.contains("typora")
        || app_lower.contains("scrivener")
    {
        return FocusMode::Writing;
    }

    // IDE / code editor apps (always Coding regardless of title)
    if app_lower.contains("intellij")
        || app_lower.contains("idea64")
        || app_lower.contains("idea.exe")
        || app_lower.contains("pycharm")
        || app_lower.contains("webstorm")
        || app_lower.contains("clion")
        || app_lower.contains("rustrover")
        || app_lower.contains("rider")
        || app_lower.contains("goland")
        || app_lower.contains("android studio")
        || app_lower.contains("sublime")
        || app_lower.contains("neovim")
        || app_lower.contains("nvim")
        || app_lower.contains("vim") && !app_lower.contains("preview")
        || app_lower.contains("emacs")
    {
        return FocusMode::Coding;
    }

    // VS Code — check title for file extensions to distinguish coding from writing
    if app_lower.contains("code") && !app_lower.contains("codex") {
        if let Some(t) = title {
            if has_code_extension(t) {
                return FocusMode::Coding;
            }
            if has_writing_extension(t) {
                return FocusMode::Writing;
            }
        }
        // Default for VS Code without recognizable extension
        return FocusMode::Coding;
    }

    // Terminal apps — likely coding
    if app_lower.contains("windowsterminal")
        || app_lower.contains("wt.exe")
        || app_lower.contains("powershell")
        || app_lower.contains("cmd.exe")
        || app_lower.contains("mintty")
        || app_lower.contains("alacritty")
        || app_lower.contains("wezterm")
    {
        return FocusMode::Coding;
    }

    // Browser — check title for clues
    if is_browser(&app_lower)
        && let Some(t) = title
    {
        let t_lower = t.to_lowercase();
        if t_lower.contains("github")
            || t_lower.contains("gitlab")
            || t_lower.contains("stack overflow")
            || t_lower.contains("stackoverflow")
            || t_lower.contains("crates.io")
            || t_lower.contains("docs.rs")
            || t_lower.contains("mdn web docs")
        {
            return FocusMode::Coding;
        }
        if t_lower.contains("google docs")
            || t_lower.contains("notion")
            || t_lower.contains("medium")
        {
            return FocusMode::Writing;
        }
    }

    // OCR screen text — use visible content to refine when app/title/url didn't match
    if let Some(ocr) = ocr_text {
        let ocr_lower = ocr.to_lowercase();

        // Code indicators in screen content
        let code_signals = ["fn ", "impl ", "pub fn", "async fn", "import ", "from ",
            "class ", "def ", "return ", "const ", "let mut", "-> ", "=> ",
            "cargo build", "cargo test", "npm run", "git commit", "git push",
            "error[", "warning[", "debug!", "traceback",
            "terminal", "console", "println!", "console.log"];
        if code_signals.iter().any(|s| ocr_lower.contains(s)) {
            return FocusMode::Coding;
        }

        // Writing / document indicators
        let writing_signals = ["chapter", "paragraph", "bibliography", "abstract",
            "introduction", "conclusion", "references", "table of contents",
            "word count", "page count", "heading 1", "heading 2"];
        if writing_signals.iter().any(|s| ocr_lower.contains(s)) {
            return FocusMode::Writing;
        }
    }

    FocusMode::Unspecified
}

/// Check if an app name corresponds to a known browser.
pub fn is_browser(app_lower: &str) -> bool {
    app_lower.contains("chrome")
        || app_lower.contains("msedge")
        || app_lower.contains("firefox")
        || app_lower.contains("brave")
        || app_lower.contains("vivaldi")
        || app_lower == "arc.exe"
        || app_lower.contains("opera") && app_lower.contains("browser")
        || app_lower == "opera.exe"
}

fn has_code_extension(title: &str) -> bool {
    let code_exts = [
        ".rs", ".py", ".js", ".ts", ".jsx", ".tsx", ".go", ".java", ".c", ".cpp", ".h", ".hpp",
        ".cs", ".rb", ".php", ".swift", ".kt", ".scala", ".zig", ".html", ".css", ".scss", ".vue",
        ".svelte", ".toml", ".yaml", ".yml", ".json", ".xml", ".sql", ".sh", ".bash", ".ps1",
        ".lua", ".r", ".dart", ".ex", ".exs", ".hs",
    ];
    // Extract first token from title (often the filename) and check suffix
    let first_token = title.split_whitespace().next().unwrap_or(title);
    code_exts.iter().any(|ext| first_token.ends_with(ext))
}

fn has_writing_extension(title: &str) -> bool {
    let writing_exts = [".md", ".txt", ".doc", ".docx", ".rtf", ".tex", ".org"];
    let first_token = title.split_whitespace().next().unwrap_or(title);
    writing_exts.iter().any(|ext| first_token.ends_with(ext))
}

/// Convert a FocusMode to a string suitable for the events table `mode` column.
pub fn focus_mode_to_str(mode: &FocusMode) -> &'static str {
    match mode {
        FocusMode::Coding => "Coding",
        FocusMode::Writing => "Writing",
        FocusMode::VideoProduction => "VideoProduction",
        FocusMode::Unspecified => "Unspecified",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vscode_with_rust_file() {
        let mode = infer_focus_mode("Code.exe", Some("main.rs - ccube"), None, None);
        assert!(matches!(mode, FocusMode::Coding));
    }

    #[test]
    fn test_vscode_with_markdown() {
        let mode = infer_focus_mode("Code.exe", Some("README.md - project"), None, None);
        assert!(matches!(mode, FocusMode::Writing));
    }

    #[test]
    fn test_vscode_no_title() {
        let mode = infer_focus_mode("Code.exe", None, None, None);
        assert!(matches!(mode, FocusMode::Coding));
    }

    #[test]
    fn test_intellij() {
        let mode = infer_focus_mode("idea64.exe", Some("Main.java"), None, None);
        assert!(matches!(mode, FocusMode::Coding));
    }

    #[test]
    fn test_davinci_resolve() {
        let mode = infer_focus_mode("Resolve.exe", Some("Project 1"), None, None);
        assert!(matches!(mode, FocusMode::VideoProduction));
    }

    #[test]
    fn test_word() {
        let mode = infer_focus_mode("WINWORD.EXE", Some("Document1.docx"), None, None);
        assert!(matches!(mode, FocusMode::Writing));
    }

    #[test]
    fn test_chrome_github_by_url() {
        let mode = infer_focus_mode(
            "chrome.exe",
            Some("rust-lang/rust - GitHub"),
            Some("https://github.com/rust-lang/rust"),
            None,
        );
        assert!(matches!(mode, FocusMode::Coding));
    }

    #[test]
    fn test_chrome_google_docs_by_url() {
        let mode = infer_focus_mode(
            "chrome.exe",
            Some("My Document - Google Docs"),
            Some("https://docs.google.com/document/d/abc"),
            None,
        );
        assert!(matches!(mode, FocusMode::Writing));
    }

    #[test]
    fn test_chrome_generic() {
        let mode = infer_focus_mode("chrome.exe", Some("YouTube"), None, None);
        assert!(matches!(mode, FocusMode::Unspecified));
    }

    #[test]
    fn test_unknown_app() {
        let mode = infer_focus_mode("calculator.exe", Some("Calculator"), None, None);
        assert!(matches!(mode, FocusMode::Unspecified));
    }

    #[test]
    fn test_terminal() {
        let mode = infer_focus_mode("WindowsTerminal.exe", Some("pwsh"), None, None);
        assert!(matches!(mode, FocusMode::Coding));
    }

    #[test]
    fn test_is_browser_detection() {
        assert!(is_browser("chrome.exe"));
        assert!(is_browser("msedge.exe"));
        assert!(is_browser("firefox.exe"));
        assert!(is_browser("brave.exe"));
        assert!(is_browser("arc.exe"));
        assert!(is_browser("opera.exe"));
        assert!(!is_browser("code.exe"));
        assert!(!is_browser("notepad.exe"));
        // Exact-match guards: these should NOT match as browsers
        assert!(!is_browser("searchapp.exe")); // "arc" substring
        assert!(!is_browser("cooperation.exe")); // "opera" substring
    }

    #[test]
    fn test_obs_not_browser() {
        // OBS should be video production, not browser
        let mode = infer_focus_mode("obs64.exe", Some("Scene 1"), None, None);
        assert!(matches!(mode, FocusMode::VideoProduction));
        // jobscheduler should NOT match "obs"
        assert!(!is_browser("jobscheduler.exe"));
    }

    #[test]
    fn test_focus_mode_to_str() {
        assert_eq!(focus_mode_to_str(&FocusMode::Coding), "Coding");
        assert_eq!(focus_mode_to_str(&FocusMode::Writing), "Writing");
        assert_eq!(
            focus_mode_to_str(&FocusMode::VideoProduction),
            "VideoProduction"
        );
        assert_eq!(focus_mode_to_str(&FocusMode::Unspecified), "Unspecified");
    }
}
