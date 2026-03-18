//! Custom tool support for external API calls.

use dashmap::DashMap;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;

use crate::error::{AgentError, AgentResult};

const DEFAULT_SPIDER_CLOUD_API_URL: &str = "https://api.spider.cloud";
const DEFAULT_SPIDER_CLOUD_AUTH_HEADER: &str = "Authorization";
const DEFAULT_TOOL_PREFIX: &str = "spider_cloud";

fn strip_bearer_prefix(value: &str) -> &str {
    let trimmed = value.trim();
    if trimmed.len() >= 7 && trimmed[..7].eq_ignore_ascii_case("bearer ") {
        trimmed[7..].trim_start()
    } else {
        trimmed
    }
}

/// HTTP method for API calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HttpMethod {
    /// GET request.
    Get,
    /// POST request.
    Post,
    /// PUT request.
    Put,
    /// PATCH request.
    Patch,
    /// DELETE request.
    Delete,
}

impl HttpMethod {
    fn as_reqwest_method(&self) -> reqwest::Method {
        match self {
            HttpMethod::Get => reqwest::Method::GET,
            HttpMethod::Post => reqwest::Method::POST,
            HttpMethod::Put => reqwest::Method::PUT,
            HttpMethod::Patch => reqwest::Method::PATCH,
            HttpMethod::Delete => reqwest::Method::DELETE,
        }
    }
}

/// Authentication configuration for custom tools.
#[derive(Debug, Clone)]
pub enum AuthConfig {
    /// No authentication.
    None,
    /// Bearer token authentication.
    Bearer(String),
    /// API key in header.
    ApiKey {
        /// Header name for the API key.
        header: String,
        /// API key value.
        key: String,
    },
    /// Basic authentication.
    Basic {
        /// Username.
        username: String,
        /// Password.
        password: String,
    },
    /// Custom header authentication.
    CustomHeader {
        /// Header name.
        name: String,
        /// Header value.
        value: String,
    },
}

/// Configuration for Spider Cloud tool registration.
///
/// By default this registers core routes:
/// - `/crawl`
/// - `/scrape`
/// - `/search`
/// - `/links`
/// - `/transform`
/// - `/unblocker`
///
/// AI routes are disabled by default and must be explicitly enabled with
/// `with_enable_ai_routes(true)` because they require a Spider Cloud AI plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SpiderCloudToolConfig {
    /// Spider Cloud API key.
    pub api_key: String,
    /// Spider Cloud API base URL.
    pub api_url: String,
    /// Prefix used for registered tool names.
    ///
    /// Default: `spider_cloud`, resulting in names like `spider_cloud_scrape`.
    /// Set to empty string for unprefixed names (`scrape`, `search`, etc.).
    pub tool_name_prefix: String,
    /// Header used for API key auth. Defaults to `Authorization`.
    pub auth_header: String,
    /// Whether to use `Bearer <key>` formatting for the Authorization header.
    ///
    /// Spider Cloud expects raw `Authorization: <key>` by default, so this is
    /// `false` unless explicitly enabled.
    pub use_bearer_auth: bool,
    /// Request timeout in seconds for each tool call.
    pub timeout_secs: u64,
    /// Register `/crawl`.
    pub include_crawl: bool,
    /// Register `/scrape`.
    pub include_scrape: bool,
    /// Register `/search`.
    pub include_search: bool,
    /// Register `/links`.
    pub include_links: bool,
    /// Register `/transform`.
    pub include_transform: bool,
    /// Register `/unblocker`.
    pub include_unblocker: bool,
    /// Register `/ai/*` routes.
    ///
    /// These routes require a paid Spider Cloud AI subscription:
    /// https://spider.cloud/ai/pricing
    pub enable_ai_routes: bool,
}

impl Default for SpiderCloudToolConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            api_url: DEFAULT_SPIDER_CLOUD_API_URL.to_string(),
            tool_name_prefix: DEFAULT_TOOL_PREFIX.to_string(),
            auth_header: DEFAULT_SPIDER_CLOUD_AUTH_HEADER.to_string(),
            use_bearer_auth: false,
            timeout_secs: 60,
            include_crawl: true,
            include_scrape: true,
            include_search: true,
            include_links: true,
            include_transform: true,
            include_unblocker: true,
            enable_ai_routes: false,
        }
    }
}

