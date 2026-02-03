//! Core Agent struct and builder for spider_agent.

use crate::config::{AgentConfig, UsageSnapshot, UsageStats};
use crate::error::{AgentError, AgentResult};
#[cfg(feature = "search")]
use crate::config::{ResearchOptions, SearchOptions};
use crate::llm::{CompletionOptions, CompletionResponse, LLMProvider, Message};
#[cfg(feature = "search")]
use crate::llm::TokenUsage;
use crate::memory::AgentMemory;
use std::sync::Arc;
use tokio::sync::Semaphore;

#[cfg(feature = "search")]
use crate::search::{SearchProvider, SearchResults};

#[cfg(feature = "chrome")]
use crate::browser::BrowserContext;

#[cfg(feature = "fs")]
use crate::temp::TempStorage;

/// Multimodal agent for web automation and research.
///
/// Designed to be wrapped in `Arc` for concurrent access.
///
/// # Example
/// ```ignore
/// use spider_agent::{Agent, AgentConfig};
/// use std::sync::Arc;
///
/// let agent = Arc::new(Agent::builder()
///     .with_openai("sk-...", "gpt-4o")
///     .with_search_serper("serper-key")
///     .build()?);
///
/// // Spawn concurrent tasks
/// let agent_clone = agent.clone();
/// tokio::spawn(async move {
///     agent_clone.search("rust web frameworks").await
/// });
/// ```
pub struct Agent {
    /// LLM provider for inference.
    llm: Option<Box<dyn LLMProvider>>,

    /// HTTP client for requests.
    client: reqwest::Client,

    /// Search provider (if configured).
    #[cfg(feature = "search")]
    search_provider: Option<Box<dyn SearchProvider>>,

    /// Browser context for Chrome automation.
    #[cfg(feature = "chrome")]
    browser: Option<BrowserContext>,

    /// Temporary storage for large operations.
    #[cfg(feature = "fs")]
    temp_storage: Option<TempStorage>,

    /// Session memory (lock-free via DashMap).
    memory: AgentMemory,

    /// Concurrency semaphore for LLM calls.
    llm_semaphore: Arc<Semaphore>,

    /// Configuration.
    config: AgentConfig,

    /// Usage statistics (atomic counters for lock-free updates).
    usage: Arc<UsageStats>,
}

impl Agent {
    /// Create a new agent builder.
    pub fn builder() -> AgentBuilder {
        AgentBuilder::new()
    }

    // ==================== Search Methods ====================

    /// Search the web and return results.
    #[cfg(feature = "search")]
    pub async fn search(&self, query: &str) -> AgentResult<SearchResults> {
        self.search_with_options(query, SearchOptions::default())
            .await
    }

    /// Search with custom options.
    #[cfg(feature = "search")]
    pub async fn search_with_options(
        &self,
        query: &str,
        options: SearchOptions,
    ) -> AgentResult<SearchResults> {
        let provider = self
            .search_provider
            .as_ref()
            .ok_or(AgentError::NotConfigured("search provider"))?;

        self.usage.increment_search_calls();

        provider
            .search(query, &options, &self.client)
            .await
            .map_err(AgentError::Search)
    }

    // ==================== LLM Methods ====================

    /// Send a prompt to the LLM and get a response.
    pub async fn prompt(&self, messages: Vec<Message>) -> AgentResult<String> {
        let response = self.complete(messages).await?;
        Ok(response.content)
    }

    /// Send a completion request with full options.
    pub async fn complete(&self, messages: Vec<Message>) -> AgentResult<CompletionResponse> {
        let llm = self
            .llm
            .as_ref()
            .ok_or(AgentError::NotConfigured("LLM provider"))?;

        let _permit = self.llm_semaphore.acquire().await.map_err(|_| {
            AgentError::Llm("Failed to acquire semaphore".to_string())
        })?;

        let options = CompletionOptions {
            temperature: self.config.temperature,
            max_tokens: self.config.max_tokens,
            json_mode: self.config.json_mode,
        };

        self.usage.increment_llm_calls();

        let response = llm.complete(messages, &options, &self.client).await?;

        // Track token usage
        self.usage.add_tokens(
            response.usage.prompt_tokens as u64,
            response.usage.completion_tokens as u64,
        );

        Ok(response)
    }

