use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc, Timelike, SubsecRound};
use std::collections::HashMap;
use anyhow::Result;
use reqwest::Client;
use std::sync::OnceLock;
use serde_json::json;

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
    /// Uses manual filtering approach for compatibility
    pub async fn get_active_window_events(&self, start: DateTime<Utc>, end: DateTime<Utc>) -> Result<Vec<serde_json::Value>, String> {
        // Get buckets to find the correct bucket names with hostname
        let buckets = self.get_buckets().await?;
        
        // Find window and AFK bucket names
        let window_bucket = buckets.keys()
            .find(|k| k.starts_with("aw-watcher-window_"))
            .cloned()
            .ok_or("No window watcher bucket found")?;
            
        let afk_bucket = buckets.keys()
            .find(|k| k.starts_with("aw-watcher-afk_"))
            .cloned();
        
        // Get window events
        let window_events = self.get_events(&window_bucket, start, end).await?;
        
        // If no AFK bucket, return all window events
        let afk_bucket = match afk_bucket {
            Some(bucket) => bucket,
            None => {
                return Ok(window_events.into_iter()
                    .map(|e| serde_json::to_value(e).unwrap_or(json!({})))
                    .collect());
            }
        };
        
        // Get AFK events
        let afk_events = self.get_events(&afk_bucket, start, end).await?;
        
        // Manual filtering: keep window events that overlap with non-AFK periods
        let mut active_events = Vec::new();
        
        for window_event in window_events {
            let window_start = window_event.timestamp;
            let window_end = window_event.timestamp + chrono::Duration::seconds(window_event.duration as i64);
            
            // Check if this window event overlaps with any non-AFK period
            let is_active = afk_events.iter().any(|afk_event| {
                if let Some(status) = afk_event.data.get("status").and_then(|v| v.as_str()) {
                    if status == "not-afk" {
                        let afk_start = afk_event.timestamp;
                        let afk_end = afk_event.timestamp + chrono::Duration::seconds(afk_event.duration as i64);
                        
                        // Check for overlap
                        return window_start < afk_end && window_end > afk_start;
                    }
                }
                false
            });
            
            if is_active || afk_events.is_empty() {
                active_events.push(serde_json::to_value(window_event).unwrap_or(json!({})));
            }
        }
        
        Ok(active_events)
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

    /// Execute a query using ActivityWatch's query API
    /// This is more efficient than fetching and filtering events manually
    async fn execute_query(&self, query: &str, timeperiods: Vec<(DateTime<Utc>, DateTime<Utc>)>) -> Result<Vec<serde_json::Value>, String> {
        let url = format!("http://{}:{}/api/0/query/", self.host, self.port);
        
        // Convert timeperiods to the format ActivityWatch expects
        let timeperiods_str: Vec<String> = timeperiods.iter()
            .map(|(start, end)| {
                format!("[{}, {}]", 
                    serde_json::to_string(&start.to_rfc3339()).unwrap(),
                    serde_json::to_string(&end.to_rfc3339()).unwrap()
                )
            })
            .collect();
        
        let query_body = json!({
            "query": [query],
            "timeperiods": timeperiods_str
        });

        let response = get_aw_client()
            .post(&url)
            .json(&query_body)
            .send()
            .await
            .map_err(|e| format!("Failed to execute query: {}", e))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_else(|_| "No error details".to_string());
            return Err(format!("Query API error {}: {}", status, error_text));
        }

        let result: Vec<Vec<serde_json::Value>> = response.json().await
            .map_err(|e| format!("Failed to parse query result: {}", e))?;
        
        // Return the first result set (we only send one query)
        Ok(result.into_iter().next().unwrap_or_default())
    }

    /// Get active window events using ActivityWatch's query API
    /// This is more efficient than the manual filtering approach
    pub async fn get_active_window_events_v2(&self, start: DateTime<Utc>, end: DateTime<Utc>) -> Result<Vec<serde_json::Value>, String> {
        // Build the query using ActivityWatch's transform functions
        let query = r#"
            afk_events = query_bucket(find_bucket("aw-watcher-afk_"));
            window_events = query_bucket(find_bucket("aw-watcher-window_"));
            
            # Filter window events to only include non-AFK periods
            window_events = filter_period_intersect(window_events, filter_keyvals(afk_events, "status", ["not-afk"]));
            
            # Merge consecutive events with same app and title
            window_events = merge_events_by_keys(window_events, ["app", "title"]);
            
            # Sort by timestamp
            RETURN = sort_by_timestamp(window_events);
        "#;

        self.execute_query(query, vec![(start, end)]).await
    }

    /// Get categorized activity data using the query API
    pub async fn get_categorized_events(&self, start: DateTime<Utc>, end: DateTime<Utc>) -> Result<serde_json::Value, String> {
        // Query that includes category information if available
        let query = r#"
            afk_events = query_bucket(find_bucket("aw-watcher-afk_"));
            window_events = query_bucket(find_bucket("aw-watcher-window_"));
            
            # Filter to only active periods
            window_events = filter_period_intersect(window_events, filter_keyvals(afk_events, "status", ["not-afk"]));
            
            # Categorize if categories are configured
            window_events = categorize(window_events, __CATEGORIES__);
            
            # Merge by app and category
            window_events = merge_events_by_keys(window_events, ["app", "$category"]);
            
            # Sort by duration descending
            window_events = sort_by_duration(window_events);
            
            # Create summary
            summary = {};
            summary["events"] = window_events;
            summary["total_duration"] = sum_durations(window_events);
            
            RETURN = summary;
        "#;

        let results = self.execute_query(query, vec![(start, end)]).await?;
        Ok(results.into_iter().next().unwrap_or(json!({})))
    }

    /// Get time-based activity statistics using the query API
    pub async fn get_activity_stats(&self, start: DateTime<Utc>, end: DateTime<Utc>) -> Result<serde_json::Value, String> {
        let query = r#"
            afk_events = query_bucket(find_bucket("aw-watcher-afk_"));
            window_events = query_bucket(find_bucket("aw-watcher-window_"));
            
            # Get active window events
            active_events = filter_period_intersect(window_events, filter_keyvals(afk_events, "status", ["not-afk"]));
            
            # Group by app
            by_app = merge_events_by_keys(active_events, ["app"]);
            by_app = sort_by_duration(by_app);
            
            # Calculate statistics
            stats = {};
            stats["total_active_time"] = sum_durations(active_events);
            stats["app_count"] = length(by_app);
            stats["top_apps"] = limit_events(by_app, 10);
            
            RETURN = stats;
        "#;

        let results = self.execute_query(query, vec![(start, end)]).await?;
        Ok(results.into_iter().next().unwrap_or(json!({})))
    }

    /// Get app usage grouped by custom categories
    /// This is useful for productivity analysis
    pub async fn get_app_usage_by_category(&self, start: DateTime<Utc>, end: DateTime<Utc>) -> Result<serde_json::Value, String> {
        let query = r#"
            afk_events = query_bucket(find_bucket("aw-watcher-afk_"));
            window_events = query_bucket(find_bucket("aw-watcher-window_"));
            
            # Get active window events
            active_events = filter_period_intersect(window_events, filter_keyvals(afk_events, "status", ["not-afk"]));
            
            # Group by app name for detailed statistics
            by_app = merge_events_by_keys(active_events, ["app"]);
            by_app = sort_by_duration(by_app);
            
            # Also get title-based grouping for more detail
            by_app_title = merge_events_by_keys(active_events, ["app", "title"]);
            
            result = {};
            result["by_app"] = by_app;
            result["by_app_title"] = limit_events(by_app_title, 50);
            result["total_active_time"] = sum_durations(active_events);
            result["event_count"] = length(active_events);
            
            RETURN = result;
        "#;

        let results = self.execute_query(query, vec![(start, end)]).await?;
        Ok(results.into_iter().next().unwrap_or(json!({})))
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

    /// Get multi-timeframe data using the efficient query API
    pub async fn get_multi_timeframe_data_v2(&self) -> Result<HashMap<String, TimeframeData>, String> {
        let now = Utc::now();
        let timeframes = vec![
            ("5_minutes", chrono::Duration::minutes(5)),
            ("10_minutes", chrono::Duration::minutes(10)),
            ("30_minutes", chrono::Duration::minutes(30)),
            ("1_hour", chrono::Duration::hours(1)),
            ("today", chrono::Duration::hours(if now.hour() == 0 { 1 } else { now.hour() as i64 })),
        ];

        let mut timeframe_data = HashMap::new();

        // Process each timeframe using the query API
        for (name, duration) in timeframes {
            let start = now - duration;
            
            // Query to get active events and statistics for this timeframe
            let query = r#"
                afk_events = query_bucket(find_bucket("aw-watcher-afk_"));
                window_events = query_bucket(find_bucket("aw-watcher-window_"));
                
                # Get active window events
                active_events = filter_period_intersect(window_events, filter_keyvals(afk_events, "status", ["not-afk"]));
                
                # Calculate statistics
                by_app = merge_events_by_keys(active_events, ["app"]);
                
                result = {};
                result["window_events"] = active_events;
                result["afk_events"] = afk_events;
                result["total_active_time"] = sum_durations(active_events);
                result["unique_apps"] = by_app;
                
                RETURN = result;
            "#;

            match self.execute_query(query, vec![(start, now)]).await {
                Ok(results) => {
                    if let Some(result) = results.into_iter().next() {
                        // Parse the result to extract events and statistics
                        let window_events_json = result.get("window_events")
                            .and_then(|v| v.as_array())
                            .cloned()
                            .unwrap_or_default();
                        
                        let afk_events_json = result.get("afk_events")
                            .and_then(|v| v.as_array())
                            .cloned()
                            .unwrap_or_default();

                        // Convert JSON events to Event structs
                        let window_events = self.json_to_events(&window_events_json);
                        let afk_events = self.json_to_events(&afk_events_json);

                        // Calculate context switches
                        let mut context_switches = 0;
                        let mut last_app = String::new();
                        let mut unique_apps = std::collections::HashSet::new();
                        
                        for event in &window_events {
                            if let Some(app) = event.data.get("app").and_then(|v| v.as_str()) {
                                unique_apps.insert(app.to_string());
                                if !last_app.is_empty() && last_app != app {
                                    context_switches += 1;
                                }
                                last_app = app.to_string();
                            }
                        }

                        let total_active_minutes = result.get("total_active_time")
                            .and_then(|v| v.as_f64())
                            .unwrap_or(0.0) / 60.0;

                        let stats = TimeframeStatistics {
                            total_events: window_events.len() as u32,
                            unique_apps,
                            total_active_minutes,
                            context_switches,
                        };

                        timeframe_data.insert(name.to_string(), TimeframeData {
                            start,
                            end: now,
                            window_events,
                            afk_events,
                            statistics: stats,
                        });
                    }
                }
                Err(e) => {
                    eprintln!("Failed to query timeframe {}: {}", name, e);
                    // Continue with other timeframes
                }
            }
        }

        if timeframe_data.is_empty() {
            Err("Failed to retrieve data for any timeframe".to_string())
        } else {
            Ok(timeframe_data)
        }
    }

    /// Helper method to convert JSON events to Event structs
    fn json_to_events(&self, events_json: &[serde_json::Value]) -> Vec<Event> {
        events_json.iter()
            .filter_map(|event_json| {
                let timestamp = event_json.get("timestamp")
                    .and_then(|v| v.as_str())
                    .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.with_timezone(&Utc))?;
                
                let duration = event_json.get("duration")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);
                
                let data = event_json.get("data")
                    .and_then(|v| v.as_object())
                    .map(|obj| obj.clone())
                    .unwrap_or_default();
                
                Some(Event {
                    timestamp,
                    duration,
                    data: data.into_iter().map(|(k, v)| (k, v)).collect(),
                })
            })
            .collect()
    }

    /// Get multi-timeframe data with AFK filtering
    /// Uses manual filtering for compatibility
    pub async fn get_multi_timeframe_data_active(&self) -> Result<HashMap<String, TimeframeData>, String> {
        let now = Utc::now();
        let timeframes = vec![
            ("5_minutes", chrono::Duration::minutes(5)),
            ("10_minutes", chrono::Duration::minutes(10)),
            ("30_minutes", chrono::Duration::minutes(30)),
            ("1_hour", chrono::Duration::hours(1)),
            ("today", chrono::Duration::hours(if now.hour() == 0 { 1 } else { now.hour() as i64 })),
        ];

        let mut timeframe_data = HashMap::new();
        
        // Get buckets once to find correct bucket names
        let buckets = self.get_buckets().await?;
        
        let window_bucket = buckets.keys()
            .find(|k| k.starts_with("aw-watcher-window_"))
            .cloned()
            .ok_or("No window watcher bucket found")?;
            
        let afk_bucket = buckets.keys()
            .find(|k| k.starts_with("aw-watcher-afk_"))
            .cloned();

        // Process each timeframe
        for (name, duration) in timeframes {
            let start = now - duration;
            
            // Get window events
            let window_events = match self.get_events(&window_bucket, start, now).await {
                Ok(events) => events,
                Err(e) => {
                    eprintln!("Failed to get window events for {}: {}", name, e);
                    continue;
                }
            };
            
            // Get AFK events if bucket exists
            let afk_events = if let Some(ref afk_bucket_name) = afk_bucket {
                self.get_events(afk_bucket_name, start, now).await.unwrap_or_default()
            } else {
                Vec::new()
            };
            
            // Filter window events by non-AFK periods
            let mut active_window_events = Vec::new();
            
            for window_event in &window_events {
                let window_start = window_event.timestamp;
                let window_end = window_event.timestamp + chrono::Duration::seconds(window_event.duration as i64);
                
                // Check if this window event overlaps with any non-AFK period
                let is_active = afk_events.is_empty() || afk_events.iter().any(|afk_event| {
                    if let Some(status) = afk_event.data.get("status").and_then(|v| v.as_str()) {
                        if status == "not-afk" {
                            let afk_start = afk_event.timestamp;
                            let afk_end = afk_event.timestamp + chrono::Duration::seconds(afk_event.duration as i64);
                            
                            // Check for overlap
                            return window_start < afk_end && window_end > afk_start;
                        }
                    }
                    false
                });
                
                if is_active {
                    active_window_events.push(window_event.clone());
                }
            }
            
            // Calculate statistics
            let mut context_switches = 0;
            let mut last_app = String::new();
            let mut unique_apps = std::collections::HashSet::new();
            let mut total_active_minutes = 0.0;
            
            for event in &active_window_events {
                if let Some(app) = event.data.get("app").and_then(|v| v.as_str()) {
                    unique_apps.insert(app.to_string());
                    if !last_app.is_empty() && last_app != app {
                        context_switches += 1;
                    }
                    last_app = app.to_string();
                    total_active_minutes += event.duration / 60.0;
                }
            }

            let stats = TimeframeStatistics {
                total_events: active_window_events.len() as u32,
                unique_apps,
                total_active_minutes,
                context_switches,
            };

            timeframe_data.insert(name.to_string(), TimeframeData {
                start,
                end: now,
                window_events: active_window_events,
                afk_events: afk_events.clone(),
                statistics: stats,
            });
        }

        if timeframe_data.is_empty() {
            Err("Failed to retrieve data for any timeframe".to_string())
        } else {
            Ok(timeframe_data)
        }
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