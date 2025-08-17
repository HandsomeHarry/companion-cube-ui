use tauri::{AppHandle, State, Manager, Emitter};
use crate::modules::app_state::{AppState, HourlySummary};
use crate::modules::utils::{send_log, UserConfig, get_configured_aw_client};
use crate::modules::pattern_analyzer::InteractionMetrics;
use sqlx::Row;

#[tauri::command]
pub async fn check_connections() -> Result<serde_json::Value, String> {
    let aw_client = crate::modules::utils::get_configured_aw_client().await;
    let aw_test = aw_client.test_connection().await;
    
    let ollama_connected = crate::modules::ai_integration::test_ollama_connection().await;
    
    // Return the expected structure for the frontend
    Ok(serde_json::json!({
        "activitywatch": aw_test.connected,
        "ollama": ollama_connected
    }))
}

#[tauri::command]
pub async fn get_current_mode(state: State<'_, AppState>) -> Result<String, String> {
    let mode = state.current_mode.lock().await;
    Ok(mode.clone())
}

#[tauri::command]
pub async fn set_mode(mode: String, state: State<'_, AppState>, app: AppHandle) -> Result<(), String> {
    send_log(&app, "info", &format!("Switching to {} mode", mode));
    
    {
        let mut current_mode = state.current_mode.lock().await;
        *current_mode = mode.clone();
    }
    
    // Save mode to persistent storage
    AppState::save_mode(&mode)?;
    
    // Tray menu update will be handled by the main module
    
    // Notify frontend
    app.emit("mode_changed", &mode).map_err(|e| e.to_string())?;
    
    // Clear last run time to let the timer handle the next execution
    {
        let mut times = state.last_summary_time.lock().await;
        times.remove(&mode);
    }
    
    // Don't generate immediate summary - let frontend request it after UI loads
    send_log(&app, "info", &format!("Mode switched to {} - summary generation will be triggered by frontend", mode));
    
    // Mode switch completed
    Ok(())
}

#[tauri::command]
pub async fn get_hourly_summary(state: State<'_, AppState>) -> Result<HourlySummary, String> {
    // First check if we have a recent summary in memory
    {
        let latest = state.latest_hourly_summary.lock().await;
        if let Some(summary) = latest.as_ref() {
            return Ok(summary.clone());
        }
    }
    
    // If not, generate a new one
    let now = chrono::Local::now();
    Ok(HourlySummary {
        summary: "No recent summary available".to_string(),
        focus_score: 50,
        last_updated: now.format("%H:%M").to_string(),
        period: format!("{}-{}", 
                       (now - chrono::Duration::minutes(30)).format("%H:%M"),
                       now.format("%H:%M")),
        current_state: "unknown".to_string(),
        work_score: 33,
        distraction_score: 33,
        neutral_score: 34,
    })
}

#[tauri::command]
pub async fn generate_hourly_summary(app: AppHandle) -> Result<HourlySummary, String> {
    send_log(&app, "info", "Manual hourly summary generation requested");
    
    // Get current mode and run its handler
    let state = app.state::<AppState>();
    let mode = {
        let current_mode = state.current_mode.lock().await;
        current_mode.clone()
    };
    
    // Run the mode-specific handler
    match crate::modules::mode_handlers::handle_mode_specific_logic(&app, &mode, &state).await {
        Ok(_) => {},
        Err(e) => {
            send_log(&app, "error", &format!("Failed to generate summary: {}", e));
            return Err(e);
        }
    }
    
    // Return the generated summary from state
    {
        let latest = state.latest_hourly_summary.lock().await;
        if let Some(summary) = latest.as_ref() {
            // Send notification with summary
            let notification_title = match mode.as_str() {
                "study_buddy" => "Study Mode Update",
                "coach" => "Coach Mode Update",
                _ => "Activity Summary"
            };
            
            // Extract key info from summary for notification
            let notification_body = format!("Focus Score: {} - {}", 
                summary.focus_score, 
                summary.current_state
            );
            
            crate::modules::utils::send_notification(&app, notification_title, &notification_body).await;
            
            return Ok(summary.clone());
        }
    }
    
    Err("Failed to retrieve generated summary".to_string())
}

