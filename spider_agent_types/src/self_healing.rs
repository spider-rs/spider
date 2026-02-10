//! Self-healing selector support.
//!
//! This module provides types and utilities for automatically healing
//! failed selectors by asking the LLM to diagnose and suggest fixes.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Configuration for self-healing behavior.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SelfHealingConfig {
    /// Enable self-healing.
    pub enabled: bool,
    /// Maximum healing attempts per selector.
    pub max_attempts: usize,
    /// Minimum confidence required to try a suggested selector.
    pub min_confidence: f64,
    /// Whether to cache healed selectors for future use.
    pub cache_healed: bool,
    /// Maximum HTML context bytes to include in diagnosis.
    pub max_context_bytes: usize,
    /// Whether to include screenshot in diagnosis requests.
    pub include_screenshot: bool,
}

impl Default for SelfHealingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_attempts: 3,
            min_confidence: 0.5,
            cache_healed: true,
            max_context_bytes: 4000,
            include_screenshot: true,
        }
    }
}

impl SelfHealingConfig {
    /// Create a new config with healing enabled.
    pub fn enabled() -> Self {
        Self {
            enabled: true,
            ..Default::default()
        }
    }

    /// Set max attempts.
    pub fn with_max_attempts(mut self, max: usize) -> Self {
        self.max_attempts = max;
        self
    }

    /// Set minimum confidence.
    pub fn with_min_confidence(mut self, conf: f64) -> Self {
        self.min_confidence = conf.clamp(0.0, 1.0);
        self
    }

    /// Set whether to cache healed selectors.
    pub fn with_cache(mut self, cache: bool) -> Self {
        self.cache_healed = cache;
        self
    }

    /// Set max context bytes.
    pub fn with_max_context(mut self, bytes: usize) -> Self {
        self.max_context_bytes = bytes;
        self
    }
}

/// A request to heal a failed selector.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealingRequest {
    /// The selector that failed.
    pub failed_selector: String,
    /// The type of action that was attempted.
    pub action_type: String,
    /// HTML context around the expected element location.
    pub html_context: String,
    /// Error message from the failure.
    pub error: String,
    /// What the element should look like/do.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub element_description: Option<String>,
    /// Previous selectors that were tried and failed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub previous_attempts: Vec<String>,
    /// Current page URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_url: Option<String>,
    /// Current page title.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_title: Option<String>,
}

impl HealingRequest {
    /// Create a new healing request.
    pub fn new(
        failed_selector: impl Into<String>,
        action_type: impl Into<String>,
        html_context: impl Into<String>,
        error: impl Into<String>,
    ) -> Self {
        Self {
            failed_selector: failed_selector.into(),
            action_type: action_type.into(),
            html_context: html_context.into(),
            error: error.into(),
            element_description: None,
            previous_attempts: Vec::new(),
            page_url: None,
            page_title: None,
        }
    }

