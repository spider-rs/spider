use crate::utils::log;
use crate::{configuration::Configuration, tokio_stream::StreamExt};
use chromiumoxide::cdp::browser_protocol::{
    browser::BrowserContextId, network::CookieParam, target::CreateTargetParams,
};
use chromiumoxide::error::CdpError;
use chromiumoxide::page::DISABLE_DIALOGS;
use chromiumoxide::Page;
use chromiumoxide::{handler::HandlerConfig, Browser, BrowserConfig};
use reqwest::cookie::{CookieStore, Jar};
use tokio::task::JoinHandle;
use url::Url;

lazy_static! {
    /// Enable loopback for proxy.
    static ref LOOP_BACK_PROXY: bool = std::env::var("LOOP_BACK_PROXY").unwrap_or_default() == "true";
}

/// parse a cookie into a jar
pub fn parse_cookies_with_jar(cookie_str: &str, url: &Url) -> Result<Vec<CookieParam>, String> {
    let jar = Jar::default();

    jar.add_cookie_str(cookie_str, url);

    // Retrieve cookies stored in the jar
    if let Some(header_value) = jar.cookies(url) {
        let cookie_header_str = header_value.to_str().map_err(|e| e.to_string())?;
        let cookie_pairs: Vec<&str> = cookie_header_str.split(';').collect();

        let mut cookies = Vec::new();

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

/// get chrome configuration
#[cfg(not(feature = "chrome_headed"))]
pub fn get_browser_config(
    proxies: &Option<Box<Vec<crate::configuration::RequestProxy>>>,
    intercept: bool,
    cache_enabled: bool,
    viewport: impl Into<Option<chromiumoxide::handler::viewport::Viewport>>,
    request_timeout: &Option<Box<core::time::Duration>>,
) -> Option<BrowserConfig> {
    let builder = BrowserConfig::builder()
        .disable_default_args()
        .request_timeout(match request_timeout.as_ref() {
            Some(timeout) => **timeout,
            _ => Default::default(),
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
            let mut chrome_args = Vec::from(CHROME_ARGS.map(|e| e.replace("://", "=").to_string()));
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
        _ => builder.args(CHROME_ARGS),
    };
    let builder = if std::env::var("CHROME_BIN").is_ok() {
        match std::env::var("CHROME_BIN") {
            Ok(v) => builder.chrome_executable(v),
            _ => builder,
        }
    } else {
        builder
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
    proxies: &Option<Box<Vec<crate::configuration::RequestProxy>>>,
    intercept: bool,
    cache_enabled: bool,
    viewport: impl Into<Option<chromiumoxide::handler::viewport::Viewport>>,
    request_timeout: &Option<Box<core::time::Duration>>,
) -> Option<BrowserConfig> {
    let builder = BrowserConfig::builder()
        .disable_default_args()
        .no_sandbox()
        .request_timeout(match request_timeout.as_ref() {
            Some(timeout) => **timeout,
            _ => Default::default(),
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
            "".to_string()
        } else {
            e.replace("://", "=").to_string()
        }
    }));

    let builder = match proxies {
        Some(proxies) => {
            chrome_args.push(string_concat!(r#"--proxy-server="#, proxies.join(";")));

            builder.args(chrome_args)
        }
        _ => builder.args(chrome_args),
    };
    let builder = if std::env::var("CHROME_BIN").is_ok() {
        match std::env::var("CHROME_BIN") {
            Ok(v) => builder.chrome_executable(v),
            _ => builder,
        }
    } else {
        builder
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
fn create_handler_config(config: &Configuration) -> HandlerConfig {
    HandlerConfig {
        request_timeout: match config.request_timeout.as_ref() {
            Some(timeout) => **timeout,
            _ => Default::default(),
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
        ignore_ads: config.chrome_intercept.block_ads,
        ignore_javascript: config.chrome_intercept.block_javascript,
        ignore_analytics: config.chrome_intercept.block_analytics,
        ignore_stylesheets: config.chrome_intercept.block_stylesheets,
        extra_headers: match config.headers {
            Some(ref headers) => {
                let hm = crate::utils::header_utils::header_map_to_hash_map(headers.inner());
                if hm.is_empty() {
                    None
                } else {
                    Some(hm)
                }
            }
            _ => None,
        },
        intercept_manager: config.chrome_intercept.intercept_manager,
        only_html: config.only_html && !config.full_resources,
        ..HandlerConfig::default()
    }
}

lazy_static! {
    static ref CHROM_BASE: Option<String> = std::env::var("CHROME_URL").ok();
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

/// Setup the browser configuration.
pub async fn setup_browser_configuration(
    config: &Configuration,
) -> Option<(Browser, chromiumoxide::Handler)> {
    let proxies = &config.proxies;

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
                match Browser::connect_with_config(&*v, create_handler_config(&config)).await {
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
                    }
                }
            }

            browser
        }
        _ => match get_browser_config(
            &proxies,
            config.chrome_intercept.enabled,
            config.cache,
            match config.viewport {
                Some(ref v) => Some(chromiumoxide::handler::viewport::Viewport::from(
                    v.to_owned(),
                )),
                _ => default_viewport(),
            },
            &config.request_timeout,
        ) {
            Some(mut browser_config) => {
                browser_config.ignore_visuals = config.chrome_intercept.block_visuals;
                browser_config.ignore_javascript = config.chrome_intercept.block_javascript;
                browser_config.ignore_ads = config.chrome_intercept.block_ads;
                browser_config.ignore_stylesheets = config.chrome_intercept.block_stylesheets;
                browser_config.ignore_analytics = config.chrome_intercept.block_analytics;
                browser_config.extra_headers = match config.headers {
                    Some(ref headers) => {
                        let hm =
                            crate::utils::header_utils::header_map_to_hash_map(headers.inner());
                        if hm.is_empty() {
                            None
                        } else {
                            Some(hm)
                        }
                    }
                    _ => None,
                };
                browser_config.intercept_manager = config.chrome_intercept.intercept_manager;
                browser_config.only_html = config.only_html && !config.full_resources;

                match Browser::launch(browser_config).await {
                    Ok(browser) => Some(browser),
                    _ => None,
                }
            }
            _ => None,
        },
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
)> {
    use chromiumoxide::{
        cdp::browser_protocol::target::CreateBrowserContextParams, error::CdpError,
    };

    let browser_configuration = setup_browser_configuration(&config).await;

    match browser_configuration {
        Some(c) => {
            let (mut browser, mut handler) = c;
            let mut context_id = handler.default_browser_context().id().cloned();

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
                                break;
                            }
                            _ => {
                                continue;
                            }
                        }
                    }
                }
            });

            if !context_id.is_some() {
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
                                    Some(proxie.replacen("socks://", "http://", 1).into());
                                // pref this connection
                                if use_plain_http {
                                    break;
                                }
                            }

                            if *LOOP_BACK_PROXY && proxie.starts_with("http://localhost") {
                                create_content.proxy_bypass_list =
                                    Some("<-loopback>;about:blank".into());
                            }

                            create_content.proxy_server = Some(proxie.into());
                        }
                    }
                }

                if let Ok(c) = browser.create_browser_context(create_content).await {
                    let _ = browser.send_new_context(c.clone()).await;
                    let _ = context_id.insert(c);
                    if !config.cookie_str.is_empty() {
                        if let Some(parsed) = url_parsed {
                            let cookies = parse_cookies_with_jar(&config.cookie_str, &*parsed);
                            if let Ok(co) = cookies {
                                let _ = browser.set_cookies(co).await;
                            };
                        };
                    }
                } else {
                    handle.abort();
                }
            }

            Some((browser, handle, context_id))
        }
        _ => None,
    }
}

