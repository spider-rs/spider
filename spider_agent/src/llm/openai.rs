//! OpenAI-compatible LLM provider implementation.

use super::{CompletionOptions, CompletionResponse, LLMProvider, Message, TokenUsage};
use crate::error::{AgentError, AgentResult};
use async_trait::async_trait;

/// Default OpenAI API endpoint.
const DEFAULT_API_URL: &str = "https://api.openai.com/v1/chat/completions";

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
}

impl OpenAIProvider {
    /// Create a new OpenAI provider.
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            api_url: DEFAULT_API_URL.to_string(),
            model: model.into(),
        }
    }

    /// Use a custom API endpoint (for compatible APIs).
    pub fn with_api_url(mut self, url: impl Into<String>) -> Self {
        self.api_url = url.into();
        self
    }

    /// Change the model.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
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
        // Build request body
        let mut body = serde_json::json!({
            "model": &self.model,
            "messages": messages,
            "temperature": options.temperature,
            "max_tokens": options.max_tokens,
        });

        if options.json_mode {
            body["response_format"] = serde_json::json!({ "type": "json_object" });
        }

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

        // Parse response
        let json: serde_json::Value = response.json().await?;

        // Extract content
        let content = json
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("message"))
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .ok_or_else(|| AgentError::MissingField("choices[0].message.content"))?
            .to_string();

        // Extract usage
        let usage = if let Some(u) = json.get("usage") {
            TokenUsage {
                prompt_tokens: u.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                completion_tokens: u
                    .get("completion_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u32,
                total_tokens: u.get("total_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
            }
        } else {
            TokenUsage::default()
        };

        Ok(CompletionResponse { content, usage })
    }

    fn provider_name(&self) -> &'static str {
        "openai"
    }

    fn is_configured(&self) -> bool {
        !self.api_key.is_empty()
    }
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
    fn test_openai_provider_custom_url() {
        let provider = OpenAIProvider::new("sk-test", "gpt-4o")
            .with_api_url("https://custom.api.com/v1/chat/completions");
        assert_eq!(
            provider.api_url,
            "https://custom.api.com/v1/chat/completions"
        );
    }
}
