use std::sync::{Arc, Mutex};
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem, CheckMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, State, AppHandle, Wry, Emitter,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use chrono::{Duration, Timelike, DateTime, Utc, SubsecRound};
use dirs;
use std::sync::OnceLock;
use futures;

static AW_HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
static OLLAMA_HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

fn get_aw_client() -> &'static reqwest::Client {
    AW_HTTP_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .no_proxy()
            .pool_max_idle_per_host(10)
            .pool_idle_timeout(std::time::Duration::from_secs(120))
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    })
}

fn get_ollama_client() -> &'static reqwest::Client {
    OLLAMA_HTTP_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .no_proxy()
            .pool_max_idle_per_host(5)
            .pool_idle_timeout(std::time::Duration::from_secs(60))
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    })
}

async fn get_configured_aw_client() -> ActivityWatchClient {
    let config = load_user_config_internal().await.unwrap_or_default();
    ActivityWatchClient::new(None, Some(config.activitywatch_port))
}

#[derive(Debug, Clone)]
pub struct ConnectionTestResult {
    pub connected: bool,
    pub buckets: HashMap<String, bool>,
    pub errors: Vec<String>,
    pub web_buckets: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ActivityWatchClient {
    base_url: String,
}

impl ActivityWatchClient {
    pub fn new(host: Option<&str>, port: Option<u16>) -> Self {
        let host = host.unwrap_or("localhost");
        let port = port.unwrap_or(5600);
        Self {
            base_url: format!("http://{}:{}/api/0", host, port),
        }
    }

    pub async fn test_connection(&self) -> ConnectionTestResult {
        let mut result = ConnectionTestResult {
            connected: false,
            buckets: HashMap::new(),
            errors: Vec::new(),
            web_buckets: Vec::new(),
        };

        // Simple connection test (remove separate timeout to use client default)
        match get_aw_client()
            .get(&format!("{}/buckets/", self.base_url))  // Add trailing slash
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => {
                result.connected = true;
                result.buckets.insert("window".to_string(), true);
                result.buckets.insert("afk".to_string(), true);
            }
            Ok(_) => {
                result.errors.push("ActivityWatch responded but with error status".to_string());
            }
            Err(_) => {
                result.errors.push("Cannot connect to ActivityWatch API".to_string());
            }
        }

        result
    }

    pub async fn get_buckets(&self) -> Result<HashMap<String, serde_json::Value>, String> {
        let response = get_aw_client()
            .get(&format!("{}/buckets/", self.base_url))  // Add trailing slash
            .send()  // Use client default timeout
            .await
            .map_err(|e| format!("Failed to fetch buckets: {}", e))?;

        if !response.status().is_success() {
            return Err(format!("ActivityWatch returned error status: {}", response.status()));
        }

        let buckets: HashMap<String, serde_json::Value> = response.json().await
            .map_err(|e| format!("Failed to parse buckets response: {}", e))?;


        Ok(buckets)
    }

    pub async fn get_events(&self, bucket_id: &str, start: DateTime<Utc>, end: DateTime<Utc>) -> Result<Vec<Event>, String> {
        let events_json = self.fetch_events_common(bucket_id, start, end).await?;
        
        let mut events = Vec::new();
        for event_json in events_json {
            if let Ok(event) = self.parse_event(event_json) {
                events.push(event);
            }
        }

        Ok(events)
    }

    pub async fn get_raw_events(&self, bucket_id: &str, start: DateTime<Utc>, end: DateTime<Utc>) -> Result<Vec<serde_json::Value>, String> {
        self.fetch_events_common(bucket_id, start, end).await
    }
    
    async fn fetch_events_common(&self, bucket_id: &str, start: DateTime<Utc>, end: DateTime<Utc>) -> Result<Vec<serde_json::Value>, String> {
        // Validate time range
        if start >= end {
            return Err(format!("Invalid time range: start {} >= end {}", start, end));
        }
        
        // Ensure end time is not in the future
        let now = Utc::now();
        let end = if end > now { now } else { end };
        
        // ActivityWatch has issues with microsecond precision - round to seconds
        let start_rounded = start.trunc_subsecs(0);
        let end_rounded = end.trunc_subsecs(0);
        
        let start_str = start_rounded.format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let end_str = end_rounded.format("%Y-%m-%dT%H:%M:%SZ").to_string();
        
        let url = format!("{}/buckets/{}/events?start={}&end={}", 
                         self.base_url, bucket_id, start_str, end_str);
        
        let response = get_aw_client()
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("Failed to fetch events from {}: {}", bucket_id, e))?;

        if !response.status().is_success() {
            if response.status().as_u16() == 500 {
                return Ok(Vec::new());
            }
            return Err(format!("ActivityWatch returned error status {} for bucket {}", 
                             response.status(), bucket_id));
        }

        let events_json: Vec<serde_json::Value> = response.json().await
            .map_err(|e| format!("Failed to parse events response from {}: {}", bucket_id, e))?;

        Ok(events_json)
    }

    /// Gets all events (window, web, AFK) from the last N hours
    pub async fn get_all_events(&self, hours_back: f64) -> Result<HashMap<String, Vec<Event>>, String> {
        let mut all_events = HashMap::new();
        
        // Simplified for legacy compatibility
        all_events.insert("window".to_string(), self.get_window_events_legacy(hours_back).await.unwrap_or_default());
        all_events.insert("web".to_string(), Vec::new()); // Web events disabled
        all_events.insert("afk".to_string(), self.get_afk_events(hours_back).await.unwrap_or_default());
        
        Ok(all_events)
    }

    /// Gets window events within a specific time range and returns raw JSON data
    pub async fn get_window_events(&self, start_time: DateTime<Utc>, end_time: DateTime<Utc>) -> Result<Vec<serde_json::Value>, String> {
        let buckets = self.get_buckets().await?;
        
        // Find window bucket - ONLY use aw-watcher-window
        let window_bucket = find_bucket_by_exact_prefix(&buckets, "aw-watcher-window");
        
        if let Some(bucket_id) = window_bucket {
            let events = self.get_raw_events(&bucket_id, start_time, end_time).await?;
            Ok(events)
        } else {
            eprintln!("No window bucket found!");
            Ok(Vec::new())
        }
    }

    /// Gets window events from the last N hours (legacy method)
    pub async fn get_window_events_legacy(&self, hours_back: f64) -> Result<Vec<Event>, String> {
        let now = Utc::now();
        let end_time = now - chrono::Duration::seconds(2);
        let start_time = end_time - chrono::Duration::minutes((hours_back * 60.0) as i64);
        
        let buckets = self.get_buckets().await?;
        
        // Find window bucket - ONLY use aw-watcher-window
        let window_bucket = find_bucket_by_exact_prefix(&buckets, "aw-watcher-window");
        
        // Removed excessive debug output
        
        if let Some(bucket_id) = window_bucket {
            let mut events = self.get_events(&bucket_id, start_time, end_time).await?;
            // Removed excessive debug output
            events.sort_by_key(|e| e.timestamp);
            Ok(events)
        } else {
            eprintln!("No window bucket found!");
            Ok(Vec::new())
        }
    }

    /// Gets AFK events from the last N hours
    pub async fn get_afk_events(&self, hours_back: f64) -> Result<Vec<Event>, String> {
        let now = Utc::now();
        let end_time = now - chrono::Duration::seconds(2);
        let start_time = end_time - chrono::Duration::minutes((hours_back * 60.0) as i64);
        
        let buckets = self.get_buckets().await?;
        
        // Find AFK bucket - ONLY use aw-watcher-afk
        let afk_bucket = find_bucket_by_exact_prefix(&buckets, "aw-watcher-afk");
        
        // Removed excessive debug output
        
        if let Some(bucket_id) = afk_bucket {
            let mut events = self.get_events(&bucket_id, start_time, end_time).await?;
            // Removed excessive debug output
            events.sort_by_key(|e| e.timestamp);
            Ok(events)
        } else {
            eprintln!("No AFK bucket found!");
            Ok(Vec::new())
        }
    }

    /// Gets events of a specific type within a specific time range
    pub async fn get_events_in_range(
        &self, 
        event_type: &str, 
        start_time: chrono::DateTime<chrono::Utc>, 
        end_time: chrono::DateTime<chrono::Utc>
    ) -> Result<Vec<Event>, String> {
        let buckets = self.get_buckets().await?;
        
        eprintln!("=== BUCKET DETECTION for {} events ===", event_type);
        eprintln!("Available buckets:");
        for (name, bucket_info) in &buckets {
            if let Some(bucket_type) = bucket_info.get("type").and_then(|v| v.as_str()) {
                eprintln!("  - {} (type: {})", name, bucket_type);
            } else {
                eprintln!("  - {} (no type field)", name);
            }
        }
        
        let bucket_id = match event_type {
            "window" => {
                eprintln!("Looking for window bucket...");
                let bucket = find_bucket_by_exact_prefix(&buckets, "aw-watcher-window");
                eprintln!("  Found window bucket: {:?}", bucket);
                bucket
            }
            "afk" => {
                eprintln!("Looking for AFK bucket...");
                let bucket = find_bucket_by_exact_prefix(&buckets, "aw-watcher-afk");
                eprintln!("  Found AFK bucket: {:?}", bucket);
                bucket
            }
            _ => None
        };
        
        if let Some(bucket_id) = bucket_id {
            eprintln!("Selected bucket: {}", bucket_id);
            eprintln!("Requesting events from {} to {}", start_time, end_time);
            
            // Test the ActivityWatch API directly
            let url = format!("{}/buckets/{}/events?start={}&end={}", 
                self.base_url, bucket_id, 
                start_time.format("%Y-%m-%dT%H:%M:%S%.fZ"),
                end_time.format("%Y-%m-%dT%H:%M:%S%.fZ")
            );
            eprintln!("Direct API URL: {}", url);
            
            // Try a simple query first to see if the bucket has ANY data
            let test_url = format!("{}/buckets/{}/events?limit=10", self.base_url, bucket_id);
            eprintln!("Testing bucket with: {}", test_url);
            
            match get_aw_client().get(&test_url).send().await {
                Ok(response) => {
                    if response.status().is_success() {
                        match response.text().await {
                            Ok(text) => {
                                eprintln!("Bucket test response (first 500 chars): {}", 
                                    &text[..text.len().min(500)]);
                                if text.trim() == "[]" {
                                    eprintln!("PROBLEM: Bucket is completely empty!");
                                }
                            }
                            Err(e) => eprintln!("Failed to read test response: {}", e)
                        }
                    } else {
                        eprintln!("Test query failed with status: {}", response.status());
                    }
                }
                Err(e) => eprintln!("Test query error: {}", e)
            }
            
            let mut events = self.get_events(&bucket_id, start_time, end_time).await?;
            eprintln!("Raw events retrieved: {}", events.len());
            events.sort_by_key(|e| e.timestamp);
            Ok(events)
        } else {
            eprintln!("ERROR: No {} bucket found!", event_type);
            Ok(Vec::new())
        }
    }

    /// Gets data for multiple timeframes efficiently with cached buckets and parallel processing
    pub async fn get_multi_timeframe_data(&self) -> Result<HashMap<String, HashMap<String, Vec<Event>>>, String> {
        eprintln!("=== Starting optimized multi-timeframe data fetch ===");
        
        // OPTIMIZATION 1: Fetch buckets only once and cache them
        let buckets = self.get_buckets().await?;
        
        // OPTIMIZATION 2: Find buckets once instead of searching repeatedly - ONLY use standard buckets
        let window_bucket = find_bucket_by_exact_prefix(&buckets, "aw-watcher-window");
        let afk_bucket = find_bucket_by_exact_prefix(&buckets, "aw-watcher-afk");
            
        eprintln!("Selected buckets - Window: {:?}, AFK: {:?}", window_bucket, afk_bucket);
        
        // OPTIMIZATION 3: Try to get latest events from each bucket first (fast check)
        let window_events = if let Some(bucket_id) = &window_bucket {
            self.get_latest_events_optimized(bucket_id).await.unwrap_or_default()
        } else {
            Vec::new()
        };
        
        let afk_events = if let Some(bucket_id) = &afk_bucket {
            self.get_latest_events_optimized(bucket_id).await.unwrap_or_default()
        } else {
            Vec::new()
        };
        
        eprintln!("Fast fetch: {} window events, {} AFK events", window_events.len(), afk_events.len());
        
        // OPTIMIZATION 4: Create all timeframes from the same data instead of multiple requests
        let timeframes = vec![
            ("5_minutes", 0.083),
            ("30_minutes", 0.5),
            ("1_hour", 1.0),
        ];
        
        let mut data = HashMap::new();
        let now = Utc::now();
        
        for (timeframe_name, hours_back) in timeframes {
            let cutoff_time = now - chrono::Duration::minutes((hours_back * 60.0) as i64);
            
            // Filter events by timeframe instead of making new requests
            let filtered_window: Vec<Event> = window_events.iter()
                .filter(|e| e.timestamp >= cutoff_time)
                .cloned()
                .collect();
                
            let filtered_afk: Vec<Event> = afk_events.iter()
                .filter(|e| e.timestamp >= cutoff_time)
                .cloned()
                .collect();
            
            let mut timeframe_data = HashMap::new();
            timeframe_data.insert("window".to_string(), filtered_window);
            timeframe_data.insert("afk".to_string(), filtered_afk);
            timeframe_data.insert("web".to_string(), Vec::new()); // Web events disabled
            
            data.insert(timeframe_name.to_string(), timeframe_data);
        }
        
        eprintln!("=== Optimized fetch complete: {} timeframes ===", data.len());
        Ok(data)
    }
    
    /// Get latest events with single optimized request (no timeframe filtering)
    async fn get_latest_events_optimized(&self, bucket_id: &str) -> Result<Vec<Event>, String> {
        let url = format!("{}/buckets/{}/events?limit=50", self.base_url, bucket_id);
        
        let response = get_aw_client()
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("Failed to fetch latest events from {}: {}", bucket_id, e))?;
            
        if !response.status().is_success() {
            return Ok(Vec::new()); // Return empty instead of failing
        }
        
        let events_json: Vec<serde_json::Value> = response.json().await
            .map_err(|e| format!("Failed to parse events: {}", e))?;
            
        let mut events = Vec::new();
        for event_json in events_json {
            if let Ok(event) = self.parse_event(event_json) {
                events.push(event);
            }
        }
        
        events.sort_by_key(|e| e.timestamp);
        Ok(events)
    }

    /// Get window events filtered by non-AFK periods (optimized version)
    pub async fn get_active_window_events(&self, start_time: DateTime<Utc>, end_time: DateTime<Utc>) -> Result<Vec<serde_json::Value>, String> {
        // Get buckets once and reuse
        let buckets = self.get_buckets().await?;
        
        // Find both buckets using the cached bucket list
        let window_bucket = find_bucket_by_exact_prefix(&buckets, "aw-watcher-window");
        let afk_bucket = find_bucket_by_exact_prefix(&buckets, "aw-watcher-afk");
        
        // Get events only if buckets exist
        let window_events = if let Some(bucket_id) = window_bucket {
            self.get_raw_events(&bucket_id, start_time, end_time).await?
        } else {
            Vec::new()
        };
        
        let afk_events = if let Some(bucket_id) = afk_bucket {
            self.get_raw_events(&bucket_id, start_time, end_time).await?
        } else {
            Vec::new()
        };
        
        // Build non-AFK time ranges from AFK events
        let mut active_ranges: Vec<(DateTime<Utc>, DateTime<Utc>)> = Vec::new();
        
        // Process each AFK event - when status is "not-afk", that's an active period
        for event in &afk_events {
            if let (Some(timestamp_str), Some(data), Some(duration)) = (
                event.get("timestamp").and_then(|v| v.as_str()),
                event.get("data").and_then(|v| v.as_object()),
                event.get("duration").and_then(|v| v.as_f64())
            ) {
                if let Ok(timestamp) = DateTime::parse_from_rfc3339(timestamp_str) {
                    let timestamp = timestamp.with_timezone(&Utc);
                    let event_end = timestamp + chrono::Duration::seconds(duration as i64);
                    
                    if let Some(status) = data.get("status").and_then(|v| v.as_str()) {
                        if status == "not-afk" {
                            // This entire event duration is an active period
                            active_ranges.push((timestamp, event_end));
                        }
                        // If status is "afk", we skip this period entirely
                    }
                }
            }
        }
        
        
        // Filter window events to only those within active ranges
        let mut filtered_events = Vec::new();
        
        for window_event in &window_events {
            if let (Some(timestamp_str), Some(duration)) = (
                window_event.get("timestamp").and_then(|v| v.as_str()),
                window_event.get("duration").and_then(|v| v.as_f64())
            ) {
                if let Ok(timestamp) = DateTime::parse_from_rfc3339(timestamp_str) {
                    let timestamp = timestamp.with_timezone(&Utc);
                    let window_end = timestamp + chrono::Duration::seconds(duration as i64);
                    
                    // Check if this window event overlaps with any active range
                    for (range_start, range_end) in &active_ranges {
                        // Check for any overlap between window event and active range
                        if timestamp < *range_end && window_end > *range_start {
                            // There is overlap, but we need to adjust the duration
                            // to only count the time within the active range
                            let overlap_start = timestamp.max(*range_start);
                            let overlap_end = window_end.min(*range_end);
                            let overlap_duration = (overlap_end - overlap_start).num_seconds() as f64;
                            
                            if overlap_duration > 0.0 {
                                // Clone the event and adjust its duration
                                let mut adjusted_event = window_event.clone();
                                if let Some(duration_val) = adjusted_event.get_mut("duration") {
                                    *duration_val = serde_json::Value::Number(
                                        serde_json::Number::from_f64(overlap_duration).unwrap_or(serde_json::Number::from(0))
                                    );
                                }
                                // Update timestamp if needed (if event started before active range)
                                if overlap_start > timestamp {
                                    if let Some(timestamp_val) = adjusted_event.get_mut("timestamp") {
                                        *timestamp_val = serde_json::Value::String(overlap_start.to_rfc3339());
                                    }
                                }
                                filtered_events.push(adjusted_event);
                                break; // Move to next window event after finding overlap
                            }
                        }
                    }
                }
            }
        }
        
        
        Ok(filtered_events)
    }
    

    /// Get multi-timeframe data with AFK filtering (optimized)
    pub async fn get_multi_timeframe_data_active(&self) -> Result<HashMap<String, HashMap<String, Vec<Event>>>, String> {
        let mut data = HashMap::new();
        let now = Utc::now();
        
        // Get buckets once and cache them
        let buckets = self.get_buckets().await?;
        let window_bucket = find_bucket_by_exact_prefix(&buckets, "aw-watcher-window");
        let afk_bucket = find_bucket_by_exact_prefix(&buckets, "aw-watcher-afk");
        
        if window_bucket.is_none() {
            return Err("No window bucket found".to_string());
        }
        
        // Fetch all data once for the maximum timeframe
        let one_hour_ago = now - chrono::Duration::hours(1);
        
        // Fetch both event types in parallel using futures
        let window_bucket_id = window_bucket.unwrap();
        let afk_bucket_id = afk_bucket.unwrap_or_default();
        
        let (all_window_events, all_afk_events) = futures::try_join!(
            self.get_raw_events(&window_bucket_id, one_hour_ago, now),
            async {
                if afk_bucket_id.is_empty() {
                    Ok(Vec::new())
                } else {
                    self.get_raw_events(&afk_bucket_id, one_hour_ago, now).await
                }
            }
        )?;
        
        // Build active ranges once
        let active_ranges = self.build_active_ranges(&all_afk_events);
        
        // Process all timeframes at once
        let timeframes = [("5_minutes", 5), ("30_minutes", 30), ("1_hour", 60)];
        
        for (timeframe_name, minutes) in timeframes {
            let start_time = now - chrono::Duration::minutes(minutes);
            
            // Filter events efficiently
            let window_events = self.filter_events_by_active_ranges(
                &all_window_events,
                &active_ranges,
                start_time,
                now
            );
            
            // Filter AFK events for this timeframe
            let afk_events: Vec<Event> = all_afk_events.iter()
                .filter_map(|event| {
                    if let Some(timestamp_str) = event.get("timestamp").and_then(|v| v.as_str()) {
                        if let Ok(timestamp) = DateTime::parse_from_rfc3339(timestamp_str) {
                            let timestamp = timestamp.with_timezone(&Utc);
                            if timestamp >= start_time && timestamp <= now {
                                return self.parse_event(event.clone()).ok();
                            }
                        }
                    }
                    None
                })
                .collect();
            
            let mut timeframe_data = HashMap::new();
            timeframe_data.insert("window".to_string(), window_events);
            timeframe_data.insert("afk".to_string(), afk_events);
            timeframe_data.insert("web".to_string(), Vec::new());
            
            data.insert(timeframe_name.to_string(), timeframe_data);
        }
        
        Ok(data)
    }
    
    /// Build active time ranges from AFK events
    fn build_active_ranges(&self, afk_events: &[serde_json::Value]) -> Vec<(DateTime<Utc>, DateTime<Utc>)> {
        let mut active_ranges = Vec::new();
        
        for event in afk_events {
            if let (Some(timestamp_str), Some(data), Some(duration)) = (
                event.get("timestamp").and_then(|v| v.as_str()),
                event.get("data").and_then(|v| v.as_object()),
                event.get("duration").and_then(|v| v.as_f64())
            ) {
                if let Ok(timestamp) = DateTime::parse_from_rfc3339(timestamp_str) {
                    let timestamp = timestamp.with_timezone(&Utc);
                    let event_end = timestamp + chrono::Duration::seconds(duration as i64);
                    
                    if let Some(status) = data.get("status").and_then(|v| v.as_str()) {
                        if status == "not-afk" {
                            active_ranges.push((timestamp, event_end));
                        }
                    }
                }
            }
        }
        
        active_ranges
    }
    
    /// Filter events by active ranges efficiently
    fn filter_events_by_active_ranges(
        &self,
        events: &[serde_json::Value],
        active_ranges: &[(DateTime<Utc>, DateTime<Utc>)],
        start_time: DateTime<Utc>,
        end_time: DateTime<Utc>
    ) -> Vec<Event> {
        let mut filtered_events = Vec::new();
        
        for event in events {
            if let Some(timestamp_str) = event.get("timestamp").and_then(|v| v.as_str()) {
                if let Ok(timestamp) = DateTime::parse_from_rfc3339(timestamp_str) {
                    let timestamp = timestamp.with_timezone(&Utc);
                    
                    // Check time bounds first
                    if timestamp < start_time || timestamp > end_time {
                        continue;
                    }
                    
                    // Check if within any active range
                    let is_active = active_ranges.iter().any(|(range_start, range_end)| {
                        timestamp >= *range_start && timestamp <= *range_end
                    });
                    
                    if is_active {
                        if let Ok(parsed_event) = self.parse_event(event.clone()) {
                            filtered_events.push(parsed_event);
                        }
                    }
                }
            }
        }
        
        filtered_events
    }

    /// Get formatted activity data for AI analysis
    pub async fn get_activity_data(&self) -> Result<String, String> {
        let now = Utc::now();
        let start_time = now - chrono::Duration::hours(1);
        let end_time = now;
        
        // Get recent window events filtered by non-AFK periods
        let window_events = self.get_active_window_events(start_time, end_time).await.unwrap_or_default();
        
        // Extract application names and titles for analysis with more detail
        let mut activity_summary = Vec::new();
        
        for event in window_events.iter().take(30) { // Increased limit for more context
            if let Some(data) = event.get("data").and_then(|v| v.as_object()) {
                if let Some(app_name) = data.get("app").and_then(|v| v.as_str()) {
                    let title = data.get("title").and_then(|v| v.as_str()).unwrap_or("Unknown");
                    // Include more detailed activity information
                    activity_summary.push(format!("Application: {} | Window Title: {}", app_name, title));
                }
            }
        }
        
        if activity_summary.is_empty() {
            Ok("No recent activity data available".to_string())
        } else {
            Ok(activity_summary.join("\n"))
        }
    }

    fn parse_event(&self, event_json: serde_json::Value) -> Result<Event, String> {
        let timestamp_str = event_json["timestamp"].as_str()
            .ok_or("Missing timestamp")?;
        let timestamp = DateTime::parse_from_rfc3339(timestamp_str)
            .map_err(|e| format!("Invalid timestamp format: {}", e))?
            .with_timezone(&Utc);

        let duration = event_json["duration"].as_f64()
            .ok_or("Missing duration")?;

        let data = event_json["data"].as_object()
            .ok_or("Missing data")?
            .clone();

        let data_map: HashMap<String, serde_json::Value> = data.into_iter().collect();

        Ok(Event {
            timestamp,
            duration,
            data: data_map,
        })
    }
}

