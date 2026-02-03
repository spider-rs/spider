//! Automation module for spider_agent.
//!
//! Provides sophisticated automation capabilities including:
//! - Action chains with conditional execution
//! - Self-healing selector cache
//! - Page observation and understanding
//! - Recovery strategies for resilient automation
//! - Content analysis for smart decisions
//!
//! This module is designed to be the core reusable automation logic
//! that can be used across spider ecosystem.

mod actions;
pub mod cache;
mod chain;
mod config;
mod content;
pub mod executor;
mod observation;
pub mod router;
mod selector_cache;

pub use actions::{ActionRecord, ActionResult, ActionType};
pub use chain::{ChainBuilder, ChainCondition, ChainContext, ChainResult, ChainStep, ChainStepResult};
pub use config::{
    AutomationConfig, CaptureProfile, CleaningIntent, CostTier, HtmlCleaningProfile, ModelPolicy,
    RecoveryStrategy, RetryPolicy, ClipViewport,
};
pub use content::ContentAnalysis;
pub use observation::{
    FormField, FormInfo, InteractiveElement, NavigationOption, PageObservation,
};
pub use selector_cache::{SelectorCache, SelectorCacheEntry};

/// URL-based prompt gating for per-URL config overrides.
///
/// This allows different prompts or configurations to be applied based on URL patterns.
/// Useful for handling different page types differently (e.g., login pages vs. product pages).
///
/// # Example
/// ```rust
/// use spider_agent::automation::{PromptUrlGate, AutomationConfig};
/// use std::collections::HashMap;
///
/// let mut url_map = HashMap::new();
/// url_map.insert(
///     "https://example.com/login".to_string(),
///     Box::new(AutomationConfig::new("Handle login page"))
/// );
///
/// let gate = PromptUrlGate {
///     prompt_url_map: Some(Box::new(url_map)),
///     paths_map: true, // Enable path-prefix matching
/// };
/// ```
#[derive(Debug, Clone, Default, PartialEq)]
#[derive(serde::Serialize, serde::Deserialize)]
pub struct PromptUrlGate {
    /// Map of URLs to config overrides.
    /// Keys can be exact URLs or path prefixes (if paths_map is true).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_url_map: Option<Box<std::collections::HashMap<String, Box<AutomationConfig>>>>,
    /// Whether to use path-prefix matching (case-insensitive).
    /// When true, URLs are matched by prefix, not just exact match.
    #[serde(default)]
    pub paths_map: bool,
}

impl PromptUrlGate {
    /// Create a new empty prompt URL gate.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create with a URL map.
    pub fn with_map(map: std::collections::HashMap<String, Box<AutomationConfig>>) -> Self {
        Self {
            prompt_url_map: Some(Box::new(map)),
            paths_map: false,
        }
    }

    /// Enable path-prefix matching.
    pub fn with_paths_map(mut self) -> Self {
        self.paths_map = true;
        self
    }

    /// Add a URL override.
    pub fn add_override(&mut self, url: impl Into<String>, config: AutomationConfig) {
        let map = self.prompt_url_map.get_or_insert_with(|| Box::new(std::collections::HashMap::new()));
        map.insert(url.into(), Box::new(config));
    }

    /// Match a URL and return the config override if any.
    ///
    /// Returns:
    /// - `None` => blocked (map exists, URL not matched)
    /// - `Some(None)` => allowed, no override
    /// - `Some(Some(cfg))` => allowed, use override config
    pub fn match_url<'a>(&'a self, url: &str) -> Option<Option<&'a AutomationConfig>> {
        let map = match self.prompt_url_map.as_deref() {
            Some(m) => m,
            None => return Some(None), // No map = allow all, no override
        };

        let url_lower = url.to_lowercase();

        // Exact match first
        if let Some(cfg) = map.get(&url_lower) {
            return Some(Some(cfg));
        }

        // Also try original case
        if let Some(cfg) = map.get(url) {
            return Some(Some(cfg));
        }

        // Path-prefix match
        if self.paths_map {
            for (pattern, cfg) in map.iter() {
                let pattern_lower = pattern.to_lowercase();
                if url_lower.starts_with(&pattern_lower) {
                    return Some(Some(cfg));
                }
            }
        }

        // Map exists but no match = blocked
        None
    }

    /// Check if a URL is allowed (matches or no map exists).
    pub fn is_allowed(&self, url: &str) -> bool {
        self.match_url(url).is_some()
    }

    /// Get the override config for a URL, if any.
    pub fn get_override(&self, url: &str) -> Option<&AutomationConfig> {
        self.match_url(url).flatten()
    }
}