#[tauri::command]
pub async fn load_user_config() -> Result<UserConfig, String> {
    crate::modules::utils::load_user_config_internal().await
}

#[tauri::command]
pub async fn save_user_config(config: UserConfig, app: AppHandle) -> Result<(), String> {
    // Check if model changed
    let old_config = crate::modules::utils::load_user_config_internal().await.ok();
    let model_changed = old_config.as_ref()
        .map(|old| old.ollama_model != config.ollama_model)
        .unwrap_or(false);
    
    // Save user config
    let data_dir = std::path::PathBuf::from("data");
    std::fs::create_dir_all(&data_dir).map_err(|e| e.to_string())?;
    
    let config_path = data_dir.join("config.json");
    let config_str = serde_json::to_string_pretty(&config)
        .map_err(|e| format!("Failed to serialize config: {}", e))?;
    
    std::fs::write(&config_path, config_str)
        .map_err(|e| format!("Failed to write config file: {}", e))?;
    
    send_log(&app, "info", &format!("Config saved. Model: {}", config.ollama_model));
    
    // If model changed, unload old model and load new one
    if model_changed {
        send_log(&app, "info", &format!("Model changed. Switching from {:?} to {}", 
            old_config.map(|c| c.ollama_model).unwrap_or_else(|| "none".to_string()), 
            config.ollama_model
        ));
        
        // Unload all models first to free VRAM
        let client = crate::modules::ai_integration::get_ollama_client();
        let unload_payload = serde_json::json!({
            "name": "", // Empty name unloads all models
            "keep_alive": 0
        });
        
        match client
            .post(format!("http://localhost:{}/api/generate", config.ollama_port))
            .json(&unload_payload)
            .send()
            .await {
            Ok(_) => send_log(&app, "info", "Unloaded previous model from VRAM"),
            Err(e) => send_log(&app, "warn", &format!("Failed to unload model: {}", e))
        }
        
        // Small delay to ensure unload completes
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        
        // Load the new model by making a test request
        send_log(&app, "info", &format!("Loading new model: {}", config.ollama_model));
        let test_payload = serde_json::json!({
            "model": config.ollama_model,
            "prompt": "test",
            "stream": false,
            "options": {
                "temperature": 0.1,
                "num_predict": 1
            }
        });
        
        match client
            .post(format!("http://localhost:{}/api/generate", config.ollama_port))
            .json(&test_payload)
            .timeout(std::time::Duration::from_secs(60)) // Longer timeout for model loading
            .send()
            .await {
            Ok(resp) => {
                if resp.status().is_success() {
                    send_log(&app, "info", &format!("Successfully loaded model: {}", config.ollama_model));
                } else {
                    send_log(&app, "error", &format!("Failed to load model: {}", resp.status()));
                }
            }
            Err(e) => send_log(&app, "error", &format!("Failed to load model: {}", e))
        }
    }
    
    Ok(())
}

#[tauri::command]
pub async fn process_interaction_metrics(
    metrics: InteractionMetrics,
    state: State<'_, AppState>
) -> Result<(), String> {
    // Store metrics in database
    state.pattern_database.store_metrics(&metrics).await?;
    
    // Process through pattern analyzer
    state.pattern_analyzer.process_interaction(metrics).await?;
    
    Ok(())
}

#[tauri::command]
pub async fn get_pattern_analysis(state: State<'_, AppState>) -> Result<String, String> {
    let analysis = state.pattern_analyzer.analyze_current_patterns().await?;
    
    // Convert to JSON for frontend
    serde_json::to_string(&analysis)
        .map_err(|e| format!("Failed to serialize analysis: {}", e))
}

#[tauri::command]
pub async fn train_user_baseline(state: State<'_, AppState>) -> Result<String, String> {
    let baseline = state.pattern_analyzer.train_baseline().await?;
    
    // Also update the baseline in AppState
    {
        let mut app_baseline = state.user_baseline.lock().await;
        *app_baseline = Some(baseline.clone());
    }
    
    Ok(format!("Baseline training complete. Productive hours: {:?}", baseline.productive_hours))
}

