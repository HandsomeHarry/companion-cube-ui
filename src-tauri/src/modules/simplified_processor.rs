use crate::modules::activity_watch::TimeframeData;
use crate::modules::productivity_calc::{calculate_productivity_metrics, calculate_focus_score};
use crate::modules::database::PatternDatabase;
use std::collections::HashMap;

pub struct ProcessedData {
    pub metrics: crate::modules::productivity_calc::ProductivityMetrics,
    pub focus_score: u32,
    pub primary_apps: Vec<(String, f64)>, // Top 3 apps by time
    pub activity_summary: String,
}

/// Process raw activity data into clean metrics
pub async fn process_activity_data(
    timeframes: &HashMap<String, TimeframeData>,
    db: &PatternDatabase,
) -> Result<ProcessedData, String> {
    // Get recent data (5 minutes)
    let recent = timeframes.get("5_minutes")
        .ok_or("No recent timeframe data")?;
    
    // Get categories for all apps
    let categories = db.get_all_app_categories().await?;
    let category_map: HashMap<String, (String, Option<String>, i32)> = categories
        .into_iter()
        .map(|(app, cat, subcat, score)| (app, (cat, subcat, score)))
        .collect();
    
    // Convert window events to categorized activities
    let mut categorized_activities = Vec::new();
    for event in &recent.window_events {
        let app_name = event.data.get("app")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        
        let (category, score) = if let Some((cat, _subcat, prod_score)) = category_map.get(app_name) {
            (cat.clone(), Some(prod_score.clone()))
        } else if let Some((cat, _subcat, prod_score)) = crate::modules::default_categories::categorize_app(app_name) {
            (cat.to_string(), Some(prod_score))
        } else {
            ("other".to_string(), None)
        };
        
        categorized_activities.push((
            app_name.to_string(),
            category,
            score,
            event.duration / 60.0 // Convert to minutes
        ));
    }
    
    // Calculate metrics
    let metrics = calculate_productivity_metrics(
        &categorized_activities,
        recent.statistics.context_switches as usize,
        recent.statistics.total_active_minutes / 60.0, // Convert to hours
    );
    
    // Calculate focus score
    let focus_score = calculate_focus_score(
        metrics.work_percentage / 100.0,
        metrics.context_switches_per_hour,
        recent.statistics.unique_apps.len(),
    );
    
    // Get top 3 apps by time
    let mut app_time: HashMap<String, f64> = HashMap::new();
    for (app, _, _, duration) in &categorized_activities {
        *app_time.entry(app.clone()).or_insert(0.0) += duration;
    }
    let mut primary_apps: Vec<(String, f64)> = app_time.into_iter().collect();
    primary_apps.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    primary_apps.truncate(3);
    
    // Create simple activity summary
    let activity_summary = create_activity_summary(&categorized_activities, &metrics);
    
    Ok(ProcessedData {
        metrics,
        focus_score,
        primary_apps,
        activity_summary,
    })
}

fn create_activity_summary(
    activities: &[(String, String, Option<i32>, f64)],
    metrics: &crate::modules::productivity_calc::ProductivityMetrics,
) -> String {
    let total_time = activities.iter().map(|(_, _, _, d)| d).sum::<f64>();
    
    if total_time < 0.5 {
        return "No significant activity detected.".to_string();
    }
    
    // Group by category
    let mut category_time: HashMap<&str, f64> = HashMap::new();
    for (_, category, _, duration) in activities {
        *category_time.entry(category.as_str()).or_insert(0.0) += duration;
    }
    
    // Sort categories by time
    let mut categories: Vec<(&str, f64)> = category_time.into_iter().collect();
    categories.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    
    // Build summary
    let primary = categories.first()
        .map(|(cat, _)| *cat)
        .unwrap_or("various");
    
    match metrics.current_state.as_str() {
        "productive" => format!("Focused on {} activities", primary),
        "moderate" => format!("Working on {} with some distractions", primary),
        "unproductive" => format!("Mostly {} activities", primary),
        "chilling" => format!("Taking a break with {} activities", primary),
        _ => "Away from computer".to_string()
    }
}

/// Create a simplified prompt for insights only
pub fn create_insight_prompt(
    processed: &ProcessedData,
    user_context: &str,
) -> String {
    let top_apps = processed.primary_apps.iter()
        .map(|(app, mins)| format!("{} ({:.0}m)", app, mins))
        .collect::<Vec<_>>()
        .join(", ");
    
    format!(
        r#"Generate encouraging ADHD productivity insights. Be supportive and constructive.

USER: {}
STATE: {} ({}% productive work)
FOCUS: {}%
TOP APPS: {}
PATTERN: {} context switches/hour

Provide a 2-3 sentence insight about their work pattern and one specific, actionable suggestion. Focus on positive reinforcement and practical advice. Address the user directly as "you".

Return JSON only:
{{
  "insight": "Your observation about their current work pattern",
  "suggestion": "One specific, actionable tip",
  "encouragement": "Brief positive reinforcement"
}}"#,
        user_context,
        processed.metrics.current_state,
        processed.metrics.work_percentage as i32,
        processed.focus_score,
        top_apps,
        processed.metrics.context_switches_per_hour as i32
    )
}