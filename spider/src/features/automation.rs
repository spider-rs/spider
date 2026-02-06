//! Automation module for spider.
//!
//! This module provides web automation capabilities through spider_agent.
//! All core automation types, the RemoteMultimodalEngine, and browser methods
//! are re-exported from spider_agent.
//!
//! Spider-specific types like `PromptConfiguration` (for configuring crawlers
//! from natural language) are defined here.
//!
//! # Feature Requirements
//!
//! - `agent` - Required for automation types and engine
//! - `agent_chrome` - Required for browser automation methods (run, act, observe)
//!
//! # Example
//!
//! ```rust,ignore
//! use spider::features::automation::{RemoteMultimodalEngine, RemoteMultimodalConfigs};
//!
//! // Create engine for extraction
//! let engine = RemoteMultimodalEngine::new(
//!     "https://api.openai.com/v1/chat/completions",
//!     "gpt-4o",
//!     None,
//! ).with_api_key(Some("sk-..."));
//!
//! // Extract from HTML
//! let result = engine.extract_from_html(html, url, title).await?;
//! ```

// =============================================================================
// Re-exports from spider_agent
// =============================================================================

#[cfg(feature = "agent")]
pub use spider_agent::automation::{
    // Core types
    ActionRecord, ActionResult, ActionType, ActResult, AutomationConfig, AutomationResult,
    AutomationUsage, CaptureProfile, CleaningIntent, ClipViewport, ContentAnalysis, CostTier,
    ExtractionSchema, HtmlCleaningProfile, ModelPolicy, PromptUrlGate, RecoveryStrategy,
    RetryPolicy, StructuredOutputConfig,
    // Chain types
    ChainBuilder, ChainCondition, ChainContext, ChainResult, ChainStep, ChainStepResult,
    // Observation types
    FormField, FormInfo, InteractiveElement, NavigationOption, PageObservation,
    // Engine and config
    RemoteMultimodalConfig, RemoteMultimodalConfigs, RemoteMultimodalEngine,
    // Error types
    EngineError, EngineResult,
    // Helper functions
    best_effort_parse_json_object, extract_assistant_content, extract_last_code_block,
    extract_last_json_array, extract_last_json_boundaries, extract_last_json_object, extract_usage,
    fnv1a64, truncate_utf8_tail,
    // HTML cleaning
    clean_html, clean_html_base, clean_html_full, clean_html_raw, clean_html_slim,
    clean_html_with_profile, clean_html_with_profile_and_intent, smart_clean_html,
    // Map result types
    categories, DiscoveredUrl, MapResult,
    // Memory operations
    AutomationMemory, MemoryOperation,
    // System prompts
    ACT_SYSTEM_PROMPT, CONFIGURATION_SYSTEM_PROMPT, DEFAULT_SYSTEM_PROMPT,
    EXTRACTION_ONLY_SYSTEM_PROMPT, EXTRACT_SYSTEM_PROMPT, MAP_SYSTEM_PROMPT,
    OBSERVE_SYSTEM_PROMPT,
    // Selector cache
    SelectorCache, SelectorCacheEntry,
    // Config helpers
    is_url_allowed, merged_config,
    // Selector cache (already exported above - SelectorCache, SelectorCacheEntry)
};

// Skills module for dynamic context injection
#[cfg(feature = "agent_skills")]
pub use spider_agent::automation::skills;

// Performance types
#[cfg(feature = "agent")]
pub use spider_agent::automation::cache::{CacheStats, CacheValue, SmartCache};
#[cfg(feature = "agent")]
pub use spider_agent::automation::executor::{BatchExecutor, ChainExecutor, PrefetchManager};
#[cfg(feature = "agent")]
pub use spider_agent::automation::router::{ModelRouter, RoutingDecision, TaskAnalysis, TaskCategory};

// Browser-specific exports (requires agent + chrome)
#[cfg(all(feature = "agent", feature = "agent_chrome"))]
pub use spider_agent::automation::run_remote_multimodal_with_page;

