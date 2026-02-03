//! Configuration types for spider_agent.

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Usage limits for controlling agent resource consumption.
#[derive(Debug, Clone, Default)]
pub struct UsageLimits {
    /// Maximum total tokens (prompt + completion).
    pub max_total_tokens: Option<u64>,
    /// Maximum prompt tokens.
    pub max_prompt_tokens: Option<u64>,
    /// Maximum completion tokens.
    pub max_completion_tokens: Option<u64>,
    /// Maximum LLM API calls.
    pub max_llm_calls: Option<u64>,
    /// Maximum search API calls.
    pub max_search_calls: Option<u64>,
    /// Maximum HTTP fetch calls.
    pub max_fetch_calls: Option<u64>,
    /// Maximum web browser calls (Chrome/WebDriver combined).
    pub max_webbrowser_calls: Option<u64>,
    /// Maximum custom tool calls.
    pub max_custom_tool_calls: Option<u64>,
    /// Maximum generic tool calls.
    pub max_tool_calls: Option<u64>,
}

impl UsageLimits {
    /// Create new usage limits with no restrictions.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set maximum total tokens.
    pub fn with_max_total_tokens(mut self, limit: u64) -> Self {
        self.max_total_tokens = Some(limit);
        self
    }

    /// Set maximum prompt tokens.
    pub fn with_max_prompt_tokens(mut self, limit: u64) -> Self {
        self.max_prompt_tokens = Some(limit);
        self
    }

    /// Set maximum completion tokens.
    pub fn with_max_completion_tokens(mut self, limit: u64) -> Self {
        self.max_completion_tokens = Some(limit);
        self
    }

    /// Set maximum LLM calls.
    pub fn with_max_llm_calls(mut self, limit: u64) -> Self {
        self.max_llm_calls = Some(limit);
        self
    }

    /// Set maximum search calls.
    pub fn with_max_search_calls(mut self, limit: u64) -> Self {
        self.max_search_calls = Some(limit);
        self
    }

    /// Set maximum fetch calls.
    pub fn with_max_fetch_calls(mut self, limit: u64) -> Self {
        self.max_fetch_calls = Some(limit);
        self
    }

    /// Set maximum web browser calls.
    pub fn with_max_webbrowser_calls(mut self, limit: u64) -> Self {
        self.max_webbrowser_calls = Some(limit);
        self
    }

    /// Set maximum custom tool calls.
    pub fn with_max_custom_tool_calls(mut self, limit: u64) -> Self {
        self.max_custom_tool_calls = Some(limit);
        self
    }

    /// Set maximum tool calls.
    pub fn with_max_tool_calls(mut self, limit: u64) -> Self {
        self.max_tool_calls = Some(limit);
        self
    }
}

/// Type of limit that was exceeded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LimitType {
    /// Total tokens limit exceeded.
    TotalTokens {
        /// Tokens used so far.
        used: u64,
        /// The limit that was set.
        limit: u64,
    },
    /// Prompt tokens limit exceeded.
    PromptTokens {
        /// Tokens used so far.
        used: u64,
        /// The limit that was set.
        limit: u64,
    },
    /// Completion tokens limit exceeded.
    CompletionTokens {
        /// Tokens used so far.
        used: u64,
        /// The limit that was set.
        limit: u64,
    },
    /// LLM calls limit exceeded.
    LlmCalls {
        /// Calls made so far.
        used: u64,
        /// The limit that was set.
        limit: u64,
    },
    /// Search calls limit exceeded.
    SearchCalls {
        /// Calls made so far.
        used: u64,
        /// The limit that was set.
        limit: u64,
    },
    /// Fetch calls limit exceeded.
    FetchCalls {
        /// Calls made so far.
        used: u64,
        /// The limit that was set.
        limit: u64,
    },
    /// Web browser calls limit exceeded.
    WebbrowserCalls {
        /// Calls made so far.
        used: u64,
        /// The limit that was set.
        limit: u64,
    },
    /// Custom tool calls limit exceeded.
    CustomToolCalls {
        /// Calls made so far.
        used: u64,
        /// The limit that was set.
        limit: u64,
    },
    /// Tool calls limit exceeded.
    ToolCalls {
        /// Calls made so far.
        used: u64,
        /// The limit that was set.
        limit: u64,
    },
}

