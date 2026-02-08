//! Remote multimodal engine for LLM-driven automation.
//!
//! Provides the core engine for making API calls to OpenAI-compatible endpoints
//! and extracting structured data from HTML content.

use reqwest::Client;
use serde::Serialize;
use std::sync::Arc;
use tokio::sync::Semaphore;

use super::{
    best_effort_parse_json_object, extract_assistant_content, extract_usage, reasoning_payload,
    truncate_utf8_tail, AutomationResult, AutomationUsage, ContentAnalysis, EngineError,
    EngineResult, ExtractionSchema, PromptUrlGate, RemoteMultimodalConfig, DEFAULT_SYSTEM_PROMPT,
    EXTRACTION_ONLY_SYSTEM_PROMPT,
};

/// Lazy-initialized HTTP client for automation.
static CLIENT: std::sync::LazyLock<Client> = std::sync::LazyLock::new(Client::new);

/// Remote multimodal engine for LLM-driven web automation.
///
/// This engine makes API calls to OpenAI-compatible endpoints (like OpenRouter)
/// to extract structured data from HTML content. It supports:
/// - HTML-only extraction (no browser required)
/// - HTML + screenshot extraction (multimodal)
/// - Configurable prompts and extraction schemas
/// - Concurrency limiting via semaphore
///
/// # Example
/// ```rust,ignore
/// use spider_agent::automation::{RemoteMultimodalEngine, RemoteMultimodalConfig};
///
/// let engine = RemoteMultimodalEngine::new(
///     "https://openrouter.ai/api/v1/chat/completions",
///     "qwen/qwen-2-vl-72b-instruct",
///     None,
/// ).with_api_key(Some("your-api-key"));
///
/// let result = engine.extract_from_html(
///     "<html><body><h1>Product</h1><p>$19.99</p></body></html>",
///     "https://example.com/product",
///     Some("Product Page"),
/// ).await?;
///
/// println!("Extracted: {:?}", result.extracted);
/// ```
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
    pub semaphore: Option<Arc<Semaphore>>,
    /// Optional vision model endpoint for dual-model routing.
    pub vision_model: Option<super::config::ModelEndpoint>,
    /// Optional text-only model endpoint for dual-model routing.
    pub text_model: Option<super::config::ModelEndpoint>,
    /// Routing mode controlling when vision vs text model is used.
    pub vision_route_mode: super::config::VisionRouteMode,
    /// Optional skill registry for dynamic context injection.
    /// When set, matching skills are automatically injected into the system prompt
    /// based on current page state (URL, title, HTML) each round.
    #[cfg(feature = "skills")]
    pub skill_registry: Option<super::skills::SkillRegistry>,
    /// Optional long-term experience memory for learning from past sessions.
    /// When set, the engine recalls relevant past strategies before automation
    /// and stores successful outcomes after completion.
    #[cfg(feature = "memvid")]
    pub experience_memory:
        Option<std::sync::Arc<tokio::sync::RwLock<super::long_term_memory::ExperienceMemory>>>,
}