impl SpiderCloudToolConfig {
    /// Create a Spider Cloud config with core routes enabled.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            ..Self::default()
        }
    }

    /// Set Spider Cloud API base URL.
    pub fn with_api_url(mut self, api_url: impl Into<String>) -> Self {
        self.api_url = api_url.into();
        self
    }

    /// Set the prefix for generated tool names.
    ///
    /// Example:
    /// - prefix `spider_cloud` -> `spider_cloud_search`
    /// - prefix `web_api` -> `web_api_search`
    /// - empty prefix -> `search`
    pub fn with_tool_name_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.tool_name_prefix = prefix.into();
        self
    }

    /// Set auth header name. Use non-default header names for custom gateways.
    pub fn with_auth_header(mut self, auth_header: impl Into<String>) -> Self {
        self.auth_header = auth_header.into();
        self
    }

    /// Enable/disable Bearer formatting for Authorization auth.
    ///
    /// When `true`, sends `Authorization: Bearer <key>`.
    /// When `false` (default), sends `Authorization: <key>`.
    pub fn with_bearer_auth(mut self, enabled: bool) -> Self {
        self.use_bearer_auth = enabled;
        self
    }

    /// Set timeout in seconds for each registered tool.
    pub fn with_timeout_secs(mut self, timeout_secs: u64) -> Self {
        self.timeout_secs = timeout_secs.max(1);
        self
    }

    /// Enable or disable `/unblocker` route registration.
    pub fn with_unblocker(mut self, enabled: bool) -> Self {
        self.include_unblocker = enabled;
        self
    }

    /// Enable or disable `/transform` route registration.
    pub fn with_transform(mut self, enabled: bool) -> Self {
        self.include_transform = enabled;
        self
    }

    /// Enable or disable AI route registration.
    ///
    /// AI routes require a paid Spider Cloud AI plan:
    /// https://spider.cloud/ai/pricing
    pub fn with_enable_ai_routes(mut self, enabled: bool) -> Self {
        self.enable_ai_routes = enabled;
        self
    }

    fn endpoint(&self, route: &str) -> String {
        format!(
            "{}/{}",
            self.api_url.trim_end_matches('/'),
            route.trim_start_matches('/')
        )
    }

    fn tool_name(&self, suffix: &str) -> String {
        let prefix = self.tool_name_prefix.trim().trim_end_matches('_');
        if prefix.is_empty() {
            suffix.to_string()
        } else {
            format!("{}_{}", prefix, suffix)
        }
    }

    fn auth_tool(&self, tool: CustomTool) -> CustomTool {
        if self
            .auth_header
            .eq_ignore_ascii_case(DEFAULT_SPIDER_CLOUD_AUTH_HEADER)
        {
            // Accept env inputs like `SPIDER_CLOUD_API_KEY=...` and
            // `SPIDER_CLOUD_API_KEY=Bearer ...` without double-prefixing.
            let token = strip_bearer_prefix(&self.api_key).to_string();
            if self.use_bearer_auth {
                tool.with_bearer_auth(token)
            } else {
                tool.with_api_key(self.auth_header.clone(), token)
            }
        } else {
            tool.with_api_key(self.auth_header.clone(), self.api_key.trim().to_string())
        }
    }

    fn build_tool(&self, name: &str, route: &str, description: &str) -> CustomTool {
        let tool = CustomTool::new(name, self.endpoint(route))
            .with_description(description)
            .with_method(HttpMethod::Post)
            .with_content_type("application/json")
            .with_timeout(Duration::from_secs(self.timeout_secs))
            .with_header(
                "User-Agent",
                format!("spider_agent/{}", env!("CARGO_PKG_VERSION")),
            );
        self.auth_tool(tool)
    }

    /// Build Spider Cloud tools from this configuration.
    pub fn to_custom_tools(&self) -> Vec<CustomTool> {
        let mut tools = Vec::new();

        if self.include_crawl {
            tools.push(self.build_tool(
                &self.tool_name("crawl"),
                "crawl",
                "Spider Cloud /crawl endpoint for crawling and extraction.",
            ));
        }
        if self.include_scrape {
            tools.push(self.build_tool(
                &self.tool_name("scrape"),
                "scrape",
                "Spider Cloud /scrape endpoint for page scraping and extraction.",
            ));
        }
        if self.include_search {
            tools.push(self.build_tool(
                &self.tool_name("search"),
                "search",
                "Spider Cloud /search endpoint for web search plus page retrieval.",
            ));
        }
        if self.include_links {
            tools.push(self.build_tool(
                &self.tool_name("links"),
                "links",
                "Spider Cloud /links endpoint for link extraction only.",
            ));
        }
        if self.include_transform {
            tools.push(self.build_tool(
                &self.tool_name("transform"),
                "transform",
                "Spider Cloud /transform endpoint for structured content transformation.",
            ));
        }
        if self.include_unblocker {
            tools.push(self.build_tool(
                &self.tool_name("unblocker"),
                "unblocker",
                "Spider Cloud /unblocker endpoint for anti-bot bypass and hard-to-reach pages.",
            ));
        }

        if self.enable_ai_routes {
            tools.push(self.build_tool(
                &self.tool_name("ai_crawl"),
                "ai/crawl",
                "Spider Cloud /ai/crawl endpoint for AI-guided crawling (AI subscription required).",
            ));
            tools.push(self.build_tool(
                &self.tool_name("ai_scrape"),
                "ai/scrape",
                "Spider Cloud /ai/scrape endpoint for AI-guided scraping (AI subscription required).",
            ));
            tools.push(self.build_tool(
                &self.tool_name("ai_search"),
                "ai/search",
                "Spider Cloud /ai/search endpoint for AI-enhanced search (AI subscription required).",
            ));
            tools.push(self.build_tool(
                &self.tool_name("ai_browser"),
                "ai/browser",
                "Spider Cloud /ai/browser endpoint for AI browser automation (AI subscription required).",
            ));
            tools.push(self.build_tool(
                &self.tool_name("ai_links"),
                "ai/links",
                "Spider Cloud /ai/links endpoint for AI link extraction (AI subscription required).",
            ));
        }

        tools
    }
}