/// Token usage tracking for automation operations with granular call tracking.
#[derive(Debug, Clone, Default)]
#[derive(serde::Serialize, serde::Deserialize)]
pub struct AutomationUsage {
    /// Prompt tokens used.
    pub prompt_tokens: u32,
    /// Completion tokens used.
    pub completion_tokens: u32,
    /// Total tokens used.
    pub total_tokens: u32,
    /// Number of LLM API calls made.
    #[serde(default)]
    pub llm_calls: u32,
    /// Number of search API calls made.
    #[serde(default)]
    pub search_calls: u32,
    /// Number of HTTP fetch calls made.
    #[serde(default)]
    pub fetch_calls: u32,
    /// Number of web browser automation calls made.
    #[serde(default)]
    pub webbrowser_calls: u32,
    /// Custom tool calls tracked by tool name.
    #[serde(default)]
    pub custom_tool_calls: std::collections::HashMap<String, u32>,
    /// Total number of API/function calls made (sum of all calls).
    #[serde(default)]
    pub api_calls: u32,
}

impl PartialEq for AutomationUsage {
    fn eq(&self, other: &Self) -> bool {
        self.prompt_tokens == other.prompt_tokens
            && self.completion_tokens == other.completion_tokens
            && self.total_tokens == other.total_tokens
            && self.llm_calls == other.llm_calls
            && self.search_calls == other.search_calls
            && self.fetch_calls == other.fetch_calls
            && self.webbrowser_calls == other.webbrowser_calls
            && self.custom_tool_calls == other.custom_tool_calls
            && self.api_calls == other.api_calls
    }
}

impl Eq for AutomationUsage {}

impl AutomationUsage {
    /// Create new usage stats (counts as 1 LLM call).
    pub fn new(prompt_tokens: u32, completion_tokens: u32) -> Self {
        Self {
            prompt_tokens,
            completion_tokens,
            total_tokens: prompt_tokens + completion_tokens,
            llm_calls: 1,
            search_calls: 0,
            fetch_calls: 0,
            webbrowser_calls: 0,
            custom_tool_calls: std::collections::HashMap::new(),
            api_calls: 1,
        }
    }

    /// Create new usage stats with API call count (legacy).
    pub fn with_api_calls(prompt_tokens: u32, completion_tokens: u32, api_calls: u32) -> Self {
        Self {
            prompt_tokens,
            completion_tokens,
            total_tokens: prompt_tokens + completion_tokens,
            llm_calls: api_calls,
            search_calls: 0,
            fetch_calls: 0,
            webbrowser_calls: 0,
            custom_tool_calls: std::collections::HashMap::new(),
            api_calls,
        }
    }

    /// Accumulate usage from another instance.
    pub fn accumulate(&mut self, other: &Self) {
        self.prompt_tokens += other.prompt_tokens;
        self.completion_tokens += other.completion_tokens;
        self.total_tokens += other.total_tokens;
        self.llm_calls += other.llm_calls;
        self.search_calls += other.search_calls;
        self.fetch_calls += other.fetch_calls;
        self.webbrowser_calls += other.webbrowser_calls;
        // Merge custom tool calls
        for (tool, count) in &other.custom_tool_calls {
            *self.custom_tool_calls.entry(tool.clone()).or_insert(0) += count;
        }
        self.api_calls += other.api_calls;
    }

    /// Increment the LLM call count.
    pub fn increment_llm_calls(&mut self) {
        self.llm_calls += 1;
        self.api_calls += 1;
    }

    /// Increment the search call count.
    pub fn increment_search_calls(&mut self) {
        self.search_calls += 1;
        self.api_calls += 1;
    }

    /// Increment the fetch call count.
    pub fn increment_fetch_calls(&mut self) {
        self.fetch_calls += 1;
        self.api_calls += 1;
    }

