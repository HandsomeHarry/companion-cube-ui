use anyhow::Result;
use serde::Deserialize;
use std::io::{BufRead, Seek, SeekFrom};

use crate::daemon_client;
use crate::paths::DataRoot;

#[derive(Deserialize)]
struct HealthResponse {
    status: String,
    uptime_s: u64,
    daemon_version: String,
}

#[derive(Deserialize)]
struct ShutdownResponse {
    #[allow(dead_code)]
    status: String,
}

/// Start the daemon as a detached background process.
pub async fn handle_start(root: &DataRoot) -> Result<()> {
    // Check if already running
    if daemon_client::is_daemon_running().await {
        println!("Daemon is already running.");
        return Ok(());
    }

    // Check for stale PID file
    let pid_file = root.data_dir.join("daemon.pid");
    if pid_file.exists() {
        let _ = std::fs::remove_file(&pid_file);
    }

    // Locate ccube-daemon binary next to ccube binary
    let self_exe = std::env::current_exe()?;
    let bin_dir = self_exe.parent().unwrap_or(std::path::Path::new("."));

    let daemon_exe = if cfg!(windows) {
        bin_dir.join("ccube-daemon.exe")
    } else {
        bin_dir.join("ccube-daemon")
    };

    if !daemon_exe.exists() {
        anyhow::bail!(
            "daemon binary not found at {}. Build it first with `cargo build`.",
            daemon_exe.display()
        );
    }

    // Spawn detached process
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        const DETACHED_PROCESS: u32 = 0x00000008;

        let child = std::process::Command::new(&daemon_exe)
            .creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS)
            .spawn()?;
        println!("Daemon starting (PID {})...", child.id());
    }

    #[cfg(not(windows))]
    {
        let child = std::process::Command::new(&daemon_exe).spawn()?;
        println!("Daemon starting (PID {})...", child.id());
    }

    // Poll /health until responsive (up to 3 seconds)
    for _ in 0..15 {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        if daemon_client::is_daemon_running().await {
            println!("Daemon started successfully.");
            return Ok(());
        }
    }

    println!(
        "Daemon process started but not yet responsive. Check `ccube daemon logs` for details."
    );
    Ok(())
}

/// Stop the daemon via HTTP, with PID fallback.
pub async fn handle_stop(root: &DataRoot) -> Result<()> {
    // Try HTTP shutdown first
    match daemon_client::post_empty::<ShutdownResponse>("/shutdown").await {
        Ok(_) => {
            println!("Daemon stopping...");

            // Poll until unreachable (up to 3 seconds)
            for _ in 0..15 {
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                if !daemon_client::is_daemon_running().await {
                    println!("Daemon stopped.");
                    return Ok(());
                }
            }

            println!("Shutdown requested but daemon still responding. It may take a moment.");
            Ok(())
        }
        Err(_) => {
            // HTTP failed — try PID-based kill
            let pid_file = root.data_dir.join("daemon.pid");
            if pid_file.exists() {
                let pid_str = std::fs::read_to_string(&pid_file)?;
                let pid = pid_str.trim();

                #[cfg(windows)]
                {
                    let output = std::process::Command::new("taskkill")
                        .args(["/PID", pid, "/F"])
                        .output()?;
                    if output.status.success() {
                        let _ = std::fs::remove_file(&pid_file);
                        println!("Daemon killed (PID {pid}).");
                    } else {
                        println!("Failed to kill daemon (PID {pid}). It may not be running.");
                    }
                }
                #[cfg(not(windows))]
                {
                    let output = std::process::Command::new("kill")
                        .arg(pid)
                        .output()?;
                    if output.status.success() {
                        let _ = std::fs::remove_file(&pid_file);
                        println!("Daemon killed (PID {pid}).");
                    } else {
                        println!("Failed to kill daemon (PID {pid}). It may not be running.");
                    }
                }
            } else {
                println!("Daemon is not running.");
            }

            Ok(())
        }
    }
}

