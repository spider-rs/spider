//! Web search integration for spider_agent.
//!
//! This module provides search provider abstractions and implementations
//! for integrating web search APIs (Serper, Brave, Bing, Tavily).

#[cfg(feature = "search_bing")]
mod bing;
#[cfg(feature = "search_brave")]
mod brave;
#[cfg(feature = "search_serper")]
mod serper;
#[cfg(feature = "search_tavily")]
mod tavily;

#[cfg(feature = "search_bing")]
pub use bing::BingProvider;
#[cfg(feature = "search_brave")]
pub use brave::BraveProvider;
#[cfg(feature = "search_serper")]
pub use serper::SerperProvider;
#[cfg(feature = "search_tavily")]
pub use tavily::TavilyProvider;

use crate::config::SearchOptions;
use crate::error::SearchError;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Search provider trait for abstracting different search APIs.
#[async_trait]
pub trait SearchProvider: Send + Sync {
    /// Execute a search query and return results.
    async fn search(
        &self,
        query: &str,
        options: &SearchOptions,
        client: &reqwest::Client,
    ) -> Result<SearchResults, SearchError>;

    /// Provider name for logging/debugging.
    fn provider_name(&self) -> &'static str;

    /// Check if the provider is properly configured.
    fn is_configured(&self) -> bool;
}

/// Unified search result format.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SearchResults {
    /// The original query.
    pub query: String,
    /// List of organic results.
    pub results: Vec<SearchResult>,
    /// Total result count (if available from provider).
    pub total_results: Option<u64>,
    /// Provider-specific metadata.
    #[serde(skip_serializing_if = "Option::is_none")]
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
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SearchResult {
    /// Result title.
    pub title: String,
    /// Result URL.
    pub url: String,
    /// Snippet/description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
    /// Publication/index date.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    /// Result position (1-indexed).
    pub position: usize,
    /// Relevance score (provider-dependent).
    #[serde(skip_serializing_if = "Option::is_none")]
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