#[tauri::command]
pub async fn test_generate() -> Result<String, String> {
    // Test command
    Ok("Test successful!".to_string())
}

#[tauri::command]
pub async fn categorize_activities_by_time(app: AppHandle) -> Result<serde_json::Value, String> {
    send_log(&app, "info", "Categorizing activities by time");
    
    let aw_client = crate::modules::utils::get_configured_aw_client().await;
    let aw_connected = aw_client.test_connection().await.connected;
    
    if !aw_connected {
        send_log(&app, "warn", "ActivityWatch not connected");
        return Ok(serde_json::json!({
            "work": 33,
            "communication": 33,
            "distraction": 34
        }));
    }
    
    // Get the last hour of data
    let now = chrono::Utc::now();
    let start = now - chrono::Duration::hours(1);
    let events = aw_client.get_active_window_events(start, now).await
        .map_err(|e| format!("Failed to get events: {}", e))?;
    
    // Use cached categories from database
    let state = app.state::<AppState>();
    let db = &state.pattern_database;
    
    // Get all app categories
    let app_categories = db.get_all_app_categories().await
        .unwrap_or_else(|_| Vec::new());
    
    // Create a map for quick lookup
    let category_map: std::collections::HashMap<String, (String, i32)> = app_categories
        .into_iter()
        .map(|(app, cat, _, score)| (app, (cat, score)))
        .collect();
    
    // Calculate time spent in each category using cached data
    let mut work_time = 0.0;
    let mut communication_time = 0.0;
    let mut distraction_time = 0.0;
    
    for event in &events {
        if let Some(data) = event.get("data").and_then(|d| d.as_object()) {
            if let Some(app) = data.get("app").and_then(|a| a.as_str()) {
                let duration = event.get("duration").and_then(|d| d.as_f64()).unwrap_or(0.0);
                
                // Use cached category or fallback based on app name
                let category = category_map.get(app)
                    .map(|(cat, _)| cat.as_str())
                    .unwrap_or_else(|| {
                        // Simple fallback categorization
                        let app_lower = app.to_lowercase();
                        if app_lower.contains("code") || app_lower.contains("vim") || 
                           app_lower.contains("terminal") || app_lower.contains("jetbrains") {
                            "work"
                        } else if app_lower.contains("slack") || app_lower.contains("teams") ||
                                  app_lower.contains("discord") || app_lower.contains("mail") {
                            "communication"
                        } else if app_lower.contains("youtube") || app_lower.contains("game") ||
                                  app_lower.contains("steam") || app_lower.contains("twitch") {
                            "entertainment"
                        } else {
                            "other"
                        }
                    });
                
                match category {
                    "work" | "development" | "productivity" => work_time += duration,
                    "communication" => communication_time += duration,
                    "entertainment" => distraction_time += duration,
                    _ => distraction_time += duration, // Count 'other' as distraction
                }
            }
        }
    }
    
    let total_time = work_time + communication_time + distraction_time;
    if total_time == 0.0 {
        return Ok(serde_json::json!({
            "work": 33,
            "communication": 33,
            "distraction": 34
        }));
    }
    
    Ok(serde_json::json!({
        "work": ((work_time / total_time * 100.0) as u32),
        "communication": ((communication_time / total_time * 100.0) as u32),
        "distraction": ((distraction_time / total_time * 100.0) as u32)
    }))
}

#[tauri::command]
pub async fn test_simple_summary(app: AppHandle) -> Result<HourlySummary, String> {
    // Test summary command
    
    let now = chrono::Local::now();
    let summary = HourlySummary {
        summary: "Test summary - if you see this, the command system is working!".to_string(),
        focus_score: 75,
        last_updated: now.format("%H:%M").to_string(),
        period: format!("{}-{}", 
                       (now - chrono::Duration::minutes(60)).format("%H:%M"),
                       now.format("%H:%M")),
        current_state: "working".to_string(),
        work_score: 70,
        distraction_score: 20,
        neutral_score: 10,
    };
    
    // Update state
    let state = app.state::<AppState>();
    {
        let mut latest = state.latest_hourly_summary.lock().await;
        *latest = Some(summary.clone());
    }
    
    // Emit event
    app.emit("hourly_summary_updated", &summary)
        .map_err(|e| format!("Failed to emit: {}", e))?;
    
    Ok(summary)
}