    // ==================== Extraction Methods ====================

    /// Extract structured data from HTML using the LLM.
    pub async fn extract(&self, html: &str, prompt: &str) -> AgentResult<serde_json::Value> {
        let cleaned_html = self.clean_html(html);
        let truncated = self.truncate_html(&cleaned_html);

        let messages = vec![
            Message::system(
                "You are a data extraction assistant. Extract the requested information from the HTML and return it as JSON.",
            ),
            Message::user(format!(
                "Extract the following from this HTML:\n\n{}\n\nHTML:\n{}",
                prompt, truncated
            )),
        ];

        let response = self.complete(messages).await?;
        self.parse_json(&response.content)
    }

    /// Extract data with a JSON schema for structured output.
    pub async fn extract_structured(
        &self,
        html: &str,
        schema: &serde_json::Value,
    ) -> AgentResult<serde_json::Value> {
        let cleaned_html = self.clean_html(html);
        let truncated = self.truncate_html(&cleaned_html);

        let messages = vec![
            Message::system(
                "You are a data extraction assistant. Extract data matching the provided JSON schema.",
            ),
            Message::user(format!(
                "Extract data matching this schema:\n{}\n\nFrom this HTML:\n{}",
                serde_json::to_string_pretty(schema).unwrap_or_default(),
                truncated
            )),
        ];

        let response = self.complete(messages).await?;
        self.parse_json(&response.content)
    }

    // ==================== HTTP Methods ====================

    /// Fetch a URL and return the HTML content.
    pub async fn fetch(&self, url: &str) -> AgentResult<FetchResult> {
        self.usage.increment_fetch_calls();

        let response = self
            .client
            .get(url)
            .send()
            .await?;

        let status = response.status();
        let headers = response.headers().clone();

        let content_type = headers
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        let html = response.text().await?;

        Ok(FetchResult {
            url: url.to_string(),
            status: status.as_u16(),
            content_type,
            html,
        })
    }

    // ==================== Research Methods ====================

