use anyhow::{Result, bail};
use std::path::Path;

const TASK_NAME: &str = "ccube-daemon";

/// Register the daemon as an autostart task via Windows Task Scheduler.
#[cfg(target_os = "windows")]
pub fn install_autostart(daemon_exe: &Path) -> Result<()> {
    let exe_str = daemon_exe
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("daemon path contains invalid UTF-8"))?;

    let output = std::process::Command::new("schtasks.exe")
        .args([
            "/Create", "/TN", TASK_NAME, "/TR", exe_str, "/SC", "ONLOGON", "/RL", "LIMITED", "/F",
        ])
        .output()?;

    if !output.status.success() {
        bail!(
            "schtasks /Create failed (exit code {:?}). \
             This usually requires administrator privileges — \
             try running the terminal as Administrator.",
            output.status.code()
        );
    }

    // Verify creation
    if !is_autostart_installed() {
        bail!("schtasks reported success but task not found on query");
    }

    Ok(())
}

/// Remove the autostart task from Windows Task Scheduler.
#[cfg(target_os = "windows")]
pub fn uninstall_autostart() -> Result<()> {
    let output = std::process::Command::new("schtasks.exe")
        .args(["/Delete", "/TN", TASK_NAME, "/F"])
        .output()?;

    if !output.status.success() {
        // If task doesn't exist, treat as success (idempotent).
        // Check exit code 1 which typically means "not found" for /Delete.
        if output.status.code() == Some(1) && !is_autostart_installed() {
            return Ok(());
        }
        bail!(
            "schtasks /Delete failed (exit code {:?}). \
             This usually requires administrator privileges — \
             try running the terminal as Administrator.",
            output.status.code()
        );
    }

    Ok(())
}

/// Check whether the autostart task is currently registered.
#[cfg(target_os = "windows")]
pub fn is_autostart_installed() -> bool {
    std::process::Command::new("schtasks.exe")
        .args(["/Query", "/TN", TASK_NAME])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(not(target_os = "windows"))]
pub fn install_autostart(_daemon_exe: &Path) -> Result<()> {
    bail!("autostart not implemented for this platform")
}

#[cfg(not(target_os = "windows"))]
pub fn uninstall_autostart() -> Result<()> {
    bail!("autostart not implemented for this platform")
}

#[cfg(not(target_os = "windows"))]
pub fn is_autostart_installed() -> bool {
    false
}
