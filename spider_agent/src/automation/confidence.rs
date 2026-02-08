//! Confidence tracking for LLM-driven automation.
//!
//! This module provides types and utilities for tracking confidence scores
//! in LLM responses, enabling smarter retry decisions and alternative selection.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A step with confidence score and alternatives.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfidentStep {
    /// The primary action to execute.
    pub action: Value,
    /// Confidence score (0.0 to 1.0).
    pub confidence: f64,
    /// Alternative actions ranked by confidence.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alternatives: Vec<Alternative>,
    /// Optional verification to run after the action.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verification: Option<Verification>,
    /// Brief description of the action.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl ConfidentStep {
    /// Create a new confident step.
    pub fn new(action: Value, confidence: f64) -> Self {
        Self {
            action,
            confidence: confidence.clamp(0.0, 1.0),
            alternatives: Vec::new(),
            verification: None,
            description: None,
        }
    }

    /// Add an alternative action.
    pub fn with_alternative(mut self, alt: Alternative) -> Self {
        self.alternatives.push(alt);
        self
    }

    /// Add multiple alternatives.
    pub fn with_alternatives(mut self, alts: Vec<Alternative>) -> Self {
        self.alternatives.extend(alts);
        self
    }

    /// Add a verification step.
    pub fn with_verification(mut self, verification: Verification) -> Self {
        self.verification = Some(verification);
        self
    }

    /// Add a description.
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Check if confidence is above a threshold.
    pub fn is_confident(&self, threshold: f64) -> bool {
        self.confidence >= threshold
    }

    /// Check if alternatives are available.
    pub fn has_alternatives(&self) -> bool {
        !self.alternatives.is_empty()
    }

    /// Get the next best alternative, if any.
    pub fn best_alternative(&self) -> Option<&Alternative> {
        self.alternatives.first()
    }

    /// Get alternatives sorted by confidence (highest first).
    pub fn sorted_alternatives(&self) -> Vec<&Alternative> {
        let mut alts: Vec<_> = self.alternatives.iter().collect();
        alts.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        alts
    }

    /// Parse a confident step from LLM JSON response.
    ///
    /// Expected format:
    /// ```json
    /// {
    ///   "action": { "Click": "button" },
    ///   "confidence": 0.85,
    ///   "alternatives": [
    ///     { "action": { "Click": ".btn" }, "confidence": 0.6 }
    ///   ]
    /// }
    /// ```
    pub fn from_json(value: &Value) -> Option<Self> {
        let action = value.get("action")?.clone();
        let confidence = value
            .get("confidence")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.5);

        let alternatives = value
            .get("alternatives")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(Alternative::from_json).collect())
            .unwrap_or_default();

        let verification = value.get("verification").and_then(Verification::from_json);

        let description = value
            .get("description")
            .and_then(|v| v.as_str())
            .map(String::from);

        Some(Self {
            action,
            confidence: confidence.clamp(0.0, 1.0),
            alternatives,
            verification,
            description,
        })
    }
}

/// An alternative action with its confidence score.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Alternative {
    /// The alternative action.
    pub action: Value,
    /// Confidence score (0.0 to 1.0).
    pub confidence: f64,
    /// Description of why this is an alternative.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl Alternative {
    /// Create a new alternative.
    pub fn new(action: Value, confidence: f64) -> Self {
        Self {
            action,
            confidence: confidence.clamp(0.0, 1.0),
            description: None,
        }
    }

    /// Add a description.
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Parse from JSON.
    pub fn from_json(value: &Value) -> Option<Self> {
        let action = value.get("action")?.clone();
        let confidence = value
            .get("confidence")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.3);
        let description = value
            .get("description")
            .and_then(|v| v.as_str())
            .map(String::from);

        Some(Self {
            action,
            confidence: confidence.clamp(0.0, 1.0),
            description,
        })
    }
}

/// Verification to run after an action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Verification {
    /// Type of verification.
    pub verification_type: VerificationType,
    /// Expected value or condition.
    pub expected: String,
    /// Whether verification failure should trigger retry.
    #[serde(default = "default_true")]
    pub retry_on_failure: bool,
}

fn default_true() -> bool {
    true
}

impl Verification {
    /// Create a URL verification.
    pub fn url_contains(pattern: impl Into<String>) -> Self {
        Self {
            verification_type: VerificationType::UrlContains,
            expected: pattern.into(),
            retry_on_failure: true,
        }
    }