    /// Add element description.
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.element_description = Some(desc.into());
        self
    }

    /// Add previous failed attempts.
    pub fn with_previous_attempts(mut self, attempts: Vec<String>) -> Self {
        self.previous_attempts = attempts;
        self
    }

    /// Add page context.
    pub fn with_page_context(mut self, url: impl Into<String>, title: impl Into<String>) -> Self {
        self.page_url = Some(url.into());
        self.page_title = Some(title.into());
        self
    }

    /// Build a prompt for the LLM to diagnose the failure.
    pub fn to_prompt(&self) -> String {
        let mut prompt = String::with_capacity(self.html_context.len() + 1024);

        prompt.push_str("SELECTOR HEALING REQUEST\n\n");

        prompt.push_str("Failed selector: ");
        prompt.push_str(&self.failed_selector);
        prompt.push_str("\nAction type: ");
        prompt.push_str(&self.action_type);
        prompt.push_str("\nError: ");
        prompt.push_str(&self.error);
        prompt.push('\n');

        if let Some(desc) = &self.element_description {
            prompt.push_str("Expected element: ");
            prompt.push_str(desc);
            prompt.push('\n');
        }

        if !self.previous_attempts.is_empty() {
            prompt.push_str("\nPrevious failed attempts:\n");
            for attempt in &self.previous_attempts {
                prompt.push_str("- ");
                prompt.push_str(attempt);
                prompt.push('\n');
            }
        }

        if let Some(url) = &self.page_url {
            prompt.push_str("\nPage URL: ");
            prompt.push_str(url);
        }
        if let Some(title) = &self.page_title {
            prompt.push_str("\nPage title: ");
            prompt.push_str(title);
        }

        prompt.push_str("\n\nHTML CONTEXT:\n");
        prompt.push_str(&self.html_context);

        prompt.push_str("\n\nAnalyze why the selector failed and suggest a working alternative.");
        prompt.push_str("\nReturn a JSON object with:\n");
        prompt.push_str("- diagnosis: explanation of why the selector failed\n");
        prompt.push_str("- suggested_selector: the recommended alternative selector\n");
        prompt.push_str("- confidence: 0.0-1.0 confidence in the suggestion\n");
        prompt.push_str("- alternatives: array of other possible selectors (optional)\n");

        prompt
    }
}

/// Diagnosis and suggested fix from the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealingDiagnosis {
    /// Explanation of why the selector failed.
    pub diagnosis: String,
    /// Suggested working selector.
    pub suggested_selector: String,
    /// Confidence in the suggestion (0.0 to 1.0).
    pub confidence: f64,
    /// Alternative selectors to try if the first fails.
    #[serde(default)]
    pub alternatives: Vec<String>,
    /// Type of issue identified.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issue_type: Option<SelectorIssueType>,
}

impl HealingDiagnosis {
    /// Create a new diagnosis.
    pub fn new(
        diagnosis: impl Into<String>,
        suggested_selector: impl Into<String>,
        confidence: f64,
    ) -> Self {
        Self {
            diagnosis: diagnosis.into(),
            suggested_selector: suggested_selector.into(),
            confidence: confidence.clamp(0.0, 1.0),
            alternatives: Vec::new(),
            issue_type: None,
        }
    }

    /// Add alternative selectors.
    pub fn with_alternatives(mut self, alts: Vec<String>) -> Self {
        self.alternatives = alts;
        self
    }

    /// Set issue type.
    pub fn with_issue_type(mut self, issue: SelectorIssueType) -> Self {
        self.issue_type = Some(issue);
        self
    }

    /// Parse from LLM JSON response.
    pub fn from_json(value: &serde_json::Value) -> Option<Self> {
        let diagnosis = value.get("diagnosis").and_then(|v| v.as_str())?.to_string();
        let suggested_selector = value
            .get("suggested_selector")
            .and_then(|v| v.as_str())?
            .to_string();
        let confidence = value
            .get("confidence")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.5);

        let alternatives = value
            .get("alternatives")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        let issue_type = value
            .get("issue_type")
            .and_then(|v| v.as_str())
            .and_then(|s| match s {
                "not_found" => Some(SelectorIssueType::ElementNotFound),
                "multiple_matches" => Some(SelectorIssueType::MultipleMatches),
                "wrong_element" => Some(SelectorIssueType::WrongElement),
                "dynamic_id" => Some(SelectorIssueType::DynamicId),
                "iframe" => Some(SelectorIssueType::InsideIframe),
                "shadow_dom" => Some(SelectorIssueType::ShadowDom),
                "timing" => Some(SelectorIssueType::TimingIssue),
                _ => None,
            });

