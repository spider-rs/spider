//! Configuration types for automation.
//!
//! Contains all configuration types for remote multimodal automation,
//! including runtime configs, retry policies, model selection, and capture profiles.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use super::ContentAnalysis;

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
#[derive(Debug, Clone, PartialEq)]
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
            max_cost_tier: CostTier::High,
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

impl HtmlCleaningProfile {
    // Size thresholds for smart cleaning decisions (in bytes).
    const SVG_HEAVY_THRESHOLD: usize = 50_000; // 50KB of SVG is heavy
    const SVG_VERY_HEAVY_THRESHOLD: usize = 100_000; // 100KB of SVG is very heavy
    const BASE64_HEAVY_THRESHOLD: usize = 100_000; // 100KB of base64 data
    const SCRIPT_HEAVY_THRESHOLD: usize = 200_000; // 200KB of scripts
    const CLEANABLE_RATIO_HIGH: f32 = 0.4; // 40% of HTML is cleanable
    const CLEANABLE_RATIO_MEDIUM: f32 = 0.25; // 25% of HTML is cleanable

    /// Determine the best cleaning profile based on content analysis.
    ///
    /// This is used when `Auto` is selected to intelligently choose
    /// the appropriate cleaning level based on the HTML content.
    pub fn from_content_analysis(analysis: &ContentAnalysis) -> Self {
        Self::from_content_analysis_with_intent(analysis, CleaningIntent::General)
    }

    /// Determine the best cleaning profile based on content analysis and intended use.
    ///
    /// Uses **byte sizes** (not just counts) for accurate decisions:
    /// - SVG > 100KB → always Slim
    /// - base64 > 100KB → always Slim
    /// - cleanable_ratio > 40% → Slim
    ///
    /// Intent modifies behavior:
    /// - `Extraction` → more aggressive, removes nav/footer/heavy elements
    /// - `Action` → preserves interactive elements (buttons, forms, links)
    /// - `General` → balanced heuristics
    pub fn from_content_analysis_with_intent(
        analysis: &ContentAnalysis,
        intent: CleaningIntent,
    ) -> Self {
        // === BYTE-SIZE BASED DECISIONS (more accurate than counts!) ===

        // Very heavy SVGs - always slim regardless of intent
        if analysis.svg_bytes > Self::SVG_VERY_HEAVY_THRESHOLD {
            return HtmlCleaningProfile::Slim;
        }

        // Very heavy base64 data (inline images, fonts) - always slim
        if analysis.base64_bytes > Self::BASE64_HEAVY_THRESHOLD {
            return HtmlCleaningProfile::Slim;
        }

        // High cleanable ratio means lots of bloat - slim is worthwhile
        if analysis.cleanable_ratio > Self::CLEANABLE_RATIO_HIGH {
            return HtmlCleaningProfile::Slim;
        }

        match intent {
            CleaningIntent::Extraction => {
                // For extraction, we can be aggressive - we only need text content

                // Heavy SVGs (50KB+) - slim them out
                if analysis.svg_bytes > Self::SVG_HEAVY_THRESHOLD {
                    return HtmlCleaningProfile::Slim;
                }

                // Large HTML with lots of scripts - aggressive
                if analysis.script_bytes > Self::SCRIPT_HEAVY_THRESHOLD {
                    return HtmlCleaningProfile::Aggressive;
                }

                // Large HTML overall
                if analysis.html_length > 100_000 {
                    return HtmlCleaningProfile::Aggressive;
                }

                // Medium cleanable ratio - slim is beneficial
                if analysis.cleanable_ratio > Self::CLEANABLE_RATIO_MEDIUM {
                    return HtmlCleaningProfile::Slim;
                }

                // Canvas/video/embeds present - slim
                if analysis.canvas_count > 0
                    || analysis.video_count > 1
                    || analysis.embed_count > 0
                {
                    return HtmlCleaningProfile::Slim;
                }

                // Low text ratio with medium+ HTML - aggressive
                if analysis.text_ratio < 0.1 && analysis.html_length > 30_000 {
                    return HtmlCleaningProfile::Aggressive;
                }

                // Default to slim for extraction (safe choice)
                HtmlCleaningProfile::Slim
            }
            CleaningIntent::Action => {
                // For actions, preserve interactive elements but remove heavy visual bloat

                // Heavy SVGs - slim (they're not interactive)
                if analysis.svg_bytes > Self::SVG_HEAVY_THRESHOLD {
                    return HtmlCleaningProfile::Slim;
                }

                // Medium cleanable ratio - default cleaning preserves interactivity
                if analysis.cleanable_ratio > Self::CLEANABLE_RATIO_MEDIUM {
                    return HtmlCleaningProfile::Default;
                }

                // Very large HTML - need some cleaning
                if analysis.html_length > 150_000 {
                    return HtmlCleaningProfile::Default;
                }

                // Keep minimal to preserve interactive elements
                HtmlCleaningProfile::Minimal
            }
            CleaningIntent::General => {
                // Balanced approach based on content characteristics

                // Heavy SVGs - slim
                if analysis.svg_bytes > Self::SVG_HEAVY_THRESHOLD {
                    return HtmlCleaningProfile::Slim;
                }

                // Medium cleanable ratio - slim is beneficial
                if analysis.cleanable_ratio > Self::CLEANABLE_RATIO_MEDIUM {
                    return HtmlCleaningProfile::Slim;
                }

                // Canvas/video present - slim
                if analysis.canvas_count > 0 || analysis.video_count > 2 {
                    return HtmlCleaningProfile::Slim;
                }

                // Low text ratio with large HTML - aggressive
                if analysis.text_ratio < 0.05 && analysis.html_length > 50_000 {
                    return HtmlCleaningProfile::Aggressive;
                }

                // Embeds present - slim
                if analysis.embed_count > 0 {
                    return HtmlCleaningProfile::Slim;
                }

                // Large HTML with moderate text - default
                if analysis.html_length > 100_000 && analysis.text_ratio < 0.15 {
                    return HtmlCleaningProfile::Default;
                }

                // Medium HTML - default
                if analysis.html_length > 30_000 {
                    return HtmlCleaningProfile::Default;
                }

                // Small HTML - minimal cleaning
                HtmlCleaningProfile::Minimal
            }
        }
    }

