#[tauri::command]
fn start_daemon() -> Result<String, String> {
    let exe_dir = std::env::current_exe()
        .map_err(|e| format!("cannot find exe dir: {e}"))?
        .parent()
        .ok_or("no parent dir")?
        .to_path_buf();

    let daemon_path = exe_dir.join("ccube-daemon");

    if !daemon_path.exists() {
        return Err(format!(
            "ccube-daemon not found at {}",
            daemon_path.display()
        ));
    }

    std::process::Command::new(&daemon_path)
        .spawn()
        .map_err(|e| format!("failed to start daemon: {e}"))?;

    Ok("Daemon started".to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![start_daemon])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
