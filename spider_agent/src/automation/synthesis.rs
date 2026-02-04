//! Multi-page synthesis support.
//!
//! This module enables analyzing and synthesizing data from multiple pages
//! in a single LLM call, turning N pages into 1 LLM call instead of N calls.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Configuration for multi-page synthesis.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SynthesisConfig {
    /// Maximum tokens to allocate per page.
    pub max_tokens_per_page: usize,
    /// Maximum number of pages to synthesize at once.
    pub max_pages: usize,
    /// Whether to pre-summarize long pages before synthesis.
    pub pre_summarize: bool,
    /// Token budget for pre-summarization.
    pub summary_tokens: usize,
    /// Whether to include page relevance scores.
    pub include_relevance: bool,
    /// Minimum relevance score to include a page.
    pub min_relevance: f64,
}

impl Default for SynthesisConfig {
    fn default() -> Self {
        Self {
            max_tokens_per_page: 4000,
            max_pages: 10,
            pre_summarize: true,
            summary_tokens: 500,
            include_relevance: true,
            min_relevance: 0.3,
        }
    }
}

impl SynthesisConfig {
    /// Create a new config.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set max tokens per page.
    pub fn with_max_tokens_per_page(mut self, tokens: usize) -> Self {
        self.max_tokens_per_page = tokens;
        self
    }

    /// Set max pages.
    pub fn with_max_pages(mut self, max: usize) -> Self {
        self.max_pages = max;
        self
    }

    /// Set pre-summarization.
    pub fn with_pre_summarize(mut self, enabled: bool) -> Self {
        self.pre_summarize = enabled;
        self
    }

    /// Set minimum relevance.
    pub fn with_min_relevance(mut self, min: f64) -> Self {
        self.min_relevance = min.clamp(0.0, 1.0);
        self
    }

    /// Calculate token budget per page given total budget and page count.
    pub fn tokens_per_page(&self, total_budget: usize, page_count: usize) -> usize {
        if page_count == 0 {
            return 0;
        }
        (total_budget / page_count).min(self.max_tokens_per_page)
    }
}

/// Context for a single page in multi-page synthesis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageContext {
    /// Page URL.
    pub url: String,
    /// Page title.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Extracted data from this page (if any).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extracted: Option<Value>,
    /// Summary of the page content.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Raw HTML content (truncated).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub html: Option<String>,
    /// Relevance score (0.0 to 1.0).
    pub relevance: f64,
    /// Page index in the original list.
    pub index: usize,
    /// Any error that occurred processing this page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl PageContext {
    /// Create a new page context.
    pub fn new(url: impl Into<String>, index: usize) -> Self {
        Self {
            url: url.into(),
            title: None,
            extracted: None,
            summary: None,
            html: None,
            relevance: 0.5,
            index,
            error: None,
        }
    }

    /// Set title.
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Set extracted data.
    pub fn with_extracted(mut self, data: Value) -> Self {
        self.extracted = Some(data);
        self
    }

    /// Set summary.
    pub fn with_summary(mut self, summary: impl Into<String>) -> Self {
        self.summary = Some(summary.into());
        self
    }

    /// Set HTML.
    pub fn with_html(mut self, html: impl Into<String>) -> Self {
        self.html = Some(html.into());
        self
    }

    /// Set relevance.
    pub fn with_relevance(mut self, relevance: f64) -> Self {
        self.relevance = relevance.clamp(0.0, 1.0);
        self
    }

    /// Set error.
    pub fn with_error(mut self, error: impl Into<String>) -> Self {
        self.error = Some(error.into());
        self
    }

    /// Check if this page has usable content.
    pub fn has_content(&self) -> bool {
        self.extracted.is_some() || self.summary.is_some() || self.html.is_some()
    }

    /// Check if this page had an error.
    pub fn has_error(&self) -> bool {
        self.error.is_some()
    }

    /// Estimate token count for this page's content.
    pub fn estimated_tokens(&self) -> usize {
        let mut tokens = 0;

        // URL + title
        tokens += self.url.len() / 4;
        if let Some(title) = &self.title {
            tokens += title.len() / 4;
        }

        // Content
        if let Some(extracted) = &self.extracted {
            tokens += extracted.to_string().len() / 4;
        }
        if let Some(summary) = &self.summary {
            tokens += summary.len() / 4;
        }
        if let Some(html) = &self.html {
            tokens += html.len() / 4;
        }

        tokens
    }
}

