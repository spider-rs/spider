//! Configuration types for automation.

use std::time::Duration;

/// Recovery strategy for handling failures during automation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[derive(serde::Serialize, serde::Deserialize)]
pub enum RecoveryStrategy {
    /// Retry the same action up to max_retries times.
    #[default]
    Retry,
    /// Try an alternative approach (re-query LLM for different solution).
    Alternative,
    /// Skip the failed step and continue with the next action.
    Skip,
    /// Abort the entire execution on failure.
    Abort,
}

/// Retry policy for automation operations.
#[derive(Debug, Clone, PartialEq, Eq)]
#[derive(serde::Serialize, serde::Deserialize)]
pub struct RetryPolicy {
    /// Maximum number of attempts.
    pub max_attempts: usize,
    /// Backoff delay between attempts in milliseconds.
    pub backoff_ms: u64,
    /// Whether to retry on JSON parse errors.
    pub retry_on_parse_error: bool,
    /// Whether to retry on step failures.
    pub retry_on_step_failure: bool,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            backoff_ms: 1000,
            retry_on_parse_error: true,
            retry_on_step_failure: true,
        }
    }
}

impl RetryPolicy {
    /// Create a new retry policy.
    pub fn new(max_attempts: usize) -> Self {
        Self {
            max_attempts,
            ..Default::default()
        }
    }

    /// No retries.
    pub fn none() -> Self {
        Self {
            max_attempts: 1,
            backoff_ms: 0,
            retry_on_parse_error: false,
            retry_on_step_failure: false,
        }
    }

    /// Set backoff delay.
    pub fn with_backoff(mut self, ms: u64) -> Self {
        self.backoff_ms = ms;
        self
    }

    /// Get backoff duration.
    pub fn backoff_duration(&self) -> Duration {
        Duration::from_millis(self.backoff_ms)
    }
}

/// Cost tier for model selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[derive(serde::Serialize, serde::Deserialize)]
pub enum CostTier {
    /// Prefer cheaper/faster models.
    Low,
    /// Balanced cost/quality.
    #[default]
    Medium,
    /// Prefer higher quality models.
    High,
}

/// Policy for selecting models based on cost/quality tradeoffs.
#[derive(Debug, Clone)]
#[derive(serde::Serialize, serde::Deserialize)]
pub struct ModelPolicy {
    /// Small/cheap model identifier.
    pub small: String,
    /// Medium model identifier.
    pub medium: String,
    /// Large/expensive model identifier.
    pub large: String,
    /// Whether large model is allowed.
    pub allow_large: bool,
    /// Maximum latency budget in ms.
    pub max_latency_ms: Option<u64>,
    /// Maximum cost tier allowed.
    pub max_cost_tier: CostTier,
}

impl Default for ModelPolicy {
    fn default() -> Self {
        Self {
            small: "gpt-4o-mini".to_string(),
            medium: "gpt-4o".to_string(),
            large: "gpt-4o".to_string(),
            allow_large: true,
            max_latency_ms: None,
            max_cost_tier: CostTier::Medium,
        }
    }
}

impl ModelPolicy {
    /// Get the appropriate model for the given cost tier.
    pub fn model_for_tier(&self, tier: CostTier) -> &str {
        match tier {
            CostTier::Low => &self.small,
            CostTier::Medium => &self.medium,
            CostTier::High if self.allow_large => &self.large,
            CostTier::High => &self.medium,
        }
    }

    /// Set small model.
    pub fn with_small(mut self, model: impl Into<String>) -> Self {
        self.small = model.into();
        self
    }

    /// Set medium model.
    pub fn with_medium(mut self, model: impl Into<String>) -> Self {
        self.medium = model.into();
        self
    }

    /// Set large model.
    pub fn with_large(mut self, model: impl Into<String>) -> Self {
        self.large = model.into();
        self
    }
}

