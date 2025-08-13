use serde::{Deserialize, Serialize};
use reqwest::Client;
use std::sync::OnceLock;
use crate::modules::pattern_analyzer::PatternPrompt;

// Global HTTP client for Ollama
static OLLAMA_CLIENT: OnceLock<Client> = OnceLock::new();

pub fn get_ollama_client() -> &'static Client {
    OLLAMA_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .pool_max_idle_per_host(1)
            .no_proxy()  // Disable proxy for all requests (Ollama is local)
            .build()
            .expect("Failed to create Ollama HTTP client")
    })
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LLMAnalysis {
    pub current_state: String,
    pub focus_trend: String,
    pub distraction_trend: String,
    pub confidence: String,
    pub primary_activity: String,
    #[serde(default = "default_professional_summary")]
    pub professional_summary: String,
    #[serde(default = "default_score")]
    pub work_score: u32,
    #[serde(default = "default_score")]
    pub distraction_score: u32,
    #[serde(default = "default_score")]
    pub neutral_score: u32,
    pub reasoning: String,
}

fn default_score() -> u32 {
    0
}

pub fn default_professional_summary() -> String {
    "Activity summary is being generated. Please wait for detailed analysis.".to_string()
}

pub async fn call_ollama_api_with_rate_limit(
    prompt: &str, 
    last_llm_call: &std::sync::Arc<std::sync::Mutex<Option<chrono::DateTime<chrono::Utc>>>>
) -> Result<String, String> {
    
    // Check rate limit (minimum 2 seconds between calls)
    {
        let mut last_call = last_llm_call.lock().unwrap();
        if let Some(last_time) = *last_call {
            let elapsed = chrono::Utc::now() - last_time;
            if elapsed.num_seconds() < 2 {
                let wait_time = 2 - elapsed.num_seconds();
                tokio::time::sleep(tokio::time::Duration::from_secs(wait_time as u64)).await;
            }
        }
        *last_call = Some(chrono::Utc::now());
    }
    
    call_ollama_api(prompt).await
}

pub async fn call_ollama_api(prompt: &str) -> Result<String, String> {
    let client = get_ollama_client();
    let config = crate::modules::utils::load_user_config_internal().await.unwrap_or_default();
    
    // Log the model being used
    eprintln!("[OLLAMA] Using model: {} (port: {})", config.ollama_model, config.ollama_port);
    
    let payload = serde_json::json!({
        "model": config.ollama_model,
        "prompt": prompt,
        "system": "You are a supportive ADHD productivity assistant. You MUST respond with ONLY valid JSON format, no other text or commentary. Be encouraging and provide actionable insights within the JSON structure. Address the user as you",
        "stream": false,
        "options": {
            "temperature": 0.3,
            "num_predict": 300,
            "top_p": 0.9
        }
    });
    
    let response = client
        .post(format!("http://localhost:{}/api/generate", config.ollama_port))
        .json(&payload)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| format!("Failed to send request to Ollama: {}", e))?;
    
    if !response.status().is_success() {
        return Err(format!("Ollama API error: {}", response.status()));
    }
    
    let result: serde_json::Value = response.json().await
        .map_err(|e| format!("Failed to parse Ollama response: {}", e))?;
    
    let ai_response = result.get("response")
        .and_then(|v| v.as_str())
        .ok_or("No response field in Ollama output")?
        .to_string();
    
    // Unload model if keep_model_loaded is false
    if !config.keep_model_loaded {
        let client = client.clone();
        let model_name = config.ollama_model.clone();
        let port = config.ollama_port;
        
        tokio::spawn(async move {
            // Small delay to ensure response is sent first
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            
            let unload_payload = serde_json::json!({
                "name": model_name,
                "keep_alive": 0
            });
            
            if let Err(e) = client
                .post(format!("http://localhost:{}/api/generate", port))
                .json(&unload_payload)
                .send()
                .await {
                eprintln!("[OLLAMA] Failed to unload model: {}", e);
            } else {
                eprintln!("[OLLAMA] Model unloaded from VRAM");
            }
        });
    }
    
    Ok(ai_response)
}

