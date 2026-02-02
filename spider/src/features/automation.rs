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

/// In-memory storage for agentic automation sessions.
///
/// This provides a key-value store and history tracking that persists across
/// automation rounds, enabling the LLM to maintain context and state without
/// relying on external storage.
///
/// # Features
/// - **Key-Value Store**: Store and retrieve arbitrary JSON values by key
/// - **Extraction History**: Accumulate extracted data across pages
/// - **URL History**: Track visited URLs for navigation context
/// - **Action Summary**: Brief history of executed actions
///
/// # Example
/// ```ignore
/// use spider::features::automation::AutomationMemory;
///
/// let mut memory = AutomationMemory::default();
/// memory.set("user_logged_in", serde_json::json!(true));
/// memory.set("cart_items", serde_json::json!(["item1", "item2"]));
///
/// // Memory is serialized and included in LLM context each round
/// let context = memory.to_context_string();
/// ```
#[cfg(feature = "serde")]
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct AutomationMemory {
    /// Key-value store for persistent data across rounds.
    #[serde(default)]
    pub store: std::collections::HashMap<String, serde_json::Value>,
    /// History of extracted data from pages (most recent last).
    #[serde(default)]
    pub extractions: Vec<serde_json::Value>,
    /// History of visited URLs (most recent last).
    #[serde(default)]
    pub visited_urls: Vec<String>,
    /// Brief summary of recent actions (most recent last, max 50).
    #[serde(default)]
    pub action_history: Vec<String>,
}

#[cfg(feature = "serde")]
impl AutomationMemory {
    /// Create a new empty memory.
    pub fn new() -> Self {
        Self::default()
    }

    /// Store a value by key.
    pub fn set(&mut self, key: impl Into<String>, value: serde_json::Value) {
        self.store.insert(key.into(), value);
    }

    /// Get a value by key.
    pub fn get(&self, key: &str) -> Option<&serde_json::Value> {
        self.store.get(key)
    }

    /// Remove a value by key.
    pub fn remove(&mut self, key: &str) -> Option<serde_json::Value> {
        self.store.remove(key)
    }

    /// Check if a key exists.
    pub fn contains(&self, key: &str) -> bool {
        self.store.contains_key(key)
    }

    /// Clear all stored data.
    pub fn clear_store(&mut self) {
        self.store.clear();
    }

    /// Add an extracted value to history.
    pub fn add_extraction(&mut self, data: serde_json::Value) {
        self.extractions.push(data);
    }

    /// Record a visited URL.
    pub fn add_visited_url(&mut self, url: impl Into<String>) {
        self.visited_urls.push(url.into());
    }

    /// Record an action summary (keeps max 50 entries).
    pub fn add_action(&mut self, action: impl Into<String>) {
        self.action_history.push(action.into());
        // Keep only the last 50 actions to avoid unbounded growth
        if self.action_history.len() > 50 {
            self.action_history.remove(0);
        }
    }

    /// Clear all history (extractions, URLs, actions) but keep the store.
    pub fn clear_history(&mut self) {
        self.extractions.clear();
        self.visited_urls.clear();
        self.action_history.clear();
    }

    /// Clear everything.
    pub fn clear_all(&mut self) {
        self.store.clear();
        self.extractions.clear();
        self.visited_urls.clear();
        self.action_history.clear();
    }

    /// Check if memory is empty.
    pub fn is_empty(&self) -> bool {
        self.store.is_empty()
            && self.extractions.is_empty()
            && self.visited_urls.is_empty()
            && self.action_history.is_empty()
    }

    /// Generate a context string for inclusion in LLM prompts.
    pub fn to_context_string(&self) -> String {
        if self.is_empty() {
            return String::new();
        }

        let mut parts = Vec::new();

        if !self.store.is_empty() {
            if let Ok(json) = serde_json::to_string_pretty(&self.store) {
                parts.push(format!("## Memory Store\n```json\n{}\n```", json));
            }
        }

        if !self.visited_urls.is_empty() {
            let recent: Vec<_> = self.visited_urls.iter().rev().take(10).collect();
            parts.push(format!(
                "## Recent URLs (last {})\n{}",
                recent.len(),
                recent
                    .iter()
                    .rev()
                    .enumerate()
                    .map(|(i, u)| format!("{}. {}", i + 1, u))
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }

        if !self.extractions.is_empty() {
            let recent: Vec<_> = self.extractions.iter().rev().take(5).collect();
            let json_strs: Vec<_> = recent
                .iter()
                .rev()
                .filter_map(|v| serde_json::to_string(v).ok())
                .collect();
            parts.push(format!(
                "## Recent Extractions (last {})\n{}",
                json_strs.len(),
                json_strs.join("\n")
            ));
        }

        if !self.action_history.is_empty() {
            let recent: Vec<_> = self.action_history.iter().rev().take(10).collect();
            parts.push(format!(
                "## Recent Actions (last {})\n{}",
                recent.len(),
                recent
                    .iter()
                    .rev()
                    .enumerate()
                    .map(|(i, a)| format!("{}. {}", i + 1, a))
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }

        parts.join("\n\n")
    }
}

/// Self-healing selector cache for agentic automation.
///
/// This cache stores mappings from natural language element descriptions to
/// CSS selectors that successfully matched elements. When an action fails
/// due to a selector not finding an element, the cache entry is invalidated
/// and the LLM is re-queried for an updated selector.
///
/// # Self-Healing Flow
/// 1. User requests action like "click the login button"
/// 2. Cache lookup: if we have a cached selector for "login button", try it
/// 3. If selector works → action succeeds, update cache timestamp
/// 4. If selector fails → invalidate cache entry, re-query LLM for new selector
/// 5. Store new selector in cache for future use
///
/// # Cache Key Strategy
/// Keys are normalized element descriptions (lowercased, trimmed).
/// Values include the selector, success count, and last used timestamp.
#[cfg(feature = "serde")]
#[derive(Debug, Clone, Default)]
pub struct SelectorCache {
    /// Maps normalized element descriptions to cached selector info.
    entries: std::collections::HashMap<String, SelectorCacheEntry>,
    /// Maximum number of entries before LRU eviction.
    max_entries: usize,
    /// Cache hit statistics.
    hits: u64,
    /// Cache miss statistics.
    misses: u64,
}

/// A single entry in the selector cache.
#[cfg(feature = "serde")]
#[derive(Debug, Clone)]
pub struct SelectorCacheEntry {
    /// The CSS selector that matched.
    pub selector: String,
    /// Number of times this selector was successfully used.
    pub success_count: u32,
    /// Number of times this selector failed (before being invalidated).
    pub failure_count: u32,
    /// Timestamp of last successful use (unix millis).
    pub last_used_ms: u64,
    /// The URL domain where this selector was discovered.
    pub domain: Option<String>,
}

#[cfg(feature = "serde")]
impl SelectorCache {
    /// Create a new selector cache with default capacity (1000 entries).
    pub fn new() -> Self {
        Self {
            entries: std::collections::HashMap::new(),
            max_entries: 1000,
            hits: 0,
            misses: 0,
        }
    }

    /// Create a new selector cache with specified capacity.
    pub fn with_capacity(max_entries: usize) -> Self {
        Self {
            entries: std::collections::HashMap::with_capacity(max_entries.min(10000)),
            max_entries,
            hits: 0,
            misses: 0,
        }
    }

    /// Normalize a description key for consistent lookup.
    fn normalize_key(description: &str) -> String {
        description.trim().to_lowercase()
    }

    /// Look up a cached selector for an element description.
    pub fn get(&mut self, description: &str, domain: Option<&str>) -> Option<&str> {
        let key = Self::normalize_key(description);
        if let Some(entry) = self.entries.get(&key) {
            // Check domain match if specified
            if let Some(d) = domain {
                if let Some(cached_domain) = &entry.domain {
                    if cached_domain != d {
                        self.misses += 1;
                        return None;
                    }
                }
            }
            self.hits += 1;
            Some(&entry.selector)
        } else {
            self.misses += 1;
            None
        }
    }

    /// Record a successful selector use.
    pub fn record_success(&mut self, description: &str, selector: &str, domain: Option<&str>) {
        let key = Self::normalize_key(description);
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        if let Some(entry) = self.entries.get_mut(&key) {
            entry.success_count = entry.success_count.saturating_add(1);
            entry.last_used_ms = now_ms;
            entry.selector = selector.to_string();
        } else {
            // Evict LRU if at capacity
            if self.entries.len() >= self.max_entries {
                self.evict_lru();
            }
            self.entries.insert(
                key,
                SelectorCacheEntry {
                    selector: selector.to_string(),
                    success_count: 1,
                    failure_count: 0,
                    last_used_ms: now_ms,
                    domain: domain.map(|s| s.to_string()),
                },
            );
        }
    }

    /// Record a selector failure and invalidate the entry.
    pub fn record_failure(&mut self, description: &str) {
        let key = Self::normalize_key(description);
        if let Some(entry) = self.entries.get_mut(&key) {
            entry.failure_count = entry.failure_count.saturating_add(1);
            // Invalidate after repeated failures
            if entry.failure_count >= 2 {
                self.entries.remove(&key);
            }
        }
    }

    /// Invalidate (remove) a cache entry.
    pub fn invalidate(&mut self, description: &str) {
        let key = Self::normalize_key(description);
        self.entries.remove(&key);
    }

    /// Clear all cache entries.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.hits = 0;
        self.misses = 0;
    }

    /// Evict the least recently used entry.
    fn evict_lru(&mut self) {
        if let Some(lru_key) = self
            .entries
            .iter()
            .min_by_key(|(_, v)| v.last_used_ms)
            .map(|(k, _)| k.clone())
        {
            self.entries.remove(&lru_key);
        }
    }

    /// Get cache statistics.
    pub fn stats(&self) -> (u64, u64, usize) {
        (self.hits, self.misses, self.entries.len())
    }
}

/// Result of the `map()` API call for page discovery.
#[cfg(feature = "serde")]
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct MapResult {
    /// The URLs discovered on the page.
    pub urls: Vec<DiscoveredUrl>,
    /// Relevance score of the page to the prompt (0.0 - 1.0).
    pub relevance: f32,
    /// Summary of what the page contains.
    pub summary: String,
    /// Suggested next URLs to explore based on the prompt.
    pub suggested_next: Vec<String>,
    /// Optional screenshot if captured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub screenshot: Option<String>,
    /// Token usage statistics.
    #[serde(default)]
    pub usage: AutomationUsage,
}

/// A discovered URL with AI-generated metadata.
#[cfg(feature = "serde")]
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct DiscoveredUrl {
    /// The URL.
    pub url: String,
    /// Link text or title.
    pub text: String,
    /// AI-generated description of what this URL likely contains.
    pub description: String,
    /// Relevance to the prompt (0.0 - 1.0).
    pub relevance: f32,
    /// Whether this URL is recommended to visit.
    pub recommended: bool,
    /// Category of the URL (navigation, content, external, etc.).
    pub category: String,
}

/// Configuration for structured output mode.
///
/// When enabled, the engine uses native JSON schema support from the API
/// (OpenAI's `response_format.json_schema` or similar) to enforce output structure.
#[cfg(feature = "serde")]
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct StructuredOutputConfig {
    /// Enable structured output mode.
    pub enabled: bool,
    /// The JSON schema to enforce.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<serde_json::Value>,
    /// Name for the schema (required by some APIs).
    #[serde(default = "default_schema_name")]
    pub schema_name: String,
    /// Whether to use strict mode (exact schema compliance).
    #[serde(default)]
    pub strict: bool,
}

#[cfg(feature = "serde")]
fn default_schema_name() -> String {
    "response".to_string()
}

#[cfg(feature = "serde")]
impl StructuredOutputConfig {
    /// Create a new structured output config with a schema.
    pub fn new(schema: serde_json::Value) -> Self {
        Self {
            enabled: true,
            schema: Some(schema),
            schema_name: "response".to_string(),
            strict: false,
        }
    }

    /// Create with strict mode enabled.
    pub fn strict(schema: serde_json::Value) -> Self {
        Self {
            enabled: true,
            schema: Some(schema),
            schema_name: "response".to_string(),
            strict: true,
        }
    }

    /// Set the schema name.
    pub fn with_name(mut self, name: &str) -> Self {
        self.schema_name = name.to_string();
        self
    }
}

// ============================================================================
// PHASE 3: ADVANCED AGENTIC FEATURES
// ============================================================================

/// Error recovery strategy for agent execution.
///
/// Defines how the agent should handle failures during multi-step execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum RecoveryStrategy {
    /// Retry the same action up to N times.
    #[default]
    Retry,
    /// Try an alternative approach (re-query LLM for different solution).
    Alternative,
    /// Skip the failed step and continue with the next action.
    Skip,
    /// Abort the entire execution on failure.
    Abort,
}

/// Configuration for the autonomous agent executor.
#[cfg(feature = "serde")]
#[derive(Debug, Clone)]
pub struct AgentConfig {
    /// The high-level goal to achieve.
    pub goal: String,
    /// Maximum number of actions before stopping.
    pub max_steps: usize,
    /// Maximum time in milliseconds before timeout.
    pub timeout_ms: u64,
    /// Strategy for handling action failures.
    pub recovery_strategy: RecoveryStrategy,
    /// Number of retries for the Retry strategy.
    pub max_retries: usize,
    /// Whether to use the selector cache for actions.
    pub use_cache: bool,
    /// Whether to take screenshots after each step.
    pub capture_screenshots: bool,
    /// URLs that indicate goal completion (optional).
    pub success_urls: Vec<String>,
    /// Text patterns that indicate goal completion (optional).
    pub success_patterns: Vec<String>,
    /// Whether to extract data when goal is reached.
    pub extract_on_success: bool,
    /// Optional extraction prompt for final data extraction.
    pub extraction_prompt: Option<String>,
}

#[cfg(feature = "serde")]
impl Default for AgentConfig {
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
        }
    }
}

#[cfg(feature = "serde")]
impl AgentConfig {
    /// Create a new agent config with a goal.
    pub fn new(goal: &str) -> Self {
        Self {
            goal: goal.to_string(),
            ..Default::default()
        }
    }

    /// Set maximum steps.
    pub fn with_max_steps(mut self, max_steps: usize) -> Self {
        self.max_steps = max_steps;
        self
    }

    /// Set timeout in milliseconds.
    pub fn with_timeout(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = timeout_ms;
        self
    }

    /// Set recovery strategy.
    pub fn with_recovery(mut self, strategy: RecoveryStrategy) -> Self {
        self.recovery_strategy = strategy;
        self
    }

    /// Set max retries for Retry strategy.
    pub fn with_retries(mut self, max_retries: usize) -> Self {
        self.max_retries = max_retries;
        self
    }

    /// Enable/disable selector cache.
    pub fn with_cache(mut self, use_cache: bool) -> Self {
        self.use_cache = use_cache;
        self
    }

    /// Add a URL pattern that indicates success.
    pub fn with_success_url(mut self, url: &str) -> Self {
        self.success_urls.push(url.to_string());
        self
    }

    /// Add a text pattern that indicates success.
    pub fn with_success_pattern(mut self, pattern: &str) -> Self {
        self.success_patterns.push(pattern.to_string());
        self
    }

    /// Enable extraction on success with a prompt.
    pub fn with_extraction(mut self, prompt: &str) -> Self {
        self.extract_on_success = true;
        self.extraction_prompt = Some(prompt.to_string());
        self
    }
}

/// Events emitted during agent execution for progress tracking.
#[cfg(feature = "serde")]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[allow(missing_docs)]
pub enum AgentEvent {
    /// Agent started execution.
    Started {
        goal: String,
        timestamp_ms: u64,
    },
    /// Agent is planning the next action.
    Planning {
        step: usize,
        current_url: String,
    },
    /// Agent is executing an action.
    Executing {
        step: usize,
        action: String,
    },
    /// Action completed successfully.
    ActionSuccess {
        step: usize,
        action: String,
        duration_ms: u64,
    },
    /// Action failed.
    ActionFailed {
        step: usize,
        action: String,
        error: String,
        will_retry: bool,
    },
    /// Agent is recovering from failure.
    Recovering {
        step: usize,
        strategy: RecoveryStrategy,
        attempt: usize,
    },
    /// Goal completion detected.
    GoalDetected {
        step: usize,
        reason: String,
    },
    /// Agent completed successfully.
    Completed {
        steps_taken: usize,
        duration_ms: u64,
        success: bool,
    },
    /// Agent was aborted or timed out.
    Aborted {
        step: usize,
        reason: String,
    },
    /// Screenshot captured.
    Screenshot {
        step: usize,
        data: String,
    },
    /// Data extracted.
    Extracted {
        step: usize,
        data: serde_json::Value,
    },
}

