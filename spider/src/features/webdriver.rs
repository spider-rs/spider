use crate::configuration::Configuration;
use crate::features::webdriver_args::get_browser_args;
use crate::features::webdriver_common::{WebDriverBrowser, WebDriverConfig};
use std::sync::Arc;
use std::time::Duration;
use thirtyfour::common::capabilities::desiredcapabilities::Capabilities;
use thirtyfour::prelude::*;
use tokio::task::JoinHandle;

/// Stealth script to hide WebDriver detection.
#[cfg(feature = "webdriver_stealth")]
pub const STEALTH_SCRIPT: &str = r#"
Object.defineProperty(navigator, 'webdriver', { get: () => undefined });
Object.defineProperty(navigator, 'plugins', { get: () => [1, 2, 3, 4, 5] });
Object.defineProperty(navigator, 'languages', { get: () => ['en-US', 'en'] });
window.chrome = { runtime: {} };
"#;

/// WebDriver control tuple type alias (mirrors BrowserControl).
pub type WebDriverControl = (
    Arc<WebDriver>,
    Option<JoinHandle<()>>,
    Option<String>,
);

/// WebDriver controller with RAII cleanup (mirrors BrowserController).
pub struct WebDriverController {
    /// The WebDriver instance.
    pub driver: WebDriverControl,
    /// Whether the driver has been closed.
    pub closed: bool,
}

impl WebDriverController {
    /// Create a new WebDriver controller.
    pub fn new(driver: WebDriverControl) -> Self {
        Self {
            driver,
            closed: false,
        }
    }

    /// Get a reference to the WebDriver.
    pub fn driver(&self) -> &Arc<WebDriver> {
        &self.driver.0
    }

    /// Dispose the WebDriver and close the session.
    pub fn dispose(&mut self) {
        if !self.closed {
            self.closed = true;
            if let Some(handler) = self.driver.1.take() {
                handler.abort();
            }
        }
    }
}

impl Drop for WebDriverController {
    fn drop(&mut self) {
        self.dispose();
    }
}

/// Launch a WebDriver session with the provided configuration.
pub async fn launch_driver(
    config: &Configuration,
) -> Option<WebDriverController> {
    let webdriver_config = config.webdriver_config.as_ref()?;
    launch_driver_base(webdriver_config, config).await
}

