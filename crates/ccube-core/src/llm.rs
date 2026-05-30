// LLM client supporting OpenAI-compatible chat completion endpoints.
// Uses ureq (blocking HTTP) — reqwest has a connector bug on some macOS versions.
// For Ollama, uses the native /api/chat endpoint with think:false to disable reasoning.

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

/// Concrete LLM client. Auto-detects Ollama vs OpenAI-compatible.
pub struct LlamaCppClient {
    base_url: String,
    timeout: Duration,
    token: Option<String>,
    provider: String,
}

impl LlamaCppClient {
    pub fn from_env() -> Result<Self, String> {
        Self::from_env_with_timeout(Duration::from_secs(120))
    }

    pub fn from_env_with_timeout(timeout: Duration) -> Result<Self, String> {
        let base_url =
            std::env::var("CCUBE_LLM_URL").unwrap_or_else(|_| "http://localhost:8080".to_string());
        let token = std::env::var("CCUBE_LLM_TOKEN")
            .ok()
            .filter(|t| !t.is_empty());
        let provider =
            std::env::var("CCUBE_LLM_PROVIDER").unwrap_or_else(|_| "openai-compatible".to_string());

        Ok(Self {
            base_url,
            timeout,
            token,
            provider,
        })
    }

    fn model() -> String {
        std::env::var("CCUBE_LLM_MODEL").unwrap_or_else(|_| "default".to_string())
    }

    fn do_request(
        &self,
        prompt: &str,
        grammar: &str,
        n_predict: u32,
        temperature: f32,
    ) -> Result<LlmResponse, LlmError> {
        if self.provider == "ollama" {
            self.do_ollama_request(prompt, n_predict, temperature)
        } else {
            self.do_openai_request(prompt, grammar, n_predict, temperature)
        }
    }

    /// Ollama native /api/chat endpoint with think:false.
    fn do_ollama_request(
        &self,
        prompt: &str,
        n_predict: u32,
        temperature: f32,
    ) -> Result<LlmResponse, LlmError> {
        let base = self.base_url.trim_end_matches('/');
        // Strip /v1 suffix for ollama native API
        let base_clean = base.strip_suffix("/v1").unwrap_or(base);
        let url = format!("{}/api/chat", base_clean);

        let body = serde_json::json!({
            "model": Self::model(),
            "messages": [{"role": "user", "content": prompt}],
            "stream": false,
            "think": false,
            "options": {
                "num_predict": n_predict,
                "temperature": temperature,
            }
        });

        let timeout_secs = self.timeout.as_secs().max(30);
        let agent = ureq::AgentBuilder::new()
            .timeout_read(Duration::from_secs(timeout_secs))
            .timeout_write(Duration::from_secs(30))
            .build();

        let mut req = agent.post(&url);
        if let Some(ref token) = self.token {
            req = req.set("Authorization", &format!("Bearer {}", token));
        }

        let response = req
            .send_json(body)
            .map_err(|e| LlmError::Unreachable(format!("{e}")))?;

        let parsed: serde_json::Value = response
            .into_json()
            .map_err(|e| LlmError::BadResponse(format!("failed to parse: {e}")))?;

        let content = parsed
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .to_string();

        let model = parsed
            .get("model")
            .and_then(|m| m.as_str())
            .map(|s| s.to_string());

        let content = strip_markdown_fences(&content);

        if content.trim().is_empty() {
            return Err(LlmError::BadResponse("empty response content".into()));
        }

        Ok(LlmResponse { content, model })
    }

    /// OpenAI-compatible /v1/chat/completions endpoint.
    fn do_openai_request(
        &self,
        prompt: &str,
        grammar: &str,
        n_predict: u32,
        temperature: f32,
    ) -> Result<LlmResponse, LlmError> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/chat/completions", base);

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
            #[serde(default)]
            reasoning: Option<String>,
        }

        let body = ChatCompletionRequest {
            model: &Self::model(),
            messages: &[ChatMessage {
                role: "user",
                content: prompt,
            }],
            max_tokens: n_predict,
            temperature,
            grammar: if grammar.is_empty() {
                None
            } else {
                Some(grammar)
            },
        };

        let timeout_secs = self.timeout.as_secs().max(30);
        let agent = ureq::AgentBuilder::new()
            .timeout_read(Duration::from_secs(timeout_secs))
            .timeout_write(Duration::from_secs(30))
            .build();

        let mut req = agent.post(&url);
        if let Some(ref token) = self.token {
            req = req.set("Authorization", &format!("Bearer {}", token));
        }

        let response = req
            .send_json(serde_json::to_value(&body).unwrap_or_default())
            .map_err(|e| LlmError::Unreachable(format!("{e}")))?;

        let parsed: ChatCompletionResponse = response
            .into_json()
            .map_err(|e| LlmError::BadResponse(format!("failed to parse: {e}")))?;

        let content = parsed
            .choices
            .into_iter()
            .next()
            .and_then(|c| {
                if let Some(content) = c.message.content {
                    if !content.trim().is_empty() {
                        return Some(content);
                    }
                }
                c.message.reasoning
            })
            .ok_or_else(|| LlmError::BadResponse("empty response".into()))?;

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