// Spawn pages support (requires agent + chrome)
#[cfg(all(feature = "agent", feature = "agent_chrome"))]
pub use spider_agent::automation::{
    run_spawn_pages_concurrent, run_spawn_pages_with_factory, run_spawn_pages_with_options,
    PageFactory, PageSetupFn, SpawnPageOptions, SpawnedPageResult,
};

// Chrome page type for browser automation (needed for both agent and stub implementations)
#[cfg(feature = "chrome")]
use chromiumoxide::Page;

// =============================================================================
// Stub types for backward compatibility (when agent feature is not enabled)
// =============================================================================

/// Token usage tracking (stub when agent feature not enabled).
#[cfg(not(feature = "agent"))]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AutomationUsage {
    /// Prompt tokens used.
    pub prompt_tokens: u32,
    /// Completion tokens used.
    pub completion_tokens: u32,
    /// Total tokens used.
    pub total_tokens: u32,
}

/// Result of automation (stub when agent feature not enabled).
#[cfg(not(feature = "agent"))]
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AutomationResult {
    /// Label for this automation.
    pub label: String,
    /// Number of steps executed.
    pub steps_executed: usize,
    /// Whether automation succeeded.
    pub success: bool,
    /// Error message if failed.
    pub error: Option<String>,
    /// Token usage.
    pub usage: AutomationUsage,
    /// Extracted data.
    #[cfg(feature = "serde")]
    pub extracted: Option<serde_json::Value>,
    /// Screenshot (base64).
    pub screenshot: Option<String>,
    /// URLs to open in new pages concurrently.
    pub spawn_pages: Vec<String>,
    /// Whether the page is relevant to crawl goals.
    pub relevant: Option<bool>,
}