/// configure the browser
pub async fn configure_browser(new_page: &Page, configuration: &Configuration) {
    let timezone_id = async {
        if let Some(timezone_id) = configuration.timezone_id.as_deref() {
            let _ = new_page
                .emulate_timezone(
                    chromiumoxide::cdp::browser_protocol::emulation::SetTimezoneOverrideParams::new(
                        timezone_id,
                    ),
                )
                .await;
        }
    };

    let locale = async {
        if let Some(locale) = configuration.locale.as_deref() {
            let _ = new_page
                .emulate_locale(
                    chromiumoxide::cdp::browser_protocol::emulation::SetLocaleOverrideParams {
                        locale: Some(locale.into()),
                    },
                )
                .await;
        }
    };

    tokio::join!(timezone_id, locale);
}

/// attempt to navigate to a page respecting the request timeout. This will attempt to get a response for up to 60 seconds. There is a bug in the browser hanging if the CDP connection or handler errors. [https://github.com/mattsse/chromiumoxide/issues/64]
#[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
pub(crate) async fn attempt_navigation(
    url: &str,
    browser: &Browser,
    request_timeout: &Option<Box<core::time::Duration>>,
    browser_context_id: &Option<BrowserContextId>,
    viewport: &Option<crate::features::chrome_common::Viewport>,
) -> Result<Page, CdpError> {
    let mut cdp_params = CreateTargetParams::new(url);

    cdp_params.background = Some(browser_context_id.is_some()); // not supported headless-shell
    cdp_params.browser_context_id.clone_from(browser_context_id);
    cdp_params.url = url.into();
    cdp_params.for_tab = Some(false);

    if let Some(vp) = viewport {
        if vp.width >= 25 {
            cdp_params.width = Some(vp.width.into());
        }
        if vp.height >= 25 {
            cdp_params.height = Some(vp.height.into());
        }
    }

    let page_result = tokio::time::timeout(
        match request_timeout {
            Some(timeout) => **timeout,
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
#[cfg(all(feature = "chrome"))]
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
                                if let Err(e) = intercept_page.execute(c).await
                                {
                                    log("Failed to fullfill auth challege request: ", e.to_string());
                                }
                            }
                            _ => {
                                log("Failed to get auth challege request handle ", &u);
                            }
                        }
                    }
                });
            }
        }
    }
}