// ─── Spider Browser Cloud ────────────────────────────────────────────────────

const DEFAULT_SPIDER_BROWSER_WSS_URL: &str = "wss://browser.spider.cloud/v1/browser";
const DEFAULT_BROWSER_TOOL_PREFIX: &str = "spider_browser";

/// Configuration for [Spider Browser Cloud](https://spider.cloud/docs/api#browser)
/// tool registration.
///
/// Registers tools that interact with a remote CDP browser session at
/// `wss://browser.spider.cloud/v1/browser?token=API_KEY`.
///
/// Tools registered by default:
/// - `spider_browser_navigate` — navigate to a URL
/// - `spider_browser_html` — get page HTML
/// - `spider_browser_screenshot` — take a screenshot
/// - `spider_browser_evaluate` — execute JavaScript
/// - `spider_browser_click` — click a CSS selector
/// - `spider_browser_fill` — fill an input element
/// - `spider_browser_wait` — wait for a selector
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SpiderBrowserToolConfig {
    /// Spider Cloud API key.
    pub api_key: String,
    /// WebSocket base URL for the browser endpoint.
    pub wss_url: String,
    /// Prefix used for registered tool names (default: `spider_browser`).
    pub tool_name_prefix: String,
    /// Enable stealth mode (anti-fingerprinting).
    pub stealth: bool,
    /// Browser type (e.g. `"chrome"`, `"firefox"`).
    pub browser: Option<String>,
    /// Country code for geo-targeting (e.g. `"us"`, `"gb"`).
    pub country: Option<String>,
    /// Request timeout in seconds for each tool call.
    pub timeout_secs: u64,
    /// Register navigate tool.
    pub include_navigate: bool,
    /// Register HTML extraction tool.
    pub include_html: bool,
    /// Register screenshot tool.
    pub include_screenshot: bool,
    /// Register JavaScript evaluation tool.
    pub include_evaluate: bool,
    /// Register click tool.
    pub include_click: bool,
    /// Register fill (type text) tool.
    pub include_fill: bool,
    /// Register wait-for-selector tool.
    pub include_wait: bool,
}

impl Default for SpiderBrowserToolConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            wss_url: DEFAULT_SPIDER_BROWSER_WSS_URL.to_string(),
            tool_name_prefix: DEFAULT_BROWSER_TOOL_PREFIX.to_string(),
            stealth: false,
            browser: None,
            country: None,
            timeout_secs: 60,
            include_navigate: true,
            include_html: true,
            include_screenshot: true,
            include_evaluate: true,
            include_click: true,
            include_fill: true,
            include_wait: true,
        }
    }
}

