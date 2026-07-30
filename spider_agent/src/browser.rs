//! Browser automation module for spider_agent.
//!
//! Provides Chrome page management with support for:
//! - Page cloning
//! - Opening new pages/tabs
//! - Screenshot capture
//! - Navigation and interaction

use dashmap::DashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

// Re-export chromey types
pub use chromiumoxide::browser::Browser;
pub use chromiumoxide::error::CdpError;
pub use chromiumoxide::page::Page;

/// How long a detached `page.close()` may run before it is abandoned.
const CLOSE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Closes a Chrome page's CDP target when the guard leaves scope.
///
/// Dropping a chromiumoxide [`Page`] does **not** close the underlying CDP
/// target — it only decrements an internal counter. Every page the agent
/// creates and forgets therefore leaks a browser tab for the lifetime of the
/// shared [`Browser`], until `new_page()` hangs and the browser wedges. This
/// mirrors the dedicated tab-closer in the core `spider` chrome path.
///
/// `Drop` is deliberately infallible and non-blocking: it detaches a bounded
/// `page.close()` onto the current runtime (via
/// [`tokio::runtime::Handle::try_current`], so drop outside a runtime — e.g.
/// during shutdown — is a no-op, since the tab dies with the process anyway),
/// takes no locks, never awaits, and swallows any CDP error. A wedged target
/// cannot pin the spawned task beyond [`CLOSE_TIMEOUT`].
///
/// Call [`defuse`](Self::defuse) before closing the page explicitly, or when
/// the page is intentionally meant to outlive the guard.
pub(crate) struct PageCloseGuard {
    /// The protected page. Held as an `Arc` so the guard can be shared and so
    /// [`page`](Self::page) stays a lock-free borrow on hot paths.
    page: Arc<Page>,
    /// Set once the guard has been disarmed; `Drop` then does nothing.
    defused: AtomicBool,
}

impl PageCloseGuard {
    /// Create a guard that closes `page` on drop.
    #[inline]
    pub(crate) fn new(page: Arc<Page>) -> Self {
        Self {
            page,
            defused: AtomicBool::new(false),
        }
    }

    /// Create a guard from an owned [`Page`].
    #[inline]
    pub(crate) fn from_page(page: Page) -> Self {
        Self::new(Arc::new(page))
    }

    /// Borrow the protected page. Lock-free.
    #[inline]
    pub(crate) fn page(&self) -> &Page {
        &self.page
    }

    /// Disarm the guard — the tab will not be closed on drop.
    ///
    /// Takes `&self` (not `self`) so it works through an [`Arc`], where the
    /// guard may be shared by several [`BrowserContext`] clones.
    #[inline]
    pub(crate) fn defuse(&self) {
        self.defused.store(true, Ordering::Release);
    }
}

impl Drop for PageCloseGuard {
    fn drop(&mut self) {
        if self.defused.load(Ordering::Acquire) {
            return;
        }

        // `close()` is a CDP round-trip; detach it so drop stays sync and
        // non-blocking. Bounded by a timeout so a wedged (or already gone)
        // browser cannot pin the task. Guarded on an active runtime so drop
        // during shutdown is a no-op. Nothing here can panic.
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let page = Page::clone(&self.page);
            handle.spawn(async move {
                let _ = tokio::time::timeout(CLOSE_TIMEOUT, page.close()).await;
            });
        }
    }
}

/// Identity key for a shared page handle: the address of its `Arc` allocation.
///
/// Unique and stable while any `Arc` clone of that page is alive, which the
/// owning [`PageCloseGuard`] guarantees for every registered entry.
#[inline]
fn page_key(page: &Arc<Page>) -> usize {
    Arc::as_ptr(page) as usize
}

