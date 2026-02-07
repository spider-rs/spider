//! Tavily AI Search API provider implementation.
//!
//! Tavily provides AI-optimized search designed for LLM applications.

use super::{SearchError, SearchProvider, SearchResult, SearchResults};
use crate::config::SearchOptions;
use async_trait::async_trait;

/// Default Tavily API endpoint.
const DEFAULT_API_URL: &str = "https://api.tavily.com/search";

/// Tavily AI Search API provider.
///
/// Provides access to Tavily's AI-optimized search API.
///
/// # Example
/// ```ignore
/// use spider_agent::search::TavilyProvider;
/// use spider_agent::config::SearchOptions;
///
/// let provider = TavilyProvider::new("your-api-key");
/// let client = reqwest::Client::new();
/// let results = provider.search("rust web crawler", &SearchOptions::default(), &client).await?;
/// ```
#[derive(Debug, Clone)]
pub struct TavilyProvider {
    api_key: String,
    api_url: Option<String>,
    /// Search depth: "basic" or "advanced".
    search_depth: String,
}

impl TavilyProvider {
    /// Create a new Tavily provider with the given API key.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            api_url: None,
            search_depth: "basic".to_string(),
        }
    }

    /// Use a custom API endpoint.
    pub fn with_api_url(mut self, url: impl Into<String>) -> Self {
        self.api_url = Some(url.into());
        self
    }

    /// Set search depth to "advanced" for more thorough results.
    pub fn with_advanced_search(mut self) -> Self {
        self.search_depth = "advanced".to_string();
        self
    }

    /// Get the API endpoint URL.
    fn endpoint(&self) -> &str {
        self.api_url.as_deref().unwrap_or(DEFAULT_API_URL)
    }
}

#[async_trait]
impl SearchProvider for TavilyProvider {
    async fn search(
        &self,
        query: &str,
        options: &SearchOptions,
        client: &reqwest::Client,
    ) -> Result<SearchResults, SearchError> {
        // Build request body
        let mut body = serde_json::json!({
            "api_key": &self.api_key,
            "query": query,
            "search_depth": &self.search_depth
        });

        if let Some(limit) = options.limit {
            body["max_results"] = serde_json::json!(limit.min(10));
        }

        // Tavily supports include/exclude domains
        if let Some(ref sites) = options.site_filter {
            body["include_domains"] = serde_json::json!(sites);
        }

        if let Some(ref exclude) = options.exclude_domains {
            body["exclude_domains"] = serde_json::json!(exclude);
        }

        let response = client
            .post(self.endpoint())
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| SearchError::RequestFailed(e.to_string()))?;

        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(SearchError::AuthenticationFailed);
        }
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(SearchError::RateLimited);
        }
        if !status.is_success() {
            return Err(SearchError::ProviderError(format!(
                "HTTP {} from Tavily API",
                status
            )));
        }

        // Parse response
        let json: serde_json::Value = response
            .json()
            .await
            .map_err(|e| SearchError::ProviderError(format!("Failed to parse response: {}", e)))?;

        // Extract results
        let mut results = SearchResults::new(query);

        if let Some(items) = json.get("results").and_then(|v| v.as_array()) {
            for (i, item) in items.iter().enumerate() {
                let title = item
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let url = item.get("url").and_then(|v| v.as_str()).unwrap_or_default();

                if url.is_empty() {
                    continue;
                }

                let mut result = SearchResult::new(title, url, i + 1);

                if let Some(snippet) = item.get("content").and_then(|v| v.as_str()) {
                    result = result.with_snippet(snippet);
                }

                // Tavily provides a relevance score
                if let Some(score) = item.get("score").and_then(|v| v.as_f64()) {
                    result = result.with_score(score as f32);
                }

                if let Some(date) = item.get("published_date").and_then(|v| v.as_str()) {
                    result = result.with_date(date);
                }

                results.push(result);
            }
        }

        // Store raw metadata (includes Tavily's AI-generated answer if available)
        results.metadata = Some(json);

        Ok(results)
    }

    fn provider_name(&self) -> &'static str {
        "tavily"
    }

    fn is_configured(&self) -> bool {
        !self.api_key.is_empty() || self.api_url.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tavily_provider_new() {
        let provider = TavilyProvider::new("test-key");
        assert_eq!(provider.endpoint(), DEFAULT_API_URL);
        assert_eq!(provider.search_depth, "basic");
        assert!(provider.is_configured());
    }

    #[test]
    fn test_tavily_provider_advanced() {
        let provider = TavilyProvider::new("test-key").with_advanced_search();
        assert_eq!(provider.search_depth, "advanced");
    }

    #[test]
    fn test_tavily_provider_custom_url() {
        let provider =
            TavilyProvider::new("test-key").with_api_url("https://custom.api.com/search");
        assert_eq!(provider.endpoint(), "https://custom.api.com/search");
    }
}
