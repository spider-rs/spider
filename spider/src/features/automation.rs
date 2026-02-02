#[cfg(feature = "chrome")]
use base64::{engine::general_purpose, Engine as _};
#[cfg(feature = "chrome")]
use chromiumoxide::{
    cdp::browser_protocol::page::CaptureScreenshotFormat, page::ScreenshotParams, Page,
};
use reqwest::Client;
#[cfg(feature = "serde")]
use serde::Serialize;
#[cfg(feature = "serde")]
use serde_json::Value;
use std::{error::Error as StdError, fmt};

lazy_static::lazy_static! {
    /// Top level client for automation.
    static ref CLIENT: Client = Client::new();
}

/// Convenience result type used throughout the remote multimodal engine.
pub type EngineResult<T> = Result<T, EngineError>;

/// Errors produced by the remote multimodal engine.
///
/// This error type is intentionally lightweight (no `anyhow`) and is suitable
/// for surfacing from public APIs (e.g. `EngineResult<T> = Result<T, EngineError>`).
///
/// It covers:
/// - transport failures when calling the remote endpoint,
/// - JSON serialization/deserialization failures,
/// - schema mismatches in OpenAI-compatible responses,
/// - non-success responses returned by the remote provider,
/// - unsupported operations due to compile-time feature flags.
#[derive(Debug)]
pub enum EngineError {
    /// HTTP-layer failure (request could not be sent, connection error, timeout, etc.).
    Http(reqwest::Error),
    #[cfg(feature = "serde")]
    /// JSON serialization/deserialization failure when building or parsing payloads.
    Json(serde_json::Error),
    /// A required field was missing in a parsed JSON payload.
    ///
    /// Example: missing `"choices[0].message.content"` in an OpenAI-compatible response.
    MissingField(&'static str),
    /// A field was present but had an unexpected type or shape.
    ///
    /// Example: `"steps"` exists but is not an array.
    InvalidField(&'static str),
    /// The remote endpoint returned a non-success status or a server-side error.
    ///
    /// The contained string should be a human-readable explanation suitable for logs.
    Remote(String),
    /// The operation is not supported in the current build configuration.
    ///
    /// Example: calling browser automation without `feature="chrome"`,
    /// or attempting to deserialize steps without `feature="serde"`.
    Unsupported(&'static str),
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EngineError::Http(e) => write!(f, "http error: {e}"),
            #[cfg(feature = "serde")]
            EngineError::Json(e) => write!(f, "json error: {e}"),
            EngineError::MissingField(s) => write!(f, "missing field: {s}"),
            EngineError::InvalidField(s) => write!(f, "invalid field: {s}"),
            EngineError::Remote(s) => write!(f, "remote error: {s}"),
            EngineError::Unsupported(s) => write!(f, "unsupported: {s}"),
        }
    }
}
impl StdError for EngineError {}

impl From<reqwest::Error> for EngineError {
    fn from(e: reqwest::Error) -> Self {
        EngineError::Http(e)
    }
}

#[cfg(feature = "serde")]
impl From<serde_json::Error> for EngineError {
    fn from(e: serde_json::Error) -> Self {
        EngineError::Json(e)
    }
}

/// JSON schema configuration for structured extraction output.
///
/// This allows you to define a schema that the model should follow when
/// extracting data from pages. Similar to OpenAI's structured outputs.
#[derive(Debug, Clone, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ExtractionSchema {
    /// A name for this extraction schema (e.g., "product_listing", "contact_info").
    pub name: String,
    /// Optional description of what data should be extracted.
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
    pub description: Option<String>,
    /// The JSON Schema definition as a string.
    ///
    /// Example:
    /// ```json
    /// {
    ///   "type": "object",
    ///   "properties": {
    ///     "title": { "type": "string" },
    ///     "price": { "type": "number" }
    ///   },
    ///   "required": ["title"]
    /// }
    /// ```
    pub schema: String,
    /// Whether to enforce strict schema adherence.
    ///
    /// When true, instructs the model to strictly follow the schema.
    /// Note: Not all models support strict mode.
    #[cfg_attr(feature = "serde", serde(default))]
    pub strict: bool,
}

impl ExtractionSchema {
    /// Create a new extraction schema.
    pub fn new(name: &str, schema: &str) -> Self {
        Self {
            name: name.to_string(),
            description: None,
            schema: schema.to_string(),
            strict: false,
        }
    }

    /// Create a new extraction schema with description.
    pub fn new_with_description(name: &str, description: &str, schema: &str) -> Self {
        Self {
            name: name.to_string(),
            description: Some(description.to_string()),
            schema: schema.to_string(),
            strict: false,
        }
    }

    /// Set strict mode.
    pub fn with_strict(mut self, strict: bool) -> Self {
        self.strict = strict;
        self
    }
}

/// Coarse cost budget the engine may spend for a single automation run.
///
/// This is used by [`ModelPolicy`] to decide whether the engine may select
/// more expensive models (e.g. "large") based on the caller's preference.
///
/// When paired with latency limits, this allows you to express:
/// - "Keep it cheap" (Low)
/// - "Balanced" (Medium)
/// - "Use best model if needed" (High)
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum CostTier {
    /// Prefer cheapest models and smallest responses.
    #[default]
    Low,
    /// Balanced cost/quality.
    Medium,
    /// Prefer quality over cost (still may respect latency).
    High,
}

/// Model selection policy for remote multimodal automation.
///
/// This policy lets the engine choose a model tier per attempt/run.
/// You can provide three model identifiers (small/medium/large) and
/// restrict when large is allowed.
///
/// Typical usage:
/// - **small**: fast + cheap, used for simple pages or quick iterations.
/// - **medium**: general default for most flows.
/// - **large**: best reasoning/vision, used only when allowed.
///
/// The engine may also consider latency and cost constraints when selecting.
/// If `allow_large == false`, the engine must not choose `large`, even if
/// `max_cost_tier` would permit it.
#[derive(Debug, Clone, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct ModelPolicy {
    /// Small/fast model identifier understood by your OpenAI-compatible endpoint.
    ///
    /// Example: `"gpt-4.1-mini"` or a local model name.
    pub small: String,

    /// Medium model identifier (balanced).
    ///
    /// This is commonly the default tier used for most runs.
    pub medium: String,

    /// Large model identifier (best reasoning/vision).
    ///
    /// This tier is only eligible if [`ModelPolicy::allow_large`] is true
    /// and [`ModelPolicy::max_cost_tier`] permits it.
    pub large: String,

    /// If false, the engine will never select the large tier.
    ///
    /// Useful for enforcing budget or avoiding slow/expensive calls globally.
    pub allow_large: bool,

    /// Optional upper bound (in milliseconds) for acceptable model latency.
    ///
    /// If set, the engine may avoid selecting models/tier choices that are
    /// expected to exceed this budget (best-effort; cannot be guaranteed).
    pub max_latency_ms: Option<u64>,

    /// Maximum cost tier the engine may use.
    ///
    /// - `Low`   => only small (and possibly medium if you treat it as low).
    /// - `Medium`=> small or medium.
    /// - `High`  => small/medium/large (if `allow_large` is also true).
    pub max_cost_tier: CostTier,
}

/// The html cleaning profile.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum HtmlCleaningProfile {
    #[default]
    /// Uses `crate::utils::clean_html(...)` (already feature-switched by your utils).
    Default,
    /// More aggressive: try `clean_html_full` if available, otherwise fall back to `clean_html`.
    Aggressive,
    /// Less aggressive: try `clean_html_base` if available, otherwise fall back to `clean_html`.
    Minimal,
    /// No cleaning (raw HTML).
    Raw,
}

/// How to capture the page for a single LLM attempt.
///
/// A `CaptureProfile` describes **how the engine should snapshot the current page**
/// when asking the remote/local multimodal chat model for an automation plan.
///
/// Each round/attempt may use a different profile (for example: first try viewport
/// screenshot + default HTML cleaning, then retry with full-page screenshot + slim
/// cleaning), which helps when the first capture misses important context.
///
/// The engine uses these fields to decide:
/// - how to take the screenshot,
/// - how to clean/truncate HTML,
/// - what extra “attempt note” to tell the model (useful for iterative retries).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct CaptureProfile {
    /// Full page screenshot (true) vs viewport only (false).
    pub full_page: bool,
    /// Omit background for transparency (if supported by your screenshot path).
    pub omit_background: bool,
    /// Optional clip viewport (if supported). None = no clip.
    pub clip: Option<crate::configuration::ClipViewport>,
    /// How to clean/prepare HTML before sending.
    pub html_cleaning: HtmlCleaningProfile,
    /// How many bytes of cleaned HTML to include.
    pub html_max_bytes: usize,
    /// Optional note injected into the user message for THIS attempt.
    pub attempt_note: Option<String>,
}

impl Default for CaptureProfile {
    /// Default capture profile:
    /// - full-page screenshot
    /// - omit background
    /// - default HTML cleaning
    /// - include up to 24,000 bytes of cleaned HTML
    fn default() -> Self {
        Self {
            full_page: true,
            omit_background: true,
            html_max_bytes: 24_000,
            attempt_note: None,
            clip: None,
            html_cleaning: HtmlCleaningProfile::Default,
        }
    }
}

/// Strategy for retrying if the plan fails or output is invalid.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RetryPolicy {
    /// Maximum number of model attempts per round.
    ///
    /// Example: `max_attempts = 3` means the engine may re-ask the model up to
    /// three times (potentially with different capture profiles) before giving up.
    pub max_attempts: usize,
    /// Delay between attempts (milliseconds).
    ///
    /// Useful for rate-limited local servers or to give the page time to settle.
    pub backoff_ms: u64,
    /// Retry when the model output cannot be parsed into a valid plan.
    pub retry_on_parse_error: bool,
    /// Retry when a step fails during execution.
    pub retry_on_step_failure: bool,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 1,
            backoff_ms: 0,
            retry_on_parse_error: false,
            retry_on_step_failure: false,
        }
    }
}

/// Runtime configuration for `RemoteMultimodalEngine`.
///
/// This struct controls:
/// 1) what context is captured (URL/title/HTML),
/// 2) how chat completion is requested (temperature/max tokens/JSON mode),
/// 3) how long the engine loops and retries,
/// 4) capture/model selection policies.
///
/// The engine should be able to **export this config** to users, and it should
/// be safe to merge with user-provided prompts (the engine can inject a read-only
/// summary of the active config into the system prompt).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
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
    /// This data is stored in the `AutomationResult.extracted` field.
    pub extra_ai_data: bool,
    /// Optional custom extraction prompt appended to the system prompt.
    ///
    /// Example: "Extract all product names and prices as a JSON array."
    pub extraction_prompt: Option<String>,
    /// Optional JSON schema for structured extraction output.
    ///
    /// When provided, the model is instructed to return the `extracted` field
    /// conforming to this schema. This enables type-safe extraction.
    ///
    /// Example schema:
    /// ```json
    /// {
    ///   "type": "object",
    ///   "properties": {
    ///     "products": {
    ///       "type": "array",
    ///       "items": {
    ///         "type": "object",
    ///         "properties": {
    ///           "name": { "type": "string" },
    ///           "price": { "type": "number" }
    ///         },
    ///         "required": ["name", "price"]
    ///       }
    ///     }
    ///   }
    /// }
    /// ```
    pub extraction_schema: Option<ExtractionSchema>,
    /// Take a screenshot after automation completes and include it in results.
    pub screenshot: bool,
}