impl SpiderBrowserToolConfig {
    /// Create a config with the given API key.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            ..Self::default()
        }
    }

    /// Set a custom WSS base URL.
    pub fn with_wss_url(mut self, url: impl Into<String>) -> Self {
        self.wss_url = url.into();
        self
    }

    /// Set the tool name prefix.
    pub fn with_tool_name_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.tool_name_prefix = prefix.into();
        self
    }

    /// Enable or disable stealth mode.
    pub fn with_stealth(mut self, stealth: bool) -> Self {
        self.stealth = stealth;
        self
    }

    /// Set the browser type to request.
    pub fn with_browser(mut self, browser: impl Into<String>) -> Self {
        self.browser = Some(browser.into());
        self
    }

    /// Set the country for geo-targeting.
    pub fn with_country(mut self, country: impl Into<String>) -> Self {
        self.country = Some(country.into());
        self
    }

    /// Set timeout in seconds for each registered tool.
    pub fn with_timeout_secs(mut self, timeout_secs: u64) -> Self {
        self.timeout_secs = timeout_secs.max(1);
        self
    }

    /// Build the full WSS connection URL with authentication and options.
    pub fn connection_url(&self) -> String {
        let mut url = self.wss_url.clone();
        if url.contains('?') {
            url.push('&');
        } else {
            url.push('?');
        }
        url.push_str("token=");
        url.push_str(&self.api_key);

        if self.stealth {
            url.push_str("&stealth=true");
        }
        if let Some(ref browser) = self.browser {
            url.push_str("&browser=");
            url.push_str(browser);
        }
        if let Some(ref country) = self.country {
            url.push_str("&country=");
            url.push_str(country);
        }
        url
    }

    fn tool_name(&self, suffix: &str) -> String {
        let prefix = self.tool_name_prefix.trim().trim_end_matches('_');
        if prefix.is_empty() {
            suffix.to_string()
        } else {
            format!("{}_{}", prefix, suffix)
        }
    }

    /// Build the browser tools as custom tool definitions.
    ///
    /// These tools use the Spider Browser Cloud REST-like interface where
    /// each tool POSTs a JSON action body to the browser endpoint.
    pub fn to_custom_tools(&self) -> Vec<CustomTool> {
        let mut tools = Vec::new();
        let base = self.connection_url();

        let build = |name: &str, desc: &str| -> CustomTool {
            CustomTool::new(name, &base)
                .with_description(desc)
                .with_method(HttpMethod::Post)
                .with_content_type("application/json")
                .with_timeout(Duration::from_secs(self.timeout_secs))
                .with_header(
                    "User-Agent",
                    format!("spider_agent/{}", env!("CARGO_PKG_VERSION")),
                )
        };

        if self.include_navigate {
            tools.push(build(
                &self.tool_name("navigate"),
                "Spider Browser Cloud: navigate to a URL. Body: {\"url\": \"...\"}",
            ));
        }
        if self.include_html {
            tools.push(build(
                &self.tool_name("html"),
                "Spider Browser Cloud: extract HTML from current page.",
            ));
        }
        if self.include_screenshot {
            tools.push(build(
                &self.tool_name("screenshot"),
                "Spider Browser Cloud: take a screenshot of the current page.",
            ));
        }
        if self.include_evaluate {
            tools.push(build(
                &self.tool_name("evaluate"),
                "Spider Browser Cloud: evaluate JavaScript on the page. Body: {\"script\": \"...\"}",
            ));
        }
        if self.include_click {
            tools.push(build(
                &self.tool_name("click"),
                "Spider Browser Cloud: click an element by CSS selector. Body: {\"selector\": \"...\"}",
            ));
        }
        if self.include_fill {
            tools.push(build(
                &self.tool_name("fill"),
                "Spider Browser Cloud: fill an input element. Body: {\"selector\": \"...\", \"value\": \"...\"}",
            ));
        }
        if self.include_wait {
            tools.push(build(
                &self.tool_name("wait"),
                "Spider Browser Cloud: wait for a CSS selector to appear. Body: {\"selector\": \"...\"}",
            ));
        }

        tools
    }
}

/// Configuration for a custom tool (external API call).
#[derive(Debug, Clone)]
pub struct CustomTool {
    /// Unique name for this tool.
    pub name: String,
    /// Description of what this tool does.
    pub description: String,
    /// Base URL for the API.
    pub base_url: String,
    /// Default HTTP method.
    pub method: HttpMethod,
    /// Authentication configuration.
    pub auth: AuthConfig,
    /// Additional headers.
    pub headers: Vec<(String, String)>,
    /// Request timeout.
    pub timeout: Duration,
    /// Content type for requests.
    pub content_type: Option<String>,
}