/// Result of autonomous agent execution.
#[cfg(feature = "serde")]
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct AgentResult {
    /// Whether the goal was achieved.
    pub success: bool,
    /// The goal that was attempted.
    pub goal: String,
    /// Number of steps taken.
    pub steps_taken: usize,
    /// Total duration in milliseconds.
    pub duration_ms: u64,
    /// Final URL after execution.
    pub final_url: String,
    /// History of actions taken.
    pub action_history: Vec<AgentActionRecord>,
    /// Extracted data (if extraction was enabled).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extracted: Option<serde_json::Value>,
    /// Final screenshot (if screenshots were enabled).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub final_screenshot: Option<String>,
    /// Error message if the agent failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Total token usage across all LLM calls.
    pub total_usage: AutomationUsage,
    /// Events that occurred during execution.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub events: Vec<AgentEvent>,
}

/// Record of a single action in the agent's history.
#[cfg(feature = "serde")]
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct AgentActionRecord {
    /// Step number (1-indexed).
    pub step: usize,
    /// The action that was taken.
    pub action: String,
    /// Whether the action succeeded.
    pub success: bool,
    /// Duration of the action in milliseconds.
    pub duration_ms: u64,
    /// URL before the action.
    pub url_before: String,
    /// URL after the action (if changed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url_after: Option<String>,
    /// Error message if the action failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Number of retries needed.
    pub retries: usize,
}

/// A single step in an action chain.
#[cfg(feature = "serde")]
#[derive(Debug, Clone)]
pub struct ChainStep {
    /// The instruction to execute.
    pub instruction: String,
    /// Optional condition that must be true to execute this step.
    pub condition: Option<ChainCondition>,
    /// Whether to continue on failure.
    pub continue_on_failure: bool,
    /// Optional extraction after this step.
    pub extract: Option<String>,
}

#[cfg(feature = "serde")]
impl ChainStep {
    /// Create a new chain step.
    pub fn new(instruction: &str) -> Self {
        Self {
            instruction: instruction.to_string(),
            condition: None,
            continue_on_failure: false,
            extract: None,
        }
    }

    /// Add a condition for this step.
    pub fn when(mut self, condition: ChainCondition) -> Self {
        self.condition = Some(condition);
        self
    }

    /// Continue chain even if this step fails.
    pub fn allow_failure(mut self) -> Self {
        self.continue_on_failure = true;
        self
    }

    /// Extract data after this step.
    pub fn then_extract(mut self, prompt: &str) -> Self {
        self.extract = Some(prompt.to_string());
        self
    }
}

/// Condition for conditional execution in action chains.
#[cfg(feature = "serde")]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum ChainCondition {
    /// Execute if URL contains this string.
    UrlContains(String),
    /// Execute if URL matches this pattern.
    UrlMatches(String),
    /// Execute if page contains this text.
    PageContains(String),
    /// Execute if an element matching this selector exists.
    ElementExists(String),
    /// Execute if previous step succeeded.
    PreviousSucceeded,
    /// Execute if previous step failed.
    PreviousFailed,
    /// Always execute (default).
    Always,
}

#[cfg(feature = "serde")]
impl Default for ChainCondition {
    fn default() -> Self {
        Self::Always
    }
}

/// Result of an action chain execution.
#[cfg(feature = "serde")]
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ChainResult {
    /// Whether all required steps completed successfully.
    pub success: bool,
    /// Number of steps executed.
    pub steps_executed: usize,
    /// Number of steps that succeeded.
    pub steps_succeeded: usize,
    /// Number of steps that failed.
    pub steps_failed: usize,
    /// Number of steps skipped (due to conditions).
    pub steps_skipped: usize,
    /// Results from each step.
    pub step_results: Vec<ChainStepResult>,
    /// Extracted data from steps with extraction.
    #[serde(default)]
    pub extractions: Vec<serde_json::Value>,
    /// Total duration in milliseconds.
    pub duration_ms: u64,
    /// Total token usage.
    pub total_usage: AutomationUsage,
}

/// Result of a single step in an action chain.
#[cfg(feature = "serde")]
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ChainStepResult {
    /// Step index (0-based).
    pub index: usize,
    /// The instruction that was executed.
    pub instruction: String,
    /// Whether the step was executed (false if condition not met).
    pub executed: bool,
    /// Whether the step succeeded (if executed).
    pub success: bool,
    /// Action taken (if executed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action_taken: Option<String>,
    /// Error message (if failed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Duration in milliseconds.
    pub duration_ms: u64,
    /// Extracted data (if extraction was requested).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extracted: Option<serde_json::Value>,
}

// ============================================================================
// CONTENT ANALYSIS FOR INTELLIGENT SCREENSHOT DETECTION
// ============================================================================

/// Result of analyzing HTML content to determine if visual capture is needed.
///
/// This analysis helps decide whether to rely on HTML text alone or require
/// a screenshot for accurate extraction, especially for pages with:
/// - iframes, videos, canvas elements
/// - Dynamically rendered content (heavy JavaScript)
/// - Minimal text but rich visual elements
/// - SVGs, images with embedded text
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ContentAnalysis {
    /// Whether the content is considered "thin" (low text content).
    pub is_thin_content: bool,
    /// Whether visual elements that need screenshot were detected.
    pub has_visual_elements: bool,
    /// Whether dynamic content indicators were found.
    pub has_dynamic_content: bool,
    /// Recommendation: true if screenshot is recommended for extraction.
    pub needs_screenshot: bool,
    /// Count of iframe elements.
    pub iframe_count: usize,
    /// Count of video elements.
    pub video_count: usize,
    /// Count of canvas elements.
    pub canvas_count: usize,
    /// Count of embed/object elements.
    pub embed_count: usize,
    /// Count of SVG elements.
    pub svg_count: usize,
    /// Approximate text content length (visible text).
    pub text_length: usize,
    /// Total HTML length.
    pub html_length: usize,
    /// Ratio of text to HTML (lower = more markup, less content).
    pub text_ratio: f32,
    /// Indicators found (for debugging).
    pub indicators: Vec<String>,

    // === BYTE SIZE TRACKING (more accurate than counts) ===

    /// Total bytes of all SVG elements (inline SVGs can be massive).
    pub svg_bytes: usize,
    /// Total bytes of all script elements.
    pub script_bytes: usize,
    /// Total bytes of all style elements.
    pub style_bytes: usize,
    /// Total bytes of base64-encoded data URIs (images, fonts, etc.).
    pub base64_bytes: usize,
    /// Total bytes that should be cleaned (scripts + styles + heavy elements).
    pub cleanable_bytes: usize,
    /// Ratio of cleanable bytes to total HTML (higher = more benefit from cleaning).
    pub cleanable_ratio: f32,
}

lazy_static::lazy_static! {
    /// Aho-Corasick pattern matcher for visual element tags (case-insensitive).
    static ref VISUAL_ELEMENT_MATCHER: aho_corasick::AhoCorasick = {
        aho_corasick::AhoCorasick::builder()
            .ascii_case_insensitive(true)
            .build(&["<iframe", "<video", "<canvas", "<embed", "<object"])
            .expect("valid patterns")
    };

    /// Aho-Corasick pattern matcher for SPA framework indicators.
    static ref SPA_INDICATOR_MATCHER: aho_corasick::AhoCorasick = {
        aho_corasick::AhoCorasick::builder()
            .ascii_case_insensitive(true)
            .build(&[
                "data-reactroot",
                "__next",
                "id=\"app\"",
                "id=\"root\"",
                "ng-app",
                "v-app",
            ])
            .expect("valid patterns")
    };
}

impl ContentAnalysis {
    /// Minimum text length to consider content "substantial".
    const MIN_TEXT_LENGTH: usize = 200;
    /// Text-to-HTML ratio below which content is considered "thin".
    const MIN_TEXT_RATIO: f32 = 0.05;

    /// Analyze HTML content to determine if screenshot is needed.
    ///
    /// Uses efficient Aho-Corasick multi-pattern matching for performance.
    ///
    /// # Example
    /// ```ignore
    /// let analysis = ContentAnalysis::analyze(html);
    /// if analysis.needs_screenshot {
    ///     // Use screenshot-based extraction
    /// } else {
    ///     // HTML-only extraction is sufficient
    /// }
    /// ```
    /// Fast analysis - single pass, no detailed byte size calculation.
    /// Use this for quick decisions. Use `analyze_full()` for smart cleaning.
    pub fn analyze(html: &str) -> Self {
        Self::analyze_internal(html, false)
    }

    /// Full analysis with byte size calculation for smart cleaning decisions.
    /// More expensive but enables byte-size-based cleaning profile selection.
    pub fn analyze_full(html: &str) -> Self {
        Self::analyze_internal(html, true)
    }

    fn analyze_internal(html: &str, calculate_sizes: bool) -> Self {
        let html_bytes = html.as_bytes();
        let html_length = html.len();

        let mut analysis = Self {
            html_length,
            ..Default::default()
        };

        // Count visual elements using Aho-Corasick (single pass, case-insensitive)
        for mat in VISUAL_ELEMENT_MATCHER.find_iter(html_bytes) {
            match mat.pattern().as_usize() {
                0 => analysis.iframe_count += 1,  // <iframe
                1 => analysis.video_count += 1,   // <video
                2 => analysis.canvas_count += 1,  // <canvas
                3 | 4 => analysis.embed_count += 1, // <embed, <object
                _ => {}
            }
        }

        // Count SVGs (fast - just count occurrences)
        analysis.svg_count = html_bytes
            .windows(4)
            .filter(|w| w.eq_ignore_ascii_case(b"<svg"))
            .count();

        // Check for SPA indicators
        analysis.has_dynamic_content = SPA_INDICATOR_MATCHER.find(html_bytes).is_some();

        // Use lol_html for efficient text extraction
        analysis.text_length = extract_text_length_fast(html);

        // Only calculate detailed byte sizes when requested (more expensive)
        if calculate_sizes {
            let heavy_sizes = calculate_heavy_element_sizes(html);
            analysis.svg_bytes = heavy_sizes.svg_bytes;
            analysis.script_bytes = heavy_sizes.script_bytes;
            analysis.style_bytes = heavy_sizes.style_bytes;
            analysis.base64_bytes = heavy_sizes.base64_bytes;
            analysis.cleanable_bytes = heavy_sizes.svg_bytes
                + heavy_sizes.script_bytes
                + heavy_sizes.style_bytes
                + heavy_sizes.base64_bytes;
        } else {
            // Fast estimation: use counts and heuristics
            // Average SVG ~5KB, script ~10KB, style ~2KB
            analysis.svg_bytes = analysis.svg_count * 5_000;
            analysis.script_bytes = estimate_script_bytes_fast(html_bytes);
            analysis.style_bytes = estimate_style_bytes_fast(html_bytes);
            analysis.base64_bytes = estimate_base64_bytes_fast(html_bytes);
            analysis.cleanable_bytes = analysis.svg_bytes
                + analysis.script_bytes
                + analysis.style_bytes
                + analysis.base64_bytes;
        }

        // Calculate ratios
        analysis.text_ratio = if html_length > 0 {
            analysis.text_length as f32 / html_length as f32
        } else {
            0.0
        };

        analysis.cleanable_ratio = if html_length > 0 {
            analysis.cleanable_bytes as f32 / html_length as f32
        } else {
            0.0
        };

        // Determine if content is "thin"
        analysis.is_thin_content = analysis.text_length < Self::MIN_TEXT_LENGTH
            || analysis.text_ratio < Self::MIN_TEXT_RATIO;

        // Check for visual elements
        analysis.has_visual_elements = analysis.iframe_count > 0
            || analysis.video_count > 0
            || analysis.canvas_count > 0
            || analysis.embed_count > 0;

        // Build indicators list (only if needed for debugging)
        if analysis.iframe_count > 0 {
            analysis.indicators.push(format!("{} iframe(s)", analysis.iframe_count));
        }
        if analysis.video_count > 0 {
            analysis.indicators.push(format!("{} video(s)", analysis.video_count));
        }
        if analysis.canvas_count > 0 {
            analysis.indicators.push(format!("{} canvas", analysis.canvas_count));
        }
        if analysis.embed_count > 0 {
            analysis.indicators.push(format!("{} embed/object", analysis.embed_count));
        }
        // Highlight large SVGs (more useful than count)
        if analysis.svg_bytes > 10_000 {
            analysis.indicators.push(format!(
                "SVG {}KB",
                analysis.svg_bytes / 1024
            ));
        }
        // Highlight high cleanable ratio
        if analysis.cleanable_ratio > 0.3 {
            analysis.indicators.push(format!(
                "cleanable {:.0}%",
                analysis.cleanable_ratio * 100.0
            ));
        }
        if analysis.is_thin_content {
            analysis.indicators.push(format!(
                "thin ({}b, {:.0}%)",
                analysis.text_length,
                analysis.text_ratio * 100.0
            ));
        }
        if analysis.has_dynamic_content {
            analysis.indicators.push("SPA".to_string());
        }

        // Final recommendation
        analysis.needs_screenshot = analysis.has_visual_elements
            || analysis.is_thin_content
            || (analysis.has_dynamic_content && analysis.text_length < 500);

        analysis
    }

    /// Quick check if screenshot is likely needed (very fast, single pass).
    ///
    /// Uses Aho-Corasick for efficient multi-pattern matching without
    /// allocating memory for lowercase conversion.
    #[inline]
    pub fn quick_needs_screenshot(html: &str) -> bool {
        let bytes = html.as_bytes();

        // Quick check for visual elements (iframe, video, canvas, embed, object)
        if VISUAL_ELEMENT_MATCHER.find(bytes).is_some() {
            return true;
        }

        // Estimate text content length (fast approximation)
        let text_len = estimate_text_length_fast(bytes);

        // Thin content needs screenshot
        if text_len < 200 {
            return true;
        }

        // SPA with minimal content needs screenshot
        if text_len < 500 && SPA_INDICATOR_MATCHER.find(bytes).is_some() {
            return true;
        }

        false
    }

    /// Check if HTML has any visual elements (iframe, video, canvas, embed, object).
    #[inline]
    pub fn has_visual_elements(html: &str) -> bool {
        VISUAL_ELEMENT_MATCHER.find(html.as_bytes()).is_some()
    }
}

/// Fast text length estimation by counting non-tag characters.
/// Doesn't allocate - just estimates the visible text length.
#[inline]
fn estimate_text_length_fast(html: &[u8]) -> usize {
    let mut text_len = 0;
    let mut in_tag = false;
    let mut in_script = 0u8; // 0=no, 1=in <script, 2=saw <, looking for /script

    for &byte in html {
        match byte {
            b'<' => {
                in_tag = true;
                if in_script == 0 {
                    in_script = 2; // might be starting </script>
                }
            }
            b'>' => {
                in_tag = false;
            }
            b's' | b'S' if in_tag => {
                // Could be <script or </script - simplified heuristic
            }
            _ if !in_tag && in_script == 0 => {
                if !byte.is_ascii_whitespace() || text_len > 0 {
                    text_len += 1;
                }
            }
            _ => {}
        }
    }

    text_len
}

/// Extract text length using lol_html streaming parser (more accurate than estimation).
fn extract_text_length_fast(html: &str) -> usize {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    let text_len = Arc::new(AtomicUsize::new(0));
    let text_len_clone = Arc::clone(&text_len);

    // Use lol_html for efficient streaming text extraction
    let mut rewriter = lol_html::HtmlRewriter::new(
        lol_html::Settings {
            element_content_handlers: vec![
                // Skip script and style content
                lol_html::text!("script", |_| Ok(())),
                lol_html::text!("style", |_| Ok(())),
            ],
            document_content_handlers: vec![
                lol_html::doc_text!(move |chunk| {
                    let text = chunk.as_str();
                    let non_ws_len = text.chars().filter(|c| !c.is_whitespace()).count();
                    text_len_clone.fetch_add(non_ws_len, Ordering::Relaxed);
                    Ok(())
                }),
            ],
            ..lol_html::Settings::new()
        },
        |_: &[u8]| {},
    );

    if rewriter.write(html.as_bytes()).is_err() {
        return estimate_text_length_fast(html.as_bytes());
    }

    let _ = rewriter.end();

    text_len.load(Ordering::Relaxed)
}