    /// Research a topic using search and extraction.
    #[cfg(feature = "search")]
    pub async fn research(
        &self,
        topic: &str,
        options: ResearchOptions,
    ) -> AgentResult<ResearchResult> {
        // Search for the topic
        let search_opts = options.search_options.clone().unwrap_or_else(|| {
            SearchOptions::new().with_limit(options.max_pages.max(5))
        });

        let search_results = self.search_with_options(topic, search_opts).await?;

        if search_results.is_empty() {
            return Ok(ResearchResult {
                topic: topic.to_string(),
                search_results,
                extractions: Vec::new(),
                summary: None,
                usage: TokenUsage::default(),
            });
        }

        // Extract from each result
        let extraction_prompt = options.extraction_prompt.clone().unwrap_or_else(|| {
            format!(
                "Extract key information relevant to: {}. Include facts, data points, and insights.",
                topic
            )
        });

        let mut extractions = Vec::new();
        let mut total_usage = TokenUsage::default();

        let max_pages = options.max_pages.min(search_results.results.len());

        for result in search_results.results.iter().take(max_pages) {
            // Fetch page
            match self.fetch(&result.url).await {
                Ok(fetch_result) => {
                    // Extract
                    match self.extract(&fetch_result.html, &extraction_prompt).await {
                        Ok(extracted) => {
                            extractions.push(PageExtraction {
                                url: result.url.clone(),
                                title: result.title.clone(),
                                extracted,
                            });
                        }
                        Err(e) => {
                            log::debug!("Extraction failed for {}: {}", result.url, e);
                        }
                    }
                }
                Err(e) => {
                    log::debug!("Fetch failed for {}: {}", result.url, e);
                }
            }
        }

        // Synthesize if requested
        let summary = if options.synthesize && !extractions.is_empty() {
            match self.synthesize_research(topic, &extractions).await {
                Ok((summary, usage)) => {
                    total_usage.accumulate(&usage);
                    Some(summary)
                }
                Err(e) => {
                    log::debug!("Synthesis failed: {}", e);
                    None
                }
            }
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

    /// Synthesize research findings into a summary.
    #[cfg(feature = "search")]
    async fn synthesize_research(
        &self,
        topic: &str,
        extractions: &[PageExtraction],
    ) -> AgentResult<(String, TokenUsage)> {
        let mut context = String::new();
        for (i, extraction) in extractions.iter().enumerate() {
            context.push_str(&format!(
                "\n\nSource {} ({}): {}\n{}",
                i + 1,
                extraction.url,
                extraction.title,
                serde_json::to_string_pretty(&extraction.extracted).unwrap_or_default()
            ));
        }

        let messages = vec![
            Message::system(
                "You are a research synthesis assistant. Summarize the findings from multiple sources into a coherent response.",
            ),
            Message::user(format!(
                "Topic: {}\n\nSources:{}\n\nProvide a comprehensive summary of the findings, citing sources where appropriate. Return as JSON with a 'summary' field.",
                topic, context
            )),
        ];

        let response = self.complete(messages).await?;
        let json = self.parse_json(&response.content)?;
        let summary = json
            .get("summary")
            .and_then(|v| v.as_str())
            .unwrap_or(&response.content)
            .to_string();

        Ok((summary, response.usage))
    }

    // ==================== Memory Methods ====================

    /// Get a value from memory (lock-free).
    pub fn memory_get(&self, key: &str) -> Option<serde_json::Value> {
        self.memory.get(key)
    }

    /// Set a value in memory (lock-free).
    pub fn memory_set(&self, key: &str, value: serde_json::Value) {
        self.memory.set(key, value);
    }

    /// Clear all memory (lock-free).
    pub fn memory_clear(&self) {
        self.memory.clear();
    }

    /// Get the memory instance for direct access.
    pub fn memory(&self) -> &AgentMemory {
        &self.memory
    }

    // ==================== Usage Methods ====================

    /// Get a snapshot of usage statistics.
    pub fn usage(&self) -> UsageSnapshot {
        self.usage.snapshot()
    }

    /// Get the raw usage stats for direct access.
    pub fn usage_stats(&self) -> &Arc<UsageStats> {
        &self.usage
    }

    /// Reset usage statistics.
    pub fn reset_usage(&self) {
        self.usage.reset();
    }

    // ==================== Browser Methods ====================

    /// Get the browser context if configured.
    #[cfg(feature = "chrome")]
    pub fn browser(&self) -> Option<&BrowserContext> {
        self.browser.as_ref()
    }

    /// Navigate to a URL using the browser.
    #[cfg(feature = "chrome")]
    pub async fn navigate(&self, url: &str) -> AgentResult<()> {
        let browser = self.browser.as_ref()
            .ok_or(AgentError::NotConfigured("browser"))?;
        browser.navigate(url).await
            .map_err(|e| AgentError::Browser(e.to_string()))
    }

    /// Get HTML from the current browser page.
    #[cfg(feature = "chrome")]
    pub async fn browser_html(&self) -> AgentResult<String> {
        let browser = self.browser.as_ref()
            .ok_or(AgentError::NotConfigured("browser"))?;
        browser.html().await
            .map_err(|e| AgentError::Browser(e.to_string()))
    }

    /// Take a screenshot of the current browser page.
    #[cfg(feature = "chrome")]
    pub async fn screenshot(&self) -> AgentResult<Vec<u8>> {
        let browser = self.browser.as_ref()
            .ok_or(AgentError::NotConfigured("browser"))?;
        browser.screenshot().await
            .map_err(|e| AgentError::Browser(e.to_string()))
    }

    /// Open a new page/tab in the browser.
    #[cfg(feature = "chrome")]
    pub async fn new_page(&self) -> AgentResult<crate::browser::BrowserContext> {
        let browser = self.browser.as_ref()
            .ok_or(AgentError::NotConfigured("browser"))?;
        browser.clone_page().await
            .map_err(|e| AgentError::Browser(e.to_string()))
    }

    /// Open a new page and navigate to URL.
    #[cfg(feature = "chrome")]
    pub async fn new_page_with_url(&self, url: &str) -> AgentResult<std::sync::Arc<crate::browser::Page>> {
        let browser = self.browser.as_ref()
            .ok_or(AgentError::NotConfigured("browser"))?;
        browser.new_page_with_url(url).await
            .map_err(|e| AgentError::Browser(e.to_string()))
    }

    /// Click an element in the browser.
    #[cfg(feature = "chrome")]
    pub async fn click(&self, selector: &str) -> AgentResult<()> {
        let browser = self.browser.as_ref()
            .ok_or(AgentError::NotConfigured("browser"))?;
        browser.click(selector).await
            .map_err(|e| AgentError::Browser(e.to_string()))
    }

    /// Type text into an element in the browser.
    #[cfg(feature = "chrome")]
    pub async fn type_text(&self, selector: &str, text: &str) -> AgentResult<()> {
        let browser = self.browser.as_ref()
            .ok_or(AgentError::NotConfigured("browser"))?;
        browser.type_text(selector, text).await
            .map_err(|e| AgentError::Browser(e.to_string()))
    }

    /// Extract from the current browser page using the LLM.
    #[cfg(feature = "chrome")]
    pub async fn extract_page(&self, prompt: &str) -> AgentResult<serde_json::Value> {
        let html = self.browser_html().await?;
        self.extract(&html, prompt).await
    }

    // ==================== Temp Storage Methods ====================

    /// Get the temp storage if configured.
    #[cfg(feature = "fs")]
    pub fn temp_storage(&self) -> Option<&TempStorage> {
        self.temp_storage.as_ref()
    }

    /// Store data in temp storage.
    #[cfg(feature = "fs")]
    pub fn store_temp(&self, name: &str, data: &[u8]) -> AgentResult<std::path::PathBuf> {
        let storage = self.temp_storage.as_ref()
            .ok_or(AgentError::NotConfigured("temp storage"))?;
        storage.store_bytes(name, data)
            .map_err(|e| AgentError::Io(e))
    }

    /// Store JSON in temp storage.
    #[cfg(feature = "fs")]
    pub fn store_temp_json(&self, name: &str, data: &serde_json::Value) -> AgentResult<std::path::PathBuf> {
        let storage = self.temp_storage.as_ref()
            .ok_or(AgentError::NotConfigured("temp storage"))?;
        storage.store_json(name, data)
            .map_err(|e| AgentError::Io(e))
    }

    /// Read data from temp storage.
    #[cfg(feature = "fs")]
    pub fn read_temp(&self, name: &str) -> AgentResult<Vec<u8>> {
        let storage = self.temp_storage.as_ref()
            .ok_or(AgentError::NotConfigured("temp storage"))?;
        storage.read_bytes(name)
            .map_err(|e| AgentError::Io(e))
    }

    /// Read JSON from temp storage.
    #[cfg(feature = "fs")]
    pub fn read_temp_json(&self, name: &str) -> AgentResult<serde_json::Value> {
        let storage = self.temp_storage.as_ref()
            .ok_or(AgentError::NotConfigured("temp storage"))?;
        storage.read_json(name)
            .map_err(|e| AgentError::Io(e))
    }

    // ==================== Helper Methods ====================

    /// Clean HTML by removing scripts, styles, etc.
    fn clean_html(&self, html: &str) -> String {
        use crate::config::HtmlCleaningMode;

        match self.config.html_cleaning {
            HtmlCleaningMode::Raw => html.to_string(),
            HtmlCleaningMode::Minimal => {
                // Remove scripts only
                remove_tags(html, &["script"])
            }
            HtmlCleaningMode::Default => {
                // Remove scripts, styles, comments
                remove_tags(html, &["script", "style", "noscript"])
            }
            HtmlCleaningMode::Aggressive => {
                // Remove more elements
                remove_tags(
                    html,
                    &["script", "style", "noscript", "svg", "canvas", "video", "audio", "iframe"],
                )
            }
        }
    }

    /// Truncate HTML to max bytes.
    fn truncate_html<'a>(&self, html: &'a str) -> &'a str {
        if html.len() <= self.config.html_max_bytes {
            html
        } else {
            // Find a safe break point
            let truncated = &html[..self.config.html_max_bytes];
            // Try to break at a tag boundary
            if let Some(pos) = truncated.rfind('<') {
                &truncated[..pos]
            } else {
                truncated
            }
        }
    }