    /// Quick check if this profile removes SVGs.
    pub fn removes_svgs(&self) -> bool {
        matches!(self, HtmlCleaningProfile::Slim | HtmlCleaningProfile::Aggressive)
    }

    /// Quick check if this profile removes video/canvas elements.
    pub fn removes_media(&self) -> bool {
        matches!(self, HtmlCleaningProfile::Slim)
    }

    /// Estimate bytes that will be removed by this cleaning profile.
    pub fn estimate_savings(&self, analysis: &ContentAnalysis) -> usize {
        match self {
            HtmlCleaningProfile::Raw => 0,
            HtmlCleaningProfile::Minimal => analysis.script_bytes + analysis.style_bytes,
            HtmlCleaningProfile::Default => {
                analysis.script_bytes + analysis.style_bytes + (analysis.base64_bytes / 2)
            }
            HtmlCleaningProfile::Slim => analysis.cleanable_bytes,
            HtmlCleaningProfile::Aggressive => {
                // Aggressive also removes nav/footer, estimate ~10% more
                analysis.cleanable_bytes + (analysis.html_length / 10)
            }
            HtmlCleaningProfile::Auto => 0, // Can't estimate without analyzing
        }
    }
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
#[derive(Debug, Clone, PartialEq)]
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
#[derive(Debug, Clone, PartialEq)]
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
    /// Optional system prompt for automation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    /// Optional extra system instructions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt_extra: Option<String>,
    /// Optional extra user message instructions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_message_extra: Option<String>,
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
            system_prompt: None,
            system_prompt_extra: None,
            user_message_extra: None,
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