/// Setup interception for chrome request. This does nothing without the 'chrome_intercept' flag.
#[cfg(all(feature = "chrome"))]
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
    let stealth_mode = cfg!(feature = "chrome_stealth") || config.stealth_mode;
    let dismiss_dialogs = config.dismiss_dialogs.unwrap_or(true); // polyfill window.alert.

    let stealth = async {
        if stealth_mode {
            match config.user_agent.as_ref() {
                Some(agent) => {
                    let _ = if dismiss_dialogs {
                        chrome_page
                            .enable_stealth_mode_with_agent_and_dimiss_dialogs(agent)
                            .await
                    } else {
                        chrome_page.enable_stealth_mode_with_agent(agent).await
                    };
                }
                _ => {
                    let _ = chrome_page.enable_stealth_mode().await;
                }
            }
        }
    };

    let eval_docs = async {
        match config.evaluate_on_new_document {
            Some(ref script) => {
                if config.fingerprint {
                    let _ = chrome_page
                        .evaluate_on_new_document(string_concat!(
                            crate::features::chrome::FP_JS,
                            script.as_str(),
                            if dismiss_dialogs && !stealth_mode {
                                DISABLE_DIALOGS
                            } else {
                                ""
                            }
                        ))
                        .await;
                } else {
                    let _ = chrome_page
                        .evaluate_on_new_document(string_concat!(
                            script.as_str(),
                            if dismiss_dialogs && !stealth_mode {
                                DISABLE_DIALOGS
                            } else {
                                ""
                            }
                        ))
                        .await;
                }
            }
            _ => {
                if config.fingerprint {
                    let _ = chrome_page
                        .evaluate_on_new_document(string_concat!(
                            crate::features::chrome::FP_JS,
                            if dismiss_dialogs && !stealth_mode {
                                DISABLE_DIALOGS
                            } else {
                                ""
                            }
                        ))
                        .await;
                }
            }
        }
    };

    if let Err(_) = tokio::time::timeout(tokio::time::Duration::from_secs(10), async {
        tokio::join!(stealth, eval_docs, configure_browser(&chrome_page, &config))
    })
    .await
    {
        log::error!("failed to setup event handlers within 10 seconds.");
    }
}