/// Heavy element byte sizes extracted from HTML.
#[derive(Debug, Default)]
struct HeavyElementSizes {
    svg_bytes: usize,
    script_bytes: usize,
    style_bytes: usize,
    base64_bytes: usize,
}

/// Calculate byte sizes of heavy elements (SVG, script, style, base64) using lol_html.
/// This is more accurate than counting elements - a single large SVG can be 500KB+.
fn calculate_heavy_element_sizes(html: &str) -> HeavyElementSizes {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    let svg_bytes = Arc::new(AtomicUsize::new(0));
    let script_bytes = Arc::new(AtomicUsize::new(0));
    let style_bytes = Arc::new(AtomicUsize::new(0));

    let svg_clone = Arc::clone(&svg_bytes);
    let script_clone = Arc::clone(&script_bytes);
    let style_clone = Arc::clone(&style_bytes);

    // Track current element being processed for size calculation
    let _current_element_size = Arc::new(AtomicUsize::new(0));
    let in_svg = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let in_script = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let in_style = Arc::new(std::sync::atomic::AtomicBool::new(false));

    let in_svg_clone = Arc::clone(&in_svg);
    let in_script_clone = Arc::clone(&in_script);
    let in_style_clone = Arc::clone(&in_style);
    let svg_size = Arc::clone(&svg_bytes);
    let script_size = Arc::clone(&script_bytes);
    let style_size = Arc::clone(&style_bytes);

    let mut rewriter = lol_html::HtmlRewriter::new(
        lol_html::Settings {
            element_content_handlers: vec![
                lol_html::element!("svg", move |el| {
                    in_svg_clone.store(true, Ordering::Relaxed);
                    // Estimate opening tag size
                    svg_clone.fetch_add(el.tag_name().len() + 2, Ordering::Relaxed);
                    Ok(())
                }),
                lol_html::element!("script", move |el| {
                    in_script_clone.store(true, Ordering::Relaxed);
                    script_clone.fetch_add(el.tag_name().len() + 2, Ordering::Relaxed);
                    Ok(())
                }),
                lol_html::element!("style", move |el| {
                    in_style_clone.store(true, Ordering::Relaxed);
                    style_clone.fetch_add(el.tag_name().len() + 2, Ordering::Relaxed);
                    Ok(())
                }),
                lol_html::text!("svg", move |chunk| {
                    svg_size.fetch_add(chunk.as_str().len(), Ordering::Relaxed);
                    Ok(())
                }),
                lol_html::text!("script", move |chunk| {
                    script_size.fetch_add(chunk.as_str().len(), Ordering::Relaxed);
                    Ok(())
                }),
                lol_html::text!("style", move |chunk| {
                    style_size.fetch_add(chunk.as_str().len(), Ordering::Relaxed);
                    Ok(())
                }),
            ],
            ..lol_html::Settings::new()
        },
        |_: &[u8]| {},
    );

    let _ = rewriter.write(html.as_bytes());
    let _ = rewriter.end();

    // Count base64 data URIs (common in SVGs and inline images)
    let base64_bytes = count_base64_bytes(html);

    HeavyElementSizes {
        svg_bytes: svg_bytes.load(Ordering::Relaxed),
        script_bytes: script_bytes.load(Ordering::Relaxed),
        style_bytes: style_bytes.load(Ordering::Relaxed),
        base64_bytes,
    }
}

/// Count bytes in base64 data URIs (data:image/..., data:font/..., etc.)
fn count_base64_bytes(html: &str) -> usize {
    let mut total = 0;
    let bytes = html.as_bytes();
    let pattern = b"data:";

    let mut i = 0;
    while i < bytes.len().saturating_sub(5) {
        if &bytes[i..i + 5] == pattern {
            // Find the end of the data URI (quote, space, or >)
            let start = i;
            while i < bytes.len() && !matches!(bytes[i], b'"' | b'\'' | b' ' | b'>' | b')') {
                i += 1;
            }
            total += i - start;
        }
        i += 1;
    }
    total
}

// === FAST ESTIMATION FUNCTIONS (no lol_html overhead) ===

/// Fast estimate of script bytes by counting <script> tags and content between them.
/// Average inline script is ~10KB, external script tag is ~100 bytes.
#[inline]
fn estimate_script_bytes_fast(html: &[u8]) -> usize {
    let mut count = 0;
    let mut i = 0;
    while i < html.len().saturating_sub(7) {
        if html[i..].starts_with(b"<script") || html[i..].starts_with(b"<SCRIPT") {
            count += 1;
            // Skip to end of script
            while i < html.len().saturating_sub(9) {
                if html[i..].starts_with(b"</script") || html[i..].starts_with(b"</SCRIPT") {
                    break;
                }
                i += 1;
            }
        }
        i += 1;
    }
    // Estimate: average script is 10KB
    count * 10_000
}

/// Fast estimate of style bytes by counting <style> tags.
#[inline]
fn estimate_style_bytes_fast(html: &[u8]) -> usize {
    let mut count = 0;
    let mut i = 0;
    while i < html.len().saturating_sub(6) {
        if html[i..].starts_with(b"<style") || html[i..].starts_with(b"<STYLE") {
            count += 1;
        }
        i += 1;
    }
    // Estimate: average style block is 2KB
    count * 2_000
}

/// Fast estimate of base64 data URI bytes.
#[inline]
fn estimate_base64_bytes_fast(html: &[u8]) -> usize {
    let mut total = 0;
    let mut i = 0;
    while i < html.len().saturating_sub(5) {
        if &html[i..i + 5] == b"data:" {
            let start = i;
            // Skip to end of data URI
            while i < html.len() && !matches!(html[i], b'"' | b'\'' | b' ' | b'>' | b')') {
                i += 1;
            }
            total += i - start;
        }
        i += 1;
    }
    total
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
    /// Slim fit: removes SVGs, canvas, video, base64 images, and other heavy nodes.
    Slim,
    /// Less aggressive: try `clean_html_base` if available, otherwise fall back to `clean_html`.
    Minimal,
    /// No cleaning (raw HTML).
    Raw,
    /// Auto-detect based on content analysis. Uses ContentAnalysis to decide:
    /// - Heavy visual elements (SVGs, canvas) → Slim
    /// - Large HTML with low text ratio → Aggressive
    /// - Normal content → Default
    Auto,
}

/// The intended use case for HTML cleaning - affects how aggressive we can be.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum CleaningIntent {
    #[default]
    /// General purpose - balanced cleaning.
    General,
    /// Extraction only - can be more aggressive, don't need interactive elements.
    Extraction,
    /// Action/navigation - preserve buttons, forms, links, interactive elements.
    Action,
}

impl HtmlCleaningProfile {
    /// Determine the best cleaning profile based on content analysis.
    ///
    /// This is used when `Auto` is selected to intelligently choose
    /// the appropriate cleaning level based on the HTML content.
    pub fn from_content_analysis(analysis: &ContentAnalysis) -> Self {
        Self::from_content_analysis_with_intent(analysis, CleaningIntent::General)
    }

    /// Determine the best cleaning profile based on content analysis and intended use.
    ///
    /// - `Extraction` intent allows more aggressive cleaning (remove nav, footer, etc.)
    /// - `Action` intent preserves interactive elements (buttons, forms, links)
    /// - `General` intent uses balanced heuristics
    /// Size thresholds for smart cleaning decisions (in bytes).
    const SVG_HEAVY_THRESHOLD: usize = 50_000; // 50KB of SVG is heavy
    const SVG_VERY_HEAVY_THRESHOLD: usize = 100_000; // 100KB of SVG is very heavy
    const BASE64_HEAVY_THRESHOLD: usize = 100_000; // 100KB of base64 data
    const SCRIPT_HEAVY_THRESHOLD: usize = 200_000; // 200KB of scripts
    const CLEANABLE_RATIO_HIGH: f32 = 0.4; // 40% of HTML is cleanable
    const CLEANABLE_RATIO_MEDIUM: f32 = 0.25; // 25% of HTML is cleanable

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
    /// Search provider configuration for web search integration.
    #[cfg(feature = "search")]
    pub search_config: Option<crate::configuration::SearchConfig>,
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
            screenshot: true,
            #[cfg(feature = "search")]
            search_config: None,
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

    /// Configure web search integration.
    #[cfg(feature = "search")]
    pub fn with_search_config(
        &mut self,
        config: Option<crate::configuration::SearchConfig>,
    ) -> &mut Self {
        self.cfg.search_config = config;
        self
    }

    /// Check if search is enabled and properly configured.
    #[cfg(feature = "search")]
    pub fn search_enabled(&self) -> bool {
        self.cfg
            .search_config
            .as_ref()
            .map(|c| c.is_enabled())
            .unwrap_or(false)
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
    ///
    /// # Example
    /// ```ignore
    /// // Use with pre-captured screenshot
    /// let result = engine.extract_with_screenshot(
    ///     &html,
    ///     "https://example.com",
    ///     Some("Page Title"),
    ///     Some(&screenshot_base64),
    /// ).await?;
    /// ```
    #[cfg(feature = "serde")]
    pub async fn extract_with_screenshot(
        &self,
        html: &str,
        url: &str,
        title: Option<&str>,
        screenshot_base64: Option<&str>,
    ) -> EngineResult<AutomationResult> {
        use serde::Serialize;

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

        if screenshot_base64.is_some() {
            user_text.push_str("\nIMPORTANT: The screenshot may contain visual information not present in the HTML (iframe content, videos, canvas drawings, dynamically rendered content). Examine the screenshot carefully.\n");
        }

        if let Some(extra) = &self.user_message_extra {
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
                        text: Some(self.system_prompt_compiled(effective_cfg)),
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

        let extracted = plan_value.get("extracted").cloned();

        Ok(AutomationResult {
            label,
            steps_executed: 0,
            success: true,
            error: None,
            usage,
            extracted,
            screenshot: screenshot_base64.map(|s| s.to_string()),
        })
    }

    /// Analyze HTML content to determine if visual capture (screenshot) is needed.
    ///
    /// This is useful before calling extraction methods to decide whether to
    /// capture a screenshot for better results.
    ///
    /// # Example
    /// ```ignore
    /// let analysis = engine.analyze_content(&html);
    /// if analysis.needs_screenshot {
    ///     log::info!("Screenshot recommended: {:?}", analysis.indicators);
    ///     // Capture screenshot and use extract_with_screenshot
    /// }
    /// ```
    pub fn analyze_content(&self, html: &str) -> ContentAnalysis {
        ContentAnalysis::analyze(html)
    }

    /// Quick check if screenshot is likely needed for extraction.
    ///
    /// Faster than `analyze_content()` but less detailed.
    pub fn needs_screenshot(&self, html: &str) -> bool {
        ContentAnalysis::quick_needs_screenshot(html)
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
        memory: Option<&AutomationMemory>,
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
    #[cfg(feature = "chrome")]
    pub async fn run_with_memory(
        &self,
        page: &Page,
        url_input: &str,
        mut memory: Option<&mut AutomationMemory>,
    ) -> EngineResult<AutomationResult> {
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

                // Capture screenshot on failure (enabled by default)
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

        // Capture final screenshot (enabled by default)
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
        memory: Option<&AutomationMemory>,
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
            memory,
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

        // Extract memory operations if present
        let memory_ops = plan_value
            .get("memory_ops")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|op| serde_json::from_value::<MemoryOperation>(op.clone()).ok())
                    .collect()
            })
            .unwrap_or_default();

        Ok(ParsedPlan {
            label,
            done,
            steps,
            usage,
            extracted,
            memory_ops,
        })
    }
}

/// Memory operation requested by the LLM.
///
/// These operations allow the model to persist data across automation rounds
/// without requiring external storage.
#[cfg(feature = "serde")]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum MemoryOperation {
    /// Store a value in memory.
    Set {
        /// The key to store under.
        key: String,
        /// The value to store (any JSON value).
        value: serde_json::Value,
    },
    /// Delete a value from memory.
    Delete {
        /// The key to delete.
        key: String,
    },
    /// Clear all stored values.
    Clear,
}

/// Parsed plan returned by the model.
///
/// This is an internal, fully-validated representation of the model output.
/// The engine converts the raw JSON plan into this type after:
/// - extracting assistant text from the provider response,
/// - parsing JSON (optionally best-effort),
/// - validating required fields (`label`, `steps`),
/// - deserializing `steps` into concrete [`crate::features::chrome_common::WebAutomation`] actions.
///
/// ## Fields
/// - `label`: Human-readable short description of the plan (from the model).
/// - `done`: Whether the model indicates no further automation is required.
///   When `done == true`, the engine treats the run as complete and stops looping.
/// - `steps`: Concrete automation steps to execute on the page.
/// - `usage`: Token usage from this inference call.
/// - `extracted`: Optional structured data extracted from the page.
/// - `memory_ops`: Memory operations to execute (set/delete/clear).
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
    /// Memory operations to execute.
    memory_ops: Vec<MemoryOperation>,
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
///   or because a [`crate::features::chrome_common::WebAutomation`] step failed to execute.
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
- Session memory (if enabled): key-value store, recent URLs, extractions, and action history

You MUST output a single JSON object ONLY (no prose), with shape:
{
  "label": "short description",
  "done": true|false,
  "steps": [ ... ],
  "memory_ops": [ ... ],  // optional
  "extracted": { ... }    // optional
}

Completion rules:
- If the task/challenge is solved OR the user goal is satisfied, set "done": true and set "steps": [].
- If additional actions are needed, set "done": false and provide next steps.

## Memory Operations (optional)

You can persist data across rounds using the "memory_ops" array. This is useful for:
- Storing extracted information for later use
- Tracking state across page navigations
- Accumulating data from multiple pages

Memory operations:
- { "op": "set", "key": "name", "value": any_json_value }  // Store a value
- { "op": "delete", "key": "name" }                        // Remove a value
- { "op": "clear" }                                        // Clear all stored values

Example with memory:
{
  "label": "Extracted product price, storing for comparison",
  "done": false,
  "steps": [{ "Click": ".next-page" }],
  "memory_ops": [
    { "op": "set", "key": "product_price", "value": 29.99 },
    { "op": "set", "key": "page_count", "value": 1 }
  ]
}

## Browser Actions

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
5) Use memory_ops to persist important data across rounds for multi-step workflows.
6) Output JSON only.
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
    clean_html_with_profile_and_intent(html, profile, CleaningIntent::General)
}

/// Clean HTML with a specific profile and intent.
///
/// The intent helps Auto mode choose the right cleaning level:
/// - `Extraction` - can be more aggressive, removes nav/footer
/// - `Action` - preserves interactive elements
/// - `General` - balanced approach
#[cfg(feature = "chrome")]
fn clean_html_with_profile_and_intent(
    html: &str,
    profile: HtmlCleaningProfile,
    intent: CleaningIntent,
) -> String {
    match profile {
        HtmlCleaningProfile::Raw => crate::utils::clean_html_raw(html),
        HtmlCleaningProfile::Default => crate::utils::clean_html(html),
        HtmlCleaningProfile::Aggressive => crate::utils::clean_html_full(html),
        HtmlCleaningProfile::Slim => crate::utils::clean_html_slim(html),
        HtmlCleaningProfile::Minimal => crate::utils::clean_html_base(html),
        HtmlCleaningProfile::Auto => {
            // Analyze content and choose the best profile based on intent
            let analysis = ContentAnalysis::analyze(html);
            let auto_profile =
                HtmlCleaningProfile::from_content_analysis_with_intent(&analysis, intent);
            // Recursively call with determined profile (won't be Auto again)
            clean_html_with_profile_and_intent(html, auto_profile, intent)
        }
    }
}

