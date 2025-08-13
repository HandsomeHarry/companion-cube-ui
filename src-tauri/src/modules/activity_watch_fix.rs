// Temporary fix to test raw JSON approach
use serde_json::Value;
use std::collections::HashMap;

pub async fn get_buckets_raw(host: &str, port: u16) -> Result<HashMap<String, Value>, String> {
    let url = format!("http://{}:{}/api/0/buckets/", host, port);
    
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .no_proxy()
        .build()
        .map_err(|e| e.to_string())?;
    
    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Failed to fetch buckets: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("ActivityWatch API error: {}", response.status()));
    }

    let buckets: HashMap<String, Value> = response.json().await
        .map_err(|e| format!("Failed to parse buckets: {}", e))?;
    
    // Print bucket structure for debugging
    for (id, bucket) in buckets.iter().take(1) {
        eprintln!("Sample bucket {}: {}", id, serde_json::to_string_pretty(bucket).unwrap_or_default());
    }
    
    Ok(buckets)
}

// Helper to find bucket by prefix
pub fn find_bucket_by_prefix(buckets: &HashMap<String, Value>, prefix: &str) -> Option<String> {
    buckets.keys()
        .find(|k| k.starts_with(prefix))
        .cloned()
}