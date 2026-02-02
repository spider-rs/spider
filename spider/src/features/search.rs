//! Web search integration for Spider.
//!
//! This module provides search provider abstractions and implementations
//! for integrating web search APIs (Serper, Brave, Bing, Tavily) with Spider's
//! crawling and automation capabilities.

use std::fmt;

/// Search provider trait for abstracting different search APIs.
pub trait SearchProvider: Send + Sync {
    /// Execute a search query and return results.
    ///
    /// The client parameter allows reusing an existing HTTP client,
    /// or passing `None` to use the provider's default client.
    /// Note: Uses `reqwest::Client` directly since search APIs don't need caching middleware.
    fn search(
        &self,
        query: &str,
        options: &SearchOptions,
        client: Option<&reqwest::Client>,
    ) -> impl std::future::Future<Output = Result<SearchResults, SearchError>> + Send;

    /// Provider name for logging/debugging.
    fn provider_name(&self) -> &'static str;
}

/// Search options controlling result count, filters, etc.
#[derive(Debug, Clone, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SearchOptions {
    /// Maximum number of results to return.
    pub limit: Option<usize>,
    /// Country/region code (e.g., "us", "uk").
    pub country: Option<String>,
    /// Language code (e.g., "en", "es").
    pub language: Option<String>,
    /// Filter to specific domains.
    pub site_filter: Option<Vec<String>>,
    /// Exclude specific domains.
    pub exclude_domains: Option<Vec<String>>,
    /// Time range filter.
    pub time_range: Option<TimeRange>,
    /// Include only pages with these words.
    pub include_keywords: Option<Vec<String>>,
}

impl SearchOptions {
    /// Create new search options with default values.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set maximum number of results.
    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Set country/region code.
    pub fn with_country(mut self, country: impl Into<String>) -> Self {
        self.country = Some(country.into());
        self
    }

    /// Set language code.
    pub fn with_language(mut self, language: impl Into<String>) -> Self {
        self.language = Some(language.into());
        self
    }

    /// Filter results to specific domains.
    pub fn with_site_filter(mut self, domains: Vec<String>) -> Self {
        self.site_filter = Some(domains);
        self
    }

    /// Exclude specific domains from results.
    pub fn with_exclude_domains(mut self, domains: Vec<String>) -> Self {
        self.exclude_domains = Some(domains);
        self
    }

    /// Set time range filter.
    pub fn with_time_range(mut self, range: TimeRange) -> Self {
        self.time_range = Some(range);
        self
    }
}

/// Time range for filtering search results.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum TimeRange {
    /// Results from the past day.
    Day,
    /// Results from the past week.
    Week,
    /// Results from the past month.
    Month,
    /// Results from the past year.
    Year,
    /// Custom date range.
    Custom {
        /// Start date (format depends on provider).
        start: String,
        /// End date (format depends on provider).
        end: String,
    },
}

/// Unified search result format.
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SearchResults {
    /// The original query.
    pub query: String,
    /// List of organic results.
    pub results: Vec<SearchResult>,
    /// Total result count (if available from provider).
    pub total_results: Option<u64>,
    /// Provider-specific metadata.
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
    pub metadata: Option<serde_json::Value>,
}

impl SearchResults {
    /// Create new search results.
    pub fn new(query: impl Into<String>) -> Self {
        Self {
            query: query.into(),
            results: Vec::new(),
            total_results: None,
            metadata: None,
        }
    }

    /// Add a result.
    pub fn push(&mut self, result: SearchResult) {
        self.results.push(result);
    }

    /// Get URLs from results.
    pub fn urls(&self) -> Vec<&str> {
        self.results.iter().map(|r| r.url.as_str()).collect()
    }

    /// Check if results are empty.
    pub fn is_empty(&self) -> bool {
        self.results.is_empty()
    }

    /// Get number of results.
    pub fn len(&self) -> usize {
        self.results.len()
    }
}

/// Individual search result.
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SearchResult {
    /// Result title.
    pub title: String,
    /// Result URL.
    pub url: String,
    /// Snippet/description.
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
    pub snippet: Option<String>,
    /// Publication/index date.
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
    pub date: Option<String>,
    /// Result position (1-indexed).
    pub position: usize,
    /// Relevance score (provider-dependent).
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
    pub score: Option<f32>,
}

impl SearchResult {
    /// Create a new search result.
    pub fn new(title: impl Into<String>, url: impl Into<String>, position: usize) -> Self {
        Self {
            title: title.into(),
            url: url.into(),
            snippet: None,
            date: None,
            position,
            score: None,
        }
    }

    /// Set snippet/description.
    pub fn with_snippet(mut self, snippet: impl Into<String>) -> Self {
        self.snippet = Some(snippet.into());
        self
    }

    /// Set date.
    pub fn with_date(mut self, date: impl Into<String>) -> Self {
        self.date = Some(date.into());
        self
    }

    /// Set relevance score.
    pub fn with_score(mut self, score: f32) -> Self {
        self.score = Some(score);
        self
    }
}

/// Search error types.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_search_options_builder() {
        let opts = SearchOptions::new()
            .with_limit(10)
            .with_country("us")
            .with_language("en");

        assert_eq!(opts.limit, Some(10));
        assert_eq!(opts.country.as_deref(), Some("us"));
        assert_eq!(opts.language.as_deref(), Some("en"));
    }

    #[test]
    fn test_search_result_builder() {
        let result = SearchResult::new("Test Title", "https://example.com", 1)
            .with_snippet("Test snippet")
            .with_score(0.95);

        assert_eq!(result.title, "Test Title");
        assert_eq!(result.url, "https://example.com");
        assert_eq!(result.position, 1);
        assert_eq!(result.snippet.as_deref(), Some("Test snippet"));
        assert_eq!(result.score, Some(0.95));
    }

    #[test]
    fn test_search_results() {
        let mut results = SearchResults::new("test query");
        results.push(SearchResult::new("Title 1", "https://example1.com", 1));
        results.push(SearchResult::new("Title 2", "https://example2.com", 2));

        assert_eq!(results.len(), 2);
        assert!(!results.is_empty());
        assert_eq!(
            results.urls(),
            vec!["https://example1.com", "https://example2.com"]
        );
    }
}