/// Smart HTML cleaner that automatically determines the best cleaning level.
///
/// This is the recommended function for cleaning HTML when you don't have
/// a specific profile preference. It analyzes the content and chooses
/// the optimal cleaning level based on:
/// - Content size and text ratio
/// - Presence of heavy elements (SVGs, canvas, video)
/// - The intended use case (extraction vs action)
#[cfg(feature = "chrome")]
pub fn smart_clean_html(html: &str, intent: CleaningIntent) -> String {
    clean_html_with_profile_and_intent(html, HtmlCleaningProfile::Auto, intent)
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

/// System prompt for configuring a web crawler from natural language.
pub const CONFIGURATION_SYSTEM_PROMPT: &str = r##"
You are a web crawler configuration assistant. Given a natural language description of crawling requirements, output a JSON configuration object.

## Available Configuration Options

### Core Crawling
- "respect_robots_txt": bool - Respect robots.txt rules (may slow crawl if delays specified)
- "subdomains": bool - Include subdomains in the crawl
- "tld": bool - Allow all TLDs for the domain
- "depth": number - Max crawl depth (default: 25, prevents infinite recursion)
- "delay": number - Polite delay between requests in milliseconds
- "request_timeout_ms": number - Request timeout in milliseconds (default: 15000, null to disable)
- "crawl_timeout_ms": number - Total crawl timeout in milliseconds (null for no limit)

### URL Filtering
- "blacklist_url": string[] - URLs/patterns to exclude (supports regex)
- "whitelist_url": string[] - Only crawl these URLs/patterns (supports regex)
- "external_domains": string[] - External domains to include in crawl

### Request Settings
- "user_agent": string - Custom User-Agent string
- "headers": object - Custom HTTP headers {"Header-Name": "value"}
- "http2_prior_knowledge": bool - Use HTTP/2 (enable if site supports it)
- "accept_invalid_certs": bool - Accept invalid SSL certificates (use carefully)

### Proxy Configuration
- "proxies": string[] - List of proxy URLs to rotate through

### Limits & Budget
- "redirect_limit": number - Max redirects per request
- "budget": object - Crawl budget per path {"path": max_pages}
- "max_page_bytes": number - Max bytes per page (null for no limit)

### Content Options
- "full_resources": bool - Collect all resources (images, scripts, etc.)
- "only_html": bool - Only fetch HTML pages (saves resources)
- "return_page_links": bool - Include links in page results

### Chrome/Browser Options (requires chrome feature)
- "use_chrome": bool - Use headless Chrome for JavaScript rendering
- "stealth_mode": string - Stealth level: "none", "basic", "low", "mid", "full"
- "viewport_width": number - Browser viewport width
- "viewport_height": number - Browser viewport height
- "wait_for_idle_network": bool - Wait for network to be idle
- "wait_for_delay_ms": number - Fixed delay after page load
- "wait_for_selector": string - CSS selector to wait for
- "evaluate_on_new_document": string - JavaScript to inject on each page

### Performance
- "shared_queue": bool - Use shared queue (even distribution, no priority)
- "retry": number - Retry attempts for failed requests

## Output Format

Return ONLY a valid JSON object with the configuration. Example:

```json
{
  "respect_robots_txt": true,
  "delay": 100,
  "depth": 10,
  "subdomains": false,
  "user_agent": "MyBot/1.0",
  "blacklist_url": ["/admin", "/private"],
  "use_chrome": false
}
```

Only include fields that need to be changed from defaults. Omit fields to use defaults.
Do not include explanations - output ONLY the JSON object.
"##;

/// Configuration response from the LLM for prompt-based crawler setup.
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

impl RemoteMultimodalEngine {
    /// Generate crawler configuration from a natural language prompt.
    ///
    /// This method sends the prompt to the configured LLM endpoint and parses
    /// the response into a `PromptConfiguration` that can be applied to a Website.
    ///
    /// # Example
    /// ```ignore
    /// let engine = RemoteMultimodalEngine::new(
    ///     "http://localhost:11434/v1/chat/completions",
    ///     "llama3",
    ///     None,
    /// );
    ///
    /// let config = engine.configure_from_prompt(
    ///     "Crawl product pages only, respect robots.txt, use 100ms delay, max depth 5"
    /// ).await?;
    /// ```
    #[cfg(feature = "serde")]
    pub async fn configure_from_prompt(
        &self,
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

        let request_body = InferenceRequest {
            model: self.model_name.clone(),
            messages: vec![
                Message {
                    role: "system".into(),
                    content: CONFIGURATION_SYSTEM_PROMPT.to_string(),
                },
                Message {
                    role: "user".into(),
                    content: format!("Configure a web crawler for the following requirements:\n\n{}", prompt),
                },
            ],
            temperature: 0.1,
            max_tokens: 2048,
            response_format: Some(ResponseFormat {
                format_type: "json_object".into(),
            }),
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
            "configure_from_prompt: status={} latency={:?} body_len={}",
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

        let config_value = best_effort_parse_json_object(&content)?;

        let config: PromptConfiguration = serde_json::from_value(config_value)
            .map_err(|e| EngineError::Remote(format!("Failed to parse configuration: {e}")))?;

        Ok(config)
    }
}

/// Generate crawler configuration from a natural language prompt.
///
/// Standalone function that creates an engine and generates configuration.
///
/// # Arguments
/// * `api_url` - OpenAI-compatible chat completions endpoint
/// * `model_name` - Model identifier (e.g., "gpt-4", "llama3", "qwen2.5")
/// * `api_key` - Optional API key for authenticated endpoints
/// * `prompt` - Natural language description of crawling requirements
///
/// # Example
/// ```ignore
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
#[cfg(feature = "serde")]
pub async fn configure_crawler_from_prompt(
    api_url: &str,
    model_name: &str,
    api_key: Option<&str>,
    prompt: &str,
) -> EngineResult<PromptConfiguration> {
    let engine = RemoteMultimodalEngine::new(api_url, model_name, None)
        .with_api_key(api_key);
    engine.configure_from_prompt(prompt).await
}

// ============================================================================
// PHASE 1: SIMPLIFIED AGENTIC APIs
// ============================================================================

/// Result of a single action execution via `act()`.
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ActResult {
    /// Whether the action was executed successfully.
    pub success: bool,
    /// Description of the action that was taken.
    pub action_taken: String,
    /// The specific action executed (if any).
    pub action_type: Option<String>,
    /// Base64-encoded screenshot after the action.
    pub screenshot: Option<String>,
    /// Error message if the action failed.
    pub error: Option<String>,
    /// Token usage for this action.
    pub usage: AutomationUsage,
}

/// An interactive element found on the page.
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct InteractiveElement {
    /// CSS selector to target this element.
    pub selector: String,
    /// Type of element (button, link, input, select, etc.).
    pub element_type: String,
    /// Visible text content of the element.
    pub text: String,
    /// Brief description of what this element does.
    pub description: String,
    /// Whether the element is currently visible.
    #[cfg_attr(feature = "serde", serde(default))]
    pub visible: bool,
    /// Whether the element is enabled/clickable.
    #[cfg_attr(feature = "serde", serde(default = "default_true"))]
    pub enabled: bool,
}

#[cfg(feature = "serde")]
fn default_true() -> bool {
    true
}

/// Information about a form on the page.
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FormInfo {
    /// CSS selector for the form.
    pub selector: String,
    /// Form name or ID if available.
    pub name: Option<String>,
    /// Action URL of the form.
    pub action: Option<String>,
    /// HTTP method (GET, POST).
    pub method: Option<String>,
    /// Input fields in the form.
    pub fields: Vec<FormField>,
}

/// A field within a form.
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FormField {
    /// Field name attribute.
    pub name: String,
    /// Field type (text, email, password, submit, etc.).
    pub field_type: String,
    /// Placeholder or label text.
    pub label: Option<String>,
    /// Whether the field is required.
    #[cfg_attr(feature = "serde", serde(default))]
    pub required: bool,
    /// Current value if any.
    pub value: Option<String>,
}

/// A navigation option on the page.
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct NavigationOption {
    /// Text of the navigation link.
    pub text: String,
    /// URL the link points to.
    pub url: Option<String>,
    /// CSS selector for the element.
    pub selector: String,
    /// Whether this is the current/active page.
    #[cfg_attr(feature = "serde", serde(default))]
    pub is_current: bool,
}

/// Result of observing the current page state via `observe()`.
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PageObservation {
    /// Current page URL.
    pub url: String,
    /// Page title.
    pub title: String,
    /// AI-generated description of what the page is about.
    pub description: String,
    /// Main purpose or type of the page (e.g., "login form", "product listing", "article").
    pub page_type: String,
    /// Interactive elements found on the page (buttons, links, inputs).
    pub interactive_elements: Vec<InteractiveElement>,
    /// Forms found on the page.
    pub forms: Vec<FormInfo>,
    /// Navigation options (menu items, breadcrumbs, pagination).
    pub navigation: Vec<NavigationOption>,
    /// Suggested next actions based on page content.
    pub suggested_actions: Vec<String>,
    /// Base64-encoded screenshot of the page.
    pub screenshot: Option<String>,
    /// Token usage for this observation.
    pub usage: AutomationUsage,
}

/// System prompt for the `act()` single-action API.
#[cfg(feature = "chrome")]
const ACT_SYSTEM_PROMPT: &str = r##"
You are a browser automation assistant that executes single actions based on natural language instructions.

Given a screenshot and page context, determine the SINGLE best action to fulfill the user's instruction.

You MUST output a JSON object with this exact shape:
{
  "action_taken": "description of what you're doing",
  "action_type": "Click|Fill|Type|Scroll|Wait|Evaluate",
  "success": true,
  "steps": [<single WebAutomation action>]
}

Rules:
1. Execute ONLY ONE action per request
2. Choose the most specific selector possible
3. If the instruction cannot be fulfilled, set success: false and explain in action_taken
4. Prefer CSS selectors over coordinates

Available WebAutomation actions:
- { "Click": "css_selector" }
- { "Fill": { "selector": "css_selector", "value": "text" } }
- { "Type": { "value": "text", "modifier": null } }
- { "ScrollY": pixels }
- { "ScrollX": pixels }
- { "Wait": milliseconds }
- { "WaitFor": "css_selector" }
- { "WaitForAndClick": "css_selector" }
- { "Evaluate": "javascript_code" }

Examples:
- "click the login button" → { "Click": "button[type='submit']" } or { "Click": ".login-btn" }
- "type hello in the search box" → { "Fill": { "selector": "input[name='search']", "value": "hello" } }
- "scroll down" → { "ScrollY": 500 }
"##;

/// System prompt for the `observe()` page understanding API.
#[cfg(feature = "chrome")]
const OBSERVE_SYSTEM_PROMPT: &str = r##"
You are a page analysis assistant that provides detailed observations about web pages.

Given a screenshot and optional HTML context, analyze the page and provide structured information.

You MUST output a JSON object with this exact shape:
{
  "description": "Brief description of what this page is about",
  "page_type": "login_form|product_listing|article|search_results|checkout|dashboard|homepage|error|other",
  "interactive_elements": [
    {
      "selector": "css_selector",
      "element_type": "button|link|input|select|checkbox|radio|textarea",
      "text": "visible text",
      "description": "what this element does",
      "visible": true,
      "enabled": true
    }
  ],
  "forms": [
    {
      "selector": "form_selector",
      "name": "form name or null",
      "action": "form action URL or null",
      "method": "GET|POST",
      "fields": [
        {
          "name": "field_name",
          "field_type": "text|email|password|submit|hidden|checkbox|radio|select",
          "label": "field label or placeholder",
          "required": true,
          "value": "current value or null"
        }
      ]
    }
  ],
  "navigation": [
    {
      "text": "link text",
      "url": "href or null",
      "selector": "css_selector",
      "is_current": false
    }
  ],
  "suggested_actions": [
    "Natural language suggestion of what can be done",
    "Another possible action"
  ]
}

Focus on:
1. Elements the user can interact with
2. The main purpose of the page
3. Available navigation paths
4. Any forms and their fields
5. Actionable suggestions based on page content
"##;

/// System prompt for the `extract()` simple extraction API.
#[cfg(any(feature = "chrome", feature = "serde"))]
const EXTRACT_SYSTEM_PROMPT: &str = r##"
You are a data extraction assistant that extracts structured data from web pages.

Given page content (HTML and/or screenshot), extract the requested data.

You MUST output a JSON object with this exact shape:
{
  "success": true,
  "data": <extracted_data_matching_requested_format>
}

Rules:
1. Extract ONLY the data requested by the user
2. If a schema is provided, the "data" field MUST conform to it
3. If data cannot be found, set success: false and data: null
4. Be precise - extract actual values from the page, don't infer or guess
5. Handle missing data gracefully with null values
"##;

#[cfg(feature = "serde")]
impl RemoteMultimodalEngine {
    /// Execute a single action on the page using natural language.
    ///
    /// This is a simplified API that translates a natural language instruction
    /// into a single browser action and executes it.
    ///
    /// # Arguments
    /// * `page` - The Chrome page to act on
    /// * `instruction` - Natural language instruction (e.g., "click the login button")
    ///
    /// # Example
    /// ```ignore
    /// let engine = RemoteMultimodalEngine::new(api_url, model, None);
    /// let result = engine.act(&page, "click the submit button").await?;
    /// if result.success {
    ///     println!("Action taken: {}", result.action_taken);
    /// }
    /// ```
    #[cfg(feature = "chrome")]
    pub async fn act(
        &self,
        page: &chromiumoxide::Page,
        instruction: &str,
    ) -> EngineResult<ActResult> {
        use serde::Serialize;

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
        struct ImageUrl {
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
        }

        // Capture screenshot
        let screenshot = self.take_final_screenshot(page).await?;
        let screenshot_url = format!("data:image/png;base64,{}", screenshot);

        // Get page context
        let url = page.url().await.ok().flatten().unwrap_or_default();
        let title = page.get_title().await.ok().flatten().unwrap_or_default();

        // Build user prompt
        let user_text = format!(
            "PAGE CONTEXT:\n- URL: {}\n- Title: {}\n\nINSTRUCTION:\n{}\n\nExecute this single action.",
            url, title, instruction
        );

        let request_body = InferenceRequest {
            model: self.model_name.clone(),
            messages: vec![
                Message {
                    role: "system".into(),
                    content: vec![ContentBlock {
                        content_type: "text".into(),
                        text: Some(ACT_SYSTEM_PROMPT.to_string()),
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
                            image_url: Some(ImageUrl { url: screenshot_url }),
                        },
                    ],
                },
            ],
            temperature: 0.1,
            max_tokens: 1024,
            response_format: Some(ResponseFormat {
                format_type: "json_object".into(),
            }),
        };

        // Acquire permit and send request
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
                "act() failed with status {}: {}",
                status, raw_body
            )));
        }

        let root: serde_json::Value = serde_json::from_str(&raw_body)?;
        let content = extract_assistant_content(&root)
            .ok_or(EngineError::MissingField("choices[0].message.content"))?;
        let usage = extract_usage(&root);

        let plan_value = best_effort_parse_json_object(&content)?;

        let action_taken = plan_value
            .get("action_taken")
            .and_then(|v| v.as_str())
            .unwrap_or("action executed")
            .to_string();

        let action_type = plan_value
            .get("action_type")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let success = plan_value
            .get("success")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        // Execute the action if provided
        let mut action_success = success;
        let mut error = None;

        if let Some(steps_arr) = plan_value.get("steps").and_then(|v| v.as_array()) {
            if let Ok(steps) = map_to_web_automation(steps_arr.clone()) {
                for step in steps {
                    if !step.run(page).await {
                        action_success = false;
                        error = Some(format!("Failed to execute: {:?}", step));
                        break;
                    }
                }
            }
        }

        // Take screenshot after action
        let final_screenshot = self.take_final_screenshot(page).await.ok();

        Ok(ActResult {
            success: action_success,
            action_taken,
            action_type,
            screenshot: final_screenshot,
            error,
            usage,
        })
    }

    /// Observe the current page state and return structured information.
    ///
    /// This method analyzes the page without taking any actions, providing
    /// information about interactive elements, forms, navigation options,
    /// and suggested next actions.
    ///
    /// # Arguments
    /// * `page` - The Chrome page to observe
    ///
    /// # Example
    /// ```ignore
    /// let engine = RemoteMultimodalEngine::new(api_url, model, None);
    /// let observation = engine.observe(&page).await?;
    /// println!("Page type: {}", observation.page_type);
    /// for elem in &observation.interactive_elements {
    ///     println!("- {} ({}): {}", elem.text, elem.element_type, elem.selector);
    /// }
    /// ```
    #[cfg(feature = "chrome")]
    pub async fn observe(
        &self,
        page: &chromiumoxide::Page,
    ) -> EngineResult<PageObservation> {
        use serde::Serialize;

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
        struct ImageUrl {
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
        }

        // Capture screenshot
        let screenshot = self.take_final_screenshot(page).await?;
        let screenshot_url = format!("data:image/png;base64,{}", screenshot);

        // Get page context
        let url = page.url().await.ok().flatten().unwrap_or_default();
        let title = page.get_title().await.ok().flatten().unwrap_or_default();

        // Get HTML for additional context (truncated)
        let html = page.content().await.unwrap_or_default();
        let html_truncated = truncate_utf8_tail(&html, 16000);

        // Build user prompt
        let user_text = format!(
            "PAGE CONTEXT:\n- URL: {}\n- Title: {}\n\nHTML (truncated):\n{}\n\nAnalyze this page and provide structured observations.",
            url, title, html_truncated
        );

        let request_body = InferenceRequest {
            model: self.model_name.clone(),
            messages: vec![
                Message {
                    role: "system".into(),
                    content: vec![ContentBlock {
                        content_type: "text".into(),
                        text: Some(OBSERVE_SYSTEM_PROMPT.to_string()),
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
                            image_url: Some(ImageUrl { url: screenshot_url }),
                        },
                    ],
                },
            ],
            temperature: 0.1,
            max_tokens: 4096,
            response_format: Some(ResponseFormat {
                format_type: "json_object".into(),
            }),
        };

        // Acquire permit and send request
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
                "observe() failed with status {}: {}",
                status, raw_body
            )));
        }

        let root: serde_json::Value = serde_json::from_str(&raw_body)?;
        let content = extract_assistant_content(&root)
            .ok_or(EngineError::MissingField("choices[0].message.content"))?;
        let usage = extract_usage(&root);

        let obs_value = best_effort_parse_json_object(&content)?;

        // Parse the observation
        let description = obs_value
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let page_type = obs_value
            .get("page_type")
            .and_then(|v| v.as_str())
            .unwrap_or("other")
            .to_string();

        let interactive_elements = obs_value
            .get("interactive_elements")
            .and_then(|v| serde_json::from_value::<Vec<InteractiveElement>>(v.clone()).ok())
            .unwrap_or_default();

        let forms = obs_value
            .get("forms")
            .and_then(|v| serde_json::from_value::<Vec<FormInfo>>(v.clone()).ok())
            .unwrap_or_default();

        let navigation = obs_value
            .get("navigation")
            .and_then(|v| serde_json::from_value::<Vec<NavigationOption>>(v.clone()).ok())
            .unwrap_or_default();

        let suggested_actions = obs_value
            .get("suggested_actions")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        Ok(PageObservation {
            url,
            title,
            description,
            page_type,
            interactive_elements,
            forms,
            navigation,
            suggested_actions,
            screenshot: Some(screenshot),
            usage,
        })
    }

    /// Extract structured data from a page using natural language.
    ///
    /// This is a simplified extraction API that takes a prompt describing
    /// what data to extract and optionally a JSON schema for the output.
    ///
    /// # Arguments
    /// * `page` - The Chrome page to extract from
    /// * `prompt` - Natural language description of what to extract
    /// * `schema` - Optional JSON schema string for the output format
    ///
    /// # Example
    /// ```ignore
    /// let engine = RemoteMultimodalEngine::new(api_url, model, None);
    ///
    /// // Simple extraction
    /// let data = engine.extract(&page, "get all product names and prices", None).await?;
    ///
    /// // With schema
    /// let schema = r#"{"type": "array", "items": {"type": "object", "properties": {"name": {"type": "string"}, "price": {"type": "number"}}}}"#;
    /// let data = engine.extract(&page, "extract product information", Some(schema)).await?;
    /// ```
    #[cfg(feature = "chrome")]
    pub async fn extract_page(
        &self,
        page: &chromiumoxide::Page,
        prompt: &str,
        schema: Option<&str>,
    ) -> EngineResult<serde_json::Value> {
        use serde::Serialize;

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
        struct ImageUrl {
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
        }

        // Capture screenshot
        let screenshot = self.take_final_screenshot(page).await?;
        let screenshot_url = format!("data:image/png;base64,{}", screenshot);

        // Get page context
        let url = page.url().await.ok().flatten().unwrap_or_default();
        let title = page.get_title().await.ok().flatten().unwrap_or_default();

        // Get HTML for additional context
        let html = page.content().await.unwrap_or_default();
        let html_truncated = truncate_utf8_tail(&html, self.cfg.html_max_bytes);

        // Build user prompt
        let mut user_text = format!(
            "PAGE CONTEXT:\n- URL: {}\n- Title: {}\n\nHTML:\n{}\n\nEXTRACTION REQUEST:\n{}",
            url, title, html_truncated, prompt
        );

        if let Some(s) = schema {
            user_text.push_str("\n\nOUTPUT SCHEMA (the 'data' field MUST conform to this):\n");
            user_text.push_str(s);
        }

        let request_body = InferenceRequest {
            model: self.model_name.clone(),
            messages: vec![
                Message {
                    role: "system".into(),
                    content: vec![ContentBlock {
                        content_type: "text".into(),
                        text: Some(EXTRACT_SYSTEM_PROMPT.to_string()),
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
                            image_url: Some(ImageUrl { url: screenshot_url }),
                        },
                    ],
                },
            ],
            temperature: 0.0,
            max_tokens: 4096,
            response_format: Some(ResponseFormat {
                format_type: "json_object".into(),
            }),
        };

        // Acquire permit and send request
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
                "extract() failed with status {}: {}",
                status, raw_body
            )));
        }

        let root: serde_json::Value = serde_json::from_str(&raw_body)?;
        let content = extract_assistant_content(&root)
            .ok_or(EngineError::MissingField("choices[0].message.content"))?;

        let result_value = best_effort_parse_json_object(&content)?;

        // Return the data field, or the whole response if no data field
        Ok(result_value
            .get("data")
            .cloned()
            .unwrap_or(result_value))
    }

    /// Execute an action with self-healing selector cache.
    ///
    /// This is an enhanced version of `act()` that uses a selector cache to
    /// avoid repeated LLM calls for similar actions. If a cached selector
    /// fails, it automatically re-queries the LLM and updates the cache.
    ///
    /// # Arguments
    /// * `page` - The Chrome page to act on
    /// * `instruction` - Natural language instruction (e.g., "click the login button")
    /// * `cache` - Mutable reference to the selector cache
    ///
    /// # Self-Healing Flow
    /// 1. Check cache for a known selector matching the instruction
    /// 2. If found, try the cached selector directly
    /// 3. If selector fails → invalidate cache, fall back to LLM
    /// 4. On success, update cache with the working selector
    ///
    /// # Example
    /// ```ignore
    /// let engine = RemoteMultimodalEngine::new(api_url, model, None);
    /// let mut cache = SelectorCache::new();
    ///
    /// // First call queries LLM, caches selector
    /// let result = engine.act_cached(&page, "click submit button", &mut cache).await?;
    ///
    /// // Second call uses cached selector (no LLM call if selector works)
    /// let result2 = engine.act_cached(&page, "click submit button", &mut cache).await?;
    /// ```
    #[cfg(feature = "chrome")]
    pub async fn act_cached(
        &self,
        page: &chromiumoxide::Page,
        instruction: &str,
        cache: &mut SelectorCache,
    ) -> EngineResult<ActResult> {
        // Extract domain for cache scoping
        let url = page.url().await.ok().flatten().unwrap_or_default();
        let domain = url::Url::parse(&url)
            .ok()
            .and_then(|u| u.host_str().map(|s| s.to_string()));

        // Check cache for a known selector
        if let Some(cached_selector) = cache.get(instruction, domain.as_deref()) {
            let selector = cached_selector.to_string();
            log::debug!("act_cached: trying cached selector '{}' for '{}'", selector, instruction);

            // Try the cached selector directly
            let try_result = self.try_selector(page, &selector).await;

            if try_result {
                // Selector worked, update cache and return success
                cache.record_success(instruction, &selector, domain.as_deref());

                let final_screenshot = self.take_final_screenshot(page).await.ok();

                return Ok(ActResult {
                    success: true,
                    action_taken: format!("Used cached selector: {}", selector),
                    action_type: Some("cached_click".to_string()),
                    screenshot: final_screenshot,
                    error: None,
                    usage: AutomationUsage::default(), // No LLM call
                });
            } else {
                // Cached selector failed, invalidate and fall through to LLM
                log::debug!("act_cached: cached selector failed, invalidating and querying LLM");
                cache.record_failure(instruction);
            }
        }

        // Fall back to regular act() with LLM
        let result = self.act(page, instruction).await?;

        // If successful and we got a selector, cache it
        if result.success {
            // Try to extract selector from the action taken
            if let Some(selector) = Self::extract_selector_from_action(&result.action_taken) {
                cache.record_success(instruction, &selector, domain.as_deref());
            }
        }

        Ok(result)
    }

    /// Try to click an element using a CSS selector.
    #[cfg(feature = "chrome")]
    async fn try_selector(&self, page: &chromiumoxide::Page, selector: &str) -> bool {
        // Try to find and click the element
        match page.find_element(selector).await {
            Ok(elem) => {
                // Element found, try to click it
                elem.click().await.is_ok()
            }
            Err(_) => false,
        }
    }

    /// Extract a CSS selector from an action description.
    #[cfg(feature = "chrome")]
    fn extract_selector_from_action(action: &str) -> Option<String> {
        // Look for common selector patterns
        // Pattern: selector='...' or selector: '...'
        if let Some(start) = action.find("selector") {
            let rest = &action[start..];
            if let Some(quote_start) = rest.find(['\'', '"']) {
                let quote_char = rest.chars().nth(quote_start)?;
                let after_quote = &rest[quote_start + 1..];
                if let Some(quote_end) = after_quote.find(quote_char) {
                    return Some(after_quote[..quote_end].to_string());
                }
            }
        }
        // Pattern: #id or .class at word boundary
        for word in action.split_whitespace() {
            if (word.starts_with('#') || word.starts_with('.')) && word.len() > 1 {
                let clean = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '#' && c != '.' && c != '-' && c != '_');
                if !clean.is_empty() {
                    return Some(clean.to_string());
                }
            }
        }
        None
    }

    /// Discover and map URLs on a page using AI-powered analysis.
    ///
    /// This method analyzes a page to discover all URLs and categorize them
    /// based on relevance to the provided prompt. It's useful for intelligent
    /// crawling where you want to prioritize certain types of pages.
    ///
    /// # Arguments
    /// * `page` - The Chrome page to analyze
    /// * `prompt` - What kind of content you're looking for (e.g., "product pages", "documentation")
    ///
    /// # Example
    /// ```ignore
    /// let engine = RemoteMultimodalEngine::new(api_url, model, None);
    /// let map = engine.map(&page, "Find all product listing pages").await?;
    ///
    /// println!("Page relevance: {}", map.relevance);
    /// for url in map.urls.iter().filter(|u| u.recommended) {
    ///     println!("Recommended: {} - {}", url.url, url.description);
    /// }
    /// ```
    #[cfg(feature = "chrome")]
    pub async fn map(
        &self,
        page: &chromiumoxide::Page,
        prompt: &str,
    ) -> EngineResult<MapResult> {
        use serde::Serialize;

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
        struct ImageUrl {
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
        }

        // Capture screenshot
        let screenshot = self.take_final_screenshot(page).await?;
        let screenshot_url = format!("data:image/png;base64,{}", screenshot);

        // Get page context
        let url = page.url().await.ok().flatten().unwrap_or_default();
        let title = page.get_title().await.ok().flatten().unwrap_or_default();

        // Get HTML for link extraction
        let html = page.content().await.unwrap_or_default();
        let html_truncated = truncate_utf8_tail(&html, self.cfg.html_max_bytes);

        // Build system prompt for mapping
        let map_system_prompt = r##"
You are a web page analyzer that discovers and categorizes URLs.

Given a page (screenshot + HTML) and a search prompt, analyze the page and identify all URLs.
For each URL, determine its relevance to the user's search prompt.

You MUST output a JSON object with this exact shape:
{
  "summary": "Brief description of what the page contains",
  "relevance": 0.0 to 1.0 (how relevant is THIS page to the prompt),
  "urls": [
    {
      "url": "full URL",
      "text": "link text or title",
      "description": "what this URL likely contains",
      "relevance": 0.0 to 1.0,
      "recommended": true/false (should user visit this?),
      "category": "navigation|content|product|documentation|external|auth|other"
    }
  ],
  "suggested_next": ["url1", "url2"] (top 3-5 URLs to explore next based on prompt)
}

Rules:
1. Include ALL URLs found on the page
2. Score relevance based on how well the URL matches the search prompt
3. Mark URLs as recommended if they're likely to contain what the user is looking for
4. Categorize URLs to help with crawl prioritization
5. For suggested_next, pick the most promising URLs to explore
"##;

        let user_text = format!(
            "PAGE CONTEXT:\n- URL: {}\n- Title: {}\n\nHTML:\n{}\n\nSEARCH PROMPT:\n{}\n\nAnalyze this page and discover all URLs. Rate their relevance to the search prompt.",
            url, title, html_truncated, prompt
        );

        let request_body = InferenceRequest {
            model: self.model_name.clone(),
            messages: vec![
                Message {
                    role: "system".into(),
                    content: vec![ContentBlock {
                        content_type: "text".into(),
                        text: Some(map_system_prompt.to_string()),
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
                            image_url: Some(ImageUrl { url: screenshot_url.clone() }),
                        },
                    ],
                },
            ],
            temperature: 0.1,
            max_tokens: 4096,
            response_format: Some(ResponseFormat {
                format_type: "json_object".into(),
            }),
        };

        // Acquire permit and send request
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
                "map() failed with status {}: {}",
                status, raw_body
            )));
        }

        let root: serde_json::Value = serde_json::from_str(&raw_body)?;
        let content = extract_assistant_content(&root)
            .ok_or(EngineError::MissingField("choices[0].message.content"))?;
        let usage = extract_usage(&root);

        let map_value = best_effort_parse_json_object(&content)?;

        // Parse the map result
        let summary = map_value
            .get("summary")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let relevance = map_value
            .get("relevance")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0) as f32;

        let urls = map_value
            .get("urls")
            .and_then(|v| serde_json::from_value::<Vec<DiscoveredUrl>>(v.clone()).ok())
            .unwrap_or_default();

        let suggested_next = map_value
            .get("suggested_next")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        Ok(MapResult {
            urls,
            relevance,
            summary,
            suggested_next,
            screenshot: Some(screenshot),
            usage,
        })
    }

    /// Extract data using structured output mode (native JSON schema).
    ///
    /// This method uses the API's native structured output support (like OpenAI's
    /// `response_format.json_schema`) to enforce a specific output structure.
    /// This is more reliable than prompt-based schema enforcement.
    ///
    /// # Arguments
    /// * `page` - The Chrome page to extract from
    /// * `prompt` - What data to extract
    /// * `config` - Structured output configuration with JSON schema
    ///
    /// # Example
    /// ```ignore
    /// let engine = RemoteMultimodalEngine::new(api_url, model, None);
    ///
    /// let schema = serde_json::json!({
    ///     "type": "object",
    ///     "properties": {
    ///         "products": {
    ///             "type": "array",
    ///             "items": {
    ///                 "type": "object",
    ///                 "properties": {
    ///                     "name": { "type": "string" },
    ///                     "price": { "type": "number" }
    ///                 },
    ///                 "required": ["name", "price"]
    ///             }
    ///         }
    ///     },
    ///     "required": ["products"]
    /// });
    ///
    /// let config = StructuredOutputConfig::strict(schema);
    /// let data = engine.extract_structured(&page, "Extract all products", config).await?;
    /// ```
    #[cfg(feature = "chrome")]
    pub async fn extract_structured(
        &self,
        page: &chromiumoxide::Page,
        prompt: &str,
        config: StructuredOutputConfig,
    ) -> EngineResult<serde_json::Value> {
        use serde::Serialize;

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
        struct ImageUrl {
            url: String,
        }

        #[derive(Serialize)]
        struct Message {
            role: String,
            content: Vec<ContentBlock>,
        }

        // OpenAI-style structured output format
        #[derive(Serialize)]
        struct JsonSchemaFormat {
            #[serde(rename = "type")]
            format_type: String,
            #[serde(skip_serializing_if = "Option::is_none")]
            json_schema: Option<JsonSchemaSpec>,
        }

        #[derive(Serialize)]
        struct JsonSchemaSpec {
            name: String,
            #[serde(skip_serializing_if = "Option::is_none")]
            schema: Option<serde_json::Value>,
            #[serde(skip_serializing_if = "std::ops::Not::not")]
            strict: bool,
        }

        #[derive(Serialize)]
        struct InferenceRequest {
            model: String,
            messages: Vec<Message>,
            temperature: f32,
            max_tokens: u16,
            #[serde(skip_serializing_if = "Option::is_none")]
            response_format: Option<JsonSchemaFormat>,
        }

        // Capture screenshot
        let screenshot = self.take_final_screenshot(page).await?;
        let screenshot_url = format!("data:image/png;base64,{}", screenshot);

        // Get page context
        let url = page.url().await.ok().flatten().unwrap_or_default();
        let title = page.get_title().await.ok().flatten().unwrap_or_default();

        // Get HTML
        let html = page.content().await.unwrap_or_default();
        let html_truncated = truncate_utf8_tail(&html, self.cfg.html_max_bytes);

        let user_text = format!(
            "PAGE CONTEXT:\n- URL: {}\n- Title: {}\n\nHTML:\n{}\n\nEXTRACTION REQUEST:\n{}",
            url, title, html_truncated, prompt
        );

        // Build response format with JSON schema
        let response_format = if config.enabled && config.schema.is_some() {
            Some(JsonSchemaFormat {
                format_type: "json_schema".to_string(),
                json_schema: Some(JsonSchemaSpec {
                    name: config.schema_name.clone(),
                    schema: config.schema.clone(),
                    strict: config.strict,
                }),
            })
        } else {
            Some(JsonSchemaFormat {
                format_type: "json_object".to_string(),
                json_schema: None,
            })
        };

        let request_body = InferenceRequest {
            model: self.model_name.clone(),
            messages: vec![
                Message {
                    role: "system".into(),
                    content: vec![ContentBlock {
                        content_type: "text".into(),
                        text: Some(EXTRACT_SYSTEM_PROMPT.to_string()),
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
                            image_url: Some(ImageUrl { url: screenshot_url }),
                        },
                    ],
                },
            ],
            temperature: 0.0,
            max_tokens: 4096,
            response_format,
        };

        // Acquire permit and send request
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
                "extract_structured() failed with status {}: {}",
                status, raw_body
            )));
        }

        let root: serde_json::Value = serde_json::from_str(&raw_body)?;
        let content = extract_assistant_content(&root)
            .ok_or(EngineError::MissingField("choices[0].message.content"))?;

        // With structured output, the response should already be valid JSON
        let result_value: serde_json::Value = serde_json::from_str(&content)
            .or_else(|_| best_effort_parse_json_object(&content))?;

        // Return the data field, or the whole response
        Ok(result_value
            .get("data")
            .cloned()
            .unwrap_or(result_value))
    }

    // ========================================================================
    // PHASE 3: ADVANCED AGENTIC METHODS
    // ========================================================================

    /// Execute an autonomous agent to achieve a goal.
    ///
    /// This is the main entry point for goal-oriented automation. The agent will
    /// autonomously plan and execute actions to achieve the specified goal,
    /// handling navigation, interactions, and error recovery.
    ///
    /// # Arguments
    /// * `page` - The Chrome page to operate on
    /// * `config` - Agent configuration including goal, limits, and recovery strategy
    ///
    /// # Example
    /// ```ignore
    /// let engine = RemoteMultimodalEngine::new(api_url, model, None);
    ///
    /// let config = AgentConfig::new("Find and add the cheapest laptop to cart")
    ///     .with_max_steps(30)
    ///     .with_success_url("/cart")
    ///     .with_extraction("Extract cart total and items");
    ///
    /// let result = engine.execute(&page, config).await?;
    /// if result.success {
    ///     println!("Goal achieved in {} steps", result.steps_taken);
    /// }
    /// ```
    #[cfg(feature = "chrome")]
    pub async fn execute(
        &self,
        page: &chromiumoxide::Page,
        config: AgentConfig,
    ) -> EngineResult<AgentResult> {
        let start_time = std::time::Instant::now();
        let start_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        let mut result = AgentResult {
            goal: config.goal.clone(),
            ..Default::default()
        };

        let mut events = Vec::new();
        events.push(AgentEvent::Started {
            goal: config.goal.clone(),
            timestamp_ms: start_ms,
        });

        let mut cache = if config.use_cache {
            SelectorCache::new()
        } else {
            SelectorCache::with_capacity(0)
        };

        let mut total_usage = AutomationUsage::default();
        let mut action_history = Vec::new();
        let timeout = std::time::Duration::from_millis(config.timeout_ms);

        // Main agent loop
        for step in 1..=config.max_steps {
            // Check timeout
            if start_time.elapsed() > timeout {
                events.push(AgentEvent::Aborted {
                    step,
                    reason: "Timeout exceeded".to_string(),
                });
                result.error = Some("Agent timed out".to_string());
                break;
            }

            let current_url = page.url().await.ok().flatten().unwrap_or_default();

            // Check for goal completion
            if let Some(reason) = self.check_goal_completion(page, &config).await {
                events.push(AgentEvent::GoalDetected {
                    step,
                    reason: reason.clone(),
                });
                result.success = true;

                // Extract data if requested
                if config.extract_on_success {
                    if let Some(prompt) = &config.extraction_prompt {
                        if let Ok(data) = self.extract_page(page, prompt, None).await {
                            result.extracted = Some(data.clone());
                            events.push(AgentEvent::Extracted { step, data });
                        }
                    }
                }
                break;
            }

            events.push(AgentEvent::Planning {
                step,
                current_url: current_url.clone(),
            });

            // Ask the LLM what to do next
            let plan_result = self.plan_next_action(page, &config.goal, &action_history).await;

            let (action_instruction, is_done) = match plan_result {
                Ok((instruction, done)) => (instruction, done),
                Err(e) => {
                    events.push(AgentEvent::ActionFailed {
                        step,
                        action: "planning".to_string(),
                        error: e.to_string(),
                        will_retry: false,
                    });
                    result.error = Some(format!("Planning failed: {}", e));
                    break;
                }
            };

            // Check if LLM thinks we're done
            if is_done {
                events.push(AgentEvent::GoalDetected {
                    step,
                    reason: "LLM indicated goal completion".to_string(),
                });
                result.success = true;

                if config.extract_on_success {
                    if let Some(prompt) = &config.extraction_prompt {
                        if let Ok(data) = self.extract_page(page, prompt, None).await {
                            result.extracted = Some(data.clone());
                            events.push(AgentEvent::Extracted { step, data });
                        }
                    }
                }
                break;
            }

            events.push(AgentEvent::Executing {
                step,
                action: action_instruction.clone(),
            });

            // Execute the action with retry logic
            let action_start = std::time::Instant::now();
            let mut retries = 0;
            #[allow(unused_assignments)]
            let mut action_result = None;

            loop {
                let act_result = if config.use_cache {
                    self.act_cached(page, &action_instruction, &mut cache).await
                } else {
                    self.act(page, &action_instruction).await
                };

                match act_result {
                    Ok(r) => {
                        total_usage.prompt_tokens += r.usage.prompt_tokens;
                        total_usage.completion_tokens += r.usage.completion_tokens;
                        total_usage.total_tokens += r.usage.total_tokens;

                        if r.success {
                            action_result = Some(Ok(r));
                            break;
                        } else {
                            // Action failed
                            match config.recovery_strategy {
                                RecoveryStrategy::Retry if retries < config.max_retries => {
                                    retries += 1;
                                    events.push(AgentEvent::Recovering {
                                        step,
                                        strategy: RecoveryStrategy::Retry,
                                        attempt: retries,
                                    });
                                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                                    continue;
                                }
                                RecoveryStrategy::Skip => {
                                    action_result = Some(Ok(r));
                                    break;
                                }
                                RecoveryStrategy::Abort => {
                                    action_result = Some(Ok(r));
                                    break;
                                }
                                _ => {
                                    action_result = Some(Ok(r));
                                    break;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        if retries < config.max_retries && config.recovery_strategy == RecoveryStrategy::Retry {
                            retries += 1;
                            events.push(AgentEvent::Recovering {
                                step,
                                strategy: RecoveryStrategy::Retry,
                                attempt: retries,
                            });
                            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                            continue;
                        }
                        action_result = Some(Err(e));
                        break;
                    }
                }
            }

            let action_duration = action_start.elapsed().as_millis() as u64;
            let url_after = page.url().await.ok().flatten();

            match action_result {
                Some(Ok(r)) => {
                    let record = AgentActionRecord {
                        step,
                        action: action_instruction.clone(),
                        success: r.success,
                        duration_ms: action_duration,
                        url_before: current_url,
                        url_after: url_after.clone(),
                        error: r.error.clone(),
                        retries,
                    };
                    action_history.push(record.clone());

                    if r.success {
                        events.push(AgentEvent::ActionSuccess {
                            step,
                            action: r.action_taken.clone(),
                            duration_ms: action_duration,
                        });

                        if config.capture_screenshots {
                            if let Some(screenshot) = r.screenshot {
                                events.push(AgentEvent::Screenshot {
                                    step,
                                    data: screenshot,
                                });
                            }
                        }
                    } else {
                        events.push(AgentEvent::ActionFailed {
                            step,
                            action: action_instruction.clone(),
                            error: r.error.unwrap_or_default(),
                            will_retry: false,
                        });

                        if config.recovery_strategy == RecoveryStrategy::Abort {
                            result.error = Some("Action failed and abort strategy triggered".to_string());
                            break;
                        }
                    }
                }
                Some(Err(e)) => {
                    events.push(AgentEvent::ActionFailed {
                        step,
                        action: action_instruction.clone(),
                        error: e.to_string(),
                        will_retry: false,
                    });

                    let record = AgentActionRecord {
                        step,
                        action: action_instruction.clone(),
                        success: false,
                        duration_ms: action_duration,
                        url_before: current_url,
                        url_after,
                        error: Some(e.to_string()),
                        retries,
                    };
                    action_history.push(record);

                    if config.recovery_strategy == RecoveryStrategy::Abort {
                        result.error = Some(format!("Action error: {}", e));
                        break;
                    }
                }
                None => {}
            }

            // Small delay between actions
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }

        let duration_ms = start_time.elapsed().as_millis() as u64;

        events.push(AgentEvent::Completed {
            steps_taken: action_history.len(),
            duration_ms,
            success: result.success,
        });

        // Capture final screenshot
        if config.capture_screenshots {
            result.final_screenshot = self.take_final_screenshot(page).await.ok();
        }

        result.steps_taken = action_history.len();
        result.duration_ms = duration_ms;
        result.final_url = page.url().await.ok().flatten().unwrap_or_default();
        result.action_history = action_history;
        result.total_usage = total_usage;
        result.events = events;

        Ok(result)
    }

    /// Plan the next action based on current state and goal.
    #[cfg(feature = "chrome")]
    async fn plan_next_action(
        &self,
        page: &chromiumoxide::Page,
        goal: &str,
        history: &[AgentActionRecord],
    ) -> EngineResult<(String, bool)> {
        use serde::Serialize;

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
        struct ImageUrl {
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
        }

        let screenshot = self.take_final_screenshot(page).await?;
        let screenshot_url = format!("data:image/png;base64,{}", screenshot);

        let url = page.url().await.ok().flatten().unwrap_or_default();
        let title = page.get_title().await.ok().flatten().unwrap_or_default();

        // Build history summary
        let history_summary = if history.is_empty() {
            "No actions taken yet.".to_string()
        } else {
            history
                .iter()
                .map(|r| {
                    format!(
                        "Step {}: {} - {}",
                        r.step,
                        r.action,
                        if r.success { "success" } else { "failed" }
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        };

        let system_prompt = r##"You are an autonomous web agent working to achieve a goal.
Analyze the current page state and decide the next action.

You MUST respond with a JSON object:
{
  "thinking": "Brief analysis of current state and what to do next",
  "done": true/false (true if the goal has been achieved),
  "next_action": "Natural language instruction for the next action (e.g., 'click the Add to Cart button')"
}

Rules:
1. Be specific about elements (use visible text, position, or purpose)
2. One action at a time
3. Set done=true only when the goal is clearly achieved
4. If stuck, try a different approach
5. Consider the action history to avoid loops
"##;

        let user_text = format!(
            "GOAL: {}\n\nCURRENT PAGE:\n- URL: {}\n- Title: {}\n\nACTION HISTORY:\n{}\n\nWhat should I do next?",
            goal, url, title, history_summary
        );

        let request_body = InferenceRequest {
            model: self.model_name.clone(),
            messages: vec![
                Message {
                    role: "system".into(),
                    content: vec![ContentBlock {
                        content_type: "text".into(),
                        text: Some(system_prompt.to_string()),
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
                            image_url: Some(ImageUrl { url: screenshot_url }),
                        },
                    ],
                },
            ],
            temperature: 0.2,
            max_tokens: 1024,
            response_format: Some(ResponseFormat {
                format_type: "json_object".into(),
            }),
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
                "plan_next_action() failed: {} - {}",
                status, raw_body
            )));
        }

        let root: serde_json::Value = serde_json::from_str(&raw_body)?;
        let content = extract_assistant_content(&root)
            .ok_or(EngineError::MissingField("choices[0].message.content"))?;

        let plan = best_effort_parse_json_object(&content)?;

        let done = plan.get("done").and_then(|v| v.as_bool()).unwrap_or(false);
        let next_action = plan
            .get("next_action")
            .and_then(|v| v.as_str())
            .unwrap_or("observe the page")
            .to_string();

        Ok((next_action, done))
    }

    /// Check if the goal has been completed based on config criteria.
    #[cfg(feature = "chrome")]
    async fn check_goal_completion(
        &self,
        page: &chromiumoxide::Page,
        config: &AgentConfig,
    ) -> Option<String> {
        let current_url = page.url().await.ok().flatten().unwrap_or_default();

        // Check success URLs
        for success_url in &config.success_urls {
            if current_url.contains(success_url) {
                return Some(format!("URL contains '{}'", success_url));
            }
        }

        // Check success patterns in page content
        if !config.success_patterns.is_empty() {
            if let Ok(content) = page.content().await {
                for pattern in &config.success_patterns {
                    if content.contains(pattern) {
                        return Some(format!("Page contains '{}'", pattern));
                    }
                }
            }
        }

        None
    }

    /// Execute a chain of actions in sequence.
    ///
    /// This method executes multiple actions in order with support for
    /// conditional execution, error handling, and data extraction between steps.
    ///
    /// # Arguments
    /// * `page` - The Chrome page to operate on
    /// * `steps` - Vector of chain steps to execute
    ///
    /// # Example
    /// ```ignore
    /// let engine = RemoteMultimodalEngine::new(api_url, model, None);
    ///
    /// let steps = vec![
    ///     ChainStep::new("click the Login button"),
    ///     ChainStep::new("type 'user@example.com' in the email field"),
    ///     ChainStep::new("type 'password123' in the password field"),
    ///     ChainStep::new("click Submit")
    ///         .then_extract("Extract any error messages"),
    /// ];
    ///
    /// let result = engine.chain(&page, steps).await?;
    /// println!("Completed {} of {} steps", result.steps_succeeded, result.steps_executed);
    /// ```
    #[cfg(feature = "chrome")]
    pub async fn chain(
        &self,
        page: &chromiumoxide::Page,
        steps: Vec<ChainStep>,
    ) -> EngineResult<ChainResult> {
        let start_time = std::time::Instant::now();
        let mut result = ChainResult::default();
        let mut total_usage = AutomationUsage::default();
        let mut extractions = Vec::new();
        let mut previous_success = true;

        for (index, step) in steps.iter().enumerate() {
            let step_start = std::time::Instant::now();

            // Check condition
            let should_execute = self.evaluate_condition(page, &step.condition, previous_success).await;

            if !should_execute {
                result.step_results.push(ChainStepResult {
                    index,
                    instruction: step.instruction.clone(),
                    executed: false,
                    success: false,
                    action_taken: None,
                    error: None,
                    duration_ms: 0,
                    extracted: None,
                });
                result.steps_skipped += 1;
                continue;
            }

            result.steps_executed += 1;

            // Execute the action
            let act_result = self.act(page, &step.instruction).await;

            let step_duration = step_start.elapsed().as_millis() as u64;

            match act_result {
                Ok(r) => {
                    total_usage.prompt_tokens += r.usage.prompt_tokens;
                    total_usage.completion_tokens += r.usage.completion_tokens;
                    total_usage.total_tokens += r.usage.total_tokens;

                    previous_success = r.success;

                    // Handle extraction if requested
                    let extracted = if r.success {
                        if let Some(extract_prompt) = &step.extract {
                            self.extract_page(page, extract_prompt, None).await.ok()
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    if let Some(ref data) = extracted {
                        extractions.push(data.clone());
                    }

                    result.step_results.push(ChainStepResult {
                        index,
                        instruction: step.instruction.clone(),
                        executed: true,
                        success: r.success,
                        action_taken: Some(r.action_taken),
                        error: r.error.clone(),
                        duration_ms: step_duration,
                        extracted,
                    });

                    if r.success {
                        result.steps_succeeded += 1;
                    } else {
                        result.steps_failed += 1;
                        if !step.continue_on_failure {
                            break;
                        }
                    }
                }
                Err(e) => {
                    previous_success = false;
                    result.steps_failed += 1;

                    result.step_results.push(ChainStepResult {
                        index,
                        instruction: step.instruction.clone(),
                        executed: true,
                        success: false,
                        action_taken: None,
                        error: Some(e.to_string()),
                        duration_ms: step_duration,
                        extracted: None,
                    });

                    if !step.continue_on_failure {
                        break;
                    }
                }
            }

            // Small delay between steps
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }

        result.success = result.steps_failed == 0 && result.steps_executed > 0;
        result.duration_ms = start_time.elapsed().as_millis() as u64;
        result.total_usage = total_usage;
        result.extractions = extractions;

        Ok(result)
    }

    /// Evaluate a chain condition.
    #[cfg(feature = "chrome")]
    async fn evaluate_condition(
        &self,
        page: &chromiumoxide::Page,
        condition: &Option<ChainCondition>,
        previous_success: bool,
    ) -> bool {
        let cond = match condition {
            Some(c) => c,
            None => return true, // No condition = always execute
        };

        match cond {
            ChainCondition::Always => true,
            ChainCondition::PreviousSucceeded => previous_success,
            ChainCondition::PreviousFailed => !previous_success,
            ChainCondition::UrlContains(s) => {
                page.url()
                    .await
                    .ok()
                    .flatten()
                    .map(|u| u.contains(s))
                    .unwrap_or(false)
            }
            ChainCondition::UrlMatches(pattern) => {
                if let Ok(re) = regex::Regex::new(pattern) {
                    page.url()
                        .await
                        .ok()
                        .flatten()
                        .map(|u| re.is_match(&u))
                        .unwrap_or(false)
                } else {
                    false
                }
            }
            ChainCondition::PageContains(text) => {
                page.content()
                    .await
                    .map(|c| c.contains(text))
                    .unwrap_or(false)
            }
            ChainCondition::ElementExists(selector) => {
                page.find_element(selector).await.is_ok()
            }
        }
    }

    /// Execute a simple goal with minimal configuration.
    ///
    /// This is a convenience method that wraps `execute()` with sensible defaults.
    ///
    /// # Example
    /// ```ignore
    /// let result = engine.agent(&page, "Sign up for the newsletter").await?;
    /// ```
    #[cfg(feature = "chrome")]
    pub async fn agent(
        &self,
        page: &chromiumoxide::Page,
        goal: &str,
    ) -> EngineResult<AgentResult> {
        let config = AgentConfig::new(goal);
        self.execute(page, config).await
    }

    /// Execute a goal with extraction on completion.
    ///
    /// # Example
    /// ```ignore
    /// let result = engine.agent_extract(
    ///     &page,
    ///     "Navigate to the pricing page",
    ///     "Extract all pricing tiers with names and prices"
    /// ).await?;
    /// ```
    #[cfg(feature = "chrome")]
    pub async fn agent_extract(
        &self,
        page: &chromiumoxide::Page,
        goal: &str,
        extraction_prompt: &str,
    ) -> EngineResult<AgentResult> {
        let config = AgentConfig::new(goal).with_extraction(extraction_prompt);
        self.execute(page, config).await
    }

    /// Search the web and return structured results.
    ///
    /// Uses the configured search provider (Serper, Brave, Bing, or Tavily) to query
    /// the web and return relevant URLs with snippets.
    ///
    /// # Arguments
    /// * `query` - The search query
    /// * `options` - Optional search options (limit, country, etc.)
    /// * `client` - Optional HTTP client to reuse (from crawl)
    ///
    /// # Example
    /// ```ignore
    /// let results = engine.search("rust web crawlers", None, None).await?;
    /// for result in results.results {
    ///     println!("{}: {}", result.title, result.url);
    /// }
    /// ```
    #[cfg(feature = "search")]
    pub async fn search(
        &self,
        query: &str,
        options: Option<crate::features::search::SearchOptions>,
        client: Option<&reqwest::Client>,
    ) -> EngineResult<crate::features::search::SearchResults> {
        use crate::configuration::SearchProviderType;
        use crate::features::search::SearchProvider;

        let config = self.cfg.search_config.as_ref().ok_or_else(|| {
            EngineError::Unsupported("Search not configured - set search_config first")
        })?;

        if !config.is_enabled() {
            return Err(EngineError::Unsupported(
                "Search not enabled - provide API key or custom API URL",
            ));
        }

        let opts = options.unwrap_or_else(|| {
            config
                .default_options
                .clone()
                .unwrap_or_default()
        });

        match config.provider {
            #[cfg(feature = "search_serper")]
            SearchProviderType::Serper => {
                let mut provider =
                    crate::features::search_providers::SerperProvider::new(&config.api_key);
                if let Some(ref url) = config.api_url {
                    provider = provider.with_api_url(url);
                }
                provider
                    .search(query, &opts, client)
                    .await
                    .map_err(|e| EngineError::Remote(e.to_string()))
            }
            #[cfg(feature = "search_brave")]
            SearchProviderType::Brave => {
                let mut provider =
                    crate::features::search_providers::BraveProvider::new(&config.api_key);
                if let Some(ref url) = config.api_url {
                    provider = provider.with_api_url(url);
                }
                provider
                    .search(query, &opts, client)
                    .await
                    .map_err(|e| EngineError::Remote(e.to_string()))
            }
            #[cfg(feature = "search_bing")]
            SearchProviderType::Bing => {
                let mut provider =
                    crate::features::search_providers::BingProvider::new(&config.api_key);
                if let Some(ref url) = config.api_url {
                    provider = provider.with_api_url(url);
                }
                provider
                    .search(query, &opts, client)
                    .await
                    .map_err(|e| EngineError::Remote(e.to_string()))
            }
            #[cfg(feature = "search_tavily")]
            SearchProviderType::Tavily => {
                let mut provider =
                    crate::features::search_providers::TavilyProvider::new(&config.api_key);
                if let Some(ref url) = config.api_url {
                    provider = provider.with_api_url(url);
                }
                provider
                    .search(query, &opts, client)
                    .await
                    .map_err(|e| EngineError::Remote(e.to_string()))
            }
            #[allow(unreachable_patterns)]
            _ => Err(EngineError::Unsupported(
                "Selected search provider feature not enabled",
            )),
        }
    }

    /// Search the web and extract data from top results.
    ///
    /// Combines search with page fetching and LLM extraction:
    /// 1. Searches using the configured provider
    /// 2. Fetches HTML from each result URL
    /// 3. Uses LLM to extract data according to the extraction prompt
    ///
    /// # Arguments
    /// * `query` - The search query
    /// * `extraction_prompt` - What data to extract from each page
    /// * `options` - Optional search options (limit, country, etc.)
    /// * `client` - Optional HTTP client to reuse
    ///
    /// # Example
    /// ```ignore
    /// let data = engine.search_and_extract(
    ///     "best rust web frameworks 2024",
    ///     "Extract framework name, description, and GitHub stars",
    ///     Some(SearchOptions::new().with_limit(5)),
    ///     None,
    /// ).await?;
    ///
    /// for (url, extracted) in data {
    ///     println!("{}: {:?}", url, extracted);
    /// }
    /// ```
    #[cfg(feature = "search")]
    pub async fn search_and_extract(
        &self,
        query: &str,
        extraction_prompt: &str,
        options: Option<crate::features::search::SearchOptions>,
        client: Option<&reqwest::Client>,
    ) -> EngineResult<Vec<(String, serde_json::Value)>> {
        // Search first
        let search_results = self.search(query, options, client).await?;

        if search_results.is_empty() {
            return Ok(Vec::new());
        }

        // Create client if not provided
        let owned_client;
        let http_client = match client {
            Some(c) => c,
            None => {
                owned_client = reqwest::ClientBuilder::new()
                    .timeout(std::time::Duration::from_secs(30))
                    .build()
                    .map_err(|e| EngineError::Remote(e.to_string()))?;
                &owned_client
            }
        };

        // Configure extraction
        let mut extraction_engine = self.clone();
        extraction_engine.cfg.extraction_prompt = Some(extraction_prompt.to_string());
        extraction_engine.cfg.extra_ai_data = true;

        let mut results = Vec::new();

        for result in search_results.results {
            // Fetch page HTML
            let response = http_client
                .get(&result.url)
                .send()
                .await
                .map_err(|e| EngineError::Remote(format!("Failed to fetch {}: {}", result.url, e)))?;

            if !response.status().is_success() {
                log::debug!("Skipping {} - HTTP {}", result.url, response.status());
                continue;
            }

            let html = response
                .text()
                .await
                .map_err(|e| EngineError::Remote(format!("Failed to read {}: {}", result.url, e)))?;

            // Extract using LLM
            match extraction_engine
                .extract_from_html(&html, &result.url, Some(&result.title))
                .await
            {
                Ok(extraction_result) => {
                    if let Some(extracted) = extraction_result.extracted {
                        results.push((result.url.clone(), extracted));
                    }
                }
                Err(e) => {
                    log::debug!("Extraction failed for {}: {}", result.url, e);
                }
            }
        }

        Ok(results)
    }

    /// Research a topic using search, extraction, and synthesis.
    ///
    /// Higher-level research function that:
    /// 1. Searches for the query
    /// 2. Fetches and extracts data from each result page
    /// 3. Synthesizes findings into a coherent summary
    ///
    /// # Arguments
    /// * `topic` - The research topic or question
    /// * `options` - Research options (max pages, synthesis, etc.)
    /// * `client` - Optional HTTP client to reuse
    ///
    /// # Example
    /// ```ignore
    /// let research = engine.research(
    ///     "How do Tokio and async-std compare?",
    ///     ResearchOptions {
    ///         max_pages: 5,
    ///         synthesize: true,
    ///         ..Default::default()
    ///     },
    ///     None,
    /// ).await?;
    ///
    /// println!("Summary: {}", research.summary.unwrap());
    /// ```
    #[cfg(feature = "search")]
    pub async fn research(
        &self,
        topic: &str,
        options: ResearchOptions,
        client: Option<&reqwest::Client>,
    ) -> EngineResult<ResearchResult> {
        use crate::features::search::SearchOptions;

        // Build search options
        let search_opts = options.search_options.clone().unwrap_or_else(|| {
            SearchOptions::new().with_limit(options.max_pages.max(5))
        });

        // Search first
        let search_results = self.search(topic, Some(search_opts), client).await?;

        // Create client if not provided
        let owned_client;
        let http_client = match client {
            Some(c) => c,
            None => {
                owned_client = reqwest::ClientBuilder::new()
                    .timeout(std::time::Duration::from_secs(30))
                    .build()
                    .map_err(|e| EngineError::Remote(e.to_string()))?;
                &owned_client
            }
        };

        // Configure extraction
        let extraction_prompt = options.extraction_prompt.clone().unwrap_or_else(|| {
            format!(
                "Extract key information relevant to: {}. Include facts, data points, and insights.",
                topic
            )
        });

        let mut extraction_engine = self.clone();
        extraction_engine.cfg.extraction_prompt = Some(extraction_prompt);
        extraction_engine.cfg.extra_ai_data = true;

        let mut extractions = Vec::new();
        let mut total_usage = AutomationUsage::default();

        // Limit pages to process
        let max_pages = options.max_pages.min(search_results.results.len());

        for result in search_results.results.iter().take(max_pages) {
            // Fetch page HTML
            let response = match http_client.get(&result.url).send().await {
                Ok(r) => r,
                Err(e) => {
                    log::debug!("Failed to fetch {}: {}", result.url, e);
                    continue;
                }
            };

            if !response.status().is_success() {
                log::debug!("Skipping {} - HTTP {}", result.url, response.status());
                continue;
            }

            let html = match response.text().await {
                Ok(h) => h,
                Err(e) => {
                    log::debug!("Failed to read {}: {}", result.url, e);
                    continue;
                }
            };

            // Extract using LLM
            match extraction_engine
                .extract_from_html(&html, &result.url, Some(&result.title))
                .await
            {
                Ok(extraction_result) => {
                    total_usage.accumulate(&extraction_result.usage);
                    if let Some(extracted) = extraction_result.extracted {
                        extractions.push(PageExtraction {
                            url: result.url.clone(),
                            title: result.title.clone(),
                            extracted,
                        });
                    }
                }
                Err(e) => {
                    log::debug!("Extraction failed for {}: {}", result.url, e);
                }
            }
        }

        // Synthesize if requested
        let summary = if options.synthesize && !extractions.is_empty() {
            Some(self.synthesize_research(topic, &extractions, &mut total_usage).await?)
        } else {
            None
        };

        Ok(ResearchResult {
            topic: topic.to_string(),
            search_results,
            extractions,
            summary,
            usage: total_usage,
        })
    }

    /// Synthesize research findings into a coherent summary.
    #[cfg(feature = "search")]
    async fn synthesize_research(
        &self,
        topic: &str,
        extractions: &[PageExtraction],
        usage: &mut AutomationUsage,
    ) -> EngineResult<String> {
        // Build context from extractions
        let mut context = String::new();
        for (i, extraction) in extractions.iter().enumerate() {
            context.push_str(&format!(
                "\n--- Source {}: {} ({})\n{}\n",
                i + 1,
                extraction.title,
                extraction.url,
                serde_json::to_string_pretty(&extraction.extracted).unwrap_or_default()
            ));
        }

        let synthesis_prompt = format!(
            "Based on the following research findings, provide a comprehensive summary answering: {}\n\n{}",
            topic, context
        );

        // Use the LLM to synthesize
        let mut synthesis_engine = self.clone();
        synthesis_engine.cfg.extraction_prompt = None;
        synthesis_engine.system_prompt = Some(
            "You are a research assistant. Synthesize the provided findings into a clear, comprehensive summary. \
             Focus on key insights, patterns, and conclusions. Be concise but thorough.".to_string()
        );

        let result = synthesis_engine
            .extract_from_html(&synthesis_prompt, "research://synthesis", None)
            .await?;

        usage.accumulate(&result.usage);

        // Extract the summary text from the result
        let summary = if let Some(ref extracted) = result.extracted {
            // Try to get summary or content field
            if let Some(s) = extracted.get("summary").or(extracted.get("content")) {
                s.as_str().map(String::from).unwrap_or_else(|| {
                    serde_json::to_string(s).unwrap_or_default()
                })
            } else if let Some(s) = extracted.as_str() {
                s.to_string()
            } else {
                serde_json::to_string(extracted).unwrap_or_default()
            }
        } else {
            String::new()
        };

        Ok(summary)
    }
}

/// Options for research tasks.
#[cfg(feature = "search")]
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ResearchOptions {
    /// Maximum pages to visit (default: 5).
    pub max_pages: usize,
    /// Search options for the initial query.
    pub search_options: Option<crate::features::search::SearchOptions>,
    /// Custom extraction prompt (default: auto-generated from topic).
    pub extraction_prompt: Option<String>,
    /// Whether to synthesize findings into a summary (default: false).
    pub synthesize: bool,
}

#[cfg(feature = "search")]
impl ResearchOptions {
    /// Create new research options with defaults.
    pub fn new() -> Self {
        Self {
            max_pages: 5,
            search_options: None,
            extraction_prompt: None,
            synthesize: false,
        }
    }

    /// Set maximum pages to visit.
    pub fn with_max_pages(mut self, max: usize) -> Self {
        self.max_pages = max;
        self
    }

    /// Set search options.
    pub fn with_search_options(mut self, opts: crate::features::search::SearchOptions) -> Self {
        self.search_options = Some(opts);
        self
    }

    /// Set custom extraction prompt.
    pub fn with_extraction_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.extraction_prompt = Some(prompt.into());
        self
    }

    /// Enable synthesis of findings into a summary.
    pub fn with_synthesis(mut self) -> Self {
        self.synthesize = true;
        self
    }
}

/// Result of a research task.
#[cfg(feature = "search")]
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ResearchResult {
    /// The original research topic/question.
    pub topic: String,
    /// Search results used.
    pub search_results: crate::features::search::SearchResults,
    /// Extracted data from each visited page.
    pub extractions: Vec<PageExtraction>,
    /// Synthesized summary (if enabled).
    pub summary: Option<String>,
    /// Token usage.
    pub usage: AutomationUsage,
}

/// Extraction from a single page during research.
#[cfg(feature = "search")]
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PageExtraction {
    /// URL of the page.
    pub url: String,
    /// Title of the page.
    pub title: String,
    /// Extracted data.
    pub extracted: serde_json::Value,
}

/// Extract structured data from HTML content using an LLM.
///
/// This is a standalone convenience function that doesn't require setting up
/// a full `RemoteMultimodalEngine`. It's ideal for simple extraction tasks.
///
/// # Arguments
/// * `api_url` - OpenAI-compatible chat completions endpoint
/// * `model` - Model identifier (e.g., "gpt-4o", "claude-3-sonnet")
/// * `api_key` - Optional API key for authenticated endpoints
/// * `html` - The HTML content to extract from
/// * `url` - The URL of the page (for context)
/// * `prompt` - What data to extract
/// * `schema` - Optional JSON schema for the output
///
/// # Example
/// ```ignore
/// let products = extract(
///     "https://api.openai.com/v1/chat/completions",
///     "gpt-4o",
///     Some("sk-..."),
///     &html,
///     "https://example.com/products",
///     "Extract all product names and prices",
///     Some(r#"{"type": "array", "items": {"properties": {"name": {"type": "string"}, "price": {"type": "number"}}}}"#),
/// ).await?;
/// ```
#[cfg(feature = "serde")]
pub async fn extract(
    api_url: &str,
    model: &str,
    api_key: Option<&str>,
    html: &str,
    url: &str,
    prompt: &str,
    schema: Option<&str>,
) -> EngineResult<serde_json::Value> {
    let mut engine = RemoteMultimodalEngine::new(api_url, model, Some(EXTRACT_SYSTEM_PROMPT.to_string()));
    if let Some(key) = api_key {
        engine = engine.with_api_key(Some(key));
    }

    // Configure for extraction
    engine.cfg.extra_ai_data = true;
    engine.cfg.extraction_prompt = Some(prompt.to_string());
    if let Some(s) = schema {
        engine.cfg.extraction_schema = Some(ExtractionSchema::new("extraction", s));
    }

    let result = engine.extract_from_html(html, url, None).await?;
    Ok(result.extracted.unwrap_or(serde_json::Value::Null))
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

    // ==========================================================================
    // Phase 2 Tests: Selector Cache, Structured Outputs, Map API
    // ==========================================================================

    /// Test SelectorCache basic operations.
    #[cfg(feature = "serde")]
    #[test]
    fn test_selector_cache_basic() {
        let mut cache = SelectorCache::new();

        // Initially empty
        assert!(cache.get("login button", None).is_none());
        let (hits, misses, _) = cache.stats();
        assert_eq!(hits, 0);
        assert_eq!(misses, 1);

        // Record a successful selector
        cache.record_success("login button", "#login-btn", None);

        // Now it should be found
        let selector = cache.get("login button", None);
        assert!(selector.is_some());
        assert_eq!(selector.unwrap(), "#login-btn");

        let (hits, misses, entries) = cache.stats();
        assert_eq!(hits, 1);
        assert_eq!(misses, 1);
        assert_eq!(entries, 1);
    }

    /// Test SelectorCache normalization (case-insensitive, trimmed).
    #[cfg(feature = "serde")]
    #[test]
    fn test_selector_cache_normalization() {
        let mut cache = SelectorCache::new();

        cache.record_success("Login Button", "#login-btn", None);

        // Should match regardless of case/whitespace
        assert!(cache.get("login button", None).is_some());
        assert!(cache.get("  LOGIN BUTTON  ", None).is_some());
        assert!(cache.get("LOGIN button", None).is_some());
    }

    /// Test SelectorCache invalidation on failure.
    #[cfg(feature = "serde")]
    #[test]
    fn test_selector_cache_invalidation() {
        let mut cache = SelectorCache::new();

        cache.record_success("submit", "#submit-btn", None);
        assert!(cache.get("submit", None).is_some());

        // First failure doesn't remove
        cache.record_failure("submit");
        assert!(cache.get("submit", None).is_some());

        // Second failure removes the entry
        cache.record_failure("submit");
        // Need to check without incrementing miss counter
        let (_, _, entries) = cache.stats();
        assert_eq!(entries, 0);
    }

    /// Test SelectorCache domain scoping.
    #[cfg(feature = "serde")]
    #[test]
    fn test_selector_cache_domain_scoping() {
        let mut cache = SelectorCache::new();

        cache.record_success("login", "#login-a", Some("example.com"));

        // Should find with same domain
        assert!(cache.get("login", Some("example.com")).is_some());

        // Should NOT find with different domain
        assert!(cache.get("login", Some("other.com")).is_none());

        // Should find with no domain filter
        assert!(cache.get("login", None).is_some());
    }

    /// Test SelectorCache LRU eviction.
    #[cfg(feature = "serde")]
    #[test]
    fn test_selector_cache_lru_eviction() {
        let mut cache = SelectorCache::with_capacity(3);

        cache.record_success("btn1", "#b1", None);
        cache.record_success("btn2", "#b2", None);
        cache.record_success("btn3", "#b3", None);

        let (_, _, entries) = cache.stats();
        assert_eq!(entries, 3);

        // Adding a 4th should evict the LRU (btn1)
        cache.record_success("btn4", "#b4", None);

        let (_, _, entries) = cache.stats();
        assert_eq!(entries, 3);

        // btn1 should be evicted
        // (We can't easily test this without accessing internals,
        // but we know capacity is maintained)
    }

    /// Test StructuredOutputConfig creation.
    #[cfg(feature = "serde")]
    #[test]
    fn test_structured_output_config() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" }
            }
        });

        let config = StructuredOutputConfig::new(schema.clone());
        assert!(config.enabled);
        assert!(!config.strict);
        assert_eq!(config.schema_name, "response");

        let strict_config = StructuredOutputConfig::strict(schema.clone());
        assert!(strict_config.enabled);
        assert!(strict_config.strict);

        let named_config = StructuredOutputConfig::new(schema).with_name("product");
        assert_eq!(named_config.schema_name, "product");
    }

    /// Test MapResult serialization.
    #[cfg(feature = "serde")]
    #[test]
    fn test_map_result_serde() {
        let result = MapResult {
            urls: vec![
                DiscoveredUrl {
                    url: "https://example.com/products".to_string(),
                    text: "Products".to_string(),
                    description: "Product listing page".to_string(),
                    relevance: 0.95,
                    recommended: true,
                    category: "content".to_string(),
                },
            ],
            relevance: 0.8,
            summary: "Homepage with navigation".to_string(),
            suggested_next: vec!["https://example.com/products".to_string()],
            screenshot: None,
            usage: AutomationUsage::default(),
        };

        // Test serialization
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("Products"));
        assert!(json.contains("0.95"));

        // Test deserialization
        let parsed: MapResult = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.urls.len(), 1);
        assert_eq!(parsed.urls[0].url, "https://example.com/products");
        assert!(parsed.urls[0].recommended);
    }

    /// Test DiscoveredUrl defaults.
    #[cfg(feature = "serde")]
    #[test]
    fn test_discovered_url_defaults() {
        let url = DiscoveredUrl::default();
        assert!(url.url.is_empty());
        assert!(!url.recommended);
        assert_eq!(url.relevance, 0.0);
    }

    /// Test AutomationMemory basic operations.
    #[cfg(feature = "serde")]
    #[test]
    fn test_automation_memory_basic() {
        let mut memory = AutomationMemory::new();

        // Store some values
        memory.set("user", serde_json::json!({"name": "test"}));
        memory.set("count", serde_json::json!(42));

        // Retrieve
        assert_eq!(memory.get("count"), Some(&serde_json::json!(42)));
        assert!(memory.get("nonexistent").is_none());

        // Remove
        memory.remove("count");
        assert!(memory.get("count").is_none());
    }

    /// Test AutomationMemory context string generation.
    #[cfg(feature = "serde")]
    #[test]
    fn test_automation_memory_context() {
        let mut memory = AutomationMemory::new();
        memory.set("key1", serde_json::json!("value1"));
        memory.add_visited_url("https://example.com");
        memory.add_action("Clicked button");

        let context = memory.to_context_string();
        assert!(context.contains("key1"));
        assert!(context.contains("example.com"));
        assert!(context.contains("Clicked button"));
    }

    // ==========================================================================
    // Phase 3 Tests: Agent Executor, Action Chaining, Recovery Strategies
    // ==========================================================================

    /// Test AgentConfig creation and builder pattern.
    #[cfg(feature = "serde")]
    #[test]
    fn test_agent_config_builder() {
        let config = AgentConfig::new("Buy the cheapest item")
            .with_max_steps(50)
            .with_timeout(60_000)
            .with_recovery(RecoveryStrategy::Alternative)
            .with_retries(5)
            .with_cache(false)
            .with_success_url("/checkout")
            .with_success_pattern("Order confirmed")
            .with_extraction("Extract order details");

        assert_eq!(config.goal, "Buy the cheapest item");
        assert_eq!(config.max_steps, 50);
        assert_eq!(config.timeout_ms, 60_000);
        assert_eq!(config.recovery_strategy, RecoveryStrategy::Alternative);
        assert_eq!(config.max_retries, 5);
        assert!(!config.use_cache);
        assert!(config.success_urls.contains(&"/checkout".to_string()));
        assert!(config.success_patterns.contains(&"Order confirmed".to_string()));
        assert!(config.extract_on_success);
        assert_eq!(config.extraction_prompt, Some("Extract order details".to_string()));
    }

    /// Test AgentConfig defaults.
    #[cfg(feature = "serde")]
    #[test]
    fn test_agent_config_defaults() {
        let config = AgentConfig::new("Test goal");

        assert_eq!(config.max_steps, 20);
        assert_eq!(config.timeout_ms, 120_000);
        assert_eq!(config.recovery_strategy, RecoveryStrategy::Retry);
        assert_eq!(config.max_retries, 3);
        assert!(config.use_cache);
        assert!(config.capture_screenshots);
        assert!(!config.extract_on_success);
    }

    /// Test RecoveryStrategy variants.
    #[cfg(feature = "serde")]
    #[test]
    fn test_recovery_strategy() {
        assert_eq!(RecoveryStrategy::default(), RecoveryStrategy::Retry);

        // Test serialization
        let json = serde_json::to_string(&RecoveryStrategy::Alternative).unwrap();
        assert!(json.contains("Alternative"));

        let parsed: RecoveryStrategy = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, RecoveryStrategy::Alternative);
    }

    /// Test ChainStep creation and builder pattern.
    #[cfg(feature = "serde")]
    #[test]
    fn test_chain_step_builder() {
        let step = ChainStep::new("click the submit button")
            .when(ChainCondition::UrlContains("/form".to_string()))
            .allow_failure()
            .then_extract("Extract form response");

        assert_eq!(step.instruction, "click the submit button");
        assert!(step.condition.is_some());
        assert!(step.continue_on_failure);
        assert_eq!(step.extract, Some("Extract form response".to_string()));
    }

    /// Test ChainCondition variants.
    #[cfg(feature = "serde")]
    #[test]
    fn test_chain_condition() {
        assert!(matches!(ChainCondition::default(), ChainCondition::Always));

        // Test various conditions
        let url_contains = ChainCondition::UrlContains("/login".to_string());
        let json = serde_json::to_string(&url_contains).unwrap();
        assert!(json.contains("/login"));

        let element_exists = ChainCondition::ElementExists("#submit-btn".to_string());
        let json2 = serde_json::to_string(&element_exists).unwrap();
        assert!(json2.contains("#submit-btn"));
    }

    /// Test AgentResult serialization.
    #[cfg(feature = "serde")]
    #[test]
    fn test_agent_result_serde() {
        let result = AgentResult {
            success: true,
            goal: "Complete checkout".to_string(),
            steps_taken: 5,
            duration_ms: 10_000,
            final_url: "https://example.com/success".to_string(),
            action_history: vec![
                AgentActionRecord {
                    step: 1,
                    action: "click add to cart".to_string(),
                    success: true,
                    duration_ms: 500,
                    url_before: "https://example.com/product".to_string(),
                    url_after: None,
                    error: None,
                    retries: 0,
                },
            ],
            extracted: Some(serde_json::json!({"order_id": "12345"})),
            final_screenshot: None,
            error: None,
            total_usage: AutomationUsage::default(),
            events: vec![],
        };

        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("Complete checkout"));
        assert!(json.contains("12345"));

        let parsed: AgentResult = serde_json::from_str(&json).unwrap();
        assert!(parsed.success);
        assert_eq!(parsed.steps_taken, 5);
        assert_eq!(parsed.action_history.len(), 1);
    }

    /// Test ChainResult serialization.
    #[cfg(feature = "serde")]
    #[test]
    fn test_chain_result_serde() {
        let result = ChainResult {
            success: true,
            steps_executed: 3,
            steps_succeeded: 3,
            steps_failed: 0,
            steps_skipped: 1,
            step_results: vec![
                ChainStepResult {
                    index: 0,
                    instruction: "click button".to_string(),
                    executed: true,
                    success: true,
                    action_taken: Some("clicked".to_string()),
                    error: None,
                    duration_ms: 200,
                    extracted: None,
                },
            ],
            extractions: vec![serde_json::json!({"data": "test"})],
            duration_ms: 5000,
            total_usage: AutomationUsage::default(),
        };

        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("click button"));

        let parsed: ChainResult = serde_json::from_str(&json).unwrap();
        assert!(parsed.success);
        assert_eq!(parsed.steps_executed, 3);
        assert_eq!(parsed.extractions.len(), 1);
    }

    /// Test AgentEvent serialization.
    #[cfg(feature = "serde")]
    #[test]
    fn test_agent_event_serde() {
        let events = vec![
            AgentEvent::Started {
                goal: "Test".to_string(),
                timestamp_ms: 1000,
            },
            AgentEvent::Planning {
                step: 1,
                current_url: "https://example.com".to_string(),
            },
            AgentEvent::ActionSuccess {
                step: 1,
                action: "clicked".to_string(),
                duration_ms: 100,
            },
            AgentEvent::Completed {
                steps_taken: 1,
                duration_ms: 500,
                success: true,
            },
        ];

        for event in &events {
            let json = serde_json::to_string(event).unwrap();
            let _parsed: AgentEvent = serde_json::from_str(&json).unwrap();
        }
    }

    /// Test AgentActionRecord defaults.
    #[cfg(feature = "serde")]
    #[test]
    fn test_agent_action_record_defaults() {
        let record = AgentActionRecord::default();
        assert_eq!(record.step, 0);
        assert!(record.action.is_empty());
        assert!(!record.success);
        assert_eq!(record.retries, 0);
    }
}
