use chrono::{DateTime, Utc, Duration};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdvancedAnalysis {
    pub rabbit_hole_detection: RabbitHoleAnalysis,
    pub return_to_task_metrics: ReturnToTaskMetrics,
    pub session_boundaries: Vec<WorkSession>,
    pub context_aware_assessment: ContextAssessment,
    pub fatigue_analysis: FatigueAnalysis,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RabbitHoleAnalysis {
    pub is_rabbit_hole: bool,
    pub semantic_coherence_score: f64, // 0-1, higher means more focused
    pub topic_drift_path: Vec<String>,
    pub initial_topic: String,
    pub current_topic: String,
    pub drift_severity: String, // "none", "mild", "moderate", "severe"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReturnToTaskMetrics {
    pub average_return_time_seconds: f64,
    pub distraction_events: Vec<DistractionEvent>,
    pub quick_reference_checks: u32,
    pub true_distractions: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DistractionEvent {
    pub timestamp: DateTime<Utc>,
    pub from_app: String,
    pub distraction_app: String,
    pub duration_seconds: f64,
    pub return_time_seconds: Option<f64>,
    pub classification: String, // "quick_check", "distraction", "task_switch"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkSession {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub duration_minutes: f64,
    pub primary_apps: Vec<String>,
    pub focus_score: f64,
    pub session_type: String, // "deep_work", "shallow_work", "mixed", "break"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextAssessment {
    pub user_role_context: String,
    pub expected_apps: Vec<String>,
    pub distraction_apps: Vec<String>,
    pub context_appropriate_score: f64,
    pub assessment: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FatigueAnalysis {
    pub fatigue_level: String, // "low", "moderate", "high", "critical"
    pub time_since_break_minutes: f64,
    pub continuous_work_minutes: f64,
    pub focus_degradation_rate: f64,
    pub recommended_action: String,
    pub break_urgency: String, // "none", "suggested", "recommended", "urgent"
}

pub struct AdvancedAnalyzer;

impl AdvancedAnalyzer {
    pub fn new() -> Self {
        Self
    }

    pub fn analyze_patterns(
        &self,
        events: &[crate::modules::activity_watch::Event],
        user_context: &str,
    ) -> AdvancedAnalysis {
        let rabbit_hole = self.detect_rabbit_holes(events);
        let return_metrics = self.analyze_return_to_task(events);
        let sessions = self.detect_session_boundaries(events);
        let context_assessment = self.assess_context_appropriateness(events, user_context);
        let fatigue = self.analyze_fatigue_patterns(events, &sessions);

        AdvancedAnalysis {
            rabbit_hole_detection: rabbit_hole,
            return_to_task_metrics: return_metrics,
            session_boundaries: sessions,
            context_aware_assessment: context_assessment,
            fatigue_analysis: fatigue,
        }
    }

    fn detect_rabbit_holes(&self, events: &[crate::modules::activity_watch::Event]) -> RabbitHoleAnalysis {
        // Analyze browser history and app switches for semantic drift
        let _topic_path: Vec<String> = Vec::new();
        let mut browser_events: Vec<(DateTime<Utc>, String)> = Vec::new();
        
        for event in events {
            if let Some(app) = event.data.get("app").and_then(|v| v.as_str()) {
                if app.to_lowercase().contains("browser") || app.to_lowercase().contains("chrome") || 
                   app.to_lowercase().contains("firefox") || app.to_lowercase().contains("edge") {
                    if let Some(title) = event.data.get("title").and_then(|v| v.as_str()) {
                        browser_events.push((event.timestamp, title.to_string()));
                    }
                }
            }
        }

        // Simple semantic analysis based on title keywords
        let (coherence_score, drift_path) = self.analyze_semantic_coherence(&browser_events);
        
        let initial_topic = browser_events.first()
            .map(|(_, title)| self.extract_topic(title))
            .unwrap_or_else(|| "Unknown".to_string());
            
        let current_topic = browser_events.last()
            .map(|(_, title)| self.extract_topic(title))
            .unwrap_or_else(|| "Unknown".to_string());

        let drift_severity = match coherence_score {
            x if x > 0.8 => "none",
            x if x > 0.6 => "mild",
            x if x > 0.4 => "moderate",
            _ => "severe",
        }.to_string();

        RabbitHoleAnalysis {
            is_rabbit_hole: coherence_score < 0.6 && browser_events.len() > 5,
            semantic_coherence_score: coherence_score,
            topic_drift_path: drift_path,
            initial_topic,
            current_topic,
            drift_severity,
        }
    }

    fn analyze_semantic_coherence(&self, browser_events: &[(DateTime<Utc>, String)]) -> (f64, Vec<String>) {
        if browser_events.len() < 2 {
            return (1.0, vec![]);
        }

        let mut topics = Vec::new();
        let mut coherence_scores = Vec::new();
        
        for (_, title) in browser_events {
            let topic = self.extract_topic(title);
            topics.push(topic.clone());
        }

        // Calculate coherence between consecutive topics
        for i in 1..topics.len() {
            let similarity = self.calculate_topic_similarity(&topics[i-1], &topics[i]);
            coherence_scores.push(similarity);
        }

        let avg_coherence = if coherence_scores.is_empty() {
            1.0
        } else {
            coherence_scores.iter().sum::<f64>() / coherence_scores.len() as f64
        };

        // Create drift path showing major topic changes
        let mut drift_path = vec![topics[0].clone()];
        for i in 1..topics.len() {
            if self.calculate_topic_similarity(&topics[i-1], &topics[i]) < 0.5 {
                drift_path.push(topics[i].clone());
            }
        }

        (avg_coherence, drift_path)
    }

    fn extract_topic(&self, title: &str) -> String {
        // Simple topic extraction based on keywords
        let title_lower = title.to_lowercase();
        
        if title_lower.contains("python") || title_lower.contains("programming") || 
           title_lower.contains("code") || title_lower.contains("async") {
            "Programming".to_string()
        } else if title_lower.contains("wikipedia") {
            if title_lower.contains("history") {
                "History".to_string()
            } else if title_lower.contains("science") {
                "Science".to_string()
            } else {
                "General Knowledge".to_string()
            }
        } else if title_lower.contains("youtube") || title_lower.contains("reddit") || 
                  title_lower.contains("twitter") || title_lower.contains("facebook") {
            "Social Media".to_string()
        } else if title_lower.contains("news") {
            "News".to_string()
        } else if title_lower.contains("email") || title_lower.contains("gmail") {
            "Email".to_string()
        } else if title_lower.contains("docs") || title_lower.contains("document") {
            "Documentation".to_string()
        } else {
            "Other".to_string()
        }
    }

    fn calculate_topic_similarity(&self, topic1: &str, topic2: &str) -> f64 {
        if topic1 == topic2 {
            1.0
        } else if (topic1 == "Programming" && topic2 == "Documentation") ||
                  (topic2 == "Programming" && topic1 == "Documentation") {
            0.8 // Related topics
        } else {
            0.2 // Different topics
        }
    }

    fn analyze_return_to_task(&self, events: &[crate::modules::activity_watch::Event]) -> ReturnToTaskMetrics {
        let mut distraction_events = Vec::new();
        let mut primary_apps: HashMap<String, f64> = HashMap::new();
        
        // Find primary apps (most used)
        for event in events {
            if let Some(app) = event.data.get("app").and_then(|v| v.as_str()) {
                *primary_apps.entry(app.to_string()).or_insert(0.0) += event.duration;
            }
        }
        
        // Get top 3 apps as primary
        let mut app_times: Vec<_> = primary_apps.iter().collect();
        app_times.sort_by(|a, b| b.1.partial_cmp(a.1).unwrap());
        let primary_app_names: Vec<String> = app_times.iter()
            .take(3)
            .map(|(app, _)| app.to_string())
            .collect();

        // Detect distractions and returns
        for i in 1..events.len() {
            if let (Some(prev_app), Some(curr_app)) = (
                events[i-1].data.get("app").and_then(|v| v.as_str()),
                events[i].data.get("app").and_then(|v| v.as_str())
            ) {
                // Check if moving from primary app to potential distraction
                if primary_app_names.contains(&prev_app.to_string()) && 
                   !primary_app_names.contains(&curr_app.to_string()) &&
                   self.is_potential_distraction(curr_app) {
                    
                    // Look for return to task
                    let mut return_time = None;
                    for j in (i+1)..events.len() {
                        if let Some(future_app) = events[j].data.get("app").and_then(|v| v.as_str()) {
                            if primary_app_names.contains(&future_app.to_string()) {
                                let distraction_duration = (events[j].timestamp - events[i].timestamp).num_seconds() as f64;
                                return_time = Some(distraction_duration);
                                break;
                            }
                        }
                    }
                    
                    let distraction_duration = events[i].duration;
                    let classification = if distraction_duration < 30.0 {
                        "quick_check".to_string()
                    } else if return_time.is_some() {
                        "distraction".to_string()
                    } else {
                        "task_switch".to_string()
                    };
                    
                    distraction_events.push(DistractionEvent {
                        timestamp: events[i].timestamp,
                        from_app: prev_app.to_string(),
                        distraction_app: curr_app.to_string(),
                        duration_seconds: distraction_duration,
                        return_time_seconds: return_time,
                        classification,
                    });
                }
            }
        }

        let quick_checks = distraction_events.iter()
            .filter(|e| e.classification == "quick_check")
            .count() as u32;
            
        let true_distractions = distraction_events.iter()
            .filter(|e| e.classification == "distraction")
            .count() as u32;
            
        let avg_return_time = distraction_events.iter()
            .filter_map(|e| e.return_time_seconds)
            .collect::<Vec<f64>>();
            
        let avg_return = if avg_return_time.is_empty() {
            0.0
        } else {
            avg_return_time.iter().sum::<f64>() / avg_return_time.len() as f64
        };

        ReturnToTaskMetrics {
            average_return_time_seconds: avg_return,
            distraction_events,
            quick_reference_checks: quick_checks,
            true_distractions,
        }
    }

    fn is_potential_distraction(&self, app: &str) -> bool {
        let app_lower = app.to_lowercase();
        app_lower.contains("youtube") || 
        app_lower.contains("reddit") || 
        app_lower.contains("twitter") || 
        app_lower.contains("facebook") ||
        app_lower.contains("instagram") ||
        app_lower.contains("tiktok") ||
        app_lower.contains("discord") ||
        app_lower.contains("slack") ||
        app_lower.contains("whatsapp")
    }

    fn detect_session_boundaries(&self, events: &[crate::modules::activity_watch::Event]) -> Vec<WorkSession> {
        let mut sessions = Vec::new();
        if events.is_empty() {
            return sessions;
        }

        let mut current_session_start = events[0].timestamp;
        let mut current_session_apps = HashMap::new();
        let mut last_event_end = events[0].timestamp + Duration::seconds(events[0].duration as i64);

        for event in events {
            let event_start = event.timestamp;
            let gap = (event_start - last_event_end).num_seconds();

            // Session boundary detection: gap > 5 minutes
            if gap > 300 {
                // End current session
                if !current_session_apps.is_empty() {
                    let session = self.create_work_session(
                        current_session_start,
                        last_event_end,
                        &current_session_apps,
                    );
                    sessions.push(session);
                }
                
                // Start new session
                current_session_start = event_start;
                current_session_apps.clear();
            }

            // Add app to current session
            if let Some(app) = event.data.get("app").and_then(|v| v.as_str()) {
                *current_session_apps.entry(app.to_string()).or_insert(0.0) += event.duration;
            }
            
            last_event_end = event_start + Duration::seconds(event.duration as i64);
        }

        // Add final session
        if !current_session_apps.is_empty() {
            let session = self.create_work_session(
                current_session_start,
                last_event_end,
                &current_session_apps,
            );
            sessions.push(session);
        }

        sessions
    }

    fn create_work_session(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        apps: &HashMap<String, f64>,
    ) -> WorkSession {
        let duration_minutes = (end - start).num_seconds() as f64 / 60.0;
        
        // Get primary apps for this session
        let mut app_times: Vec<_> = apps.iter().collect();
        app_times.sort_by(|a, b| b.1.partial_cmp(a.1).unwrap());
        let primary_apps: Vec<String> = app_times.iter()
            .take(3)
            .map(|(app, _)| app.to_string())
            .collect();

        // Calculate focus score based on app diversity
        let focus_score = if apps.len() == 1 {
            1.0
        } else if apps.len() <= 3 {
            0.8
        } else if apps.len() <= 5 {
            0.6
        } else {
            0.4
        };

        let session_type = if duration_minutes > 45.0 && focus_score > 0.7 {
            "deep_work"
        } else if duration_minutes < 15.0 {
            "break"
        } else if focus_score > 0.5 {
            "shallow_work"
        } else {
            "mixed"
        }.to_string();

        WorkSession {
            start,
            end,
            duration_minutes,
            primary_apps,
            focus_score,
            session_type,
        }
    }

    fn assess_context_appropriateness(
        &self,
        events: &[crate::modules::activity_watch::Event],
        user_context: &str,
    ) -> ContextAssessment {
        let context_lower = user_context.to_lowercase();
        
        // Determine user role and expected apps
        let (user_role, expected_apps, distraction_apps) = if context_lower.contains("social media manager") {
            (
                "Social Media Manager",
                vec!["twitter", "facebook", "instagram", "linkedin", "hootsuite", "buffer"],
                vec!["games", "netflix", "youtube"],
            )
        } else if context_lower.contains("developer") || context_lower.contains("programmer") {
            (
                "Software Developer",
                vec!["vscode", "code", "terminal", "chrome", "firefox", "slack", "github"],
                vec!["facebook", "instagram", "tiktok", "games"],
            )
        } else if context_lower.contains("writer") || context_lower.contains("content") {
            (
                "Content Creator",
                vec!["word", "docs", "notion", "obsidian", "chrome", "firefox"],
                vec!["games", "tiktok", "instagram"],
            )
        } else if context_lower.contains("designer") {
            (
                "Designer",
                vec!["figma", "sketch", "photoshop", "illustrator", "chrome"],
                vec!["games", "tiktok", "facebook"],
            )
        } else {
            (
                "General Professional",
                vec!["chrome", "firefox", "word", "excel", "slack", "teams"],
                vec!["games", "tiktok", "instagram", "facebook", "youtube"],
            )
        };

        // Analyze app usage
        let mut context_appropriate_time = 0.0;
        let mut total_time = 0.0;
        let _assessment_details: Vec<String> = Vec::new();

        for event in events {
            if let Some(app) = event.data.get("app").and_then(|v| v.as_str()) {
                let app_lower = app.to_lowercase();
                total_time += event.duration;
                
                let is_expected = expected_apps.iter().any(|&exp| app_lower.contains(exp));
                let is_distraction = distraction_apps.iter().any(|&dist| app_lower.contains(dist));
                
                if is_expected {
                    context_appropriate_time += event.duration;
                } else if !is_distraction {
                    // Neutral apps
                    context_appropriate_time += event.duration * 0.5;
                }
            }
        }

        let context_score = if total_time > 0.0 {
            context_appropriate_time / total_time
        } else {
            0.5
        };

        let assessment = match context_score {
            x if x > 0.8 => "Excellent alignment with professional context",
            x if x > 0.6 => "Good alignment with occasional off-task moments",
            x if x > 0.4 => "Moderate alignment - consider refocusing on core tasks",
            _ => "Low alignment - significant time on non-contextual activities",
        }.to_string();

        ContextAssessment {
            user_role_context: user_role.to_string(),
            expected_apps: expected_apps.iter().map(|s| s.to_string()).collect(),
            distraction_apps: distraction_apps.iter().map(|s| s.to_string()).collect(),
            context_appropriate_score: context_score,
            assessment,
        }
    }

    fn analyze_fatigue_patterns(
        &self,
        _events: &[crate::modules::activity_watch::Event],
        sessions: &[WorkSession],
    ) -> FatigueAnalysis {
        let now = Utc::now();
        
        // Find last break
        let last_break = sessions.iter()
            .filter(|s| s.session_type == "break")
            .map(|s| s.end)
            .max();
            
        let time_since_break = if let Some(break_time) = last_break {
            (now - break_time).num_minutes() as f64
        } else if !sessions.is_empty() {
            (now - sessions[0].start).num_minutes() as f64
        } else {
            0.0
        };

        // Calculate continuous work time
        let continuous_work = sessions.iter()
            .filter(|s| s.session_type != "break")
            .map(|s| s.duration_minutes)
            .sum::<f64>();

        // Analyze focus degradation
        let recent_focus_scores: Vec<f64> = sessions.iter()
            .rev()
            .take(3)
            .map(|s| s.focus_score)
            .collect();
            
        let focus_degradation = if recent_focus_scores.len() >= 2 {
            let recent_avg = recent_focus_scores.iter().sum::<f64>() / recent_focus_scores.len() as f64;
            let earlier_avg = sessions.iter()
                .take(sessions.len().saturating_sub(3))
                .map(|s| s.focus_score)
                .sum::<f64>() / sessions.len().saturating_sub(3).max(1) as f64;
            (earlier_avg - recent_avg).max(0.0)
        } else {
            0.0
        };

        // Determine fatigue level
        let (fatigue_level, break_urgency, recommended_action) = match (time_since_break, continuous_work, focus_degradation) {
            (t, _, _) if t < 30.0 => ("low", "none", "Continue working, you're doing well"),
            (t, w, d) if t < 60.0 && w < 90.0 && d < 0.2 => ("low", "none", "Good work rhythm, keep it up"),
            (t, w, _) if t < 90.0 && w < 120.0 => ("moderate", "suggested", "Consider a 5-minute break soon"),
            (t, w, d) if t < 120.0 && w < 180.0 && d < 0.3 => ("moderate", "recommended", "You've been working hard, take a 10-minute break"),
            (t, _, _) if t >= 120.0 => ("high", "urgent", "You need a break now - step away for 15 minutes"),
            (_, w, _) if w >= 180.0 => ("critical", "urgent", "Extended work detected - take a proper break immediately"),
            _ => ("high", "recommended", "Your focus is declining, time for a refreshing break"),
        };

        FatigueAnalysis {
            fatigue_level: fatigue_level.to_string(),
            time_since_break_minutes: time_since_break,
            continuous_work_minutes: continuous_work,
            focus_degradation_rate: focus_degradation,
            recommended_action: recommended_action.to_string(),
            break_urgency: break_urgency.to_string(),
        }
    }
}