/// Multi-page context for synthesis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultiPageContext {
    /// All pages in the context.
    pub pages: Vec<PageContext>,
    /// Total token budget for synthesis.
    pub total_token_budget: usize,
    /// The synthesis goal/question.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub goal: Option<String>,
    /// Synthesis configuration.
    #[serde(skip)]
    pub config: SynthesisConfig,
}

impl MultiPageContext {
    /// Create a new multi-page context.
    pub fn new(total_token_budget: usize) -> Self {
        Self {
            pages: Vec::new(),
            total_token_budget,
            goal: None,
            config: SynthesisConfig::default(),
        }
    }

    /// Set the configuration.
    pub fn with_config(mut self, config: SynthesisConfig) -> Self {
        self.config = config;
        self
    }

    /// Set the goal.
    pub fn with_goal(mut self, goal: impl Into<String>) -> Self {
        self.goal = Some(goal.into());
        self
    }

    /// Add a page.
    pub fn add_page(&mut self, page: PageContext) {
        self.pages.push(page);
    }

    /// Get the number of pages.
    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    /// Get pages sorted by relevance (highest first).
    pub fn pages_by_relevance(&self) -> Vec<&PageContext> {
        let mut pages: Vec<_> = self.pages.iter().collect();
        pages.sort_by(|a, b| b.relevance.partial_cmp(&a.relevance).unwrap_or(std::cmp::Ordering::Equal));
        pages
    }

    /// Get pages that meet minimum relevance threshold.
    pub fn relevant_pages(&self) -> Vec<&PageContext> {
        self.pages
            .iter()
            .filter(|p| p.relevance >= self.config.min_relevance)
            .collect()
    }

    /// Truncate pages to fit within token budget.
    pub fn fit_to_budget(&mut self) {
        // Sort by relevance
        self.pages.sort_by(|a, b| {
            b.relevance.partial_cmp(&a.relevance).unwrap_or(std::cmp::Ordering::Equal)
        });

        // Limit to max pages
        if self.pages.len() > self.config.max_pages {
            self.pages.truncate(self.config.max_pages);
        }

        // Calculate tokens per page
        let tokens_per_page = self.config.tokens_per_page(self.total_token_budget, self.pages.len());

        // Truncate each page's content to fit
        for page in &mut self.pages {
            let mut current_tokens = page.estimated_tokens();

            // Truncate HTML first (least valuable usually)
            if current_tokens > tokens_per_page {
                if let Some(html) = &mut page.html {
                    let target_len = (tokens_per_page * 4).min(html.len());
                    *html = truncate_to_char_boundary(html, target_len);
                    current_tokens = page.estimated_tokens();
                }
            }

            // Truncate summary if still too long
            if current_tokens > tokens_per_page {
                if let Some(summary) = &mut page.summary {
                    let target_len = (tokens_per_page * 4).min(summary.len());
                    *summary = truncate_to_char_boundary(summary, target_len);
                }
            }
        }
    }

    /// Build the synthesis prompt.
    pub fn to_prompt(&self) -> String {
        let mut prompt = String::with_capacity(self.total_token_budget * 4);

        prompt.push_str("MULTI-PAGE SYNTHESIS REQUEST\n\n");

        if let Some(goal) = &self.goal {
            prompt.push_str("Goal: ");
            prompt.push_str(goal);
            prompt.push_str("\n\n");
        }

        prompt.push_str(&format!("Pages to analyze: {}\n\n", self.pages.len()));

        for (i, page) in self.pages.iter().enumerate() {
            prompt.push_str(&format!("=== PAGE {} ===\n", i + 1));
            prompt.push_str("URL: ");
            prompt.push_str(&page.url);
            prompt.push('\n');

            if let Some(title) = &page.title {
                prompt.push_str("Title: ");
                prompt.push_str(title);
                prompt.push('\n');
            }

            prompt.push_str(&format!("Relevance: {:.2}\n", page.relevance));

            if let Some(extracted) = &page.extracted {
                prompt.push_str("Extracted Data:\n");
                prompt.push_str(&serde_json::to_string_pretty(extracted).unwrap_or_default());
                prompt.push_str("\n\n");
            }

            if let Some(summary) = &page.summary {
                prompt.push_str("Summary:\n");
                prompt.push_str(summary);
                prompt.push_str("\n\n");
            }

            if let Some(html) = &page.html {
                prompt.push_str("HTML Content:\n");
                prompt.push_str(html);
                prompt.push_str("\n\n");
            }

            if let Some(error) = &page.error {
                prompt.push_str("Error: ");
                prompt.push_str(error);
                prompt.push_str("\n\n");
            }

            prompt.push('\n');
        }

        prompt.push_str("TASK:\n");
        prompt.push_str("Synthesize the information from all pages above. Return a JSON object with:\n");
        prompt.push_str("- synthesis: the combined analysis/answer\n");
        prompt.push_str("- page_contributions: array of { page_index, contribution } for each page\n");
        prompt.push_str("- confidence: overall confidence in the synthesis (0.0-1.0)\n");

        prompt
    }