pub(crate) type BrowserControl = (
    std::sync::Arc<chromiumoxide::Browser>,
    Option<tokio::task::JoinHandle<()>>,
    Option<chromiumoxide::cdp::browser_protocol::browser::BrowserContextId>,
);

/// Once cell browser
#[cfg(feature = "smart")]
pub(crate) type OnceBrowser = tokio::sync::OnceCell<Option<BrowserController>>;

/// Create the browser controller to auto drop connections.
pub(crate) struct BrowserController {
    /// The browser.
    pub browser: BrowserControl,
    /// Closed browser.
    pub closed: bool,
}

impl BrowserController {
    /// A new browser controller.
    pub(crate) fn new(browser: BrowserControl) -> Self {
        BrowserController {
            browser,
            closed: false,
        }
    }

    /// Dispose the browser context and join handler.
    pub(crate) async fn dispose(&mut self) {
        if !self.closed {
            // assume close will always happen.
            self.closed = true;
            if let Some(id) = self.browser.2.take() {
                if let Some(handler) = self.browser.1.take() {
                    let _ = self.browser.0.quit_incognito_context_base(id).await;
                    handler.abort();
                }
            }
        }
    }
}

impl Drop for BrowserController {
    fn drop(&mut self) {
        if !self.closed {
            self.closed = true;
            if let Some(id) = self.browser.2.take() {
                if let Some(handler) = self.browser.1.take() {
                    let browser = self.browser.0.to_owned();
                    tokio::task::spawn(async move {
                        let _ = browser.quit_incognito_context_base(id).await;
                        handler.abort();
                    });
                }
            }
        }
    }
}

/// static chrome arguments to start
#[cfg(all(feature = "chrome_cpu", feature = "real_browser"))]
pub static CHROME_ARGS: [&'static str; 27] = [
    if cfg!(feature = "chrome_headless_new") {
        "--headless=new"
    } else {
        "--headless"
    },
    "--disable-extensions",
    "--disable-component-extensions-with-background-pages",
    "--disable-background-networking",
    "--disable-component-update",
    "--disable-client-side-phishing-detection",
    "--disable-sync",
    "--metrics-recording-only",
    "--disable-default-apps",
    "--mute-audio",
    "--no-default-browser-check",
    "--no-first-run",
    "--disable-gpu",
    "--disable-gpu-sandbox",
    "--disable-setuid-sandbox",
    "--disable-dev-shm-usage",
    "--disable-backgrounding-occluded-windows",
    "--disable-renderer-backgrounding",
    "--disable-background-timer-throttling",
    "--disable-ipc-flooding-protection",
    "--password-store=basic",
    "--use-mock-keychain",
    "--force-fieldtrials=*BackgroundTracing/default/",
    "--disable-hang-monitor",
    "--disable-prompt-on-repost",
    "--disable-domain-reliability",
    "--disable-features=InterestFeedContentSuggestions,PrivacySandboxSettings4,AutofillServerCommunication,CalculateNativeWinOcclusion,OptimizationHints,AudioServiceOutOfProcess,IsolateOrigins,site-per-process,ImprovedCookieControls,LazyFrameLoading,GlobalMediaControls,DestroyProfileOnBrowserClose,MediaRouter,DialMediaRouteProvider,AcceptCHFrame,AutoExpandDetailsElement,CertificateTransparencyComponentUpdater,AvoidUnnecessaryBeforeUnloadCheckSync,Translate"
];