        Some(Self {
            diagnosis,
            suggested_selector,
            confidence: confidence.clamp(0.0, 1.0),
            alternatives,
            issue_type,
        })
    }

    /// Check if the suggestion meets minimum confidence.
    pub fn is_confident(&self, min_confidence: f64) -> bool {
        self.confidence >= min_confidence
    }

    /// Get all selectors to try (suggested + alternatives).
    pub fn all_selectors(&self) -> Vec<&str> {
        let mut selectors = vec![self.suggested_selector.as_str()];
        selectors.extend(self.alternatives.iter().map(|s| s.as_str()));
        selectors
    }
}

/// Types of selector issues.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectorIssueType {
    /// Element doesn't exist in DOM.
    ElementNotFound,
    /// Selector matches multiple elements.
    MultipleMatches,
    /// Selector matches wrong element.
    WrongElement,
    /// Selector uses dynamic/changing ID.
    DynamicId,
    /// Element is inside an iframe.
    InsideIframe,
    /// Element is inside shadow DOM.
    ShadowDom,
    /// Element not ready/loaded yet.
    TimingIssue,
    /// Unknown issue.
    Unknown,
}

/// Result of a healing attempt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealingResult {
    /// Whether healing was successful.
    pub success: bool,
    /// The working selector (if found).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub working_selector: Option<String>,
    /// Number of attempts made.
    pub attempts: usize,
    /// All diagnosis received.
    pub diagnoses: Vec<HealingDiagnosis>,
    /// Total duration in milliseconds.
    pub duration_ms: u64,
    /// Final error if healing failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl HealingResult {
    /// Create a successful result.
    pub fn success(selector: impl Into<String>, attempts: usize, duration_ms: u64) -> Self {
        Self {
            success: true,
            working_selector: Some(selector.into()),
            attempts,
            diagnoses: Vec::new(),
            duration_ms,
            error: None,
        }
    }

    /// Create a failed result.
    pub fn failure(error: impl Into<String>, attempts: usize, duration_ms: u64) -> Self {
        Self {
            success: false,
            working_selector: None,
            attempts,
            diagnoses: Vec::new(),
            duration_ms,
            error: Some(error.into()),
        }
    }

    /// Add diagnoses.
    pub fn with_diagnoses(mut self, diagnoses: Vec<HealingDiagnosis>) -> Self {
        self.diagnoses = diagnoses;
        self
    }
}

/// Cache for healed selectors.
#[derive(Debug, Clone, Default)]
pub struct HealedSelectorCache {
    /// Map of (page_pattern, original_selector) -> healed_selector.
    cache: HashMap<(String, String), CachedSelector>,
    /// Hit count for analytics.
    hits: usize,
    /// Miss count for analytics.
    misses: usize,
}

impl HealedSelectorCache {
    /// Create a new empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// Get a cached selector.
    pub fn get(&mut self, page_pattern: &str, original: &str) -> Option<&str> {
        let key = (page_pattern.to_string(), original.to_string());
        if let Some(cached) = self.cache.get(&key) {
            // Check if still valid (not expired)
            if cached.is_valid() {
                self.hits += 1;
                return Some(&cached.healed_selector);
            }
        }
        self.misses += 1;
        None
    }

    /// Store a healed selector.
    pub fn store(
        &mut self,
        page_pattern: impl Into<String>,
        original: impl Into<String>,
        healed: impl Into<String>,
        confidence: f64,
    ) {
        let key = (page_pattern.into(), original.into());
        let cached = CachedSelector {
            healed_selector: healed.into(),
            confidence,
            created_at: std::time::Instant::now(),
            use_count: 0,
            success_count: 0,
        };
        self.cache.insert(key, cached);
    }

    /// Record usage of a cached selector.
    pub fn record_usage(&mut self, page_pattern: &str, original: &str, success: bool) {
        let key = (page_pattern.to_string(), original.to_string());
        if let Some(cached) = self.cache.get_mut(&key) {
            cached.use_count += 1;
            if success {
                cached.success_count += 1;
            }
        }
    }

