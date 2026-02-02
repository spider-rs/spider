//! Bing Web Search API provider implementation.
//!
//! Microsoft Bing provides web search via Azure Cognitive Services.

use super::{SearchError, SearchOptions, SearchProvider, SearchResult, SearchResults};

/// Default Bing Search API endpoint.
const DEFAULT_API_URL: &str = "https://api.bing.microsoft.com/v7.0/search";

/// Bing Web Search API provider.
///
/// Provides access to Microsoft Bing web search via Azure Cognitive Services.
///
/// # Example
/// ```ignore
/// use spider::features::search_providers::BingProvider;
/// use spider::features::search::{SearchOptions, SearchProvider};
///
/// let provider = BingProvider::new("your-api-key");
/// let results = provider.search("rust web crawler", &SearchOptions::default(), None).await?;
/// ```
#[derive(Debug, Clone)]
pub struct BingProvider {
    api_key: String,
    api_url: Option<String>,
}

impl BingProvider {
    /// Create a new Bing provider with the given API key.
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

impl SearchProvider for BingProvider {
    async fn search(
        &self,
        query: &str,
        options: &SearchOptions,
        client: Option<&crate::Client>,
    ) -> Result<SearchResults, SearchError> {
        // Build query parameters
        let mut params = vec![("q", query.to_string())];

        if let Some(limit) = options.limit {
            params.push(("count", limit.min(50).to_string()));
        }

        if let Some(ref country) = options.country {
            params.push(("cc", country.clone()));
        }

        if let Some(ref language) = options.language {
            params.push(("setLang", language.clone()));
        }

        // Use provided client or create a new one
        let response = if let Some(c) = client {
            c.get(self.endpoint())
                .header("Ocp-Apim-Subscription-Key", &self.api_key)
                .query(&params)
                .send()
                .await
        } else {
            let c = crate::ClientBuilder::new()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .map_err(|e| SearchError::RequestFailed(e.to_string()))?;

            c.get(self.endpoint())
                .header("Ocp-Apim-Subscription-Key", &self.api_key)
                .query(&params)
                .send()
                .await
        };

        let response = response.map_err(|e| SearchError::RequestFailed(e.to_string()))?;

        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED
            || status == reqwest::StatusCode::FORBIDDEN
        {
            return Err(SearchError::AuthenticationFailed);
        }
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(SearchError::RateLimited);
        }
        if !status.is_success() {
            return Err(SearchError::ProviderError(format!(
                "HTTP {} from Bing API",
                status
            )));
        }

        // Parse response
        let json: serde_json::Value = response
            .json()
            .await
            .map_err(|e| SearchError::ProviderError(format!("Failed to parse response: {}", e)))?;

        // Extract web pages
        let mut results = SearchResults::new(query);

        if let Some(pages) = json
            .get("webPages")
            .and_then(|v| v.get("value"))
            .and_then(|v| v.as_array())
        {
            for (i, item) in pages.iter().enumerate() {
                let title = item
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let url = item
                    .get("url")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();

                if url.is_empty() {
                    continue;
                }

                let mut result = SearchResult::new(title, url, i + 1);

                if let Some(snippet) = item.get("snippet").and_then(|v| v.as_str()) {
                    result = result.with_snippet(snippet);
                }

                if let Some(date) = item.get("dateLastCrawled").and_then(|v| v.as_str()) {
                    result = result.with_date(date);
                }

                results.push(result);
            }
        }

        // Extract total results
        if let Some(total) = json
            .get("webPages")
            .and_then(|v| v.get("totalEstimatedMatches"))
            .and_then(|v| v.as_u64())
        {
            results.total_results = Some(total);
        }

        // Store raw metadata
        results.metadata = Some(json);

        Ok(results)
    }

    fn provider_name(&self) -> &'static str {
        "bing"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bing_provider_new() {
        let provider = BingProvider::new("test-key");
        assert_eq!(provider.endpoint(), DEFAULT_API_URL);
    }

    #[test]
    fn test_bing_provider_custom_url() {
        let provider = BingProvider::new("test-key")
            .with_api_url("https://custom.api.com/search");
        assert_eq!(provider.endpoint(), "https://custom.api.com/search");
    }
}
