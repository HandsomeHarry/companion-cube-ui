use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc, Timelike, SubsecRound};
use std::collections::HashMap;
use anyhow::Result;
use reqwest::Client;
use std::sync::OnceLock;

// Global HTTP client for ActivityWatch
static AW_CLIENT: OnceLock<Client> = OnceLock::new();

pub fn get_aw_client() -> &'static Client {
    AW_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .pool_max_idle_per_host(2)
            .no_proxy()  // Disable proxy for all requests (ActivityWatch is local)
            .build()
            .expect("Failed to create ActivityWatch HTTP client")
    })
}

#[derive(Debug, Clone)]
pub struct ActivityWatchClient {
    host: String,
    port: u16,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Event {
    pub timestamp: DateTime<Utc>,
    pub duration: f64,
    pub data: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Bucket {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub hostname: String,
    #[serde(rename = "type")]
    pub bucket_type: String,
}

impl ActivityWatchClient {
    pub fn new(host: String, port: u16) -> Self {
        Self { host, port }
    }

    pub async fn get_events(&self, bucket: &str, start: DateTime<Utc>, end: DateTime<Utc>) -> Result<Vec<Event>, String> {
        // ActivityWatch has issues with microsecond precision - round to seconds
        let start_rounded = start.trunc_subsecs(0);
        let end_rounded = end.trunc_subsecs(0);
        
        let start_str = start_rounded.format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let end_str = end_rounded.format("%Y-%m-%dT%H:%M:%SZ").to_string();
        
        let url = format!(
            "http://{}:{}/api/0/buckets/{}/events?start={}&end={}",
            self.host, self.port, bucket, start_str, end_str
        );

        let response = match get_aw_client()
            .get(&url)
            .send()
            .await {
            Ok(resp) => resp,
            Err(e) => {
                eprintln!("ERROR: Failed to fetch events from {}: {}", url, e);
                return Err(format!("Failed to fetch events: {}", e));
            }
        };

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_else(|_| "No error details".to_string());
            eprintln!("ERROR: ActivityWatch API error {} for URL: {}", status, url);
            eprintln!("ERROR: Response: {}", error_text);
            return Err(format!("ActivityWatch API error: {}", status));
        }

        let events: Vec<Event> = response.json().await
            .map_err(|e| format!("Failed to parse events: {}", e))?;

        Ok(events)
    }

    /// Get window events filtered by non-AFK periods
    pub async fn get_active_window_events(&self, start: DateTime<Utc>, end: DateTime<Utc>) -> Result<Vec<serde_json::Value>, String> {
        // First, get all buckets to find the right ones
        let buckets = self.get_buckets().await?;
        
        // Find window and AFK buckets using pattern matching
        let window_bucket_id = buckets.keys()
            .find(|id| id.starts_with("aw-watcher-window_"))
            .cloned()
            .ok_or("No window watcher bucket found")?;
            
        let afk_bucket_id = buckets.keys()
            .find(|id| id.starts_with("aw-watcher-afk_"))
            .cloned()
            .ok_or("No AFK watcher bucket found")?;

        // Fetch both window and AFK events
        let (window_events, afk_events) = futures::try_join!(
            self.get_events(&window_bucket_id, start, end),
            self.get_events(&afk_bucket_id, start, end)
        )?;

        // Process AFK events to find active periods
        let mut active_periods = Vec::new();
        for event in &afk_events {
            if let Some(status) = event.data.get("status").and_then(|v| v.as_str()) {
                if status == "not-afk" {
                    let period_start = event.timestamp;
                    let period_end = event.timestamp + chrono::Duration::seconds(event.duration as i64);
                    active_periods.push((period_start, period_end));
                }
            }
        }

        // Merge overlapping active periods
        active_periods.sort_by_key(|p| p.0);
        let mut merged_periods: Vec<(DateTime<Utc>, DateTime<Utc>)> = Vec::new();
        for period in active_periods {
            if let Some(last) = merged_periods.last_mut() {
                if period.0 <= last.1 {
                    last.1 = last.1.max(period.1);
                    continue;
                }
            }
            merged_periods.push(period);
        }

        // Filter window events to only include those within active periods
        let mut active_window_events = Vec::new();
        for event in &window_events {
            let event_end = event.timestamp + chrono::Duration::seconds(event.duration as i64);
            
            for &(period_start, period_end) in &merged_periods {
                // Check if event overlaps with active period
                if event.timestamp < period_end && event_end > period_start {
                    // Calculate the overlap
                    let overlap_start = event.timestamp.max(period_start);
                    let overlap_end = event_end.min(period_end);
                    let overlap_duration = (overlap_end - overlap_start).num_seconds() as f64;
                    
                    if overlap_duration > 0.0 {
                        // Create a modified event with adjusted timestamp and duration
                        let event_json = serde_json::json!({
                            "timestamp": overlap_start.to_rfc3339(),
                            "duration": overlap_duration,
                            "data": event.data
                        });
                        
                        active_window_events.push(event_json);
                    }
                }
            }
        }

        Ok(active_window_events)
    }

    pub async fn get_buckets(&self) -> Result<HashMap<String, serde_json::Value>, String> {
        let url = format!("http://{}:{}/api/0/buckets/", self.host, self.port);
        
        let response = get_aw_client()
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("Failed to fetch buckets: {}", e))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_else(|_| "No error details".to_string());
            eprintln!("ERROR: ActivityWatch API error {} - {}", status, error_text);
            return Err(format!("ActivityWatch API error: {}", status));
        }

        let buckets: HashMap<String, serde_json::Value> = response.json().await
            .map_err(|e| format!("Failed to parse buckets: {}", e))?;
        