    /// Set system prompt.
    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(prompt.into());
        self
    }

    /// Set extra system instructions.
    pub fn with_system_prompt_extra(mut self, extra: impl Into<String>) -> Self {
        self.system_prompt_extra = Some(extra.into());
        self
    }

    /// Set extra user message instructions.
    pub fn with_user_message_extra(mut self, extra: impl Into<String>) -> Self {
        self.user_message_extra = Some(extra.into());
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

// =============================================================================
// REMOTE MULTIMODAL ENGINE CONFIGURATION
// =============================================================================

/// Runtime configuration for `RemoteMultimodalEngine`.
///
/// This struct controls:
/// 1) what context is captured (URL/title/HTML),
/// 2) how chat completion is requested (temperature/max tokens/JSON mode),
/// 3) how long the engine loops and retries,
/// 4) capture/model selection policies.
///
/// The engine should be able to **export this config** to users, and it should
/// be safe to merge with user-provided prompts.
#[derive(Debug, Clone, PartialEq)]
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct RemoteMultimodalConfig {
    // -----------------------------------------------------------------
    // Context capture
    // -----------------------------------------------------------------
    /// Whether to include cleaned HTML in the model input.
    pub include_html: bool,
    /// Maximum number of bytes of cleaned HTML to include (global default).
    ///
    /// A `CaptureProfile` may override this with its own `html_max_bytes`.
    pub html_max_bytes: usize,
    /// Whether to include the current URL in the model input.
    pub include_url: bool,
    /// Whether to include the current document title in the model input.
    pub include_title: bool,
    /// Whether to include screenshots in the LLM request.
    ///
    /// When `None` (default), automatically detects based on model name.
    /// Vision models (gpt-4o, claude-3, etc.) will receive screenshots,
    /// while text-only models will not.
    ///
    /// Set to `Some(true)` to always include screenshots.
    /// Set to `Some(false)` to never include screenshots.
    pub include_screenshot: Option<bool>,

    // -----------------------------------------------------------------
    // LLM knobs
    // -----------------------------------------------------------------
    /// Sampling temperature used by the remote/local model.
    pub temperature: f32,
    /// Maximum tokens the model is allowed to generate for the plan.
    pub max_tokens: u16,
    /// If true, include `response_format: {"type":"json_object"}` in the request.
    ///
    /// Some local servers ignore or reject this; disable if you see 400 errors.
    pub request_json_object: bool,
    /// Best-effort JSON extraction (strip fences / extract `{...}`).
    pub best_effort_json_extract: bool,

    // -----------------------------------------------------------------
    // Loop + retry
    // -----------------------------------------------------------------
    /// Maximum number of plan/execute/re-capture rounds before giving up.
    ///
    /// Each round is:
    /// 1) capture state
    /// 2) ask model for plan
    /// 3) execute steps
    /// 4) optionally wait
    /// 5) re-capture and decide whether complete
    pub max_rounds: usize,

    /// Retry policy for model output parsing failures and/or execution failures.
    pub retry: RetryPolicy,

    // -----------------------------------------------------------------
    // Capture / model policies
    // -----------------------------------------------------------------
    /// Capture profiles to try across attempts.
    ///
    /// If empty, the engine should build a sensible default list.
    pub capture_profiles: Vec<CaptureProfile>,

    /// Model selection policy (small/medium/large).
    ///
    /// The engine may choose a model size depending on constraints such as
    /// latency limits, cost tier, and whether retries are escalating.
    pub model_policy: ModelPolicy,

    /// Optional: wait after executing a plan before re-capturing state (ms).
    ///
    /// This is useful for pages that animate, load asynchronously, or perform
    /// challenge transitions after clicks.
    pub post_plan_wait_ms: u64,
    /// Maximum number of concurrent LLM HTTP requests for this engine instance.
    /// If `None`, no throttling is applied.
    pub max_inflight_requests: Option<usize>,

    // -----------------------------------------------------------------
    // Extraction
    // -----------------------------------------------------------------
    /// Enable extraction mode to return structured data from pages.
    ///
    /// When enabled, the model is instructed to include an `extracted` field
    /// in its JSON response containing data extracted from the page.
    pub extra_ai_data: bool,
    /// Optional custom extraction prompt appended to the system prompt.
    ///
    /// Example: "Extract all product names and prices as a JSON array."
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extraction_prompt: Option<String>,
    /// Optional JSON schema for structured extraction output.
    ///
    /// When provided, the model is instructed to return the `extracted` field
    /// conforming to this schema. This enables type-safe extraction.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extraction_schema: Option<super::ExtractionSchema>,
    /// Take a screenshot after automation completes and include it in results.
    pub screenshot: bool,

    // -----------------------------------------------------------------
    // Claude-optimized features
    // -----------------------------------------------------------------
    /// Tool calling mode for structured action output.
    ///
    /// - `JsonObject` (default): Use JSON object mode
    /// - `ToolCalling`: Use OpenAI-compatible tool/function calling
    /// - `Auto`: Auto-select based on model capabilities
    #[serde(default)]
    pub tool_calling_mode: super::tool_calling::ToolCallingMode,

    /// HTML diff mode for condensed page state.
    ///
    /// When enabled, sends only HTML changes after the first round,
    /// potentially reducing tokens by 50-70%.
    #[serde(default)]
    pub html_diff_mode: super::html_diff::HtmlDiffMode,

    /// Planning mode configuration.
    ///
    /// When enabled, allows the LLM to plan multiple steps upfront,
    /// reducing round-trips. Set to `None` to disable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub planning_mode: Option<super::planning::PlanningModeConfig>,

    /// Multi-page synthesis configuration.
    ///
    /// When configured, enables analyzing multiple pages in a single
    /// LLM call. Set to `None` to disable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub synthesis_config: Option<super::synthesis::SynthesisConfig>,

    /// Confidence-based retry strategy.
    ///
    /// When configured, uses confidence scores to make smarter retry
    /// decisions. Set to `None` for default retry behavior.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence_strategy: Option<super::confidence::ConfidenceRetryStrategy>,

    /// Self-healing configuration for automatic selector repair.
    ///
    /// When enabled, failed selectors trigger an LLM call to diagnose
    /// and suggest alternatives. Set to `None` to disable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub self_healing: Option<super::self_healing::SelfHealingConfig>,

    /// Enable concurrent execution of independent actions.
    ///
    /// When true, actions without dependencies can run in parallel
    /// using `tokio::JoinSet`.
    #[serde(default)]
    pub concurrent_execution: bool,
}

impl Default for RemoteMultimodalConfig {
    fn default() -> Self {
        Self {
            include_html: true,
            html_max_bytes: 24_000,
            include_url: true,
            include_title: true,
            include_screenshot: None, // Auto-detect based on model
            temperature: 0.1,
            max_tokens: 1024,
            request_json_object: true,
            best_effort_json_extract: true,
            max_rounds: 6,
            retry: RetryPolicy::default(),
            model_policy: ModelPolicy::default(),
            capture_profiles: Vec::new(),
            post_plan_wait_ms: 350,
            max_inflight_requests: None,
            extra_ai_data: false,
            extraction_prompt: None,
            extraction_schema: None,
            screenshot: true,
            // Claude-optimized features (all disabled by default for backward compatibility)
            tool_calling_mode: super::tool_calling::ToolCallingMode::default(),
            html_diff_mode: super::html_diff::HtmlDiffMode::default(),
            planning_mode: None,
            synthesis_config: None,
            confidence_strategy: None,
            self_healing: None,
            concurrent_execution: false,
        }
    }
}

impl RemoteMultimodalConfig {
    /// Create a new config with default settings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a config optimized for maximum speed and efficiency.
    ///
    /// Enables all performance-positive features:
    /// - `ToolCallingMode::Auto` for reliable action parsing
    /// - `HtmlDiffMode::Auto` for 50-70% token reduction
    /// - `ConfidenceRetryStrategy` for smarter retries
    /// - `concurrent_execution` for parallel action execution
    ///
    /// These features have zero or positive performance impact.
    pub fn fast() -> Self {
        Self {
            tool_calling_mode: super::tool_calling::ToolCallingMode::Auto,
            html_diff_mode: super::html_diff::HtmlDiffMode::Auto,
            confidence_strategy: Some(super::confidence::ConfidenceRetryStrategy::default()),
            concurrent_execution: true,
            ..Self::default()
        }
    }

    /// Create a config optimized for maximum speed with planning enabled.
    ///
    /// Includes all `fast()` features plus:
    /// - `PlanningModeConfig` for multi-step planning (fewer round-trips)
    /// - `SelfHealingConfig` for auto-repair of failed selectors
    ///
    /// Best for complex multi-step automations.
    pub fn fast_with_planning() -> Self {
        Self {
            planning_mode: Some(super::planning::PlanningModeConfig::default()),
            self_healing: Some(super::self_healing::SelfHealingConfig::default()),
            ..Self::fast()
        }
    }

    /// Set whether to include HTML.
    pub fn with_html(mut self, include: bool) -> Self {
        self.include_html = include;
        self
    }

    /// Set maximum HTML bytes.
    pub fn with_html_max_bytes(mut self, bytes: usize) -> Self {
        self.html_max_bytes = bytes;
        self
    }

    /// Set temperature.
    pub fn with_temperature(mut self, temp: f32) -> Self {
        self.temperature = temp;
        self
    }

    /// Set max tokens.
    pub fn with_max_tokens(mut self, tokens: u16) -> Self {
        self.max_tokens = tokens;
        self
    }

    /// Set max rounds.
    pub fn with_max_rounds(mut self, rounds: usize) -> Self {
        self.max_rounds = rounds;
        self
    }

    /// Set retry policy.
    pub fn with_retry(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    /// Set model policy.
    pub fn with_model_policy(mut self, policy: ModelPolicy) -> Self {
        self.model_policy = policy;
        self
    }

    /// Enable extraction mode.
    pub fn with_extraction(mut self, enabled: bool) -> Self {
        self.extra_ai_data = enabled;
        self
    }

    /// Set extraction prompt.
    pub fn with_extraction_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.extraction_prompt = Some(prompt.into());
        self
    }

    /// Set extraction schema.
    pub fn with_extraction_schema(mut self, schema: super::ExtractionSchema) -> Self {
        self.extraction_schema = Some(schema);
        self
    }

    /// Enable/disable screenshots.
    pub fn with_screenshot(mut self, enabled: bool) -> Self {
        self.screenshot = enabled;
        self
    }

    /// Set whether to include screenshots in LLM requests.
    ///
    /// - `Some(true)`: Always include screenshots
    /// - `Some(false)`: Never include screenshots
    /// - `None`: Auto-detect based on model name (default)
    pub fn with_include_screenshot(mut self, include: Option<bool>) -> Self {
        self.include_screenshot = include;
        self
    }

    /// Add a capture profile.
    pub fn add_capture_profile(&mut self, profile: CaptureProfile) {
        self.capture_profiles.push(profile);
    }

    // -------------------------------------------------------------------------
    // Claude-optimized feature builders
    // -------------------------------------------------------------------------

    /// Set tool calling mode.
    pub fn with_tool_calling_mode(mut self, mode: super::tool_calling::ToolCallingMode) -> Self {
        self.tool_calling_mode = mode;
        self
    }

    /// Set HTML diff mode for condensed page state.
    pub fn with_html_diff_mode(mut self, mode: super::html_diff::HtmlDiffMode) -> Self {
        self.html_diff_mode = mode;
        self
    }

    /// Enable planning mode with configuration.
    pub fn with_planning_mode(mut self, config: super::planning::PlanningModeConfig) -> Self {
        self.planning_mode = Some(config);
        self
    }

    /// Enable multi-page synthesis with configuration.
    pub fn with_synthesis_config(mut self, config: super::synthesis::SynthesisConfig) -> Self {
        self.synthesis_config = Some(config);
        self
    }

    /// Set confidence-based retry strategy.
    pub fn with_confidence_strategy(
        mut self,
        strategy: super::confidence::ConfidenceRetryStrategy,
    ) -> Self {
        self.confidence_strategy = Some(strategy);
        self
    }

    /// Enable self-healing with configuration.
    pub fn with_self_healing(mut self, config: super::self_healing::SelfHealingConfig) -> Self {
        self.self_healing = Some(config);
        self
    }

    /// Enable/disable concurrent execution of independent actions.
    pub fn with_concurrent_execution(mut self, enabled: bool) -> Self {
        self.concurrent_execution = enabled;
        self
    }
}

/// Everything needed to configure RemoteMultimodalEngine.
///
/// This is the complete configuration bundle that includes:
/// - API endpoint and credentials
/// - Model selection
/// - System/user prompts
/// - Runtime configuration
/// - URL gating
#[derive(Debug, Clone)]
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct RemoteMultimodalConfigs {
    /// OpenAI-compatible chat completions URL.
    pub api_url: String,
    /// Optional bearer key for `Authorization: Bearer ...`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// Model name/id for the target endpoint.
    pub model_name: String,
    /// Optional base system prompt (None => engine default).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    /// Optional extra system instructions appended at runtime.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt_extra: Option<String>,
    /// Optional extra user instructions appended at runtime.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_message_extra: Option<String>,
    /// Runtime knobs (capture policies, retry, looping, etc.)
    pub cfg: RemoteMultimodalConfig,
    /// Optional URL gating and per-URL overrides.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_url_gate: Option<super::PromptUrlGate>,
    /// Optional concurrency limit for remote inference calls.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub concurrency_limit: Option<usize>,
    /// Semaphore control for concurrency limiting.
    #[serde(skip, default = "RemoteMultimodalConfigs::default_semaphore")]
    pub semaphore: OnceLock<Arc<tokio::sync::Semaphore>>,
}

impl PartialEq for RemoteMultimodalConfigs {
    fn eq(&self, other: &Self) -> bool {
        self.api_url == other.api_url
            && self.api_key == other.api_key
            && self.model_name == other.model_name
            && self.system_prompt == other.system_prompt
            && self.system_prompt_extra == other.system_prompt_extra
            && self.user_message_extra == other.user_message_extra
            && self.cfg == other.cfg
            && self.prompt_url_gate == other.prompt_url_gate
            && self.concurrency_limit == other.concurrency_limit
        // NOTE: intentionally ignoring `semaphore`
    }
}

impl Eq for RemoteMultimodalConfigs {}

impl Default for RemoteMultimodalConfigs {
    fn default() -> Self {
        Self {
            api_url: String::new(),
            api_key: None,
            model_name: String::new(),
            system_prompt: None,
            system_prompt_extra: None,
            user_message_extra: None,
            cfg: RemoteMultimodalConfig::default(),
            prompt_url_gate: None,
            concurrency_limit: None,
            semaphore: Self::default_semaphore(),
        }
    }
}

impl RemoteMultimodalConfigs {
    /// Create a new remote multimodal config bundle.
    ///
    /// This sets the minimum required fields:
    /// - `api_url`: the OpenAI-compatible `/v1/chat/completions` endpoint
    /// - `model_name`: the model identifier understood by that endpoint
    ///
    /// All other fields fall back to [`Default::default`].
    ///
    /// # Example
    /// ```rust
    /// use spider_agent::automation::RemoteMultimodalConfigs;
    ///
    /// let mm = RemoteMultimodalConfigs::new(
    ///     "http://localhost:11434/v1/chat/completions",
    ///     "qwen2.5-vl",
    /// );
    /// ```
    pub fn new(api_url: impl Into<String>, model_name: impl Into<String>) -> Self {
        Self {
            api_url: api_url.into(),
            model_name: model_name.into(),
            ..Default::default()
        }
    }

    /// Default semaphore.
    fn default_semaphore() -> OnceLock<Arc<tokio::sync::Semaphore>> {
        OnceLock::new()
    }

    /// Get (and lazily init) the shared semaphore from `concurrency_limit`.
    /// This is safe to call concurrently; `OnceLock` handles the race.
    pub fn get_or_init_semaphore(&self) -> Option<Arc<tokio::sync::Semaphore>> {
        let n = self.concurrency_limit?;
        if n == 0 {
            return None;
        }
        Some(
            self.semaphore
                .get_or_init(|| Arc::new(tokio::sync::Semaphore::new(n)))
                .clone(),
        )
    }

    /// Attach an optional API key for authenticated endpoints.
    ///
    /// When set, the engine will send:
    /// `Authorization: Bearer <api_key>`
    pub fn with_api_key(mut self, key: impl Into<String>) -> Self {
        self.api_key = Some(key.into());
        self
    }

    /// Set the base system prompt for the model.
    ///
    /// - `Some(prompt)` uses your prompt as the base system prompt.
    /// - `None` means the engine should use its built-in default system prompt.
    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(prompt.into());
        self
    }

    /// Append additional system-level instructions.
    ///
    /// This is appended after the base system prompt and before any runtime config summary
    /// the engine might embed.
    pub fn with_system_prompt_extra(mut self, extra: impl Into<String>) -> Self {
        self.system_prompt_extra = Some(extra.into());
        self
    }

    /// Append additional user instructions for the task.
    ///
    /// This is appended to the user message after the captured page context.
    pub fn with_user_message_extra(mut self, extra: impl Into<String>) -> Self {
        self.user_message_extra = Some(extra.into());
        self
    }

    /// Replace the runtime automation configuration.
    pub fn with_cfg(mut self, cfg: RemoteMultimodalConfig) -> Self {
        self.cfg = cfg;
        self
    }

    /// Set optional URL gating and per-URL overrides.
    pub fn with_prompt_url_gate(mut self, gate: super::PromptUrlGate) -> Self {
        self.prompt_url_gate = Some(gate);
        self
    }

    /// Set an optional concurrency limit for remote inference calls.
    pub fn with_concurrency_limit(mut self, limit: usize) -> Self {
        self.concurrency_limit = Some(limit);
        self
    }

    /// Enable extraction mode to return structured data from pages.
    pub fn with_extra_ai_data(mut self, enabled: bool) -> Self {
        self.cfg.extra_ai_data = enabled;
        self
    }

    /// Set a custom extraction prompt.
    pub fn with_extraction_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.cfg.extraction_prompt = Some(prompt.into());
        self
    }

    /// Enable screenshot capture after automation completes.
    pub fn with_screenshot(mut self, enabled: bool) -> Self {
        self.cfg.screenshot = enabled;
        self
    }

    /// Set a JSON schema for structured extraction output.
    pub fn with_extraction_schema(mut self, schema: super::ExtractionSchema) -> Self {
        self.cfg.extraction_schema = Some(schema);
        self
    }

    /// Check if the configured model supports vision/multimodal input.
    ///
    /// Uses the `supports_vision` function to detect based on model name.
    pub fn model_supports_vision(&self) -> bool {
        supports_vision(&self.model_name)
    }

    /// Determine whether to include screenshots in LLM requests.
    ///
    /// This respects the `include_screenshot` config override:
    /// - `Some(true)`: Always include screenshots
    /// - `Some(false)`: Never include screenshots
    /// - `None`: Auto-detect based on model name
    pub fn should_include_screenshot(&self) -> bool {
        match self.cfg.include_screenshot {
            Some(explicit) => explicit,
            None => self.model_supports_vision(),
        }
    }

    /// Filter screenshot based on model capabilities.
    ///
    /// Returns the screenshot if the model supports vision and screenshots
    /// are enabled, otherwise returns `None`.
    pub fn filter_screenshot<'a>(&self, screenshot: Option<&'a str>) -> Option<&'a str> {
        if self.should_include_screenshot() {
            screenshot
        } else {
            None
        }
    }
}