/// static chrome arguments to start
#[cfg(all(not(feature = "chrome_cpu"), feature = "real_browser"))]
pub static CHROME_ARGS: [&'static str; 24] = [
    if cfg!(feature = "chrome_headless_new") {
        "--headless=new"
    } else {
        "--headless"
    },
    "--disable-extensions",
    "--disable-component-extensions-with-background-pages",
    "--disable-background-networking",
    "--disable-component-update",
    "--disable-client-side-phishing-detection",
    "--disable-sync",
    "--disable-dev-shm-usage",
    "--metrics-recording-only",
    "--disable-default-apps",
    "--mute-audio",
    "--no-default-browser-check",
    "--no-first-run",
    "--disable-backgrounding-occluded-windows",
    "--disable-renderer-backgrounding",
    "--disable-background-timer-throttling",
    "--disable-ipc-flooding-protection",
    "--password-store=basic",
    "--use-mock-keychain",
    "--force-fieldtrials=*BackgroundTracing/default/",
    "--disable-hang-monitor",
    "--disable-prompt-on-repost",
    "--disable-domain-reliability",
    "--disable-features=InterestFeedContentSuggestions,PrivacySandboxSettings4,AutofillServerCommunication,CalculateNativeWinOcclusion,OptimizationHints,AudioServiceOutOfProcess,IsolateOrigins,site-per-process,ImprovedCookieControls,LazyFrameLoading,GlobalMediaControls,DestroyProfileOnBrowserClose,MediaRouter,DialMediaRouteProvider,AcceptCHFrame,AutoExpandDetailsElement,CertificateTransparencyComponentUpdater,AvoidUnnecessaryBeforeUnloadCheckSync,Translate"
];

// One of the configs below is detected by CF bots. We need to take a look at the optimal args 03/25/24.
#[cfg(all(not(feature = "chrome_cpu"), not(feature = "real_browser")))]
/// static chrome arguments to start application ref [https://github.com/a11ywatch/chrome/blob/main/src/main.rs#L13]
static CHROME_ARGS: [&'static str; 60] = [
    if cfg!(feature = "chrome_headless_new") { "--headless=new" } else { "--headless" },
    "--no-sandbox",
    "--no-first-run",
    "--hide-scrollbars",
    // "--allow-pre-commit-input",
    // "--user-data-dir=~/.config/google-chrome",
    "--allow-running-insecure-content",
    "--autoplay-policy=user-gesture-required",
    "--ignore-certificate-errors",
    "--no-default-browser-check",
    "--no-zygote",
    "--disable-setuid-sandbox",
    "--disable-dev-shm-usage", // required or else docker containers may crash not enough memory
    "--disable-threaded-scrolling",
    "--disable-demo-mode",
    "--disable-dinosaur-easter-egg",
    "--disable-fetching-hints-at-navigation-start",
    "--disable-site-isolation-trials",
    "--disable-web-security",
    "--disable-threaded-animation",
    "--disable-sync",
    "--disable-print-preview",
    "--disable-partial-raster",
    "--disable-in-process-stack-traces",
    "--disable-v8-idle-tasks",
    "--disable-low-res-tiling",
    "--disable-speech-api",
    "--disable-smooth-scrolling",
    "--disable-default-apps",
    "--disable-prompt-on-repost",
    "--disable-domain-reliability",
    "--disable-component-update",
    "--disable-background-timer-throttling",
    "--disable-breakpad",
    "--disable-software-rasterizer",
    "--disable-extensions",
    "--disable-popup-blocking",
    "--disable-hang-monitor",
    "--disable-image-animation-resync",
    "--disable-client-side-phishing-detection",
    "--disable-component-extensions-with-background-pages",
    "--disable-ipc-flooding-protection",
    "--disable-background-networking",
    "--disable-renderer-backgrounding",
    "--disable-field-trial-config",
    "--disable-back-forward-cache",
    "--disable-backgrounding-occluded-windows",
    "--force-fieldtrials=*BackgroundTracing/default/",
    // "--enable-automation",
    "--log-level=3",
    "--enable-logging=stderr",
    "--enable-features=SharedArrayBuffer,NetworkService,NetworkServiceInProcess",
    "--metrics-recording-only",
    "--use-mock-keychain",
    "--force-color-profile=srgb",
    "--mute-audio",
    "--no-service-autorun",
    "--password-store=basic",
    "--export-tagged-pdf",
    "--no-pings",
    "--use-gl=swiftshader",
    "--window-size=1920,1080",
    "--disable-features=InterestFeedContentSuggestions,PrivacySandboxSettings4,AutofillServerCommunication,CalculateNativeWinOcclusion,OptimizationHints,AudioServiceOutOfProcess,IsolateOrigins,site-per-process,ImprovedCookieControls,LazyFrameLoading,GlobalMediaControls,DestroyProfileOnBrowserClose,MediaRouter,DialMediaRouteProvider,AcceptCHFrame,AutoExpandDetailsElement,CertificateTransparencyComponentUpdater,AvoidUnnecessaryBeforeUnloadCheckSync,Translate"
];

