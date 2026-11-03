//! Keenable Search API provider implementation.
//!
//! Keenable works without an API key: requests go to the free public endpoint
//! unless a key is configured, in which case the keyed endpoint is used.

use super::{SearchError, SearchOptions, SearchProvider, SearchResult, SearchResults};

/// Keyless public Keenable endpoint.
const PUBLIC_API_URL: &str = "https://api.keenable.ai/v1/search/public";
/// Keyed Keenable endpoint (higher limits).
const KEYED_API_URL: &str = "https://api.keenable.ai/v1/search";
/// Maximum characters kept from a snippet when a result has no description.
const MAX_SNIPPET_CHARS: usize = 1000;

/// Keenable Search API provider.
///
/// # Example
/// ```ignore
/// use spider::features::search_providers::KeenableProvider;
/// use spider::features::search::{SearchOptions, SearchProvider};
///
/// // No API key required.
/// let provider = KeenableProvider::new();
/// // Or with a key for higher limits:
/// let provider = KeenableProvider::with_api_key("your-api-key");
/// let results = provider.search("rust web crawler", &SearchOptions::default(), None).await?;
/// ```
#[derive(Debug, Clone, Default)]
pub struct KeenableProvider {
    api_key: Option<String>,
    api_url: Option<String>,
}

impl KeenableProvider {
    /// Create a keyless Keenable provider (uses the free public endpoint).
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a Keenable provider with an API key (uses the keyed endpoint).
    pub fn with_api_key(api_key: impl Into<String>) -> Self {
        let key = api_key.into();
        Self {
            api_key: if key.trim().is_empty() {
                None
            } else {
                Some(key)
            },
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
        match (&self.api_url, &self.api_key) {
            (Some(url), _) => url,
            (None, Some(_)) => KEYED_API_URL,
            (None, None) => PUBLIC_API_URL,
        }
    }
}

impl SearchProvider for KeenableProvider {
    async fn search(
        &self,
        query: &str,
        options: &SearchOptions,
        client: Option<&reqwest::Client>,
    ) -> Result<SearchResults, SearchError> {
        let mut body = serde_json::json!({ "query": query });

        if let Some(limit) = options.limit {
            body["max_results"] = serde_json::json!(limit);
        }

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

        let mut request = c
            .post(self.endpoint())
            .header("Content-Type", "application/json")
            .header("X-Keenable-Title", "spider");
        if let Some(ref key) = self.api_key {
            request = request.header("X-API-Key", key);
        }

        let response = request
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
                "HTTP {} from Keenable API",
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
        "keenable"
    }
}

/// Convert a Keenable JSON response into [`SearchResults`].
fn parse_results(query: &str, json: serde_json::Value) -> SearchResults {
    let mut results = SearchResults::new(query);

    if let Some(items) = json.get("results").and_then(|v| v.as_array()) {
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

            // Prefer the short description; fall back to a truncated snippet.
            let description = item
                .get("description")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty());
            if let Some(desc) = description {
                result = result.with_snippet(desc);
            } else if let Some(snippet) = item.get("snippet").and_then(|v| v.as_str()) {
                let truncated: String = snippet.chars().take(MAX_SNIPPET_CHARS).collect();
                result = result.with_snippet(truncated);
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
    fn test_keenable_keyless_uses_public_endpoint() {
        assert_eq!(KeenableProvider::new().endpoint(), PUBLIC_API_URL);
        assert_eq!(
            KeenableProvider::with_api_key("").endpoint(),
            PUBLIC_API_URL
        );
    }

    #[test]
    fn test_keenable_key_uses_keyed_endpoint() {
        assert_eq!(
            KeenableProvider::with_api_key("test-key").endpoint(),
            KEYED_API_URL
        );
    }

    #[test]
    fn test_keenable_custom_url() {
        let provider = KeenableProvider::new().with_api_url("https://custom.api.com/search");
        assert_eq!(provider.endpoint(), "https://custom.api.com/search");
    }

    #[test]
    fn test_keenable_parse_results() {
        let long = "x".repeat(2000);
        let json = serde_json::json!({
            "results": [
                { "title": "A", "url": "https://a.example", "description": "short", "snippet": "long" },
                { "title": "No URL", "description": "dropped" },
                { "title": "B", "url": "https://b.example", "snippet": long },
            ]
        });
        let results = parse_results("q", json);
        assert_eq!(results.results.len(), 2);
        assert_eq!(results.results[0].snippet.as_deref(), Some("short"));
        assert_eq!(results.results[1].position, 2);
        assert_eq!(
            results.results[1].snippet.as_ref().map(|s| s.len()),
            Some(MAX_SNIPPET_CHARS)
        );
    }
}
