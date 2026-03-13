//! OpenAI-compatible LLM provider implementation.

use super::{CompletionOptions, CompletionResponse, LLMProvider, Message, TokenUsage};
use crate::error::{AgentError, AgentResult};
use async_trait::async_trait;

/// Default OpenAI Chat Completions endpoint.
const DEFAULT_API_URL: &str = "https://api.openai.com/v1/chat/completions";

/// Default OpenAI Responses endpoint.
const DEFAULT_RESPONSES_URL: &str = "https://api.openai.com/v1/responses";

/// Which OpenAI API surface to target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OpenAiApiMode {
    /// Chat Completions API (`/v1/chat/completions`).  Default.
    #[default]
    Completions,
    /// Responses API (`/v1/responses`).
    Responses,
}

/// OpenAI-compatible LLM provider.
///
/// Works with OpenAI API and compatible endpoints (Anthropic, local models, etc.).
///
/// # Example
/// ```ignore
/// use spider_agent::llm::{OpenAIProvider, Message, CompletionOptions};
///
/// let provider = OpenAIProvider::new("sk-...", "gpt-4o");
/// let client = reqwest::Client::new();
///
/// let messages = vec![
///     Message::system("You are a helpful assistant."),
///     Message::user("What is Rust?"),
/// ];
///
/// let response = provider.complete(messages, &CompletionOptions::default(), &client).await?;
/// println!("{}", response.content);
/// ```
#[derive(Debug, Clone)]
pub struct OpenAIProvider {
    api_key: String,
    api_url: String,
    model: String,
    api_mode: OpenAiApiMode,
}

impl OpenAIProvider {
    /// Create a new OpenAI provider (Chat Completions API by default).
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            api_url: DEFAULT_API_URL.to_string(),
            model: model.into(),
            api_mode: OpenAiApiMode::Completions,
        }
    }

    /// Use a custom API endpoint (for compatible APIs).
    ///
    /// If the URL does not already end with the expected path for the
    /// current API mode, it is appended automatically so callers can pass
    /// just a base URL (e.g. `https://my-server.com/v1`).
    pub fn with_api_url(mut self, url: impl Into<String>) -> Self {
        self.api_url = normalize_api_url(url.into(), self.api_mode);
        self
    }

    /// Change the model.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    /// Switch to the OpenAI Responses API (`/v1/responses`).
    ///
    /// The Responses API uses `instructions` + `input` instead of a
    /// `messages` array and returns structured `output` items.
    ///
    /// # Example
    /// ```ignore
    /// let provider = OpenAIProvider::new("sk-...", "gpt-4o")
    ///     .with_responses_api();
    /// ```
    pub fn with_responses_api(mut self) -> Self {
        self.api_mode = OpenAiApiMode::Responses;
        // Reset URL to the correct default when switching mode, unless the
        // caller already set a custom URL.
        if self.api_url == DEFAULT_API_URL {
            self.api_url = DEFAULT_RESPONSES_URL.to_string();
        } else {
            self.api_url = normalize_api_url(self.api_url, OpenAiApiMode::Responses);
        }
        self
    }
}

#[async_trait]
impl LLMProvider for OpenAIProvider {
    async fn complete(
        &self,
        messages: Vec<Message>,
        options: &CompletionOptions,
        client: &reqwest::Client,
    ) -> AgentResult<CompletionResponse> {
        let body = match self.api_mode {
            OpenAiApiMode::Completions => build_completions_body(&self.model, &messages, options),
            OpenAiApiMode::Responses => build_responses_body(&self.model, &messages, options),
        };

        let response = client
            .post(&self.api_url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(AgentError::Llm("Authentication failed".to_string()));
        }
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(AgentError::RateLimited);
        }
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(AgentError::Llm(format!("HTTP {}: {}", status, error_text)));
        }

        let json: serde_json::Value = response.json().await?;

        match self.api_mode {
            OpenAiApiMode::Completions => parse_completions_response(json),
            OpenAiApiMode::Responses => parse_responses_response(json),
        }
    }

    fn provider_name(&self) -> &'static str {
        match self.api_mode {
            OpenAiApiMode::Completions => "openai",
            OpenAiApiMode::Responses => "openai_responses",
        }
    }

    fn is_configured(&self) -> bool {
        !self.api_key.is_empty()
    }
}