#[cfg(all(feature = "chrome_cpu", not(feature = "real_browser")))]
/// static chrome arguments to start application ref [https://github.com/a11ywatch/chrome/blob/main/src/main.rs#L13]
static CHROME_ARGS: [&'static str; 63] = [
    if cfg!(feature = "chrome_headless_new") { "--headless=new" } else { "--headless" },
    "--no-sandbox",
    "--no-first-run",
    "--hide-scrollbars",
    // "--allow-pre-commit-input",
    // "--user-data-dir=~/.config/google-chrome",
    "--allow-running-insecure-content",
    "--autoplay-policy=user-gesture-required",
    "--ignore-certificate-errors",
    "--no-default-browser-check",
    "--no-zygote",
    "--in-process-gpu",
    "--disable-gpu",
    "--disable-gpu-sandbox",
    "--disable-setuid-sandbox",
    "--disable-dev-shm-usage", // required or else docker containers may crash not enough memory
    "--disable-threaded-scrolling",
    "--disable-demo-mode",
    "--disable-dinosaur-easter-egg",
    "--disable-fetching-hints-at-navigation-start",
    "--disable-site-isolation-trials",
    "--disable-web-security",
    "--disable-threaded-animation",
    "--disable-sync",
    "--disable-print-preview",
    "--disable-partial-raster",
    "--disable-in-process-stack-traces",
    "--disable-v8-idle-tasks",
    "--disable-low-res-tiling",
    "--disable-speech-api",
    "--disable-smooth-scrolling",
    "--disable-default-apps",
    "--disable-prompt-on-repost",
    "--disable-domain-reliability",
    "--disable-component-update",
    "--disable-background-timer-throttling",
    "--disable-breakpad",
    "--disable-software-rasterizer",
    "--disable-extensions",
    "--disable-popup-blocking",
    "--disable-hang-monitor",
    "--disable-image-animation-resync",
    "--disable-client-side-phishing-detection",
    "--disable-component-extensions-with-background-pages",
    "--disable-ipc-flooding-protection",
    "--disable-background-networking",
    "--disable-renderer-backgrounding",
    "--disable-field-trial-config",
    "--disable-back-forward-cache",
    "--disable-backgrounding-occluded-windows",
    "--force-fieldtrials=*BackgroundTracing/default/",
    // "--enable-automation",
    "--log-level=3",
    "--enable-logging=stderr",
    "--enable-features=SharedArrayBuffer,NetworkService,NetworkServiceInProcess",
    "--metrics-recording-only",
    "--use-mock-keychain",
    "--force-color-profile=srgb",
    "--mute-audio",
    "--no-service-autorun",
    "--password-store=basic",
    "--export-tagged-pdf",
    "--no-pings",
    "--use-gl=swiftshader",
    "--window-size=1920,1080",
    "--disable-features=InterestFeedContentSuggestions,PrivacySandboxSettings4,AutofillServerCommunication,CalculateNativeWinOcclusion,OptimizationHints,AudioServiceOutOfProcess,IsolateOrigins,site-per-process,ImprovedCookieControls,LazyFrameLoading,GlobalMediaControls,DestroyProfileOnBrowserClose,MediaRouter,DialMediaRouteProvider,AcceptCHFrame,AutoExpandDetailsElement,CertificateTransparencyComponentUpdater,AvoidUnnecessaryBeforeUnloadCheckSync,Translate"
];