/// Re-export from llm_models_spider for auto-updated vision model detection.
///
/// This uses the `llm_models_spider` crate which is automatically updated
/// via GitHub Actions to fetch the latest model capabilities from OpenRouter.
pub use llm_models_spider::supports_vision;

/// Merge a base config with an override config.
///
/// Override values take precedence. This is used for URL-specific config overrides.
pub fn merged_config(
    base: &RemoteMultimodalConfig,
    override_cfg: &RemoteMultimodalConfig,
) -> RemoteMultimodalConfig {
    let mut out = base.clone();

    out.include_html = override_cfg.include_html;
    out.html_max_bytes = override_cfg.html_max_bytes;
    out.include_url = override_cfg.include_url;
    out.include_title = override_cfg.include_title;
    out.include_screenshot = override_cfg.include_screenshot;

    out.temperature = override_cfg.temperature;
    out.max_tokens = override_cfg.max_tokens;
    out.request_json_object = override_cfg.request_json_object;
    out.best_effort_json_extract = override_cfg.best_effort_json_extract;

    out.max_rounds = override_cfg.max_rounds;
    out.post_plan_wait_ms = override_cfg.post_plan_wait_ms;

    out.retry = override_cfg.retry.clone();
    out.model_policy = override_cfg.model_policy.clone();

    if !override_cfg.capture_profiles.is_empty() {
        out.capture_profiles = override_cfg.capture_profiles.clone();
    }

    // Extraction settings
    out.extra_ai_data = override_cfg.extra_ai_data;
    out.extraction_prompt = override_cfg.extraction_prompt.clone();
    out.extraction_schema = override_cfg.extraction_schema.clone();
    out.screenshot = override_cfg.screenshot;

    out
}