impl RemoteMultimodalEngine {
    /// Create a new remote multimodal engine.
    ///
    /// # Arguments
    /// * `api_url` - OpenAI-compatible chat completions endpoint URL
    /// * `model_name` - Model identifier (e.g., "gpt-4o", "qwen/qwen-2-vl-72b-instruct")
    /// * `system_prompt` - Optional custom system prompt (defaults to built-in)
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
            vision_model: None,
            text_model: None,
            vision_route_mode: super::config::VisionRouteMode::default(),
            #[cfg(feature = "skills")]
            skill_registry: None,
            #[cfg(feature = "memvid")]
            experience_memory: None,
        }
    }

    /// Set/clear the API key (Bearer token).
    pub fn with_api_key(mut self, key: Option<&str>) -> Self {
        self.api_key = key.map(|k| k.to_string());
        self
    }

    /// Set the runtime configuration.
    pub fn with_config(mut self, cfg: RemoteMultimodalConfig) -> Self {
        self.cfg = cfg;
        self
    }

    /// Set maximum concurrent LLM requests.
    pub fn with_max_inflight_requests(&mut self, n: usize) -> &mut Self {
        if n > 0 {
            self.semaphore = Some(Arc::new(Semaphore::new(n)));
        } else {
            self.semaphore = None;
        }
        self
    }

    /// Provide a shared semaphore for concurrency control.
    pub fn with_semaphore(&mut self, sem: Option<Arc<Semaphore>>) -> &mut Self {
        self.semaphore = sem;
        self
    }

    /// Set extra system prompt instructions.
    pub fn with_system_prompt_extra(&mut self, extra: Option<&str>) -> &mut Self {
        self.system_prompt_extra = extra.map(|s| s.to_string());
        self
    }

    /// Set extra user message instructions.
    pub fn with_user_message_extra(&mut self, extra: Option<&str>) -> &mut Self {
        self.user_message_extra = extra.map(|s| s.to_string());
        self
    }

    /// Set URL-based gating.
    pub fn with_prompt_url_gate(&mut self, gate: Option<PromptUrlGate>) -> &mut Self {
        self.prompt_url_gate = gate;
        self
    }

    /// Set a skill registry for dynamic context injection.
    ///
    /// When set, matching skills are automatically injected into the system prompt
    /// each round based on the current page state (URL, title, HTML).
    #[cfg(feature = "skills")]
    pub fn with_skill_registry(
        &mut self,
        registry: Option<super::skills::SkillRegistry>,
    ) -> &mut Self {
        self.skill_registry = registry;
        self
    }

    /// Set a long-term experience memory for learning across sessions.
    ///
    /// When set, the engine will recall past successful strategies before
    /// each automation run and store new experiences after successful runs.
    #[cfg(feature = "memvid")]
    pub fn with_experience_memory(
        &mut self,
        memory: Option<
            std::sync::Arc<tokio::sync::RwLock<super::long_term_memory::ExperienceMemory>>,
        >,
    ) -> &mut Self {
        self.experience_memory = memory;
        self
    }

    /// Set the full runtime configuration.
    pub fn with_remote_multimodal_config(&mut self, cfg: RemoteMultimodalConfig) -> &mut Self {
        self.cfg = cfg;
        self
    }

    /// Enable/disable extraction mode.
    pub fn with_extra_ai_data(&mut self, enabled: bool) -> &mut Self {
        self.cfg.extra_ai_data = enabled;
        self
    }

    /// Set the extraction prompt.
    pub fn with_extraction_prompt(&mut self, prompt: Option<&str>) -> &mut Self {
        self.cfg.extraction_prompt = prompt.map(|s| s.to_string());
        self
    }

    /// Enable/disable screenshot in results.
    pub fn with_screenshot(&mut self, enabled: bool) -> &mut Self {
        self.cfg.screenshot = enabled;
        self
    }

    /// Set extraction schema.
    pub fn with_extraction_schema(&mut self, schema: Option<ExtractionSchema>) -> &mut Self {
        self.cfg.extraction_schema = schema;
        self
    }

    /// Get current configuration.
    pub fn config(&self) -> &RemoteMultimodalConfig {
        &self.cfg
    }

    /// Get prompt URL gate.
    pub fn prompt_url_gate(&self) -> Option<&PromptUrlGate> {
        self.prompt_url_gate.as_ref()
    }

    /// Clone with a different configuration.
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
            vision_model: self.vision_model.clone(),
            text_model: self.text_model.clone(),
            vision_route_mode: self.vision_route_mode,
            #[cfg(feature = "skills")]
            skill_registry: self.skill_registry.clone(),
            #[cfg(feature = "memvid")]
            experience_memory: self.experience_memory.clone(),
        }
    }

    /// Acquire LLM permit for concurrency control.
    pub async fn acquire_llm_permit(&self) -> Option<tokio::sync::OwnedSemaphorePermit> {
        match &self.semaphore {
            Some(sem) => Some(sem.clone().acquire_owned().await.ok()?),
            None => None,
        }
    }

    /// Analyze HTML content for extraction decisions.
    pub fn analyze_content(&self, html: &str) -> ContentAnalysis {
        ContentAnalysis::analyze(html)
    }

    /// Quick check if screenshot is likely needed for extraction.
    pub fn needs_screenshot(&self, html: &str) -> bool {
        ContentAnalysis::quick_needs_screenshot(html)
    }

    /// Resolve per-URL runtime settings from prompt URL gate.
    ///
    /// Returns `None` when the URL is blocked by the gate.
    #[allow(clippy::type_complexity)]
    fn resolve_runtime_for_url(
        &self,
        url: &str,
    ) -> Option<(
        RemoteMultimodalConfig,
        Option<String>,
        Option<String>,
        Option<String>,
    )> {
        let mut effective_cfg = self.cfg.clone();
        let mut effective_system_prompt = self.system_prompt.clone();
        let mut effective_system_prompt_extra = self.system_prompt_extra.clone();
        let mut effective_user_message_extra = self.user_message_extra.clone();

        if let Some(gate) = &self.prompt_url_gate {
            let gate_match = gate.match_url(url)?;
            if let Some(override_cfg) = gate_match {
                let defaults = super::AutomationConfig::default();

                if override_cfg.max_steps != defaults.max_steps {
                    effective_cfg.max_rounds = override_cfg.max_steps.max(1);
                }
                if override_cfg.max_retries != defaults.max_retries {
                    effective_cfg.retry.max_attempts = override_cfg.max_retries.max(1);
                }
                if override_cfg.capture_screenshots != defaults.capture_screenshots {
                    effective_cfg.screenshot = override_cfg.capture_screenshots;
                }
                if override_cfg.capture_profile != defaults.capture_profile {
                    effective_cfg.capture_profiles = vec![override_cfg.capture_profile.clone()];
                }
                if override_cfg.extract_on_success || override_cfg.extraction_prompt.is_some() {
                    effective_cfg.extra_ai_data = true;
                }
                if let Some(extraction_prompt) = &override_cfg.extraction_prompt {
                    if !extraction_prompt.trim().is_empty() {
                        effective_cfg.extraction_prompt = Some(extraction_prompt.clone());
                    }
                }

                if let Some(system_prompt) = &override_cfg.system_prompt {
                    if !system_prompt.trim().is_empty() {
                        effective_system_prompt = Some(system_prompt.clone());
                    }
                }
                if let Some(system_prompt_extra) = &override_cfg.system_prompt_extra {
                    if !system_prompt_extra.trim().is_empty() {
                        effective_system_prompt_extra = Some(system_prompt_extra.clone());
                    }
                }
                if let Some(user_message_extra) = &override_cfg.user_message_extra {
                    if !user_message_extra.trim().is_empty() {
                        effective_user_message_extra = Some(user_message_extra.clone());
                    }
                }
            }
        }

        Some((
            effective_cfg,
            effective_system_prompt,
            effective_system_prompt_extra,
            effective_user_message_extra,
        ))
    }

    /// Compile the system prompt with configuration.
    /// Uses `EXTRACTION_ONLY_SYSTEM_PROMPT` for single-round extraction mode,
    /// otherwise `DEFAULT_SYSTEM_PROMPT` is always the base.
    pub fn system_prompt_compiled(&self, effective_cfg: &RemoteMultimodalConfig) -> String {
        let mut s = if effective_cfg.is_extraction_only() {
            EXTRACTION_ONLY_SYSTEM_PROMPT.to_string()
        } else {
            DEFAULT_SYSTEM_PROMPT.to_string()
        };

        // Always keep the default system prompt first, then append configured guidance.
        if let Some(base) = &self.system_prompt {
            if !base.trim().is_empty() {
                s.push_str("\n\n---\nCONFIGURED SYSTEM INSTRUCTIONS:\n");
                s.push_str(base.trim());
            }
        }

        // Add any extra system prompt content (but never replace the default)
        if let Some(extra) = &self.system_prompt_extra {
            if !extra.trim().is_empty() {
                s.push_str("\n\n---\nADDITIONAL INSTRUCTIONS:\n");
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

        // Add relevance gate instructions
        if effective_cfg.relevance_gate {
            s.push_str("\n\n---\nRELEVANCE GATE ENABLED:\n");
            s.push_str("Include a \"relevant\": true|false field in your JSON response.\n");
            s.push_str("Set true if the page content is relevant to the extraction/crawl goals.\n");
            s.push_str(
                "Set false if the page is off-topic, a 404, login wall, or otherwise not useful.\n",
            );
            if let Some(prompt) = &effective_cfg.relevance_prompt {
                s.push_str("\nRelevance criteria: ");
                s.push_str(prompt.trim());
                s.push('\n');
            } else if let Some(ep) = &effective_cfg.extraction_prompt {
                s.push_str("\nJudge relevance against: ");
                s.push_str(ep.trim());
                s.push('\n');
            }
        }

        s.push_str("\n\n---\nRUNTIME CONFIG (read-only):\n");
        s.push_str(&format!(
            "- include_url: {}\n- include_title: {}\n- include_html: {}\n- html_max_bytes: {}\n- temperature: {}\n- max_tokens: {}\n- request_json_object: {}\n- best_effort_json_extract: {}\n- max_rounds: {}\n- extra_ai_data: {}\n- relevance_gate: {}\n",
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
            effective_cfg.relevance_gate,
        ));

        s
    }

    // ── dual-model routing ──────────────────────────────────────────

    /// Set the vision model endpoint for dual-model routing.
    pub fn with_vision_model(
        &mut self,
        endpoint: Option<super::config::ModelEndpoint>,
    ) -> &mut Self {
        self.vision_model = endpoint;
        self
    }

    /// Set the text model endpoint for dual-model routing.
    pub fn with_text_model(&mut self, endpoint: Option<super::config::ModelEndpoint>) -> &mut Self {
        self.text_model = endpoint;
        self
    }

    /// Set the vision routing mode.
    pub fn with_vision_route_mode(&mut self, mode: super::config::VisionRouteMode) -> &mut Self {
        self.vision_route_mode = mode;
        self
    }

    /// Whether dual-model routing is active.
    pub fn has_dual_model_routing(&self) -> bool {
        self.vision_model.is_some() || self.text_model.is_some()
    }

    /// Resolve (api_url, model_name, api_key) for the current round.
    ///
    /// Delegates to the same logic as [`RemoteMultimodalConfigs::resolve_model_for_round`]
    /// but uses the engine's own fields.
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

    /// Decide whether to use vision this round.
    pub fn should_use_vision_this_round(
        &self,
        round_idx: usize,
        stagnated: bool,
        action_stuck_rounds: usize,
        force_vision: bool,
    ) -> bool {
        if !self.has_dual_model_routing() {
            return true;
        }
        if force_vision {
            return true;
        }
        match self.vision_route_mode {
            super::config::VisionRouteMode::AlwaysPrimary => true,
            super::config::VisionRouteMode::TextFirst => {
                round_idx == 0 || stagnated || action_stuck_rounds >= 3
            }
            super::config::VisionRouteMode::VisionFirst => {
                round_idx < 2 || stagnated || action_stuck_rounds >= 3
            }
            super::config::VisionRouteMode::AgentDriven => false,
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
    pub async fn extract_from_html(
        &self,
        html: &str,
        url: &str,
        title: Option<&str>,
    ) -> EngineResult<AutomationResult> {
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
            #[serde(skip_serializing_if = "Option::is_none")]
            reasoning: Option<serde_json::Value>,
        }

        let Some((
            effective_cfg,
            effective_system_prompt,
            effective_system_prompt_extra,
            effective_user_message_extra,
        )) = self.resolve_runtime_for_url(url)
        else {
            return Ok(AutomationResult {
                label: "url_not_allowed".into(),
                steps_executed: 0,
                success: true,
                error: None,
                usage: AutomationUsage::default(),
                extracted: None,
                screenshot: None,
                spawn_pages: Vec::new(),
                relevant: None,
                reasoning: None,
            });
        };

        let mut prompt_engine = self.clone();
        prompt_engine.system_prompt = effective_system_prompt;
        prompt_engine.system_prompt_extra = effective_system_prompt_extra;

        // Build user prompt with HTML context
        let mut user_text =
            String::with_capacity(256 + html.len().min(effective_cfg.html_max_bytes));
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

        user_text.push_str(
            "\n\nTASK:\nExtract structured data from the HTML above. Return a JSON object with:\n",
        );
        user_text.push_str("- \"label\": short description of what was extracted\n");
        user_text.push_str("- \"done\": true\n");
        user_text.push_str("- \"steps\": [] (empty, no browser automation)\n");
        user_text.push_str("- \"extracted\": the structured data extracted from the page\n");
        if effective_cfg.relevance_gate {
            user_text.push_str(
                "- \"relevant\": true if page is relevant to the goal, false otherwise\n",
            );
        }

        if let Some(extra) = &effective_user_message_extra {
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
                        text: Some(prompt_engine.system_prompt_compiled(&effective_cfg)),
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
            reasoning: reasoning_payload(&effective_cfg),
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

        // Extract relevance field if gate is enabled
        let relevant = if effective_cfg.relevance_gate {
            Some(
                plan_value
                    .get("relevant")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true),
            )
        } else {
            None
        };
        let reasoning = plan_value.get("reasoning").and_then(|v| {
            if let Some(s) = v.as_str() {
                let trimmed = s.trim();
                return if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_string())
                };
            }
            if v.is_null() {
                None
            } else {
                Some(v.to_string())
            }
        });

        // Try to get extracted field, or fallback to the entire response
        let extracted = plan_value.get("extracted").cloned().or_else(|| {
            // If no explicit "extracted" field but response looks like extracted data
            // (has no standard automation fields), use the whole response
            if plan_value.get("label").is_none()
                && plan_value.get("done").is_none()
                && plan_value.get("steps").is_none()
            {
                // Response doesn't have automation structure, treat as direct extraction
                Some(plan_value.clone())
            } else {
                // In extraction mode, if response has automation structure but no extracted,
                // check if there's any non-automation data to extract
                let mut extracted_data = serde_json::Map::new();
                if let Some(obj) = plan_value.as_object() {
                    for (key, value) in obj {
                        // Skip known automation fields
                        if !matches!(
                            key.as_str(),
                            "label"
                                | "done"
                                | "steps"
                                | "memory_ops"
                                | "extracted"
                                | "relevant"
                                | "reasoning"
                        ) {
                            extracted_data.insert(key.clone(), value.clone());
                        }
                    }
                }
                if !extracted_data.is_empty() {
                    Some(serde_json::Value::Object(extracted_data))
                } else {
                    None
                }
            }
        });

        Ok(AutomationResult {
            label,
            steps_executed: 0,
            success: true,
            error: None,
            usage,
            extracted,
            screenshot: None,
            spawn_pages: Vec::new(),
            relevant,
            reasoning,
        })
    }

    /// Extract structured data from HTML with an optional screenshot.
    ///
    /// This method combines HTML text with a screenshot for more accurate extraction,
    /// especially useful for pages with visual content that isn't in the HTML
    /// (iframes, videos, canvas, dynamically rendered content).
    ///
    /// # Arguments
    /// * `html` - The raw HTML content
    /// * `url` - The URL of the page (for context)
    /// * `title` - Optional page title
    /// * `screenshot_base64` - Optional base64-encoded screenshot (PNG/JPEG)
    pub async fn extract_with_screenshot(
        &self,
        html: &str,
        url: &str,
        title: Option<&str>,
        screenshot_base64: Option<&str>,
    ) -> EngineResult<AutomationResult> {
        #[derive(Serialize)]
        struct ContentBlock {
            #[serde(rename = "type")]
            content_type: String,
            #[serde(skip_serializing_if = "Option::is_none")]
            text: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            image_url: Option<ImageUrlBlock>,
        }

        #[derive(Serialize)]
        struct ImageUrlBlock {
            url: String,
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
            response_format: Option<ResponseFormat>,
            #[serde(skip_serializing_if = "Option::is_none")]
            reasoning: Option<serde_json::Value>,
        }

        let Some((
            effective_cfg,
            effective_system_prompt,
            effective_system_prompt_extra,
            effective_user_message_extra,
        )) = self.resolve_runtime_for_url(url)
        else {
            return Ok(AutomationResult {
                label: "url_not_allowed".into(),
                steps_executed: 0,
                success: true,
                error: None,
                usage: AutomationUsage::default(),
                extracted: None,
                screenshot: None,
                spawn_pages: Vec::new(),
                relevant: None,
                reasoning: None,
            });
        };

        let mut prompt_engine = self.clone();
        prompt_engine.system_prompt = effective_system_prompt;
        prompt_engine.system_prompt_extra = effective_system_prompt_extra;

        // Build user prompt with HTML context
        let mut user_text =
            String::with_capacity(256 + html.len().min(effective_cfg.html_max_bytes));
        user_text.push_str("EXTRACTION CONTEXT:\n");
        user_text.push_str("- url: ");
        user_text.push_str(url);
        user_text.push('\n');
        if let Some(t) = title {
            user_text.push_str("- title: ");
            user_text.push_str(t);
            user_text.push('\n');
        }

        // Analyze content and note if screenshot is being used
        if screenshot_base64.is_some() {
            user_text.push_str("- screenshot: provided (use for visual content not in HTML)\n");
        }

        user_text.push_str("\nHTML CONTENT:\n");
        let html_truncated = truncate_utf8_tail(html, effective_cfg.html_max_bytes);
        user_text.push_str(&html_truncated);

        user_text.push_str("\n\nTASK:\nExtract structured data from the page. Use both the HTML and screenshot (if provided) to extract information. Return a JSON object with:\n");
        user_text.push_str("- \"label\": short description of what was extracted\n");
        user_text.push_str("- \"done\": true\n");
        user_text.push_str("- \"steps\": [] (empty, no browser automation)\n");
        user_text.push_str("- \"extracted\": the structured data extracted from the page\n");
        if effective_cfg.relevance_gate {
            user_text.push_str(
                "- \"relevant\": true if page is relevant to the goal, false otherwise\n",
            );
        }

        if screenshot_base64.is_some() {
            user_text.push_str("\nIMPORTANT: The screenshot may contain visual information not present in the HTML (iframe content, videos, canvas drawings, dynamically rendered content). Examine the screenshot carefully.\n");
        }

        if let Some(extra) = &effective_user_message_extra {
            if !extra.trim().is_empty() {
                user_text.push_str("\n---\nUSER INSTRUCTIONS:\n");
                user_text.push_str(extra.trim());
                user_text.push('\n');
            }
        }

        // Build message content
        let mut user_content = vec![ContentBlock {
            content_type: "text".into(),
            text: Some(user_text),
            image_url: None,
        }];

        // Add screenshot if provided
        if let Some(screenshot) = screenshot_base64 {
            let image_url = if screenshot.starts_with("data:") {
                screenshot.to_string()
            } else {
                format!("data:image/png;base64,{}", screenshot)
            };
            user_content.push(ContentBlock {
                content_type: "image_url".into(),
                text: None,
                image_url: Some(ImageUrlBlock { url: image_url }),
            });
        }

        let request_body = InferenceRequest {
            model: self.model_name.clone(),
            messages: vec![
                Message {
                    role: "system".into(),
                    content: vec![ContentBlock {
                        content_type: "text".into(),
                        text: Some(prompt_engine.system_prompt_compiled(&effective_cfg)),
                        image_url: None,
                    }],
                },
                Message {
                    role: "user".into(),
                    content: user_content,
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
            reasoning: reasoning_payload(&effective_cfg),
        };

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
            "remote_multimodal extract_with_screenshot: status={} latency={:?} body_len={}",
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

        // Extract relevance field if gate is enabled
        let relevant = if effective_cfg.relevance_gate {
            Some(
                plan_value
                    .get("relevant")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true),
            )
        } else {
            None
        };
        let reasoning = plan_value.get("reasoning").and_then(|v| {
            if let Some(s) = v.as_str() {
                let trimmed = s.trim();
                return if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_string())
                };
            }
            if v.is_null() {
                None
            } else {
                Some(v.to_string())
            }
        });

        // Try to get extracted field, or fallback to the entire response
        let extracted = plan_value.get("extracted").cloned().or_else(|| {
            // If no explicit "extracted" field but response looks like extracted data
            // (has no standard automation fields), use the whole response
            if plan_value.get("label").is_none()
                && plan_value.get("done").is_none()
                && plan_value.get("steps").is_none()
            {
                Some(plan_value.clone())
            } else {
                // Extract non-automation fields
                let mut extracted_data = serde_json::Map::new();
                if let Some(obj) = plan_value.as_object() {
                    for (key, value) in obj {
                        if !matches!(
                            key.as_str(),
                            "label"
                                | "done"
                                | "steps"
                                | "memory_ops"
                                | "extracted"
                                | "relevant"
                                | "reasoning"
                        ) {
                            extracted_data.insert(key.clone(), value.clone());
                        }
                    }
                }
                if !extracted_data.is_empty() {
                    Some(serde_json::Value::Object(extracted_data))
                } else {
                    None
                }
            }
        });

        Ok(AutomationResult {
            label,
            steps_executed: 0,
            success: true,
            error: None,
            usage,
            extracted,
            screenshot: None,
            spawn_pages: Vec::new(),
            relevant,
            reasoning,
        })
    }

    /// Send a raw chat completion request and get the response.
    ///
    /// This is a lower-level method for custom use cases.
    pub async fn chat_completion(
        &self,
        system_prompt: &str,
        user_message: &str,
    ) -> EngineResult<(String, AutomationUsage)> {
        #[derive(Serialize)]
        struct Message {
            role: String,
            content: String,
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
            response_format: Option<ResponseFormat>,
            #[serde(skip_serializing_if = "Option::is_none")]
            reasoning: Option<serde_json::Value>,
        }

        let request_body = InferenceRequest {
            model: self.model_name.clone(),
            messages: vec![
                Message {
                    role: "system".into(),
                    content: system_prompt.to_string(),
                },
                Message {
                    role: "user".into(),
                    content: user_message.to_string(),
                },
            ],
            temperature: self.cfg.temperature,
            max_tokens: self.cfg.max_tokens,
            response_format: if self.cfg.request_json_object {
                Some(ResponseFormat {
                    format_type: "json_object".into(),
                })
            } else {
                None
            },
            reasoning: reasoning_payload(&self.cfg),
        };

        let _permit = self.acquire_llm_permit().await;

        let mut req = CLIENT.post(&self.api_url).json(&request_body);
        if let Some(key) = &self.api_key {
            req = req.bearer_auth(key);
        }

        let http_resp = req.send().await?;
        let status = http_resp.status();
        let raw_body = http_resp.text().await?;

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

        Ok((content, usage))
    }

    // ===== URL Pre-filter Classification =====

    /// Classify a batch of URLs as relevant or irrelevant using the text model.
    /// Returns a `Vec<bool>` parallel to the input URLs (`true` = relevant).
    ///
    /// Uses `resolve_model_for_round(false)` to get the cheap/fast text model.
    /// On any failure (HTTP, parse, length mismatch), returns all `true` (safe fallback).
    pub async fn classify_urls(
        &self,
        urls: &[&str],
        relevance_prompt: Option<&str>,
        extraction_prompt: Option<&str>,
        max_tokens: u16,
    ) -> EngineResult<Vec<bool>> {
        if urls.is_empty() {
            return Ok(Vec::new());
        }

        let criteria = relevance_prompt
            .or(extraction_prompt)
            .unwrap_or("General web crawling");

        let system = format!(
            "You are a URL relevance classifier. Given a list of URLs, determine which are relevant to the crawl goal.\nGoal: {}\n\nRespond ONLY with a JSON array of 1s and 0s, one per URL. 1=relevant, 0=irrelevant.\nExample: [1,0,1,1,0]",
            criteria
        );

        let mut user_msg = String::with_capacity(urls.len() * 80);
        user_msg.push_str("Classify these URLs:\n");
        for (i, url) in urls.iter().enumerate() {
            user_msg.push_str(&format!("{}. {}\n", i + 1, url));
        }

        // Use the text model endpoint (cheap/fast)
        let (api_url, model_name, api_key) = self.resolve_model_for_round(false);

        #[derive(Serialize)]
        struct Message {
            role: String,
            content: String,
        }
        #[derive(Serialize)]
        struct InferenceRequest {
            model: String,
            messages: Vec<Message>,
            temperature: f32,
            max_tokens: u16,
            #[serde(skip_serializing_if = "Option::is_none")]
            reasoning: Option<serde_json::Value>,
        }

        let request_body = InferenceRequest {
            model: model_name.to_string(),
            messages: vec![
                Message {
                    role: "system".into(),
                    content: system,
                },
                Message {
                    role: "user".into(),
                    content: user_msg,
                },
            ],
            temperature: 0.0,
            max_tokens,
            reasoning: reasoning_payload(&self.cfg),
        };

        let _permit = self.acquire_llm_permit().await;

        let mut req = CLIENT.post(api_url).json(&request_body);
        if let Some(key) = api_key {
            req = req.bearer_auth(key);
        }

        let http_resp = match req.send().await {
            Ok(r) => r,
            Err(e) => {
                log::warn!("url_prefilter: HTTP error, assuming all relevant: {e}");
                return Ok(vec![true; urls.len()]);
            }
        };

        if !http_resp.status().is_success() {
            log::warn!(
                "url_prefilter: non-success status {}, assuming all relevant",
                http_resp.status()
            );
            return Ok(vec![true; urls.len()]);
        }

        let raw_body = http_resp.text().await.unwrap_or_default();
        let root: serde_json::Value = match serde_json::from_str(&raw_body) {
            Ok(v) => v,
            Err(_) => return Ok(vec![true; urls.len()]),
        };

        let content = match extract_assistant_content(&root) {
            Some(c) => c,
            None => return Ok(vec![true; urls.len()]),
        };

        Ok(parse_url_classifications(&content, urls.len()))
    }

    // ===== New Feature Integration Methods =====

    /// Generate an extraction schema from example data.
    ///
    /// Uses the schema generation utilities to create a JSON schema
    /// from example outputs. Useful for zero-config extraction setup.
    pub fn generate_schema_from_examples(
        &self,
        examples: &[serde_json::Value],
        name: Option<&str>,
        description: Option<&str>,
    ) -> super::schema_gen::GeneratedSchema {
        let request = super::schema_gen::SchemaGenerationRequest {
            examples: examples.to_vec(),
            description: description.map(|s| s.to_string()),
            strict: false,
            name: name.map(|s| s.to_string()),
        };
        super::schema_gen::generate_schema(&request)
    }

    /// Infer a JSON schema from a single example value.
    pub fn infer_schema(&self, example: &serde_json::Value) -> serde_json::Value {
        super::schema_gen::infer_schema(example)
    }

    /// Build a schema generation prompt for LLM-assisted schema creation.
    pub fn build_schema_prompt(
        &self,
        examples: &[serde_json::Value],
        description: Option<&str>,
    ) -> String {
        let request = super::schema_gen::SchemaGenerationRequest {
            examples: examples.to_vec(),
            description: description.map(|s| s.to_string()),
            strict: false,
            name: None,
        };
        super::schema_gen::build_schema_generation_prompt(&request)
    }

    /// Parse tool calls from an LLM response.
    ///
    /// Extracts OpenAI-compatible tool calls from a response JSON.
    pub fn parse_tool_calls(
        &self,
        response: &serde_json::Value,
    ) -> Vec<super::tool_calling::ToolCall> {
        super::tool_calling::parse_tool_calls(response)
    }

    /// Convert tool calls to automation step actions.
    pub fn tool_calls_to_steps(
        &self,
        calls: &[super::tool_calling::ToolCall],
    ) -> Vec<serde_json::Value> {
        super::tool_calling::tool_calls_to_steps(calls)
    }

    /// Get all available action tool schemas.
    ///
    /// Returns OpenAI-compatible tool definitions for all supported actions.
    pub fn action_tool_schemas(&self) -> Vec<super::tool_calling::ToolDefinition> {
        super::tool_calling::ActionToolSchemas::all()
    }

    /// Extract HTML context around selectors for self-healing.
    pub fn extract_html_context(&self, html: &str, max_bytes: usize) -> String {
        super::self_healing::extract_html_context(html, max_bytes)
    }

    /// Create a new dependency graph for concurrent execution.
    pub fn create_dependency_graph(
        &self,
        steps: Vec<super::concurrent_chain::DependentStep>,
    ) -> Result<super::concurrent_chain::DependencyGraph, String> {
        super::concurrent_chain::DependencyGraph::new(steps)
    }

    /// Execute a dependency graph with the provided executor.
    ///
    /// This enables parallel execution of independent steps using `tokio::JoinSet`.
    pub async fn execute_dependency_graph<F, Fut>(
        &self,
        graph: &mut super::concurrent_chain::DependencyGraph,
        config: &super::concurrent_chain::ConcurrentChainConfig,
        executor: F,
    ) -> super::concurrent_chain::ConcurrentChainResult
    where
        F: Fn(super::concurrent_chain::DependentStep) -> Fut + Clone + Send + Sync + 'static,
        Fut: std::future::Future<Output = super::concurrent_chain::StepResult> + Send + 'static,
    {
        super::concurrent_chain::execute_graph(graph, config, executor).await
    }
}

