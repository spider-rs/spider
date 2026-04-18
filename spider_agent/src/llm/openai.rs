//! OpenAI-compatible LLM provider implementation.

use super::{CompletionOptions, CompletionResponse, LLMProvider, Message, TokenUsage};
use crate::error::{AgentError, AgentResult};
use async_trait::async_trait;

/// Default OpenAI base URL.
const DEFAULT_OPENAI_BASE: &str = "https://api.openai.com";

/// Which OpenAI API surface to target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OpenAiApiMode {
    /// Auto-detect from the URL: `api.openai.com` → Responses,
    /// everything else → Completions.  This is the default.
    #[default]
    Auto,
    /// Chat Completions API (`/v1/chat/completions`).
    Completions,
    /// Responses API (`/v1/responses`).
    Responses,
}

/// Resolved (non-Auto) mode used at request time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResolvedMode {
    Completions,
    Responses,
}

/// Returns true when the URL points at OpenAI's first-party API.
fn is_openai_url(url: &str) -> bool {
    // Fast byte check — avoid allocations.
    url.starts_with("https://api.openai.com") || url.starts_with("http://api.openai.com")
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
    /// Create a new OpenAI provider.
    ///
    /// Defaults to `Auto` mode: uses the Responses API for `api.openai.com`
    /// and Chat Completions for everything else.
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            api_url: DEFAULT_OPENAI_BASE.to_string(),
            model: model.into(),
            api_mode: OpenAiApiMode::Auto,
        }
    }

    /// Use a custom API endpoint (for compatible APIs).
    ///
    /// In `Auto` mode the path suffix is resolved at request time based on
    /// the host.  In explicit `Completions` / `Responses` mode the correct
    /// suffix is appended immediately if missing.
    pub fn with_api_url(mut self, url: impl Into<String>) -> Self {
        let raw = url.into();
        match self.api_mode {
            OpenAiApiMode::Auto => {
                // Store the base; path is resolved at request time.
                self.api_url = strip_known_suffixes(raw);
            }
            mode => {
                self.api_url = normalize_api_url(raw, mode);
            }
        }
        self
    }

    /// Change the model.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    /// Force the Chat Completions API (`/v1/chat/completions`).
    pub fn with_completions_api(mut self) -> Self {
        self.api_mode = OpenAiApiMode::Completions;
        self.api_url = normalize_api_url(self.api_url, OpenAiApiMode::Completions);
        self
    }

    /// Force the Responses API (`/v1/responses`).
    ///
    /// The Responses API uses `instructions` + `input` instead of a
    /// `messages` array and returns structured `output` items.
    pub fn with_responses_api(mut self) -> Self {
        self.api_mode = OpenAiApiMode::Responses;
        self.api_url = normalize_api_url(self.api_url, OpenAiApiMode::Responses);
        self
    }

    /// Resolve the effective mode (never `Auto`).
    fn resolved_mode(&self) -> ResolvedMode {
        match self.api_mode {
            OpenAiApiMode::Completions => ResolvedMode::Completions,
            OpenAiApiMode::Responses => ResolvedMode::Responses,
            OpenAiApiMode::Auto => {
                if is_openai_url(&self.api_url) {
                    ResolvedMode::Responses
                } else {
                    ResolvedMode::Completions
                }
            }
        }
    }

    /// Build the full request URL, appending the path suffix when in Auto mode.
    fn request_url(&self) -> String {
        match self.api_mode {
            OpenAiApiMode::Auto => {
                let base = self.api_url.trim_end_matches('/');
                // Already has a known endpoint suffix — use as-is.
                if base.ends_with("/responses") || base.ends_with("/chat/completions") {
                    return base.to_string();
                }
                let suffix = match self.resolved_mode() {
                    ResolvedMode::Responses => "/responses",
                    ResolvedMode::Completions => "/chat/completions",
                };
                format!("{}{}", base, suffix)
            }
            _ => self.api_url.clone(),
        }
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
        let mode = self.resolved_mode();

        let body = match mode {
            ResolvedMode::Completions => build_completions_body(&self.model, &messages, options),
            ResolvedMode::Responses => build_responses_body(&self.model, &messages, options),
        };

        let url = self.request_url();

        let response = client
            .post(&url)
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

        match mode {
            ResolvedMode::Completions => parse_completions_response(json),
            ResolvedMode::Responses => parse_responses_response(json),
        }
    }

    fn provider_name(&self) -> &'static str {
        match self.resolved_mode() {
            ResolvedMode::Completions => "openai",
            ResolvedMode::Responses => "openai_responses",
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
    let mut input_items = Vec::with_capacity(messages.len());

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

/// Strip `/chat/completions` or `/responses` suffix, returning just the base.
fn strip_known_suffixes(mut url: String) -> String {
    while url.ends_with('/') {
        url.pop();
    }
    if let Some(base) = url.strip_suffix("/chat/completions") {
        url.truncate(base.len());
    } else if let Some(base) = url.strip_suffix("/responses") {
        url.truncate(base.len());
    }
    url
}

/// Ensure the URL ends with the correct path for the given API mode.
///
/// Only called with explicit `Completions` / `Responses` (never `Auto`).
/// Avoids allocation when the URL is already correct.
fn normalize_api_url(url: String, mode: OpenAiApiMode) -> String {
    let mut base = strip_known_suffixes(url);
    let suffix = match mode {
        OpenAiApiMode::Completions | OpenAiApiMode::Auto => "/chat/completions",
        OpenAiApiMode::Responses => "/responses",
    };
    base.push_str(suffix);
    base
}

#[cfg(test)]
mod tests {
    use super::*;

    // --------------- Constructor & config ---------------

    #[test]
    fn test_openai_provider_new() {
        let p = OpenAIProvider::new("sk-test", "gpt-4o");
        assert!(p.is_configured());
        assert_eq!(p.model, "gpt-4o");
        assert_eq!(p.api_mode, OpenAiApiMode::Auto);
    }

    #[test]
    fn test_openai_provider_empty_key_not_configured() {
        assert!(!OpenAIProvider::new("", "gpt-4o").is_configured());
    }

    // --------------- Auto mode detection ---------------

    #[test]
    fn test_auto_mode_openai_resolves_to_responses() {
        let p = OpenAIProvider::new("sk-test", "gpt-4o");
        assert_eq!(p.resolved_mode(), ResolvedMode::Responses);
        assert_eq!(p.request_url(), "https://api.openai.com/responses");
    }

    #[test]
    fn test_auto_mode_custom_url_resolves_to_completions() {
        let p = OpenAIProvider::new("sk-test", "gpt-4o").with_api_url("https://api.xyz.com/llm/v1");
        assert_eq!(p.resolved_mode(), ResolvedMode::Completions);
        assert_eq!(
            p.request_url(),
            "https://api.xyz.com/llm/v1/chat/completions"
        );
    }

    #[test]
    fn test_auto_mode_localhost_resolves_to_completions() {
        let p = OpenAIProvider::new("sk-test", "gpt-4o").with_api_url("https://localhost:8000");
        assert_eq!(p.resolved_mode(), ResolvedMode::Completions);
        assert_eq!(p.request_url(), "https://localhost:8000/chat/completions");
    }

    #[test]
    fn test_auto_mode_strips_completions_suffix_stores_base() {
        let p = OpenAIProvider::new("sk-test", "gpt-4o")
            .with_api_url("https://api.xyz.com/v1/chat/completions");
        assert_eq!(p.api_url, "https://api.xyz.com/v1");
        assert_eq!(p.request_url(), "https://api.xyz.com/v1/chat/completions");
    }

    #[test]
    fn test_auto_mode_strips_responses_suffix_stores_base() {
        let p = OpenAIProvider::new("sk-test", "gpt-4o")
            .with_api_url("https://api.xyz.com/v1/responses");
        assert_eq!(p.api_url, "https://api.xyz.com/v1");
    }

    #[test]
    fn test_auto_mode_trailing_slash() {
        let p =
            OpenAIProvider::new("sk-test", "gpt-4o").with_api_url("https://api.xyz.com/llm/v1/");
        assert_eq!(p.api_url, "https://api.xyz.com/llm/v1");
    }

    // --------------- Explicit Completions mode ---------------

    #[test]
    fn test_explicit_completions_url() {
        let p = OpenAIProvider::new("sk-test", "gpt-4o").with_completions_api();
        assert_eq!(p.api_mode, OpenAiApiMode::Completions);
        assert!(p.request_url().ends_with("/chat/completions"));
    }

    #[test]
    fn test_explicit_completions_custom_url_base() {
        let p = OpenAIProvider::new("sk-test", "gpt-4o")
            .with_completions_api()
            .with_api_url("https://api.xyz.com/v1");
        assert_eq!(p.request_url(), "https://api.xyz.com/v1/chat/completions");
    }

    #[test]
    fn test_explicit_completions_custom_url_full() {
        let p = OpenAIProvider::new("sk-test", "gpt-4o")
            .with_completions_api()
            .with_api_url("https://api.xyz.com/v1/chat/completions");
        assert_eq!(p.request_url(), "https://api.xyz.com/v1/chat/completions");
    }

    // --------------- Explicit Responses mode ---------------

    #[test]
    fn test_explicit_responses_url() {
        let p = OpenAIProvider::new("sk-test", "gpt-4o").with_responses_api();
        assert_eq!(p.api_mode, OpenAiApiMode::Responses);
        assert!(p.request_url().ends_with("/responses"));
    }

    #[test]
    fn test_explicit_responses_custom_base() {
        let p = OpenAIProvider::new("sk-test", "gpt-4o")
            .with_responses_api()
            .with_api_url("https://custom.api.com/v1");
        assert_eq!(p.request_url(), "https://custom.api.com/v1/responses");
    }

    #[test]
    fn test_explicit_responses_full_url() {
        let p = OpenAIProvider::new("sk-test", "gpt-4o")
            .with_responses_api()
            .with_api_url("https://custom.api.com/v1/responses");
        assert_eq!(p.request_url(), "https://custom.api.com/v1/responses");
    }

    #[test]
    fn test_explicit_responses_switches_from_completions_url() {
        let p = OpenAIProvider::new("sk-test", "gpt-4o")
            .with_completions_api()
            .with_api_url("https://custom.api.com/v1/chat/completions")
            .with_responses_api();
        assert_eq!(p.request_url(), "https://custom.api.com/v1/responses");
    }

    // --------------- Provider name ---------------

    #[test]
    fn test_provider_name_auto_openai() {
        let p = OpenAIProvider::new("sk-test", "gpt-4o");
        assert_eq!(p.provider_name(), "openai_responses");
    }

    #[test]
    fn test_provider_name_auto_custom() {
        let p = OpenAIProvider::new("sk-test", "gpt-4o").with_api_url("https://vllm.local");
        assert_eq!(p.provider_name(), "openai");
    }

    #[test]
    fn test_provider_name_explicit_responses() {
        let p = OpenAIProvider::new("sk-test", "gpt-4o").with_responses_api();
        assert_eq!(p.provider_name(), "openai_responses");
    }

    #[test]
    fn test_provider_name_explicit_completions() {
        let p = OpenAIProvider::new("sk-test", "gpt-4o").with_completions_api();
        assert_eq!(p.provider_name(), "openai");
    }

    // --------------- normalize / strip helpers ---------------

    #[test]
    fn test_normalize_completions_bare_host() {
        assert_eq!(
            normalize_api_url("https://localhost:8000".into(), OpenAiApiMode::Completions),
            "https://localhost:8000/chat/completions"
        );
    }

    #[test]
    fn test_normalize_responses_bare_host() {
        assert_eq!(
            normalize_api_url("https://localhost:8000".into(), OpenAiApiMode::Responses),
            "https://localhost:8000/responses"
        );
    }

    #[test]
    fn test_normalize_strips_completions_for_responses() {
        assert_eq!(
            normalize_api_url(
                "https://api.example.com/v1/chat/completions".into(),
                OpenAiApiMode::Responses
            ),
            "https://api.example.com/v1/responses"
        );
    }

    #[test]
    fn test_normalize_strips_responses_for_completions() {
        assert_eq!(
            normalize_api_url(
                "https://api.example.com/v1/responses".into(),
                OpenAiApiMode::Completions
            ),
            "https://api.example.com/v1/chat/completions"
        );
    }

    #[test]
    fn test_strip_known_suffixes_completions() {
        assert_eq!(
            strip_known_suffixes("https://api.example.com/v1/chat/completions".into()),
            "https://api.example.com/v1"
        );
    }

    #[test]
    fn test_strip_known_suffixes_responses() {
        assert_eq!(
            strip_known_suffixes("https://api.example.com/v1/responses".into()),
            "https://api.example.com/v1"
        );
    }

    #[test]
    fn test_strip_known_suffixes_no_suffix() {
        assert_eq!(
            strip_known_suffixes("https://api.example.com/v1".into()),
            "https://api.example.com/v1"
        );
    }

    #[test]
    fn test_is_openai_url_true() {
        assert!(is_openai_url("https://api.openai.com/v1/responses"));
        assert!(is_openai_url("https://api.openai.com"));
    }

    #[test]
    fn test_is_openai_url_false() {
        assert!(!is_openai_url("https://api.xyz.com/v1"));
        assert!(!is_openai_url("https://localhost:8000"));
        assert!(!is_openai_url("https://openai.example.com"));
    }

    // --------------- Request body builders ---------------

    #[test]
    fn test_build_responses_body_system_becomes_instructions() {
        let messages = vec![Message::system("You are helpful."), Message::user("Hello")];
        let body = build_responses_body("gpt-4o", &messages, &CompletionOptions::default());
        assert_eq!(body["instructions"], "You are helpful.");
        assert!(body.get("messages").is_none());
        let input = body["input"].as_array().expect("input should be array");
        assert_eq!(input.len(), 1);
        assert_eq!(input[0]["role"], "user");
    }

    #[test]
    fn test_build_responses_body_no_system() {
        let messages = vec![Message::user("Hello")];
        let body = build_responses_body("gpt-4o", &messages, &CompletionOptions::default());
        assert!(body.get("instructions").is_none());
        assert_eq!(body["input"].as_array().expect("array").len(), 1);
    }

    #[test]
    fn test_build_responses_body_json_mode() {
        let mut opts = CompletionOptions::default();
        opts.json_mode = true;
        let body = build_responses_body("gpt-4o", &[Message::user("Hi")], &opts);
        assert_eq!(body["text"]["format"]["type"], "json_object");
    }

    #[test]
    fn test_build_completions_body_json_mode() {
        let mut opts = CompletionOptions::default();
        opts.json_mode = true;
        let body = build_completions_body("gpt-4o", &[Message::user("Hi")], &opts);
        assert_eq!(body["response_format"]["type"], "json_object");
    }

    // --------------- Response parsers ---------------

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
            "output": [{ "type": "message", "content": [{ "type": "output_text", "text": "Extracted" }] }]
        });
        let resp = parse_responses_response(json).expect("should parse");
        assert_eq!(resp.content, "Extracted");
    }

    #[test]
    fn test_parse_responses_missing_output() {
        let err = parse_responses_response(serde_json::json!({})).unwrap_err();
        assert!(format!("{}", err).contains("Missing field"));
    }

    // --------------- Usage extraction ---------------

    #[test]
    fn test_extract_usage_completions_keys() {
        let json = serde_json::json!({
            "usage": { "prompt_tokens": 10, "completion_tokens": 20, "total_tokens": 30 }
        });
        let u = extract_usage(&json);
        assert_eq!(
            (u.prompt_tokens, u.completion_tokens, u.total_tokens),
            (10, 20, 30)
        );
    }

    #[test]
    fn test_extract_usage_responses_keys() {
        let json = serde_json::json!({
            "usage": { "input_tokens": 10, "output_tokens": 20, "total_tokens": 30 }
        });
        let u = extract_usage(&json);
        assert_eq!((u.prompt_tokens, u.completion_tokens), (10, 20));
    }

    #[test]
    fn test_extract_usage_missing() {
        let u = extract_usage(&serde_json::json!({}));
        assert_eq!(
            (u.prompt_tokens, u.completion_tokens, u.total_tokens),
            (0, 0, 0)
        );
    }
}
