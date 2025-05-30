/// Absolute path domain handling.
pub mod abs;
/// Connect layer for reqwest.
pub mod connect;
/// Generic CSS selectors.
pub mod css_selectors;
/// Utils to modify the HTTP header.
pub mod header_utils;
/// String interner.
pub mod interner;
/// A trie struct.
pub mod trie;

#[cfg(feature = "balance")]
/// CPU and Memory detection to balance limitations.
pub mod detect_system;
use crate::page::{AntiBotTech, Metadata};
use crate::{page::STREAMING_CHUNK_SIZE, RelativeSelectors};
use abs::parse_absolute_url;
use aho_corasick::AhoCorasick;
use auto_encoder::is_binary_file;
use bytes::BufMut;
use case_insensitive_string::CaseInsensitiveString;
#[cfg(feature = "chrome")]
use hashbrown::HashMap;
use lol_html::{send::HtmlRewriter, OutputSink};
use phf::phf_set;
use std::str::FromStr;
use std::sync::Arc;
use std::{
    future::Future,
    time::{Duration, Instant},
};
use tokio::sync::Semaphore;
use url::Url;

#[cfg(feature = "chrome")]
use crate::features::chrome_common::{AutomationScripts, ExecutionScripts};
use crate::page::{MAX_PRE_ALLOCATED_HTML_PAGE_SIZE, MAX_PRE_ALLOCATED_HTML_PAGE_SIZE_USIZE};
use crate::tokio_stream::StreamExt;
use crate::Client;

#[cfg(feature = "cache_chrome_hybrid")]
use http_cache_semantics::{RequestLike, ResponseLike};

use log::{info, log_enabled, Level};

#[cfg(not(feature = "rquest"))]
use reqwest::{Response, StatusCode};
#[cfg(feature = "rquest")]
use rquest::{Response, StatusCode};

/// The request error.
#[cfg(all(not(feature = "cache_request"), not(feature = "rquest")))]
pub(crate) type RequestError = reqwest::Error;

/// The request error (for `rquest`).
#[cfg(all(not(feature = "cache_request"), feature = "rquest"))]
pub(crate) type RequestError = rquest::Error;

/// The request error (for `reqwest_middleware` with caching).
#[cfg(feature = "cache_request")]
pub(crate) type RequestError = reqwest_middleware::Error;

/// The request response.
pub(crate) type RequestResponse = Response;

/// The wait for duration timeouts.
#[cfg(feature = "chrome")]
const WAIT_TIMEOUTS: [u64; 6] = [0, 20, 50, 100, 100, 500];
/// The wait for duration timeouts.
#[cfg(feature = "chrome")]
const DOM_WAIT_TIMEOUTS: [u64; 6] = [100, 200, 300, 300, 400, 500];

/// Ignore the content types.
pub static IGNORE_CONTENT_TYPES: phf::Set<&'static str> = phf_set! {
    "application/pdf",
    "application/zip",
    "application/x-rar-compressed",
    "application/x-tar",
    "image/png",
    "image/jpeg",
    "image/gif",
    "image/bmp",
    "image/svg+xml",
    "video/mp4",
    "video/x-msvideo",
    "video/x-matroska",
    "video/webm",
    "audio/mpeg",
    "audio/ogg",
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    "application/vnd.ms-excel",
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    "application/vnd.ms-powerpoint",
    "application/vnd.openxmlformats-officedocument.presentationml.presentation",
    "application/x-7z-compressed",
    "application/x-rpm",
    "application/x-shockwave-flash",
};

lazy_static! {
    /// Scan for error anti-bot pages.
    static ref AC_BODY_SCAN: AhoCorasick = AhoCorasick::new([
        "cf-error-code",
        "Access to this page has been denied",
        "DataDome",
        "perimeterx",
        "funcaptcha",
        "Request unsuccessful. Incapsula incident ID",
    ]).unwrap();

    static ref AC_URL_SCAN: AhoCorasick = AhoCorasick::builder()
        .match_kind(aho_corasick::MatchKind::LeftmostFirst) // optional: stops at first match
        .build([
            "/cdn-cgi/challenge-platform",       // 0
            "datadome.co",                       // 1
            "dd-api.io",                         // 2
            "perimeterx.net",                    // 3
            "px-captcha",                        // 4
            "arkoselabs.com",                    // 5
            "funcaptcha",                        // 6
            "kasada.io",                         // 7
            "fingerprint.com",                   // 8
            "fpjs.io",                           // 9
            "incapsula",                         // 10
            "imperva",                           // 11
            "radwarebotmanager",                 // 12
            "reblaze.com",                       // 13
            "cheq.ai",                           // 14
        ])
        .unwrap();
}

#[cfg(feature = "fs")]
lazy_static! {
    static ref TMP_DIR: String = {
        use std::fs;
        let mut tmp = std::env::temp_dir();

        tmp.push("spider/");

        // make sure spider dir is created.
        match fs::create_dir_all(&tmp) {
            Ok(_) => {
                let dir_name = tmp.display().to_string();

                match std::time::SystemTime::now().duration_since(std::time::SystemTime::UNIX_EPOCH) {
                    Ok(dur) => {
                        string_concat!(dir_name, dur.as_secs().to_string())
                    }
                    _ => dir_name,
                }
            }
            _ => "/tmp/".to_string()
        }
    };
}

#[cfg(feature = "chrome")]
lazy_static! {
    /// Mask the chrome connection interception bytes from responses. Rejected responses send 17.0 bytes for the response.
    pub(crate) static ref MASK_BYTES_INTERCEPTION: bool = {
        std::env::var("MASK_BYTES_INTERCEPTION").unwrap_or_default() == "true"
    };
    /// Cloudflare turnstile wait.
    pub(crate) static ref CF_WAIT_FOR: crate::features::chrome_common::WaitFor = {
        let mut wait_for = crate::features::chrome_common::WaitFor::default();
        wait_for.delay = crate::features::chrome_common::WaitForDelay::new(Some(core::time::Duration::from_millis(1000))).into();
        // wait_for.dom = crate::features::chrome_common::WaitForSelector::new(Some(core::time::Duration::from_millis(1000)), "body".into()).into();
        wait_for.idle_network = crate::features::chrome_common::WaitForIdleNetwork::new(core::time::Duration::from_secs(8).into()).into();
        wait_for
    };
}

lazy_static! {
    /// Prevent fetching resources beyond the bytes limit.
    pub(crate) static ref MAX_SIZE_BYTES: usize = {
        match std::env::var("SPIDER_MAX_SIZE_BYTES") {
            Ok(b) => {
                const DEFAULT_MAX_SIZE_BYTES: usize = 1_073_741_824; // 1GB in bytes

                let b = b.parse::<usize>().unwrap_or(DEFAULT_MAX_SIZE_BYTES);

                if b == 0 {
                    0
                } else {
                    b.max(1_048_576) // min 1mb
                }
            },
            _ => 0
        }
    };
}

#[cfg(all(feature = "chrome", feature = "real_browser"))]
lazy_static! {
    static ref CF_END: &'static [u8; 62] =
        b"target=\"_blank\">Cloudflare</a></div></div></div></body></html>";
    static ref CF_END2: &'static [u8; 72] =
        b"Performance &amp; security by Cloudflare</div></div></div></body></html>";
    static ref CF_HEAD: &'static [u8; 34] = b"<html><head>\n    <style global=\"\">";
    static ref CF_MOCK_FRAME: &'static [u8; 137] = b"<iframe height=\"1\" width=\"1\" style=\"position: absolute; top: 0px; left: 0px; border: none; visibility: hidden;\"></iframe>\n\n</body></html>";
    static ref CF_JUST_A_MOMENT:&'static [u8; 81] = b"<!DOCTYPE html><html lang=\"en-US\" dir=\"ltr\"><head><title>Just a moment...</title>";
}

lazy_static! {
    /// Apache server forbidden.
    pub static ref APACHE_FORBIDDEN: &'static [u8; 317] = br#"<!DOCTYPE HTML PUBLIC "-//IETF//DTD HTML 2.0//EN">
<html><head>
<title>403 Forbidden</title>
</head><body>
<h1>Forbidden</h1>
<p>You don't have permission to access this resource.</p>
<p>Additionally, a 403 Forbidden
error was encountered while trying to use an ErrorDocument to handle the request.</p>
</body></html>"#;

    /// Open Resty forbidden.
    pub static ref OPEN_RESTY_FORBIDDEN: &'static [u8; 125] = br#"<html><head><title>403 Forbidden</title></head>
<body>
<center><h1>403 Forbidden</h1></center>
<hr><center>openresty</center>"#;
}

/// Detect if a page is forbidden and should not retry.
pub fn detect_hard_forbidden_content(b: &[u8]) -> bool {
    b == *APACHE_FORBIDDEN || b.starts_with(*OPEN_RESTY_FORBIDDEN)
}

/// Is cloudflare turnstile page? This does nothing without the real_browser feature enabled.
#[cfg(all(feature = "chrome", feature = "real_browser"))]
pub(crate) fn detect_cf_turnstyle(b: &Vec<u8>) -> bool {
    let cf = CF_END.as_ref();
    let cf2 = CF_END2.as_ref();
    let cn = CF_HEAD.as_ref();
    let cnf = CF_MOCK_FRAME.as_ref();

    b.ends_with(cf)
        || b.ends_with(cf2)
        || b.starts_with(cn) && b.ends_with(cnf)
        || b.starts_with(CF_JUST_A_MOMENT.as_ref())
}