    /// Parse JSON from LLM response.
    fn parse_json(&self, content: &str) -> AgentResult<serde_json::Value> {
        // Try direct parse first
        if let Ok(json) = serde_json::from_str(content) {
            return Ok(json);
        }

        // Try to extract JSON from markdown code block
        if let Some(start) = content.find("```json") {
            let start = start + 7;
            if let Some(end) = content[start..].find("```") {
                let json_str = content[start..start + end].trim();
                if let Ok(json) = serde_json::from_str(json_str) {
                    return Ok(json);
                }
            }
        }

        // Try to extract JSON from plain code block
        if let Some(start) = content.find("```") {
            let start = start + 3;
            // Skip language identifier if present
            let start = content[start..]
                .find('\n')
                .map(|n| start + n + 1)
                .unwrap_or(start);
            if let Some(end) = content[start..].find("```") {
                let json_str = content[start..start + end].trim();
                if let Ok(json) = serde_json::from_str(json_str) {
                    return Ok(json);
                }
            }
        }

        // Try to find JSON object in content
        if let Some(start) = content.find('{') {
            if let Some(end) = content.rfind('}') {
                let json_str = &content[start..=end];
                if let Ok(json) = serde_json::from_str(json_str) {
                    return Ok(json);
                }
            }
        }

        Err(AgentError::Json(serde_json::from_str::<serde_json::Value>(content).unwrap_err()))
    }
}

