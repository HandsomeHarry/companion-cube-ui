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
    if current_state.contains("needs_nudge") || current_state.contains("distracted") {
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
    if current_state == "needs_nudge" || current_state.contains("distracted") {
        send_notification(app, "Study Focus", &config.study_notification_prompt).await;
        // User distracted - notification sent
    } else if current_state == "flow" || current_state == "working" {
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
    let current_state = if focus_score > 70 { "flow" } else if focus_score > 50 { "working" } else { "needs_nudge" };
    
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
    use crate::modules::event_processor::EventProcessor;
    use crate::modules::ai_integration::{call_ollama_api, parse_llm_response};
    
    
    // Get multi-timeframe data
    let timeframes = match aw_client.get_multi_timeframe_data_active().await {
        Ok(data) => data,
        Err(e) => return Err(format!("Failed to get activity data: {}", e))
    };
    
    // Process data for LLM
    let processor = EventProcessor::new();
    
    // Load user context
    let config = load_user_config_internal().await.unwrap_or_default();
    let user_context = config.user_context.clone();
    
    // Prepare data with advanced analysis
    let raw_data = processor.prepare_raw_data_with_advanced_analysis(&timeframes, &user_context);
    
    // Create enhanced prompt with advanced patterns and app categories
    let prompt = if let Some(app_handle) = app {
        let state = app_handle.state::<AppState>();
        let db = &state.pattern_database;
        processor.create_state_analysis_prompt_with_categories(&raw_data, &user_context, db).await
    } else {
        processor.create_enhanced_analysis_prompt(&raw_data, &user_context)
    };
    
    // Check if Ollama is available
    let ollama_connected = crate::modules::ai_integration::test_ollama_connection().await;
    
    if ollama_connected {
        // Call Ollama API
        match call_ollama_api(&prompt).await {
            Ok(response) => {
                // Parse response
                match parse_llm_response(&response) {
                    Ok(analysis) => {
                        let focus_score = match analysis.confidence.as_str() {
                            "high" => match analysis.current_state.as_str() {
                                "flow" => 90,
                                "working" => 70,
                                "needs_nudge" => 40,
                                _ => 20,
                            },
                            "medium" => match analysis.current_state.as_str() {
                                "flow" => 80,
                                "working" => 60,
                                "needs_nudge" => 35,
                                _ => 20,
                            },
                            _ => 50,
                        };
                        
                        // Use professional_summary if available, otherwise fall back to primary_activity
                        let summary = if !analysis.professional_summary.is_empty() && 
                                       analysis.professional_summary != crate::modules::ai_integration::default_professional_summary() {
                            analysis.professional_summary.clone()
                        } else {
                            analysis.primary_activity.clone()
                        };
                        
                        // Check for fatigue warnings from advanced analysis
                        if let (Some(app_handle), Some(ref advanced)) = (app, &raw_data.advanced_analysis) {
                            if advanced.fatigue_analysis.break_urgency == "urgent" || advanced.fatigue_analysis.break_urgency == "recommended" {
                                send_notification(app_handle, "Break Time", &advanced.fatigue_analysis.recommended_action).await;
                                send_log(app_handle, "warn", &format!("Fatigue detected: {}", advanced.fatigue_analysis.recommended_action));
                            }
                            
                            // Warn about rabbit holes
                            if advanced.rabbit_hole_detection.is_rabbit_hole {
                                // Rabbit hole detected
                            }
                        }
                        
                        Ok((summary, focus_score, analysis.current_state.clone(), 
                            analysis.work_score, analysis.distraction_score, analysis.neutral_score))
                    },
                    Err(e) => {
                        // Fallback on parsing error
                        // Parse error - use fallback
                        generate_fallback_summary(&raw_data, now)
                    }
                }
            },
            Err(e) => {
                // Ollama call failed - use fallback
                generate_fallback_summary(&raw_data, now)
            }
        }
    } else {
        // Fallback when Ollama is not available
        generate_fallback_summary(&raw_data, now)
    }
}

async fn generate_study_focused_summary(aw_client: &crate::modules::activity_watch::ActivityWatchClient, now: chrono::DateTime<Local>, study_focus: &str, app: &AppHandle) -> Result<(String, u32, String, u32, u32, u32), String> {
    use crate::modules::event_processor::EventProcessor;
    use crate::modules::ai_integration::{call_ollama_api, parse_llm_response};
    
    // Get multi-timeframe data
    let timeframes = aw_client.get_multi_timeframe_data_active().await?;
    
    // Process data for LLM
    let processor = EventProcessor::new();
    
    // Create study-specific context
    let study_context = format!("Currently studying: {}. Focus on whether activities align with study goals.", study_focus);
    
    // Prepare data with advanced analysis
    let raw_data = processor.prepare_raw_data_with_advanced_analysis(&timeframes, &study_context);
    
    // Create enhanced prompt with study focus and app categories
    let state = app.state::<AppState>();
    let db = &state.pattern_database;
    let prompt = processor.create_state_analysis_prompt_with_categories(&raw_data, &study_context, db).await;
    
    // Check if Ollama is available
    let ollama_connected = crate::modules::ai_integration::test_ollama_connection().await;
    
    if ollama_connected {
        match call_ollama_api(&prompt).await {
            Ok(response) => {
                match parse_llm_response(&response) {
                    Ok(analysis) => {
                        let focus_score = match analysis.confidence.as_str() {
                            "high" => match analysis.current_state.as_str() {
                                "flow" => 90,
                                "working" => 70,
                                "needs_nudge" => 40,
                                _ => 20,
                            },
                            "medium" => match analysis.current_state.as_str() {
                                "flow" => 80,
                                "working" => 60,
                                "needs_nudge" => 35,
                                _ => 20,
                            },
                            _ => 50,
                        };
                        
                        // Use professional_summary if available, otherwise fall back to primary_activity
                        let base_summary = if !analysis.professional_summary.is_empty() && 
                                            analysis.professional_summary != crate::modules::ai_integration::default_professional_summary() {
                            analysis.professional_summary.clone()
                        } else {
                            analysis.primary_activity.clone()
                        };
                        
                        // Add study context to summary
                        let summary = format!("{} [Study Focus: {}]", base_summary, study_focus);
                        
                        Ok((summary, focus_score, analysis.current_state.clone(), 
                            analysis.work_score, analysis.distraction_score, analysis.neutral_score))
                    },
                    Err(e) => {
                        // Parse error - use fallback
                        generate_fallback_summary(&raw_data, now)
                    }
                }
            },
            Err(e) => {
                // Ollama call failed - use fallback
                generate_fallback_summary(&raw_data, now)
            }
        }
    } else {
        generate_fallback_summary(&raw_data, now)
    }
}

// Helper function for fallback when AI is not available
fn generate_fallback_summary(raw_data: &crate::modules::event_processor::RawDataForLLM, _now: chrono::DateTime<Local>) -> Result<(String, u32, String, u32, u32, u32), String> {
    // Simple heuristic-based analysis
    let recent_stats = raw_data.timeframes.get("5_minutes")
        .map(|tf| &tf.statistics);
    let medium_stats = raw_data.timeframes.get("30_minutes")
        .map(|tf| &tf.statistics);
    let _hour_stats = raw_data.timeframes.get("1_hour")
        .map(|tf| &tf.statistics);
    
    let (summary, focus_score, state, work_score, distraction_score, neutral_score) = if let Some(stats) = recent_stats {
        let apps: Vec<String> = stats.unique_apps.iter().cloned().collect();
        
        // Generate a 3-sentence professional summary
        let sentence1 = if apps.is_empty() {
            "No recent activity has been detected in the monitoring period.".to_string()
        } else {
            let app_list = apps.iter()
                .take(3)
                .map(|a| {
                    let (clean_name, _) = crate::modules::utils::extract_app_and_exe_name(a);
                    clean_name
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!("Your recent activity has primarily involved {} with approximately {:.0} minutes of active engagement.", 
                    app_list, stats.total_active_minutes)
        };
        
        let sentence2 = if let Some(med_stats) = medium_stats {
            format!("Over the past 30 minutes, you have switched contexts {} times across {} different applications, indicating a {} level of task switching.", 
                    med_stats.context_switches, 
                    med_stats.unique_apps.len(),
                    if med_stats.context_switches < 5 { "low" } else if med_stats.context_switches < 10 { "moderate" } else { "high" })
        } else {
            "Activity patterns over the extended period could not be analyzed.".to_string()
        };
        
        let sentence3 = match stats.context_switches {
            0 if apps.len() == 1 => "You are currently in a deep focus state with sustained attention on a single task, which is optimal for productivity.".to_string(),
            n if n < 5 => "Your work pattern shows good focus with minimal distractions, suggesting effective task management.".to_string(),
            _ => "Consider reducing context switches to improve focus and productivity in your current workflow.".to_string(),
        };
        
        let full_summary = format!("{} {} {}", sentence1, sentence2, sentence3);
        
        let focus_score = if stats.context_switches == 0 && apps.len() == 1 {
            85
        } else if stats.context_switches < 3 {
            65
        } else {
            40
        };
        
        let state = if stats.context_switches == 0 && apps.len() == 1 {
            "flow"
        } else if stats.context_switches < 5 {
            "working"
        } else {
            "needs_nudge"
        };
        
        // Calculate basic work/distraction scores from app usage
        let work_score = if stats.context_switches == 0 && apps.len() == 1 { 85 } else if stats.context_switches < 5 { 65 } else { 40 };
        let distraction_score = if stats.context_switches > 10 { 40 } else if stats.context_switches > 5 { 25 } else { 10 };
        let neutral_score = 100 - work_score - distraction_score;
        
        (full_summary, focus_score, state.to_string(), work_score, distraction_score, neutral_score)
    } else {
        ("Unable to analyze activity data at this time. Please ensure ActivityWatch is running and collecting data. Check your system configuration if this issue persists. Consider restarting the monitoring service.".to_string(), 50, "unknown".to_string(), 33, 33, 34)
    };
    
    Ok((summary, focus_score, state, work_score, distraction_score, neutral_score))
}

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