/// Handle protected pages via chrome. This does nothing without the real_browser feature enabled.
#[cfg(all(feature = "chrome", feature = "real_browser"))]
async fn cf_handle(
    b: &mut Vec<u8>,
    page: &chromiumoxide::Page,
) -> Result<bool, chromiumoxide::error::CdpError> {
    let mut validated = false;

    let page_result = tokio::time::timeout(tokio::time::Duration::from_secs(30), async {
        let mut wait_for = CF_WAIT_FOR.clone();

        page_wait(&page, &Some(wait_for.clone())).await;

        let _ = page
            .evaluate(r###"document.querySelectorAll("iframe").forEach(el=>el.click());"###)
            .await;

        wait_for.page_navigations = true;

        page_wait(&page, &Some(wait_for.clone())).await;

        if let Ok(next_content) = page.outer_html_bytes().await {
            let next_content = if !detect_cf_turnstyle(&next_content) {
                validated = true;
                // we should use wait for dom instead.
                wait_for.delay = crate::features::chrome_common::WaitForDelay::new(Some(
                    core::time::Duration::from_secs(4),
                ))
                .into();
                page_wait(&page, &Some(wait_for)).await;
                match page.outer_html_bytes().await {
                    Ok(nc) => nc,
                    _ => next_content,
                }
            } else {
                next_content
            };

            *b = next_content;
        }
    })
    .await;

    match page_result {
        Ok(_) => Ok(validated),
        _ => Err(chromiumoxide::error::CdpError::Timeout),
    }
}

/// Handle cloudflare protected pages via chrome. This does nothing without the real_browser feature enabled.
#[cfg(all(feature = "chrome", not(feature = "real_browser")))]
async fn cf_handle(
    _b: &mut Vec<u8>,
    _page: &chromiumoxide::Page,
) -> Result<(), chromiumoxide::error::CdpError> {
    Ok(())
}

/// The response of a web page.
#[derive(Debug, Default)]
pub struct PageResponse {
    /// The page response resource.
    pub content: Option<Box<Vec<u8>>>,
    /// The headers of the response. (Always None if a webdriver protocol is used for fetching.).
    pub headers: Option<reqwest::header::HeaderMap>,
    #[cfg(feature = "remote_addr")]
    /// The remote address of the page.
    pub remote_addr: Option<core::net::SocketAddr>,
    #[cfg(feature = "cookies")]
    /// The cookies of the response.
    pub cookies: Option<reqwest::header::HeaderMap>,
    /// The status code of the request.
    pub status_code: StatusCode,
    /// The final url destination after any redirects.
    pub final_url: Option<String>,
    /// The message of the response error if any.
    pub error_for_status: Option<Result<Response, RequestError>>,
    #[cfg(feature = "chrome")]
    /// The screenshot bytes of the page. The ScreenShotConfig bytes boolean needs to be set to true.
    pub screenshot_bytes: Option<Vec<u8>>,
    #[cfg(feature = "openai")]
    /// The credits used from OpenAI in order.
    pub openai_credits_used: Option<Vec<crate::features::openai_common::OpenAIUsage>>,
    #[cfg(feature = "openai")]
    /// The extra data from the AI, example extracting data etc...
    pub extra_ai_data: Option<Vec<crate::page::AIResults>>,
    /// A WAF was found on the page.
    pub waf_check: bool,
    /// The total bytes transferred for the page. Mainly used for chrome events. Inspect the content for bytes when using http instead.
    pub bytes_transferred: Option<f64>,
    /// The signature of the page to use for handling de-duplication.
    pub signature: Option<u64>,
    #[cfg(feature = "chrome")]
    /// All of the response events mapped with the amount of bytes used.
    pub response_map: Option<HashMap<String, f64>>,
    #[cfg(feature = "chrome")]
    /// All of the request events mapped with the time period of the event sent.
    pub request_map: Option<HashMap<String, f64>>,
    /// The anti-bot tech used.
    pub anti_bot_tech: crate::page::AntiBotTech,
    /// The metadata of the page.
    pub metadata: Option<Box<Metadata>>,
}

/// wait for event with timeout
#[cfg(feature = "chrome")]
pub async fn wait_for_event<T>(page: &chromiumoxide::Page, timeout: Option<core::time::Duration>)
where
    T: chromiumoxide::cdp::IntoEventKind + Unpin + std::fmt::Debug,
{
    if let Ok(mut events) = page.event_listener::<T>().await {
        let wait_until = async {
            let mut index = 0;

            loop {
                let current_timeout = WAIT_TIMEOUTS[index];
                let sleep = tokio::time::sleep(tokio::time::Duration::from_millis(current_timeout));

                tokio::select! {
                    _ = sleep => (),
                    v = events.next() => {
                        if !v.is_none () {
                            break;
                        }
                    }
                }

                index = (index + 1) % WAIT_TIMEOUTS.len();
            }
        };
        match timeout {
            Some(timeout) => if let Err(_) = tokio::time::timeout(timeout, wait_until).await {},
            _ => wait_until.await,
        }
    }
}

/// wait for a selector
#[cfg(feature = "chrome")]
pub async fn wait_for_selector(
    page: &chromiumoxide::Page,
    timeout: Option<core::time::Duration>,
    selector: &str,
) -> bool {
    let mut valid = false;
    let wait_until = async {
        let mut index = 0;

        loop {
            let current_timeout = WAIT_TIMEOUTS[index];
            let sleep = tokio::time::sleep(tokio::time::Duration::from_millis(current_timeout));

            tokio::select! {
                _ = sleep => (),
                v = page.find_element(selector) => {
                    if v.is_ok() {
                        valid = true;
                        break;
                    }
                }
            }

            index = (index + 1) % WAIT_TIMEOUTS.len();
        }
    };

    match timeout {
        Some(timeout) => {
            if let Err(_) = tokio::time::timeout(timeout, wait_until).await {
                valid = false;
            }
        }
        _ => wait_until.await,
    };

    valid
}

/// wait for dom to finish updating target selector
#[cfg(feature = "chrome")]
pub async fn wait_for_dom(
    page: &chromiumoxide::Page,
    timeout: Option<core::time::Duration>,
    selector: &str,
) {
    let max_duration = timeout.unwrap_or_else(|| core::time::Duration::from_millis(500));
    let mut deadline = tokio::time::Instant::now() + max_duration;

    let script = crate::features::chrome_common::generate_wait_for_dom_js_code_with_selector_base(
        max_duration.as_millis() as u32,
        selector,
    );

    let wait_until = async {
        let mut index = 0;

        loop {
            if tokio::time::Instant::now() >= deadline {
                break;
            }

            let current_timeout = DOM_WAIT_TIMEOUTS[index];
            let result = page.evaluate(script.clone()).await;

            if let Ok(vv) = &result {
                let value = vv.value();
                if let Some(value) = value {
                    if let Some(v) = value.as_bool() {
                        if v {
                            break;
                        } else {
                            tokio::time::sleep(tokio::time::Duration::from_millis(current_timeout))
                                .await;
                            deadline = tokio::time::Instant::now() + max_duration;
                        }
                    }
                }
            }

            index = (index + 1) % WAIT_TIMEOUTS.len();
        }
    };

    let _ = tokio::time::timeout(max_duration, wait_until).await;
}

/// Get the output path of a screenshot and create any parent folders if needed.
#[cfg(feature = "chrome")]
pub async fn create_output_path(
    base: &std::path::PathBuf,
    target_url: &str,
    format: &str,
) -> String {
    let out = string_concat!(
        &percent_encoding::percent_encode(
            target_url.as_bytes(),
            percent_encoding::NON_ALPHANUMERIC
        )
        .to_string(),
        format
    );

    let b = base.join(&out);

    if let Some(p) = b.parent() {
        let _ = tokio::fs::create_dir_all(&p).await;
    }

    b.display().to_string()
}

#[cfg(feature = "chrome")]
/// Wait for page events.
/// 1. First wait for idle networks.
/// 2. Wait for selectors.
/// 3. Wait for the dom element to finish updated.
/// 4. Wait for hard delay.
pub async fn page_wait(
    page: &chromiumoxide::Page,
    wait_for: &Option<crate::configuration::WaitFor>,
) {
    if let Some(wait_for) = wait_for {
        if let Some(ref wait) = wait_for.idle_network {
            wait_for_event::<chromiumoxide::cdp::browser_protocol::network::EventLoadingFinished>(
                page,
                wait.timeout,
            )
            .await;
        }

        if let Some(ref wait) = wait_for.selector {
            wait_for_selector(page, wait.timeout, &wait.selector).await;
        }

        if let Some(ref wait) = wait_for.dom {
            wait_for_dom(page, wait.timeout, &wait.selector).await;
        }

        if let Some(ref wait) = wait_for.delay {
            if let Some(timeout) = wait.timeout {
                tokio::time::sleep(timeout).await
            }
        }
    }
}

#[derive(Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg(feature = "openai")]
/// The json response from OpenAI.
pub struct JsonResponse {
    /// The content returned.
    content: Vec<String>,
    /// The js script for the browser.
    js: String,
    #[cfg_attr(feature = "serde", serde(default))]
    /// The AI failed to parse the data.
    error: Option<String>,
}

/// Handle the OpenAI credits used. This does nothing without 'openai' feature flag.
#[cfg(feature = "openai")]
pub fn handle_openai_credits(
    page_response: &mut PageResponse,
    tokens_used: crate::features::openai_common::OpenAIUsage,
) {
    match page_response.openai_credits_used.as_mut() {
        Some(v) => v.push(tokens_used),
        None => page_response.openai_credits_used = Some(vec![tokens_used]),
    };
}

#[cfg(not(feature = "openai"))]
/// Handle the OpenAI credits used. This does nothing without 'openai' feature flag.
pub fn handle_openai_credits(
    _page_response: &mut PageResponse,
    _tokens_used: crate::features::openai_common::OpenAIUsage,
) {
}

/// Handle extra OpenAI data used. This does nothing without 'openai' feature flag.
#[cfg(feature = "openai")]
pub fn handle_extra_ai_data(
    page_response: &mut PageResponse,
    prompt: &str,
    x: JsonResponse,
    screenshot_output: Option<Vec<u8>>,
    error: Option<String>,
) {
    let ai_response = crate::page::AIResults {
        input: prompt.into(),
        js_output: x.js,
        content_output: x
            .content
            .iter()
            .map(|c| c.trim_start().into())
            .collect::<Vec<_>>(),
        screenshot_output,
        error,
    };

    match page_response.extra_ai_data.as_mut() {
        Some(v) => v.push(ai_response),
        None => page_response.extra_ai_data = Some(Vec::from([ai_response])),
    };
}

/// Accepts different header types (for flexibility).
pub enum HeaderSource<'a> {
    /// From reqwest or internal HeaderMap.
    HeaderMap(&'a crate::client::header::HeaderMap),
    /// From a string-based HashMap.
    Map(&'a std::collections::HashMap<String, String>),
}

/// Detect from headers.
pub fn detect_anti_bot_from_headers(headers: &HeaderSource) -> Option<AntiBotTech> {
    macro_rules! has_key {
        ($key:expr) => {
            match headers {
                HeaderSource::HeaderMap(hm) => hm.contains_key($key),
                HeaderSource::Map(map) => map.contains_key($key),
            }
        };
    }

    if has_key!("cf-chl-bypass") || has_key!("cf-ray") {
        return Some(AntiBotTech::Cloudflare);
    }
    if has_key!("x-captcha-endpoint") {
        return Some(AntiBotTech::DataDome);
    }
    if has_key!("x-perimeterx") || has_key!("pxhd") {
        return Some(AntiBotTech::PerimeterX);
    }
    if has_key!("x-akamaibot") {
        return Some(AntiBotTech::AkamaiBotManager);
    }
    if has_key!("x-imperva-id") || has_key!("x-iinfo") {
        return Some(AntiBotTech::Imperva);
    }
    if has_key!("x-reblaze-uuid") {
        return Some(AntiBotTech::Reblaze);
    }

    None
}

/// Detect the anti-bot technology.
pub fn detect_anti_bot_from_body(body: &Vec<u8>) -> Option<AntiBotTech> {
    // Scan body for anti-bot fingerprints (only for small pages)
    if body.len() < 30_000 {
        if let Ok(finder) = AC_BODY_SCAN.try_find_iter(body) {
            for mat in finder {
                match mat.pattern().as_usize() {
                    0 => return Some(AntiBotTech::Cloudflare),
                    1 | 2 => return Some(AntiBotTech::DataDome),
                    3 => return Some(AntiBotTech::PerimeterX),
                    4 => return Some(AntiBotTech::ArkoseLabs),
                    5 => return Some(AntiBotTech::Imperva),
                    _ => (),
                }
            }
        }
    }

    None
}

/// Detect antibot from url
pub fn detect_antibot_from_url(url: &str) -> Option<AntiBotTech> {
    if let Some(mat) = AC_URL_SCAN.find(url) {
        let tech = match mat.pattern().as_usize() {
            0 => AntiBotTech::Cloudflare,
            1 | 2 => AntiBotTech::DataDome,
            3 | 4 => AntiBotTech::PerimeterX,
            5 | 6 => AntiBotTech::ArkoseLabs,
            7 => AntiBotTech::Kasada,
            8 | 9 => AntiBotTech::FingerprintJS,
            10 | 11 => AntiBotTech::Imperva,
            12 => AntiBotTech::RadwareBotManager,
            13 => AntiBotTech::Reblaze,
            14 => AntiBotTech::CHEQ,
            _ => return None,
        };
        Some(tech)
    } else {
        None
    }
}
/// Detect the anti-bot used from the request.
pub fn detect_anti_bot_tech_response(
    url: &str,
    headers: &HeaderSource,
    body: &Vec<u8>,
    subject_name: Option<&str>,
) -> AntiBotTech {
    // Check by TLS subject (Chrome/CDP TLS details)
    if let Some(subject) = subject_name {
        if subject == "challenges.cloudflare.com" {
            return AntiBotTech::Cloudflare;
        }
    }

    if let Some(tech) = detect_anti_bot_from_headers(headers) {
        return tech;
    }

    if let Some(tech) = detect_antibot_from_url(url) {
        return tech;
    }

    if let Some(anti_bot) = detect_anti_bot_from_body(body) {
        return anti_bot;
    }

    AntiBotTech::None
}

/// Extract to JsonResponse struct. This does nothing without 'openai' feature flag.
#[cfg(feature = "openai")]
pub fn handle_ai_data(js: &str) -> Option<JsonResponse> {
    match serde_json::from_str::<JsonResponse>(&js) {
        Ok(x) => Some(x),
        _ => None,
    }
}

#[cfg(feature = "chrome")]
#[derive(Default, Clone, Debug)]
/// The chrome HTTP response.
pub struct ChromeHTTPReqRes {
    /// Is the request blocked by a firewall?
    pub waf_check: bool,
    /// The HTTP status code.
    pub status_code: StatusCode,
    /// The HTTP method of the request.
    pub method: String,
    /// The HTTP response headers for the request.
    pub response_headers: std::collections::HashMap<String, String>,
    /// The HTTP request headers for the request.
    pub request_headers: std::collections::HashMap<String, String>,
    /// The HTTP protocol of the request.
    pub protocol: String,
    /// The anti-bot tech used.
    pub anti_bot_tech: crate::page::AntiBotTech,
}

#[cfg(feature = "chrome")]
/// Perform a http future with chrome.
pub async fn perform_chrome_http_request(
    page: &chromiumoxide::Page,
    source: &str,
    referrer: Option<String>,
) -> Result<ChromeHTTPReqRes, chromiumoxide::error::CdpError> {
    let mut waf_check = false;
    let mut status_code = StatusCode::OK;
    let mut method = String::from("GET");
    let mut response_headers: std::collections::HashMap<String, String> =
        std::collections::HashMap::default();
    let mut request_headers = std::collections::HashMap::default();
    let mut protocol = String::from("http/1.1");
    let mut anti_bot_tech = AntiBotTech::default();

    let frame_id = page.mainframe().await?;

    let page_base =
        page.http_future(chromiumoxide::cdp::browser_protocol::page::NavigateParams {
            url: source.to_string(),
            transition_type: Some(
                chromiumoxide::cdp::browser_protocol::page::TransitionType::Other,
            ),
            frame_id,
            referrer,
            referrer_policy: None,
        })?;

    match page_base.await {
        Ok(page_base) => {
            if let Some(http_request) = page_base {
                if let Some(http_method) = http_request.method.as_deref() {
                    method = http_method.into();
                }

                request_headers.clone_from(&http_request.headers);

                if let Some(ref response) = http_request.response {
                    if let Some(ref p) = response.protocol {
                        protocol.clone_from(p);
                    }

                    if let Some(res_headers) = response.headers.inner().as_object() {
                        for (k, v) in res_headers {
                            response_headers.insert(k.to_string(), v.to_string());
                        }
                    }

                    let mut firewall = false;

                    if !response.url.starts_with(source) {
                        match &response.security_details {
                            Some(security_details) => {
                                anti_bot_tech = detect_anti_bot_tech_response(
                                    &response.url,
                                    &HeaderSource::Map(&response_headers),
                                    &Default::default(),
                                    Some(&security_details.subject_name),
                                );
                                firewall = true;
                            }
                            _ => {
                                anti_bot_tech = detect_anti_bot_tech_response(
                                    &response.url,
                                    &HeaderSource::Map(&response_headers),
                                    &Default::default(),
                                    None,
                                );
                                if anti_bot_tech == AntiBotTech::Cloudflare {
                                    if let Some(xframe_options) =
                                        response_headers.get("x-frame-options")
                                    {
                                        if xframe_options == r#"\"DENY\""# {
                                            firewall = true;
                                        }
                                    } else if let Some(encoding) =
                                        response_headers.get("Accept-Encoding")
                                    {
                                        if encoding == r#"cf-ray"# {
                                            firewall = true;
                                        }
                                    }
                                } else {
                                    firewall = true;
                                }
                            }
                        };

                        waf_check = firewall && !matches!(anti_bot_tech, AntiBotTech::None);

                        if !waf_check {
                            waf_check = match response.protocol {
                                Some(ref protocol) => protocol == "blob",
                                _ => false,
                            }
                        }
                    }

                    status_code = StatusCode::from_u16(response.status as u16)
                        .unwrap_or_else(|_| StatusCode::EXPECTATION_FAILED);
                } else {
                    if let Some(failure_text) = &http_request.failure_text {
                        if failure_text == "net::ERR_FAILED" {
                            waf_check = true;
                        }
                    }
                }
            }
        }
        Err(e) => return Err(e),
    }

    Ok(ChromeHTTPReqRes {
        waf_check,
        status_code,
        method,
        response_headers,
        request_headers,
        protocol,
        anti_bot_tech,
    })
}

/// Use OpenAI to extend the crawl. This does nothing without 'openai' feature flag.
#[cfg(all(feature = "chrome", not(feature = "openai")))]
pub async fn run_openai_request(
    _source: &str,
    _page: &chromiumoxide::Page,
    _wait_for: &Option<crate::configuration::WaitFor>,
    _openai_config: &Option<Box<crate::configuration::GPTConfigs>>,
    _page_response: &mut PageResponse,
    _ok: bool,
) {
}

/// Use OpenAI to extend the crawl. This does nothing without 'openai' feature flag.
#[cfg(all(feature = "chrome", feature = "openai"))]
pub async fn run_openai_request(
    source: &str,
    page: &chromiumoxide::Page,
    wait_for: &Option<crate::configuration::WaitFor>,
    openai_config: &Option<Box<crate::configuration::GPTConfigs>>,
    mut page_response: &mut PageResponse,
    ok: bool,
) {
    if let Some(gpt_configs) = openai_config {
        let gpt_configs = match gpt_configs.prompt_url_map {
            Some(ref h) => {
                let c = h.get::<case_insensitive_string::CaseInsensitiveString>(&source.into());

                if !c.is_some() && gpt_configs.paths_map {
                    h.get::<case_insensitive_string::CaseInsensitiveString>(
                        &get_path_from_url(&source).into(),
                    )
                } else {
                    c
                }
            }
            _ => Some(gpt_configs),
        };

        if let Some(gpt_configs) = gpt_configs {
            let mut prompts = gpt_configs.prompt.clone();

            while let Some(prompt) = prompts.next() {
                let gpt_results = if !gpt_configs.model.is_empty() && ok {
                    openai_request(
                        gpt_configs,
                        match page_response.content.as_ref() {
                            Some(html) => auto_encoder::auto_encode_bytes(html),
                            _ => Default::default(),
                        },
                        &source,
                        &prompt,
                    )
                    .await
                } else {
                    Default::default()
                };

                let js_script = gpt_results.response;
                let tokens_used = gpt_results.usage;
                let gpt_error = gpt_results.error;

                // set the credits used for the request
                handle_openai_credits(&mut page_response, tokens_used);

                let json_res = if gpt_configs.extra_ai_data {
                    match handle_ai_data(&js_script) {
                        Some(jr) => jr,
                        _ => {
                            let mut jr = JsonResponse::default();
                            jr.error = Some("An issue occured with serialization.".into());

                            jr
                        }
                    }
                } else {
                    let mut x = JsonResponse::default();
                    x.js = js_script;
                    x
                };

                // perform the js script on the page.
                if !json_res.js.is_empty() {
                    let html: Option<Box<Vec<u8>>> = match page
                        .evaluate_function(string_concat!(
                            "async function() { ",
                            json_res.js,
                            "; return document.documentElement.outerHTML; }"
                        ))
                        .await
                    {
                        Ok(h) => match h.into_value() {
                            Ok(hh) => Some(hh),
                            _ => None,
                        },
                        _ => None,
                    };

                    if html.is_some() {
                        page_wait(&page, &wait_for).await;
                        if json_res.js.len() <= 400 && json_res.js.contains("window.location") {
                            if let Ok(b) = page.outer_html_bytes().await {
                                page_response.content = Some(b.into());
                            }
                        } else {
                            page_response.content = html;
                        }
                    }
                }

                // attach the data to the page
                if gpt_configs.extra_ai_data {
                    let screenshot_bytes = if gpt_configs.screenshot && !json_res.js.is_empty() {
                        let format = chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat::Png;

                        let screenshot_configs = chromiumoxide::page::ScreenshotParams::builder()
                            .format(format)
                            .full_page(true)
                            .quality(45)
                            .omit_background(false);

                        match page.screenshot(screenshot_configs.build()).await {
                            Ok(b) => {
                                log::debug!("took screenshot: {:?}", source);
                                Some(b)
                            }
                            Err(e) => {
                                log::error!("failed to take screenshot: {:?} - {:?}", e, source);
                                None
                            }
                        }
                    } else {
                        None
                    };

                    handle_extra_ai_data(
                        page_response,
                        &prompt,
                        json_res,
                        screenshot_bytes,
                        gpt_error,
                    );
                }
            }
        }
    }
}