/// Remove specified HTML tags from content.
fn remove_tags(html: &str, tags: &[&str]) -> String {
    let mut result = html.to_string();
    for tag in tags {
        // Remove opening and closing tags with content
        let open_tag = format!("<{}", tag);
        let close_tag = format!("</{}>", tag);

        while let Some(start) = result.to_lowercase().find(&open_tag) {
            if let Some(end) = result[start..].to_lowercase().find(&close_tag) {
                let end = start + end + close_tag.len();
                result = format!("{}{}", &result[..start], &result[end..]);
            } else {
                break;
            }
        }
    }
    result
}

/// Result from fetching a URL.
#[derive(Debug, Clone)]
pub struct FetchResult {
    /// The URL that was fetched.
    pub url: String,
    /// HTTP status code.
    pub status: u16,
    /// Content type header.
    pub content_type: String,
    /// HTML content.
    pub html: String,
}

/// Result from research.
#[cfg(feature = "search")]
#[derive(Debug, Clone)]
pub struct ResearchResult {
    /// The original research topic.
    pub topic: String,
    /// Search results used.
    pub search_results: SearchResults,
    /// Extracted data from each page.
    pub extractions: Vec<PageExtraction>,
    /// Synthesized summary.
    pub summary: Option<String>,
    /// Token usage.
    pub usage: TokenUsage,
}

/// Extraction from a single page.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PageExtraction {
    /// Page URL.
    pub url: String,
    /// Page title.
    pub title: String,
    /// Extracted data.
    pub extracted: serde_json::Value,
}

/// Agent builder for configuring and creating agents.
pub struct AgentBuilder {
    config: AgentConfig,
    llm: Option<Box<dyn LLMProvider>>,
    #[cfg(feature = "search")]
    search_provider: Option<Box<dyn SearchProvider>>,
    #[cfg(feature = "chrome")]
    browser: Option<BrowserContext>,
    #[cfg(feature = "fs")]
    enable_temp_storage: bool,
}

impl AgentBuilder {
    /// Create a new builder with defaults.
    pub fn new() -> Self {
        Self {
            config: AgentConfig::default(),
            llm: None,
            #[cfg(feature = "search")]
            search_provider: None,
            #[cfg(feature = "chrome")]
            browser: None,
            #[cfg(feature = "fs")]
            enable_temp_storage: false,
        }
    }