    /// Create an element exists verification.
    pub fn element_exists(selector: impl Into<String>) -> Self {
        Self {
            verification_type: VerificationType::ElementExists,
            expected: selector.into(),
            retry_on_failure: true,
        }
    }

    /// Create a text contains verification.
    pub fn text_contains(text: impl Into<String>) -> Self {
        Self {
            verification_type: VerificationType::TextContains,
            expected: text.into(),
            retry_on_failure: true,
        }
    }

    /// Create a JS condition verification.
    pub fn js_condition(condition: impl Into<String>) -> Self {
        Self {
            verification_type: VerificationType::JsCondition,
            expected: condition.into(),
            retry_on_failure: true,
        }
    }

    /// Parse from JSON.
    pub fn from_json(value: &Value) -> Option<Self> {
        let type_str = value.get("type").and_then(|v| v.as_str())?;
        let expected = value.get("expected").and_then(|v| v.as_str())?.to_string();
        let retry_on_failure = value
            .get("retry_on_failure")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let verification_type = match type_str {
            "url_contains" => VerificationType::UrlContains,
            "element_exists" => VerificationType::ElementExists,
            "text_contains" => VerificationType::TextContains,
            "js_condition" => VerificationType::JsCondition,
            _ => return None,
        };

        Some(Self {
            verification_type,
            expected,
            retry_on_failure,
        })
    }
}

/// Types of verification checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationType {
    /// Check if URL contains a pattern.
    UrlContains,
    /// Check if an element exists.
    ElementExists,
    /// Check if page text contains a string.
    TextContains,
    /// Evaluate a JavaScript condition.
    JsCondition,
}

/// Strategy for retrying based on confidence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfidenceRetryStrategy {
    /// Minimum confidence to accept without retry.
    pub confidence_threshold: f64,
    /// Whether to try alternatives on failure.
    pub use_alternatives: bool,
    /// Maximum alternatives to try.
    pub max_alternatives: usize,
    /// Whether to lower threshold after each retry.
    pub adaptive_threshold: bool,
    /// How much to lower threshold per retry.
    pub threshold_decay: f64,
}

impl Default for ConfidenceRetryStrategy {
    fn default() -> Self {
        Self {
            confidence_threshold: 0.7,
            use_alternatives: true,
            max_alternatives: 3,
            adaptive_threshold: true,
            threshold_decay: 0.1,
        }
    }
}

impl ConfidenceRetryStrategy {
    /// Create a new strategy.
    pub fn new(threshold: f64) -> Self {
        Self {
            confidence_threshold: threshold.clamp(0.0, 1.0),
            ..Default::default()
        }
    }

    /// High confidence strategy (threshold = 0.8).
    pub fn high() -> Self {
        Self::new(0.8)
    }

    /// Medium confidence strategy (threshold = 0.6).
    pub fn medium() -> Self {
        Self::new(0.6)
    }

    /// Low confidence strategy (threshold = 0.4).
    pub fn low() -> Self {
        Self::new(0.4)
    }

    /// Set whether to use alternatives.
    pub fn with_alternatives(mut self, use_alts: bool) -> Self {
        self.use_alternatives = use_alts;
        self
    }

    /// Set maximum alternatives.
    pub fn with_max_alternatives(mut self, max: usize) -> Self {
        self.max_alternatives = max;
        self
    }

    /// Set adaptive threshold.
    pub fn with_adaptive(mut self, adaptive: bool) -> Self {
        self.adaptive_threshold = adaptive;
        self
    }

    /// Get threshold for a given retry attempt.
    pub fn threshold_for_attempt(&self, attempt: usize) -> f64 {
        if self.adaptive_threshold {
            let decay = self.threshold_decay * attempt as f64;
            (self.confidence_threshold - decay).max(0.2)
        } else {
            self.confidence_threshold
        }
    }

    /// Check if a step passes confidence threshold for a given attempt.
    pub fn should_accept(&self, step: &ConfidentStep, attempt: usize) -> bool {
        step.confidence >= self.threshold_for_attempt(attempt)
    }

    /// Check if alternatives should be tried.
    pub fn should_try_alternatives(&self, step: &ConfidentStep, attempt: usize) -> bool {
        self.use_alternatives && step.has_alternatives() && attempt < self.max_alternatives
    }
}

