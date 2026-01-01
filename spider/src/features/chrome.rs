use crate::features::chrome_args::CHROME_ARGS;
use crate::utils::{log,detect_chrome::get_detect_chrome_executable};
use crate::{configuration::Configuration, tokio_stream::StreamExt};
use chromiumoxide::cdp::browser_protocol::browser::{
    SetDownloadBehaviorBehavior, SetDownloadBehaviorParamsBuilder,
};
use chromiumoxide::cdp::browser_protocol::{
    browser::BrowserContextId, emulation::SetGeolocationOverrideParams, network::CookieParam,
    target::CreateTargetParams,
};
use chromiumoxide::error::CdpError;
use chromiumoxide::handler::REQUEST_TIMEOUT;
use chromiumoxide::serde_json;
use chromiumoxide::Page;
use chromiumoxide::{handler::HandlerConfig, Browser, BrowserConfig};
use lazy_static::lazy_static;
use std::time::Duration;
use tokio::task::JoinHandle;
use url::Url;

lazy_static! {
    /// Enable loopback for proxy.
    static ref LOOP_BACK_PROXY: bool = std::env::var("LOOP_BACK_PROXY").unwrap_or_default() == "true";
}

#[cfg(feature = "cookies")]
/// Parse a cookie into a jar. This does nothing without the 'cookies' flag.
pub fn parse_cookies_with_jar(cookie_str: &str, url: &Url) -> Result<Vec<CookieParam>, String> {
    use reqwest::cookie::{CookieStore, Jar};
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

/// Parse a cookie into a jar. This does nothing without the 'cookies' flag.
#[cfg(not(feature = "cookies"))]
pub fn parse_cookies_with_jar(cookie_str: &str, url: &Url) -> Result<Vec<CookieParam>, String> {
    Ok(Default::default())
}

/// Handle the browser cookie configurations.
#[cfg(not(feature = "cookies"))]
pub async fn set_cookies(config: &Configuration, url_parsed: &Option<Box<Url>>, browser: &Browser) {
}

/// Handle the browser cookie configurations.
#[cfg(feature = "cookies")]
pub async fn set_cookies(config: &Configuration, url_parsed: &Option<Box<Url>>, browser: &Browser) {
    if !config.cookie_str.is_empty() {
        if let Some(parsed) = url_parsed {
            let cookies = parse_cookies_with_jar(&config.cookie_str, &*parsed);
            if let Ok(co) = cookies {
                let _ = browser.set_cookies(co).await;
            };
        };
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
        whitelist_patterns: config.chrome_intercept.whitelist_patterns.clone(),
        blacklist_patterns: config.chrome_intercept.blacklist_patterns.clone(),
        ignore_ads: config.chrome_intercept.block_ads,
        ignore_javascript: config.chrome_intercept.block_javascript,
        ignore_analytics: config.chrome_intercept.block_analytics,
        ignore_stylesheets: config.chrome_intercept.block_stylesheets,
        extra_headers: match &config.headers {
            Some(headers) => {
                let mut hm = crate::utils::header_utils::header_map_to_hash_map(headers.inner());
                hm.remove("user-agent");

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
        max_bytes_allowed: config.max_bytes_allowed,
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
                browser_config.whitelist_patterns =
                    config.chrome_intercept.whitelist_patterns.clone();
                browser_config.blacklist_patterns =
                    config.chrome_intercept.blacklist_patterns.clone();
                browser_config.ignore_stylesheets = config.chrome_intercept.block_stylesheets;
                browser_config.ignore_analytics = config.chrome_intercept.block_analytics;
                browser_config.extra_headers = match config.headers {
                    Some(ref headers) => {
                        let mut hm =
                            crate::utils::header_utils::header_map_to_hash_map(headers.inner());

                        hm.remove("user-agent");

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
                set_cookies(&config, &url_parsed, &browser).await;
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
        if let Some(geo) = detect_geo_info(&new_page).await {
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
    request_timeout: &Option<Box<core::time::Duration>>,
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
                                if let Err(e) = intercept_page.send_command(c).await
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
    let ua_opt = config.user_agent.as_deref().filter(|ua| !ua.is_empty());

    let ua_for_profiles: &str = ua_opt.as_deref().map_or("", |v| v);

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
        &gpu_profile,
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

    if tokio::time::timeout(tokio::time::Duration::from_secs(15), async {
        tokio::join!(
            apply_page_setup,
            disable_log,
            bypass_csp,
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