// --------------- Chat Completions helpers ---------------

fn build_completions_body(
    model: &str,
    messages: &[Message],
    options: &CompletionOptions,
) -> serde_json::Value {
    let mut body = serde_json::json!({
        "model": model,
        "messages": messages,
        "temperature": options.temperature,
        "max_tokens": options.max_tokens,
    });
    if options.json_mode {
        body["response_format"] = serde_json::json!({ "type": "json_object" });
    }
    body
}

fn parse_completions_response(json: serde_json::Value) -> AgentResult<CompletionResponse> {
    let content = json
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .ok_or_else(|| AgentError::MissingField("choices[0].message.content"))?
        .to_string();

    let usage = extract_usage(&json);
    Ok(CompletionResponse { content, usage })
}

// --------------- Responses API helpers ---------------

fn build_responses_body(
    model: &str,
    messages: &[Message],
    options: &CompletionOptions,
) -> serde_json::Value {
    // Separate system messages into `instructions` and the rest into `input`.
    let mut instructions = String::new();
    let mut input_items = Vec::new();

    for msg in messages {
        if msg.role == "system" {
            if !instructions.is_empty() {
                instructions.push('\n');
            }
            instructions.push_str(msg.content.as_text());
        } else {
            input_items.push(serde_json::json!({
                "role": &msg.role,
                "content": &msg.content,
            }));
        }
    }

    let mut body = serde_json::json!({
        "model": model,
        "input": input_items,
        "temperature": options.temperature,
    });

    if !instructions.is_empty() {
        body["instructions"] = serde_json::json!(instructions);
    }

    if options.json_mode {
        body["text"] = serde_json::json!({
            "format": { "type": "json_object" }
        });
    }

    body
}

fn parse_responses_response(json: serde_json::Value) -> AgentResult<CompletionResponse> {
    // Try the `output_text` shorthand first (present in newer API responses).
    if let Some(text) = json.get("output_text").and_then(|v| v.as_str()) {
        return Ok(CompletionResponse {
            content: text.to_string(),
            usage: extract_usage(&json),
        });
    }

    // Fall back to walking the `output` array.
    let content = json
        .get("output")
        .and_then(|o| o.as_array())
        .and_then(|arr| {
            for item in arr {
                if item.get("type").and_then(|t| t.as_str()) == Some("message") {
                    if let Some(parts) = item.get("content").and_then(|c| c.as_array()) {
                        for part in parts {
                            if part.get("type").and_then(|t| t.as_str()) == Some("output_text") {
                                return part.get("text").and_then(|t| t.as_str());
                            }
                        }
                    }
                }
            }
            None
        })
        .ok_or_else(|| AgentError::MissingField("output[].content[].text"))?
        .to_string();

    Ok(CompletionResponse {
        content,
        usage: extract_usage(&json),
    })
}

// --------------- Shared helpers ---------------