/// Parse a URL classification response into a `Vec<bool>`.
///
/// Expects a JSON array of 0/1 integers, e.g. `[1,0,1,1,0]`.
/// On parse failure or length mismatch, returns all `true` (safe fallback).
fn parse_url_classifications(response: &str, expected_len: usize) -> Vec<bool> {
    // Try to find a JSON array in the response
    let trimmed = response.trim();

    // Find the array boundaries
    let start = match trimmed.find('[') {
        Some(i) => i,
        None => return vec![true; expected_len],
    };
    let end = match trimmed.rfind(']') {
        Some(i) => i + 1,
        None => return vec![true; expected_len],
    };

    let arr_str = &trimmed[start..end];

    let arr: Vec<serde_json::Value> = match serde_json::from_str(arr_str) {
        Ok(v) => v,
        Err(_) => return vec![true; expected_len],
    };

    if arr.len() != expected_len {
        log::warn!(
            "url_prefilter: classification length mismatch (got {}, expected {}), assuming all relevant",
            arr.len(),
            expected_len
        );
        return vec![true; expected_len];
    }

    arr.iter()
        .map(|v| {
            // Accept 1/0 as integers or booleans
            v.as_i64().map(|n| n != 0).unwrap_or_else(|| {
                v.as_bool().unwrap_or(true) // default to relevant
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_url_classifications_valid() {
        assert_eq!(
            parse_url_classifications("[1,0,1]", 3),
            vec![true, false, true]
        );
    }

    #[test]
    fn test_parse_url_classifications_booleans() {
        assert_eq!(
            parse_url_classifications("[true,false,true]", 3),
            vec![true, false, true]
        );
    }

    #[test]
    fn test_parse_url_classifications_length_mismatch() {
        // Length mismatch → all true
        assert_eq!(
            parse_url_classifications("[1,0]", 3),
            vec![true, true, true]
        );
    }

    #[test]
    fn test_parse_url_classifications_invalid_json() {
        assert_eq!(parse_url_classifications("not json", 2), vec![true, true]);
    }

    #[test]
    fn test_parse_url_classifications_embedded_array() {
        // Array embedded in surrounding text
        assert_eq!(
            parse_url_classifications("Here are the results: [1,0,1,0]", 4),
            vec![true, false, true, false]
        );
    }

    #[test]
    fn test_parse_url_classifications_empty() {
        assert_eq!(parse_url_classifications("[]", 0), Vec::<bool>::new());
    }

    #[test]
    fn test_engine_new() {
        let engine = RemoteMultimodalEngine::new(
            "https://api.openai.com/v1/chat/completions",
            "gpt-4o",
            None,
        );

        assert_eq!(engine.api_url, "https://api.openai.com/v1/chat/completions");
        assert_eq!(engine.model_name, "gpt-4o");
        assert!(engine.api_key.is_none());
        assert!(engine.system_prompt.is_none());
    }

    #[test]
    fn test_engine_with_api_key() {
        let engine = RemoteMultimodalEngine::new(
            "https://api.openai.com/v1/chat/completions",
            "gpt-4o",
            None,
        )
        .with_api_key(Some("sk-test"));

        assert_eq!(engine.api_key, Some("sk-test".to_string()));
    }

    #[test]
    fn test_engine_system_prompt_compiled() {
        // System prompt is locked to DEFAULT_SYSTEM_PROMPT
        // Custom instructions go through system_prompt_extra
        let mut engine = RemoteMultimodalEngine::new(
            "https://api.openai.com/v1/chat/completions",
            "gpt-4o",
            None,
        );
        engine.with_system_prompt_extra(Some("Custom instructions"));

        let compiled = engine.system_prompt_compiled(&RemoteMultimodalConfig::default());
        assert!(compiled.starts_with(super::DEFAULT_SYSTEM_PROMPT));
        assert!(compiled.contains("Custom instructions"));
        assert!(compiled.contains("RUNTIME CONFIG"));
    }

    #[test]
    fn test_engine_system_prompt_with_extraction() {
        let cfg = RemoteMultimodalConfig {
            extra_ai_data: true,
            extraction_schema: Some(ExtractionSchema::new("products", r#"{"type":"array"}"#)),
            ..Default::default()
        };

        let engine = RemoteMultimodalEngine::new(
            "https://api.openai.com/v1/chat/completions",
            "gpt-4o",
            None,
        );

        let compiled = engine.system_prompt_compiled(&cfg);
        assert!(compiled.contains("EXTRACTION MODE ENABLED"));
        assert!(compiled.contains("products"));
    }

    #[test]
    fn test_engine_system_prompt_extraction_only() {
        let cfg = RemoteMultimodalConfig::new()
            .with_extraction(true)
            .with_max_rounds(1);
        assert!(cfg.is_extraction_only());

        let engine = RemoteMultimodalEngine::new(
            "https://api.openai.com/v1/chat/completions",
            "gpt-4o",
            None,
        );

        let compiled = engine.system_prompt_compiled(&cfg);
        // Should use focused extraction prompt, not full automation prompt
        assert!(compiled.contains("data extraction assistant"));
        assert!(!compiled.contains("ClickPoint"));
        assert!(!compiled.contains("SetViewport"));
    }

    #[test]
    fn test_engine_analyze_content() {
        let engine = RemoteMultimodalEngine::new(
            "https://api.openai.com/v1/chat/completions",
            "gpt-4o",
            None,
        );

        let html = "<html><body><p>Test content</p></body></html>";
        let analysis = engine.analyze_content(html);
        assert!(!analysis.has_visual_elements);
    }

    #[test]
    fn test_engine_needs_screenshot() {
        let engine = RemoteMultimodalEngine::new(
            "https://api.openai.com/v1/chat/completions",
            "gpt-4o",
            None,
        );

        assert!(engine.needs_screenshot("<iframe src='x'></iframe>"));
        assert!(!engine.needs_screenshot(&"a".repeat(2000)));
    }

    #[test]
    fn test_engine_clone_with_cfg() {
        let engine = RemoteMultimodalEngine::new(
            "https://api.openai.com/v1/chat/completions",
            "gpt-4o",
            None,
        )
        .with_api_key(Some("sk-test"));

        let new_cfg = RemoteMultimodalConfig {
            max_rounds: 10,
            ..Default::default()
        };

        let cloned = engine.clone_with_cfg(new_cfg);
        assert_eq!(cloned.api_key, Some("sk-test".to_string()));
        assert_eq!(cloned.cfg.max_rounds, 10);
    }

    // ── Dual-model routing engine tests ──────────────────────────────

    #[test]
    fn test_engine_dual_model_routing_setup() {
        let mut engine = RemoteMultimodalEngine::new("https://api.example.com", "gpt-4o", None);
        assert!(!engine.has_dual_model_routing());

        engine.with_vision_model(Some(crate::automation::config::ModelEndpoint::new(
            "gpt-4o",
        )));
        engine.with_text_model(Some(crate::automation::config::ModelEndpoint::new(
            "gpt-4o-mini",
        )));
        assert!(engine.has_dual_model_routing());
    }

    #[test]
    fn test_engine_resolve_model_for_round() {
        let mut engine = RemoteMultimodalEngine::new("https://api.example.com", "primary", None);
        engine.api_key = Some("sk-parent".to_string());
        engine.with_vision_model(Some(crate::automation::config::ModelEndpoint::new(
            "vision-model",
        )));
        engine.with_text_model(Some(
            crate::automation::config::ModelEndpoint::new("text-model")
                .with_api_url("https://text.api.com")
                .with_api_key("sk-text"),
        ));

        // Vision round → vision model, inherits parent URL/key
        let (url, model, key) = engine.resolve_model_for_round(true);
        assert_eq!(model, "vision-model");
        assert_eq!(url, "https://api.example.com");
        assert_eq!(key, Some("sk-parent"));

        // Text round → text model with its own URL/key
        let (url, model, key) = engine.resolve_model_for_round(false);
        assert_eq!(model, "text-model");
        assert_eq!(url, "https://text.api.com");
        assert_eq!(key, Some("sk-text"));
    }

    #[test]
    fn test_engine_should_use_vision_this_round() {
        let mut engine = RemoteMultimodalEngine::new("https://api.example.com", "gpt-4o", None);
        engine.with_vision_model(Some(crate::automation::config::ModelEndpoint::new(
            "gpt-4o",
        )));
        engine.with_text_model(Some(crate::automation::config::ModelEndpoint::new(
            "gpt-4o-mini",
        )));
        engine.with_vision_route_mode(crate::automation::config::VisionRouteMode::TextFirst);

        // Round 0 → vision
        assert!(engine.should_use_vision_this_round(0, false, 0, false));
        // Round 1+ → text
        assert!(!engine.should_use_vision_this_round(1, false, 0, false));
        // Stagnation → vision
        assert!(engine.should_use_vision_this_round(3, true, 0, false));
        // Stuck ≥ 3 → vision
        assert!(engine.should_use_vision_this_round(3, false, 3, false));
        // Force → vision
        assert!(engine.should_use_vision_this_round(5, false, 0, true));
    }

    #[test]
    fn test_engine_no_routing_always_vision() {
        let engine = RemoteMultimodalEngine::new("https://api.example.com", "gpt-4o", None);
        // No dual routing → always returns true
        assert!(!engine.has_dual_model_routing());
        assert!(engine.should_use_vision_this_round(0, false, 0, false));
        assert!(engine.should_use_vision_this_round(99, false, 0, false));
    }

    #[test]
    fn test_engine_system_prompt_extraction_only_with_schema() {
        let mut cfg = RemoteMultimodalConfig::new()
            .with_extraction(true)
            .with_max_rounds(1);
        cfg.extraction_schema = Some(ExtractionSchema::new("products", r#"{"type":"array"}"#));
        cfg.extraction_prompt = Some("Extract all products".to_string());

        let engine = RemoteMultimodalEngine::new("https://api.example.com", "gpt-4o", None);

        let compiled = engine.system_prompt_compiled(&cfg);
        // Should use focused prompt
        assert!(compiled.contains("data extraction assistant"));
        // Should include extraction schema and prompt
        assert!(compiled.contains("EXTRACTION MODE ENABLED"));
        assert!(compiled.contains("products"));
        assert!(compiled.contains("Extract all products"));
    }

    #[test]
    fn test_engine_system_prompt_multi_round_extraction_uses_default() {
        // extra_ai_data=true but max_rounds=6 → NOT extraction-only
        let cfg = RemoteMultimodalConfig {
            extra_ai_data: true,
            ..Default::default()
        };
        assert!(!cfg.is_extraction_only());

        let engine = RemoteMultimodalEngine::new("https://api.example.com", "gpt-4o", None);

        let compiled = engine.system_prompt_compiled(&cfg);
        // Should use full automation prompt
        assert!(compiled.contains("ClickPoint"));
        assert!(compiled.contains("SetViewport"));
        assert!(compiled.contains("EXTRACTION MODE ENABLED"));
    }

    #[test]
    fn test_engine_resolve_runtime_for_url_override() {
        let mut url_map = std::collections::HashMap::new();
        let override_cfg = crate::automation::AutomationConfig::new("override goal")
            .with_max_steps(2)
            .with_retries(5)
            .with_system_prompt("override system")
            .with_system_prompt_extra("override extra")
            .with_user_message_extra("override user")
            .with_extraction("extract fields");
        url_map.insert("https://example.com".to_string(), Box::new(override_cfg));
        let gate = crate::automation::PromptUrlGate::with_map(url_map);

        let mut engine = RemoteMultimodalEngine::new("https://api.example.com", "gpt-4o", None);
        engine.with_prompt_url_gate(Some(gate));

        let resolved = engine
            .resolve_runtime_for_url("https://example.com")
            .expect("url should be allowed");
        let (cfg, system_prompt, system_prompt_extra, user_message_extra) = resolved;
        assert_eq!(cfg.max_rounds, 2);
        assert_eq!(cfg.retry.max_attempts, 5);
        assert!(cfg.extra_ai_data);
        assert_eq!(cfg.extraction_prompt.as_deref(), Some("extract fields"));
        assert_eq!(system_prompt.as_deref(), Some("override system"));
        assert_eq!(system_prompt_extra.as_deref(), Some("override extra"));
        assert_eq!(user_message_extra.as_deref(), Some("override user"));
    }

    #[test]
    fn test_engine_resolve_runtime_for_url_blocked() {
        let mut url_map = std::collections::HashMap::new();
        url_map.insert(
            "https://allowed.com".to_string(),
            Box::new(crate::automation::AutomationConfig::new("allowed")),
        );
        let gate = crate::automation::PromptUrlGate::with_map(url_map);

        let mut engine = RemoteMultimodalEngine::new("https://api.example.com", "gpt-4o", None);
        engine.with_prompt_url_gate(Some(gate));

        assert!(engine
            .resolve_runtime_for_url("https://blocked.com")
            .is_none());
    }
}