/// Represents an HTTP version
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum HttpVersion {
    /// HTTP Version 0.9
    Http09,
    /// HTTP Version 1.0
    Http10,
    /// HTTP Version 1.1
    Http11,
    /// HTTP Version 2.0
    H2,
    /// HTTP Version 3.0
    H3,
}

/// A basic generic type that represents an HTTP response.
#[derive(Debug, Clone)]
pub struct HttpResponse {
    /// HTTP response body
    pub body: Vec<u8>,
    /// HTTP response headers
    pub headers: std::collections::HashMap<String, String>,
    /// HTTP response status code
    pub status: u16,
    /// HTTP response url
    pub url: url::Url,
    /// HTTP response version
    pub version: HttpVersion,
}

/// A HTTP request type for caching.
#[cfg(feature = "cache_chrome_hybrid")]
pub struct HttpRequestLike {
    ///  The URI component of a request.
    pub uri: http::uri::Uri,
    /// The http method.
    pub method: reqwest::Method,
    /// The http headers.
    pub headers: http::HeaderMap,
}

#[cfg(feature = "cache_chrome_hybrid")]
/// A HTTP response type for caching.
pub struct HttpResponseLike {
    /// The http status code.
    pub status: StatusCode,
    /// The http headers.
    pub headers: http::HeaderMap,
}

#[cfg(feature = "cache_chrome_hybrid")]
impl RequestLike for HttpRequestLike {
    fn uri(&self) -> http::uri::Uri {
        self.uri.clone()
    }
    fn is_same_uri(&self, other: &http::Uri) -> bool {
        &self.uri == other
    }
    fn method(&self) -> &reqwest::Method {
        &self.method
    }
    fn headers(&self) -> &http::HeaderMap {
        &self.headers
    }
}

#[cfg(feature = "cache_chrome_hybrid")]
impl ResponseLike for HttpResponseLike {
    fn status(&self) -> StatusCode {
        self.status
    }
    fn headers(&self) -> &http::HeaderMap {
        &self.headers
    }
}

/// Convert headers to header map
#[cfg(any(
    feature = "cache_chrome_hybrid",
    feature = "headers",
    feature = "cookies"
))]
pub fn convert_headers(
    headers: &std::collections::HashMap<String, String>,
) -> reqwest::header::HeaderMap {
    let mut header_map = reqwest::header::HeaderMap::new();

    for (index, items) in headers.iter().enumerate() {
        if let Ok(head) = reqwest::header::HeaderValue::from_str(items.1) {
            use std::str::FromStr;
            if let Ok(key) = reqwest::header::HeaderName::from_str(items.0) {
                header_map.insert(key, head);
            }
        }
        // mal headers
        if index > 1000 {
            break;
        }
    }

    header_map
}

#[cfg(feature = "cache_chrome_hybrid")]
/// Store the page to cache to be re-used across HTTP request.
pub async fn put_hybrid_cache(
    cache_key: &str,
    http_response: HttpResponse,
    method: &str,
    http_request_headers: std::collections::HashMap<String, String>,
) {
    use crate::http_cache_reqwest::CacheManager;
    use http_cache_semantics::CachePolicy;

    match http_response.url.as_str().parse::<http::uri::Uri>() {
        Ok(u) => {
            let req = HttpRequestLike {
                uri: u,
                method: reqwest::Method::from_bytes(method.as_bytes())
                    .unwrap_or(reqwest::Method::GET),
                headers: convert_headers(&http_response.headers),
            };

            let res = HttpResponseLike {
                status: StatusCode::from_u16(http_response.status)
                    .unwrap_or(StatusCode::EXPECTATION_FAILED),
                headers: convert_headers(&http_request_headers),
            };

            let policy = CachePolicy::new(&req, &res);

            let _ = crate::website::CACACHE_MANAGER
                .put(
                    cache_key.into(),
                    http_cache_reqwest::HttpResponse {
                        url: http_response.url,
                        body: http_response.body,
                        headers: http_response.headers,
                        version: match http_response.version {
                            HttpVersion::H2 => http_cache::HttpVersion::H2,
                            HttpVersion::Http10 => http_cache::HttpVersion::Http10,
                            HttpVersion::H3 => http_cache::HttpVersion::H3,
                            HttpVersion::Http09 => http_cache::HttpVersion::Http09,
                            HttpVersion::Http11 => http_cache::HttpVersion::Http11,
                        },
                        status: http_response.status,
                    },
                    policy,
                )
                .await;
        }
        _ => (),
    }
}

#[cfg(not(feature = "cache_chrome_hybrid"))]
/// Store the page to cache to be re-used across HTTP request.
pub async fn put_hybrid_cache(
    _cache_key: &str,
    _http_response: HttpResponse,
    _method: &str,
    _http_request_headers: std::collections::HashMap<String, String>,
) {
}

/// Subtract the duration with overflow handling.
#[cfg(feature = "chrome")]
fn sub_duration(
    base_timeout: std::time::Duration,
    elapsed: std::time::Duration,
) -> std::time::Duration {
    match base_timeout.checked_sub(elapsed) {
        Some(remaining_time) => remaining_time,
        None => Default::default(),
    }
}

/// Get the initial page headers of the page with navigation.
#[cfg(feature = "chrome")]
async fn navigate(
    page: &chromiumoxide::Page,
    url: &str,
    chrome_http_req_res: &mut ChromeHTTPReqRes,
    referrer: Option<String>,
) -> Result<(), chromiumoxide::error::CdpError> {
    *chrome_http_req_res = perform_chrome_http_request(page, url, referrer).await?;
    Ok(())
}

#[cfg(all(feature = "real_browser", feature = "chrome"))]
/// generate random mouse movement.
async fn perform_smart_mouse_movement(
    page: &chromiumoxide::Page,
    viewport: &Option<crate::configuration::Viewport>,
) {
    use chromiumoxide::layout::Point;
    use fastrand::Rng;
    use spider_fingerprint::spoof_mouse_movement::GaussianMouse;
    use tokio::time::{sleep, Duration};

    let (viewport_width, viewport_height) = match viewport {
        Some(vp) => (vp.width as f64, vp.height as f64),
        None => (800.0, 600.0),
    };

    let mut rng = Rng::new();

    for (x, y) in GaussianMouse::generate_random_coordinates(viewport_width, viewport_height) {
        let _ = page.move_mouse(Point::new(x, y)).await;

        // Occasionally introduce a short pause (~25%)
        if rng.f32() < 0.25 {
            let delay_micros = if rng.f32() < 0.9 {
                rng.u64(300..=1200) // 0.3–1.2 ms
            } else {
                rng.u64(2000..=8000) // rare 2–8 ms (real hesitation)
            };
            sleep(Duration::from_micros(delay_micros)).await;
        }
    }
}

#[cfg(all(not(feature = "real_browser"), feature = "chrome"))]
async fn perform_smart_mouse_movement(
    _page: &chromiumoxide::Page,
    _viewport: &Option<crate::configuration::Viewport>,
) {
}

/// Cache the chrome response
#[cfg(all(feature = "chrome", feature = "cache_chrome_hybrid"))]
pub async fn cache_chrome_response(
    target_url: &str,
    page_response: &PageResponse,
    chrome_http_req_res: ChromeHTTPReqRes,
) {
    if let Ok(u) = url::Url::parse(target_url) {
        let http_response = HttpResponse {
            url: u,
            body: match page_response.content.as_ref() {
                Some(b) => b.into(),
                _ => Default::default(),
            },
            status: chrome_http_req_res.status_code.into(),
            version: match chrome_http_req_res.protocol.as_str() {
                "http/0.9" => HttpVersion::Http09,
                "http/1" | "http/1.0" => HttpVersion::Http10,
                "http/1.1" => HttpVersion::Http11,
                "http/2.0" | "http/2" => HttpVersion::H2,
                "http/3.0" | "http/3" => HttpVersion::H3,
                _ => HttpVersion::Http11,
            },
            headers: chrome_http_req_res.response_headers,
        };
        put_hybrid_cache(
            &string_concat!("GET", ":", target_url),
            http_response,
            &"GET",
            chrome_http_req_res.request_headers,
        )
        .await;
    }
}

/// Cache the chrome response
#[cfg(all(feature = "chrome", not(feature = "cache_chrome_hybrid")))]
pub async fn cache_chrome_response(
    _target_url: &str,
    _page_response: &PageResponse,
    _chrome_http_req_res: ChromeHTTPReqRes,
) {
}

/// 5 mins in ms
pub(crate) const FIVE_MINUTES: u32 = 300000;

/// Max page timeout for events.
#[cfg(feature = "chrome")]
const MAX_PAGE_TIMEOUT: tokio::time::Duration =
    tokio::time::Duration::from_millis(FIVE_MINUTES as u64);
/// Half of the max timeout
#[cfg(feature = "chrome")]
const HALF_MAX_PAGE_TIMEOUT: tokio::time::Duration =
    tokio::time::Duration::from_millis(FIVE_MINUTES as u64 / 2);

#[cfg(all(feature = "chrome", feature = "headers"))]
fn store_headers(page_response: &PageResponse, chrome_http_req_res: &mut ChromeHTTPReqRes) {
    if let Some(response_headers) = &page_response.headers {
        chrome_http_req_res.response_headers =
            crate::utils::header_utils::header_map_to_hash_map(&response_headers);
    }
}

#[cfg(all(feature = "chrome", not(feature = "headers")))]
fn store_headers(_page_response: &PageResponse, _chrome_http_req_res: &mut ChromeHTTPReqRes) {}