fn extract_usage(json: &serde_json::Value) -> TokenUsage {
    if let Some(u) = json.get("usage") {
        TokenUsage {
            prompt_tokens: u
                .get("prompt_tokens")
                .or_else(|| u.get("input_tokens"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32,
            completion_tokens: u
                .get("completion_tokens")
                .or_else(|| u.get("output_tokens"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32,
            total_tokens: u.get("total_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
        }
    } else {
        TokenUsage::default()
    }
}

/// Ensure the URL ends with the correct path for the given API mode.
///
/// Accepts both full endpoints and base-only URLs.  A trailing slash on
/// the input is tolerated.  Avoids allocation when the URL is already correct.
fn normalize_api_url(mut url: String, mode: OpenAiApiMode) -> String {
    // Strip trailing slashes in-place.
    while url.ends_with('/') {
        url.pop();
    }

    let suffix = match mode {
        OpenAiApiMode::Completions => "/chat/completions",
        OpenAiApiMode::Responses => "/responses",
    };

    if !url.ends_with(suffix) {
        // When switching modes, strip the *other* mode's suffix first.
        match mode {
            OpenAiApiMode::Responses => {
                if let Some(base) = url.strip_suffix("/chat/completions") {
                    url.truncate(base.len());
                }
            }
            OpenAiApiMode::Completions => {
                if let Some(base) = url.strip_suffix("/responses") {
                    url.truncate(base.len());
                }
            }
        }
        url.push_str(suffix);
    }

    url
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_openai_provider_new() {
        let provider = OpenAIProvider::new("sk-test", "gpt-4o");
        assert!(provider.is_configured());
        assert_eq!(provider.model, "gpt-4o");
    }

    #[test]
    fn test_openai_provider_custom_url_full_path() {
        let provider = OpenAIProvider::new("sk-test", "gpt-4o")
            .with_api_url("https://custom.api.com/v1/chat/completions");
        assert_eq!(
            provider.api_url,
            "https://custom.api.com/v1/chat/completions"
        );
    }

    #[test]
    fn test_openai_provider_custom_url_base_only() {
        let provider =
            OpenAIProvider::new("sk-test", "gpt-4o").with_api_url("https://api.xyz.com/llm/v1");
        assert_eq!(
            provider.api_url,
            "https://api.xyz.com/llm/v1/chat/completions"
        );
    }

    #[test]
    fn test_openai_provider_custom_url_trailing_slash() {
        let provider =
            OpenAIProvider::new("sk-test", "gpt-4o").with_api_url("https://api.xyz.com/llm/v1/");
        assert_eq!(
            provider.api_url,
            "https://api.xyz.com/llm/v1/chat/completions"
        );
    }

    #[test]
    fn test_openai_provider_custom_url_full_path_trailing_slash() {
        let provider = OpenAIProvider::new("sk-test", "gpt-4o")
            .with_api_url("https://custom.api.com/v1/chat/completions/");
        assert_eq!(
            provider.api_url,
            "https://custom.api.com/v1/chat/completions"
        );
    }

    #[test]
    fn test_normalize_api_url_bare_host() {
        assert_eq!(
            normalize_api_url("https://localhost:8000".into(), OpenAiApiMode::Completions),
            "https://localhost:8000/chat/completions"
        );
    }

    #[test]
    fn test_normalize_api_url_idempotent() {
        let url = "https://api.openai.com/v1/chat/completions".to_string();
        assert_eq!(
            normalize_api_url(url.clone(), OpenAiApiMode::Completions),
            url
        );
    }

    #[test]
    fn test_openai_provider_empty_key_not_configured() {
        let provider = OpenAIProvider::new("", "gpt-4o");
        assert!(!provider.is_configured());
    }

    #[test]
    fn test_responses_api_default_url() {
        let provider = OpenAIProvider::new("sk-test", "gpt-4o").with_responses_api();
        assert_eq!(provider.api_url, DEFAULT_RESPONSES_URL);
        assert_eq!(provider.api_mode, OpenAiApiMode::Responses);
    }

    #[test]
    fn test_responses_api_custom_base_url() {
        let provider = OpenAIProvider::new("sk-test", "gpt-4o")
            .with_responses_api()
            .with_api_url("https://custom.api.com/v1");
        assert_eq!(provider.api_url, "https://custom.api.com/v1/responses");
    }

    #[test]
    fn test_responses_api_full_url_preserved() {
        let provider = OpenAIProvider::new("sk-test", "gpt-4o")
            .with_responses_api()
            .with_api_url("https://custom.api.com/v1/responses");
        assert_eq!(provider.api_url, "https://custom.api.com/v1/responses");
    }

    #[test]
    fn test_responses_api_switches_from_completions_url() {
        let provider = OpenAIProvider::new("sk-test", "gpt-4o")
            .with_api_url("https://custom.api.com/v1/chat/completions")
            .with_responses_api();
        assert_eq!(provider.api_url, "https://custom.api.com/v1/responses");
    }

    #[test]
    fn test_normalize_url_responses_mode_bare_host() {
        assert_eq!(
            normalize_api_url("https://localhost:8000".into(), OpenAiApiMode::Responses),
            "https://localhost:8000/responses"
        );
    }

    #[test]
    fn test_normalize_url_strips_completions_for_responses() {
        assert_eq!(
            normalize_api_url(
                "https://api.example.com/v1/chat/completions".into(),
                OpenAiApiMode::Responses
            ),
            "https://api.example.com/v1/responses"
        );
    }

    #[test]
    fn test_normalize_url_strips_responses_for_completions() {
        assert_eq!(
            normalize_api_url(
                "https://api.example.com/v1/responses".into(),
                OpenAiApiMode::Completions
            ),
            "https://api.example.com/v1/chat/completions"
        );
    }

    #[test]
    fn test_responses_provider_name() {
        let provider = OpenAIProvider::new("sk-test", "gpt-4o").with_responses_api();
        assert_eq!(provider.provider_name(), "openai_responses");
    }

    #[test]
    fn test_completions_provider_name() {
        let provider = OpenAIProvider::new("sk-test", "gpt-4o");
        assert_eq!(provider.provider_name(), "openai");
    }

    #[test]
    fn test_build_responses_body_system_becomes_instructions() {
        let messages = vec![Message::system("You are helpful."), Message::user("Hello")];
        let options = CompletionOptions::default();
        let body = build_responses_body("gpt-4o", &messages, &options);

        assert_eq!(body["instructions"], "You are helpful.");
        assert!(body.get("messages").is_none());
        let input = body["input"].as_array().expect("input should be array");
        assert_eq!(input.len(), 1);
        assert_eq!(input[0]["role"], "user");
    }

    #[test]
    fn test_build_responses_body_no_system() {
        let messages = vec![Message::user("Hello")];
        let options = CompletionOptions::default();
        let body = build_responses_body("gpt-4o", &messages, &options);

        assert!(body.get("instructions").is_none());
        assert_eq!(body["input"].as_array().expect("array").len(), 1);
    }

    #[test]
    fn test_build_responses_body_json_mode() {
        let messages = vec![Message::user("Hello")];
        let mut options = CompletionOptions::default();
        options.json_mode = true;
        let body = build_responses_body("gpt-4o", &messages, &options);
        assert_eq!(body["text"]["format"]["type"], "json_object");
    }

    #[test]
    fn test_build_completions_body_json_mode() {
        let messages = vec![Message::user("Hello")];
        let mut options = CompletionOptions::default();
        options.json_mode = true;
        let body = build_completions_body("gpt-4o", &messages, &options);
        assert_eq!(body["response_format"]["type"], "json_object");
    }

    #[test]
    fn test_parse_responses_output_text_shorthand() {
        let json = serde_json::json!({
            "output_text": "Hello world",
            "usage": { "input_tokens": 10, "output_tokens": 5, "total_tokens": 15 }
        });
        let resp = parse_responses_response(json).expect("should parse");
        assert_eq!(resp.content, "Hello world");
        assert_eq!(resp.usage.prompt_tokens, 10);
        assert_eq!(resp.usage.completion_tokens, 5);
        assert_eq!(resp.usage.total_tokens, 15);
    }

    #[test]
    fn test_parse_responses_output_array() {
        let json = serde_json::json!({
            "output": [{
                "type": "message",
                "content": [{
                    "type": "output_text",
                    "text": "Extracted data"
                }]
            }]
        });
        let resp = parse_responses_response(json).expect("should parse");
        assert_eq!(resp.content, "Extracted data");
    }

    #[test]
    fn test_parse_responses_missing_output() {
        let json = serde_json::json!({});
        let err = parse_responses_response(json).unwrap_err();
        assert!(format!("{}", err).contains("Missing field"));
    }

    #[test]
    fn test_extract_usage_openai_keys() {
        let json = serde_json::json!({
            "usage": { "prompt_tokens": 10, "completion_tokens": 20, "total_tokens": 30 }
        });
        let u = extract_usage(&json);
        assert_eq!(u.prompt_tokens, 10);
        assert_eq!(u.completion_tokens, 20);
        assert_eq!(u.total_tokens, 30);
    }

    #[test]
    fn test_extract_usage_responses_keys() {
        let json = serde_json::json!({
            "usage": { "input_tokens": 10, "output_tokens": 20, "total_tokens": 30 }
        });
        let u = extract_usage(&json);
        assert_eq!(u.prompt_tokens, 10);
        assert_eq!(u.completion_tokens, 20);
    }

    #[test]
    fn test_extract_usage_missing() {
        let json = serde_json::json!({});
        let u = extract_usage(&json);
        assert_eq!(u.prompt_tokens, 0);
        assert_eq!(u.completion_tokens, 0);
        assert_eq!(u.total_tokens, 0);
    }
}
