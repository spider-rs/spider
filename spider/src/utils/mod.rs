/// Absolute path domain handling.
pub mod abs;
/// Utils to modify the HTTP header.
pub mod header_utils;
/// String interner.
pub mod interner;
/// A trie struct.
pub mod trie;

#[cfg(feature = "balance")]
/// CPU detection to balance limitations.
pub mod detect_cpu;

use crate::RelativeSelectors;
use abs::parse_absolute_url;
use auto_encoder::is_binary_file;
use bytes::{BufMut, BytesMut};
use case_insensitive_string::CaseInsensitiveString;
use lol_html::send::HtmlRewriter;
use lol_html::OutputSink;
use phf::phf_set;
use std::future::Future;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::Semaphore;
use url::Url;

#[cfg(feature = "chrome")]
use crate::features::chrome_common::{AutomationScripts, ExecutionScripts};
use crate::tokio_stream::StreamExt;
use crate::Client;
#[cfg(feature = "cache_chrome_hybrid")]
use http_cache_semantics::{RequestLike, ResponseLike};

use log::{info, log_enabled, Level};

use reqwest::{
    header::{HeaderName, HeaderValue},
    Error, Response, StatusCode,
};

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

lazy_static! {
    /// Prevent fetching resources beyond the bytes limit.
    static ref MAX_SIZE_BYTES: usize = {
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

/// Handle protected pages via chrome. This does nothing without the real_browser feature enabled.
#[cfg(all(feature = "chrome", feature = "real_browser"))]
async fn cf_handle(
    b: &mut bytes::Bytes,
    page: &chromiumoxide::Page,
) -> Result<(), chromiumoxide::error::CdpError> {
    use crate::configuration::{WaitFor, WaitForDelay, WaitForIdleNetwork};
    lazy_static! {
        static ref CF_END: &'static [u8; 62] =
            b"target=\"_blank\">Cloudflare</a></div></div></div></body></html>";
        static ref CF_END2: &'static [u8; 72] =
            b"Performance &amp; security by Cloudflare</div></div></div></body></html>";
        static ref CF_HEAD: &'static [u8; 34] = b"<html><head>\n    <style global=\"\">";
        static ref CF_MOCK_FRAME: &'static [u8; 137] = b"<iframe height=\"1\" width=\"1\" style=\"position: absolute; top: 0px; left: 0px; border: none; visibility: hidden;\"></iframe>\n\n</body></html>";
    };

    let cf = CF_END.as_ref();
    let cf2 = CF_END2.as_ref();
    let cn = CF_HEAD.as_ref();
    let cnf = CF_MOCK_FRAME.as_ref();

    if b.ends_with(cf) || b.ends_with(cf2) || b.starts_with(cn) && b.ends_with(cnf) {
        let page_result = tokio::time::timeout(tokio::time::Duration::from_secs(30), async {
            let mut wait_for = WaitFor::default();
            wait_for.delay = WaitForDelay::new(Some(core::time::Duration::from_secs(1))).into();
            wait_for.idle_network =
                WaitForIdleNetwork::new(core::time::Duration::from_secs(8).into()).into();
            page_wait(&page, &Some(wait_for.clone())).await;

            let _ = page
                .evaluate(r###"document.querySelectorAll("iframe").forEach(el=>el.click());"###)
                .await;

            wait_for.page_navigations = true;
            page_wait(&page, &Some(wait_for.clone())).await;

            if let Ok(next_content) = page.content_bytes().await {
                let next_content = if next_content.ends_with(cf)
                    || next_content.ends_with(cf2)
                    || next_content.starts_with(cn) && next_content.ends_with(cnf)
                {
                    wait_for.delay =
                        WaitForDelay::new(Some(core::time::Duration::from_secs(4))).into();
                    page_wait(&page, &Some(wait_for)).await;
                    match page.content_bytes().await {
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
            Ok(_) => Ok(()),
            _ => Err(chromiumoxide::error::CdpError::Timeout),
        }
    } else {
        Ok(())
    }
}

/// Handle cloudflare protected pages via chrome. This does nothing without the real_browser feature enabled.
#[cfg(all(feature = "chrome", not(feature = "real_browser")))]
async fn cf_handle(
    _b: &mut bytes::Bytes,
    _page: &chromiumoxide::Page,
) -> Result<(), chromiumoxide::error::CdpError> {
    Ok(())
}

/// The response of a web page.
#[derive(Debug, Default)]
pub struct PageResponse {
    /// The page response resource.
    pub content: Option<Box<bytes::Bytes>>,
    #[cfg(feature = "headers")]
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
    pub error_for_status: Option<Result<Response, Error>>,
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
}

/// wait for event with timeout
#[cfg(feature = "chrome")]
pub async fn wait_for_event<T>(page: &chromiumoxide::Page, timeout: Option<core::time::Duration>)
where
    T: chromiumoxide::cdp::IntoEventKind + Unpin + std::fmt::Debug,
{
    match page.event_listener::<T>().await {
        Ok(mut events) => {
            let wait_until = async {
                loop {
                    let sleep = tokio::time::sleep(tokio::time::Duration::from_millis(500));
                    tokio::pin!(sleep);
                    tokio::select! {
                        _ = &mut sleep => break,
                        v = events.next() => {
                            if !v.is_none () {
                                break;
                            }
                        }
                    }
                }
            };
            match timeout {
                Some(timeout) => if let Err(_) = tokio::time::timeout(timeout, wait_until).await {},
                _ => wait_until.await,
            }
        }
        _ => (),
    }
}

/// wait for a selector
#[cfg(feature = "chrome")]
pub async fn wait_for_selector(
    page: &chromiumoxide::Page,
    timeout: Option<core::time::Duration>,
    selector: &str,
) {
    let wait_until = async {
        loop {
            let sleep = tokio::time::sleep(tokio::time::Duration::from_millis(50));
            tokio::pin!(sleep);
            tokio::select! {
                _ = &mut sleep => (),
                v = page.find_element(selector) => {
                    if v.is_ok() {
                        break
                    }
                }
            }
        }
    };
    match timeout {
        Some(timeout) => if let Err(_) = tokio::time::timeout(timeout, wait_until).await {},
        _ => wait_until.await,
    }
}

/// wait for a selector
#[cfg(feature = "chrome")]
pub async fn wait_for_dom(
    page: &chromiumoxide::Page,
    timeout: Option<core::time::Duration>,
    selector: &str,
) {
    let script = crate::features::chrome_common::generate_wait_for_dom_js_code_with_selector_base(
        if let Some(dur) = timeout {
            dur.as_millis() as u32
        } else {
            500
        },
        selector,
    );

    let wait_until = async {
        loop {
            let sleep = tokio::time::sleep(tokio::time::Duration::from_millis(50));
            tokio::pin!(sleep);

            tokio::select! {
                _ = &mut sleep => (),
                v = page
                .evaluate(
                    script.clone(),
                ) => {
                    if v.is_ok() {
                        break
                    }
                }
            }
        }
    };

    match timeout {
        Some(timeout) => if let Err(_) = tokio::time::timeout(timeout, wait_until).await {},
        _ => wait_until.await,
    }
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

    match b.parent() {
        Some(p) => {
            let _ = tokio::fs::create_dir_all(&p).await;
        }
        _ => (),
    }

    b.display().to_string()
}

#[cfg(feature = "chrome")]
/// Wait for page events.
pub async fn page_wait(
    page: &chromiumoxide::Page,
    wait_for: &Option<crate::configuration::WaitFor>,
) {
    if let Some(wait_for) = wait_for {
        let wait_for_idle_network = async {
            if let Some(ref wait) = wait_for.idle_network {
                wait_for_event::<
                chromiumoxide::cdp::browser_protocol::network::EventLoadingFinished,
            >(page, wait.timeout)
            .await;
            }
        };

        let wait_for_selector = async {
            if let Some(ref wait) = wait_for.selector {
                wait_for_selector(page, wait.timeout, &wait.selector).await;
            }
        };

        let wait_for_dom = async {
            if let Some(ref wait) = wait_for.dom {
                wait_for_dom(page, wait.timeout, &wait.selector).await;
            }
        };

        let wait_for_delay = async {
            if let Some(ref wait) = wait_for.delay {
                if let Some(timeout) = wait.timeout {
                    tokio::time::sleep(timeout).await
                }
            }
        };

        tokio::join!(
            wait_for_idle_network,
            wait_for_selector,
            wait_for_dom,
            wait_for_delay
        );
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
}

#[cfg(feature = "chrome")]
/// Perform a http future with chrome.
pub async fn perform_chrome_http_request(
    page: &chromiumoxide::Page,
    source: &str,
) -> Result<ChromeHTTPReqRes, chromiumoxide::error::CdpError> {
    let mut waf_check = false;
    let mut status_code = StatusCode::OK;
    let mut method = String::from("GET");
    let mut response_headers = std::collections::HashMap::default();
    let mut request_headers = std::collections::HashMap::default();
    let mut protocol = String::from("http/1.1");

    let page_base =
        page.http_future(chromiumoxide::cdp::browser_protocol::page::NavigateParams {
            url: source.to_string(),
            transition_type: Some(
                chromiumoxide::cdp::browser_protocol::page::TransitionType::Other,
            ),
            frame_id: None,
            referrer: None,
            referrer_policy: None,
        })?;

    match page_base.await {
        Ok(page_base) => {
            match page_base {
                Some(http_request) => {
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

                        if !response.url.starts_with(source) {
                            waf_check = match response.security_details {
                                Some(ref security_details) => {
                                    if security_details.subject_name == "challenges.cloudflare.com"
                                    {
                                        true
                                    } else {
                                        false
                                    }
                                }
                                _ => response.url.contains("/cdn-cgi/challenge-platform"),
                            };
                            if !waf_check {
                                waf_check = match response.protocol {
                                    Some(ref protocol) => protocol == "blob",
                                    _ => false,
                                }
                            }
                        }

                        status_code = StatusCode::from_u16(response.status as u16)
                            .unwrap_or_else(|_| StatusCode::EXPECTATION_FAILED);
                    }
                }
                _ => {
                    if let Ok(scode) = StatusCode::from_u16(599) {
                        status_code = scode;
                    }
                }
            };
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
    })
}

/// Use OpenAI to extend the crawl. This does nothing without 'openai' feature flag.
#[cfg(all(feature = "chrome", not(feature = "openai")))]
pub async fn run_openai_request(
    _source: &str,
    _page: &chromiumoxide::Page,
    _wait_for: &Option<crate::configuration::WaitFor>,
    _openai_config: &Option<crate::configuration::GPTConfigs>,
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
    openai_config: &Option<crate::configuration::GPTConfigs>,
    mut page_response: &mut PageResponse,
    ok: bool,
) {
    match &openai_config {
        Some(gpt_configs) => {
            let gpt_configs = match gpt_configs.prompt_url_map {
                Some(ref h) => {
                    let c = h.get::<case_insensitive_string::CaseInsensitiveString>(&source.into());

                    if !c.is_some() && gpt_configs.paths_map {
                        match url::Url::parse(source) {
                            Ok(u) => h.get::<case_insensitive_string::CaseInsensitiveString>(
                                &u.path().into(),
                            ),
                            _ => None,
                        }
                    } else {
                        c
                    }
                }
                _ => Some(gpt_configs),
            };

            match gpt_configs {
                Some(gpt_configs) => {
                    let mut prompts = gpt_configs.prompt.clone();

                    while let Some(prompt) = prompts.next() {
                        let gpt_results = if !gpt_configs.model.is_empty() && ok {
                            openai_request(
                                gpt_configs,
                                match page_response.content.as_ref() {
                                    Some(html) => String::from_utf8_lossy(html).to_string(),
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
                            let html: Option<Box<bytes::Bytes>> = match page
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
                                if json_res.js.len() <= 400
                                    && json_res.js.contains("window.location")
                                {
                                    match page.content_bytes().await {
                                        Ok(b) => {
                                            page_response.content = Some(b.into());
                                        }
                                        _ => (),
                                    }
                                } else {
                                    page_response.content = html;
                                }
                            }
                        }

                        // attach the data to the page
                        if gpt_configs.extra_ai_data {
                            let screenshot_bytes = if gpt_configs.screenshot
                                && !json_res.js.is_empty()
                            {
                                let format = chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat::Png;

                                let screenshot_configs =
                                    chromiumoxide::page::ScreenshotParams::builder()
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
                                        log::error!(
                                            "failed to take screenshot: {:?} - {:?}",
                                            e,
                                            source
                                        );
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
                _ => (),
            }
        }
        _ => (),
    };
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

/// Get the initial page headers of the page with navigation.
#[cfg(feature = "chrome")]
async fn navigate(
    page: &chromiumoxide::Page,
    url: &str,
    chrome_http_req_res: &mut ChromeHTTPReqRes,
) -> Result<(), chromiumoxide::error::CdpError> {
    *chrome_http_req_res = perform_chrome_http_request(page, url).await?;
    Ok(())
}

#[cfg(all(feature = "real_browser", feature = "chrome"))]
/// generate random mouse movement.
async fn perform_smart_mouse_movement(
    page: &chromiumoxide::Page,
    viewport: &Option<crate::configuration::Viewport>,
) {
    use crate::features::chrome_mouse_movements::GaussianMouse;
    use chromiumoxide::layout::Point;
    let (viewport_width, viewport_height) = match viewport {
        Some(vp) => (vp.width as f64, vp.height as f64),
        _ => (1280.0, 720.0),
    };
    for (x, y) in GaussianMouse::generate_random_coordinates(viewport_width, viewport_height) {
        let _ = page.move_mouse(Point::new(x, y)).await;
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
                Some(b) => b.clone().to_vec(),
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

/// Max page timeout for events.
#[cfg(feature = "chrome")]
const MAX_PAGE_TIMEOUT: tokio::time::Duration = tokio::time::Duration::from_secs(60);

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
    openai_config: &Option<crate::configuration::GPTConfigs>,
    url_target: Option<&str>,
    execution_scripts: &Option<ExecutionScripts>,
    automation_scripts: &Option<AutomationScripts>,
    viewport: &Option<crate::configuration::Viewport>,
    request_timeout: &Option<Box<std::time::Duration>>,
) -> Result<PageResponse, chromiumoxide::error::CdpError> {
    use std::ops::Div;
    let mut chrome_http_req_res = ChromeHTTPReqRes::default();

    let listener = page
        .event_listener::<chromiumoxide::cdp::browser_protocol::network::EventLoadingFinished>()
        .await;

    // Listen for network events. todo: capture the last values endtime to track period.
    let bytes_collected_handle = tokio::spawn(async move {
        let mut total = 0.0;

        if let Ok(mut listener) = listener {
            while let Some(event) = listener.next().await {
                total += event.encoded_data_length;
            }
        }

        total
    });

    let page_navigation = async {
        if !page_set {
            // used for smart mode re-rendering direct assigning html
            if content {
                if let Ok(frame) = page.mainframe().await {
                    let _ = page
                        .execute(
                            chromiumoxide::cdp::browser_protocol::page::SetDocumentContentParams {
                                frame_id: frame.unwrap_or_default(),
                                html: source.to_string(),
                            },
                        )
                        .await;

                    // perform extra navigate to trigger page actions.
                    if let Some(u) = url_target {
                        if u.starts_with("http") {
                            let _ = page
                                .evaluate(format!(r#"window.location = "{}";"#, u))
                                .await;
                        }
                    }
                }
            } else {
                if let Err(e) = navigate(page, source, &mut chrome_http_req_res).await {
                    return Err(e);
                };
            }
        }

        Ok(())
    };

    let request_timeout = tokio::time::timeout(
        match request_timeout {
            Some(timeout) => **timeout.min(&Box::new(MAX_PAGE_TIMEOUT)),
            _ => MAX_PAGE_TIMEOUT,
        },
        page_navigation,
    )
    .await;

    let timeout_error = if let Err(elasped) = request_timeout {
        Some(elasped)
    } else {
        None
    };

    let mut page_response = if timeout_error.is_none()
        && chrome_http_req_res.status_code.is_success()
    {
        // we do not need to wait for navigation if content is assigned. The method set_content already handles this.
        let final_url = if wait_for_navigation {
            let last_redirect = tokio::time::timeout(tokio::time::Duration::from_secs(15), async {
                match page.wait_for_navigation_response().await {
                    Ok(u) => get_last_redirect(&source, &u, &page).await,
                    _ => None,
                }
            })
            .await;

            match last_redirect {
                Ok(last) => last,
                _ => None,
            }
        } else {
            None
        };

        if chrome_http_req_res.waf_check {
            if let Err(elasped) = tokio::time::timeout(
                tokio::time::Duration::from_secs(4),
                perform_smart_mouse_movement(&page, &viewport),
            )
            .await
            {
                log::warn!("mouse movement timeout exceeded {elasped}");
            }
        }

        if let Err(elasped) =
            tokio::time::timeout(MAX_PAGE_TIMEOUT, page_wait(&page, &wait_for)).await
        {
            log::warn!("max wait for timeout {elasped}");
        }

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

            if let Err(elasped) = tokio::time::timeout(MAX_PAGE_TIMEOUT, async {
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
                log::warn!("mouse movement timeout exceeded {elasped}");
            }
        }

        let mut res: Box<bytes::Bytes> = match page.content_bytes().await {
            Ok(b) => b.into(),
            _ => Default::default(),
        };

        if cfg!(feature = "real_browser") {
            let _ = tokio::time::timeout(
                tokio::time::Duration::from_secs(10),
                cf_handle(&mut res, &page),
            )
            .await;
        };

        let ok = res.len() > 0;

        if chrome_http_req_res.waf_check && res.starts_with(b"<html><head>\n    <style global=") && res.ends_with(b";</script><iframe height=\"1\" width=\"1\" style=\"position: absolute; top: 0px; left: 0px; border: none; visibility: hidden;\"></iframe>\n\n</body></html>"){
            chrome_http_req_res.status_code = StatusCode::FORBIDDEN;
        }

        let mut page_response = set_page_response(ok, res, &mut chrome_http_req_res, final_url);

        set_page_response_headers(&mut chrome_http_req_res, &mut page_response);
        set_page_response_cookies(&mut page_response, &page).await;

        if openai_config.is_some() {
            run_openai_request(
                match url_target {
                    Some(ref ut) => ut,
                    _ => source,
                },
                page,
                wait_for,
                openai_config,
                &mut page_response,
                ok,
            )
            .await;
        }

        if cfg!(feature = "chrome_screenshot") || screenshot.is_some() {
            let _ = tokio::time::timeout(
                tokio::time::Duration::from_secs(30),
                perform_screenshot(source, page, screenshot, &mut page_response),
            )
            .await;
        }
        page_response.status_code = chrome_http_req_res.status_code;
        page_response.waf_check = chrome_http_req_res.waf_check;
        if !page_set {
            let _ = tokio::time::timeout(
                tokio::time::Duration::from_secs(10),
                cache_chrome_response(&source, &page_response, chrome_http_req_res),
            )
            .await;
        }

        page_response
    } else {
        let mut page_response = PageResponse::default();
        set_page_response_headers(&mut chrome_http_req_res, &mut page_response);
        page_response.status_code = chrome_http_req_res.status_code;
        page_response.waf_check = chrome_http_req_res.waf_check;

        if let Some(_elasped) = timeout_error {
            page_response.status_code = StatusCode::REQUEST_TIMEOUT;
        }

        page_response
    };

    // run initial handling hidden anchors
    // if let Ok(new_links) = page.evaluate(crate::features::chrome::ANCHOR_EVENTS).await {
    //     if let Ok(results) = new_links.into_value::<hashbrown::HashSet<CaseInsensitiveString>>() {
    //         links.extend(page.extract_links_raw(&base, &results).await);
    //     }
    // }

    if cfg!(not(feature = "chrome_store_page")) {
        let _ = tokio::time::timeout(
            MAX_PAGE_TIMEOUT.div(2),
            page.execute(chromiumoxide::cdp::browser_protocol::page::CloseParams::default()),
        )
        .await;
        // we want to use a sync impl to get bytes when storing the page.
        if let Ok(transferred) = bytes_collected_handle.await {
            page_response.bytes_transferred = Some(transferred);
        }
    }

    Ok(page_response)
}

/// Set the page response.
#[cfg(feature = "chrome")]
fn set_page_response(
    ok: bool,
    res: Box<bytes::Bytes>,
    chrome_http_req_res: &mut ChromeHTTPReqRes,
    final_url: Option<String>,
) -> PageResponse {
    PageResponse {
        content: if ok { Some(res.into()) } else { None },
        status_code: chrome_http_req_res.status_code,
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
pub fn get_cookies(res: &Response) -> Option<reqwest::header::HeaderMap> {
    let mut headers = reqwest::header::HeaderMap::new();

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
pub fn get_cookies(res: &Response) -> Option<reqwest::header::HeaderMap> {
    None
}

/// Block streaming
fn block_streaming(res: &Response, only_html: bool) -> bool {
    let mut block_streaming = false;

    if only_html {
        if let Some(content_type) = res.headers().get(reqwest::header::CONTENT_TYPE) {
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
    #[cfg(feature = "headers")]
    let headers = res.headers().clone();
    #[cfg(feature = "remote_addr")]
    let remote_addr = res.remote_addr();
    let cookies = get_cookies(&res);

    let mut content: Option<Box<bytes::Bytes>> = None;

    if !block_streaming(&res, only_html) {
        let mut stream = res.bytes_stream();
        let mut data: BytesMut = BytesMut::new();
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
        ..Default::default()
    }
}

/// Handle the response bytes writing links while crawling
pub async fn handle_response_bytes_writer<'h, O>(
    res: Response,
    target_url: &str,
    only_html: bool,
    rewriter: &mut HtmlRewriter<'h, O>,
    collected_bytes: &mut BytesMut,
) -> (PageResponse, bool)
where
    O: OutputSink + Send + 'static,
{
    let u = res.url().as_str();

    let final_url = if target_url != u {
        Some(u.into())
    } else {
        None
    };

    let status_code: StatusCode = res.status();
    #[cfg(feature = "headers")]
    let headers = res.headers().clone();
    #[cfg(feature = "remote_addr")]
    let remote_addr = res.remote_addr();
    let cookies = get_cookies(&res);

    // let mut content: Option<Box<bytes::Bytes>> = None;
    let mut rewrite_error = false;

    if !block_streaming(&res, only_html) {
        let mut stream = res.bytes_stream();
        // let mut data: BytesMut = BytesMut::new();
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
    }

    (
        PageResponse {
            #[cfg(feature = "headers")]
            headers: Some(headers),
            #[cfg(feature = "remote_addr")]
            remote_addr,
            #[cfg(feature = "cookies")]
            cookies,
            // content,
            final_url,
            status_code,
            ..Default::default()
        },
        rewrite_error,
    )
}

/// Setup default response
pub(crate) fn setup_default_response(target_url: &str, res: &Response) -> PageResponse {
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
        cookies: get_cookies(res),
        status_code: res.status(),
        final_url: rd,
        ..Default::default()
    }
}

/// Perform a network request to a resource extracting all content streaming.
async fn fetch_page_html_raw_base(
    target_url: &str,
    client: &Client,
    only_html: bool,
) -> PageResponse {
    match client.get(target_url).send().await {
        Ok(res) if res.status().is_success() => {
            handle_response_bytes(res, target_url, only_html).await
        }
        Ok(res) => setup_default_response(target_url, &res),
        Err(_) => {
            log::info!("error fetching {}", target_url);
            let mut page_response = PageResponse::default();
            if let Ok(status_code) = StatusCode::from_u16(599) {
                page_response.status_code = status_code;
            }
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
pub async fn fetch_page(target_url: &str, client: &Client) -> Option<bytes::Bytes> {
    match client.get(target_url).send().await {
        Ok(res) if res.status().is_success() => match res.bytes().await {
            Ok(text) => Some(text),
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
    Success(reqwest::header::HeaderMap, Option<bytes::Bytes>),
    /// No success extracting content
    NoSuccess(reqwest::header::HeaderMap),
    /// A network error occured.
    FetchError,
}

#[cfg(all(feature = "decentralized", feature = "headers"))]
/// Perform a network request to a resource with the response headers..
pub async fn fetch_page_and_headers(target_url: &str, client: &Client) -> FetchPageResult {
    match client.get(target_url).send().await {
        Ok(res) if res.status().is_success() => {
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
    use crate::tokio::io::AsyncReadExt;
    use crate::tokio::io::AsyncWriteExt;
    use bytes::BytesMut;
    use percent_encoding::utf8_percent_encode;
    use percent_encoding::NON_ALPHANUMERIC;
    use tendril::fmt::Slice;

    match client.get(target_url).send().await {
        Ok(res) if res.status().is_success() => {
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
            let mut data: BytesMut = BytesMut::new();
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

                                        match file.write_all(data.as_bytes()).await {
                                            Ok(_) => {
                                                data.clear();
                                            }
                                            _ => (),
                                        };
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

                    match tokio::fs::File::open(&file_path).await {
                        Ok(mut b) => match b.read_to_end(&mut buffer).await {
                            _ => (),
                        },
                        _ => (),
                    };

                    match tokio::fs::remove_file(file_path).await {
                        _ => (),
                    };

                    buffer.into()
                } else {
                    data.into()
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
        Err(_) => {
            log::info!("error fetching {}", target_url);
            let mut page_response = PageResponse::default();
            if let Ok(status_code) = StatusCode::from_u16(599) {
                page_response.status_code = status_code;
            }
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
    openai_config: &Option<crate::configuration::GPTConfigs>,
    execution_scripts: &Option<ExecutionScripts>,
    automation_scripts: &Option<AutomationScripts>,
    viewport: &Option<crate::configuration::Viewport>,
    request_timeout: &Option<tokio::time::Duration>,
) -> PageResponse {
    use crate::tokio::io::{AsyncReadExt, AsyncWriteExt};
    use percent_encoding::utf8_percent_encode;
    use percent_encoding::NON_ALPHANUMERIC;
    use tendril::fmt::Slice;

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
                request_timeout,
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
                    use bytes::BytesMut;

                    match client.get(target_url).send().await {
                        Ok(res) if res.status().is_success() => {
                            #[cfg(feature = "headers")]
                            let headers = res.headers().clone();
                            let cookies = get_cookies(&res);
                            let status_code = res.status();
                            let mut stream = res.bytes_stream();
                            let mut data: BytesMut = BytesMut::new();

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

                                                        match file.write_all(data.as_bytes()).await
                                                        {
                                                            Ok(_) => {
                                                                data.clear();
                                                            }
                                                            _ => (),
                                                        };
                                                    }
                                                    _ => data.put(text),
                                                };
                                            } else {
                                                match &file.as_mut().unwrap().write_all(&text).await
                                                {
                                                    Ok(_) => (),
                                                    _ => data.put(text),
                                                };
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

                                    match tokio::fs::File::open(&file_path).await {
                                        Ok(mut b) => match b.read_to_end(&mut buffer).await {
                                            _ => (),
                                        },
                                        _ => (),
                                    };

                                    match tokio::fs::remove_file(file_path).await {
                                        _ => (),
                                    };

                                    buffer.into()
                                } else {
                                    data.into()
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
                        Err(_) => {
                            log::info!("error fetching {}", target_url);
                            let mut page_response = PageResponse::default();
                            if let Ok(status_code) = StatusCode::from_u16(599) {
                                page_response.status_code = status_code;
                            }
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
    openai_config: &Option<crate::configuration::GPTConfigs>,
    execution_scripts: &Option<ExecutionScripts>,
    automation_scripts: &Option<AutomationScripts>,
    viewport: &Option<crate::configuration::Viewport>,
    request_timeout: &Option<Box<std::time::Duration>>,
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
    openai_config: &Option<crate::configuration::GPTConfigs>,
    execution_scripts: &Option<ExecutionScripts>,
    automation_scripts: &Option<AutomationScripts>,
    viewport: &Option<crate::configuration::Viewport>,
    request_timeout: &Option<Box<tokio::time::Duration>>,
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
                    use bytes::BytesMut;

                    match client.get(target_url).send().await {
                        Ok(res) if res.status().is_success() => {
                            #[cfg(feature = "headers")]
                            let headers = res.headers().clone();
                            #[cfg(feature = "remote_addr")]
                            let remote_addr = res.remote_addr();
                            let cookies = get_cookies(&res);
                            let status_code = res.status();
                            let mut stream = res.bytes_stream();
                            let mut data: BytesMut = BytesMut::new();

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
                        Err(_) => {
                            log::info!("error fetching {}", target_url);
                            let mut page_response = PageResponse::default();
                            if let Ok(status_code) = StatusCode::from_u16(599) {
                                page_response.status_code = status_code;
                            }
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
/// Perform a request to OpenAI Chat. This does nothing without the 'openai' flag enabled.
pub async fn openai_request_base(
    gpt_configs: &crate::configuration::GPTConfigs,
    resource: String,
    url: &str,
    prompt: &str,
) -> crate::features::openai_common::OpenAIReturn {
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
    };

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
                        if let Ok(mut schema) = serde_json::from_str::<serde_json::Value>(&schema) {
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

                                    match response.usage.take() {
                                        Some(usage) => {
                                            tokens_used.prompt_tokens = usage.prompt_tokens;
                                            tokens_used.completion_tokens = usage.completion_tokens;
                                            tokens_used.total_tokens = usage.total_tokens;
                                        }
                                        _ => (),
                                    };

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
            use std::hash::{DefaultHasher, Hash, Hasher};
            let mut s = DefaultHasher::new();

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
pub(crate) fn setup_website_selectors(
    domain_parsed: &Option<Box<Url>>,
    url: &str,
    allowed: AllowedDomainTypes,
) -> Option<RelativeSelectors> {
    use crate::page::{get_page_selectors, get_page_selectors_base};
    let subdomains = allowed.subdomains;
    let tld = allowed.tld;

    match domain_parsed {
        Some(u) => get_page_selectors_base(u, subdomains, tld),
        _ => get_page_selectors(url, subdomains, tld),
    }
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
    if let Some(s) = setup_website_selectors(domain_parsed, url.inner(), allowed) {
        base.0 = s.0;
        base.1 = s.1;
        if let Some(prior_domain) = prior_domain {
            if let Some(dname) = prior_domain.host_str() {
                base.2 = dname.into();
            }
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

/// Return the semaphore that should be used.
#[cfg(feature = "balance")]
pub async fn get_semaphore(semaphore: &Arc<Semaphore>, detect: bool) -> &Arc<Semaphore> {
    let cpu_load = if detect {
        crate::utils::detect_cpu::get_global_cpu_usage().await
    } else {
        0
    };

    if cpu_load >= 70 {
        &*crate::website::SEM_SHARED
    } else {
        semaphore
    }
}

/// Return the semaphore that should be used.
#[cfg(not(feature = "balance"))]
pub async fn get_semaphore(semaphore: &Arc<Semaphore>, _detect: bool) -> &Arc<Semaphore> {
    semaphore
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