#[cfg(feature = "chrome")]
/// Perform a network request to a resource extracting all content as text streaming via chrome.
pub async fn fetch_page_html_chrome_base(
    source: &str,
    page: &chromiumoxide::Page,
    content: bool,
    wait_for_navigation: bool,
    wait_for: &Option<crate::configuration::WaitFor>,
    screenshot: &Option<crate::configuration::ScreenShotConfig>,
    page_set: bool,
    openai_config: &Option<Box<crate::configuration::GPTConfigs>>,
    url_target: Option<&str>,
    execution_scripts: &Option<ExecutionScripts>,
    automation_scripts: &Option<AutomationScripts>,
    viewport: &Option<crate::configuration::Viewport>,
    request_timeout: &Option<Box<std::time::Duration>>,
    track_events: &Option<crate::configuration::ChromeEventTracker>,
    referrer: Option<String>,
    max_page_bytes: Option<f64>,
) -> Result<PageResponse, chromiumoxide::error::CdpError> {
    use crate::page::{is_asset_url, DOWNLOADABLE_MEDIA_TYPES, UNKNOWN_STATUS_ERROR};
    use chromiumoxide::{
        cdp::browser_protocol::network::{
            EventLoadingFailed, EventRequestWillBeSent, EventResponseReceived,
            GetResponseBodyParams, RequestId, ResourceType,
        },
        error::CdpError,
    };
    use hashbrown::HashMap;
    use std::time::Duration;
    use tokio::{
        sync::{oneshot, OnceCell},
        time::Instant,
    };

    #[derive(Debug, Clone, Default)]
    /// Map of the response.
    struct ResponseMap {
        /// The url of the request
        url: String,
        /// The network request was skipped.
        skipped: bool,
        /// The bytes transferred
        bytes_transferred: f64,
    }

    #[derive(Debug, Clone, Default)]
    struct ResponseBase {
        /// The map of the response.
        response_map: Option<HashMap<String, ResponseMap>>,
        /// The headers of request.
        headers: Option<chromiumoxide::cdp::browser_protocol::network::Headers>,
        /// The status code.
        status_code: Option<i64>,
    }

    let mut chrome_http_req_res = ChromeHTTPReqRes::default();

    // the base networking timeout to prevent any hard hangs.
    let mut base_timeout = match request_timeout {
        Some(timeout) => **timeout.min(&Box::new(MAX_PAGE_TIMEOUT)),
        _ => MAX_PAGE_TIMEOUT,
    };

    // track the initial base without modifying.
    let base_timeout_measurement = base_timeout;
    let target_url = url_target.unwrap_or(source);
    let asset = is_asset_url(target_url);

    let (tx1, rx1) = if asset {
        let c = oneshot::channel::<Option<RequestId>>();

        (Some(c.0), Some(c.1))
    } else {
        (None, None)
    };

    let (track_requests, track_responses) = match track_events {
        Some(tracker) => (tracker.requests, tracker.responses),
        _ => (false, false),
    };

    let (event_loading_listener, cancel_listener, received_listener, event_sent_listener) = tokio::join!(
        page.event_listener::<chromiumoxide::cdp::browser_protocol::network::EventLoadingFinished>(
        ),
        page.event_listener::<EventLoadingFailed>(),
        async {
            if asset || track_responses {
                page.event_listener::<EventResponseReceived>().await
            } else {
                Err(CdpError::NotFound)
            }
        },
        async {
            if track_requests {
                page.event_listener::<EventRequestWillBeSent>().await
            } else {
                Err(CdpError::NotFound)
            }
        },
    );

    let (tx, rx) = oneshot::channel::<bool>();

    let page_clone = if max_page_bytes.is_some() {
        Some(page.clone())
    } else {
        None
    };

    // Listen for network events. todo: capture the last values endtime to track period.
    let bytes_collected_handle = tokio::spawn(async move {
        let finished_media: Option<OnceCell<RequestId>> =
            if asset { Some(OnceCell::new()) } else { None };

        let f1 = async {
            let mut total = 0.0;
            let total_max = max_page_bytes.unwrap_or_default();

            let mut response_map: Option<HashMap<String, f64>> = if track_responses {
                Some(HashMap::new())
            } else {
                None
            };

            if let Ok(mut listener) = event_loading_listener {
                if asset {
                    while let Some(event) = listener.next().await {
                        total += event.encoded_data_length;

                        if let Some(response_map) = response_map.as_mut() {
                            response_map
                                .entry(event.request_id.inner().clone())
                                .and_modify(|e| *e += event.encoded_data_length)
                                .or_insert(event.encoded_data_length);
                        }

                        if let Some(page_clone) = &page_clone {
                            if total > total_max {
                                let _ = page_clone.block_all_urls().await;
                            }
                        }

                        if let Some(once) = &finished_media {
                            if let Some(request_id) = once.get() {
                                if request_id == &event.request_id {
                                    if let Some(tx1) = tx1 {
                                        let _ = tx1.send(Some(request_id.clone()));
                                        break;
                                    }
                                }
                            }
                        }
                    }
                } else {
                    while let Some(event) = listener.next().await {
                        total += event.encoded_data_length;
                        if let Some(response_map) = response_map.as_mut() {
                            response_map
                                .entry(event.request_id.inner().clone())
                                .and_modify(|e| *e += event.encoded_data_length)
                                .or_insert(event.encoded_data_length);
                        }
                        if let Some(page_clone) = &page_clone {
                            if total > total_max {
                                let _ = page_clone.block_all_urls().await;
                            }
                        }
                    }
                }
            }

            (total, response_map)
        };

        let f2 = async {
            if let Ok(mut listener) = cancel_listener {
                let mut net_aborted = false;

                while let Some(event) = listener.next().await {
                    if event.r#type == ResourceType::Document
                        && event.error_text == "net::ERR_ABORTED"
                        && event.canceled.unwrap_or_default()
                    {
                        net_aborted = true;
                        break;
                    }
                }

                if net_aborted {
                    let _ = tx.send(true);
                }
            }
        };

        let f3 = async {
            let mut response_map: Option<HashMap<String, ResponseMap>> = if track_responses {
                Some(HashMap::new())
            } else {
                None
            };

            let mut status_code = None;
            let mut headers = None;

            if asset || response_map.is_some() {
                if let Ok(mut listener) = received_listener {
                    let mut initial_asset = false;
                    let mut allow_download = false;
                    let mut intial_request = false;

                    while let Some(event) = listener.next().await {
                        if !intial_request && event.r#type == ResourceType::Document {
                            let redirect =
                                event.response.status >= 300 && event.response.status <= 399;

                            if !redirect {
                                intial_request = true;
                                status_code = Some(event.response.status);
                                headers = Some(event.response.headers.clone());
                            }
                        }
                        // check if media asset needs to be downloaded.
                        if asset {
                            if !initial_asset && event.r#type == ResourceType::Document {
                                allow_download =
                                    DOWNLOADABLE_MEDIA_TYPES.contains(&event.response.mime_type);
                            }
                            if event.r#type == ResourceType::Media && allow_download {
                                if let Some(once) = &finished_media {
                                    let _ = once.set(event.request_id.clone());
                                }
                            }
                            initial_asset = true;
                        }

                        if let Some(response_map) = response_map.as_mut() {
                            response_map.insert(
                                event.request_id.inner().clone(),
                                ResponseMap {
                                    url: event.response.url.clone(),
                                    bytes_transferred: event.response.encoded_data_length,
                                    skipped: *MASK_BYTES_INTERCEPTION
                                        && event.response.connection_id == 0.0
                                        && event.response.encoded_data_length <= 17.0,
                                },
                            );
                        }
                    }
                }
            }

            ResponseBase {
                response_map,
                status_code,
                headers,
            }
        };

        let f4 = async {
            let mut request_map: Option<HashMap<String, f64>> = if track_requests {
                Some(HashMap::new())
            } else {
                None
            };

            if request_map.is_some() {
                if let Some(response_map) = request_map.as_mut() {
                    if let Ok(mut listener) = event_sent_listener {
                        while let Some(event) = listener.next().await {
                            response_map
                                .insert(event.request.url.clone(), *event.timestamp.inner());
                        }
                    }
                }
            }

            request_map
        };

        let (t1, _, res_map, req_map) = tokio::join!(f1, f2, f3, f4);

        (t1.0, t1.1, res_map, req_map)
    });

    let mut block_bytes = false;

    let page_navigation = async {
        if !page_set {
            // used for smart mode re-rendering direct assigning html
            if content {
                if let Ok(frame) = page.mainframe().await {
                    let html = rewrite_base_tag(&source, &url_target).await;

                    if let Err(e) = page
                        .execute(
                            chromiumoxide::cdp::browser_protocol::page::SetDocumentContentParams {
                                frame_id: frame.unwrap_or_default(),
                                html,
                            },
                        )
                        .await
                    {
                        log::info!(
                            "Set Content Error({:?}) - {:?}",
                            e,
                            &url_target.unwrap_or(source)
                        );
                        if let chromiumoxide::error::CdpError::Timeout = e {
                            block_bytes = true;
                        }
                    }
                }
            } else {
                if let Err(e) = navigate(page, source, &mut chrome_http_req_res, referrer).await {
                    log::info!(
                        "Navigation Error({:?}) - {:?}",
                        e,
                        &url_target.unwrap_or(source)
                    );
                    if let chromiumoxide::error::CdpError::Timeout = e {
                        block_bytes = true;
                    }
                    return Err(e);
                };
            }
        }

        Ok(())
    };

    let start_time = Instant::now();

    let mut request_cancelled = false;

    let page_navigate = async {
        if cfg!(feature = "real_browser") {
            let notify = tokio::sync::Notify::new();

            let mouse_loop = async {
                let mut index = 0;

                loop {
                    tokio::select! {
                        _ = notify.notified() => {
                            break;
                        }
                        _ = perform_smart_mouse_movement(&page, &viewport) => {
                            tokio::time::sleep(std::time::Duration::from_millis(WAIT_TIMEOUTS[index])).await;
                        }
                    }

                    index = (index + 1) % WAIT_TIMEOUTS.len();
                }
            };

            let navigation_loop = async {
                let result = page_navigation.await;
                notify.notify_waiters();
                result
            };

            let (result, _) = tokio::join!(navigation_loop, mouse_loop);

            result
        } else {
            page_navigation.await
        }
    };

    tokio::select! {
        v = tokio::time::timeout(base_timeout + Duration::from_millis(50), page_navigate) => {
            if v.is_err() {
                request_cancelled = true;
            }
        }
        v = rx => {
            if let Ok(v) = v {
                request_cancelled = !v;
            }
        }
    };

    base_timeout = sub_duration(base_timeout_measurement, start_time.elapsed());

    // we do not need to wait for navigation if content is assigned. The method set_content already handles this.
    let final_url = if wait_for_navigation && !request_cancelled && !block_bytes {
        let last_redirect = tokio::time::timeout(base_timeout, async {
            match page.wait_for_navigation_response().await {
                Ok(u) => get_last_redirect(&source, &u, &page).await,
                _ => None,
            }
        })
        .await;
        base_timeout = sub_duration(base_timeout_measurement, start_time.elapsed());
        match last_redirect {
            Ok(last) => last,
            _ => None,
        }
    } else {
        None
    };

    let chrome_http_req_res1 = if asset {
        Some(chrome_http_req_res.clone())
    } else {
        None
    };

    let run_events = !base_timeout.is_zero()
        && !block_bytes
        && !request_cancelled
        && (!chrome_http_req_res.status_code.is_server_error()
            && !chrome_http_req_res.status_code.is_client_error()
            || chrome_http_req_res.status_code == *UNKNOWN_STATUS_ERROR
            || chrome_http_req_res.status_code == 404
            || chrome_http_req_res.status_code == 403
            || chrome_http_req_res.status_code.is_redirection()
            || chrome_http_req_res.status_code.is_success());

    block_bytes = chrome_http_req_res.status_code == StatusCode::REQUEST_TIMEOUT;

    let waf_check = chrome_http_req_res.waf_check;
    let mut status_code = chrome_http_req_res.status_code;
    let mut anti_bot_tech = chrome_http_req_res.anti_bot_tech;
    let mut validate_cf = false;

    let run_page_response = async move {
        let mut page_response = if run_events {
            if waf_check {
                base_timeout = sub_duration(base_timeout_measurement, start_time.elapsed());
                if let Err(elasped) = tokio::time::timeout(
                    base_timeout,
                    perform_smart_mouse_movement(&page, &viewport),
                )
                .await
                {
                    log::warn!("mouse movement timeout exceeded {elasped}");
                }
            }

            if wait_for.is_some() {
                base_timeout = sub_duration(base_timeout_measurement, start_time.elapsed());
                if let Err(elasped) =
                    tokio::time::timeout(base_timeout, page_wait(&page, &wait_for)).await
                {
                    log::warn!("max wait for timeout {elasped}");
                }
            }

            base_timeout = sub_duration(base_timeout_measurement, start_time.elapsed());

            if execution_scripts.is_some() || automation_scripts.is_some() {
                let target_url = if final_url.is_some() {
                    match final_url.as_ref() {
                        Some(ref u) => u.to_string(),
                        _ => Default::default(),
                    }
                } else if url_target.is_some() {
                    url_target.unwrap_or_default().to_string()
                } else {
                    source.to_string()
                };

                if let Err(elasped) = tokio::time::timeout(base_timeout, async {
                    tokio::join!(
                        crate::features::chrome_common::eval_execution_scripts(
                            &page,
                            &target_url,
                            &execution_scripts
                        ),
                        crate::features::chrome_common::eval_automation_scripts(
                            &page,
                            &target_url,
                            &automation_scripts
                        )
                    );
                })
                .await
                {
                    log::warn!("eval scripts timeout exceeded {elasped}");
                }
            }

            let xml_target = match &final_url {
                Some(f) => f.ends_with(".xml"),
                _ => target_url.ends_with(".xml"),
            };

            let page_fn = async {
                if xml_target {
                    match page.content_bytes_xml().await {
                        Ok(page_bytes) => {
                            if page_bytes.is_empty() {
                                page.outer_html_bytes().await
                            } else {
                                Ok(page_bytes)
                            }
                        }
                        _ => page.outer_html_bytes().await,
                    }
                } else {
                    page.outer_html_bytes().await
                }
            };

            let results = tokio::time::timeout(base_timeout.max(HALF_MAX_PAGE_TIMEOUT), page_fn);

            let mut res: Box<Vec<u8>> = match results.await {
                Ok(v) => v.map(Box::new).unwrap_or_default(),
                _ => Default::default(),
            };

            let forbidden = waf_check && res.starts_with(b"<html><head>\n    <style global=") && res.ends_with(b";</script><iframe height=\"1\" width=\"1\" style=\"position: absolute; top: 0px; left: 0px; border: none; visibility: hidden;\"></iframe>\n\n</body></html>");

            if cfg!(feature = "real_browser") {
                // we can skip this check after a set bytes
                if res.len() <= crate::page::TURNSTILE_WALL_PAGE_SIZE
                    && anti_bot_tech == AntiBotTech::Cloudflare
                    || waf_check
                {
                    // detect the turnstile page.
                    if detect_cf_turnstyle(&res) {
                        if let Err(_e) = tokio::time::timeout(base_timeout, async {
                            if let Ok(success) = cf_handle(&mut res, &page).await {
                                if success {
                                    status_code = StatusCode::OK;
                                }
                            }
                        })
                        .await
                        {
                            validate_cf = true;
                        }
                    }
                }
            };

            let ok = !res.is_empty();

            if validate_cf && ok {
                if !detect_cf_turnstyle(&res) && status_code == StatusCode::FORBIDDEN {
                    status_code = StatusCode::OK;
                }
            }

            let mut page_response = set_page_response(
                ok,
                res,
                if forbidden {
                    StatusCode::FORBIDDEN
                } else {
                    status_code
                },
                final_url,
            );

            base_timeout = sub_duration(base_timeout_measurement, start_time.elapsed());

            let _ = tokio::time::timeout(
                base_timeout,
                set_page_response_cookies(&mut page_response, &page),
            )
            .await;

            if openai_config.is_some() && !base_timeout.is_zero() {
                base_timeout = sub_duration(base_timeout_measurement, start_time.elapsed());

                let openai_request = run_openai_request(
                    match url_target {
                        Some(ref ut) => ut,
                        _ => source,
                    },
                    page,
                    wait_for,
                    openai_config,
                    &mut page_response,
                    ok,
                );

                let _ = tokio::time::timeout(base_timeout, openai_request).await;
            }

            if cfg!(feature = "chrome_screenshot") || screenshot.is_some() {
                let _ = tokio::time::timeout(
                    base_timeout + tokio::time::Duration::from_secs(30),
                    perform_screenshot(source, page, screenshot, &mut page_response),
                )
                .await;
            }

            page_response
        } else {
            let res = if !block_bytes {
                let results = tokio::time::timeout(
                    base_timeout.max(HALF_MAX_PAGE_TIMEOUT),
                    page.outer_html_bytes(),
                );

                match results.await {
                    Ok(v) => v.map(Box::new).unwrap_or_default(),
                    _ => Default::default(),
                }
            } else {
                Default::default()
            };

            let mut page_response = set_page_response(!res.is_empty(), res, status_code, final_url);

            if !block_bytes {
                let _ = tokio::time::timeout(
                    base_timeout,
                    set_page_response_cookies(&mut page_response, &page),
                )
                .await;
            }

            if base_timeout.is_zero() && page_response.content.is_none() {
                page_response.status_code = StatusCode::REQUEST_TIMEOUT;
            }

            page_response
        };

        if content {
            if let Some(final_url) = &page_response.final_url {
                if final_url == "about:blank" {
                    page_response.final_url = None;
                }
            }
        }

        page_response
    };

    let mut content: Option<Box<Vec<u8>>> = None;

    let page_response = match rx1 {
        Some(rx1) => {
            tokio::select! {
                v = tokio::time::timeout(base_timeout, run_page_response) => {
                    v.map_err(|_| CdpError::Timeout)
                }
                c = rx1 => {
                    if let Ok(c) = c {
                        if let Some(c) = c {
                            let params =
                            GetResponseBodyParams::new(c.clone());

                            if let Ok(command_response) = page.execute(params).await {
                              let body_response = command_response;

                              let media_file = if body_response.base64_encoded {
                                  chromiumoxide::utils::base64::decode(
                                      &body_response.body,
                                  )
                                  .unwrap_or_default()
                              } else {
                                  body_response.body.as_bytes().to_vec()
                              };
                              content = Some(media_file.into());
                          }
                        }
                    }

                    let mut page_response = PageResponse::default();

                    let _ = tokio::time::timeout(
                        base_timeout,
                        set_page_response_cookies(&mut page_response, &page),
                    )
                    .await;

                    if let Some(mut chrome_http_req_res1) = chrome_http_req_res1 {
                        set_page_response_headers(&mut chrome_http_req_res1, &mut page_response);

                        page_response.status_code = chrome_http_req_res1.status_code;
                        page_response.waf_check = chrome_http_req_res1.waf_check;

                        if !page_set {
                            let _ = tokio::time::timeout(
                                base_timeout,
                                cache_chrome_response(&source, &page_response, chrome_http_req_res1),
                            )
                            .await;
                        }

                    }

                    Ok(page_response)
                }
            }
        }
        _ => Ok(run_page_response.await),
    };

    let mut page_response = page_response.unwrap_or_default();

    set_page_response_headers(&mut chrome_http_req_res, &mut page_response);
    page_response.status_code = chrome_http_req_res.status_code;
    page_response.waf_check = chrome_http_req_res.waf_check;

    if content.is_some() {
        page_response.content = content.map(|f| f.into());
    }

    if page_response.status_code == *UNKNOWN_STATUS_ERROR && page_response.content.is_some() {
        page_response.status_code = StatusCode::OK;
    }

    // run initial handling hidden anchors
    // if let Ok(new_links) = page.evaluate(crate::features::chrome::ANCHOR_EVENTS).await {
    //     if let Ok(results) = new_links.into_value::<hashbrown::HashSet<CaseInsensitiveString>>() {
    //         links.extend(page.extract_links_raw(&base, &results).await);
    //     }
    // }

    if cfg!(not(feature = "chrome_store_page")) {
        let _ = tokio::time::timeout(
            base_timeout.max(HALF_MAX_PAGE_TIMEOUT),
            page.execute(chromiumoxide::cdp::browser_protocol::page::CloseParams::default()),
        )
        .await;

        if let Ok((mut transferred, bytes_map, mut rs, request_map)) = bytes_collected_handle.await
        {
            let response_map = rs.response_map;

            if response_map.is_some() {
                let mut _response_map = HashMap::new();

                if let Some(response_map) = response_map {
                    if let Some(bytes_map) = bytes_map {
                        let detect_anti_bots =
                            response_map.len() <= 4 && anti_bot_tech == AntiBotTech::None;

                        for item in response_map {
                            if detect_anti_bots && item.1.url.contains("_Incapsula_Resource?") {
                                anti_bot_tech = AntiBotTech::Imperva;
                            }

                            let b = if item.1.skipped {
                                0.0
                            } else {
                                match bytes_map.get(&item.0) {
                                    Some(f) => *f,
                                    _ => 0.0,
                                }
                            };

                            if item.1.skipped {
                                transferred -= item.1.bytes_transferred;
                            }

                            _response_map.insert(item.1.url, b);
                        }
                    }
                }

                page_response.response_map = Some(_response_map);

                if let Some(status) = rs.status_code {
                    if let Ok(scode) = status.try_into() {
                        if let Ok(status) = StatusCode::from_u16(scode) {
                            page_response.status_code = status;
                        }
                    }
                }

                set_page_response_headers_raw(&mut rs.headers, &mut page_response);
                store_headers(&page_response, &mut chrome_http_req_res);

                if anti_bot_tech == AntiBotTech::None {
                    let final_url = match &page_response.final_url {
                        Some(final_url) => final_url,
                        _ => target_url,
                    };
                    if let Some(h) = &page_response.headers {
                        if let Some(content) = &page_response.content {
                            anti_bot_tech = detect_anti_bot_tech_response(
                                &final_url,
                                &HeaderSource::HeaderMap(h),
                                &content,
                                None,
                            );
                        }
                    }
                }

                if let Some(content) = &page_response.content {
                    // validate if the turnstile page is still open.
                    if anti_bot_tech == AntiBotTech::Cloudflare
                        && page_response.status_code == StatusCode::FORBIDDEN
                    {
                        let cf_turnstile = detect_cf_turnstyle(&content);

                        if !cf_turnstile {
                            page_response.status_code = StatusCode::OK;
                        }
                    }
                }

                if !page_set {
                    let _ = tokio::time::timeout(
                        base_timeout,
                        cache_chrome_response(&source, &page_response, chrome_http_req_res),
                    )
                    .await;
                }
            }
            if request_map.is_some() {
                page_response.request_map = request_map;
            }

            page_response.bytes_transferred = Some(transferred);
        }
    }

    page_response.anti_bot_tech = anti_bot_tech;

    Ok(page_response)
}

