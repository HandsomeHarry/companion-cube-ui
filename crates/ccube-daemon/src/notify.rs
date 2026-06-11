//! Nudge notification delivery, per platform.
//!
//! macOS has two paths: when running from inside `Companion Cube.app` (see
//! `scripts/make-bundle.sh`), notifications post through
//! `UNUserNotificationCenter` and carry our name and icon. Un-bundled dev
//! builds (`cargo run`) fall back to `osascript`, which posts under Script
//! Editor's identity — functional, just not branded. UNUserNotificationCenter
//! *crashes* (uncatchable NSException) in processes without a bundle
//! identity, so the gate is mandatory, not cosmetic.

/// Deliver a nudge notification. Never blocks the async runtime; failures
/// are logged and swallowed (a missed banner must never hurt the daemon).
pub fn send_nudge(decision_id: i64, message: &str) {
    let msg = message.to_string();
    std::thread::spawn(move || deliver(decision_id, &msg));
}

#[cfg(target_os = "macos")]
fn deliver(decision_id: i64, msg: &str) {
    if is_bundled() {
        deliver_un(decision_id, msg);
    } else {
        deliver_osascript(msg);
    }
}

/// True when the executable lives inside an .app bundle.
#[cfg(target_os = "macos")]
fn is_bundled() -> bool {
    std::env::current_exe()
        .map(|p| p.to_string_lossy().contains(".app/Contents/MacOS/"))
        .unwrap_or(false)
}

/// Native UNUserNotificationCenter path — our bundle identity, our icon.
#[cfg(target_os = "macos")]
fn deliver_un(decision_id: i64, msg: &str) {
    use objc2_foundation::NSString;
    use objc2_user_notifications::{
        UNAuthorizationOptions, UNMutableNotificationContent, UNNotificationRequest,
        UNNotificationSound, UNUserNotificationCenter,
    };

    // Ask once per process; the OS remembers the user's answer across runs.
    static AUTH: std::sync::Once = std::sync::Once::new();

    let center = unsafe { UNUserNotificationCenter::currentNotificationCenter() };

    AUTH.call_once(|| {
        let handler = block2::RcBlock::new(|granted: objc2::runtime::Bool, _err| {
            if !granted.as_bool() {
                tracing::warn!("notification authorization denied by user");
            }
        });
        center.requestAuthorizationWithOptions_completionHandler(
            UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
            &handler,
        );
    });

    let content = unsafe { UNMutableNotificationContent::new() };
    unsafe {
        content.setTitle(&NSString::from_str("Companion Cube"));
        content.setBody(&NSString::from_str(msg));
        content.setSound(Some(&UNNotificationSound::defaultSound()));
    }

    let identifier = NSString::from_str(&format!("ccube-nudge-{decision_id}"));
    let request = unsafe {
        UNNotificationRequest::requestWithIdentifier_content_trigger(
            &identifier,
            &content,
            None, // deliver immediately
        )
    };

    let completion = block2::RcBlock::new(|err: *mut objc2_foundation::NSError| {
        if !err.is_null() {
            tracing::warn!("failed to post UN notification");
        } else {
            tracing::debug!("nudge notification sent (UNUserNotificationCenter)");
        }
    });
    center.addNotificationRequest_withCompletionHandler(&request, Some(&completion));
}

/// osascript fallback for un-bundled builds. The message is passed as an
/// argv item (never interpolated into the script) so LLM-generated text
/// cannot inject AppleScript.
#[cfg(target_os = "macos")]
fn deliver_osascript(msg: &str) {
    let script = concat!(
        "on run argv\n",
        "display notification (item 1 of argv) ",
        "with title \"Companion Cube\" sound name \"Glass\"\n",
        "end run"
    );
    match std::process::Command::new("osascript")
        .args(["-e", script, msg])
        .output()
    {
        Ok(o) if o.status.success() => tracing::debug!("nudge notification sent (osascript)"),
        Ok(o) => tracing::warn!(
            stderr = %String::from_utf8_lossy(&o.stderr),
            "osascript notification failed"
        ),
        Err(e) => tracing::warn!(error = %e, "failed to send nudge notification"),
    }
}

/// PowerShell balloon tip. The message rides in an environment variable
/// rather than the script string, preventing command injection from
/// LLM-generated output.
#[cfg(windows)]
fn deliver(decision_id: i64, msg: &str) {
    use std::os::windows::process::CommandExt;
    let script = concat!(
        "Add-Type -AssemblyName System.Windows.Forms;",
        "$n = New-Object System.Windows.Forms.NotifyIcon;",
        "$n.Icon = [System.Drawing.SystemIcons]::Information;",
        "$n.BalloonTipTitle = 'Companion Cube #' + $env:CCUBE_DECISION_ID;",
        "$n.BalloonTipText = $env:CCUBE_NUDGE_MSG;",
        "$n.Visible = $true;",
        "$n.ShowBalloonTip(8000);",
        "Start-Sleep -Seconds 9;",
        "$n.Dispose()"
    );
    match std::process::Command::new("powershell")
        .args(["-NoProfile", "-WindowStyle", "Hidden", "-Command", script])
        .env("CCUBE_NUDGE_MSG", msg)
        .env("CCUBE_DECISION_ID", decision_id.to_string())
        .creation_flags(0x08000000) // CREATE_NO_WINDOW
        .output()
    {
        Ok(_) => tracing::debug!("nudge notification sent"),
        Err(e) => tracing::warn!(error = %e, "failed to send nudge notification"),
    }
}

#[cfg(all(not(windows), not(target_os = "macos")))]
fn deliver(decision_id: i64, msg: &str) {
    let title = format!("Companion Cube #{decision_id}");
    match std::process::Command::new("notify-send")
        .args([&title, msg])
        .output()
    {
        Ok(_) => tracing::debug!("nudge notification sent"),
        Err(e) => tracing::warn!(error = %e, "failed to send nudge notification"),
    }
}
