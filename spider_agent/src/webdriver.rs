//! WebDriver automation module for spider_agent.
//!
//! Provides WebDriver session management with support for:
//! - Multiple browser backends (Chrome, Firefox, Edge)
//! - Page navigation and interaction
//! - Screenshot capture
//! - JavaScript execution

use std::sync::Arc;
use thirtyfour::prelude::*;
use thirtyfour::error::WebDriverError;

/// Result type for WebDriver operations.
pub type WebDriverResult<T> = Result<T, WebDriverError>;

/// WebDriver context for browser automation.
///
/// Wraps a thirtyfour WebDriver session with utilities for agent operations.
#[derive(Clone)]
pub struct WebDriverContext {
    /// The WebDriver session.
    driver: Arc<WebDriver>,
}

impl WebDriverContext {
    /// Create a new WebDriver context from an existing WebDriver.
    pub fn new(driver: Arc<WebDriver>) -> Self {
        Self { driver }
    }

    /// Create from a WebDriver (wraps in Arc).
    pub fn from_driver(driver: WebDriver) -> Self {
        Self {
            driver: Arc::new(driver),
        }
    }

    /// Get the WebDriver instance.
    pub fn driver(&self) -> &Arc<WebDriver> {
        &self.driver
    }

    /// Navigate to a URL.
    pub async fn navigate(&self, url: &str) -> WebDriverResult<()> {
        self.driver.goto(url).await
    }

    /// Get the current URL.
    pub async fn url(&self) -> WebDriverResult<String> {
        Ok(self.driver.current_url().await?.to_string())
    }

    /// Get the page HTML content.
    pub async fn html(&self) -> WebDriverResult<String> {
        self.driver.source().await
    }

    /// Get the page title.
    pub async fn title(&self) -> WebDriverResult<String> {
        self.driver.title().await
    }

    /// Take a screenshot and return PNG bytes.
    pub async fn screenshot(&self) -> WebDriverResult<Vec<u8>> {
        self.driver.screenshot_as_png().await
    }

    /// Find an element by CSS selector.
    pub async fn find_element(&self, selector: &str) -> WebDriverResult<WebElement> {
        self.driver.find(By::Css(selector)).await
    }

    /// Find all elements matching a CSS selector.
    pub async fn find_elements(&self, selector: &str) -> WebDriverResult<Vec<WebElement>> {
        self.driver.find_all(By::Css(selector)).await
    }

    /// Click an element by CSS selector.
    pub async fn click(&self, selector: &str) -> WebDriverResult<()> {
        let element = self.find_element(selector).await?;
        element.click().await
    }

    /// Type text into an element.
    pub async fn type_text(&self, selector: &str, text: &str) -> WebDriverResult<()> {
        let element = self.find_element(selector).await?;
        element.send_keys(text).await
    }

    /// Clear an element's value.
    pub async fn clear(&self, selector: &str) -> WebDriverResult<()> {
        let element = self.find_element(selector).await?;
        element.clear().await
    }

    /// Get an element's text content.
    pub async fn get_text(&self, selector: &str) -> WebDriverResult<String> {
        let element = self.find_element(selector).await?;
        element.text().await
    }

    /// Get an element's attribute value.
    pub async fn get_attribute(&self, selector: &str, attr: &str) -> WebDriverResult<Option<String>> {
        let element = self.find_element(selector).await?;
        element.attr(attr).await
    }

    /// Execute JavaScript and return the result as JSON.
    pub async fn execute<T: serde::de::DeserializeOwned>(&self, script: &str) -> WebDriverResult<T> {
        let ret = self.driver.execute(script, vec![]).await?;
        ret.convert()
    }

    /// Execute JavaScript without returning a value.
    pub async fn execute_script(&self, script: &str) -> WebDriverResult<()> {
        self.driver.execute(script, vec![]).await?;
        Ok(())
    }

    /// Wait for an element to be present.
    pub async fn wait_for(&self, selector: &str, timeout_secs: u64) -> WebDriverResult<WebElement> {
        let elem = self
            .driver
            .query(By::Css(selector))
            .wait(std::time::Duration::from_secs(timeout_secs), std::time::Duration::from_millis(100))
            .first()
            .await?;
        Ok(elem)
    }

    /// Wait for element to be clickable.
    pub async fn wait_for_clickable(&self, selector: &str, timeout_secs: u64) -> WebDriverResult<WebElement> {
        let elem = self
            .driver
            .query(By::Css(selector))
            .wait(std::time::Duration::from_secs(timeout_secs), std::time::Duration::from_millis(100))
            .and_clickable()
            .first()
            .await?;
        Ok(elem)
    }

    /// Open a new tab/window.
    pub async fn new_tab(&self) -> WebDriverResult<WindowHandle> {
        self.driver.new_tab().await
    }

    /// Open a new window.
    pub async fn new_window(&self) -> WebDriverResult<WindowHandle> {
        self.driver.new_window().await
    }

    /// Switch to a window by handle.
    pub async fn switch_to_window(&self, handle: &WindowHandle) -> WebDriverResult<()> {
        self.driver.switch_to_window(handle.clone()).await
    }

    /// Get all window handles.
    pub async fn windows(&self) -> WebDriverResult<Vec<WindowHandle>> {
        self.driver.windows().await
    }

    /// Get current window handle.
    pub async fn current_window(&self) -> WebDriverResult<WindowHandle> {
        self.driver.window().await
    }

    /// Close the current window.
    pub async fn close_window(&self) -> WebDriverResult<()> {
        self.driver.close_window().await
    }

    /// Refresh the page.
    pub async fn refresh(&self) -> WebDriverResult<()> {
        self.driver.refresh().await
    }

    /// Go back in history.
    pub async fn back(&self) -> WebDriverResult<()> {
        self.driver.back().await
    }

    /// Go forward in history.
    pub async fn forward(&self) -> WebDriverResult<()> {
        self.driver.forward().await
    }

    /// Set window size.
    pub async fn set_window_size(&self, width: u32, height: u32) -> WebDriverResult<()> {
        self.driver.set_window_rect(0, 0, width, height).await?;
        Ok(())
    }

    /// Maximize window.
    pub async fn maximize_window(&self) -> WebDriverResult<()> {
        self.driver.maximize_window().await
    }

    /// Minimize window.
    pub async fn minimize_window(&self) -> WebDriverResult<()> {
        self.driver.minimize_window().await
    }

    /// Get all cookies.
    pub async fn cookies(&self) -> WebDriverResult<Vec<Cookie>> {
        self.driver.get_all_cookies().await
    }

    /// Delete a cookie by name.
    pub async fn delete_cookie(&self, name: &str) -> WebDriverResult<()> {
        self.driver.delete_cookie(name).await
    }

    /// Delete all cookies.
    pub async fn delete_all_cookies(&self) -> WebDriverResult<()> {
        self.driver.delete_all_cookies().await
    }
}

impl std::fmt::Debug for WebDriverContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebDriverContext")
            .field("driver", &"WebDriver { ... }")
            .finish()
    }
}

// Re-export useful types
pub use thirtyfour::{
    By, Cookie, WebDriver, WebElement, WindowHandle,
};
