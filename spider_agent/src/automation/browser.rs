//! Browser-specific automation methods for chromiumoxide integration.
//!
//! This module provides browser automation capabilities that require a
//! Chrome browser page. All methods are gated behind `#[cfg(feature = "chrome")]`.

#[cfg(feature = "chrome")]
use base64::{engine::general_purpose, Engine as _};
#[cfg(feature = "chrome")]
use chromiumoxide::{
    cdp::browser_protocol::page::CaptureScreenshotFormat,
    layout::Point,
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
    pub relevant: Option<bool>,
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
        // Only apply if input is empty (first view) - don't grayscale after typing
        let _ = page.evaluate(r#"
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
        "#).await;

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
        let _ = page.evaluate(r#"
            document.documentElement.style.filter = '';
            document.body.style.filter = '';
            document.querySelectorAll('canvas, img, svg, div').forEach(el => {
                el.style.filter = '';
            });
        "#).await;

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
        action_stuck_rounds: usize,
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

        // Include user instructions if provided
        if let Some(extra) = &self.user_message_extra {
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
                    spawn_pages: Vec::new(),
                    relevant: None,
                });
            }
        }

        let base_effective_cfg: RemoteMultimodalConfig = self.cfg.clone();

        // Extraction-only optimization: skip screenshots unless explicitly requested.
        // Saves ~35k tokens per call for vision-capable models doing text extraction.
        let extraction_only = base_effective_cfg.is_extraction_only();
        let skip_screenshot_for_extraction =
            extraction_only && base_effective_cfg.include_screenshot != Some(true);

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
        let mut last_relevant: Option<bool> = None;
        let mut all_spawn_pages: Vec<String> = Vec::new();
        // Stuck-loop detection: track hashed step sequences across rounds
        let mut recent_step_hashes: std::collections::VecDeque<u64> =
            std::collections::VecDeque::new();
        let mut action_stuck_rounds: usize = 0;
        // Dual-model routing: set by `request_vision` memory_op to force vision next round
        let mut force_vision_next_round: bool = false;

        // Effective system_prompt_extra — may be augmented with recalled experience context.
        let mut effective_system_prompt_extra = self.system_prompt_extra.clone();

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
                            if ctx.is_empty() { None } else {
                                log::info!(
                                    "Recalled {} strategies ({} chars)",
                                    experiences.len(), ctx.len(),
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

        let rounds = base_effective_cfg.max_rounds.max(1);
        for round_idx in 0..rounds {
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

            // Capture state – skip screenshot when text-only round
            let html_fut = self.html_context_with_profile(page, &base_effective_cfg, cap);
            let url_fut = async {
                Ok::<String, EngineError>(self.url_context(page, &base_effective_cfg).await)
            };
            let title_fut = async {
                Ok::<String, EngineError>(self.title_context(page, &base_effective_cfg).await)
            };

            let (screenshot, html, mut url_now, mut title_now) =
                if use_vision && !skip_screenshot_for_extraction {
                    let screenshot_fut = self.screenshot_as_data_url_with_profile(page, cap);
                    let (s, h, u, t) =
                        tokio::try_join!(screenshot_fut, html_fut, url_fut, title_fut)?;
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
                            log::debug!(
                                "Running pre_evaluate for skill '{}' ({} bytes)",
                                skill_name,
                                js.len()
                            );
                            let _ = page.evaluate(*js).await;
                        }
                        // Re-capture title after pre_evaluate (JS sets document.title)
                        let new_title =
                            self.title_context(page, &base_effective_cfg).await;
                        if !new_title.is_empty() && new_title != title_now {
                            log::debug!(
                                "Pre-evaluate updated title: '{}' -> '{}'",
                                &title_now[..title_now.len().min(80)],
                                &new_title[..new_title.len().min(80)]
                            );
                            title_now = new_title;
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
                )
            {
                true // upgraded to vision mid-round
            } else {
                use_vision
            };

            // Late screenshot capture when upgrading to vision after stagnation detected
            let screenshot = if use_vision && screenshot.is_empty() {
                self.screenshot_as_data_url_with_profile(page, cap).await?
            } else {
                screenshot
            };

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
                    action_stuck_rounds,
                    memory.as_deref(),
                    use_vision,
                    effective_system_prompt_extra.as_deref(),
                )
                .await?;

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
                    log::warn!(
                        "Action loop detected: {} consecutive identical step sequences",
                        action_stuck_rounds
                    );
                }
            }

            // Process memory operations from the plan
            if let Some(ref mut mem) = memory {
                for op in &plan.memory_ops {
                    match op {
                        MemoryOperation::Set { key, value } => {
                            // Detect `request_vision` memory_op for dual-model routing
                            if key == "request_vision" {
                                force_vision_next_round = true;
                                log::debug!("Agent requested vision for next round");
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
                                        mem.set("_active_skill".to_string(), serde_json::json!(skill_name));
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

            // Save extracted data if present
            if plan.extracted.is_some() {
                last_extracted = plan.extracted.clone();
                // Also store in memory if available
                if let (Some(ref mut mem), Some(ref extracted)) = (&mut memory, &plan.extracted) {
                    mem.add_extraction(extracted.clone());
                }
            }

            // Execute steps (even if done=true, we need to process OpenPage actions).
            // When stuck in a loop for 5+ rounds, skip repeated steps and auto-inspect DOM.
            if action_stuck_rounds >= 5 {
                log::warn!(
                    "Skipping {} repeated steps - auto-inspecting DOM (stuck {} rounds)",
                    plan.steps.len(),
                    action_stuck_rounds
                );
                // Inject DOM inspection so the model gets real state data next round
                let _ = page
                    .evaluate(
                        r#"document.title = 'AUTO_DOM_INSPECT:' + JSON.stringify({
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
                // Record in memory that DOM was auto-inspected
                if let Some(ref mut mem) = memory {
                    mem.add_action(format!(
                        "SYSTEM: Skipped repeated steps (stuck {}x). DOM auto-inspected - check page title for element states.",
                        action_stuck_rounds
                    ));
                }
            } else if !plan.steps.is_empty() {
                let (steps_executed, spawn_pages) = self
                    .execute_steps(page, &plan.steps, &base_effective_cfg)
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
                    spawn_pages: all_spawn_pages,
                    relevant: last_relevant,
                });
            }

            // Post-step delay
            if base_effective_cfg.post_plan_wait_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(
                    base_effective_cfg.post_plan_wait_ms,
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
            spawn_pages: all_spawn_pages,
            relevant: last_relevant,
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
        action_stuck_rounds: usize,
        memory: Option<&AutomationMemory>,
        use_vision: bool,
        recalled_context: Option<&str>,
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
                    memory,
                    use_vision,
                    recalled_context,
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
        action_stuck_rounds: usize,
        memory: Option<&AutomationMemory>,
        use_vision: bool,
        recalled_context: Option<&str>,
    ) -> EngineResult<AutomationPlan> {
        use super::{
            best_effort_parse_json_object, extract_assistant_content, extract_usage,
            DEFAULT_SYSTEM_PROMPT, EXTRACTION_ONLY_SYSTEM_PROMPT,
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

        // Build system prompt — use focused extraction prompt for single-round extraction
        let mut system_msg = if effective_cfg.is_extraction_only() {
            EXTRACTION_ONLY_SYSTEM_PROMPT.to_string()
        } else {
            DEFAULT_SYSTEM_PROMPT.to_string()
        };
        // Add recalled experience context (from long-term memory)
        if let Some(ctx) = recalled_context {
            system_msg.push_str("\n\n");
            system_msg.push_str(ctx);
        }
        // Add any extra system prompt content (but never replace the default)
        if let Some(extra) = &self.system_prompt_extra {
            system_msg.push_str("\n\n");
            system_msg.push_str(extra);
        }
        // Inject matching skills from the skill registry (limited by config)
        #[cfg(feature = "skills")]
        if let Some(ref registry) = self.skill_registry {
            log::debug!("Skill registry: {} skills, checking url={} title={} html_len={}", registry.len(), url_now, title_now, html.len());
            let mut skill_ctx = registry.match_context_limited(
                url_now,
                title_now,
                html,
                effective_cfg.max_skills_per_round,
                effective_cfg.max_skill_context_chars,
            );
            // Also inject agent-requested skills from memory
            if let Some(ref mem) = memory {
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
                let matched: Vec<_> = registry.find_matching(url_now, title_now, html).iter().map(|s| s.name.as_str()).collect();
                log::debug!("Injecting {} skills ({} chars): {:?}", matched.len(), skill_ctx.len(), matched);
                system_msg.push_str("\n\n---\nACTIVATED SKILLS:\n");
                system_msg.push_str(&skill_ctx);
            } else if !registry.is_empty() {
                log::debug!("No skills matched for url={} title={} html_len={}", url_now, title_now, html.len());
                // No skills matched, but skills are available. Show catalog.
                let catalog: Vec<&str> = registry.skill_names().collect();
                if !catalog.is_empty() {
                    system_msg.push_str("\n\nAvailable skills (request via memory_ops `request_skill`): ");
                    system_msg.push_str(&catalog.join(", "));
                }
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
                system_msg
                    .push_str("The \"extracted\" field MUST conform to this JSON Schema:\n");
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
                system_msg.push_str("\n---\nRELEVANCE GATE: Include \"relevant\": true|false in your response.\n");
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
            memory,
        );

        // Inject text-only mode hint when dual routing skips screenshot
        if self.has_dual_model_routing() && !use_vision {
            system_msg.push_str("\n\n---\nMODE: TEXT-ONLY (no screenshot this round). Use HTML context and memory to decide actions. If you need visual information, set `{\"op\":\"set\",\"key\":\"request_vision\",\"value\":true}` in memory_ops to receive a screenshot next round.\n");
        }

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

        let response_format = if effective_cfg.request_json_object {
            Some(serde_json::json!({ "type": "json_object" }))
        } else {
            None
        };

        let request = Request {
            model: resolved_model.to_string(),
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
        let content = extract_assistant_content(&body)
            .ok_or_else(|| EngineError::MissingField("choices[0].message.content"))?;
        log::debug!("LLM response content: {}", content);
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
        log::debug!("Parsed plan - label: {}, done: {}, steps: {:?}", label, done, steps);

        // Extract relevance field if gate is enabled
        let relevant = if effective_cfg.relevance_gate {
            Some(parsed.get("relevant").and_then(|v| v.as_bool()).unwrap_or(true))
        } else {
            None
        };

        // Try to get extracted field, or fallback to the entire response when in extraction mode.
        // Treat `extracted: {}` (empty object) the same as missing for recovery purposes.
        let raw_extracted = parsed.get("extracted").cloned().and_then(|v| {
            if v.as_object().map_or(false, |o| o.is_empty()) {
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
                            "label" | "done" | "steps" | "memory_ops" | "extracted" | "relevant"
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
                            if let (Some(sel), Some(val)) =
                                (fill.get("selector").and_then(|s| s.as_str()), fill.get("value"))
                            {
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
        })
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
                                let _ = page.click_and_hold(point, std::time::Duration::from_millis(hold_ms)).await;
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
                let _ = page.click_and_hold(point, std::time::Duration::from_millis(hold_ms)).await;
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
                if let Ok(elements) = page.find_elements(r#"a, button, [onclick], [role="button"]"#).await {
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
                                            let _ = page.click_and_drag_smooth(from_point, to_point).await;
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
                let _ = page.click_and_drag_smooth(
                    Point::new(from_x, from_y),
                    Point::new(to_x, to_y),
                ).await;
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
                        let _ = page.evaluate(format!(
                            "document.querySelector('{}').value = ''",
                            sel.replace('\'', "\\'")
                        )).await;
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

            // === Viewport / Device Metrics ===
            "SetViewport" => {
                if let Some(obj) = value.as_object() {
                    let width = obj.get("width").and_then(|v| v.as_i64()).unwrap_or(1280);
                    let height = obj.get("height").and_then(|v| v.as_i64()).unwrap_or(960);
                    let device_scale_factor = obj
                        .get("device_scale_factor")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(2.0);
                    let mobile = obj
                        .get("mobile")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);

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
        let (steps_executed, _spawn_pages) = self.execute_steps(page, &[step.clone()], &self.cfg).await?;

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
    engine.with_vision_model(cfgs.vision_model.clone());
    engine.with_text_model(cfgs.text_model.clone());
    engine.with_vision_route_mode(cfgs.vision_route_mode);
    #[cfg(feature = "skills")]
    if let Some(ref registry) = cfgs.skill_registry {
        engine.with_skill_registry(Some(registry.clone()));
    }

    engine.run(page, url).await
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
        self.result
            .as_ref()
            .ok()
            .map(|r| r.spawn_pages.as_slice())
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
pub type PageSetupFn = Box<dyn Fn(&Page) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> + Send + Sync>;

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

            // Build page-specific config with options
            let mut page_cfg = super::RemoteMultimodalConfig::new()
                .with_max_rounds(options.max_rounds.max(1))
                .with_screenshot(options.screenshot);

            // Enable extraction if prompt is provided
            if let Some(ref prompt) = options.extraction_prompt {
                page_cfg = page_cfg.with_extraction(true).with_extraction_prompt(prompt.clone());
            }

            let mut page_cfgs = super::RemoteMultimodalConfigs::new(
                &base_cfgs.api_url,
                &base_cfgs.model_name,
            )
            .with_cfg(page_cfg);

            // Copy API key
            if let Some(ref key) = base_cfgs.api_key {
                page_cfgs = page_cfgs.with_api_key(key.clone());
            }

            // Add user message extra if provided
            if let Some(ref msg) = options.user_message_extra {
                page_cfgs = page_cfgs.with_user_message_extra(msg.clone());
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
                let bytes = if bytes_val > 0 { Some(bytes_val as f64) } else { None };
                let map = if !response_map.is_empty() {
                    Some(response_map.iter().map(|e| (e.key().clone(), *e.value() as f64)).collect())
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

                // Build page-specific config with options
                let mut page_cfg = super::RemoteMultimodalConfig::new()
                    .with_max_rounds(options.max_rounds.max(1))
                    .with_screenshot(options.screenshot);

                // Enable extraction if prompt is provided
                if let Some(ref prompt) = options.extraction_prompt {
                    page_cfg = page_cfg.with_extraction(true).with_extraction_prompt(prompt.clone());
                }

                let mut page_cfgs = super::RemoteMultimodalConfigs::new(
                    &base_cfgs.api_url,
                    &base_cfgs.model_name,
                )
                .with_cfg(page_cfg);

                // Copy API key
                if let Some(ref key) = base_cfgs.api_key {
                    page_cfgs = page_cfgs.with_api_key(key.clone());
                }

                // Add user message extra if provided
                if let Some(ref msg) = options.user_message_extra {
                    page_cfgs = page_cfgs.with_user_message_extra(msg.clone());
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
            result: Ok(crate::automation::AutomationResult::success("test", 1)
                .with_spawn_pages(vec![
                    "https://example.com/page1".to_string(),
                    "https://example.com/page2".to_string(),
                ])),
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
}