/// Tracker for confidence statistics across an automation session.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConfidenceTracker {
    /// Confidence scores for each step (step_id, confidence).
    step_confidences: Vec<(String, f64)>,
    /// Running sum for average calculation.
    sum: f64,
    /// Minimum confidence seen.
    min: f64,
    /// Maximum confidence seen.
    max: f64,
    /// Number of steps below threshold.
    low_confidence_count: usize,
    /// Threshold for "low confidence" tracking.
    low_threshold: f64,
}

impl ConfidenceTracker {
    /// Create a new tracker.
    pub fn new() -> Self {
        Self {
            step_confidences: Vec::new(),
            sum: 0.0,
            min: 1.0,
            max: 0.0,
            low_confidence_count: 0,
            low_threshold: 0.5,
        }
    }

    /// Create with a custom low threshold.
    pub fn with_low_threshold(threshold: f64) -> Self {
        Self {
            low_threshold: threshold.clamp(0.0, 1.0),
            ..Self::new()
        }
    }

    /// Record a confidence score.
    pub fn record(&mut self, step_id: impl Into<String>, confidence: f64) {
        let conf = confidence.clamp(0.0, 1.0);
        self.step_confidences.push((step_id.into(), conf));
        self.sum += conf;
        self.min = self.min.min(conf);
        self.max = self.max.max(conf);
        if conf < self.low_threshold {
            self.low_confidence_count += 1;
        }
    }

    /// Record from a ConfidentStep.
    pub fn record_step(&mut self, step_id: impl Into<String>, step: &ConfidentStep) {
        self.record(step_id, step.confidence);
    }

    /// Get the average confidence.
    pub fn average(&self) -> f64 {
        if self.step_confidences.is_empty() {
            0.0
        } else {
            self.sum / self.step_confidences.len() as f64
        }
    }

    /// Get the minimum confidence.
    pub fn min(&self) -> f64 {
        if self.step_confidences.is_empty() {
            0.0
        } else {
            self.min
        }
    }

    /// Get the maximum confidence.
    pub fn max(&self) -> f64 {
        self.max
    }

    /// Get the number of steps tracked.
    pub fn count(&self) -> usize {
        self.step_confidences.len()
    }

    /// Get the number of low-confidence steps.
    pub fn low_confidence_count(&self) -> usize {
        self.low_confidence_count
    }

    /// Get the ratio of low-confidence steps.
    pub fn low_confidence_ratio(&self) -> f64 {
        if self.step_confidences.is_empty() {
            0.0
        } else {
            self.low_confidence_count as f64 / self.step_confidences.len() as f64
        }
    }

    /// Get all recorded confidences.
    pub fn confidences(&self) -> &[(String, f64)] {
        &self.step_confidences
    }

    /// Check if overall confidence is healthy.
    pub fn is_healthy(&self) -> bool {
        self.average() >= 0.6 && self.low_confidence_ratio() < 0.3
    }

    /// Get a summary of confidence statistics.
    pub fn summary(&self) -> ConfidenceSummary {
        ConfidenceSummary {
            count: self.count(),
            average: self.average(),
            min: self.min(),
            max: self.max(),
            low_confidence_count: self.low_confidence_count,
            low_confidence_ratio: self.low_confidence_ratio(),
            is_healthy: self.is_healthy(),
        }
    }
}