#[tauri::command]
pub async fn generate_daily_summary_command(app: AppHandle) -> Result<serde_json::Value, String> {
    send_log(&app, "info", "Generating daily summary");
    
    match generate_daily_summary_internal(app).await {
        Ok(summary) => Ok(summary),
        Err(e) => {
            // Daily summary error
            Err(e)
        }
    }
}

async fn generate_daily_summary_internal(app: AppHandle) -> Result<serde_json::Value, String> {
    
    let _state = app.state::<AppState>();
    
    // Get ActivityWatch data for the whole day
    let now = chrono::Local::now();
    // Get current time
    
    let start_of_day = now.date_naive().and_hms_opt(0, 0, 0)
        .ok_or("Failed to create start of day time")?
        .and_local_timezone(chrono::Local)
        .single()
        .ok_or("Failed to convert to local timezone")?
        .with_timezone(&chrono::Utc);
    let end_of_day = chrono::Utc::now();
    
    // Set date range
    
    let aw_client = crate::modules::utils::get_configured_aw_client().await;
    // Fetch events
    let events = match aw_client.get_active_window_events(start_of_day, end_of_day).await {
        Ok(events) => {
            send_log(&app, "info", &format!("Retrieved {} events for daily summary", events.len()));
            events
        },
        Err(e) => {
            send_log(&app, "error", &format!("Failed to get daily events: {}", e));
            return Ok(serde_json::json!({
                "date": now.format("%Y-%m-%d").to_string(),
                "summary": "Unable to generate daily summary - ActivityWatch data unavailable.",
                "total_active_time": 0,
                "top_applications": Vec::<String>::new(),
                "total_sessions": 0,
                "generated_at": now.format("%Y-%m-%d %H:%M:%S").to_string()
            }));
        }
    };
    
    // Calculate statistics
    // Calculate statistics
    let mut app_time_map: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    let mut total_time = 0.0;
    let mut session_count = 0;
    let mut last_event_time: Option<chrono::DateTime<chrono::Utc>> = None;
    
    for (idx, event) in events.iter().enumerate() {
        if idx < 5 {
            send_log(&app, "trace", &format!("Event {}: {:?}", idx, event));
        }
        if let Some(data) = event.get("data").and_then(|d| d.as_object()) {
            if let Some(app_name) = data.get("app").and_then(|a| a.as_str()) {
                let duration = event.get("duration").and_then(|d| d.as_f64()).unwrap_or(0.0);
                *app_time_map.entry(app_name.to_string()).or_insert(0.0) += duration;
                total_time += duration;
                
                // Count sessions (gap > 5 minutes = new session)
                if let Some(timestamp_str) = event.get("timestamp").and_then(|t| t.as_str()) {
                    match chrono::DateTime::parse_from_rfc3339(timestamp_str) {
                        Ok(timestamp) => {
                            let timestamp = timestamp.with_timezone(&chrono::Utc);
                            if let Some(last) = last_event_time {
                                let gap = timestamp.signed_duration_since(last).num_minutes();
                                if gap > 5 {
                                    session_count += 1;
                                }
                            } else {
                                session_count = 1;
                            }
                            last_event_time = Some(timestamp);
                        },
                        Err(e) => {
                            send_log(&app, "warn", &format!("Failed to parse timestamp: {} - {}", timestamp_str, e));
                        }
                    }
                }
            }
        }
    }
    
    if events.is_empty() {
        send_log(&app, "warn", "No events found for today");
    }
    
    // Get top applications
    // Process statistics
    let mut app_time_vec: Vec<(String, f64)> = app_time_map.into_iter().collect();
    app_time_vec.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let top_apps: Vec<String> = app_time_vec.iter()
        .take(5)
        .map(|(app, _)| app.clone())
        .collect();
    // Get top apps
    
    // Generate summary using LLM if available
    // Check Ollama
    let summary = if crate::modules::ai_integration::test_ollama_connection().await {
        send_log(&app, "info", "Ollama connected, generating AI summary");
        let user_config = crate::modules::utils::load_user_config_internal().await?;
        
        let prompt = format!(
            r#"Generate a daily summary (3 sentences) based on the user's computer activity. Respond with plain text only, no JSON.

USER CONTEXT: {}

ACTIVITY DATA:
- Total active time: {:.1} hours
- Number of work sessions: {}
- Top applications: {}

Write exactly 3 sentences:
First sentence: Summarize overall productivity and time usage
Second sentence: Highlight main activities or focus areas
Third sentence: Offer a brief insight or encouragement

Keep the tone professional and supportive. Do not use JSON format or bullet points."#,
            user_config.user_context,
            total_time / 3600.0,
            session_count,
            top_apps.join(", ")
        );
        
        // Call Ollama
        match crate::modules::ai_integration::call_ollama_api(&prompt).await {
            Ok(response) => {
                send_log(&app, "info", "Successfully generated AI daily summary");
                // Try to parse JSON response and extract meaningful text
                if let Ok(json_response) = serde_json::from_str::<serde_json::Value>(&response) {
                    // Extract summary parts from JSON
                    let mut summary_parts = Vec::new();
                    
                    if let Some(productivity) = json_response.get("Overall Productivity Summary").and_then(|v| v.as_str()) {
                        summary_parts.push(productivity);
                    }
                    if let Some(activities) = json_response.get("Main Activities & Focus Areas").and_then(|v| v.as_str()) {
                        summary_parts.push(activities);
                    }
                    if let Some(insight) = json_response.get("Insight & Encouragement").and_then(|v| v.as_str()) {
                        summary_parts.push(insight);
                    }
                    
                    if !summary_parts.is_empty() {
                        summary_parts.join(" ")
                    } else {
                        // If no specific fields found, try to extract any string values
                        json_response.as_object()
                            .map(|obj| {
                                obj.values()
                                    .filter_map(|v| v.as_str())
                                    .collect::<Vec<_>>()
                                    .join(" ")
                            })
                            .unwrap_or(response)
                    }
                } else {
                    // If not JSON, return as is
                    response
                }
            },
            Err(e) => {
                send_log(&app, "error", &format!("LLM error: {}", e));
                format!("You spent {:.1} hours active today across {} sessions. Your top applications were: {}.",
                    total_time / 3600.0, session_count, top_apps.join(", "))
            }
        }
    } else {
        send_log(&app, "warn", "Ollama not connected, using fallback summary");
        format!("You spent {:.1} hours active today across {} sessions. Your top applications were: {}.",
            total_time / 3600.0, session_count, top_apps.join(", "))
    };
    
    // Get database and store the summary
    let state = app.state::<AppState>();
    let db = &state.pattern_database;
    
    // Store in database
    let date_str = now.format("%Y-%m-%d").to_string();
    db.store_daily_summary(
        &date_str,
        &summary,
        total_time as i64,
        session_count,
        &top_apps,
        None, // focus_score - will be calculated if we have hourly data
        None, // work_percentage
        None, // distraction_percentage  
        None  // neutral_percentage
    ).await
    .map_err(|e| format!("Failed to store daily summary: {}", e))?;
    
    // Summary generated
    
    // Create response
    let daily_summary = serde_json::json!({
        "date": date_str,
        "summary": summary,
        "total_active_time": total_time as i64,
        "top_applications": top_apps,
        "total_sessions": session_count,
        "generated_at": now.format("%Y-%m-%d %H:%M:%S").to_string()
    });
    
    // Emit event
    app.emit("daily_summary_updated", &daily_summary)
        .map_err(|e| e.to_string())?;
    
    send_log(&app, "info", "Daily summary generation completed successfully");
    Ok(daily_summary)
}

