use std::time::Duration;

/// The supported WebDriver browser types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, strum::EnumString, strum::Display)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum WebDriverBrowser {
    #[default]
    #[cfg_attr(feature = "serde", serde(rename = "chrome"))]
    /// Google Chrome browser.
    Chrome,
    #[cfg_attr(feature = "serde", serde(rename = "firefox"))]
    /// Mozilla Firefox browser.
    Firefox,
    #[cfg_attr(feature = "serde", serde(rename = "edge"))]
    /// Microsoft Edge browser.
    Edge,
}

/// Configuration for WebDriver connections.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct WebDriverConfig {
    /// The WebDriver server URL (e.g., "http://localhost:4444").
    pub server_url: String,
    /// The browser to use for WebDriver sessions.
    pub browser: WebDriverBrowser,
    /// Run the browser in headless mode.
    pub headless: bool,
    /// Custom browser arguments.
    pub browser_args: Option<Vec<String>>,
    /// Request timeout for WebDriver commands.
    pub timeout: Option<Duration>,
    /// Proxy server URL.
    pub proxy: Option<String>,
    /// User agent string to use.
    pub user_agent: Option<String>,
    /// Viewport width.
    pub viewport_width: Option<u32>,
    /// Viewport height.
    pub viewport_height: Option<u32>,
    /// Accept insecure certificates.
    pub accept_insecure_certs: bool,
    /// Page load strategy (normal, eager, none).
    pub page_load_strategy: Option<String>,
}

impl Default for WebDriverConfig {
    fn default() -> Self {
        Self {
            server_url: "http://localhost:4444".to_string(),
            browser: WebDriverBrowser::Chrome,
            headless: true,
            browser_args: None,
            timeout: Some(Duration::from_secs(60)),
            proxy: None,
            user_agent: None,
            viewport_width: None,
            viewport_height: None,
            accept_insecure_certs: false,
            page_load_strategy: None,
        }
    }
}

impl WebDriverConfig {
    /// Create a new WebDriverConfig with default values.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the WebDriver server URL.
    pub fn with_server_url(mut self, server_url: impl Into<String>) -> Self {
        self.server_url = server_url.into();
        self
    }

    /// Set the browser type.
    pub fn with_browser(mut self, browser: WebDriverBrowser) -> Self {
        self.browser = browser;
        self
    }

    /// Set whether to run in headless mode.
    pub fn with_headless(mut self, headless: bool) -> Self {
        self.headless = headless;
        self
    }

    /// Set custom browser arguments.
    pub fn with_browser_args(mut self, args: Vec<String>) -> Self {
        self.browser_args = Some(args);
        self
    }

    /// Set the request timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Set the proxy server URL.
    pub fn with_proxy(mut self, proxy: impl Into<String>) -> Self {
        self.proxy = Some(proxy.into());
        self
    }

    /// Set the user agent string.
    pub fn with_user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.user_agent = Some(user_agent.into());
        self
    }

    /// Set the viewport dimensions.
    pub fn with_viewport(mut self, width: u32, height: u32) -> Self {
        self.viewport_width = Some(width);
        self.viewport_height = Some(height);
        self
    }

    /// Set whether to accept insecure certificates.
    pub fn with_accept_insecure_certs(mut self, accept: bool) -> Self {
        self.accept_insecure_certs = accept;
        self
    }

    /// Set the page load strategy.
    pub fn with_page_load_strategy(mut self, strategy: impl Into<String>) -> Self {
        self.page_load_strategy = Some(strategy.into());
        self
    }

    /// Build the configuration.
    pub fn build(self) -> Self {
        self
    }
}

/// WebDriver intercept configuration (limited compared to CDP).
#[derive(Debug, Clone, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct WebDriverInterceptConfiguration {
    /// Enable interception (limited in WebDriver).
    pub enabled: bool,
}

impl WebDriverInterceptConfiguration {
    /// Create a new intercept configuration.
    pub fn new(enabled: bool) -> Self {
        Self { enabled }
    }
}
