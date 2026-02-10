//! Map result types for page URL discovery.
//!
//! Contains types for representing discovered URLs and page mapping results.

use crate::AutomationUsage;

/// Result of the `map()` API call for page discovery.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct MapResult {
    /// The URLs discovered on the page.
    pub urls: Vec<DiscoveredUrl>,
    /// Relevance score of the page to the prompt (0.0 - 1.0).
    pub relevance: f32,
    /// Summary of what the page contains.
    pub summary: String,
    /// Suggested next URLs to explore based on the prompt.
    pub suggested_next: Vec<String>,
    /// Optional screenshot if captured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub screenshot: Option<String>,
    /// Token usage statistics.
    #[serde(default)]
    pub usage: AutomationUsage,
}

impl MapResult {
    /// Create a new empty map result.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create with a summary.
    pub fn with_summary(summary: impl Into<String>) -> Self {
        Self {
            summary: summary.into(),
            ..Default::default()
        }
    }

    /// Add a discovered URL.
    pub fn add_url(&mut self, url: DiscoveredUrl) {
        self.urls.push(url);
    }

    /// Add a suggested next URL.
    pub fn add_suggested(&mut self, url: impl Into<String>) {
        self.suggested_next.push(url.into());
    }

    /// Set the relevance score.
    pub fn with_relevance(mut self, relevance: f32) -> Self {
        self.relevance = relevance.clamp(0.0, 1.0);
        self
    }

    /// Set the screenshot.
    pub fn with_screenshot(mut self, screenshot: impl Into<String>) -> Self {
        self.screenshot = Some(screenshot.into());
        self
    }

    /// Set the usage statistics.
    pub fn with_usage(mut self, usage: AutomationUsage) -> Self {
        self.usage = usage;
        self
    }

    /// Get URLs sorted by relevance (highest first).
    pub fn urls_by_relevance(&self) -> Vec<&DiscoveredUrl> {
        let mut urls: Vec<_> = self.urls.iter().collect();
        urls.sort_by(|a, b| {
            b.relevance
                .partial_cmp(&a.relevance)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        urls
    }

    /// Get only recommended URLs.
    pub fn recommended_urls(&self) -> Vec<&DiscoveredUrl> {
        self.urls.iter().filter(|u| u.recommended).collect()
    }

    /// Get URLs by category.
    pub fn urls_by_category(&self, category: &str) -> Vec<&DiscoveredUrl> {
        self.urls
            .iter()
            .filter(|u| u.category.eq_ignore_ascii_case(category))
            .collect()
    }
}

/// A discovered URL with AI-generated metadata.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct DiscoveredUrl {
    /// The URL.
    pub url: String,
    /// Link text or title.
    pub text: String,
    /// AI-generated description of what this URL likely contains.
    pub description: String,
    /// Relevance to the prompt (0.0 - 1.0).
    pub relevance: f32,
    /// Whether this URL is recommended to visit.
    pub recommended: bool,
    /// Category of the URL (navigation, content, external, etc.).
    pub category: String,
}

impl DiscoveredUrl {
    /// Create a new discovered URL.
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            ..Default::default()
        }
    }

    /// Create with full details.
    pub fn with_details(
        url: impl Into<String>,
        text: impl Into<String>,
        description: impl Into<String>,
        relevance: f32,
        category: impl Into<String>,
    ) -> Self {
        Self {
            url: url.into(),
            text: text.into(),
            description: description.into(),
            relevance: relevance.clamp(0.0, 1.0),
            recommended: relevance > 0.5,
            category: category.into(),
        }
    }

    /// Set the text.
    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.text = text.into();
        self
    }

    /// Set the description.
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    /// Set the relevance.
    pub fn with_relevance(mut self, relevance: f32) -> Self {
        self.relevance = relevance.clamp(0.0, 1.0);
        self
    }

    /// Set whether recommended.
    pub fn with_recommended(mut self, recommended: bool) -> Self {
        self.recommended = recommended;
        self
    }

    /// Set the category.
    pub fn with_category(mut self, category: impl Into<String>) -> Self {
        self.category = category.into();
        self
    }

    /// Check if this is a navigation URL.
    pub fn is_navigation(&self) -> bool {
        self.category.eq_ignore_ascii_case("navigation")
    }

    /// Check if this is a content URL.
    pub fn is_content(&self) -> bool {
        self.category.eq_ignore_ascii_case("content")
    }

    /// Check if this is an external URL.
    pub fn is_external(&self) -> bool {
        self.category.eq_ignore_ascii_case("external")
    }
}

/// URL category constants.
pub mod categories {
    /// Navigation URLs (menu items, breadcrumbs, pagination).
    pub const NAVIGATION: &str = "navigation";
    /// Content URLs (articles, products, main content pages).
    pub const CONTENT: &str = "content";
    /// External URLs (links to other domains).
    pub const EXTERNAL: &str = "external";
    /// Resource URLs (images, scripts, stylesheets).
    pub const RESOURCE: &str = "resource";
    /// Action URLs (forms, buttons that trigger actions).
    pub const ACTION: &str = "action";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_discovered_url() {
        let url = DiscoveredUrl::with_details(
            "https://example.com/product",
            "Product Page",
            "A product listing page",
            0.8,
            "content",
        );

        assert!(url.recommended);
        assert!(url.is_content());
        assert!(!url.is_external());
    }

    #[test]
    fn test_map_result() {
        let mut result = MapResult::with_summary("Test page").with_relevance(0.75);

        result.add_url(DiscoveredUrl::new("https://example.com/a").with_relevance(0.9));
        result.add_url(DiscoveredUrl::new("https://example.com/b").with_relevance(0.5));

        let by_relevance = result.urls_by_relevance();
        assert_eq!(by_relevance[0].url, "https://example.com/a");
    }

    #[test]
    fn test_relevance_clamping() {
        let url = DiscoveredUrl::new("test").with_relevance(1.5);
        assert_eq!(url.relevance, 1.0);

        let url = DiscoveredUrl::new("test").with_relevance(-0.5);
        assert_eq!(url.relevance, 0.0);
    }
}