/// Check if a URL is allowed by the gate.
///
/// Returns:
/// - `true` if the URL is allowed (no gate, or gate allows the URL)
/// - `false` if the URL is blocked
pub fn is_url_allowed(gate: Option<&super::PromptUrlGate>, url: &str) -> bool {
    match gate {
        Some(g) => g.is_allowed(url),
        None => true,
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

    #[test]
    fn test_remote_multimodal_config_defaults() {
        let cfg = RemoteMultimodalConfig::default();

        assert!(cfg.include_html);
        assert_eq!(cfg.html_max_bytes, 24_000);
        assert!(cfg.include_url);
        assert!(cfg.include_title);
        assert_eq!(cfg.temperature, 0.1);
        assert_eq!(cfg.max_tokens, 1024);
        assert!(cfg.request_json_object);
        assert!(cfg.best_effort_json_extract);
        assert_eq!(cfg.max_rounds, 6);
        assert!(cfg.screenshot);
        assert!(!cfg.extra_ai_data);
    }

    #[test]
    fn test_remote_multimodal_config_builder() {
        let cfg = RemoteMultimodalConfig::new()
            .with_html(false)
            .with_temperature(0.5)
            .with_max_rounds(10)
            .with_extraction(true)
            .with_extraction_prompt("Extract products");

        assert!(!cfg.include_html);
        assert_eq!(cfg.temperature, 0.5);
        assert_eq!(cfg.max_rounds, 10);
        assert!(cfg.extra_ai_data);
        assert_eq!(cfg.extraction_prompt, Some("Extract products".to_string()));
    }

    #[test]
    fn test_remote_multimodal_configs_new() {
        let configs = RemoteMultimodalConfigs::new(
            "http://localhost:11434/v1/chat/completions",
            "qwen2.5-vl",
        );

        assert_eq!(configs.api_url, "http://localhost:11434/v1/chat/completions");
        assert_eq!(configs.model_name, "qwen2.5-vl");
        assert!(configs.api_key.is_none());
        assert!(configs.system_prompt.is_none());
    }

    #[test]
    fn test_remote_multimodal_configs_builder() {
        let configs = RemoteMultimodalConfigs::new("https://api.openai.com/v1/chat/completions", "gpt-4o")
            .with_api_key("sk-test")
            .with_system_prompt("You are a helpful assistant.")
            .with_concurrency_limit(5)
            .with_screenshot(true);

        assert_eq!(configs.api_key, Some("sk-test".to_string()));
        assert_eq!(configs.system_prompt, Some("You are a helpful assistant.".to_string()));
        assert_eq!(configs.concurrency_limit, Some(5));
        assert!(configs.cfg.screenshot);
    }

    #[test]
    fn test_html_cleaning_profile_analysis() {
        use super::ContentAnalysis;

        // Test with high SVG bytes - should return Slim
        let mut analysis = ContentAnalysis::default();
        analysis.svg_bytes = 150_000; // > SVG_VERY_HEAVY_THRESHOLD
        assert_eq!(
            HtmlCleaningProfile::from_content_analysis(&analysis),
            HtmlCleaningProfile::Slim
        );

        // Test with small HTML - should return Minimal
        let mut analysis = ContentAnalysis::default();
        analysis.html_length = 5_000;
        analysis.text_ratio = 0.3;
        assert_eq!(
            HtmlCleaningProfile::from_content_analysis(&analysis),
            HtmlCleaningProfile::Minimal
        );
    }

    #[test]
    fn test_html_cleaning_profile_estimate_savings() {
        use super::ContentAnalysis;

        let mut analysis = ContentAnalysis::default();
        analysis.script_bytes = 10_000;
        analysis.style_bytes = 5_000;
        analysis.cleanable_bytes = 20_000;
        analysis.html_length = 50_000;

        assert_eq!(HtmlCleaningProfile::Raw.estimate_savings(&analysis), 0);
        assert_eq!(HtmlCleaningProfile::Minimal.estimate_savings(&analysis), 15_000);
        assert_eq!(HtmlCleaningProfile::Slim.estimate_savings(&analysis), 20_000);
    }

    #[test]
    fn test_supports_vision_openai() {
        // GPT-4o variants (all vision)
        assert!(supports_vision("gpt-4o"));
        assert!(supports_vision("gpt-4o-mini"));

        // GPT-4 Turbo with vision
        assert!(supports_vision("gpt-4-turbo"));

        // o1/o3 models
        assert!(supports_vision("o1"));
        assert!(supports_vision("o3"));

        // Non-vision models
        assert!(!supports_vision("gpt-3.5-turbo"));
    }

    #[test]
    fn test_supports_vision_anthropic() {
        // Claude 3+ are multimodal
        assert!(supports_vision("claude-3-sonnet-20240229"));
        assert!(supports_vision("claude-3-opus-20240229"));
        assert!(supports_vision("claude-3-haiku-20240307"));
        assert!(supports_vision("claude-3-5-sonnet-20241022"));

        // Claude 2 is not vision
        assert!(!supports_vision("claude-2"));
        assert!(!supports_vision("claude-2.1"));
        assert!(!supports_vision("claude-instant-1.2"));
    }

    #[test]
    fn test_supports_vision_qwen() {
        assert!(supports_vision("qwen2-vl-72b"));
        assert!(supports_vision("qwen2.5-vl-7b"));
        assert!(supports_vision("qwen-vl-max"));
        assert!(supports_vision("qwq-32b"));

        // Non-VL Qwen
        assert!(!supports_vision("qwen2-72b"));
        assert!(!supports_vision("qwen2.5-7b"));
    }

    #[test]
    fn test_supports_vision_gemini() {
        assert!(supports_vision("gemini-1.5-pro"));
        assert!(supports_vision("gemini-1.5-flash"));
        assert!(supports_vision("gemini-2.0-flash"));
        assert!(supports_vision("gemini-pro-vision"));
    }

    #[test]
    fn test_supports_vision_other() {
        // Models from OpenRouter's vision list
        assert!(supports_vision("pixtral-12b"));
        assert!(supports_vision("llama-3.2-11b-vision-instruct"));
        assert!(supports_vision("internvl3-78b"));
        assert!(supports_vision("molmo-2-8b"));

        // Non-vision models
        assert!(!supports_vision("llama-3-70b-instruct"));
        assert!(!supports_vision("mistral-7b-instruct"));
        assert!(!supports_vision("mixtral-8x7b-instruct"));
    }

    #[test]
    fn test_supports_vision_case_insensitive() {
        assert!(supports_vision("GPT-4O"));
        assert!(supports_vision("Claude-3-Sonnet"));
        assert!(supports_vision("QWEN2-VL"));
    }

    #[test]
    fn test_remote_multimodal_configs_vision_detection() {
        // Vision model
        let cfg = RemoteMultimodalConfigs::new(
            "https://api.openai.com/v1/chat/completions",
            "gpt-4o",
        );
        assert!(cfg.model_supports_vision());
        assert!(cfg.should_include_screenshot());

        // Non-vision model
        let cfg = RemoteMultimodalConfigs::new(
            "https://api.openai.com/v1/chat/completions",
            "gpt-3.5-turbo",
        );
        assert!(!cfg.model_supports_vision());
        assert!(!cfg.should_include_screenshot());

        // Explicit override to enable screenshots on non-vision model
        let mut cfg = RemoteMultimodalConfigs::new(
            "https://api.openai.com/v1/chat/completions",
            "gpt-3.5-turbo",
        );
        cfg.cfg.include_screenshot = Some(true);
        assert!(cfg.should_include_screenshot());

        // Explicit override to disable screenshots on vision model
        let mut cfg = RemoteMultimodalConfigs::new(
            "https://api.openai.com/v1/chat/completions",
            "gpt-4o",
        );
        cfg.cfg.include_screenshot = Some(false);
        assert!(!cfg.should_include_screenshot());
    }

    #[test]
    fn test_filter_screenshot() {
        let screenshot = "base64data...";

        // Vision model - screenshot passes through
        let cfg = RemoteMultimodalConfigs::new(
            "https://api.openai.com/v1/chat/completions",
            "gpt-4o",
        );
        assert_eq!(cfg.filter_screenshot(Some(screenshot)), Some(screenshot));

        // Non-vision model - screenshot filtered out
        let cfg = RemoteMultimodalConfigs::new(
            "https://api.openai.com/v1/chat/completions",
            "gpt-3.5-turbo",
        );
        assert_eq!(cfg.filter_screenshot(Some(screenshot)), None);

        // No screenshot provided
        let cfg = RemoteMultimodalConfigs::new(
            "https://api.openai.com/v1/chat/completions",
            "gpt-4o",
        );
        assert_eq!(cfg.filter_screenshot(None), None);
    }
}