impl Default for RemoteMultimodalConfig {
    fn default() -> Self {
        Self {
            include_html: true,
            html_max_bytes: 24_000,
            include_url: true,
            include_title: true,
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
            screenshot: false,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
/// Determines whether automation should run for a given URL and optionally returns a URL-scoped override config.
pub struct PromptUrlGate {
    /// Prompt url map.
    pub prompt_url_map: Option<
        Box<
            hashbrown::HashMap<
                case_insensitive_string::CaseInsensitiveString,
                Box<RemoteMultimodalConfig>,
            >,
        >,
    >,
    /// Paths mapping.
    pub paths_map: bool,
}

/// Everything needed to configure RemoteMultimodalEngine.
#[derive(Debug, Clone)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(default)
)]
pub struct RemoteMultimodalConfigs {
    /// OpenAI-compatible chat completions URL.
    pub api_url: String,
    /// Optional bearer key for `Authorization: Bearer ...`
    pub api_key: Option<String>,
    /// Model name/id for the target endpoint.
    pub model_name: String,
    /// Optional base system prompt (None => engine default).
    pub system_prompt: Option<String>,
    /// Optional extra system instructions appended at runtime.
    pub system_prompt_extra: Option<String>,
    /// Optional extra user instructions appended at runtime.
    pub user_message_extra: Option<String>,
    /// Runtime knobs (capture policies, retry, looping, etc.)
    pub cfg: RemoteMultimodalConfig,
    /// Optional URL gating and per-URL overrides (like prompt_url_map in GPTConfigs)
    pub prompt_url_gate: Option<PromptUrlGate>,
    /// Optional concurrency limit for remote inference calls.
    /// (Engine can build a Semaphore(limit) if desired.)
    pub concurrency_limit: Option<usize>,
    /// Semaphore control.
    #[cfg_attr(
        feature = "serde",
        serde(skip, default = "RemoteMultimodalConfigs::default_semaphore")
    )]
    pub semaphore: std::sync::OnceLock<std::sync::Arc<tokio::sync::Semaphore>>,
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
    /// Create a new remote multimodal config bundle, similar to `GPTConfigs::new(...)`.
    ///
    /// This sets the minimum required fields:
    /// - `api_url`: the OpenAI-compatible `/v1/chat/completions` endpoint
    /// - `model_name`: the model identifier understood by that endpoint
    ///
    /// All other fields fall back to [`Default::default`].
    ///
    /// # Example
    /// ```rust
    /// use spider::features::automation::RemoteMultimodalConfigs;
    ///
    /// let mm = RemoteMultimodalConfigs::new(
    ///     "http://localhost:11434/v1/chat/completions",
    ///     "qwen2.5-vl",
    /// );
    /// ```
    pub fn new(api_url: &str, model_name: &str) -> Self {
        Self {
            api_url: api_url.into(),
            model_name: model_name.into(),
            ..Default::default()
        }
    }

    /// Default semaphore.
    fn default_semaphore() -> std::sync::OnceLock<std::sync::Arc<tokio::sync::Semaphore>> {
        std::sync::OnceLock::new()
    }

    /// Get (and lazily init) the shared semaphore from `concurrency_limit`.
    /// This is safe to call concurrently; `OnceLock` handles the race.
    pub fn get_or_init_semaphore(&self) -> Option<std::sync::Arc<tokio::sync::Semaphore>> {
        let n = self.concurrency_limit?;
        if n == 0 {
            return None;
        }
        Some(
            self.semaphore
                .get_or_init(|| std::sync::Arc::new(tokio::sync::Semaphore::new(n)))
                .clone(),
        )
    }

    /// Attach an optional API key for authenticated endpoints.
    ///
    /// When set, the engine will send:
    /// `Authorization: Bearer <api_key>`
    ///
    /// Pass `None` to clear the key.
    ///
    /// # Example
    /// ```rust
    /// use spider::features::automation::RemoteMultimodalConfigs;
    /// let mm = RemoteMultimodalConfigs::new("https://api.openai.com/v1/chat/completions", "gpt-4.1")
    ///     .with_api_key(Some("sk-..."));
    /// ```
    pub fn with_api_key(mut self, key: Option<&str>) -> Self {
        self.api_key = key.map(|k| k.to_string());
        self
    }

    /// Set the base system prompt for the model.
    ///
    /// - `Some(prompt)` uses your prompt as the base system prompt.
    /// - `None` means the engine should use its built-in default system prompt.
    ///
    /// # Example
    /// ```rust
    /// use spider::features::automation::RemoteMultimodalConfigs;
    /// let mm = RemoteMultimodalConfigs::new("http://localhost:11434/v1/chat/completions", "model")
    ///     .with_system_prompt(Some("You are a careful web automation agent. Output JSON only."));
    /// ```
    pub fn with_system_prompt(mut self, prompt: Option<&str>) -> Self {
        self.system_prompt = prompt.map(|p| p.to_string());
        self
    }

    /// Append additional system-level instructions.
    ///
    /// This is appended after the base system prompt and before any runtime config summary
    /// the engine might embed.
    ///
    /// Use this for global constraints such as:
    /// - "Never log in"
    /// - "Avoid clicking ads"
    /// - "Prefer selectors over coordinates"
    ///
    /// # Example
    /// ```rust
    /// use spider::features::automation::RemoteMultimodalConfigs;
    /// let mm = RemoteMultimodalConfigs::new("http://localhost:11434/v1/chat/completions", "model")
    ///     .with_system_prompt_extra(Some("Never submit credentials. Output JSON only."));
    /// ```
    pub fn with_system_prompt_extra(mut self, extra: Option<&str>) -> Self {
        self.system_prompt_extra = extra.map(|s| s.to_string());
        self
    }

    /// Append additional user instructions for the task.
    ///
    /// This is appended to the user message after the captured page context
    /// (URL/title/HTML as configured).
    ///
    /// Use this for per-run intent such as:
    /// - "Goal: reach pricing page"
    /// - "Dismiss the popup and stop"
    /// - "Scroll until the table is visible"
    ///
    /// # Example
    /// ```rust
    /// use spider::features::automation::RemoteMultimodalConfigs;
    /// let mm = RemoteMultimodalConfigs::new("http://localhost:11434/v1/chat/completions", "model")
    ///     .with_user_message_extra(Some("Goal: close any modal, then stop once main content is visible."));
    /// ```
    pub fn with_user_message_extra(mut self, extra: Option<&str>) -> Self {
        self.user_message_extra = extra.map(|s| s.to_string());
        self
    }

    /// Replace the runtime automation configuration (capture + looping + retry + model policy).
    ///
    /// This controls behavior such as:
    /// - whether to include HTML/URL/title,
    /// - max rounds / retry policy,
    /// - capture profiles,
    /// - model selection policy.
    ///
    /// # Example
    /// ```rust
    /// use spider::features::automation::{RemoteMultimodalConfigs, RemoteMultimodalConfig};
    /// let cfg = RemoteMultimodalConfig { include_html: false, ..Default::default() };
    /// let mm = RemoteMultimodalConfigs::new("http://localhost:11434/v1/chat/completions", "model")
    ///     .with_cfg(cfg);
    /// ```
    pub fn with_cfg(mut self, cfg: RemoteMultimodalConfig) -> Self {
        self.cfg = cfg;
        self
    }

    /// Set optional URL gating and per-URL overrides.
    ///
    /// If set, automation will only run for URLs that match the gate.
    /// The gate may also return a URL-scoped override config used for that run.
    ///
    /// Pass `None` to disable gating.
    ///
    /// # Example
    /// ```rust
    /// use spider::features::automation::{RemoteMultimodalConfigs, PromptUrlGate};
    /// let gate = PromptUrlGate { prompt_url_map: None, paths_map: false };
    /// let mm = RemoteMultimodalConfigs::new("http://localhost:11434/v1/chat/completions", "model")
    ///     .with_prompt_url_gate(Some(gate));
    /// ```
    pub fn with_prompt_url_gate(mut self, gate: Option<PromptUrlGate>) -> Self {
        self.prompt_url_gate = gate;
        self
    }

    /// Set an optional concurrency limit for remote inference calls.
    ///
    /// This is a *configuration hint*; the engine can translate it into a shared
    /// semaphore (e.g. `Semaphore::new(limit)`) to bound concurrent LLM requests.
    ///
    /// Pass `None` to disable any explicit engine-level concurrency bound.
    ///
    /// # Example
    /// ```rust
    /// use spider::features::automation::RemoteMultimodalConfigs;
    /// let mm = RemoteMultimodalConfigs::new("http://localhost:11434/v1/chat/completions", "model")
    ///     .with_concurrency_limit(Some(8));
    /// ```
    pub fn with_concurrency_limit(mut self, limit: Option<usize>) -> Self {
        self.concurrency_limit = limit;
        self
    }

    /// Enable extraction mode to return structured data from pages.
    ///
    /// When enabled, the model is instructed to include an `extracted` field
    /// in its JSON response containing data extracted from the page.
    ///
    /// # Example
    /// ```rust
    /// use spider::features::automation::RemoteMultimodalConfigs;
    /// let mm = RemoteMultimodalConfigs::new("http://localhost:11434/v1/chat/completions", "model")
    ///     .with_extra_ai_data(true);
    /// ```
    pub fn with_extra_ai_data(mut self, enabled: bool) -> Self {
        self.cfg.extra_ai_data = enabled;
        self
    }

    /// Set a custom extraction prompt.
    ///
    /// This prompt is appended to the system prompt when `extra_ai_data` is enabled.
    /// Use it to specify what data to extract from the page.
    ///
    /// # Example
    /// ```rust
    /// use spider::features::automation::RemoteMultimodalConfigs;
    /// let mm = RemoteMultimodalConfigs::new("http://localhost:11434/v1/chat/completions", "model")
    ///     .with_extra_ai_data(true)
    ///     .with_extraction_prompt(Some("Extract all product names and prices as a JSON array."));
    /// ```
    pub fn with_extraction_prompt(mut self, prompt: Option<&str>) -> Self {
        self.cfg.extraction_prompt = prompt.map(|p| p.to_string());
        self
    }

    /// Enable screenshot capture after automation completes.
    ///
    /// When enabled, a screenshot is taken and returned in the `AutomationResult`.
    ///
    /// # Example
    /// ```rust
    /// use spider::features::automation::RemoteMultimodalConfigs;
    /// let mm = RemoteMultimodalConfigs::new("http://localhost:11434/v1/chat/completions", "model")
    ///     .with_screenshot(true);
    /// ```
    pub fn with_screenshot(mut self, enabled: bool) -> Self {
        self.cfg.screenshot = enabled;
        self
    }

    /// Set a JSON schema for structured extraction output.
    ///
    /// When provided, the model is instructed to return the `extracted` field
    /// conforming to this schema. This enables type-safe extraction.
    ///
    /// # Example
    /// ```rust
    /// use spider::features::automation::{RemoteMultimodalConfigs, ExtractionSchema};
    /// let schema = ExtractionSchema::new("products", r#"{"type": "array", "items": {"type": "object", "properties": {"name": {"type": "string"}, "price": {"type": "number"}}}}"#);
    /// let mm = RemoteMultimodalConfigs::new("http://localhost:11434/v1/chat/completions", "model")
    ///     .with_extra_ai_data(true)
    ///     .with_extraction_schema(Some(schema));
    /// ```
    pub fn with_extraction_schema(mut self, schema: Option<ExtractionSchema>) -> Self {
        self.cfg.extraction_schema = schema;
        self
    }
}

