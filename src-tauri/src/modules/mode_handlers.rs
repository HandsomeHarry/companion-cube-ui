use tauri::{AppHandle, Manager, Emitter};
use chrono::{Local, Timelike};
use crate::modules::app_state::{AppState, HourlySummary};
use crate::modules::utils::{send_log, send_notification, load_user_config_internal, get_configured_aw_client};

pub async fn handle_mode_specific_logic(app: &AppHandle, mode: &str, _state: &AppState) -> Result<(), String> {
    
    match mode {
        "ghost" => handle_ghost_mode(app).await,
        "chill" => handle_chill_mode(app).await,
        "study_buddy" => handle_study_mode(app).await,
        "coach" => handle_coach_mode(app).await,
        _ => {
            send_log(app, "warn", &format!("Unknown mode: {}", mode));
            Ok(())
        }
    }
}

pub async fn handle_ghost_mode(app: &AppHandle) -> Result<(), String> {
    // Ghost mode hourly summary
    
    // Generate regular hourly summary (existing logic)
    let now = Local::now();
    let data_dir = std::path::PathBuf::from("data");
    std::fs::create_dir_all(&data_dir).map_err(|e| e.to_string())?;
    let summary_file = data_dir.join("hourly_summary.txt");
    
    let (summary_text, focus_score, current_state, work_score, distraction_score, neutral_score) = generate_new_hourly_summary(now, &summary_file).await?;
    
    // Save to JSON file for ghost mode
    let ghost_file = data_dir.join("ghost_summaries.json");
    let mut summaries: Vec<serde_json::Value> = if ghost_file.exists() {
        let content = std::fs::read_to_string(&ghost_file).unwrap_or_default();
        serde_json::from_str(&content).unwrap_or_default()
    } else {
        Vec::new()
    };
    
    summaries.push(serde_json::json!({
        "timestamp": now.format("%Y-%m-%d %H:%M:%S").to_string(),
        "hour": now.hour(),
        "mode": "ghost",
        "summary": summary_text,
        "focus_score": focus_score,
        "state": current_state
    }));
    
    std::fs::write(&ghost_file, serde_json::to_string_pretty(&summaries).unwrap())
        .map_err(|e| e.to_string())?;
    
    // Emit event to update frontend
    let hourly_summary = HourlySummary {
        summary: summary_text,
        focus_score,
        last_updated: now.format("%H:%M").to_string(),
        period: format!("{}-{}", 
                       (now - chrono::Duration::minutes(60)).format("%H:%M"),
                       now.format("%H:%M")),
        current_state,
        work_score,
        distraction_score,
        neutral_score,
    };
    
    // Emit event
    
    // Store in app state
    {
        let state = app.state::<AppState>();
        {
            let mut latest = state.latest_hourly_summary.lock().await;
            *latest = Some(hourly_summary.clone());
        }
    }
    
    app.emit("hourly_summary_updated", &hourly_summary)
        .map_err(|e| format!("Failed to emit summary update: {}", e))?;
    
    // Summary saved
    Ok(())
}

pub async fn handle_chill_mode(app: &AppHandle) -> Result<(), String> {
    // Chill mode check
    
    let aw_client = get_configured_aw_client().await;
    let aw_connected = aw_client.test_connection().await.connected;
    
    if !aw_connected {
        send_log(app, "warn", "ActivityWatch not connected, skipping chill mode check");
        return Ok(());
    }
    
    // Generate activity summary using the same logic as manual generation
    let now = Local::now();
    let (summary_text, focus_score, current_state, work_score, distraction_score, neutral_score) = generate_ai_summary_with_app(&aw_client, now, Some(app)).await?;
    
    // Check if user needs a nudge
    if current_state == "unproductive" {
        let config = load_user_config_internal().await.unwrap_or_default();
        send_notification(app, "Time for a change?", &config.chill_notification_prompt).await;
    }
    
    // Emit event to update frontend
    let hourly_summary = HourlySummary {
        summary: summary_text.clone(),
        focus_score,
        last_updated: now.format("%H:%M").to_string(),
        period: format!("{}-{}", 
                       (now - chrono::Duration::minutes(60)).format("%H:%M"),
                       now.format("%H:%M")),
        current_state: current_state.clone(),
        work_score,
        distraction_score,
        neutral_score,
    };
    
    // Emit event
    
    // Store in app state
    {
        let state = app.state::<AppState>();
        {
            let mut latest = state.latest_hourly_summary.lock().await;
            *latest = Some(hourly_summary.clone());
        }
    }
    
    app.emit("hourly_summary_updated", &hourly_summary)
        .map_err(|e| format!("Failed to emit summary update: {}", e))?;
    
    // Log the summary
    // Chill mode completed
    
    // Check completed
    Ok(())
}