    /// Get total estimated tokens across all pages.
    pub fn total_estimated_tokens(&self) -> usize {
        self.pages.iter().map(|p| p.estimated_tokens()).sum()
    }
}

/// Result of multi-page synthesis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SynthesisResult {
    /// The synthesized output.
    pub synthesis: Value,
    /// Contribution from each page.
    pub page_contributions: Vec<PageContribution>,
    /// Overall confidence in the synthesis.
    pub confidence: f64,
    /// Number of pages used.
    pub pages_used: usize,
    /// Token usage for the synthesis.
    pub tokens_used: usize,
    /// Duration in milliseconds.
    pub duration_ms: u64,
}

impl SynthesisResult {
    /// Create a new synthesis result.
    pub fn new(synthesis: Value, confidence: f64) -> Self {
        Self {
            synthesis,
            page_contributions: Vec::new(),
            confidence: confidence.clamp(0.0, 1.0),
            pages_used: 0,
            tokens_used: 0,
            duration_ms: 0,
        }
    }

    /// Add page contributions.
    pub fn with_contributions(mut self, contributions: Vec<PageContribution>) -> Self {
        self.pages_used = contributions.len();
        self.page_contributions = contributions;
        self
    }

    /// Set token usage.
    pub fn with_tokens(mut self, tokens: usize) -> Self {
        self.tokens_used = tokens;
        self
    }

    /// Set duration.
    pub fn with_duration(mut self, ms: u64) -> Self {
        self.duration_ms = ms;
        self
    }

    /// Parse from LLM JSON response.
    pub fn from_json(value: &Value) -> Option<Self> {
        let synthesis = value.get("synthesis")?.clone();
        let confidence = value
            .get("confidence")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.5);

        let page_contributions = value
            .get("page_contributions")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| PageContribution::from_json(v))
                    .collect()
            })
            .unwrap_or_default();

        Some(Self {
            synthesis,
            page_contributions,
            confidence: confidence.clamp(0.0, 1.0),
            pages_used: 0,
            tokens_used: 0,
            duration_ms: 0,
        })
    }

    /// Get pages that contributed significantly.
    pub fn significant_contributors(&self, min_contribution: f64) -> Vec<&PageContribution> {
        self.page_contributions
            .iter()
            .filter(|c| c.weight >= min_contribution)
            .collect()
    }
}

/// Contribution of a single page to the synthesis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageContribution {
    /// Index of the page (0-based).
    pub page_index: usize,
    /// URL of the page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Description of the contribution.
    pub contribution: String,
    /// Weight/importance of this page's contribution (0.0-1.0).
    pub weight: f64,
    /// Key data points from this page.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub key_points: Vec<String>,
}

impl PageContribution {
    /// Create a new page contribution.
    pub fn new(page_index: usize, contribution: impl Into<String>, weight: f64) -> Self {
        Self {
            page_index,
            url: None,
            contribution: contribution.into(),
            weight: weight.clamp(0.0, 1.0),
            key_points: Vec::new(),
        }
    }

    /// Set URL.
    pub fn with_url(mut self, url: impl Into<String>) -> Self {
        self.url = Some(url.into());
        self
    }

    /// Add key points.
    pub fn with_key_points(mut self, points: Vec<String>) -> Self {
        self.key_points = points;
        self
    }

    /// Parse from JSON.
    pub fn from_json(value: &Value) -> Option<Self> {
        let page_index = value.get("page_index").and_then(|v| v.as_u64())? as usize;
        let contribution = value
            .get("contribution")
            .and_then(|v| v.as_str())?
            .to_string();
        let weight = value
            .get("weight")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.5);
        let url = value
            .get("url")
            .and_then(|v| v.as_str())
            .map(String::from);
        let key_points = value
            .get("key_points")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        Some(Self {
            page_index,
            url,
            contribution,
            weight: weight.clamp(0.0, 1.0),
            key_points,
        })
    }
}