    /// Get cache statistics.
    pub fn stats(&self) -> CacheStats {
        CacheStats {
            entries: self.cache.len(),
            hits: self.hits,
            misses: self.misses,
            hit_rate: if self.hits + self.misses > 0 {
                self.hits as f64 / (self.hits + self.misses) as f64
            } else {
                0.0
            },
        }
    }

    /// Clear the cache.
    pub fn clear(&mut self) {
        self.cache.clear();
    }

    /// Remove expired entries.
    pub fn cleanup(&mut self, max_age_secs: u64) {
        let now = std::time::Instant::now();
        self.cache
            .retain(|_, v| now.duration_since(v.created_at).as_secs() < max_age_secs);
    }
}

/// A cached healed selector.
#[derive(Debug, Clone)]
struct CachedSelector {
    healed_selector: String,
    confidence: f64,
    created_at: std::time::Instant,
    use_count: usize,
    success_count: usize,
}

impl CachedSelector {
    /// Check if the cached selector is still valid.
    fn is_valid(&self) -> bool {
        // Invalid if older than 1 hour or low success rate with many uses
        let age = self.created_at.elapsed().as_secs();
        if age > 3600 {
            return false;
        }
        // Low confidence selectors have shorter TTL
        if self.confidence < 0.7 && age > 1800 {
            return false;
        }
        if self.use_count >= 5 {
            let success_rate = self.success_count as f64 / self.use_count as f64;
            if success_rate < 0.5 {
                return false;
            }
        }
        true
    }
}

/// Cache statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheStats {
    /// Number of entries.
    pub entries: usize,
    /// Cache hits.
    pub hits: usize,
    /// Cache misses.
    pub misses: usize,
    /// Hit rate (0.0 to 1.0).
    pub hit_rate: f64,
}

/// Statistics about healing operations.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HealingStats {
    /// Total healing attempts.
    pub total_attempts: usize,
    /// Successful healings.
    pub successful: usize,
    /// Failed healings.
    pub failed: usize,
    /// Total LLM calls for healing.
    pub llm_calls: usize,
    /// Average attempts per successful healing.
    pub avg_attempts_per_success: f64,
    /// Most common issue types.
    pub issue_counts: HashMap<String, usize>,
}

impl HealingStats {
    /// Create new stats tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a healing attempt.
    pub fn record(&mut self, result: &HealingResult) {
        self.total_attempts += 1;
        self.llm_calls += result.diagnoses.len();

        if result.success {
            self.successful += 1;
        } else {
            self.failed += 1;
        }

        // Update average attempts per success
        if self.successful > 0 {
            // Simplified calculation - in production would track running sum
            self.avg_attempts_per_success = result.attempts as f64;
        }

        // Count issue types
        for diag in &result.diagnoses {
            if let Some(issue) = &diag.issue_type {
                let key = format!("{:?}", issue);
                *self.issue_counts.entry(key).or_insert(0) += 1;
            }
        }
    }

    /// Get success rate.
    pub fn success_rate(&self) -> f64 {
        if self.total_attempts == 0 {
            0.0
        } else {
            self.successful as f64 / self.total_attempts as f64
        }
    }
}

