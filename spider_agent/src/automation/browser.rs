//! Browser-specific automation methods for chromiumoxide integration.
//!
//! This module provides browser automation capabilities that require a
//! Chrome browser page. All methods are gated behind `#[cfg(feature = "chrome")]`.

#[cfg(feature = "chrome")]
use base64::{engine::general_purpose, Engine as _};
#[cfg(feature = "chrome")]
use chromiumoxide::{
    cdp::browser_protocol::page::CaptureScreenshotFormat, layout::Point, page::ScreenshotParams,
    Page,
};

use super::{
    clean_html_with_profile, parse_tool_calls, tool_calls_to_steps, truncate_utf8_tail, ActResult,
    ActionToolSchemas, AutomationMemory, AutomationResult, AutomationUsage, CaptureProfile,
    EngineError, EngineResult, HtmlCleaningProfile, MemoryOperation, PageObservation,
    RemoteMultimodalConfig, RemoteMultimodalEngine,
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
    pub relevant: Option<bool>,
    pub reasoning: Option<String>,
}

#[cfg(feature = "chrome")]
impl RemoteMultimodalEngine {
    // -----------------------------------------------------------------
    // Capture helpers
    // -----------------------------------------------------------------

    /// Whether this round should include screenshot capture/input.
    ///
    /// This enforces text-only behavior when the resolved round model does not
    /// support vision, while still honoring explicit `include_screenshot` overrides.
    #[inline]
    fn should_include_screenshot_for_round(
        &self,
        effective_cfg: &RemoteMultimodalConfig,
        use_vision: bool,
    ) -> bool {
        if !use_vision {
            return false;
        }
        match effective_cfg.include_screenshot {
            Some(explicit) => explicit,
            None => {
                let (_, model_name, _) = self.resolve_model_for_round(use_vision);
                super::supports_vision(model_name)
            }
        }
    }

    /// Whether the resolved "vision route" model can actually process image input.
    #[inline]
    fn has_vision_capable_route(&self) -> bool {
        let (_, model_name, _) = self.resolve_model_for_round(true);
        super::supports_vision(model_name)
    }

    /// Apply a text-only flavor to the system prompt for rounds where no image
    /// is provided. This keeps action bindings intact while removing screenshot
    /// expectations that can mislead text-only models.
    #[inline]
    fn apply_text_only_prompt_flavor(system_msg: &mut String, can_request_vision: bool) {
        const SCREENSHOT_INPUT_LINE: &str =
            "- Screenshot of current page state (may be omitted in text-only rounds)\n";
        const TEXT_ONLY_INPUT_LINE: &str =
            "- No screenshot is provided this round; use URL/title/HTML context.\n";

        if system_msg.contains(SCREENSHOT_INPUT_LINE) {
            *system_msg = system_msg.replacen(SCREENSHOT_INPUT_LINE, TEXT_ONLY_INPUT_LINE, 1);
        }

        if system_msg.contains("→ visible in screenshot") {
            *system_msg =
                system_msg.replace("→ visible in screenshot", "→ visible in next round context");
        }

        if can_request_vision {
            system_msg.push_str("\n\n---\nMODE: TEXT-ONLY (no screenshot this round). Use HTML context and memory to decide actions. If you need visual information, set `{\"op\":\"set\",\"key\":\"request_vision\",\"value\":true}` in memory_ops to receive a screenshot next round.\n");
        } else {
            system_msg.push_str(
                "\n\n---\nMODE: TEXT-ONLY (no screenshot available for this model/config). Use HTML context and memory only.\n",
            );
        }
    }

    /// Capture screenshot as data URL with profile settings.
    /// Automatically applies grayscale filter for text CAPTCHAs to improve readability.
    pub(crate) async fn screenshot_as_data_url_with_profile(
        &self,
        page: &Page,
        cap: &CaptureProfile,
    ) -> EngineResult<String> {
        // Auto-detect text CAPTCHA and apply grayscale for better readability
        // Only apply if input is empty (first view) - don't grayscale after typing
        let _ = page
            .evaluate(
                r#"
            (() => {
                const text = (document.body?.innerText || '').toLowerCase();
                const input = document.querySelector('input[type="text"], input:not([type])');
                const inputEmpty = input && !input.value;
                const hasTextCaptcha = inputEmpty && (
                    text.includes('enter the text') ||
                    text.includes('enter the') ||
                    text.includes('type the') ||
                    text.includes('wiggles') ||
                    text.includes('level 3') ||
                    text.includes('below')
                );
                if (hasTextCaptcha) {
                    // Grayscale + high contrast to make text clearer
                    const filter = 'grayscale(100%) contrast(150%)';
                    document.documentElement.style.filter = filter;
                    document.querySelectorAll('canvas, img, svg, div').forEach(el => {
                        el.style.filter = filter;
                    });
                }
            })()
        "#,
            )
            .await;

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
        let _ = page
            .evaluate(
                r#"
            document.documentElement.style.filter = '';
            document.body.style.filter = '';
            document.querySelectorAll('canvas, img, svg, div').forEach(el => {
                el.style.filter = '';
            });
        "#,
            )
            .await;

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
    #[allow(clippy::too_many_arguments)]
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
        action_stuck_rounds: usize,
        loop_blocklist: &[String],
        memory: Option<&AutomationMemory>,
        user_message_extra: Option<&str>,
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

        // Include user instructions if provided
        if let Some(extra) = user_message_extra {
            if !extra.trim().is_empty() {
                out.push_str("---\nUSER INSTRUCTIONS:\n");
                out.push_str(extra.trim());
                out.push_str("\n\n");
            }
        }

        // Stuck-loop warning: inject when model repeats identical actions
        if action_stuck_rounds >= 3 {
            out.push_str("--- ACTION LOOP DETECTED ---\n");
            out.push_str(&format!(
                "You have repeated the EXACT same actions {} consecutive rounds with ZERO progress.\n",
                action_stuck_rounds
            ));
            out.push_str("Your current approach is NOT WORKING. You MUST change strategy NOW:\n");
            out.push_str("1. Use Evaluate to inspect DOM state - read element text, check CSS classes, find the actual button/element state\n");
            out.push_str("2. Try completely different selectors or use ClickPoint at exact pixel coordinates\n");
            out.push_str("3. If clicking a button repeatedly does nothing, the task may have FAILED - look for error messages, try refreshing the page\n");
            out.push_str("4. If you think you won/completed something but Verify does not advance, you likely LOST or the answer was wrong - retry\n");
            out.push_str("5. Do NOT repeat the same steps again. Your next response MUST contain different actions.\n\n");
        }

        if !loop_blocklist.is_empty() {
            out.push_str("LOOP BLOCKLIST (DO NOT REPEAT THESE EXACT ACTIONS):\n");
            for blocked in loop_blocklist.iter().take(10) {
                out.push_str("- ");
                out.push_str(blocked);
                out.push('\n');
            }
            out.push_str(
                "Use different selectors/coordinates, or switch interaction method entirely.\n\n",
            );
        }

        if effective_cfg.is_extraction_only() {
            out.push_str(
                "TASK:\nExtract structured data from the page above. Return JSON with label, done: true, steps: [], and extracted data.\n",
            );
        } else {
            out.push_str(
                "TASK:\nReturn the next automation steps as a single JSON object (no prose).\n",
            );
        }

        out
    }

    /// Build a compact blocklist summary for repeated action plans.
    ///
    /// This is fed back into the next prompt to discourage repeating exactly
    /// the same actions after loop detection.
    fn summarize_step_blocklist(steps: &[serde_json::Value], max_items: usize) -> Vec<String> {
        use std::collections::HashSet;

        let mut seen = HashSet::new();
        let mut out = Vec::new();

        for step in steps {
            let Some(obj) = step.as_object() else {
                continue;
            };

            for (action, value) in obj {
                let mut summary = String::from(action);

                if let Some(selector) = value.as_str() {
                    summary.push_str(": selector=");
                    summary.push_str(selector);
                } else if let Some(selector) = value.get("selector").and_then(|v| v.as_str()) {
                    summary.push_str(": selector=");
                    summary.push_str(selector);
                } else if let (Some(x), Some(y)) = (
                    value.get("x").and_then(|v| v.as_f64()),
                    value.get("y").and_then(|v| v.as_f64()),
                ) {
                    summary.push_str(": point=(");
                    summary.push_str(&format!("{x:.1}, {y:.1}"));
                    summary.push(')');
                } else {
                    let rendered = truncate_utf8_tail(&value.to_string(), 100);
                    if !rendered.is_empty() {
                        summary.push_str(": ");
                        summary.push_str(&rendered);
                    }
                }

                if seen.insert(summary.clone()) {
                    out.push(summary);
                    if out.len() >= max_items {
                        return out;
                    }
                }
            }
        }

        out
    }

    /// Extract a stable level key from model `extracted` payload.
    ///
    /// Expected fields (when present): `current_level` and `level_name`.
    fn extracted_level_key(extracted: Option<&serde_json::Value>) -> Option<String> {
        let extracted = extracted?.as_object()?;
        let level_num = extracted.get("current_level").and_then(|v| v.as_u64());
        let level_name = extracted
            .get("level_name")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());

        if level_num.is_none() && level_name.is_none() {
            return None;
        }

        let mut key = String::new();
        if let Some(n) = level_num {
            key.push('L');
            key.push_str(&n.to_string());
        } else {
            key.push_str("L?");
        }

