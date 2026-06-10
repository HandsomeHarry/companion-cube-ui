use anyhow::Result;
use serde::de::DeserializeOwned;
use serde::Serialize;

// All daemon API routes live under /api since the browser pivot (the bare
// root serves the embedded SvelteKit UI, which answers any path with HTML 200).
const DAEMON_URL: &str = "http://127.0.0.1:7431/api";

/// Check if the daemon is running by hitting /health with a short timeout.
pub async fn is_daemon_running() -> bool {
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(500))
        .build()
    {
        Ok(c) => c,
        Err(_) => return false,
    };
    client
        .get(format!("{DAEMON_URL}/health"))
        .send()
        .await
        .ok()
        .is_some_and(|r| r.status().is_success())
}

/// Extract a meaningful error message from a non-success daemon response.
/// Attempts to parse the daemon's `{"error": {"message": "..."}}` envelope,
/// falling back to the raw HTTP status text.
async fn extract_error(resp: reqwest::Response) -> anyhow::Error {
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();

    // Try to parse the daemon's structured error envelope
    if let Ok(envelope) = serde_json::from_str::<serde_json::Value>(&body)
        && let Some(msg) = envelope
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
    {
        return anyhow::anyhow!("daemon error (HTTP {}): {}", status.as_u16(), msg);
    }

    anyhow::anyhow!("daemon returned HTTP {}: {}", status.as_u16(), body)
}

/// GET a JSON response from the daemon.
pub async fn get_json<T: DeserializeOwned>(path: &str) -> Result<T> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()?;
    let resp = client
        .get(format!("{DAEMON_URL}{path}"))
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(extract_error(resp).await);
    }
    Ok(resp.json::<T>().await?)
}

/// POST to the daemon and get a JSON response.
pub async fn post_empty<T: DeserializeOwned>(path: &str) -> Result<T> {
    post_empty_timeout(path, std::time::Duration::from_secs(5)).await
}

/// POST to the daemon with a custom timeout and get a JSON response.
pub async fn post_empty_timeout<T: DeserializeOwned>(
    path: &str,
    timeout: std::time::Duration,
) -> Result<T> {
    let client = reqwest::Client::builder().timeout(timeout).build()?;
    let resp = client
        .post(format!("{DAEMON_URL}{path}"))
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(extract_error(resp).await);
    }
    Ok(resp.json::<T>().await?)
}

/// POST a JSON body to the daemon and get a JSON response.
pub async fn post_json<B: Serialize, T: DeserializeOwned>(path: &str, body: &B) -> Result<T> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()?;
    let resp = client
        .post(format!("{DAEMON_URL}{path}"))
        .json(body)
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(extract_error(resp).await);
    }
    Ok(resp.json::<T>().await?)
}