impl PromptUrlGate {
    /// Returns:
    /// - `None` => blocked (map exists, URL not matched)
    /// - `Some(None)` => allowed, no override
    /// - `Some(Some(cfg))` => allowed, use override config
    pub fn match_url<'a>(&'a self, url: &str) -> Option<Option<&'a RemoteMultimodalConfig>> {
        let map = self.prompt_url_map.as_deref()?;
        let url_ci = case_insensitive_string::CaseInsensitiveString::new(url);

        // Exact match first
        if let Some(cfg) = map.get(&url_ci) {
            return Some(Some(cfg));
        }

        // Path-prefix match
        if self.paths_map {
            for (k, v) in map.iter() {
                if url_ci.starts_with(k.inner().as_str()) {
                    return Some(Some(v));
                }
            }
        }

        None
    }
}

/// Engine that calls an OpenAI-compatible multimodal chat endpoint (local or remote),
/// asks it for JSON automation plans, executes them, re-captures state, and repeats
/// until completion.
///
/// ## Concurrency & throttling
/// This engine optionally supports **concurrency control** via an internal
/// [`tokio::sync::Semaphore`].
///
/// When a semaphore is configured:
/// - Each LLM request attempt acquires a permit before sending the HTTP request.
/// - The permit is held for the duration of the request (including response body read).
/// - This limits the number of **in-flight LLM requests** across all clones of the engine
///   that share the same semaphore.
///
/// This is useful for:
/// - Respecting provider rate limits,
/// - Avoiding overload of local LLM servers,
/// - Controlling CPU / memory pressure in high-concurrency crawls.
///
/// ### Design notes
/// - The semaphore is **shared** (`Arc<Semaphore>`), not recreated per request.
/// - If `semaphore` is `None`, no throttling is applied.
/// - Multiple `RemoteMultimodalEngine` instances may share the same semaphore
///   to enforce a **global concurrency limit**.
///
/// The semaphore can be configured either by:
/// - Providing a maximum number of in-flight requests (engine creates the semaphore), or
/// - Supplying an external semaphore to coordinate concurrency across systems.
#[derive(Debug, Clone)]
pub struct RemoteMultimodalEngine {
    /// Full OpenAI-compatible chat completions endpoint URL.
    pub api_url: String,
    /// Optional bearer token for authenticated endpoints.
    pub api_key: Option<String>,
    /// Model identifier understood by the endpoint.
    pub model_name: String,
    /// Optional base system prompt for the model.
    pub system_prompt: Option<String>,
    /// Optional extra system instructions appended at runtime.
    pub system_prompt_extra: Option<String>,
    /// Optional extra user instructions appended at runtime.
    pub user_message_extra: Option<String>,
    /// Runtime configuration controlling capture, retry, and model policy.
    pub cfg: RemoteMultimodalConfig,
    /// Optional URL-based gate controlling whether automation runs for a given URL
    /// and allowing per-URL config overrides.
    pub prompt_url_gate: Option<PromptUrlGate>,
    /// Optional semaphore used to limit concurrent in-flight LLM requests.
    ///
    /// When present, each LLM request attempt acquires a permit before sending
    /// the HTTP request and releases it when the attempt completes.
    ///
    /// This semaphore is shared across engine clones via `Arc` and may also be
    /// supplied externally to enforce a global concurrency limit.
    pub semaphore: Option<std::sync::Arc<tokio::sync::Semaphore>>,
}

impl RemoteMultimodalEngine {
    /// New configuration.
    pub fn new<S: Into<String>>(api_url: S, model_name: S, system_prompt: Option<String>) -> Self {
        Self {
            api_url: api_url.into(),
            api_key: None,
            model_name: model_name.into(),
            system_prompt,
            system_prompt_extra: None,
            user_message_extra: None,
            cfg: RemoteMultimodalConfig::default(),
            prompt_url_gate: None,
            semaphore: None,
        }
    }
    /// Set/clear the API key (Bearer token).
    pub fn with_api_key(mut self, key: Option<&str>) -> Self {
        self.api_key = key.map(|k| k.to_string());
        self
    }
    /// With config.
    pub fn with_config(mut self, cfg: RemoteMultimodalConfig) -> Self {
        self.cfg = cfg;
        self
    }

    /// Limit the number of concurrent in-flight LLM requests made by this engine.
    ///
    /// This creates an internal semaphore shared by clones of the engine.
    pub fn with_max_inflight_requests(&mut self, n: usize) -> &mut Self {
        if n == 0 {
            self.semaphore = None;
            self.cfg.max_inflight_requests = None;
        } else {
            self.semaphore = Some(std::sync::Arc::new(tokio::sync::Semaphore::new(n)));
            self.cfg.max_inflight_requests = Some(n);
        }
        self
    }

    /// Provide an external semaphore to throttle LLM requests globally.
    ///
    /// Use this when you want multiple engines/components to share one throttle.
    pub fn with_semaphore(
        &mut self,
        sem: Option<std::sync::Arc<tokio::sync::Semaphore>>,
    ) -> &mut Self {
        self.semaphore = sem;
        self
    }

    /// With system prompt extra.
    pub fn with_system_prompt_extra(&mut self, extra: Option<&str>) -> &mut Self {
        self.system_prompt_extra = extra.map(|s| s.to_string());
        self
    }

    /// With user message extra.
    pub fn with_user_message_extra(&mut self, extra: Option<&str>) -> &mut Self {
        self.user_message_extra = extra.map(|s| s.to_string());
        self
    }

    /// With the prompt url gate.
    pub fn with_prompt_url_gate(&mut self, gate: Option<PromptUrlGate>) -> &mut Self {
        self.prompt_url_gate = gate;
        self
    }

    /// With the remote multimodal configuration.
    pub fn with_remote_multimodal_config(&mut self, cfg: RemoteMultimodalConfig) -> &mut Self {
        self.cfg = cfg;
        self
    }

    /// Enable extraction mode to return structured data from pages.
    pub fn with_extra_ai_data(&mut self, enabled: bool) -> &mut Self {
        self.cfg.extra_ai_data = enabled;
        self
    }

    /// Set a custom extraction prompt.
    pub fn with_extraction_prompt(&mut self, prompt: Option<&str>) -> &mut Self {
        self.cfg.extraction_prompt = prompt.map(|p| p.to_string());
        self
    }

    /// Enable screenshot capture after automation completes.
    pub fn with_screenshot(&mut self, enabled: bool) -> &mut Self {
        self.cfg.screenshot = enabled;
        self
    }

    /// Set a JSON schema for structured extraction output.
    pub fn with_extraction_schema(&mut self, schema: Option<ExtractionSchema>) -> &mut Self {
        self.cfg.extraction_schema = schema;
        self
    }

    /// Acquire the permit.
    pub async fn acquire_llm_permit(&self) -> Option<tokio::sync::OwnedSemaphorePermit> {
        match &self.semaphore {
            Some(sem) => Some(sem.clone().acquire_owned().await.ok()?),
            None => None,
        }
    }

    /// Extract structured data from raw HTML content (no browser required).
    ///
    /// This method enables extraction from HTTP responses without Chrome.
    /// It sends the HTML to the multimodal model and returns extracted data.
    ///
    /// # Arguments
    /// * `html` - The raw HTML content to extract from
    /// * `url` - The URL of the page (for context)
    /// * `title` - Optional page title
    ///
    /// # Returns
    /// An `AutomationResult` containing the extracted data in the `extracted` field.
    #[cfg(feature = "serde")]
    pub async fn extract_from_html(
        &self,
        html: &str,
        url: &str,
        title: Option<&str>,
    ) -> EngineResult<AutomationResult> {
        use serde::Serialize;

        #[derive(Serialize)]
        struct ContentBlock {
            #[serde(rename = "type")]
            content_type: String,
            #[serde(skip_serializing_if = "Option::is_none")]
            text: Option<String>,
        }

        #[derive(Serialize)]
        struct Message {
            role: String,
            content: Vec<ContentBlock>,
        }

        #[derive(Serialize)]
        struct ResponseFormat {
            #[serde(rename = "type")]
            format_type: String,
        }

        #[derive(Serialize)]
        struct InferenceRequest {
            model: String,
            messages: Vec<Message>,
            temperature: f32,
            max_tokens: u16,
            #[serde(skip_serializing_if = "Option::is_none")]
            #[serde(rename = "response_format")]
            response_format: Option<ResponseFormat>,
        }

        let effective_cfg = &self.cfg;

        // Build user prompt with HTML context
        let mut user_text = String::with_capacity(256 + html.len().min(effective_cfg.html_max_bytes));
        user_text.push_str("EXTRACTION CONTEXT:\n");
        user_text.push_str("- url: ");
        user_text.push_str(url);
        user_text.push('\n');
        if let Some(t) = title {
            user_text.push_str("- title: ");
            user_text.push_str(t);
            user_text.push('\n');
        }
        user_text.push_str("\nHTML CONTENT:\n");

        // Truncate HTML if needed
        let html_truncated = truncate_utf8_tail(html, effective_cfg.html_max_bytes);
        user_text.push_str(&html_truncated);

        user_text.push_str("\n\nTASK:\nExtract structured data from the HTML above. Return a JSON object with:\n");
        user_text.push_str("- \"label\": short description of what was extracted\n");
        user_text.push_str("- \"done\": true\n");
        user_text.push_str("- \"steps\": [] (empty, no browser automation)\n");
        user_text.push_str("- \"extracted\": the structured data extracted from the page\n");

        if let Some(extra) = &self.user_message_extra {
            if !extra.trim().is_empty() {
                user_text.push_str("\n---\nUSER INSTRUCTIONS:\n");
                user_text.push_str(extra.trim());
                user_text.push('\n');
            }
        }

        let request_body = InferenceRequest {
            model: self.model_name.clone(),
            messages: vec![
                Message {
                    role: "system".into(),
                    content: vec![ContentBlock {
                        content_type: "text".into(),
                        text: Some(self.system_prompt_compiled(effective_cfg)),
                    }],
                },
                Message {
                    role: "user".into(),
                    content: vec![ContentBlock {
                        content_type: "text".into(),
                        text: Some(user_text),
                    }],
                },
            ],
            temperature: effective_cfg.temperature,
            max_tokens: effective_cfg.max_tokens,
            response_format: if effective_cfg.request_json_object {
                Some(ResponseFormat {
                    format_type: "json_object".into(),
                })
            } else {
                None
            },
        };

        // Acquire permit before sending
        let _permit = self.acquire_llm_permit().await;

        let mut req = CLIENT.post(&self.api_url).json(&request_body);
        if let Some(key) = &self.api_key {
            req = req.bearer_auth(key);
        }

        let start = std::time::Instant::now();
        let http_resp = req.send().await?;
        let status = http_resp.status();
        let raw_body = http_resp.text().await?;

        log::debug!(
            "remote_multimodal extract_from_html: status={} latency={:?} body_len={}",
            status,
            start.elapsed(),
            raw_body.len()
        );

        if !status.is_success() {
            return Err(EngineError::Remote(format!(
                "non-success status {status}: {raw_body}"
            )));
        }

        let root: serde_json::Value = serde_json::from_str(&raw_body)
            .map_err(|e| EngineError::Remote(format!("JSON parse error: {e}")))?;

        let content = extract_assistant_content(&root)
            .ok_or(EngineError::MissingField("choices[0].message.content"))?;

        let usage = extract_usage(&root);

        let plan_value = if effective_cfg.best_effort_json_extract {
            best_effort_parse_json_object(&content)?
        } else {
            serde_json::from_str::<serde_json::Value>(&content)
                .map_err(|e| EngineError::Remote(format!("JSON parse error: {e}")))?
        };

        let label = plan_value
            .get("label")
            .and_then(|v| v.as_str())
            .unwrap_or("extraction")
            .to_string();

        let extracted = plan_value.get("extracted").cloned();

        Ok(AutomationResult {
            label,
            steps_executed: 0,
            success: true,
            error: None,
            usage,
            extracted,
            screenshot: None,
        })
    }

