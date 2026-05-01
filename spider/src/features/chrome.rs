use crate::features::chrome_args::CHROME_ARGS;
use crate::utils::{detect_chrome::get_detect_chrome_executable, log};
use crate::{
    configuration::{Configuration, RedirectPolicy},
    tokio_stream::StreamExt,
};
use chromiumoxide::cdp::browser_protocol::browser::{
    SetDownloadBehaviorBehavior, SetDownloadBehaviorParamsBuilder,
};
use chromiumoxide::cdp::browser_protocol::{
    browser::BrowserContextId,
    emulation::{SetGeolocationOverrideParams, SetScriptExecutionDisabledParams},
    network::CookieParam,
    target::CreateTargetParams,
};
use chromiumoxide::error::CdpError;
use chromiumoxide::handler::REQUEST_TIMEOUT;
use chromiumoxide::serde_json;
use chromiumoxide::Page;
use chromiumoxide::{handler::HandlerConfig, Browser, BrowserConfig};
use lazy_static::lazy_static;
#[cfg(feature = "cookies")]
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;
use url::Url;

lazy_static! {
    /// Enable loopback for proxy.
    static ref LOOP_BACK_PROXY: bool = std::env::var("LOOP_BACK_PROXY").unwrap_or_default() == "true";
}

#[cfg(feature = "cookies")]
/// Parse a cookie into a jar. This does nothing without the 'cookies' flag.
pub fn parse_cookies_with_jar(
    jar: &Arc<crate::client::cookie::Jar>,
    cookie_str: &str,
    url: &Url,
) -> Result<Vec<CookieParam>, String> {
    use crate::client::cookie::CookieStore;

    // Retrieve cookies stored in the jar
    if let Some(header_value) = jar.cookies(url) {
        let cookie_header_str = header_value.to_str().map_err(|e| e.to_string())?;
        let cookie_pairs: Vec<&str> = cookie_header_str.split(';').collect();

        // Cap to bound pre-allocation against malicious Set-Cookie headers.
        let mut cookies = Vec::with_capacity(cookie_pairs.len().min(256));

        for pair in cookie_pairs {
            let parts: Vec<&str> = pair.trim().splitn(2, '=').collect();

            if parts.len() == 2 {
                let name = parts[0].trim();
                let value = parts[1].trim();

                let mut builder = CookieParam::builder()
                    .name(name)
                    .value(value)
                    .url(url.as_str());

                if let Some(domain) = url.domain() {
                    builder = builder.domain(domain.to_string());
                }

                let path = url.path();
                builder = builder.path(if path.is_empty() { "/" } else { path });

                if cookie_str.contains("Secure") {
                    builder = builder.secure(true);
                }

                if cookie_str.contains("HttpOnly") {
                    builder = builder.http_only(true);
                }
                match builder.build() {
                    Ok(cookie_param) => cookies.push(cookie_param),
                    Err(e) => return Err(e),
                }
            } else {
                return Err(format!("Invalid cookie pair: {}", pair));
            }
        }

        Ok(cookies)
    } else {
        Err("No cookies found".to_string())
    }
}

/// Parse a cookie into a jar. This does nothing without the 'cookies' flag.
#[cfg(not(feature = "cookies"))]
pub fn parse_cookies_with_jar(cookie_str: &str, url: &Url) -> Result<Vec<CookieParam>, String> {
    Ok(Default::default())
}

#[cfg(feature = "cookies")]
/// Seed jar from cookie header.
pub fn seed_jar_from_cookie_header(
    jar: &std::sync::Arc<crate::client::cookie::Jar>,
    cookie_header: &str,
    url: &url::Url,
) -> Result<(), String> {
    for pair in cookie_header.split(';') {
        let pair = pair.trim();
        if pair.is_empty() {
            continue;
        }

        let (name, value) = pair
            .split_once('=')
            .ok_or_else(|| format!("Invalid cookie pair: {pair}"))?;

        let set_cookie = format!("{}={}; Path=/", name.trim(), value.trim());
        jar.add_cookie_str(&set_cookie, url);
    }
    Ok(())
}

#[cfg(all(feature = "cookies", feature = "chrome"))]
/// Set the page cookies.
pub async fn set_page_cookies(
    page: &chromiumoxide::Page,
    cookies: Vec<chromiumoxide::cdp::browser_protocol::network::CookieParam>,
) -> Result<(), String> {
    use chromiumoxide::cdp::browser_protocol::network::SetCookiesParams;

    if cookies.is_empty() {
        return Ok(());
    }

    page.execute(SetCookiesParams::new(cookies))
        .await
        .map_err(|e| e.to_string())?;

    Ok(())
}

#[cfg(feature = "cookies")]
/// Set cookie params from jar.
pub fn cookie_params_from_jar(
    jar: &std::sync::Arc<crate::client::cookie::Jar>,
    url: &url::Url,
) -> Result<Vec<chromiumoxide::cdp::browser_protocol::network::CookieParam>, String> {
    use crate::client::cookie::CookieStore;
    use chromiumoxide::cdp::browser_protocol::network::CookieParam;

    let Some(header_value) = jar.cookies(url) else {
        return Ok(Vec::new());
    };

    let s = header_value.to_str().map_err(|e| e.to_string())?;
    // Cap to bound pre-allocation against malicious cookie headers.
    let mut out =
        Vec::with_capacity((memchr::memchr_iter(b';', s.as_bytes()).count() + 1).min(256));

    for pair in s.split(';') {
        let pair = pair.trim();
        if pair.is_empty() {
            continue;
        }

        let (name, value) = pair
            .split_once('=')
            .ok_or_else(|| format!("Invalid cookie pair: {pair}"))?;

        let cp = CookieParam::builder()
            .name(name.trim())
            .value(value.trim())
            .url(url.as_str())
            .build()
            .map_err(|e| e.to_string())?;

        out.push(cp);
    }

    Ok(out)
}

/// Handle the browser cookie configurations.
#[cfg(feature = "cookies")]
pub async fn set_cookies(
    jar: &Arc<crate::client::cookie::Jar>,
    config: &Configuration,
    url_parsed: &Option<Box<Url>>,
    browser: &Browser,
) {
    if config.cookie_str.is_empty() {
        return;
    }

    let Some(parsed) = url_parsed.as_deref() else {
        return;
    };

    let _ = seed_jar_from_cookie_header(jar, &config.cookie_str, parsed);

    match parse_cookies_with_jar(jar, &config.cookie_str, parsed) {
        Ok(cookies) if !cookies.is_empty() => {
            let _ = browser.set_cookies(cookies).await;
        }
        _ => {}
    }
}

/// Patch Chrome args to enable the built-in AI (LanguageModel / Gemini Nano).
///
/// Removes `OptimizationHints` from `--disable-features` (which blocks the
/// on-device model), and adds the required `--enable-features` flags.
fn patch_chrome_ai_args(args: &mut Vec<String>) {
    for arg in args.iter_mut() {
        // Remove OptimizationHints from --disable-features
        if arg.starts_with("--disable-features=") {
            let features: Vec<&str> = arg["--disable-features=".len()..]
                .split(',')
                .filter(|f| *f != "OptimizationHints")
                .collect();
            *arg = format!("--disable-features={}", features.join(","));
        }
        // Append AI features to existing --enable-features
        if arg.starts_with("--enable-features=") {
            arg.push_str(",OptimizationGuideOnDeviceModel:BypassPerfRequirement/true,PromptAPIForGeminiNano,PromptAPIForGeminiNanoMultimodalInput");
        }
    }
    // If no --enable-features existed, add one
    if !args.iter().any(|a| a.starts_with("--enable-features=")) {
        args.push("--enable-features=OptimizationGuideOnDeviceModel:BypassPerfRequirement/true,PromptAPIForGeminiNano,PromptAPIForGeminiNanoMultimodalInput".to_string());
    }
}