/// Utility to truncate a string to a char boundary.
fn truncate_to_char_boundary(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        return s.to_string();
    }

    let mut end = max_len;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }

    format!("{}...[truncated]", &s[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_synthesis_config() {
        let config = SynthesisConfig::new()
            .with_max_pages(5)
            .with_max_tokens_per_page(2000)
            .with_min_relevance(0.5);

        assert_eq!(config.max_pages, 5);
        assert_eq!(config.max_tokens_per_page, 2000);
        assert_eq!(config.min_relevance, 0.5);
    }

    #[test]
    fn test_tokens_per_page() {
        let config = SynthesisConfig::new().with_max_tokens_per_page(1000);

        // 5000 tokens / 5 pages = 1000 per page (at max)
        assert_eq!(config.tokens_per_page(5000, 5), 1000);

        // 10000 tokens / 5 pages = 2000 per page, but capped at 1000
        assert_eq!(config.tokens_per_page(10000, 5), 1000);

        // 2000 tokens / 5 pages = 400 per page
        assert_eq!(config.tokens_per_page(2000, 5), 400);
    }

    #[test]
    fn test_page_context() {
        let page = PageContext::new("https://example.com", 0)
            .with_title("Example")
            .with_relevance(0.8)
            .with_summary("A test page");

        assert!(page.has_content());
        assert!(!page.has_error());
        assert!(page.estimated_tokens() > 0);
    }

    #[test]
    fn test_multi_page_context() {
        let mut ctx = MultiPageContext::new(10000)
            .with_goal("Compare products")
            .with_config(SynthesisConfig::new().with_max_pages(3));

        ctx.add_page(PageContext::new("https://a.com", 0).with_relevance(0.9));
        ctx.add_page(PageContext::new("https://b.com", 1).with_relevance(0.7));
        ctx.add_page(PageContext::new("https://c.com", 2).with_relevance(0.5));

        assert_eq!(ctx.page_count(), 3);

        let by_relevance = ctx.pages_by_relevance();
        assert_eq!(by_relevance[0].url, "https://a.com");
    }

    #[test]
    fn test_fit_to_budget() {
        let mut ctx = MultiPageContext::new(1000)
            .with_config(SynthesisConfig::new().with_max_pages(2));

        ctx.add_page(PageContext::new("https://a.com", 0).with_relevance(0.9));
        ctx.add_page(PageContext::new("https://b.com", 1).with_relevance(0.8));
        ctx.add_page(PageContext::new("https://c.com", 2).with_relevance(0.7));

        ctx.fit_to_budget();

        // Should be limited to 2 pages
        assert_eq!(ctx.page_count(), 2);
        // Should be sorted by relevance
        assert_eq!(ctx.pages[0].url, "https://a.com");
    }

    #[test]
    fn test_synthesis_result() {
        let result = SynthesisResult::new(
            serde_json::json!({"answer": "Combined data"}),
            0.85,
        )
        .with_contributions(vec![
            PageContribution::new(0, "Provided main data", 0.7),
            PageContribution::new(1, "Supplementary info", 0.3),
        ])
        .with_tokens(500);

        assert_eq!(result.pages_used, 2);
        assert_eq!(result.confidence, 0.85);

        let significant = result.significant_contributors(0.5);
        assert_eq!(significant.len(), 1);
    }

    #[test]
    fn test_page_contribution_parsing() {
        let json = serde_json::json!({
            "page_index": 0,
            "url": "https://example.com",
            "contribution": "Main source of data",
            "weight": 0.8,
            "key_points": ["Point 1", "Point 2"]
        });

        let contrib = PageContribution::from_json(&json).unwrap();
        assert_eq!(contrib.page_index, 0);
        assert_eq!(contrib.weight, 0.8);
        assert_eq!(contrib.key_points.len(), 2);
    }

    #[test]
    fn test_to_prompt() {
        let mut ctx = MultiPageContext::new(10000)
            .with_goal("Find the best product");

        ctx.add_page(
            PageContext::new("https://a.com", 0)
                .with_title("Product A")
                .with_summary("A great product"),
        );

        let prompt = ctx.to_prompt();
        assert!(prompt.contains("MULTI-PAGE SYNTHESIS"));
        assert!(prompt.contains("Find the best product"));
        assert!(prompt.contains("https://a.com"));
        assert!(prompt.contains("Product A"));
    }

    #[test]
    fn test_truncate_to_char_boundary() {
        let s = "Hello, 世界!";
        let truncated = truncate_to_char_boundary(s, 10);
        assert!(truncated.len() <= 25); // Includes truncation suffix
        assert!(truncated.ends_with("...[truncated]"));
    }
}
