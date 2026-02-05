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

    /// Click all elements matching a selector.
    /// Returns the number of elements clicked.
    pub async fn click_all(&self, selector: &str) -> Result<usize, CdpError> {
        let elements = self.page.find_elements(selector).await?;
        let count = elements.len();
        for element in elements {
            let _ = element.click().await;
        }
        Ok(count)
    }

    /// Click at specific x,y coordinates with smooth human-like mouse movement.
    pub async fn click_point(&self, x: f64, y: f64) -> Result<(), CdpError> {
        use chromiumoxide::layout::Point;
        self.page.click_smooth(Point::new(x, y)).await?;
        Ok(())
    }

    /// Click and hold on an element with smooth mouse movement.
    pub async fn click_hold(&self, selector: &str, hold_ms: u64) -> Result<(), CdpError> {
        let element = self.page.find_element(selector).await?;
        let point = element.clickable_point().await?;
        self.page.move_mouse_smooth(point).await?;
        self.page.click_and_hold(point, std::time::Duration::from_millis(hold_ms)).await?;
        Ok(())
    }

    /// Click and hold at a specific point with smooth mouse movement.
    pub async fn click_hold_point(&self, x: f64, y: f64, hold_ms: u64) -> Result<(), CdpError> {
        use chromiumoxide::layout::Point;
        let point = Point::new(x, y);
        self.page.move_mouse_smooth(point).await?;
        self.page.click_and_hold(point, std::time::Duration::from_millis(hold_ms)).await?;
        Ok(())
    }

    /// Click and drag from one element to another.
    pub async fn click_drag(&self, from_selector: &str, to_selector: &str, modifier: Option<i64>) -> Result<(), CdpError> {
        let from_elem = self.page.find_element(from_selector).await?;
        let to_elem = self.page.find_element(to_selector).await?;

        let from_point = from_elem.clickable_point().await?;
        let to_point = to_elem.clickable_point().await?;

        self.click_drag_point((from_point.x, from_point.y), (to_point.x, to_point.y), modifier).await
    }

    /// Click and drag from one point to another with smooth bezier movement.
    pub async fn click_drag_point(&self, from: (f64, f64), to: (f64, f64), modifier: Option<i64>) -> Result<(), CdpError> {
        use chromiumoxide::layout::Point;
        let from_point = Point::new(from.0, from.1);
        let to_point = Point::new(to.0, to.1);
        match modifier {
            Some(m) => self.page.click_and_drag_smooth_with_modifier(from_point, to_point, m).await?,
            None => self.page.click_and_drag_smooth(from_point, to_point).await?,
        };
        Ok(())
    }

    /// Click all clickable elements on the page.
    pub async fn click_all_clickable(&self) -> Result<usize, CdpError> {
        // Find common clickable elements
        let script = r#"
            Array.from(document.querySelectorAll('a, button, [onclick], [role="button"], input[type="submit"], input[type="button"]'))
                .filter(el => {
                    const style = window.getComputedStyle(el);
                    return style.display !== 'none' && style.visibility !== 'hidden' && el.offsetParent !== null;
                })
                .length
        "#;

        let count: usize = self.page.evaluate(script).await?.into_value()
            .map_err(|e| CdpError::ChromeMessage(format!("Failed to count clickable elements: {}", e)))?;

        // Click each one (with error handling)
        let click_script = r#"
            const elements = Array.from(document.querySelectorAll('a, button, [onclick], [role="button"], input[type="submit"], input[type="button"]'))
                .filter(el => {
                    const style = window.getComputedStyle(el);
                    return style.display !== 'none' && style.visibility !== 'hidden' && el.offsetParent !== null;
                });
            elements.forEach(el => { try { el.click(); } catch(e) {} });
            elements.length
        "#;

        let clicked: usize = self.page.evaluate(click_script).await?.into_value()
            .unwrap_or(0);

        Ok(clicked.min(count))
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

    /// Wait for a selector with timeout.
    pub async fn wait_for_timeout(&self, selector: &str, timeout_ms: u64) -> Result<(), CdpError> {
        let timeout = std::time::Duration::from_millis(timeout_ms);
        tokio::time::timeout(timeout, self.page.find_element(selector))
            .await
            .map_err(|_| CdpError::Timeout)?
            .map(|_| ())
    }

    /// Wait for navigation to complete.
    pub async fn wait_for_navigation(&self) -> Result<(), CdpError> {
        // Wait for load event
        self.page.evaluate("new Promise(r => { if (document.readyState === 'complete') r(); else window.addEventListener('load', r); })").await?;
        Ok(())
    }

    /// Wait for DOM to stabilize (no mutations for a period).
    pub async fn wait_for_dom(&self, selector: Option<&str>, timeout_ms: u32) -> Result<(), CdpError> {
        let sel = selector.unwrap_or("body");
        let script = format!(r#"
            new Promise((resolve, reject) => {{
                const timeout = {};
                const target = document.querySelector('{}');
                if (!target) {{ resolve(); return; }}

                let timer;
                const observer = new MutationObserver(() => {{
                    clearTimeout(timer);
                    timer = setTimeout(() => {{
                        observer.disconnect();
                        resolve();
                    }}, 100);
                }});

                observer.observe(target, {{ childList: true, subtree: true, attributes: true }});

                timer = setTimeout(() => {{
                    observer.disconnect();
                    resolve();
                }}, 100);

                setTimeout(() => {{
                    observer.disconnect();
                    resolve();
                }}, timeout);
            }})
        "#, timeout_ms, sel);

        self.page.evaluate(script).await?;
        Ok(())
    }

    /// Wait for element then click it.
    pub async fn wait_and_click(&self, selector: &str) -> Result<(), CdpError> {
        let element = self.page.find_element(selector).await?;
        element.click().await?;
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

    /// Scroll horizontally by pixels.
    pub async fn scroll_x(&self, pixels: i32) -> Result<(), CdpError> {
        let script = format!("window.scrollBy({}, 0)", pixels);
        self.page.evaluate(script).await?;
        Ok(())
    }

    /// Scroll vertically by pixels.
    pub async fn scroll_y(&self, pixels: i32) -> Result<(), CdpError> {
        let script = format!("window.scrollBy(0, {})", pixels);
        self.page.evaluate(script).await?;
        Ok(())
    }

    /// Infinite scroll - scroll to the bottom of the page repeatedly.
    /// Returns when no new content is loaded after scrolling.
    pub async fn infinite_scroll(&self, max_scrolls: u32) -> Result<usize, CdpError> {
        let script = r#"
            (async function() {
                const maxScrolls = arguments[0];
                let lastHeight = document.body.scrollHeight;
                let scrollCount = 0;

                while (scrollCount < maxScrolls) {
                    window.scrollTo(0, document.body.scrollHeight);
                    await new Promise(r => setTimeout(r, 1000));
                    const newHeight = document.body.scrollHeight;
                    if (newHeight === lastHeight) break;
                    lastHeight = newHeight;
                    scrollCount++;
                }
                return scrollCount;
            })
        "#;

        let count: usize = self.page
            .evaluate(format!("({script})({max_scrolls})"))
            .await?
            .into_value()
            .unwrap_or(0);

        Ok(count)
    }

    /// Fill an input element with a value (clears existing content first).
    pub async fn fill(&self, selector: &str, value: &str) -> Result<(), CdpError> {
        let element = self.page.find_element(selector).await?;

        // Clear existing value via triple-click + delete
        element.click().await?;
        element.click().await?;
        element.click().await?;

        // Clear with keyboard
        use chromiumoxide::cdp::browser_protocol::input::{DispatchKeyEventParams, DispatchKeyEventType};
        self.page.execute(
            DispatchKeyEventParams::builder()
                .r#type(DispatchKeyEventType::KeyDown)
                .key("a")
                .modifiers(2) // Ctrl/Cmd
                .build()
                .unwrap()
        ).await?;
        self.page.execute(
            DispatchKeyEventParams::builder()
                .r#type(DispatchKeyEventType::KeyUp)
                .key("a")
                .build()
                .unwrap()
        ).await?;

        // Type new value
        element.type_str(value).await?;
        Ok(())
    }

    /// Find all elements matching a selector.
    pub async fn find_elements(&self, selector: &str) -> Result<Vec<chromiumoxide::element::Element>, CdpError> {
        self.page.find_elements(selector).await
    }

    /// Get element bounding box via JavaScript.
    pub async fn get_element_bounds(&self, selector: &str) -> Result<Option<(f64, f64, f64, f64)>, CdpError> {
        let script = format!(
            r#"
            (function() {{
                const el = document.querySelector('{}');
                if (!el) return null;
                const rect = el.getBoundingClientRect();
                return [rect.x, rect.y, rect.width, rect.height];
            }})()
            "#,
            selector.replace('\'', "\\'")
        );

        let result: Option<Vec<f64>> = self.page.evaluate(script).await?.into_value()
            .map_err(|e| CdpError::ChromeMessage(format!("Failed to get bounds: {}", e)))?;

        Ok(result.and_then(|v| {
            if v.len() >= 4 {
                Some((v[0], v[1], v[2], v[3]))
            } else {
                None
            }
        }))
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
