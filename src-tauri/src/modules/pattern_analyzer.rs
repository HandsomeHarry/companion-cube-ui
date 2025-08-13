use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc, Duration};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Core metrics for pattern analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InteractionMetrics {
    pub timestamp: DateTime<Utc>,
    pub mouse: MouseMetrics,
    pub keyboard: KeyboardMetrics,
    pub application: ApplicationMetrics,
    pub browser: Option<BrowserMetrics>,
    pub workflow: WorkflowMetrics,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MouseMetrics {
    pub movement_velocity: f64,     // pixels per second
    pub acceleration: f64,          // change in velocity
    pub click_frequency: u32,       // clicks per minute
    pub click_intervals: Vec<f64>,  // time between clicks
    pub idle_time: f64,            // seconds without movement
    pub distance_traveled: f64,     // total pixels moved
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyboardMetrics {
    pub typing_speed: f64,          // words per minute
    pub burst_patterns: Vec<TypingBurst>,
    pub inter_keystroke_timing: Vec<f64>, // milliseconds between keystrokes
    pub correction_rate: f64,       // backspace frequency
    pub idle_periods: Vec<f64>,     // gaps in typing
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypingBurst {
    pub start_time: DateTime<Utc>,
    pub duration: f64,
    pub keystroke_count: u32,
    pub average_interval: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplicationMetrics {
    pub app_name: String,
    pub window_title: String,
    pub time_spent: f64,            // seconds
    pub switch_count: u32,          // times switched to this app
    pub interaction_density: f64,   // interactions per minute
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserMetrics {
    pub tab_count: u32,
    pub tab_switches: u32,
    pub domains_visited: Vec<DomainVisit>,
    pub semantic_coherence: f64,    // 0-1 score of topic consistency
    pub rabbit_hole_score: f64,     // topic drift measurement
    pub dwell_times: HashMap<String, f64>, // URL -> seconds
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainVisit {
    pub domain: String,
    pub visit_time: DateTime<Utc>,
    pub duration: f64,
    pub page_title: String,
    pub semantic_category: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowMetrics {
    pub app_sequence: Vec<String>,
    pub session_boundaries: Vec<SessionBoundary>,
    pub efficiency_score: f64,
    pub context_switches: u32,
    pub productive_periods: Vec<ProductivePeriod>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionBoundary {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub session_type: SessionType,
    pub productivity_score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SessionType {
    DeepWork,
    ShallowWork,
    Communication,
    Research,
    Entertainment,
    Break,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProductivePeriod {
    pub start: DateTime<Utc>,
    pub duration: f64,
    pub primary_activity: String,
    pub flow_score: f64,
}

/// User baseline patterns learned during training period
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserBaseline {
    pub training_start: DateTime<Utc>,
    pub training_end: Option<DateTime<Utc>>,
    pub is_trained: bool,
    pub focused_session_characteristics: FocusCharacteristics,
    pub typical_workflows: Vec<WorkflowPattern>,
    pub productive_hours: Vec<u32>, // hours of day when most productive
    pub interaction_baselines: InteractionBaselines,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FocusCharacteristics {
    pub average_session_length: f64,
    pub typical_apps: Vec<String>,
    pub mouse_velocity_range: (f64, f64),
    pub typing_speed_range: (f64, f64),
    pub click_frequency_range: (u32, u32),
    pub minimal_context_switches: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowPattern {
    pub name: String,
    pub app_sequence: Vec<String>,
    pub average_duration: f64,
    pub frequency: u32, // times observed
    pub time_of_day_preference: Vec<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InteractionBaselines {
    pub normal_mouse_velocity: f64,
    pub normal_click_rate: f64,
    pub normal_typing_speed: f64,
    pub normal_app_switches: f64,
    pub break_patterns: Vec<BreakPattern>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BreakPattern {
    pub typical_duration: f64,
    pub frequency_per_hour: f64,
    pub trigger_indicators: Vec<String>,
}

/// Main pattern analyzer that processes and analyzes user patterns
pub struct PatternAnalyzer {
    current_metrics: Arc<Mutex<Vec<InteractionMetrics>>>,
    training_data: Arc<Mutex<Vec<InteractionMetrics>>>,
    user_baseline: Arc<Mutex<Option<UserBaseline>>>,
    db_path: String,
}

impl PatternAnalyzer {
    pub fn new(db_path: String) -> Self {
        Self {
            current_metrics: Arc::new(Mutex::new(Vec::new())),
            training_data: Arc::new(Mutex::new(Vec::new())),
            user_baseline: Arc::new(Mutex::new(None)),
            db_path,
        }
    }
    
    /// Set the user baseline (e.g., loaded from database)
    pub async fn set_baseline(&self, baseline: UserBaseline) {
        let mut stored_baseline = self.user_baseline.lock().await;
        *stored_baseline = Some(baseline);
    }

    /// Process incoming interaction data
    pub async fn process_interaction(&self, metrics: InteractionMetrics) -> Result<(), String> {
        let mut current = self.current_metrics.lock().await;
        current.push(metrics.clone());
        
        // Keep only last hour of data in memory
        let one_hour_ago = Utc::now() - Duration::hours(1);
        current.retain(|m| m.timestamp > one_hour_ago);

        // If in training mode, also add to training data
        let baseline = self.user_baseline.lock().await;
        if let Some(ref base) = *baseline {
            if !base.is_trained {
                let mut training = self.training_data.lock().await;
                training.push(metrics);
            }
        }

        Ok(())
    }

    /// Analyze patterns and detect anomalies
    pub async fn analyze_current_patterns(&self) -> Result<PatternAnalysis, String> {
        let metrics = self.current_metrics.lock().await;
        let baseline = self.user_baseline.lock().await;
        
        if metrics.is_empty() {
            return Err("No metrics available for analysis".to_string());
        }

        let analysis = if let Some(ref base) = *baseline {
            self.analyze_with_baseline(&metrics, base).await?
        } else {
            self.analyze_without_baseline(&metrics).await?
        };

        Ok(analysis)
    }

    /// Train baseline patterns from collected data
    pub async fn train_baseline(&self) -> Result<UserBaseline, String> {
        let training_data = self.training_data.lock().await;
        
        if training_data.len() < 1000 { // Minimum data points for training
            return Err("Insufficient training data".to_string());
        }

        let baseline = self.calculate_baseline(&training_data)?;
        
        let mut stored_baseline = self.user_baseline.lock().await;
        *stored_baseline = Some(baseline.clone());

        // Persist to database
        self.save_baseline_to_db(&baseline).await?;

        Ok(baseline)
    }

    /// Format pattern data for LLM consumption
    pub async fn format_for_llm(&self) -> Result<PatternPrompt, String> {
        let analysis = self.analyze_current_patterns().await?;
        let baseline = self.user_baseline.lock().await;
        
        let prompt = PatternPrompt {
            user_baseline: baseline.clone(),
            current_session: analysis.session_summary,
            detailed_timeline: analysis.timeline,
            interaction_metrics: analysis.aggregated_metrics,
            anomaly_indicators: analysis.anomalies,
            workflow_analysis: analysis.workflow_state,
            recommendations_context: analysis.recommendation_context,
        };

        Ok(prompt)
    }

    async fn analyze_with_baseline(&self, metrics: &[InteractionMetrics], baseline: &UserBaseline) -> Result<PatternAnalysis, String> {
        // Complex analysis comparing current patterns to baseline
        let session_summary = self.summarize_session(metrics)?;
        let timeline = self.create_detailed_timeline(metrics)?;
        let aggregated = self.aggregate_metrics(metrics)?;
        let anomalies = self.detect_anomalies(metrics, baseline)?;
        let workflow = self.analyze_workflow(metrics, baseline)?;
        let context = self.create_recommendation_context(metrics, baseline)?;

        Ok(PatternAnalysis {
            timestamp: Utc::now(),
            session_summary,
            timeline,
            aggregated_metrics: aggregated,
            anomalies,
            workflow_state: workflow,
            recommendation_context: context,
            focus_score: self.calculate_focus_score(metrics, baseline)?,
            distraction_sources: self.identify_distractions(metrics, baseline)?,
        })
    }

    async fn analyze_without_baseline(&self, metrics: &[InteractionMetrics]) -> Result<PatternAnalysis, String> {
        // Simpler analysis without baseline comparison
        let session_summary = self.summarize_session(metrics)?;
        let timeline = self.create_detailed_timeline(metrics)?;
        let aggregated = self.aggregate_metrics(metrics)?;

        Ok(PatternAnalysis {
            timestamp: Utc::now(),
            session_summary,
            timeline,
            aggregated_metrics: aggregated,
            anomalies: vec![],
            workflow_state: WorkflowState::Unknown,
            recommendation_context: HashMap::new(),
            focus_score: 50.0, // Default neutral score
            distraction_sources: vec![],
        })
    }

    fn calculate_baseline(&self, data: &[InteractionMetrics]) -> Result<UserBaseline, String> {
        // Statistical analysis to determine baseline patterns
        // This is where the ML magic happens - analyzing 3 days of data
        
        let focused_chars = self.extract_focus_characteristics(data)?;
        let workflows = self.extract_workflow_patterns(data)?;
        let productive_hours = self.extract_productive_hours(data)?;
        let interaction_baselines = self.calculate_interaction_baselines(data)?;

        Ok(UserBaseline {
            training_start: data.first().map(|m| m.timestamp).unwrap_or(Utc::now()),
            training_end: Some(Utc::now()),
            is_trained: true,
            focused_session_characteristics: focused_chars,
            typical_workflows: workflows,
            productive_hours,
            interaction_baselines,
        })
    }

    // Placeholder implementations for complex analysis functions
    fn summarize_session(&self, _metrics: &[InteractionMetrics]) -> Result<SessionSummary, String> {
        Ok(SessionSummary::default())
    }

    fn create_detailed_timeline(&self, _metrics: &[InteractionMetrics]) -> Result<Vec<TimelineEvent>, String> {
        Ok(vec![])
    }

    fn aggregate_metrics(&self, _metrics: &[InteractionMetrics]) -> Result<AggregatedMetrics, String> {
        Ok(AggregatedMetrics::default())
    }

    fn detect_anomalies(&self, _metrics: &[InteractionMetrics], _baseline: &UserBaseline) -> Result<Vec<Anomaly>, String> {
        Ok(vec![])
    }

    fn analyze_workflow(&self, _metrics: &[InteractionMetrics], _baseline: &UserBaseline) -> Result<WorkflowState, String> {
        Ok(WorkflowState::Unknown)
    }

    fn create_recommendation_context(&self, _metrics: &[InteractionMetrics], _baseline: &UserBaseline) -> Result<HashMap<String, String>, String> {
        Ok(HashMap::new())
    }

    fn calculate_focus_score(&self, _metrics: &[InteractionMetrics], _baseline: &UserBaseline) -> Result<f64, String> {
        Ok(75.0)
    }

    fn identify_distractions(&self, _metrics: &[InteractionMetrics], _baseline: &UserBaseline) -> Result<Vec<DistractionSource>, String> {
        Ok(vec![])
    }

    fn extract_focus_characteristics(&self, _data: &[InteractionMetrics]) -> Result<FocusCharacteristics, String> {
        Ok(FocusCharacteristics {
            average_session_length: 45.0 * 60.0,
            typical_apps: vec!["Code.exe".to_string(), "Firefox.exe".to_string()],
            mouse_velocity_range: (10.0, 500.0),
            typing_speed_range: (30.0, 80.0),
            click_frequency_range: (5, 30),
            minimal_context_switches: 3,
        })
    }

    fn extract_workflow_patterns(&self, _data: &[InteractionMetrics]) -> Result<Vec<WorkflowPattern>, String> {
        Ok(vec![])
    }

    fn extract_productive_hours(&self, _data: &[InteractionMetrics]) -> Result<Vec<u32>, String> {
        Ok(vec![9, 10, 11, 14, 15, 16])
    }

    fn calculate_interaction_baselines(&self, _data: &[InteractionMetrics]) -> Result<InteractionBaselines, String> {
        Ok(InteractionBaselines {
            normal_mouse_velocity: 250.0,
            normal_click_rate: 15.0,
            normal_typing_speed: 50.0,
            normal_app_switches: 10.0,
            break_patterns: vec![],
        })
    }

    async fn save_baseline_to_db(&self, _baseline: &UserBaseline) -> Result<(), String> {
        // Database persistence logic
        Ok(())
    }
}

/// Analysis result structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternAnalysis {
    pub timestamp: DateTime<Utc>,
    pub session_summary: SessionSummary,
    pub timeline: Vec<TimelineEvent>,
    pub aggregated_metrics: AggregatedMetrics,
    pub anomalies: Vec<Anomaly>,
    pub workflow_state: WorkflowState,
    pub recommendation_context: HashMap<String, String>,
    pub focus_score: f64,
    pub distraction_sources: Vec<DistractionSource>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionSummary {
    pub duration: f64,
    pub primary_activity: String,
    pub productivity_rating: f64,
    pub key_insights: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineEvent {
    pub timestamp: DateTime<Utc>,
    pub event_type: String,
    pub description: String,
    pub significance: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AggregatedMetrics {
    pub total_interactions: u64,
    pub average_focus_score: f64,
    pub context_switch_rate: f64,
    pub productive_time_ratio: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Anomaly {
    pub anomaly_type: AnomalyType,
    pub severity: f64,
    pub description: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AnomalyType {
    UnusualInteractionPattern,
    ExtendedInactivity,
    RapidContextSwitching,
    AbnormalTypingPattern,
    UnknownWorkflow,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WorkflowState {
    InFlow,
    Building,
    Disrupted,
    Transitioning,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DistractionSource {
    pub source_type: String,
    pub confidence: f64,
    pub duration: f64,
    pub impact_score: f64,
}

/// Formatted prompt structure for LLM
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternPrompt {
    pub user_baseline: Option<UserBaseline>,
    pub current_session: SessionSummary,
    pub detailed_timeline: Vec<TimelineEvent>,
    pub interaction_metrics: AggregatedMetrics,
    pub anomaly_indicators: Vec<Anomaly>,
    pub workflow_analysis: WorkflowState,
    pub recommendations_context: HashMap<String, String>,
}