impl CustomTool {
    /// Create a new custom tool with GET method.
    pub fn new(name: impl Into<String>, base_url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: String::new(),
            base_url: base_url.into(),
            method: HttpMethod::Get,
            auth: AuthConfig::None,
            headers: Vec::new(),
            timeout: Duration::from_secs(30),
            content_type: None,
        }
    }

    /// Set description.
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = desc.into();
        self
    }

    /// Set HTTP method.
    pub fn with_method(mut self, method: HttpMethod) -> Self {
        self.method = method;
        self
    }

    /// Set bearer token authentication.
    pub fn with_bearer_auth(mut self, token: impl Into<String>) -> Self {
        self.auth = AuthConfig::Bearer(token.into());
        self
    }

    /// Set API key authentication.
    pub fn with_api_key(mut self, header: impl Into<String>, key: impl Into<String>) -> Self {
        self.auth = AuthConfig::ApiKey {
            header: header.into(),
            key: key.into(),
        };
        self
    }

    /// Set basic authentication.
    pub fn with_basic_auth(
        mut self,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        self.auth = AuthConfig::Basic {
            username: username.into(),
            password: password.into(),
        };
        self
    }

    /// Set custom header authentication.
    pub fn with_custom_auth(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.auth = AuthConfig::CustomHeader {
            name: name.into(),
            value: value.into(),
        };
        self
    }

    /// Add a custom header.
    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    /// Set request timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Set content type.
    pub fn with_content_type(mut self, content_type: impl Into<String>) -> Self {
        self.content_type = Some(content_type.into());
        self
    }

    /// Build the headers for a request.
    fn build_headers(&self) -> AgentResult<HeaderMap> {
        let mut headers = HeaderMap::new();

        // Add authentication
        match &self.auth {
            AuthConfig::None => {}
            AuthConfig::Bearer(token) => {
                headers.insert(
                    reqwest::header::AUTHORIZATION,
                    HeaderValue::from_str(&format!("Bearer {}", token))
                        .map_err(|e| AgentError::Tool(format!("Invalid bearer token: {}", e)))?,
                );
            }
            AuthConfig::ApiKey { header, key } => {
                let header_name = HeaderName::try_from(header.as_str())
                    .map_err(|e| AgentError::Tool(format!("Invalid header name: {}", e)))?;
                let header_value = HeaderValue::from_str(key)
                    .map_err(|e| AgentError::Tool(format!("Invalid API key: {}", e)))?;
                headers.insert(header_name, header_value);
            }
            AuthConfig::Basic { username, password } => {
                let credentials = base64::Engine::encode(
                    &base64::engine::general_purpose::STANDARD,
                    format!("{}:{}", username, password),
                );
                headers.insert(
                    reqwest::header::AUTHORIZATION,
                    HeaderValue::from_str(&format!("Basic {}", credentials))
                        .map_err(|e| AgentError::Tool(format!("Invalid basic auth: {}", e)))?,
                );
            }
            AuthConfig::CustomHeader { name, value } => {
                let header_name = HeaderName::try_from(name.as_str())
                    .map_err(|e| AgentError::Tool(format!("Invalid header name: {}", e)))?;
                let header_value = HeaderValue::from_str(value)
                    .map_err(|e| AgentError::Tool(format!("Invalid header value: {}", e)))?;
                headers.insert(header_name, header_value);
            }
        }

        // Add content type if specified
        if let Some(ref ct) = self.content_type {
            headers.insert(
                reqwest::header::CONTENT_TYPE,
                HeaderValue::from_str(ct)
                    .map_err(|e| AgentError::Tool(format!("Invalid content type: {}", e)))?,
            );
        }

        // Add custom headers
        for (name, value) in &self.headers {
            let header_name = HeaderName::try_from(name.as_str())
                .map_err(|e| AgentError::Tool(format!("Invalid header name '{}': {}", name, e)))?;
            let header_value = HeaderValue::from_str(value).map_err(|e| {
                AgentError::Tool(format!("Invalid header value for '{}': {}", name, e))
            })?;
            headers.insert(header_name, header_value);
        }

        Ok(headers)
    }
}

/// Result from executing a custom tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomToolResult {
    /// The tool name that was executed.
    pub tool_name: String,
    /// HTTP status code.
    pub status: u16,
    /// Response body.
    pub body: String,
    /// Response headers.
    pub headers: Vec<(String, String)>,
    /// Whether the request was successful (2xx status).
    pub success: bool,
}

/// Registry for custom tools.
#[derive(Debug, Default)]
pub struct CustomToolRegistry {
    tools: DashMap<String, Arc<CustomTool>>,
}