/// Engine error type (stub when agent feature not enabled).
#[cfg(not(feature = "agent"))]
#[derive(Debug)]
pub enum EngineError {
    /// HTTP error.
    Http(reqwest::Error),
    /// JSON error.
    #[cfg(feature = "serde")]
    Json(serde_json::Error),
    /// Missing field.
    MissingField(&'static str),
    /// Invalid field.
    InvalidField(&'static str),
    /// Remote error.
    Remote(String),
    /// Unsupported operation.
    Unsupported(&'static str),
}

#[cfg(not(feature = "agent"))]
impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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

#[cfg(not(feature = "agent"))]
impl std::error::Error for EngineError {}

#[cfg(not(feature = "agent"))]
impl From<reqwest::Error> for EngineError {
    fn from(e: reqwest::Error) -> Self {
        EngineError::Http(e)
    }
}

#[cfg(all(not(feature = "agent"), feature = "serde"))]
impl From<serde_json::Error> for EngineError {
    fn from(e: serde_json::Error) -> Self {
        EngineError::Json(e)
    }
}

/// Convenience result type.
#[cfg(not(feature = "agent"))]
pub type EngineResult<T> = Result<T, EngineError>;

/// Remote multimodal configs (stub when agent feature not enabled).
#[cfg(not(feature = "agent"))]
#[derive(Debug, Clone, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RemoteMultimodalConfigs {
    /// API URL.
    pub api_url: String,
    /// API key.
    pub api_key: Option<String>,
    /// Model name.
    pub model_name: String,
}

#[cfg(not(feature = "agent"))]
impl RemoteMultimodalConfigs {
    /// Create a new remote multimodal config (stub - does nothing without agent feature).
    pub fn new(api_url: impl Into<String>, model_name: impl Into<String>) -> Self {
        Self {
            api_url: api_url.into(),
            model_name: model_name.into(),
            ..Default::default()
        }
    }

    /// Set the API key (stub - does nothing without agent feature).
    pub fn with_api_key(mut self, api_key: Option<impl Into<String>>) -> Self {
        self.api_key = api_key.map(|k| k.into());
        self
    }
}

// =============================================================================
// Spider-specific types and functions
// =============================================================================

/// Configuration response from the LLM for prompt-based crawler setup.
///
/// This type is specific to spider's Website configuration. Use it with
/// `configure_crawler_from_prompt` to generate crawler settings from
/// natural language descriptions.
///
/// # Example
///
/// ```rust,ignore
/// use spider::features::automation::configure_crawler_from_prompt;
///
/// let config = configure_crawler_from_prompt(
///     "http://localhost:11434/v1/chat/completions",
///     "llama3",
///     None,
///     "Crawl only blog posts, max 50 pages, respect robots.txt"
/// ).await?;
///
/// // Apply to website
/// website.apply_prompt_configuration(&config);
/// ```
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct PromptConfiguration {
    /// Respect robots.txt rules.
    pub respect_robots_txt: Option<bool>,
    /// Crawl subdomains.
    pub subdomains: Option<bool>,
    /// Crawl top-level domain variants.
    pub tld: Option<bool>,
    /// Maximum crawl depth.
    pub depth: Option<usize>,
    /// Delay between requests in milliseconds.
    pub delay: Option<u64>,
    /// Request timeout in milliseconds.
    pub request_timeout_ms: Option<u64>,
    /// Total crawl timeout in milliseconds.
    pub crawl_timeout_ms: Option<u64>,
    /// URL patterns to exclude.
    pub blacklist_url: Option<Vec<String>>,
    /// URL patterns to include exclusively.
    pub whitelist_url: Option<Vec<String>>,
    /// External domains to allow crawling.
    pub external_domains: Option<Vec<String>>,
    /// User agent string.
    pub user_agent: Option<String>,
    /// Custom HTTP headers.
    pub headers: Option<std::collections::HashMap<String, String>>,
    /// Use HTTP/2 prior knowledge.
    pub http2_prior_knowledge: Option<bool>,
    /// Accept invalid SSL certificates.
    pub accept_invalid_certs: Option<bool>,
    /// Proxy URLs for requests.
    pub proxies: Option<Vec<String>>,
    /// Maximum redirect limit.
    pub redirect_limit: Option<usize>,
    /// Budget limits per path or domain.
    pub budget: Option<std::collections::HashMap<String, u32>>,
    /// Maximum bytes per page.
    pub max_page_bytes: Option<f64>,
    /// Crawl all resources including assets.
    pub full_resources: Option<bool>,
    /// Only crawl HTML pages.
    pub only_html: Option<bool>,
    /// Return discovered links with pages.
    pub return_page_links: Option<bool>,
    /// Use headless Chrome for rendering.
    pub use_chrome: Option<bool>,
    /// Stealth mode level: "none", "basic", "low", "mid", "full".
    pub stealth_mode: Option<String>,
    /// Browser viewport width.
    pub viewport_width: Option<u32>,
    /// Browser viewport height.
    pub viewport_height: Option<u32>,
    /// Wait for network to be idle.
    pub wait_for_idle_network: Option<bool>,
    /// Delay after page load in milliseconds.
    pub wait_for_delay_ms: Option<u64>,
    /// CSS selector to wait for.
    pub wait_for_selector: Option<String>,
    /// JavaScript to inject on each page.
    pub evaluate_on_new_document: Option<String>,
    /// Use shared queue for even distribution.
    pub shared_queue: Option<bool>,
    /// Retry attempts for failed requests.
    pub retry: Option<u8>,
}

/// Generate crawler configuration from a natural language prompt.
///
/// This function creates a RemoteMultimodalEngine and uses it to generate
/// a PromptConfiguration from the given prompt. The configuration can then
/// be applied to a Website using `apply_prompt_configuration`.
///
/// # Arguments
/// * `api_url` - OpenAI-compatible chat completions endpoint
/// * `model_name` - Model identifier (e.g., "gpt-4", "llama3", "qwen2.5")
/// * `api_key` - Optional API key for authenticated endpoints
/// * `prompt` - Natural language description of crawling requirements
///
/// # Example
///
/// ```rust,ignore
/// let config = configure_crawler_from_prompt(
///     "http://localhost:11434/v1/chat/completions",
///     "llama3",
///     None,
///     "Crawl only blog posts, max 50 pages, respect robots.txt"
/// ).await?;
///
/// website.apply_prompt_configuration(&config);
/// ```
#[cfg(all(feature = "agent", feature = "serde"))]
pub async fn configure_crawler_from_prompt(
    api_url: &str,
    model_name: &str,
    api_key: Option<&str>,
    prompt: &str,
) -> EngineResult<PromptConfiguration> {
    use serde::Serialize;

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
    }

    static CLIENT: std::sync::LazyLock<reqwest::Client> =
        std::sync::LazyLock::new(reqwest::Client::new);

    let request_body = InferenceRequest {
        model: model_name.to_string(),
        messages: vec![
            Message {
                role: "system".into(),
                content: CONFIGURATION_SYSTEM_PROMPT.to_string(),
            },
            Message {
                role: "user".into(),
                content: format!(
                    "Configure a web crawler for the following requirements:\n\n{}",
                    prompt
                ),
            },
        ],
        temperature: 0.1,
        max_tokens: 2048,
        response_format: Some(ResponseFormat {
            format_type: "json_object".into(),
        }),
    };

    let mut req = CLIENT.post(api_url).json(&request_body);
    if let Some(key) = api_key {
        req = req.bearer_auth(key);
    }

    let http_resp = req.send().await.map_err(EngineError::Http)?;
    let status = http_resp.status();
    let raw_body = http_resp.text().await.map_err(EngineError::Http)?;

    if !status.is_success() {
        return Err(EngineError::Remote(format!(
            "non-success status {status}: {raw_body}"
        )));
    }

    let root: serde_json::Value = serde_json::from_str(&raw_body)
        .map_err(|e| EngineError::Remote(format!("JSON parse error: {e}")))?;

    let content = extract_assistant_content(&root)
        .ok_or(EngineError::MissingField("choices[0].message.content"))?;

    let config_value = best_effort_parse_json_object(&content)?;

    let config: PromptConfiguration = serde_json::from_value(config_value)
        .map_err(|e| EngineError::Remote(format!("Failed to parse configuration: {e}")))?;

    Ok(config)
}

/// Run remote multi-modal automation if enabled in the configuration.
///
/// This is a convenience function that checks if automation is configured
/// and runs it on the given browser page.
///
/// # Arguments
/// * `cfgs` - Optional automation configuration
/// * `page` - Chrome browser page
/// * `url` - URL being processed
///
/// # Returns
/// * `Ok(None)` - Automation not configured
/// * `Ok(Some(result))` - Automation result
/// * `Err(e)` - Automation failed
#[cfg(all(feature = "agent", feature = "agent_chrome"))]
pub async fn run_remote_multimodal_if_enabled(
    cfgs: &Option<Box<RemoteMultimodalConfigs>>,
    page: &Page,
    url: &str,
) -> EngineResult<Option<AutomationResult>> {
    let cfgs = match cfgs.as_deref() {
        Some(c) => c,
        None => return Ok(None),
    };

    let result = run_remote_multimodal_with_page(cfgs, page, url).await?;

    // Increment relevance credits for irrelevant pages
    if result.relevant == Some(false) {
        cfgs.relevance_credits.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    Ok(Some(result))
}

/// Run remote multi-modal extraction on raw HTML content (no browser required).
///
/// This function enables extraction from HTTP responses without requiring Chrome.
/// It sends the HTML content to the multimodal model for structured data extraction.
///
/// Note: This only supports extraction (`extra_ai_data`), not browser automation.
#[cfg(all(feature = "agent", feature = "serde"))]
pub async fn run_remote_multimodal_extraction(
    cfgs: &Option<Box<RemoteMultimodalConfigs>>,
    html: &str,
    url: &str,
    title: Option<&str>,
) -> EngineResult<Option<AutomationResult>> {
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
        if !gate.is_allowed(url) {
            return Ok(Some(AutomationResult {
                label: "url_not_allowed".into(),
                steps_executed: 0,
                success: true,
                error: None,
                usage: AutomationUsage::default(),
                extracted: None,
                screenshot: None,
                spawn_pages: Vec::new(),
                relevant: None,
            }));
        }
    }

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

    let result = engine.extract_from_html(html, url, title).await?;

    // Increment relevance credits for irrelevant pages
    if result.relevant == Some(false) {
        cfgs.relevance_credits.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    Ok(Some(result))
}

// =============================================================================
// URL pre-filter
// =============================================================================

/// Pre-filter URLs using LLM classification. Returns only URLs classified as relevant.
///
/// Both `url_prefilter` and `relevance_gate` must be enabled on the config.
/// On any error, all input URLs are returned (safe fallback).
#[cfg(all(feature = "agent", feature = "serde"))]
pub(crate) async fn prefilter_urls(
    cfgs: &RemoteMultimodalConfigs,
    urls: &hashbrown::HashSet<case_insensitive_string::CaseInsensitiveString>,
) -> hashbrown::HashSet<case_insensitive_string::CaseInsensitiveString> {
    use case_insensitive_string::CaseInsensitiveString;

    if !cfgs.cfg.url_prefilter || !cfgs.cfg.relevance_gate || urls.is_empty() {
        return urls.clone();
    }

    let batch_size = cfgs.cfg.url_prefilter_batch_size;
    let max_tokens = cfgs.cfg.url_prefilter_max_tokens;

    // Partition URLs into cached and uncached
    let mut relevant_set: hashbrown::HashSet<CaseInsensitiveString> =
        hashbrown::HashSet::with_capacity(urls.len());
    let mut uncached: Vec<CaseInsensitiveString> = Vec::new();

    {
        let cache = match cfgs.url_prefilter_cache.read() {
            Ok(c) => c,
            Err(_) => return urls.clone(), // poisoned lock fallback
        };
        for url in urls {
            let path = url_to_cache_key(url.inner().as_str());
            match cache.get(&path) {
                Some(true) => {
                    relevant_set.insert(url.clone());
                }
                Some(false) => {
                    // cached as irrelevant — skip
                }
                None => {
                    uncached.push(url.clone());
                }
            }
        }
    }

    if uncached.is_empty() {
        return relevant_set;
    }

    // Build engine from cfgs
    let sem = cfgs.get_or_init_semaphore();
    let mut engine = RemoteMultimodalEngine::new(
        cfgs.api_url.clone(),
        cfgs.model_name.clone(),
        cfgs.system_prompt.clone(),
    )
    .with_api_key(cfgs.api_key.as_deref());

    engine.with_semaphore(sem);

    // Copy dual-model routing so text model is used
    engine.with_vision_model(cfgs.vision_model.clone());
    engine.with_text_model(cfgs.text_model.clone());
    engine.with_vision_route_mode(cfgs.vision_route_mode);

    // Process in batches
    for batch in uncached.chunks(batch_size) {
        let url_strs: Vec<&str> = batch.iter().map(|u| u.inner().as_str()).collect();

        let classifications = match engine
            .classify_urls(
                &url_strs,
                cfgs.cfg.relevance_prompt.as_deref(),
                cfgs.cfg.extraction_prompt.as_deref(),
                max_tokens,
            )
            .await
        {
            Ok(c) => c,
            Err(e) => {
                log::warn!("url_prefilter: classify_urls error, assuming all relevant: {e}");
                // On error, include all from this batch
                relevant_set.extend(batch.iter().cloned());
                continue;
            }
        };

        // Update cache and build relevant set
        let mut cache = match cfgs.url_prefilter_cache.write() {
            Ok(c) => c,
            Err(_) => {
                // poisoned lock — include all
                relevant_set.extend(batch.iter().cloned());
                continue;
            }
        };

        for (url, &is_relevant) in batch.iter().zip(classifications.iter()) {
            let path = url_to_cache_key(url.inner().as_str());
            cache.insert(path, is_relevant);
            if is_relevant {
                relevant_set.insert(url.clone());
            }
        }
    }

    relevant_set
}

/// Extract a cache key from a URL (path portion only, or full URL if parse fails).
#[cfg(all(feature = "agent", feature = "serde"))]
fn url_to_cache_key(url: &str) -> String {
    // Try to extract just the path from the URL
    if let Some(start) = url.find("://") {
        if let Some(path_start) = url[start + 3..].find('/') {
            return url[start + 3 + path_start..].to_string();
        }
    }
    url.to_string()
}

// =============================================================================
// Conversion helpers
// =============================================================================

/// Extension trait for converting AutomationResult to spider's AutomationResults.
#[cfg(feature = "agent")]
pub trait AutomationResultExt {
    /// Convert to spider's AutomationResults format.
    fn to_automation_results(&self) -> crate::page::AutomationResults;
}

#[cfg(all(feature = "agent", feature = "serde"))]
impl AutomationResultExt for AutomationResult {
    fn to_automation_results(&self) -> crate::page::AutomationResults {
        crate::page::AutomationResults {
            input: self.label.clone(),
            content_output: self.extracted.clone().unwrap_or(serde_json::Value::Null),
            screenshot_output: self.screenshot.clone(),
            error: self.error.clone(),
            usage: Some(self.usage.clone()),
            relevant: self.relevant,
        }
    }
}

#[cfg(all(feature = "agent", not(feature = "serde")))]
impl AutomationResultExt for AutomationResult {
    fn to_automation_results(&self) -> crate::page::AutomationResults {
        crate::page::AutomationResults {
            input: self.label.clone(),
            content_output: String::new(),
            screenshot_output: self.screenshot.clone(),
            error: self.error.clone(),
            usage: Some(self.usage.clone()),
            relevant: self.relevant,
        }
    }
}

// =============================================================================
// Spawn pages helper for spider
// =============================================================================

/// Process spawn_pages URLs concurrently with full spider configuration.
///
/// This function creates new pages using the browser, applies spider's full
/// configuration (stealth mode, fingerprinting, event tracking, etc.), and runs
/// automation on each page concurrently.
///
/// # Arguments
/// * `browser` - The browser to create new pages from
/// * `browser_context_id` - Optional browser context ID for incognito mode
/// * `urls` - URLs to open (typically from `page_response.spawn_pages`)
/// * `mm_cfgs` - Configuration for the automation engine
/// * `spider_config` - Spider configuration to apply to each spawned page
/// * `options` - Additional spawn options (extraction, screenshots, etc.)
///
/// # Example
/// ```ignore
/// if let Some(spawn_urls) = page_response.spawn_pages.take() {
///     let options = SpawnPageOptions::new()
///         .with_extraction("Extract page content");
///
///     let results = process_spawn_pages_with_config(
///         &browser,
///         &browser_context_id,
///         spawn_urls,
///         &mm_config,
///         &configuration,
///         options,
///     ).await;
/// }
/// ```
#[cfg(all(feature = "agent", feature = "agent_chrome"))]
pub async fn process_spawn_pages_with_config(
    browser: &std::sync::Arc<chromiumoxide::browser::Browser>,
    browser_context_id: &Option<chromiumoxide::cdp::browser_protocol::browser::BrowserContextId>,
    urls: Vec<String>,
    mm_cfgs: &RemoteMultimodalConfigs,
    spider_config: &crate::configuration::Configuration,
    options: SpawnPageOptions,
) -> Vec<SpawnedPageResult> {
    use std::sync::Arc;

    if urls.is_empty() {
        return Vec::new();
    }

    let mm_cfgs = Arc::new(mm_cfgs.clone());
    let spider_config = Arc::new(spider_config.clone());
    let options = Arc::new(options);
    let browser_context_id = browser_context_id.clone();

    let mut handles = Vec::with_capacity(urls.len());

    // Check if we should track network events
    let track_events = spider_config.track_events.as_ref().map(|t| t.responses).unwrap_or(false);

    for url in urls {
        let browser = browser.clone();
        let mm_cfgs = mm_cfgs.clone();
        let spider_config = spider_config.clone();
        let options = options.clone();
        let browser_context_id = browser_context_id.clone();
        let url_clone = url.clone();

        handles.push(tokio::spawn(async move {
            use chromiumoxide::cdp::browser_protocol::network::EventDataReceived;
            use tokio_stream::StreamExt;

            // Create new page using attempt_navigation (spider's standard approach)
            let new_page = match crate::features::chrome::attempt_navigation(
                &url_clone,
                &browser,
                &spider_config.request_timeout,
                &browser_context_id,
                &spider_config.viewport,
            )
            .await
            {
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

            // Apply spider configuration (stealth, fingerprinting, event tracking)
            crate::features::chrome::setup_chrome_events(&new_page, &spider_config).await;

            // Set up bytes tracking listener if event tracking is enabled
            let bytes_listener = if track_events {
                new_page.event_listener::<EventDataReceived>().await.ok()
            } else {
                None
            };

            // Track bytes in a separate task while automation runs
            use std::sync::atomic::{AtomicU64, Ordering};
            let total_bytes: std::sync::Arc<AtomicU64> = std::sync::Arc::new(AtomicU64::new(0));
            let response_map: std::sync::Arc<dashmap::DashMap<String, u64>> =
                std::sync::Arc::new(dashmap::DashMap::new());

            let total_bytes_clone = total_bytes.clone();
            let response_map_clone = response_map.clone();
            let listener_handle = if let Some(mut listener) = bytes_listener {
                Some(tokio::spawn(async move {
                    while let Some(event) = listener.next().await {
                        let bytes = event.encoded_data_length as u64;
                        total_bytes_clone.fetch_add(bytes, Ordering::Relaxed);
                        response_map_clone
                            .entry(event.request_id.inner().clone())
                            .and_modify(|v| *v += bytes)
                            .or_insert(bytes);
                    }
                }))
            } else {
                None
            };

            // Build page-specific config with options
            let mut page_cfg = RemoteMultimodalConfig::new()
                .with_max_rounds(options.max_rounds.max(1))
                .with_screenshot(options.screenshot);

            // Enable extraction if prompt is provided
            if let Some(ref prompt) = options.extraction_prompt {
                page_cfg = page_cfg.with_extraction(true).with_extraction_prompt(prompt.clone());
            }

            let mut page_cfgs = RemoteMultimodalConfigs::new(
                &mm_cfgs.api_url,
                &mm_cfgs.model_name,
            )
            .with_cfg(page_cfg);

            // Copy API key
            if let Some(ref key) = mm_cfgs.api_key {
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

            // Stop the listener and collect bytes data
            if let Some(handle) = listener_handle {
                handle.abort();
            }

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

    // Collect all results
    let mut results = Vec::with_capacity(handles.len());
    for handle in handles {
        match handle.await {
            Ok(spawn_result) => results.push(spawn_result),
            Err(e) => {
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
// Stub implementations (for various feature combinations)
// =============================================================================

/// Stub implementation when agent_chrome feature is not enabled.
/// Returns Ok(None) to indicate automation is not configured.
#[cfg(all(feature = "chrome", not(feature = "agent_chrome")))]
pub async fn run_remote_multimodal_if_enabled(
    _cfgs: &Option<Box<RemoteMultimodalConfigs>>,
    _page: &Page,
    _url: &str,
) -> EngineResult<Option<AutomationResult>> {
    Ok(None)
}

/// Extension trait for converting AutomationResult (stub when agent not enabled).
#[cfg(all(feature = "chrome", not(feature = "agent")))]
pub trait AutomationResultExt {
    /// Convert to spider's AutomationResults format.
    fn to_automation_results(&self) -> crate::page::AutomationResults;
}

#[cfg(all(feature = "chrome", not(feature = "agent")))]
impl AutomationResultExt for AutomationResult {
    fn to_automation_results(&self) -> crate::page::AutomationResults {
        crate::page::AutomationResults {
            input: self.label.clone(),
            content_output: Default::default(),
            screenshot_output: self.screenshot.clone(),
            error: self.error.clone(),
            usage: Some(self.usage.clone()),
            relevant: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prompt_configuration_default() {
        let config = PromptConfiguration::default();
        assert!(config.respect_robots_txt.is_none());
        assert!(config.depth.is_none());
    }

    #[cfg(feature = "serde")]
    #[test]
    fn test_prompt_configuration_serde() {
        let json = r#"{"respect_robots_txt": true, "depth": 5}"#;
        let config: PromptConfiguration = serde_json::from_str(json).unwrap();
        assert_eq!(config.respect_robots_txt, Some(true));
        assert_eq!(config.depth, Some(5));
    }

    #[cfg(feature = "agent")]
    #[test]
    fn test_automation_result_spawn_pages() {
        // Test that spawn_pages can be set and checked
        let result = AutomationResult::success("test", 1)
            .with_spawn_pages(vec![
                "https://example.com/page1".to_string(),
                "https://example.com/page2".to_string(),
            ]);

        assert!(result.has_spawn_pages());
        assert_eq!(result.spawn_pages.len(), 2);
        assert_eq!(result.spawn_pages[0], "https://example.com/page1");
        assert_eq!(result.spawn_pages[1], "https://example.com/page2");

        // Test empty spawn_pages
        let result = AutomationResult::success("test", 1);
        assert!(!result.has_spawn_pages());
        assert!(result.spawn_pages.is_empty());
    }

    #[cfg(all(feature = "agent", feature = "agent_chrome"))]
    #[test]
    fn test_spawn_page_options() {
        let options = SpawnPageOptions::new()
            .with_extraction("Extract content")
            .with_screenshot(true)
            .with_max_rounds(2);

        assert_eq!(options.extraction_prompt, Some("Extract content".to_string()));
        assert!(options.screenshot);
        assert_eq!(options.max_rounds, 2);
    }

    #[cfg(all(feature = "agent", feature = "agent_chrome"))]
    #[test]
    fn test_spawned_page_result_accessors() {
        // Test success case
        let result = SpawnedPageResult {
            url: "https://example.com".to_string(),
            result: Ok(AutomationResult::success("test", 1)
                .with_extracted(serde_json::json!({"key": "value"}))),
            bytes_transferred: Some(2048.0),
            response_map: None,
        };

        assert!(result.is_ok());
        assert!(result.error().is_none());
        assert_eq!(result.label(), Some("test"));
        assert!(result.extracted().is_some());
        assert_eq!(result.bytes_transferred, Some(2048.0));

        // Test error case
        let error_result = SpawnedPageResult {
            url: "https://example.com".to_string(),
            result: Err("Connection error".to_string()),
            bytes_transferred: None,
            response_map: None,
        };

        assert!(!error_result.is_ok());
        assert_eq!(error_result.error(), Some("Connection error"));
        assert!(error_result.label().is_none());
    }

    #[cfg(all(feature = "agent", feature = "agent_chrome"))]
    #[test]
    fn test_spawned_page_result_with_response_map() {
        let mut response_map = std::collections::HashMap::new();
        response_map.insert("req-123".to_string(), 1024.0);
        response_map.insert("req-456".to_string(), 2048.0);

        let result = SpawnedPageResult {
            url: "https://example.com".to_string(),
            result: Ok(AutomationResult::success("test", 1)),
            bytes_transferred: Some(3072.0),
            response_map: Some(response_map),
        };

        assert!(result.bytes_transferred.is_some());
        assert_eq!(result.bytes_transferred.unwrap(), 3072.0);
        assert!(result.response_map.is_some());
        assert_eq!(result.response_map.as_ref().unwrap().len(), 2);
    }
}