impl std::fmt::Display for LimitType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TotalTokens { used, limit } => {
                write!(f, "total tokens ({} used, {} limit)", used, limit)
            }
            Self::PromptTokens { used, limit } => {
                write!(f, "prompt tokens ({} used, {} limit)", used, limit)
            }
            Self::CompletionTokens { used, limit } => {
                write!(f, "completion tokens ({} used, {} limit)", used, limit)
            }
            Self::LlmCalls { used, limit } => {
                write!(f, "LLM calls ({} used, {} limit)", used, limit)
            }
            Self::SearchCalls { used, limit } => {
                write!(f, "search calls ({} used, {} limit)", used, limit)
            }
            Self::FetchCalls { used, limit } => {
                write!(f, "fetch calls ({} used, {} limit)", used, limit)
            }
            Self::WebbrowserCalls { used, limit } => {
                write!(f, "web browser calls ({} used, {} limit)", used, limit)
            }
            Self::CustomToolCalls { used, limit } => {
                write!(f, "custom tool calls ({} used, {} limit)", used, limit)
            }
            Self::ToolCalls { used, limit } => {
                write!(f, "tool calls ({} used, {} limit)", used, limit)
            }
        }
    }
}

/// Agent configuration.
#[derive(Debug, Clone)]
pub struct AgentConfig {
    /// System prompt for LLM.
    pub system_prompt: Option<String>,

    /// Max concurrent LLM calls.
    pub max_concurrent_llm_calls: usize,

    /// LLM temperature (0.0 - 1.0).
    pub temperature: f32,

    /// Max tokens for LLM response.
    pub max_tokens: u16,

    /// Request timeout.
    pub timeout: Duration,

    /// Retry configuration.
    pub retry: RetryConfig,

    /// Max HTML bytes to send to LLM.
    pub html_max_bytes: usize,

    /// HTML cleaning mode.
    pub html_cleaning: HtmlCleaningMode,

    /// Whether to request JSON output from LLM.
    pub json_mode: bool,

    /// Usage limits for resource control.
    pub limits: UsageLimits,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            system_prompt: None,
            max_concurrent_llm_calls: 5,
            temperature: 0.1,
            max_tokens: 4096,
            timeout: Duration::from_secs(60),
            retry: RetryConfig::default(),
            html_max_bytes: 24_000,
            html_cleaning: HtmlCleaningMode::Default,
            json_mode: true,
            limits: UsageLimits::default(),
        }
    }
}

impl AgentConfig {
    /// Create a new config with defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set system prompt.
    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(prompt.into());
        self
    }

    /// Set max concurrent LLM calls.
    pub fn with_max_concurrent_llm_calls(mut self, n: usize) -> Self {
        self.max_concurrent_llm_calls = n;
        self
    }

    /// Set LLM temperature.
    pub fn with_temperature(mut self, temp: f32) -> Self {
        self.temperature = temp.clamp(0.0, 2.0);
        self
    }

    /// Set max tokens.
    pub fn with_max_tokens(mut self, tokens: u16) -> Self {
        self.max_tokens = tokens;
        self
    }

    /// Set request timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Set retry config.
    pub fn with_retry(mut self, retry: RetryConfig) -> Self {
        self.retry = retry;
        self
    }

    /// Set HTML max bytes.
    pub fn with_html_max_bytes(mut self, bytes: usize) -> Self {
        self.html_max_bytes = bytes;
        self
    }

    /// Set HTML cleaning mode.
    pub fn with_html_cleaning(mut self, mode: HtmlCleaningMode) -> Self {
        self.html_cleaning = mode;
        self
    }

    /// Enable or disable JSON mode.
    pub fn with_json_mode(mut self, enabled: bool) -> Self {
        self.json_mode = enabled;
        self
    }

    /// Set usage limits.
    pub fn with_limits(mut self, limits: UsageLimits) -> Self {
        self.limits = limits;
        self
    }
}