    /// System prompt compiled.
    pub fn system_prompt_compiled(&self, effective_cfg: &RemoteMultimodalConfig) -> String {
        let mut s = self
            .system_prompt
            .as_deref()
            .unwrap_or(DEFAULT_SYSTEM_PROMPT)
            .to_string();

        if let Some(extra) = &self.system_prompt_extra {
            if !extra.trim().is_empty() {
                s.push_str("\n\n---\nADDITIONAL SYSTEM INSTRUCTIONS:\n");
                s.push_str(extra.trim());
            }
        }

        // Add extraction instructions when extra_ai_data is enabled
        if effective_cfg.extra_ai_data {
            s.push_str("\n\n---\nEXTRACTION MODE ENABLED:\n");
            s.push_str("Include an \"extracted\" field in your JSON response containing structured data extracted from the page.\n");

            // Add schema instructions if provided
            if let Some(schema) = &effective_cfg.extraction_schema {
                s.push_str("\nExtraction Schema: ");
                s.push_str(&schema.name);
                s.push('\n');
                if let Some(desc) = &schema.description {
                    s.push_str("Description: ");
                    s.push_str(desc.trim());
                    s.push('\n');
                }
                s.push_str("\nThe \"extracted\" field MUST conform to this JSON Schema:\n");
                s.push_str(&schema.schema);
                s.push('\n');
                if schema.strict {
                    s.push_str("\nSTRICT MODE: You MUST follow the schema exactly. Do not add extra fields or omit required fields.\n");
                }
            } else {
                s.push_str("The \"extracted\" field should be a JSON object or array with the relevant data.\n");
            }

            if let Some(extraction_prompt) = &effective_cfg.extraction_prompt {
                s.push_str("\nExtraction instructions: ");
                s.push_str(extraction_prompt.trim());
                s.push('\n');
            }

            s.push_str("\nExample response with extraction:\n");
            s.push_str("{\n  \"label\": \"extract_products\",\n  \"done\": true,\n  \"steps\": [],\n  \"extracted\": {\"products\": [{\"name\": \"Product A\", \"price\": 19.99}]}\n}\n");
        }

        s.push_str("\n\n---\nRUNTIME CONFIG (read-only):\n");
        s.push_str(&format!(
            "- include_url: {}\n- include_title: {}\n- include_html: {}\n- html_max_bytes: {}\n- temperature: {}\n- max_tokens: {}\n- request_json_object: {}\n- best_effort_json_extract: {}\n- max_rounds: {}\n- extra_ai_data: {}\n",
            effective_cfg.include_url,
            effective_cfg.include_title,
            effective_cfg.include_html,
            effective_cfg.html_max_bytes,
            effective_cfg.temperature,
            effective_cfg.max_tokens,
            effective_cfg.request_json_object,
            effective_cfg.best_effort_json_extract,
            effective_cfg.max_rounds,
            effective_cfg.extra_ai_data,
        ));

        s
    }

    // -----------------------------------------------------------------
    // Capture helpers
    // -----------------------------------------------------------------

    #[cfg(feature = "chrome")]
    async fn screenshot_as_data_url_with_profile(
        &self,
        page: &Page,
        cap: &CaptureProfile,
    ) -> EngineResult<String> {
        // clip is kept for forward-compat; not wired here (chromiumoxide uses CDP viewport types).
        let params = ScreenshotParams::builder()
            .format(CaptureScreenshotFormat::Png)
            .full_page(cap.full_page)
            .omit_background(cap.omit_background)
            .build();

        let png = page
            .screenshot(params)
            .await
            .map_err(|e| EngineError::Remote(format!("screenshot failed: {e}")))?;

        let b64 = general_purpose::STANDARD.encode(png);
        Ok(format!("data:image/png;base64,{}", b64))
    }

    #[cfg(feature = "chrome")]
    /// Take a final screenshot and return as base64 string.
    async fn take_final_screenshot(&self, page: &Page) -> EngineResult<String> {
        let params = ScreenshotParams::builder()
            .format(CaptureScreenshotFormat::Png)
            .full_page(true)
            .omit_background(true)
            .build();

        let png = page
            .screenshot(params)
            .await
            .map_err(|e| EngineError::Remote(format!("screenshot failed: {e}")))?;

        Ok(general_purpose::STANDARD.encode(png))
    }

    #[cfg(feature = "chrome")]
    /// Title context.
    async fn title_context(&self, page: &Page, effective_cfg: &RemoteMultimodalConfig) -> String {
        if !effective_cfg.include_title {
            return String::new();
        }
        match page.get_title().await {
            Ok(t) => t.unwrap_or_default(),
            Err(_) => String::new(),
        }
    }

    #[cfg(feature = "chrome")]
    /// Build the user prompt for this round using captured state.
    /// Keep this as the single place to format the context.
    fn build_user_prompt(
        &self,
        effective_cfg: &RemoteMultimodalConfig,
        _cap: &CaptureProfile,
        url_input: &str,
        url_now: &str,
        title_now: &str,
        html: &str,
        round_idx: usize,
        stagnated: bool,
    ) -> String {
        // pre-size: helps avoid repeated allocations
        let mut out = String::with_capacity(
            256 + url_input.len()
                + url_now.len()
                + title_now.len()
                + html.len().min(effective_cfg.html_max_bytes),
        );

        out.push_str("RUN CONTEXT:\n");
        out.push_str("- url_input: ");
        out.push_str(url_input);
        out.push('\n');
        out.push_str("- round: ");
        out.push_str(&(round_idx + 1).to_string());
        out.push('\n');
        out.push_str("- stagnated: ");
        out.push_str(if stagnated { "true" } else { "false" });
        out.push_str("\n\n");

        if effective_cfg.include_url && !url_now.is_empty() {
            out.push_str("CURRENT URL:\n");
            out.push_str(url_now);
            out.push_str("\n\n");
        }

        if effective_cfg.include_title && !title_now.is_empty() {
            out.push_str("PAGE TITLE:\n");
            out.push_str(title_now);
            out.push_str("\n\n");
        }

        if effective_cfg.include_html && !html.is_empty() {
            out.push_str("HTML CONTEXT:\n");
            out.push_str(html);
            out.push_str("\n\n");
        }

        out.push_str(
            "TASK:\nReturn the next automation steps as a single JSON object (no prose).\n",
        );

        out
    }

    #[cfg(feature = "chrome")]
    /// Html context profile.
    async fn html_context_with_profile(
        &self,
        page: &Page,
        effective_cfg: &RemoteMultimodalConfig,
        cap: &CaptureProfile,
    ) -> EngineResult<String> {
        if !effective_cfg.include_html {
            return Ok(String::new());
        }

        let raw_html = page
            .content()
            .await
            .map_err(|e| EngineError::Remote(format!("page.content() failed: {e}")))?;

        let cleaned = clean_html_with_profile(&raw_html, cap.html_cleaning);

        // effective max = min(profile, cfg)
        let max = cap.html_max_bytes.min(effective_cfg.html_max_bytes);
        Ok(truncate_utf8_tail(&cleaned, max))
    }

    #[cfg(feature = "chrome")]
    /// Url context.
    async fn url_context(&self, page: &Page, effective_cfg: &RemoteMultimodalConfig) -> String {
        if !effective_cfg.include_url {
            return String::new();
        }
        // IMPORTANT: your Page::url() appears to return Option<String> (based on earlier error).
        // We normalize it to a plain String here.
        match page.url().await {
            Ok(u_opt) => u_opt.unwrap_or_default(),
            Err(_) => String::new(),
        }
    }

    // -----------------------------------------------------------------
    // Main loop: iterate until done
    // -----------------------------------------------------------------

