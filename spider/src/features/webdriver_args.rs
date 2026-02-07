/// Chrome WebDriver arguments for headless mode.
#[cfg(not(feature = "webdriver_headed"))]
pub(crate) static CHROME_WEBDRIVER_ARGS: &[&str] = &[
    "--headless",
    "--disable-gpu",
    "--no-sandbox",
    "--disable-dev-shm-usage",
    "--disable-extensions",
    "--disable-background-networking",
    "--disable-sync",
    "--disable-default-apps",
    "--mute-audio",
    "--no-first-run",
    "--disable-popup-blocking",
    "--disable-hang-monitor",
    "--disable-prompt-on-repost",
    "--disable-client-side-phishing-detection",
    "--disable-component-update",
    "--disable-backgrounding-occluded-windows",
    "--disable-renderer-backgrounding",
    "--disable-background-timer-throttling",
    "--disable-ipc-flooding-protection",
    "--password-store=basic",
    "--use-mock-keychain",
    "--metrics-recording-only",
];

/// Chrome WebDriver arguments for headed mode.
#[cfg(feature = "webdriver_headed")]
pub(crate) static CHROME_WEBDRIVER_ARGS: &[&str] = &[
    "--disable-gpu",
    "--no-sandbox",
    "--disable-dev-shm-usage",
    "--disable-extensions",
    "--disable-background-networking",
    "--disable-sync",
    "--disable-default-apps",
    "--mute-audio",
    "--no-first-run",
    "--disable-popup-blocking",
    "--disable-hang-monitor",
    "--disable-prompt-on-repost",
    "--disable-client-side-phishing-detection",
    "--disable-component-update",
    "--disable-backgrounding-occluded-windows",
    "--disable-renderer-backgrounding",
    "--disable-background-timer-throttling",
    "--disable-ipc-flooding-protection",
    "--password-store=basic",
    "--use-mock-keychain",
    "--metrics-recording-only",
];

/// Firefox WebDriver arguments for headless mode.
#[cfg(not(feature = "webdriver_headed"))]
pub(crate) static FIREFOX_WEBDRIVER_ARGS: &[&str] = &["-headless", "-no-remote", "-new-instance"];

/// Firefox WebDriver arguments for headed mode.
#[cfg(feature = "webdriver_headed")]
pub(crate) static FIREFOX_WEBDRIVER_ARGS: &[&str] = &["-no-remote", "-new-instance"];

/// Edge WebDriver arguments for headless mode.
#[cfg(not(feature = "webdriver_headed"))]
pub(crate) static EDGE_WEBDRIVER_ARGS: &[&str] = &[
    "--headless",
    "--disable-gpu",
    "--no-sandbox",
    "--disable-dev-shm-usage",
    "--disable-extensions",
    "--disable-background-networking",
    "--disable-sync",
    "--disable-default-apps",
    "--mute-audio",
    "--no-first-run",
    "--disable-popup-blocking",
    "--disable-hang-monitor",
    "--disable-prompt-on-repost",
    "--disable-client-side-phishing-detection",
    "--disable-component-update",
    "--disable-backgrounding-occluded-windows",
    "--disable-renderer-backgrounding",
    "--disable-background-timer-throttling",
    "--disable-ipc-flooding-protection",
    "--password-store=basic",
    "--use-mock-keychain",
    "--metrics-recording-only",
];

/// Edge WebDriver arguments for headed mode.
#[cfg(feature = "webdriver_headed")]
pub(crate) static EDGE_WEBDRIVER_ARGS: &[&str] = &[
    "--disable-gpu",
    "--no-sandbox",
    "--disable-dev-shm-usage",
    "--disable-extensions",
    "--disable-background-networking",
    "--disable-sync",
    "--disable-default-apps",
    "--mute-audio",
    "--no-first-run",
    "--disable-popup-blocking",
    "--disable-hang-monitor",
    "--disable-prompt-on-repost",
    "--disable-client-side-phishing-detection",
    "--disable-component-update",
    "--disable-backgrounding-occluded-windows",
    "--disable-renderer-backgrounding",
    "--disable-background-timer-throttling",
    "--disable-ipc-flooding-protection",
    "--password-store=basic",
    "--use-mock-keychain",
    "--metrics-recording-only",
];

/// Get the default arguments for a browser type.
pub(crate) fn get_browser_args(
    browser: &super::webdriver_common::WebDriverBrowser,
) -> &'static [&'static str] {
    match browser {
        super::webdriver_common::WebDriverBrowser::Chrome => CHROME_WEBDRIVER_ARGS,
        super::webdriver_common::WebDriverBrowser::Firefox => FIREFOX_WEBDRIVER_ARGS,
        super::webdriver_common::WebDriverBrowser::Edge => EDGE_WEBDRIVER_ARGS,
    }
}