        if let Some(name) = level_name {
            let normalized = name
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() {
                        c.to_ascii_lowercase()
                    } else {
                        '-'
                    }
                })
                .collect::<String>();
            key.push(':');
            key.push_str(normalized.trim_matches('-'));
        }

        Some(key)
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
        let mut effective_cfg: RemoteMultimodalConfig = self.cfg.clone();
        let mut effective_system_prompt = self.system_prompt.clone();
        let mut effective_system_prompt_extra = self.system_prompt_extra.clone();
        let mut effective_user_message_extra = self.user_message_extra.clone();

        // 0) URL gating check + per-URL override application
        if let Some(gate) = &self.prompt_url_gate {
            let gate_match = gate.match_url(url_input);
            if gate_match.is_none() {
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
            }

            if let Some(Some(override_cfg)) = gate_match {
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

        // Extraction-only optimization: skip screenshots unless explicitly requested.
        // Saves ~35k tokens per call for vision-capable models doing text extraction.
        let extraction_only = effective_cfg.is_extraction_only();
        let skip_screenshot_for_extraction =
            extraction_only && effective_cfg.include_screenshot != Some(true);

        // capture profiles fallback
        let capture_profiles: Vec<CaptureProfile> = if effective_cfg.capture_profiles.is_empty() {
            vec![
                CaptureProfile {
                    full_page: false,
                    omit_background: true,
                    html_cleaning: HtmlCleaningProfile::Default,
                    html_max_bytes: effective_cfg.html_max_bytes,
                    attempt_note: Some("default profile 1: viewport screenshot".into()),
                    ..Default::default()
                },
                CaptureProfile {
                    full_page: true,
                    omit_background: true,
                    html_cleaning: HtmlCleaningProfile::Aggressive,
                    html_max_bytes: effective_cfg.html_max_bytes,
                    attempt_note: Some(
                        "default profile 2: full-page screenshot + aggressive HTML".into(),
                    ),
                    ..Default::default()
                },
            ]
        } else {
            effective_cfg.capture_profiles.clone()
        };

        let mut total_steps_executed = 0usize;
        let mut last_label = String::from("automation");
        let mut last_sig: Option<StateSignature> = None;
        let mut total_usage = AutomationUsage::default();
        let mut last_extracted: Option<serde_json::Value> = None;
        let mut last_relevant: Option<bool> = None;
        let mut last_reasoning: Option<String> = None;
        let mut all_spawn_pages: Vec<String> = Vec::new();
        // Stuck-loop detection: track hashed step sequences across rounds
        let mut recent_step_hashes: std::collections::VecDeque<u64> =
            std::collections::VecDeque::new();
        let mut action_stuck_rounds: usize = 0;
        // Compact summary of repeated actions to block in the next round.
        let mut loop_blocklist: Vec<String> = Vec::new();
        // Dual-model routing: set by `request_vision` memory_op to force vision next round
        let mut force_vision_next_round: bool = false;

        // Recall learned strategies from long-term experience memory.
        // Uses spawn_blocking to contain !Send ffmpeg temporaries from memvid-rs.
        #[cfg(feature = "memvid")]
        if let Some(ref exp_mem) = self.experience_memory {
            let exp_mem = exp_mem.clone();
            let query = format!("{} {}", url_input, last_label);
            let handle = tokio::runtime::Handle::current();
            let recalled = tokio::task::spawn_blocking(move || {
                handle.block_on(async move {
                    {
                        let mem = exp_mem.read().await;
                        mem.clear_cache();
                    }
                    let mut mem = exp_mem.write().await;
                    let max_recall = mem.config.max_recall;
                    let max_context_chars = mem.config.max_context_chars;
                    match mem.recall(&query, max_recall).await {
                        Ok(experiences) if !experiences.is_empty() => {
                            let ctx = super::long_term_memory::ExperienceMemory::recall_to_context(
                                &experiences,
                                max_context_chars,
                            );
                            if ctx.is_empty() {
                                None
                            } else {
                                log::info!(
                                    "Recalled {} strategies ({} chars)",
                                    experiences.len(),
                                    ctx.len(),
                                );
                                Some(ctx)
                            }
                        }
                        Ok(_) => None,
                        Err(e) => {
                            log::warn!("Failed to recall experiences: {}", e);
                            None
                        }
                    }
                })
            })
            .await
            .ok()
            .flatten();

            if let Some(learned_context) = recalled {
                let existing = effective_system_prompt_extra.take().unwrap_or_default();
                effective_system_prompt_extra = Some(if existing.is_empty() {
                    learned_context
                } else {
                    format!("{}\n\n{}", learned_context, existing)
                });
            }
        }

        // Chrome AI warm-up (when enabled or as last-resort fallback)
        let use_chrome_ai = self.should_use_chrome_ai();
        if use_chrome_ai {
            log::info!("Chrome AI mode: using in-page LanguageModel for inference");
            if let Err(e) = Self::warm_chrome_ai(page).await {
                log::warn!("Chrome AI warm-up failed: {e}");
            }
        }

        let rounds = effective_cfg.max_rounds.max(1);
        for round_idx in 0..rounds {
            let mut current_level_attempts: Option<u32> = None;
            // pick capture profile by round (clamp to last)
            let cap = capture_profiles
                .get(round_idx)
                .unwrap_or_else(|| capture_profiles.last().expect("non-empty capture_profiles"));

            // Dual-model routing decision (before capture)
            let use_vision = self.should_use_vision_this_round(
                round_idx,
                last_sig.as_ref().map(|_| false).unwrap_or(false), // stagnation not yet known; re-checked below
                action_stuck_rounds,
                force_vision_next_round,
            );
            // Reset per-round force flag
            force_vision_next_round = false;

            // Capture state – skip screenshot when this round is text-only
            let include_screenshot_this_round = use_vision
                && !skip_screenshot_for_extraction
                && self.should_include_screenshot_for_round(&effective_cfg, use_vision);

            let html_fut = self.html_context_with_profile(page, &effective_cfg, cap);
            let url_fut =
                async { Ok::<String, EngineError>(self.url_context(page, &effective_cfg).await) };
            let title_fut =
                async { Ok::<String, EngineError>(self.title_context(page, &effective_cfg).await) };

            #[allow(unused_mut)]
            let (screenshot, html, mut url_now, mut title_now) = if include_screenshot_this_round {
                let screenshot_fut = self.screenshot_as_data_url_with_profile(page, cap);
                // Screenshot failures are non-fatal — continue without an image.
                let (s_res, h, u, t) =
                    tokio::join!(screenshot_fut, html_fut, url_fut, title_fut);
                let h = h?;
                let u = u?;
                let t = t?;
                let s = match s_res {
                    Ok(s) => s,
                    Err(e) => {
                        log::warn!("Screenshot failed (non-fatal), continuing text-only: {e}");
                        String::new()
                    }
                };
                (s, h, u, t)
            } else {
                let (h, u, t) = tokio::try_join!(html_fut, url_fut, title_fut)?;
                (String::new(), h, u, t)
            };

            // Fallback to the input URL if page.url() is empty/unsupported.
            if url_now.is_empty() {
                url_now = url_input.to_string();
            }

            // quick stagnation heuristic
            let sig = StateSignature::new(&url_now, &title_now, &html);
            let stagnated = last_sig.as_ref().map(|p| p.eq_soft(&sig)).unwrap_or(false);
            last_sig = Some(sig);

            // Run pre_evaluate JS from matching skills BEFORE the LLM sees the page.
            // This lets the engine execute critical JS (board solvers, grid extractors)
            // so the model only reads results from document.title and emits click actions.
            #[cfg(feature = "skills")]
            {
                if let Some(ref registry) = self.skill_registry {
                    let pre_evals = registry.find_pre_evaluates(&url_now, &title_now, &html);
                    if !pre_evals.is_empty() {
                        for (skill_name, js) in &pre_evals {
                            log::info!(
                                "Running pre_evaluate for skill '{}' ({} bytes)",
                                skill_name,
                                js.len()
                            );
                            let _ = page.evaluate(*js).await;
                        }
                        // Re-capture title after pre_evaluate (JS sets document.title)
                        let new_title = self.title_context(page, &effective_cfg).await;
                        if !new_title.is_empty() && new_title != title_now {
                            log::info!(
                                "Pre-evaluate updated title: '{}' -> '{}'",
                                &title_now[..title_now.len().min(80)],
                                &new_title[..new_title.len().min(80)]
                            );
                            title_now = new_title.clone();

                            // TTT game loop: play entire game(s) before the LLM sees
                            // the page. Handles draws by waiting for auto-reset and
                            // replaying. Up to 3 games within one LLM round.
                            if new_title.starts_with("TTT:") {
                                let ttt_js: Option<&str> = pre_evals.iter()
                                    .find(|(name, _)| *name == "tic-tac-toe")
                                    .map(|(_, js)| *js);
                                if let Some(ttt_js) = ttt_js {
                                    let mut games_played = 0u32;
                                    for _ttt_step in 0..30 {
                                        // Parse current game state
                                        let (my_win, th_win, full) = if let Some(json_str) = title_now.strip_prefix("TTT:") {
                                            if let Ok(data) = serde_json::from_str::<serde_json::Value>(json_str) {
                                                (
                                                    data.get("myWin").and_then(|v| v.as_bool()).unwrap_or(false),
                                                    data.get("thWin").and_then(|v| v.as_bool()).unwrap_or(false),
                                                    data.get("full").and_then(|v| v.as_bool()).unwrap_or(false),
                                                )
                                            } else { (false, false, false) }
                                        } else { break; }; // title changed away from TTT

                                        if my_win || th_win {
                                            log::info!("TTT game over: myWin={}, thWin={}", my_win, th_win);
                                            if my_win {
                                                // Engine clicks verify immediately — don't rely
                                                // on the model since the game auto-resets quickly.
                                                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                                                let _ = page.click(chromiumoxide::layout::Point::new(0.0, 0.0)).await; // defocus
                                                let verify_js = r#"(function(){
                                                    var btn=document.querySelector('#captcha-verify-button,button.captcha-verify,.verify-button,[class*=verify]');
                                                    if(btn){var r=btn.getBoundingClientRect();
                                                    var ev={bubbles:true,cancelable:true,clientX:r.x+r.width/2,clientY:r.y+r.height/2};
                                                    btn.dispatchEvent(new PointerEvent('pointerdown',ev));
                                                    btn.dispatchEvent(new MouseEvent('mousedown',ev));
                                                    btn.dispatchEvent(new PointerEvent('pointerup',ev));
                                                    btn.dispatchEvent(new MouseEvent('mouseup',ev));
                                                    btn.dispatchEvent(new MouseEvent('click',ev));
                                                    return 'clicked';}return 'not_found';})()
                                                "#;
                                                let _ = page.evaluate(verify_js).await;
                                                log::info!("TTT: engine clicked verify after win");
                                                tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
                                                // Update title for model context
                                                let post = self.title_context(page, &effective_cfg).await;
                                                if !post.is_empty() { title_now = post; }
                                            }
                                            break;
                                        }
                                        if full {
                                            // Draw — click refresh to reset, then play again
                                            games_played += 1;
                                            if games_played >= 3 { break; }
                                            log::info!("TTT draw (game {}), clicking refresh...", games_played);
                                            let refresh_js = r#"(function(){var btn=document.querySelector('.captcha-refresh,[class*=refresh],[class*=retry]');if(btn){btn.click();return 'clicked';}return 'not_found';})()"#;
                                            let _ = page.evaluate(refresh_js).await;
                                            tokio::time::sleep(std::time::Duration::from_millis(2000)).await;
                                        } else {
                                            // Wait for AI response
                                            tokio::time::sleep(std::time::Duration::from_millis(700)).await;
                                        }
                                        // Re-run pre_evaluate for next move
                                        let _ = page.evaluate(ttt_js).await;
                                        let updated = self.title_context(page, &effective_cfg).await;
                                        if !updated.is_empty() {
                                            log::info!(
                                                "TTT loop (game {}): '{}'",
                                                games_played + 1, &updated[..updated.len().min(80)]
                                            );
                                            title_now = updated;
                                        }
                                    }
                                }
                            }

                            // WAM: Whack-a-Mole engine loop — repeatedly detect and click moles
                            // since they appear/disappear too fast for LLM round-trips.
                            if new_title.starts_with("WAM:") {
                                let wam_js: Option<&str> = pre_evals.iter()
                                    .find(|(name, _)| *name == "whack-a-mole")
                                    .map(|(_, js)| *js);
                                if let Some(wam_js) = wam_js {
                                    let mut total_hits = 0u32;
                                    // Run up to 20 iterations over ~10 seconds
                                    for _wam_step in 0..20 {
                                        if let Some(json_str) = title_now.strip_prefix("WAM:") {
                                            if let Ok(data) = serde_json::from_str::<serde_json::Value>(json_str) {
                                                let clicked = data.get("clicked").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                                                total_hits += clicked;
                                            }
                                        }
                                        if total_hits >= 5 { break; }
                                        // Wait for new moles to appear
                                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                                        // Re-run pre_evaluate to detect and click new moles
                                        let _ = page.evaluate(wam_js).await;
                                        let updated = self.title_context(page, &effective_cfg).await;
                                        if !updated.is_empty() {
                                            title_now = updated;
                                        }
                                    }
                                    if total_hits >= 5 {
                                        log::info!("WAM: engine hit {} moles, clicking verify", total_hits);
                                        // Mark done to prevent re-running
                                        let done_title = format!("WAM_DONE:{{\"hits\":{}}}", total_hits);
                                        let marker_js = format!(
                                            "document.title={t};if(!document.getElementById('wam-engine-done')){{var d=document.createElement('div');d.id='wam-engine-done';d.style.display='none';d.dataset.t={t};document.body.appendChild(d);}}",
                                            t = serde_json::to_string(&done_title).unwrap_or_default()
                                        );
                                        let _ = page.evaluate(marker_js).await;
                                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                                        let verify_js = r#"(function(){
                                            var btn=document.querySelector('#captcha-verify-button,button.captcha-verify,.verify-button,[class*=verify]');
                                            if(btn){var r=btn.getBoundingClientRect();
                                            var ev={bubbles:true,cancelable:true,clientX:r.x+r.width/2,clientY:r.y+r.height/2};
                                            btn.dispatchEvent(new PointerEvent('pointerdown',ev));
                                            btn.dispatchEvent(new MouseEvent('mousedown',ev));
                                            btn.dispatchEvent(new PointerEvent('pointerup',ev));
                                            btn.dispatchEvent(new MouseEvent('mouseup',ev));
                                            btn.dispatchEvent(new MouseEvent('click',ev));
                                            return 'clicked';}return 'not_found';})()
                                        "#;
                                        let _ = page.evaluate(verify_js).await;
                                        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
                                        let post = self.title_context(page, &effective_cfg).await;
                                        if !post.is_empty() { title_now = post; }
                                    }
                                }
                            }

                            // NEST: Nested grid engine loop — detect stop sign overlap
                            // and click boxes automatically, wait for subdivision, repeat.
                            if new_title.starts_with("NEST:") {
                                let nest_js: Option<&str> = pre_evals.iter()
                                    .find(|(name, _)| *name == "nested-grid")
                                    .map(|(_, js)| *js);
                                if let Some(nest_js) = nest_js {
                                    let mut total_clicked = 0u32;
                                    // Up to 8 subdivision rounds
                                    for _nest_step in 0..8 {
                                        if let Some(json_str) = title_now.strip_prefix("NEST:") {
                                            if let Ok(data) = serde_json::from_str::<serde_json::Value>(json_str) {
                                                let has_sign = data.get("hasSign").and_then(|v| v.as_bool()).unwrap_or(false);
                                                let to_click = data.get("toClick").and_then(|v| v.as_array());
                                                let boxes = data.get("boxes").and_then(|v| v.as_array());
                                                if !has_sign { break; } // can't detect sign, let LLM handle
                                                if let (Some(to_click), Some(boxes)) = (to_click, boxes) {
                                                    if to_click.is_empty() { break; } // all done
                                                    // Build id→center map
                                                    let box_map: std::collections::HashMap<i64, (f64, f64)> = boxes.iter().filter_map(|b| {
                                                        let id = b.get("id").and_then(|v| v.as_i64())?;
                                                        let x = b.get("x").and_then(|v| v.as_f64())?;
                                                        let y = b.get("y").and_then(|v| v.as_f64())?;
                                                        Some((id, (x, y)))
                                                    }).collect();
                                                    use chromiumoxide::cdp::browser_protocol::input::{
                                                        DispatchMouseEventParams, DispatchMouseEventType, MouseButton,
                                                    };
                                                    let mut clicked_this_round = 0u32;
                                                    for tc_id in to_click {
                                                        let id = tc_id.as_i64().unwrap_or(-1);
                                                        if let Some((x, y)) = box_map.get(&id) {
                                                            // CDP click: move → press → release
                                                            if let Ok(cmd) = DispatchMouseEventParams::builder()
                                                                .x(*x).y(*y)
                                                                .button(MouseButton::None).buttons(0)
                                                                .r#type(DispatchMouseEventType::MouseMoved).build() {
                                                                let _ = page.send_command(cmd).await;
                                                            }
                                                            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
                                                            if let Ok(cmd) = DispatchMouseEventParams::builder()
                                                                .x(*x).y(*y)
                                                                .button(MouseButton::Left).buttons(1).click_count(1)
                                                                .r#type(DispatchMouseEventType::MousePressed).build() {
                                                                let _ = page.send_command(cmd).await;
                                                            }
                                                            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
                                                            if let Ok(cmd) = DispatchMouseEventParams::builder()
                                                                .x(*x).y(*y)
                                                                .button(MouseButton::Left).buttons(0).click_count(1)
                                                                .r#type(DispatchMouseEventType::MouseReleased).build() {
                                                                let _ = page.send_command(cmd).await;
                                                            }
                                                            tokio::time::sleep(std::time::Duration::from_millis(80)).await;
                                                            clicked_this_round += 1;
                                                        }
                                                    }
                                                    total_clicked += clicked_this_round;
                                                    log::info!("NEST: clicked {} boxes (total {})", clicked_this_round, total_clicked);
                                                    // Wait for subdivision animation
                                                    tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
                                                    // Re-run pre_evaluate to detect new leaf boxes
                                                    let _ = page.evaluate(nest_js).await;
                                                    let updated = self.title_context(page, &effective_cfg).await;
                                                    if !updated.is_empty() {
                                                        title_now = updated;
                                                    }
                                                } else { break; }
                                            } else { break; }
                                        } else { break; }
                                    }
                                    if total_clicked > 0 {
                                        log::info!("NEST: engine auto-clicked {} total boxes, clicking verify", total_clicked);
                                        let done_title = format!("NEST_DONE:{{\"clicked\":{}}}", total_clicked);
                                        let marker_js = format!(
                                            "document.title={t};if(!document.getElementById('nest-engine-done')){{var d=document.createElement('div');d.id='nest-engine-done';d.style.display='none';d.dataset.t={t};document.body.appendChild(d);}}",
                                            t = serde_json::to_string(&done_title).unwrap_or_default()
                                        );
                                        let _ = page.evaluate(marker_js).await;
                                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                                        let verify_js = r#"(function(){
                                            var btn=document.querySelector('#captcha-verify-button,button.captcha-verify,.verify-button,[class*=verify]');
                                            if(btn){var r=btn.getBoundingClientRect();
                                            var ev={bubbles:true,cancelable:true,clientX:r.x+r.width/2,clientY:r.y+r.height/2};
                                            btn.dispatchEvent(new PointerEvent('pointerdown',ev));
                                            btn.dispatchEvent(new MouseEvent('mousedown',ev));
                                            btn.dispatchEvent(new PointerEvent('pointerup',ev));
                                            btn.dispatchEvent(new MouseEvent('mouseup',ev));
                                            btn.dispatchEvent(new MouseEvent('click',ev));
                                            return 'clicked';}return 'not_found';})()
                                        "#;
                                        let _ = page.evaluate(verify_js).await;
                                        log::info!("NEST: engine clicked verify");
                                        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
                                        let post = self.title_context(page, &effective_cfg).await;
                                        if !post.is_empty() { title_now = post; }
                                    }
                                }
                            }

                            // CIRCLE: engine draws a circle via CDP mouse drag through
                            // pre-computed points, then clicks verify.
                            if new_title.starts_with("CIRCLE:") {
                                if let Some(json_str) = new_title.strip_prefix("CIRCLE:") {
                                    if let Ok(data) = serde_json::from_str::<serde_json::Value>(json_str) {
                                        let pts = data.get("pts").and_then(|v| v.as_array());
                                        if let Some(pts) = pts {
                                            let coords: Vec<(f64, f64)> = pts.iter().filter_map(|p| {
                                                let x = p.get("x").and_then(|v| v.as_f64())?;
                                                let y = p.get("y").and_then(|v| v.as_f64())?;
                                                Some((x, y))
                                            }).collect();
                                            if coords.len() >= 2 {
                                                use chromiumoxide::cdp::browser_protocol::input::{
                                                    DispatchMouseEventParams, DispatchMouseEventType, MouseButton,
                                                };
                                                // Move to start
                                                let (sx, sy) = coords[0];
                                                if let Ok(cmd) = DispatchMouseEventParams::builder()
                                                    .x(sx).y(sy)
                                                    .button(MouseButton::None).buttons(0)
                                                    .r#type(DispatchMouseEventType::MouseMoved).build() {
                                                    let _ = page.send_command(cmd).await;
                                                }
                                                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                                                // Press at start
                                                if let Ok(cmd) = DispatchMouseEventParams::builder()
                                                    .x(sx).y(sy)
                                                    .button(MouseButton::Left).buttons(1)
                                                    .r#type(DispatchMouseEventType::MousePressed).build() {
                                                    let _ = page.send_command(cmd).await;
                                                }
                                                // Drag through all points
                                                for (x, y) in &coords[1..] {
                                                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                                                    if let Ok(cmd) = DispatchMouseEventParams::builder()
                                                        .x(*x).y(*y)
                                                        .button(MouseButton::Left).buttons(1)
                                                        .r#type(DispatchMouseEventType::MouseMoved).build() {
                                                        let _ = page.send_command(cmd).await;
                                                    }
                                                }
                                                // Release at end
                                                let (ex, ey) = *coords.last().unwrap_or(&(sx, sy));
                                                tokio::time::sleep(std::time::Duration::from_millis(30)).await;
                                                if let Ok(cmd) = DispatchMouseEventParams::builder()
                                                    .x(ex).y(ey)
                                                    .button(MouseButton::Left).buttons(0)
                                                    .r#type(DispatchMouseEventType::MouseReleased).build() {
                                                    let _ = page.send_command(cmd).await;
                                                }
                                                log::info!("CIRCLE: engine drew circle with {} points", coords.len());
                                                let done_title = format!("CIRCLE_DONE:{{\"points\":{}}}", coords.len());
                                                let marker_js = format!(
                                                    "document.title={t};if(!document.getElementById('circle-engine-done')){{var d=document.createElement('div');d.id='circle-engine-done';d.style.display='none';d.dataset.t={t};document.body.appendChild(d);}}",
                                                    t = serde_json::to_string(&done_title).unwrap_or_default()
                                                );
                                                let _ = page.evaluate(marker_js).await;
                                                tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
                                                let verify_js = r#"(function(){
                                                    var btn=document.querySelector('#captcha-verify-button,button.captcha-verify,.verify-button,[class*=verify]');
                                                    if(btn){var r=btn.getBoundingClientRect();
                                                    var ev={bubbles:true,cancelable:true,clientX:r.x+r.width/2,clientY:r.y+r.height/2};
                                                    btn.dispatchEvent(new PointerEvent('pointerdown',ev));
                                                    btn.dispatchEvent(new MouseEvent('mousedown',ev));
                                                    btn.dispatchEvent(new PointerEvent('pointerup',ev));
                                                    btn.dispatchEvent(new MouseEvent('mouseup',ev));
                                                    btn.dispatchEvent(new MouseEvent('click',ev));
                                                    return 'clicked';}return 'not_found';})()
                                                "#;
                                                let _ = page.evaluate(verify_js).await;
                                                log::info!("CIRCLE: engine clicked verify");
                                                tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
                                                let post = self.title_context(page, &effective_cfg).await;
                                                if !post.is_empty() { title_now = post; }
                                            }
                                        }
                                    }
                                }
                            }

                            // WS_DRAG: engine clicks each word-search cell individually via CDP.
                            // Each cell gets a full click (move→press→release) for reliable
                            // click-to-select behavior, then engine clicks verify.
                            if new_title.starts_with("WS_DRAG:") {
                                if let Some(json_str) = new_title.strip_prefix("WS_DRAG:") {
                                    if let Ok(data) = serde_json::from_str::<serde_json::Value>(json_str) {
                                        let words = data.get("words").and_then(|v| v.as_array());
                                        let drags = data.get("drags").and_then(|v| v.as_array());
                                        if let (Some(words), Some(drags)) = (words, drags) {
                                            let mut dragged = Vec::new();
                                            let mut all_coords: Vec<(f64, f64)> = Vec::new();
                                            for (wi, drag_pts) in drags.iter().enumerate() {
                                                if let Some(pts) = drag_pts.as_array() {
                                                    let coords: Vec<(f64, f64)> = pts.iter().filter_map(|p| {
                                                        let x = p.get("x").and_then(|v| v.as_f64())?;
                                                        let y = p.get("y").and_then(|v| v.as_f64())?;
                                                        Some((x, y))
                                                    }).collect();
                                                    if !coords.is_empty() {
                                                        all_coords.extend_from_slice(&coords);
                                                        if let Some(w) = words.get(wi).and_then(|v| v.as_str()) {
                                                            dragged.push(w.to_string());
                                                        }
                                                    }
                                                }
                                            }
                                            // Deduplicate cells by rounded coordinate
                                            {
                                                let mut seen = std::collections::HashSet::new();
                                                all_coords.retain(|(x, y)| seen.insert((*x as i64, *y as i64)));
                                            }
                                            if !all_coords.is_empty() {
                                                use chromiumoxide::cdp::browser_protocol::input::{
                                                    DispatchMouseEventParams, DispatchMouseEventType, MouseButton,
                                                };
                                                // Click each cell center individually (click-to-select)
                                                for (x, y) in &all_coords {
                                                    if let Ok(cmd) = DispatchMouseEventParams::builder()
                                                        .x(*x).y(*y)
                                                        .button(MouseButton::None).buttons(0)
                                                        .r#type(DispatchMouseEventType::MouseMoved).build() {
                                                        let _ = page.send_command(cmd).await;
                                                    }
                                                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                                                    if let Ok(cmd) = DispatchMouseEventParams::builder()
                                                        .x(*x).y(*y)
                                                        .button(MouseButton::Left).buttons(1).click_count(1)
                                                        .r#type(DispatchMouseEventType::MousePressed).build() {
                                                        let _ = page.send_command(cmd).await;
                                                    }
                                                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                                                    if let Ok(cmd) = DispatchMouseEventParams::builder()
                                                        .x(*x).y(*y)
                                                        .button(MouseButton::Left).buttons(0).click_count(1)
                                                        .r#type(DispatchMouseEventType::MouseReleased).build() {
                                                        let _ = page.send_command(cmd).await;
                                                    }
                                                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                                                }
                                            }
                                            if !dragged.is_empty() {
                                                log::info!("WS engine clicked {} cells for {} words: {:?}", all_coords.len(), dragged.len(), dragged);
                                                let done_title = format!("WS_DONE:{{\"dragged\":{}}}", serde_json::to_string(&dragged).unwrap_or_default());
                                                // Set title + DOM marker to prevent re-clicking
                                                let marker_js = format!(
                                                    "document.title={t};if(!document.getElementById('ws-engine-done')){{var d=document.createElement('div');d.id='ws-engine-done';d.style.display='none';d.dataset.t={t};document.body.appendChild(d);}}",
                                                    t = serde_json::to_string(&done_title).unwrap_or_default()
                                                );
                                                let _ = page.evaluate(marker_js).await;
                                                // Engine clicks verify immediately after selecting cells
                                                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                                                let verify_js = r#"(function(){
                                                    var btn=document.querySelector('#captcha-verify-button,button.captcha-verify,.verify-button,[class*=verify]');
                                                    if(btn){var r=btn.getBoundingClientRect();
                                                    var ev={bubbles:true,cancelable:true,clientX:r.x+r.width/2,clientY:r.y+r.height/2};
                                                    btn.dispatchEvent(new PointerEvent('pointerdown',ev));
                                                    btn.dispatchEvent(new MouseEvent('mousedown',ev));
                                                    btn.dispatchEvent(new PointerEvent('pointerup',ev));
                                                    btn.dispatchEvent(new MouseEvent('mouseup',ev));
                                                    btn.dispatchEvent(new MouseEvent('click',ev));
                                                    return 'clicked';}return 'not_found';})()
                                                "#;
                                                let _ = page.evaluate(verify_js).await;
                                                log::info!("WS: engine clicked verify after selecting cells");
                                                tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
                                                let post = self.title_context(page, &effective_cfg).await;
                                                title_now = if !post.is_empty() { post } else { done_title };
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Re-evaluate vision decision now that stagnation is known.
            // If stagnation/stuck upgraded us from text→vision, capture screenshot now.
            let use_vision = if !use_vision
                && self.has_dual_model_routing()
                && self.should_use_vision_this_round(
                    round_idx,
                    stagnated,
                    action_stuck_rounds,
                    false,
                ) {
                true // upgraded to vision mid-round
            } else {
                use_vision
            };

            // Late screenshot capture when upgrading to vision after stagnation detected
            let include_screenshot_this_round = use_vision
                && !skip_screenshot_for_extraction
                && self.should_include_screenshot_for_round(&effective_cfg, use_vision);
            let screenshot = if include_screenshot_this_round && screenshot.is_empty() {
                match self.screenshot_as_data_url_with_profile(page, cap).await {
                    Ok(s) => s,
                    Err(e) => {
                        log::warn!("Late screenshot failed (non-fatal): {e}");
                        String::new()
                    }
                }
            } else {
                screenshot
            };

            // Ask model (with retry policy) - pass memory as immutable ref for context
            let plan = if use_chrome_ai {
                self.infer_plan_chrome_ai_with_retry(
                    page,
                    &effective_cfg,
                    cap,
                    url_input,
                    &url_now,
                    &title_now,
                    &html,
                    &screenshot,
                    round_idx,
                    stagnated,
                    action_stuck_rounds,
                    &loop_blocklist,
                    memory.as_deref(),
                    effective_system_prompt.as_deref(),
                    effective_system_prompt_extra.as_deref(),
                    effective_user_message_extra.as_deref(),
                )
                .await?
            } else {
                self.infer_plan_with_retry(
                    &effective_cfg,
                    cap,
                    url_input,
                    &url_now,
                    &title_now,
                    &html,
                    &screenshot,
                    round_idx,
                    stagnated,
                    action_stuck_rounds,
                    &loop_blocklist,
                    memory.as_deref(),
                    use_vision,
                    effective_system_prompt.as_deref(),
                    effective_system_prompt_extra.as_deref(),
                    effective_user_message_extra.as_deref(),
                )
                .await?
            };

            // Accumulate token usage from this round
            total_usage.accumulate(&plan.usage);
            last_label = plan.label.clone();

            // Stuck-loop detection: hash step structure (ignoring Evaluate code content
            // which LLMs often vary slightly while functionally repeating the same approach)
            {
                use std::hash::{Hash, Hasher};
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                for step in &plan.steps {
                    if let Some(obj) = step.as_object() {
                        for (key, val) in obj {
                            key.hash(&mut hasher);
                            // Hash Evaluate as just the action name, not the JS code
                            if key != "Evaluate" {
                                val.to_string().hash(&mut hasher);
                            }
                        }
                    }
                }
                let step_hash = hasher.finish();

                recent_step_hashes.push_back(step_hash);
                if recent_step_hashes.len() > 10 {
                    recent_step_hashes.pop_front();
                }

                action_stuck_rounds = recent_step_hashes
                    .iter()
                    .rev()
                    .take_while(|h| **h == step_hash)
                    .count();

                if action_stuck_rounds >= 3 {
                    loop_blocklist = Self::summarize_step_blocklist(&plan.steps, 10);
                    // Escalate only when a vision-capable route exists.
                    if self.has_vision_capable_route() {
                        force_vision_next_round = true;
                    }
                    log::warn!(
                        "Action loop detected: {} consecutive identical step sequences",
                        action_stuck_rounds
                    );
                } else if !stagnated {
                    loop_blocklist.clear();
                }
            }

            // Process memory operations from the plan
            if let Some(ref mut mem) = memory {
                for op in &plan.memory_ops {
                    match op {
                        MemoryOperation::Set { key, value } => {
                            // Detect `request_vision` memory_op for dual-model routing
                            if key == "request_vision" {
                                if self.has_vision_capable_route() {
                                    force_vision_next_round = true;
                                    log::debug!("Agent requested vision for next round");
                                }
                                continue; // don't persist this transient key
                            }
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
                // Handle skill requests via memory_ops
                #[cfg(feature = "skills")]
                if let Some(ref registry) = self.skill_registry {
                    for op in &plan.memory_ops {
                        if let MemoryOperation::Set { key, value } = op {
                            if key == "request_skill" {
                                if let Some(skill_name) = value.as_str() {
                                    if registry.get(skill_name).is_some() {
                                        log::info!("Agent requested skill: {}", skill_name);
                                        mem.set(
                                            "_active_skill".to_string(),
                                            serde_json::json!(skill_name),
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
                // Record this round's URL and action
                mem.add_visited_url(&url_now);
                mem.add_action(format!("Round {}: {}", round_idx + 1, &plan.label));
            }

            // Save relevance flag if present
            if plan.relevant.is_some() {
                last_relevant = plan.relevant;
            }
            if plan.reasoning.is_some() {
                last_reasoning = plan.reasoning.clone();
            }

            // Save extracted data if present
            if plan.extracted.is_some() {
                last_extracted = plan.extracted.clone();
                // Also store in memory if available
                if let (Some(ref mut mem), Some(ref extracted)) = (&mut memory, &plan.extracted) {
                    mem.add_extraction(extracted.clone());
                    if let Some(level_key) = Self::extracted_level_key(Some(extracted)) {
                        let attempts = mem.increment_level_attempt(&level_key);
                        mem.set("_current_level_key", serde_json::json!(level_key));
                        mem.set("_current_level_attempts", serde_json::json!(attempts));
                        current_level_attempts = Some(attempts);
                    }
                }
            }

            // Execute steps (even if done=true, we need to process OpenPage actions).
            // When stuck in a loop for multiple rounds, skip repeated steps and auto-inspect DOM.
            let has_structured_level_state = current_level_attempts.is_some();
            let should_force_level_refresh = !plan.done
                && has_structured_level_state
                && current_level_attempts.unwrap_or(0) >= 12;

            if should_force_level_refresh {
                log::warn!(
                    "Level attempts reached {} - forcing captcha refresh recovery",
                    current_level_attempts.unwrap_or(0)
                );
                // Use dispatchEvent with full pointer/mouse events (not el.click()
                // which doesn't trigger real browser events on most pages).
                // Generic selectors cover refresh/retry/reset patterns dynamically.
                let recovery_steps = vec![
                    serde_json::json!({ "Evaluate": r#"
(() => {
  const sels = [
    '[class*=refresh]', '[data-action=refresh]',
    '[aria-label*=refresh i]', '[title*=refresh i]',
    'button[id*=refresh i]', 'button[class*=refresh i]',
    'button[id*=retry i]', 'button[class*=retry i]',
    'button[id*=reset i]', 'button[class*=reset i]',
    'button[id*=new i]', 'a[id*=refresh i]'
  ];
  for (const sel of sels) {
    const el = document.querySelector(sel);
    if (el && !el.disabled && el.offsetParent !== null) {
      const r = el.getBoundingClientRect();
      const cx = r.x + r.width / 2, cy = r.y + r.height / 2;
      const opts = { bubbles: true, cancelable: true, clientX: cx, clientY: cy };
      el.dispatchEvent(new PointerEvent('pointerdown', opts));
      el.dispatchEvent(new MouseEvent('mousedown', opts));
      el.dispatchEvent(new PointerEvent('pointerup', opts));
      el.dispatchEvent(new MouseEvent('mouseup', opts));
      el.dispatchEvent(new MouseEvent('click', opts));
      document.title = 'RECOVERY:clicked=' + sel;
      return;
    }
  }
  document.title = 'RECOVERY:no_refresh_btn';
})()
"# }),
                    serde_json::json!({ "Wait": 1000 }),
                    // Clear stale RECOVERY title so it doesn't confuse next rounds
                    serde_json::json!({ "Evaluate": "try{document.title=document.title.replace(/^RECOVERY:.*/,'')}catch(e){}" }),
                ];
                let (steps_executed, spawn_pages) = self
                    .execute_steps(page, &recovery_steps, &effective_cfg)
                    .await?;
                total_steps_executed += steps_executed;
                all_spawn_pages.extend(spawn_pages);
                recent_step_hashes.clear();
                action_stuck_rounds = 0;
                loop_blocklist.clear();
                if self.has_vision_capable_route() {
                    force_vision_next_round = true;
                }
                if let Some(ref mut mem) = memory {
                    // Reset level attempt counter so the agent gets fresh tries
                    if let Some(ref key) = mem
                        .get("_current_level_key")
                        .and_then(|v| v.as_str().map(String::from))
                    {
                        mem.reset_level_attempt(key);
                    }
                    mem.add_action(format!(
                        "SYSTEM: Forced refresh recovery after {} attempts on same level. Counter reset.",
                        current_level_attempts.unwrap_or(0)
                    ));
                }
            } else if action_stuck_rounds >= 5 {
                let stuck_rounds = action_stuck_rounds;
                log::warn!(
                    "Skipping {} repeated steps - auto-inspecting DOM (stuck {} rounds)",
                    plan.steps.len(),
                    stuck_rounds
                );
                if self.has_vision_capable_route() {
                    force_vision_next_round = true;
                }
                // Inject DOM inspection so the model gets real state data next round
                let _ = page
                    .evaluate(
                        r#"document.title = 'AUTO_DOM_INSPECT:' + JSON.stringify({
                        level_text: (document.querySelector('h2,h3,.level-title,.challenge-title')?.textContent || '').trim().slice(0, 120),
                        prompt_text: (document.querySelector('.prompt,.instruction,.challenge-prompt,.captcha-instructions')?.textContent || document.body?.innerText || '').trim().slice(0, 180),
                        selected_count: [...document.querySelectorAll(
                            '.selected,.active,[aria-checked="true"],[aria-pressed="true"],[class*="selected"],[class*="active"]'
                        )].length,
                        verify_buttons: [...document.querySelectorAll('button,input[type=button],input[type=submit]')].slice(0, 6).map(el => ({
                            text: (el.textContent || el.value || '').trim().slice(0, 40),
                            dis: !!el.disabled
                        })),
                        els: [...document.querySelectorAll(
                            'button, [onclick], [class*=item], [class*=cell], [class*=grid] > *, input, select, [class*=square], [class*=piece], [class*=tile]'
                        )].slice(0, 30).map((el, i) => ({
                            n: i,
                            tag: el.tagName,
                            cls: el.className.split(' ').slice(0, 4).join(' '),
                            text: el.textContent.trim().slice(0, 50),
                            sel: el.classList.contains('selected') || el.classList.contains('active') || el.getAttribute('aria-checked') === 'true',
                            dis: el.disabled || el.classList.contains('disabled')
                        }))
                    })"#,
                    )
                    .await;
                // Reset loop streak after injecting explicit recovery context.
                // This avoids burning rounds on repeated skip-only cycles.
                recent_step_hashes.clear();
                action_stuck_rounds = 0;
                // Record in memory that DOM was auto-inspected
                if let Some(ref mut mem) = memory {
                    mem.add_action(format!(
                        "SYSTEM: Skipped repeated steps (stuck {}x). DOM auto-inspected - check page title for element states.",
                        stuck_rounds
                    ));
                }
            } else if !plan.steps.is_empty() {
                let (steps_executed, spawn_pages) = self
                    .execute_steps(page, &plan.steps, &effective_cfg)
                    .await?;
                total_steps_executed += steps_executed;
                all_spawn_pages.extend(spawn_pages);
            }

            // Done condition (model-driven)
            if plan.done {
                // Store successful experience in long-term memory
                #[cfg(feature = "memvid")]
                if total_steps_executed > 0 {
                    if let Some(ref exp_mem) = self.experience_memory {
                        if let Some(ref mem) = memory {
                            let record = super::long_term_memory::ExperienceRecord::from_session(
                                url_input,
                                &plan.label,
                                mem,
                                total_steps_executed,
                                (round_idx + 1) as u32,
                            );
                            let exp_mem = exp_mem.clone();
                            let handle = tokio::runtime::Handle::current();
                            let _ = tokio::task::spawn_blocking(move || {
                                handle.block_on(async move {
                                    let mut exp = exp_mem.write().await;
                                    if let Err(e) = exp.store_experience(&record).await {
                                        log::warn!("Failed to store experience: {}", e);
                                    }
                                })
                            })
                            .await;
                        }
                    }
                }

                // Capture final screenshot (enabled by default)
                let final_screenshot = if effective_cfg.screenshot {
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
                    spawn_pages: all_spawn_pages,
                    relevant: last_relevant,
                    reasoning: last_reasoning,
                });
            }

            // Post-step delay
            if effective_cfg.post_plan_wait_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(
                    effective_cfg.post_plan_wait_ms,
                ))
                .await;
            }
        }

        // Store experience after all rounds complete
        #[cfg(feature = "memvid")]
        if total_steps_executed > 0 {
            if let Some(ref exp_mem) = self.experience_memory {
                if let Some(ref mem) = memory {
                    let record = super::long_term_memory::ExperienceRecord::from_session(
                        url_input,
                        &last_label,
                        mem,
                        total_steps_executed,
                        rounds as u32,
                    );
                    let exp_mem = exp_mem.clone();
                    let handle = tokio::runtime::Handle::current();
                    let _ = tokio::task::spawn_blocking(move || {
                        handle.block_on(async move {
                            let mut exp = exp_mem.write().await;
                            if let Err(e) = exp.store_experience(&record).await {
                                log::warn!("Failed to store experience: {}", e);
                            }
                        })
                    })
                    .await;
                }
            }
        }

        // Final screenshot after all rounds
        let final_screenshot = if effective_cfg.screenshot {
            self.take_final_screenshot(page).await.ok()
        } else {
            None
        };

        Ok(AutomationResult {
            label: last_label,
            steps_executed: total_steps_executed,
            success: false,
            error: Some(format!(
                "automation did not complete within {} round(s)",
                rounds
            )),
            usage: total_usage,
            extracted: last_extracted,
            screenshot: final_screenshot,
            spawn_pages: all_spawn_pages,
            relevant: last_relevant,
            reasoning: last_reasoning,
        })
    }

    /// Infer plan with retry policy.
    #[allow(clippy::too_many_arguments)]
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
        action_stuck_rounds: usize,
        loop_blocklist: &[String],
        memory: Option<&AutomationMemory>,
        use_vision: bool,
        base_system_prompt: Option<&str>,
        system_prompt_extra: Option<&str>,
        user_message_extra: Option<&str>,
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
                    action_stuck_rounds,
                    loop_blocklist,
                    memory,
                    use_vision,
                    base_system_prompt,
                    system_prompt_extra,
                    user_message_extra,
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
    #[allow(clippy::too_many_arguments)]
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
        action_stuck_rounds: usize,
        loop_blocklist: &[String],
        memory: Option<&AutomationMemory>,
        use_vision: bool,
        base_system_prompt: Option<&str>,
        system_prompt_extra: Option<&str>,
        user_message_extra: Option<&str>,
    ) -> EngineResult<AutomationPlan> {
        use super::{
            best_effort_parse_json_object, extract_assistant_content, extract_usage,
            reasoning_payload, DEFAULT_SYSTEM_PROMPT, EXTRACTION_ONLY_SYSTEM_PROMPT,
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
            #[serde(skip_serializing_if = "Option::is_none")]
            reasoning: Option<serde_json::Value>,
            #[serde(skip_serializing_if = "Option::is_none")]
            tools: Option<Vec<serde_json::Value>>,
            #[serde(skip_serializing_if = "Option::is_none")]
            tool_choice: Option<serde_json::Value>,
        }

        // Build system prompt — use focused extraction prompt for single-round extraction
        let mut system_msg = if effective_cfg.is_extraction_only() {
            EXTRACTION_ONLY_SYSTEM_PROMPT.to_string()
        } else {
            DEFAULT_SYSTEM_PROMPT.to_string()
        };
        if let Some(base) = base_system_prompt {
            if !base.trim().is_empty() {
                system_msg.push_str("\n\n---\nCONFIGURED SYSTEM INSTRUCTIONS:\n");
                system_msg.push_str(base.trim());
            }
        }
        if let Some(extra) = system_prompt_extra {
            if !extra.trim().is_empty() {
                system_msg.push_str("\n\n---\nADDITIONAL INSTRUCTIONS:\n");
                system_msg.push_str(extra.trim());
            }
        }
        // Inject matching skills from the skill registry (limited by config)
        #[cfg(feature = "skills")]
        if let Some(ref registry) = self.skill_registry {
            log::debug!(
                "Skill registry: {} skills, checking url={} title={} html_len={}",
                registry.len(),
                url_now,
                title_now,
                html.len()
            );
            let mut skill_ctx = registry.match_context_limited(
                url_now,
                title_now,
                html,
                effective_cfg.max_skills_per_round,
                effective_cfg.max_skill_context_chars,
            );
            // Also inject agent-requested skills from memory
            if let Some(mem) = memory {
                if let Some(active) = mem.get("_active_skill") {
                    if let Some(name) = active.as_str() {
                        if let Some(skill) = registry.get(name) {
                            if !skill_ctx.contains(&skill.name) {
                                if !skill_ctx.is_empty() {
                                    skill_ctx.push_str("\n\n");
                                }
                                skill_ctx.push_str("## Skill: ");
                                skill_ctx.push_str(&skill.name);
                                skill_ctx.push('\n');
                                skill_ctx.push_str(&skill.content);
                            }
                        }
                    }
                }
            }
            if !skill_ctx.is_empty() {
                // Log which skills matched
                let matched: Vec<_> = registry
                    .find_matching(url_now, title_now, html)
                    .iter()
                    .map(|s| s.name.as_str())
                    .collect();
                log::debug!(
                    "Injecting {} skills ({} chars): {:?}",
                    matched.len(),
                    skill_ctx.len(),
                    matched
                );
                system_msg.push_str("\n\n---\nACTIVATED SKILLS:\n");
                system_msg.push_str(&skill_ctx);
            } else if !registry.is_empty() {
                log::debug!(
                    "No skills matched for url={} title={} html_len={}",
                    url_now,
                    title_now,
                    html.len()
                );
                // Keep unmatched rounds lean: do not inject skill catalog text.
            }
        }

        // In extraction-only mode, inject schema / extraction prompt / relevance
        // instructions into the system prompt (mirrors engine.rs system_prompt_compiled).
        if effective_cfg.is_extraction_only() {
            if let Some(schema) = &effective_cfg.extraction_schema {
                system_msg.push_str("\n\n---\nExtraction Schema: ");
                system_msg.push_str(&schema.name);
                system_msg.push('\n');
                if let Some(desc) = &schema.description {
                    system_msg.push_str("Description: ");
                    system_msg.push_str(desc.trim());
                    system_msg.push('\n');
                }
                system_msg.push_str("The \"extracted\" field MUST conform to this JSON Schema:\n");
                system_msg.push_str(&schema.schema);
                system_msg.push('\n');
                if schema.strict {
                    system_msg.push_str("STRICT MODE: Follow the schema exactly.\n");
                }
            }
            if let Some(extraction_prompt) = &effective_cfg.extraction_prompt {
                system_msg.push_str("\nExtraction instructions: ");
                system_msg.push_str(extraction_prompt.trim());
                system_msg.push('\n');
            }
            if effective_cfg.relevance_gate {
                system_msg.push_str(
                    "\n---\nRELEVANCE GATE: Include \"relevant\": true|false in your response.\n",
                );
                if let Some(prompt) = &effective_cfg.relevance_prompt {
                    system_msg.push_str("Relevance criteria: ");
                    system_msg.push_str(prompt.trim());
                    system_msg.push('\n');
                } else if let Some(ep) = &effective_cfg.extraction_prompt {
                    system_msg.push_str("Judge relevance against: ");
                    system_msg.push_str(ep.trim());
                    system_msg.push('\n');
                }
            }
        }

        if screenshot.is_empty() {
            Self::apply_text_only_prompt_flavor(&mut system_msg, self.has_dual_model_routing());
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
            action_stuck_rounds,
            loop_blocklist,
            memory,
            user_message_extra,
        );

        // Build user content – omit image_url block when text-only
        let user_content = if use_vision && !screenshot.is_empty() {
            serde_json::json!([
                { "type": "text", "text": user_text },
                {
                    "type": "image_url",
                    "image_url": { "url": screenshot }
                }
            ])
        } else {
            serde_json::json!([
                { "type": "text", "text": user_text }
            ])
        };

        // Resolve model endpoint for this round (dual-model routing)
        let (resolved_api_url, resolved_model, resolved_api_key) =
            self.resolve_model_for_round(use_vision);

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

        let use_tools = !effective_cfg.is_extraction_only()
            && effective_cfg
                .tool_calling_mode
                .should_use_tools(resolved_model);

        let response_format = if use_tools {
            None
        } else if effective_cfg.request_json_object {
            Some(serde_json::json!({ "type": "json_object" }))
        } else {
            None
        };

        let tools = if use_tools {
            Some(
                ActionToolSchemas::all()
                    .into_iter()
                    .filter_map(|tool| serde_json::to_value(tool).ok())
                    .collect::<Vec<_>>(),
            )
        } else {
            None
        };

        let tool_choice = if use_tools {
            Some(serde_json::json!("auto"))
        } else {
            None
        };

        let request = Request {
            model: resolved_model.to_string(),
            messages,
            temperature: Some(effective_cfg.temperature),
            max_tokens: Some(effective_cfg.max_tokens as u32),
            response_format,
            reasoning: reasoning_payload(effective_cfg),
            tools,
            tool_choice,
        };

        // Acquire semaphore if configured
        let _permit = self.acquire_llm_permit().await;

        // Make HTTP request with 2 minute timeout for LLM calls
        static CLIENT: std::sync::LazyLock<reqwest::Client> = std::sync::LazyLock::new(|| {
            reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new())
        });

        let mut req = CLIENT.post(resolved_api_url).json(&request);
        if let Some(key) = resolved_api_key {
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
        let content = extract_assistant_content(&body).unwrap_or_default();
        if !content.is_empty() {
            log::debug!("LLM response content: {}", content);
        }
        let usage = extract_usage(&body);

        let tool_steps = if use_tools {
            let tool_calls = parse_tool_calls(&body);
            if !tool_calls.is_empty() {
                tool_calls_to_steps(&tool_calls)
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };

        if content.trim().is_empty() && tool_steps.is_empty() {
            return Err(EngineError::MissingField("choices[0].message.content"));
        }

        // Parse JSON response
        let parsed = if content.trim().is_empty() {
            serde_json::json!({
                "label": "automation",
                "done": false,
                "steps": []
            })
        } else if effective_cfg.best_effort_json_extract {
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

        let done = parsed
            .get("done")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let mut steps = parsed
            .get("steps")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        if !tool_steps.is_empty() {
            steps.extend(tool_steps);
        }
        log::debug!(
            "Parsed plan - label: {}, done: {}, steps: {:?}",
            label,
            done,
            steps
        );

        // Extract relevance field if gate is enabled
        let relevant = if effective_cfg.relevance_gate {
            Some(
                parsed
                    .get("relevant")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true),
            )
        } else {
            None
        };
        let reasoning = parsed.get("reasoning").and_then(|v| {
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

        // Try to get extracted field, or fallback to the entire response when in extraction mode.
        // Treat `extracted: {}` (empty object) the same as missing for recovery purposes.
        let raw_extracted = parsed.get("extracted").cloned().and_then(|v| {
            if v.as_object().is_some_and(|o| o.is_empty()) {
                None // empty object → trigger fallback chain
            } else {
                Some(v)
            }
        });

        let extracted = raw_extracted.or_else(|| {
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
                    // Last resort: recover data from Fill steps.
                    // Weak models sometimes emit Fill actions instead of extracted data.
                    let mut fill_data = serde_json::Map::new();
                    for step in &steps {
                        if let Some(fill) = step.get("Fill") {
                            if let (Some(sel), Some(val)) = (
                                fill.get("selector").and_then(|s| s.as_str()),
                                fill.get("value"),
                            ) {
                                // Use the selector (or last segment) as key
                                let key = sel
                                    .rsplit_once(' ')
                                    .map(|(_, last)| last)
                                    .unwrap_or(sel)
                                    .trim_start_matches('#')
                                    .trim_start_matches('.');
                                if !key.is_empty() {
                                    fill_data.insert(key.to_string(), val.clone());
                                }
                            }
                        }
                    }
                    if !fill_data.is_empty() {
                        log::debug!("Recovered {} fields from Fill steps", fill_data.len());
                        Some(serde_json::Value::Object(fill_data))
                    } else {
                        None
                    }
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
            relevant,
            reasoning,
        })
    }

    // ── Chrome built-in AI inference ──────────────────────────────────

    /// Warm up Chrome's built-in LanguageModel (Gemini Nano).
    ///
    /// Creates a session and sends a trivial prompt to ensure the model
    /// is loaded and ready. Mirrors `warm_gemini_model` from solvers.rs.
    async fn warm_chrome_ai(page: &Page) -> EngineResult<()> {
        use chromiumoxide::cdp::js_protocol::runtime::EvaluateParams;

        // No try/catch — errors propagate to Rust so we can detect missing API.
        // First checks typeof to give a clear error before attempting .create().
        let js = r#"(async()=>{if(typeof LanguageModel==="undefined")throw new ReferenceError("LanguageModel is not defined");const s=await LanguageModel.create({expectedInputs:[{type:"text",languages:["en"]}],expectedOutputs:[{type:"text",languages:["en"]}]});const r=await s.prompt([{role:"user",content:[{type:"text",value:"ping"}]}]);return "ok:"+r.slice(0,20)})()"#;

        let params = EvaluateParams::builder()
            .expression(js)
            .await_promise(true)
            .build()
            .expect("valid evaluate params");

        match tokio::time::timeout(std::time::Duration::from_secs(60), page.evaluate(params)).await
        {
            Ok(Ok(_)) => {
                log::info!("Chrome AI (LanguageModel) warm-up successful");
                Ok(())
            }
            Ok(Err(e)) => {
                let msg = format!("{e}");
                if Self::is_chrome_ai_missing(&msg) {
                    log::warn!("Chrome AI not available: {msg}");
                    Err(EngineError::Remote(
                        "Chrome LanguageModel is not available. Enable chrome://flags/#optimization-guide-on-device-model and chrome://flags/#prompt-api-for-gemini-nano".to_string(),
                    ))
                } else {
                    log::warn!("Chrome AI warm-up error (non-fatal): {msg}");
                    Ok(())
                }
            }
            Err(_) => {
                log::warn!("Chrome AI warm-up timed out (60s) — continuing anyway");
                Ok(())
            }
        }
    }

    /// Check if an error message indicates Chrome's LanguageModel is unavailable.
    fn is_chrome_ai_missing(err: &str) -> bool {
        err.contains("LanguageModel is not defined")
            || err.contains("ReferenceError")
            || err.contains("Uncaught ReferenceError")
            || err.contains("cannot read property 'create' of undefined")
    }

    /// Invalidate the cached Chrome AI session on the page.
    ///
    /// Called after errors or timeouts so the next retry creates a fresh session
    /// rather than reusing one that may be in a broken state (e.g. after navigation).
    async fn invalidate_chrome_ai_session(page: &Page) {
        use chromiumoxide::cdp::js_protocol::runtime::EvaluateParams;
        let _ = page
            .evaluate(
                EvaluateParams::builder()
                    .expression(
                        "try{if(window.__spiderSession){window.__spiderSession.destroy();}window.__spiderSession=null;window.__spiderSessionHash=0;}catch(e){}",
                    )
                    .build()
                    .expect("valid params"),
            )
            .await;
    }

    /// Infer plan via Chrome AI with retry policy.
    ///
    /// Same retry/backoff logic as `infer_plan_with_retry`, but delegates
    /// to `infer_plan_chrome_ai_once` for in-page inference.
    #[allow(clippy::too_many_arguments)]
    async fn infer_plan_chrome_ai_with_retry(
        &self,
        page: &Page,
        effective_cfg: &RemoteMultimodalConfig,
        cap: &CaptureProfile,
        url_input: &str,
        url_now: &str,
        title_now: &str,
        html: &str,
        screenshot: &str,
        round_idx: usize,
        stagnated: bool,
        action_stuck_rounds: usize,
        loop_blocklist: &[String],
        memory: Option<&AutomationMemory>,
        base_system_prompt: Option<&str>,
        system_prompt_extra: Option<&str>,
        user_message_extra: Option<&str>,
    ) -> EngineResult<AutomationPlan> {
        let max_attempts = effective_cfg.retry.max_attempts.max(1);
        let mut last_err = None;

        for attempt in 0..max_attempts {
            match self
                .infer_plan_chrome_ai_once(
                    page,
                    effective_cfg,
                    cap,
                    url_input,
                    url_now,
                    title_now,
                    html,
                    screenshot,
                    round_idx,
                    stagnated,
                    action_stuck_rounds,
                    loop_blocklist,
                    memory,
                    base_system_prompt,
                    system_prompt_extra,
                    user_message_extra,
                )
                .await
            {
                Ok(plan) => return Ok(plan),
                Err(e) => {
                    last_err = Some(e);
                    if attempt + 1 < max_attempts {
                        let power = attempt.min(6);
                        let delay = effective_cfg.retry.backoff_ms * (1 << power);
                        tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                    }
                }
            }
        }

        Err(last_err.unwrap_or_else(|| {
            EngineError::Remote("chrome_ai: max retries exceeded".to_string())
        }))
    }

    /// Single plan inference via Chrome's built-in LanguageModel API.
    ///
    /// Uses a compact `CHROME_AI_SYSTEM_PROMPT`, passes it via the
    /// `systemPrompt` parameter in `LanguageModel.create()` for proper role
    /// separation, and reuses the session across rounds via a JS global.
    #[allow(clippy::too_many_arguments)]
    async fn infer_plan_chrome_ai_once(
        &self,
        page: &Page,
        effective_cfg: &RemoteMultimodalConfig,
        cap: &CaptureProfile,
        url_input: &str,
        url_now: &str,
        title_now: &str,
        html: &str,
        screenshot: &str,
        round_idx: usize,
        stagnated: bool,
        action_stuck_rounds: usize,
        loop_blocklist: &[String],
        memory: Option<&AutomationMemory>,
        base_system_prompt: Option<&str>,
        system_prompt_extra: Option<&str>,
        user_message_extra: Option<&str>,
    ) -> EngineResult<AutomationPlan> {
        use super::{
            best_effort_parse_json_object, fnv1a64, CHROME_AI_SYSTEM_PROMPT,
            EXTRACTION_ONLY_SYSTEM_PROMPT,
        };
        use chromiumoxide::cdp::js_protocol::runtime::EvaluateParams;

        // ── Build system prompt (compact Chrome AI variant) ──
        let mut system_msg = if effective_cfg.is_extraction_only() {
            EXTRACTION_ONLY_SYSTEM_PROMPT.to_string()
        } else {
            CHROME_AI_SYSTEM_PROMPT.to_string()
        };
        if let Some(base) = base_system_prompt {
            if !base.trim().is_empty() {
                system_msg.push_str("\n\n---\nCONFIGURED SYSTEM INSTRUCTIONS:\n");
                system_msg.push_str(base.trim());
            }
        }
        if let Some(extra) = system_prompt_extra {
            if !extra.trim().is_empty() {
                system_msg.push_str("\n\n---\nADDITIONAL INSTRUCTIONS:\n");
                system_msg.push_str(extra.trim());
            }
        }

        // Skill injection
        #[cfg(feature = "skills")]
        if let Some(ref registry) = self.skill_registry {
            let mut skill_ctx = registry.match_context_limited(
                url_now,
                title_now,
                html,
                effective_cfg.max_skills_per_round,
                effective_cfg.max_skill_context_chars,
            );
            if let Some(mem) = memory {
                if let Some(active) = mem.get("_active_skill") {
                    if let Some(name) = active.as_str() {
                        if let Some(skill) = registry.get(name) {
                            if !skill_ctx.contains(&skill.name) {
                                if !skill_ctx.is_empty() {
                                    skill_ctx.push_str("\n\n");
                                }
                                skill_ctx.push_str("## Skill: ");
                                skill_ctx.push_str(&skill.name);
                                skill_ctx.push('\n');
                                skill_ctx.push_str(&skill.content);
                            }
                        }
                    }
                }
            }
            if !skill_ctx.is_empty() {
                system_msg.push_str("\n\n---\nACTIVATED SKILLS:\n");
                system_msg.push_str(&skill_ctx);
            }
        }

        // Text-only flavor when no screenshot
        if screenshot.is_empty() {
            Self::apply_text_only_prompt_flavor(&mut system_msg, false);
        }

        // When stuck for 5+ rounds, destroy the LM session so the model
        // restarts with a fresh system prompt. Prevents infinite loops.
        if action_stuck_rounds >= 5 {
            Self::invalidate_chrome_ai_session(page).await;
        }

        // ── Probe interactive elements for small-model guidance ──
        // Annotates visible interactive elements with `data-spider-idx` attributes
        // and returns a numbered list. The model clicks by index (e.g. [0], [1])
        // instead of guessing CSS selectors — much more reliable for small models.
        let interactive_hint = {
            let probe_js = r#"(()=>{try{document.querySelectorAll('[data-spider-idx]').forEach(el=>el.removeAttribute('data-spider-idx'));const seen=new Set();const items=[];let idx=0;function add(el,skipEmptyLinks){if(idx>=15||seen.has(el))return;const r=el.getBoundingClientRect();if(r.width<1||r.height<1)return;if(el.closest('footer,nav,.footer,.nav,.site-footer,.site-header,header'))return;seen.add(el);const tag=el.tagName.toLowerCase();const ty=el.getAttribute('type')||'';const role=el.getAttribute('role')||'';const txt=(el.textContent||el.getAttribute('aria-label')||el.getAttribute('placeholder')||'').trim().replace(/\s+/g,' ').slice(0,40);if(tag==='a'){if(!txt||skipEmptyLinks)return;const h=el.getAttribute('href')||'';if(h&&!h.startsWith('#')&&!h.startsWith('javascript')&&new URL(h,location.href).pathname!==location.pathname)return;}el.setAttribute('data-spider-idx',String(idx));const desc=role||ty||(tag==='a'?'link':tag);items.push('['+idx+'] '+desc+(txt?' "'+txt.replace(/"/g,"'")+'"':''));idx++;}const hi=document.querySelectorAll('button,input,select,textarea,label,[role="button"],[role="checkbox"],[role="link"],[onclick]');for(const el of hi)add(el,false);const links=document.querySelectorAll('a');for(const el of links)add(el,true);const all=document.querySelectorAll('div,span,li');for(const el of all){if(idx>=15)break;if(seen.has(el))continue;try{if(getComputedStyle(el).cursor==='pointer'){add(el,false);}}catch(e){}}return items.join('\n');}catch(e){return'';}})();"#;
            match page
                .evaluate(
                    EvaluateParams::builder()
                        .expression(probe_js)
                        .build()
                        .expect("valid params"),
                )
                .await
            {
                Ok(eval) => eval
                    .value()
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
                    .unwrap_or_default(),
                Err(_) => String::new(),
            }
        };

        if !interactive_hint.is_empty() {
            log::debug!("Chrome AI element probe found:\n{}", interactive_hint);
        }

        // ── Build user prompt (reuse existing method) ──
        let mut user_text = self.build_user_prompt(
            effective_cfg,
            cap,
            url_input,
            url_now,
            title_now,
            html,
            round_idx,
            stagnated,
            action_stuck_rounds,
            loop_blocklist,
            memory,
            user_message_extra,
        );

        // Strip framework data-* attributes from user prompt to prevent small
        // models from hallucinating selectors like [data-v-abc123] from raw HTML.
        // Preserves our own data-spider-idx references in the CLICKABLE ELEMENTS section.
        user_text = Self::strip_framework_data_attrs(&user_text);

        // Prepend indexed interactive elements list — model clicks by number
        if !interactive_hint.is_empty() {
            user_text = format!(
                "CLICKABLE ELEMENTS (click by index, e.g. {{\"Click\":\"[data-spider-idx='0']\"}} ):\n{interactive_hint}\n\n{user_text}"
            );
        }

        // Append a response primer to help smaller models produce valid JSON.
        // This nudges the model to start its response with the expected format.
        user_text.push_str("\nRespond with JSON only. Start with {\"label\":");

        // ── Smart context budgeting ──
        // System prompt goes to systemPrompt param (untruncated — it's compact).
        // User prompt: truncate only the HTML section if over budget.
        let max_user_chars = self.chrome_ai_max_user_chars;
        let user_text = if user_text.len() > max_user_chars {
            Self::truncate_chrome_ai_user_prompt(&user_text, max_user_chars)
        } else {
            user_text
        };

        // ── Hash system prompt for session reuse ──
        let sys_hash = fnv1a64(system_msg.as_bytes());

        // ── Build JavaScript for in-page inference with session reuse ──
        let escaped_system = serde_json::to_string(&system_msg)
            .unwrap_or_else(|_| format!("\"{}\"", system_msg.replace('\"', "\\\"")));
        let escaped_user = serde_json::to_string(&user_text)
            .unwrap_or_else(|_| format!("\"{}\"", user_text.replace('\"', "\\\"")));

        let has_screenshot = !screenshot.is_empty();

        // Pass model parameters from config.
        // Chrome LanguageModel API requires BOTH topK and temperature, or neither.
        let temperature = effective_cfg.temperature;
        let top_k = if temperature < 0.01 { 1 } else { 40 };

        // JS template: try/catch around session creation and prompting.
        // On error, destroy stale session so next retry starts fresh.
        let js = if has_screenshot {
            let escaped_screenshot = serde_json::to_string(&screenshot).unwrap_or_default();
            format!(
                r#"(async()=>{{try{{
const h={hash};
if(!window.__spiderSession||window.__spiderSessionHash!==h){{
window.__spiderSession=await LanguageModel.create({{
systemPrompt:{system},
temperature:{temperature},topK:{top_k},
expectedInputs:[{{type:"text",languages:["en"]}},{{type:"image"}}],
expectedOutputs:[{{type:"text",languages:["en"]}}]
}});
window.__spiderSessionHash=h;
}}
const s=window.__spiderSession;
const resp=await fetch({screenshot});
const blob=await resp.blob();
const msg=[{{role:"user",content:[{{type:"text",value:{user}}},{{type:"image",value:blob}}]}}];
return await s.prompt(msg);
}}catch(e){{try{{window.__spiderSession?.destroy();}}catch(_){{}}window.__spiderSession=null;throw e;}}
}})()"#,
                hash = sys_hash,
                system = escaped_system,
                temperature = temperature,
                top_k = top_k,
                screenshot = escaped_screenshot,
                user = escaped_user,
            )
        } else {
            format!(
                r#"(async()=>{{try{{
const h={hash};
if(!window.__spiderSession||window.__spiderSessionHash!==h){{
window.__spiderSession=await LanguageModel.create({{
systemPrompt:{system},
temperature:{temperature},topK:{top_k},
expectedInputs:[{{type:"text",languages:["en"]}}],
expectedOutputs:[{{type:"text",languages:["en"]}}]
}});
window.__spiderSessionHash=h;
}}
const s=window.__spiderSession;
const msg=[{{role:"user",content:[{{type:"text",value:{user}}}]}}];
return await s.prompt(msg);
}}catch(e){{try{{window.__spiderSession?.destroy();}}catch(_){{}}window.__spiderSession=null;throw e;}}
}})()"#,
                hash = sys_hash,
                system = escaped_system,
                temperature = temperature,
                top_k = top_k,
                user = escaped_user,
            )
        };

        // ── Evaluate on page ──
        let params = EvaluateParams::builder()
            .expression(&js)
            .await_promise(true)
            .build()
            .expect("valid evaluate params");

        let eval_result =
            tokio::time::timeout(std::time::Duration::from_secs(120), page.evaluate(params)).await;

        let content = match eval_result {
            Ok(Ok(eval)) => match eval.value() {
                Some(serde_json::Value::String(s)) => s.to_string(),
                Some(v) => v.to_string(),
                None => {
                    return Err(EngineError::Remote(
                        "chrome_ai: empty response from LanguageModel".to_string(),
                    ));
                }
            },
            Ok(Err(e)) => {
                let msg = format!("{e}");
                if Self::is_chrome_ai_missing(&msg) {
                    return Err(EngineError::Remote(format!(
                        "Chrome LanguageModel not available: {msg}. Enable chrome://flags/#optimization-guide-on-device-model and chrome://flags/#prompt-api-for-gemini-nano"
                    )));
                }
                // Session may have been invalidated (e.g. navigation); clear it
                Self::invalidate_chrome_ai_session(page).await;
                return Err(EngineError::Remote(format!("chrome_ai: eval error: {msg}")));
            }
            Err(_) => {
                // Timeout — session likely stuck; clear for next attempt
                Self::invalidate_chrome_ai_session(page).await;
                return Err(EngineError::Remote(
                    "chrome_ai: inference timed out (120s)".to_string(),
                ));
            }
        };

        log::debug!("Chrome AI response: {}", content);

        // ── Parse response ──
        let parsed = if effective_cfg.best_effort_json_extract {
            best_effort_parse_json_object(&content)?
        } else {
            serde_json::from_str(&content)?
        };

        // Normalize: small models may return simplified formats without `steps` array.
        // Convert {"action":"click","element":"sel"} → {"steps":[{"Click":"sel"}]}
        let parsed = Self::normalize_chrome_ai_response(parsed);

        let label = parsed
            .get("label")
            .and_then(|v| v.as_str())
            .unwrap_or("automation")
            .to_string();

        let done = parsed
            .get("done")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let steps = parsed
            .get("steps")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let relevant = if effective_cfg.relevance_gate {
            Some(
                parsed
                    .get("relevant")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true),
            )
        } else {
            None
        };

        let reasoning = parsed.get("reasoning").and_then(|v| {
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

        let raw_extracted = parsed.get("extracted").cloned().and_then(|v| {
            if v.as_object().is_some_and(|o| o.is_empty()) {
                None
            } else {
                Some(v)
            }
        });

        let extracted = raw_extracted.or_else(|| {
            if parsed.get("label").is_none()
                && parsed.get("done").is_none()
                && parsed.get("steps").is_none()
            {
                Some(parsed.clone())
            } else if effective_cfg.extra_ai_data {
                let mut data = serde_json::Map::new();
                if let Some(obj) = parsed.as_object() {
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
                            data.insert(key.clone(), value.clone());
                        }
                    }
                }
                if !data.is_empty() {
                    Some(serde_json::Value::Object(data))
                } else {
                    None
                }
            } else {
                None
            }
        });

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
            usage: AutomationUsage {
                llm_calls: 1,
                ..AutomationUsage::default()
            },
            relevant,
            reasoning,
        })
    }

    /// Truncate a Chrome AI user prompt by shrinking the HTML context section.
    ///
    /// Finds the `HTML CONTEXT:\n` marker and truncates only the HTML portion,
    /// preserving everything after it (task instructions, memory, etc.).
    fn truncate_chrome_ai_user_prompt(user_text: &str, max_chars: usize) -> String {
        const HTML_MARKER: &str = "HTML CONTEXT:\n";
        if let Some(html_start) = user_text.find(HTML_MARKER) {
            let html_content_start = html_start + HTML_MARKER.len();
            // Find end of HTML section — look for next section marker
            let after_html = &user_text[html_content_start..];
            let html_end = after_html
                .find("\n\nUSER INSTRUCTIONS:")
                .or_else(|| after_html.find("\n\nTASK:"))
                .or_else(|| after_html.find("\n\nMEMORY:"))
                .map(|pos| html_content_start + pos)
                .unwrap_or(user_text.len());

            let before_html = &user_text[..html_content_start];
            let html_section = &user_text[html_content_start..html_end];
            let after_section = &user_text[html_end..];

            let non_html_len = before_html.len() + after_section.len();
            if non_html_len >= max_chars {
                // Even without HTML we're over budget — truncate from tail
                return truncate_utf8_tail(user_text, max_chars);
            }
            let html_budget = max_chars - non_html_len;
            let truncated_html = truncate_utf8_tail(html_section, html_budget);
            format!("{before_html}{truncated_html}{after_section}")
        } else {
            // No HTML section found — truncate from tail as fallback
            truncate_utf8_tail(user_text, max_chars)
        }
    }

    /// Strip framework data-* attributes from text to prevent small models from
    /// hallucinating selectors like `[data-v-abc123]` from raw HTML content.
    ///
    /// Removes patterns like `data-v-XXXXX=""` and `data-reactid="N"` that
    /// Vue.js, React, and Angular inject into the DOM. Preserves `data-spider-idx`.
    fn strip_framework_data_attrs(text: &str) -> String {
        let mut result = String::with_capacity(text.len());
        let mut i = 0;
        let bytes = text.as_bytes();
        let len = bytes.len();

        while i < len {
            // Look for "data-" pattern
            if i + 5 < len && &text[i..i + 5] == "data-" {
                // Don't strip our own data-spider-idx
                if i + 15 < len && &text[i..i + 15] == "data-spider-idx" {
                    result.push_str("data-spider-idx");
                    i += 15;
                    continue;
                }
                // Check for framework patterns: data-v-, data-reactid, data-react-, data-ng-
                let rest = &text[i + 5..];
                let is_framework = rest.starts_with("v-")
                    || rest.starts_with("react")
                    || rest.starts_with("ng-")
                    || rest.starts_with("testid");
                if is_framework {
                    // Skip the entire attribute: data-xxx="..." or data-xxx='' or data-xxx
                    let attr_start = i;
                    // Skip attribute name
                    while i < len && bytes[i] != b'=' && bytes[i] != b' ' && bytes[i] != b'>' {
                        i += 1;
                    }
                    // Skip ="value" if present
                    if i < len && bytes[i] == b'=' {
                        i += 1; // skip =
                        if i < len && (bytes[i] == b'"' || bytes[i] == b'\'') {
                            let quote = bytes[i];
                            i += 1; // skip opening quote
                            while i < len && bytes[i] != quote {
                                i += 1;
                            }
                            if i < len {
                                i += 1; // skip closing quote
                            }
                        }
                    }
                    // Skip trailing whitespace if the attr was preceded by a space
                    if attr_start > 0
                        && text.as_bytes()[attr_start - 1] == b' '
                        && i < len
                        && bytes[i] == b' '
                    {
                        i += 1;
                    }
                    continue;
                }
            }
            result.push(text[i..].chars().next().unwrap_or(' '));
            i += text[i..].chars().next().map_or(1, |c| c.len_utf8());
        }

        result
    }

    /// Normalize Chrome AI responses from small models that may not follow the
    /// exact output schema.
    ///
    /// Handles common simplified formats:
    /// - `{"action":"click","element":"sel"}` → `{"label":"click sel","done":false,"steps":[{"Click":"sel"}]}`
    /// - `{"action":"fill","element":"sel","value":"text"}` → `{"steps":[{"Fill":{"selector":"sel","value":"text"}}]}`
    /// - `{"action":"scroll","value":300}` → `{"steps":[{"ScrollY":300}]}`
    /// - `{"action":"navigate","url":"..."}` → `{"steps":[{"Navigate":"..."}]}`
    fn normalize_chrome_ai_response(parsed: serde_json::Value) -> serde_json::Value {
        // If it already has a `steps` array, return as-is
        if parsed.get("steps").and_then(|v| v.as_array()).is_some() {
            return parsed;
        }

        let obj = match parsed.as_object() {
            Some(o) => o,
            None => return parsed,
        };

        // Check if any value is a step-like object: {"Click":"sel"}, {"Fill":{...}}, etc.
        // Small models sometimes wrap steps like: {"action": {"Click":"[0]"}, "label":"..."}
        static KNOWN_ACTIONS: &[&str] = &[
            "Click",
            "ClickPoint",
            "ClickAll",
            "Fill",
            "Press",
            "ScrollY",
            "ScrollTo",
            "Navigate",
            "Wait",
            "WaitFor",
            "Evaluate",
            "SetViewport",
            "ClickDragPoint",
        ];
        for (_key, val) in obj {
            if let Some(step_obj) = val.as_object() {
                if step_obj.len() == 1 {
                    if let Some(action_name) = step_obj.keys().next() {
                        if KNOWN_ACTIONS.iter().any(|a| a == action_name) {
                            // Found an embedded step — normalize index references in the value
                            let mut step = val.clone();
                            if let Some(inner) = step
                                .as_object_mut()
                                .and_then(|o| o.get_mut(action_name))
                            {
                                if let Some(s) = inner.as_str() {
                                    let t = s
                                        .trim()
                                        .trim_start_matches('[')
                                        .trim_end_matches(']');
                                    if !t.is_empty()
                                        && t.len() <= 2
                                        && t.chars().all(|c| c.is_ascii_digit())
                                    {
                                        *inner = serde_json::json!(format!(
                                            "[data-spider-idx='{t}']"
                                        ));
                                    }
                                }
                            }
                            let label = obj
                                .get("label")
                                .and_then(|v| v.as_str())
                                .unwrap_or("automation");
                            let done = obj
                                .get("done")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false);
                            log::debug!(
                                "Normalized Chrome AI response: embedded step {:?}",
                                step
                            );
                            return serde_json::json!({
                                "label": label,
                                "done": done,
                                "steps": [step],
                            });
                        }
                    }
                }
            }
        }

        // Extract simplified fields — small models use various field names
        let mut action = obj
            .get("action")
            .or_else(|| obj.get("action_type"))
            .or_else(|| obj.get("type"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_lowercase();

        // Infer action from "label" when no explicit action field is present.
        // Small models often put the action name in "label": "Click", "Scroll", etc.
        if action.is_empty() {
            if let Some(label) = obj.get("label").and_then(|v| v.as_str()) {
                let lower = label.to_lowercase();
                if lower.starts_with("click") || lower.starts_with("tap") {
                    action = "click".to_string();
                } else if lower.starts_with("evaluat") {
                    action = "evaluate".to_string();
                } else if lower.starts_with("scroll") {
                    action = "scroll".to_string();
                } else if lower.starts_with("fill") || lower.starts_with("type") {
                    action = "fill".to_string();
                } else if lower.starts_with("press") || lower.starts_with("key") {
                    action = "press".to_string();
                } else if lower.starts_with("wait") {
                    action = "wait".to_string();
                } else if lower.starts_with("navigate") || lower.starts_with("go") {
                    action = "navigate".to_string();
                }
            }
        }

        // Helper: check if a string looks like a CSS selector or index reference
        fn looks_like_selector(s: &str) -> bool {
            let t = s.trim();
            t.starts_with('.')
                || t.starts_with('#')
                || t.starts_with('[')
                || t.contains("data-spider-idx")
        }
        // Extract a numeric index from strings like "0", "[0]", "[0] link", etc.
        fn extract_index_from_str(s: &str) -> Option<u64> {
            let t = s.trim();
            // Direct number: "0", "12"
            if let Ok(n) = t.parse::<u64>() {
                return Some(n);
            }
            // Bracketed: "[0]", "[0] link", "[12] button"
            if t.starts_with('[') {
                if let Some(end) = t.find(']') {
                    let inner = &t[1..end];
                    if let Ok(n) = inner.trim().parse::<u64>() {
                        return Some(n);
                    }
                }
            }
            None
        }

        // Helper: extract a numeric index from any JSON value.
        // Handles: 0, "0", "[0]", "[0] link", [0], ["0"]
        fn extract_index(v: &serde_json::Value) -> Option<u64> {
            if let Some(n) = v.as_u64() {
                return Some(n);
            }
            if let Some(s) = v.as_str() {
                return extract_index_from_str(s);
            }
            // Array with a single element: [0] → 0, ["0"] → 0
            if let Some(arr) = v.as_array() {
                if arr.len() == 1 {
                    if let Some(n) = arr[0].as_u64() {
                        return Some(n);
                    }
                    if let Some(s) = arr[0].as_str() {
                        return extract_index_from_str(s);
                    }
                }
            }
            None
        }

        // Search for element/index across ALL values in the response.
        // Small models use arbitrary field names — we scan everything.
        let raw_element = {
            let mut found = String::new();

            // 1. Scan all values: prefer selectors, then indexes, then bare names
            for (key, val) in obj {
                // Skip metadata fields
                if matches!(
                    key.as_str(),
                    "label" | "action" | "action_type" | "type" | "done"
                        | "extracted" | "memory_ops" | "reasoning"
                ) {
                    continue;
                }

                // Check for selector-like strings
                if let Some(s) = val.as_str() {
                    if looks_like_selector(s) {
                        found = s.to_string();
                        break;
                    }
                }

                // Check for index values (number, string, or array)
                if let Some(idx) = extract_index(val) {
                    if idx < 100 {
                        found = format!("[data-spider-idx='{idx}']");
                        break;
                    }
                }

                // Check for bare element names (strings with hyphens)
                if found.is_empty() {
                    if let Some(s) = val.as_str() {
                        if !s.is_empty() && s.contains('-') && !s.contains(' ') {
                            found = s.to_string();
                            // Don't break — keep looking for better matches
                        }
                    }
                }
            }

            // Convert index-like string values to data-spider-idx selectors
            if let Some(idx) = extract_index_from_str(&found) {
                format!("[data-spider-idx='{idx}']")
            } else {
                found
            }
        };

        let raw_element = raw_element.as_str();

        // If the element looks like a bare name (no CSS prefix), try both class and ID.
        // Small models often output "my-class" instead of ".my-class" or "#my-class".
        // CSS comma selectors let querySelector match whichever exists.
        let element = if !raw_element.is_empty()
            && !raw_element.starts_with('.')
            && !raw_element.starts_with('#')
            && !raw_element.starts_with('[')
            && !raw_element.contains(' ')
            && !raw_element.contains('>')
            && !raw_element.contains(':')
            && !raw_element.contains('/')
            && raw_element.contains('-')
        {
            format!(".{raw_element}, #{raw_element}")
        } else {
            raw_element.to_string()
        };
        let element = element.as_str();
        let value = obj.get("value");
        let url = obj
            .get("url")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if action.is_empty() && element.is_empty() {
            return parsed;
        }

        // Build the step
        let step: serde_json::Value = match action.as_str() {
            "click" | "tap" if !element.is_empty() => {
                serde_json::json!({ "Click": element })
            }
            "fill" | "type" | "input" if !element.is_empty() => {
                let val = value
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                serde_json::json!({ "Fill": { "selector": element, "value": val } })
            }
            "scroll" | "scrolldown" | "scrollup" => {
                let amount = value
                    .and_then(|v| v.as_i64())
                    .unwrap_or(300);
                serde_json::json!({ "ScrollY": amount })
            }
            "navigate" | "goto" if !url.is_empty() => {
                serde_json::json!({ "Navigate": url })
            }
            "wait" => {
                let ms = value.and_then(|v| v.as_u64()).unwrap_or(1000);
                serde_json::json!({ "Wait": ms })
            }
            "press" => {
                let key = value
                    .and_then(|v| v.as_str())
                    .unwrap_or("Enter");
                serde_json::json!({ "Press": key })
            }
            // Evaluate: small models sometimes emit {"label":"Evaluate","selector":"..."}
            // Build a simple DOM-read JS snippet or pass through the element as a selector read.
            "evaluate" | "eval" => {
                let js_code = if let Some(v) = value.and_then(|v| v.as_str()) {
                    v.to_string()
                } else if !element.is_empty() {
                    format!("document.title=document.querySelector('{}')?.textContent?.trim()?.slice(0,200)||'empty'", element.replace('\'', "\\'"))
                } else {
                    "document.title=document.body?.innerText?.slice(0,200)||'empty'".to_string()
                };
                serde_json::json!({ "Evaluate": js_code })
            }
            // If action is unrecognized but element is a valid selector, try Click
            _ if !element.is_empty() => {
                serde_json::json!({ "Click": element })
            }
            _ => return parsed,
        };

        let label = obj
            .get("label")
            .or_else(|| obj.get("action_taken"))
            .or_else(|| obj.get("description"))
            .and_then(|v| v.as_str())
            .unwrap_or("automation");
        let done = obj
            .get("done")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        log::debug!(
            "Normalized Chrome AI response: action={}, element={} → {:?}",
            action,
            element,
            step
        );

        let mut result = serde_json::json!({
            "label": label,
            "done": done,
            "steps": [step],
        });

        // Preserve any extracted data
        if let Some(extracted) = obj.get("extracted") {
            result
                .as_object_mut()
                .unwrap()
                .insert("extracted".to_string(), extracted.clone());
        }
        if let Some(mem_ops) = obj.get("memory_ops") {
            result
                .as_object_mut()
                .unwrap()
                .insert("memory_ops".to_string(), mem_ops.clone());
        }

        result
    }

    /// Execute automation steps on the page.
    ///
    /// Handles WebAutomation enum-style actions like `{ "Click": "selector" }`.
    /// Returns (steps_executed, spawn_pages) where spawn_pages contains URLs
    /// from `OpenPage` actions that should be opened in new browser tabs.
    async fn execute_steps(
        &self,
        page: &Page,
        steps: &[serde_json::Value],
        _cfg: &RemoteMultimodalConfig,
    ) -> EngineResult<(usize, Vec<String>)> {
        let mut executed = 0;
        let mut spawn_pages = Vec::new();

        for step in steps {
            log::debug!("Executing step: {:?}", step);
            // Handle WebAutomation enum-style format: { "ActionName": value }
            if let Some(obj) = step.as_object() {
                for (action, value) in obj {
                    log::debug!("Action: {}, Value: {:?}", action, value);
                    // Handle OpenPage specially - collect URLs instead of executing
                    if action == "OpenPage" {
                        log::info!("OpenPage action detected: {:?}", value);
                        if let Some(url) = value.as_str() {
                            spawn_pages.push(url.to_string());
                            executed += 1;
                        } else if let Some(urls) = value.as_array() {
                            // Support array of URLs: { "OpenPage": ["url1", "url2"] }
                            for url_val in urls {
                                if let Some(url) = url_val.as_str() {
                                    spawn_pages.push(url.to_string());
                                }
                            }
                            executed += 1;
                        }
                        continue;
                    }

                    let success = self.execute_single_action(page, action, value).await;
                    if success {
                        executed += 1;
                    }
                }
            }
        }

        Ok((executed, spawn_pages))
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
                // Move mouse first to set hover target, then click
                let point = Point::new(x, y);
                let _ = page.move_mouse_smooth(point).await;
                let _ = page.click(point).await;
                true
            }
            "ClickHold" => {
                let selector = value.get("selector").and_then(|v| v.as_str());
                let hold_ms = value.get("hold_ms").and_then(|v| v.as_u64()).unwrap_or(500);
                if let Some(sel) = selector {
                    if let Ok(elem) = page.find_element(sel).await {
                        if let Ok(sv) = elem.scroll_into_view().await {
                            if let Ok(point) = sv.clickable_point().await {
                                let _ = page.move_mouse_smooth(point).await;
                                let _ = page
                                    .click_and_hold(
                                        point,
                                        std::time::Duration::from_millis(hold_ms),
                                    )
                                    .await;
                                return true;
                            }
                        }
                    }
                }
                false
            }
            "ClickHoldPoint" => {
                let x = value.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let y = value.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let hold_ms = value.get("hold_ms").and_then(|v| v.as_u64()).unwrap_or(500);
                let point = Point::new(x, y);
                let _ = page.move_mouse_smooth(point).await;
                let _ = page
                    .click_and_hold(point, std::time::Duration::from_millis(hold_ms))
                    .await;
                true
            }
            "DoubleClick" => {
                if let Some(selector) = value.as_str() {
                    if let Ok(elem) = page.find_element(selector).await {
                        if let Ok(sv) = elem.scroll_into_view().await {
                            if let Ok(point) = sv.clickable_point().await {
                                let _ = page.move_mouse_smooth(point).await;
                                let _ = page.double_click(point).await;
                                return true;
                            }
                        }
                    }
                }
                false
            }
            "DoubleClickPoint" => {
                let x = value.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let y = value.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let point = Point::new(x, y);
                let _ = page.move_mouse_smooth(point).await;
                let _ = page.double_click(point).await;
                true
            }
            "RightClick" => {
                if let Some(selector) = value.as_str() {
                    if let Ok(elem) = page.find_element(selector).await {
                        if let Ok(sv) = elem.scroll_into_view().await {
                            if let Ok(point) = sv.clickable_point().await {
                                let _ = page.move_mouse_smooth(point).await;
                                let _ = page.right_click(point).await;
                                return true;
                            }
                        }
                    }
                }
                false
            }
            "RightClickPoint" => {
                let x = value.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let y = value.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let point = Point::new(x, y);
                let _ = page.move_mouse_smooth(point).await;
                let _ = page.right_click(point).await;
                true
            }
            "ClickAllClickable" => {
                if let Ok(elements) = page
                    .find_elements(r#"a, button, [onclick], [role="button"]"#)
                    .await
                {
                    for elem in elements {
                        let _ = elem.click().await;
                    }
                }
                true
            }

            // === Drag Actions ===
            "ClickDrag" => {
                let from = value.get("from").and_then(|v| v.as_str());
                let to = value.get("to").and_then(|v| v.as_str());
                if let (Some(from_sel), Some(to_sel)) = (from, to) {
                    if let Ok(from_elem) = page.find_element(from_sel).await {
                        if let Ok(from_sv) = from_elem.scroll_into_view().await {
                            if let Ok(from_point) = from_sv.clickable_point().await {
                                if let Ok(to_elem) = page.find_element(to_sel).await {
                                    if let Ok(to_sv) = to_elem.scroll_into_view().await {
                                        if let Ok(to_point) = to_sv.clickable_point().await {
                                            let _ = page
                                                .click_and_drag_smooth(from_point, to_point)
                                                .await;
                                            return true;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                false
            }
            "ClickDragPoint" => {
                let from_x = value.get("from_x").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let from_y = value.get("from_y").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let to_x = value.get("to_x").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let to_y = value.get("to_y").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let _ = page
                    .click_and_drag_smooth(Point::new(from_x, from_y), Point::new(to_x, to_y))
                    .await;
                true
            }

            // === Input Actions ===
            "Fill" => {
                let selector = value.get("selector").and_then(|v| v.as_str());
                let text = value.get("value").and_then(|v| v.as_str());
                if let (Some(sel), Some(txt)) = (selector, text) {
                    if let Ok(elem) = page.find_element(sel).await {
                        let _ = elem.click().await;
                        // Clear existing value before typing
                        let _ = page
                            .evaluate(format!(
                                "document.querySelector('{}').value = ''",
                                sel.replace('\'', "\\'")
                            ))
                            .await;
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
                    let _ = page
                        .evaluate(format!(
                            "document.activeElement.value += '{}'",
                            txt.replace('\'', "\\'")
                        ))
                        .await;
                    return true;
                }
                false
            }
            "Clear" => {
                if let Some(selector) = value.as_str() {
                    let _ = page
                        .evaluate(format!("document.querySelector('{}').value = ''", selector))
                        .await;
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
                let _ = page
                    .evaluate(format!("window.scrollBy({}, 0)", pixels))
                    .await;
                true
            }
            "ScrollY" => {
                let pixels = value.as_i64().unwrap_or(300);
                let _ = page
                    .evaluate(format!("window.scrollBy(0, {})", pixels))
                    .await;
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
                let _ = page
                    .evaluate(format!("window.scrollTo({}, {})", x, y))
                    .await;
                true
            }
            "InfiniteScroll" => {
                let max_scrolls = value.as_u64().unwrap_or(5);
                for _ in 0..max_scrolls {
                    let _ = page
                        .evaluate("window.scrollTo(0, document.body.scrollHeight)")
                        .await;
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
                let timeout = value
                    .get("timeout")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(5000);
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
                let timeout = value
                    .get("timeout")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(5000);
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
                let _ = page.move_mouse_smooth(Point::new(x, y)).await;
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
                    let _ = page
                        .evaluate(format!("document.querySelector('{}')?.focus()", selector))
                        .await;
                    return true;
                }
                false
            }
            "Blur" => {
                if let Some(selector) = value.as_str() {
                    let _ = page
                        .evaluate(format!("document.querySelector('{}')?.blur()", selector))
                        .await;
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

            // === Viewport / Device Metrics ===
            "SetViewport" => {
                if let Some(obj) = value.as_object() {
                    let width = obj.get("width").and_then(|v| v.as_i64()).unwrap_or(1280);
                    let height = obj.get("height").and_then(|v| v.as_i64()).unwrap_or(960);
                    let device_scale_factor = obj
                        .get("device_scale_factor")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(2.0);
                    let mobile = obj.get("mobile").and_then(|v| v.as_bool()).unwrap_or(false);

                    use chromiumoxide::cdp::browser_protocol::emulation::SetDeviceMetricsOverrideParams;
                    let params = SetDeviceMetricsOverrideParams::new(
                        width,
                        height,
                        device_scale_factor,
                        mobile,
                    );
                    let _ = page.execute(params).await;
                    log::debug!(
                        "SetViewport: {}x{} @ {}x DPR",
                        width,
                        height,
                        device_scale_factor
                    );
                    return true;
                }
                false
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

        static CLIENT: std::sync::LazyLock<reqwest::Client> = std::sync::LazyLock::new(|| {
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
        let usage = extract_usage(&body);

        let step = best_effort_parse_json_object(&content)?;

        // Execute the action
        let (steps_executed, _spawn_pages) = self
            .execute_steps(page, std::slice::from_ref(&step), &self.cfg)
            .await?;

        let action_type = step
            .get("action")
            .and_then(|v| v.as_str())
            .map(String::from);
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

        static CLIENT: std::sync::LazyLock<reqwest::Client> = std::sync::LazyLock::new(|| {
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
        let usage = extract_usage(&body);

        let parsed = best_effort_parse_json_object(&content)?;

        // Build PageObservation from parsed JSON
        let mut obs = PageObservation::new(&url)
            .with_title(&title)
            .with_usage(usage);

        if let Some(desc) = parsed.get("description").and_then(|v| v.as_str()) {
            obs = obs.with_description(desc);
        }

        if let Some(pt) = parsed.get("page_type").and_then(|v| v.as_str()) {
            obs = obs.with_page_type(pt);
        }

        // Parse interactive elements
        if let Some(elements) = parsed
            .get("interactive_elements")
            .and_then(|v| v.as_array())
        {
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
    engine.with_vision_model(cfgs.vision_model.clone());
    engine.with_text_model(cfgs.text_model.clone());
    engine.with_vision_route_mode(cfgs.vision_route_mode);
    engine.with_chrome_ai(cfgs.use_chrome_ai);
    engine.with_chrome_ai_max_user_chars(cfgs.chrome_ai_max_user_chars);
    #[cfg(feature = "skills")]
    if let Some(ref registry) = cfgs.skill_registry {
        engine.with_skill_registry(Some(registry.clone()));
    }

    // Enable session memory for multi-round automation so memory_ops,
    // level-attempt tracking, and force-refresh recovery all work.
    if cfgs.cfg.max_rounds > 1 {
        let mut mem = super::AutomationMemory::new();
        engine.run_with_memory(page, url, Some(&mut mem)).await
    } else {
        engine.run(page, url).await
    }
}

/// Result from processing a spawned page.
#[cfg(feature = "chrome")]
#[derive(Debug, Clone)]
pub struct SpawnedPageResult {
    /// The URL that was opened.
    pub url: String,
    /// The automation result from the page.
    pub result: Result<AutomationResult, String>,
    /// Total bytes transferred for this page (from network events).
    pub bytes_transferred: Option<f64>,
    /// Map of request IDs to bytes transferred (for detailed network tracking).
    pub response_map: Option<std::collections::HashMap<String, f64>>,
}

#[cfg(feature = "chrome")]
impl SpawnedPageResult {
    /// Check if the page was processed successfully.
    pub fn is_ok(&self) -> bool {
        self.result.is_ok()
    }

    /// Get the extracted data from this page, if any.
    pub fn extracted(&self) -> Option<&serde_json::Value> {
        self.result.as_ref().ok().and_then(|r| r.extracted.as_ref())
    }

    /// Get the screenshot from this page, if any (base64 encoded).
    pub fn screenshot(&self) -> Option<&str> {
        self.result
            .as_ref()
            .ok()
            .and_then(|r| r.screenshot.as_deref())
    }

    /// Get the label/description from this page.
    pub fn label(&self) -> Option<&str> {
        self.result.as_ref().ok().map(|r| r.label.as_str())
    }

    /// Get the error message if the page failed.
    pub fn error(&self) -> Option<&str> {
        self.result.as_ref().err().map(|s| s.as_str())
    }

    /// Get any additional spawn_pages from this page (for recursive crawling).
    pub fn spawn_pages(&self) -> Option<&[String]> {
        self.result.as_ref().ok().map(|r| r.spawn_pages.as_slice())
    }

    /// Get the token usage from this page.
    pub fn usage(&self) -> Option<&super::AutomationUsage> {
        self.result.as_ref().ok().map(|r| &r.usage)
    }
}

/// Callback type for setting up event tracking on spawned pages.
///
/// This allows spider to propagate ChromeEventTracker or similar tracking
/// from the main page to spawned pages.
#[cfg(feature = "chrome")]
pub type PageSetupFn = Box<
    dyn Fn(&Page) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> + Send + Sync,
>;

/// Options for configuring spawned page automation.
///
/// Configure extraction prompts, screenshots, and event tracking propagation
/// for pages spawned from `OpenPage` actions.
#[cfg(feature = "chrome")]
#[derive(Default)]
pub struct SpawnPageOptions {
    /// Custom extraction prompt for each page.
    /// If set, enables extraction mode and uses this prompt.
    pub extraction_prompt: Option<String>,
    /// Whether to capture screenshots from each page.
    pub screenshot: bool,
    /// Maximum rounds of automation per page.
    pub max_rounds: usize,
    /// Additional user message to append to each page's automation.
    pub user_message_extra: Option<String>,
    /// Optional callback to setup event tracking on each spawned page.
    /// Use this to propagate network event tracking from the main page.
    pub page_setup: Option<std::sync::Arc<PageSetupFn>>,
    /// Whether to track bytes transferred via CDP network events.
    pub track_bytes: bool,
}

#[cfg(feature = "chrome")]
impl SpawnPageOptions {
    /// Create new options with defaults (screenshot enabled, 1 round).
    pub fn new() -> Self {
        Self {
            extraction_prompt: None,
            screenshot: true,
            max_rounds: 1,
            user_message_extra: None,
            page_setup: None,
            track_bytes: false,
        }
    }

    /// Enable extraction with a custom prompt.
    pub fn with_extraction(mut self, prompt: impl Into<String>) -> Self {
        self.extraction_prompt = Some(prompt.into());
        self
    }

    /// Enable or disable screenshots.
    pub fn with_screenshot(mut self, enabled: bool) -> Self {
        self.screenshot = enabled;
        self
    }

    /// Set maximum automation rounds per page.
    pub fn with_max_rounds(mut self, rounds: usize) -> Self {
        self.max_rounds = rounds;
        self
    }

    /// Add extra user instructions for each page.
    pub fn with_user_message(mut self, message: impl Into<String>) -> Self {
        self.user_message_extra = Some(message.into());
        self
    }

    /// Set a page setup callback for event tracking propagation.
    /// This callback is called on each spawned page to setup event listeners,
    /// network tracking, etc. from the parent page context.
    ///
    /// # Example
    /// ```ignore
    /// use std::sync::Arc;
    ///
    /// let options = SpawnPageOptions::new()
    ///     .with_page_setup(Arc::new(|page: &Page| {
    ///         Box::pin(async move {
    ///             // Setup event tracking on this page
    ///             // e.g., setup_chrome_events(page, tracker).await;
    ///         })
    ///     }));
    /// ```
    pub fn with_page_setup(mut self, setup: std::sync::Arc<PageSetupFn>) -> Self {
        self.page_setup = Some(setup);
        self
    }

    /// Enable bytes tracking via CDP network events.
    /// When enabled, each spawned page will track bytes transferred and
    /// populate `SpawnedPageResult.bytes_transferred` and `response_map`.
    pub fn with_track_bytes(mut self, enabled: bool) -> Self {
        self.track_bytes = enabled;
        self
    }
}

// Manual Debug implementation since PageSetupFn doesn't implement Debug
#[cfg(feature = "chrome")]
impl std::fmt::Debug for SpawnPageOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpawnPageOptions")
            .field("extraction_prompt", &self.extraction_prompt)
            .field("screenshot", &self.screenshot)
            .field("max_rounds", &self.max_rounds)
            .field("user_message_extra", &self.user_message_extra)
            .field("page_setup", &self.page_setup.as_ref().map(|_| "<fn>"))
            .field("track_bytes", &self.track_bytes)
            .finish()
    }
}

// Manual Clone implementation
#[cfg(feature = "chrome")]
impl Clone for SpawnPageOptions {
    fn clone(&self) -> Self {
        Self {
            extraction_prompt: self.extraction_prompt.clone(),
            screenshot: self.screenshot,
            max_rounds: self.max_rounds,
            user_message_extra: self.user_message_extra.clone(),
            page_setup: self.page_setup.clone(),
            track_bytes: self.track_bytes,
        }
    }
}

/// Process spawn_pages URLs concurrently with a browser.
///
/// This function takes URLs from `AutomationResult.spawn_pages` and runs
/// automation on each in a new browser page concurrently.
///
/// # Arguments
/// * `browser` - The browser to create new pages from
/// * `urls` - URLs to open (typically from `result.spawn_pages`)
/// * `cfgs` - Configuration for the automation engine
///
/// # Returns
/// A vector of results, one for each URL, in completion order.
///
/// # Example
/// ```ignore
/// let result = run_remote_multimodal_with_page(&config, &page, url).await?;
/// if result.has_spawn_pages() {
///     let spawn_results = run_spawn_pages_concurrent(
///         &browser,
///         result.spawn_pages,
///         &config,
///     ).await;
///     for spawn_result in spawn_results {
///         println!("{}: {:?}", spawn_result.url, spawn_result.result);
///     }
/// }
/// ```
#[cfg(feature = "chrome")]
pub async fn run_spawn_pages_concurrent(
    browser: &std::sync::Arc<chromiumoxide::browser::Browser>,
    urls: Vec<String>,
    cfgs: &super::RemoteMultimodalConfigs,
) -> Vec<SpawnedPageResult> {
    run_spawn_pages_with_options(browser, urls, cfgs, SpawnPageOptions::new()).await
}

/// Process spawn_pages URLs concurrently with custom options.
///
/// This is the full-featured version that allows customizing extraction,
/// screenshots, and other options for each spawned page.
///
/// # Arguments
/// * `browser` - The browser to create new pages from
/// * `urls` - URLs to open (typically from `result.spawn_pages`)
/// * `base_cfgs` - Base configuration (API URL, model, etc.)
/// * `options` - Options for extraction, screenshots, etc.
///
/// # Example
/// ```ignore
/// let options = SpawnPageOptions::new()
///     .with_extraction("Extract the main content, title, and any links")
///     .with_screenshot(true)
///     .with_max_rounds(2);
///
/// let spawn_results = run_spawn_pages_with_options(
///     &browser,
///     result.spawn_pages,
///     &config,
///     options,
/// ).await;
///
/// for spawn_result in &spawn_results {
///     if let Some(data) = spawn_result.extracted() {
///         println!("Extracted from {}: {}", spawn_result.url, data);
///     }
///     if let Some(screenshot) = spawn_result.screenshot() {
///         // Save screenshot (base64 encoded PNG)
///         println!("Got screenshot from {} ({} bytes)", spawn_result.url, screenshot.len());
///     }
/// }
/// ```
#[cfg(feature = "chrome")]
pub async fn run_spawn_pages_with_options(
    browser: &std::sync::Arc<chromiumoxide::browser::Browser>,
    urls: Vec<String>,
    base_cfgs: &super::RemoteMultimodalConfigs,
    options: SpawnPageOptions,
) -> Vec<SpawnedPageResult> {
    use chromiumoxide::cdp::browser_protocol::network::EventDataReceived;
    use dashmap::DashMap;
    use futures::StreamExt;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    if urls.is_empty() {
        return Vec::new();
    }

    let base_cfgs = Arc::new(base_cfgs.clone());
    let options = Arc::new(options);
    let mut handles = Vec::with_capacity(urls.len());

    // Spawn all pages concurrently
    for url in urls {
        let browser = browser.clone();
        let base_cfgs = base_cfgs.clone();
        let options = options.clone();
        let url_clone = url.clone();

        handles.push(tokio::spawn(async move {
            // Create new page for this URL
            let new_page = match browser.new_page(&url_clone).await {
                Ok(page) => page,
                Err(e) => {
                    return SpawnedPageResult {
                        url: url_clone,
                        result: Err(format!("Failed to create page: {}", e)),
                        bytes_transferred: None,
                        response_map: None,
                    };
                }
            };

            // Run page setup callback if provided (for event tracking propagation)
            if let Some(ref setup) = options.page_setup {
                setup(&new_page).await;
            }

            // Set up bytes tracking if enabled
            let total_bytes: Arc<AtomicU64> = Arc::new(AtomicU64::new(0));
            let response_map: Arc<DashMap<String, u64>> = Arc::new(DashMap::new());

            let bytes_listener = if options.track_bytes {
                new_page.event_listener::<EventDataReceived>().await.ok()
            } else {
                None
            };

            // Spawn bytes tracking task if we have a listener
            let tracking_handle = if let Some(mut listener) = bytes_listener {
                let total_bytes = total_bytes.clone();
                let response_map = response_map.clone();
                Some(tokio::spawn(async move {
                    while let Some(event) = listener.next().await {
                        let bytes = event.encoded_data_length as u64;
                        total_bytes.fetch_add(bytes, Ordering::Relaxed);
                        response_map
                            .entry(event.request_id.inner().to_string())
                            .and_modify(|v| *v += bytes)
                            .or_insert(bytes);
                    }
                }))
            } else {
                None
            };

            // Build page-specific config from the full base config to preserve
            // routing, schemas, relevance gates, and other production knobs.
            let mut page_cfgs = (*base_cfgs).clone();
            let mut page_cfg = page_cfgs.cfg.clone();
            page_cfg.max_rounds = options.max_rounds.max(1);
            page_cfg.screenshot = options.screenshot;

            // Enable extraction if prompt is provided
            if let Some(ref prompt) = options.extraction_prompt {
                page_cfg.extra_ai_data = true;
                page_cfg.extraction_prompt = Some(prompt.clone());
            }

            page_cfgs.cfg = page_cfg;

            // Add/override user message extra if provided
            if let Some(ref msg) = options.user_message_extra {
                page_cfgs.user_message_extra = Some(msg.clone());
            }

            // Run automation on the new page
            let result = run_remote_multimodal_with_page(&page_cfgs, &new_page, &url_clone)
                .await
                .map_err(|e| format!("Automation failed: {}", e));

            // Stop bytes tracking
            if let Some(handle) = tracking_handle {
                handle.abort();
            }

            // Collect bytes data
            let (bytes_transferred, response_map_out) = {
                let bytes_val = total_bytes.load(Ordering::Relaxed);
                let bytes = if bytes_val > 0 {
                    Some(bytes_val as f64)
                } else {
                    None
                };
                let map = if !response_map.is_empty() {
                    Some(
                        response_map
                            .iter()
                            .map(|e| (e.key().clone(), *e.value() as f64))
                            .collect(),
                    )
                } else {
                    None
                };
                (bytes, map)
            };

            SpawnedPageResult {
                url: url_clone,
                result,
                bytes_transferred,
                response_map: response_map_out,
            }
        }));
    }

    // Collect all results (concurrent execution, wait for all)
    let mut results = Vec::with_capacity(handles.len());
    for handle in handles {
        match handle.await {
            Ok(spawn_result) => results.push(spawn_result),
            Err(e) => {
                // JoinError - task panicked or was cancelled
                results.push(SpawnedPageResult {
                    url: "unknown".to_string(),
                    result: Err(format!("Task failed: {}", e)),
                    bytes_transferred: None,
                    response_map: None,
                });
            }
        }
    }

    results
}

/// Page factory type for creating new pages.
/// The factory receives a URL and should return a new Page navigated to that URL.
#[cfg(feature = "chrome")]
pub type PageFactory<E> = Box<
    dyn Fn(String) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Page, E>> + Send>>
        + Send
        + Sync,
>;

/// Process spawn_pages URLs concurrently using a page factory function.
///
/// This version allows spider or other consumers to provide their own method
/// for creating pages, rather than requiring direct Browser access. This is
/// useful when the entry point only has access to a Page reference.
///
/// # Arguments
/// * `page_factory` - A function that creates new pages from URLs
/// * `urls` - URLs to open (typically from `result.spawn_pages`)
/// * `base_cfgs` - Base configuration (API URL, model, etc.)
/// * `options` - Options for extraction, screenshots, event tracking, etc.
///
/// # Example
/// ```ignore
/// use std::sync::Arc;
///
/// // Create a page factory from the browser
/// let browser = Arc::clone(&browser);
/// let page_factory: PageFactory<String> = Box::new(move |url| {
///     let browser = browser.clone();
///     Box::pin(async move {
///         browser.new_page(&url).await.map_err(|e| e.to_string())
///     })
/// });
///
/// let options = SpawnPageOptions::new()
///     .with_extraction("Extract page content")
///     .with_page_setup(Arc::new(|page| {
///         Box::pin(async move {
///             // Setup event tracking propagated from main page
///         })
///     }));
///
/// let spawn_results = run_spawn_pages_with_factory(
///     Arc::new(page_factory),
///     result.spawn_pages,
///     &config,
///     options,
/// ).await;
/// ```
#[cfg(feature = "chrome")]
pub async fn run_spawn_pages_with_factory<E: std::fmt::Display + Send + 'static>(
    page_factory: std::sync::Arc<PageFactory<E>>,
    urls: Vec<String>,
    base_cfgs: &super::RemoteMultimodalConfigs,
    options: SpawnPageOptions,
) -> Vec<SpawnedPageResult> {
    use std::sync::Arc;

    if urls.is_empty() {
        return Vec::new();
    }

    let base_cfgs = Arc::new(base_cfgs.clone());
    let options = Arc::new(options);
    let mut handles = Vec::with_capacity(urls.len());

    // Spawn all pages concurrently
    for url in urls {
        let page_factory = page_factory.clone();
        let base_cfgs = base_cfgs.clone();
        let options = options.clone();
        let url_clone = url.clone();

        handles.push(tokio::spawn(async move {
            let result = async {
                // Create new page using the factory
                let new_page = page_factory(url_clone.clone())
                    .await
                    .map_err(|e| format!("Failed to create page: {}", e))?;

                // Run page setup callback if provided (for event tracking propagation)
                if let Some(ref setup) = options.page_setup {
                    setup(&new_page).await;
                }

                // Build page-specific config from the full base config to preserve
                // routing, schemas, relevance gates, and other production knobs.
                let mut page_cfgs = (*base_cfgs).clone();
                let mut page_cfg = page_cfgs.cfg.clone();
                page_cfg.max_rounds = options.max_rounds.max(1);
                page_cfg.screenshot = options.screenshot;

                // Enable extraction if prompt is provided
                if let Some(ref prompt) = options.extraction_prompt {
                    page_cfg.extra_ai_data = true;
                    page_cfg.extraction_prompt = Some(prompt.clone());
                }

                page_cfgs.cfg = page_cfg;

                // Add/override user message extra if provided
                if let Some(ref msg) = options.user_message_extra {
                    page_cfgs.user_message_extra = Some(msg.clone());
                }

                // Run automation on the new page
                run_remote_multimodal_with_page(&page_cfgs, &new_page, &url_clone)
                    .await
                    .map_err(|e| format!("Automation failed: {}", e))
            }
            .await;

            SpawnedPageResult {
                url: url_clone,
                result,
                bytes_transferred: None, // Set by caller if needed
                response_map: None,
            }
        }));
    }

    // Collect all results (concurrent execution, wait for all)
    let mut results = Vec::with_capacity(handles.len());
    for handle in handles {
        match handle.await {
            Ok(spawn_result) => results.push(spawn_result),
            Err(e) => {
                // JoinError - task panicked or was cancelled
                results.push(SpawnedPageResult {
                    url: "unknown".to_string(),
                    result: Err(format!("Task failed: {}", e)),
                    bytes_transferred: None,
                    response_map: None,
                });
            }
        }
    }

    results
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::automation::DEFAULT_SYSTEM_PROMPT;

    #[test]
    fn test_spawn_page_options_default() {
        let options = SpawnPageOptions::new();
        assert!(options.extraction_prompt.is_none());
        assert!(options.screenshot); // Default is true
        assert_eq!(options.max_rounds, 1);
        assert!(options.user_message_extra.is_none());
        assert!(options.page_setup.is_none());
    }

    #[test]
    fn test_spawn_page_options_builder() {
        let options = SpawnPageOptions::new()
            .with_extraction("Extract the title and main content")
            .with_screenshot(false)
            .with_max_rounds(3)
            .with_user_message("Additional instructions");

        assert_eq!(
            options.extraction_prompt,
            Some("Extract the title and main content".to_string())
        );
        assert!(!options.screenshot);
        assert_eq!(options.max_rounds, 3);
        assert_eq!(
            options.user_message_extra,
            Some("Additional instructions".to_string())
        );
    }

    #[test]
    fn test_spawn_page_options_clone() {
        let options = SpawnPageOptions::new()
            .with_extraction("test")
            .with_screenshot(true)
            .with_max_rounds(2);

        let cloned = options.clone();
        assert_eq!(cloned.extraction_prompt, options.extraction_prompt);
        assert_eq!(cloned.screenshot, options.screenshot);
        assert_eq!(cloned.max_rounds, options.max_rounds);
    }

    #[test]
    fn test_spawn_page_options_debug() {
        let options = SpawnPageOptions::new().with_extraction("test");
        let debug_str = format!("{:?}", options);
        assert!(debug_str.contains("SpawnPageOptions"));
        assert!(debug_str.contains("extraction_prompt"));
    }

    #[test]
    fn test_spawned_page_result_success() {
        let result = SpawnedPageResult {
            url: "https://example.com".to_string(),
            result: Ok(crate::automation::AutomationResult::success("test", 1)
                .with_extracted(serde_json::json!({"title": "Example"}))
                .with_screenshot("base64data".to_string())),
            bytes_transferred: Some(1024.0),
            response_map: None,
        };

        assert!(result.is_ok());
        assert!(result.error().is_none());
        assert_eq!(result.label(), Some("test"));
        assert!(result.extracted().is_some());
        assert_eq!(result.screenshot(), Some("base64data"));
        assert!(result.usage().is_some());
        assert_eq!(result.bytes_transferred, Some(1024.0));
    }

    #[test]
    fn test_spawned_page_result_error() {
        let result = SpawnedPageResult {
            url: "https://example.com".to_string(),
            result: Err("Connection failed".to_string()),
            bytes_transferred: None,
            response_map: None,
        };

        assert!(!result.is_ok());
        assert_eq!(result.error(), Some("Connection failed"));
        assert!(result.label().is_none());
        assert!(result.extracted().is_none());
        assert!(result.screenshot().is_none());
        assert!(result.usage().is_none());
    }

    #[test]
    fn test_spawned_page_result_spawn_pages() {
        let result = SpawnedPageResult {
            url: "https://example.com".to_string(),
            result: Ok(
                crate::automation::AutomationResult::success("test", 1).with_spawn_pages(vec![
                    "https://example.com/page1".to_string(),
                    "https://example.com/page2".to_string(),
                ]),
            ),
            bytes_transferred: None,
            response_map: None,
        };

        let spawn_pages = result.spawn_pages();
        assert!(spawn_pages.is_some());
        assert_eq!(spawn_pages.unwrap().len(), 2);
    }

    #[test]
    fn test_spawned_page_result_with_bytes_tracking() {
        let mut response_map = std::collections::HashMap::new();
        response_map.insert("req1".to_string(), 512.0);
        response_map.insert("req2".to_string(), 1024.0);

        let result = SpawnedPageResult {
            url: "https://example.com".to_string(),
            result: Ok(crate::automation::AutomationResult::success("test", 1)),
            bytes_transferred: Some(1536.0),
            response_map: Some(response_map),
        };

        assert_eq!(result.bytes_transferred, Some(1536.0));
        assert!(result.response_map.is_some());
        assert_eq!(result.response_map.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn test_open_page_action_parsing_single_url() {
        // Test that OpenPage with a single URL is correctly detected
        let step = serde_json::json!({ "OpenPage": "https://example.com" });
        if let Some(obj) = step.as_object() {
            for (action, value) in obj {
                if action == "OpenPage" {
                    if let Some(url) = value.as_str() {
                        assert_eq!(url, "https://example.com");
                    } else {
                        panic!("Expected string URL");
                    }
                }
            }
        }
    }

    #[test]
    fn test_open_page_action_parsing_multiple_urls() {
        // Test that OpenPage with multiple URLs is correctly detected
        let step = serde_json::json!({ "OpenPage": ["https://a.com", "https://b.com"] });
        if let Some(obj) = step.as_object() {
            for (action, value) in obj {
                if action == "OpenPage" {
                    if let Some(urls) = value.as_array() {
                        assert_eq!(urls.len(), 2);
                        assert_eq!(urls[0].as_str().unwrap(), "https://a.com");
                        assert_eq!(urls[1].as_str().unwrap(), "https://b.com");
                    } else {
                        panic!("Expected array of URLs");
                    }
                }
            }
        }
    }

    #[test]
    fn test_text_only_prompt_flavor_preserves_bindings() {
        let mut prompt = DEFAULT_SYSTEM_PROMPT.to_string();
        RemoteMultimodalEngine::apply_text_only_prompt_flavor(&mut prompt, false);

        assert!(prompt.contains("No screenshot is provided this round"));
        assert!(!prompt
            .contains("- Screenshot of current page state (may be omitted in text-only rounds)"));
        // Keep explicit bindings/format rules for accuracy.
        assert!(prompt.contains("ClickPoint"));
        assert!(prompt.contains("SetViewport"));
        assert!(prompt.contains("JSON only"));
    }

    #[test]
    fn test_should_include_screenshot_for_round_respects_model_capability() {
        let engine = RemoteMultimodalEngine::new("https://api.example.com", "gpt-3.5-turbo", None);
        let cfg = RemoteMultimodalConfig::default();
        assert!(!engine.should_include_screenshot_for_round(&cfg, true));

        let vision_engine = RemoteMultimodalEngine::new("https://api.example.com", "gpt-4o", None);
        assert!(vision_engine.should_include_screenshot_for_round(&cfg, true));
    }

    #[test]
    fn test_should_include_screenshot_for_round_honors_override() {
        let engine = RemoteMultimodalEngine::new("https://api.example.com", "gpt-3.5-turbo", None);
        let cfg_true = RemoteMultimodalConfig {
            include_screenshot: Some(true),
            ..Default::default()
        };
        assert!(engine.should_include_screenshot_for_round(&cfg_true, true));

        let cfg_false = RemoteMultimodalConfig {
            include_screenshot: Some(false),
            ..Default::default()
        };
        assert!(!engine.should_include_screenshot_for_round(&cfg_false, true));
    }

    #[test]
    fn test_summarize_step_blocklist_dedup_and_selector_priority() {
        let steps = vec![
            serde_json::json!({"Click":"button.verify"}),
            serde_json::json!({"Click":"button.verify"}),
            serde_json::json!({"Fill":{"selector":"input[name='email']","value":"a@b.c"}}),
            serde_json::json!({"ClickPoint":{"x":120.0,"y":240.0}}),
        ];

        let blocklist = RemoteMultimodalEngine::summarize_step_blocklist(&steps, 10);
        assert_eq!(blocklist.len(), 3);
        assert!(blocklist
            .iter()
            .any(|s| s.contains("Click: selector=button.verify")));
        assert!(blocklist
            .iter()
            .any(|s| s.contains("Fill: selector=input[name='email']")));
        assert!(blocklist
            .iter()
            .any(|s| s.contains("ClickPoint: point=(120.0, 240.0)")));
    }

    #[test]
    fn test_build_user_prompt_includes_loop_blocklist() {
        let engine = RemoteMultimodalEngine::new("https://api.example.com", "gpt-4o", None);
        let cfg = RemoteMultimodalConfig::default();
        let cap = CaptureProfile::default();
        let blocklist = vec![
            "Click: selector=button.verify".to_string(),
            "ClickPoint: point=(100.0, 200.0)".to_string(),
        ];

        let prompt = engine.build_user_prompt(
            &cfg,
            &cap,
            "https://example.com",
            "https://example.com",
            "Example Title",
            "<html></html>",
            2,
            true,
            3,
            &blocklist,
            None,
            None,
        );

        assert!(prompt.contains("LOOP BLOCKLIST"));
        assert!(prompt.contains("Click: selector=button.verify"));
        assert!(prompt.contains("ClickPoint: point=(100.0, 200.0)"));
    }

    #[test]
    fn test_extracted_level_key() {
        let val = serde_json::json!({
            "current_level": 7,
            "level_name": "Word Search"
        });
        assert_eq!(
            RemoteMultimodalEngine::extracted_level_key(Some(&val)).as_deref(),
            Some("L7:word-search")
        );
    }
}
