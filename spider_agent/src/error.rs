//! Error types for spider_agent.

use crate::config::LimitType;
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
    /// WebDriver automation error.
    #[cfg(feature = "webdriver")]
    WebDriver(String),
    /// IO error (file operations).
    #[cfg(feature = "fs")]
    Io(std::io::Error),
    /// Tool execution error.
    Tool(String),
    /// Rate limit exceeded.
    RateLimited,
    /// Timeout.
    Timeout,
    /// Usage limit exceeded.
    LimitExceeded(LimitType),
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
            #[cfg(feature = "webdriver")]
            Self::WebDriver(msg) => write!(f, "WebDriver error: {}", msg),
            #[cfg(feature = "fs")]
            Self::Io(e) => write!(f, "IO error: {}", e),
            Self::Tool(msg) => write!(f, "Tool error: {}", msg),
            Self::RateLimited => write!(f, "Rate limit exceeded"),
            Self::Timeout => write!(f, "Request timed out"),
            Self::LimitExceeded(limit) => write!(f, "Usage limit exceeded: {}", limit),
        }
    }
}

impl std::error::Error for AgentError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Http(e) => Some(e),
            Self::Json(e) => Some(e),
            Self::Search(e) => Some(e),
            #[cfg(feature = "fs")]
            Self::Io(e) => Some(e),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_error_display_variants() {
        let err = AgentError::MissingField("content");
        assert_eq!(format!("{}", err), "Missing field: content");

        let err = AgentError::InvalidField("type");
        assert_eq!(format!("{}", err), "Invalid field: type");

        let err = AgentError::Remote("server down".into());
        assert_eq!(format!("{}", err), "Remote error: server down");

        let err = AgentError::NotConfigured("api_key");
        assert_eq!(format!("{}", err), "Not configured: api_key");

        let err = AgentError::Llm("model not found".into());
        assert_eq!(format!("{}", err), "LLM error: model not found");

        let err = AgentError::Tool("execution failed".into());
        assert_eq!(format!("{}", err), "Tool error: execution failed");

        let err = AgentError::RateLimited;
        assert_eq!(format!("{}", err), "Rate limit exceeded");

        let err = AgentError::Timeout;
        assert_eq!(format!("{}", err), "Request timed out");

        let err = AgentError::LimitExceeded(LimitType::TotalTokens {
            used: 100,
            limit: 50,
        });
        assert!(format!("{}", err).contains("Usage limit exceeded"));
    }

    #[test]
    fn test_search_error_display_variants() {
        assert_eq!(
            format!("{}", SearchError::RequestFailed("timeout".into())),
            "Search request failed: timeout"
        );
        assert_eq!(
            format!("{}", SearchError::AuthenticationFailed),
            "Search authentication failed"
        );
        assert_eq!(
            format!("{}", SearchError::RateLimited),
            "Search rate limit exceeded"
        );
        assert_eq!(
            format!("{}", SearchError::InvalidQuery("empty".into())),
            "Invalid search query: empty"
        );
        assert_eq!(
            format!("{}", SearchError::ProviderError("api error".into())),
            "Search provider error: api error"
        );
        assert_eq!(
            format!("{}", SearchError::NoProvider),
            "No search provider configured"
        );
    }

    #[test]
    fn test_from_serde_json_error() {
        let json_err = serde_json::from_str::<serde_json::Value>("invalid").unwrap_err();
        let agent_err: AgentError = json_err.into();
        assert!(format!("{}", agent_err).starts_with("JSON error:"));
    }

    #[test]
    fn test_from_search_error() {
        let search_err = SearchError::NoProvider;
        let agent_err: AgentError = search_err.into();
        assert!(format!("{}", agent_err).contains("Search error:"));
    }

    #[test]
    fn test_error_source_chain() {
        use std::error::Error;

        let json_err = serde_json::from_str::<serde_json::Value>("invalid").unwrap_err();
        let agent_err = AgentError::Json(json_err);
        assert!(agent_err.source().is_some());

        let search_err = AgentError::Search(SearchError::NoProvider);
        assert!(search_err.source().is_some());

        let remote_err = AgentError::Remote("test".into());
        assert!(remote_err.source().is_none());

        let timeout_err = AgentError::Timeout;
        assert!(timeout_err.source().is_none());
    }
}