#[tauri::command]
pub async fn get_daily_summary(app: AppHandle) -> Result<serde_json::Value, String> {
    let state = app.state::<AppState>();
    let db = &state.pattern_database;
    
    let now = chrono::Local::now();
    let date_str = now.format("%Y-%m-%d").to_string();
    
    match db.get_daily_summary(&date_str).await? {
        Some(summary) => Ok(summary),
        None => {
            // Return empty summary if none exists
            Ok(serde_json::json!({
                "date": date_str,
                "summary": "No daily summary available yet. Click 'Generate' to create one.",
                "total_active_time": 0,
                "top_applications": Vec::<String>::new(),
                "total_sessions": 0,
                "generated_at": ""
            }))
        }
    }
}

#[tauri::command]
pub async fn get_ollama_models() -> Result<Vec<String>, String> {
    let config = crate::modules::utils::load_user_config_internal().await?;
    let client = crate::modules::ai_integration::get_ollama_client();
    
    match client
        .get(format!("http://localhost:{}/api/tags", config.ollama_port))
        .send()
        .await
    {
        Ok(response) => {
            if response.status().is_success() {
                match response.json::<serde_json::Value>().await {
                    Ok(data) => {
                        if let Some(models) = data.get("models").and_then(|m| m.as_array()) {
                            let model_names: Vec<String> = models
                                .iter()
                                .filter_map(|model| model.get("name").and_then(|n| n.as_str()))
                                .map(|s| s.to_string())
                                .collect();
                            Ok(model_names)
                        } else {
                            Ok(vec!["mistral".to_string()])
                        }
                    }
                    Err(_) => Ok(vec!["mistral".to_string()])
                }
            } else {
                Ok(vec!["mistral".to_string()])
            }
        }
        Err(_) => Ok(vec!["mistral".to_string()])
    }
}

