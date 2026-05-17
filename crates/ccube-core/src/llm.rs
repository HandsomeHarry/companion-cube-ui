// LLM client supporting OpenAI-compatible chat completion endpoints.
// Works with both llama.cpp (OpenAI-compatible mode) and OpenAI API proxies.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Response from an LLM completion call.
#[derive(Debug)]
pub struct LlmResponse {
    pub content: String,
    pub model: Option<String>,
}

/// Errors that can occur during LLM calls.
#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("LLM unreachable: {0}")]
    Unreachable(String),
    #[error("LLM bad response: {0}")]
    BadResponse(String),
}

/// Trait for LLM backends, enabling test mocking.
#[async_trait]
pub trait LlmBackend: Send + Sync {
    async fn complete(
        &self,
        prompt: &str,
        grammar: &str,
        n_predict: u32,
        temperature: f32,
    ) -> Result<LlmResponse, LlmError>;
}

/// Concrete LLM client using the OpenAI chat completions protocol.
pub struct LlamaCppClient {
    base_url: String,
    http: reqwest::Client,
    /// Stored for potential inspection; consumed during construction.
    #[allow(dead_code)]
    token: Option<String>,
}

// -- OpenAI chat completions request / response shapes --

#[derive(Serialize)]
struct ChatCompletionRequest<'a> {
    model: &'a str,
    messages: &'a [ChatMessage<'a>],
    max_tokens: u32,
    temperature: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    grammar: Option<&'a str>,
}

#[derive(Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct ChatCompletionResponse {
    #[allow(dead_code)]
    id: Option<String>,
    choices: Vec<Choice>,
    model: Option<String>,
}

#[derive(Deserialize)]
struct Choice {
    message: MessageContent,
}

#[derive(Deserialize)]
struct MessageContent {
    content: Option<String>,
}

impl LlamaCppClient {
    /// Create a client from `CCUBE_LLM_URL` (default `http://localhost:8080`).
    /// If `CCUBE_LLM_TOKEN` is set, it is sent as a Bearer token.
    pub fn from_env() -> Result<Self, String> {
        Self::from_env_with_timeout(Duration::from_secs(10))
    }

    /// Create a client with a custom timeout.
    /// Use longer timeouts for curator/reflector calls that produce more output.
    pub fn from_env_with_timeout(timeout: Duration) -> Result<Self, String> {
        let base_url =
            std::env::var("CCUBE_LLM_URL").unwrap_or_else(|_| "http://localhost:8080".to_string());

        let token = std::env::var("CCUBE_LLM_TOKEN").ok().filter(|t| !t.is_empty());

        let mut builder = reqwest::Client::builder().timeout(timeout);

        // Attach Bearer token if provided
        if let Some(ref t) = token {
            let mut headers = reqwest::header::HeaderMap::new();
            let auth_value = format!("Bearer {}", t);
            headers.insert(
                reqwest::header::AUTHORIZATION,
                reqwest::header::HeaderValue::from_str(&auth_value)
                    .map_err(|e| format!("invalid CCUBE_LLM_TOKEN: {e}"))?,
            );
            builder = builder.default_headers(headers);
        }

        let http = builder
            .build()
            .map_err(|e| format!("failed to build HTTP client: {e}"))?;

        Ok(Self {
            base_url,
            http,
            token,
        })
    }

    /// The model identifier sent in the request body.
    /// Read from `CCUBE_LLM_MODEL` or defaults to "default".
    fn model() -> String {
        std::env::var("CCUBE_LLM_MODEL").unwrap_or_else(|_| "default".to_string())
    }
}

#[async_trait]
impl LlmBackend for LlamaCppClient {
    async fn complete(
        &self,
        prompt: &str,
        grammar: &str,
        n_predict: u32,
        temperature: f32,
    ) -> Result<LlmResponse, LlmError> {
        // Strip trailing slash so we can append cleanly
        let base = self.base_url.trim_end_matches('/');

        // Try OpenAI chat completions endpoint first.
        let url = format!("{}/chat/completions", base);

        let body = ChatCompletionRequest {
            model: &Self::model(),
            messages: &[ChatMessage {
                role: "user",
                content: prompt,
            }],
            max_tokens: n_predict,
            temperature,
            grammar: if grammar.is_empty() { None } else { Some(grammar) },
        };

        let resp = self
            .http
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| LlmError::Unreachable(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();
            return Err(LlmError::Unreachable(format!(
                "HTTP {}: {}",
                status, body_text
            )));
        }

        let parsed: ChatCompletionResponse = resp
            .json()
            .await
            .map_err(|e| LlmError::BadResponse(format!("failed to parse response: {e}")))?;

        let content = parsed
            .choices
            .into_iter()
            .next()
            .and_then(|c| c.message.content)
            .ok_or_else(|| LlmError::BadResponse("empty response — no choices".into()))?;

        // Strip markdown code fences — many LLMs wrap JSON in ```json ... ``` blocks
        let content = strip_markdown_fences(&content);

        if content.trim().is_empty() {
            return Err(LlmError::BadResponse("empty response content".into()));
        }

        Ok(LlmResponse {
            content,
            model: parsed.model,
        })
    }
}

