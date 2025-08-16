use std::collections::HashMap;
use chrono::{DateTime, Utc};

#[derive(Debug, Clone)]
pub struct ProductivityMetrics {
    pub productive_minutes: f64,
    pub moderate_minutes: f64,
    pub unproductive_minutes: f64,
    pub neutral_minutes: f64,
    pub work_percentage: f64,
    pub distraction_percentage: f64,
    pub neutral_percentage: f64,
    pub current_state: String,
    pub context_switches_per_hour: f64,
}

/// Calculate productivity metrics from categorized activities
pub fn calculate_productivity_metrics(
    activities: &[(String, String, Option<i32>, f64)], // (app_name, category, score, duration_minutes)
    context_switches: usize,
    total_hours: f64,
) -> ProductivityMetrics {
    let mut category_time: HashMap<&str, f64> = HashMap::new();
    let mut total_minutes = 0.0;
    
    // Sum up time by category
    for (_app, category, score, duration) in activities {
        total_minutes += duration;
        
        // Group by productivity level based on score
        let productivity_level = match score {
            Some(s) if *s >= 80 => "productive",
            Some(s) if *s >= 60 => "moderate", 
            Some(s) if *s >= 40 => "neutral",
            Some(s) if *s < 40 => "unproductive",
            _ => match category.as_str() {
                "work" | "development" => "productive",
                "productivity" => "moderate",
                "communication" | "system" => "neutral",
                "entertainment" => "unproductive",
                _ => "neutral"
            }
        };
        
        *category_time.entry(productivity_level).or_insert(0.0) += duration;
    }
    
    // Calculate percentages
    let productive_minutes = *category_time.get("productive").unwrap_or(&0.0);
    let moderate_minutes = *category_time.get("moderate").unwrap_or(&0.0);
    let unproductive_minutes = *category_time.get("unproductive").unwrap_or(&0.0);
    let neutral_minutes = *category_time.get("neutral").unwrap_or(&0.0);
    
    let work_percentage = if total_minutes > 0.0 {
        ((productive_minutes + moderate_minutes) / total_minutes * 100.0).round()
    } else { 0.0 };
    
    let distraction_percentage = if total_minutes > 0.0 {
        (unproductive_minutes / total_minutes * 100.0).round()
    } else { 0.0 };
    
    let neutral_percentage = if total_minutes > 0.0 {
        (neutral_minutes / total_minutes * 100.0).round()
    } else { 0.0 };
    
    // Determine current state based on recent activity (last 5 minutes)
    let current_state = determine_current_state(
        productive_minutes,
        moderate_minutes,
        unproductive_minutes,
        neutral_minutes,
        total_minutes,
        context_switches as f64 / total_hours.max(0.1)
    );
    
    ProductivityMetrics {
        productive_minutes,
        moderate_minutes,
        unproductive_minutes,
        neutral_minutes,
        work_percentage,
        distraction_percentage,
        neutral_percentage,
        current_state,
        context_switches_per_hour: context_switches as f64 / total_hours.max(0.1),
    }
}

/// Determine current state based on activity patterns
fn determine_current_state(
    productive: f64,
    moderate: f64,
    unproductive: f64,
    _neutral: f64,
    total: f64,
    context_switches_per_hour: f64,
) -> String {
    if total < 0.5 {
        return "afk".to_string();
    }
    
    let productive_ratio = (productive + moderate) / total;
    let unproductive_ratio = unproductive / total;
    
    // High context switching indicates distraction
    let is_distracted = context_switches_per_hour > 30.0;
    
    if productive_ratio > 0.7 && !is_distracted {
        "productive".to_string()
    } else if productive_ratio > 0.5 {
        "moderate".to_string()
    } else if unproductive_ratio > 0.5 || is_distracted {
        "unproductive".to_string()
    } else {
        "chilling".to_string()
    }
}

/// Calculate focus score (0-100)
pub fn calculate_focus_score(
    productive_ratio: f64,
    context_switches_per_hour: f64,
    unique_apps: usize,
) -> u32 {
    // Base score from productive time
    let base_score = (productive_ratio * 100.0) as u32;
    
    // Penalty for context switching
    let switch_penalty = (context_switches_per_hour.min(60.0) / 60.0 * 20.0) as u32;
    
    // Penalty for too many apps
    let app_penalty = if unique_apps > 10 { 10 } else { 0 };
    
    base_score.saturating_sub(switch_penalty).saturating_sub(app_penalty)
}

/// Aggregate similar activities (e.g., multiple quick switches to same app)
pub fn aggregate_activities(
    activities: Vec<(DateTime<Utc>, String, String, f64)>
) -> Vec<(String, String, f64, i32)> { // (app_name, title_summary, total_duration, occurrence_count)
    let mut aggregated: HashMap<String, (String, f64, i32)> = HashMap::new();
    
    for (_timestamp, app_name, title, duration) in activities {
        let entry = aggregated.entry(app_name.clone()).or_insert((String::new(), 0.0, 0));
        entry.1 += duration;
        entry.2 += 1;
        
        // Keep the most recent title
        if !title.is_empty() {
            entry.0 = title;
        }
    }
    
    aggregated.into_iter()
        .map(|(app, (title, duration, count))| (app, title, duration, count))
        .filter(|(_, _, duration, _)| *duration > 0.01) // Filter out tiny durations
        .collect()
}