    /// Runs iterative automation until the model declares completion or `max_rounds` is reached.
    ///
    /// Contract:
    /// - Each round captures screenshot + (optional) URL/title/HTML
    /// - Model returns a JSON object:
    ///   `{ "label": "...", "done": true|false, "steps": [ ... ] }`
    /// - If `done == true` OR `steps` empty => stop successfully.
    /// - Otherwise execute steps and loop again.
    ///
    /// URL gating:
    /// - If `prompt_url_gate` is set, automation only runs for matched URLs.
    /// - A match may supply an override config (merged onto base config).
    #[cfg(feature = "chrome")]
    pub async fn run(&self, page: &Page, url_input: &str) -> EngineResult<AutomationResult> {
        // 0) URL gating + config override
        let cfg_override: Option<&RemoteMultimodalConfig> =
            if let Some(gate) = &self.prompt_url_gate {
                match gate.match_url(url_input) {
                    Some(maybe_cfg) => maybe_cfg,
                    None => {
                        return Ok(AutomationResult {
                            label: "url_not_allowed".into(),
                            steps_executed: 0,
                            success: true,
                            error: None,
                            usage: AutomationUsage::default(),
                            extracted: None,
                            screenshot: None,
                        });
                    }
                }
            } else {
                None
            };

        let base_effective_cfg: RemoteMultimodalConfig = match cfg_override {
            Some(override_cfg) => merged_config(&self.cfg, override_cfg),
            None => self.cfg.clone(),
        };

        // capture profiles fallback
        let capture_profiles: Vec<CaptureProfile> =
            if base_effective_cfg.capture_profiles.is_empty() {
                vec![
                    CaptureProfile {
                        full_page: false,
                        omit_background: true,
                        html_cleaning: HtmlCleaningProfile::Default,
                        html_max_bytes: base_effective_cfg.html_max_bytes,
                        attempt_note: Some("default profile 1: viewport screenshot".into()),
                        ..Default::default()
                    },
                    CaptureProfile {
                        full_page: true,
                        omit_background: true,
                        html_cleaning: HtmlCleaningProfile::Aggressive,
                        html_max_bytes: base_effective_cfg.html_max_bytes,
                        attempt_note: Some(
                            "default profile 2: full-page screenshot + aggressive HTML".into(),
                        ),
                        ..Default::default()
                    },
                ]
            } else {
                base_effective_cfg.capture_profiles.clone()
            };

        let mut total_steps_executed = 0usize;
        let mut last_label = String::from("automation");
        let mut last_sig: Option<StateSignature> = None;
        let mut total_usage = AutomationUsage::default();
        let mut last_extracted: Option<serde_json::Value> = None;

        let rounds = base_effective_cfg.max_rounds.max(1);
        for round_idx in 0..rounds {
            // pick capture profile by round (clamp to last)
            let cap = capture_profiles
                .get(round_idx)
                .unwrap_or_else(|| capture_profiles.last().expect("non-empty capture_profiles"));

            // Capture state
            let screenshot_fut = self.screenshot_as_data_url_with_profile(page, cap);
            let html_fut = self.html_context_with_profile(page, &base_effective_cfg, cap);

            // IMPORTANT: actually use the helper methods (they were previously unused).
            // They return plain Strings, so wrap into Ok(..) to keep try_join! shape.
            let url_fut = async {
                Ok::<String, EngineError>(self.url_context(page, &base_effective_cfg).await)
            };
            let title_fut = async {
                Ok::<String, EngineError>(self.title_context(page, &base_effective_cfg).await)
            };

            let (screenshot, html, mut url_now, title_now) =
                tokio::try_join!(screenshot_fut, html_fut, url_fut, title_fut)?;

            // Fallback to the input URL if page.url() is empty/unsupported.
            if url_now.is_empty() {
                url_now = url_input.to_string();
            }

            // quick stagnation heuristic (don’t hard-stop; just a hint to the model)
            let sig = StateSignature::new(&url_now, &title_now, &html);
            let stagnated = last_sig.as_ref().map(|p| p.eq_soft(&sig)).unwrap_or(false);
            last_sig = Some(sig);

            // Ask model (with retry policy)
            let plan = self
                .infer_plan_with_retry(
                    &base_effective_cfg,
                    cap,
                    url_input,
                    &url_now,
                    &title_now,
                    &html,
                    &screenshot,
                    round_idx,
                    stagnated,
                )
                .await?;

            // Accumulate token usage from this round
            total_usage.accumulate(&plan.usage);
            last_label = plan.label.clone();

            // Save extracted data if present
            if plan.extracted.is_some() {
                last_extracted = plan.extracted.clone();
            }

            // Done condition (model-driven)
            if plan.done || plan.steps.is_empty() {
                // Take final screenshot if enabled
                let final_screenshot = if base_effective_cfg.screenshot {
                    self.take_final_screenshot(page).await.ok()
                } else {
                    None
                };

                return Ok(AutomationResult {
                    label: plan.label,
                    steps_executed: total_steps_executed,
                    success: true,
                    error: None,
                    usage: total_usage,
                    extracted: last_extracted,
                    screenshot: final_screenshot,
                });
            }

            // Execute steps
            let mut executed_this_round = 0usize;
            let mut failed: Option<String> = None;

            for step in &plan.steps {
                if !step.run(page).await {
                    failed = Some(format!(
                        "round {} step {} failed: {:?}",
                        round_idx, executed_this_round, step
                    ));
                    break;
                }
                executed_this_round += 1;
                total_steps_executed += 1;
            }

            if let Some(err) = failed {
                // optional retry-on-step-failure at the outer loop level:
                if base_effective_cfg.retry.retry_on_step_failure
                    && round_idx + 1 < rounds
                    && base_effective_cfg.retry.backoff_ms > 0
                {
                    tokio::time::sleep(std::time::Duration::from_millis(
                        base_effective_cfg.retry.backoff_ms,
                    ))
                    .await;
                    continue;
                }

                // Take screenshot on failure if enabled
                let final_screenshot = if base_effective_cfg.screenshot {
                    self.take_final_screenshot(page).await.ok()
                } else {
                    None
                };

                return Ok(AutomationResult {
                    label: last_label,
                    steps_executed: total_steps_executed,
                    success: false,
                    error: Some(err),
                    usage: total_usage,
                    extracted: last_extracted,
                    screenshot: final_screenshot,
                });
            }

            // Wait a bit before re-capture
            if base_effective_cfg.post_plan_wait_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(
                    base_effective_cfg.post_plan_wait_ms,
                ))
                .await;
            }
        }

        // Take final screenshot if enabled
        let final_screenshot = if base_effective_cfg.screenshot {
            self.take_final_screenshot(page).await.ok()
        } else {
            None
        };

        Ok(AutomationResult {
            label: last_label,
            steps_executed: total_steps_executed,
            success: false,
            error: Some("max_rounds reached without model declaring done".into()),
            usage: total_usage,
            extracted: last_extracted,
            screenshot: final_screenshot,
        })
    }

    // -----------------------------------------------------------------
    // Inference (single plan) with retry policy
    // -----------------------------------------------------------------

    #[cfg(feature = "chrome")]
    async fn infer_plan_with_retry(
        &self,
        effective_cfg: &RemoteMultimodalConfig,
        cap: &CaptureProfile,
        url_input: &str,
        url_now: &str,
        title_now: &str,
        html: &str,
        screenshot: &str,
        round_idx: usize,
        stagnated: bool,
    ) -> EngineResult<ParsedPlan> {
        let max_attempts = effective_cfg.retry.max_attempts.max(1);
        let mut last_err: Option<EngineError> = None;

        // Build the user prompt ONCE so retries don't accidentally drift.
        // This also ensures `build_user_prompt` is used and stays the single source of truth.
        let user_prompt = self.build_user_prompt(
            effective_cfg,
            cap,
            url_input,
            url_now,
            title_now,
            html,
            round_idx,
            stagnated,
        );

        for attempt_idx in 0..max_attempts {
            match self
                .infer_plan_once(effective_cfg, cap, &user_prompt, screenshot, attempt_idx)
                .await
            {
                Ok(plan) => return Ok(plan),
                Err(e) => {
                    let retryable_parse = matches!(
                        e,
                        EngineError::Json(_)
                            | EngineError::InvalidField(_)
                            | EngineError::MissingField(_)
                    );

                    let should_retry = effective_cfg.retry.retry_on_parse_error
                        && retryable_parse
                        && attempt_idx + 1 < max_attempts;

                    last_err = Some(e);

                    if should_retry {
                        let backoff_ms = effective_cfg.retry.backoff_ms;
                        if backoff_ms > 0 {
                            tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
                        }
                        continue;
                    }

                    return Err(last_err.take().expect("just set"));
                }
            }
        }

        Err(last_err.unwrap_or_else(|| EngineError::Remote("inference failed".into())))
    }

    #[cfg(feature = "chrome")]
    // One inference attempt (no retry here).
    ///
    /// `user_prompt` is pre-built by `build_user_prompt` so it stays consistent across retries.
    async fn infer_plan_once(
        &self,
        effective_cfg: &RemoteMultimodalConfig,
        cap: &CaptureProfile,
        user_prompt: &str,
        screenshot: &str,
        attempt_idx: usize,
    ) -> EngineResult<ParsedPlan> {
        use serde::Serialize;
        use serde_json::Value;

        #[derive(Serialize)]
        struct ImageUrl {
            url: String,
        }

        #[derive(Serialize)]
        struct ContentBlock {
            #[serde(rename = "type")]
            content_type: String,
            #[serde(skip_serializing_if = "Option::is_none")]
            text: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            image_url: Option<ImageUrl>,
        }

        #[derive(Serialize)]
        struct Message {
            role: String,
            content: Vec<ContentBlock>,
        }

        #[derive(Serialize)]
        struct ResponseFormat {
            #[serde(rename = "type")]
            format_type: String,
        }

        #[derive(Serialize)]
        struct InferenceRequest {
            model: String,
            messages: Vec<Message>,
            temperature: f32,
            max_tokens: u16,
            #[serde(skip_serializing_if = "Option::is_none")]
            #[serde(rename = "response_format")]
            response_format: Option<ResponseFormat>,
        }

        // Include per-attempt note (optional) without rebuilding the whole prompt.
        // This is useful when retrying: you can tell the model it is a retry.
        let mut user_text = String::with_capacity(user_prompt.len() + 128);
        user_text.push_str(user_prompt);

        // Retry hint for the model (helps in practice).
        if attempt_idx > 0 {
            user_text.push_str("\n---\nRETRY:\n");
            user_text.push_str("The previous response was invalid or could not be parsed. Return ONLY a single JSON object.\n");
        }

        if let Some(note) = &cap.attempt_note {
            if !note.trim().is_empty() {
                user_text.push_str("\n---\nCAPTURE PROFILE NOTE:\n");
                user_text.push_str(note.trim());
                user_text.push('\n');
            }
        }

        if let Some(extra) = &self.user_message_extra {
            if !extra.trim().is_empty() {
                user_text.push_str("\n---\nUSER INSTRUCTIONS:\n");
                user_text.push_str(extra.trim());
                user_text.push('\n');
            }
        }

        let request_body = InferenceRequest {
            model: self.model_name.clone(),
            messages: vec![
                Message {
                    role: "system".into(),
                    content: vec![ContentBlock {
                        content_type: "text".into(),
                        text: Some(self.system_prompt_compiled(effective_cfg)),
                        image_url: None,
                    }],
                },
                Message {
                    role: "user".into(),
                    content: vec![
                        ContentBlock {
                            content_type: "text".into(),
                            text: Some(user_text),
                            image_url: None,
                        },
                        ContentBlock {
                            content_type: "image_url".into(),
                            text: None,
                            image_url: Some(ImageUrl {
                                url: screenshot.to_string(),
                            }),
                        },
                    ],
                },
            ],
            temperature: effective_cfg.temperature,
            max_tokens: effective_cfg.max_tokens,
            response_format: if effective_cfg.request_json_object {
                Some(ResponseFormat {
                    format_type: "json_object".into(),
                })
            } else {
                None
            },
        };

        // Acquire permit BEFORE sending to throttle concurrent LLM calls.
        let _permit = self.acquire_llm_permit().await;

        let mut req = CLIENT.post(&self.api_url).json(&request_body);
        if let Some(key) = &self.api_key {
            req = req.bearer_auth(key);
        }

        let start = std::time::Instant::now();
        let http_resp = req.send().await?;
        let status = http_resp.status();
        let raw_body = http_resp.text().await?;

        log::debug!(
            "remote_multimodal: status={} latency={:?} body_len={}",
            status,
            start.elapsed(),
            raw_body.len()
        );

        if !status.is_success() {
            return Err(EngineError::Remote(format!(
                "non-success status {status}: {raw_body}"
            )));
        }

        let root: Value = serde_json::from_str(&raw_body)?;
        let content = extract_assistant_content(&root)
            .ok_or(EngineError::MissingField("choices[0].message.content"))?;

        // Extract token usage from OpenAI-compatible response
        let usage = extract_usage(&root);

        let plan_value = if effective_cfg.best_effort_json_extract {
            best_effort_parse_json_object(&content)?
        } else {
            serde_json::from_str::<Value>(&content)?
        };

        let label = plan_value
            .get("label")
            .and_then(|v| v.as_str())
            .unwrap_or("automation_plan")
            .to_string();

        let done = plan_value
            .get("done")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let steps_arr = match plan_value.get("steps") {
            Some(v) => v
                .as_array()
                .ok_or(EngineError::InvalidField("steps must be an array"))?
                .clone(),
            None => {
                // If model omitted steps but said done=true, treat as done.
                if done {
                    Vec::new()
                } else {
                    return Err(EngineError::MissingField("steps"));
                }
            }
        };

        let steps = map_to_web_automation(steps_arr)?;

        // Extract structured data if present (used when extra_ai_data is enabled)
        let extracted = plan_value.get("extracted").cloned();

        Ok(ParsedPlan {
            label,
            done,
            steps,
            usage,
            extracted,
        })
    }
}