// Application state
struct AppState {
    current_mode: Arc<Mutex<String>>,
    last_summary_time: Arc<Mutex<HashMap<String, DateTime<Utc>>>>,
    latest_hourly_summary: Arc<Mutex<Option<HourlySummary>>>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct TodoItem {
    id: String,
    text: String,
    completed: bool,
    created_at: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct CoachTodoList {
    todos: Vec<TodoItem>,
    generated_at: String,
    context: String,
}

impl AppState {
    fn new() -> Self {
        let saved_mode = Self::load_mode().unwrap_or_else(|| "coach".to_string());
        Self {
            current_mode: Arc::new(Mutex::new(saved_mode)),
            last_summary_time: Arc::new(Mutex::new(HashMap::new())),
            latest_hourly_summary: Arc::new(Mutex::new(None)),
        }
    }
    
    fn load_mode() -> Option<String> {
        let config_dir = dirs::config_dir()?.join("companion-cube");
        let mode_file = config_dir.join("mode.txt");
        std::fs::read_to_string(mode_file).ok()
    }
    
    fn save_mode(mode: &str) -> Result<(), String> {
        let config_dir = dirs::config_dir()
            .ok_or("Could not find config directory")?
            .join("companion-cube");
        std::fs::create_dir_all(&config_dir).map_err(|e| e.to_string())?;
        let mode_file = config_dir.join("mode.txt");
        std::fs::write(mode_file, mode).map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
struct ConnectionStatus {
    activitywatch: bool,
    ollama: bool,
    errors: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct LogMessage {
    level: String,
    message: String,
    timestamp: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct HourlySummary {
    summary: String,
    focus_score: u32,
    last_updated: String,
    period: String,
    current_state: String,
}

// Helper function to send log messages to frontend
fn send_log(app: &AppHandle, level: &str, message: &str) {
    let log_msg = LogMessage {
        level: level.to_string(),
        message: message.to_string(),
        timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
    };
    
    if let Err(e) = app.emit("log_message", &log_msg) {
        eprintln!("Failed to send log message: {}", e);
    }
}

// Helper function to send notifications
async fn send_notification(app: &AppHandle, title: &str, body: &str) {
    let config = load_user_config_internal().await.unwrap_or_default();
    
    if !config.notifications_enabled {
        send_log(app, "debug", "Notifications disabled, skipping notification");
        return;
    }
    
    send_log(app, "info", &format!("Sending notification: {}", title));
    
    // Check if webhook is configured
    if let Some(webhook_url) = &config.notification_webhook {
        match send_webhook_notification(webhook_url, title, body).await {
            Ok(_) => send_log(app, "info", "Webhook notification sent successfully"),
            Err(e) => send_log(app, "error", &format!("Failed to send webhook notification: {}", e)),
        }
    } else {
        // Send system notification (placeholder - could be implemented with tauri-plugin-notification)
        send_log(app, "info", &format!("System notification: {} - {}", title, body));
        
        // Emit to frontend for display
        if let Err(e) = app.emit("show_notification", &serde_json::json!({
            "title": title,
            "body": body,
            "timestamp": chrono::Local::now().format("%H:%M:%S").to_string()
        })) {
            eprintln!("Failed to emit notification: {}", e);
        }
    }
}

// Helper function to send webhook notifications
async fn send_webhook_notification(webhook_url: &str, title: &str, body: &str) -> Result<(), String> {
    let payload = serde_json::json!({
        "title": title,
        "body": body,
        "timestamp": chrono::Utc::now().to_rfc3339()
    });
    
    let client = get_aw_client();
    let response = client
        .post(webhook_url)
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("Failed to send webhook: {}", e))?;
    
    if response.status().is_success() {
        Ok(())
    } else {
        Err(format!("Webhook returned error status: {}", response.status()))
    }
}

// Mode-specific timing logic
fn get_mode_interval_minutes(mode: &str) -> u32 {
    match mode {
        "ghost" => 60,    // Every hour
        "chill" => 60,    // Every hour
        "study_buddy" => 5,    // Every 5 minutes
        "coach" => 15,    // Every 15 minutes
        _ => 60,          // Default to hourly
    }
}

fn should_run_summary(mode: &str, last_run: Option<DateTime<Utc>>) -> bool {
    let now = Utc::now();
    
    // First check if we already ran in this minute
    if let Some(last) = last_run {
        let elapsed_seconds = now.signed_duration_since(last).num_seconds();
        if elapsed_seconds < 60 {
            return false; // Already ran in this minute
        }
    }
    
    // Check if we're at the appropriate time mark
    match mode {
        "ghost" | "chill" => {
            // Run at whole hours (XX:00)
            now.minute() == 0
        }
        "study_buddy" => {
            // Run every 5 minutes (XX:00, XX:05, XX:10, etc.)
            now.minute() % 5 == 0
        }
        "coach" => {
            // Run every 15 minutes (XX:00, XX:15, XX:30, XX:45)
            now.minute() % 15 == 0
        }
        _ => {
            // Default: check elapsed time
            let interval_minutes = get_mode_interval_minutes(mode);
            match last_run {
                Some(last) => {
                    let elapsed = now.signed_duration_since(last);
                    elapsed.num_minutes() >= interval_minutes as i64
                }
                None => true,
            }
        }
    }
}


// Mode-specific summary generation and logic
async fn handle_mode_specific_logic(app: &AppHandle, mode: &str, state: &AppState) -> Result<(), String> {
    let now = Utc::now();
    
    // Check if we should run based on timing
    let last_run = {
        let times = state.last_summary_time.lock().map_err(|_| "Failed to acquire timing lock")?;
        times.get(mode).cloned()
    };
    
    if !should_run_summary(mode, last_run) {
        return Ok(());
    }
    
    // Update last run time
    {
        let mut times = state.last_summary_time.lock().map_err(|_| "Failed to acquire timing lock")?;
        times.insert(mode.to_string(), now);
    }
    
    send_log(app, "info", &format!("Running {} mode logic at {:02}:{:02}", mode, now.hour(), now.minute()));
    
    match mode {
        "ghost" => handle_ghost_mode(app).await,
        "chill" => handle_chill_mode(app).await,
        "study_buddy" => handle_study_mode(app).await,
        "coach" => handle_coach_mode(app).await,
        _ => Ok(()),
    }
}

async fn handle_ghost_mode(app: &AppHandle) -> Result<(), String> {
    send_log(app, "info", "Ghost mode: generating hourly summary");
    
    // Generate regular hourly summary (existing logic)
    let now = chrono::Local::now();
    let data_dir = std::path::PathBuf::from("data");
    std::fs::create_dir_all(&data_dir).map_err(|e| e.to_string())?;
    let summary_file = data_dir.join("hourly_summary.txt");
    
    let (summary_text, focus_score, current_state) = generate_new_hourly_summary(now, &summary_file).await?;
    
    // Save to JSON file for ghost mode
    let ghost_file = data_dir.join("ghost_summaries.json");
    let mut summaries: Vec<serde_json::Value> = if ghost_file.exists() {
        let content = std::fs::read_to_string(&ghost_file).unwrap_or_default();
        serde_json::from_str(&content).unwrap_or_default()
    } else {
        Vec::new()
    };
    
    summaries.push(serde_json::json!({
        "timestamp": now.format("%Y-%m-%d %H:%M:%S").to_string(),
        "hour": now.hour(),
        "mode": "ghost",
        "summary": summary_text,
        "focus_score": focus_score,
        "state": current_state
    }));
    
    std::fs::write(&ghost_file, serde_json::to_string_pretty(&summaries).unwrap())
        .map_err(|e| e.to_string())?;
    
    // Emit event to update frontend
    let hourly_summary = HourlySummary {
        summary: summary_text,
        focus_score,
        last_updated: now.format("%H:%M").to_string(),
        period: format!("{}-{}", 
                       (now - chrono::Duration::minutes(60)).format("%H:%M"),
                       now.format("%H:%M")),
        current_state,
    };
    
    send_log(app, "debug", &format!("Emitting hourly_summary_updated: {:?}", hourly_summary));
    
    // Store in app state
    if let Some(state) = app.try_state::<AppState>() {
        if let Ok(mut latest) = state.latest_hourly_summary.lock() {
            *latest = Some(hourly_summary.clone());
        }
    }
    
    app.emit("hourly_summary_updated", &hourly_summary)
        .map_err(|e| format!("Failed to emit summary update: {}", e))?;
    
    send_log(app, "info", "Ghost mode summary saved and UI updated");
    Ok(())
}

async fn handle_chill_mode(app: &AppHandle) -> Result<(), String> {
    send_log(app, "info", "Chill mode: checking for excessive fun");
    
    let aw_client = get_configured_aw_client().await;
    let aw_connected = aw_client.test_connection().await.connected;
    
    if !aw_connected {
        send_log(app, "warn", "ActivityWatch not connected, skipping chill mode check");
        return Ok(());
    }
    
    // Generate activity summary using the same logic as manual generation
    let now = chrono::Local::now();
    let (summary_text, focus_score, current_state) = generate_ai_summary(&aw_client, now).await?;
    
    // Check if user needs a nudge
    if current_state.contains("needs_nudge") || current_state.contains("distracted") {
        let config = load_user_config_internal().await.unwrap_or_default();
        send_notification(app, "Time for a change?", &config.chill_notification_prompt).await;
    }
    
    // Emit event to update frontend
    let hourly_summary = HourlySummary {
        summary: summary_text.clone(),
        focus_score,
        last_updated: now.format("%H:%M").to_string(),
        period: format!("{}-{}", 
                       (now - chrono::Duration::minutes(60)).format("%H:%M"),
                       now.format("%H:%M")),
        current_state: current_state.clone(),
    };
    
    send_log(app, "debug", &format!("Emitting hourly_summary_updated for chill mode: {:?}", hourly_summary));
    
    // Store in app state
    if let Some(state) = app.try_state::<AppState>() {
        if let Ok(mut latest) = state.latest_hourly_summary.lock() {
            *latest = Some(hourly_summary.clone());
        }
    }
    
    app.emit("hourly_summary_updated", &hourly_summary)
        .map_err(|e| format!("Failed to emit summary update: {}", e))?;
    
    // Log the summary
    send_log(app, "info", &format!("Chill mode summary: {} (state: {})", summary_text, current_state));
    
    send_log(app, "info", "Chill mode check completed");
    Ok(())
}

async fn handle_study_mode(app: &AppHandle) -> Result<(), String> {
    send_log(app, "info", "Study mode: checking focus on study topic");
    
    let aw_client = get_configured_aw_client().await;
    let aw_connected = aw_client.test_connection().await.connected;
    
    if !aw_connected {
        send_log(app, "warn", "ActivityWatch not connected, skipping study mode check");
        return Ok(());
    }
    
    let config = load_user_config_internal().await.unwrap_or_default();
    let study_focus = if config.study_focus.is_empty() {
        "general studying".to_string()
    } else {
        config.study_focus.clone()
    };
    
    // Generate study-focused summary using study context
    let now = chrono::Local::now();
    let (summary_text, focus_score, current_state) = generate_study_focused_summary(&aw_client, now, &study_focus).await?;
    
    // Check if user is distracted from studying
    if current_state == "needs_nudge" || current_state.contains("distracted") {
        send_notification(app, "Study Focus", &config.study_notification_prompt).await;
        send_log(app, "info", &format!("Study mode: User distracted, sending nudge. State: {}", current_state));
    } else if current_state == "flow" || current_state == "working" {
        // User is actively studying, do not disturb
        send_log(app, "info", "Study mode: User actively studying, no notification sent");
    }
    
    // Save summary
    let data_dir = std::path::PathBuf::from("data");
    std::fs::create_dir_all(&data_dir).map_err(|e| e.to_string())?;
    let summary_file = data_dir.join("study_summary.txt");
    
    let entry = format!("---\n{}\n{}\n{}\n{}\nMode: study\nFocus: {}\n", 
                       now.format("%H:%M"), summary_text, focus_score, current_state, study_focus);
    std::fs::write(&summary_file, entry).map_err(|e| e.to_string())?;
    
    // Emit event to update frontend
    let hourly_summary = HourlySummary {
        summary: summary_text.clone(),
        focus_score,
        last_updated: now.format("%H:%M").to_string(),
        period: format!("{}-{}", 
                       (now - chrono::Duration::minutes(5)).format("%H:%M"),
                       now.format("%H:%M")),
        current_state: current_state.clone(),
    };
    
    // Store in app state
    if let Some(state) = app.try_state::<AppState>() {
        if let Ok(mut latest) = state.latest_hourly_summary.lock() {
            *latest = Some(hourly_summary.clone());
        }
    }
    
    app.emit("hourly_summary_updated", &hourly_summary)
        .map_err(|e| format!("Failed to emit summary update: {}", e))?;
    
    send_log(app, "info", "Study mode check completed");
    Ok(())
}

async fn handle_coach_mode(app: &AppHandle) -> Result<(), String> {
    send_log(app, "info", "Coach mode: generating todo list");
    
    let aw_client = get_configured_aw_client().await;
    let aw_connected = aw_client.test_connection().await.connected;
    
    if !aw_connected {
        send_log(app, "warn", "ActivityWatch not connected, skipping coach mode check");
        return Ok(());
    }
    
    let config = load_user_config_internal().await.unwrap_or_default();
    let coach_task = if config.coach_task.is_empty() {
        "complete daily tasks".to_string()
    } else {
        config.coach_task.clone()
    };
    
    // Generate comprehensive activity summary like manual generation
    let now = chrono::Local::now();
    let (summary_text, focus_score, current_state) = generate_ai_summary(&aw_client, now).await?;
    
    // Also generate todo list for coach mode
    let todo_list = generate_coach_todo_list(&aw_client, now, &coach_task).await?;
    
    // Save todo list
    let data_dir = std::path::PathBuf::from("data");
    std::fs::create_dir_all(&data_dir).map_err(|e| e.to_string())?;
    let todo_file = data_dir.join("coach_todos.json");
    
    let json_content = serde_json::to_string_pretty(&todo_list).map_err(|e| e.to_string())?;
    std::fs::write(&todo_file, json_content).map_err(|e| e.to_string())?;
    
    // Send notification to check todos
    send_notification(app, "Coach Check-in", &config.coach_notification_prompt).await;
    
    // Emit event to update frontend with comprehensive summary
    let hourly_summary = HourlySummary {
        summary: summary_text,
        focus_score,
        last_updated: now.format("%H:%M").to_string(),
        period: format!("{}-{}", 
                       (now - chrono::Duration::minutes(15)).format("%H:%M"),
                       now.format("%H:%M")),
        current_state,
    };
    
    // Store in app state
    if let Some(state) = app.try_state::<AppState>() {
        if let Ok(mut latest) = state.latest_hourly_summary.lock() {
            *latest = Some(hourly_summary.clone());
        }
    }
    
    app.emit("hourly_summary_updated", &hourly_summary)
        .map_err(|e| format!("Failed to emit summary update: {}", e))?;
    
    send_log(app, "info", "Coach mode todo list generated");
    Ok(())
}

// Tauri commands
#[tauri::command]
async fn check_connections(app: AppHandle, _state: State<'_, AppState>) -> Result<ConnectionStatus, String> {
    send_log(&app, "debug", "Starting connection check...");
    
    // Test ActivityWatch connection
    let aw_client = get_configured_aw_client().await;
    let aw_test = aw_client.test_connection().await;
    
    if aw_test.connected {
        send_log(&app, "info", "ActivityWatch connection successful");
    } else {
        send_log(&app, "warn", "ActivityWatch connection failed");
    }
    
    // Test Ollama connection
    let ollama_available = test_ollama_connection().await;
    
    if ollama_available {
        send_log(&app, "info", "Ollama connection successful");
    } else {
        send_log(&app, "warn", "Ollama connection failed");
    }
    
    send_log(&app, "debug", "Connection check completed");
    
    Ok(ConnectionStatus {
        activitywatch: aw_test.connected,
        ollama: ollama_available,
        errors: aw_test.errors,
    })
}

#[tauri::command]
async fn get_current_mode(state: State<'_, AppState>) -> Result<String, String> {
    let mode = state.current_mode.lock().map_err(|_| "Failed to acquire mode lock")?;
    Ok(mode.clone())
}

#[tauri::command]
async fn set_mode(mode: String, state: State<'_, AppState>, app: AppHandle) -> Result<(), String> {
    send_log(&app, "info", &format!("Switching to {} mode", mode));
    
    {
        let mut current_mode = state.current_mode.lock().map_err(|_| "Failed to acquire mode lock")?;
        *current_mode = mode.clone();
    }
    
    // Save mode to persistent storage
    AppState::save_mode(&mode)?;
    
    // Update tray icon menu
    update_tray_menu(&app, &mode).map_err(|e| e.to_string())?;
    
    // Notify frontend
    app.emit("mode_changed", &mode).map_err(|e| e.to_string())?;
    
    // Generate immediate summary when switching to study mode
    if mode == "study_buddy" {
        send_log(&app, "info", "Study mode activated - generating immediate summary");
        let app_clone = app.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_study_mode(&app_clone).await {
                send_log(&app_clone, "error", &format!("Failed to generate study mode summary: {}", e));
            }
        });
    }
    
    // Clear last run time to ensure immediate execution
    {
        let mut times = state.last_summary_time.lock().map_err(|_| "Failed to acquire timing lock")?;
        times.remove(&mode);
    }
    
    // Generate summary immediately for the new mode
    send_log(&app, "info", "Generating initial summary for new mode");
    if let Err(e) = handle_mode_specific_logic(&app, &mode, &state).await {
        send_log(&app, "error", &format!("Failed to generate initial summary: {}", e));
    }
    
    send_log(&app, "debug", &format!("Mode switch to {} completed and saved", mode));
    
    Ok(())
}

#[tauri::command]
async fn get_hourly_summary(state: State<'_, AppState>) -> Result<HourlySummary, String> {
    let now = chrono::Local::now();
    
    // First check if we have a recent summary in memory
    if let Ok(latest) = state.latest_hourly_summary.lock() {
        if let Some(summary) = latest.as_ref() {
            return Ok(summary.clone());
        }
    }
    
    // Get current mode
    let current_mode = {
        let mode = state.current_mode.lock().map_err(|_| "Failed to acquire mode lock")?;
        mode.clone()
    };
    
    // Check for existing hourly summary file
    let data_dir = std::path::PathBuf::from("data");
    std::fs::create_dir_all(&data_dir).map_err(|e| e.to_string())?;
    
    let summary_file = data_dir.join("hourly_summary.txt");
    let current_hour = now.hour();
    
    // Check if we have a recent summary (within the last hour)
    let (summary_text, focus_score, current_state) = if summary_file.exists() {
        match std::fs::read_to_string(&summary_file) {
            Ok(content) => {
                // Parse the most recent summary from the appended file
                let entries: Vec<&str> = content.split("---").collect();
                if let Some(last_entry) = entries.last() {
                    let lines: Vec<&str> = last_entry.lines().filter(|line| !line.trim().is_empty()).collect();
                    if lines.len() >= 4 {
                        let stored_hour: u32 = lines[0].parse().unwrap_or(0);
                        let stored_summary = lines[1].to_string();
                        let stored_score: u32 = lines[2].parse().unwrap_or(calculate_time_based_focus_score(current_hour));
                        let stored_state = lines[3].to_string();
                        
                        // If summary is from the same hour, use it
                        if stored_hour == current_hour {
                            (stored_summary, stored_score, stored_state)
                        } else {
                            // Generate new summary for this hour based on current mode
                            generate_mode_specific_hourly_summary(&current_mode, now, &summary_file).await?
                        }
                    } else {
                        // Invalid format, generate new
                        generate_mode_specific_hourly_summary(&current_mode, now, &summary_file).await?
                    }
                } else {
                    // No entries found, generate new
                    generate_mode_specific_hourly_summary(&current_mode, now, &summary_file).await?
                }
            }
            Err(_) => {
                // Can't read file, generate new
                generate_mode_specific_hourly_summary(&current_mode, now, &summary_file).await?
            }
        }
    } else {
        // No summary file exists, generate new
        generate_mode_specific_hourly_summary(&current_mode, now, &summary_file).await?
    };
    
    let hourly_summary = HourlySummary {
        summary: summary_text,
        focus_score,
        last_updated: now.format("%H:%M").to_string(),
        period: format!("{}-{}", 
                       (now - Duration::minutes(30)).format("%H:%M"),
                       now.format("%H:%M")),
        current_state,
    };
    
    Ok(hourly_summary)
}

// Helper function to parse AI response
fn parse_ai_response(response: String) -> Result<(String, u32, String), String> {
    match serde_json::from_str::<serde_json::Value>(&response) {
        Ok(parsed) => {
            let summary = parsed.get("summary")
                .and_then(|s| s.as_str())
                .unwrap_or("AI generated summary")
                .to_string();
            
            let focus_score = parsed.get("focus_score")
                .and_then(|s| s.as_u64())
                .unwrap_or(50) as u32;
            
            let state = parsed.get("state")
                .and_then(|s| s.as_str())
                .unwrap_or("working")
                .to_string();
            
            Ok((summary, focus_score, state))
        }
        Err(_) => {
            // If parsing fails, extract what we can from the text
            let summary = if response.len() > 100 {
                format!("{}...", response.chars().take(100).collect::<String>())
            } else {
                response
            };
            
            Ok((summary, 50, "working".to_string()))
        }
    }
}

// Mode-specific summary generation functions
async fn generate_chill_mode_summary(aw_client: &ActivityWatchClient, now: chrono::DateTime<chrono::Local>) -> Result<(String, u32, String), String> {
    let activity_data = aw_client.get_activity_data().await?;
    
    // Analyze for excessive gaming/entertainment
    let prompt = format!(
        "Analyze chill mode activity for healthy balance vs excessive entertainment. Return JSON only.

Recent activity:
{}

CHILL MODE RULES:
- Flow: Creative/learning activities, active engagement
- Working: Light productivity mixed with breaks
- Needs_nudge: Excessive gaming (>2hrs), passive bingeing, social media spirals
- AFK: User away

Evaluate:
1. Time spent on each activity type
2. Active engagement vs passive consumption
3. Variety vs hyperfocus on one activity
4. Signs of healthy relaxation vs avoidance

Return JSON: {{\"summary\": \"brief analysis of leisure balance\", \"focus_score\": 0-100, \"state\": \"flow/working/needs_nudge/afk\"}}",
        activity_data
    );
    
    match call_ollama_api(&prompt).await {
        Ok(response) => parse_ai_response(response),
        Err(_) => {
            // Fallback analysis based on activity patterns
            let state = if activity_data.to_lowercase().contains("youtube") || 
                         activity_data.to_lowercase().contains("game") || 
                         activity_data.to_lowercase().contains("twitch") {
                "needs_nudge"
            } else {
                "working"
            };
            
            Ok((
                "Monitoring your chill time activities...".to_string(),
                calculate_time_based_focus_score(now.hour()),
                state.to_string()
            ))
        }
    }
}

async fn generate_study_mode_summary(aw_client: &ActivityWatchClient, now: chrono::DateTime<chrono::Local>, study_focus: &str) -> Result<(String, u32, String), String> {
    let activity_data = aw_client.get_activity_data().await?;
    
    let focus_context = if study_focus.is_empty() {
        "general studying".to_string()
    } else {
        study_focus.to_string()
    };
    
    let prompt = format!(
        "Analyze study focus for: '{}'. Return JSON only.

Recent activity:
{}

STUDY MODE ASSESSMENT:
- Focused: Study materials, research tools, note-taking apps aligned with objective
- Working: General productivity that may support learning
- Distracted: Off-topic browsing, organization tasks, mild diversions
- Gaming: High-distraction entertainment that competes with study focus

Evaluate:
1. Activity alignment with study objective
2. Deep focus vs fragmented attention
3. Learning-appropriate tools vs passive consumption
4. Signs of productive study vs procrastination

Return JSON: {{\"summary\": \"analysis of study focus with guidance\", \"focus_score\": 0-100, \"state\": \"focused/working/distracted/gaming\"}}",
        focus_context, activity_data
    );
    
    match call_ollama_api(&prompt).await {
        Ok(response) => parse_ai_response(response),
        Err(_) => {
            // Fallback analysis
            let state = if activity_data.to_lowercase().contains("youtube") || 
                         activity_data.to_lowercase().contains("game") || 
                         activity_data.to_lowercase().contains("social") {
                "distracted"
            } else {
                "focused"
            };
            
            Ok((
                format!("Monitoring study focus on: {}", focus_context),
                calculate_time_based_focus_score(now.hour()),
                state.to_string()
            ))
        }
    }
}

async fn generate_coach_todo_list(aw_client: &ActivityWatchClient, now: chrono::DateTime<chrono::Local>, coach_task: &str) -> Result<CoachTodoList, String> {
    let activity_data = aw_client.get_activity_data().await?;
    
    let task_context = if coach_task.is_empty() {
        "general productivity tasks".to_string()
    } else {
        coach_task.to_string()
    };
    
    let prompt = format!(
        "Generate 3-5 actionable todos based on goal: '{}' and recent activity. Return JSON only.

Recent activity:
{}

TODO GUIDELINES:
- Specific: Clear actions, not vague goals
- Achievable: 15-45 minute tasks
- Contextual: Match current tools and workflow
- Progressive: Each task sets up the next
- ADHD-friendly: Mix of challenging and easy wins

PRIORITIES:
- High: Critical path, momentum builders, blocking issues
- Medium: Important but flexible timing
- Low: Nice-to-have, administrative tasks

Analyze:
1. Current work patterns and tools in use
2. Natural next steps from recent activity
3. Balance between challenge and achievability
4. Alignment with stated goal

Return JSON: {{\"todos\": [{{\"text\": \"specific actionable todo item\", \"priority\": \"high/medium/low\"}}]}}",
        task_context, activity_data
    );
    
    match call_ollama_api(&prompt).await {
        Ok(response) => {
            // Parse AI response and create todo list
            match parse_todo_response(response) {
                Ok(todos) => Ok(CoachTodoList {
                    todos,
                    generated_at: now.format("%Y-%m-%d %H:%M:%S").to_string(),
                    context: task_context.to_string(),
                }),
                Err(_) => create_fallback_todo_list(&task_context, now),
            }
        }
        Err(_) => create_fallback_todo_list(&task_context, now),
    }
}

fn parse_todo_response(response: String) -> Result<Vec<TodoItem>, String> {
    let parsed: serde_json::Value = serde_json::from_str(&response)
        .map_err(|e| format!("Failed to parse todo response: {}", e))?;
    
    let todos_array = parsed.get("todos")
        .and_then(|t| t.as_array())
        .ok_or("No todos array found")?;
    
    let mut todos = Vec::new();
    for (i, todo_json) in todos_array.iter().enumerate() {
        let text = todo_json.get("text")
            .and_then(|t| t.as_str())
            .unwrap_or("Complete task")
            .to_string();
        
        todos.push(TodoItem {
            id: format!("todo_{}", i),
            text,
            completed: false,
            created_at: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        });
    }
    
    Ok(todos)
}

fn create_fallback_todo_list(coach_task: &str, now: chrono::DateTime<chrono::Local>) -> Result<CoachTodoList, String> {
    let todos = vec![
        TodoItem {
            id: "todo_0".to_string(),
            text: format!("Work on: {}", coach_task),
            completed: false,
            created_at: now.format("%Y-%m-%d %H:%M:%S").to_string(),
        },
        TodoItem {
            id: "todo_1".to_string(),
            text: "Review progress and adjust approach".to_string(),
            completed: false,
            created_at: now.format("%Y-%m-%d %H:%M:%S").to_string(),
        },
        TodoItem {
            id: "todo_2".to_string(),
            text: "Take a short break if needed".to_string(),
            completed: false,
            created_at: now.format("%Y-%m-%d %H:%M:%S").to_string(),
        },
    ];
    
    Ok(CoachTodoList {
        todos,
        generated_at: now.format("%Y-%m-%d %H:%M:%S").to_string(),
        context: coach_task.to_string(),
    })
}

async fn generate_mode_specific_hourly_summary(mode: &str, now: chrono::DateTime<chrono::Local>, summary_file: &std::path::Path) -> Result<(String, u32, String), String> {
    let aw_client = get_configured_aw_client().await;
    let aw_connected = aw_client.test_connection().await.connected;
    let ollama_available = test_ollama_connection().await;
    
    let (summary_text, focus_score, current_state) = if aw_connected && ollama_available {
        // Use mode-specific context for analysis
        match mode {
            "study_buddy" => {
                // Load study focus context
                let config = load_user_config_internal().await.unwrap_or_default();
                let study_focus = if config.study_focus.is_empty() {
                    "general studying".to_string()
                } else {
                    config.study_focus.clone()
                };
                match generate_study_focused_summary(&aw_client, now, &study_focus).await {
                    Ok(result) => result,
                    Err(_) => generate_time_based_summary(now)
                }
            },
            _ => {
                // All other modes use general context
                match generate_ai_summary(&aw_client, now).await {
                    Ok(result) => result,
                    Err(_) => generate_time_based_summary(now)
                }
            }
        }
    } else {
        // Fallback to time-based summary when connections unavailable
        generate_time_based_summary(now)
    };
    
    // Append the summary to file (don't overwrite)
    let file_content = format!("{}\n{}\n{}\n{}\n{}\n---\n", 
                              now.hour(), 
                              summary_text, 
                              focus_score,
                              current_state,
                              now.format("%Y-%m-%d %H:%M:%S"));
    
    use std::fs::OpenOptions;
    use std::io::Write;
    
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(summary_file)
        .map_err(|e| e.to_string())?;
    
    file.write_all(file_content.as_bytes()).map_err(|e| e.to_string())?;
    
    Ok((summary_text, focus_score, current_state))
}

async fn generate_new_hourly_summary(now: chrono::DateTime<chrono::Local>, summary_file: &std::path::Path) -> Result<(String, u32, String), String> {
    // Test both connections first
    let aw_client = get_configured_aw_client().await;
    let aw_connected = aw_client.test_connection().await.connected;
    let ollama_available = test_ollama_connection().await;
    
    let (summary_text, focus_score, current_state) = if aw_connected && ollama_available {
        // We have both connections - generate real AI summary
        match generate_ai_summary(&aw_client, now).await {
            Ok((ai_summary, ai_score, ai_state)) => {
                (ai_summary, ai_score, ai_state)
            },
            Err(e) => {
                eprintln!("AI summary failed: {}, falling back to time-based", e);
                generate_time_based_summary(now)
            }
        }
    } else {
        // Fallback to time-based summary
        generate_time_based_summary(now)
    };
    
    // Append the summary to file (don't overwrite)
    let file_content = format!("{}\n{}\n{}\n{}\n{}\n---\n", 
                              now.hour(), 
                              summary_text, 
                              focus_score,
                              current_state,
                              now.format("%Y-%m-%d %H:%M:%S"));
    
    use std::fs::OpenOptions;
    use std::io::Write;
    
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(summary_file)
        .map_err(|e| e.to_string())?;
    
    file.write_all(file_content.as_bytes()).map_err(|e| e.to_string())?;
    
    Ok((summary_text, focus_score, current_state))
}

fn calculate_time_based_focus_score(hour: u32) -> u32 {
    match hour {
        9..=11 => 88,   // Morning peak productivity
        14..=16 => 82,  // Afternoon focused work
        19..=21 => 75,  // Evening sessions
        7..=8 => 70,    // Early morning
        22..=23 => 65,  // Late evening
        _ => 60,        // Other times
    }
}

fn generate_time_based_summary(now: chrono::DateTime<chrono::Local>) -> (String, u32, String) {
    let current_hour = now.hour();
    let base_score = calculate_time_based_focus_score(current_hour);
    
    let summary = match current_hour {
        6..=8 => format!("Good morning! Your early start at {} shows great initiative. The system is tracking your morning productivity patterns and initial focus establishment.", now.format("%H:%M")),
        9..=11 => format!("Morning peak productivity detected! Your session from {} shows optimal focus window timing. Brain chemistry is naturally primed for deep work during this period.", (now - Duration::minutes(30)).format("%H:%M")),
        12..=13 => format!("Midday transition period from {}. Energy levels typically fluctuate during lunch hours. Consider this a natural rhythm rather than a productivity concern.", (now - Duration::minutes(30)).format("%H:%M")),
        14..=16 => format!("Afternoon focus session detected from {}. Post-lunch cognitive recovery shows good attention management. This is a strong secondary productivity window.", (now - Duration::minutes(30)).format("%H:%M")),
        17..=19 => format!("Evening work session from {}. Extended day productivity indicates strong work ethic. Monitor energy levels to maintain sustainable pace.", (now - Duration::minutes(30)).format("%H:%M")),
        20..=22 => format!("Late evening session from {}. Night owl productivity pattern detected. Ensure adequate rest for next day's cognitive performance.", (now - Duration::minutes(30)).format("%H:%M")),
        _ => format!("Activity session from {} logged. Productivity patterns vary by individual chronotype. System is learning your optimal focus windows.", (now - Duration::minutes(30)).format("%H:%M")),
    };
    
    // Determine current state based on time of day
    let current_state = match current_hour {
        9..=11 | 14..=16 => "working",  // Peak productivity hours
        7..=8 | 19..=21 => "working",   // Secondary work hours
        _ => "working",                 // Default to working during fallback
    };
    
    (summary, base_score, current_state.to_string())
}

// Event processing structures adapted from event_processor.rs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    timestamp: DateTime<Utc>,
    duration: f64,
    data: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProcessedEvent {
    app: String,
    title: String,
    duration_minutes: f64,
    timestamp: DateTime<Utc>,
    raw_duration_seconds: f64,
    timeframe_source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TimeframeData {
    timeframe: String,
    window_events: Vec<ProcessedEvent>,
    web_events: Vec<ProcessedEvent>,
    afk_events: Vec<Event>,
    statistics: TimeframeStatistics,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct TimeframeStatistics {
    total_events: usize,
    unique_apps: Vec<String>,
    unique_domains: Vec<String>,
    context_switches: u32,
    total_active_minutes: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TimelineEvent {
    event_type: String,
    name: String,
    title: String,
    duration_minutes: f64,
    timestamp: DateTime<Utc>,
    timeframe_source: String,
    priority: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ContextSwitch {
    from_app: String,
    to_app: String,
    timestamp: DateTime<Utc>,
    switch_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CrossTimeframePatterns {
    productivity_trend: String,
    focus_pattern: String,
    distraction_evolution: String,
    peak_activity_periods: Vec<String>,
    dominant_apps_by_timeframe: HashMap<String, Vec<String>>,
    web_browsing_behavior: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RawDataForLLM {
    timeframes: HashMap<String, TimeframeData>,
    activity_timeline: Vec<TimelineEvent>,
    context_switches: Vec<ContextSwitch>,
    patterns: CrossTimeframePatterns,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LLMAnalysis {
    current_state: String,
    focus_trend: String,
    distraction_trend: String,
    confidence: String,
    primary_activity: String,
    reasoning: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DailySummary {
    date: String,
    summary: String,
    total_active_time: f64,
    top_applications: Vec<String>,
    total_sessions: u32,
    generated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ThirtyMinuteSummary {
    timestamp: chrono::DateTime<chrono::Utc>,
    period: String,
    summary: String,
    active_minutes: f64,
    top_apps: Vec<String>,
}

/// Result of processing window events
struct ProcessedWindowEvents {
    events: Vec<ProcessedEvent>,
    unique_apps: Vec<String>,
    context_switches: u32,
    total_active_minutes: f64,
}

struct EventProcessor {
    // Simplified processor for LLM-based analysis
}

impl EventProcessor {
    fn new() -> Self {
        Self {}
    }


    /// Prepares raw data for LLM analysis
    fn prepare_raw_data_for_llm(&self, multi_timeframe_data: &HashMap<String, HashMap<String, Vec<Event>>>) -> RawDataForLLM {
        let mut raw_data = RawDataForLLM {
            timeframes: HashMap::new(),
            activity_timeline: Vec::new(),
            context_switches: Vec::new(),
            patterns: CrossTimeframePatterns {
                productivity_trend: "unknown".to_string(),
                focus_pattern: "unknown".to_string(),
                distraction_evolution: "unknown".to_string(),
                peak_activity_periods: Vec::new(),
                dominant_apps_by_timeframe: HashMap::new(),
                web_browsing_behavior: "unknown".to_string(),
            },
        };

        // Process each timeframe
        for (timeframe, data) in multi_timeframe_data {
            let mut timeframe_data = TimeframeData {
                timeframe: timeframe.clone(),
                window_events: Vec::new(),
                web_events: Vec::new(),
                afk_events: data.get("afk").cloned().unwrap_or_default(),
                statistics: TimeframeStatistics {
                    total_events: 0,
                    unique_apps: Vec::new(),
                    unique_domains: Vec::new(),
                    context_switches: 0,
                    total_active_minutes: 0.0,
                },
            };

            // Process window events
            if let Some(window_events) = data.get("window") {
                let processed = self.process_window_events_for_llm(window_events);
                timeframe_data.window_events = processed.events;
                timeframe_data.statistics.unique_apps = processed.unique_apps;
                timeframe_data.statistics.context_switches = processed.context_switches;
                timeframe_data.statistics.total_active_minutes = processed.total_active_minutes;
            }

            // Skip web events processing - removed due to timing inaccuracies
            timeframe_data.web_events = Vec::new();
            
            timeframe_data.statistics.total_events = 
                timeframe_data.window_events.len() + timeframe_data.web_events.len();

            raw_data.timeframes.insert(timeframe.clone(), timeframe_data);
        }

        // Generate comprehensive activity timeline
        raw_data.activity_timeline = self.create_prioritized_timeline(&raw_data.timeframes);

        // Extract context switches
        if let Some(recent_data) = raw_data.timeframes.get("5_minutes") {
            raw_data.context_switches = self.extract_context_switches(&recent_data.window_events);
        }

        // Analyze cross-timeframe patterns
        raw_data.patterns = self.analyze_cross_timeframe_patterns(&raw_data.timeframes);

        raw_data
    }

    /// Processes window events for LLM analysis
    fn process_window_events_for_llm(&self, events: &[Event]) -> ProcessedWindowEvents {
        let mut result = ProcessedWindowEvents {
            events: Vec::new(),
            unique_apps: Vec::new(),
            context_switches: 0,
            total_active_minutes: 0.0,
        };

        if events.is_empty() {
            return result;
        }

        // Sort by timestamp
        let mut sorted_events = events.to_vec();
        sorted_events.sort_by_key(|e| e.timestamp);

        let mut unique_apps_set = HashSet::new();
        let mut last_app: Option<String> = None;
        let mut total_duration = 0.0;

        for event in &sorted_events {
            let app = event.data.get("app")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            
            let title = event.data.get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            
            let duration = event.duration / 60.0; // Convert to minutes

            // Skip very short events (under 5 seconds)
            if duration >= 0.08 {
                let processed_event = ProcessedEvent {
                    app: app.clone(),
                    title,
                    duration_minutes: (duration * 100.0).round() / 100.0, // Round to 2 decimal places
                    timestamp: event.timestamp,
                    raw_duration_seconds: event.duration,
                    timeframe_source: None,
                };
                result.events.push(processed_event);

                // Track statistics
                if !app.is_empty() {
                    unique_apps_set.insert(app.to_lowercase());
                    total_duration += duration;

                    // Count context switches
                    if let Some(ref last) = last_app {
                        if last != &app.to_lowercase() {
                            result.context_switches += 1;
                        }
                    }
                    last_app = Some(app.to_lowercase());
                }
            }
        }

        result.unique_apps = unique_apps_set.into_iter().collect();
        result.total_active_minutes = (total_duration * 100.0).round() / 100.0;

        result
    }

    /// Creates a prioritized timeline for LLM analysis
    fn create_prioritized_timeline(&self, timeframes: &HashMap<String, TimeframeData>) -> Vec<TimelineEvent> {
        let mut timeline = Vec::new();

        // Priority 1: All events from 5-minute timeframe (but limit to 30 total)
        if let Some(five_min_data) = timeframes.get("5_minutes") {
            let mut five_min_events = Vec::new();

            for event in &five_min_data.window_events {
                five_min_events.push(TimelineEvent {
                    event_type: "app".to_string(),
                    name: event.app.clone(),
                    title: event.title.clone(),
                    duration_minutes: event.duration_minutes,
                    timestamp: event.timestamp,
                    timeframe_source: "5_minutes".to_string(),
                    priority: "current".to_string(),
                });
            }

            // Sort 5-minute events by recency and limit to 30
            five_min_events.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
            timeline.extend(five_min_events.into_iter().take(30));
        }

        // Priority 2: Representative events from longer timeframes for context
        for timeframe_name in &["10_minutes", "30_minutes", "1_hour"] {
            if let Some(timeframe_data) = timeframes.get(*timeframe_name) {
                // Get most significant events (longest duration)
                let mut significant_events: Vec<_> = timeframe_data.window_events.iter()
                    .map(|event| TimelineEvent {
                        event_type: "app".to_string(),
                        name: event.app.clone(),
                        title: event.title.clone(),
                        duration_minutes: event.duration_minutes,
                        timestamp: event.timestamp,
                        timeframe_source: timeframe_name.to_string(),
                        priority: "context".to_string(),
                    })
                    .collect();

                significant_events.sort_by(|a, b| {
                    b.duration_minutes.partial_cmp(&a.duration_minutes).unwrap()
                });

                // Add top 5 longest events from each timeframe
                for event in significant_events.into_iter().take(5) {
                    // Avoid duplicates from 5-minute timeframe
                    let is_duplicate = timeline.iter().any(|e| {
                        e.timestamp == event.timestamp && e.event_type == "app"
                    });
                    
                    if !is_duplicate {
                        timeline.push(event);
                    }
                }
            }
        }

        // Sort final timeline by priority (current first) then by timestamp
        timeline.sort_by(|a, b| {
            match (a.priority.as_str(), b.priority.as_str()) {
                ("current", "context") => std::cmp::Ordering::Less,
                ("context", "current") => std::cmp::Ordering::Greater,
                _ => b.timestamp.cmp(&a.timestamp),
            }
        });

        // Final limit to ensure we don't exceed context window
        timeline.into_iter().take(150).collect()
    }

    /// Extracts context switches from window events
    fn extract_context_switches(&self, window_events: &[ProcessedEvent]) -> Vec<ContextSwitch> {
        let mut switches = Vec::new();
        let mut last_app: Option<String> = None;

        for event in window_events {
            if let Some(ref last) = last_app {
                if last != &event.app {
                    switches.push(ContextSwitch {
                        from_app: last.clone(),
                        to_app: event.app.clone(),
                        timestamp: event.timestamp,
                        switch_type: "app_change".to_string(),
                    });
                }
            }
            last_app = Some(event.app.clone());
        }

        switches
    }

    /// Analyzes patterns across different timeframes
    fn analyze_cross_timeframe_patterns(&self, timeframes: &HashMap<String, TimeframeData>) -> CrossTimeframePatterns {
        let mut patterns = CrossTimeframePatterns {
            productivity_trend: "unknown".to_string(),
            focus_pattern: "unknown".to_string(),
            distraction_evolution: "unknown".to_string(),
            peak_activity_periods: Vec::new(),
            dominant_apps_by_timeframe: HashMap::new(),
            web_browsing_behavior: "unknown".to_string(),
        };

        // Analyze productivity trend across timeframes
        let timeframe_order = vec!["5_minutes", "10_minutes", "30_minutes", "1_hour"];
        let mut switch_counts = Vec::new();

        for tf in &timeframe_order {
            if let Some(tf_data) = timeframes.get(*tf) {
                switch_counts.push(tf_data.statistics.context_switches);
            }
        }

        // Determine productivity trend
        if switch_counts.len() >= 2 {
            let recent_switches: u32 = switch_counts.iter().take(2).sum();
            let older_switches: u32 = switch_counts.iter().skip(2).sum();

            if recent_switches > (older_switches as f64 * 1.5) as u32 {
                patterns.productivity_trend = "declining".to_string();
            } else if (recent_switches as f64) < older_switches as f64 * 0.5 {
                patterns.productivity_trend = "improving".to_string();
            } else {
                patterns.productivity_trend = "stable".to_string();
            }
        }

        // Analyze dominant apps by timeframe
        for (tf_name, tf_data) in timeframes {
            let apps: Vec<String> = tf_data.statistics.unique_apps.iter()
                .take(3)
                .cloned()
                .collect();
            patterns.dominant_apps_by_timeframe.insert(tf_name.clone(), apps);
        }

        // Analyze web browsing behavior
        patterns.web_browsing_behavior = "normal_browsing".to_string();

        patterns
    }

    fn create_state_analysis_prompt(&self, raw_data: &RawDataForLLM, user_context: &str) -> String {
        let recent_timeframe = raw_data.timeframes.get("5_minutes");
        let medium_timeframe = raw_data.timeframes.get("30_minutes");
        
        let recent_stats = recent_timeframe
            .map(|tf| &tf.statistics)
            .cloned()
            .unwrap_or_default();
        let medium_stats = medium_timeframe
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
STATUS: {} | Recent: {}m active, {} switches, {} apps | Longer: {}m active, {} switches, {} apps

RECENT ACTIVITY:
{}

CONTEXT SWITCHES:
{}

ANALYSIS RULES:
- Flow: 15+ min single app, minimal switches
- Working: Mixed productive apps, <5 switches
- Needs_nudge: 5+ switches OR games/entertainment
- AFK: User away

Evaluate:
1. App types and session lengths
2. Switch frequency patterns
3. Alignment with user context
4. Trend: improving/declining/stable focus

Return JSON only:
{{
 "current_state": "flow|working|needs_nudge|afk",
 "focus_trend": "maintaining_focus|entering_focus|losing_focus|variable|none", 
 "distraction_trend": "low|moderate|increasing|decreasing|high",
 "confidence": "high|medium|low",
 "primary_activity": "Brief description of main activity",
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
            self.format_timeline_for_prompt(timeline),
            self.format_context_switches_for_prompt(context_switches)
        )
    }


    fn format_timeline_for_prompt(&self, timeline: &[TimelineEvent]) -> String {
        if timeline.is_empty() {
            return "No activity detected".to_string();
        }
        
        let mut formatted = Vec::new();
        for event in timeline.iter().take(10) {
            let title_part = if event.title.is_empty() { "" } else { &format!(" → {}", event.title) };
            let (app_name, _exe_name) = extract_app_and_exe_name(&event.name);
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
            let (from_app, _from_exe) = extract_app_and_exe_name(&switch.from_app);
            let (to_app, _to_exe) = extract_app_and_exe_name(&switch.to_app);
            formatted.push(format!(
                "{} → {} at {}",
                from_app,
                to_app,
                switch.timestamp.format("%H:%M")
            ));
        }
        
        formatted.join("\n")
    }
}

async fn generate_daily_summary(aw_client: &ActivityWatchClient) -> Result<DailySummary, String> {
    // Load user configuration
    let user_config = load_user_config_internal().await.unwrap_or_default();
    eprintln!("Using user context for daily summary: {}", user_config.user_context);
    
    let now = chrono::Local::now();
    
    // Calculate correct time range: from start of today to now
    let day_start = now.date_naive()
        .and_hms_opt(0, 0, 0)
        .ok_or("Failed to create start of day")?
        .and_local_timezone(chrono::Local)
        .single()
        .ok_or("Failed to convert to local timezone")?;
    let day_start_utc = day_start.with_timezone(&chrono::Utc);
    let now_utc = now.with_timezone(&chrono::Utc);
    
    
    // Test ActivityWatch connection first
    let _buckets = match aw_client.get_buckets().await {
        Ok(buckets) => buckets,
        Err(e) => {
            eprintln!("ERROR: Failed to connect to ActivityWatch: {}", e);
            return Err(format!("ActivityWatch connection failed: {}", e));
        }
    };
    
    // Skip verbose data availability tests for performance
    
    // Get window events filtered by non-AFK periods
    let window_events_json = match aw_client.get_active_window_events(day_start_utc, now_utc).await {
        Ok(events) => events,
        Err(e) => {
            eprintln!("ERROR: Failed to get active window events: {}", e);
            Vec::new()
        }
    };
    
    // Convert JSON events to Event structs for compatibility
    let mut window_events = Vec::new();
    for event_json in window_events_json {
        if let Ok(timestamp_str) = event_json.get("timestamp")
            .and_then(|v| v.as_str())
            .ok_or("Missing timestamp") {
            if let Ok(timestamp) = DateTime::parse_from_rfc3339(timestamp_str) {
                let timestamp = timestamp.with_timezone(&Utc);
                let duration = event_json.get("duration").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let data = event_json.get("data").and_then(|v| v.as_object()).cloned().unwrap_or_default();
                let data_map: HashMap<String, serde_json::Value> = data.into_iter().collect();
                
                window_events.push(Event {
                    timestamp,
                    duration,
                    data: data_map,
                });
            }
        }
    }
    
    eprintln!("Converted {} events to Event structs", window_events.len());
    if !window_events.is_empty() {
        eprintln!("First active event: timestamp={}, app={}", 
            window_events[0].timestamp,
            window_events[0].data.get("app").unwrap_or(&serde_json::Value::String("unknown".to_string()))
        );
    }
    
    let _afk_events = match aw_client.get_events_in_range("afk", day_start_utc, now_utc).await {
        Ok(events) => {
            eprintln!("Successfully retrieved {} AFK events for today", events.len());
            events
        }
        Err(e) => {
            eprintln!("ERROR: Failed to get AFK events: {}", e);
            Vec::new()
        }
    };
    
    // If no data for full day, try a shorter range (last 4 hours) - still using AFK filtering
    let final_window_events = if window_events.is_empty() {
        eprintln!("No active data for full day, trying last 4 hours with AFK filtering...");
        let four_hours_ago = now_utc - chrono::Duration::hours(4);
        
        // Get AFK-filtered window events for last 4 hours
        let window_events_4h_json = aw_client.get_active_window_events(four_hours_ago, now_utc).await.unwrap_or_default();
        
        // Convert to Event structs
        let mut window_4h = Vec::new();
        for event_json in window_events_4h_json {
            if let Ok(timestamp_str) = event_json.get("timestamp")
                .and_then(|v| v.as_str())
                .ok_or("Missing timestamp") {
                if let Ok(timestamp) = DateTime::parse_from_rfc3339(timestamp_str) {
                    let timestamp = timestamp.with_timezone(&Utc);
                    let duration = event_json.get("duration").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let data = event_json.get("data").and_then(|v| v.as_object()).cloned().unwrap_or_default();
                    let data_map: HashMap<String, serde_json::Value> = data.into_iter().collect();
                    
                    window_4h.push(Event {
                        timestamp,
                        duration,
                        data: data_map,
                    });
                }
            }
        }
        
        eprintln!("4-hour fallback with AFK filtering: {} active window events", window_4h.len());
        window_4h
    } else {
        window_events
    };
    
    // Process and analyze daily data
    let mut app_durations: HashMap<String, f64> = HashMap::new();
    let mut app_titles: HashMap<String, Vec<String>> = HashMap::new();
    let mut total_active_time = 0.0;
    let mut session_count = 0;
    
    eprintln!("=== Processing {} window events ===", final_window_events.len());
    
    // Calculate total time spent in each application and collect titles
    for (i, event) in final_window_events.iter().enumerate() {
        if let Some(app) = event.data.get("app").and_then(|v| v.as_str()) {
            let duration_minutes = event.duration / 60.0;
            let title = event.data.get("title").and_then(|v| v.as_str()).unwrap_or("Unknown");
            
            *app_durations.entry(app.to_string()).or_insert(0.0) += duration_minutes;
            
            // Collect unique titles for each app (up to 3 most common)
            let titles = app_titles.entry(app.to_string()).or_insert_with(Vec::new);
            if !titles.contains(&title.to_string()) && titles.len() < 3 {
                titles.push(title.to_string());
            }
            
            total_active_time += duration_minutes;
            session_count += 1;
            
            if i < 3 { // Log first few events for debugging
                eprintln!("Event {}: {} - {} for {:.1} min", i+1, app, title, duration_minutes);
            }
        }
    }
    
    eprintln!("=== Processing Results ===");
    eprintln!("Total active time: {:.1} minutes ({:.1} hours)", total_active_time, total_active_time / 60.0);
    eprintln!("Total sessions: {}", session_count);
    eprintln!("Unique applications: {}", app_durations.len());
    
    // Get top applications by time spent
    let mut top_apps: Vec<(String, f64)> = app_durations.into_iter().collect();
    top_apps.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let top_applications: Vec<String> = top_apps.iter()
        .take(5)
        .map(|(app, time)| {
            let titles = app_titles.get(app).cloned().unwrap_or_default();
            if titles.is_empty() {
                format!("{} ({:.1}h)", app, time / 60.0)
            } else {
                // Include up to 2 sample titles
                let title_sample = titles.iter()
                    .take(2)
                    .map(|t| {
                        // Truncate long titles (UTF-8 safe)
                        if t.chars().count() > 40 {
                            format!("{}...", t.chars().take(37).collect::<String>())
                        } else {
                            t.clone()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{} ({:.1}h) - {}", app, time / 60.0, title_sample)
            }
        })
        .collect();
    
    // Calculate time periods and patterns
    let active_hours = total_active_time / 60.0;
    let productivity_ratio = if total_active_time > 0.0 {
        // Simple heuristic: longer app sessions = more focused
        let avg_session_length = total_active_time / session_count.max(1) as f64;
        (avg_session_length / 10.0).min(1.0) * 100.0
    } else {
        0.0
    };
    
    // Create comprehensive daily summary prompt
    let prompt = format!(
        r#"Create an empathetic daily productivity summary. Address user as 'you'. Plain text only, no JSON or formatting.

USER CONTEXT: {}
DATE: {}
ACTIVE TIME: {:.1}h (excluding breaks)
SWITCHES: {} | AVG SESSION: {:.1}min | FOCUS SCORE: {:.0}%
WORK PERIOD: {} to {} | TOTAL APPS: {}

TOP APPLICATIONS:
{}

ANALYSIS GUIDELINES:
- Acknowledge main activities and time investment
- Identify positive patterns or achievements
- Gently suggest improvement areas
- Consider focus quality vs quantity
- Assess work-life balance

Write 2-3 supportive sentences under 100 words. Be encouraging about progress while noting areas for optimization. Focus on actionable insights rather than judgment."#,
        now.format("%A, %B %d, %Y"),
        user_config.user_context,
        active_hours,
        session_count,
        if session_count > 0 { total_active_time / session_count as f64 } else { 0.0 },
        productivity_ratio,
        if top_applications.is_empty() {
            "No significant application usage detected".to_string()
        } else {
            top_applications.join("\n")
        },
        day_start.format("%H:%M"),
        now.format("%H:%M"),
        top_apps.len()
    );
    
    // Reduce verbose logging
    eprintln!("Calling LLM for daily summary...");
    
    // Call LLM for daily summary
    let summary_text = match call_ollama_api(&prompt).await {
        Ok(response) => {
            // eprintln!("LLM Response received: {} characters", response.len());
            
            // Try to parse JSON if the LLM returned JSON despite instructions
            let cleaned_response = if response.trim().starts_with('{') {
                // LLM returned JSON, try to extract the summary field
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&response) {
                    if let Some(summary) = json.get("Summary").and_then(|v| v.as_str()) {
                        summary.to_string()
                    } else if let Some(summary) = json.get("summary").and_then(|v| v.as_str()) {
                        summary.to_string()
                    } else {
                        // Try to concatenate all string values in the JSON
                        let mut parts = Vec::new();
                        if let Some(obj) = json.as_object() {
                            for (_, value) in obj {
                                if let Some(s) = value.as_str() {
                                    parts.push(s);
                                }
                            }
                        }
                        parts.join(" ")
                    }
                } else {
                    // Failed to parse JSON, use the raw response
                    response
                }
            } else {
                // Not JSON, use as-is
                response
            };
            
            // Clean any remaining formatting issues
            let final_response = cleaned_response
                .replace("\\n", " ")
                .replace("\\\"", "\"")
                .trim()
                .to_string();
            
            if final_response.len() > 50 {
                final_response
            } else {
                eprintln!("LLM response too short after cleaning, using fallback");
                format!("Daily Summary: {:.1}h active across {} applications. Primary focus: {}. {} total sessions with {:.1} min average duration.",
                    active_hours,
                    top_apps.len(),
                    top_apps.first().map(|(app, _)| app.as_str()).unwrap_or("various tasks"),
                    session_count,
                    if session_count > 0 { total_active_time / session_count as f64 } else { 0.0 }
                )
            }
        }
        Err(e) => {
            eprintln!("LLM call failed: {}, using fallback", e);
            format!("\n\nToday's Activity: {:.1} hours active, {} applications used. Top focus: {}. Productivity sessions: {}.",
                active_hours,
                top_apps.len(),
                top_apps.first().map(|(app, _)| app.as_str()).unwrap_or("various tasks"),
                session_count
            )
        }
    };
    
    eprintln!("=== Final Daily Summary ===");
    eprintln!("Summary text: {}", summary_text);
    eprintln!("Active hours: {:.1}", active_hours);
    eprintln!("Total sessions: {}", session_count);
    
    // Save daily summary to separate file
    let data_dir = std::path::PathBuf::from("data");
    std::fs::create_dir_all(&data_dir).map_err(|e| e.to_string())?;
    let daily_summary_file = data_dir.join("daily_summary.txt");
    
    let daily_summary_content = format!(
        "Date: {}\nGenerated at: {}\nActive time: {:.1} hours\nTotal sessions: {}\nTop applications:\n{}\n\nSummary:\n{}\n\n{}\n",
        now.format("%Y-%m-%d"),
        now.format("%Y-%m-%d %H:%M:%S"),
        active_hours,
        session_count,
        top_applications.join("\n"),
        summary_text,
        "=".repeat(80)
    );
    
    use std::fs::OpenOptions;
    use std::io::Write;
    
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&daily_summary_file)
        .map_err(|e| e.to_string())?;
    
    file.write_all(daily_summary_content.as_bytes()).map_err(|e| e.to_string())?;
    
    eprintln!("\n\nDaily summary saved to: {:?}", daily_summary_file);
    
    Ok(DailySummary {
        date: now.format("%Y-%m-%d").to_string(),
        summary: summary_text,
        total_active_time: active_hours,
        top_applications: top_applications,
        total_sessions: session_count,
        generated_at: now.format("%H:%M").to_string(),
    })
}

async fn generate_ai_summary(aw_client: &ActivityWatchClient, now: chrono::DateTime<chrono::Local>) -> Result<(String, u32, String), String> {
    // Load user configuration
    let user_config = load_user_config_internal().await.unwrap_or_default();
    generate_ai_summary_with_context(aw_client, now, &user_config.user_context).await
}

async fn generate_study_focused_summary(aw_client: &ActivityWatchClient, now: chrono::DateTime<chrono::Local>, study_focus: &str) -> Result<(String, u32, String), String> {
    let study_context = format!("Currently studying: {}. Focus on study-related activities, educational content, and potential distractions from this learning objective.", study_focus);
    generate_ai_summary_with_context(aw_client, now, &study_context).await
}

async fn generate_ai_summary_with_context(aw_client: &ActivityWatchClient, now: chrono::DateTime<chrono::Local>, context: &str) -> Result<(String, u32, String), String> {
    // Initialize event processor
    let processor = EventProcessor::new();
    
    // Get multi-timeframe data from ActivityWatch with AFK filtering
    let multi_timeframe_data = match aw_client.get_multi_timeframe_data_active().await {
        Ok(data) => data,
        Err(e) => {
            eprintln!("Failed to fetch multi-timeframe data: {}", e);
            return Ok(generate_time_based_summary(now));
        }
    };
    
    // Prepare raw data for LLM analysis using the proper EventProcessor method
    let raw_data = processor.prepare_raw_data_for_llm(&multi_timeframe_data);
    
    // Create the comprehensive state analysis prompt with custom context
    let prompt = processor.create_state_analysis_prompt(&raw_data, context);
    
    // Call Ollama API with the proper prompt
    let ollama_response = call_ollama_api(&prompt).await?;
    
    // Parse the LLM analysis response with robust JSON extraction
    match parse_llm_response(&ollama_response) {
        Ok(analysis) => {
            // Generate user-friendly summary from LLM analysis
            let summary = generate_user_summary(&analysis);
            let focus_score = calculate_focus_score_from_analysis(&analysis, now.hour());
            
            Ok((summary, focus_score, analysis.current_state))
        },
        Err(e) => {
            eprintln!("Failed to parse LLM response: {}", e);
            eprintln!("Raw response: {}", ollama_response);
            
            // Create a meaningful fallback based on whether we have activity data
            let summary = if raw_data.activity_timeline.is_empty() {
                "💤 No significant activity detected in recent time window. Take a break or start a new task when ready.".to_string()
            } else {
                format!("💼 Working with {} for the past 30 minutes. Keep up the productive momentum!", 
                    raw_data.activity_timeline.first()
                        .map(|e| e.name.as_str())
                        .unwrap_or("current application"))
            };
            let score = calculate_time_based_focus_score(now.hour());
            let fallback_state = if raw_data.activity_timeline.is_empty() { "afk" } else { "working" };
            Ok((summary, score, fallback_state.to_string()))
        }
    }
}

/// Robust JSON parsing that can handle partial/malformed LLM responses
fn parse_llm_response(response: &str) -> Result<LLMAnalysis, String> {
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
    let focus_trend = extract_field(cleaned_response, "focus_trend").unwrap_or("stable".to_string());
    let distraction_trend = extract_field(cleaned_response, "distraction_trend").unwrap_or("low".to_string());
    let confidence = extract_field(cleaned_response, "confidence").unwrap_or("medium".to_string());
    let primary_activity = extract_field(cleaned_response, "primary_activity").unwrap_or("current work".to_string());
    let reasoning = extract_field(cleaned_response, "reasoning").unwrap_or("Activity analysis based on current session.".to_string());
    
    Ok(LLMAnalysis {
        current_state,
        focus_trend,
        distraction_trend,
        confidence,
        primary_activity,
        reasoning,
    })
}

/// Extract a field value from JSON-like text
fn extract_field(text: &str, field_name: &str) -> Option<String> {
    let pattern = format!("\"{}\"", field_name);
    if let Some(start) = text.find(&pattern) {
        if let Some(colon_pos) = text[start..].find(':') {
            let value_start = start + colon_pos + 1;
            let remainder = &text[value_start..];
            
            // Skip whitespace and quotes
            let remainder = remainder.trim_start().trim_start_matches('"');
            
            // Find the end of the value (either quote or comma)
            if let Some(end) = remainder.find('"').or_else(|| remainder.find(',')) {
                return Some(remainder[..end].trim().to_string());
            }
        }
    }
    None
}

// Removed redundant fetch_activity_data function - now using ActivityWatchClient::get_multi_timeframe_data() and EventProcessor::prepare_raw_data_for_llm()


// Helper function to extract clean app name and exe name from full path
fn extract_app_and_exe_name(full_path: &str) -> (String, String) {
    let path = full_path.trim();
    
    // Handle common patterns
    if path.is_empty() {
        return ("Unknown".to_string(), "unknown".to_string());
    }
    
    // Extract just the filename from the path
    let filename = if path.contains('\\') {
        path.split('\\').last().unwrap_or(path)
    } else if path.contains('/') {
        path.split('/').last().unwrap_or(path)
    } else {
        path
    };
    
    // Remove .exe extension if present
    let app_name = if filename.to_lowercase().ends_with(".exe") {
        &filename[..filename.len() - 4]
    } else {
        filename
    };
    
    // Create a clean display name
    let clean_name = match app_name.to_lowercase().as_str() {
        "chrome" => "Google Chrome",
        "firefox" => "Firefox",
        "code" => "VS Code",
        "devenv" => "Visual Studio",
        "notepad++" => "Notepad++",
        "slack" => "Slack",
        "discord" => "Discord",
        "teams" => "Microsoft Teams",
        "outlook" => "Outlook",
        "spotify" => "Spotify",
        "explorer" => "File Explorer",
        "cmd" => "Command Prompt",
        "powershell" => "PowerShell",
        "windowsterminal" => "Windows Terminal",
        _ => app_name,
    };
    
    (clean_name.to_string(), filename.to_string())
}

// Find bucket by exact prefix pattern (e.g., "aw-watcher-window" matches "aw-watcher-window_HarryYu-Desktop")
fn find_bucket_by_exact_prefix(buckets: &HashMap<String, serde_json::Value>, prefix: &str) -> Option<String> {
    // Look for buckets that start with the exact prefix
    // This handles computer-specific suffixes like "_HarryYu-Desktop"
    for (bucket_id, _) in buckets {
        if bucket_id.starts_with(prefix) {
            // Ensure this is a standard ActivityWatch bucket, not a plugin
            // Standard buckets follow pattern: aw-watcher-{type}_{hostname}
            if prefix == "aw-watcher-window" && bucket_id.starts_with("aw-watcher-window") {
                // eprintln!("Found window bucket: {}", bucket_id);
                return Some(bucket_id.clone());
            } else if prefix == "aw-watcher-afk" && bucket_id.starts_with("aw-watcher-afk") {
                // eprintln!("Found AFK bucket: {}", bucket_id);
                return Some(bucket_id.clone());
            }
        }
    }
    
    // eprintln!("WARNING: No bucket found with prefix: {}", prefix);
    None
}

// Removed redundant process_window_events function - now using EventProcessor::process_window_events_for_llm()

// Removed redundant calculate_timeframe_statistics function - now handled in EventProcessor::process_window_events_for_llm()

// Removed redundant create_activity_timeline function - now using EventProcessor::create_prioritized_timeline()

// Removed redundant extract_context_switches function - now using EventProcessor::extract_context_switches()

// Removed redundant analyze_patterns function - now using EventProcessor::analyze_cross_timeframe_patterns()

fn clean_bracketed_text(text: &str) -> String {
    let mut cleaned = text.trim();
    
    // Remove common prefixes that break formatting
    cleaned = cleaned
        .strip_prefix("📊 Activity detected: [.")
        .or_else(|| cleaned.strip_prefix("📊 Activity detected: ["))
        .or_else(|| cleaned.strip_prefix("📊 Activity detected:"))
        .or_else(|| cleaned.strip_prefix("Activity detected: ["))
        .or_else(|| cleaned.strip_prefix("Activity detected:"))
        .unwrap_or(cleaned);
    
    // Remove brackets
    cleaned = cleaned
        .strip_prefix('[')
        .unwrap_or(cleaned)
        .strip_suffix(']')
        .unwrap_or(cleaned);
    
    // Remove trailing period if it's the only character
    if cleaned == "." {
        return String::new();
    }
    
    cleaned.trim().to_string()
}

fn generate_user_summary(analysis: &LLMAnalysis) -> String {
    let clean_activity = clean_bracketed_text(&analysis.primary_activity);
    let clean_reasoning = clean_bracketed_text(&analysis.reasoning);
    
    match analysis.current_state.as_str() {
        "flow" => format!("🔥 Flow state detected! {}. {}", clean_activity, clean_reasoning),
        "working" => format!("💼 Working... {}. {}", clean_activity, clean_reasoning),
        "needs_nudge" => format!("🎯 You need a nudge bro. {}. {}", clean_activity, clean_reasoning),
        "afk" => format!("💤 Welcome back! Ready to dive into your next task. {}", clean_reasoning),
        _ => format!("{}. {}", clean_activity, clean_reasoning),
    }
}

fn calculate_focus_score_from_analysis(analysis: &LLMAnalysis, hour: u32) -> u32 {
    let base_score = calculate_time_based_focus_score(hour);
    
    // Adjust based on LLM analysis
    let state_modifier = match analysis.current_state.as_str() {
        "flow" => 15,
        "working" => 5,
        "needs_nudge" => -10,
        "afk" => -20,
        _ => 0,
    };
    
    let focus_trend_modifier = match analysis.focus_trend.as_str() {
        "maintaining_focus" => 10,
        "entering_focus" => 5,
        "losing_focus" => -5,
        "variable" => -3,
        _ => 0,
    };
    
    let confidence_modifier = match analysis.confidence.as_str() {
        "high" => 0,
        "medium" => -2,
        "low" => -5,
        _ => 0,
    };
    
    let final_score = (base_score as i32 + state_modifier + focus_trend_modifier + confidence_modifier).max(0).min(100) as u32;
    final_score
}

async fn call_ollama_api(prompt: &str) -> Result<String, String> {
    let client = get_ollama_client();
    let config = load_user_config_internal().await.unwrap_or_default();
    
    eprintln!("=== Sending prompt to Ollama ===");
    eprintln!("Prompt length: {} characters", prompt.len());
    eprintln!("Prompt preview:\n{}", prompt.chars().take(500).collect::<String>());
    
    let payload = serde_json::json!({
        "model": "mistral",
        "prompt": prompt,
        "system": "You are a supportive ADHD productivity assistant. You MUST respond with ONLY valid JSON format, no other text or commentary. Be encouraging and provide actionable insights within the JSON structure. Address the user as you",
        "stream": false,
        "options": {
            "temperature": 0.3,
            "num_predict": 300,
            "num_ctx": 2048,
            "top_k": 40,
            "top_p": 0.9
        }
    });
    
    let response = client
        .post(format!("http://localhost:{}/api/generate", config.ollama_port))
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("Failed to send request to Ollama: {}", e))?;
    
    if !response.status().is_success() {
        return Err(format!("Ollama returned error status: {}", response.status()));
    }
    
    let response_text = response.text().await
        .map_err(|e| format!("Failed to read Ollama response: {}", e))?;
    
    eprintln!("=== Ollama Response ===");
    eprintln!("Response length: {} characters", response_text.len());
    
    // Parse Ollama's response format
    match serde_json::from_str::<serde_json::Value>(&response_text) {
        Ok(json) => {
            if let Some(response_content) = json["response"].as_str() {
                Ok(response_content.to_string())
            } else {
                Err("No 'response' field in Ollama response".to_string())
            }
        },
        Err(e) => Err(format!("Failed to parse Ollama JSON response: {}", e))
    }
}

#[tauri::command]
async fn debug_activitywatch() -> Result<String, String> {
    let aw_client = get_configured_aw_client().await;
    
    
    // Test bucket access
    let buckets = aw_client.get_buckets().await?;
    let mut debug_info = format!("Found {} buckets:\n", buckets.len());
    
    for (bucket_id, bucket_info) in &buckets {
        let bucket_type = bucket_info.get("type").and_then(|t| t.as_str()).unwrap_or("unknown");
        debug_info.push_str(&format!("- {}: {}\n", bucket_id, bucket_type));
    }
    
    // Test direct event fetching from each bucket
    for (bucket_id, _) in buckets.iter().take(3) {
        let now = Utc::now();
        let start = now - chrono::Duration::hours(1);
        
        match aw_client.get_events(bucket_id, start, now).await {
            Ok(events) => {
                debug_info.push_str(&format!("\nBucket {}: {} events in last hour\n", bucket_id, events.len()));
                if !events.is_empty() {
                    debug_info.push_str(&format!("Latest event: {:?}\n", events.last().unwrap()));
                }
            }
            Err(e) => {
                debug_info.push_str(&format!("\nBucket {}: Error - {}\n", bucket_id, e));
            }
        }
    }
    
    Ok(debug_info)
}

#[tauri::command]
async fn generate_daily_summary_command() -> Result<DailySummary, String> {
    let aw_client = get_configured_aw_client().await;
    generate_daily_summary(&aw_client).await
}

#[tauri::command]
async fn generate_hourly_summary(app: AppHandle) -> Result<HourlySummary, String> {
    send_log(&app, "info", "Starting hourly summary generation...");
    
    let now = chrono::Local::now();
    let data_dir = std::path::PathBuf::from("data");
    std::fs::create_dir_all(&data_dir).map_err(|e| e.to_string())?;
    let summary_file = data_dir.join("hourly_summary.txt");
    
    send_log(&app, "debug", "Generating new hourly summary...");
    
    // Force generate a new summary
    let (summary_text, focus_score, current_state) = generate_new_hourly_summary(now, &summary_file).await?;
    
    send_log(&app, "info", "Hourly summary generated successfully!");
    
    let hourly_summary = HourlySummary {
        summary: summary_text,
        focus_score,
        last_updated: now.format("%H:%M").to_string(),
        period: format!("{}-{}", 
                       (now - chrono::Duration::minutes(30)).format("%H:%M"),
                       now.format("%H:%M")),
        current_state,
    };
    
    // Store in app state and emit event
    if let Some(state) = app.try_state::<AppState>() {
        if let Ok(mut latest) = state.latest_hourly_summary.lock() {
            *latest = Some(hourly_summary.clone());
        }
    }
    
    // Emit event to update frontend
    app.emit("hourly_summary_updated", &hourly_summary)
        .map_err(|e| format!("Failed to emit summary update: {}", e))?;
    
    // Return the HourlySummary object directly
    Ok(hourly_summary)
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct UserConfig {
    user_context: String,
    activitywatch_port: u16,
    ollama_port: u16,
    // Mode-specific contexts
    study_focus: String,
    coach_task: String,
    // Notification prompts
    chill_notification_prompt: String,
    study_notification_prompt: String,
    coach_notification_prompt: String,
    // Notification settings
    notifications_enabled: bool,
    notification_webhook: Option<String>,
}

impl Default for UserConfig {
    fn default() -> Self {
        Self {
            user_context: "".to_string(),
            activitywatch_port: 5600,
            ollama_port: 11434,
            study_focus: "".to_string(),
            coach_task: "".to_string(),
            chill_notification_prompt: "Hey! You've been having fun for a while now. Maybe it's time to take a break or switch to something productive? 🌟".to_string(),
            study_notification_prompt: "Looks like you got distracted from studying. Let's get back on track! 📚".to_string(),
            coach_notification_prompt: "Time to check your progress! Please review and update your todo list. ✓".to_string(),
            notifications_enabled: true,
            notification_webhook: None,
        }
    }
}

async fn load_user_config_internal() -> Result<UserConfig, String> {
    let data_dir = std::path::PathBuf::from("data");
    let config_file = data_dir.join("config.json");
    
    if config_file.exists() {
        match std::fs::read_to_string(&config_file) {
            Ok(content) => {
                match serde_json::from_str::<UserConfig>(&content) {
                    Ok(config) => Ok(config),
                    Err(_) => Ok(UserConfig::default())
                }
            }
            Err(_) => Ok(UserConfig::default())
        }
    } else {
        Ok(UserConfig::default())
    }
}

#[tauri::command]
async fn load_user_config() -> Result<UserConfig, String> {
    let current_dir = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    eprintln!("Current working directory: {:?}", current_dir);
    
    let data_dir = std::path::PathBuf::from("data");
    let config_file = data_dir.join("config.json");
    
    eprintln!("Loading config from: {:?}", config_file.canonicalize().unwrap_or(config_file.clone()));
    eprintln!("Config file exists: {}", config_file.exists());
    
    if config_file.exists() {
        match std::fs::read_to_string(&config_file) {
            Ok(content) => {
                eprintln!("Config file content: {}", content);
                match serde_json::from_str::<UserConfig>(&content) {
                    Ok(config) => {
                        eprintln!("Successfully parsed config: {:?}", config);
                        Ok(config)
                    },
                    Err(e) => {
                        eprintln!("Failed to parse config JSON: {}", e);
                        // If parsing fails, return default config
                        Ok(UserConfig::default())
                    }
                }
            }
            Err(e) => {
                eprintln!("Failed to read config file: {}", e);
                // If file read fails, return default config
                Ok(UserConfig::default())
            }
        }
    } else {
        eprintln!("Config file doesn't exist, returning default config");
        // If file doesn't exist, return default config
        Ok(UserConfig::default())
    }
}

#[tauri::command]
async fn save_user_config(config: UserConfig) -> Result<(), String> {
    let data_dir = std::path::PathBuf::from("data");
    eprintln!("Saving config to data directory: {:?}", data_dir.canonicalize().unwrap_or(data_dir.clone()));
    
    std::fs::create_dir_all(&data_dir).map_err(|e| {
        eprintln!("Failed to create data directory: {}", e);
        e.to_string()
    })?;
    
    let config_file = data_dir.join("config.json");
    eprintln!("Config file path: {:?}", config_file);
    
    let json_content = serde_json::to_string_pretty(&config).map_err(|e| {
        eprintln!("Failed to serialize config: {}", e);
        e.to_string()
    })?;
    
    eprintln!("Config content to write: {}", json_content);
    
    std::fs::write(&config_file, &json_content).map_err(|e| {
        eprintln!("Failed to write config file: {}", e);
        e.to_string()
    })?;
    
    eprintln!("Config saved successfully to: {:?}", config_file);
    
    Ok(())
}

#[tauri::command]
async fn show_connection_help() -> Result<String, String> {
    let help_text = r#"
Connection Help:

ActivityWatch:
1. Download from https://activitywatch.net/
2. Install and run ActivityWatch
3. Ensure it's running on localhost:5600
4. Check the ActivityWatch icon in your system tray

Ollama:
1. Download from https://ollama.ai
2. Install Ollama
3. Run 'ollama serve' in terminal
4. Pull a model: 'ollama pull mistral'
5. Ensure it's running on localhost:11434

If issues persist, check firewall settings and ensure no other applications are using these ports.
"#;
    Ok(help_text.to_string())
}

async fn test_ollama_connection() -> bool {
    let client = get_ollama_client();
    let config = load_user_config_internal().await.unwrap_or_default();
    match client
        .get(format!("http://localhost:{}/api/tags", config.ollama_port))
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
    {
        Ok(response) => response.status().is_success(),
        Err(_) => false
    }
}

fn create_tray_menu(app: &AppHandle, current_mode: &str) -> Result<Menu<Wry>, tauri::Error> {
    
    let dashboard = MenuItem::with_id(app, "dashboard", "Dashboard", true, None::<&str>)?;
    
    // Create CheckMenuItem for each mode with appropriate checked state
    let ghost = CheckMenuItem::with_id(app, "ghost", "Ghost Mode", true, current_mode == "ghost", None::<&str>)?;
    let chill = CheckMenuItem::with_id(app, "chill", "Chill Mode", true, current_mode == "chill", None::<&str>)?;
    let buddy = CheckMenuItem::with_id(app, "study_buddy", "Study Buddy Mode", true, current_mode == "study_buddy", None::<&str>)?;
    let coach = CheckMenuItem::with_id(app, "coach", "Coach Mode", true, current_mode == "coach", None::<&str>)?;
    
    let check = MenuItem::with_id(app, "check", "Check Ollama and AW", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    
    let menu = Menu::with_items(app, &[
        &dashboard,
        &PredefinedMenuItem::separator(app)?,
        &ghost,
        &chill,
        &buddy,
        &coach,
        &PredefinedMenuItem::separator(app)?,
        &check,
        &PredefinedMenuItem::separator(app)?,
        &quit,
    ])?;
    
    Ok(menu)
}

fn update_tray_menu(app: &AppHandle, current_mode: &str) -> Result<(), tauri::Error> {
    let menu = create_tray_menu(app, current_mode)?;
    if let Some(tray) = app.tray_by_id("main") {
        tray.set_menu(Some(menu))?;
    }
    Ok(())
}

fn handle_tray_event(app: &AppHandle, event: TrayIconEvent) {
    match event {
        TrayIconEvent::Click { 
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } => {
            // Single left click - could be used for quick actions
        }
        TrayIconEvent::DoubleClick { 
            button: MouseButton::Left,
            ..
        } => {
            show_main_window(app);
        }
        _ => {
            // Handle menu events through the menu setup callback
        }
    }
}

#[derive(Serialize, Deserialize)]
struct ActivityClassification {
    work: u32,
    communication: u32,
    distraction: u32,
}

#[tauri::command]
async fn classify_activities(app: AppHandle, _state: State<'_, AppState>) -> Result<ActivityClassification, String> {
    send_log(&app, "debug", "Starting activity classification...");
    
    // Get activity data from ActivityWatch
    let aw_client = get_configured_aw_client().await;
    let aw_test = aw_client.test_connection().await;
    
    if !aw_test.connected {
        send_log(&app, "warn", "ActivityWatch not connected, using fallback data");
        return Ok(ActivityClassification {
            work: 70,
            communication: 20,
            distraction: 10,
        });
    }
    
    // Fetch recent activity data (last 4 hours)
    let now = chrono::Utc::now();
    let start_time = now - chrono::Duration::hours(4);
    
    // Get window events filtered by non-AFK periods
    let window_events = match aw_client.get_active_window_events(start_time, now).await {
        Ok(events) => events,
        Err(e) => {
            send_log(&app, "error", &format!("Failed to get window events: {}", e));
            return Ok(ActivityClassification {
                work: 60,
                communication: 25,
                distraction: 15,
            });
        }
    };
    
    // Sum up time spent per application and collect titles
    let mut app_times: HashMap<String, u64> = HashMap::new();
    let mut app_titles: HashMap<String, Vec<String>> = HashMap::new();
    
    for event in &window_events {
        if let (Some(data), Some(duration)) = (event.get("data").and_then(|v| v.as_object()), event.get("duration").and_then(|v| v.as_u64())) {
            if let Some(app_name) = data.get("app").and_then(|v| v.as_str()) {
                *app_times.entry(app_name.to_string()).or_insert(0) += duration;
                
                // Collect sample titles for context
                if let Some(title) = data.get("title").and_then(|v| v.as_str()) {
                    let titles = app_titles.entry(app_name.to_string()).or_insert_with(Vec::new);
                    if !titles.contains(&title.to_string()) && titles.len() < 3 {
                        titles.push(title.to_string());
                    }
                }
            }
        }
    }
    
    // Test Ollama connection for AI classification
    let ollama_available = test_ollama_connection().await;
    
    if !ollama_available {
        send_log(&app, "warn", "Ollama not available, using heuristic classification");
        return Ok(classify_activities_heuristic(&app_times));
    }
    
    // Create prompt for AI classification with titles
    let app_list: Vec<String> = app_times.iter()
        .map(|(app, duration)| {
            let titles = app_titles.get(app).cloned().unwrap_or_default();
            if titles.is_empty() {
                format!("{}: {}s", app, duration)
            } else {
                let title_sample = titles.iter()
                    .take(2)
                    .map(|t| if t.chars().count() > 30 { format!("{}...", t.chars().take(27).collect::<String>()) } else { t.clone() })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{}: {}s ({})", app, duration, title_sample)
            }
        })
        .collect();
    
    let prompt = format!(
        "Classify app usage into work/communication/distraction percentages. Return JSON only.

Application usage:
{}

CATEGORIES:
Work: IDEs, editors, terminals, design tools, research browsers, productivity apps
Communication: Slack, Teams, Discord (work), email, video calls, collaboration tools
Distraction: Games, entertainment, social media, non-educational YouTube, shopping

ANALYSIS RULES:
1. Consider window titles for context (work Discord vs gaming Discord)
2. Weight by time spent - longer sessions influence percentages more
3. Educational content = work, entertainment content = distraction
4. Productive browser usage = work, casual browsing = distraction

Return JSON with percentages that sum to 100: {{\"work\":60,\"communication\":25,\"distraction\":15}}",
        app_list.join("\n")
    );
    
    // Make request to Ollama
    let config = load_user_config_internal().await.unwrap_or_default();
    match get_ollama_client()
        .post(format!("http://localhost:{}/api/generate", config.ollama_port))
        .json(&serde_json::json!({
            "model": "mistral",
            "prompt": prompt,
            "stream": false,
            "options": {
                "temperature": 0.1,
                "top_p": 0.9,
                "num_predict": 100
            }
        }))
        .send()
        .await
    {
        Ok(response) => {
            if let Ok(json) = response.json::<serde_json::Value>().await {
                if let Some(ai_response) = json.get("response").and_then(|v| v.as_str()) {
                    // Try to parse JSON from AI response
                    if let Ok(classification) = serde_json::from_str::<ActivityClassification>(ai_response) {
                        send_log(&app, "info", "AI activity classification successful");
                        return Ok(classification);
                    }
                }
            }
        }
        Err(e) => {
            send_log(&app, "warn", &format!("Ollama request failed: {}", e));
        }
    }
    
    // Fallback to heuristic classification
    send_log(&app, "info", "Using heuristic activity classification");
    Ok(classify_activities_heuristic(&app_times))
}

fn classify_activities_heuristic(app_times: &HashMap<String, u64>) -> ActivityClassification {
    let total_time: u64 = app_times.values().sum();
    if total_time == 0 {
        return ActivityClassification {
            work: 50,
            communication: 30,
            distraction: 20,
        };
    }
    
    let mut work_time = 0u64;
    let mut comm_time = 0u64;
    let mut distraction_time = 0u64;
    
    for (app, time) in app_times {
        let app_lower = app.to_lowercase();
        if app_lower.contains("code") || app_lower.contains("terminal") || app_lower.contains("vim") || 
           app_lower.contains("ide") || app_lower.contains("editor") || app_lower.contains("git") ||
           app_lower.contains("develop") || app_lower.contains("studio") {
            work_time += time;
        } else if app_lower.contains("slack") || app_lower.contains("discord") || app_lower.contains("teams") ||
                  app_lower.contains("zoom") || app_lower.contains("mail") || app_lower.contains("chat") {
            comm_time += time;
        } else {
            distraction_time += time;
        }
    }
    
    let work_pct = ((work_time * 100) / total_time) as u32;
    let comm_pct = ((comm_time * 100) / total_time) as u32;
    let distraction_pct = 100 - work_pct - comm_pct;
    
    ActivityClassification {
        work: work_pct,
        communication: comm_pct,
        distraction: distraction_pct,
    }
}

fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        if let Err(e) = window.show() {
            eprintln!("Failed to show window: {}", e);
        }
        if let Err(e) = window.set_focus() {
            eprintln!("Failed to focus window: {}", e);
        }
    }
}

fn handle_menu_event(app: &AppHandle, event: tauri::menu::MenuEvent) {
    match event.id.as_ref() {
        "dashboard" => {
            send_log(app, "info", "Opening dashboard from tray menu");
            show_main_window(app);
        }
        "ghost" | "chill" | "study_buddy" | "coach" => {
            let mode = event.id.0.clone();
            send_log(app, "info", &format!("Mode selected from tray menu: {}", mode));
            
            if let Err(e) = app.emit("set_mode", &mode) {
                send_log(app, "error", &format!("Failed to emit mode change: {}", e));
            }
            
            // Update app state and trigger mode logic
            if let Some(state) = app.try_state::<AppState>() {
                // Update mode
                if let Ok(mut current_mode) = state.current_mode.lock() {
                    *current_mode = mode.clone();
                }
                
                // Save mode to persistent storage
                if let Err(e) = AppState::save_mode(&mode) {
                    send_log(app, "error", &format!("Failed to save mode: {}", e));
                }
                
                // Clear last run time to ensure immediate execution
                if let Ok(mut times) = state.last_summary_time.lock() {
                    times.remove(&mode);
                }
                
                // Update tray menu
                if let Err(e) = update_tray_menu(app, &mode) {
                    send_log(app, "error", &format!("Failed to update tray menu: {}", e));
                } else {
                    send_log(app, "debug", "Tray menu updated successfully");
                }
                
                // Note: Initial summary will be generated by the background timer on the next minute tick
                send_log(app, "info", "Mode switched, initial summary will be generated by background timer");
            }
        }
        "check" => {
            send_log(app, "info", "Connection check requested from tray menu");
            if let Err(e) = app.emit("check_connections", ()) {
                send_log(app, "error", &format!("Failed to emit check connections: {}", e));
            }
            
            // Also show help dialog
            let app_clone = app.clone();
            tauri::async_runtime::spawn(async move {
                if let Ok(help) = show_connection_help().await {
                    send_log(&app_clone, "info", "Connection help displayed");
                    println!("Connection Help:\n{}", help);
                }
            });
        }
        "quit" => {
            send_log(app, "info", "Application quit requested from tray menu");
            std::process::exit(0);
        }
        _ => {
            send_log(app, "debug", &format!("Unknown menu item clicked: {}", event.id.0));
        }
    }
}

#[tauri::command]
async fn get_coach_todos() -> Result<CoachTodoList, String> {
    let data_dir = std::path::PathBuf::from("data");
    let todo_file = data_dir.join("coach_todos.json");
    
    if todo_file.exists() {
        match std::fs::read_to_string(&todo_file) {
            Ok(content) => {
                match serde_json::from_str::<CoachTodoList>(&content) {
                    Ok(todos) => Ok(todos),
                    Err(_) => {
                        // Create fallback todos if parsing fails
                        let now = chrono::Local::now();
                        create_fallback_todo_list("complete daily tasks", now)
                    }
                }
            }
            Err(_) => {
                let now = chrono::Local::now();
                create_fallback_todo_list("complete daily tasks", now)
            }
        }
    } else {
        let now = chrono::Local::now();
        create_fallback_todo_list("complete daily tasks", now)
    }
}

#[tauri::command]
async fn update_coach_todo(todo_id: String, completed: bool) -> Result<(), String> {
    let data_dir = std::path::PathBuf::from("data");
    let todo_file = data_dir.join("coach_todos.json");
    
    if todo_file.exists() {
        match std::fs::read_to_string(&todo_file) {
            Ok(content) => {
                match serde_json::from_str::<CoachTodoList>(&content) {
                    Ok(mut todos) => {
                        // Update the specific todo
                        for todo in &mut todos.todos {
                            if todo.id == todo_id {
                                todo.completed = completed;
                                break;
                            }
                        }
                        
                        // Save back to file
                        let json_content = serde_json::to_string_pretty(&todos)
                            .map_err(|e| format!("Failed to serialize todos: {}", e))?;
                        std::fs::write(&todo_file, json_content)
                            .map_err(|e| format!("Failed to write todos: {}", e))?;
                        
                        Ok(())
                    }
                    Err(e) => Err(format!("Failed to parse todos: {}", e))
                }
            }
            Err(e) => Err(format!("Failed to read todos: {}", e))
        }
    } else {
        Err("No todo file found".to_string())
    }
}


#[tauri::command]
async fn install_ollama(app: AppHandle) -> Result<String, String> {
    send_log(&app, "info", "Starting Ollama installation...");
    
    #[cfg(target_os = "windows")]
    {
        let script = r#"
# Download Ollama installer
$url = "https://ollama.com/download/OllamaSetup.exe"
$output = "$env:TEMP\OllamaSetup.exe"
Write-Host "Downloading Ollama..."
Invoke-WebRequest -Uri $url -OutFile $output
Write-Host "Installing Ollama..."
Start-Process -FilePath $output -Wait
Write-Host "Starting Ollama service..."
Start-Process "ollama" -ArgumentList "serve"
Start-Sleep -Seconds 5
Write-Host "Pulling mistral model..."
Start-Process "ollama" -ArgumentList "pull mistral" -Wait
Write-Host "Installation complete!"
"#;
        
        match std::process::Command::new("powershell")
            .args(&["-ExecutionPolicy", "Bypass", "-Command", script])
            .output()
        {
            Ok(output) => {
                let result = String::from_utf8_lossy(&output.stdout);
                send_log(&app, "info", &result);
                if output.status.success() {
                    Ok("Ollama installed successfully".to_string())
                } else {
                    Err(format!("Installation failed: {}", String::from_utf8_lossy(&output.stderr)))
                }
            }
            Err(e) => Err(format!("Failed to run installation script: {}", e))
        }
    }
    
    #[cfg(target_os = "macos")]
    {
        let script = r#"
#!/bin/bash
echo "Downloading Ollama..."
curl -fsSL https://ollama.com/install.sh | sh
echo "Starting Ollama service..."
ollama serve &
sleep 5
echo "Pulling mistral model..."
ollama pull mistral
echo "Installation complete!"
"#;
        
        match std::process::Command::new("sh")
            .arg("-c")
            .arg(script)
            .output()
        {
            Ok(output) => {
                let result = String::from_utf8_lossy(&output.stdout);
                send_log(&app, "info", &result);
                if output.status.success() {
                    Ok("Ollama installed successfully".to_string())
                } else {
                    Err(format!("Installation failed: {}", String::from_utf8_lossy(&output.stderr)))
                }
            }
            Err(e) => Err(format!("Failed to run installation script: {}", e))
        }
    }
    
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        Err("Installation script not available for this platform".to_string())
    }
}

#[tauri::command]
async fn install_activitywatch(app: AppHandle) -> Result<String, String> {
    send_log(&app, "info", "Starting ActivityWatch installation...");
    
    #[cfg(target_os = "windows")]
    {
        let script = r#"
# Download ActivityWatch
$url = "https://github.com/ActivityWatch/activitywatch/releases/latest/download/activitywatch-v0.13.2-windows-x86_64.zip"
$output = "$env:TEMP\activitywatch.zip"
$extractPath = "$env:LOCALAPPDATA\ActivityWatch"

Write-Host "Downloading ActivityWatch..."
Invoke-WebRequest -Uri $url -OutFile $output

Write-Host "Extracting ActivityWatch..."
Expand-Archive -Path $output -DestinationPath $extractPath -Force

Write-Host "Starting ActivityWatch..."
$awPath = Join-Path $extractPath "activitywatch\aw-qt.exe"
Start-Process $awPath

Write-Host "Installation complete!"
"#;
        
        match std::process::Command::new("powershell")
            .args(&["-ExecutionPolicy", "Bypass", "-Command", script])
            .output()
        {
            Ok(output) => {
                let result = String::from_utf8_lossy(&output.stdout);
                send_log(&app, "info", &result);
                if output.status.success() {
                    Ok("ActivityWatch installed successfully".to_string())
                } else {
                    Err(format!("Installation failed: {}", String::from_utf8_lossy(&output.stderr)))
                }
            }
            Err(e) => Err(format!("Failed to run installation script: {}", e))
        }
    }
    
    #[cfg(target_os = "macos")]
    {
        let script = r#"
#!/bin/bash
# Download ActivityWatch
url="https://github.com/ActivityWatch/activitywatch/releases/latest/download/activitywatch-v0.13.2-macos-arm64.dmg"
output="/tmp/activitywatch.dmg"

echo "Downloading ActivityWatch..."
curl -L "$url" -o "$output"

echo "Mounting DMG..."
hdiutil attach "$output"

echo "Copying to Applications..."
cp -R "/Volumes/ActivityWatch/ActivityWatch.app" "/Applications/"

echo "Unmounting DMG..."
hdiutil detach "/Volumes/ActivityWatch"

echo "Starting ActivityWatch..."
open "/Applications/ActivityWatch.app"

echo "Installation complete!"
"#;
        
        match std::process::Command::new("sh")
            .arg("-c")
            .arg(script)
            .output()
        {
            Ok(output) => {
                let result = String::from_utf8_lossy(&output.stdout);
                send_log(&app, "info", &result);
                if output.status.success() {
                    Ok("ActivityWatch installed successfully".to_string())
                } else {
                    Err(format!("Installation failed: {}", String::from_utf8_lossy(&output.stderr)))
                }
            }
            Err(e) => Err(format!("Failed to run installation script: {}", e))
        }
    }
    
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        Err("Installation script not available for this platform".to_string())
    }
}

#[tauri::command]
async fn test_notification(app: AppHandle) -> Result<(), String> {
    let test_message = "🔔 Test notification from Companion Cube! Your notification system is working properly.";
    
    send_notification(&app, "Notification Test", test_message).await;
    send_log(&app, "info", "Test notification sent successfully");
    
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app_state = AppState::new();

    tauri::Builder::default()
        .manage(app_state)
        .plugin(tauri_plugin_log::Builder::default().build())
        .invoke_handler(tauri::generate_handler![
            check_connections,
            get_current_mode,
            set_mode,
            get_hourly_summary,
            generate_hourly_summary,
            show_connection_help,
            debug_activitywatch,
            generate_daily_summary_command,
            load_user_config,
            save_user_config,
            classify_activities,
            get_coach_todos,
            update_coach_todo,
            install_ollama,
            install_activitywatch,
            test_notification
        ])
        .on_window_event(|window, event| {
            match event {
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    // Hide window instead of closing
                    if let Err(e) = window.hide() {
                        eprintln!("Failed to hide window: {}", e);
                    }
                    api.prevent_close();
                }
                _ => {}
            }
        })
        .setup(|app| {
            let app_handle = app.handle();
            
            send_log(&app_handle, "info", "Companion Cube starting up...");
            
            // Set dark title bar for Windows
            if let Some(_window) = app.get_webview_window("main") {
                #[cfg(target_os = "windows")]
                {
                    let _ = _window.set_theme(Some(tauri::Theme::Dark));
                }
            }
            send_log(&app_handle, "debug", "Creating system tray icon");
            
            // Create tray icon with menu using current mode
            let current_mode = {
                let app_state = app.state::<AppState>();
                let mode = app_state.current_mode.lock().unwrap().clone();
                mode
            };
            let menu = create_tray_menu(&app_handle, &current_mode)?;
            
            let _tray = TrayIconBuilder::with_id("main")
                .tooltip("Companion Cube - ADHD Productivity Assistant")
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_tray_icon_event(|tray, event| {
                    handle_tray_event(tray.app_handle(), event);
                })
                .build(app)?;
            
            send_log(&app_handle, "info", "System tray icon created successfully");
            
            // Set up menu event handler
            let app_handle_clone = app_handle.clone();
            app.on_menu_event(move |_app, event| {
                handle_menu_event(&app_handle_clone, event);
            });
            
            send_log(&app_handle, "info", "Companion Cube initialization complete");
            send_log(&app_handle, "info", &format!("Application running in {} mode", current_mode));
            
            // Start background timer for mode-specific logic
            let app_handle_timer = app_handle.clone();
            let current_mode_arc = app.state::<AppState>().current_mode.clone();
            let last_summary_arc = app.state::<AppState>().last_summary_time.clone();
            tauri::async_runtime::spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(60)); // Check every minute
                // Set to tick immediately at the start of each period
                interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                
                send_log(&app_handle_timer, "info", "Background timer initialized, checking every minute");
                
                loop {
                    interval.tick().await;
                    
                    let now = chrono::Utc::now();
                    
                    // Get current mode
                    let current_mode = {
                        if let Ok(mode) = current_mode_arc.lock() {
                            mode.clone()
                        } else {
                            continue;
                        }
                    };
                    
                    // Log the check (only in debug mode to avoid spam)
                    if now.second() < 5 {  // Only log near the start of the minute
                        send_log(&app_handle_timer, "debug", 
                            &format!("Timer check at {:02}:{:02} for {} mode", 
                                now.minute(), now.second(), current_mode));
                    }
                    
                    // Create a temporary AppState for the check
                    let temp_state = AppState {
                        current_mode: current_mode_arc.clone(),
                        last_summary_time: last_summary_arc.clone(),
                        latest_hourly_summary: Arc::new(Mutex::new(None)),
                    };
                    
                    // Check if we should run mode logic
                    if let Err(e) = handle_mode_specific_logic(&app_handle_timer, &current_mode, &temp_state).await {
                        send_log(&app_handle_timer, "error", &format!("Mode logic error: {}", e));
                    }
                }
            });
            
            send_log(&app_handle, "info", "Background timer started for mode-specific checks");
            
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}