    /// Increment the web browser call count.
    pub fn increment_webbrowser_calls(&mut self) {
        self.webbrowser_calls += 1;
        self.api_calls += 1;
    }

    /// Increment a custom tool call count by name.
    pub fn increment_custom_tool_calls(&mut self, tool_name: &str) {
        *self.custom_tool_calls.entry(tool_name.to_string()).or_insert(0) += 1;
        self.api_calls += 1;
    }

    /// Get the call count for a specific custom tool.
    pub fn get_custom_tool_calls(&self, tool_name: &str) -> u32 {
        self.custom_tool_calls.get(tool_name).copied().unwrap_or(0)
    }

    /// Get total custom tool calls across all tools.
    pub fn total_custom_tool_calls(&self) -> u32 {
        self.custom_tool_calls.values().sum()
    }

    /// Increment the API call count (legacy, prefer specific methods).
    pub fn increment_api_calls(&mut self) {
        self.api_calls += 1;
    }

    /// Check if any tokens were used.
    pub fn is_empty(&self) -> bool {
        self.total_tokens == 0
    }
}

impl std::ops::Add for AutomationUsage {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        let mut result = Self {
            prompt_tokens: self.prompt_tokens + other.prompt_tokens,
            completion_tokens: self.completion_tokens + other.completion_tokens,
            total_tokens: self.total_tokens + other.total_tokens,
            llm_calls: self.llm_calls + other.llm_calls,
            search_calls: self.search_calls + other.search_calls,
            fetch_calls: self.fetch_calls + other.fetch_calls,
            webbrowser_calls: self.webbrowser_calls + other.webbrowser_calls,
            custom_tool_calls: self.custom_tool_calls.clone(),
            api_calls: self.api_calls + other.api_calls,
        };
        for (tool, count) in &other.custom_tool_calls {
            *result.custom_tool_calls.entry(tool.clone()).or_insert(0) += count;
        }
        result
    }
}

impl std::ops::AddAssign for AutomationUsage {
    fn add_assign(&mut self, other: Self) {
        self.accumulate(&other);
    }
}

/// Schema for structured data extraction.
///
/// Define what data to extract from pages with JSON Schema.
#[derive(Debug, Clone, Default, PartialEq)]
#[derive(serde::Serialize, serde::Deserialize)]
pub struct ExtractionSchema {
    /// Name for the schema (e.g., "product_listing").
    pub name: String,
    /// Optional description of what to extract.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// JSON Schema definition as a string.
    pub schema: String,
    /// Whether to enforce strict schema adherence.
    #[serde(default)]
    pub strict: bool,
}

impl ExtractionSchema {
    /// Create a new extraction schema.
    pub fn new(name: impl Into<String>, schema: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: None,
            schema: schema.into(),
            strict: false,
        }
    }

    /// Create with description.
    pub fn with_description(
        name: impl Into<String>,
        description: impl Into<String>,
        schema: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            description: Some(description.into()),
            schema: schema.into(),
            strict: false,
        }
    }

    /// Set strict mode.
    pub fn strict(mut self) -> Self {
        self.strict = true;
        self
    }
}

/// Configuration for structured output mode.
#[derive(Debug, Clone, Default)]
#[derive(serde::Serialize, serde::Deserialize)]
pub struct StructuredOutputConfig {
    /// Enable structured output mode.
    pub enabled: bool,
    /// The JSON schema to enforce.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<serde_json::Value>,
    /// Name for the schema.
    #[serde(default = "default_schema_name")]
    pub schema_name: String,
    /// Whether to use strict mode.
    #[serde(default)]
    pub strict: bool,
}

fn default_schema_name() -> String {
    "response".to_string()
}

impl StructuredOutputConfig {
    /// Create a new structured output config with schema.
    pub fn new(schema: serde_json::Value) -> Self {
        Self {
            enabled: true,
            schema: Some(schema),
            schema_name: "response".to_string(),
            strict: false,
        }
    }

    /// Create with strict mode.
    pub fn strict(schema: serde_json::Value) -> Self {
        Self {
            enabled: true,
            schema: Some(schema),
            schema_name: "response".to_string(),
            strict: true,
        }
    }

    /// Set the schema name.
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.schema_name = name.into();
        self
    }
}