/// Enhanced Ollama API call for pattern analysis
pub async fn call_ollama_with_patterns(prompt: &PatternPrompt) -> Result<LLMAnalysis, String> {
    let formatted_prompt = format_pattern_prompt(prompt)?;
    let response = call_ollama_api(&formatted_prompt).await?;
    parse_llm_response(&response)
}

/// Format pattern data into comprehensive prompt
fn format_pattern_prompt(pattern_prompt: &PatternPrompt) -> Result<String, String> {
    let baseline_context = if let Some(ref baseline) = pattern_prompt.user_baseline {
        format!(
            r#"USER BASELINE (3-day training):
- Typical focus session: {:.0} minutes
- Productive hours: {:?}
- Normal mouse velocity: {:.0}-{:.0} px/s
- Normal typing speed: {:.0}-{:.0} WPM
- Typical apps: {}
- Context switch threshold: {} switches/hour"#,
            baseline.focused_session_characteristics.average_session_length / 60.0,
            baseline.productive_hours,
            baseline.focused_session_characteristics.mouse_velocity_range.0,
            baseline.focused_session_characteristics.mouse_velocity_range.1,
            baseline.focused_session_characteristics.typing_speed_range.0,
            baseline.focused_session_characteristics.typing_speed_range.1,
            baseline.focused_session_characteristics.typical_apps.join(", "),
            baseline.focused_session_characteristics.minimal_context_switches
        )
    } else {
        "USER BASELINE: Not yet established (in training mode)".to_string()
    };

    let anomalies_text = if pattern_prompt.anomaly_indicators.is_empty() {
        "No significant anomalies detected".to_string()
    } else {
        pattern_prompt.anomaly_indicators.iter()
            .map(|a| format!("- {:?}: {} (severity: {:.1})", a.anomaly_type, a.description, a.severity))
            .collect::<Vec<_>>()
            .join("\n")
    };

    let prompt = format!(
        r#"Analyze comprehensive user interaction patterns for ADHD productivity assessment. Return JSON only.

{}

CURRENT SESSION: {} minutes, primary: {}, productivity: {:.0}%

INTERACTION METRICS:
- Total interactions: {}
- Average focus score: {:.1}
- Context switch rate: {:.1} per hour
- Productive time ratio: {:.1}%

ANOMALY DETECTION:
{}

WORKFLOW STATE: {:?}

DETAILED TIMELINE (last 10 events):
{}

ANALYSIS REQUIREMENTS:
1. Compare current patterns against user baseline (if available)
2. Identify focus/distraction indicators from interaction patterns
3. Assess workflow efficiency and coherence
4. Provide actionable recommendations

Return JSON:
{{
  "current_state": "flow|working|needs_nudge|afk",
  "focus_trend": "maintaining_focus|entering_focus|losing_focus|variable|none",
  "distraction_trend": "low|moderate|increasing|decreasing|high",
  "confidence": "high|medium|low",
  "primary_activity": "Specific description of main activity with pattern insights",
  "reasoning": "2-3 sentences explaining pattern analysis and deviations from baseline"
}}"#,
        baseline_context,
        pattern_prompt.current_session.duration / 60.0,
        pattern_prompt.current_session.primary_activity,
        pattern_prompt.current_session.productivity_rating * 100.0,
        pattern_prompt.interaction_metrics.total_interactions,
        pattern_prompt.interaction_metrics.average_focus_score,
        pattern_prompt.interaction_metrics.context_switch_rate,
        pattern_prompt.interaction_metrics.productive_time_ratio * 100.0,
        anomalies_text,
        pattern_prompt.workflow_analysis,
        pattern_prompt.detailed_timeline.iter()
            .take(10)
            .map(|e| format!("{}: {} (significance: {:.1})", 
                e.timestamp.format("%H:%M"), 
                e.description, 
                e.significance))
            .collect::<Vec<_>>()
            .join("\n")
    );

    Ok(prompt)
}