pub async fn handle_study_mode(app: &AppHandle) -> Result<(), String> {
    // Study mode check
    
    let aw_client = get_configured_aw_client().await;
    let aw_connected = aw_client.test_connection().await.connected;
    
    if !aw_connected {
        send_log(app, "warn", "ActivityWatch not connected, skipping study mode check");
        return Ok(());
    }
    
    let config = load_user_config_internal().await.unwrap_or_default();
    let study_focus = if config.study_focus.is_empty() {
        "general studying".to_string()
    } else {
        config.study_focus.clone()
    };
    
    // Generate study-focused summary using study context
    let now = Local::now();
    let (summary_text, focus_score, current_state, work_score, distraction_score, neutral_score) = generate_study_focused_summary(&aw_client, now, &study_focus, app).await?;
    
    // Check if user is distracted from studying
    if current_state == "unproductive" {
        send_notification(app, "Study Focus", &config.study_notification_prompt).await;
        // User distracted - notification sent
    } else if current_state == "productive" || current_state == "moderate" {
        // Good focus detected
    }
    
    // Save study summary
    let data_dir = std::path::PathBuf::from("data");
    std::fs::create_dir_all(&data_dir).map_err(|e| e.to_string())?;
    let summary_file = data_dir.join("study_summary.txt");
    
    let entry = format!("---\n{}\n{}\n{}\n{}\nMode: study\nFocus: {}\n", 
                       now.format("%H:%M"), summary_text, focus_score, current_state, study_focus);
    std::fs::write(&summary_file, entry).map_err(|e| e.to_string())?;
    
    // Emit event to update frontend
    let hourly_summary = HourlySummary {
        summary: summary_text.clone(),
        focus_score,
        last_updated: now.format("%H:%M").to_string(),
        period: format!("{}-{}", 
                       (now - chrono::Duration::minutes(5)).format("%H:%M"),
                       now.format("%H:%M")),
        current_state: current_state.clone(),
        work_score,
        distraction_score,
        neutral_score,
    };
    
    // Store in app state
    {
        let state = app.state::<AppState>();
        {
            let mut latest = state.latest_hourly_summary.lock().await;
            *latest = Some(hourly_summary.clone());
        }
    }
    
    app.emit("hourly_summary_updated", &hourly_summary)
        .map_err(|e| format!("Failed to emit summary update: {}", e))?;
    
    // Study mode completed
    Ok(())
}

pub async fn handle_coach_mode(app: &AppHandle) -> Result<(), String> {
    // Coach mode todo generation
    
    let aw_client = get_configured_aw_client().await;
    let aw_connected = aw_client.test_connection().await.connected;
    
    if !aw_connected {
        send_log(app, "warn", "ActivityWatch not connected, skipping coach mode check");
        return Ok(());
    }
    
    let config = load_user_config_internal().await.unwrap_or_default();
    let coach_task = if config.coach_task.is_empty() {
        "complete daily tasks".to_string()
    } else {
        config.coach_task.clone()
    };
    
    // Generate comprehensive activity summary like manual generation
    let now = Local::now();
    let (summary_text, focus_score, current_state, work_score, distraction_score, neutral_score) = generate_ai_summary_with_app(&aw_client, now, Some(app)).await?;
    
    // Also generate todo list for coach mode
    let todo_list = generate_coach_todo_list(&aw_client, now, &coach_task).await?;
    
    // Save todo list
    let data_dir = std::path::PathBuf::from("data");
    std::fs::create_dir_all(&data_dir).map_err(|e| e.to_string())?;
    let todo_file = data_dir.join("coach_todos.json");
    
    let json_content = serde_json::to_string_pretty(&todo_list).map_err(|e| e.to_string())?;
    std::fs::write(&todo_file, json_content).map_err(|e| e.to_string())?;
    
    // Send notification to check todos
    send_notification(app, "Coach Check-in", &config.coach_notification_prompt).await;
    
    // Emit event to update frontend with comprehensive summary
    let hourly_summary = HourlySummary {
        summary: summary_text,
        focus_score,
        last_updated: now.format("%H:%M").to_string(),
        period: format!("{}-{}", 
                       (now - chrono::Duration::minutes(15)).format("%H:%M"),
                       now.format("%H:%M")),
        current_state,
        work_score,
        distraction_score,
        neutral_score,
    };
    
    // Store in app state
    {
        let state = app.state::<AppState>();
        {
            let mut latest = state.latest_hourly_summary.lock().await;
            *latest = Some(hourly_summary.clone());
        }
    }
    
    app.emit("hourly_summary_updated", &hourly_summary)
        .map_err(|e| format!("Failed to emit summary update: {}", e))?;
    
    // Todo list generated
    Ok(())
}