/// HTML cleaning profile for content processing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[derive(serde::Serialize, serde::Deserialize)]
pub enum HtmlCleaningProfile {
    /// Standard cleaning - removes scripts, styles, comments.
    #[default]
    Default,
    /// Aggressive cleaning - heavy cleanup for extraction.
    Aggressive,
    /// Slim cleaning - removes SVGs, canvas, heavy nodes.
    Slim,
    /// Minimal cleaning - preserve interactivity.
    Minimal,
    /// No cleaning - raw HTML.
    Raw,
    /// Auto-select based on content analysis.
    Auto,
}

/// Intent for HTML cleaning decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[derive(serde::Serialize, serde::Deserialize)]
pub enum CleaningIntent {
    /// General purpose - balanced cleaning.
    #[default]
    General,
    /// Extraction focused - aggressive, text-only.
    Extraction,
    /// Action focused - preserve interactivity.
    Action,
}

/// Capture profile for screenshots and HTML.
#[derive(Debug, Clone)]
#[derive(serde::Serialize, serde::Deserialize)]
pub struct CaptureProfile {
    /// Capture full page screenshot.
    pub full_page: bool,
    /// Omit background in screenshot.
    pub omit_background: bool,
    /// Clip viewport for screenshot.
    pub clip: Option<ClipViewport>,
    /// HTML cleaning profile.
    pub html_cleaning: HtmlCleaningProfile,
    /// Maximum HTML bytes to capture.
    pub html_max_bytes: usize,
    /// Note about this capture attempt.
    pub attempt_note: Option<String>,
}

impl Default for CaptureProfile {
    fn default() -> Self {
        Self {
            full_page: true,
            omit_background: true,
            clip: None,
            html_cleaning: HtmlCleaningProfile::Default,
            html_max_bytes: 24_000,
            attempt_note: None,
        }
    }
}

impl CaptureProfile {
    /// Create a profile for extraction (aggressive cleaning).
    pub fn for_extraction() -> Self {
        Self {
            html_cleaning: HtmlCleaningProfile::Aggressive,
            ..Default::default()
        }
    }

    /// Create a profile for actions (preserve interactivity).
    pub fn for_action() -> Self {
        Self {
            html_cleaning: HtmlCleaningProfile::Minimal,
            full_page: false,
            ..Default::default()
        }
    }

    /// Set HTML max bytes.
    pub fn with_max_bytes(mut self, bytes: usize) -> Self {
        self.html_max_bytes = bytes;
        self
    }
}

/// Clip viewport for screenshots.
#[derive(Debug, Clone, Copy, PartialEq)]
#[derive(serde::Serialize, serde::Deserialize)]
pub struct ClipViewport {
    /// X coordinate.
    pub x: f64,
    /// Y coordinate.
    pub y: f64,
    /// Width.
    pub width: f64,
    /// Height.
    pub height: f64,
}

impl ClipViewport {
    /// Create a new clip viewport.
    pub fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

/// Main automation configuration.
#[derive(Debug, Clone)]
#[derive(serde::Serialize, serde::Deserialize)]
pub struct AutomationConfig {
    /// The goal to achieve.
    pub goal: String,
    /// Maximum number of steps before stopping.
    pub max_steps: usize,
    /// Timeout in milliseconds.
    pub timeout_ms: u64,
    /// Recovery strategy for failures.
    pub recovery_strategy: RecoveryStrategy,
    /// Maximum retries per step.
    pub max_retries: usize,
    /// Whether to use selector cache.
    pub use_cache: bool,
    /// Whether to capture screenshots after each step.
    pub capture_screenshots: bool,
    /// URLs that indicate success.
    pub success_urls: Vec<String>,
    /// Text patterns that indicate success.
    pub success_patterns: Vec<String>,
    /// Whether to extract data on success.
    pub extract_on_success: bool,
    /// Extraction prompt for final data.
    pub extraction_prompt: Option<String>,
    /// Capture profile.
    pub capture_profile: CaptureProfile,
    /// Retry policy.
    pub retry_policy: RetryPolicy,
    /// Model policy.
    pub model_policy: ModelPolicy,
}

impl Default for AutomationConfig {
    fn default() -> Self {
        Self {
            goal: String::new(),
            max_steps: 20,
            timeout_ms: 120_000, // 2 minutes
            recovery_strategy: RecoveryStrategy::Retry,
            max_retries: 3,
            use_cache: true,
            capture_screenshots: true,
            success_urls: Vec::new(),
            success_patterns: Vec::new(),
            extract_on_success: false,
            extraction_prompt: None,
            capture_profile: CaptureProfile::default(),
            retry_policy: RetryPolicy::default(),
            model_policy: ModelPolicy::default(),
        }
    }
}

impl AutomationConfig {
    /// Create a new config with a goal.
    pub fn new(goal: impl Into<String>) -> Self {
        Self {
            goal: goal.into(),
            ..Default::default()
        }
    }