#[tauri::command]
pub async fn get_app_categories(state: State<'_, AppState>) -> Result<Vec<serde_json::Value>, String> {
    let db = &state.pattern_database;
    
    // Get all unique apps from activities table
    let all_apps_query = sqlx::query(
        "SELECT DISTINCT app_name FROM activities ORDER BY app_name"
    )
    .fetch_all(&db.pool)
    .await
    .map_err(|e| format!("Failed to get all apps: {}", e))?;
    
    let all_apps: Vec<String> = all_apps_query.iter()
        .map(|row| row.get("app_name"))
        .collect();
    
    // Get categorized apps
    let categories = db.get_all_app_categories().await?;
    let category_map: std::collections::HashMap<String, (String, Option<String>, i32)> = categories
        .into_iter()
        .map(|(app, cat, subcat, score)| (app, (cat, subcat, score)))
        .collect();
    
    // Combine all apps with their categories (or default if uncategorized)
    let mut result = Vec::new();
    for app_name in all_apps {
        if let Some((category, subcategory, score)) = category_map.get(&app_name) {
            result.push(serde_json::json!({
                "app_name": app_name,
                "category": category,
                "subcategory": subcategory,
                "productivity_score": score
            }));
        } else {
            // Uncategorized app - use defaults
            result.push(serde_json::json!({
                "app_name": app_name,
                "category": "uncategorized",
                "subcategory": null,
                "productivity_score": 50
            }));
        }
    }
    
    Ok(result)
}

#[tauri::command]
pub async fn update_app_category(
    app_name: String,
    category: String,
    subcategory: Option<String>,
    productivity_score: i32,
    state: State<'_, AppState>
) -> Result<(), String> {
    let db = &state.pattern_database;
    db.set_app_category(
        &app_name,
        &category,
        subcategory.as_deref(),
        Some(productivity_score),
        false // user_modified, not auto_detected
    ).await?;
    
    Ok(())
}

#[tauri::command]
pub async fn bulk_update_categories(
    updates: Vec<serde_json::Value>,
    state: State<'_, AppState>
) -> Result<(), String> {
    let db = &state.pattern_database;
    
    for update in updates {
        if let Some(obj) = update.as_object() {
            let app_name = obj.get("app_name")
                .and_then(|v| v.as_str())
                .ok_or("Missing app_name")?;
            let category = obj.get("category")
                .and_then(|v| v.as_str())
                .ok_or("Missing category")?;
            let subcategory = obj.get("subcategory")
                .and_then(|v| v.as_str());
            let productivity_score = obj.get("productivity_score")
                .and_then(|v| v.as_i64())
                .map(|v| v as i32)
                .unwrap_or(50);
            
            db.set_app_category(
                app_name,
                category,
                subcategory,
                Some(productivity_score),
                false // user_modified
            ).await?;
        }
    }
    
    Ok(())
}