/// Parsed plan returned by the model.
///
/// This is an internal, fully-validated representation of the model output.
/// The engine converts the raw JSON plan into this type after:
/// - extracting assistant text from the provider response,
/// - parsing JSON (optionally best-effort),
/// - validating required fields (`label`, `steps`),
/// - deserializing `steps` into concrete [`WebAutomation`] actions.
///
/// ## Fields
/// - `label`: Human-readable short description of the plan (from the model).
/// - `done`: Whether the model indicates no further automation is required.
///   When `done == true`, the engine treats the run as complete and stops looping.
/// - `steps`: Concrete automation steps to execute on the page.
/// - `usage`: Token usage from this inference call.
/// - `extracted`: Optional structured data extracted from the page.
///
/// Note: `ParsedPlan` is intentionally not public because its shape may change
/// as the engine evolves its planning loop (e.g., adding confidence, reasons,
/// or termination hints).
#[derive(Debug, Clone)]
#[cfg(feature = "chrome")]
struct ParsedPlan {
    /// Human-readable short description of the plan (from the model).
    label: String,
    /// Whether the model indicates completion (no more steps required).
    done: bool,
    /// Concrete automation steps to execute.
    steps: Vec<crate::features::chrome_common::WebAutomation>,
    /// Token usage from this inference call.
    usage: AutomationUsage,
    /// Structured data extracted from the page (when extraction is enabled).
    extracted: Option<serde_json::Value>,
}

/// Token usage returned from OpenAI-compatible endpoints.
///
/// This struct tracks the token consumption for remote multimodal automation,
/// conforming to the OpenAI API response format.
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AutomationUsage {
    /// The number of tokens used in the prompt.
    pub prompt_tokens: u32,
    /// The number of tokens used in the completion.
    pub completion_tokens: u32,
    /// The total number of tokens used.
    pub total_tokens: u32,
}

impl AutomationUsage {
    /// Create a new usage instance.
    pub fn new(prompt_tokens: u32, completion_tokens: u32, total_tokens: u32) -> Self {
        Self {
            prompt_tokens,
            completion_tokens,
            total_tokens,
        }
    }

    /// Accumulate usage from another instance.
    pub fn accumulate(&mut self, other: &Self) {
        self.prompt_tokens = self.prompt_tokens.saturating_add(other.prompt_tokens);
        self.completion_tokens = self.completion_tokens.saturating_add(other.completion_tokens);
        self.total_tokens = self.total_tokens.saturating_add(other.total_tokens);
    }
}

/// Result returned to the caller.
///
/// This struct summarizes the outcome of running the remote multimodal automation loop.
/// It is returned by methods like `RemoteMultimodalEngine::run(...)`.
///
/// ## Semantics
/// - `success == true` means the engine finished without encountering a fatal error.
///   This typically implies the engine reached a completion condition (model said `done`,
///   no steps were needed, or the engine determined the page is stable / complete).
/// - `success == false` means execution stopped due to an error (HTTP/JSON/provider error)
///   or because a [`WebAutomation`] step failed to execute.
///
/// If `success == false`, `error` should contain a human-readable explanation.
///
/// ## Fields
/// - `label`: The last plan label produced by the model (or an engine-generated label
///   such as `"url_not_allowed"`).
/// - `steps_executed`: Total number of automation steps executed across the run.
/// - `success`: Whether the run completed successfully.
/// - `error`: Present when `success == false`.
/// - `usage`: Token usage accumulated across all inference rounds.
/// - `extracted`: Structured data extracted from the page (when `extra_ai_data` is enabled).
/// - `screenshot`: Base64-encoded screenshot (when `screenshot` is enabled).
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AutomationResult {
    /// Human-readable description of the last plan executed.
    pub label: String,
    /// Total number of steps executed before the run stopped.
    pub steps_executed: usize,
    /// Whether the run completed successfully.
    pub success: bool,
    /// Error message if the run failed.
    pub error: Option<String>,
    /// Token usage accumulated across all inference rounds.
    pub usage: AutomationUsage,
    /// Structured data extracted from the page (when `extra_ai_data` is enabled).
    #[cfg(feature = "serde")]
    pub extracted: Option<serde_json::Value>,
    /// Base64-encoded screenshot of the page after automation (when `screenshot` is enabled).
    pub screenshot: Option<String>,
}

#[cfg(feature = "chrome")]
impl AutomationResult {
    /// Convert to `AutomationResults` for storage in `Page.metadata.automation`.
    ///
    /// This allows the extraction results to be stored alongside other automation
    /// data in the page metadata, similar to how WebAutomation scripts store their results.
    ///
    /// # Example
    /// ```ignore
    /// let result = engine.run(&page, url).await?;
    ///
    /// // Store in page metadata
    /// if let Some(metadata) = page.metadata.as_mut() {
    ///     let automation_results = result.to_automation_results();
    ///     match metadata.automation.as_mut() {
    ///         Some(v) => v.push(automation_results),
    ///         None => metadata.automation = Some(vec![automation_results]),
    ///     }
    /// }
    /// ```
    pub fn to_automation_results(&self) -> crate::page::AutomationResults {
        crate::page::AutomationResults {
            input: self.label.clone(),
            content_output: self.extracted.clone().unwrap_or(serde_json::Value::Null),
            screenshot_output: self.screenshot.clone(),
            error: self.error.clone(),
        }
    }

    /// Convert to `AutomationResults` with a custom input label.
    pub fn to_automation_results_with_input(&self, input: &str) -> crate::page::AutomationResults {
        crate::page::AutomationResults {
            input: input.to_string(),
            content_output: self.extracted.clone().unwrap_or(serde_json::Value::Null),
            screenshot_output: self.screenshot.clone(),
            error: self.error.clone(),
        }
    }
}

/// A cheap signature of page state used to detect "no progress".
///
/// The engine captures a `StateSignature` after each round (plan → execute → re-capture)
/// and compares it to the previous signature. If the signature does not change across
/// rounds, the engine can conclude that automation is not making progress and stop early
/// (to avoid infinite loops).
///
/// This type is intentionally **lightweight**:
/// - It does not store full HTML.
/// - It uses a hash of only the tail of the cleaned HTML (where many challenge pages
///   place dynamic state).
///
/// ## Fields
/// - `url`: Current page URL after capture.
/// - `title`: Current document title after capture.
/// - `html_tail_hash`: Hash of the last N bytes of cleaned HTML.
/// - `html_len`: Total length of cleaned HTML (bytes or chars depending on capture).
#[derive(Debug, Clone)]
#[cfg(feature = "chrome")]
struct StateSignature {
    /// Current page URL.
    url: String,
    /// Current document title.
    title: String,
    /// Hash of the last N bytes of cleaned HTML (tail hash).
    html_tail_hash: u64,
    /// Total cleaned HTML length (used with the tail hash for stability checks).
    html_len: usize,
}

#[cfg(feature = "chrome")]
impl StateSignature {
    fn new(url: &str, title: &str, html: &str) -> Self {
        // Hash only tail slice to keep it cheap; still useful for stagnation detection.
        let tail = truncate_utf8_tail(html, 2048);
        let h = fnv1a64(tail.as_bytes());
        Self {
            url: url.to_string(),
            title: title.to_string(),
            html_tail_hash: h,
            html_len: html.len(),
        }
    }

    fn eq_soft(&self, other: &Self) -> bool {
        self.url == other.url
            && self.title == other.title
            && self.html_tail_hash == other.html_tail_hash
            && self.html_len == other.html_len
    }
}

#[cfg(feature = "chrome")]
fn fnv1a64(bytes: &[u8]) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;
    let mut h = FNV_OFFSET;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(FNV_PRIME);
    }
    h
}