/// Fingerprint handling
pub(crate) static FP_JS: &'static str = r#"const toBlob=HTMLCanvasElement.prototype.toBlob,toDataURL=HTMLCanvasElement.prototype.toDataURL,getImageData=CanvasRenderingContext2D.prototype.getImageData,noisify=function(e,t){let o={r:Math.floor(10*Math.random())-5,g:Math.floor(10*Math.random())-5,b:Math.floor(10*Math.random())-5,a:Math.floor(10*Math.random())-5},r=e.width,n=e.height,a=getImageData.apply(t,[0,0,r,n]);for(let i=0;i<n;i++)for(let f=0;f<r;f++){let l=i*(4*r)+4*f;a.data[l+0]=a.data[l+0]+o.r,a.data[l+1]=a.data[l+1]+o.g,a.data[l+2]=a.data[l+2]+o.b,a.data[l+3]=a.data[l+3]+o.a}t.putImageData(a,0,0)};Object.defineProperty(HTMLCanvasElement.prototype,"toBlob",{value:function(){return noisify(this,this.getContext("2d")),toBlob.apply(this,arguments)}}),Object.defineProperty(HTMLCanvasElement.prototype,"toDataURL",{value:function(){return noisify(this,this.getContext("2d")),toDataURL.apply(this,arguments)}}),Object.defineProperty(CanvasRenderingContext2D.prototype,"getImageData",{value:function(){return noisify(this.canvas,this),getImageData.apply(this,arguments)}});const config={random:{value:function(){return Math.random()},item:function(e){let t=e.length*config.random.value();return e[Math.floor(t)]},array:function(e){let t=config.random.item(e);return new Int32Array([t,t])},items:function(e,t){let o=e.length,r=Array(t),n=Array(o);for(t>o&&(t=o);t--;){let a=Math.floor(config.random.value()*o);r[t]=e[a in n?n[a]:a],n[a]=--o in n?n[o]:o}return r}},spoof:{webgl:{buffer:function(e){let t=e.prototype.bufferData;Object.defineProperty(e.prototype,"bufferData",{value:function(){let e=Math.floor(10*config.random.value()),o=.1*config.random.value()*arguments[1][e];return arguments[1][e]=arguments[1][e]+o,t.apply(this,arguments)}})},parameter:function(e){e.prototype.getParameter,Object.defineProperty(e.prototype,"getParameter",{value:function(){let e=new Float32Array([1,8192]);if(3415===arguments[0])return 0;if(3414===arguments[0])return 24;if(35661===arguments[0])return config.random.items([128,192,256]);if(3386===arguments[0])return config.random.array([8192,16384,32768]);if(36349===arguments[0]||36347===arguments[0])return config.random.item([4096,8192]);else if(34047===arguments[0]||34921===arguments[0])return config.random.items([2,4,8,16]);else if(7937===arguments[0]||33901===arguments[0]||33902===arguments[0])return e;else if(34930===arguments[0]||36348===arguments[0]||35660===arguments[0])return config.random.item([16,32,64]);else if(34076===arguments[0]||34024===arguments[0]||3379===arguments[0])return config.random.item([16384,32768]);else if(3413===arguments[0]||3412===arguments[0]||3411===arguments[0]||3410===arguments[0]||34852===arguments[0])return config.random.item([2,4,8,16]);else return config.random.item([0,2,4,8,16,32,64,128,256,512,1024,2048,4096,])}})}}}};config.spoof.webgl.buffer(WebGLRenderingContext),config.spoof.webgl.buffer(WebGL2RenderingContext),config.spoof.webgl.parameter(WebGLRenderingContext),config.spoof.webgl.parameter(WebGL2RenderingContext);const rand={noise:function(){return Math.floor(Math.random()+(Math.random()<Math.random()?-1:1)*Math.random())},sign:function(){let e=[-1,-1,-1,-1,-1,-1,1,-1,-1,-1],t=Math.floor(Math.random()*e.length);return e[t]}};Object.defineProperty(HTMLElement.prototype,"offsetHeight",{get(){let e=Math.floor(this.getBoundingClientRect().height),t=e&&1===rand.sign(),o=t?e+rand.noise():e;return o}}),Object.defineProperty(HTMLElement.prototype,"offsetWidth",{get(){let e=Math.floor(this.getBoundingClientRect().width),t=e&&1===rand.sign(),o=t?e+rand.noise():e;return o}});const context={BUFFER:null,getChannelData:function(e){let t=e.prototype.getChannelData;Object.defineProperty(e.prototype,"getChannelData",{value:function(){let e=t.apply(this,arguments);if(context.BUFFER!==e){context.BUFFER=e;for(let o=0;o<e.length;o+=100){let r=Math.floor(Math.random()*o);e[r]=e[r]+1e-7*Math.random()}}return e}})},createAnalyser:function(e){let t=e.prototype.__proto__.createAnalyser;Object.defineProperty(e.prototype.__proto__,"createAnalyser",{value:function(){let e=t.apply(this,arguments),o=e.__proto__.getFloatFrequencyData;return Object.defineProperty(e.__proto__,"getFloatFrequencyData",{value:function(){let e=o.apply(this,arguments);for(let t=0;t<arguments[0].length;t+=100){let r=Math.floor(Math.random()*t);arguments[0][r]=arguments[0][r]+.1*Math.random()}return e}}),e}})}};context.getChannelData(AudioBuffer),context.createAnalyser(AudioContext),context.getChannelData(OfflineAudioContext),context.createAnalyser(OfflineAudioContext),navigator.mediaDevices.getUserMedia=navigator.webkitGetUserMedia=navigator.mozGetUserMedia=navigator.getUserMedia=webkitRTCPeerConnection=RTCPeerConnection=MediaStreamTrack=void 0;const getParameter=WebGLRenderingContext.prototype.getParameter;WebGLRenderingContext.prototype.getParameter=function(e){return 37445===e?"Intel Open Source Technology Center":37446===e?"Mesa DRI Intel(R) Ivybridge Mobile ":getParameter.call(this,e)};const newProto=navigator.__proto__;delete newProto.webdriver,navigator.__proto__=newProto;"#;
// /// Handle extracting links from anchors that are not found.
// pub(crate) static ANCHOR_EVENTS: &'static str = r###"() => new Promise((resolve) => { const _pageRoutes = new Set(), _originalPushState = window.history.pushState; window.history.pushState = function(_state, _title, _url) { _pageRoutes.add(_url) }; function _onRouteChange() { _pageRoutes.add(window.location.href) } document.querySelectorAll("a:not([href])").forEach(_anchor => { _anchor.click() }); window.addEventListener("popstate", _onRouteChange); return resolve(Array.from(_pageRoutes)); } )"###;