/// Set the page response.
#[cfg(feature = "chrome")]
fn set_page_response(
    ok: bool,
    res: Box<Vec<u8>>,
    status_code: StatusCode,
    final_url: Option<String>,
) -> PageResponse {
    PageResponse {
        content: if ok { Some(res.into()) } else { None },
        status_code,
        final_url,
        ..Default::default()
    }
}

/// Set the page response.
#[cfg(all(feature = "chrome", feature = "headers"))]
fn set_page_response_headers(
    chrome_http_req_res: &mut ChromeHTTPReqRes,
    page_response: &mut PageResponse,
) {
    let response_headers = convert_headers(&chrome_http_req_res.response_headers);

    if !response_headers.is_empty() {
        page_response.headers = Some(response_headers);
    }
}

/// Set the page response.
#[cfg(all(feature = "chrome", not(feature = "headers")))]
fn set_page_response_headers(
    _chrome_http_req_res: &mut ChromeHTTPReqRes,
    _page_response: &mut PageResponse,
) {
}

/// Set the page response.
#[cfg(all(feature = "chrome", feature = "headers"))]
fn set_page_response_headers_raw(
    chrome_http_req_res: &mut Option<chromiumoxide::cdp::browser_protocol::network::Headers>,
    page_response: &mut PageResponse,
) {
    if let Some(chrome_headers) = chrome_http_req_res {
        let mut header_map = reqwest::header::HeaderMap::new();

        if let Some(obj) = chrome_headers.inner().as_object() {
            for (index, (key, value)) in obj.iter().enumerate() {
                use std::str::FromStr;
                if let (Ok(header_name), Ok(header_value)) = (
                    reqwest::header::HeaderName::from_str(key),
                    reqwest::header::HeaderValue::from_str(&value.to_string()),
                ) {
                    header_map.insert(header_name, header_value);
                }
                if index > 1000 {
                    break;
                }
            }
        }
        if !header_map.is_empty() {
            page_response.headers = Some(header_map);
        }
    }
}

/// Set the page response.
#[cfg(all(feature = "chrome", not(feature = "headers")))]
fn set_page_response_headers_raw(
    _chrome_http_req_res: &mut Option<chromiumoxide::cdp::browser_protocol::network::Headers>,
    _page_response: &mut PageResponse,
) {
}

/// Set the page response.
#[cfg(all(feature = "chrome", feature = "cookies"))]
async fn set_page_response_cookies(page_response: &mut PageResponse, page: &chromiumoxide::Page) {
    if let Ok(mut cookies) = page.get_cookies().await {
        let mut cookies_map: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();

        for cookie in cookies.drain(..) {
            cookies_map.insert(cookie.name, cookie.value);
        }

        let response_headers = convert_headers(&cookies_map);

        if !response_headers.is_empty() {
            page_response.cookies = Some(response_headers);
        }
    }
}

/// Set the page response.
#[cfg(all(feature = "chrome", not(feature = "cookies")))]
async fn set_page_response_cookies(_page_response: &mut PageResponse, _page: &chromiumoxide::Page) {
}

/// Perform a screenshot shortcut.
#[cfg(feature = "chrome")]
pub async fn perform_screenshot(
    target_url: &str,
    page: &chromiumoxide::Page,
    screenshot: &Option<crate::configuration::ScreenShotConfig>,
    page_response: &mut PageResponse,
) {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;

    match screenshot {
        Some(ref ss) => {
            let output_format = string_concat!(
                ".",
                ss.params
                    .cdp_params
                    .format
                    .as_ref()
                    .unwrap_or_else(|| &crate::configuration::CaptureScreenshotFormat::Png)
                    .to_string()
            );
            let ss_params = chromiumoxide::page::ScreenshotParams::from(ss.params.clone());

            let full_page = ss_params.full_page.unwrap_or_default();
            let omit_background = ss_params.omit_background.unwrap_or_default();
            let mut cdp_params = ss_params.cdp_params;

            cdp_params.optimize_for_speed = Some(true);

            if full_page {
                cdp_params.capture_beyond_viewport = Some(true);
            }

            if omit_background {
                let _ = page.execute(chromiumoxide::cdp::browser_protocol::emulation::SetDefaultBackgroundColorOverrideParams {
                    color: Some(chromiumoxide::cdp::browser_protocol::dom::Rgba {
                        r: 0,
                        g: 0,
                        b: 0,
                        a: Some(0.),
                    }),
                })
                .await;
            }

            match page.execute(cdp_params).await {
                Ok(b) => match STANDARD.decode(&b.data) {
                    Ok(b) => {
                        if ss.save {
                            let output_path = create_output_path(
                                &ss.output_dir.clone().unwrap_or_else(|| "./storage/".into()),
                                &target_url,
                                &output_format,
                            )
                            .await;
                            let _ = tokio::fs::write(output_path, &b).await;
                        }
                        if ss.bytes {
                            page_response.screenshot_bytes = Some(b);
                        }
                    }
                    _ => (),
                },
                Err(e) => {
                    log::error!("failed to take screenshot: {:?} - {:?}", e, target_url)
                }
            };

            if omit_background {
                let _ = page.execute(chromiumoxide::cdp::browser_protocol::emulation::SetDefaultBackgroundColorOverrideParams { color: None })
                        .await;
            }
        }
        _ => {
            let output_path = create_output_path(
                &std::env::var("SCREENSHOT_DIRECTORY")
                    .unwrap_or_else(|_| "./storage/".to_string())
                    .into(),
                &target_url,
                &".png",
            )
            .await;

            match page
                .save_screenshot(
                    chromiumoxide::page::ScreenshotParams::builder()
                        .format(
                            chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat::Png,
                        )
                        .full_page(match std::env::var("SCREENSHOT_FULL_PAGE") {
                            Ok(t) => t == "true",
                            _ => true,
                        })
                        .omit_background(match std::env::var("SCREENSHOT_OMIT_BACKGROUND") {
                            Ok(t) => t == "true",
                            _ => true,
                        })
                        .build(),
                    &output_path,
                )
                .await
            {
                Ok(_) => log::debug!("saved screenshot: {:?}", output_path),
                Err(e) => log::error!("failed to save screenshot: {:?} - {:?}", e, output_path),
            };
        }
    }
}

#[cfg(feature = "chrome")]
/// Check if url matches the last item in a redirect chain for chrome CDP
pub async fn get_last_redirect(
    target_url: &str,
    u: &Option<std::sync::Arc<chromiumoxide::handler::http::HttpRequest>>,
    page: &chromiumoxide::Page,
) -> Option<String> {
    if let Some(http_request) = u {
        if let Some(redirect) = http_request.redirect_chain.last() {
            if let Some(url) = redirect.url.as_ref() {
                return if target_url != url {
                    Some(url.clone())
                } else {
                    None
                };
            }
        }
    }
    page.url().await.ok()?
}

/// The response cookies mapped. This does nothing without the cookies feature flag enabled.
#[cfg(feature = "cookies")]
pub fn get_cookies(res: &Response) -> Option<crate::client::header::HeaderMap> {
    use crate::client::header::{HeaderMap, HeaderName, HeaderValue};

    let mut headers = HeaderMap::new();

    for cookie in res.cookies() {
        if let Ok(h) = HeaderValue::from_str(cookie.value()) {
            if let Ok(n) = HeaderName::from_str(cookie.name()) {
                headers.insert(n, h);
            }
        }
    }

    if !headers.is_empty() {
        Some(headers)
    } else {
        None
    }
}

#[cfg(not(feature = "cookies"))]
/// The response cookies mapped. This does nothing without the cookies feature flag enabled.
pub fn get_cookies(res: &Response) -> Option<crate::client::header::HeaderMap> {
    None
}

/// Block streaming
pub(crate) fn block_streaming(res: &Response, only_html: bool) -> bool {
    let mut block_streaming = false;

    if only_html {
        if let Some(content_type) = res.headers().get(crate::client::header::CONTENT_TYPE) {
            if let Ok(content_type_str) = content_type.to_str() {
                if IGNORE_CONTENT_TYPES.contains(content_type_str) {
                    block_streaming = true;
                }
            }
        }
    }

    block_streaming
}

/// Handle the response bytes
pub async fn handle_response_bytes(
    res: Response,
    target_url: &str,
    only_html: bool,
) -> PageResponse {
    let u = res.url().as_str();

    let rd = if target_url != u {
        Some(u.into())
    } else {
        None
    };

    let status_code: StatusCode = res.status();
    let headers = res.headers().clone();
    #[cfg(feature = "remote_addr")]
    let remote_addr = res.remote_addr();
    let cookies = get_cookies(&res);

    let mut content: Option<Box<Vec<u8>>> = None;
    let mut anti_bot_tech = AntiBotTech::default();

    if !block_streaming(&res, only_html) {
        let mut data = match res.content_length() {
            Some(cap) if cap >= MAX_PRE_ALLOCATED_HTML_PAGE_SIZE => {
                Vec::with_capacity(cap.max(MAX_PRE_ALLOCATED_HTML_PAGE_SIZE) as usize)
            }
            _ => Vec::with_capacity(MAX_PRE_ALLOCATED_HTML_PAGE_SIZE_USIZE),
        };
        let mut stream = res.bytes_stream();
        let mut first_bytes = true;

        while let Some(item) = stream.next().await {
            match item {
                Ok(text) => {
                    if only_html && first_bytes {
                        first_bytes = false;
                        if is_binary_file(&text) {
                            break;
                        }
                    }

                    let limit = *MAX_SIZE_BYTES;

                    if limit > 0 && data.len() + text.len() > limit {
                        break;
                    }

                    data.put(text)
                }
                Err(e) => {
                    log::error!("{e} in {}", target_url);
                    break;
                }
            }
        }

        anti_bot_tech = detect_anti_bot_tech_response(
            &target_url,
            &HeaderSource::HeaderMap(&headers),
            &data,
            None,
        );
        content.replace(Box::new(data.into()));
    }

    PageResponse {
        #[cfg(feature = "headers")]
        headers: Some(headers),
        #[cfg(feature = "remote_addr")]
        remote_addr,
        #[cfg(feature = "cookies")]
        cookies,
        content,
        final_url: rd,
        status_code,
        anti_bot_tech,
        ..Default::default()
    }
}

/// Handle the response bytes writing links while crawling
pub async fn handle_response_bytes_writer<'h, O>(
    res: Response,
    target_url: &str,
    only_html: bool,
    rewriter: &mut HtmlRewriter<'h, O>,
    collected_bytes: &mut Vec<u8>,
) -> (PageResponse, bool)
where
    O: OutputSink + Send + 'static,
{
    let u = res.url().as_str();

    let final_url: Option<String> = if target_url != u {
        Some(u.into())
    } else {
        None
    };

    let status_code: StatusCode = res.status();
    let headers = res.headers().clone();
    #[cfg(feature = "remote_addr")]
    let remote_addr = res.remote_addr();
    let cookies = get_cookies(&res);
    let mut anti_bot_tech = AntiBotTech::default();

    let mut rewrite_error = false;

    if !block_streaming(&res, only_html) {
        let mut stream = res.bytes_stream();
        let mut first_bytes = true;
        let mut data_len = 0;

        while let Some(item) = stream.next().await {
            match item {
                Ok(res_bytes) => {
                    if only_html && first_bytes {
                        first_bytes = false;
                        if is_binary_file(&res_bytes) {
                            break;
                        }
                    }
                    let limit = *MAX_SIZE_BYTES;
                    let bytes_len = res_bytes.len();

                    if limit > 0 && data_len + bytes_len > limit {
                        break;
                    }

                    data_len += bytes_len;

                    if !rewrite_error {
                        if rewriter.write(&res_bytes).is_err() {
                            rewrite_error = true;
                        }
                    }

                    collected_bytes.put(res_bytes);
                }
                Err(e) => {
                    log::error!("{e} in {}", target_url);
                    break;
                }
            }
        }

        anti_bot_tech = detect_anti_bot_tech_response(
            &target_url,
            &HeaderSource::HeaderMap(&headers),
            &collected_bytes,
            None,
        );
    }

    (
        PageResponse {
            #[cfg(feature = "headers")]
            headers: Some(headers),
            #[cfg(feature = "remote_addr")]
            remote_addr,
            #[cfg(feature = "cookies")]
            cookies,
            final_url,
            status_code,
            anti_bot_tech,
            ..Default::default()
        },
        rewrite_error,
    )
}

/// Continue to parse a valid web page.
pub(crate) fn valid_parsing_status(res: &Response) -> bool {
    res.status().is_success() || res.status() == 404
}

/// Perform a network request to a resource extracting all content streaming.
async fn fetch_page_html_raw_base(
    target_url: &str,
    client: &Client,
    only_html: bool,
) -> PageResponse {
    match client.get(target_url).send().await {
        Ok(res) if valid_parsing_status(&res) => {
            handle_response_bytes(res, target_url, only_html).await
        }
        Ok(res) => handle_response_bytes(res, target_url, only_html).await,
        Err(err) => {
            log::info!("error fetching {}", target_url);
            let mut page_response = PageResponse::default();

            if let Some(status_code) = err.status() {
                page_response.status_code = status_code;
            } else {
                page_response.status_code = crate::page::get_error_http_status_code(&err);
            }

            page_response.error_for_status = Some(Err(err));
            page_response
        }
    }
}