/// Show daemon status.
pub async fn handle_status(_root: &DataRoot) -> Result<()> {
    match daemon_client::get_json::<HealthResponse>("/health").await {
        Ok(health) => {
            println!("Daemon:     running ({})", health.status);
            println!("Version:    {}", health.daemon_version);

            let hours = health.uptime_s / 3600;
            let mins = (health.uptime_s % 3600) / 60;
            let secs = health.uptime_s % 60;
            println!("Uptime:     {hours}h {mins}m {secs}s");
        }
        Err(_) => {
            println!("Daemon:     not running");
        }
    }

    let installed = ccube_core::service::is_autostart_installed();
    println!(
        "Autostart:  {}",
        if installed {
            "installed"
        } else {
            "not installed"
        }
    );

    Ok(())
}

/// Tail daemon logs from daemon.ndjson.
pub fn handle_logs(root: &DataRoot, follow: bool, agent: Option<&str>) -> Result<()> {
    let log_file = match agent {
        Some("detector") => root.logs_dir.join("detector.ndjson"),
        Some("curator") => root.logs_dir.join("curator.ndjson"),
        Some("reflector") => root.logs_dir.join("reflector.ndjson"),
        _ => root.logs_dir.join("daemon.ndjson"),
    };

    if !log_file.exists() {
        println!("No log file found at {}", log_file.display());
        return Ok(());
    }

    if follow {
        // Tail mode: seek to end, then poll for new lines
        let file = std::fs::File::open(&log_file)?;
        let mut reader = std::io::BufReader::new(file);
        reader.seek(SeekFrom::End(0))?;

        println!("Following {}... (Ctrl+C to stop)", log_file.display());
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) => {
                    std::thread::sleep(std::time::Duration::from_millis(500));
                }
                Ok(_) => {
                    print_log_line(&line);
                }
                Err(e) => {
                    eprintln!("Error reading log: {e}");
                    break;
                }
            }
        }
    } else {
        // Show last 50 lines
        let content = std::fs::read_to_string(&log_file)?;
        let lines: Vec<&str> = content.lines().collect();
        let start = if lines.len() > 50 {
            lines.len() - 50
        } else {
            0
        };

        for line in &lines[start..] {
            print_log_line(line);
        }
    }

    Ok(())
}

/// Install the daemon as an autostart service.
pub fn handle_install(_root: &DataRoot) -> Result<()> {
    let self_exe = std::env::current_exe()?;
    let bin_dir = self_exe.parent().unwrap_or(std::path::Path::new("."));

    let daemon_exe = if cfg!(windows) {
        bin_dir.join("ccube-daemon.exe")
    } else {
        bin_dir.join("ccube-daemon")
    };

    if !daemon_exe.exists() {
        anyhow::bail!(
            "daemon binary not found at {}. Build it first.",
            daemon_exe.display()
        );
    }

    ccube_core::service::install_autostart(&daemon_exe)?;
    println!("Autostart installed. Daemon will start automatically on next logon.");
    Ok(())
}

/// Remove the daemon autostart registration.
pub fn handle_uninstall() -> Result<()> {
    ccube_core::service::uninstall_autostart()?;
    println!("Autostart removed.");
    Ok(())
}

/// Pretty-print a single ndjson log line.
fn print_log_line(line: &str) {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return;
    }

    // Try to parse as JSON for pretty display
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed) {
        let ts = val.get("timestamp").and_then(|v| v.as_str()).unwrap_or("");
        let level = val.get("level").and_then(|v| v.as_str()).unwrap_or("?");
        let msg = val
            .get("fields")
            .and_then(|f| f.get("message"))
            .and_then(|v| v.as_str())
            .unwrap_or("");

        // Extract just the time portion from the timestamp
        let time_part = if ts.len() >= 19 { &ts[11..19] } else { ts };

        println!("[{time_part}] {level:>5} {msg}");
    } else {
        // Not JSON, print as-is
        print!("{line}");
    }
}
