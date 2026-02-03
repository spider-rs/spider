//! Error types for spider_agent.

use std::fmt;

/// Agent error types.
#[derive(Debug)]
pub enum AgentError {
    /// HTTP request failed.
    Http(reqwest::Error),
    /// JSON serialization/deserialization error.
    Json(serde_json::Error),
    /// Missing required field in response.
    MissingField(&'static str),
    /// Invalid field type in response.
    InvalidField(&'static str),
    /// Remote API error.
    Remote(String),
    /// Feature not enabled or configured.
    NotConfigured(&'static str),
    /// Search error.
    Search(SearchError),
    /// LLM provider error.
    Llm(String),
    /// Browser automation error.
    #[cfg(feature = "chrome")]
    Browser(String),
    /// Tool execution error.
    Tool(String),
    /// Rate limit exceeded.
    RateLimited,
    /// Timeout.
    Timeout,
}

impl fmt::Display for AgentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http(e) => write!(f, "HTTP error: {}", e),
            Self::Json(e) => write!(f, "JSON error: {}", e),
            Self::MissingField(field) => write!(f, "Missing field: {}", field),
            Self::InvalidField(field) => write!(f, "Invalid field: {}", field),
            Self::Remote(msg) => write!(f, "Remote error: {}", msg),
            Self::NotConfigured(what) => write!(f, "Not configured: {}", what),
            Self::Search(e) => write!(f, "Search error: {}", e),
            Self::Llm(msg) => write!(f, "LLM error: {}", msg),
            #[cfg(feature = "chrome")]
            Self::Browser(msg) => write!(f, "Browser error: {}", msg),
            Self::Tool(msg) => write!(f, "Tool error: {}", msg),
            Self::RateLimited => write!(f, "Rate limit exceeded"),
            Self::Timeout => write!(f, "Request timed out"),
        }
    }
}

impl std::error::Error for AgentError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Http(e) => Some(e),
            Self::Json(e) => Some(e),
            Self::Search(e) => Some(e),
            _ => None,
        }
    }
}

impl From<reqwest::Error> for AgentError {
    fn from(e: reqwest::Error) -> Self {
        if e.is_timeout() {
            Self::Timeout
        } else {
            Self::Http(e)
        }
    }
}

impl From<serde_json::Error> for AgentError {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}

impl From<SearchError> for AgentError {
    fn from(e: SearchError) -> Self {
        Self::Search(e)
    }
}

/// Search-specific error types.
#[derive(Debug)]
pub enum SearchError {
    /// API request failed.
    RequestFailed(String),
    /// Invalid API key or authentication.
    AuthenticationFailed,
    /// Rate limit exceeded.
    RateLimited,
    /// Invalid query or parameters.
    InvalidQuery(String),
    /// Provider-specific error.
    ProviderError(String),
    /// No search provider configured.
    NoProvider,
}

impl fmt::Display for SearchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RequestFailed(msg) => write!(f, "Search request failed: {}", msg),
            Self::AuthenticationFailed => write!(f, "Search authentication failed"),
            Self::RateLimited => write!(f, "Search rate limit exceeded"),
            Self::InvalidQuery(msg) => write!(f, "Invalid search query: {}", msg),
            Self::ProviderError(msg) => write!(f, "Search provider error: {}", msg),
            Self::NoProvider => write!(f, "No search provider configured"),
        }
    }
}

impl std::error::Error for SearchError {}

/// Result type for agent operations.
pub type AgentResult<T> = Result<T, AgentError>;