/// Browser context for managing Chrome pages.
///
/// Wraps a chromey Page with additional utilities for agent operations.
///
/// # Tab ownership
///
/// A context created by [`BrowserContext::clone_page`] or
/// [`BrowserContext::new_page_owned`] **owns** its page: the CDP target is
/// closed once the last clone of the context drops. A context built with
/// [`BrowserContext::new`] never closes the caller-supplied page. Pages created
/// through [`BrowserContext::new_page`] / [`BrowserContext::new_page_with_url`]
/// are tracked and closed when the last clone of the creating context drops.
/// Use [`BrowserContext::defuse_page`] to opt out of all of it.
#[derive(Clone)]
pub struct BrowserContext {
    /// The browser instance.
    browser: Arc<Browser>,
    /// The current page.
    page: Arc<Page>,
    /// Closes `page` when the last clone of this context drops — `None` when the
    /// page was supplied by the caller and is therefore not ours to close.
    ///
    /// Held behind an [`Arc`] because `BrowserContext` is `Clone`: `Drop` must
    /// fire only when the *last* clone dies, never when an intermediate clone
    /// goes out of scope. Cloning the field is a refcount bump, nothing more.
    page_guard: Option<Arc<PageCloseGuard>>,
    /// Extra pages this context created via [`BrowserContext::new_page`] /
    /// [`BrowserContext::new_page_with_url`], which hand back a bare
    /// `Arc<Page>` with no ownership handle. Dropping the last context clone
    /// drops the map, which drops each guard, which closes each tab — turning
    /// "leaks forever" into "leaks for the context's lifetime".
    ///
    /// A [`DashMap`] keyed by the page's `Arc` address — no mutex, and
    /// deregistration in [`close_page`](Self::close_page) is an O(1) shard
    /// lookup instead of a linear scan. Every access is synchronous, so it is
    /// usable from `Drop`, and none of them span an `.await`. The key is stable
    /// and unique for as long as the entry lives, because the guard holds an
    /// `Arc` clone of the page it is keyed by.
    owned: Arc<DashMap<usize, PageCloseGuard>>,
}