/// Extract HTML context around a selector's expected location.
pub fn extract_html_context(html: &str, max_bytes: usize) -> String {
    // For now, just truncate. A full implementation would:
    // 1. Parse HTML
    // 2. Find elements that might match the selector
    // 3. Extract surrounding context

    if html.len() <= max_bytes {
        return html.to_string();
    }

    // Take from the middle of interactive content
    let body_start = html.find("<body").unwrap_or(0);
    let form_start = html.find("<form").unwrap_or(body_start);
    let main_start = html.find("<main").unwrap_or(form_start);

    let start = main_start.min(html.len().saturating_sub(max_bytes));
    let end = (start + max_bytes).min(html.len());

    // Find UTF-8 boundaries
    let mut actual_start = start;
    while actual_start < html.len() && !html.is_char_boundary(actual_start) {
        actual_start += 1;
    }

    let mut actual_end = end;
    while actual_end > actual_start && !html.is_char_boundary(actual_end) {
        actual_end -= 1;
    }

    format!(
        "...[context start]...\n{}\n...[context end]...",
        &html[actual_start..actual_end]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_healing_config() {
        let config = SelfHealingConfig::enabled()
            .with_max_attempts(5)
            .with_min_confidence(0.7);

        assert!(config.enabled);
        assert_eq!(config.max_attempts, 5);
        assert_eq!(config.min_confidence, 0.7);
    }

    #[test]
    fn test_healing_request() {
        let request = HealingRequest::new(
            "button.submit",
            "Click",
            "<div><button class='btn'>Submit</button></div>",
            "Element not found",
        )
        .with_description("The submit button at the bottom of the form")
        .with_page_context("https://example.com/form", "Contact Form");

        let prompt = request.to_prompt();
        assert!(prompt.contains("button.submit"));
        assert!(prompt.contains("Element not found"));
        assert!(prompt.contains("submit button"));
    }

    #[test]
    fn test_healing_diagnosis() {
        let diag = HealingDiagnosis::new("The class name was changed", "button.btn-submit", 0.85)
            .with_alternatives(vec!["button[type='submit']".to_string()])
            .with_issue_type(SelectorIssueType::DynamicId);

        assert!(diag.is_confident(0.8));
        assert!(!diag.is_confident(0.9));
        assert_eq!(diag.all_selectors().len(), 2);
    }

    #[test]
    fn test_healing_diagnosis_parsing() {
        let json = serde_json::json!({
            "diagnosis": "The button has a dynamic ID",
            "suggested_selector": "button[data-action='submit']",
            "confidence": 0.9,
            "alternatives": ["form button:last-child"],
            "issue_type": "dynamic_id"
        });

        let diag = HealingDiagnosis::from_json(&json).unwrap();
        assert_eq!(diag.confidence, 0.9);
        assert_eq!(diag.issue_type, Some(SelectorIssueType::DynamicId));
    }

    #[test]
    fn test_healing_result() {
        let success = HealingResult::success("button.new-class", 2, 500);
        assert!(success.success);
        assert_eq!(
            success.working_selector,
            Some("button.new-class".to_string())
        );

        let failure = HealingResult::failure("No working selector found", 3, 1000);
        assert!(!failure.success);
        assert!(failure.working_selector.is_none());
    }

    #[test]
    fn test_healed_selector_cache() {
        let mut cache = HealedSelectorCache::new();

        // Store and retrieve
        cache.store("example.com/*", "button.old", "button.new", 0.9);
        let result = cache.get("example.com/*", "button.old");
        assert_eq!(result, Some("button.new"));

        // Check stats
        let stats = cache.stats();
        assert_eq!(stats.entries, 1);
        assert_eq!(stats.hits, 1);
    }

    #[test]
    fn test_healing_stats() {
        let mut stats = HealingStats::new();

        let result = HealingResult::success("btn", 2, 300)
            .with_diagnoses(vec![HealingDiagnosis::new("Issue 1", "sel1", 0.5)
                .with_issue_type(SelectorIssueType::DynamicId)]);

        stats.record(&result);

        assert_eq!(stats.successful, 1);
        assert_eq!(stats.total_attempts, 1);
        assert_eq!(stats.success_rate(), 1.0);
    }

    #[test]
    fn test_extract_html_context() {
        let html =
            "<html><body><main><form><input><button>Submit</button></form></main></body></html>";
        let context = extract_html_context(html, 50);
        assert!(context.len() <= 100); // Accounts for wrapper text
    }

    #[test]
    fn test_cache_cleanup() {
        let mut cache = HealedSelectorCache::new();
        cache.store("page", "sel1", "healed1", 0.9);

        // Cleanup with very short max age - should remove entry
        cache.cleanup(0);
        let stats = cache.stats();
        assert_eq!(stats.entries, 0);
    }
}
