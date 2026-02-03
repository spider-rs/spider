//! LLM provider abstractions for spider_agent.

#[cfg(feature = "openai")]
mod openai;

#[cfg(feature = "openai")]
pub use openai::OpenAIProvider;

use crate::error::AgentResult;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// LLM provider trait for abstracting different LLM APIs.
#[async_trait]
pub trait LLMProvider: Send + Sync {
    /// Send a completion request and return the response text.
    async fn complete(
        &self,
        messages: Vec<Message>,
        options: &CompletionOptions,
        client: &reqwest::Client,
    ) -> AgentResult<CompletionResponse>;

    /// Provider name for logging/debugging.
    fn provider_name(&self) -> &'static str;

    /// Check if the provider is properly configured.
    fn is_configured(&self) -> bool;
}

/// A message in a conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    /// Role: "system", "user", or "assistant".
    pub role: String,
    /// Message content.
    pub content: MessageContent,
}

impl Message {
    /// Create a system message.
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".to_string(),
            content: MessageContent::Text(content.into()),
        }
    }

    /// Create a user message.
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".to_string(),
            content: MessageContent::Text(content.into()),
        }
    }

    /// Create an assistant message.
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: "assistant".to_string(),
            content: MessageContent::Text(content.into()),
        }
    }

    /// Create a user message with an image.
    pub fn user_with_image(text: impl Into<String>, image_base64: impl Into<String>) -> Self {
        Self {
            role: "user".to_string(),
            content: MessageContent::MultiPart(vec![
                ContentPart::Text {
                    text: text.into(),
                },
                ContentPart::ImageUrl {
                    image_url: ImageUrl {
                        url: format!("data:image/png;base64,{}", image_base64.into()),
                    },
                },
            ]),
        }
    }
}

/// Message content - either text or multi-part.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    /// Plain text content.
    Text(String),
    /// Multi-part content (text + images).
    MultiPart(Vec<ContentPart>),
}

/// A part of multi-part content.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentPart {
    /// Text part.
    Text { text: String },
    /// Image URL part.
    ImageUrl { image_url: ImageUrl },
}

/// Image URL for vision models.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageUrl {
    /// URL (can be data URL with base64).
    pub url: String,
}

/// Options for completion requests.
#[derive(Debug, Clone)]
pub struct CompletionOptions {
    /// Temperature (0.0 - 2.0).
    pub temperature: f32,
    /// Max tokens to generate.
    pub max_tokens: u16,
    /// Request JSON output.
    pub json_mode: bool,
}

impl Default for CompletionOptions {
    fn default() -> Self {
        Self {
            temperature: 0.1,
            max_tokens: 4096,
            json_mode: true,
        }
    }
}

/// Response from a completion request.
#[derive(Debug, Clone)]
pub struct CompletionResponse {
    /// The generated text.
    pub content: String,
    /// Token usage.
    pub usage: TokenUsage,
}

/// Token usage from a completion.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    /// Prompt tokens.
    pub prompt_tokens: u32,
    /// Completion tokens.
    pub completion_tokens: u32,
    /// Total tokens.
    pub total_tokens: u32,
}

impl TokenUsage {
    /// Accumulate usage from another.
    pub fn accumulate(&mut self, other: &TokenUsage) {
        self.prompt_tokens += other.prompt_tokens;
        self.completion_tokens += other.completion_tokens;
        self.total_tokens += other.total_tokens;
    }
}
