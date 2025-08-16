// Module declarations
mod modules;

use modules::{
    app_state::AppState,
    pattern_analyzer::InteractionMetrics,
    tauri_commands::*,
    mode_handlers::handle_mode_specific_logic,
    utils::send_log,
};

use tauri::{
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    menu::{MenuBuilder, MenuEvent, MenuItemBuilder, CheckMenuItemBuilder},
    Manager, WindowEvent, Emitter, State, Listener,
};
use tokio::time::{interval, MissedTickBehavior};
use chrono::{Utc, Timelike};


#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_log::Builder::new()
            .target(tauri_plugin_log::Target::new(
                tauri_plugin_log::TargetKind::Stdout,
            ))
            .build())
        .manage(tauri::async_runtime::block_on(AppState::new()).expect("Failed to initialize app state"))
        .invoke_handler(tauri::generate_handler![
            check_connections,
            get_current_mode,
            set_mode,
            get_hourly_summary,
            generate_hourly_summary,
            get_daily_summary,
            generate_daily_summary_command,
            get_ollama_models,
            load_user_config,
            save_user_config,
            process_interaction_metrics,
            get_pattern_analysis,
            train_user_baseline,
            test_generate,
            test_simple_summary,
            categorize_activities_by_time,
            get_app_categories,
            update_app_category,
            bulk_update_categories,
            get_activity_history,
            sync_all_activities,
            debug_database_state,
            get_loaded_ollama_model,
        ])
        .on_window_event(|window, event| {
            match event {
                WindowEvent::CloseRequested { api, .. } => {
                    // Hide window instead of closing
                    if let Err(e) = window.hide() {
                        eprintln!("Failed to hide window: {}", e);
                    }
                    api.prevent_close();
                }
                _ => {}
            }
        })
        .setup(|app| {
            let app_handle = app.handle();
            let _state: State<AppState> = app.state();
            
            // Initialize system tray
            setup_system_tray(&app_handle)?;
            
            // Initialize interaction tracker
            let interaction_tracker = modules::interaction_tracker::InteractionTracker::new();
            let tracker_handle = app_handle.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = interaction_tracker.start_tracking(tracker_handle).await {
                    eprintln!("Failed to start interaction tracking: {}", e);
                }
            });
            
            // Set up pattern analysis event listener
            let pattern_handle = app_handle.clone();
            let listen_handle = pattern_handle.clone();
            listen_handle.listen("interaction_metrics", move |event| {
                if let Ok(metrics) = serde_json::from_str::<InteractionMetrics>(&event.payload()) {
                    let handle = pattern_handle.clone();
                    tauri::async_runtime::spawn(async move {
                        let state = handle.state::<AppState>();
                        if let Err(e) = state.pattern_analyzer.process_interaction(metrics.clone()).await {
                            send_log(&handle, "error", &format!("Failed to process metrics: {}", e));
                        }
                        if let Err(e) = state.pattern_database.store_metrics(&metrics).await {
                            send_log(&handle, "error", &format!("Failed to store metrics: {}", e));
                        }
                    });
                }
            });
            
            // Set up background timer for mode-specific logic
            setup_background_timer(app_handle.clone());
            
            send_log(&app_handle, "info", "Companion Cube initialized successfully");
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn setup_system_tray(app: &tauri::AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let state = app.state::<AppState>();
    let current_mode = tauri::async_runtime::block_on(async {
        let mode = state.current_mode.lock().await;
        mode.clone()
    });
    
    update_tray_menu(app, &current_mode)?;
    Ok(())
}