/// Retry configuration.
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Max retry attempts.
    pub max_attempts: usize,
    /// Backoff delay between attempts.
    pub backoff: Duration,
    /// Retry on parse errors.
    pub retry_on_parse_error: bool,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            backoff: Duration::from_millis(500),
            retry_on_parse_error: true,
        }
    }
}

impl RetryConfig {
    /// Create a new retry config.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set max attempts.
    pub fn with_max_attempts(mut self, n: usize) -> Self {
        self.max_attempts = n;
        self
    }

    /// Set backoff delay.
    pub fn with_backoff(mut self, backoff: Duration) -> Self {
        self.backoff = backoff;
        self
    }
}

/// HTML cleaning mode.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum HtmlCleaningMode {
    /// Standard cleaning - removes scripts, styles, comments.
    #[default]
    Default,
    /// Aggressive cleaning - also removes SVGs, images, etc.
    Aggressive,
    /// Minimal cleaning - only removes scripts.
    Minimal,
    /// No cleaning - raw HTML.
    Raw,
}

/// Search options for web search.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SearchOptions {
    /// Maximum number of results to return.
    pub limit: Option<usize>,
    /// Country/region code (e.g., "us", "uk").
    pub country: Option<String>,
    /// Language code (e.g., "en", "es").
    pub language: Option<String>,
    /// Filter to specific domains.
    pub site_filter: Option<Vec<String>>,
    /// Exclude specific domains.
    pub exclude_domains: Option<Vec<String>>,
    /// Time range filter.
    pub time_range: Option<TimeRange>,
}

impl SearchOptions {
    /// Create new search options with defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set maximum number of results.
    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Set country/region code.
    pub fn with_country(mut self, country: impl Into<String>) -> Self {
        self.country = Some(country.into());
        self
    }

    /// Set language code.
    pub fn with_language(mut self, language: impl Into<String>) -> Self {
        self.language = Some(language.into());
        self
    }

    /// Filter results to specific domains.
    pub fn with_site_filter(mut self, domains: Vec<String>) -> Self {
        self.site_filter = Some(domains);
        self
    }

    /// Exclude specific domains from results.
    pub fn with_exclude_domains(mut self, domains: Vec<String>) -> Self {
        self.exclude_domains = Some(domains);
        self
    }

    /// Set time range filter.
    pub fn with_time_range(mut self, range: TimeRange) -> Self {
        self.time_range = Some(range);
        self
    }
}

/// Time range for filtering search results.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TimeRange {
    /// Results from the past day.
    Day,
    /// Results from the past week.
    Week,
    /// Results from the past month.
    Month,
    /// Results from the past year.
    Year,
    /// Custom date range.
    Custom {
        /// Start date (format depends on provider).
        start: String,
        /// End date (format depends on provider).
        end: String,
    },
}

/// Research options for research tasks.
#[derive(Debug, Clone, Default)]
pub struct ResearchOptions {
    /// Maximum pages to visit.
    pub max_pages: usize,
    /// Search options for the query.
    pub search_options: Option<SearchOptions>,
    /// Custom extraction prompt.
    pub extraction_prompt: Option<String>,
    /// Whether to synthesize findings into a summary.
    pub synthesize: bool,
}

impl ResearchOptions {
    /// Create new research options with defaults.
    pub fn new() -> Self {
        Self {
            max_pages: 5,
            search_options: None,
            extraction_prompt: None,
            synthesize: true,
        }
    }

    /// Set max pages to visit.
    pub fn with_max_pages(mut self, n: usize) -> Self {
        self.max_pages = n;
        self
    }

