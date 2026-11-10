//! SpecStream Search API provider implementation.
//!
//! SpecStream returns web results with a source URL, snippet and, when known,
//! the publish date of each page.

use super::{SearchError, SearchOptions, SearchProvider, SearchResult, SearchResults};

/// Default SpecStream Search API endpoint.
const DEFAULT_API_URL: &str = "https://api.specstream.ai/v1/web-search";
/// Largest number of results SpecStream returns in one call.
const MAX_RESULTS: usize = 100;

/// SpecStream Search API provider.
///
/// # Example
/// ```ignore
/// use spider::features::search_providers::SpecStreamProvider;
/// use spider::features::search::{SearchOptions, SearchProvider};
///
/// let provider = SpecStreamProvider::new("your-api-key");
/// let results = provider.search("rust web crawler", &SearchOptions::default(), None).await?;
/// ```
#[derive(Debug, Clone)]
pub struct SpecStreamProvider {
    api_key: String,
    api_url: Option<String>,
}

impl SpecStreamProvider {
    /// Create a new SpecStream provider with the given API key.
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

/// Build the query parameters for a search call.
fn build_params(query: &str, options: &SearchOptions) -> Vec<(&'static str, String)> {
    // `fast` returns the ranked results with snippets and skips the written answer.
    let mut params = vec![("q", query.to_string()), ("speed", "fast".to_string())];

    if let Some(limit) = options.limit {
        params.push(("num", limit.clamp(1, MAX_RESULTS).to_string()));
    }

    if let Some(ref country) = options.country {
        params.push(("country", country.clone()));
    }

    if let Some(ref language) = options.language {
        params.push(("language", language.clone()));
    }

    params
}

impl SearchProvider for SpecStreamProvider {
    async fn search(
        &self,
        query: &str,
        options: &SearchOptions,
        client: Option<&reqwest::Client>,
    ) -> Result<SearchResults, SearchError> {
        let params = build_params(query, options);

        let owned_client;
        let c = match client {
            Some(c) => c,
            None => {
                owned_client = reqwest::ClientBuilder::new()
                    .timeout(std::time::Duration::from_secs(30))
                    .build()
                    .map_err(|e| SearchError::RequestFailed(e.to_string()))?;
                &owned_client
            }
        };

        let response = c
            .get(self.endpoint())
            .bearer_auth(&self.api_key)
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
                "HTTP {} from SpecStream API",
                status
            )));
        }

        let json: serde_json::Value = response
            .json()
            .await
            .map_err(|e| SearchError::ProviderError(format!("Failed to parse response: {}", e)))?;

        Ok(parse_results(query, json))
    }

    fn provider_name(&self) -> &'static str {
        "specstream"
    }
}

/// Convert a SpecStream JSON response into [`SearchResults`].
fn parse_results(query: &str, json: serde_json::Value) -> SearchResults {
    let mut results = SearchResults::new(query);

    if let Some(items) = json
        .get("data")
        .and_then(|v| v.get("sources"))
        .and_then(|v| v.as_array())
    {
        for item in items {
            let url = item.get("url").and_then(|v| v.as_str()).unwrap_or_default();
            if url.is_empty() {
                continue;
            }
            let title = item
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or_default();

            let mut result = SearchResult::new(title, url, results.results.len() + 1);

            if let Some(snippet) = item
                .get("snippet")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
            {
                result = result.with_snippet(snippet);
            }

            if let Some(published) = item.get("published").and_then(|v| v.as_str()) {
                result = result.with_date(published);
            }

            results.push(result);
        }
    }

    results.metadata = Some(json);
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_specstream_provider_new() {
        let provider = SpecStreamProvider::new("test-key");
        assert_eq!(provider.endpoint(), DEFAULT_API_URL);
    }

    #[test]
    fn test_specstream_provider_custom_url() {
        let provider =
            SpecStreamProvider::new("test-key").with_api_url("https://custom.api.com/search");
        assert_eq!(provider.endpoint(), "https://custom.api.com/search");
    }

    #[test]
    fn test_specstream_build_params() {
        let options = SearchOptions::new()
            .with_limit(500)
            .with_country("DE")
            .with_language("de");
        let params = build_params("rust crawler", &options);
        assert_eq!(
            params,
            vec![
                ("q", "rust crawler".to_string()),
                ("speed", "fast".to_string()),
                ("num", "100".to_string()),
                ("country", "DE".to_string()),
                ("language", "de".to_string()),
            ]
        );
        let params = build_params("q", &SearchOptions::default());
        assert!(!params.iter().any(|(k, _)| *k == "num"));
    }

    #[test]
    fn test_specstream_parse_results() {
        let json = serde_json::json!({
            "data": {
                "request_id": "req_1",
                "query": "q",
                "sources": [
                    {
                        "url": "https://a.example",
                        "title": "A",
                        "site": "a.example",
                        "snippet": "short",
                        "cited": false,
                        "published": "2026-10-01"
                    },
                    { "url": "", "title": "No URL", "site": "", "snippet": "dropped", "cited": false },
                    { "url": "https://b.example", "title": "B", "site": "b.example", "snippet": "", "cited": false }
                ]
            }
        });
        let results = parse_results("q", json);
        assert_eq!(results.results.len(), 2);
        assert_eq!(results.results[0].snippet.as_deref(), Some("short"));
        assert_eq!(results.results[0].date.as_deref(), Some("2026-10-01"));
        assert_eq!(results.results[1].position, 2);
        assert_eq!(results.results[1].snippet, None);
        assert_eq!(results.results[1].date, None);
        assert!(results.metadata.is_some());
    }
}
