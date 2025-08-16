use crate::modules::activity_watch::TimeframeData;
use crate::modules::productivity_calc::{calculate_productivity_metrics, calculate_focus_score};
use crate::modules::database::PatternDatabase;
use crate::modules::event_processor::{TimelineEvent, ContextSwitch};
use std::collections::HashMap;

pub struct EnhancedAnalysisData {
    pub local_metrics: crate::modules::productivity_calc::ProductivityMetrics,
    pub focus_score: u32,
    pub detailed_timeline: Vec<TimelineEvent>,
    pub context_switches: Vec<ContextSwitch>,
    pub app_categories: HashMap<String, (String, Option<String>, i32)>,
    pub timeframe_stats: HashMap<String, TimeframeStats>,
}

#[derive(Debug, Clone)]
pub struct TimeframeStats {
    pub active_minutes: f64,
    pub unique_apps: usize,
    pub context_switches: usize,
    pub top_apps: Vec<(String, f64)>,
}

/// Process activity data but keep all details for LLM
pub async fn process_for_enhanced_analysis(
    timeframes: &HashMap<String, TimeframeData>,
    db: &PatternDatabase,
) -> Result<EnhancedAnalysisData, String> {
    // Get categories for all apps
    let categories = db.get_all_app_categories().await?;
    let mut category_map: HashMap<String, (String, Option<String>, i32)> = categories
        .into_iter()
        .map(|(app, cat, subcat, score)| (app, (cat, subcat, score)))
        .collect();
    
    // Add default categories for uncategorized apps
    for timeframe_data in timeframes.values() {
        for event in &timeframe_data.window_events {
            if let Some(app_name) = event.data.get("app").and_then(|v| v.as_str()) {
                if !category_map.contains_key(app_name) {
                    if let Some((cat, subcat, score)) = crate::modules::default_categories::categorize_app(app_name) {
                        category_map.insert(app_name.to_string(), (cat.to_string(), subcat.map(|s| s.to_string()), score));
                    }
                }
            }
        }
    }
    
    // Build detailed timeline
    let detailed_timeline = build_detailed_timeline(timeframes, &category_map);
    let context_switches = detect_context_switches(&detailed_timeline);
    
    // Calculate local metrics for the most recent timeframe
    let recent = timeframes.get("5_minutes")
        .ok_or("No recent timeframe data")?;
    
    let mut categorized_activities = Vec::new();
    for event in &recent.window_events {
        let app_name = event.data.get("app")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        
        let (category, score) = if let Some((cat, _subcat, prod_score)) = category_map.get(app_name) {
            (cat.clone(), Some(*prod_score))
        } else {
            ("other".to_string(), None)
        };
        
        categorized_activities.push((
            app_name.to_string(),
            category,
            score,
            event.duration / 60.0
        ));
    }
    
    let local_metrics = calculate_productivity_metrics(
        &categorized_activities,
        recent.statistics.context_switches as usize,
        recent.statistics.total_active_minutes / 60.0,
    );
    
    let focus_score = calculate_focus_score(
        local_metrics.work_percentage / 100.0,
        local_metrics.context_switches_per_hour,
        recent.statistics.unique_apps.len(),
    );
    
    // Build timeframe statistics
    let mut timeframe_stats = HashMap::new();
    for (name, data) in timeframes {
        let mut app_time: HashMap<String, f64> = HashMap::new();
        for event in &data.window_events {
            if let Some(app) = event.data.get("app").and_then(|v| v.as_str()) {
                *app_time.entry(app.to_string()).or_insert(0.0) += event.duration / 60.0;
            }
        }
        
        let mut top_apps: Vec<(String, f64)> = app_time.into_iter().collect();
        top_apps.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        top_apps.truncate(5);
        
        timeframe_stats.insert(name.clone(), TimeframeStats {
            active_minutes: data.statistics.total_active_minutes,
            unique_apps: data.statistics.unique_apps.len(),
            context_switches: data.statistics.context_switches as usize,
            top_apps,
        });
    }
    
    Ok(EnhancedAnalysisData {
        local_metrics,
        focus_score,
        detailed_timeline,
        context_switches,
        app_categories: category_map,
        timeframe_stats,
    })
}

fn build_detailed_timeline(
    timeframes: &HashMap<String, TimeframeData>,
    category_map: &HashMap<String, (String, Option<String>, i32)>,
) -> Vec<TimelineEvent> {
    let mut all_events = Vec::new();
    
    // Get the most detailed recent data
    if let Some(data) = timeframes.get("30_minutes") {
        for event in &data.window_events {
            let app_name = event.data.get("app")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            
            let title = event.data.get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            
            let (category, subcategory, score) = category_map.get(&app_name)
                .cloned()
                .unwrap_or_else(|| ("uncategorized".to_string(), None, 50));
            
            all_events.push(TimelineEvent {
                timestamp: event.timestamp,
                name: app_name.clone(),
                title: title.clone(),
                duration_minutes: event.duration / 60.0,
                category: Some(category),
                subcategory,
                productivity_score: Some(score),
            });
        }
    }
    
    // Sort by timestamp
    all_events.sort_by_key(|e| e.timestamp);
    all_events
}

