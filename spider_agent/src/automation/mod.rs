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
mod chain;
mod config;
mod content;
mod observation;
mod selector_cache;

pub use actions::{ActionRecord, ActionResult, ActionType};
pub use chain::{ChainBuilder, ChainCondition, ChainContext, ChainResult, ChainStep, ChainStepResult};
pub use config::{
    AutomationConfig, CaptureProfile, CleaningIntent, CostTier, HtmlCleaningProfile, ModelPolicy,
    RecoveryStrategy, RetryPolicy,
};
pub use content::ContentAnalysis;
pub use observation::{
    FormField, FormInfo, InteractiveElement, NavigationOption, PageObservation,
};
pub use selector_cache::{SelectorCache, SelectorCacheEntry};

/// Token usage tracking for automation operations.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[derive(serde::Serialize, serde::Deserialize)]
pub struct AutomationUsage {
    /// Prompt tokens used.
    pub prompt_tokens: u32,
    /// Completion tokens used.
    pub completion_tokens: u32,
    /// Total tokens used.
    pub total_tokens: u32,
}

impl AutomationUsage {
    /// Create new usage stats.
    pub fn new(prompt_tokens: u32, completion_tokens: u32) -> Self {
        Self {
            prompt_tokens,
            completion_tokens,
            total_tokens: prompt_tokens + completion_tokens,
        }
    }

    /// Accumulate usage from another instance.
    pub fn accumulate(&mut self, other: &Self) {
        self.prompt_tokens += other.prompt_tokens;
        self.completion_tokens += other.completion_tokens;
        self.total_tokens += other.total_tokens;
    }

    /// Check if any tokens were used.
    pub fn is_empty(&self) -> bool {
        self.total_tokens == 0
    }
}

impl std::ops::Add for AutomationUsage {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self {
            prompt_tokens: self.prompt_tokens + other.prompt_tokens,
            completion_tokens: self.completion_tokens + other.completion_tokens,
            total_tokens: self.total_tokens + other.total_tokens,
        }
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
}
