use crate::features::chrome_args::CHROME_ARGS;
use crate::utils::log;
use crate::{
    configuration::{Configuration, Fingerprint},
    tokio_stream::StreamExt,
};
use chromiumoxide::cdp::browser_protocol::browser::{
    SetDownloadBehaviorBehavior, SetDownloadBehaviorParamsBuilder,
};
use chromiumoxide::cdp::browser_protocol::{
    browser::BrowserContextId, network::CookieParam, target::CreateTargetParams,
};
use chromiumoxide::error::CdpError;
use chromiumoxide::handler::REQUEST_TIMEOUT;
use chromiumoxide::Page;
use chromiumoxide::{handler::HandlerConfig, Browser, BrowserConfig};
use lazy_static::lazy_static;
use reqwest::cookie::{CookieStore, Jar};
use spider_fingerprint::builder::AgentOs;
use spider_fingerprint::spoofs::{
    resolve_dpr, spoof_history_length_script, spoof_media_codecs_script, spoof_media_labels_script,
    spoof_screen_script_rng, DISABLE_DIALOGS, SPOOF_NOTIFICATIONS, SPOOF_PERMISSIONS_QUERY,
};
use std::time::Duration;
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
    proxies: &Option<Vec<crate::configuration::RequestProxy>>,
    intercept: bool,
    cache_enabled: bool,
    viewport: impl Into<Option<chromiumoxide::handler::viewport::Viewport>>,
    request_timeout: &Option<Box<core::time::Duration>>,
) -> Option<BrowserConfig> {
    let builder = BrowserConfig::builder()
        .disable_default_args()
        .request_timeout(match request_timeout.as_ref() {
            Some(timeout) => **timeout,
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
    proxies: &Option<Vec<crate::configuration::RequestProxy>>,
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
            "".to_string()
        } else {
            e.replace("://", "=").to_string()
        }
    }));

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
        ignore_ads: config.chrome_intercept.block_ads,
        ignore_javascript: config.chrome_intercept.block_javascript,
        ignore_analytics: config.chrome_intercept.block_analytics,
        ignore_stylesheets: config.chrome_intercept.block_stylesheets,
        extra_headers: match config.headers {
            Some(ref headers) => {
                let mut hm = crate::utils::header_utils::header_map_to_hash_map(headers.inner());

                if cfg!(feature = "real_browser") {
                    crate::utils::header_utils::rewrite_headers_to_title_case(&mut hm);
                }

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
                        let mut hm =
                            crate::utils::header_utils::header_map_to_hash_map(headers.inner());
                        crate::utils::header_utils::rewrite_headers_to_title_case(&mut hm);

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
            let mut context_id = None;

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
                if !config.cookie_str.is_empty() {
                    if let Some(parsed) = url_parsed {
                        let cookies = parse_cookies_with_jar(&config.cookie_str, &*parsed);
                        if let Ok(co) = cookies {
                            let _ = browser.set_cookies(co).await;
                        };
                    };
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

            Some((browser, handle, context_id))
        }
        _ => None,
    }
}