/// Summary of confidence statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfidenceSummary {
    /// Number of steps tracked.
    pub count: usize,
    /// Average confidence.
    pub average: f64,
    /// Minimum confidence.
    pub min: f64,
    /// Maximum confidence.
    pub max: f64,
    /// Number of low-confidence steps.
    pub low_confidence_count: usize,
    /// Ratio of low-confidence steps.
    pub low_confidence_ratio: f64,
    /// Whether confidence is healthy.
    pub is_healthy: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_confident_step_creation() {
        let step = ConfidentStep::new(serde_json::json!({"Click": "button"}), 0.85)
            .with_description("Click the button")
            .with_alternative(Alternative::new(serde_json::json!({"Click": ".btn"}), 0.6));

        assert_eq!(step.confidence, 0.85);
        assert!(step.is_confident(0.8));
        assert!(!step.is_confident(0.9));
        assert!(step.has_alternatives());
    }

    #[test]
    fn test_confidence_clamping() {
        let step = ConfidentStep::new(serde_json::json!({}), 1.5);
        assert_eq!(step.confidence, 1.0);

        let step = ConfidentStep::new(serde_json::json!({}), -0.5);
        assert_eq!(step.confidence, 0.0);
    }

    #[test]
    fn test_sorted_alternatives() {
        let step = ConfidentStep::new(serde_json::json!({}), 0.9)
            .with_alternative(Alternative::new(serde_json::json!({"a": 1}), 0.3))
            .with_alternative(Alternative::new(serde_json::json!({"b": 2}), 0.7))
            .with_alternative(Alternative::new(serde_json::json!({"c": 3}), 0.5));

        let sorted = step.sorted_alternatives();
        assert_eq!(sorted.len(), 3);
        assert_eq!(sorted[0].confidence, 0.7);
        assert_eq!(sorted[1].confidence, 0.5);
        assert_eq!(sorted[2].confidence, 0.3);
    }

    #[test]
    fn test_verification() {
        let v = Verification::url_contains("/dashboard");
        assert_eq!(v.verification_type, VerificationType::UrlContains);
        assert_eq!(v.expected, "/dashboard");
        assert!(v.retry_on_failure);
    }

    #[test]
    fn test_retry_strategy_threshold() {
        let strategy = ConfidenceRetryStrategy::new(0.8).with_adaptive(true);

        assert!((strategy.threshold_for_attempt(0) - 0.8).abs() < 0.001);
        assert!((strategy.threshold_for_attempt(1) - 0.7).abs() < 0.001);
        assert!((strategy.threshold_for_attempt(2) - 0.6).abs() < 0.001);

        // Should not go below 0.2
        assert!((strategy.threshold_for_attempt(10) - 0.2).abs() < 0.001);
    }

    #[test]
    fn test_retry_strategy_acceptance() {
        let strategy = ConfidenceRetryStrategy::new(0.7);

        let high_conf = ConfidentStep::new(serde_json::json!({}), 0.8);
        let low_conf = ConfidentStep::new(serde_json::json!({}), 0.5);

        assert!(strategy.should_accept(&high_conf, 0));
        assert!(!strategy.should_accept(&low_conf, 0));

        // With adaptive threshold, lower confidence accepted on later attempts
        let adaptive = strategy.with_adaptive(true);
        assert!(adaptive.should_accept(&low_conf, 3)); // threshold = 0.4
    }

    #[test]
    fn test_confidence_tracker() {
        let mut tracker = ConfidenceTracker::new();

        tracker.record("step1", 0.9);
        tracker.record("step2", 0.7);
        tracker.record("step3", 0.4);

        assert_eq!(tracker.count(), 3);
        assert!((tracker.average() - 0.666).abs() < 0.01);
        assert_eq!(tracker.min(), 0.4);
        assert_eq!(tracker.max(), 0.9);
        assert_eq!(tracker.low_confidence_count(), 1);
    }

    #[test]
    fn test_confidence_tracker_health() {
        let mut healthy = ConfidenceTracker::new();
        healthy.record("s1", 0.8);
        healthy.record("s2", 0.9);
        healthy.record("s3", 0.7);
        assert!(healthy.is_healthy());

        let mut unhealthy = ConfidenceTracker::new();
        unhealthy.record("s1", 0.3);
        unhealthy.record("s2", 0.4);
        unhealthy.record("s3", 0.2);
        assert!(!unhealthy.is_healthy());
    }

    #[test]
    fn test_parse_from_json() {
        let json = serde_json::json!({
            "action": { "Click": "button" },
            "confidence": 0.85,
            "alternatives": [
                { "action": { "Click": ".btn" }, "confidence": 0.6 }
            ],
            "verification": {
                "type": "element_exists",
                "expected": ".success"
            }
        });

        let step = ConfidentStep::from_json(&json).expect("valid JSON");
        assert_eq!(step.confidence, 0.85);
        assert_eq!(step.alternatives.len(), 1);
        assert!(step.verification.is_some());
    }

    #[test]
    fn test_confidence_summary() {
        let mut tracker = ConfidenceTracker::new();
        tracker.record("a", 0.8);
        tracker.record("b", 0.6);

        let summary = tracker.summary();
        assert_eq!(summary.count, 2);
        assert_eq!(summary.average, 0.7);
        assert_eq!(summary.min, 0.6);
        assert_eq!(summary.max, 0.8);
    }
}