    /// Set search options.
    pub fn with_search_options(mut self, options: SearchOptions) -> Self {
        self.search_options = Some(options);
        self
    }

    /// Set extraction prompt.
    pub fn with_extraction_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.extraction_prompt = Some(prompt.into());
        self
    }

    /// Enable or disable synthesis.
    pub fn with_synthesize(mut self, enabled: bool) -> Self {
        self.synthesize = enabled;
        self
    }
}

/// Usage statistics for tracking agent operations.
///
/// Uses atomic counters for lock-free concurrent updates.
#[derive(Debug)]
pub struct UsageStats {
    /// Total LLM prompt tokens used.
    pub prompt_tokens: AtomicU64,
    /// Total LLM completion tokens used.
    pub completion_tokens: AtomicU64,
    /// Total LLM calls made.
    pub llm_calls: AtomicU64,
    /// Total search calls made.
    pub search_calls: AtomicU64,
    /// Total HTTP fetch calls made.
    pub fetch_calls: AtomicU64,
    /// Total web browser calls made (Chrome/WebDriver combined).
    pub webbrowser_calls: AtomicU64,
    /// Custom tool calls tracked by tool name (lock-free via DashMap).
    pub custom_tool_calls: DashMap<String, AtomicU64>,
    /// Total tool calls made.
    pub tool_calls: AtomicU64,
}

impl Default for UsageStats {
    fn default() -> Self {
        Self {
            prompt_tokens: AtomicU64::new(0),
            completion_tokens: AtomicU64::new(0),
            llm_calls: AtomicU64::new(0),
            search_calls: AtomicU64::new(0),
            fetch_calls: AtomicU64::new(0),
            webbrowser_calls: AtomicU64::new(0),
            custom_tool_calls: DashMap::new(),
            tool_calls: AtomicU64::new(0),
        }
    }
}

impl UsageStats {
    /// Create new usage stats.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add tokens from an LLM response.
    pub fn add_tokens(&self, prompt: u64, completion: u64) {
        self.prompt_tokens.fetch_add(prompt, Ordering::Relaxed);
        self.completion_tokens.fetch_add(completion, Ordering::Relaxed);
    }

    /// Increment LLM call count.
    pub fn increment_llm_calls(&self) {
        self.llm_calls.fetch_add(1, Ordering::Relaxed);
    }

    /// Increment search call count.
    pub fn increment_search_calls(&self) {
        self.search_calls.fetch_add(1, Ordering::Relaxed);
    }

    /// Increment fetch call count.
    pub fn increment_fetch_calls(&self) {
        self.fetch_calls.fetch_add(1, Ordering::Relaxed);
    }

    /// Increment web browser call count (Chrome/WebDriver).
    pub fn increment_webbrowser_calls(&self) {
        self.webbrowser_calls.fetch_add(1, Ordering::Relaxed);
    }

    /// Increment custom tool call count for a specific tool.
    pub fn increment_custom_tool_calls(&self, tool_name: &str) {
        self.custom_tool_calls
            .entry(tool_name.to_string())
            .or_insert_with(|| AtomicU64::new(0))
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Get custom tool call count for a specific tool.
    pub fn get_custom_tool_calls(&self, tool_name: &str) -> u64 {
        self.custom_tool_calls
            .get(tool_name)
            .map(|v| v.load(Ordering::Relaxed))
            .unwrap_or(0)
    }

    /// Get total custom tool calls across all tools.
    pub fn total_custom_tool_calls(&self) -> u64 {
        self.custom_tool_calls
            .iter()
            .map(|entry| entry.value().load(Ordering::Relaxed))
            .sum()
    }

    /// Increment tool call count.
    pub fn increment_tool_calls(&self) {
        self.tool_calls.fetch_add(1, Ordering::Relaxed);
    }

    /// Get total tokens used.
    pub fn total_tokens(&self) -> u64 {
        self.prompt_tokens.load(Ordering::Relaxed)
            + self.completion_tokens.load(Ordering::Relaxed)
    }

    /// Get a snapshot of all stats.
    pub fn snapshot(&self) -> UsageSnapshot {
        let custom_tool_calls: HashMap<String, u64> = self
            .custom_tool_calls
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().load(Ordering::Relaxed)))
            .collect();