#[tauri::command]
pub async fn get_activity_history(
    time_range: String,
    state: State<'_, AppState>,
    _app: AppHandle
) -> Result<serde_json::Value, String> {
    let db = &state.pattern_database;
    let now = chrono::Utc::now();
    
    let (start, end) = match time_range.as_str() {
        "hour" => (now - chrono::Duration::hours(1), now),
        "day" => (now - chrono::Duration::days(1), now),
        "week" => (now - chrono::Duration::weeks(1), now),
        _ => return Err("Invalid time range".to_string())
    };
    
    // Get category statistics
    let category_stats = db.get_category_statistics(start, end).await?;
    
    // Get hourly breakdown
    let hourly_breakdown = db.get_hourly_breakdown(start, end).await?;
    
    // Get top apps
    let top_apps = db.get_top_apps(start, end, 10).await?;
    
    Ok(serde_json::json!({
        "time_range": time_range,
        "start": start.to_rfc3339(),
        "end": end.to_rfc3339(),
        "category_statistics": category_stats,
        "hourly_breakdown": hourly_breakdown,
        "top_apps": top_apps
    }))
}

#[tauri::command]
pub async fn sync_all_activities(
    app: AppHandle,
    state: State<'_, AppState>
) -> Result<String, String> {
    send_log(&app, "info", "Starting full activity sync from ActivityWatch");
    
    let db = &state.pattern_database;
    let aw_client = get_configured_aw_client().await;
    
    // Check connection
    if !aw_client.test_connection().await.connected {
        return Err("ActivityWatch not connected".to_string());
    }
    
    // Get data from the last 30 days
    let end = chrono::Utc::now();
    let start = end - chrono::Duration::days(30);
    
    send_log(&app, "info", &format!("Fetching activities from {} to {}", start.format("%Y-%m-%d"), end.format("%Y-%m-%d")));
    
    match aw_client.get_active_window_events(start, end).await {
        Ok(events) => {
            send_log(&app, "info", &format!("Retrieved {} events from ActivityWatch", events.len()));
            
            let count = db.store_activities(&events).await?;
            
            send_log(&app, "info", &format!("Stored {} new activities (duplicates ignored)", count));
            
            // Get all unique uncategorized apps and categorize them
            let uncategorized_apps = db.get_uncategorized_apps().await?;
            send_log(&app, "info", &format!("Found {} uncategorized apps", uncategorized_apps.len()));
            
            if !uncategorized_apps.is_empty() {
                // Categorize all apps at once
                send_log(&app, "info", "Categorizing all uncategorized apps...");
                if let Err(e) = categorize_all_apps(&app, db, uncategorized_apps).await {
                    send_log(&app, "warn", &format!("Failed to categorize some apps: {}", e));
                }
                
                // Update activities with new categories
                let update_result = sqlx::query(
                    "UPDATE activities 
                     SET category = (SELECT category FROM app_categories WHERE app_categories.app_name = activities.app_name)
                     WHERE category IS NULL"
                )
                .execute(&db.pool)
                .await;
                
                match update_result {
                    Ok(result) => send_log(&app, "info", &format!("Updated {} activities with categories", result.rows_affected())),
                    Err(e) => send_log(&app, "warn", &format!("Failed to update activity categories: {}", e))
                }
            }
            
            // Get final statistics
            let total_activities = db.get_activity_count().await.unwrap_or(0);
            let categorized_count = db.get_categorized_app_count().await.unwrap_or(0);
            
            // Debug: Check what apps we have in activities
            let debug_apps = sqlx::query("SELECT DISTINCT app_name FROM activities LIMIT 10")
                .fetch_all(&db.pool)
                .await
                .map_err(|e| format!("Debug query failed: {}", e))?;
            
            let app_names: Vec<String> = debug_apps.iter()
                .map(|row| row.get("app_name"))
                .collect();
            
            send_log(&app, "debug", &format!("Sample apps in activities: {:?}", app_names));
            
            // Debug: Check if we have any activities with timestamps
            let recent_count = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM activities WHERE timestamp > datetime('now', '-1 day')"
            )
            .fetch_one(&db.pool)
            .await
            .unwrap_or(0);
            
            send_log(&app, "debug", &format!("Activities from last 24 hours: {}", recent_count));
            
            Ok(format!(
                "Sync complete! {} new activities stored. Total: {} activities, {} apps categorized", 
                count, total_activities, categorized_count
            ))
        }
        Err(e) => {
            send_log(&app, "error", &format!("Failed to fetch activities: {}", e));
            Err(format!("Failed to fetch activities: {}", e))
        }
    }
}

