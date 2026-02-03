//! Browser automation module for spider_agent.
//!
//! Provides Chrome page management with support for:
//! - Page cloning
//! - Opening new pages/tabs
//! - Screenshot capture
//! - Navigation and interaction

use std::sync::Arc;

// Re-export chromey types
pub use chromiumoxide::browser::Browser;
pub use chromiumoxide::page::Page;
pub use chromiumoxide::error::CdpError;

/// Browser context for managing Chrome pages.
///
/// Wraps a chromey Page with additional utilities for agent operations.
#[derive(Clone)]
pub struct BrowserContext {
    /// The browser instance.
    browser: Arc<Browser>,
    /// The current page.
    page: Arc<Page>,
}

impl BrowserContext {
    /// Create a new browser context from an existing browser and page.
    pub fn new(browser: Arc<Browser>, page: Arc<Page>) -> Self {
        Self { browser, page }
    }

    /// Get the current page.
    pub fn page(&self) -> &Arc<Page> {
        &self.page
    }

    /// Get the browser instance.
    pub fn browser(&self) -> &Arc<Browser> {
        &self.browser
    }

    /// Open a new page/tab in the browser.
    pub async fn new_page(&self) -> Result<Arc<Page>, CdpError> {
        let page = self.browser.new_page("about:blank").await?;
        Ok(Arc::new(page))
    }

    /// Open a new page and navigate to URL.
    pub async fn new_page_with_url(&self, url: &str) -> Result<Arc<Page>, CdpError> {
        let page = self.browser.new_page(url).await?;
        Ok(Arc::new(page))
    }

    /// Clone the current page context (opens a new page with same URL).
    pub async fn clone_page(&self) -> Result<BrowserContext, CdpError> {
        let url = self.page.url().await?.unwrap_or_else(|| "about:blank".to_string());
        let new_page = self.browser.new_page(&url).await?;
        Ok(BrowserContext {
            browser: self.browser.clone(),
            page: Arc::new(new_page),
        })
    }

    /// Navigate to a URL.
    pub async fn navigate(&self, url: &str) -> Result<(), CdpError> {
        self.page.goto(url).await?;
        Ok(())
    }

    /// Get the current URL.
    pub async fn url(&self) -> Result<Option<String>, CdpError> {
        self.page.url().await
    }

    /// Get the page HTML content.
    pub async fn html(&self) -> Result<String, CdpError> {
        self.page.content().await
    }

    /// Take a screenshot and return PNG bytes.
    pub async fn screenshot(&self) -> Result<Vec<u8>, CdpError> {
        self.page.screenshot(
            chromiumoxide::page::ScreenshotParams::builder()
                .full_page(true)
                .build()
        ).await
    }

    /// Take a screenshot of the visible viewport.
    pub async fn screenshot_viewport(&self) -> Result<Vec<u8>, CdpError> {
        self.page.screenshot(
            chromiumoxide::page::ScreenshotParams::builder()
                .full_page(false)
                .build()
        ).await
    }

    /// Click an element by selector.
    pub async fn click(&self, selector: &str) -> Result<(), CdpError> {
        let element = self.page.find_element(selector).await?;
        element.click().await?;
        Ok(())
    }

    /// Type text into an element.
    pub async fn type_text(&self, selector: &str, text: &str) -> Result<(), CdpError> {
        let element = self.page.find_element(selector).await?;
        element.click().await?;
        element.type_str(text).await?;
        Ok(())
    }

    /// Wait for a selector to appear.
    pub async fn wait_for(&self, selector: &str) -> Result<(), CdpError> {
        self.page.find_element(selector).await?;
        Ok(())
    }

    /// Evaluate JavaScript and return the result.
    pub async fn evaluate<T: serde::de::DeserializeOwned>(&self, script: &str) -> Result<T, CdpError> {
        self.page.evaluate(script).await?.into_value()
            .map_err(|e| CdpError::ChromeMessage(format!("JSON conversion error: {}", e)))
    }

    /// Execute JavaScript without returning a value.
    pub async fn execute(&self, script: &str) -> Result<(), CdpError> {
        self.page.evaluate(script).await?;
        Ok(())
    }

    /// Close the current page.
    /// Note: This clones the page internally since close() takes ownership.
    pub async fn close(&self) -> Result<(), CdpError> {
        // Chrome pages need to be explicitly closed via the browser
        // Since Page::close takes ownership, we use evaluate to close
        self.page.evaluate("window.close()").await?;
        Ok(())
    }

    /// Set the current page (switch to a different page).
    pub fn set_page(&mut self, page: Arc<Page>) {
        self.page = page;
    }

    /// Create a new context with a different page (immutable version).
    pub fn with_page(&self, page: Arc<Page>) -> Self {
        Self {
            browser: self.browser.clone(),
            page,
        }
    }
}

impl std::fmt::Debug for BrowserContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrowserContext")
            .field("browser", &"Browser { ... }")
            .field("page", &"Page { ... }")
            .finish()
    }
}
