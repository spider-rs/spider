//! Runtime configuration for remote multimodal automation.
//!
//! This module contains [`RemoteMultimodalConfigs`], the high-level configuration
//! bundle that wires together all engine settings, concurrency control, and
//! feature-gated capabilities (skills, S3, Chrome AI).
//!
//! Pure data types (e.g. [`RemoteMultimodalConfig`], [`ModelEndpoint`],
//! [`VisionRouteMode`]) live in the [`spider_agent_types`] crate and are
//! re-exported at the parent module level for backward compatibility.

use std::sync::{Arc, OnceLock};

use super::{
    ExtractionSchema, ModelEndpoint, PromptUrlGate, RemoteMultimodalConfig, VisionRouteMode,
};

/// Default value for `chrome_ai_max_user_chars`.
fn default_chrome_ai_max_user_chars() -> usize {
    6000
}

/// Top-level configuration bundle for remote multimodal automation.
///
/// This struct combines all the settings needed to drive the
/// [`RemoteMultimodalEngine`](super::RemoteMultimodalEngine):
///
/// - **API connection** (`api_url`, `api_key`, `model_name`)
/// - **Prompt configuration** (`system_prompt`, `system_prompt_extra`, `user_message_extra`)
/// - **Runtime configuration** ([`RemoteMultimodalConfig`])
/// - **URL gating** ([`PromptUrlGate`])
/// - **Dual-model routing** (`vision_model`, `text_model`, `vision_route_mode`)
/// - **Chrome AI** (`use_chrome_ai`, `chrome_ai_max_user_chars`)
/// - **Skills** (feature-gated `skill_registry`, `s3_skill_source`)
/// - **Concurrency** (`concurrency_limit`, lazy semaphore)
/// - **Relevance tracking** (`relevance_credits`, `url_prefilter_cache`)
///
/// # Example
/// ```rust,ignore
/// use spider_agent::automation::RemoteMultimodalConfigs;
///
/// let mm = RemoteMultimodalConfigs::new(
///     "https://openrouter.ai/api/v1/chat/completions",
///     "qwen/qwen-2-vl-72b-instruct",
/// )
/// .with_api_key("sk-or-...")
/// .with_concurrency_limit(5);
/// ```
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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
    pub prompt_url_gate: Option<PromptUrlGate>,
    /// Optional concurrency limit for remote inference calls.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub concurrency_limit: Option<usize>,
    /// Optional vision model endpoint for dual-model routing.
    /// When set alongside `text_model`, the engine routes per-round
    /// based on [`VisionRouteMode`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vision_model: Option<ModelEndpoint>,
    /// Optional text-only model endpoint for dual-model routing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_model: Option<ModelEndpoint>,
    /// Routing mode controlling when vision vs text model is used.
    #[serde(default)]
    pub vision_route_mode: VisionRouteMode,
    /// Optional pool of model endpoints for per-round complexity routing.
    ///
    /// When 3+ models are provided, the engine automatically routes simple
    /// rounds to cheap/fast models and complex rounds to powerful/expensive
    /// models — with zero extra LLM calls for the routing decision.
    ///
    /// Pools with 0-2 models are ignored (existing single/dual routing applies).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub model_pool: Vec<ModelEndpoint>,
    /// Use Chrome's built-in LanguageModel API (Gemini Nano) for inference.
    ///
    /// When `true`, the automation loop evaluates JavaScript on the page via
    /// `page.evaluate()` calling `LanguageModel.create()` + `session.prompt()`
    /// instead of making HTTP API calls. This enables running the agent
    /// without any external API key.
    ///
    /// When left `false` (default), Chrome AI is still used as a **last-resort
    /// fallback** if both `api_url` and `api_key` are empty.
    ///
    /// Requires Chrome with built-in AI enabled:
    /// - `chrome://flags/#optimization-guide-on-device-model` → Enabled
    /// - `chrome://flags/#prompt-api-for-gemini-nano` → Enabled
    #[serde(default)]
    pub use_chrome_ai: bool,
    /// Maximum user-prompt characters for Chrome AI inference.
    ///
    /// Gemini Nano has limited context compared to cloud models. This budget
    /// controls the max length of the user message (HTML context, URL, title,
    /// task instructions). When the user prompt exceeds this limit, the HTML
    /// context section is truncated while preserving task instructions and memory.
    ///
    /// Default: 6000 chars. Only used when Chrome AI is the active inference path.
    #[serde(default = "default_chrome_ai_max_user_chars")]
    pub chrome_ai_max_user_chars: usize,
    /// Optional skill registry for dynamic context injection.
    /// When set, matching skills are automatically injected into the system prompt
    /// based on current page state (URL, title, HTML) each round.
    #[cfg(feature = "skills")]
    #[serde(skip)]
    pub skill_registry: Option<super::skills::SkillRegistry>,
    /// S3 source for loading skills at startup.
    /// When set, skills are fetched from the S3 bucket and merged with any
    /// built-in or manually registered skills.
    #[cfg(feature = "skills_s3")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub s3_skill_source: Option<super::skills::S3SkillSource>,
    /// Semaphore control for concurrency limiting.
    #[serde(skip, default = "RemoteMultimodalConfigs::default_semaphore")]
    pub semaphore: OnceLock<Arc<tokio::sync::Semaphore>>,
    /// Counter for pages deemed irrelevant — each unit = one budget credit to restore.
    #[serde(skip)]
    pub relevance_credits: Arc<std::sync::atomic::AtomicU32>,
    /// Cache of URL path → relevant classification to avoid re-classifying.
    #[serde(skip)]
    pub url_prefilter_cache: Arc<dashmap::DashMap<String, bool>>,
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
            && self.vision_model == other.vision_model
            && self.text_model == other.text_model
            && self.vision_route_mode == other.vision_route_mode
            && self.model_pool == other.model_pool
            && self.use_chrome_ai == other.use_chrome_ai
            && self.chrome_ai_max_user_chars == other.chrome_ai_max_user_chars
        // NOTE: intentionally ignoring `semaphore` and `skill_registry`
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
            vision_model: None,
            text_model: None,
            vision_route_mode: VisionRouteMode::default(),
            model_pool: Vec::new(),
            use_chrome_ai: false,
            chrome_ai_max_user_chars: default_chrome_ai_max_user_chars(),
            #[cfg(feature = "skills")]
            skill_registry: Some(super::skills::builtin_web_challenges()),
            #[cfg(feature = "skills_s3")]
            s3_skill_source: None,
            semaphore: Self::default_semaphore(),
            relevance_credits: Arc::new(std::sync::atomic::AtomicU32::new(0)),
            url_prefilter_cache: Arc::new(dashmap::DashMap::new()),
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
    pub fn with_prompt_url_gate(mut self, gate: PromptUrlGate) -> Self {
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
    pub fn with_extraction_schema(mut self, schema: ExtractionSchema) -> Self {
        self.cfg.extraction_schema = Some(schema);
        self
    }

    /// Check if the configured model supports vision/multimodal input.
    ///
    /// Uses the `supports_vision` function to detect based on model name.
    pub fn model_supports_vision(&self) -> bool {
        super::supports_vision(&self.model_name)
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

    // ── dual-model routing ──────────────────────────────────────────

    /// Set the vision model endpoint for dual-model routing.
    pub fn with_vision_model(mut self, endpoint: ModelEndpoint) -> Self {
        self.vision_model = Some(endpoint);
        self
    }

    /// Set the text model endpoint for dual-model routing.
    pub fn with_text_model(mut self, endpoint: ModelEndpoint) -> Self {
        self.text_model = Some(endpoint);
        self
    }

    /// Set the vision routing mode.
    pub fn with_vision_route_mode(mut self, mode: VisionRouteMode) -> Self {
        self.vision_route_mode = mode;
        self
    }

    /// Convenience: set both vision and text model endpoints at once.
    pub fn with_dual_models(mut self, vision: ModelEndpoint, text: ModelEndpoint) -> Self {
        self.vision_model = Some(vision);
        self.text_model = Some(text);
        self
    }

    /// Set a pool of model endpoints for per-round complexity routing.
    ///
    /// When 3+ models are provided, the engine uses [`auto_policy`] to
    /// assign models to cost tiers, then picks cheap models for simple
    /// rounds and expensive models for complex rounds.
    pub fn with_model_pool(mut self, pool: Vec<ModelEndpoint>) -> Self {
        self.model_pool = pool;
        self
    }

    // ── S3 skill source ─────────────────────────────────────────────

    /// Set an S3 source for loading skills at startup.
    #[cfg(feature = "skills_s3")]
    pub fn with_s3_skill_source(mut self, source: super::skills::S3SkillSource) -> Self {
        self.s3_skill_source = Some(source);
        self
    }

    /// Enable relevance gating with optional custom criteria prompt.
    pub fn with_relevance_gate(mut self, prompt: Option<String>) -> Self {
        self.cfg.relevance_gate = true;
        self.cfg.relevance_prompt = prompt;
        self
    }

    /// Enable URL-level pre-filtering before HTTP fetch.
    /// Requires `relevance_gate` to also be enabled.
    pub fn with_url_prefilter(mut self, batch_size: Option<usize>) -> Self {
        self.cfg.url_prefilter = true;
        if let Some(bs) = batch_size {
            self.cfg.url_prefilter_batch_size = bs;
        }
        self
    }

    /// Enable Chrome built-in AI (LanguageModel / Gemini Nano) for inference.
    ///
    /// When enabled, the engine uses `page.evaluate()` to call Chrome's
    /// `LanguageModel.create()` + `session.prompt()` instead of HTTP API calls.
    /// No API key is required.
    ///
    /// Even when not explicitly enabled, Chrome AI is used as a last-resort
    /// fallback if both `api_url` and `api_key` are empty.
    pub fn with_chrome_ai(mut self, enabled: bool) -> Self {
        self.use_chrome_ai = enabled;
        self
    }

    /// Set the maximum user-prompt character budget for Chrome AI inference.
    pub fn with_chrome_ai_max_user_chars(mut self, chars: usize) -> Self {
        self.chrome_ai_max_user_chars = chars;
        self
    }

    /// Whether Chrome AI should be used for inference in this configuration.
    ///
    /// Returns `true` when explicitly enabled OR when no API endpoint is
    /// configured (last-resort fallback).
    pub fn should_use_chrome_ai(&self) -> bool {
        self.use_chrome_ai || (self.api_url.is_empty() && self.api_key.is_none())
    }

    /// Whether dual-model routing is active
    /// (at least one of `vision_model` / `text_model` is configured).
    pub fn has_dual_model_routing(&self) -> bool {
        self.vision_model.is_some() || self.text_model.is_some()
    }

    /// Resolve the (api_url, model_name, api_key) triple for the current round.
    ///
    /// * `use_vision == true`  → prefer `vision_model`, fall back to primary.
    /// * `use_vision == false` → prefer `text_model`,   fall back to primary.
    ///
    /// Fields left as `None` on the chosen [`ModelEndpoint`] inherit from
    /// the parent (`self.api_url` / `self.api_key`).
    pub fn resolve_model_for_round(&self, use_vision: bool) -> (&str, &str, Option<&str>) {
        let endpoint = if use_vision {
            self.vision_model.as_ref()
        } else {
            self.text_model.as_ref()
        };

        match endpoint {
            Some(ep) => {
                let url = ep.api_url.as_deref().unwrap_or(&self.api_url);
                let key = ep.api_key.as_deref().or(self.api_key.as_deref());
                (url, &ep.model_name, key)
            }
            None => (&self.api_url, &self.model_name, self.api_key.as_deref()),
        }
    }

    /// Decide whether to use vision this round, based on the configured
    /// [`VisionRouteMode`] and current loop state.
    ///
    /// `force_vision` is an explicit per-round override (e.g. from `request_vision`).
    pub fn should_use_vision_this_round(
        &self,
        round_idx: usize,
        stagnated: bool,
        action_stuck_rounds: usize,
        force_vision: bool,
    ) -> bool {
        if !self.has_dual_model_routing() {
            return true; // no routing → always include screenshot (current behaviour)
        }
        if force_vision {
            return true;
        }
        match self.vision_route_mode {
            VisionRouteMode::AlwaysPrimary => true,
            VisionRouteMode::TextFirst => round_idx == 0 || stagnated || action_stuck_rounds >= 3,
            VisionRouteMode::VisionFirst => round_idx < 2 || stagnated || action_stuck_rounds >= 3,
            VisionRouteMode::AgentDriven => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_remote_multimodal_configs_new() {
        let configs = RemoteMultimodalConfigs::new(
            "http://localhost:11434/v1/chat/completions",
            "qwen2.5-vl",
        );

        assert_eq!(
            configs.api_url,
            "http://localhost:11434/v1/chat/completions"
        );
        assert_eq!(configs.model_name, "qwen2.5-vl");
        assert!(configs.api_key.is_none());
        assert!(configs.system_prompt.is_none());
    }

    #[test]
    fn test_remote_multimodal_configs_builder() {
        let configs =
            RemoteMultimodalConfigs::new("https://api.openai.com/v1/chat/completions", "gpt-4o")
                .with_api_key("sk-test")
                .with_system_prompt("You are a helpful assistant.")
                .with_concurrency_limit(5)
                .with_screenshot(true);

        assert_eq!(configs.api_key, Some("sk-test".to_string()));
        assert_eq!(
            configs.system_prompt,
            Some("You are a helpful assistant.".to_string())
        );
        assert_eq!(configs.concurrency_limit, Some(5));
        assert!(configs.cfg.screenshot);
    }

    #[test]
    fn test_remote_multimodal_configs_vision_detection() {
        // Vision model
        let cfg =
            RemoteMultimodalConfigs::new("https://api.openai.com/v1/chat/completions", "gpt-4o");
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
        let mut cfg =
            RemoteMultimodalConfigs::new("https://api.openai.com/v1/chat/completions", "gpt-4o");
        cfg.cfg.include_screenshot = Some(false);
        assert!(!cfg.should_include_screenshot());
    }

    #[test]
    fn test_filter_screenshot() {
        let screenshot = "base64data...";

        // Vision model - screenshot passes through
        let cfg =
            RemoteMultimodalConfigs::new("https://api.openai.com/v1/chat/completions", "gpt-4o");
        assert_eq!(cfg.filter_screenshot(Some(screenshot)), Some(screenshot));

        // Non-vision model - screenshot filtered out
        let cfg = RemoteMultimodalConfigs::new(
            "https://api.openai.com/v1/chat/completions",
            "gpt-3.5-turbo",
        );
        assert_eq!(cfg.filter_screenshot(Some(screenshot)), None);

        // No screenshot provided
        let cfg =
            RemoteMultimodalConfigs::new("https://api.openai.com/v1/chat/completions", "gpt-4o");
        assert_eq!(cfg.filter_screenshot(None), None);
    }

    #[test]
    fn test_has_dual_model_routing() {
        // No routing by default
        let cfg = RemoteMultimodalConfigs::new("https://api.example.com", "gpt-4o");
        assert!(!cfg.has_dual_model_routing());

        // Vision only
        let cfg = RemoteMultimodalConfigs::new("https://api.example.com", "gpt-4o")
            .with_vision_model(ModelEndpoint::new("gpt-4o"));
        assert!(cfg.has_dual_model_routing());

        // Text only
        let cfg = RemoteMultimodalConfigs::new("https://api.example.com", "gpt-4o")
            .with_text_model(ModelEndpoint::new("gpt-4o-mini"));
        assert!(cfg.has_dual_model_routing());

        // Both
        let cfg = RemoteMultimodalConfigs::new("https://api.example.com", "gpt-4o")
            .with_dual_models(
                ModelEndpoint::new("gpt-4o"),
                ModelEndpoint::new("gpt-4o-mini"),
            );
        assert!(cfg.has_dual_model_routing());
    }

    #[test]
    fn test_resolve_model_for_round_no_routing() {
        let cfg = RemoteMultimodalConfigs::new("https://api.example.com", "gpt-4o")
            .with_api_key("sk-parent");

        // Without dual routing, always returns primary
        let (url, model, key) = cfg.resolve_model_for_round(true);
        assert_eq!(url, "https://api.example.com");
        assert_eq!(model, "gpt-4o");
        assert_eq!(key, Some("sk-parent"));

        let (url, model, key) = cfg.resolve_model_for_round(false);
        assert_eq!(url, "https://api.example.com");
        assert_eq!(model, "gpt-4o");
        assert_eq!(key, Some("sk-parent"));
    }

    #[test]
    fn test_resolve_model_for_round_dual() {
        let cfg = RemoteMultimodalConfigs::new("https://api.example.com", "gpt-4o")
            .with_api_key("sk-parent")
            .with_dual_models(
                ModelEndpoint::new("gpt-4o"),
                ModelEndpoint::new("gpt-4o-mini"),
            );

        // Vision round → vision model, inherits parent URL/key
        let (url, model, key) = cfg.resolve_model_for_round(true);
        assert_eq!(model, "gpt-4o");
        assert_eq!(url, "https://api.example.com");
        assert_eq!(key, Some("sk-parent"));

        // Text round → text model, inherits parent URL/key
        let (url, model, key) = cfg.resolve_model_for_round(false);
        assert_eq!(model, "gpt-4o-mini");
        assert_eq!(url, "https://api.example.com");
        assert_eq!(key, Some("sk-parent"));
    }

    #[test]
    fn test_resolve_model_cross_provider() {
        // Vision on OpenAI, text on Groq — different URLs and keys
        let cfg =
            RemoteMultimodalConfigs::new("https://api.openai.com/v1/chat/completions", "gpt-4o")
                .with_api_key("sk-openai")
                .with_vision_model(ModelEndpoint::new("gpt-4o"))
                .with_text_model(
                    ModelEndpoint::new("llama-3.3-70b-versatile")
                        .with_api_url("https://api.groq.com/openai/v1/chat/completions")
                        .with_api_key("gsk-groq"),
                );

        // Vision → uses OpenAI (inherits parent)
        let (url, model, key) = cfg.resolve_model_for_round(true);
        assert_eq!(url, "https://api.openai.com/v1/chat/completions");
        assert_eq!(model, "gpt-4o");
        assert_eq!(key, Some("sk-openai"));

        // Text → uses Groq (endpoint overrides)
        let (url, model, key) = cfg.resolve_model_for_round(false);
        assert_eq!(url, "https://api.groq.com/openai/v1/chat/completions");
        assert_eq!(model, "llama-3.3-70b-versatile");
        assert_eq!(key, Some("gsk-groq"));
    }

    #[test]
    fn test_vision_route_mode_always_primary() {
        let cfg = RemoteMultimodalConfigs::new("https://api.example.com", "gpt-4o")
            .with_dual_models(
                ModelEndpoint::new("gpt-4o"),
                ModelEndpoint::new("gpt-4o-mini"),
            )
            .with_vision_route_mode(VisionRouteMode::AlwaysPrimary);

        // AlwaysPrimary → always vision (true) regardless of round/state
        assert!(cfg.should_use_vision_this_round(0, false, 0, false));
        assert!(cfg.should_use_vision_this_round(5, false, 0, false));
        assert!(cfg.should_use_vision_this_round(10, false, 0, false));
    }

    #[test]
    fn test_vision_route_mode_text_first() {
        let cfg = RemoteMultimodalConfigs::new("https://api.example.com", "gpt-4o")
            .with_dual_models(
                ModelEndpoint::new("gpt-4o"),
                ModelEndpoint::new("gpt-4o-mini"),
            )
            .with_vision_route_mode(VisionRouteMode::TextFirst);

        // Round 0 → vision
        assert!(cfg.should_use_vision_this_round(0, false, 0, false));
        // Round 1+ (no stagnation) → text
        assert!(!cfg.should_use_vision_this_round(1, false, 0, false));
        assert!(!cfg.should_use_vision_this_round(5, false, 0, false));
        // Stagnation → upgrade to vision
        assert!(cfg.should_use_vision_this_round(3, true, 0, false));
        // Stuck ≥ 3 → upgrade to vision
        assert!(cfg.should_use_vision_this_round(5, false, 3, false));
        // Force vision override
        assert!(cfg.should_use_vision_this_round(5, false, 0, true));
    }

    #[test]
    fn test_vision_route_mode_vision_first() {
        let cfg = RemoteMultimodalConfigs::new("https://api.example.com", "gpt-4o")
            .with_dual_models(
                ModelEndpoint::new("gpt-4o"),
                ModelEndpoint::new("gpt-4o-mini"),
            )
            .with_vision_route_mode(VisionRouteMode::VisionFirst);

        // Rounds 0-1 → vision
        assert!(cfg.should_use_vision_this_round(0, false, 0, false));
        assert!(cfg.should_use_vision_this_round(1, false, 0, false));
        // Round 2+ → text
        assert!(!cfg.should_use_vision_this_round(2, false, 0, false));
        assert!(!cfg.should_use_vision_this_round(5, false, 0, false));
        // Stagnation → upgrade to vision
        assert!(cfg.should_use_vision_this_round(5, true, 0, false));
        // Stuck ≥ 3 → upgrade to vision
        assert!(cfg.should_use_vision_this_round(5, false, 3, false));
    }

    #[test]
    fn test_no_dual_routing_always_returns_true() {
        // Without dual routing set up, should_use_vision_this_round always returns true
        // (backwards compatible: every round gets a screenshot)
        let cfg = RemoteMultimodalConfigs::new("https://api.example.com", "gpt-4o");
        assert!(!cfg.has_dual_model_routing());
        assert!(cfg.should_use_vision_this_round(0, false, 0, false));
        assert!(cfg.should_use_vision_this_round(5, false, 0, false));
        assert!(cfg.should_use_vision_this_round(99, false, 0, false));
    }

    #[test]
    fn test_with_dual_models_builder() {
        let cfg = RemoteMultimodalConfigs::new("https://api.example.com", "primary")
            .with_dual_models(
                ModelEndpoint::new("vision-model"),
                ModelEndpoint::new("text-model"),
            )
            .with_vision_route_mode(VisionRouteMode::TextFirst);

        assert!(cfg.has_dual_model_routing());
        assert_eq!(
            cfg.vision_model.as_ref().unwrap().model_name,
            "vision-model"
        );
        assert_eq!(cfg.text_model.as_ref().unwrap().model_name, "text-model");
        assert_eq!(cfg.vision_route_mode, VisionRouteMode::TextFirst);
    }

    #[test]
    fn test_configs_serde_with_dual_models() {
        let cfg = RemoteMultimodalConfigs::new("https://api.example.com", "gpt-4o")
            .with_api_key("sk-test")
            .with_dual_models(
                ModelEndpoint::new("gpt-4o"),
                ModelEndpoint::new("gpt-4o-mini")
                    .with_api_url("https://other.api.com")
                    .with_api_key("sk-other"),
            )
            .with_vision_route_mode(VisionRouteMode::TextFirst);

        let json = serde_json::to_string(&cfg).unwrap();
        let deserialized: RemoteMultimodalConfigs = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.model_name, "gpt-4o");
        assert!(deserialized.has_dual_model_routing());
        assert_eq!(
            deserialized.vision_model.as_ref().unwrap().model_name,
            "gpt-4o"
        );
        assert_eq!(
            deserialized.text_model.as_ref().unwrap().model_name,
            "gpt-4o-mini"
        );
        assert_eq!(
            deserialized.text_model.as_ref().unwrap().api_url.as_deref(),
            Some("https://other.api.com")
        );
        assert_eq!(deserialized.vision_route_mode, VisionRouteMode::TextFirst);
    }

    #[cfg(feature = "skills")]
    #[test]
    fn test_default_configs_auto_load_builtin_skills() {
        let cfg = RemoteMultimodalConfigs::default();
        let registry = cfg
            .skill_registry
            .as_ref()
            .expect("default config should auto-load built-in skills");
        assert!(
            registry.get("image-grid-selection").is_some(),
            "expected image-grid-selection built-in skill"
        );
        assert!(
            registry.get("tic-tac-toe").is_some(),
            "expected tic-tac-toe built-in skill"
        );
    }

    #[cfg(feature = "skills")]
    #[test]
    fn test_new_configs_auto_load_builtin_skills() {
        let cfg = RemoteMultimodalConfigs::new("https://api.example.com", "model");
        let registry = cfg
            .skill_registry
            .as_ref()
            .expect("new config should auto-load built-in skills");
        assert!(
            registry.get("word-search").is_some(),
            "expected word-search built-in skill"
        );
    }

    // ── Phase 4: Config integration with router ─────────────────────────

    #[test]
    fn test_selector_to_dual_model_config() {
        use super::super::router::{ModelRequirements, ModelSelector, SelectionStrategy};

        let mut selector = ModelSelector::new(&["gpt-4o", "gpt-4o-mini", "gpt-3.5-turbo"]);

        // Pick best vision model
        let vision_reqs = ModelRequirements::default().with_vision();
        let vision_pick = selector.select(&vision_reqs).expect("should find a vision model");

        // Pick cheapest text model
        selector.set_strategy(SelectionStrategy::CheapestFirst);
        let text_reqs = ModelRequirements::default();
        let text_pick = selector.select(&text_reqs).expect("should find a text model");

        // Build config with dual models from selector picks
        let cfg = RemoteMultimodalConfigs::new("https://api.example.com", &vision_pick.name)
            .with_dual_models(
                ModelEndpoint::new(&vision_pick.name),
                ModelEndpoint::new(&text_pick.name),
            )
            .with_vision_route_mode(VisionRouteMode::TextFirst);

        // Verify resolve_model_for_round returns selector's picks
        let (_, model, _) = cfg.resolve_model_for_round(true);
        assert_eq!(model, vision_pick.name, "vision round should use vision pick");

        let (_, model, _) = cfg.resolve_model_for_round(false);
        assert_eq!(model, text_pick.name, "text round should use text pick");
    }

    #[test]
    fn test_auto_policy_to_configs_round_trip() {
        use super::super::router::auto_policy;

        let policy = auto_policy(&["gpt-4o", "gpt-4o-mini", "gpt-3.5-turbo"]);

        // Build config using policy's tier assignments
        let cfg = RemoteMultimodalConfigs::new("https://api.example.com", &policy.large)
            .with_dual_models(
                ModelEndpoint::new(&policy.large),
                ModelEndpoint::new(&policy.small),
            );

        // Serde round-trip
        let json = serde_json::to_string(&cfg).unwrap();
        let deserialized: RemoteMultimodalConfigs = serde_json::from_str(&json).unwrap();

        // Verify resolution survives round-trip
        let (_, vision_model, _) = deserialized.resolve_model_for_round(true);
        let (_, text_model, _) = deserialized.resolve_model_for_round(false);
        assert_eq!(vision_model, policy.large);
        assert_eq!(text_model, policy.small);
    }

    #[test]
    fn test_vision_routing_with_real_capabilities() {
        // Use real model names with known vision capabilities
        let cfg = RemoteMultimodalConfigs::new("https://api.example.com", "gpt-4o")
            .with_dual_models(
                ModelEndpoint::new("gpt-4o"),        // vision-capable
                ModelEndpoint::new("gpt-3.5-turbo"), // text-only
            )
            .with_vision_route_mode(VisionRouteMode::TextFirst);

        // Round 0 (TextFirst) → vision
        assert!(cfg.should_use_vision_this_round(0, false, 0, false));
        let (_, model, _) = cfg.resolve_model_for_round(true);
        assert_eq!(model, "gpt-4o");
        assert!(
            llm_models_spider::supports_vision(model),
            "vision-round model should support vision"
        );

        // Round 3 (no stagnation) → text
        assert!(!cfg.should_use_vision_this_round(3, false, 0, false));
        let (_, model, _) = cfg.resolve_model_for_round(false);
        assert_eq!(model, "gpt-3.5-turbo");
        assert!(
            !llm_models_spider::supports_vision(model),
            "text-round model should NOT support vision"
        );
    }

    #[test]
    fn test_single_model_config_e2e() {
        use super::super::router::auto_policy;

        // User has exactly one model — the most common real-world case
        let policy = auto_policy(&["gpt-4o"]);

        // Build config from single-model policy (no dual routing)
        let cfg = RemoteMultimodalConfigs::new("https://api.openai.com/v1/chat/completions", &policy.large)
            .with_api_key("sk-test");

        // No dual routing active
        assert!(!cfg.has_dual_model_routing());

        // Both vision and text rounds resolve to the same single model
        let (url, model, key) = cfg.resolve_model_for_round(true);
        assert_eq!(model, "gpt-4o");
        assert_eq!(key, Some("sk-test"));

        let (url2, model2, key2) = cfg.resolve_model_for_round(false);
        assert_eq!(url, url2, "single model: same URL for both modes");
        assert_eq!(model, model2, "single model: same model for both modes");
        assert_eq!(key, key2, "single model: same key for both modes");

        // Serde round-trip preserves single-model config
        let json = serde_json::to_string(&cfg).unwrap();
        let deserialized: RemoteMultimodalConfigs = serde_json::from_str(&json).unwrap();
        assert!(!deserialized.has_dual_model_routing());
        let (_, m, _) = deserialized.resolve_model_for_round(true);
        assert_eq!(m, "gpt-4o");
    }

    #[test]
    fn test_model_resolution_consistency() {
        let cfg = RemoteMultimodalConfigs::new("https://api.example.com", "gpt-4o")
            .with_api_key("sk-test")
            .with_dual_models(
                ModelEndpoint::new("gpt-4o"),
                ModelEndpoint::new("gpt-4o-mini")
                    .with_api_url("https://other.api.com")
                    .with_api_key("sk-other"),
            );

        // Call many times — must always return the same result
        for _ in 0..100 {
            let (url, model, key) = cfg.resolve_model_for_round(true);
            assert_eq!(url, "https://api.example.com");
            assert_eq!(model, "gpt-4o");
            assert_eq!(key, Some("sk-test"));

            let (url, model, key) = cfg.resolve_model_for_round(false);
            assert_eq!(url, "https://other.api.com");
            assert_eq!(model, "gpt-4o-mini");
            assert_eq!(key, Some("sk-other"));
        }
    }

    // ── model_pool tests ──────────────────────────────────────────────

    #[test]
    fn test_model_pool_default_empty() {
        let cfg = RemoteMultimodalConfigs::default();
        assert!(cfg.model_pool.is_empty());
    }

    #[test]
    fn test_model_pool_builder() {
        let cfg = RemoteMultimodalConfigs::new("https://api.example.com", "gpt-4o")
            .with_model_pool(vec![
                ModelEndpoint::new("gpt-4o"),
                ModelEndpoint::new("gpt-4o-mini"),
                ModelEndpoint::new("deepseek-chat")
                    .with_api_url("https://api.deepseek.com/v1/chat/completions")
                    .with_api_key("sk-ds"),
            ]);
        assert_eq!(cfg.model_pool.len(), 3);
        assert_eq!(cfg.model_pool[2].model_name, "deepseek-chat");
        assert_eq!(
            cfg.model_pool[2].api_url.as_deref(),
            Some("https://api.deepseek.com/v1/chat/completions")
        );
    }

    #[test]
    fn test_model_pool_serde_round_trip() {
        let cfg = RemoteMultimodalConfigs::new("https://api.example.com", "gpt-4o")
            .with_model_pool(vec![
                ModelEndpoint::new("gpt-4o"),
                ModelEndpoint::new("gpt-4o-mini"),
                ModelEndpoint::new("deepseek-chat"),
            ]);

        let json = serde_json::to_string(&cfg).unwrap();
        assert!(json.contains("model_pool"));
        let deserialized: RemoteMultimodalConfigs = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.model_pool.len(), 3);
        assert_eq!(deserialized.model_pool[0].model_name, "gpt-4o");
        assert_eq!(deserialized.model_pool[1].model_name, "gpt-4o-mini");
        assert_eq!(deserialized.model_pool[2].model_name, "deepseek-chat");
    }

    #[test]
    fn test_model_pool_empty_omitted_from_json() {
        let cfg = RemoteMultimodalConfigs::new("https://api.example.com", "gpt-4o");
        let json = serde_json::to_string(&cfg).unwrap();
        assert!(
            !json.contains("model_pool"),
            "empty model_pool should be omitted from JSON"
        );
    }

    #[test]
    fn test_model_pool_equality() {
        let a = RemoteMultimodalConfigs::new("https://api.example.com", "gpt-4o")
            .with_model_pool(vec![ModelEndpoint::new("gpt-4o")]);
        let b = RemoteMultimodalConfigs::new("https://api.example.com", "gpt-4o")
            .with_model_pool(vec![ModelEndpoint::new("gpt-4o")]);
        let c = RemoteMultimodalConfigs::new("https://api.example.com", "gpt-4o")
            .with_model_pool(vec![ModelEndpoint::new("gpt-4o-mini")]);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }
}