        UsageSnapshot {
            prompt_tokens: self.prompt_tokens.load(Ordering::Relaxed),
            completion_tokens: self.completion_tokens.load(Ordering::Relaxed),
            llm_calls: self.llm_calls.load(Ordering::Relaxed),
            search_calls: self.search_calls.load(Ordering::Relaxed),
            fetch_calls: self.fetch_calls.load(Ordering::Relaxed),
            webbrowser_calls: self.webbrowser_calls.load(Ordering::Relaxed),
            custom_tool_calls,
            tool_calls: self.tool_calls.load(Ordering::Relaxed),
        }
    }

    /// Reset all counters.
    pub fn reset(&self) {
        self.prompt_tokens.store(0, Ordering::Relaxed);
        self.completion_tokens.store(0, Ordering::Relaxed);
        self.llm_calls.store(0, Ordering::Relaxed);
        self.search_calls.store(0, Ordering::Relaxed);
        self.fetch_calls.store(0, Ordering::Relaxed);
        self.webbrowser_calls.store(0, Ordering::Relaxed);
        self.custom_tool_calls.clear();
        self.tool_calls.store(0, Ordering::Relaxed);
    }

    // ==================== Limit Checking Methods ====================

    /// Check if LLM call limit would be exceeded.
    pub fn check_llm_limit(&self, limits: &UsageLimits) -> Option<LimitType> {
        if let Some(limit) = limits.max_llm_calls {
            let used = self.llm_calls.load(Ordering::Relaxed);
            if used >= limit {
                return Some(LimitType::LlmCalls { used, limit });
            }
        }
        None
    }

    /// Check if search call limit would be exceeded.
    pub fn check_search_limit(&self, limits: &UsageLimits) -> Option<LimitType> {
        if let Some(limit) = limits.max_search_calls {
            let used = self.search_calls.load(Ordering::Relaxed);
            if used >= limit {
                return Some(LimitType::SearchCalls { used, limit });
            }
        }
        None
    }

    /// Check if fetch call limit would be exceeded.
    pub fn check_fetch_limit(&self, limits: &UsageLimits) -> Option<LimitType> {
        if let Some(limit) = limits.max_fetch_calls {
            let used = self.fetch_calls.load(Ordering::Relaxed);
            if used >= limit {
                return Some(LimitType::FetchCalls { used, limit });
            }
        }
        None
    }

    /// Check if web browser call limit would be exceeded.
    pub fn check_webbrowser_limit(&self, limits: &UsageLimits) -> Option<LimitType> {
        if let Some(limit) = limits.max_webbrowser_calls {
            let used = self.webbrowser_calls.load(Ordering::Relaxed);
            if used >= limit {
                return Some(LimitType::WebbrowserCalls { used, limit });
            }
        }
        None
    }

    /// Check if custom tool call limit would be exceeded (total across all tools).
    pub fn check_custom_tool_limit(&self, limits: &UsageLimits) -> Option<LimitType> {
        if let Some(limit) = limits.max_custom_tool_calls {
            let used = self.total_custom_tool_calls();
            if used >= limit {
                return Some(LimitType::CustomToolCalls { used, limit });
            }
        }
        None
    }

    /// Check if tool call limit would be exceeded.
    pub fn check_tool_limit(&self, limits: &UsageLimits) -> Option<LimitType> {
        if let Some(limit) = limits.max_tool_calls {
            let used = self.tool_calls.load(Ordering::Relaxed);
            if used >= limit {
                return Some(LimitType::ToolCalls { used, limit });
            }
        }
        None
    }

    /// Check if token limits would be exceeded.
    pub fn check_token_limits(&self, limits: &UsageLimits) -> Option<LimitType> {
        let prompt = self.prompt_tokens.load(Ordering::Relaxed);
        let completion = self.completion_tokens.load(Ordering::Relaxed);
        let total = prompt + completion;

        if let Some(limit) = limits.max_total_tokens {
            if total >= limit {
                return Some(LimitType::TotalTokens { used: total, limit });
            }
        }

        if let Some(limit) = limits.max_prompt_tokens {
            if prompt >= limit {
                return Some(LimitType::PromptTokens { used: prompt, limit });
            }
        }

        if let Some(limit) = limits.max_completion_tokens {
            if completion >= limit {
                return Some(LimitType::CompletionTokens { used: completion, limit });
            }
        }

        None
    }
}