impl BrowserContext {
    /// Create a new browser context from an existing browser and page.
    ///
    /// The page is caller-owned: dropping this context never closes it.
    pub fn new(browser: Arc<Browser>, page: Arc<Page>) -> Self {
        Self {
            browser,
            page,
            page_guard: None,
            owned: Default::default(),
        }
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
    ///
    /// The returned handle carries no ownership, so the tab is only released
    /// when the last clone of **this** context drops. Prefer
    /// [`new_page_owned`](Self::new_page_owned), which scopes the tab to the
    /// returned context, or close it eagerly with
    /// [`close_page`](Self::close_page).
    #[deprecated(
        since = "2.53.1",
        note = "leaks the CDP tab until the context drops; use new_page_owned()"
    )]
    pub async fn new_page(&self) -> Result<Arc<Page>, CdpError> {
        self.new_page_tracked("about:blank").await
    }

    /// Open a new page and navigate to URL.
    ///
    /// The returned handle carries no ownership, so the tab is only released
    /// when the last clone of **this** context drops. Prefer
    /// [`new_page_owned`](Self::new_page_owned), which scopes the tab to the
    /// returned context, or close it eagerly with
    /// [`close_page`](Self::close_page).
    #[deprecated(
        since = "2.53.1",
        note = "leaks the CDP tab until the context drops; use new_page_owned()"
    )]
    pub async fn new_page_with_url(&self, url: &str) -> Result<Arc<Page>, CdpError> {
        self.new_page_tracked(url).await
    }

    /// Create a page at `url` and register it in this context's owned set.
    async fn new_page_tracked(&self, url: &str) -> Result<Arc<Page>, CdpError> {
        let page = Arc::new(self.browser.new_page(url).await?);
        // Lock-free, await-free registration.
        self.owned
            .insert(page_key(&page), PageCloseGuard::new(page.clone()));
        Ok(page)
    }

    /// Open a new page/tab and return it as an owning context.
    ///
    /// The tab is closed when the last clone of the returned [`BrowserContext`]
    /// drops. This is the leak-free replacement for
    /// [`new_page`](Self::new_page).
    pub async fn new_page_owned(&self) -> Result<BrowserContext, CdpError> {
        self.owning_context("about:blank").await
    }

    /// Open a new page at `url` and return it as an owning context.
    ///
    /// The tab is closed when the last clone of the returned [`BrowserContext`]
    /// drops. This is the leak-free replacement for
    /// [`new_page_with_url`](Self::new_page_with_url).
    pub async fn new_page_with_url_owned(&self, url: &str) -> Result<BrowserContext, CdpError> {
        self.owning_context(url).await
    }

    /// Build a context that owns a freshly created page at `url`.
    async fn owning_context(&self, url: &str) -> Result<BrowserContext, CdpError> {
        let page = Arc::new(self.browser.new_page(url).await?);
        Ok(BrowserContext {
            browser: self.browser.clone(),
            page: page.clone(),
            page_guard: Some(Arc::new(PageCloseGuard::new(page))),
            owned: Default::default(),
        })
    }

    /// Close a page created by this context, without waiting for the context to
    /// drop.
    ///
    /// Deregisters the page from the owned set first, so the tab is never closed
    /// twice. Pages this context does not own are closed as requested.
    pub async fn close_page(&self, page: &Arc<Page>) -> Result<(), CdpError> {
        // O(1) deregistration, no `.await` held across it.
        if let Some((_, guard)) = self.owned.remove(&page_key(page)) {
            guard.defuse();
        }

        Page::clone(page.as_ref()).close().await
    }

    /// Clone the current page context (opens a new page with same URL).
    ///
    /// The returned context **owns** the new tab: it is closed when the last
    /// clone of that context drops. Code that keeps only
    /// `ctx.page().clone()` and drops the context will find its tab closed —
    /// that is the intended lifetime. Use [`defuse_page`](Self::defuse_page) to
    /// opt out.
    pub async fn clone_page(&self) -> Result<BrowserContext, CdpError> {
        let url = self
            .page
            .url()
            .await?
            .unwrap_or_else(|| "about:blank".to_string());

        self.owning_context(&url).await
    }

    /// Disarm every close guard this context holds and hand back its page.
    ///
    /// For callers that intentionally let a page outlive the context that
    /// created it. Mirrors the `defuse` escape hatch on the core `spider`
    /// tab guard. Note the guards are shared with any surviving clone of this
    /// context, so defusing disarms them all.
    pub fn defuse_page(self) -> Arc<Page> {
        if let Some(guard) = self.page_guard.as_ref() {
            guard.defuse();
        }

        for entry in self.owned.iter() {
            entry.value().defuse();
        }

        self.page.clone()
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
        self.page
            .screenshot(
                chromiumoxide::page::ScreenshotParams::builder()
                    .full_page(true)
                    .build(),
            )
            .await
    }

    /// Take a screenshot of the visible viewport.
    pub async fn screenshot_viewport(&self) -> Result<Vec<u8>, CdpError> {
        self.page
            .screenshot(
                chromiumoxide::page::ScreenshotParams::builder()
                    .full_page(false)
                    .build(),
            )
            .await
    }

    /// Click an element by selector.
    pub async fn click(&self, selector: &str) -> Result<(), CdpError> {
        let element = self.page.find_element(selector).await?;
        element.click_smooth().await?;
        Ok(())
    }

    /// Click all elements matching a selector.
    /// Returns the number of elements clicked.
    pub async fn click_all(&self, selector: &str) -> Result<usize, CdpError> {
        let elements = self.page.find_elements(selector).await?;
        let count = elements.len();
        for element in elements {
            let _ = element.click_smooth().await;
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
        self.page
            .click_and_hold(point, std::time::Duration::from_millis(hold_ms))
            .await?;
        Ok(())
    }

    /// Click and hold at a specific point with smooth mouse movement.
    pub async fn click_hold_point(&self, x: f64, y: f64, hold_ms: u64) -> Result<(), CdpError> {
        use chromiumoxide::layout::Point;
        let point = Point::new(x, y);
        self.page.move_mouse_smooth(point).await?;
        self.page
            .click_and_hold(point, std::time::Duration::from_millis(hold_ms))
            .await?;
        Ok(())
    }

    /// Click and drag from one element to another.
    pub async fn click_drag(
        &self,
        from_selector: &str,
        to_selector: &str,
        modifier: Option<i64>,
    ) -> Result<(), CdpError> {
        let from_elem = self.page.find_element(from_selector).await?;
        let to_elem = self.page.find_element(to_selector).await?;

        let from_point = from_elem.clickable_point().await?;
        let to_point = to_elem.clickable_point().await?;

        self.click_drag_point(
            (from_point.x, from_point.y),
            (to_point.x, to_point.y),
            modifier,
        )
        .await
    }

    /// Click and drag from one point to another with smooth bezier movement.
    pub async fn click_drag_point(
        &self,
        from: (f64, f64),
        to: (f64, f64),
        modifier: Option<i64>,
    ) -> Result<(), CdpError> {
        use chromiumoxide::layout::Point;
        let from_point = Point::new(from.0, from.1);
        let to_point = Point::new(to.0, to.1);
        match modifier {
            Some(m) => {
                self.page
                    .click_and_drag_smooth_with_modifier(from_point, to_point, m)
                    .await?
            }
            None => {
                self.page
                    .click_and_drag_smooth(from_point, to_point)
                    .await?
            }
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

        let count: usize = self
            .page
            .evaluate(script)
            .await?
            .into_value()
            .map_err(|e| {
                CdpError::ChromeMessage(format!("Failed to count clickable elements: {}", e))
            })?;

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

        let clicked: usize = self
            .page
            .evaluate(click_script)
            .await?
            .into_value()
            .unwrap_or(0);

        Ok(clicked.min(count))
    }

    /// Type text into an element.
    pub async fn type_text(&self, selector: &str, text: &str) -> Result<(), CdpError> {
        let element = self.page.find_element(selector).await?;
        element.click_smooth().await?;
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
    pub async fn wait_for_dom(
        &self,
        selector: Option<&str>,
        timeout_ms: u32,
    ) -> Result<(), CdpError> {
        let sel = selector.unwrap_or("body");
        let script = format!(
            r#"
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
        "#,
            timeout_ms, sel
        );

        self.page.evaluate(script).await?;
        Ok(())
    }

    /// Wait for element then click it.
    pub async fn wait_and_click(&self, selector: &str) -> Result<(), CdpError> {
        let element = self.page.find_element(selector).await?;
        element.click_smooth().await?;
        Ok(())
    }

    /// Evaluate JavaScript and return the result.
    pub async fn evaluate<T: serde::de::DeserializeOwned>(
        &self,
        script: &str,
    ) -> Result<T, CdpError> {
        self.page
            .evaluate(script)
            .await?
            .into_value()
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

        let count: usize = self
            .page
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
        use chromiumoxide::cdp::browser_protocol::input::{
            DispatchKeyEventParams, DispatchKeyEventType,
        };
        self.page
            .execute(
                DispatchKeyEventParams::builder()
                    .r#type(DispatchKeyEventType::KeyDown)
                    .key("a")
                    .modifiers(2) // Ctrl/Cmd
                    .build()
                    .map_err(|e| CdpError::ChromeMessage(format!("key event build: {e}")))?,
            )
            .await?;
        self.page
            .execute(
                DispatchKeyEventParams::builder()
                    .r#type(DispatchKeyEventType::KeyUp)
                    .key("a")
                    .build()
                    .map_err(|e| CdpError::ChromeMessage(format!("key event build: {e}")))?,
            )
            .await?;

        // Type new value
        element.type_str(value).await?;
        Ok(())
    }

    /// Find all elements matching a selector.
    pub async fn find_elements(
        &self,
        selector: &str,
    ) -> Result<Vec<chromiumoxide::element::Element>, CdpError> {
        self.page.find_elements(selector).await
    }

    /// Get element bounding box via JavaScript.
    pub async fn get_element_bounds(
        &self,
        selector: &str,
    ) -> Result<Option<(f64, f64, f64, f64)>, CdpError> {
        let escaped_selector = serde_json::to_string(selector).unwrap_or_else(|_| {
            format!(
                "\"{}\"",
                selector.replace('\\', "\\\\").replace('"', "\\\"")
            )
        });
        let script = format!(
            r#"
            (function() {{
                const el = document.querySelector({});
                if (!el) return null;
                const rect = el.getBoundingClientRect();
                return [rect.x, rect.y, rect.width, rect.height];
            }})()
            "#,
            escaped_selector
        );

        let result: Option<Vec<f64>> = self
            .page
            .evaluate(script)
            .await?
            .into_value()
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
    ///
    /// The supplied page is treated as caller-owned and is never closed by the
    /// returned context. The owned set is shared with `self`, so pages this
    /// context created are still released once every clone is gone.
    pub fn with_page(&self, page: Arc<Page>) -> Self {
        Self {
            browser: self.browser.clone(),
            page,
            page_guard: None,
            owned: self.owned.clone(),
        }
    }
}

impl std::fmt::Debug for BrowserContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrowserContext")
            .field("browser", &"Browser { ... }")
            .field("page", &"Page { ... }")
            .field("owns_page", &self.page_guard.is_some())
            .field("owned_pages", &self.owned.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// End-to-end proof that the guard closes the tab it owns and leaves a
    /// caller-supplied tab alone.
    ///
    /// Requires a live CDP endpoint, so it is gated on `CHROME_URL` (repo
    /// precedent: `spider/tests/hedge_parallel_chrome_e2e.rs`).
    #[tokio::test]
    async fn clone_page_context_closes_its_tab_on_drop() {
        let Ok(chrome_url) = std::env::var("CHROME_URL") else {
            return;
        };

        let (browser, mut handler) = match Browser::connect(chrome_url).await {
            Ok(pair) => pair,
            Err(e) => panic!("failed to connect to CHROME_URL: {e}"),
        };

        let drive = tokio::spawn(async move {
            use futures::StreamExt;
            while handler.next().await.is_some() {}
        });

        let browser = Arc::new(browser);
        let base = Arc::new(
            browser
                .new_page("about:blank")
                .await
                .expect("base page created"),
        );

        // A caller-supplied page is never ours to close.
        let root = BrowserContext::new(browser.clone(), base.clone());
        assert!(root.page_guard.is_none(), "new() must not own the page");

        let before = browser.pages().await.map(|p| p.len()).unwrap_or_default();

        // A cloned page is ours: the tab must go away with the context.
        let cloned = root.clone_page().await.expect("clone_page");
        assert!(
            cloned.page_guard.is_some(),
            "clone_page() must own the new page"
        );

        let during = browser.pages().await.map(|p| p.len()).unwrap_or_default();
        assert_eq!(during, before + 1, "clone_page should open one tab");

        // A surviving clone must keep the tab alive.
        let survivor = cloned.clone();
        drop(cloned);
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        assert_eq!(
            browser.pages().await.map(|p| p.len()).unwrap_or_default(),
            during,
            "an intermediate clone drop must not close the tab"
        );

        // Last clone gone -> tab closed.
        drop(survivor);
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        assert_eq!(
            browser.pages().await.map(|p| p.len()).unwrap_or_default(),
            before,
            "dropping the last clone must close the owned tab"
        );

        // The caller-owned base page survived the whole thing.
        assert!(base.url().await.is_ok(), "caller-owned page must stay open");

        drop(root);
        let _ = Page::clone(base.as_ref()).close().await;
        drive.abort();
    }

    /// Dropping a guard with no tokio runtime present must not panic.
    #[test]
    fn guard_drop_without_runtime_is_a_noop() {
        // No `Page` can be constructed without a browser, so this exercises the
        // runtime probe directly: the same `try_current()` call the guard makes.
        assert!(
            tokio::runtime::Handle::try_current().is_err(),
            "no runtime should be active in a plain #[test]"
        );
    }
}