    /// Set maximum steps.
    pub fn with_max_steps(mut self, steps: usize) -> Self {
        self.max_steps = steps;
        self
    }

    /// Set timeout in milliseconds.
    pub fn with_timeout(mut self, ms: u64) -> Self {
        self.timeout_ms = ms;
        self
    }

    /// Set recovery strategy.
    pub fn with_recovery(mut self, strategy: RecoveryStrategy) -> Self {
        self.recovery_strategy = strategy;
        self
    }

    /// Set max retries.
    pub fn with_retries(mut self, retries: usize) -> Self {
        self.max_retries = retries;
        self
    }

    /// Enable/disable selector cache.
    pub fn with_cache(mut self, enabled: bool) -> Self {
        self.use_cache = enabled;
        self
    }

    /// Enable/disable screenshots.
    pub fn with_screenshots(mut self, enabled: bool) -> Self {
        self.capture_screenshots = enabled;
        self
    }

    /// Add a success URL pattern.
    pub fn with_success_url(mut self, url: impl Into<String>) -> Self {
        self.success_urls.push(url.into());
        self
    }

    /// Add a success text pattern.
    pub fn with_success_pattern(mut self, pattern: impl Into<String>) -> Self {
        self.success_patterns.push(pattern.into());
        self
    }

    /// Enable extraction on success.
    pub fn with_extraction(mut self, prompt: impl Into<String>) -> Self {
        self.extract_on_success = true;
        self.extraction_prompt = Some(prompt.into());
        self
    }

    /// Set capture profile.
    pub fn with_capture_profile(mut self, profile: CaptureProfile) -> Self {
        self.capture_profile = profile;
        self
    }

    /// Set retry policy.
    pub fn with_retry_policy(mut self, policy: RetryPolicy) -> Self {
        self.retry_policy = policy;
        self
    }

    /// Set model policy.
    pub fn with_model_policy(mut self, policy: ModelPolicy) -> Self {
        self.model_policy = policy;
        self
    }

    /// Get timeout as Duration.
    pub fn timeout_duration(&self) -> std::time::Duration {
        std::time::Duration::from_millis(self.timeout_ms)
    }

    /// Check if a URL matches success criteria.
    pub fn is_success_url(&self, url: &str) -> bool {
        self.success_urls.iter().any(|pattern| url.contains(pattern))
    }

    /// Check if text matches success criteria.
    pub fn matches_success_pattern(&self, text: &str) -> bool {
        self.success_patterns
            .iter()
            .any(|pattern| text.contains(pattern))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_automation_config() {
        let config = AutomationConfig::new("Login to dashboard")
            .with_max_steps(10)
            .with_timeout(60_000)
            .with_success_url("/dashboard")
            .with_extraction("Extract user info");

        assert_eq!(config.goal, "Login to dashboard");
        assert_eq!(config.max_steps, 10);
        assert!(config.extract_on_success);
        assert!(config.is_success_url("https://example.com/dashboard"));
    }

    #[test]
    fn test_retry_policy() {
        let policy = RetryPolicy::new(5).with_backoff(2000);

        assert_eq!(policy.max_attempts, 5);
        assert_eq!(policy.backoff_ms, 2000);
        assert_eq!(policy.backoff_duration(), Duration::from_millis(2000));
    }

    #[test]
    fn test_model_policy() {
        let policy = ModelPolicy::default();

        assert_eq!(policy.model_for_tier(CostTier::Low), "gpt-4o-mini");
        assert_eq!(policy.model_for_tier(CostTier::High), "gpt-4o");
    }
}