// ---------------------------------------------------------------------
// Response parsing helpers (handles local OpenAI-like variants)
// ---------------------------------------------------------------------
#[cfg(feature = "serde")]
fn extract_assistant_content(root: &Value) -> Option<String> {
    let choice0 = root.get("choices")?.as_array()?.get(0)?;
    let msg = choice0.get("message").or_else(|| choice0.get("delta"))?;

    if let Some(c) = msg.get("content") {
        if let Some(s) = c.as_str() {
            return Some(s.to_string());
        }
        if let Some(arr) = c.as_array() {
            let mut out = String::new();
            for block in arr {
                if let Some(t) = block.get("text").and_then(|v| v.as_str()) {
                    out.push_str(t);
                } else if let Some(t) = block.get("content").and_then(|v| v.as_str()) {
                    out.push_str(t);
                }
            }
            if !out.is_empty() {
                return Some(out);
            }
        }
    }

    root.get("output_text")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Extract token usage from an OpenAI-compatible response.
///
/// The response format follows the OpenAI API structure:
/// ```json
/// {
///   "usage": {
///     "prompt_tokens": 123,
///     "completion_tokens": 456,
///     "total_tokens": 579
///   }
/// }
/// ```
///
/// Returns a default `AutomationUsage` if the usage field is missing or malformed.
#[cfg(feature = "serde")]
fn extract_usage(root: &Value) -> AutomationUsage {
    let usage = match root.get("usage") {
        Some(u) => u,
        None => return AutomationUsage::default(),
    };

    let prompt_tokens = usage
        .get("prompt_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;

    let completion_tokens = usage
        .get("completion_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;

    let total_tokens = usage
        .get("total_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or_else(|| (prompt_tokens + completion_tokens) as u64) as u32;

    AutomationUsage::new(prompt_tokens, completion_tokens, total_tokens)
}

/// Extract the LAST ```json``` or ``` code block from text.
/// Thinking/reasoning models often output multiple blocks, refining their answer.
/// The last block is typically the final, valid JSON.
#[cfg(feature = "serde")]
fn extract_last_code_block(s: &str) -> Option<&str> {
    let mut last_block: Option<&str> = None;
    let mut search_start = 0;

    // Find all ```json blocks and keep track of the last one
    while let Some(rel_start) = s[search_start..].find("```json") {
        let abs_start = search_start + rel_start + 7; // skip "```json"
        if abs_start < s.len() {
            if let Some(rel_end) = s[abs_start..].find("```") {
                let block = s[abs_start..abs_start + rel_end].trim();
                if !block.is_empty() {
                    last_block = Some(block);
                }
                search_start = abs_start + rel_end + 3;
            } else {
                // No closing fence, take rest of string
                let block = s[abs_start..].trim();
                if !block.is_empty() {
                    last_block = Some(block);
                }
                break;
            }
        } else {
            break;
        }
    }

    // If no ```json found, try generic ``` blocks
    if last_block.is_none() {
        search_start = 0;
        while let Some(rel_start) = s[search_start..].find("```") {
            let after_fence = search_start + rel_start + 3;
            if after_fence >= s.len() {
                break;
            }

            // Skip language identifier if present (e.g., ```javascript)
            let rest = &s[after_fence..];
            let content_start = rest
                .find('\n')
                .map(|i| after_fence + i + 1)
                .unwrap_or(after_fence);

            if content_start < s.len() {
                if let Some(rel_end) = s[content_start..].find("```") {
                    let block = s[content_start..content_start + rel_end].trim();
                    // Only consider blocks that look like JSON
                    if !block.is_empty()
                        && (block.starts_with('{') || block.starts_with('['))
                    {
                        last_block = Some(block);
                    }
                    search_start = content_start + rel_end + 3;
                } else {
                    break;
                }
            } else {
                break;
            }
        }
    }

    last_block
}

/// Extract the last balanced JSON object or array from text.
/// Uses proper brace matching to handle nested structures.
/// Returns the byte range (start, end) of the extracted JSON.
#[cfg(feature = "serde")]
fn extract_last_json_boundaries(s: &str, open: char, close: char) -> Option<(usize, usize)> {
    let bytes = s.as_bytes();
    let open_byte = open as u8;
    let close_byte = close as u8;

    // Find the last closing brace/bracket
    let mut end_pos = None;
    let mut i = bytes.len();
    while i > 0 {
        i -= 1;
        if bytes[i] == close_byte {
            end_pos = Some(i);
            break;
        }
    }

    let end_pos = end_pos?;

    // Walk backwards from end_pos, counting braces to find the matching opener
    // We need to handle:
    // 1. Nested braces of the same type
    // 2. Strings (don't count braces inside strings)
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escape_next = false;
    let mut pos = end_pos + 1;

    while pos > 0 {
        pos -= 1;
        let ch = bytes[pos];

        if escape_next {
            escape_next = false;
            continue;
        }

        // Check for escape (but we're going backwards, so check if previous char is backslash)
        // This is tricky going backwards - simplified: just track string state
        if ch == b'"' && !escape_next {
            in_string = !in_string;
            continue;
        }

        if in_string {
            // Check if this quote was escaped (look ahead since we're going backwards)
            if pos > 0 && bytes[pos - 1] == b'\\' {
                // Count consecutive backslashes
                let mut backslash_count = 0;
                let mut check_pos = pos - 1;
                while check_pos > 0 && bytes[check_pos] == b'\\' {
                    backslash_count += 1;
                    if check_pos == 0 {
                        break;
                    }
                    check_pos -= 1;
                }
                // If odd number of backslashes, this quote is escaped
                if backslash_count % 2 == 1 {
                    in_string = !in_string; // undo the toggle
                }
            }
            continue;
        }

        if ch == close_byte {
            depth += 1;
        } else if ch == open_byte {
            depth -= 1;
            if depth == 0 {
                return Some((pos, end_pos + 1));
            }
        }
    }

    None
}

/// Extract the last JSON object from text with proper brace matching.
#[cfg(feature = "serde")]
fn extract_last_json_object(s: &str) -> Option<&str> {
    extract_last_json_boundaries(s, '{', '}').map(|(start, end)| &s[start..end])
}

/// Extract the last JSON array from text with proper brace matching.
#[cfg(feature = "serde")]
fn extract_last_json_array(s: &str) -> Option<&str> {
    extract_last_json_boundaries(s, '[', ']').map(|(start, end)| &s[start..end])
}

/// Best effort parse the json object.
///
/// Handles common LLM output quirks:
/// - Multiple ```json``` blocks (uses the LAST one, as thinking models refine answers)
/// - Reasoning/thinking text before JSON
/// - Nested JSON structures (proper brace matching)
/// - Malformed JSON (when `llm_json` feature is enabled)
#[cfg(feature = "serde")]
fn best_effort_parse_json_object(s: &str) -> EngineResult<Value> {
    // Try direct parse first
    if let Ok(v) = serde_json::from_str::<Value>(s) {
        return Ok(v);
    }

    let trimmed = s.trim();

    // 1. Try to extract the LAST code block (thinking models refine their answer)
    if let Some(block) = extract_last_code_block(trimmed) {
        if let Ok(v) = serde_json::from_str::<Value>(block) {
            return Ok(v);
        }

        // Try llm_json repair on the extracted block
        #[cfg(feature = "llm_json")]
        if let Ok(v) = llm_json::loads(block, &Default::default()) {
            return Ok(v);
        }

        // The code block might have prose - try extracting JSON from within it
        if let Some(obj) = extract_last_json_object(block) {
            if let Ok(v) = serde_json::from_str::<Value>(obj) {
                return Ok(v);
            }
            #[cfg(feature = "llm_json")]
            if let Ok(v) = llm_json::loads(obj, &Default::default()) {
                return Ok(v);
            }
        }
    }

    // 2. Strip markdown fences if at boundaries
    let unfenced = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .map(|x| x.trim())
        .unwrap_or(trimmed);
    let unfenced = unfenced
        .strip_suffix("```")
        .map(|x| x.trim())
        .unwrap_or(unfenced);

    if let Ok(v) = serde_json::from_str::<Value>(unfenced) {
        return Ok(v);
    }

    // 3. Extract last JSON object with proper brace matching
    if let Some(obj) = extract_last_json_object(unfenced) {
        if let Ok(v) = serde_json::from_str::<Value>(obj) {
            return Ok(v);
        }

        #[cfg(feature = "llm_json")]
        if let Ok(v) = llm_json::loads(obj, &Default::default()) {
            return Ok(v);
        }
    }

    // 4. Extract last JSON array with proper bracket matching
    if let Some(arr) = extract_last_json_array(unfenced) {
        if let Ok(v) = serde_json::from_str::<Value>(arr) {
            return Ok(v);
        }

        #[cfg(feature = "llm_json")]
        if let Ok(v) = llm_json::loads(arr, &Default::default()) {
            return Ok(v);
        }
    }

    // 5. Try llm_json repair on unfenced content as last resort
    #[cfg(feature = "llm_json")]
    if let Ok(v) = llm_json::loads(unfenced, &Default::default()) {
        return Ok(v);
    }

    // 6. Final attempt: try llm_json on the entire input
    #[cfg(feature = "llm_json")]
    if let Ok(v) = llm_json::loads(trimmed, &Default::default()) {
        return Ok(v);
    }

    Err(EngineError::InvalidField(
        "assistant content was not a JSON object",
    ))
}

/// Map raw JSON -> concrete WebAutomation
#[cfg(feature = "chrome")]
fn map_to_web_automation(
    raw_steps: Vec<Value>,
) -> EngineResult<Vec<crate::features::chrome_common::WebAutomation>> {
    #[cfg(feature = "serde")]
    {
        let mut out = Vec::with_capacity(raw_steps.len());
        for v in raw_steps {
            let step: crate::features::chrome_common::WebAutomation =
                serde_json::from_value(v).map_err(EngineError::Json)?;
            out.push(step);
        }
        Ok(out)
    }

    #[cfg(not(feature = "serde"))]
    {
        let _ = raw_steps;
        Err(EngineError::Unsupported(
            "mapping steps requires feature=serde (WebAutomation must be Deserialize)",
        ))
    }
}

/// Default system prompt web challenge automation (iterative).
pub const DEFAULT_SYSTEM_PROMPT: &str = r##"
You are an advanced web automation agent designed to solve CTFs, puzzles, and bypass anti‑bot challenges.

You receive (each round):
- A screenshot (image_url)
- Optional current URL
- Optional page title
- Optional cleaned HTML context
- Round/attempt metadata

You MUST output a single JSON object ONLY (no prose), with shape:
{
  "label": "short description",
  "done": true|false,
  "steps": [ ... ]
}

Completion rules:
- If the task/challenge is solved OR the user goal is satisfied, set "done": true and set "steps": [].
- If additional actions are needed, set "done": false and provide next steps.

The steps MUST be valid Rust-like enum objects for `WebAutomation` (externally deserialized).
Use ONLY the actions listed below and follow their exact shapes.

Allowed `WebAutomation` actions:

- { "Evaluate": "javascript code" }

- { "Click": "css_selector" }
- { "ClickAll": "css_selector" }
- { "ClickPoint": { "x": 123.0, "y": 456.0 } }

- { "ClickHold": { "selector": "css_selector", "hold_ms": 800 } }
- { "ClickHoldPoint": { "x": 100.0, "y": 200.0, "hold_ms": 800 } }

- { "ClickDrag": { "from": "css_selector", "to": "css_selector", "modifier": 8 } }
- { "ClickDragPoint": { "from_x": 10.0, "from_y": 10.0, "to_x": 300.0, "to_y": 300.0, "modifier": null } }

- { "ClickAllClickable": null }

- { "Wait": 1000 }
- { "WaitForNavigation": null }

- { "WaitForDom": { "selector": "#container", "timeout": 5000 } }   // selector may be null
- { "WaitFor": "css_selector" }
- { "WaitForWithTimeout": { "selector": "css_selector", "timeout": 8000 } }
- { "WaitForAndClick": "css_selector" }

- { "ScrollX": 200 }
- { "ScrollY": 600 }
- { "InfiniteScroll": 10 }

- { "Fill": { "selector": "#input", "value": "text" } }
- { "Type": { "value": "text", "modifier": null } }

- { "Screenshot": { "full_page": true, "omit_background": true, "output": "out.png" } }

- { "ValidateChain": null }

Rules:
1) Prefer selector-based actions over coordinate clicks.
2) Use WaitFor / WaitForWithTimeout before clicking if the page is dynamic.
3) Use WaitForNavigation when a click likely triggers navigation.
4) If you see stagnation (state not changing), try a different strategy: different selector, scroll, or small waits.
5) Output JSON only.
"##;