impl CustomToolRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            tools: DashMap::new(),
        }
    }

    /// Register a custom tool.
    pub fn register(&self, tool: CustomTool) {
        self.tools.insert(tool.name.clone(), Arc::new(tool));
    }

    /// Get a tool by name.
    pub fn get(&self, name: &str) -> Option<Arc<CustomTool>> {
        self.tools.get(name).map(|r| r.clone())
    }

    /// Remove a tool.
    pub fn remove(&self, name: &str) -> Option<Arc<CustomTool>> {
        self.tools.remove(name).map(|(_, v)| v)
    }

    /// List all registered tools.
    pub fn list(&self) -> Vec<String> {
        self.tools.iter().map(|e| e.key().clone()).collect()
    }

    /// Check if a tool is registered.
    pub fn contains(&self, name: &str) -> bool {
        self.tools.contains_key(name)
    }

    /// Clear all tools.
    pub fn clear(&self) {
        self.tools.clear();
    }

    /// Register Spider Cloud tools from a shared config.
    ///
    /// Returns the number of tools registered.
    pub fn register_spider_cloud(&self, config: &SpiderCloudToolConfig) -> usize {
        let tools = config.to_custom_tools();
        let count = tools.len();
        for tool in tools {
            self.register(tool);
        }
        count
    }

    /// Register Spider Browser Cloud tools from a shared config.
    ///
    /// Returns the number of tools registered.
    pub fn register_spider_browser(&self, config: &SpiderBrowserToolConfig) -> usize {
        let tools = config.to_custom_tools();
        let count = tools.len();
        for tool in tools {
            self.register(tool);
        }
        count
    }

    /// Execute a custom tool.
    pub async fn execute(
        &self,
        name: &str,
        client: &reqwest::Client,
        path: Option<&str>,
        query: Option<&[(&str, &str)]>,
        body: Option<&str>,
    ) -> AgentResult<CustomToolResult> {
        let tool = self
            .get(name)
            .ok_or_else(|| AgentError::Tool(format!("Custom tool '{}' not found", name)))?;

        // Build URL
        let mut url = tool.base_url.clone();
        if let Some(p) = path {
            if !url.ends_with('/') && !p.starts_with('/') {
                url.push('/');
            }
            url.push_str(p);
        }

        // Build request
        let mut request = client
            .request(tool.method.as_reqwest_method(), &url)
            .timeout(tool.timeout)
            .headers(tool.build_headers()?);

        // Add query parameters
        if let Some(q) = query {
            request = request.query(q);
        }

        // Add body
        if let Some(b) = body {
            request = request.body(b.to_string());
        }

        // Execute
        let response = request.send().await?;

        let status = response.status().as_u16();
        let success = response.status().is_success();

        let headers: Vec<(String, String)> = response
            .headers()
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
            .collect();

        let body = response.text().await?;

        Ok(CustomToolResult {
            tool_name: name.to_string(),
            status,
            body,
            headers,
            success,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_custom_tool_builder() {
        let tool = CustomTool::new("my_api", "https://api.example.com")
            .with_description("My custom API")
            .with_method(HttpMethod::Post)
            .with_bearer_auth("secret_token")
            .with_header("X-Custom", "value")
            .with_timeout(Duration::from_secs(60))
            .with_content_type("application/json");

        assert_eq!(tool.name, "my_api");
        assert_eq!(tool.base_url, "https://api.example.com");
        assert_eq!(tool.description, "My custom API");
        assert_eq!(tool.method, HttpMethod::Post);
        assert_eq!(tool.timeout, Duration::from_secs(60));
        assert_eq!(tool.content_type, Some("application/json".to_string()));
        assert_eq!(tool.headers.len(), 1);
        assert!(matches!(tool.auth, AuthConfig::Bearer(_)));
    }

    #[test]
    fn test_custom_tool_registry() {
        let registry = CustomToolRegistry::new();

        // Register tools
        let tool1 = CustomTool::new("api_1", "https://api1.example.com");
        let tool2 = CustomTool::new("api_2", "https://api2.example.com");

        registry.register(tool1);
        registry.register(tool2);

        // Check registration
        assert!(registry.contains("api_1"));
        assert!(registry.contains("api_2"));
        assert!(!registry.contains("api_3"));

        // List tools
        let tools = registry.list();
        assert_eq!(tools.len(), 2);
        assert!(tools.contains(&"api_1".to_string()));
        assert!(tools.contains(&"api_2".to_string()));

        // Get tool
        let tool = registry.get("api_1");
        assert!(tool.is_some());
        assert_eq!(tool.unwrap().base_url, "https://api1.example.com");

        // Remove tool
        let removed = registry.remove("api_1");
        assert!(removed.is_some());
        assert!(!registry.contains("api_1"));

        // Clear
        registry.clear();
        assert!(registry.list().is_empty());
    }

    #[test]
    fn test_auth_config_variants() {
        let tool =
            CustomTool::new("test", "https://example.com").with_api_key("X-API-Key", "my_key");
        assert!(matches!(tool.auth, AuthConfig::ApiKey { .. }));

        let tool = CustomTool::new("test", "https://example.com").with_basic_auth("user", "pass");
        assert!(matches!(tool.auth, AuthConfig::Basic { .. }));

        let tool = CustomTool::new("test", "https://example.com")
            .with_custom_auth("X-Custom-Auth", "token123");
        assert!(matches!(tool.auth, AuthConfig::CustomHeader { .. }));
    }

    #[test]
    fn test_http_method_conversion() {
        assert_eq!(HttpMethod::Get.as_reqwest_method(), reqwest::Method::GET);
        assert_eq!(HttpMethod::Post.as_reqwest_method(), reqwest::Method::POST);
        assert_eq!(HttpMethod::Put.as_reqwest_method(), reqwest::Method::PUT);
        assert_eq!(
            HttpMethod::Patch.as_reqwest_method(),
            reqwest::Method::PATCH
        );
        assert_eq!(
            HttpMethod::Delete.as_reqwest_method(),
            reqwest::Method::DELETE
        );
    }

    #[test]
    fn test_custom_tool_result() {
        let result = CustomToolResult {
            tool_name: "my_api".to_string(),
            status: 200,
            body: r#"{"success": true}"#.to_string(),
            headers: vec![("content-type".to_string(), "application/json".to_string())],
            success: true,
        };

        assert_eq!(result.tool_name, "my_api");
        assert_eq!(result.status, 200);
        assert!(result.success);
    }

    #[test]
    fn test_spider_cloud_tools_default_routes_only() {
        let cfg = SpiderCloudToolConfig::new("sk_spider_cloud");
        let tools = cfg.to_custom_tools();
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();

        assert_eq!(tools.len(), 6);
        assert!(names.contains(&"spider_cloud_crawl"));
        assert!(names.contains(&"spider_cloud_scrape"));
        assert!(names.contains(&"spider_cloud_search"));
        assert!(names.contains(&"spider_cloud_links"));
        assert!(names.contains(&"spider_cloud_transform"));
        assert!(names.contains(&"spider_cloud_unblocker"));

        assert!(!names.contains(&"spider_cloud_ai_crawl"));
        assert!(!names.contains(&"spider_cloud_ai_scrape"));
        assert!(!names.contains(&"spider_cloud_ai_search"));
        assert!(!names.contains(&"spider_cloud_ai_browser"));
        assert!(!names.contains(&"spider_cloud_ai_links"));

        // Default auth should be raw Authorization header (not Bearer).
        let crawl = tools
            .iter()
            .find(|t| t.name == "spider_cloud_crawl")
            .expect("crawl tool");
        assert!(matches!(
            crawl.auth,
            AuthConfig::ApiKey {
                ref header,
                ref key
            } if header == "Authorization" && key == "sk_spider_cloud"
        ));
    }

    #[test]
    fn test_spider_cloud_tools_with_ai_subscription_enabled() {
        let cfg = SpiderCloudToolConfig::new("sk_spider_cloud").with_enable_ai_routes(true);
        let tools = cfg.to_custom_tools();
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();

        assert_eq!(tools.len(), 11);
        assert!(names.contains(&"spider_cloud_ai_crawl"));
        assert!(names.contains(&"spider_cloud_ai_scrape"));
        assert!(names.contains(&"spider_cloud_ai_search"));
        assert!(names.contains(&"spider_cloud_ai_browser"));
        assert!(names.contains(&"spider_cloud_ai_links"));
    }

    #[test]
    fn test_spider_cloud_registry_registration() {
        let registry = CustomToolRegistry::new();
        let cfg = SpiderCloudToolConfig::new("sk_spider_cloud")
            .with_unblocker(true)
            .with_transform(true)
            .with_enable_ai_routes(false);
        let count = registry.register_spider_cloud(&cfg);

        assert_eq!(count, 6);
        assert!(registry.contains("spider_cloud_crawl"));
        assert!(registry.contains("spider_cloud_transform"));
        assert!(registry.contains("spider_cloud_unblocker"));
        assert!(!registry.contains("spider_cloud_ai_scrape"));
    }

    #[test]
    fn test_spider_cloud_bearer_auth_opt_in() {
        let cfg = SpiderCloudToolConfig::new("sk_spider_cloud").with_bearer_auth(true);
        let tools = cfg.to_custom_tools();
        let crawl = tools
            .iter()
            .find(|t| t.name == "spider_cloud_crawl")
            .expect("crawl tool");
        assert!(matches!(crawl.auth, AuthConfig::Bearer(ref t) if t == "sk_spider_cloud"));
    }

    #[test]
    fn test_spider_cloud_strips_bearer_prefix_in_default_mode() {
        let cfg = SpiderCloudToolConfig::new("Bearer sk_spider_cloud");
        let tools = cfg.to_custom_tools();
        let crawl = tools
            .iter()
            .find(|t| t.name == "spider_cloud_crawl")
            .expect("crawl tool");
        assert!(matches!(
            crawl.auth,
            AuthConfig::ApiKey {
                ref header,
                ref key
            } if header == "Authorization" && key == "sk_spider_cloud"
        ));
    }

    #[test]
    fn test_spider_cloud_bearer_opt_in_avoids_double_prefix() {
        let cfg = SpiderCloudToolConfig::new("Bearer sk_spider_cloud").with_bearer_auth(true);
        let tools = cfg.to_custom_tools();
        let crawl = tools
            .iter()
            .find(|t| t.name == "spider_cloud_crawl")
            .expect("crawl tool");
        assert!(matches!(crawl.auth, AuthConfig::Bearer(ref t) if t == "sk_spider_cloud"));
    }

    #[test]
    fn test_spider_cloud_custom_prefix_and_api_url() {
        let cfg = SpiderCloudToolConfig::new("sk_spider_cloud")
            .with_api_url("https://custom.provider.local/v1")
            .with_tool_name_prefix("web_api")
            .with_enable_ai_routes(false);
        let tools = cfg.to_custom_tools();

        let transform = tools
            .iter()
            .find(|t| t.name == "web_api_transform")
            .expect("transform tool with custom prefix");
        assert_eq!(
            transform.base_url,
            "https://custom.provider.local/v1/transform"
        );
    }

    #[test]
    fn test_spider_cloud_empty_prefix_uses_plain_names() {
        let cfg = SpiderCloudToolConfig::new("sk_spider_cloud").with_tool_name_prefix("");
        let tools = cfg.to_custom_tools();
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();

        assert!(names.contains(&"crawl"));
        assert!(names.contains(&"search"));
        assert!(names.contains(&"transform"));
    }

    // ─── Spider Browser Cloud Tests ──────────────────────────────────────

    #[test]
    fn test_spider_browser_config_defaults() {
        let cfg = SpiderBrowserToolConfig::new("test-key");
        assert_eq!(cfg.api_key, "test-key");
        assert_eq!(cfg.wss_url, "wss://browser.spider.cloud/v1/browser");
        assert!(!cfg.stealth);
        assert!(cfg.browser.is_none());
        assert!(cfg.country.is_none());
        assert_eq!(cfg.timeout_secs, 60);
        assert!(cfg.include_navigate);
        assert!(cfg.include_html);
        assert!(cfg.include_screenshot);
        assert!(cfg.include_evaluate);
        assert!(cfg.include_click);
        assert!(cfg.include_fill);
        assert!(cfg.include_wait);
    }

    #[test]
    fn test_spider_browser_connection_url_basic() {
        let cfg = SpiderBrowserToolConfig::new("sk-abc");
        assert_eq!(
            cfg.connection_url(),
            "wss://browser.spider.cloud/v1/browser?token=sk-abc"
        );
    }

    #[test]
    fn test_spider_browser_connection_url_with_options() {
        let cfg = SpiderBrowserToolConfig::new("key")
            .with_stealth(true)
            .with_browser("chrome")
            .with_country("gb");
        assert_eq!(
            cfg.connection_url(),
            "wss://browser.spider.cloud/v1/browser?token=key&stealth=true&browser=chrome&country=gb"
        );
    }

    #[test]
    fn test_spider_browser_custom_wss_url() {
        let cfg =
            SpiderBrowserToolConfig::new("key").with_wss_url("wss://custom.example.com/browser");
        assert_eq!(
            cfg.connection_url(),
            "wss://custom.example.com/browser?token=key"
        );
    }

    #[test]
    fn test_spider_browser_to_custom_tools_count() {
        let cfg = SpiderBrowserToolConfig::new("key");
        let tools = cfg.to_custom_tools();
        assert_eq!(tools.len(), 7); // navigate, html, screenshot, evaluate, click, fill, wait
    }

    #[test]
    fn test_spider_browser_to_custom_tools_names() {
        let cfg = SpiderBrowserToolConfig::new("key");
        let tools = cfg.to_custom_tools();
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"spider_browser_navigate"));
        assert!(names.contains(&"spider_browser_html"));
        assert!(names.contains(&"spider_browser_screenshot"));
        assert!(names.contains(&"spider_browser_evaluate"));
        assert!(names.contains(&"spider_browser_click"));
        assert!(names.contains(&"spider_browser_fill"));
        assert!(names.contains(&"spider_browser_wait"));
    }

    #[test]
    fn test_spider_browser_custom_prefix() {
        let cfg = SpiderBrowserToolConfig::new("key").with_tool_name_prefix("remote_browser");
        let tools = cfg.to_custom_tools();
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"remote_browser_navigate"));
        assert!(names.contains(&"remote_browser_html"));
    }

    #[test]
    fn test_spider_browser_tools_use_wss_base_url() {
        let cfg = SpiderBrowserToolConfig::new("my-key").with_stealth(true);
        let tools = cfg.to_custom_tools();
        for tool in &tools {
            assert!(tool
                .base_url
                .starts_with("wss://browser.spider.cloud/v1/browser?token=my-key"));
            assert!(tool.base_url.contains("stealth=true"));
            assert_eq!(tool.method, HttpMethod::Post);
        }
    }

    #[test]
    fn test_spider_browser_registry_register() {
        let registry = CustomToolRegistry::new();
        let cfg = SpiderBrowserToolConfig::new("key");
        let count = registry.register_spider_browser(&cfg);
        assert_eq!(count, 7);
        assert!(registry.contains("spider_browser_navigate"));
        assert!(registry.contains("spider_browser_html"));
    }
}
