use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use chrono::{Local, Timelike};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct UserConfig {
    pub user_context: String,
    pub activitywatch_port: u16,
    pub ollama_port: u16,
    pub study_focus: String,
    pub coach_task: String,
    #[serde(default = "default_ghost_prompt")]
    pub ghost_notification_prompt: String,
    #[serde(default = "default_chill_prompt")]
    pub chill_notification_prompt: String,
    #[serde(default = "default_study_prompt")]
    pub study_notification_prompt: String,
    #[serde(default = "default_coach_prompt")]
    pub coach_notification_prompt: String,
    #[serde(default)]
    pub notifications_enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notification_webhook: Option<String>,
    #[serde(default = "default_ollama_model")]
    pub ollama_model: String,
    #[serde(default = "default_keep_model_loaded")]
    pub keep_model_loaded: bool,
}

fn default_keep_model_loaded() -> bool {
    false
}

fn default_ghost_prompt() -> String {
    "👻 Silent monitoring mode active".to_string()
}

fn default_chill_prompt() -> String {
    "Hey! You've been having fun for a while now. Maybe it's time to take a break or switch to something productive? 🌟".to_string()
}

fn default_study_prompt() -> String {
    "Looks like you got distracted from studying. Let's get back on track! 📚".to_string()
}

fn default_coach_prompt() -> String {
    "Time to check your progress! Please review and update your todo list. ✓".to_string()
}

fn default_ollama_model() -> String {
    "mistral".to_string()
}

impl Default for UserConfig {
    fn default() -> Self {
        Self {
            user_context: "I am a person with ADHD looking to improve my productivity.".to_string(),
            activitywatch_port: 5600,
            ollama_port: 11434,
            study_focus: String::new(),
            coach_task: String::new(),
            ghost_notification_prompt: default_ghost_prompt(),
            chill_notification_prompt: default_chill_prompt(),
            study_notification_prompt: default_study_prompt(),
            coach_notification_prompt: default_coach_prompt(),
            notifications_enabled: true,
            notification_webhook: None,
            ollama_model: default_ollama_model(),
            keep_model_loaded: default_keep_model_loaded(),
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct LogMessage {
    pub level: String,
    pub message: String,
    pub timestamp: String,
}

pub fn send_log(app: &AppHandle, level: &str, message: &str) {
    let log_message = LogMessage {
        level: level.to_string(),
        message: message.to_string(),
        timestamp: Local::now().format("%H:%M:%S").to_string(),
    };
    
    if let Err(e) = app.emit("log_message", &log_message) {
        eprintln!("Failed to emit log message: {}", e);
    }
    
    // Also print to console
    eprintln!("[{}] {}: {}", log_message.timestamp, level.to_uppercase(), message);
}

pub async fn send_notification(app: &AppHandle, title: &str, body: &str) {
    let notification_data = serde_json::json!({
        "title": title,
        "body": body,
        "timestamp": Local::now().format("%H:%M:%S").to_string()
    });
    
    if let Err(e) = app.emit("show_notification", &notification_data) {
        eprintln!("Failed to emit notification: {}", e);
    }
}

pub async fn load_user_config_internal() -> Result<UserConfig, String> {
    let data_dir = std::path::PathBuf::from("data");
    let config_path = data_dir.join("config.json");
    
    if !config_path.exists() {
        return Ok(UserConfig::default());
    }
    
    let config_str = std::fs::read_to_string(&config_path)
        .map_err(|e| format!("Failed to read config file: {}", e))?;
    
    let config: UserConfig = serde_json::from_str(&config_str)
        .map_err(|e| format!("Failed to parse config: {}", e))?;
    
    eprintln!("DEBUG: Loaded user config - context: {}, study_focus: {}, coach_task: {}", 
        config.user_context, config.study_focus, config.coach_task);
    
    Ok(config)
}

pub fn extract_app_and_exe_name(full_path: &str) -> (String, String) {
    // Handle Windows paths
    let path = if full_path.contains('\\') {
        full_path.split('\\').last().unwrap_or(full_path)
    } else {
        full_path.split('/').last().unwrap_or(full_path)
    };
    
    // Remove .exe extension if present
    let app_name = if path.to_lowercase().ends_with(".exe") {
        &path[..path.len() - 4]
    } else {
        path
    };
    
    (app_name.to_string(), path.to_string())
}

pub fn calculate_time_based_focus_score(hour: u32) -> u32 {
    match hour {
        9..=11 => 80,  // Morning focus
        14..=16 => 75, // Afternoon focus
        12..=13 => 60, // Lunch time
        17..=18 => 65, // Early evening
        19..=22 => 55, // Evening
        _ => 40,       // Late night/early morning
    }
}

pub async fn get_configured_aw_client() -> crate::modules::activity_watch::ActivityWatchClient {
    let config = load_user_config_internal().await.unwrap_or_default();
    crate::modules::activity_watch::ActivityWatchClient::new("localhost".to_string(), config.activitywatch_port)
}

pub fn generate_time_based_summary() -> String {
    let hour = Local::now().hour();
    match hour {
        6..=8 => "Starting the day - establishing focus patterns".to_string(),
        9..=11 => "Morning work session - peak productivity time".to_string(),
        12..=13 => "Mid-day transition - maintaining momentum".to_string(),
        14..=16 => "Afternoon focus period - deep work time".to_string(),
        17..=19 => "Evening wind-down - wrapping up tasks".to_string(),
        20..=23 => "Night session - light activities".to_string(),
        _ => "Late night activity - rest recommended".to_string(),
    }
}