async fn categorize_all_apps(
    app: &AppHandle,
    db: &crate::modules::database::PatternDatabase,
    apps: Vec<String>
) -> Result<(), String> {
    if apps.is_empty() {
        return Ok(());
    }
    
    send_log(app, "info", &format!("Categorizing {} apps...", apps.len()));
    
    // Sort apps alphabetically
    let mut sorted_apps = apps;
    sorted_apps.sort();
    
    // Create batches of 10 apps
    for batch in sorted_apps.chunks(10) {
        let prompt = format!(
            r#"Categorize these applications. For each app, provide:
1. Category: one of [work, communication, entertainment, development, productivity, system, other]
2. Explanation: LESS THAN 5 WORDS describing why this category was chosen

Apps to categorize:
{}

Return JSON only in this exact format:
{{
  "app_name": {{"category": "category_name", "explanation": "short explanation", "productivity_score": 0-100}}
}}

Example:
{{
  "Discord.exe": {{"category": "communication", "explanation": "chat and voice app", "productivity_score": 40}},
  "Code.exe": {{"category": "development", "explanation": "coding IDE", "productivity_score": 90}},
  "chrome.exe": {{"category": "productivity", "explanation": "web browser", "productivity_score": 70}}
}}"#,
            batch.join("\n")
        );
        
        match crate::modules::ai_integration::call_ollama_api(&prompt).await {
            Ok(response) => {
                send_log(app, "debug", &format!("LLM response for batch: {}", response));
                // Try to parse the response as JSON
                match serde_json::from_str::<serde_json::Value>(&response) {
                    Ok(categories) => {
                        if let Some(obj) = categories.as_object() {
                            send_log(app, "debug", &format!("Parsed {} apps from LLM response", obj.len()));
                            for (app_name, data) in obj {
                                if let Some(cat_obj) = data.as_object() {
                                    let category = cat_obj.get("category")
                                        .and_then(|c| c.as_str())
                                        .unwrap_or("other");
                                    let explanation = cat_obj.get("explanation")
                                        .and_then(|e| e.as_str());
                                    let productivity_score = cat_obj.get("productivity_score")
                                        .and_then(|p| p.as_i64())
                                        .map(|p| p as i32)
                                        .unwrap_or(50);
                                    
                                    // Store with explanation as subcategory (if provided and short)
                                    let subcategory = explanation.filter(|e| e.split_whitespace().count() < 5);
                                    
                                    if let Err(e) = db.set_app_category(
                                        app_name,
                                        category,
                                        subcategory,
                                        Some(productivity_score),
                                        true // auto_detected
                                    ).await {
                                        send_log(app, "warn", &format!("Failed to save category for {}: {}", app_name, e));
                                    } else {
                                        send_log(app, "debug", &format!("Categorized {} as {} (score: {})", app_name, category, productivity_score));
                                    }
                                }
                            }
                        } else {
                            send_log(app, "warn", "LLM response was not a JSON object");
                        }
                    }
                    Err(e) => {
                        send_log(app, "error", &format!("Failed to parse LLM response as JSON: {}. Response: {}", e, response));
                    }
                }
            }
            Err(e) => {
                send_log(app, "error", &format!("Failed to categorize batch: {}", e));
            }
        }
        
        // Small delay between batches to avoid overwhelming the LLM
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    }
    
    send_log(app, "info", "App categorization completed");
    Ok(())
}

