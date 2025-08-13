use std::sync::Arc;
use tokio::sync::Mutex;
use std::collections::HashMap;
use chrono::{DateTime, Utc};
use crate::modules::pattern_analyzer::{PatternAnalyzer, UserBaseline};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HourlySummary {
    pub summary: String,
    pub focus_score: u32,
    pub last_updated: String,
    pub period: String,
    pub current_state: String,
    #[serde(default)]
    pub work_score: u32,
    #[serde(default)]
    pub distraction_score: u32,
    #[serde(default)]
    pub neutral_score: u32,
}

pub struct AppState {
    pub current_mode: Arc<Mutex<String>>,
    pub last_summary_time: Arc<Mutex<HashMap<String, DateTime<Utc>>>>,
    pub latest_hourly_summary: Arc<Mutex<Option<HourlySummary>>>,
    pub pattern_analyzer: Arc<PatternAnalyzer>,
    pub pattern_database: Arc<crate::modules::database::PatternDatabase>,
    pub user_baseline: Arc<Mutex<Option<UserBaseline>>>,
    pub last_llm_call: Arc<Mutex<Option<DateTime<Utc>>>>,
}

impl AppState {
    pub async fn new() -> Result<Self, String> {
        let saved_mode = Self::load_mode().unwrap_or_else(|| "coach".to_string());
        
        // Initialize pattern database
        let db_path = dirs::config_dir()
            .ok_or("Could not find config directory")?
            .join("companion-cube")
            .join("patterns.db");
        
        std::fs::create_dir_all(db_path.parent().unwrap()).map_err(|e| e.to_string())?;
        
        let pattern_database = Arc::new(
            crate::modules::database::PatternDatabase::new(db_path.to_str().unwrap()).await?
        );
        
        let pattern_analyzer = Arc::new(
            PatternAnalyzer::new(db_path.to_str().unwrap().to_string())
        );
        
        // Load existing baseline if available and set it in the pattern analyzer
        let user_baseline = if let Ok(Some(baseline)) = pattern_database.get_baseline().await {
            pattern_analyzer.set_baseline(baseline.clone()).await;
            Arc::new(Mutex::new(Some(baseline)))
        } else {
            Arc::new(Mutex::new(None))
        };
        
        let state = Self {
            current_mode: Arc::new(Mutex::new(saved_mode)),
            last_summary_time: Arc::new(Mutex::new(HashMap::new())),
            latest_hourly_summary: Arc::new(Mutex::new(None)),
            pattern_analyzer,
            pattern_database,
            user_baseline,
            last_llm_call: Arc::new(Mutex::new(None)),
        };
        
        // Start background sync task
        let db_clone = state.pattern_database.clone();
        tokio::spawn(async move {
            Self::background_activity_sync(db_clone).await;
        });
        
        Ok(state)
    }
    
    async fn background_activity_sync(db: Arc<crate::modules::database::PatternDatabase>) {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(300)); // 5 minutes
        
        loop {
            interval.tick().await;
            
            // Get recent activities from ActivityWatch
            if let Err(e) = Self::sync_recent_activities(&db).await {
                eprintln!("Failed to sync activities: {}", e);
            }
            
            // Auto-categorize new apps
            if let Err(e) = Self::auto_categorize_apps(&db).await {
                eprintln!("Failed to auto-categorize apps: {}", e);
            }
        }
    }
    
    async fn sync_recent_activities(db: &Arc<crate::modules::database::PatternDatabase>) -> Result<(), String> {
        let aw_client = crate::modules::utils::get_configured_aw_client().await;
        
        // Get activities from the last hour
        let end = chrono::Utc::now();
        let start = end - chrono::Duration::hours(1);
        
        match aw_client.get_active_window_events(start, end).await {
            Ok(events) => {
                let _count = db.store_activities(&events).await?;
                // Silent sync - only log errors
                Ok(())
            }
            Err(e) => Err(format!("Failed to get events from ActivityWatch: {}", e))
        }
    }
    
    pub async fn auto_categorize_apps(db: &Arc<crate::modules::database::PatternDatabase>) -> Result<(), String> {
        let uncategorized = db.get_uncategorized_apps().await?;
        
        if uncategorized.is_empty() {
            return Ok(());
        }
        
        // Only categorize up to 10 apps at a time to avoid overwhelming the LLM
        let apps_to_categorize: Vec<_> = uncategorized.into_iter().take(10).collect();
        
        if crate::modules::ai_integration::test_ollama_connection().await {
            let prompt = format!(
                "Categorize these applications into one of these categories: work, communication, entertainment, development, productivity, system, other.
                
Apps to categorize:
{}

Respond with JSON only, in this format:
{{
  \"app_name\": {{\"category\": \"category_name\", \"subcategory\": \"optional_subcategory\", \"productivity_score\": 0-100}}
}}

Example:
{{
  \"Discord.exe\": {{\"category\": \"communication\", \"subcategory\": \"chat\", \"productivity_score\": 40}},
  \"Code.exe\": {{\"category\": \"development\", \"subcategory\": \"ide\", \"productivity_score\": 90}}
}}",
                apps_to_categorize.join("\n")
            );
            
            match crate::modules::ai_integration::call_ollama_api(&prompt).await {
                Ok(response) => {
                    if let Ok(categories) = serde_json::from_str::<serde_json::Value>(&response) {
                        if let Some(obj) = categories.as_object() {
                            for (app_name, data) in obj {
                                if let Some(cat_obj) = data.as_object() {
                                    let category = cat_obj.get("category")
                                        .and_then(|c| c.as_str())
                                        .unwrap_or("other");
                                    let subcategory = cat_obj.get("subcategory")
                                        .and_then(|s| s.as_str());
                                    let productivity_score = cat_obj.get("productivity_score")
                                        .and_then(|p| p.as_i64())
                                        .map(|p| p as i32);
                                    
                                    let _ = db.set_app_category(
                                        app_name,
                                        category,
                                        subcategory,
                                        productivity_score,
                                        true // auto_detected
                                    ).await;
                                }
                            }
                            // Silent categorization
                        }
                    }
                }
                Err(_) => {} // Silent failure - will retry next cycle
            }
        }
        
        Ok(())
    }
    
    pub fn load_mode() -> Option<String> {
        let config_dir = dirs::config_dir()?.join("companion-cube");
        let mode_file = config_dir.join("mode.txt");
        std::fs::read_to_string(mode_file).ok()
    }
    
    pub fn save_mode(mode: &str) -> Result<(), String> {
        let config_dir = dirs::config_dir()
            .ok_or("Could not find config directory")?
            .join("companion-cube");
        std::fs::create_dir_all(&config_dir).map_err(|e| e.to_string())?;
        let mode_file = config_dir.join("mode.txt");
        std::fs::write(mode_file, mode).map_err(|e| e.to_string())?;
        Ok(())
    }
}