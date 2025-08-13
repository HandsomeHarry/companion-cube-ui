use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use crate::modules::activity_watch::TimeframeData;
use crate::modules::advanced_analyzer::{AdvancedAnalyzer, AdvancedAnalysis};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineEvent {
    pub timestamp: DateTime<Utc>,
    pub name: String,
    pub title: String,
    pub duration_minutes: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextSwitch {
    pub timestamp: DateTime<Utc>,
    pub from_app: String,
    pub to_app: String,
}

#[derive(Debug, Clone)]
pub struct RawDataForLLM {
    pub timeframes: HashMap<String, TimeframeData>,
    pub activity_timeline: Vec<TimelineEvent>,
    pub context_switches: Vec<ContextSwitch>,
    pub advanced_analysis: Option<AdvancedAnalysis>,
}

pub struct EventProcessor;

impl EventProcessor {
    pub fn new() -> Self {
        Self
    }
    
    pub fn prepare_raw_data_for_llm(&self, timeframes: &HashMap<String, TimeframeData>) -> RawDataForLLM {
        let activity_timeline = self.build_activity_timeline(timeframes);
        let context_switches = self.detect_context_switches(timeframes);
        
        RawDataForLLM {
            timeframes: timeframes.clone(),
            activity_timeline,
            context_switches,
            advanced_analysis: None, // Deprecated - use prepare_raw_data_with_advanced_analysis instead
        }
    }
    
    pub fn prepare_raw_data_with_advanced_analysis(
        &self, 
        timeframes: &HashMap<String, TimeframeData>,
        user_context: &str
    ) -> RawDataForLLM {
        let mut raw_data = self.prepare_raw_data_for_llm(timeframes);
        
        // Get all events for advanced analysis - use today's data for comprehensive analysis
        let mut all_events = Vec::new();
        if let Some(today_data) = timeframes.get("today") {
            all_events.extend(today_data.window_events.clone());
        } else if let Some(hour_data) = timeframes.get("1_hour") {
            all_events.extend(hour_data.window_events.clone());
        }
        
        // Always perform advanced analysis for ADHD support
        let analyzer = AdvancedAnalyzer::new();
        let advanced = analyzer.analyze_patterns(&all_events, user_context);
        raw_data.advanced_analysis = Some(advanced);
        
        raw_data
    }
    
    pub async fn create_state_analysis_prompt_with_categories(
        &self, 
        raw_data: &RawDataForLLM, 
        user_context: &str,
        db: &crate::modules::database::PatternDatabase
    ) -> String {
        // Get all app categories for reference
        let app_categories = db.get_all_app_categories().await
            .unwrap_or_else(|_| Vec::new());
        
        let category_map: std::collections::HashMap<String, (String, Option<String>, i32)> = app_categories
            .into_iter()
            .map(|(app, cat, subcat, score)| (app, (cat, subcat, score)))
            .collect();
        
        // Format the timeline with categories
        let timeline_with_categories = self.format_timeline_with_categories(&raw_data.activity_timeline, &category_map);
        
        let recent_timeframe = raw_data.timeframes.get("5_minutes");
        let medium_timeframe = raw_data.timeframes.get("30_minutes");
        let hour_timeframe = raw_data.timeframes.get("1_hour");
        let today_timeframe = raw_data.timeframes.get("today");
        
        let recent_stats = recent_timeframe
            .map(|tf| &tf.statistics)
            .cloned()
            .unwrap_or_default();
        let medium_stats = medium_timeframe
            .map(|tf| &tf.statistics)
            .cloned()
            .unwrap_or_default();
        let hour_stats = hour_timeframe
            .map(|tf| &tf.statistics)
            .cloned()
            .unwrap_or_default();
        let today_stats = today_timeframe
            .map(|tf| &tf.statistics)
            .cloned()
            .unwrap_or_default();
        
        let context_switches = &raw_data.context_switches;
        
        // Check AFK status
        let is_currently_afk = recent_timeframe
            .map(|tf| {
                tf.window_events.is_empty() || 
                tf.statistics.total_active_minutes < 0.5
            })
            .unwrap_or(false);
        
        format!(
            r#"Analyze ADHD productivity state with app categories. Return ONLY JSON, no other text.

USER CONTEXT: {}
STATUS: {} | Recent: {}m active, {} switches, {} apps | Last 30min: {}m active, {} switches, {} apps | Last hour: {}m active, {} switches | Today: {}m active, {} apps

RECENT ACTIVITY WITH CATEGORIES:
{}

CONTEXT SWITCHES:
{}

APP CATEGORY LEGEND:
- work: Productive work applications (productivity score: 80-100)
- development: Programming and development tools (productivity score: 90-100)  
- communication: Chat, email, video calls (productivity score: 40-60)
- entertainment: Games, videos, social media (productivity score: 0-30)
- productivity: Tools and utilities (productivity score: 60-80)
- system: OS and system utilities (productivity score: 50)
- other: Uncategorized apps

EVALUATE BASED ON:
1. Category transitions - moving between work/entertainment/communication
2. Time spent in each category (use exact categories shown above)
3. Productivity scores of apps being used
4. Context appropriateness for user's role
5. Natural work sessions and break patterns

Return JSON only:
{{
  "current_state": "flow|working|needs_nudge|afk",
  "focus_trend": "maintaining_focus|entering_focus|losing_focus|variable|none",
  "distraction_trend": "low|moderate|increasing|decreasing|high",
  "confidence": "high|medium|low",
  "primary_activity": "Brief description of main activity with category",
  "professional_summary": "A detailed 3-sentence professional summary: First sentence describes the primary applications and their categories engaged with during this period. Second sentence analyzes the work pattern and productivity level based on category transitions and time allocation. Third sentence provides insight on focus state and recommendations for improvement or continuation of current workflow.",
  "work_score": 0-100 (percentage of time in work/development categories),
  "distraction_score": 0-100 (percentage of time in entertainment category),
  "neutral_score": 0-100 (percentage of time in communication/system/other),
  "reasoning": "Clear explanation including app categories used"
}}"#,
            user_context,
            if is_currently_afk { "AFK" } else { "Active" },
            recent_stats.total_active_minutes,
            recent_stats.context_switches,
            recent_stats.unique_apps.len(),
            medium_stats.total_active_minutes,
            medium_stats.context_switches,
            medium_stats.unique_apps.len(),
            hour_stats.total_active_minutes,
            hour_stats.context_switches,
            today_stats.total_active_minutes,
            today_stats.unique_apps.len(),
            timeline_with_categories,
            self.format_context_switches_for_prompt(context_switches)
        )
    }
    
    fn format_timeline_with_categories(
        &self, 
        timeline: &[TimelineEvent], 
        category_map: &std::collections::HashMap<String, (String, Option<String>, i32)>
    ) -> String {
        if timeline.is_empty() {
            return "No activity detected".to_string();
        }
        
        let mut formatted = Vec::new();
        let events_to_show = if timeline.len() > 20 { 20 } else { timeline.len() };
        
        for event in timeline.iter().rev().take(events_to_show).rev() {
            let title_part = if event.title.is_empty() { "" } else { &format!(" → {}", event.title) };
            let (app_name, _exe_name) = crate::modules::utils::extract_app_and_exe_name(&event.name);
            
            let category_info = category_map.get(&event.name)
                .or_else(|| category_map.get(&app_name))
                .map(|(cat, subcat, score)| {
                    if let Some(sub) = subcat {
                        format!(" [{}:{}, score:{}]", cat, sub, score)
                    } else {
                        format!(" [{}, score:{}]", cat, score)
                    }
                })
                .unwrap_or_else(|| " [uncategorized]".to_string());
            
            formatted.push(format!(
                "• {} - {}{}{} ({}min)",
                event.timestamp.format("%H:%M"),
                app_name,
                category_info,
                title_part,
                event.duration_minutes
            ));
        }
        
        formatted.join("\n")
    }
    
    pub fn create_state_analysis_prompt(&self, raw_data: &RawDataForLLM, user_context: &str) -> String {
        let recent_timeframe = raw_data.timeframes.get("5_minutes");
        let medium_timeframe = raw_data.timeframes.get("30_minutes");
        let hour_timeframe = raw_data.timeframes.get("1_hour");
        let today_timeframe = raw_data.timeframes.get("today");
        
        let recent_stats = recent_timeframe
            .map(|tf| &tf.statistics)
            .cloned()
            .unwrap_or_default();
        let medium_stats = medium_timeframe
            .map(|tf| &tf.statistics)
            .cloned()
            .unwrap_or_default();
        let hour_stats = hour_timeframe
            .map(|tf| &tf.statistics)
            .cloned()
            .unwrap_or_default();
        let today_stats = today_timeframe
            .map(|tf| &tf.statistics)
            .cloned()
            .unwrap_or_default();
        
        let timeline = &raw_data.activity_timeline;
        let context_switches = &raw_data.context_switches;
        
        // Check AFK status
        let is_currently_afk = recent_timeframe
            .map(|tf| {
                if tf.afk_events.is_empty() {
                    false
                } else {
                    tf.afk_events.iter()
                        .max_by_key(|e| e.timestamp)
                        .and_then(|e| e.data.get("status"))
                        .and_then(|s| s.as_str())
                        .map(|s| s == "afk")
                        .unwrap_or(false)
                }
            })
            .unwrap_or(false);
        
        format!(
            r#"Analyze ADHD productivity state. Return ONLY JSON, no other text.

USER CONTEXT: {}
STATUS: {} | Recent: {}m active, {} switches, {} apps | Last 30min: {}m active, {} switches, {} apps | Last hour: {}m active, {} switches | Today: {}m active, {} apps

RECENT ACTIVITY:
{}

CONTEXT SWITCHES:
{}

EVALUATE BASED ON:
1. Semantic coherence - are activities following a logical thread?
2. Return-to-task patterns - quick checks vs true distractions
3. Natural work sessions and break patterns
4. Context appropriateness for user's role
5. Fatigue indicators and time since last break

Return JSON only:
{{
  "current_state": "flow|working|needs_nudge|afk",
  "focus_trend": "maintaining_focus|entering_focus|losing_focus|variable|none",
  "distraction_trend": "low|moderate|increasing|decreasing|high",
  "confidence": "high|medium|low",
  "primary_activity": "Brief description of main activity",
  "professional_summary": "A detailed 3-sentence professional summary: First sentence describes the primary applications and tasks engaged with during this period. Second sentence analyzes the work pattern and productivity level based on context switches and time allocation. Third sentence provides insight on focus state and recommendations for improvement or continuation of current workflow.",
  "work_score": 0-100 (percentage of time spent on productive work),
  "distraction_score": 0-100 (percentage of time spent on distractions),
  "neutral_score": 0-100 (percentage of time on neutral activities like breaks),
  "reasoning": "Clear explanation of state assessment"
}}"#,
            user_context,
            if is_currently_afk { "AFK" } else { "Active" },
            recent_stats.total_active_minutes,
            recent_stats.context_switches,
            recent_stats.unique_apps.len(),
            medium_stats.total_active_minutes,
            medium_stats.context_switches,
            medium_stats.unique_apps.len(),
            hour_stats.total_active_minutes,
            hour_stats.context_switches,
            today_stats.total_active_minutes,
            today_stats.unique_apps.len(),
            self.format_timeline_for_prompt(timeline),
            self.format_context_switches_for_prompt(context_switches)
        )
    }
    
    pub fn create_enhanced_analysis_prompt(&self, raw_data: &RawDataForLLM, user_context: &str) -> String {
        let base_prompt = self.create_state_analysis_prompt(raw_data, user_context);
        
        // If we have advanced analysis, add it to the prompt
        if let Some(ref advanced) = raw_data.advanced_analysis {
            let advanced_section = format!(
                r#"
ADVANCED PATTERNS DETECTED:

RABBIT HOLE ANALYSIS:
- Semantic coherence: {:.2} (1.0 = focused, 0.0 = completely drifted)
- Topic drift: {} → {} 
- Drift severity: {}
- Is rabbit hole: {}

RETURN-TO-TASK METRICS:
- Average return time: {:.0}s
- Quick reference checks: {}
- True distractions: {}

FATIGUE ANALYSIS:
- Fatigue level: {}
- Time since break: {:.0} minutes
- Continuous work: {:.0} minutes
- Break urgency: {}
- Recommendation: {}

CONTEXT ASSESSMENT:
- Role: {}
- Context appropriateness: {:.2}
- Assessment: {}

Consider these advanced patterns when making your assessment. If fatigue is high or break is urgent, emphasize this in your professional_summary."#,
                advanced.rabbit_hole_detection.semantic_coherence_score,
                advanced.rabbit_hole_detection.initial_topic,
                advanced.rabbit_hole_detection.current_topic,
                advanced.rabbit_hole_detection.drift_severity,
                advanced.rabbit_hole_detection.is_rabbit_hole,
                advanced.return_to_task_metrics.average_return_time_seconds,
                advanced.return_to_task_metrics.quick_reference_checks,
                advanced.return_to_task_metrics.true_distractions,
                advanced.fatigue_analysis.fatigue_level,
                advanced.fatigue_analysis.time_since_break_minutes,
                advanced.fatigue_analysis.continuous_work_minutes,
                advanced.fatigue_analysis.break_urgency,
                advanced.fatigue_analysis.recommended_action,
                advanced.context_aware_assessment.user_role_context,
                advanced.context_aware_assessment.context_appropriate_score,
                advanced.context_aware_assessment.assessment
            );
            
            // Insert the advanced section before the JSON format specification
            base_prompt.replace("Return JSON only:", &format!("{}\n\nReturn JSON only:", advanced_section))
        } else {
            base_prompt
        }
    }
    
    fn build_activity_timeline(&self, timeframes: &HashMap<String, TimeframeData>) -> Vec<TimelineEvent> {
        let mut timeline = Vec::new();
        
        // Include data from today for comprehensive daily summary
        if let Some(today_data) = timeframes.get("today") {
            for event in &today_data.window_events {
                if let Some(app) = event.data.get("app").and_then(|v| v.as_str()) {
                    timeline.push(TimelineEvent {
                        timestamp: event.timestamp,
                        name: app.to_string(),
                        title: event.data.get("title")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        duration_minutes: event.duration / 60.0,
                    });
                }
            }
        } else if let Some(recent) = timeframes.get("30_minutes") {
            // Fallback to 30 minutes if today data not available
            for event in &recent.window_events {
                if let Some(app) = event.data.get("app").and_then(|v| v.as_str()) {
                    timeline.push(TimelineEvent {
                        timestamp: event.timestamp,
                        name: app.to_string(),
                        title: event.data.get("title")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        duration_minutes: event.duration / 60.0,
                    });
                }
            }
        }
        
        timeline.sort_by_key(|e| e.timestamp);
        timeline
    }
    
    fn detect_context_switches(&self, timeframes: &HashMap<String, TimeframeData>) -> Vec<ContextSwitch> {
        let mut switches = Vec::new();
        
        if let Some(recent) = timeframes.get("30_minutes") {
            let mut last_app: Option<&str> = None;
            
            for event in &recent.window_events {
                if let Some(app) = event.data.get("app").and_then(|v| v.as_str()) {
                    if let Some(prev) = last_app {
                        if prev != app {
                            switches.push(ContextSwitch {
                                timestamp: event.timestamp,
                                from_app: prev.to_string(),
                                to_app: app.to_string(),
                            });
                        }
                    }
                    last_app = Some(app);
                }
            }
        }
        
        switches
    }
    
    fn format_timeline_for_prompt(&self, timeline: &[TimelineEvent]) -> String {
        if timeline.is_empty() {
            return "No activity detected".to_string();
        }
        
        let mut formatted = Vec::new();
        // Show more events if we have today's data
        let events_to_show = if timeline.len() > 20 { 20 } else { timeline.len() };
        for event in timeline.iter().rev().take(events_to_show).rev() {
            let title_part = if event.title.is_empty() { "" } else { &format!(" → {}", event.title) };
            let (app_name, _exe_name) = crate::modules::utils::extract_app_and_exe_name(&event.name);
            formatted.push(format!(
                "• {} - {}{} ({}min)",
                event.timestamp.format("%H:%M"),
                app_name,
                title_part,
                event.duration_minutes
            ));
        }
        
        formatted.join("\n")
    }
    
    fn format_context_switches_for_prompt(&self, switches: &[ContextSwitch]) -> String {
        if switches.is_empty() {
            return "No context switches detected".to_string();
        }
        
        let mut formatted = Vec::new();
        for switch in switches.iter().take(5) {
            let (from_app, _from_exe) = crate::modules::utils::extract_app_and_exe_name(&switch.from_app);
            let (to_app, _to_exe) = crate::modules::utils::extract_app_and_exe_name(&switch.to_app);
            formatted.push(format!(
                "• {} → {} at {}",
                from_app,
                to_app,
                switch.timestamp.format("%H:%M")
            ));
        }
        
        formatted.join("\n")
    }
}

impl Default for crate::modules::activity_watch::TimeframeStatistics {
    fn default() -> Self {
        Self {
            total_events: 0,
            unique_apps: std::collections::HashSet::new(),
            total_active_minutes: 0.0,
            context_switches: 0,
        }
    }
}

// Clone implementation removed - using derive Clone instead