/// Snapshot of usage statistics.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageSnapshot {
    /// Total LLM prompt tokens.
    pub prompt_tokens: u64,
    /// Total LLM completion tokens.
    pub completion_tokens: u64,
    /// Total LLM calls.
    pub llm_calls: u64,
    /// Total search calls.
    pub search_calls: u64,
    /// Total HTTP fetch calls.
    pub fetch_calls: u64,
    /// Total web browser calls (Chrome/WebDriver combined).
    pub webbrowser_calls: u64,
    /// Custom tool calls by tool name.
    pub custom_tool_calls: HashMap<String, u64>,
    /// Total tool calls.
    pub tool_calls: u64,
}

impl UsageSnapshot {
    /// Get total tokens.
    pub fn total_tokens(&self) -> u64 {
        self.prompt_tokens + self.completion_tokens
    }

    /// Get total custom tool calls across all tools.
    pub fn total_custom_tool_calls(&self) -> u64 {
        self.custom_tool_calls.values().sum()
    }

    /// Get call count for a specific custom tool.
    pub fn get_custom_tool_calls(&self, tool_name: &str) -> u64 {
        self.custom_tool_calls.get(tool_name).copied().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_usage_limits_builder() {
        let limits = UsageLimits::new()
            .with_max_total_tokens(10000)
            .with_max_llm_calls(100)
            .with_max_search_calls(50)
            .with_max_fetch_calls(200)
            .with_max_webbrowser_calls(30)
            .with_max_custom_tool_calls(25)
            .with_max_tool_calls(500);

        assert_eq!(limits.max_total_tokens, Some(10000));
        assert_eq!(limits.max_llm_calls, Some(100));
        assert_eq!(limits.max_search_calls, Some(50));
        assert_eq!(limits.max_fetch_calls, Some(200));
        assert_eq!(limits.max_webbrowser_calls, Some(30));
        assert_eq!(limits.max_custom_tool_calls, Some(25));
        assert_eq!(limits.max_tool_calls, Some(500));
    }

    #[test]
    fn test_usage_stats_tracking() {
        let stats = UsageStats::new();

        // Track various calls
        stats.increment_llm_calls();
        stats.increment_llm_calls();
        stats.increment_search_calls();
        stats.increment_fetch_calls();
        stats.increment_fetch_calls();
        stats.increment_fetch_calls();
        stats.increment_webbrowser_calls();
        stats.increment_custom_tool_calls("my_api");
        stats.increment_custom_tool_calls("my_api");
        stats.increment_custom_tool_calls("other_api");
        stats.add_tokens(100, 50);

        let snapshot = stats.snapshot();
        assert_eq!(snapshot.llm_calls, 2);
        assert_eq!(snapshot.search_calls, 1);
        assert_eq!(snapshot.fetch_calls, 3);
        assert_eq!(snapshot.webbrowser_calls, 1);
        assert_eq!(snapshot.prompt_tokens, 100);
        assert_eq!(snapshot.completion_tokens, 50);
        assert_eq!(snapshot.total_tokens(), 150);

        // Check custom tool tracking
        assert_eq!(snapshot.get_custom_tool_calls("my_api"), 2);
        assert_eq!(snapshot.get_custom_tool_calls("other_api"), 1);
        assert_eq!(snapshot.get_custom_tool_calls("unknown"), 0);
        assert_eq!(snapshot.total_custom_tool_calls(), 3);
    }

    #[test]
    fn test_usage_stats_reset() {
        let stats = UsageStats::new();
        stats.increment_llm_calls();
        stats.increment_search_calls();
        stats.increment_custom_tool_calls("my_api");
        stats.add_tokens(100, 50);

        stats.reset();

        let snapshot = stats.snapshot();
        assert_eq!(snapshot.llm_calls, 0);
        assert_eq!(snapshot.search_calls, 0);
        assert_eq!(snapshot.prompt_tokens, 0);
        assert_eq!(snapshot.total_custom_tool_calls(), 0);
    }

    #[test]
    fn test_limit_checking_llm() {
        let stats = UsageStats::new();
        let limits = UsageLimits::new().with_max_llm_calls(3);

        // Under limit
        stats.increment_llm_calls();
        stats.increment_llm_calls();
        assert!(stats.check_llm_limit(&limits).is_none());

        // At limit
        stats.increment_llm_calls();
        let exceeded = stats.check_llm_limit(&limits);
        assert!(exceeded.is_some());
        match exceeded.unwrap() {
            LimitType::LlmCalls { used, limit } => {
                assert_eq!(used, 3);
                assert_eq!(limit, 3);
            }
            _ => panic!("Expected LlmCalls limit type"),
        }
    }

    #[test]
    fn test_limit_checking_tokens() {
        let stats = UsageStats::new();
        let limits = UsageLimits::new()
            .with_max_total_tokens(100)
            .with_max_prompt_tokens(60);

        // Under limit
        stats.add_tokens(30, 20);
        assert!(stats.check_token_limits(&limits).is_none());

        // Prompt limit exceeded
        stats.add_tokens(40, 0);
        let exceeded = stats.check_token_limits(&limits);
        assert!(exceeded.is_some());
        match exceeded.unwrap() {
            LimitType::PromptTokens { used, limit } => {
                assert_eq!(used, 70);
                assert_eq!(limit, 60);
            }
            _ => panic!("Expected PromptTokens limit type"),
        }
    }

    #[test]
    fn test_limit_checking_custom_tools() {
        let stats = UsageStats::new();
        let limits = UsageLimits::new().with_max_custom_tool_calls(5);

        stats.increment_custom_tool_calls("api_a");
        stats.increment_custom_tool_calls("api_b");
        stats.increment_custom_tool_calls("api_a");
        stats.increment_custom_tool_calls("api_c");
        assert!(stats.check_custom_tool_limit(&limits).is_none());

        stats.increment_custom_tool_calls("api_a");
        let exceeded = stats.check_custom_tool_limit(&limits);
        assert!(exceeded.is_some());
        match exceeded.unwrap() {
            LimitType::CustomToolCalls { used, limit } => {
                assert_eq!(used, 5);
                assert_eq!(limit, 5);
            }
            _ => panic!("Expected CustomToolCalls limit type"),
        }
    }

    #[test]
    fn test_agent_config_with_limits() {
        let limits = UsageLimits::new()
            .with_max_llm_calls(100)
            .with_max_search_calls(50);

        let config = AgentConfig::new().with_limits(limits);

        assert_eq!(config.limits.max_llm_calls, Some(100));
        assert_eq!(config.limits.max_search_calls, Some(50));
    }

    #[test]
    fn test_limit_type_display() {
        let limit = LimitType::LlmCalls { used: 10, limit: 5 };
        assert_eq!(format!("{}", limit), "LLM calls (10 used, 5 limit)");

        let limit = LimitType::CustomToolCalls { used: 25, limit: 20 };
        assert_eq!(format!("{}", limit), "custom tool calls (25 used, 20 limit)");

        let limit = LimitType::TotalTokens { used: 1000, limit: 500 };
        assert_eq!(format!("{}", limit), "total tokens (1000 used, 500 limit)");
    }
}
