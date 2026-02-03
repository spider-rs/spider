//! Configuration types for spider_agent.

use serde::{Deserialize, Serialize};
use std::time::Duration;

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
