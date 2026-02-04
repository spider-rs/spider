//! Browser-specific automation methods for chromiumoxide integration.
//!
//! This module provides browser automation capabilities that require a
//! Chrome browser page. All methods are gated behind `#[cfg(feature = "chrome")]`.

#[cfg(feature = "chrome")]
use base64::{engine::general_purpose, Engine as _};
#[cfg(feature = "chrome")]
use chromiumoxide::{
    cdp::browser_protocol::{
        input::{DispatchMouseEventParams, DispatchMouseEventType, MouseButton},
        page::CaptureScreenshotFormat,
    },
    page::ScreenshotParams,
    Page,
};

use super::{
    clean_html_with_profile, truncate_utf8_tail, ActResult, AutomationMemory,
    AutomationResult, AutomationUsage, CaptureProfile, EngineError, EngineResult,
    HtmlCleaningProfile, MemoryOperation, PageObservation, RemoteMultimodalConfig,
    RemoteMultimodalEngine,
};

/// State signature for stagnation detection.
#[cfg(feature = "chrome")]
#[derive(Debug, Clone)]
pub(crate) struct StateSignature {
    /// Current page URL.
    url: String,
    /// Current document title.
    title: String,
    /// Hash of the last N bytes of cleaned HTML (tail hash).
    html_tail_hash: u64,
    /// Total cleaned HTML length.
    html_len: usize,
}

#[cfg(feature = "chrome")]
impl StateSignature {
    pub fn new(url: &str, title: &str, html: &str) -> Self {
        use super::fnv1a64;
        let tail = truncate_utf8_tail(html, 2048);
        let h = fnv1a64(tail.as_bytes());
        Self {
            url: url.to_string(),
            title: title.to_string(),
            html_tail_hash: h,
            html_len: html.len(),
        }
    }

    pub fn eq_soft(&self, other: &Self) -> bool {
        self.url == other.url
            && self.title == other.title
            && self.html_tail_hash == other.html_tail_hash
            && self.html_len == other.html_len
    }
}

/// Internal plan from LLM response.
#[cfg(feature = "chrome")]
#[derive(Debug, Clone, Default)]
pub(crate) struct AutomationPlan {
    pub label: String,
    pub done: bool,
    pub steps: Vec<serde_json::Value>,
    pub extracted: Option<serde_json::Value>,
    pub memory_ops: Vec<MemoryOperation>,
    pub usage: AutomationUsage,
}

#[cfg(feature = "chrome")]
impl RemoteMultimodalEngine {
    // -----------------------------------------------------------------
    // Capture helpers
    // -----------------------------------------------------------------