/// Result of an automation operation.
#[derive(Debug, Clone, Default)]
#[derive(serde::Serialize, serde::Deserialize)]
pub struct AutomationResult {
    /// Label for this automation.
    pub label: String,
    /// Number of steps executed.
    pub steps_executed: usize,
    /// Whether automation succeeded.
    pub success: bool,
    /// Error message if failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Token usage.
    #[serde(default)]
    pub usage: AutomationUsage,
    /// Extracted data.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extracted: Option<serde_json::Value>,
    /// Screenshot (base64).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub screenshot: Option<String>,
}

impl AutomationResult {
    /// Create a successful result.
    pub fn success(label: impl Into<String>, steps: usize) -> Self {
        Self {
            label: label.into(),
            steps_executed: steps,
            success: true,
            ..Default::default()
        }
    }

    /// Create a failed result.
    pub fn failure(label: impl Into<String>, error: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            success: false,
            error: Some(error.into()),
            ..Default::default()
        }
    }

    /// Add extracted data.
    pub fn with_extracted(mut self, data: serde_json::Value) -> Self {
        self.extracted = Some(data);
        self
    }

    /// Add screenshot.
    pub fn with_screenshot(mut self, screenshot: impl Into<String>) -> Self {
        self.screenshot = Some(screenshot.into());
        self
    }

    /// Add usage stats.
    pub fn with_usage(mut self, usage: AutomationUsage) -> Self {
        self.usage = usage;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_automation_usage() {
        let mut usage1 = AutomationUsage::new(100, 50);
        assert_eq!(usage1.total_tokens, 150);

        let usage2 = AutomationUsage::new(200, 100);
        usage1.accumulate(&usage2);

        assert_eq!(usage1.prompt_tokens, 300);
        assert_eq!(usage1.completion_tokens, 150);
        assert_eq!(usage1.total_tokens, 450);
    }

    #[test]
    fn test_extraction_schema() {
        let schema = ExtractionSchema::new("products", r#"{"type": "array"}"#).strict();

        assert_eq!(schema.name, "products");
        assert!(schema.strict);
        assert!(schema.description.is_none());
    }

    #[test]
    fn test_automation_result() {
        let result = AutomationResult::success("test", 5)
            .with_extracted(serde_json::json!({"data": "test"}))
            .with_usage(AutomationUsage::new(100, 50));

        assert!(result.success);
        assert_eq!(result.steps_executed, 5);
        assert!(result.extracted.is_some());
    }

    #[test]
    fn test_prompt_url_gate_empty() {
        let gate = PromptUrlGate::new();
        // Empty gate allows all URLs with no override
        assert!(gate.is_allowed("https://example.com"));
        assert!(gate.get_override("https://example.com").is_none());
    }

    #[test]
    fn test_prompt_url_gate_exact_match() {
        let mut gate = PromptUrlGate::new();
        gate.add_override("https://example.com/login", AutomationConfig::new("Login handling"));

        // Exact match returns override
        assert!(gate.is_allowed("https://example.com/login"));
        let override_cfg = gate.get_override("https://example.com/login");
        assert!(override_cfg.is_some());
        assert_eq!(override_cfg.unwrap().goal, "Login handling");

        // Non-matching URL is blocked (map exists but no match)
        assert!(!gate.is_allowed("https://example.com/other"));
    }

    #[test]
    fn test_prompt_url_gate_path_prefix() {
        let mut gate = PromptUrlGate::new().with_paths_map();
        gate.add_override("https://example.com/admin", AutomationConfig::new("Admin handling"));

        // Path prefix match
        assert!(gate.is_allowed("https://example.com/admin/users"));
        assert!(gate.is_allowed("https://example.com/admin"));

        // Non-matching path is blocked
        assert!(!gate.is_allowed("https://example.com/public"));
    }

    #[test]
    fn test_prompt_url_gate_case_insensitive() {
        let mut gate = PromptUrlGate::new().with_paths_map();
        gate.add_override("https://example.com/Admin", AutomationConfig::new("Admin"));

        // Case-insensitive matching
        assert!(gate.is_allowed("https://example.com/admin"));
        assert!(gate.is_allowed("https://example.com/ADMIN"));
        assert!(gate.is_allowed("https://example.com/Admin/Users"));
    }
}