/// Robust JSON parsing that can handle partial/malformed LLM responses
pub fn parse_llm_response(response: &str) -> Result<LLMAnalysis, String> {
    // Clean the response text first to remove common prefixes that break JSON parsing
    let cleaned_response = response
        .trim()
        .strip_prefix("📊 Activity detected: [")
        .unwrap_or(response)
        .strip_prefix("📊 Activity detected:")
        .unwrap_or(response)
        .strip_prefix("Activity detected:")
        .unwrap_or(response)
        .strip_prefix("[")
        .unwrap_or(response)
        .strip_suffix("]")
        .unwrap_or(response)
        .trim();
    
    // First try direct JSON parsing on cleaned response
    if let Ok(analysis) = serde_json::from_str::<LLMAnalysis>(cleaned_response) {
        return Ok(analysis);
    }
    
    // If that fails, try to extract JSON from the response text
    let json_start = cleaned_response.find('{');
    let json_end = cleaned_response.rfind('}');
    
    if let (Some(start), Some(end)) = (json_start, json_end) {
        let json_str = &cleaned_response[start..=end];
        if let Ok(analysis) = serde_json::from_str::<LLMAnalysis>(json_str) {
            return Ok(analysis);
        }
    }
    
    // If JSON parsing fails, try to extract individual fields manually
    let current_state = extract_field(cleaned_response, "current_state").unwrap_or("working".to_string());
    let focus_trend = extract_field(cleaned_response, "focus_trend").unwrap_or("variable".to_string());
    let distraction_trend = extract_field(cleaned_response, "distraction_trend").unwrap_or("moderate".to_string());
    let confidence = extract_field(cleaned_response, "confidence").unwrap_or("low".to_string());
    let primary_activity = extract_field(cleaned_response, "primary_activity")
        .unwrap_or("Unable to determine primary activity".to_string());
    let professional_summary = extract_field(cleaned_response, "professional_summary")
        .unwrap_or(default_professional_summary());
    let work_score = extract_field(cleaned_response, "work_score")
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(50);
    let distraction_score = extract_field(cleaned_response, "distraction_score")
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(30);
    let neutral_score = extract_field(cleaned_response, "neutral_score")
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(20);
    let reasoning = extract_field(cleaned_response, "reasoning")
        .unwrap_or("Analysis incomplete due to parsing error".to_string());
    
    Ok(LLMAnalysis {
        current_state,
        focus_trend,
        distraction_trend,
        confidence,
        primary_activity,
        professional_summary,
        work_score,
        distraction_score,
        neutral_score,
        reasoning,
    })
}

fn extract_field(text: &str, field_name: &str) -> Option<String> {
    // Try to find the field in JSON-like format
    let patterns = vec![
        format!(r#""{}"\s*:\s*"([^"]+)""#, field_name),
        format!(r#"{}\s*:\s*"([^"]+)""#, field_name),
        format!(r#""{}":"([^"]+)""#, field_name),
    ];
    
    for pattern in patterns {
        if let Ok(re) = regex::Regex::new(&pattern) {
            if let Some(captures) = re.captures(text) {
                if let Some(value) = captures.get(1) {
                    return Some(value.as_str().to_string());
                }
            }
        }
    }
    
    None
}

pub async fn test_ollama_connection() -> bool {
    let client = get_ollama_client();
    let config = crate::modules::utils::load_user_config_internal().await.unwrap_or_default();
    
    match client
        .get(format!("http://localhost:{}/api/tags", config.ollama_port))
        .send()
        .await
    {
        Ok(response) => response.status().is_success(),
        Err(_) => false,
    }
}