    /// Capture screenshot as data URL with profile settings.
    /// Automatically applies grayscale filter for text CAPTCHAs to improve readability.
    pub(crate) async fn screenshot_as_data_url_with_profile(
        &self,
        page: &Page,
        cap: &CaptureProfile,
    ) -> EngineResult<String> {
        // Auto-detect text CAPTCHA and apply grayscale for better readability
        let is_text_captcha = page.evaluate(r#"
            (() => {
                const text = document.body?.innerText?.toLowerCase() || '';
                return text.includes('enter the text') ||
                       text.includes('type the') ||
                       text.includes('wiggles') ||
                       text.includes('distorted');
            })()
        "#).await.ok().and_then(|v| v.value().and_then(|v| v.as_bool())).unwrap_or(false);

        if is_text_captcha {
            // Apply grayscale to remove distracting colored lines
            let _ = page.evaluate("document.body.style.filter = 'grayscale(100%)'").await;
        }

        let params = ScreenshotParams::builder()
            .format(CaptureScreenshotFormat::Png)
            .full_page(cap.full_page)
            .omit_background(cap.omit_background)
            .build();

        let png = page
            .screenshot(params)
            .await
            .map_err(|e| EngineError::Remote(format!("screenshot failed: {e}")))?;

        // Restore color after screenshot
        if is_text_captcha {
            let _ = page.evaluate("document.body.style.filter = ''").await;
        }

        let b64 = general_purpose::STANDARD.encode(png);
        Ok(format!("data:image/png;base64,{}", b64))
    }

    /// Take a final screenshot and return as base64 string.
    pub(crate) async fn take_final_screenshot(&self, page: &Page) -> EngineResult<String> {
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

    /// Get page title context.
    pub(crate) async fn title_context(
        &self,
        page: &Page,
        effective_cfg: &RemoteMultimodalConfig,
    ) -> String {
        if !effective_cfg.include_title {
            return String::new();
        }
        match page.get_title().await {
            Ok(t) => t.unwrap_or_default(),
            Err(_) => String::new(),
        }
    }

    /// Get page URL context.
    pub(crate) async fn url_context(
        &self,
        page: &Page,
        effective_cfg: &RemoteMultimodalConfig,
    ) -> String {
        if !effective_cfg.include_url {
            return String::new();
        }
        match page.url().await {
            Ok(u_opt) => u_opt.unwrap_or_default(),
            Err(_) => String::new(),
        }
    }

    /// Get cleaned HTML context with profile settings.
    pub(crate) async fn html_context_with_profile(
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

    /// Build the user prompt for a round using captured state.
    pub(crate) fn build_user_prompt(
        &self,
        effective_cfg: &RemoteMultimodalConfig,
        _cap: &CaptureProfile,
        url_input: &str,
        url_now: &str,
        title_now: &str,
        html: &str,
        round_idx: usize,
        stagnated: bool,
        memory: Option<&AutomationMemory>,
    ) -> String {
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

        // Include memory context if available and non-empty
        if let Some(mem) = memory {
            if !mem.is_empty() {
                let mem_ctx = mem.to_context_string();
                if !mem_ctx.is_empty() {
                    out.push_str("SESSION MEMORY:\n");
                    out.push_str(&mem_ctx);
                    out.push_str("\n\n");
                }
            }
        }

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

    // -----------------------------------------------------------------
    // Main automation loop
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
    pub async fn run(&self, page: &Page, url_input: &str) -> EngineResult<AutomationResult> {
        self.run_with_memory(page, url_input, None).await
    }

    /// Runs iterative automation with session memory for agentic workflows.
    ///
    /// Same as `run()` but accepts a mutable memory reference that persists
    /// data across automation rounds. The LLM can read from and write to
    /// this memory using memory operations in its responses.
    ///
    /// # Arguments
    /// * `page` - The Chrome page to automate
    /// * `url_input` - The URL being processed
    /// * `memory` - Optional mutable memory for session state
    ///
    /// # Memory Operations
    /// The LLM can include a `memory_ops` array in its response:
    /// ```json
    /// {
    ///   "label": "...",
    ///   "done": false,
    ///   "steps": [...],
    ///   "memory_ops": [
    ///     { "op": "set", "key": "user_id", "value": "12345" },
    ///     { "op": "delete", "key": "temp" },
    ///     { "op": "clear" }
    ///   ]
    /// }
    /// ```
    pub async fn run_with_memory(
        &self,
        page: &Page,
        url_input: &str,
        mut memory: Option<&mut AutomationMemory>,
    ) -> EngineResult<AutomationResult> {
        // 0) URL gating check
        if let Some(gate) = &self.prompt_url_gate {
            if !gate.is_allowed(url_input) {
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

        let base_effective_cfg: RemoteMultimodalConfig = self.cfg.clone();

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

            // quick stagnation heuristic
            let sig = StateSignature::new(&url_now, &title_now, &html);
            let stagnated = last_sig.as_ref().map(|p| p.eq_soft(&sig)).unwrap_or(false);
            last_sig = Some(sig);

            // Ask model (with retry policy) - pass memory as immutable ref for context
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
                    memory.as_deref(),
                )
                .await?;

            // Accumulate token usage from this round
            total_usage.accumulate(&plan.usage);
            last_label = plan.label.clone();

            // Process memory operations from the plan
            if let Some(ref mut mem) = memory {
                for op in &plan.memory_ops {
                    match op {
                        MemoryOperation::Set { key, value } => {
                            mem.set(key.clone(), value.clone());
                        }
                        MemoryOperation::Delete { key } => {
                            mem.remove(key);
                        }
                        MemoryOperation::Clear => {
                            mem.clear_store();
                        }
                    }
                }
                // Record this round's URL and action
                mem.add_visited_url(&url_now);
                mem.add_action(format!("Round {}: {}", round_idx + 1, &plan.label));
            }

            // Save extracted data if present
            if plan.extracted.is_some() {
                last_extracted = plan.extracted.clone();
                // Also store in memory if available
                if let (Some(ref mut mem), Some(ref extracted)) = (&mut memory, &plan.extracted) {
                    mem.add_extraction(extracted.clone());
                }
            }

            // Done condition (model-driven)
            if plan.done || plan.steps.is_empty() {
                // Capture final screenshot (enabled by default)
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
            let steps_executed = self
                .execute_steps(page, &plan.steps, &base_effective_cfg)
                .await?;
            total_steps_executed += steps_executed;

            // Post-step delay
            if base_effective_cfg.post_plan_wait_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(
                    base_effective_cfg.post_plan_wait_ms,
                ))
                .await;
            }
        }

        // Final screenshot after all rounds
        let final_screenshot = if base_effective_cfg.screenshot {
            self.take_final_screenshot(page).await.ok()
        } else {
            None
        };

        Ok(AutomationResult {
            label: last_label,
            steps_executed: total_steps_executed,
            success: true,
            error: None,
            usage: total_usage,
            extracted: last_extracted,
            screenshot: final_screenshot,
        })
    }

    /// Infer plan with retry policy.
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
        memory: Option<&AutomationMemory>,
    ) -> EngineResult<AutomationPlan> {
        let max_attempts = effective_cfg.retry.max_attempts.max(1);
        let mut last_err = None;

        for attempt in 0..max_attempts {
            match self
                .infer_plan_once(
                    effective_cfg,
                    cap,
                    url_input,
                    url_now,
                    title_now,
                    html,
                    screenshot,
                    round_idx,
                    stagnated,
                    memory,
                )
                .await
            {
                Ok(plan) => return Ok(plan),
                Err(e) => {
                    last_err = Some(e);
                    if attempt + 1 < max_attempts {
                        // Exponential backoff with capped power
                        let power = attempt.min(6);
                        let delay = effective_cfg.retry.backoff_ms * (1 << power);
                        tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                    }
                }
            }
        }

        Err(last_err.unwrap_or_else(|| EngineError::Remote("max retries exceeded".to_string())))
    }

    /// Single plan inference attempt.
    async fn infer_plan_once(
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
        memory: Option<&AutomationMemory>,
    ) -> EngineResult<AutomationPlan> {
        use super::{
            best_effort_parse_json_object, extract_assistant_content, extract_usage,
            DEFAULT_SYSTEM_PROMPT,
        };
        use serde::Serialize;

        #[derive(Serialize)]
        struct Message {
            role: String,
            content: serde_json::Value,
        }

        #[derive(Serialize)]
        struct Request {
            model: String,
            messages: Vec<Message>,
            #[serde(skip_serializing_if = "Option::is_none")]
            temperature: Option<f32>,
            #[serde(skip_serializing_if = "Option::is_none")]
            max_tokens: Option<u32>,
            #[serde(skip_serializing_if = "Option::is_none")]
            response_format: Option<serde_json::Value>,
        }

        // Build system prompt
        let system_content = self
            .system_prompt
            .as_deref()
            .unwrap_or(DEFAULT_SYSTEM_PROMPT);
        let mut system_msg = system_content.to_string();
        if let Some(extra) = &self.system_prompt_extra {
            system_msg.push_str("\n\n");
            system_msg.push_str(extra);
        }

        // Build user prompt
        let user_text = self.build_user_prompt(
            effective_cfg,
            cap,
            url_input,
            url_now,
            title_now,
            html,
            round_idx,
            stagnated,
            memory,
        );

        // Build user content with image
        let user_content = serde_json::json!([
            { "type": "text", "text": user_text },
            {
                "type": "image_url",
                "image_url": { "url": screenshot }
            }
        ]);

        let messages = vec![
            Message {
                role: "system".to_string(),
                content: serde_json::Value::String(system_msg),
            },
            Message {
                role: "user".to_string(),
                content: user_content,
            },
        ];

        let response_format = if effective_cfg.request_json_object {
            Some(serde_json::json!({ "type": "json_object" }))
        } else {
            None
        };

        let request = Request {
            model: self.model_name.clone(),
            messages,
            temperature: Some(effective_cfg.temperature),
            max_tokens: Some(effective_cfg.max_tokens as u32),
            response_format,
        };

        // Acquire semaphore if configured
        let _permit = self.acquire_llm_permit().await;

        // Make HTTP request with 2 minute timeout for LLM calls
        static CLIENT: std::sync::LazyLock<reqwest::Client> =
            std::sync::LazyLock::new(|| {
                reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(120))
                    .build()
                    .unwrap_or_else(|_| reqwest::Client::new())
            });

        let mut req = CLIENT.post(&self.api_url).json(&request);
        if let Some(ref key) = self.api_key {
            req = req.bearer_auth(key);
        }

        let resp = req.send().await?;
        let status = resp.status();
        let body: serde_json::Value = resp.json().await?;

        if !status.is_success() {
            return Err(EngineError::Remote(format!(
                "API error {}: {}",
                status,
                serde_json::to_string_pretty(&body).unwrap_or_default()
            )));
        }

        // Extract content and usage
        let content = extract_assistant_content(&body)
            .ok_or_else(|| EngineError::MissingField("choices[0].message.content"))?;
        let mut usage = extract_usage(&body);
        usage.increment_llm_calls();

        // Parse JSON response
        let parsed = if effective_cfg.best_effort_json_extract {
            best_effort_parse_json_object(&content)?
        } else {
            serde_json::from_str(&content)?
        };

        // Extract plan fields
        let label = parsed
            .get("label")
            .and_then(|v| v.as_str())
            .unwrap_or("automation")
            .to_string();

        let done = parsed.get("done").and_then(|v| v.as_bool()).unwrap_or(false);

        let steps = parsed
            .get("steps")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        // Try to get extracted field, or fallback to the entire response when in extraction mode
        let extracted = parsed.get("extracted").cloned().or_else(|| {
            // If no explicit "extracted" field but response looks like extracted data
            // (has no standard automation fields), use the whole response
            if parsed.get("label").is_none()
                && parsed.get("done").is_none()
                && parsed.get("steps").is_none()
            {
                // Response doesn't have automation structure, treat as direct extraction
                Some(parsed.clone())
            } else if effective_cfg.extra_ai_data {
                // In extraction mode, if response has automation structure but no extracted,
                // check if there's any non-automation data to extract
                let mut extracted_data = serde_json::Map::new();
                if let Some(obj) = parsed.as_object() {
                    for (key, value) in obj {
                        // Skip known automation fields
                        if !matches!(
                            key.as_str(),
                            "label" | "done" | "steps" | "memory_ops" | "extracted"
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
            } else {
                None
            }
        });

        // Parse memory operations
        let memory_ops = parsed
            .get("memory_ops")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|op| serde_json::from_value(op.clone()).ok())
                    .collect()
            })
            .unwrap_or_default();

        Ok(AutomationPlan {
            label,
            done,
            steps,
            extracted,
            memory_ops,
            usage,
        })
    }

    /// Execute automation steps on the page.
    ///
    /// Handles WebAutomation enum-style actions like `{ "Click": "selector" }`.
    async fn execute_steps(
        &self,
        page: &Page,
        steps: &[serde_json::Value],
        _cfg: &RemoteMultimodalConfig,
    ) -> EngineResult<usize> {
        let mut executed = 0;

        for step in steps {
            // Handle WebAutomation enum-style format: { "ActionName": value }
            if let Some(obj) = step.as_object() {
                for (action, value) in obj {
                    let success = self.execute_single_action(page, action, value).await;
                    if success {
                        executed += 1;
                    }
                }
            }
        }

        Ok(executed)
    }

    /// Execute a single WebAutomation action.
    async fn execute_single_action(
        &self,
        page: &Page,
        action: &str,
        value: &serde_json::Value,
    ) -> bool {
        match action {
            // === Click Actions ===
            "Click" => {
                if let Some(selector) = value.as_str() {
                    if let Ok(elem) = page.find_element(selector).await {
                        return elem.click().await.is_ok();
                    }
                }
                false
            }
            "ClickAll" => {
                if let Some(selector) = value.as_str() {
                    if let Ok(elements) = page.find_elements(selector).await {
                        for elem in elements {
                            let _ = elem.click().await;
                        }
                        return true;
                    }
                }
                false
            }
            "ClickPoint" => {
                let x = value.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let y = value.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0);
                // Use CDP mouse events for trusted clicks (important for CAPTCHAs)
                let down = DispatchMouseEventParams::builder()
                    .x(x)
                    .y(y)
                    .r#type(DispatchMouseEventType::MousePressed)
                    .button(MouseButton::Left)
                    .click_count(1)
                    .build();
                let up = DispatchMouseEventParams::builder()
                    .x(x)
                    .y(y)
                    .r#type(DispatchMouseEventType::MouseReleased)
                    .button(MouseButton::Left)
                    .click_count(1)
                    .build();
                if let (Ok(d), Ok(u)) = (down, up) {
                    let _ = page.execute(d).await;
                    let _ = page.execute(u).await;
                }
                true
            }
            "ClickHold" => {
                let selector = value.get("selector").and_then(|v| v.as_str());
                let hold_ms = value.get("hold_ms").and_then(|v| v.as_u64()).unwrap_or(500);
                if let Some(sel) = selector {
                    let _ = page.evaluate(format!(r#"
                        (async () => {{
                            const el = document.querySelector('{}');
                            if (el) {{
                                el.dispatchEvent(new MouseEvent('mousedown', {{bubbles: true}}));
                                await new Promise(r => setTimeout(r, {}));
                                el.dispatchEvent(new MouseEvent('mouseup', {{bubbles: true}}));
                            }}
                        }})()
                    "#, sel, hold_ms)).await;
                    return true;
                }
                false
            }
            "ClickHoldPoint" => {
                let x = value.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let y = value.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let hold_ms = value.get("hold_ms").and_then(|v| v.as_u64()).unwrap_or(500);
                // Use CDP for trusted clicks
                let down = DispatchMouseEventParams::builder()
                    .x(x)
                    .y(y)
                    .r#type(DispatchMouseEventType::MousePressed)
                    .button(MouseButton::Left)
                    .click_count(1)
                    .build();
                let up = DispatchMouseEventParams::builder()
                    .x(x)
                    .y(y)
                    .r#type(DispatchMouseEventType::MouseReleased)
                    .button(MouseButton::Left)
                    .click_count(1)
                    .build();
                if let (Ok(d), Ok(u)) = (down, up) {
                    let _ = page.execute(d).await;
                    tokio::time::sleep(std::time::Duration::from_millis(hold_ms)).await;
                    let _ = page.execute(u).await;
                }
                true
            }
            "DoubleClick" => {
                if let Some(selector) = value.as_str() {
                    let _ = page.evaluate(format!(
                        "document.querySelector('{}')?.dispatchEvent(new MouseEvent('dblclick', {{bubbles: true}}))",
                        selector
                    )).await;
                    return true;
                }
                false
            }
            "DoubleClickPoint" => {
                let x = value.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let y = value.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0);
                // CDP double click with click_count=2
                let down = DispatchMouseEventParams::builder()
                    .x(x)
                    .y(y)
                    .r#type(DispatchMouseEventType::MousePressed)
                    .button(MouseButton::Left)
                    .click_count(2)
                    .build();
                let up = DispatchMouseEventParams::builder()
                    .x(x)
                    .y(y)
                    .r#type(DispatchMouseEventType::MouseReleased)
                    .button(MouseButton::Left)
                    .click_count(2)
                    .build();
                if let (Ok(d), Ok(u)) = (down, up) {
                    let _ = page.execute(d).await;
                    let _ = page.execute(u).await;
                }
                true
            }
            "RightClick" => {
                if let Some(selector) = value.as_str() {
                    let _ = page.evaluate(format!(
                        "document.querySelector('{}')?.dispatchEvent(new MouseEvent('contextmenu', {{bubbles: true}}))",
                        selector
                    )).await;
                    return true;
                }
                false
            }
            "RightClickPoint" => {
                let x = value.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let y = value.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0);
                // CDP right click
                let down = DispatchMouseEventParams::builder()
                    .x(x)
                    .y(y)
                    .r#type(DispatchMouseEventType::MousePressed)
                    .button(MouseButton::Right)
                    .click_count(1)
                    .build();
                let up = DispatchMouseEventParams::builder()
                    .x(x)
                    .y(y)
                    .r#type(DispatchMouseEventType::MouseReleased)
                    .button(MouseButton::Right)
                    .click_count(1)
                    .build();
                if let (Ok(d), Ok(u)) = (down, up) {
                    let _ = page.execute(d).await;
                    let _ = page.execute(u).await;
                }
                true
            }
            "ClickAllClickable" => {
                let _ = page.evaluate(r#"
                    document.querySelectorAll('a, button, [onclick], [role="button"]').forEach(el => el.click())
                "#).await;
                true
            }

            // === Drag Actions ===
            "ClickDrag" => {
                let from = value.get("from").and_then(|v| v.as_str());
                let to = value.get("to").and_then(|v| v.as_str());
                if let (Some(from_sel), Some(to_sel)) = (from, to) {
                    let _ = page.evaluate(format!(r#"
                        (async () => {{
                            const from = document.querySelector('{}');
                            const to = document.querySelector('{}');
                            if (from && to) {{
                                const fromRect = from.getBoundingClientRect();
                                const toRect = to.getBoundingClientRect();
                                from.dispatchEvent(new MouseEvent('mousedown', {{bubbles: true, clientX: fromRect.x + fromRect.width/2, clientY: fromRect.y + fromRect.height/2}}));
                                to.dispatchEvent(new MouseEvent('mousemove', {{bubbles: true, clientX: toRect.x + toRect.width/2, clientY: toRect.y + toRect.height/2}}));
                                to.dispatchEvent(new MouseEvent('mouseup', {{bubbles: true, clientX: toRect.x + toRect.width/2, clientY: toRect.y + toRect.height/2}}));
                            }}
                        }})()
                    "#, from_sel, to_sel)).await;
                    return true;
                }
                false
            }
            "ClickDragPoint" => {
                let from_x = value.get("from_x").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let from_y = value.get("from_y").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let to_x = value.get("to_x").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let to_y = value.get("to_y").and_then(|v| v.as_f64()).unwrap_or(0.0);
                // CDP drag: mouse down at start, move to end, mouse up
                let down = DispatchMouseEventParams::builder()
                    .x(from_x)
                    .y(from_y)
                    .r#type(DispatchMouseEventType::MousePressed)
                    .button(MouseButton::Left)
                    .click_count(1)
                    .build();
                let mv = DispatchMouseEventParams::builder()
                    .x(to_x)
                    .y(to_y)
                    .r#type(DispatchMouseEventType::MouseMoved)
                    .button(MouseButton::Left)
                    .build();
                let up = DispatchMouseEventParams::builder()
                    .x(to_x)
                    .y(to_y)
                    .r#type(DispatchMouseEventType::MouseReleased)
                    .button(MouseButton::Left)
                    .click_count(1)
                    .build();
                if let (Ok(d), Ok(m), Ok(u)) = (down, mv, up) {
                    let _ = page.execute(d).await;
                    let _ = page.execute(m).await;
                    let _ = page.execute(u).await;
                }
                true
            }

            // === Input Actions ===
            "Fill" => {
                let selector = value.get("selector").and_then(|v| v.as_str());
                let text = value.get("value").and_then(|v| v.as_str());
                if let (Some(sel), Some(txt)) = (selector, text) {
                    if let Ok(elem) = page.find_element(sel).await {
                        let _ = elem.click().await;
                        let _ = elem.type_str(txt).await;
                        return true;
                    }
                }
                false
            }
            "Type" => {
                let text = value.get("value").and_then(|v| v.as_str());
                if let Some(txt) = text {
                    // Type into the currently focused element
                    let _ = page.evaluate(format!(
                        "document.activeElement.value += '{}'",
                        txt.replace('\'', "\\'")
                    )).await;
                    return true;
                }
                false
            }
            "Clear" => {
                if let Some(selector) = value.as_str() {
                    let _ = page.evaluate(format!(
                        "document.querySelector('{}').value = ''",
                        selector
                    )).await;
                    return true;
                }
                false
            }
            "Press" => {
                if let Some(key) = value.as_str() {
                    let _ = page.evaluate(format!(r#"
                        document.activeElement.dispatchEvent(new KeyboardEvent('keydown', {{key: '{}', bubbles: true}}));
                        document.activeElement.dispatchEvent(new KeyboardEvent('keypress', {{key: '{}', bubbles: true}}));
                        document.activeElement.dispatchEvent(new KeyboardEvent('keyup', {{key: '{}', bubbles: true}}));
                    "#, key, key, key)).await;
                    return true;
                }
                false
            }
            "KeyDown" => {
                if let Some(key) = value.as_str() {
                    let _ = page.evaluate(format!(
                        "document.activeElement.dispatchEvent(new KeyboardEvent('keydown', {{key: '{}', bubbles: true}}))",
                        key
                    )).await;
                    return true;
                }
                false
            }
            "KeyUp" => {
                if let Some(key) = value.as_str() {
                    let _ = page.evaluate(format!(
                        "document.activeElement.dispatchEvent(new KeyboardEvent('keyup', {{key: '{}', bubbles: true}}))",
                        key
                    )).await;
                    return true;
                }
                false
            }

            // === Scroll Actions ===
            "ScrollX" => {
                let pixels = value.as_i64().unwrap_or(0);
                let _ = page.evaluate(format!("window.scrollBy({}, 0)", pixels)).await;
                true
            }
            "ScrollY" => {
                let pixels = value.as_i64().unwrap_or(300);
                let _ = page.evaluate(format!("window.scrollBy(0, {})", pixels)).await;
                true
            }
            "ScrollTo" => {
                if let Some(selector) = value.get("selector").and_then(|v| v.as_str()) {
                    let _ = page.evaluate(format!(
                        "document.querySelector('{}')?.scrollIntoView({{behavior: 'smooth', block: 'center'}})",
                        selector
                    )).await;
                    return true;
                }
                false
            }
            "ScrollToPoint" => {
                let x = value.get("x").and_then(|v| v.as_i64()).unwrap_or(0);
                let y = value.get("y").and_then(|v| v.as_i64()).unwrap_or(0);
                let _ = page.evaluate(format!("window.scrollTo({}, {})", x, y)).await;
                true
            }
            "InfiniteScroll" => {
                let max_scrolls = value.as_u64().unwrap_or(5);
                for _ in 0..max_scrolls {
                    let _ = page.evaluate("window.scrollTo(0, document.body.scrollHeight)").await;
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                }
                true
            }

            // === Wait Actions ===
            "Wait" => {
                let ms = value.as_u64().unwrap_or(1000);
                tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
                true
            }
            "WaitFor" => {
                if let Some(selector) = value.as_str() {
                    for _ in 0..50 {
                        if page.find_element(selector).await.is_ok() {
                            return true;
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    }
                }
                false
            }
            "WaitForWithTimeout" => {
                let selector = value.get("selector").and_then(|v| v.as_str());
                let timeout = value.get("timeout").and_then(|v| v.as_u64()).unwrap_or(5000);
                if let Some(sel) = selector {
                    let iterations = timeout / 100;
                    for _ in 0..iterations {
                        if page.find_element(sel).await.is_ok() {
                            return true;
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    }
                }
                false
            }
            "WaitForAndClick" => {
                if let Some(selector) = value.as_str() {
                    for _ in 0..50 {
                        if let Ok(elem) = page.find_element(selector).await {
                            return elem.click().await.is_ok();
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    }
                }
                false
            }
            "WaitForNavigation" => {
                // Wait a bit for navigation to complete
                tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
                true
            }
            "WaitForDom" => {
                let selector = value.get("selector").and_then(|v| v.as_str());
                let timeout = value.get("timeout").and_then(|v| v.as_u64()).unwrap_or(5000);
                tokio::time::sleep(std::time::Duration::from_millis(timeout.min(5000))).await;
                if let Some(sel) = selector {
                    return page.find_element(sel).await.is_ok();
                }
                true
            }

            // === Navigation Actions ===
            "Navigate" => {
                if let Some(url) = value.as_str() {
                    return page.goto(url).await.is_ok();
                }
                false
            }
            "GoBack" => {
                let _ = page.evaluate("window.history.back()").await;
                true
            }
            "GoForward" => {
                let _ = page.evaluate("window.history.forward()").await;
                true
            }
            "Reload" => {
                let _ = page.evaluate("window.location.reload()").await;
                true
            }

            // === Hover Actions ===
            "Hover" => {
                if let Some(selector) = value.as_str() {
                    let _ = page.evaluate(format!(
                        "document.querySelector('{}')?.dispatchEvent(new MouseEvent('mouseover', {{bubbles: true}}))",
                        selector
                    )).await;
                    return true;
                }
                false
            }
            "HoverPoint" => {
                let x = value.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let y = value.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let _ = page.evaluate(format!(
                    "document.elementFromPoint({}, {})?.dispatchEvent(new MouseEvent('mouseover', {{bubbles: true}}))",
                    x, y
                )).await;
                true
            }

            // === Select/Focus Actions ===
            "Select" => {
                let selector = value.get("selector").and_then(|v| v.as_str());
                let opt_value = value.get("value").and_then(|v| v.as_str());
                if let (Some(sel), Some(val)) = (selector, opt_value) {
                    let _ = page.evaluate(format!(
                        "document.querySelector('{}').value = '{}'; document.querySelector('{}').dispatchEvent(new Event('change', {{bubbles: true}}))",
                        sel, val, sel
                    )).await;
                    return true;
                }
                false
            }
            "Focus" => {
                if let Some(selector) = value.as_str() {
                    let _ = page.evaluate(format!(
                        "document.querySelector('{}')?.focus()",
                        selector
                    )).await;
                    return true;
                }
                false
            }
            "Blur" => {
                if let Some(selector) = value.as_str() {
                    let _ = page.evaluate(format!(
                        "document.querySelector('{}')?.blur()",
                        selector
                    )).await;
                    return true;
                }
                false
            }

            // === JavaScript ===
            "Evaluate" => {
                if let Some(code) = value.as_str() {
                    let _ = page.evaluate(code).await;
                    return true;
                }
                false
            }

            // === Screenshot ===
            "Screenshot" => {
                // Screenshot is handled separately - just mark as executed
                true
            }

            // === Validation ===
            "ValidateChain" => {
                // Validation step - always succeeds
                true
            }

            _ => {
                log::debug!("Unknown action: {}", action);
                false
            }
        }
    }

    // -----------------------------------------------------------------
    // Single-action APIs (act, observe)
    // -----------------------------------------------------------------

    /// Execute a single action on the page based on a natural language instruction.
    ///
    /// # Arguments
    /// * `page` - The Chrome page to act on
    /// * `instruction` - Natural language instruction (e.g., "Click the login button")
    ///
    /// # Returns
    /// `ActResult` with success status, action description, and optional screenshot.
    pub async fn act(&self, page: &Page, instruction: &str) -> EngineResult<ActResult> {
        use super::{best_effort_parse_json_object, extract_assistant_content, extract_usage};
        use serde::Serialize;

        #[derive(Serialize)]
        struct Message {
            role: String,
            content: serde_json::Value,
        }

        #[derive(Serialize)]
        struct Request {
            model: String,
            messages: Vec<Message>,
            #[serde(skip_serializing_if = "Option::is_none")]
            temperature: Option<f32>,
            #[serde(skip_serializing_if = "Option::is_none")]
            max_tokens: Option<u32>,
            response_format: serde_json::Value,
        }

        // Capture page state
        let screenshot = self
            .screenshot_as_data_url_with_profile(
                page,
                &CaptureProfile {
                    full_page: false,
                    omit_background: true,
                    ..Default::default()
                },
            )
            .await?;

        let html = page
            .content()
            .await
            .map_err(|e| EngineError::Remote(format!("page.content() failed: {e}")))?;
        let cleaned_html = clean_html_with_profile(&html, HtmlCleaningProfile::Default);
        let truncated_html = truncate_utf8_tail(&cleaned_html, self.cfg.html_max_bytes);

        let system_prompt = r#"You are a browser automation agent. Given a screenshot and HTML, execute the user's instruction by returning a JSON object with a single step.

Response format:
{
  "action": "click" | "type" | "scroll" | "wait" | "navigate" | "press" | "select",
  "selector": "CSS selector if needed",
  "text": "text to type if action is type",
  "url": "URL if action is navigate",
  "key": "key name if action is press",
  "value": "value if action is select",
  "x": 0,
  "y": 300,
  "ms": 1000,
  "description": "Brief description of what this action does"
}

Only return the JSON object, no other text."#;

        let user_content = serde_json::json!([
            {
                "type": "text",
                "text": format!("HTML:\n{}\n\nInstruction: {}", truncated_html, instruction)
            },
            {
                "type": "image_url",
                "image_url": { "url": screenshot }
            }
        ]);

        let messages = vec![
            Message {
                role: "system".to_string(),
                content: serde_json::Value::String(system_prompt.to_string()),
            },
            Message {
                role: "user".to_string(),
                content: user_content,
            },
        ];

        let request = Request {
            model: self.model_name.clone(),
            messages,
            temperature: Some(self.cfg.temperature),
            max_tokens: Some(self.cfg.max_tokens as u32),
            response_format: serde_json::json!({ "type": "json_object" }),
        };

        // Acquire semaphore if configured
        let _permit = self.acquire_llm_permit().await;

        static CLIENT: std::sync::LazyLock<reqwest::Client> =
            std::sync::LazyLock::new(|| {
                reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(120))
                    .build()
                    .unwrap_or_else(|_| reqwest::Client::new())
            });

        let mut req = CLIENT.post(&self.api_url).json(&request);
        if let Some(ref key) = self.api_key {
            req = req.bearer_auth(key);
        }

        let resp = req.send().await?;
        let status = resp.status();
        let body: serde_json::Value = resp.json().await?;

        if !status.is_success() {
            return Ok(ActResult::failure(format!(
                "API error {}: {}",
                status,
                serde_json::to_string_pretty(&body).unwrap_or_default()
            )));
        }

        let content = extract_assistant_content(&body)
            .ok_or_else(|| EngineError::MissingField("choices[0].message.content"))?;
        let mut usage = extract_usage(&body);
        usage.increment_llm_calls();

        let step = best_effort_parse_json_object(&content)?;

        // Execute the action
        let steps_executed = self.execute_steps(page, &[step.clone()], &self.cfg).await?;

        let action_type = step.get("action").and_then(|v| v.as_str()).map(String::from);
        let description = step
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("Action executed")
            .to_string();

        // Take screenshot after action
        let after_screenshot = if self.cfg.screenshot {
            self.take_final_screenshot(page).await.ok()
        } else {
            None
        };

        Ok(ActResult {
            success: steps_executed > 0,
            action_taken: description,
            action_type,
            screenshot: after_screenshot,
            error: None,
            usage,
        })
    }

    /// Observe the current page state and return structured information.
    ///
    /// # Arguments
    /// * `page` - The Chrome page to observe
    ///
    /// # Returns
    /// `PageObservation` with interactive elements, forms, navigation options, etc.
    pub async fn observe(&self, page: &Page) -> EngineResult<PageObservation> {
        use super::{best_effort_parse_json_object, extract_assistant_content, extract_usage};
        use serde::Serialize;

        #[derive(Serialize)]
        struct Message {
            role: String,
            content: serde_json::Value,
        }

        #[derive(Serialize)]
        struct Request {
            model: String,
            messages: Vec<Message>,
            #[serde(skip_serializing_if = "Option::is_none")]
            temperature: Option<f32>,
            #[serde(skip_serializing_if = "Option::is_none")]
            max_tokens: Option<u32>,
            response_format: serde_json::Value,
        }

        // Capture page state
        let screenshot = self
            .screenshot_as_data_url_with_profile(
                page,
                &CaptureProfile {
                    full_page: true,
                    omit_background: true,
                    ..Default::default()
                },
            )
            .await?;

        let url = page.url().await.ok().flatten().unwrap_or_default();
        let title = page.get_title().await.ok().flatten().unwrap_or_default();

        let html = page
            .content()
            .await
            .map_err(|e| EngineError::Remote(format!("page.content() failed: {e}")))?;
        let cleaned_html = clean_html_with_profile(&html, HtmlCleaningProfile::Default);
        let truncated_html = truncate_utf8_tail(&cleaned_html, self.cfg.html_max_bytes);

        let system_prompt = r#"You are a page analysis agent. Analyze the screenshot and HTML to describe the page state.

Return a JSON object:
{
  "description": "Brief description of the page",
  "page_type": "login|search|product|list|article|form|dashboard|other",
  "interactive_elements": [
    {
      "selector": "CSS selector",
      "element_type": "button|link|input|select|textarea|checkbox|radio",
      "text": "visible text",
      "description": "what this element does",
      "visible": true,
      "enabled": true
    }
  ],
  "forms": [
    {
      "selector": "form CSS selector",
      "description": "what this form does",
      "fields": [
        { "name": "field name", "field_type": "text|email|password|etc", "label": "label text", "required": true }
      ]
    }
  ],
  "navigation": [
    { "text": "link text", "url": "href", "selector": "CSS selector" }
  ],
  "suggested_actions": ["action 1", "action 2"]
}

Only return the JSON object."#;

        let user_content = serde_json::json!([
            {
                "type": "text",
                "text": format!("URL: {}\nTitle: {}\n\nHTML:\n{}", url, title, truncated_html)
            },
            {
                "type": "image_url",
                "image_url": { "url": screenshot }
            }
        ]);

        let messages = vec![
            Message {
                role: "system".to_string(),
                content: serde_json::Value::String(system_prompt.to_string()),
            },
            Message {
                role: "user".to_string(),
                content: user_content,
            },
        ];

        let request = Request {
            model: self.model_name.clone(),
            messages,
            temperature: Some(self.cfg.temperature),
            max_tokens: Some(self.cfg.max_tokens as u32),
            response_format: serde_json::json!({ "type": "json_object" }),
        };

        // Acquire semaphore if configured
        let _permit = self.acquire_llm_permit().await;

        static CLIENT: std::sync::LazyLock<reqwest::Client> =
            std::sync::LazyLock::new(|| {
                reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(120))
                    .build()
                    .unwrap_or_else(|_| reqwest::Client::new())
            });

        let mut req = CLIENT.post(&self.api_url).json(&request);
        if let Some(ref key) = self.api_key {
            req = req.bearer_auth(key);
        }

        let resp = req.send().await?;
        let status = resp.status();
        let body: serde_json::Value = resp.json().await?;

        if !status.is_success() {
            return Err(EngineError::Remote(format!(
                "API error {}: {}",
                status,
                serde_json::to_string_pretty(&body).unwrap_or_default()
            )));
        }

        let content = extract_assistant_content(&body)
            .ok_or_else(|| EngineError::MissingField("choices[0].message.content"))?;
        let mut usage = extract_usage(&body);
        usage.increment_llm_calls();

        let parsed = best_effort_parse_json_object(&content)?;

        // Build PageObservation from parsed JSON
        let mut obs = PageObservation::new(&url).with_title(&title).with_usage(usage);

        if let Some(desc) = parsed.get("description").and_then(|v| v.as_str()) {
            obs = obs.with_description(desc);
        }

        if let Some(pt) = parsed.get("page_type").and_then(|v| v.as_str()) {
            obs = obs.with_page_type(pt);
        }

        // Parse interactive elements
        if let Some(elements) = parsed.get("interactive_elements").and_then(|v| v.as_array()) {
            for elem in elements {
                if let Ok(ie) = serde_json::from_value(elem.clone()) {
                    obs.interactive_elements.push(ie);
                }
            }
        }

        // Parse forms
        if let Some(forms) = parsed.get("forms").and_then(|v| v.as_array()) {
            for form in forms {
                if let Ok(fi) = serde_json::from_value(form.clone()) {
                    obs.forms.push(fi);
                }
            }
        }

        // Parse navigation
        if let Some(navs) = parsed.get("navigation").and_then(|v| v.as_array()) {
            for nav in navs {
                if let Ok(no) = serde_json::from_value(nav.clone()) {
                    obs.navigation.push(no);
                }
            }
        }

        // Parse suggested actions
        if let Some(actions) = parsed.get("suggested_actions").and_then(|v| v.as_array()) {
            for action in actions {
                if let Some(s) = action.as_str() {
                    obs.suggested_actions.push(s.to_string());
                }
            }
        }

        if self.cfg.screenshot {
            obs = obs.with_screenshot(screenshot);
        }

        Ok(obs)
    }
}

/// Run remote multi-modal automation with a browser page.
///
/// This is a convenience function that creates an engine from the configs
/// and runs automation on the page.
#[cfg(feature = "chrome")]
pub async fn run_remote_multimodal_with_page(
    cfgs: &super::RemoteMultimodalConfigs,
    page: &Page,
    url: &str,
) -> EngineResult<AutomationResult> {
    let sem = cfgs.get_or_init_semaphore();
    let mut engine = RemoteMultimodalEngine::new(
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

    engine.run(page, url).await
}