/// get chrome configuration
#[cfg(not(feature = "chrome_headed"))]
pub fn get_browser_config(
    proxies: &Option<Vec<crate::configuration::RequestProxy>>,
    intercept: bool,
    cache_enabled: bool,
    viewport: impl Into<Option<chromiumoxide::handler::viewport::Viewport>>,
    request_timeout: &Option<core::time::Duration>,
    use_chrome_ai: bool,
) -> Option<BrowserConfig> {
    let builder = BrowserConfig::builder()
        .disable_default_args()
        .no_sandbox()
        .request_timeout(match request_timeout.as_ref() {
            Some(timeout) => *timeout,
            _ => Duration::from_millis(REQUEST_TIMEOUT),
        });

    let builder = if cache_enabled {
        builder.enable_cache()
    } else {
        builder.disable_cache()
    };

    // request interception is required for all browser.new_page() creations. We also have to use "about:blank" as the base page to setup the listeners and navigate afterwards or the request will hang.
    let builder = if intercept {
        builder.enable_request_intercept()
    } else {
        builder
    };

    let builder = match proxies {
        Some(proxies) => {
            let mut chrome_args = Vec::from(CHROME_ARGS.map(|e| e.replace("://", "=")));
            if use_chrome_ai {
                patch_chrome_ai_args(&mut chrome_args);
            }
            let base_proxies = proxies
                .iter()
                .filter_map(|p| {
                    if p.ignore == crate::configuration::ProxyIgnore::Chrome {
                        None
                    } else {
                        Some(p.addr.to_owned())
                    }
                })
                .collect::<Vec<String>>();

            if !base_proxies.is_empty() {
                chrome_args.push(string_concat!(r#"--proxy-server="#, base_proxies.join(";")));
            }

            builder.args(chrome_args)
        }
        _ => {
            if use_chrome_ai {
                let mut chrome_args: Vec<String> =
                    CHROME_ARGS.iter().map(|e| e.to_string()).collect();
                patch_chrome_ai_args(&mut chrome_args);
                builder.args(chrome_args)
            } else {
                builder.args(CHROME_ARGS)
            }
        }
    };
    let builder = match get_detect_chrome_executable() {
        Some(v) => builder.chrome_executable(v),
        _ => builder,
    };

    match builder.viewport(viewport).build() {
        Ok(b) => Some(b),
        Err(error) => {
            log("", error);
            None
        }
    }
}

/// get chrome configuration headful
#[cfg(feature = "chrome_headed")]
pub fn get_browser_config(
    proxies: &Option<Vec<crate::configuration::RequestProxy>>,
    intercept: bool,
    cache_enabled: bool,
    viewport: impl Into<Option<chromiumoxide::handler::viewport::Viewport>>,
    request_timeout: &Option<core::time::Duration>,
    use_chrome_ai: bool,
) -> Option<BrowserConfig> {
    let builder = BrowserConfig::builder()
        .disable_default_args()
        .no_sandbox()
        .request_timeout(match request_timeout.as_ref() {
            Some(timeout) => *timeout,
            _ => Duration::from_millis(REQUEST_TIMEOUT),
        })
        .with_head();

    let builder = if cache_enabled {
        builder.enable_cache()
    } else {
        builder.disable_cache()
    };

    let builder = if intercept {
        builder.enable_request_intercept()
    } else {
        builder
    };

    let mut chrome_args = Vec::from(CHROME_ARGS.map(|e| {
        if e == "--headless" {
            String::new()
        } else {
            e.replace("://", "=")
        }
    }));

    if use_chrome_ai {
        patch_chrome_ai_args(&mut chrome_args);
    }

    let builder = match proxies {
        Some(proxies) => {
            let base_proxies = proxies
                .iter()
                .filter_map(|p| {
                    if p.ignore == crate::configuration::ProxyIgnore::Chrome {
                        None
                    } else {
                        Some(p.addr.to_owned())
                    }
                })
                .collect::<Vec<String>>();

            chrome_args.push(string_concat!(r#"--proxy-server="#, base_proxies.join(";")));

            builder.args(chrome_args)
        }
        _ => builder.args(chrome_args),
    };
    let builder = match get_detect_chrome_executable() {
        Some(v) => builder.chrome_executable(v),
        _ => builder,
    };
    match builder.viewport(viewport).build() {
        Ok(b) => Some(b),
        Err(error) => {
            log("", error);
            None
        }
    }
}

/// create the browser handler configuration
pub fn create_handler_config(config: &Configuration) -> HandlerConfig {
    HandlerConfig {
        request_timeout: match config.request_timeout.as_ref() {
            Some(timeout) => *timeout,
            _ => Duration::from_millis(REQUEST_TIMEOUT),
        },
        request_intercept: config.chrome_intercept.enabled,
        cache_enabled: config.cache,
        service_worker_enabled: config.service_worker_enabled,
        viewport: match config.viewport {
            Some(ref v) => Some(chromiumoxide::handler::viewport::Viewport::from(
                v.to_owned(),
            )),
            _ => default_viewport(),
        },
        ignore_visuals: config.chrome_intercept.block_visuals,
        whitelist_patterns: config.chrome_intercept.whitelist_patterns.clone(),
        blacklist_patterns: config.chrome_intercept.blacklist_patterns.clone(),
        ignore_ads: config.chrome_intercept.block_ads,
        ignore_javascript: config.chrome_intercept.block_javascript,
        ignore_analytics: config.chrome_intercept.block_analytics,
        ignore_stylesheets: config.chrome_intercept.block_stylesheets,
        extra_headers: match &config.headers {
            Some(headers) => {
                let mut hm = crate::utils::header_utils::header_map_to_hash_map(headers.inner());

                cleanup_invalid_headers(&mut hm);

                if hm.is_empty() {
                    None
                } else {
                    if cfg!(feature = "real_browser") {
                        crate::utils::header_utils::rewrite_headers_to_title_case(&mut hm);
                    }
                    Some(hm)
                }
            }
            _ => None,
        },
        intercept_manager: config.chrome_intercept.intercept_manager,
        only_html: config.only_html && !config.full_resources,
        max_bytes_allowed: config.max_bytes_allowed,
        // Only enforce the redirect cap on the Chrome path when the user
        // explicitly opted in via `with_redirect_limit`. Otherwise leave
        // Chromium's internal ~20-hop cap in effect to preserve prior
        // behavior on pages with long but legitimate redirect chains.
        // `RedirectPolicy::None` also disables enforcement so HTTP and
        // Chrome agree.
        max_redirects: if config.redirect_limit_set
            && !matches!(config.redirect_policy, RedirectPolicy::None)
        {
            Some(config.redirect_limit)
        } else {
            None
        },
        // JS / meta-refresh loop guard. `None` preserves prior behavior.
        max_main_frame_navigations: config.max_main_frame_navigations,
        ..HandlerConfig::default()
    }
}

lazy_static! {
    static ref CHROM_BASE: Option<String> = std::env::var("CHROME_URL").ok();
}

/// Lock-free failover across multiple remote Chrome endpoints.
///
/// Tracks per-endpoint consecutive errors with atomics. When an endpoint
/// exceeds `max_retries` failures it is skipped and the next one is tried.
/// Once all endpoints have been exhausted, returns `None`.
///
/// Zero overhead when only one endpoint is configured (inline fast-path).
#[derive(Debug)]
pub struct ChromeConnectionFailover {
    urls: Vec<String>,
    /// Per-endpoint consecutive error count.
    errors: Vec<std::sync::atomic::AtomicU32>,
    /// Per-endpoint cooldown deadline as Unix milliseconds. `0` means
    /// not in cooldown — the endpoint is considered healthy. A non-zero
    /// value means "skip this endpoint while `now < dead_until[i]`".
    /// Set by `mark_url_bad`; consulted by `connect()`. Lock-free —
    /// stale reads are harmless (a recently-marked endpoint may be
    /// retried once before the AtomicU64 settles, or a recovered
    /// endpoint may be skipped one extra time).
    dead_until: Vec<std::sync::atomic::AtomicU64>,
    /// Index of the most recently-successful endpoint, or `usize::MAX`
    /// if no successful connection has happened yet. Lock-free — last
    /// writer wins under concurrent connects, which is acceptable since
    /// we only consult this for "best-effort" URL lookup.
    last_idx: std::sync::atomic::AtomicUsize,
    /// Max retries per endpoint before moving to the next.
    max_retries: u32,
}

/// Lazy, lock-free, leak-free holder for a `ChromeConnectionFailover`.
///
/// Constructed lazily on first call to `get_or_init`, then reused across
/// every subsequent `setup_browser_configuration` invocation. After init,
/// access is a single atomic load — no parking, no awaits, no mutexes.
///
/// Reset (via `with_chrome_connections`) replaces the cell with a fresh
/// empty one; the previous `Arc<ChromeConnectionFailover>` drops cleanly
/// once the last in-flight reader releases it.
#[derive(Debug, Default)]
pub struct LazyChromeFailover {
    inner: std::sync::Arc<std::sync::OnceLock<std::sync::Arc<ChromeConnectionFailover>>>,
}

impl Clone for LazyChromeFailover {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl PartialEq for LazyChromeFailover {
    fn eq(&self, _other: &Self) -> bool {
        // Runtime-only cache; not part of configuration identity.
        true
    }
}

impl LazyChromeFailover {
    /// Borrow the failover, lazily initializing it on first call.
    /// The closure runs exactly once for the lifetime of this cell.
    #[inline]
    pub fn get_or_init(
        &self,
        urls: &[String],
        max_retries: u32,
    ) -> &std::sync::Arc<ChromeConnectionFailover> {
        self.inner.get_or_init(|| {
            std::sync::Arc::new(ChromeConnectionFailover::new(urls.to_vec(), max_retries))
        })
    }

    /// Mark a URL as dead on the inner failover (no-op if the cell hasn't
    /// been initialized yet, or the URL isn't part of the failover list).
    /// Lock-free: a single atomic store on the matching slot.
    #[inline]
    pub fn mark_url_bad(&self, url: &str, cooldown_millis: u64) {
        if let Some(failover) = self.inner.get() {
            failover.mark_url_bad(url, cooldown_millis);
        }
    }

    /// Borrowed URL of the most recently-successful endpoint, or `None`
    /// when the cell hasn't initialized or no successful connect has
    /// happened yet. Lock-free: one atomic load + one slice lookup.
    #[inline]
    pub fn last_connected_url(&self) -> Option<&str> {
        self.inner.get().and_then(|f| f.last_connected_url())
    }
}

impl ChromeConnectionFailover {
    /// Create a failover from a list of URLs.
    pub fn new(urls: Vec<String>, max_retries: u32) -> Self {
        let errors = urls
            .iter()
            .map(|_| std::sync::atomic::AtomicU32::new(0))
            .collect();
        let dead_until = urls
            .iter()
            .map(|_| std::sync::atomic::AtomicU64::new(0))
            .collect();
        Self {
            urls,
            errors,
            dead_until,
            last_idx: std::sync::atomic::AtomicUsize::new(usize::MAX),
            max_retries,
        }
    }

    /// Borrowed URL of the most recently-successful endpoint, or `None`
    /// if nothing has connected yet (or the stored index has been
    /// invalidated). Used by `Configuration::chrome_fetch_params` to
    /// auto-populate the chrome endpoint URL on every chrome fetch path
    /// without forcing call sites to thread `BrowserController` through
    /// their signatures.
    #[inline]
    pub fn last_connected_url(&self) -> Option<&str> {
        let idx = self.last_idx.load(std::sync::atomic::Ordering::Acquire);
        self.urls.get(idx).map(|s| s.as_str())
    }

    /// Current Unix time in milliseconds. Saturates to 0 on a clock skew
    /// far enough into the past to underflow `Duration` (effectively
    /// impossible on a sane host). Never panics.
    #[inline]
    fn now_millis() -> u64 {
        match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
            Ok(d) => {
                let m = d.as_millis();
                if m > u128::from(u64::MAX) {
                    u64::MAX
                } else {
                    m as u64
                }
            }
            Err(_) => 0,
        }
    }

    /// Mark a URL as dead for `cooldown_millis`. Subsequent `connect()`
    /// calls skip the matching endpoint until the cooldown elapses.
    /// Unknown URLs are ignored (no panic, no allocation). Lock-free:
    /// a single atomic store on the matching slot.
    pub fn mark_url_bad(&self, url: &str, cooldown_millis: u64) {
        if cooldown_millis == 0 {
            return;
        }
        let until = Self::now_millis().saturating_add(cooldown_millis);
        for (i, u) in self.urls.iter().enumerate() {
            if u == url {
                self.dead_until[i].store(until, std::sync::atomic::Ordering::Release);
                log::warn!(
                    "[chrome-failover] endpoint {} ({}) marked dead for {}ms",
                    i,
                    url,
                    cooldown_millis
                );
                return;
            }
        }
    }

    /// Try to establish a browser connection, failing over across endpoints.
    ///
    /// For each endpoint: retry up to `max_retries` times with backoff.
    /// If all retries fail, move to the next endpoint. Returns the first
    /// successful connection (paired with the URL it succeeded on) or
    /// `None` if all endpoints are exhausted.
    ///
    /// Endpoints whose `dead_until` cooldown has not yet elapsed are
    /// skipped without a connection attempt. If every endpoint is in
    /// cooldown the function falls through to a second pass that ignores
    /// the cooldown — preserving the prior "always-try-something"
    /// contract so callers don't see a sudden None return when they
    /// would have seen a slow recovery attempt before.
    pub async fn connect(
        &self,
        config: &Configuration,
    ) -> Option<(Browser, chromiumoxide::Handler, String)> {
        let handler_config_base = create_handler_config(config);

        for pass in 0u8..2 {
            // Pass 0 honors cooldowns; pass 1 ignores them so we never
            // return None solely because every endpoint is in cooldown.
            let now = if pass == 0 { Self::now_millis() } else { 0 };
            let mut all_cooled_down = true;

            for (idx, url) in self.urls.iter().enumerate() {
                if pass == 0 {
                    let until = self.dead_until[idx].load(std::sync::atomic::Ordering::Acquire);
                    if until > now {
                        log::debug!(
                            "[chrome-failover] endpoint {} ({}) skipped (cooldown {}ms remaining)",
                            idx,
                            url,
                            until.saturating_sub(now)
                        );
                        continue;
                    }
                }
                all_cooled_down = false;
                let err_count = &self.errors[idx];

                for attempt in 0..=self.max_retries {
                    match Browser::connect_with_config(url.as_str(), handler_config_base.clone())
                        .await
                    {
                        Ok((browser, handler)) => {
                            // Reset error count and clear any cooldown on success.
                            err_count.store(0, std::sync::atomic::Ordering::Relaxed);
                            self.dead_until[idx].store(0, std::sync::atomic::Ordering::Release);
                            // Record the successful endpoint for later URL
                            // lookup (e.g. auto-populating chrome_endpoint_url
                            // on subsequent chrome_fetch_params() calls).
                            self.last_idx
                                .store(idx, std::sync::atomic::Ordering::Release);
                            if idx > 0 {
                                log::info!(
                                    "[chrome-failover] connected to endpoint {} ({}) after skipping {}",
                                    idx,
                                    url,
                                    idx
                                );
                            }
                            return Some((browser, handler, url.clone()));
                        }
                        Err(e) => {
                            let n =
                                err_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                            log::warn!(
                                "[chrome-failover] endpoint {} ({}) attempt {}/{} failed: {:?}",
                                idx,
                                url,
                                attempt + 1,
                                self.max_retries + 1,
                                e
                            );
                            if attempt < self.max_retries {
                                let backoff =
                                    crate::utils::backoff::backoff_delay(attempt, 100, 5_000);
                                tokio::time::sleep(backoff).await;
                            } else {
                                log::warn!(
                                    "[chrome-failover] endpoint {} exhausted ({} errors), trying next",
                                    idx,
                                    n
                                );
                            }
                        }
                    }
                }
            }

            // Nothing was actually attempted this pass — every endpoint
            // was in cooldown. Fall through to pass 1 (ignore cooldowns).
            if !all_cooled_down {
                break;
            }
        }

        log::error!(
            "[chrome-failover] all {} endpoints exhausted",
            self.urls.len()
        );
        None
    }

    /// Number of endpoints.
    #[inline]
    pub fn len(&self) -> usize {
        self.urls.len()
    }

    /// Whether the failover list is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.urls.is_empty()
    }
}

/// Get the default viewport
#[cfg(not(feature = "real_browser"))]
pub fn default_viewport() -> Option<chromiumoxide::handler::viewport::Viewport> {
    None
}

/// Get the default viewport
#[cfg(feature = "real_browser")]
pub fn default_viewport() -> Option<chromiumoxide::handler::viewport::Viewport> {
    use super::chrome_viewport::get_random_viewport;
    Some(chromiumoxide::handler::viewport::Viewport::from(
        get_random_viewport(),
    ))
}

/// Cleanup the headermap.
pub fn cleanup_invalid_headers(hm: &mut std::collections::HashMap<String, String>) {
    hm.remove("User-Agent");
    hm.remove("user-agent");
    hm.remove("host");
    hm.remove("Host");
    hm.remove("connection");
    hm.remove("Connection");
    hm.remove("content-length");
    hm.remove("Content-Length");
}

/// Setup the browser configuration.
///
/// Returns `Option<(Browser, Handler, Option<String>)>` where the third
/// element is the connected endpoint URL when one was used. The URL is
/// `Some(..)` for the multi-endpoint failover and single-remote paths
/// (so callers can later mark it bad on the failover) and `None` for
/// the local-launch path.
pub async fn setup_browser_configuration(
    config: &Configuration,
) -> Option<(Browser, chromiumoxide::Handler, Option<String>)> {
    let proxies = &config.proxies;

    // ── Multi-endpoint failover path (priority) ──
    if let Some(ref urls) = config.chrome_connection_urls {
        if !urls.is_empty() {
            let failover = config.chrome_failover.get_or_init(urls, 3);
            return failover
                .connect(config)
                .await
                .map(|(b, h, u)| (b, h, Some(u)));
        }
    }

    // ── Single-endpoint path (unchanged behavior) ──
    let chrome_connection = if config.chrome_connection_url.is_some() {
        config.chrome_connection_url.as_ref()
    } else {
        CHROM_BASE.as_ref()
    };

    match chrome_connection {
        Some(v) => {
            let mut attempts = 0;
            let max_retries = 10;
            let mut browser = None;

            // Attempt reconnections for instances that may be on load balancers (LBs)
            // experiencing shutdowns or degradation. This logic implements a retry
            // mechanism to improve robustness by allowing multiple attempts to establish.
            while attempts <= max_retries {
                match Browser::connect_with_config(v, create_handler_config(config)).await {
                    Ok(b) => {
                        browser = Some(b);
                        break;
                    }
                    Err(err) => {
                        log::error!("{:?}", err);
                        attempts += 1;
                        if attempts > max_retries {
                            log::error!("Exceeded maximum retry attempts");
                            break;
                        }
                        let backoff = crate::utils::backoff::backoff_delay(attempts, 100, 5_000);
                        tokio::time::sleep(backoff).await;
                    }
                }
            }

            browser.map(|(b, h)| (b, h, Some(v.clone())))
        }
        _ => match get_browser_config(
            proxies,
            config.chrome_intercept.enabled,
            config.cache,
            match config.viewport {
                Some(ref v) => Some(chromiumoxide::handler::viewport::Viewport::from(
                    v.to_owned(),
                )),
                _ => default_viewport(),
            },
            &config.request_timeout,
            config
                .remote_multimodal
                .as_ref()
                .map(|m| m.should_use_chrome_ai())
                .unwrap_or(false),
        ) {
            Some(mut browser_config) => {
                browser_config.ignore_visuals = config.chrome_intercept.block_visuals;
                browser_config.ignore_javascript = config.chrome_intercept.block_javascript;
                browser_config.ignore_ads = config.chrome_intercept.block_ads;
                browser_config.whitelist_patterns =
                    config.chrome_intercept.whitelist_patterns.clone();
                browser_config.blacklist_patterns =
                    config.chrome_intercept.blacklist_patterns.clone();
                browser_config.ignore_stylesheets = config.chrome_intercept.block_stylesheets;
                browser_config.ignore_analytics = config.chrome_intercept.block_analytics;
                browser_config.extra_headers = match &config.headers {
                    Some(headers) => {
                        let mut hm =
                            crate::utils::header_utils::header_map_to_hash_map(headers.inner());

                        cleanup_invalid_headers(&mut hm);

                        if hm.is_empty() {
                            None
                        } else {
                            if cfg!(feature = "real_browser") {
                                crate::utils::header_utils::rewrite_headers_to_title_case(&mut hm);
                            }
                            Some(hm)
                        }
                    }
                    _ => None,
                };
                browser_config.intercept_manager = config.chrome_intercept.intercept_manager;
                browser_config.only_html = config.only_html && !config.full_resources;

                match Browser::launch(browser_config).await {
                    Ok((browser, handler)) => Some((browser, handler, None)),
                    Err(e) => {
                        log::error!("Browser::launch() failed: {:?}", e);
                        None
                    }
                }
            }
            _ => None,
        },
    }
}

/// Launch a chromium browser with configurations and wait until the instance is up.
///
/// The fifth tuple element is the connected endpoint URL when one was
/// used (multi-endpoint failover or single remote endpoint) and `None`
/// when the browser was launched locally.
pub async fn launch_browser_base(
    config: &Configuration,
    url_parsed: &Option<Box<Url>>,
    jar: Option<&std::sync::Arc<crate::client::cookie::Jar>>,
) -> Option<(
    Browser,
    tokio::task::JoinHandle<()>,
    Option<BrowserContextId>,
    std::sync::Arc<std::sync::atomic::AtomicBool>,
    Option<String>,
)> {
    use chromiumoxide::{
        cdp::browser_protocol::target::CreateBrowserContextParams, error::CdpError,
    };

    let browser_configuration = setup_browser_configuration(config).await;

    match browser_configuration {
        Some(c) => {
            let (mut browser, mut handler, connected_url) = c;
            let mut context_id = None;

            let browser_dead = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let browser_dead_signal = browser_dead.clone();

            // Spawn a new task that continuously polls the handler
            // we might need a select with closing in case handler stalls.
            let handle = tokio::task::spawn(async move {
                while let Some(k) = handler.next().await {
                    if let Err(e) = k {
                        match e {
                            CdpError::Ws(_)
                            | CdpError::LaunchExit(_, _)
                            | CdpError::LaunchTimeout(_)
                            | CdpError::LaunchIo(_, _) => {
                                browser_dead_signal
                                    .store(true, std::sync::atomic::Ordering::Release);
                                log::error!("Browser handler fatal error: {:?}", e);
                                break;
                            }
                            _ => {
                                continue;
                            }
                        }
                    }
                }
                // Handler stream ended — browser is gone.
                browser_dead_signal.store(true, std::sync::atomic::Ordering::Release);
            });

            let mut create_content = CreateBrowserContextParams::default();
            create_content.dispose_on_detach = Some(true);

            if let Some(ref proxies) = config.proxies {
                let use_plain_http = proxies.len() >= 2;

                for proxie in proxies.iter() {
                    if proxie.ignore == crate::configuration::ProxyIgnore::Chrome {
                        continue;
                    }

                    let proxie = &proxie.addr;

                    if !proxie.is_empty() {
                        // pick the socks:// proxy over http if found.
                        if proxie.starts_with("socks://") {
                            create_content.proxy_server =
                                Some(proxie.replacen("socks://", "http://", 1));
                            // pref this connection
                            if use_plain_http {
                                break;
                            }
                        }

                        if *LOOP_BACK_PROXY && proxie.starts_with("http://localhost") {
                            create_content.proxy_bypass_list =
                                    // https://source.chromium.org/chromium/chromium/src/+/main:net/proxy_resolution/proxy_bypass_rules.cc
                                    Some("<-loopback>;localhost;[::1]".into());
                        }

                        create_content.proxy_server = Some(proxie.into());
                    }
                }
            }

            if let Ok(c) = browser.create_browser_context(create_content).await {
                let _ = browser.send_new_context(c.clone()).await;
                let _ = context_id.insert(c);
                if let Some(jar) = jar {
                    set_cookies(jar, config, url_parsed, &browser).await;
                }
                if let Some(id) = &browser.browser_context.id {
                    let cmd = SetDownloadBehaviorParamsBuilder::default();

                    if let Ok(cmd) = cmd
                        .behavior(SetDownloadBehaviorBehavior::Deny)
                        .events_enabled(false)
                        .browser_context_id(id.clone())
                        .build()
                    {
                        let _ = browser.execute(cmd).await;
                    }
                }
            } else {
                handle.abort();
            }

            Some((browser, handle, context_id, browser_dead, connected_url))
        }
        _ => None,
    }
}

/// Launch a chromium browser with configurations and wait until the instance is up.
pub async fn launch_browser(
    config: &Configuration,
    url_parsed: &Option<Box<Url>>,
) -> Option<(
    Browser,
    tokio::task::JoinHandle<()>,
    Option<BrowserContextId>,
    std::sync::Arc<std::sync::atomic::AtomicBool>,
    Option<String>,
)> {
    launch_browser_base(config, url_parsed, None).await
}

/// Launch a chromium browser with configurations and wait until the instance is up.
pub async fn launch_browser_cookies(
    config: &Configuration,
    url_parsed: &Option<Box<Url>>,
    jar: Option<&Arc<crate::client::cookie::Jar>>,
) -> Option<(
    Browser,
    tokio::task::JoinHandle<()>,
    Option<BrowserContextId>,
    std::sync::Arc<std::sync::atomic::AtomicBool>,
    Option<String>,
)> {
    launch_browser_base(config, url_parsed, jar).await
}

/// Represents IP-based geolocation and network metadata.
#[derive(Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct GeoInfo {
    /// The public IP address detected.
    pub ip: Option<String>,
    /// The CIDR network range of the IP.
    pub network: Option<String>,
    /// IP version (e.g., "IPv4" or "IPv6").
    pub version: Option<String>,
    /// The city associated with the IP.
    pub city: Option<String>,
    /// The region (e.g., state or province).
    pub region: Option<String>,
    /// Short regional code (e.g., "CA").
    pub region_code: Option<String>,
    /// Two-letter country code (e.g., "US").
    pub country: Option<String>,
    /// Full country name.
    pub country_name: Option<String>,
    /// Same as `country`, often redundant.
    pub country_code: Option<String>,
    /// ISO 3166-1 alpha-3 country code (e.g., "USA").
    pub country_code_iso3: Option<String>,
    /// Capital of the country.
    pub country_capital: Option<String>,
    /// Top-level domain of the country (e.g., ".us").
    pub country_tld: Option<String>,
    /// Continent code (e.g., "NA").
    pub continent_code: Option<String>,
    /// Whether the country is in the European Union.
    pub in_eu: Option<bool>,
    /// Postal or ZIP code.
    pub postal: Option<String>,
    /// Approximate latitude of the IP location.
    pub latitude: Option<f64>,
    /// Approximate longitude of the IP location.
    pub longitude: Option<f64>,
    /// Timezone identifier (e.g., "America/New_York").
    pub timezone: Option<String>,
    /// UTC offset string (e.g., "-0400").
    pub utc_offset: Option<String>,
    /// Country calling code (e.g., "+1").
    pub country_calling_code: Option<String>,
    /// ISO 4217 currency code (e.g., "USD").
    pub currency: Option<String>,
    /// Currency name (e.g., "Dollar").
    pub currency_name: Option<String>,
    /// Comma-separated preferred language codes.
    pub languages: Option<String>,
    /// Country surface area in square kilometers.
    pub country_area: Option<f64>,
    /// Approximate country population.
    pub country_population: Option<u64>,
    /// ASN (Autonomous System Number) of the IP.
    pub asn: Option<String>,
    /// ISP or organization name.
    pub org: Option<String>,
}

/// Auto-detect the geo-location.
#[cfg(feature = "serde")]
pub async fn detect_geo_info(new_page: &Page) -> Option<GeoInfo> {
    use rand::prelude::IndexedRandom;
    let apis = [
        "https://ipapi.co/json",
        "https://ipinfo.io/json",
        "https://ipwho.is/",
    ];

    let url = apis.choose(&mut rand::rng())?;

    new_page.goto(*url).await.ok()?;
    new_page.wait_for_navigation().await.ok()?;

    let html = new_page.content().await.ok()?;

    let json_start = html.find("<pre>")? + "<pre>".len();
    let json_end = html.find("</pre>")?;
    let json = html.get(json_start..json_end)?.trim();

    serde_json::from_str(json).ok()
}

#[cfg(not(feature = "serde"))]
/// Auto-detect the geo-location.
pub async fn detect_geo_info(new_page: &Page) -> Option<GeoInfo> {
    None
}

/// configure the browser.
pub async fn configure_browser(new_page: &Page, configuration: &Configuration) {
    let mut timezone = configuration.timezone_id.is_some();
    let mut locale = configuration.locale.is_some();

    let mut timezone_value = configuration.timezone_id.clone();
    let mut locale_value = configuration.locale.clone();

    let mut emulate_geolocation = None;

    // get the locale of the proxy.
    if configuration.auto_geolocation && configuration.proxies.is_some() && !timezone && !locale {
        if let Some(geo) = detect_geo_info(new_page).await {
            if let Some(languages) = geo.languages {
                if let Some(locale_v) = languages.split(',').next() {
                    if !locale_v.is_empty() {
                        locale_value = Some(Box::new(locale_v.into()));
                    }
                }
            }

            if let Some(timezone_v) = geo.timezone {
                if !timezone_v.is_empty() {
                    timezone_value = Some(Box::new(timezone_v));
                }
            }

            timezone = timezone_value.is_some();
            locale = locale_value.is_some();

            let mut geo_location_override = SetGeolocationOverrideParams::default();

            geo_location_override.latitude = geo.latitude;
            geo_location_override.longitude = geo.longitude;
            geo_location_override.accuracy = Some(0.7);

            emulate_geolocation = Some(geo_location_override);
        }
    }

    if timezone && locale {
        let geo = async {
            if let Some(geolocation) = emulate_geolocation {
                let _ = new_page.emulate_geolocation(geolocation).await;
            }
        };
        let timezone_id = async {
            if let Some(timezone_id) = timezone_value.as_deref() {
                if !timezone_id.is_empty() {
                    let _ = new_page
                    .emulate_timezone(
                        chromiumoxide::cdp::browser_protocol::emulation::SetTimezoneOverrideParams::new(
                            timezone_id,
                        ),
                    )
                    .await;
                }
            }
        };

        let locale = async {
            if let Some(locale) = locale_value.as_deref() {
                if !locale.is_empty() {
                    let _ = new_page
                        .emulate_locale(
                            chromiumoxide::cdp::browser_protocol::emulation::SetLocaleOverrideParams {
                                locale: Some(locale.into()),
                            },
                        )
                        .await;
                }
            }
        };

        tokio::join!(timezone_id, locale, geo);
    } else if timezone {
        if let Some(timezone_id) = timezone_value.as_deref() {
            if !timezone_id.is_empty() {
                let _ = new_page
                    .emulate_timezone(
                        chromiumoxide::cdp::browser_protocol::emulation::SetTimezoneOverrideParams::new(
                            timezone_id,
                        ),
                    )
                    .await;
            }
        }
    } else if locale {
        if let Some(locale) = locale_value.as_deref() {
            if !locale.is_empty() {
                let _ = new_page
                    .emulate_locale(
                        chromiumoxide::cdp::browser_protocol::emulation::SetLocaleOverrideParams {
                            locale: Some(locale.into()),
                        },
                    )
                    .await;
            }
        }
    }
}

/// attempt to navigate to a page respecting the request timeout. This will attempt to get a response for up to 60 seconds. There is a bug in the browser hanging if the CDP connection or handler errors. [https://github.com/mattsse/chromiumoxide/issues/64]
#[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
pub(crate) async fn attempt_navigation(
    url: &str,
    browser: &Browser,
    request_timeout: &Option<core::time::Duration>,
    browser_context_id: &Option<BrowserContextId>,
    viewport: &Option<crate::features::chrome_common::Viewport>,
) -> Result<Page, CdpError> {
    let mut cdp_params = CreateTargetParams::new(url);

    cdp_params.background = Some(browser_context_id.is_some()); // not supported headless-shell
    cdp_params.browser_context_id.clone_from(browser_context_id);
    cdp_params.for_tab = Some(false);

    if viewport.is_some() {
        browser
            .config()
            .and_then(|c| c.viewport.as_ref())
            .and_then(|b_vp| {
                viewport.as_ref().map(|vp| {
                    let new_viewport = b_vp.width == vp.width && b_vp.height == vp.height;

                    if !new_viewport {
                        if vp.width >= 25 {
                            cdp_params.width = Some(vp.width.into());
                        }
                        if vp.height >= 25 {
                            cdp_params.height = Some(vp.height.into());
                        }
                        cdp_params.new_window = Some(true);
                    }
                })
            });
    }

    let page_result = tokio::time::timeout(
        match request_timeout {
            Some(timeout) => *timeout,
            _ => tokio::time::Duration::from_secs(60),
        },
        browser.new_page(cdp_params),
    )
    .await;

    match page_result {
        Ok(page) => page,
        Err(_) => Err(CdpError::Timeout),
    }
}

/// close the browser and open handles
pub async fn close_browser(
    browser_handle: JoinHandle<()>,
    _browser: &Browser,
    _context_id: &mut Option<BrowserContextId>,
) {
    if !browser_handle.is_finished() {
        browser_handle.abort();
    }
}

/// Setup interception for auth challenges. This does nothing without the 'chrome_intercept' flag.
#[cfg(feature = "chrome")]
pub async fn setup_auth_challenge_response(
    page: &chromiumoxide::Page,
    chrome_intercept: bool,
    auth_challenge_response: &Option<crate::configuration::AuthChallengeResponse>,
) {
    if chrome_intercept {
        if let Some(ref auth_challenge_response) = auth_challenge_response {
            if let Ok(mut rp) = page
                .event_listener::<chromiumoxide::cdp::browser_protocol::fetch::EventAuthRequired>()
                .await
            {
                let intercept_page = page.clone();
                let auth_challenge_response = auth_challenge_response.clone();

                // we may need return for polling
                crate::utils::spawn_task("auth_interception", async move {
                    while let Some(event) = rp.next().await {
                        let u = &event.request.url;
                        let acr = chromiumoxide::cdp::browser_protocol::fetch::AuthChallengeResponse::from(auth_challenge_response.clone());

                        match chromiumoxide::cdp::browser_protocol::fetch::ContinueWithAuthParams::builder()
                        .request_id(event.request_id.clone())
                        .auth_challenge_response(acr)
                        .build() {
                            Ok(c) => {
                                if let Err(e) = intercept_page.send_command(c).await
                                {
                                    log("Failed to fullfill auth challege request: ", e.to_string());
                                }
                            }
                            _ => {
                                log("Failed to get auth challege request handle ", u);
                            }
                        }
                    }
                });
            }
        }
    }
}

/// Setup interception for chrome request. This does nothing without the 'chrome_intercept' flag.
#[cfg(feature = "chrome")]
pub async fn setup_chrome_interception_base(
    page: &chromiumoxide::Page,
    chrome_intercept: bool,
    auth_challenge_response: &Option<crate::configuration::AuthChallengeResponse>,
    _ignore_visuals: bool,
    _host_name: &str,
) -> Option<tokio::task::JoinHandle<()>> {
    if chrome_intercept {
        setup_auth_challenge_response(page, chrome_intercept, auth_challenge_response).await;
    }
    None
}

/// establish all the page events.
pub async fn setup_chrome_events(chrome_page: &chromiumoxide::Page, config: &Configuration) {
    let ua_opt = config.user_agent.as_deref().filter(|ua| !ua.is_empty());

    let ua_for_profiles: &str = ua_opt.map_or("", |v| v);

    let mut emulation_config =
        spider_fingerprint::EmulationConfiguration::setup_defaults(ua_for_profiles);

    let stealth_mode = config.stealth_mode;
    let use_stealth = stealth_mode.stealth();
    let block_ads = config.chrome_intercept.block_ads;

    emulation_config.dismiss_dialogs = config.dismiss_dialogs.unwrap_or(true);
    emulation_config.fingerprint = config.fingerprint;
    emulation_config.tier = stealth_mode;
    emulation_config.user_agent_data = Some(!ua_for_profiles.is_empty());

    let viewport = config.viewport.as_ref().map(|vp| (*vp).into());

    let gpu_profile = spider_fingerprint::profiles::gpu::select_random_gpu_profile(
        spider_fingerprint::get_agent_os(ua_for_profiles),
    );

    let merged_script = spider_fingerprint::emulate_with_profile(
        ua_for_profiles,
        &emulation_config,
        &viewport.as_ref(),
        &config.evaluate_on_new_document,
        gpu_profile,
    );

    let should_inject_script =
        (use_stealth || config.evaluate_on_new_document.is_some()) && merged_script.is_some();

    let hc: u32 = gpu_profile.hardware_concurrency.try_into().unwrap_or(8);

    let apply_page_setup = {
        async move {
            let f_script = async {
                if should_inject_script {
                    let _ = chrome_page
                        .add_script_to_evaluate_on_new_document(merged_script)
                        .await;
                }
            };

            let f_adblock = async {
                if block_ads {
                    let _ = chrome_page.set_ad_blocking_enabled(true).await;
                }
            };

            let f_ua = async {
                if !ua_for_profiles.is_empty() {
                    let _ = chrome_page.set_user_agent(ua_for_profiles).await;
                }
            };

            let f_hc = async {
                if use_stealth {
                    let _ = chrome_page.emulate_hardware_concurrency(hc.into()).await;
                }
            };

            tokio::join!(f_script, f_adblock, f_ua, f_hc);
        }
    };

    let disable_log = async {
        if config.disable_log {
            let _ = chrome_page.disable_log().await;
        }
    };

    let bypass_csp = async {
        if config.bypass_csp {
            let _ = chrome_page.set_bypass_csp(true).await;
        }
    };

    let disable_js = async {
        if config.disable_javascript {
            let _ = chrome_page
                .execute(SetScriptExecutionDisabledParams::new(true))
                .await;
        }
    };

    if tokio::time::timeout(tokio::time::Duration::from_secs(15), async {
        tokio::join!(
            apply_page_setup,
            disable_log,
            bypass_csp,
            disable_js,
            configure_browser(chrome_page, config),
        )
    })
    .await
    .is_err()
    {
        log::error!("failed to setup event handlers within 15 seconds.");
    }
}

pub(crate) type BrowserControl = (
    std::sync::Arc<chromiumoxide::Browser>,
    Option<tokio::task::JoinHandle<()>>,
    Option<chromiumoxide::cdp::browser_protocol::browser::BrowserContextId>,
);

/// Once cell browser
#[cfg(all(feature = "smart", not(feature = "decentralized")))]
pub(crate) type OnceBrowser = tokio::sync::OnceCell<Option<BrowserController>>;

/// Create the browser controller to auto drop connections.
pub struct BrowserController {
    /// The browser.
    pub browser: BrowserControl,
    /// Closed browser.
    pub closed: bool,
    /// Signal set by the handler task when the browser process dies or the
    /// WebSocket disconnects. Spawned page-fetch tasks should check this
    /// before creating new tabs to avoid wasting work on a dead browser.
    pub browser_dead: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// The endpoint URL this browser was connected to (multi-endpoint
    /// failover or single-remote path) or `None` for a local launch.
    /// Used by callers to mark the URL bad on the failover when the
    /// first-byte watchdog (or another health check) determines the
    /// backend has gone dark.
    pub connected_url: Option<String>,
}

impl BrowserController {
    /// A new browser controller.
    pub(crate) fn new(
        browser: BrowserControl,
        browser_dead: std::sync::Arc<std::sync::atomic::AtomicBool>,
        connected_url: Option<String>,
    ) -> Self {
        BrowserController {
            browser,
            closed: false,
            browser_dead,
            connected_url,
        }
    }
    /// Dispose the browser context and join handler.
    pub fn dispose(&mut self) {
        if !self.closed {
            self.closed = true;
            if let Some(handler) = self.browser.1.take() {
                handler.abort();
            }
        }
    }
}

impl Drop for BrowserController {
    fn drop(&mut self) {
        self.dispose();
    }
}

/// A lightweight second WebSocket connection to the same Chrome instance,
/// used for hedged requests. Owns its own handler task and cleans up on drop.
#[cfg(feature = "hedge")]
pub(crate) struct HedgeBrowser {
    pub browser: Browser,
    pub context_id: Option<BrowserContextId>,
    handler: Option<JoinHandle<()>>,
    /// The endpoint URL this hedge connected to. Used by the first-byte
    /// watchdog inside `chrome_page_fetch_on!` so a hedge timeout marks
    /// the *hedge's* URL bad on the failover, not the primary's.
    pub connected_url: Option<String>,
    /// Per-hedge `browser_dead` AtomicBool. Flipped by the hedge's own
    /// CDP handler on fatal WS / launch errors, mirroring the primary's
    /// `BrowserController.browser_dead` plumbing. Lets a hedge-specific
    /// failure surface independently of the primary.
    pub browser_dead: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

#[cfg(feature = "hedge")]
impl HedgeBrowser {
    /// Pure-data check: should the hedge escalate to a new WS connection?
    ///
    /// Uses signals already collected by HedgeTracker — zero extra latency,
    /// no CDP round-trips, no probes. The logic:
    ///
    /// - **browser_dead** → connection confirmed gone, must use new WS.
    /// - **3+ consecutive errors** → repeated failures point to a
    ///   connection-level problem, not just one slow page.
    /// - **hedge win rate > 60% with 8+ samples** → hedges consistently
    ///   beat the primary, suggesting systemic primary-path issues.
    ///
    /// Returns `false` (use cheap same-browser tab hedge) in all other
    /// cases — which is the overwhelmingly common path.
    #[inline]
    pub fn should_new_connection(
        tracker: &crate::utils::hedge::HedgeTracker,
        browser_dead: &std::sync::atomic::AtomicBool,
    ) -> bool {
        if browser_dead.load(std::sync::atomic::Ordering::Acquire) {
            return true;
        }
        if tracker.consecutive_errors() >= 3 {
            return true;
        }
        let fires = tracker.hedge_fires();
        fires >= 8 && tracker.hedge_win_rate_pct() > 60
    }

    /// Open a fresh WS connection for the hedge request.
    ///
    /// When `chrome_connection_urls` has a second entry, connects to that
    /// URL so the load balancer can route to a **different** backend
    /// instance.  Falls back to `chrome_connection_url` (re-enters the LB),
    /// then to the primary browser's direct websocket address.
    ///
    /// Captures the chosen URL on `connected_url` and creates a fresh
    /// `browser_dead` AtomicBool wired into the handler task — so when
    /// the first-byte watchdog fires inside a hedge fetch it marks the
    /// hedge's URL bad on the failover (not the primary's) and signals
    /// the hedge's own death flag (not the primary's).
    pub async fn connect(primary: &Browser, config: &Configuration) -> Option<Self> {
        let ws_url = config
            .chrome_connection_urls
            .as_ref()
            .and_then(|urls| urls.get(1).cloned())
            .or_else(|| config.chrome_connection_url.clone())
            .unwrap_or_else(|| primary.websocket_address().clone());

        let handler_config = create_handler_config(config);

        let (mut browser, mut handler) = match tokio::time::timeout(
            Duration::from_secs(10),
            Browser::connect_with_config(ws_url.clone(), handler_config),
        )
        .await
        {
            Ok(Ok(pair)) => pair,
            Ok(Err(e)) => {
                log::warn!(
                    "[hedge-chrome] failed to open second WS connection: {:?}",
                    e
                );
                return None;
            }
            Err(_) => {
                log::warn!("[hedge-chrome] second WS connection timed out (10s)");
                return None;
            }
        };

        let browser_dead = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let browser_dead_signal = browser_dead.clone();

        // Spawn handler — lightweight, just polls CDP messages.
        // Mirrors the primary's `launch_browser_base` handler pattern:
        // fatal CDP errors flip `browser_dead`; non-fatal continue. On
        // stream end, also flips dead since the WS is gone.
        let handle = tokio::task::spawn(async move {
            while let Some(k) = handler.next().await {
                if let Err(e) = k {
                    match e {
                        CdpError::Ws(_)
                        | CdpError::LaunchExit(_, _)
                        | CdpError::LaunchTimeout(_)
                        | CdpError::LaunchIo(_, _) => {
                            browser_dead_signal.store(true, std::sync::atomic::Ordering::Release);
                            break;
                        }
                        _ => continue,
                    }
                }
            }
            browser_dead_signal.store(true, std::sync::atomic::Ordering::Release);
        });

        // Create an isolated browser context so tabs don't collide with primary.
        let mut create_ctx =
            chromiumoxide::cdp::browser_protocol::target::CreateBrowserContextParams::default();
        create_ctx.dispose_on_detach = Some(true);

        let context_id = match browser.create_browser_context(create_ctx).await {
            Ok(id) => {
                let _ = browser.send_new_context(id.clone()).await;
                Some(id)
            }
            Err(e) => {
                log::debug!(
                    "[hedge-chrome] browser context creation failed (non-fatal): {:?}",
                    e
                );
                None
            }
        };

        Some(Self {
            browser,
            context_id,
            handler: Some(handle),
            connected_url: Some(ws_url),
            browser_dead,
        })
    }

    /// Async teardown: disposes the isolated browser context on the remote
    /// Chrome (freeing its storage + cookies) before aborting the CDP
    /// message pump and dropping the websocket. Callers should prefer this
    /// over relying on `Drop`, which only has a sync-path fallback and
    /// cannot issue CDP commands without the message pump alive.
    pub async fn close(mut self) {
        if let Some(ctx_id) = self.context_id.take() {
            let _ = tokio::time::timeout(
                Duration::from_secs(2),
                self.browser.dispose_browser_context(ctx_id),
            )
            .await;
        }
        if let Some(h) = self.handler.take() {
            h.abort();
        }
        // self.browser drops here — websocket closes, context (now already
        // disposed) cleanup is belt-and-suspenders.
    }
}

#[cfg(feature = "hedge")]
impl Drop for HedgeBrowser {
    fn drop(&mut self) {
        // Fallback path only: callers should prefer `close().await` so the
        // context is explicitly disposed on the remote browser. In Drop we
        // can't issue async CDP commands, so we rely on the experimental
        // `dispose_on_detach=true` set at context creation plus websocket
        // disconnect to clean the context up.
        if let Some(h) = self.handler.take() {
            h.abort();
        }
    }
}

/// Global sender for the background tab-closer task.  `TabCloseGuard::Drop`
/// pushes pages here instead of spawning a fresh task each time — under heavy
/// crawl cancellation we used to spawn thousands of single-use tasks per
/// second; one long-lived watcher amortises that to a single channel send.
///
/// Initialised once via `OnceLock` on the first `Drop` that needs to close a
/// tab.  Lives in a dedicated OS thread with its own `current_thread` runtime
/// so the watcher survives across tokio runtime lifetimes — important for
/// `#[tokio::test]` bodies that spin up + tear down a runtime per test, which
/// would otherwise silently lose closes after the first runtime shuts down.
#[cfg(all(feature = "chrome", not(feature = "decentralized")))]
static TAB_CLOSER_TX: std::sync::OnceLock<tokio::sync::mpsc::UnboundedSender<chromiumoxide::Page>> =
    std::sync::OnceLock::new();

/// Tracks `target_id`s currently queued for close so re-entrant Drop paths
/// don't hammer `page.close()` on the same tab twice.  Entries are removed
/// after the close future resolves (or times out).
#[cfg(all(feature = "chrome", not(feature = "decentralized")))]
static TAB_CLOSER_IN_FLIGHT: std::sync::OnceLock<
    dashmap::DashMap<chromiumoxide::cdp::browser_protocol::target::TargetId, ()>,
> = std::sync::OnceLock::new();

#[cfg(all(feature = "chrome", not(feature = "decentralized")))]
fn tab_closer_in_flight(
) -> &'static dashmap::DashMap<chromiumoxide::cdp::browser_protocol::target::TargetId, ()> {
    // Lock-free lazy init via get/set. `get_or_init` would block concurrent
    // first-callers inside std's internal mutex; with get/set, losers of the
    // atomic set() race simply drop their freshly-constructed DashMap (cheap)
    // and return the winner's map.
    if let Some(m) = TAB_CLOSER_IN_FLIGHT.get() {
        return m;
    }
    let _ = TAB_CLOSER_IN_FLIGHT.set(dashmap::DashMap::new());
    TAB_CLOSER_IN_FLIGHT
        .get()
        .expect("dedup map was just set on this path")
}

#[cfg(all(feature = "chrome", not(feature = "decentralized")))]
fn tab_closer() -> &'static tokio::sync::mpsc::UnboundedSender<chromiumoxide::Page> {
    // Fast path: already initialised — pure atomic load, no lock.
    if let Some(tx) = TAB_CLOSER_TX.get() {
        return tx;
    }

    // Slow path: lock-free lazy init. We create the channel unconditionally
    // (cheap), then atomically try to publish `tx`. Only the set() winner
    // spawns the watcher thread — losers drop their rx here with nothing
    // else to clean up. This avoids `get_or_init`'s internal Mutex and the
    // wasted thread spawns that a naïve get/set pattern would produce on
    // every concurrent first-call.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<chromiumoxide::Page>();

    match TAB_CLOSER_TX.set(tx) {
        Ok(()) => {
            // We won the race — spawn the watcher to service our rx.
            let spawn_result = std::thread::Builder::new()
                .name("spider-tab-closer".into())
                .spawn(move || {
                    let runtime = match tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                    {
                        Ok(rt) => rt,
                        Err(e) => {
                            log::error!("[tab-closer] runtime build failed: {:?}", e);
                            return;
                        }
                    };
                    // RAII cleanup that always removes the target_id from the
                    // in-flight dedup map on any exit path — success, timeout,
                    // panic, or task abort — so a close-task failure can never
                    // leave a stale entry that would permanently block re-queuing
                    // for the same tab.
                    struct DedupCleanup {
                        map: &'static dashmap::DashMap<
                            chromiumoxide::cdp::browser_protocol::target::TargetId,
                            (),
                        >,
                        key: Option<chromiumoxide::cdp::browser_protocol::target::TargetId>,
                    }
                    impl Drop for DedupCleanup {
                        fn drop(&mut self) {
                            if let Some(k) = self.key.take() {
                                self.map.remove(&k);
                            }
                        }
                    }

                    runtime.block_on(async move {
                        let in_flight = tab_closer_in_flight();
                        // Process closes concurrently on this runtime — close work
                        // is CDP round-trip bound (mostly awaiting a response), so
                        // a single-thread event loop with many in-flight tasks
                        // saturates throughput without multi-thread overhead.
                        let mut pending: tokio::task::JoinSet<()> = tokio::task::JoinSet::new();
                        loop {
                            tokio::select! {
                                biased;
                                // Reap completed closes to keep JoinSet bounded.
                                // We ignore the Result: task panics are logged by
                                // the JoinSet itself and the DedupCleanup Drop
                                // will have already freed the dedup entry.
                                _ = pending.join_next(), if !pending.is_empty() => {}
                                page = rx.recv() => {
                                    match page {
                                        Some(page) => {
                                            let target_id = page.target_id().clone();
                                            pending.spawn(async move {
                                                let _cleanup = DedupCleanup {
                                                    map: in_flight,
                                                    key: Some(target_id),
                                                };
                                                let _ = tokio::time::timeout(
                                                    tokio::time::Duration::from_secs(5),
                                                    page.close(),
                                                )
                                                .await;
                                                // _cleanup's Drop removes the
                                                // dedup entry here, regardless of
                                                // whether page.close returned Ok,
                                                // Err, or timed out.
                                            });
                                        }
                                        None => {
                                            // Sender is `static` — this only
                                            // fires at process exit; drain
                                            // in-flight closes before shutting
                                            // down. Each has a 5s timeout so
                                            // drain is bounded.
                                            while pending.join_next().await.is_some() {}
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                    });
                });

            if let Err(e) = spawn_result {
                log::error!(
                    "[tab-closer] thread spawn failed: {:?} — tab closes will be dropped",
                    e
                );
            }
        }
        Err(_our_tx) => {
            // Lost the atomic set() race. `_our_tx` drops at end of this arm,
            // our locally-owned `rx` drops when this function returns — neither
            // is referenced elsewhere, so nothing to tear down.
        }
    }

    TAB_CLOSER_TX
        .get()
        .expect("tab-closer sender was just published on this path")
}

/// Guard that closes a Chrome tab when dropped.
///
/// chromiumoxide's `Page` does **not** close the underlying Chrome tab on drop —
/// it only decrements an internal counter.  When `tokio::select!` cancels the
/// losing future during a hedge race, the tab stays open and keeps consuming
/// browser resources.  Over time the leaked tabs exhaust Chrome, causing
/// `browser.new_page()` to hang and deadlocking the crawl.
///
/// `TabCloseGuard` holds a clone of the `Page` handle.  On drop it hands the
/// page off to a background watcher (single OS thread + tokio runtime) via a
/// channel send — no per-Drop `tokio::spawn`.  The watcher dedups by
/// `target_id` so re-entrant cleanup paths don't double-close the same tab.
/// Call [`defuse`](Self::defuse) before an explicit `.close().await` to skip
/// the channel send entirely.
#[cfg(all(feature = "chrome", not(feature = "decentralized")))]
pub(crate) struct TabCloseGuard(Option<chromiumoxide::Page>);

#[cfg(all(feature = "chrome", not(feature = "decentralized")))]
impl TabCloseGuard {
    /// Create a guard that will close `page` on drop.
    #[inline]
    pub fn new(page: chromiumoxide::Page) -> Self {
        Self(Some(page))
    }

    /// Disarm the guard — the caller will close the tab explicitly.
    #[inline]
    pub fn defuse(mut self) {
        self.0 = None;
        // self is dropped here; Drop sees None → no-op.
    }
}

#[cfg(all(feature = "chrome", not(feature = "decentralized")))]
impl Drop for TabCloseGuard {
    fn drop(&mut self) {
        if let Some(page) = self.0.take() {
            // Skip if a close for this target_id is already queued — prevents
            // hammering page.close on the same tab when multiple guards
            // (or a defuse-followed-by-explicit-close path) race.
            let target_id = page.target_id().clone();
            if tab_closer_in_flight().insert(target_id, ()).is_none() {
                let _ = tab_closer().send(page);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_handler_config_max_redirects_defaults_to_none() {
        let cfg = Configuration::default();
        let hc = create_handler_config(&cfg);
        assert!(
            hc.max_redirects.is_none(),
            "without explicit opt-in, Chrome path must not cap redirects"
        );
    }

    #[test]
    fn test_handler_config_max_redirects_plumbs_opt_in_value() {
        let mut cfg = Configuration::default();
        cfg.with_redirect_limit(5);
        let hc = create_handler_config(&cfg);
        assert_eq!(hc.max_redirects, Some(5));
    }

    #[test]
    fn test_handler_config_max_redirects_disabled_by_policy_none() {
        let mut cfg = Configuration::default();
        cfg.with_redirect_limit(5);
        cfg.with_redirect_policy(RedirectPolicy::None);
        let hc = create_handler_config(&cfg);
        assert!(
            hc.max_redirects.is_none(),
            "RedirectPolicy::None must disable the Chrome cap too, matching HTTP semantics"
        );
    }

    #[test]
    fn test_handler_config_max_main_frame_navigations_defaults_to_none() {
        let cfg = Configuration::default();
        let hc = create_handler_config(&cfg);
        assert!(
            hc.max_main_frame_navigations.is_none(),
            "default Configuration must not cap main-frame navigations"
        );
    }

    #[test]
    fn test_handler_config_max_main_frame_navigations_plumbs_opt_in_value() {
        let mut cfg = Configuration::default();
        cfg.with_max_main_frame_navigations(Some(20));
        let hc = create_handler_config(&cfg);
        assert_eq!(hc.max_main_frame_navigations, Some(20));
    }

    #[test]
    fn test_lazy_chrome_failover_is_init_once_per_cell() {
        let cell = LazyChromeFailover::default();
        let urls = vec!["ws://a".to_string(), "ws://b".to_string()];
        let a = std::sync::Arc::clone(cell.get_or_init(&urls, 3));
        let b = std::sync::Arc::clone(cell.get_or_init(&urls, 3));
        assert!(
            std::sync::Arc::ptr_eq(&a, &b),
            "second get_or_init must return the same Arc — closure ran twice"
        );
    }

    #[test]
    fn test_lazy_chrome_failover_clone_shares_state() {
        let cell = LazyChromeFailover::default();
        let clone = cell.clone();
        let urls = vec!["ws://a".to_string(), "ws://b".to_string()];
        let from_orig = std::sync::Arc::clone(cell.get_or_init(&urls, 3));
        let from_clone = std::sync::Arc::clone(clone.get_or_init(&urls, 3));
        assert!(
            std::sync::Arc::ptr_eq(&from_orig, &from_clone),
            "clones share the OnceLock — second init must observe the first's value"
        );
    }

    #[test]
    fn test_with_chrome_connections_resets_lazy_failover() {
        let mut cfg = Configuration::default();
        cfg.with_chrome_connections(vec!["ws://a".into(), "ws://b".into()]);
        let first = std::sync::Arc::clone(
            cfg.chrome_failover
                .get_or_init(cfg.chrome_connection_urls.as_deref().unwrap(), 3),
        );
        cfg.with_chrome_connections(vec!["ws://c".into(), "ws://d".into()]);
        let second = std::sync::Arc::clone(
            cfg.chrome_failover
                .get_or_init(cfg.chrome_connection_urls.as_deref().unwrap(), 3),
        );
        assert!(
            !std::sync::Arc::ptr_eq(&first, &second),
            "URL change must replace the cell — fresh failover required"
        );
    }

    #[test]
    fn test_chrome_first_byte_timeout_defaults_off() {
        let cfg = Configuration::default();
        assert!(
            cfg.chrome_first_byte_timeout.is_none(),
            "watchdog must be off by default to keep behavior identical"
        );
        let params = cfg.chrome_fetch_params();
        assert!(params.first_byte_timeout.is_none());
        assert!(
            params.browser_dead.is_none(),
            "no browser_dead should be plumbed through Configuration::chrome_fetch_params"
        );
    }

    #[test]
    fn test_with_chrome_first_byte_timeout_plumbs_into_params() {
        let mut cfg = Configuration::default();
        cfg.with_chrome_first_byte_timeout(Some(std::time::Duration::from_secs(5)));
        let params = cfg.chrome_fetch_params();
        assert_eq!(
            *params.first_byte_timeout,
            Some(std::time::Duration::from_secs(5))
        );
    }

    #[test]
    fn test_chrome_fetch_params_with_browser_dead_attaches_signal() {
        let cfg = Configuration::default();
        let dead = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let params = cfg.chrome_fetch_params().with_browser_dead(&dead);
        assert!(params.browser_dead.is_some());
        assert!(std::sync::Arc::ptr_eq(params.browser_dead.unwrap(), &dead));
    }

    #[test]
    fn test_chrome_fetch_params_with_chrome_endpoint_attaches_url() {
        let cfg = Configuration::default();
        let url = "ws://b".to_string();
        let params = cfg.chrome_fetch_params().with_chrome_endpoint(Some(&url));
        assert_eq!(params.chrome_endpoint_url, Some("ws://b"));
    }

    #[test]
    fn test_chrome_failover_mark_url_bad_records_cooldown() {
        let urls = vec!["ws://a".to_string(), "ws://b".to_string()];
        let failover = ChromeConnectionFailover::new(urls, 3);
        // Pre-condition: both endpoints healthy.
        assert_eq!(
            failover.dead_until[0].load(std::sync::atomic::Ordering::Acquire),
            0
        );
        assert_eq!(
            failover.dead_until[1].load(std::sync::atomic::Ordering::Acquire),
            0
        );

        failover.mark_url_bad("ws://b", 30_000);
        assert_eq!(
            failover.dead_until[0].load(std::sync::atomic::Ordering::Acquire),
            0
        );
        let until_b = failover.dead_until[1].load(std::sync::atomic::Ordering::Acquire);
        assert!(until_b > 0, "ws://b should have a cooldown set");
        let now = ChromeConnectionFailover::now_millis();
        assert!(
            until_b > now,
            "cooldown deadline ({}) must be in the future relative to now ({})",
            until_b,
            now
        );
    }

    #[test]
    fn test_chrome_failover_mark_url_bad_unknown_url_no_op() {
        let urls = vec!["ws://a".to_string()];
        let failover = ChromeConnectionFailover::new(urls, 3);
        // Should not panic, should not modify any slot.
        failover.mark_url_bad("ws://does-not-exist", 30_000);
        assert_eq!(
            failover.dead_until[0].load(std::sync::atomic::Ordering::Acquire),
            0
        );
    }

    #[test]
    fn test_chrome_failover_mark_url_bad_zero_cooldown_no_op() {
        let urls = vec!["ws://a".to_string()];
        let failover = ChromeConnectionFailover::new(urls, 3);
        failover.mark_url_bad("ws://a", 0);
        assert_eq!(
            failover.dead_until[0].load(std::sync::atomic::Ordering::Acquire),
            0
        );
    }

    #[test]
    fn test_lazy_chrome_failover_mark_url_bad_no_op_when_uninit() {
        // Cell hasn't been initialized — mark_url_bad must be a no-op
        // (no panic, no allocation).
        let cell = LazyChromeFailover::default();
        cell.mark_url_bad("ws://a", 30_000);
    }

    #[test]
    fn test_lazy_chrome_failover_mark_url_bad_propagates_after_init() {
        let cell = LazyChromeFailover::default();
        let urls = vec!["ws://a".to_string(), "ws://b".to_string()];
        let failover = std::sync::Arc::clone(cell.get_or_init(&urls, 3));
        cell.mark_url_bad("ws://a", 30_000);
        let until_a = failover.dead_until[0].load(std::sync::atomic::Ordering::Acquire);
        assert!(
            until_a > 0,
            "mark via the cell must propagate to the inner failover"
        );
    }

    #[test]
    fn test_chrome_failover_last_connected_url_starts_none() {
        let urls = vec!["ws://a".to_string(), "ws://b".to_string()];
        let failover = ChromeConnectionFailover::new(urls, 3);
        assert!(
            failover.last_connected_url().is_none(),
            "no connect attempted yet — must report None"
        );
    }

    #[test]
    fn test_chrome_failover_last_connected_url_records_idx() {
        let urls = vec!["ws://a".to_string(), "ws://b".to_string()];
        let failover = ChromeConnectionFailover::new(urls, 3);
        // Simulate a successful connect at idx 1.
        failover
            .last_idx
            .store(1, std::sync::atomic::Ordering::Release);
        assert_eq!(failover.last_connected_url(), Some("ws://b"));
    }

    #[test]
    fn test_chrome_fetch_params_auto_populates_endpoint_from_single_url() {
        let mut cfg = Configuration::default();
        cfg.with_chrome_connection(Some("ws://lone".to_string()));
        let params = cfg.chrome_fetch_params();
        assert_eq!(params.chrome_endpoint_url, Some("ws://lone"));
    }

    #[test]
    fn test_chrome_fetch_params_auto_populates_endpoint_from_failover() {
        let mut cfg = Configuration::default();
        cfg.with_chrome_connections(vec!["ws://a".into(), "ws://b".into()]);
        // Force-init the failover and mark idx 1 as last-successful.
        let failover = std::sync::Arc::clone(
            cfg.chrome_failover
                .get_or_init(cfg.chrome_connection_urls.as_deref().unwrap(), 3),
        );
        failover
            .last_idx
            .store(1, std::sync::atomic::Ordering::Release);
        let params = cfg.chrome_fetch_params();
        assert_eq!(params.chrome_endpoint_url, Some("ws://b"));
    }

    #[test]
    fn test_chrome_fetch_params_endpoint_none_when_no_chrome_remote() {
        let cfg = Configuration::default();
        let params = cfg.chrome_fetch_params();
        assert!(
            params.chrome_endpoint_url.is_none(),
            "no remote URL configured — must be None"
        );
    }

    #[test]
    fn test_chrome_first_byte_timeout_jitter_defaults_off() {
        let cfg = Configuration::default();
        assert!(cfg.chrome_first_byte_timeout_jitter.is_none());
        let params = cfg.chrome_fetch_params();
        assert!(
            params.first_byte_timeout_jitter.is_none(),
            "jitter must be None by default"
        );
    }

    #[test]
    fn test_with_chrome_first_byte_timeout_jitter_plumbs_into_params() {
        let mut cfg = Configuration::default();
        cfg.with_chrome_first_byte_timeout(Some(std::time::Duration::from_secs(5)));
        cfg.with_chrome_first_byte_timeout_jitter(Some(std::time::Duration::from_secs(2)));
        let params = cfg.chrome_fetch_params();
        assert_eq!(
            *params.first_byte_timeout,
            Some(std::time::Duration::from_secs(5))
        );
        assert_eq!(
            *params.first_byte_timeout_jitter,
            Some(std::time::Duration::from_secs(2))
        );
    }

    /// The jitter helper used inside `fetch_page_html_chrome_base`:
    /// given a base + jitter, the resulting timeout must always lie in
    /// `[base, base + jitter)`. Mirrors the actual computation done at
    /// the watchdog set-up site so a regression there is caught here.
    #[test]
    fn test_jitter_window_bounds() {
        let base = std::time::Duration::from_millis(5_000);
        let jitter = std::time::Duration::from_millis(2_000);
        for _ in 0..1024 {
            let j_ms = jitter.as_millis() as u64;
            let extra = if j_ms > 0 { fastrand::u64(0..j_ms) } else { 0 };
            let actual = base.saturating_add(std::time::Duration::from_millis(extra));
            assert!(
                actual >= base,
                "jittered timeout must be >= base (got {actual:?} vs base {base:?})"
            );
            assert!(
                actual < base + jitter,
                "jittered timeout must be < base + jitter (got {actual:?} vs cap {:?})",
                base + jitter
            );
        }
    }

    #[test]
    fn test_jitter_zero_window_is_noop() {
        // Zero jitter → no random call, deterministic timeout = base.
        let base = std::time::Duration::from_millis(5_000);
        let jitter = std::time::Duration::from_millis(0);
        let j_ms = jitter.as_millis() as u64;
        let extra = if j_ms > 0 { fastrand::u64(0..j_ms) } else { 0 };
        let actual = base.saturating_add(std::time::Duration::from_millis(extra));
        assert_eq!(actual, base);
    }

    /// Repeated `mark_url_bad` on the same URL must update the deadline
    /// (last write wins), not stack — otherwise an aggressive caller
    /// could drift the cooldown past `u64::MAX`.
    #[test]
    fn test_mark_url_bad_repeated_updates_deadline() {
        let urls = vec!["ws://a".to_string()];
        let failover = ChromeConnectionFailover::new(urls, 3);

        failover.mark_url_bad("ws://a", 1_000);
        let after_first = failover.dead_until[0].load(std::sync::atomic::Ordering::Acquire);
        assert!(after_first > 0);

        // Mark again with a much longer cooldown — should overwrite.
        failover.mark_url_bad("ws://a", 60_000);
        let after_second = failover.dead_until[0].load(std::sync::atomic::Ordering::Acquire);
        assert!(
            after_second > after_first,
            "second mark_url_bad with longer cooldown must extend the deadline (got {} → {})",
            after_first,
            after_second
        );

        // And shorter — should overwrite (last write wins).
        failover.mark_url_bad("ws://a", 100);
        let after_third = failover.dead_until[0].load(std::sync::atomic::Ordering::Acquire);
        assert!(
            after_third < after_second,
            "shorter mark_url_bad must overwrite the deadline (no stacking) — got {} → {}",
            after_second,
            after_third
        );
    }

    /// After a real cooldown elapses, `dead_until` is "in the past" so
    /// `connect()`'s pass-0 cooldown check (`until > now`) lets the
    /// endpoint through. Validates the deadline math actually expires.
    #[test]
    fn test_cooldown_deadline_elapses_in_real_time() {
        let urls = vec!["ws://a".to_string()];
        let failover = ChromeConnectionFailover::new(urls, 3);
        // 50ms cooldown — short enough to elapse during the test.
        failover.mark_url_bad("ws://a", 50);
        let until = failover.dead_until[0].load(std::sync::atomic::Ordering::Acquire);
        let before = ChromeConnectionFailover::now_millis();
        assert!(
            until > before,
            "deadline must be in the future immediately after mark"
        );
        std::thread::sleep(std::time::Duration::from_millis(120));
        let after = ChromeConnectionFailover::now_millis();
        assert!(
            until < after,
            "after sleeping past the cooldown, deadline must be in the past — \
             until={until}ms, now_after_sleep={after}ms"
        );
    }

    /// HTTP first-byte deadline: with no base timeout, returns None.
    #[test]
    fn test_http_first_byte_deadline_none_when_base_unset() {
        let d =
            crate::utils::first_byte_deadline(None, Some(std::time::Duration::from_secs(5)));
        assert_eq!(d, None);
    }

    /// HTTP first-byte deadline: with base + zero/None jitter, returns base.
    #[test]
    fn test_http_first_byte_deadline_base_only_returns_base() {
        let base = std::time::Duration::from_secs(8);
        let d = crate::utils::first_byte_deadline(Some(base), None);
        assert_eq!(d, Some(base));
        let d2 = crate::utils::first_byte_deadline(
            Some(base),
            Some(std::time::Duration::from_secs(0)),
        );
        assert_eq!(d2, Some(base));
    }

    /// HTTP first-byte deadline: with base + jitter, must lie in
    /// `[base, base + jitter)`.
    #[test]
    fn test_http_first_byte_deadline_jitter_window_bounds() {
        let base = std::time::Duration::from_secs(8);
        let jitter = std::time::Duration::from_secs(2);
        for _ in 0..1024 {
            let d = crate::utils::first_byte_deadline(Some(base), Some(jitter)).unwrap();
            assert!(d >= base, "jittered deadline must be >= base");
            assert!(
                d < base + jitter,
                "jittered deadline must be < base + jitter"
            );
        }
    }

    /// HTTP first-byte page response builder must produce a 524.
    #[test]
    fn test_http_build_first_byte_timeout_page_response_status_524() {
        let pr = crate::utils::build_first_byte_timeout_page_response("https://x.example/");
        assert_eq!(pr.status_code, reqwest::StatusCode::GATEWAY_TIMEOUT);
        assert_eq!(pr.final_url, Some("https://x.example/".to_string()));
    }
}