fn update_tray_menu(app: &tauri::AppHandle, current_mode: &str) -> Result<(), Box<dyn std::error::Error>> {
    // Create checkable menu items for modes
    let ghost_item = CheckMenuItemBuilder::with_id("ghost", "Ghost Mode")
        .checked(current_mode == "ghost")
        .build(app)?;
    
    let chill_item = CheckMenuItemBuilder::with_id("chill", "Chill Mode")
        .checked(current_mode == "chill")
        .build(app)?;
    
    let study_item = CheckMenuItemBuilder::with_id("study_buddy", "Study Mode")
        .checked(current_mode == "study_buddy")
        .build(app)?;
    
    let coach_item = CheckMenuItemBuilder::with_id("coach", "Coach Mode")
        .checked(current_mode == "coach")
        .build(app)?;
    
    let _separator = tauri::menu::PredefinedMenuItem::separator(app)?;
    let dashboard_item = MenuItemBuilder::with_id("dashboard", "Dashboard")
        .build(app)?;
    let check_item = MenuItemBuilder::with_id("check", "Check Ollama and AW")
        .build(app)?;
    let quit_item = MenuItemBuilder::with_id("quit", "Quit")
        .build(app)?;
    
    // Build menu
    let menu = MenuBuilder::new(app)
        .item(&ghost_item)
        .item(&chill_item)
        .item(&study_item)
        .item(&coach_item)
        .separator()
        .item(&dashboard_item)
        .item(&check_item)
        .separator()
        .item(&quit_item)
        .build()?;
    
    // Update or create tray icon
    if let Some(tray) = app.tray_by_id("main") {
        tray.set_menu(Some(menu))?;
        tray.set_tooltip(Some(&format!("Companion Cube - {} Mode", current_mode)))?;
    } else {
        let _tray = TrayIconBuilder::with_id("main")
            .tooltip(&format!("Companion Cube - {} Mode", current_mode))
            .icon(app.default_window_icon().unwrap().clone())
            .menu(&menu)
            .on_menu_event(handle_menu_event)
            .on_tray_icon_event(|tray, event| {
                if let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } = event {
                    if let Some(app) = tray.app_handle().get_webview_window("main") {
                        let _ = app.show();
                        let _ = app.set_focus();
                    }
                }
            })
            .build(app)?;
    }
    
    Ok(())
}

fn handle_menu_event(app: &tauri::AppHandle, event: MenuEvent) {
    match event.id.0.as_str() {
        "dashboard" => {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }
        "ghost" | "chill" | "study_buddy" | "coach" => {
            let mode = event.id.0.clone();
            let app_clone = app.clone();
            tauri::async_runtime::spawn(async move {
                let state = app_clone.state::<AppState>();
                if let Err(e) = set_mode(mode.clone(), state, app_clone.clone()).await {
                    send_log(&app_clone, "error", &format!("Failed to set mode: {}", e));
                } else {
                    // Update tray menu to show checkmark on new mode
                    if let Err(e) = update_tray_menu(&app_clone, &mode) {
                        send_log(&app_clone, "error", &format!("Failed to update tray menu: {}", e));
                    }
                }
            });
        }
        "check" => {
            send_log(app, "info", "Connection check requested from tray menu");
            if let Err(e) = app.emit("check_connections", ()) {
                send_log(app, "error", &format!("Failed to emit check connections: {}", e));
            }
        }
        "quit" => {
            send_log(app, "info", "Application quit requested from tray menu");
            std::process::exit(0);
        }
        _ => {
            send_log(app, "debug", &format!("Unknown menu item clicked: {}", event.id.0));
        }
    }
}

fn setup_background_timer(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut timer = interval(std::time::Duration::from_secs(60));
        timer.set_missed_tick_behavior(MissedTickBehavior::Skip);
        
        loop {
            timer.tick().await;
            
            let state = app.state::<AppState>();
            let (current_mode, should_run) = {
                let mode = state.current_mode.lock().await;
                let should_run = should_run_summary(&mode, &state).await;
                (mode.clone(), should_run)
            };
            
            if should_run {
                if let Err(e) = handle_mode_specific_logic(&app, &current_mode, &state).await {
                    send_log(&app, "error", &format!("Mode logic error: {}", e));
                }
                
                // Update last run time
                let mut times = state.last_summary_time.lock().await;
                times.insert(current_mode, Utc::now());
            }
        }
    });
}

async fn should_run_summary(mode: &str, state: &AppState) -> bool {
    let now = Utc::now();
    let times = state.last_summary_time.lock().await;
    
    if let Some(last_run) = times.get(mode) {
        let elapsed = now.signed_duration_since(*last_run).num_seconds();
        if elapsed < 60 {
            return false;
        }
    }
    
    match mode {
        "ghost" | "chill" => now.minute() == 0,
        "study_buddy" => now.minute() % 5 == 0,
        "coach" => now.minute() % 15 == 0,
        _ => false
    }
}