/// Merged prompt configuration.
#[cfg(feature = "chrome")]
fn merged_config(
    base: &RemoteMultimodalConfig,
    override_cfg: &RemoteMultimodalConfig,
) -> RemoteMultimodalConfig {
    let mut out = base.clone();

    out.include_html = override_cfg.include_html;
    out.html_max_bytes = override_cfg.html_max_bytes;
    out.include_url = override_cfg.include_url;
    out.include_title = override_cfg.include_title;

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

/// Clean HTML using your existing utils + optional additional variants.
#[cfg(feature = "chrome")]
fn clean_html_with_profile(html: &str, profile: HtmlCleaningProfile) -> String {
    match profile {
        HtmlCleaningProfile::Raw => crate::utils::clean_html_raw(html),
        HtmlCleaningProfile::Default => crate::utils::clean_html(html),
        HtmlCleaningProfile::Aggressive => crate::utils::clean_html_full(html),
        HtmlCleaningProfile::Minimal => crate::utils::clean_html_base(html),
    }
}

/// Take the last `max_bytes` of a UTF-8 string without splitting code points.
/// Returns a string with a `...[truncated]...` prefix when truncated.
pub fn truncate_utf8_tail(s: &str, max_bytes: usize) -> String {
    let bytes = s.as_bytes();
    if bytes.len() <= max_bytes {
        return s.to_string();
    }

    let mut start = bytes.len().saturating_sub(max_bytes);
    while start < bytes.len() && !s.is_char_boundary(start) {
        start += 1;
    }

    let tail = &s[start..];
    let mut out = String::with_capacity(tail.len() + 20);
    out.push_str("...[truncated]...");
    out.push_str(tail);
    out
}

#[cfg(feature = "chrome")]
/// Resolve a per-URL effective config (base + optional override) using the engine's URL gate.
///
/// Returns:
/// - `Ok(None)` if automation is *not allowed* for this URL (gate exists, but no match)
/// - `Ok(Some(cfg))` if automation is allowed, with merged effective config
/// - `Ok(Some(base_cfg))` if there is no gate on the engine
pub fn effective_multimodal_config_for_url(
    engine: &RemoteMultimodalEngine,
    url: &str,
) -> EngineResult<Option<RemoteMultimodalConfig>> {
    // You need these accessors on the engine. If you don't have them yet,
    // add them (shown below).
    let base = engine.config().clone();

    if let Some(gate) = engine.prompt_url_gate() {
        match gate.match_url(url) {
            Some(Some(override_cfg)) => Ok(Some(merged_config(&base, override_cfg))),
            Some(None) => Ok(Some(base)),
            None => Ok(None), // gate exists but URL not allowed
        }
    } else {
        Ok(Some(base))
    }
}

#[cfg(feature = "chrome")]
/// Run the remote multi-modal configuration with a browser page.
pub async fn run_remote_multimodal_if_enabled(
    cfgs: &Option<Box<crate::features::automation::RemoteMultimodalConfigs>>,
    page: &chromiumoxide::Page,
    url: &str,
) -> Result<
    Option<crate::features::automation::AutomationResult>,
    crate::features::automation::EngineError,
> {
    let cfgs = match cfgs.as_deref() {
        Some(c) => c,
        None => return Ok(None),
    };

    let sem = cfgs.get_or_init_semaphore();
    let result = crate::features::automation::RemoteMultimodalEngine::new(
        cfgs.api_url.clone(),
        cfgs.model_name.clone(),
        cfgs.system_prompt.clone(),
    )
    .with_api_key(cfgs.api_key.as_deref())
    .with_system_prompt_extra(cfgs.system_prompt_extra.as_deref())
    .with_user_message_extra(cfgs.user_message_extra.as_deref())
    .with_remote_multimodal_config(cfgs.cfg.clone())
    .with_prompt_url_gate(cfgs.prompt_url_gate.clone())
    .with_semaphore(sem)
    .run(page, url)
    .await?;

    Ok(Some(result))
}

/// Run remote multi-modal extraction on raw HTML content (no browser required).
///
/// This function enables extraction from HTTP responses without requiring Chrome.
/// It sends the HTML content to the multimodal model for structured data extraction.
///
/// Note: This only supports extraction (`extra_ai_data`), not browser automation.
#[cfg(feature = "serde")]
pub async fn run_remote_multimodal_extraction(
    cfgs: &Option<Box<crate::features::automation::RemoteMultimodalConfigs>>,
    html: &str,
    url: &str,
    title: Option<&str>,
) -> Result<Option<crate::features::automation::AutomationResult>, crate::features::automation::EngineError>
{
    let cfgs = match cfgs.as_deref() {
        Some(c) => c,
        None => return Ok(None),
    };

    // Only run if extraction is enabled
    if !cfgs.cfg.extra_ai_data {
        return Ok(None);
    }

    // Check URL gating
    if let Some(gate) = &cfgs.prompt_url_gate {
        if gate.match_url(url).is_none() {
            return Ok(Some(AutomationResult {
                label: "url_not_allowed".into(),
                steps_executed: 0,
                success: true,
                error: None,
                usage: AutomationUsage::default(),
                extracted: None,
                screenshot: None,
            }));
        }
    }

    let sem = cfgs.get_or_init_semaphore();
    let mut engine = crate::features::automation::RemoteMultimodalEngine::new(
        cfgs.api_url.clone(),
        cfgs.model_name.clone(),
        cfgs.system_prompt.clone(),
    )
    .with_api_key(cfgs.api_key.as_deref());

    engine.with_system_prompt_extra(cfgs.system_prompt_extra.as_deref());
    engine.with_user_message_extra(cfgs.user_message_extra.as_deref());
    engine.with_remote_multimodal_config(cfgs.cfg.clone());
    engine.with_prompt_url_gate(cfgs.prompt_url_gate.clone());
    engine.with_semaphore(sem);

    let result = engine.extract_from_html(html, url, title).await?;
    Ok(Some(result))
}

#[cfg(feature = "chrome")]
impl RemoteMultimodalEngine {
    /// Borrow the engine config.
    #[inline]
    pub fn config(&self) -> &RemoteMultimodalConfig {
        &self.cfg
    }

    /// Borrow the optional URL gate.
    #[inline]
    pub fn prompt_url_gate(&self) -> Option<&PromptUrlGate> {
        self.prompt_url_gate.as_ref()
    }

    /// Clone the engine but replace the config.
    ///
    /// This avoids making callers rebuild the engine. Useful for per-URL overrides.
    #[inline]
    pub fn clone_with_cfg(&self, cfg: RemoteMultimodalConfig) -> Self {
        Self {
            api_url: self.api_url.clone(),
            api_key: self.api_key.clone(),
            model_name: self.model_name.clone(),
            system_prompt: self.system_prompt.clone(),
            system_prompt_extra: self.system_prompt_extra.clone(),
            user_message_extra: self.user_message_extra.clone(),
            cfg,
            prompt_url_gate: self.prompt_url_gate.clone(),
            semaphore: self.semaphore.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test extraction from thinking model output with multiple JSON blocks.
    /// The model refines its answer through multiple ```json``` blocks,
    /// and we should extract the LAST one.
    #[cfg(feature = "serde")]
    #[test]
    fn test_extract_last_json_from_thinking_output() {
        let thinking_output = r#"1. **Analyze the Request:**

* **Goal:** Extract book details from the provided HTML content.

* **Output Format:** Valid JSON.

* **Input:** A snippet of HTML representing a product page for a book named "A Light in the Attic".

2. **Scan the HTML for Key Data Points:**

* *Title:* Found in `<h1>A Light in the Attic</h1>`.

* *Price:* Found in `<p class="price_color">£51.77</p>`.

* *Availability:* Found in `<p class="instock availability">... In stock (22 available) ...</p>`.

* *Rating:* Found in `<p class="star-rating Three">... 5 stars ...</p>`.

* *UPC:* Found in `<tr><th>UPC</th><td>a897fe39b1053632</td></tr>`.

* *Product Type:* Found in `<tr><th>Product Type</th><td>Books</td></tr>`.

* *Number of Reviews:* Found in `<tr><th>Number of reviews</th><td>0</td></tr>`.

3. **Structure the JSON:**

* I need a root object. Let's call it `book`.

* Inside `book`, I'll list the extracted fields.

* *Drafting the JSON structure:*

```json
{
"title": "A Light in the Attic",
"price": "£51.77",
"availability": "In stock (22 available)",
"rating": "5",
"upc": "a897fe39b1053632",
"product_type": "Books",
"number_of_reviews": "0"
}
```

4. **Refining the Data:**

* *Title:* "A Light in the Attic"

* *Price:* "£51.77"

* *Availability:* "In stock (22 available)" (or just "In stock"). The HTML says "In stock (22 available)". I'll keep the full string or just "In stock". Let's stick to the text in the HTML for accuracy.

* *Rating:* The class is `star-rating Three`. The text says "Five stars" (implied by the 5 icons). I'll extract the number of stars or the class name. "5" is a safe integer.

* *UPC:* "a897fe39b1053632"

* *Product Type:* "Books"

* *Number of Reviews:* "0"

5. **Final JSON Construction:**

```json
{
"title": "A Light in the Attic",
"price": "£51.77",
"availability": "In stock (22 available)",
"rating": "5",
"upc": "a897fe39b1053632",
"product_type": "Books",
"number_of_reviews": "0"
}
```

6. **Verification:**

* Does the JSON contain all the relevant book info? Yes.

* Is the JSON valid? Yes.

* Are the values extracted correctly from the HTML tags? Yes.

7. **Final Output Generation:** (Produce the JSON block).</think>```json
{
"title": "A Light in the Attic",
"price": "£51.77",
"availability": "In stock (22 available)",
"rating": "5",
"upc": "a897fe39b1053632",
"product_type": "Books",
"number_of_reviews": "0"
}
```"#;

        let result = best_effort_parse_json_object(thinking_output);
        assert!(result.is_ok(), "Should successfully parse thinking output");

        let json = result.unwrap();
        assert_eq!(json["title"], "A Light in the Attic");
        assert_eq!(json["price"], "£51.77");
        assert_eq!(json["upc"], "a897fe39b1053632");
        assert_eq!(json["product_type"], "Books");
    }

    /// Test extraction of code blocks - should get the LAST one.
    #[cfg(feature = "serde")]
    #[test]
    fn test_extract_last_code_block() {
        let input = r#"Here's my first attempt:
```json
{"version": 1}
```
Actually, let me fix that:
```json
{"version": 2, "fixed": true}
```
"#;

        let block = extract_last_code_block(input);
        assert!(block.is_some());
        let block = block.unwrap();
        assert!(block.contains("version"));
        assert!(block.contains("fixed"));

        // Parse it and verify it's version 2
        let json: serde_json::Value = serde_json::from_str(block).unwrap();
        assert_eq!(json["version"], 2);
        assert_eq!(json["fixed"], true);
    }

    /// Test proper brace matching for nested JSON.
    #[cfg(feature = "serde")]
    #[test]
    fn test_extract_nested_json() {
        let input = r#"Some text before {"outer": {"inner": {"deep": 1}}} more text"#;

        let extracted = extract_last_json_object(input);
        assert!(extracted.is_some());

        let json: serde_json::Value = serde_json::from_str(extracted.unwrap()).unwrap();
        assert_eq!(json["outer"]["inner"]["deep"], 1);
    }

    /// Test that we don't get tripped up by braces in strings.
    #[cfg(feature = "serde")]
    #[test]
    fn test_braces_in_strings() {
        let input = r#"{"message": "Use {curly} braces", "count": 1}"#;

        let extracted = extract_last_json_object(input);
        assert!(extracted.is_some());

        let json: serde_json::Value = serde_json::from_str(extracted.unwrap()).unwrap();
        assert_eq!(json["message"], "Use {curly} braces");
        assert_eq!(json["count"], 1);
    }

    /// Test array extraction.
    #[cfg(feature = "serde")]
    #[test]
    fn test_extract_array() {
        let input = r#"The items are: [{"id": 1}, {"id": 2}] end"#;

        let extracted = extract_last_json_array(input);
        assert!(extracted.is_some());

        let json: serde_json::Value = serde_json::from_str(extracted.unwrap()).unwrap();
        assert!(json.is_array());
        assert_eq!(json[0]["id"], 1);
        assert_eq!(json[1]["id"], 2);
    }
}