        Ok(buckets)
    }

    pub async fn test_connection(&self) -> ConnectionStatus {
        let url = format!("http://{}:{}/api/0/info", self.host, self.port);
        
        match get_aw_client().get(&url).send().await {
            Ok(response) if response.status().is_success() => {
                ConnectionStatus {
                    connected: true,
                    activitywatch: true,
                    errors: vec![],
                }
            }
            Ok(response) => {
                ConnectionStatus {
                    connected: false,
                    activitywatch: false,
                    errors: vec![format!("ActivityWatch API returned: {}", response.status())],
                }
            }
            Err(e) => {
                ConnectionStatus {
                    connected: false,
                    activitywatch: false,
                    errors: vec![format!("Failed to connect to ActivityWatch: {}", e)],
                }
            }
        }
    }

    /// Get activity data for AI analysis
    pub async fn get_activity_data(&self) -> Result<String, String> {
        let now = Utc::now();
        let start = now - chrono::Duration::minutes(30);
        
        let events = self.get_active_window_events(start, now).await?;
        
        let mut activity_summary = Vec::new();
        activity_summary.push(format!("Activity from {} to {}", start.format("%H:%M"), now.format("%H:%M")));
        
        for event in events.iter().take(20) {
            if let Ok(timestamp) = event.get("timestamp")
                .and_then(|v| v.as_str())
                .ok_or("Missing timestamp")
                .and_then(|t| DateTime::parse_from_rfc3339(t).map_err(|_| "Invalid timestamp")) {
                
                let duration = event.get("duration").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let data = event.get("data").and_then(|v| v.as_object());
                
                if let Some(data) = data {
                    let app = data.get("app").and_then(|v| v.as_str()).unwrap_or("Unknown");
                    let title = data.get("title").and_then(|v| v.as_str()).unwrap_or("");
                    
                    let truncated_title = if title.chars().count() > 50 {
                        title.chars().take(50).collect::<String>()
                    } else {
                        title.to_string()
                    };
                    
                    activity_summary.push(format!(
                        "{}: {} - {} ({}s)",
                        timestamp.format("%H:%M"),
                        app,
                        truncated_title,
                        duration as i32
                    ));
                }
            }
        }
        
        if activity_summary.len() == 1 {
            Ok("No recent activity data available".to_string())
        } else {
            Ok(activity_summary.join("\n"))
        }
    }

    /// Get multi-timeframe data with AFK filtering
    pub async fn get_multi_timeframe_data_active(&self) -> Result<HashMap<String, TimeframeData>, String> {
        let now = Utc::now();
        let timeframes = vec![
            ("5_minutes", chrono::Duration::minutes(5)),
            ("10_minutes", chrono::Duration::minutes(10)),
            ("30_minutes", chrono::Duration::minutes(30)),
            ("1_hour", chrono::Duration::hours(1)),
            ("today", chrono::Duration::hours(if now.hour() == 0 { 1 } else { now.hour() as i64 })),
        ];

        // Get all buckets once
        let buckets = self.get_buckets().await?;
        
        let window_bucket_id = buckets.keys()
            .find(|id| id.starts_with("aw-watcher-window_"))
            .cloned()
            .ok_or("No window bucket found")?;
        
        let afk_bucket_id = buckets.keys()
            .find(|id| id.starts_with("aw-watcher-afk_"))
            .cloned()
            .ok_or("No AFK bucket found")?;

        // Fetch data for the longest timeframe (today)
        let max_duration = timeframes.iter().map(|(_, d)| d).max().unwrap();
        let start = now - *max_duration;
        let (all_window_events, all_afk_events) = futures::try_join!(
            self.get_events(&window_bucket_id, start, now),
            self.get_events(&afk_bucket_id, start, now)
        )?;

        let mut timeframe_data = HashMap::new();

        // Process each timeframe by filtering the already-fetched data
        for (name, duration) in timeframes {
            let timeframe_start = now - duration;
            
            // Filter events for this timeframe
            let window_events: Vec<Event> = all_window_events.iter()
                .filter(|e| e.timestamp >= timeframe_start)
                .cloned()
                .collect();
            
            let afk_events: Vec<Event> = all_afk_events.iter()
                .filter(|e| e.timestamp >= timeframe_start)
                .cloned()
                .collect();

            // Calculate statistics
            let mut stats = TimeframeStatistics {
                total_events: window_events.len() as u32,
                unique_apps: std::collections::HashSet::new(),
                total_active_minutes: 0.0,
                context_switches: 0,
            };

            let mut last_app = String::new();
            for event in &window_events {
                if let Some(app) = event.data.get("app").and_then(|v| v.as_str()) {
                    stats.unique_apps.insert(app.to_string());
                    if !last_app.is_empty() && last_app != app {
                        stats.context_switches += 1;
                    }
                    last_app = app.to_string();
                }
                stats.total_active_minutes += event.duration / 60.0;
            }

            timeframe_data.insert(name.to_string(), TimeframeData {
                start: timeframe_start,
                end: now,
                window_events,
                afk_events,
                statistics: stats,
            });
        }

        Ok(timeframe_data)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ConnectionStatus {
    pub connected: bool,
    pub activitywatch: bool,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct TimeframeData {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub window_events: Vec<Event>,
    pub afk_events: Vec<Event>,
    pub statistics: TimeframeStatistics,
}

#[derive(Debug, Clone)]
pub struct TimeframeStatistics {
    pub total_events: u32,
    pub unique_apps: std::collections::HashSet<String>,
    pub total_active_minutes: f64,
    pub context_switches: u32,
}