/// Perform a network request to a resource extracting all content streaming.
pub async fn fetch_page_html_raw(target_url: &str, client: &Client) -> PageResponse {
    fetch_page_html_raw_base(target_url, client, false).await
}

/// Perform a network request to a resource extracting all content streaming.
pub async fn fetch_page_html_raw_only_html(target_url: &str, client: &Client) -> PageResponse {
    fetch_page_html_raw_base(target_url, client, false).await
}

/// Perform a network request to a resource extracting all content as text.
#[cfg(feature = "decentralized")]
pub async fn fetch_page(target_url: &str, client: &Client) -> Option<Vec<u8>> {
    match client.get(target_url).send().await {
        Ok(res) if valid_parsing_status(&res) => match res.bytes().await {
            Ok(text) => Some(text.into()),
            Err(_) => {
                log("- error fetching {}", &target_url);
                None
            }
        },
        Ok(_) => None,
        Err(_) => {
            log("- error parsing html bytes {}", &target_url);
            None
        }
    }
}

#[cfg(all(feature = "decentralized", feature = "headers"))]
/// Fetch a page with the headers returned.
pub enum FetchPageResult {
    /// Success extracting contents of the page
    Success(reqwest::header::HeaderMap, Option<Vec<u8>>),
    /// No success extracting content
    NoSuccess(reqwest::header::HeaderMap),
    /// A network error occured.
    FetchError,
}

#[cfg(all(feature = "decentralized", feature = "headers"))]
/// Perform a network request to a resource with the response headers..
pub async fn fetch_page_and_headers(target_url: &str, client: &Client) -> FetchPageResult {
    match client.get(target_url).send().await {
        Ok(res) if valid_parsing_status(&res) => {
            let headers = res.headers().clone();
            let b = match res.bytes().await {
                Ok(text) => Some(text),
                Err(_) => {
                    log("- error fetching {}", &target_url);
                    None
                }
            };
            FetchPageResult::Success(headers, b)
        }
        Ok(res) => FetchPageResult::NoSuccess(res.headers().clone()),
        Err(_) => {
            log("- error parsing html bytes {}", &target_url);
            FetchPageResult::FetchError
        }
    }
}

#[cfg(all(not(feature = "fs"), not(feature = "chrome")))]
/// Perform a network request to a resource extracting all content as text streaming.
pub async fn fetch_page_html(target_url: &str, client: &Client) -> PageResponse {
    fetch_page_html_raw(target_url, client).await
}