/// configure the browser
pub async fn configure_browser(new_page: &Page, configuration: &Configuration) {
    let timezone_id = async {
        if let Some(timezone_id) = configuration.timezone_id.as_deref() {
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
        if let Some(locale) = configuration.locale.as_deref() {
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
    use rand::Rng;

    let stealth_mode = config.stealth_mode;
    let stealth = stealth_mode.stealth();
    let dismiss_dialogs = config.dismiss_dialogs.unwrap_or(true);

    let mut firefox_agent = false;

    let agent_os = config
        .user_agent
        .as_deref()
        .map(|ua| {
            let mut agent_os = AgentOs::Linux;
            if ua.contains("Chrome") {
                if ua.contains("Linux") {
                    agent_os = AgentOs::Linux;
                } else if ua.contains("Mac") {
                    agent_os = AgentOs::Mac;
                } else if ua.contains("Windows") {
                    agent_os = AgentOs::Windows;
                } else if ua.contains("Android") {
                    agent_os = AgentOs::Android;
                }
            } else {
                firefox_agent = ua.contains("Firefox");
            }

            agent_os
        })
        .unwrap_or(AgentOs::Linux);

    let spoof_script = if stealth && !firefox_agent {
        &spider_fingerprint::spoof_user_agent::spoof_user_agent_data_high_entropy_values(
            &spider_fingerprint::spoof_user_agent::build_high_entropy_data(&config.user_agent),
            spider_fingerprint::spoof_user_agent::UserAgentDataSpoofDegree::Real,
        )
    } else {
        &Default::default()
    };

    let linux = agent_os == AgentOs::Linux;

    let mut fingerprint_gpu = false;
    let fingerprint = match config.fingerprint {
        Fingerprint::Basic => true,
        Fingerprint::NativeGPU => {
            fingerprint_gpu = true;
            true
        }
        _ => false,
    };

    let fp_script = if fingerprint {
        let fp_script = if linux {
            if fingerprint_gpu {
                &*crate::features::chrome::FP_JS_GPU_LINUX
            } else {
                &*crate::features::chrome::FP_JS_LINUX
            }
        } else if agent_os == AgentOs::Mac {
            if fingerprint_gpu {
                &*crate::features::chrome::FP_JS_GPU_MAC
            } else {
                &*crate::features::chrome::FP_JS_MAC
            }
        } else if agent_os == AgentOs::Windows {
            if fingerprint_gpu {
                &*crate::features::chrome::FP_JS_GPU_WINDOWS
            } else {
                &*crate::features::chrome::FP_JS_WINDOWS
            }
        } else {
            &*crate::features::chrome::FP_JS
        };
        fp_script
    } else {
        &Default::default()
    };

    let disable_dialogs = if dismiss_dialogs { DISABLE_DIALOGS } else { "" };

    let screen_spoof = if let Some(viewport) = &config.viewport {
        let dpr = resolve_dpr(
            viewport.emulating_mobile,
            viewport.device_scale_factor,
            agent_os,
        );

        spoof_screen_script_rng(
            viewport.width,
            viewport.height,
            dpr,
            viewport.emulating_mobile,
            &mut rand::rng(),
            agent_os
        )
    } else {
        Default::default()
    };

    // Final combined script to inject
    let merged_script = if let Some(script) = config.evaluate_on_new_document.as_deref() {
        if fingerprint {
            Some(string_concat!(
                &fp_script,
                &spoof_script,
                disable_dialogs,
                spider_fingerprint::wrap_eval_script(&script),
                screen_spoof,
                spoof_media_codecs_script(),
                SPOOF_NOTIFICATIONS,
                SPOOF_PERMISSIONS_QUERY,
                spoof_media_labels_script(agent_os),
                spoof_history_length_script(rand::rng().random_range(1..=6))
            ))
        } else {
            Some(string_concat!(
                &spoof_script,
                disable_dialogs,
                spider_fingerprint::wrap_eval_script(&script),
                screen_spoof,
                spoof_media_codecs_script(),
                SPOOF_NOTIFICATIONS,
                SPOOF_PERMISSIONS_QUERY,
                spoof_media_labels_script(agent_os),
                spoof_history_length_script(rand::rng().random_range(1..=6))
            ))
        }
    } else if fingerprint {
        Some(string_concat!(
            &fp_script,
            &spoof_script,
            disable_dialogs,
            screen_spoof,
            spoof_media_codecs_script(),
            SPOOF_NOTIFICATIONS,
            SPOOF_PERMISSIONS_QUERY,
            spoof_media_labels_script(agent_os),
            spoof_history_length_script(rand::rng().random_range(1..=6))
        ))
    } else if stealth {
        Some(string_concat!(
            &spoof_script,
            disable_dialogs,
            screen_spoof,
            spoof_media_codecs_script(),
            SPOOF_NOTIFICATIONS,
            SPOOF_PERMISSIONS_QUERY,
            spoof_media_labels_script(agent_os),
            spoof_history_length_script(rand::rng().random_range(1..=6))
        ))
    } else {
        None
    };

    let stealth = async {
        match config.user_agent.as_deref() {
            Some(agent) if stealth => {
                let _ = tokio::join!(
                    chrome_page._enable_stealth_mode(
                        merged_script.as_deref(),
                        Some(agent_os),
                        Some(stealth_mode)
                    ),
                    chrome_page.set_user_agent(agent.as_str())
                );
            }
            Some(agent) => {
                let _ = tokio::join!(
                    chrome_page.set_user_agent(agent.as_str()),
                    chrome_page.add_script_to_evaluate_on_new_document(merged_script)
                );
            }
            None if stealth => {
                let _ = chrome_page
                    ._enable_stealth_mode(
                        merged_script.as_deref(),
                        Some(agent_os),
                        Some(stealth_mode),
                    )
                    .await;
            }
            None => (),
        }
    };
    if let Err(_) = tokio::time::timeout(tokio::time::Duration::from_secs(10), async {
        tokio::join!(stealth, configure_browser(&chrome_page, &config))
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
pub struct BrowserController {
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

/// Mac canvas fingerprint.
pub static CANVAS_FP_MAC: &str = r#"(()=>{const toBlob=HTMLCanvasElement.prototype.toBlob,toDataURL=HTMLCanvasElement.prototype.toDataURL,getImageData=CanvasRenderingContext2D.prototype.getImageData,noisify=function(e,t){let o={r:Math.floor(10*Math.random())-5,g:Math.floor(10*Math.random())-5,b:Math.floor(10*Math.random())-5,a:Math.floor(10*Math.random())-5},r=e.width,n=e.height,a=getImageData.apply(t,[0,0,r,n]);for(let i=0;i<n;i++)for(let f=0;f<r;f++){let l=i*(4*r)+4*f;a.data[l+0]+=o.r,a.data[l+1]+=o.g,a.data[l+2]+=o.b,a.data[l+3]+=o.a}t.putImageData(a,0,0)};Object.defineProperty(HTMLCanvasElement.prototype,'toBlob',{value:function(){return noisify(this,this.getContext('2d')),toBlob.apply(this,arguments)}}),Object.defineProperty(HTMLCanvasElement.prototype,'toDataURL',{value:function(){return noisify(this,this.getContext('2d')),toDataURL.apply(this,arguments)}}),Object.defineProperty(CanvasRenderingContext2D.prototype,'getImageData',{value:function(){return noisify(this.canvas,this),getImageData.apply(this,arguments)}}); })();"#;
/// Windows canvas fingerprint.
pub static CANVAS_FP_WINDOWS: &str = r#"(()=>{const toBlob=HTMLCanvasElement.prototype.toBlob,toDataURL=HTMLCanvasElement.prototype.toDataURL,getImageData=CanvasRenderingContext2D.prototype.getImageData,noisify=function(e,t){let o={r:Math.floor(6*Math.random())-3,g:Math.floor(6*Math.random())-3,b:Math.floor(6*Math.random())-3,a:Math.floor(6*Math.random())-3},r=e.width,n=e.height,a=getImageData.apply(t,[0,0,r,n]);for(let f=0;f<r;f++)for(let i=0;i<n;i++){let l=i*(4*r)+4*f;a.data[l+0]+=o.r,a.data[l+1]+=o.g,a.data[l+2]+=o.b,a.data[l+3]+=o.a}t.putImageData(a,0,0)};Object.defineProperty(HTMLCanvasElement.prototype,'toBlob',{value:function(){return noisify(this,this.getContext('2d')),toBlob.apply(this,arguments)}}),Object.defineProperty(HTMLCanvasElement.prototype,'toDataURL',{value:function(){return noisify(this,this.getContext('2d')),toDataURL.apply(this,arguments)}}),Object.defineProperty(CanvasRenderingContext2D.prototype,'getImageData',{value:function(){return noisify(this.canvas,this),getImageData.apply(this,arguments)}}); })();"#;
/// Linux canvas fingerprint.
pub static CANVAS_FP_LINUX: &str = r#"(()=>{const toBlob=HTMLCanvasElement.prototype.toBlob,toDataURL=HTMLCanvasElement.prototype.toDataURL,getImageData=CanvasRenderingContext2D.prototype.getImageData,noisify=function(e,t){const o={r:Math.floor(10*Math.random())-5,g:Math.floor(10*Math.random())-5,b:Math.floor(10*Math.random())-5,a:Math.floor(10*Math.random())-5},r=e.width,n=e.height,a=t.getImageData(0,0,r,n);for(let i=0;i<r*n*4;i+=4)a.data[i]+=o.r,a.data[i+1]+=o.g,a.data[i+2]+=o.b,a.data[i+3]+=o.a;t.putImageData(a,0,0)};Object.defineProperty(HTMLCanvasElement.prototype,'toBlob',{value:function(){return noisify(this,this.getContext('2d')),toBlob.apply(this,arguments)}}),Object.defineProperty(HTMLCanvasElement.prototype,'toDataURL',{value:function(){return noisify(this,this.getContext('2d')),toDataURL.apply(this,arguments)}}),Object.defineProperty(CanvasRenderingContext2D.prototype,'getImageData',{value:function(){return noisify(this.canvas,this),getImageData.apply(this,arguments)}}); })();"#;

/// Fingerprint JS to spoof.
pub static SPOOF_FINGERPRINT: &str = r###"(()=>{const config={random:{value:()=>Math.random(),item:e=>e[Math.floor(e.length*Math.random())],array:e=>new Int32Array([e[Math.floor(e.length*Math.random())],e[Math.floor(e.length*Math.random())]]),items:(e,t)=>{let r=e.length,a=Array(t),n=Array(r);for(t>r&&(t=r);t--;){let o=Math.floor(Math.random()*r);a[t]=e[o in n?n[o]:o],n[o]=--r in n?n[r]:r}return a}},spoof:{webgl:{buffer:e=>{let t=e.prototype.bufferData;Object.defineProperty(e.prototype,'bufferData',{value:function(){let e=Math.floor(10*Math.random()),r=.1*Math.random()*arguments[1][e];return arguments[1][e]+=r,t.apply(this,arguments)}})},parameter:e=>{Object.defineProperty(e.prototype,'getParameter',{value:function(){let a=new Float32Array([1,8192]);switch(arguments[0]){case 3415:return 0;case 3414:return 24;case 35661:return config.random.items([128,192,256]);case 3386:return config.random.array([8192,16384,32768]);case 36349:case 36347:return config.random.item([4096,8192]);case 34047:case 34921:return config.random.items([2,4,8,16]);case 7937:case 33901:case 33902:return a;case 34930:case 36348:case 35660:return config.random.item([16,32,64]);case 34076:case 34024:case 3379:return config.random.item([16384,32768]);case 3413:case 3412:case 3411:case 3410:case 34852:return config.random.item([2,4,8,16]);default:return config.random.item([0,2,4,8,16,32,64,128,256,512,1024,2048,4096])}}})}}}};config.spoof.webgl.buffer(WebGLRenderingContext);config.spoof.webgl.buffer(WebGL2RenderingContext);config.spoof.webgl.parameter(WebGLRenderingContext);config.spoof.webgl.parameter(WebGL2RenderingContext);const rand={noise:()=>Math.floor(Math.random()+(Math.random()<Math.random()?-1:1)*Math.random()),sign:()=>[-1,-1,-1,-1,-1,-1,1,-1,-1,-1][Math.floor(10*Math.random())]};Object.defineProperty(HTMLElement.prototype,'offsetHeight',{get:function(){let e=Math.floor(this.getBoundingClientRect().height);return e&&1===rand.sign()?e+rand.noise():e}});Object.defineProperty(HTMLElement.prototype,'offsetWidth',{get:function(){let e=Math.floor(this.getBoundingClientRect().width);return e&&1===rand.sign()?e+rand.noise():e}});const ctx={BUFFER:null,getChannelData:e=>{let t=e.prototype.getChannelData;Object.defineProperty(e.prototype,'getChannelData',{value:function(){let d=t.apply(this,arguments);if(ctx.BUFFER!==d){ctx.BUFFER=d;for(let i=0;i<d.length;i+=100){d[Math.floor(Math.random()*i)]+=1e-7*Math.random()}}return d}})},createAnalyser:e=>{let t=e.prototype.__proto__.createAnalyser;Object.defineProperty(e.prototype.__proto__,'createAnalyser',{value:function(){let a=t.apply(this,arguments),r=a.__proto__.getFloatFrequencyData;Object.defineProperty(a.__proto__,'getFloatFrequencyData',{value:function(){let arr=r.apply(this,arguments);for(let i=0;i<arguments[0].length;i+=100){arguments[0][Math.floor(Math.random()*i)]+=.1*Math.random()}return arr}});return a}})} };ctx.getChannelData(AudioBuffer);ctx.createAnalyser(AudioContext);ctx.getChannelData(OfflineAudioContext);ctx.createAnalyser(OfflineAudioContext);window.webkitRTCPeerConnection=void 0;window.RTCPeerConnection=void 0;window.MediaStreamTrack=void 0; })();"###;
/// Base fingerprint JS.
pub static BASE_FP_JS: &str = r#"{{CANVAS_FP}}{{SPOOF_FINGERPRINT}}"#;

/// Spoof the m1 gpu from the code.
pub(crate) fn spoof_m1_gpu(spoof: &str) -> String {
    spoof
        .replace("Intel Open Source Technology Center", "Apple Inc.")
        .replace(
            "Mesa DRI Intel(R) Ivybridge Mobile",
            "ANGLE (Apple, ANGLE Metal Renderer: Apple M1 Max, Unspecified Version)",
        )
}

/// Spoof the NVIDIA GeForce from the code..
pub(crate) fn spoof_windows_nvidea_gpu(spoof: &str) -> String {
    spoof
        .replace("Intel Open Source Technology Center", "NVIDIA Corporation")
        .replace(
            "Mesa DRI Intel(R) Ivybridge Mobile",
            "NVIDIA GeForce GTX 1650/PCIe/SSE2",
        )
}

lazy_static! {
    /// Fingerprint gpu is not enabled for Mac.
    pub(crate) static ref FP_JS_MAC: String =  spoof_m1_gpu(&BASE_FP_JS.replacen("{{CANVAS_FP}}", CANVAS_FP_MAC, 1)).replacen("{{SPOOF_FINGERPRINT}}", SPOOF_FINGERPRINT, 1).replace("\n", "");
    /// Fingerprint gpu was enabled on the Mac. The full spoof is not required.
    pub(crate) static ref FP_JS_GPU_MAC: String = spoof_m1_gpu(&BASE_FP_JS.replacen("{{CANVAS_FP}}", CANVAS_FP_MAC, 1)).replacen("{{SPOOF_FINGERPRINT}}", "", 1).replace("\n", "");
    /// Fingerprint gpu is not enabled for Linux.
    pub(crate) static ref FP_JS_LINUX: String = BASE_FP_JS.replacen("{{CANVAS_FP}}", CANVAS_FP_LINUX, 1).replacen("{{SPOOF_FINGERPRINT}}", SPOOF_FINGERPRINT, 1) .replace("\n", "");
    /// Fingerprint gpu was enabled on the Linux. The full spoof is not required.
    pub(crate) static ref FP_JS_GPU_LINUX: String = BASE_FP_JS.replacen("{{CANVAS_FP}}", CANVAS_FP_LINUX, 1).replacen("{{SPOOF_FINGERPRINT}}", "", 1) .replace("\n", "");
    /// Fingerprint gpu is not enabled for WINDOWS.
    pub(crate) static ref FP_JS_WINDOWS: String = spoof_windows_nvidea_gpu(&BASE_FP_JS.replacen("{{CANVAS_FP}}", CANVAS_FP_WINDOWS, 1)).replacen("{{SPOOF_FINGERPRINT}}", SPOOF_FINGERPRINT, 1) .replace("\n", "");
    /// Fingerprint gpu was enabled on the WINDOWS. The full spoof is not required.
    pub(crate) static ref FP_JS_GPU_WINDOWS: String = spoof_windows_nvidea_gpu(&BASE_FP_JS.replacen("{{CANVAS_FP}}", CANVAS_FP_WINDOWS, 1)).replacen("{{SPOOF_FINGERPRINT}}", "", 1) .replace("\n", "");
}

#[cfg(target_os = "macos")]
lazy_static! {
    /// The gpu is not enabled.
    pub(crate) static ref FP_JS: String = FP_JS_MAC.clone();
    /// The gpu was enabled on the machine. The spoof is not required.
    pub(crate) static ref FP_JS_GPU: String = FP_JS_GPU_MAC.clone();
}

#[cfg(target_os = "windows")]
lazy_static! {
    /// The gpu is not enabled.
    pub(crate) static ref FP_JS: String = FP_JS_WINDOWS.clone();
    /// The gpu was enabled on the machine. The spoof is not required.
    pub(crate) static ref FP_JS_GPU: String = FP_JS_GPU_WINDOWS.clone();
}

#[cfg(target_os = "linux")]
lazy_static! {
    /// The gpu is not enabled.
    pub(crate) static ref FP_JS: String = BASE_FP_JS.replacen("{{CANVAS_FP}}", CANVAS_FP_LINUX, 1).replacen("{{SPOOF_FINGERPRINT}}", SPOOF_FINGERPRINT, 1) .replace("\n", "");
    /// The gpu was enabled on the machine. The spoof is not required.
    pub(crate) static ref FP_JS_GPU: String = BASE_FP_JS.replacen("{{CANVAS_FP}}", CANVAS_FP_LINUX, 1).replacen("{{SPOOF_FINGERPRINT}}", "", 1) .replace("\n", "");
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
lazy_static! {
    /// The gpu is not enabled.
    pub(crate) static ref FP_JS: String = BASE_FP_JS.replacen("{{CANVAS_FP}}", CANVAS_FP_LINUX, 1).replacen("{{SPOOF_FINGERPRINT}}", SPOOF_FINGERPRINT, 1) .replace("\n", "");
    /// The gpu was enabled on the machine. The spoof is not required.
    pub(crate) static ref FP_JS_GPU: String = BASE_FP_JS.replacen("{{CANVAS_FP}}",CANVAS_FP_LINUX, 1).replacen("{{SPOOF_FINGERPRINT}}", "", 1) .replace("\n", "");
}