/// Strip markdown code fences (```json ... ```) from LLM output if present.
/// Many LLMs wrap JSON in code fences when grammar constraints aren't
/// enforced server-side (e.g. OpenAI API ignores GBNF grammars).
fn strip_markdown_fences(s: &str) -> String {
    let s = s.trim();
    if let Some(after_open) = s.strip_prefix("```") {
        // after_open includes everything after the opening ```
        // e.g. "json\n{...}\n```" or "\n{...}\n```"
        // Find the end of the first line (language tag or empty)
        let content_start = after_open.find('\n').map(|i| i + 1).unwrap_or(0);
        let content = &after_open[content_start.min(after_open.len())..];
        // Find and strip the closing ``` if present
        if let Some(end) = content.rfind("```") {
            return content[..end].trim().to_string();
        }
        return content.trim().to_string();
    }
    s.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------
    // strip_markdown_fences tests
    // ------------------------------------------------------------------

    #[test]
    fn test_strip_no_fences() {
        assert_eq!(strip_markdown_fences("plain text"), "plain text");
    }

    #[test]
    fn test_strip_plain_json() {
        let json = r#"{"key":"value"}"#;
        assert_eq!(strip_markdown_fences(json), json);
    }

    #[test]
    fn test_strip_with_lang_tag() {
        let wrapped = "```json\n{\"key\":\"value\"}\n```".to_string();
        assert_eq!(strip_markdown_fences(&wrapped), r#"{"key":"value"}"#);
    }

    #[test]
    fn test_strip_without_lang_tag() {
        let wrapped = "```\n{\"key\":\"value\"}\n```".to_string();
        assert_eq!(strip_markdown_fences(&wrapped), r#"{"key":"value"}"#);
    }

    #[test]
    fn test_strip_multiline_with_fences() {
        let wrapped = "```json\n{\n  \"new_patterns_md\": \"§ rule 1\",\n  \"rationale\": \"merged\"\n}\n```".to_string();
        let result = strip_markdown_fences(&wrapped);
        assert!(result.contains("\"new_patterns_md\""));
        assert!(result.contains("\"rationale\""));
        assert!(!result.contains("```"));
    }

    #[test]
    fn test_strip_no_closing_fence() {
        let wrapped = "```json\n{\"key\":\"value\"}".to_string();
        assert_eq!(strip_markdown_fences(&wrapped), r#"{"key":"value"}"#);
    }

    #[test]
    fn test_strip_whitespace_around() {
        let wrapped = "  \n```json\n{\"key\":\"value\"}\n```\n  ".to_string();
        assert_eq!(strip_markdown_fences(&wrapped), r#"{"key":"value"}"#);
    }

    struct MockLlm {
        response: Result<String, LlmError>,
    }

    #[async_trait]
    impl LlmBackend for MockLlm {
        async fn complete(
            &self,
            _prompt: &str,
            _grammar: &str,
            _n_predict: u32,
            _temperature: f32,
        ) -> Result<LlmResponse, LlmError> {
            match &self.response {
                Ok(content) => Ok(LlmResponse {
                    content: content.clone(),
                    model: Some("test-model".to_string()),
                }),
                Err(_) => Err(LlmError::Unreachable("mock unreachable".into())),
            }
        }
    }

    #[tokio::test]
    async fn test_mock_returns_content() {
        let llm = MockLlm {
            response: Ok(r#"{"decision":"silent","reasoning":"test"}"#.to_string()),
        };
        let resp = llm.complete("prompt", "", 512, 0.2).await.unwrap();
        assert!(resp.content.contains("silent"));
        assert_eq!(resp.model.as_deref(), Some("test-model"));
    }

    #[tokio::test]
    async fn test_mock_unreachable() {
        let llm = MockLlm {
            response: Err(LlmError::Unreachable("down".into())),
        };
        let err = llm.complete("prompt", "", 512, 0.2).await.unwrap_err();
        assert!(matches!(err, LlmError::Unreachable(_)));
    }
}
