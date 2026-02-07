//! Brave Search API provider implementation.
//!
//! Brave provides privacy-focused web search with good quality results.

use super::{SearchError, SearchProvider, SearchResult, SearchResults};
use crate::config::SearchOptions;
use async_trait::async_trait;

/// Default Brave Search API endpoint.
const DEFAULT_API_URL: &str = "https://api.search.brave.com/res/v1/web/search";

/// Brave Search API provider.
///
/// Provides access to Brave's privacy-focused web search API.
///
/// # Example
/// ```ignore
/// use spider_agent::search::BraveProvider;
/// use spider_agent::config::SearchOptions;
///
/// let provider = BraveProvider::new("your-api-key");
/// let client = reqwest::Client::new();
/// let results = provider.search("rust web crawler", &SearchOptions::default(), &client).await?;
/// ```
#[derive(Debug, Clone)]
pub struct BraveProvider {
    api_key: String,
    api_url: Option<String>,
}

impl BraveProvider {
    /// Create a new Brave provider with the given API key.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            api_url: None,
        }
    }

    /// Use a custom API endpoint.
    pub fn with_api_url(mut self, url: impl Into<String>) -> Self {
        self.api_url = Some(url.into());
        self
    }

    /// Get the API endpoint URL.
    fn endpoint(&self) -> &str {
        self.api_url.as_deref().unwrap_or(DEFAULT_API_URL)
    }
}

#[async_trait]
impl SearchProvider for BraveProvider {
    async fn search(
        &self,
        query: &str,
        options: &SearchOptions,
        client: &reqwest::Client,
    ) -> Result<SearchResults, SearchError> {
        // Build query parameters
        let mut params = vec![("q", query.to_string())];

        if let Some(limit) = options.limit {
            params.push(("count", limit.min(20).to_string()));
        }

        if let Some(ref country) = options.country {
            params.push(("country", country.clone()));
        }

        if let Some(ref language) = options.language {
            params.push(("search_lang", language.clone()));
        }

        let response = client
            .get(self.endpoint())
            .header("X-Subscription-Token", &self.api_key)
            .header("Accept", "application/json")
            .query(&params)
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
                "HTTP {} from Brave API",
                status
            )));
        }

        // Parse response
        let json: serde_json::Value = response
            .json()
            .await
            .map_err(|e| SearchError::ProviderError(format!("Failed to parse response: {}", e)))?;

        // Extract web results
        let mut results = SearchResults::new(query);

        if let Some(web) = json
            .get("web")
            .and_then(|v| v.get("results"))
            .and_then(|v| v.as_array())
        {
            for (i, item) in web.iter().enumerate() {
                let title = item
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let url = item.get("url").and_then(|v| v.as_str()).unwrap_or_default();

                if url.is_empty() {
                    continue;
                }

                let mut result = SearchResult::new(title, url, i + 1);

                if let Some(snippet) = item.get("description").and_then(|v| v.as_str()) {
                    result = result.with_snippet(snippet);
                }

                if let Some(age) = item.get("age").and_then(|v| v.as_str()) {
                    result = result.with_date(age);
                }

                results.push(result);
            }
        }

        // Store raw metadata
        results.metadata = Some(json);

        Ok(results)
    }

    fn provider_name(&self) -> &'static str {
        "brave"
    }

    fn is_configured(&self) -> bool {
        !self.api_key.is_empty() || self.api_url.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_brave_provider_new() {
        let provider = BraveProvider::new("test-key");
        assert_eq!(provider.endpoint(), DEFAULT_API_URL);
        assert!(provider.is_configured());
    }

    #[test]
    fn test_brave_provider_custom_url() {
        let provider = BraveProvider::new("test-key").with_api_url("https://custom.api.com/search");
        assert_eq!(provider.endpoint(), "https://custom.api.com/search");
    }
}