fn detect_context_switches(timeline: &[TimelineEvent]) -> Vec<ContextSwitch> {
    let mut switches = Vec::new();
    
    for i in 1..timeline.len() {
        let prev = &timeline[i - 1];
        let curr = &timeline[i];
        
        if prev.name != curr.name {
            switches.push(ContextSwitch {
                timestamp: curr.timestamp,
                from_app: prev.name.clone(),
                to_app: curr.name.clone(),
                from_category: prev.category.clone(),
                to_category: curr.category.clone(),
            });
        }
    }
    
    switches
}

/// Create an enhanced prompt with full data for local LLM
pub fn create_enhanced_prompt(
    data: &EnhancedAnalysisData,
    user_context: &str,
) -> String {
    // Format detailed timeline
    let timeline_str = data.detailed_timeline.iter()
        .map(|event| {
            let cat_info = if let Some(ref cat) = event.category {
                format!(" [{}{}]", 
                    cat,
                    event.productivity_score.map(|s| format!(", score:{}", s)).unwrap_or_default()
                )
            } else {
                " [uncategorized]".to_string()
            };
            
            format!("• {} - {}{} → {} ({:.2}min)",
                event.timestamp.format("%H:%M:%S"),
                event.name,
                cat_info,
                if event.title.is_empty() { "No title" } else { &event.title },
                event.duration_minutes
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    
    // Format context switches
    let switches_str = data.context_switches.iter()
        .map(|switch| {
            format!("• {} → {} at {}",
                switch.from_app,
                switch.to_app,
                switch.timestamp.format("%H:%M:%S")
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    
    // Format timeframe comparisons
    let timeframe_comparison = data.timeframe_stats.iter()
        .map(|(name, stats)| {
            format!("{}: {:.0}min active, {} apps, {} switches",
                name,
                stats.active_minutes,
                stats.unique_apps,
                stats.context_switches
            )
        })
        .collect::<Vec<_>>()
        .join(" | ");
    
    let prompt_str = format!(
        r#"Analyze ADHD user's detailed activity patterns. You have full timeline access. Be specific and insightful.

USER CONTEXT: {}

LOCAL METRICS (calculated):
- Current State: {} ({}% productive work)
- Focus Score: {}%
- Context Switches/hr: {:.0}
- Work: {}%, Distraction: {}%, Neutral: {}%

TIMEFRAME COMPARISON:
{}

DETAILED ACTIVITY TIMELINE (last 30 min):
{}

ALL CONTEXT SWITCHES:
{}

ANALYSIS REQUIREMENTS:
1. Identify specific productivity patterns (not just general state)
2. Spot potential rabbit holes or hyperfocus sessions
3. Analyze task fragmentation and attention span
4. Identify productive vs unproductive app combinations
5. Suggest specific interventions based on actual behavior
6. Consider ADHD-specific patterns (hyperfocus, task switching, dopamine seeking)

IMPORTANT: Write concise, informative summaries that give the user clear insight into their activity patterns. Keep it to 2-3 sentences that capture the key patterns and productivity insights.

Return ONLY this JSON (no other text):
{{
  "current_state": "productive|moderate|chilling|unproductive|afk",
  "confidence": "high|medium|low",
  "primary_activity": "Brief description of main activity",
  "professional_summary": "Write a concise 2-3 sentence summary. Include key apps and main productivity pattern. For example: You spent most time coding in VS Code with moderate focus, interrupted by frequent Weixin checks. Consider batching communication to maintain deeper focus periods.",
  "work_score": 50,
  "distraction_score": 30,
  "neutral_score": 20,
  "focus_trend": "maintaining_focus",
  "distraction_trend": "low",
  "reasoning": "Explanation of scores and state"
}}"#,
        user_context,
        data.local_metrics.current_state,
        data.local_metrics.work_percentage as i32,
        data.focus_score,
        data.local_metrics.context_switches_per_hour,
        data.local_metrics.work_percentage as i32,
        data.local_metrics.distraction_percentage as i32,
        data.local_metrics.neutral_percentage as i32,
        timeframe_comparison,
        timeline_str,
        switches_str
    );
    
    eprintln!("[ENHANCED PROMPT] Length: {} chars", prompt_str.len());
    eprintln!("[ENHANCED PROMPT] Contains professional_summary instruction: {}", 
        prompt_str.contains("4-5 sentence detailed summary"));
    
    prompt_str
}