// Actual implementation of AI summary generation
async fn generate_new_hourly_summary(now: chrono::DateTime<Local>, summary_file: &std::path::Path) -> Result<(String, u32, String, u32, u32, u32), String> {
    // This is for ghost mode - generate time-based summary without ActivityWatch
    let hour = now.hour();
    let focus_score = crate::modules::utils::calculate_time_based_focus_score(hour);
    
    let summary = crate::modules::utils::generate_time_based_summary();
    let current_state = if focus_score > 80 { "productive" } else if focus_score > 60 { "moderate" } else if focus_score > 40 { "chilling" } else { "unproductive" };
    
    // Save to file
    let entry = format!("---\n{}\n{}\n{}\n{}\n", 
                       now.format("%H:%M"), summary, focus_score, current_state);
    std::fs::write(summary_file, entry).map_err(|e| e.to_string())?;
    
    Ok((summary, focus_score, current_state.to_string(), 50, 30, 20)) // Default scores for time-based summary
}

// Removed unused generate_ai_summary function

async fn generate_ai_summary_with_app(
    aw_client: &crate::modules::activity_watch::ActivityWatchClient, 
    now: chrono::DateTime<Local>,
    app: Option<&AppHandle>
) -> Result<(String, u32, String, u32, u32, u32), String> {
    use crate::modules::enhanced_processor::{process_for_enhanced_analysis, create_enhanced_prompt};
    use crate::modules::ai_integration::{call_ollama_api, parse_llm_response};
    
    eprintln!("\n[AI SUMMARY] ==================== STARTING GENERATION ====================");
    eprintln!("[AI SUMMARY] Timestamp: {}", now.format("%Y-%m-%d %H:%M:%S"));
    eprintln!("[AI SUMMARY] Type: Enhanced hourly summary with full timeline");
    
    // Get multi-timeframe data
    eprintln!("[AI SUMMARY] Fetching multi-timeframe activity data...");
    let timeframes = match aw_client.get_multi_timeframe_data_active().await {
        Ok(data) => {
            eprintln!("[AI SUMMARY] Successfully fetched data for {} timeframes", data.len());
            data
        },
        Err(e) => {
            eprintln!("[AI SUMMARY] ERROR: Failed to get activity data: {}", e);
            return Err(format!("Failed to get activity data: {}", e))
        }
    };
    
    // Process data locally first
    let state = app.ok_or("App handle required for database access")?
        .state::<AppState>();
    let db = &state.pattern_database;
    
    eprintln!("[AI SUMMARY] Processing activity data with enhanced analysis...");
    let enhanced_data = process_for_enhanced_analysis(&timeframes, db).await?;
    
    // Log processed metrics
    eprintln!("[AI SUMMARY] Local metrics calculated:");
    eprintln!("  - State: {}", enhanced_data.local_metrics.current_state);
    eprintln!("  - Work: {}%", enhanced_data.local_metrics.work_percentage);
    eprintln!("  - Distraction: {}%", enhanced_data.local_metrics.distraction_percentage);
    eprintln!("  - Focus Score: {}%", enhanced_data.focus_score);
    eprintln!("  - Context Switches/hr: {:.0}", enhanced_data.local_metrics.context_switches_per_hour);
    eprintln!("  - Timeline Events: {}", enhanced_data.detailed_timeline.len());
    eprintln!("  - Context Switches: {}", enhanced_data.context_switches.len());
    
    // Load user context
    let config = load_user_config_internal().await.unwrap_or_default();
    let user_context = config.user_context.clone();
    
    // Create enhanced prompt with full timeline
    let prompt = create_enhanced_prompt(&enhanced_data, &user_context);
    
    // Use local metrics as fallback values
    let focus_score = enhanced_data.focus_score;
    let mut current_state = enhanced_data.local_metrics.current_state.clone();
    let work_score = enhanced_data.local_metrics.work_percentage as u32;
    let distraction_score = enhanced_data.local_metrics.distraction_percentage as u32;
    let neutral_score = enhanced_data.local_metrics.neutral_percentage as u32;
    
    // Check if Ollama is available for enhanced analysis
    let ollama_connected = crate::modules::ai_integration::test_ollama_connection().await;
    
    let summary = if ollama_connected {
        eprintln!("[AI SUMMARY] Ollama connected, requesting enhanced analysis...");
        match call_ollama_api(&prompt).await {
            Ok(response) => {
                // Parse the enhanced analysis
                match parse_llm_response(&response) {
                    Ok(analysis) => {
                        // Update state if LLM has high confidence
                        if analysis.confidence == "high" {
                            current_state = analysis.current_state.clone();
                        }
                        
                        // Use the professional summary from LLM
                        eprintln!("[AI SUMMARY] Parsed analysis:");
                        eprintln!("  - professional_summary: {}", analysis.professional_summary);
                        eprintln!("  - primary_activity: {}", analysis.primary_activity);
                        eprintln!("  - current_state: {}", analysis.current_state);
                        
                        // Always prefer professional_summary if it exists and is not default
                        let has_professional = !analysis.professional_summary.is_empty() 
                            && analysis.professional_summary != crate::modules::ai_integration::default_professional_summary();
                        
                        let summary = if has_professional {
                            eprintln!("[AI SUMMARY] Using professional_summary: {}", analysis.professional_summary);
                            analysis.professional_summary.clone()
                        } else {
                            // If no professional summary, create one from available data
                            eprintln!("[AI SUMMARY] No professional_summary, creating from primary_activity and other fields");
                            eprintln!("[AI SUMMARY] primary_activity: {}", analysis.primary_activity);
                            
                            // Create a more detailed summary from all available fields
                            format!("{} You're in a {} state with {}% focus. {} {}", 
                                analysis.primary_activity,
                                analysis.current_state,
                                focus_score,
                                analysis.reasoning,
                                if analysis.confidence == "high" { "Activity patterns are clear." } else { "Activity patterns show some variability." }
                            )
                        };
                        
                        // Log any ADHD insights detected
                        if let Ok(full_response) = serde_json::from_str::<serde_json::Value>(&response) {
                            if let Some(insights) = full_response.get("adhd_insights") {
                                eprintln!("[AI SUMMARY] ADHD Insights: {:?}", insights);
                            }
                            if let Some(suggestions) = full_response.get("personalized_suggestions") {
                                eprintln!("[AI SUMMARY] Suggestions: {:?}", suggestions);
                            }
                        }
                        
                        summary
                    },
                    Err(e) => {
                        eprintln!("[AI SUMMARY] Failed to parse LLM response: {}", e);
                        // Use local summary
                        format!("You've been primarily using {} with {} context switches in the last hour.",
                            enhanced_data.detailed_timeline.last()
                                .map(|e| e.name.as_str())
                                .unwrap_or("various apps"),
                            enhanced_data.context_switches.len()
                        )
                    }
                }
            },
            Err(e) => {
                eprintln!("[AI SUMMARY] Ollama call failed: {}, using local summary", e);
                // Generate summary from timeline
                let top_apps = enhanced_data.detailed_timeline.iter()
                    .take(3)
                    .map(|e| e.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("Recent activity includes {} with {} total context switches.",
                    top_apps,
                    enhanced_data.context_switches.len()
                )
            }
        }
    } else {
        eprintln!("[AI SUMMARY] Ollama not connected, using local summary");
        // Generate summary from timeline when Ollama is not available
        let main_app = enhanced_data.detailed_timeline
            .iter()
            .max_by_key(|e| (e.duration_minutes * 100.0) as i64)
            .map(|e| e.name.as_str())
            .unwrap_or("various applications");
        format!("You've spent most time in {} with a {}% productivity rate.",
            main_app,
            enhanced_data.local_metrics.work_percentage as i32
        )
    };
    
    eprintln!("[AI SUMMARY] Final summary: {}", summary);
    Ok((summary, focus_score, current_state, work_score, distraction_score, neutral_score))
}

async fn generate_study_focused_summary(aw_client: &crate::modules::activity_watch::ActivityWatchClient, now: chrono::DateTime<Local>, study_focus: &str, app: &AppHandle) -> Result<(String, u32, String, u32, u32, u32), String> {
    use crate::modules::enhanced_processor::{process_for_enhanced_analysis, create_enhanced_prompt};
    use crate::modules::ai_integration::{call_ollama_api, parse_llm_response};
    
    eprintln!("\n[AI SUMMARY] ==================== STARTING STUDY MODE GENERATION ====================");
    eprintln!("[AI SUMMARY] Timestamp: {}", now.format("%Y-%m-%d %H:%M:%S"));
    eprintln!("[AI SUMMARY] Study Focus: {}", study_focus);
    eprintln!("[AI SUMMARY] Type: Study-focused 5-minute summary");
    
    // Get multi-timeframe data
    eprintln!("[AI SUMMARY] Fetching multi-timeframe activity data...");
    let timeframes = aw_client.get_multi_timeframe_data_active().await?;
    
    // Process data locally first
    let state = app.state::<AppState>();
    let db = &state.pattern_database;
    
    eprintln!("[AI SUMMARY] Processing activity data with enhanced analysis...");
    let enhanced_data = process_for_enhanced_analysis(&timeframes, db).await?;
    
    // Log processed metrics
    eprintln!("[AI SUMMARY] Study mode metrics:");
    eprintln!("  - State: {}", enhanced_data.local_metrics.current_state);
    eprintln!("  - Work: {}%", enhanced_data.local_metrics.work_percentage);
    eprintln!("  - Focus Score: {}%", enhanced_data.focus_score);
    eprintln!("  - Timeline Events: {}", enhanced_data.detailed_timeline.len());
    
    // Create study-specific context
    let study_context = format!("address the user as harry. Currently studying: {}. Analyze whether activities align with study goals. Pay special attention to distractions from study material.", study_focus);
    
    // Create enhanced prompt for study analysis
    let prompt = create_enhanced_prompt(&enhanced_data, &study_context);
    
    // Use local metrics as fallback values
    let focus_score = enhanced_data.focus_score;
    let mut current_state = enhanced_data.local_metrics.current_state.clone();
    let work_score = enhanced_data.local_metrics.work_percentage as u32;
    let distraction_score = enhanced_data.local_metrics.distraction_percentage as u32;
    let neutral_score = enhanced_data.local_metrics.neutral_percentage as u32;
    
    // Check if Ollama is available
    let ollama_connected = crate::modules::ai_integration::test_ollama_connection().await;
    
    let base_summary = if ollama_connected {
        eprintln!("[AI SUMMARY] Ollama connected, requesting study analysis...");
        match call_ollama_api(&prompt).await {
            Ok(response) => {
                match parse_llm_response(&response) {
                    Ok(analysis) => {
                        // Update state if LLM has high confidence
                        if analysis.confidence == "high" {
                            current_state = analysis.current_state.clone();
                        }
                        
                        // Check for study-specific patterns
                        if let Ok(full_response) = serde_json::from_str::<serde_json::Value>(&response) {
                            if let Some(details) = full_response.get("detailed_analysis") {
                                if let Some(distractions) = details.get("distraction_triggers") {
                                    eprintln!("[AI SUMMARY] Study distractions detected: {:?}", distractions);
                                }
                            }
                        }
                        
                        analysis.professional_summary
                    },
                    Err(e) => {
                        eprintln!("[AI SUMMARY] Failed to parse study response: {}", e);
                        format!("Study session analysis: {} context switches detected while studying {}.",
                            enhanced_data.context_switches.len(),
                            study_focus
                        )
                    }
                }
            },
            Err(_) => {
                eprintln!("[AI SUMMARY] Ollama call failed, using local summary");
                format!("Study session: focused on {} with {} context switches", 
                    study_focus,
                    enhanced_data.context_switches.len()
                )
            }
        }
    } else {
        eprintln!("[AI SUMMARY] Ollama not connected, using local summary");
        format!("Study session: focused on {} with {} context switches", 
                    study_focus,
                    enhanced_data.context_switches.len()
                )
    };
    
    // Add study context to summary
    let summary = format!("{} [Study Focus: {}]", base_summary, study_focus);
    
    eprintln!("[AI SUMMARY] Final study summary: {}", summary);
    Ok((summary, focus_score, current_state, work_score, distraction_score, neutral_score))
}

// Removed unused fallback function - now using local metrics calculation

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
struct TodoItem {
    id: String,
    text: String,
    completed: bool,
    created_at: String,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
struct CoachTodoList {
    todos: Vec<TodoItem>,
    generated_at: String,
    context: String,
}

async fn generate_coach_todo_list(_aw_client: &crate::modules::activity_watch::ActivityWatchClient, now: chrono::DateTime<Local>, coach_task: &str) -> Result<CoachTodoList, String> {
    Ok(CoachTodoList {
        todos: vec![
            TodoItem {
                id: "1".to_string(),
                text: format!("Work on: {}", coach_task),
                completed: false,
                created_at: now.format("%Y-%m-%d %H:%M:%S").to_string(),
            }
        ],
        generated_at: now.format("%Y-%m-%d %H:%M:%S").to_string(),
        context: coach_task.to_string(),
    })
}