/// Launch a WebDriver session with the base configuration.
pub async fn launch_driver_base(
    webdriver_config: &WebDriverConfig,
    config: &Configuration,
) -> Option<WebDriverController> {
    let server_url = &webdriver_config.server_url;

    let caps = build_capabilities(webdriver_config, config).await?;

    let mut attempts = 0;
    let max_retries = 10;
    let mut driver: Option<WebDriver> = None;

    while attempts <= max_retries {
        match WebDriver::new(server_url, caps.clone()).await {
            Ok(d) => {
                driver = Some(d);
                break;
            }
            Err(err) => {
                log::error!("WebDriver connection error: {:?}", err);
                attempts += 1;
                if attempts > max_retries {
                    log::error!("Exceeded maximum retry attempts for WebDriver connection");
                    break;
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }
    }

    let driver = driver?;
    let driver_arc = Arc::new(driver);

    // Set up viewport if configured
    if let (Some(width), Some(height)) = (
        webdriver_config.viewport_width,
        webdriver_config.viewport_height,
    ) {
        if let Err(e) = driver_arc
            .set_window_rect(0, 0, width, height)
            .await
        {
            log::warn!("Failed to set viewport: {:?}", e);
        }
    }

    // Set up timeouts
    if let Some(timeout) = webdriver_config.timeout {
        let timeouts = TimeoutConfiguration::new(
            Some(timeout),
            Some(timeout),
            Some(timeout),
        );
        if let Err(e) = driver_arc.update_timeouts(timeouts).await {
            log::warn!("Failed to set timeouts: {:?}", e);
        }
    }

    Some(WebDriverController::new((
        driver_arc,
        None,
        Some(server_url.clone()),
    )))
}

/// Build browser capabilities based on configuration.
async fn build_capabilities(
    webdriver_config: &WebDriverConfig,
    config: &Configuration,
) -> Option<Capabilities> {
    match webdriver_config.browser {
        WebDriverBrowser::Chrome => build_chrome_capabilities(webdriver_config, config).await,
        WebDriverBrowser::Firefox => build_firefox_capabilities(webdriver_config, config).await,
        WebDriverBrowser::Edge => build_edge_capabilities(webdriver_config, config).await,
    }
}

/// Build Chrome capabilities.
async fn build_chrome_capabilities(
    webdriver_config: &WebDriverConfig,
    config: &Configuration,
) -> Option<Capabilities> {
    let mut caps = DesiredCapabilities::chrome();

    // Set accept insecure certs
    if webdriver_config.accept_insecure_certs {
        if let Err(e) = caps.accept_insecure_certs(true) {
            log::warn!("Failed to set accept_insecure_certs: {:?}", e);
        }
    }

    // Set page load strategy
    if let Some(ref strategy) = webdriver_config.page_load_strategy {
        let strategy = match strategy.as_str() {
            "eager" => thirtyfour::PageLoadStrategy::Eager,
            "none" => thirtyfour::PageLoadStrategy::None,
            _ => thirtyfour::PageLoadStrategy::Normal,
        };
        if let Err(e) = caps.set_page_load_strategy(strategy) {
            log::warn!("Failed to set page_load_strategy: {:?}", e);
        }
    }

    // Collect all arguments
    let mut args: Vec<String> = Vec::new();

    // Add default browser args
    let default_args = get_browser_args(&WebDriverBrowser::Chrome);
    for arg in default_args {
        args.push(arg.to_string());
    }

    // Add custom browser args
    if let Some(ref custom_args) = webdriver_config.browser_args {
        args.extend(custom_args.clone());
    }

    // Add headless argument if needed
    if webdriver_config.headless && !args.iter().any(|a| a.contains("headless")) {
        args.push("--headless".to_string());
    }

    // Add user agent
    if let Some(ref ua) = webdriver_config.user_agent {
        args.push(format!("--user-agent={}", ua));
    } else if let Some(ref ua) = config.user_agent {
        args.push(format!("--user-agent={}", ua));
    }

    // Add proxy
    if let Some(ref proxy) = webdriver_config.proxy {
        args.push(format!("--proxy-server={}", proxy));
    }

    // Add viewport
    if let (Some(width), Some(height)) = (
        webdriver_config.viewport_width,
        webdriver_config.viewport_height,
    ) {
        args.push(format!("--window-size={},{}", width, height));
    }

    // Add all args to capabilities
    for arg in args {
        if let Err(e) = caps.add_arg(&arg) {
            log::warn!("Failed to add Chrome arg '{}': {:?}", arg, e);
        }
    }

    Some(caps.into())
}

/// Build Firefox capabilities.
async fn build_firefox_capabilities(
    webdriver_config: &WebDriverConfig,
    _config: &Configuration,
) -> Option<Capabilities> {
    let mut caps = DesiredCapabilities::firefox();

    // Set accept insecure certs
    if webdriver_config.accept_insecure_certs {
        if let Err(e) = caps.accept_insecure_certs(true) {
            log::warn!("Failed to set accept_insecure_certs: {:?}", e);
        }
    }

    // Set page load strategy
    if let Some(ref strategy) = webdriver_config.page_load_strategy {
        let strategy = match strategy.as_str() {
            "eager" => thirtyfour::PageLoadStrategy::Eager,
            "none" => thirtyfour::PageLoadStrategy::None,
            _ => thirtyfour::PageLoadStrategy::Normal,
        };
        if let Err(e) = caps.set_page_load_strategy(strategy) {
            log::warn!("Failed to set page_load_strategy: {:?}", e);
        }
    }

    // Collect all arguments
    let mut args: Vec<String> = Vec::new();

    // Add default browser args
    let default_args = get_browser_args(&WebDriverBrowser::Firefox);
    for arg in default_args {
        args.push(arg.to_string());
    }

    // Add custom browser args
    if let Some(ref custom_args) = webdriver_config.browser_args {
        args.extend(custom_args.clone());
    }

    // Add headless argument if needed
    if webdriver_config.headless && !args.iter().any(|a| a.contains("headless")) {
        args.push("-headless".to_string());
    }

    // Add all args to capabilities
    for arg in args {
        if let Err(e) = caps.add_arg(&arg) {
            log::warn!("Failed to add Firefox arg '{}': {:?}", arg, e);
        }
    }

    Some(caps.into())
}

/// Build Edge capabilities.
async fn build_edge_capabilities(
    webdriver_config: &WebDriverConfig,
    config: &Configuration,
) -> Option<Capabilities> {
    let mut caps = DesiredCapabilities::edge();

    // Set accept insecure certs
    if webdriver_config.accept_insecure_certs {
        if let Err(e) = caps.accept_insecure_certs(true) {
            log::warn!("Failed to set accept_insecure_certs: {:?}", e);
        }
    }

    // Set page load strategy
    if let Some(ref strategy) = webdriver_config.page_load_strategy {
        let strategy = match strategy.as_str() {
            "eager" => thirtyfour::PageLoadStrategy::Eager,
            "none" => thirtyfour::PageLoadStrategy::None,
            _ => thirtyfour::PageLoadStrategy::Normal,
        };
        if let Err(e) = caps.set_page_load_strategy(strategy) {
            log::warn!("Failed to set page_load_strategy: {:?}", e);
        }
    }

    // Collect all arguments
    let mut args: Vec<String> = Vec::new();

    // Add default browser args
    let default_args = get_browser_args(&WebDriverBrowser::Edge);
    for arg in default_args {
        args.push(arg.to_string());
    }

    // Add custom browser args
    if let Some(ref custom_args) = webdriver_config.browser_args {
        args.extend(custom_args.clone());
    }

    // Add headless argument if needed
    if webdriver_config.headless && !args.iter().any(|a| a.contains("headless")) {
        args.push("--headless".to_string());
    }

    // Add user agent
    if let Some(ref ua) = webdriver_config.user_agent {
        args.push(format!("--user-agent={}", ua));
    } else if let Some(ref ua) = config.user_agent {
        args.push(format!("--user-agent={}", ua));
    }

    // Add proxy
    if let Some(ref proxy) = webdriver_config.proxy {
        args.push(format!("--proxy-server={}", proxy));
    }

    // Add viewport
    if let (Some(width), Some(height)) = (
        webdriver_config.viewport_width,
        webdriver_config.viewport_height,
    ) {
        args.push(format!("--window-size={},{}", width, height));
    }

    // Add all args to capabilities
    for arg in args {
        if let Err(e) = caps.add_arg(&arg) {
            log::warn!("Failed to add Edge arg '{}': {:?}", arg, e);
        }
    }

    Some(caps.into())
}

/// Setup WebDriver events and stealth mode.
#[cfg(feature = "webdriver_stealth")]
pub async fn setup_driver_events(driver: &WebDriver, _config: &Configuration) {
    // Inject stealth script
    if let Err(e) = driver.execute(STEALTH_SCRIPT, vec![]).await {
        log::warn!("Failed to inject stealth script: {:?}", e);
    }
}

/// Setup WebDriver events (no-op without stealth feature).
#[cfg(not(feature = "webdriver_stealth"))]
pub async fn setup_driver_events(_driver: &WebDriver, _config: &Configuration) {
    // No stealth injection without the feature
}

/// Attempt to navigate to a URL.
pub async fn attempt_navigation(
    url: &str,
    driver: &WebDriver,
    timeout: &Option<Duration>,
) -> Result<(), WebDriverError> {
    let nav_future = driver.goto(url);

    match timeout {
        Some(t) => {
            match tokio::time::timeout(*t, nav_future).await {
                Ok(result) => result,
                Err(_) => Err(WebDriverError::Timeout("Navigation timeout".to_string())),
            }
        }
        None => nav_future.await,
    }
}

/// Get the page content (HTML source).
pub async fn get_page_content(driver: &WebDriver) -> Result<String, WebDriverError> {
    driver.source().await
}

/// Get the current URL.
pub async fn get_current_url(driver: &WebDriver) -> Result<String, WebDriverError> {
    driver.current_url().await.map(|u| u.to_string())
}

/// Get the page title.
pub async fn get_page_title(driver: &WebDriver) -> Result<String, WebDriverError> {
    driver.title().await
}

/// Take a screenshot of the page.
#[cfg(feature = "webdriver_screenshot")]
pub async fn take_screenshot(driver: &WebDriver) -> Result<Vec<u8>, WebDriverError> {
    driver.screenshot_as_png().await
}

/// Take a screenshot (stub without feature).
#[cfg(not(feature = "webdriver_screenshot"))]
pub async fn take_screenshot(_driver: &WebDriver) -> Result<Vec<u8>, WebDriverError> {
    Err(WebDriverError::FatalError("Screenshot feature not enabled".to_string()))
}

/// Execute JavaScript on the page and return the result as a JSON value.
pub async fn execute_script(
    driver: &WebDriver,
    script: &str,
) -> Result<serde_json::Value, WebDriverError> {
    let result = driver.execute(script, vec![]).await?;
    Ok(result.json().clone())
}

/// Wait for an element to be present.
pub async fn wait_for_element(
    driver: &WebDriver,
    selector: &str,
    timeout: Duration,
) -> Result<WebElement, WebDriverError> {
    driver
        .query(By::Css(selector))
        .wait(timeout, Duration::from_millis(100))
        .first()
        .await
}

/// Close the WebDriver session (consumes the driver).
pub async fn close_driver(driver: WebDriver) {
    if let Err(e) = driver.quit().await {
        log::warn!("Failed to close WebDriver session: {:?}", e);
    }
}

/// Get a random viewport for stealth purposes.
#[cfg(feature = "real_browser")]
pub fn get_random_webdriver_viewport() -> (u32, u32) {
    use super::chrome_viewport::get_random_viewport;
    let vp = get_random_viewport();
    (vp.width, vp.height)
}

/// Get a default viewport.
#[cfg(not(feature = "real_browser"))]
pub fn get_random_webdriver_viewport() -> (u32, u32) {
    (1920, 1080)
}