    /// Set the agent configuration.
    pub fn with_config(mut self, config: AgentConfig) -> Self {
        self.config = config;
        self
    }

    /// Set the system prompt.
    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.config.system_prompt = Some(prompt.into());
        self
    }

    /// Set max concurrent LLM calls.
    pub fn with_max_concurrent_llm_calls(mut self, n: usize) -> Self {
        self.config.max_concurrent_llm_calls = n;
        self
    }

    /// Configure with OpenAI provider.
    #[cfg(feature = "openai")]
    pub fn with_openai(mut self, api_key: impl Into<String>, model: impl Into<String>) -> Self {
        self.llm = Some(Box::new(crate::llm::OpenAIProvider::new(api_key, model)));
        self
    }

    /// Configure with OpenAI-compatible provider.
    #[cfg(feature = "openai")]
    pub fn with_openai_compatible(
        mut self,
        api_url: impl Into<String>,
        api_key: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        self.llm = Some(Box::new(
            crate::llm::OpenAIProvider::new(api_key, model).with_api_url(api_url),
        ));
        self
    }

    /// Configure with Serper search provider.
    #[cfg(feature = "search_serper")]
    pub fn with_search_serper(mut self, api_key: impl Into<String>) -> Self {
        self.search_provider = Some(Box::new(crate::search::SerperProvider::new(api_key)));
        self
    }

    /// Configure with Brave search provider.
    #[cfg(feature = "search_brave")]
    pub fn with_search_brave(mut self, api_key: impl Into<String>) -> Self {
        self.search_provider = Some(Box::new(crate::search::BraveProvider::new(api_key)));
        self
    }

    /// Configure with Bing search provider.
    #[cfg(feature = "search_bing")]
    pub fn with_search_bing(mut self, api_key: impl Into<String>) -> Self {
        self.search_provider = Some(Box::new(crate::search::BingProvider::new(api_key)));
        self
    }

    /// Configure with Tavily search provider.
    #[cfg(feature = "search_tavily")]
    pub fn with_search_tavily(mut self, api_key: impl Into<String>) -> Self {
        self.search_provider = Some(Box::new(crate::search::TavilyProvider::new(api_key)));
        self
    }

    /// Configure with a browser context for Chrome automation.
    #[cfg(feature = "chrome")]
    pub fn with_browser(mut self, browser: BrowserContext) -> Self {
        self.browser = Some(browser);
        self
    }

    /// Configure with a browser from existing browser and page.
    #[cfg(feature = "chrome")]
    pub fn with_browser_page(
        mut self,
        browser: std::sync::Arc<crate::browser::Browser>,
        page: std::sync::Arc<crate::browser::Page>,
    ) -> Self {
        self.browser = Some(BrowserContext::new(browser, page));
        self
    }

    /// Enable temporary filesystem storage for large operations.
    #[cfg(feature = "fs")]
    pub fn with_temp_storage(mut self) -> Self {
        self.enable_temp_storage = true;
        self
    }

    /// Build the agent.
    pub fn build(self) -> AgentResult<Agent> {
        let client = reqwest::Client::builder()
            .timeout(self.config.timeout)
            .build()
            .map_err(|e| AgentError::Http(e))?;

        let semaphore = Arc::new(Semaphore::new(self.config.max_concurrent_llm_calls));

        #[cfg(feature = "fs")]
        let temp_storage = if self.enable_temp_storage {
            Some(TempStorage::new().map_err(|e| AgentError::Io(e))?)
        } else {
            None
        };

        Ok(Agent {
            llm: self.llm,
            client,
            #[cfg(feature = "search")]
            search_provider: self.search_provider,
            #[cfg(feature = "chrome")]
            browser: self.browser,
            #[cfg(feature = "fs")]
            temp_storage,
            memory: AgentMemory::new(),
            llm_semaphore: semaphore,
            config: self.config,
            usage: Arc::new(UsageStats::new()),
        })
    }
}

impl Default for AgentBuilder {
    fn default() -> Self {
        Self::new()
    }
}