#[async_trait]
impl LlmBackend for LlamaCppClient {
    async fn complete(
        &self,
        prompt: &str,
        grammar: &str,
        n_predict: u32,
        temperature: f32,
    ) -> Result<LlmResponse, LlmError> {
        let prompt = prompt.to_string();
        let grammar = grammar.to_string();
        let timeout = self.timeout;
        let provider = self.provider.clone();
        let base_url = self.base_url.clone();
        let token = self.token.clone();

        tokio::task::spawn_blocking(move || {
            let client = LlamaCppClient {
                base_url,
                timeout,
                token,
                provider,
            };
            client.do_request(&prompt, &grammar, n_predict, temperature)
        })
        .await
        .map_err(|e| LlmError::Unreachable(format!("task panicked: {e}")))?
    }
}

/// Strip markdown code fences from LLM output.
fn strip_markdown_fences(s: &str) -> String {
    let s = s.trim();
    if let Some(after_open) = s.strip_prefix("```") {
        let content_start = after_open.find('\n').map(|i| i + 1).unwrap_or(0);
        let content = &after_open[content_start.min(after_open.len())..];
        if let Some(end) = content.rfind("```") {
            return content[..end].trim().to_string();
        }
        return content.trim().to_string();
    }
    s.to_string()
}

/// Send a screenshot to a vision-capable Ollama model for activity classification.
/// Returns the raw JSON string from the model (activity, category, is_distraction).
/// Uses CCUBE_VISION_MODEL env var (default: "gemma4:e4b").
pub fn vision_classify(png_bytes: &[u8]) -> Result<String, LlmError> {
    use base64::Engine;
    let img_b64 = base64::engine::general_purpose::STANDARD.encode(png_bytes);

    let model = std::env::var("CCUBE_VISION_MODEL")
        .unwrap_or_else(|_| "gemma4:e4b".to_string());

    let base_url = std::env::var("CCUBE_LLM_URL")
        .unwrap_or_else(|_| "http://localhost:11434".to_string());
    let base_clean = base_url.trim_end_matches('/').strip_suffix("/v1").unwrap_or(&base_url);
    let url = format!("{}/api/chat", base_clean);

    let token = std::env::var("CCUBE_LLM_TOKEN")
        .ok()
        .filter(|t| !t.is_empty());

    let prompt = r#"Look at this screenshot. What is the user doing? Reply in exactly this JSON format:
{"activity": "short description", "category": "coding|writing|browsing|communication|media|design|gaming|system|other", "is_distraction": true or false}

Respond with ONLY the JSON object, no other text."#;

    let body = serde_json::json!({
        "model": model,
        "messages": [{
            "role": "user",
            "content": prompt,
            "images": [img_b64]
        }],
        "stream": false,
        "think": false,
        "options": {
            "num_predict": 100,
            "temperature": 0.2
        }
    });

    let agent = ureq::AgentBuilder::new()
        .timeout_read(Duration::from_secs(30))
        .timeout_write(Duration::from_secs(10))
        .build();

    let mut req = agent.post(&url);
    if let Some(ref t) = token {
        req = req.set("Authorization", &format!("Bearer {}", t));
    }

    let response = req
        .send_json(body)
        .map_err(|e| LlmError::Unreachable(format!("vision: {e}")))?;

    let parsed: serde_json::Value = response
        .into_json()
        .map_err(|e| LlmError::BadResponse(format!("vision parse: {e}")))?;

    let content = parsed
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .unwrap_or("")
        .to_string();

    let content = strip_markdown_fences(&content);

    if content.trim().is_empty() {
        return Err(LlmError::BadResponse("vision returned empty".into()));
    }

    Ok(content.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_no_fences() {
        assert_eq!(strip_markdown_fences("plain text"), "plain text");
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