/// Perform a network request to a resource extracting all content as text streaming.
#[cfg(all(feature = "fs", not(feature = "chrome")))]
pub async fn fetch_page_html(target_url: &str, client: &Client) -> PageResponse {
    use crate::bytes::BufMut;
    use crate::tokio::io::{AsyncReadExt, AsyncWriteExt};
    use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};

    match client.get(target_url).send().await {
        Ok(res) if valid_parsing_status(&res) => {
            let u = res.url().as_str();

            let rd = if target_url != u {
                Some(u.into())
            } else {
                None
            };

            let status_code = res.status();
            let cookies = get_cookies(&res);
            #[cfg(feature = "headers")]
            let headers = res.headers().clone();
            #[cfg(feature = "remote_addr")]
            let remote_addr = res.remote_addr();
            let mut stream = res.bytes_stream();
            let mut data = Vec::new();
            let mut file: Option<tokio::fs::File> = None;
            let mut file_path = String::new();

            while let Some(item) = stream.next().await {
                match item {
                    Ok(text) => {
                        let wrote_disk = file.is_some();

                        // perform operations entire in memory to build resource
                        if !wrote_disk && data.capacity() < 8192 {
                            data.put(text);
                        } else {
                            if !wrote_disk {
                                file_path = string_concat!(
                                    TMP_DIR,
                                    &utf8_percent_encode(target_url, NON_ALPHANUMERIC).to_string()
                                );
                                match tokio::fs::File::create(&file_path).await {
                                    Ok(f) => {
                                        let file = file.insert(f);

                                        data.put(text);

                                        if let Ok(_) = file.write_all(&data.as_ref()).await {
                                            data.clear();
                                        }
                                    }
                                    _ => data.put(text),
                                };
                            } else {
                                if let Some(f) = file.as_mut() {
                                    if let Err(_) = f.write_all(&text).await {
                                        data.put(text)
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        log::error!("{e} in {}", target_url);
                        break;
                    }
                }
            }

            PageResponse {
                #[cfg(feature = "headers")]
                headers: Some(headers),
                #[cfg(feature = "remote_addr")]
                remote_addr,
                #[cfg(feature = "cookies")]
                cookies,
                content: Some(if file.is_some() {
                    let mut buffer = vec![];

                    if let Ok(mut b) = tokio::fs::File::open(&file_path).await {
                        if let Ok(_) = b.read_to_end(&mut buffer).await {
                            let _ = tokio::fs::remove_file(file_path).await;
                        }
                    }

                    Box::new(buffer.into())
                } else {
                    Box::new(data.into())
                }),
                status_code,
                final_url: rd,
                ..Default::default()
            }
        }
        Ok(res) => {
            let u = res.url().as_str();

            let rd = if target_url != u {
                Some(u.into())
            } else {
                None
            };

            PageResponse {
                #[cfg(feature = "headers")]
                headers: Some(res.headers().clone()),
                #[cfg(feature = "remote_addr")]
                remote_addr: res.remote_addr(),
                #[cfg(feature = "cookies")]
                cookies: get_cookies(&res),
                status_code: res.status(),
                final_url: rd,
                ..Default::default()
            }
        }
        Err(err) => {
            log::info!("error fetching {}", target_url);
            let mut page_response = PageResponse::default();

            if let Some(status_code) = err.status() {
                page_response.status_code = status_code;
            } else {
                page_response.status_code = crate::page::get_error_http_status_code(&err);
            }

            page_response.error_for_status = Some(Err(err));
            page_response
        }
    }
}

/// Perform a network request to a resource extracting all content as text streaming.
#[cfg(all(feature = "fs", feature = "chrome"))]
/// Perform a network request to a resource extracting all content as text streaming via chrome.
pub async fn fetch_page_html(
    target_url: &str,
    client: &Client,
    page: &chromiumoxide::Page,
    wait_for: &Option<crate::configuration::WaitFor>,
    screenshot: &Option<crate::configuration::ScreenShotConfig>,
    page_set: bool,
    openai_config: &Option<Box<crate::configuration::GPTConfigs>>,
    execution_scripts: &Option<ExecutionScripts>,
    automation_scripts: &Option<AutomationScripts>,
    viewport: &Option<crate::configuration::Viewport>,
    request_timeout: &Option<Box<std::time::Duration>>,
    track_events: &Option<crate::configuration::ChromeEventTracker>,
    referrer: Option<String>,
    max_page_bytes: Option<f64>,
) -> PageResponse {
    use crate::tokio::io::{AsyncReadExt, AsyncWriteExt};
    use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};

    match &page {
        page => {
            match fetch_page_html_chrome_base(
                &target_url,
                &page,
                false,
                true,
                wait_for,
                screenshot,
                page_set,
                openai_config,
                None,
                execution_scripts,
                automation_scripts,
                &viewport,
                &request_timeout,
                &track_events,
                referrer,
                max_page_bytes,
            )
            .await
            {
                Ok(page) => page,
                _ => {
                    log::info!(
                        "- error fetching chrome page defaulting to raw http request {}",
                        &target_url,
                    );

                    use crate::bytes::BufMut;

                    match client.get(target_url).send().await {
                        Ok(res) if valid_parsing_status(&res) => {
                            #[cfg(feature = "headers")]
                            let headers = res.headers().clone();
                            let cookies = get_cookies(&res);
                            let status_code = res.status();
                            let mut stream = res.bytes_stream();
                            let mut data = Vec::new();

                            let mut file: Option<tokio::fs::File> = None;
                            let mut file_path = String::new();

                            while let Some(item) = stream.next().await {
                                match item {
                                    Ok(text) => {
                                        let wrote_disk = file.is_some();

                                        // perform operations entire in memory to build resource
                                        if !wrote_disk && data.capacity() < 8192 {
                                            data.put(text);
                                        } else {
                                            if !wrote_disk {
                                                file_path = string_concat!(
                                                    TMP_DIR,
                                                    &utf8_percent_encode(
                                                        target_url,
                                                        NON_ALPHANUMERIC
                                                    )
                                                    .to_string()
                                                );
                                                match tokio::fs::File::create(&file_path).await {
                                                    Ok(f) => {
                                                        let file = file.insert(f);

                                                        data.put(text);

                                                        if let Ok(_) =
                                                            file.write_all(&data.as_ref()).await
                                                        {
                                                            data.clear();
                                                        }
                                                    }
                                                    _ => data.put(text),
                                                };
                                            } else {
                                                if let Some(f) = file.as_mut() {
                                                    if let Ok(_) = f.write_all(&text).await {
                                                        data.put(text)
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        log::error!("{e} in {}", target_url);
                                        break;
                                    }
                                }
                            }

                            PageResponse {
                                #[cfg(feature = "headers")]
                                headers: Some(headers),
                                #[cfg(feature = "remote_addr")]
                                remote_addr: res.remote_addr(),
                                #[cfg(feature = "cookies")]
                                cookies,
                                content: Some(if file.is_some() {
                                    let mut buffer = vec![];

                                    if let Ok(mut b) = tokio::fs::File::open(&file_path).await {
                                        if let Ok(_) = b.read_to_end(&mut buffer).await {
                                            let _ = tokio::fs::remove_file(file_path).await;
                                        }
                                    }

                                    Box::new(buffer.into())
                                } else {
                                    Box::new(data.into())
                                }),
                                status_code,
                                ..Default::default()
                            }
                        }

                        Ok(res) => PageResponse {
                            #[cfg(feature = "headers")]
                            headers: Some(res.headers().clone()),
                            #[cfg(feature = "remote_addr")]
                            remote_addr: res.remote_addr(),
                            #[cfg(feature = "cookies")]
                            cookies: get_cookies(&res),
                            status_code: res.status(),
                            ..Default::default()
                        },
                        Err(err) => {
                            log::info!("error fetching {}", target_url);
                            let mut page_response = PageResponse::default();

                            if let Some(status_code) = err.status() {
                                page_response.status_code = status_code;
                            } else {
                                page_response.status_code =
                                    crate::page::get_error_http_status_code(&err);
                            }

                            page_response.error_for_status = Some(Err(err));
                            page_response
                        }
                    }
                }
            }
        }
    }
}

#[cfg(all(not(feature = "fs"), feature = "chrome"))]
/// Perform a network request to a resource extracting all content as text streaming via chrome.
pub async fn fetch_page_html(
    target_url: &str,
    client: &Client,
    page: &chromiumoxide::Page,
    wait_for: &Option<crate::configuration::WaitFor>,
    screenshot: &Option<crate::configuration::ScreenShotConfig>,
    page_set: bool,
    openai_config: &Option<Box<crate::configuration::GPTConfigs>>,
    execution_scripts: &Option<ExecutionScripts>,
    automation_scripts: &Option<AutomationScripts>,
    viewport: &Option<crate::configuration::Viewport>,
    request_timeout: &Option<Box<std::time::Duration>>,
    track_events: &Option<crate::configuration::ChromeEventTracker>,
    referrer: Option<String>,
    max_page_bytes: Option<f64>,
) -> PageResponse {
    match fetch_page_html_chrome_base(
        &target_url,
        &page,
        false,
        true,
        wait_for,
        screenshot,
        page_set,
        openai_config,
        None,
        execution_scripts,
        automation_scripts,
        viewport,
        request_timeout,
        track_events,
        referrer,
        max_page_bytes,
    )
    .await
    {
        Ok(page) => page,
        Err(err) => {
            log::error!("{:?}", err);
            fetch_page_html_raw(&target_url, &client).await
        }
    }
}

#[cfg(feature = "chrome")]
/// Perform a network request to a resource extracting all content as text streaming via chrome.
pub async fn fetch_page_html_chrome(
    target_url: &str,
    client: &Client,
    page: &chromiumoxide::Page,
    wait_for: &Option<crate::configuration::WaitFor>,
    screenshot: &Option<crate::configuration::ScreenShotConfig>,
    page_set: bool,
    openai_config: &Option<Box<crate::configuration::GPTConfigs>>,
    execution_scripts: &Option<ExecutionScripts>,
    automation_scripts: &Option<AutomationScripts>,
    viewport: &Option<crate::configuration::Viewport>,
    request_timeout: &Option<Box<std::time::Duration>>,
    track_events: &Option<crate::configuration::ChromeEventTracker>,
    referrer: Option<String>,
    max_page_bytes: Option<f64>,
) -> PageResponse {
    match &page {
        page => {
            match fetch_page_html_chrome_base(
                &target_url,
                &page,
                false,
                true,
                wait_for,
                screenshot,
                page_set,
                openai_config,
                None,
                execution_scripts,
                automation_scripts,
                viewport,
                request_timeout,
                track_events,
                referrer,
                max_page_bytes,
            )
            .await
            {
                Ok(page) => page,
                Err(err) => {
                    log::error!(
                        "{:?}. Error requesting: {} - defaulting to raw http request",
                        err,
                        target_url
                    );

                    use crate::bytes::BufMut;

                    match client.get(target_url).send().await {
                        Ok(res) if valid_parsing_status(&res) => {
                            #[cfg(feature = "headers")]
                            let headers = res.headers().clone();
                            #[cfg(feature = "remote_addr")]
                            let remote_addr = res.remote_addr();
                            let cookies = get_cookies(&res);
                            let status_code = res.status();
                            let mut stream = res.bytes_stream();
                            let mut data = Vec::new();

                            while let Some(item) = stream.next().await {
                                match item {
                                    Ok(text) => {
                                        let limit = *MAX_SIZE_BYTES;

                                        if limit > 0 && data.len() + text.len() > limit {
                                            break;
                                        }
                                        data.put(text)
                                    }
                                    Err(e) => {
                                        log::error!("{e} in {}", target_url);
                                        break;
                                    }
                                }
                            }

                            PageResponse {
                                #[cfg(feature = "headers")]
                                headers: Some(headers),
                                #[cfg(feature = "remote_addr")]
                                remote_addr,
                                #[cfg(feature = "cookies")]
                                cookies,
                                content: Some(Box::new(data.into())),
                                status_code,
                                ..Default::default()
                            }
                        }
                        Ok(res) => PageResponse {
                            #[cfg(feature = "headers")]
                            headers: Some(res.headers().clone()),
                            #[cfg(feature = "remote_addr")]
                            remote_addr: res.remote_addr(),
                            #[cfg(feature = "cookies")]
                            cookies: get_cookies(&res),
                            status_code: res.status(),
                            ..Default::default()
                        },
                        Err(err) => {
                            log::info!("error fetching {}", target_url);
                            let mut page_response = PageResponse::default();

                            if let Some(status_code) = err.status() {
                                page_response.status_code = status_code;
                            } else {
                                page_response.status_code =
                                    crate::page::get_error_http_status_code(&err);
                            }

                            page_response.error_for_status = Some(Err(err));
                            page_response
                        }
                    }
                }
            }
        }
    }
}

#[cfg(not(feature = "openai"))]
/// Perform a request to OpenAI Chat. This does nothing without the 'openai' flag enabled.
pub async fn openai_request(
    _gpt_configs: &crate::configuration::GPTConfigs,
    _resource: String,
    _url: &str,
    _prompt: &str,
) -> crate::features::openai_common::OpenAIReturn {
    Default::default()
}

#[cfg(feature = "openai")]
lazy_static! {
    static ref CORE_BPE_TOKEN_COUNT: tiktoken_rs::CoreBPE = tiktoken_rs::cl100k_base().unwrap();
    static ref SEM: tokio::sync::Semaphore = {
        let logical = num_cpus::get();
        let physical = num_cpus::get_physical();

        let sem_limit = if logical > physical {
            (logical) / (physical)
        } else {
            logical
        };

        let (sem_limit, sem_max) = if logical == physical {
            (sem_limit * physical, 20)
        } else {
            (sem_limit * 4, 10)
        };
        let sem_limit = sem_limit / 3;
        tokio::sync::Semaphore::const_new(sem_limit.max(sem_max))
    };
    static ref CLIENT: async_openai::Client<async_openai::config::OpenAIConfig> =
        async_openai::Client::new();
}

#[cfg(feature = "openai")]
/// Perform a request to OpenAI Chat. This does nothing without the 'openai' flag enabled.
pub async fn openai_request_base(
    gpt_configs: &crate::configuration::GPTConfigs,
    resource: String,
    url: &str,
    prompt: &str,
) -> crate::features::openai_common::OpenAIReturn {
    match SEM.acquire().await {
        Ok(permit) => {
            let mut chat_completion_defaults =
                async_openai::types::CreateChatCompletionRequestArgs::default();
            let gpt_base = chat_completion_defaults
                .max_tokens(gpt_configs.max_tokens)
                .model(&gpt_configs.model);
            let gpt_base = match gpt_configs.user {
                Some(ref user) => gpt_base.user(user),
                _ => gpt_base,
            };
            let gpt_base = match gpt_configs.temperature {
                Some(temp) => gpt_base.temperature(temp),
                _ => gpt_base,
            };
            let gpt_base = match gpt_configs.top_p {
                Some(tp) => gpt_base.top_p(tp),
                _ => gpt_base,
            };

            let core_bpe = match tiktoken_rs::get_bpe_from_model(&gpt_configs.model) {
                Ok(bpe) => Some(bpe),
                _ => None,
            };

            let (tokens, prompt_tokens) = match core_bpe {
                Some(ref core_bpe) => (
                    core_bpe.encode_with_special_tokens(&resource),
                    core_bpe.encode_with_special_tokens(&prompt),
                ),
                _ => (
                    CORE_BPE_TOKEN_COUNT.encode_with_special_tokens(&resource),
                    CORE_BPE_TOKEN_COUNT.encode_with_special_tokens(&prompt),
                ),
            };

            // // we can use the output count later to perform concurrent actions.
            let output_tokens_count = tokens.len() + prompt_tokens.len();

            let mut max_tokens = crate::features::openai::calculate_max_tokens(
                &gpt_configs.model,
                gpt_configs.max_tokens,
                &&crate::features::openai::BROWSER_ACTIONS_SYSTEM_PROMPT_COMPLETION.clone(),
                &resource,
                &prompt,
            );

            // we need to slim down the content to fit the window.
            let resource = if output_tokens_count > max_tokens {
                let r = clean_html(&resource);

                max_tokens = crate::features::openai::calculate_max_tokens(
                    &gpt_configs.model,
                    gpt_configs.max_tokens,
                    &&crate::features::openai::BROWSER_ACTIONS_SYSTEM_PROMPT_COMPLETION.clone(),
                    &r,
                    &prompt,
                );

                let (tokens, prompt_tokens) = match core_bpe {
                    Some(ref core_bpe) => (
                        core_bpe.encode_with_special_tokens(&r),
                        core_bpe.encode_with_special_tokens(&prompt),
                    ),
                    _ => (
                        CORE_BPE_TOKEN_COUNT.encode_with_special_tokens(&r),
                        CORE_BPE_TOKEN_COUNT.encode_with_special_tokens(&prompt),
                    ),
                };

                let output_tokens_count = tokens.len() + prompt_tokens.len();

                if output_tokens_count > max_tokens {
                    let r = clean_html_slim(&r);

                    max_tokens = crate::features::openai::calculate_max_tokens(
                        &gpt_configs.model,
                        gpt_configs.max_tokens,
                        &&crate::features::openai::BROWSER_ACTIONS_SYSTEM_PROMPT_COMPLETION.clone(),
                        &r,
                        &prompt,
                    );

                    let (tokens, prompt_tokens) = match core_bpe {
                        Some(ref core_bpe) => (
                            core_bpe.encode_with_special_tokens(&r),
                            core_bpe.encode_with_special_tokens(&prompt),
                        ),
                        _ => (
                            CORE_BPE_TOKEN_COUNT.encode_with_special_tokens(&r),
                            CORE_BPE_TOKEN_COUNT.encode_with_special_tokens(&prompt),
                        ),
                    };

                    let output_tokens_count = tokens.len() + prompt_tokens.len();

                    if output_tokens_count > max_tokens {
                        clean_html_full(&r)
                    } else {
                        r
                    }
                } else {
                    r
                }
            } else {
                clean_html(&resource)
            };

            let mut tokens_used = crate::features::openai_common::OpenAIUsage::default();
            let json_mode = gpt_configs.extra_ai_data;

            let response_format = {
                let mut mode = if json_mode {
                    async_openai::types::ResponseFormat::JsonObject
                } else {
                    async_openai::types::ResponseFormat::Text
                };

                if let Some(ref structure) = gpt_configs.json_schema {
                    if let Some(ref schema) = structure.schema {
                        if let Ok(mut schema) =
                            crate::features::serde_json::from_str::<serde_json::Value>(&schema)
                        {
                            if json_mode {
                                // Insert the "js" property into the schema's properties. Todo: capture if the js property exist and re-word prompt to match new js property with after removal.
                                if let Some(properties) = schema.get_mut("properties") {
                                    if let Some(properties_map) = properties.as_object_mut() {
                                        properties_map.insert(
                                            "js".to_string(),
                                            serde_json::json!({
                                                "type": "string"
                                            }),
                                        );
                                    }
                                }
                            }

                            mode = async_openai::types::ResponseFormat::JsonSchema {
                                json_schema: async_openai::types::ResponseFormatJsonSchema {
                                    description: structure.description.clone(),
                                    name: structure.name.clone(),
                                    schema: if schema.is_null() { None } else { Some(schema) },
                                    strict: structure.strict,
                                },
                            }
                        }
                    }
                }

                mode
            };

            match async_openai::types::ChatCompletionRequestAssistantMessageArgs::default()
                .content(string_concat!("URL: ", url, "\n", "HTML: ", resource))
                .build()
            {
                Ok(resource_completion) => {
                    let mut messages: Vec<async_openai::types::ChatCompletionRequestMessage> =
                        vec![crate::features::openai::BROWSER_ACTIONS_SYSTEM_PROMPT.clone()];

                    if json_mode {
                        messages.push(
                            crate::features::openai::BROWSER_ACTIONS_SYSTEM_EXTRA_PROMPT.clone(),
                        );
                    }

                    messages.push(resource_completion.into());

                    if !prompt.is_empty() {
                        messages.push(
                            match async_openai::types::ChatCompletionRequestUserMessageArgs::default()
                            .content(prompt)
                            .build()
                        {
                            Ok(o) => o,
                            _ => Default::default(),
                        }
                        .into()
                        )
                    }

                    let v = match gpt_base
                        .max_tokens(max_tokens as u32)
                        .messages(messages)
                        .response_format(response_format)
                        .build()
                    {
                        Ok(request) => {
                            let res = match gpt_configs.api_key {
                                Some(ref key) => {
                                    if !key.is_empty() {
                                        let conf = CLIENT.config().to_owned();
                                        async_openai::Client::with_config(conf.with_api_key(key))
                                            .chat()
                                            .create(request)
                                            .await
                                    } else {
                                        CLIENT.chat().create(request).await
                                    }
                                }
                                _ => CLIENT.chat().create(request).await,
                            };

                            match res {
                                Ok(mut response) => {
                                    let mut choice = response.choices.first_mut();

                                    if let Some(usage) = response.usage.take() {
                                        tokens_used.prompt_tokens = usage.prompt_tokens;
                                        tokens_used.completion_tokens = usage.completion_tokens;
                                        tokens_used.total_tokens = usage.total_tokens;
                                    }

                                    match choice.as_mut() {
                                        Some(c) => match c.message.content.take() {
                                            Some(content) => content,
                                            _ => Default::default(),
                                        },
                                        _ => Default::default(),
                                    }
                                }
                                Err(err) => {
                                    log::error!("{:?}", err);
                                    Default::default()
                                }
                            }
                        }
                        _ => Default::default(),
                    };

                    drop(permit);

                    crate::features::openai_common::OpenAIReturn {
                        response: v,
                        usage: tokens_used,
                        error: None,
                    }
                }
                Err(e) => {
                    let mut d = crate::features::openai_common::OpenAIReturn::default();

                    d.error = Some(e.to_string());

                    d
                }
            }
        }
        Err(e) => {
            let mut d = crate::features::openai_common::OpenAIReturn::default();

            d.error = Some(e.to_string());

            d
        }
    }
}

#[cfg(all(feature = "openai", not(feature = "cache_openai")))]
/// Perform a request to OpenAI Chat. This does nothing without the 'openai' flag enabled.
pub async fn openai_request(
    gpt_configs: &crate::configuration::GPTConfigs,
    resource: String,
    url: &str,
    prompt: &str,
) -> crate::features::openai_common::OpenAIReturn {
    openai_request_base(gpt_configs, resource, url, prompt).await
}

#[cfg(all(feature = "openai", feature = "cache_openai"))]
/// Perform a request to OpenAI Chat. This does nothing without the 'openai' flag enabled.
pub async fn openai_request(
    gpt_configs: &crate::configuration::GPTConfigs,
    resource: String,
    url: &str,
    prompt: &str,
) -> crate::features::openai_common::OpenAIReturn {
    match &gpt_configs.cache {
        Some(cache) => {
            use std::hash::{Hash, Hasher};
            let mut s = ahash::AHasher::default();

            url.hash(&mut s);
            prompt.hash(&mut s);
            gpt_configs.model.hash(&mut s);
            gpt_configs.max_tokens.hash(&mut s);
            gpt_configs.extra_ai_data.hash(&mut s);
            // non-determinstic
            resource.hash(&mut s);

            let key = s.finish();

            match cache.get(&key).await {
                Some(cache) => {
                    let mut c = cache;
                    c.usage.cached = true;
                    c
                }
                _ => {
                    let r = openai_request_base(gpt_configs, resource, url, prompt).await;
                    let _ = cache.insert(key, r.clone()).await;
                    r
                }
            }
        }
        _ => openai_request_base(gpt_configs, resource, url, prompt).await,
    }
}

/// Clean the html removing css and js default using the scraper crate.
pub fn clean_html_raw(html: &str) -> String {
    html.to_string()
}

/// Clean the html removing css and js
#[cfg(feature = "openai")]
pub fn clean_html_base(html: &str) -> String {
    use lol_html::{doc_comments, element, rewrite_str, RewriteStrSettings};

    match rewrite_str(
        html,
        RewriteStrSettings {
            element_content_handlers: vec![
                element!("script", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("style", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("link", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("iframe", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("[style*='display:none']", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("[id*='ad']", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("[class*='ad']", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("[id*='tracking']", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("[class*='tracking']", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("meta", |el| {
                    if let Some(attribute) = el.get_attribute("name") {
                        if attribute != "title" && attribute != "description" {
                            el.remove();
                        }
                    } else {
                        el.remove();
                    }
                    Ok(())
                }),
            ],
            document_content_handlers: vec![doc_comments!(|c| {
                c.remove();
                Ok(())
            })],
            ..RewriteStrSettings::default()
        },
    ) {
        Ok(r) => r,
        _ => html.into(),
    }
}

/// Make sure the base tag exist on the page.
#[cfg(feature = "chrome")]
pub async fn rewrite_base_tag(html: &str, base_url: &Option<&str>) -> String {
    use lol_html::{element, html_content::ContentType};
    use std::sync::OnceLock;

    if html.is_empty() {
        return Default::default();
    }

    let base_tag_inserted = OnceLock::new();
    let already_present = OnceLock::new();

    let base_url_len = base_url.map(|s| s.len());

    let rewriter_settings: lol_html::Settings<'_, '_, lol_html::send::SendHandlerTypes> =
        lol_html::send::Settings {
            element_content_handlers: vec![
                // Handler for <base> to mark if it is present with href
                element!("base", {
                    |el| {
                        // check base tags that do not exist yet.
                        if base_tag_inserted.get().is_none() {
                            // Check if a <base> with href already exists
                            if let Some(attr) = el.get_attribute("href") {
                                let valid_http =
                                    attr.starts_with("http://") || attr.starts_with("https://");

                                // we can validate if the domain is the same if not to remove it.
                                if valid_http {
                                    let _ = base_tag_inserted.set(true);
                                    let _ = already_present.set(true);
                                } else {
                                    el.remove();
                                }
                            } else {
                                el.remove();
                            }
                        }

                        Ok(())
                    }
                }),
                // Handler for <head> to insert <base> tag if not present
                element!("head", {
                    |el: &mut lol_html::send::Element| {
                        if let Some(handlers) = el.end_tag_handlers() {
                            let base_tag_inserted = base_tag_inserted.clone();
                            let base_url =
                                format!(r#"<base href="{}">"#, base_url.unwrap_or_default());

                            handlers.push(Box::new(move |end| {
                                if base_tag_inserted.get().is_none() {
                                    let _ = base_tag_inserted.set(true);
                                    end.before(&base_url, ContentType::Html);
                                }
                                Ok(())
                            }))
                        }
                        Ok(())
                    }
                }),
                // Handler for html if <head> not present to insert <head><base></head> tag if not present
                element!("html", {
                    |el: &mut lol_html::send::Element| {
                        if let Some(handlers) = el.end_tag_handlers() {
                            let base_tag_inserted = base_tag_inserted.clone();
                            let base_url = format!(
                                r#"<head><base href="{}"></head>"#,
                                base_url.unwrap_or_default()
                            );

                            handlers.push(Box::new(move |end| {
                                if base_tag_inserted.get().is_none() {
                                    let _ = base_tag_inserted.set(true);
                                    end.before(&base_url, ContentType::Html);
                                }
                                Ok(())
                            }))
                        }
                        Ok(())
                    }
                }),
            ],
            ..lol_html::send::Settings::new_for_handler_types()
        };

    let mut buffer = Vec::with_capacity(
        html.len()
            + match base_url_len {
                Some(l) => l + 29,
                _ => 0,
            },
    );

    let mut rewriter = lol_html::send::HtmlRewriter::new(rewriter_settings, |c: &[u8]| {
        buffer.extend_from_slice(c);
    });

    let mut stream = tokio_stream::iter(html.as_bytes().chunks(*STREAMING_CHUNK_SIZE));

    let mut wrote_error = false;

    while let Some(chunk) = stream.next().await {
        // early exist
        if already_present.get().is_some() {
            break;
        }
        if rewriter.write(chunk).is_err() {
            wrote_error = true;
            break;
        }
    }

    if !wrote_error {
        let _ = rewriter.end();
    }

    if already_present.get().is_some() {
        html.to_string()
    } else {
        auto_encoder::auto_encode_bytes(&buffer)
    }
}

/// Clean the HTML to slim fit GPT models. This removes base64 images from the prompt.
#[cfg(feature = "openai")]
pub fn clean_html_slim(html: &str) -> String {
    use lol_html::{doc_comments, element, rewrite_str, RewriteStrSettings};
    match rewrite_str(
        html,
        RewriteStrSettings {
            element_content_handlers: vec![
                element!("script", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("style", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("svg", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("noscript", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("link", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("iframe", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("canvas", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("video", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("img", |el| {
                    if let Some(src) = el.get_attribute("src") {
                        if src.starts_with("data:image") {
                            el.remove();
                        }
                    }
                    Ok(())
                }),
                element!("picture", |el| {
                    if let Some(src) = el.get_attribute("src") {
                        if src.starts_with("data:image") {
                            el.remove();
                        }
                    }
                    Ok(())
                }),
                element!("[style*='display:none']", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("[id*='ad']", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("[class*='ad']", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("[id*='tracking']", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("[class*='tracking']", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("meta", |el| {
                    if let Some(attribute) = el.get_attribute("name") {
                        if attribute != "title" && attribute != "description" {
                            el.remove();
                        }
                    } else {
                        el.remove();
                    }
                    Ok(())
                }),
            ],
            document_content_handlers: vec![doc_comments!(|c| {
                c.remove();
                Ok(())
            })],
            ..RewriteStrSettings::default()
        },
    ) {
        Ok(r) => r,
        _ => html.into(),
    }
}

/// Clean the most of the extra properties in the html to fit the context.
#[cfg(feature = "openai")]
pub fn clean_html_full(html: &str) -> String {
    use lol_html::{doc_comments, element, rewrite_str, RewriteStrSettings};

    match rewrite_str(
        html,
        RewriteStrSettings {
            element_content_handlers: vec![
                element!("nav, footer", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("meta", |el| {
                    let name = el.get_attribute("name").map(|n| n.to_lowercase());

                    if !matches!(name.as_deref(), Some("viewport") | Some("charset")) {
                        el.remove();
                    }

                    Ok(())
                }),
                element!("*", |el| {
                    let attrs_to_keep = ["id", "data-", "class"];
                    let attributes_list = el.attributes().iter();
                    let mut remove_list = Vec::new();

                    for attr in attributes_list {
                        if !attrs_to_keep.contains(&attr.name().as_str()) {
                            remove_list.push(attr.name());
                        }
                    }

                    for attr in remove_list {
                        el.remove_attribute(&attr);
                    }

                    Ok(())
                }),
            ],
            document_content_handlers: vec![doc_comments!(|c| {
                c.remove();
                Ok(())
            })],
            ..RewriteStrSettings::default()
        },
    ) {
        Ok(r) => r,
        _ => html.into(),
    }
}

/// Clean the html removing css and js
#[cfg(not(feature = "openai"))]
pub fn clean_html(html: &str) -> String {
    clean_html_raw(html)
}

/// Clean the html removing css and js
#[cfg(all(feature = "openai", not(feature = "openai_slim_fit")))]
pub fn clean_html(html: &str) -> String {
    clean_html_base(html)
}

/// Clean the html removing css and js
#[cfg(all(feature = "openai", feature = "openai_slim_fit"))]
pub fn clean_html(html: &str) -> String {
    clean_html_slim(html)
}

#[cfg(not(feature = "openai"))]
/// Clean and remove all base64 images from the prompt.
pub fn clean_html_slim(html: &str) -> String {
    html.into()
}

/// Log to console if configuration verbose.
pub fn log(message: &'static str, data: impl AsRef<str>) {
    if log_enabled!(Level::Info) {
        info!("{message} - {}", data.as_ref());
    }
}

#[cfg(feature = "control")]
/// determine action
#[derive(PartialEq, Debug)]
pub enum Handler {
    /// Crawl start state
    Start,
    /// Crawl pause state
    Pause,
    /// Crawl resume
    Resume,
    /// Crawl shutdown
    Shutdown,
}

#[cfg(feature = "control")]
lazy_static! {
    /// control handle for crawls
    pub static ref CONTROLLER: std::sync::Arc<tokio::sync::RwLock<(tokio::sync::watch::Sender<(String, Handler)>,
        tokio::sync::watch::Receiver<(String, Handler)>)>> =
            std::sync::Arc::new(tokio::sync::RwLock::new(tokio::sync::watch::channel(("handles".to_string(), Handler::Start))));
}

#[cfg(feature = "control")]
/// Pause a target website running crawl. The crawl_id is prepended directly to the domain and required if set. ex: d22323edsd-https://mydomain.com
pub async fn pause(target: &str) {
    match CONTROLLER
        .write()
        .await
        .0
        .send((target.into(), Handler::Pause))
    {
        _ => (),
    };
}

#[cfg(feature = "control")]
/// Resume a target website crawl. The crawl_id is prepended directly to the domain and required if set. ex: d22323edsd-https://mydomain.com
pub async fn resume(target: &str) {
    match CONTROLLER
        .write()
        .await
        .0
        .send((target.into(), Handler::Resume))
    {
        _ => (),
    };
}

#[cfg(feature = "control")]
/// Shutdown a target website crawl. The crawl_id is prepended directly to the domain and required if set. ex: d22323edsd-https://mydomain.com
pub async fn shutdown(target: &str) {
    match CONTROLLER
        .write()
        .await
        .0
        .send((target.into(), Handler::Shutdown))
    {
        _ => (),
    };
}

#[cfg(feature = "control")]
/// Reset a target website crawl. The crawl_id is prepended directly to the domain and required if set. ex: d22323edsd-https://mydomain.com
pub async fn reset(target: &str) {
    match CONTROLLER
        .write()
        .await
        .0
        .send((target.into(), Handler::Start))
    {
        _ => (),
    };
}

/// Setup selectors for handling link targets.
pub(crate) fn setup_website_selectors(url: &str, allowed: AllowedDomainTypes) -> RelativeSelectors {
    let subdomains = allowed.subdomains;
    let tld = allowed.tld;

    crate::page::get_page_selectors_base(url, subdomains, tld)
}

/// Allow subdomains or tlds.
#[derive(Debug, Default, Clone, Copy)]
pub struct AllowedDomainTypes {
    /// Subdomains
    pub subdomains: bool,
    /// Tlds
    pub tld: bool,
}

impl AllowedDomainTypes {
    /// A new domain type.
    pub fn new(subdomains: bool, tld: bool) -> Self {
        Self { subdomains, tld }
    }
}

/// Modify the selectors for targetting a website.
pub(crate) fn modify_selectors(
    prior_domain: &Option<Box<Url>>,
    domain: &str,
    domain_parsed: &mut Option<Box<Url>>,
    url: &mut Box<CaseInsensitiveString>,
    base: &mut RelativeSelectors,
    allowed: AllowedDomainTypes,
) {
    *domain_parsed = parse_absolute_url(domain);
    *url = Box::new(domain.into());
    let s = setup_website_selectors(url.inner(), allowed);
    base.0 = s.0;
    base.1 = s.1;
    if let Some(prior_domain) = prior_domain {
        if let Some(dname) = prior_domain.host_str() {
            base.2 = dname.into();
        }
    }
}

/// Get the last segment path.
pub fn get_last_segment(path: &str) -> &str {
    if let Some(pos) = path.rfind('/') {
        let next_position = pos + 1;
        if next_position < path.len() {
            &path[next_position..]
        } else {
            ""
        }
    } else {
        path
    }
}

/// Get the path from a url
pub(crate) fn get_path_from_url(url: &str) -> &str {
    if let Some(start_pos) = url.find("//") {
        let mut pos = start_pos + 2;

        if let Some(third_slash_pos) = url[pos..].find('/') {
            pos += third_slash_pos;
            &url[pos..]
        } else {
            "/"
        }
    } else {
        "/"
    }
}

/// Get the domain from a url.
pub(crate) fn get_domain_from_url(url: &str) -> &str {
    if let Some(start_pos) = url.find("//") {
        let pos = start_pos + 2;

        if let Some(first_slash_pos) = url[pos..].find('/') {
            &url[pos..pos + first_slash_pos]
        } else {
            &url[pos..]
        }
    } else {
        if let Some(first_slash_pos) = url.find('/') {
            &url[..first_slash_pos]
        } else {
            &url
        }
    }
}

/// Determine if networking is capable for a URL.
pub fn networking_capable(url: &str) -> bool {
    url.starts_with("https://")
        || url.starts_with("http://")
        || url.starts_with("file://")
        || url.starts_with("ftp://")
}

/// Prepare the url for parsing if it fails. Use this method if the url does not start with http or https.
pub fn prepare_url(u: &str) -> String {
    if let Some(index) = u.find("://") {
        let split_index = u
            .char_indices()
            .nth(index + 3)
            .map(|(i, _)| i)
            .unwrap_or(u.len());

        format!("https://{}", &u[split_index..])
    } else {
        format!("https://{}", u)
    }
}

/// normalize the html markup to prevent Maliciousness.
pub(crate) async fn normalize_html(html: &[u8]) -> Vec<u8> {
    use lol_html::{element, send::Settings};

    let mut output = Vec::new();

    let mut rewriter = HtmlRewriter::new(
        Settings {
            element_content_handlers: vec![
                element!("a[href]", |el| {
                    el.remove_attribute("href");
                    Ok(())
                }),
                element!("script, style, iframe, base, noscript", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("*", |el| {
                    let mut remove_attr = vec![];

                    for attr in el.attributes() {
                        let name = attr.name();
                        let remove =
                            !(name.starts_with("data-") || name == "id" || name == "class");
                        if remove {
                            remove_attr.push(name);
                        }
                    }

                    for name in remove_attr {
                        el.remove_attribute(&name);
                    }

                    Ok(())
                }),
            ],
            ..Settings::new_send()
        },
        |c: &[u8]| output.extend_from_slice(c),
    );

    let chunks = html.chunks(*STREAMING_CHUNK_SIZE);
    let mut stream = tokio_stream::iter(chunks);
    let mut wrote_error = false;

    while let Some(chunk) = stream.next().await {
        if rewriter.write(chunk).is_err() {
            wrote_error = true;
            break;
        }
    }

    if !wrote_error {
        let _ = rewriter.end();
    }

    output
}

/// Hash html markup.
pub(crate) async fn hash_html(html: &[u8]) -> u64 {
    let normalized_html = normalize_html(html).await;

    if !normalized_html.is_empty() {
        use std::hash::{Hash, Hasher};
        let mut s = ahash::AHasher::default();
        normalized_html.hash(&mut s);
        let key = s.finish();
        key
    } else {
        Default::default()
    }
}

#[cfg(feature = "tracing")]
/// Spawns a new asynchronous task.
pub(crate) fn spawn_task<F>(task_name: &str, future: F) -> tokio::task::JoinHandle<F::Output>
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    tokio::task::Builder::new()
        .name(task_name)
        .spawn(future)
        .expect("failed to spawn task")
}

#[cfg(not(feature = "tracing"))]
#[allow(unused)]
/// Spawns a new asynchronous task.
pub(crate) fn spawn_task<F>(_task_name: &str, future: F) -> tokio::task::JoinHandle<F::Output>
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    tokio::task::spawn(future)
}

#[cfg(feature = "tracing")]
/// Spawn a joinset.
pub(crate) fn spawn_set<F, T>(
    task_name: &str,
    set: &mut tokio::task::JoinSet<T>,
    future: F,
) -> tokio::task::AbortHandle
where
    F: Future<Output = T>,
    F: Send + 'static,
    T: Send + 'static,
{
    set.build_task()
        .name(task_name)
        .spawn(future)
        .expect("set should spawn")
}

#[cfg(not(feature = "tracing"))]
/// Spawn a joinset.
pub(crate) fn spawn_set<F, T>(
    _task_name: &str,
    set: &mut tokio::task::JoinSet<T>,
    future: F,
) -> tokio::task::AbortHandle
where
    F: Future<Output = T>,
    F: Send + 'static,
    T: Send + 'static,
{
    set.spawn(future)
}

#[cfg(feature = "balance")]
/// Period to wait to rebalance cpu in means of IO being main impact.
const REBALANCE_TIME: std::time::Duration = std::time::Duration::from_millis(100);

/// Return the semaphore that should be used.
#[cfg(feature = "balance")]
pub async fn get_semaphore(semaphore: &Arc<Semaphore>, detect: bool) -> &Arc<Semaphore> {
    let cpu_load = if detect {
        crate::utils::detect_system::get_global_cpu_state().await
    } else {
        0
    };

    if cpu_load == 2 {
        tokio::time::sleep(REBALANCE_TIME).await;
    }

    if cpu_load >= 1 {
        &*crate::website::SEM_SHARED
    } else {
        semaphore
    }
}

/// Check if the crawl duration is expired.
pub fn crawl_duration_expired(crawl_timeout: &Option<Duration>, start: &Option<Instant>) -> bool {
    crawl_timeout
        .and_then(|duration| start.map(|start| start.elapsed() >= duration))
        .unwrap_or(false)
}

/// is the content html and safe for formatting.
static HTML_TAGS: phf::Set<&'static [u8]> = phf_set! {
    b"<!doctype html",
    b"<html",
    b"<document",
};

/// Check if the content is HTML.
pub fn is_html_content_check(bytes: &[u8]) -> bool {
    let check_bytes = if bytes.len() > 1024 {
        &bytes[..1024]
    } else {
        bytes
    };

    for tag in HTML_TAGS.iter() {
        if check_bytes
            .windows(tag.len())
            .any(|window| window.eq_ignore_ascii_case(tag))
        {
            return true;
        }
    }

    false
}

/// Return the semaphore that should be used.
#[cfg(not(feature = "balance"))]
pub async fn get_semaphore(semaphore: &Arc<Semaphore>, _detect: bool) -> &Arc<Semaphore> {
    semaphore
}

#[derive(Debug)]
/// Html output sink for the rewriter.
#[cfg(feature = "smart")]
pub(crate) struct HtmlOutputSink {
    /// The bytes collected.
    pub(crate) data: Vec<u8>,
    /// The sender to send once finished.
    pub(crate) sender: Option<tokio::sync::oneshot::Sender<Vec<u8>>>,
}

#[cfg(feature = "smart")]
impl HtmlOutputSink {
    /// A new output sink.
    pub(crate) fn new(sender: tokio::sync::oneshot::Sender<Vec<u8>>) -> Self {
        HtmlOutputSink {
            data: Vec::new(),
            sender: Some(sender),
        }
    }
}

#[cfg(feature = "smart")]
impl OutputSink for HtmlOutputSink {
    fn handle_chunk(&mut self, chunk: &[u8]) {
        self.data.extend_from_slice(chunk);
        if chunk.len() == 0 {
            if let Some(sender) = self.sender.take() {
                let data_to_send = std::mem::take(&mut self.data);
                let _ = sender.send(data_to_send);
            }
        }
    }
}

/// Emit a log info event.
#[cfg(feature = "tracing")]
pub fn emit_log(link: &str) {
    tracing::info!("fetch {}", &link);
}
/// Emit a log info event.
#[cfg(not(feature = "tracing"))]
pub fn emit_log(link: &str) {
    log::info!("fetch {}", &link);
}

/// Emit a log info event.
#[cfg(feature = "tracing")]
pub fn emit_log_shutdown(link: &str) {
    tracing::info!("shutdown {}", &link);
}
/// Emit a log info event.
#[cfg(not(feature = "tracing"))]
pub fn emit_log_shutdown(link: &str) {
    log::info!("shutdown {}", &link);
}
