/// Absolute path domain handling.
pub mod abs;
/// Connect layer for reqwest.
pub mod connect;
/// Generic CSS selectors.
pub mod css_selectors;
/// Fragment templates.
pub mod templates;

#[cfg(feature = "adaptive_concurrency")]
/// AIMD-based adaptive concurrency controller.
pub mod adaptive_concurrency;
#[cfg(feature = "auto_throttle")]
/// Latency-based auto-throttle for adaptive crawl delay per domain.
pub mod auto_throttle;
/// Exponential backoff with jitter for retry logic.
pub mod backoff;
#[cfg(feature = "bloom")]
/// mmap-backed bloom filter with hugepage support for URL deduplication.
pub mod bloom;
#[cfg(feature = "request_coalesce")]
/// Request coalescing to dedup concurrent in-flight requests.
pub mod coalesce;
#[cfg(feature = "chrome")]
pub(crate) mod detect_chrome;
#[cfg(any(feature = "balance", feature = "disk"))]
/// CPU and Memory detection to balance limitations.
pub mod detect_system;
#[cfg(feature = "dns_cache")]
/// DNS pre-resolution cache with TTL.
pub mod dns_cache;
#[cfg(feature = "etag_cache")]
/// ETag / conditional-request cache for bandwidth-efficient re-crawls.
pub mod etag_cache;
#[cfg(feature = "priority_frontier")]
/// Prioritized URL frontier with dedup and optional domain round-robin.
pub mod frontier;
#[cfg(feature = "h2_multiplex")]
/// HTTP/2 multiplexing tracker for per-origin stream management.
pub mod h2_tracker;
/// Utils to modify the HTTP header.
pub mod header_utils;
#[cfg(feature = "hedge")]
/// Work-stealing (hedged requests) for slow crawl requests.
pub mod hedge;
#[cfg(feature = "balance")]
/// Disk-backed HTML spool for memory-balanced crawling.
pub mod html_spool;
/// String interner.
pub mod interner;
#[cfg(feature = "numa")]
/// NUMA-aware thread pinning for multi-socket servers.
pub mod numa;
#[cfg(feature = "parallel_backends")]
/// Parallel crawl backends — race alternative engines alongside the primary crawl.
pub mod parallel_backends;
#[cfg(feature = "rate_limit")]
/// Per-domain token bucket rate limiter.
pub mod rate_limiter;
#[cfg(feature = "robots_cache")]
/// Cross-crawl robots.txt cache with TTL-based expiry.
pub mod robots_cache;
#[cfg(feature = "chrome")]
/// Chrome tab pooling for reusing CDP tabs across page visits.
pub mod tab_pool;
/// A trie struct.
pub mod trie;
/// Async file I/O with optional io_uring acceleration.
pub mod uring_fs;
/// Validate html false positives.
pub mod validation;
#[cfg(feature = "balance")]
/// Lock-free crawl vitals for intelligent scaling (requests, bytes, pages, tabs).
pub mod vitals;
#[cfg(feature = "wait_guard")]
/// Adaptive wait-for timeout guard for chronic bad domains.
pub mod wait_guard;
#[cfg(feature = "warc")]
/// WARC 1.1 file writer for web archive output.
pub mod warc;
#[cfg(feature = "zero_copy")]
/// Zero-copy byte-level parsing for HTTP wire formats and protocol structures.
pub mod zero_copy;

#[cfg(feature = "chrome")]
use crate::features::automation::RemoteMultimodalConfigs;
use crate::{
    page::{
        AntiBotTech, Metadata, REWRITER_YIELD_INTERVAL, REWRITER_YIELD_THRESHOLD,
        STREAMING_CHUNK_SIZE,
    },
    RelativeSelectors,
};
use abs::parse_absolute_url;
use aho_corasick::AhoCorasick;
use auto_encoder::is_binary_file;
use case_insensitive_string::CaseInsensitiveString;

#[cfg(feature = "chrome")]
use hashbrown::HashMap;
use hashbrown::HashSet;

use lol_html::{send::HtmlRewriter, OutputSink};
use phf::phf_set;
use reqwest::header::CONTENT_LENGTH;
#[cfg(feature = "chrome")]
use reqwest::header::{HeaderMap, HeaderValue};
use std::{
    error::Error,
    future::Future,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Semaphore;
use url::Url;

#[cfg(feature = "chrome")]
use crate::features::chrome_common::{AutomationScripts, ExecutionScripts};
use crate::page::{MAX_CONTENT_LENGTH, MAX_PREALLOC, MAX_PRE_ALLOCATED_HTML_PAGE_SIZE_USIZE};
use crate::tokio_stream::StreamExt;
use crate::Client;

#[cfg(any(feature = "cache_chrome_hybrid", feature = "cache_chrome_hybrid_mem"))]
use http_cache_semantics::{RequestLike, ResponseLike};

use log::{info, log_enabled, Level};

#[cfg(any(not(feature = "wreq"), feature = "cache_request"))]
use reqwest::{Response, StatusCode};
#[cfg(all(feature = "wreq", not(feature = "cache_request")))]
use wreq::{Response, StatusCode};

#[cfg(all(not(feature = "cache_request"), not(feature = "wreq")))]
pub(crate) type RequestError = reqwest::Error;

/// The request error (for `wreq`).
#[cfg(all(not(feature = "cache_request"), feature = "wreq"))]
pub(crate) type RequestError = wreq::Error;

/// The request error (for `reqwest_middleware` with caching).
#[cfg(feature = "cache_request")]
pub(crate) type RequestError = reqwest_middleware::Error;

/// The request response.
#[cfg(not(feature = "decentralized"))]
pub(crate) type RequestResponse = Response;

/// The wait for duration timeouts.
#[cfg(feature = "chrome")]
const WAIT_TIMEOUTS: [u64; 6] = [0, 20, 50, 100, 100, 500];
// /// The wait for duration timeouts.
// #[cfg(feature = "chrome")]
// const DOM_WAIT_TIMEOUTS: [u64; 6] = [100, 200, 300, 300, 400, 500];

/// Hop-by-hop headers that must be stripped from synthetic CDP fulfill responses.
/// Includes both lowercase (reqwest-normalized) and Title-Case forms to be
/// safe against any header source.
#[cfg(feature = "chrome")]
pub(crate) static HOP_BY_HOP_HEADERS: phf::Set<&'static str> = phf_set! {
    "content-length",    "Content-Length",
    "transfer-encoding", "Transfer-Encoding",
    "connection",        "Connection",
    "keep-alive",        "Keep-Alive",
    "proxy-connection",  "Proxy-Connection",
    "te",                "Te",  "TE",
    "trailers",          "Trailers",
    "upgrade",           "Upgrade",
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

    /// Empty html.
    pub static ref EMPTY_HTML_BASIC: &'static [u8; 13] = b"<html></html>";

    /// Scan for error anti-bot pages (24 patterns).
    static ref AC_BODY_SCAN: AhoCorasick = AhoCorasick::builder()
        .match_kind(aho_corasick::MatchKind::LeftmostFirst)
        .build([
            "cf-error-code",                                     // 0  → Cloudflare
            "Access to this page has been denied",               // 1  → Cloudflare
            "data-translate=\"block_headline\"",                 // 2  → Cloudflare WAF hard block
            "DataDome",                                          // 3  → DataDome
            "perimeterx",                                        // 4  → PerimeterX
            "funcaptcha",                                        // 5  → ArkoseLabs
            "Request unsuccessful. Incapsula incident ID",       // 6  → Imperva
            "_____tmd_____",                                      // 7  → AlibabaTMD
            "x5secdata",                                         // 8  → AlibabaTMD
            "ak_bmsc",                                           // 9  → Akamai Bot Manager
            "challenge-platform",                                // 10 → Cloudflare
            "cf-challenge",                                      // 11 → Cloudflare
            "ddos-guard",                                        // 12 → DDoS-Guard (lowercase)
            "px-captcha",                                        // 13 → PerimeterX
            "verify you are human",                              // 14 → Generic anti-bot
            "prove you're not a robot",                          // 15 → Generic anti-bot
            "Sucuri Website Firewall",                           // 16 → Sucuri
            "kpsdk",                                             // 17 → Kasada SDK
            "_Incapsula_Resource",                               // 18 → Imperva
            "Vercel Security Checkpoint",                        // 19 → Vercel
            "Generated by Wordfence",                            // 20 → Wordfence
            "Attention Required! | Cloudflare",                  // 21 → Cloudflare block page
            "aws-waf-token",                                     // 22 → AWS WAF
            "DDoS-Guard",                                        // 23 → DDoS-Guard (capitalized)
        ])
        .unwrap();

    /// Scan URLs for anti-bot service endpoints (24 patterns).
    static ref AC_URL_SCAN: AhoCorasick = AhoCorasick::builder()
        .match_kind(aho_corasick::MatchKind::LeftmostFirst)
        .build([
            "/cdn-cgi/challenge-platform",       // 0  → Cloudflare
            "datadome.co",                       // 1  → DataDome
            "dd-api.io",                         // 2  → DataDome
            "perimeterx.net",                    // 3  → PerimeterX
            "px-captcha",                        // 4  → PerimeterX
            "arkoselabs.com",                    // 5  → ArkoseLabs
            "funcaptcha",                        // 6  → ArkoseLabs
            "kasada.io",                         // 7  → Kasada
            "fingerprint.com",                   // 8  → FingerprintJS
            "fpjs.io",                           // 9  → FingerprintJS
            "incapsula",                         // 10 → Imperva
            "imperva",                           // 11 → Imperva
            "radwarebotmanager",                 // 12 → RadwareBotManager
            "reblaze.com",                       // 13 → Reblaze
            "cheq.ai",                           // 14 → CHEQ
            "_____tmd_____/punish",              // 15 → AlibabaTMD
            "hcaptcha.com",                      // 16 → HCaptcha
            "api.geetest.com",                   // 17 → GeeTest
            "geevisit.com",                      // 18 → GeeTest
            "queue-it.net",                      // 19 → QueueIt
            "ddos-guard.net",                    // 20 → DDoSGuard
            "/_Incapsula_Resource",              // 21 → Imperva
            "/cdn-cgi/bm/cv/",                   // 22 → Cloudflare Bot Management
            "sucuri.net",                        // 23 → Sucuri
        ])
        .unwrap();

    /// Scan `server` response header values (case-insensitive).
    static ref AC_SERVER_SCAN: AhoCorasick = AhoCorasick::builder()
        .match_kind(aho_corasick::MatchKind::LeftmostFirst)
        .ascii_case_insensitive(true)
        .build([
            "cloudflare",                        // 0 → Cloudflare
            "akamai",                            // 1 → Akamai (covers AkamaiGHost, AkamaiNetStorage)
            "sucuri",                            // 2 → Sucuri (covers Sucuri/Cloudproxy)
            "ddos-guard",                        // 3 → DDoS-Guard
            "datadome",                          // 4 → DataDome
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

/// Detect if openresty hard 403 is forbidden and should not retry.
#[inline(always)]
pub fn detect_open_resty_forbidden(b: &[u8]) -> bool {
    b.starts_with(*OPEN_RESTY_FORBIDDEN)
}

/// Detect if a page is forbidden and should not retry.
#[inline(always)]
pub fn detect_hard_forbidden_content(b: &[u8]) -> bool {
    b == *APACHE_FORBIDDEN || detect_open_resty_forbidden(b)
}

/// Returns true if the body should NOT be cached (empty, near-empty, or known-bad HTML).
///
/// HTML-specific heuristics (empty `<body>`, skeleton pages) are only applied
/// when the content looks like HTML (starts with `<`).  Non-HTML assets such as
/// JSON, images, CSS, JS, fonts, etc. short-circuit after the basic
/// empty / whitespace check.
#[inline]
pub fn is_cacheable_body_empty(body: &[u8]) -> bool {
    if body.is_empty() {
        return true;
    }
    let trimmed = body.trim_ascii();
    if trimmed.is_empty() {
        return true;
    }
    // Non-HTML content: if it doesn't start with '<' it's not markup —
    // skip the HTML-specific heuristics entirely.
    if trimmed[0] != b'<' {
        return false;
    }
    // --- HTML-specific checks ---
    if trimmed == *crate::utils::templates::EMPTY_HTML || trimmed == *EMPTY_HTML_BASIC {
        return true;
    }
    // Detect pages with HTML structure but empty <body> (< 2KB only for speed).
    // Case-insensitive matching without allocating a lowercase copy.
    if trimmed.len() <= 2048 {
        // Use memchr SIMD scan for '<' then verify tag prefix,
        // instead of O(n*5) windows scan.
        let body_open = {
            let mut found = None;
            let mut off = 0;
            while let Some(p) = memchr::memchr(b'<', &trimmed[off..]) {
                let abs = off + p;
                if trimmed.len() - abs >= 5 && trimmed[abs..abs + 5].eq_ignore_ascii_case(b"<body")
                {
                    found = Some(abs);
                    break;
                }
                off = abs + 1;
            }
            found
        };
        if let Some(body_open) = body_open {
            if let Some(gt) = memchr::memchr(b'>', &trimmed[body_open..]) {
                let content_start = body_open + gt + 1;
                // Same memchr pattern for </body>.
                let close = {
                    let mut found = None;
                    let mut off = 0;
                    let region = &trimmed[content_start..];
                    while let Some(p) = memchr::memchr(b'<', &region[off..]) {
                        let abs = off + p;
                        if region.len() - abs >= 7
                            && region[abs..abs + 7].eq_ignore_ascii_case(b"</body>")
                        {
                            found = Some(abs);
                            break;
                        }
                        off = abs + 1;
                    }
                    found
                };
                if let Some(close) = close {
                    let content_end = content_start + close;
                    if trimmed[content_start..content_end]
                        .iter()
                        .all(|c| c.is_ascii_whitespace())
                    {
                        return true;
                    }
                }
            }
        }
    }
    false
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

/// Per-chunk idle timeout for body streaming. If no data arrives within
/// this duration, the stream is terminated and any partial data collected
/// so far is returned. Prevents tarpitting / slow-drip antibot attacks.
/// Set via SPIDER_CHUNK_IDLE_TIMEOUT_SECS (default: 30 seconds, 0 = disabled).
pub fn chunk_idle_timeout() -> Option<Duration> {
    let secs = std::env::var("SPIDER_CHUNK_IDLE_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(30);

    if secs == 0 {
        None
    } else {
        Some(Duration::from_secs(secs))
    }
}

/// The response of a web page.
#[derive(Debug, Default)]
pub struct PageResponse {
    /// The page response resource.
    pub content: Option<Vec<u8>>,
    /// Additional content keyed by return format (populated when multiple
    /// formats are requested via [`SpiderCloudConfig::with_return_formats`]).
    #[cfg(feature = "spider_cloud")]
    pub content_map: Option<hashbrown::HashMap<String, bytes::Bytes>>,
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
    #[cfg(feature = "gemini")]
    /// The credits used from Gemini in order.
    pub gemini_credits_used: Option<Vec<crate::features::gemini_common::GeminiUsage>>,
    #[cfg(feature = "gemini")]
    /// The extra data from the Gemini AI.
    pub extra_gemini_data: Option<Vec<crate::page::AIResults>>,
    /// The usage from remote multimodal automation (extraction, etc.).
    /// Works with both Chrome and HTTP-only crawls.
    pub remote_multimodal_usage: Option<Vec<crate::features::automation::AutomationUsage>>,
    /// The extra data from the remote multimodal automation (extraction results, etc.).
    /// Works with both Chrome and HTTP-only crawls.
    pub extra_remote_multimodal_data: Option<Vec<crate::page::AutomationResults>>,
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
    /// The duration of the request.
    #[cfg(feature = "time")]
    pub duration: Option<tokio::time::Instant>,
    /// URLs to spawn as new pages from OpenPage actions.
    /// These are URLs the automation agent requested to open in new tabs.
    /// The caller (website.rs) should create new pages for these using the browser.
    pub spawn_pages: Option<Vec<String>>,
    /// Whether the response content was truncated due to a stream error,
    /// chunk idle timeout, or Content-Length mismatch (fewer bytes received than expected).
    pub content_truncated: bool,
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
                        if v.is_some() {
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
    let max = timeout.unwrap_or_else(|| core::time::Duration::from_millis(1200));

    let script = crate::features::chrome_common::generate_wait_for_dom_js_v2(
        max.as_millis() as u32,
        selector,
        500,
        2,
        true,
        false,
    );

    let hard = max + core::time::Duration::from_millis(200);

    let _ = tokio::time::timeout(hard, async {
        if let Ok(v) = page.evaluate(script).await {
            if let Some(val) = v.value().and_then(|x| x.as_bool()) {
                let _ = val;
            }
        }
    })
    .await;
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
        let _ = uring_fs::create_dir_all(p.display().to_string()).await;
    }

    b.display().to_string()
}

#[cfg(feature = "chrome")]
/// Wait for page events in two phases:
/// 1. Network waits run concurrently (resources must arrive before DOM settles).
/// 2. Selector + DOM + delay run concurrently (check rendering results).
pub async fn page_wait(
    page: &chromiumoxide::Page,
    wait_for: &Option<crate::configuration::WaitFor>,
) {
    if let Some(wait_for) = wait_for {
        // Phase 1: wait for network to settle (all network conditions concurrent).
        tokio::join!(
            async {
                if let Some(wait) = &wait_for.idle_network {
                    wait_for_event::<
                        chromiumoxide::cdp::browser_protocol::network::EventLoadingFinished,
                    >(page, wait.timeout)
                    .await;
                }
            },
            async {
                if let Some(wait) = &wait_for.almost_idle_network0 {
                    let timeout = wait.timeout.unwrap_or(core::time::Duration::from_secs(30));
                    let _ = page
                        .wait_for_network_almost_idle_with_timeout(timeout)
                        .await;
                }
            },
            async {
                if let Some(wait) = &wait_for.idle_network0 {
                    let timeout = wait.timeout.unwrap_or(core::time::Duration::from_secs(30));
                    let _ = page.wait_for_network_idle_with_timeout(timeout).await;
                }
            },
        );

        // Phase 2: network is settled — check DOM/selector/delay concurrently.
        tokio::join!(
            async {
                if let Some(wait) = &wait_for.selector {
                    wait_for_selector(page, wait.timeout, &wait.selector).await;
                }
            },
            async {
                if let Some(wait) = &wait_for.dom {
                    wait_for_dom(page, wait.timeout, &wait.selector).await;
                }
            },
            async {
                if let Some(wait) = &wait_for.delay {
                    if let Some(timeout) = wait.timeout {
                        tokio::time::sleep(timeout).await;
                    }
                }
            },
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

#[cfg(feature = "gemini")]
/// Handle the Gemini credits used.
pub fn handle_gemini_credits(
    page_response: &mut PageResponse,
    tokens_used: crate::features::gemini_common::GeminiUsage,
) {
    match page_response.gemini_credits_used.as_mut() {
        Some(v) => v.push(tokens_used),
        None => page_response.gemini_credits_used = Some(vec![tokens_used]),
    };
}

#[cfg(not(feature = "gemini"))]
/// Handle the Gemini credits used. This does nothing without 'gemini' feature flag.
pub fn handle_gemini_credits(
    _page_response: &mut PageResponse,
    _tokens_used: crate::features::gemini_common::GeminiUsage,
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

/// Has the header value.
#[inline(always)]
fn header_value<'a>(headers: &'a HeaderSource, key: &str) -> Option<&'a str> {
    match headers {
        HeaderSource::HeaderMap(hm) => hm.get(key).and_then(|v| v.to_str().ok()),
        HeaderSource::Map(map) => map.get(key).map(|s| s.as_str()),
    }
}

#[inline(always)]
/// Has the header key.
fn has_key(headers: &HeaderSource, key: &str) -> bool {
    match headers {
        HeaderSource::HeaderMap(hm) => hm.contains_key(key),
        HeaderSource::Map(map) => map.contains_key(key),
    }
}

#[inline(always)]
/// Equal case.
fn eq_icase_trim(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b)
}

/// Detect from headers (optimized: minimal lookups, no allocations).
#[inline]
pub fn detect_anti_bot_from_headers(headers: &HeaderSource) -> Option<AntiBotTech> {
    // Cloudflare (most common — check first)
    if has_key(headers, "cf-chl-bypass")
        || has_key(headers, "cf-ray")
        || has_key(headers, "cf-mitigated")
    {
        return Some(AntiBotTech::Cloudflare);
    }

    // DataDome
    if has_key(headers, "x-captcha-endpoint") || has_key(headers, "x-datadome") {
        return Some(AntiBotTech::DataDome);
    }

    // PerimeterX
    if has_key(headers, "x-perimeterx") || has_key(headers, "pxhd") {
        return Some(AntiBotTech::PerimeterX);
    }

    // Akamai
    if has_key(headers, "x-akamaibot") {
        return Some(AntiBotTech::AkamaiBotManager);
    }

    // Imperva (strong signals first)
    if has_key(headers, "x-imperva-id") || has_key(headers, "x-iinfo") {
        return Some(AntiBotTech::Imperva);
    }

    // Reblaze
    if has_key(headers, "x-reblaze-uuid") {
        return Some(AntiBotTech::Reblaze);
    }

    // Sucuri
    if has_key(headers, "x-sucuri-id") {
        return Some(AntiBotTech::Sucuri);
    }

    // x-cdn value check (Imperva CDN)
    if header_value(headers, "x-cdn").is_some_and(|v| eq_icase_trim(v, "imperva")) {
        return Some(AntiBotTech::Imperva);
    }

    // Server header (last — requires value inspection via AC scan)
    if let Some(server) = header_value(headers, "server") {
        if let Some(mat) = AC_SERVER_SCAN.find(server.as_bytes()) {
            let tech = match mat.pattern().as_usize() {
                0 => AntiBotTech::Cloudflare,
                1 => AntiBotTech::AkamaiBotManager,
                2 => AntiBotTech::Sucuri,
                3 => AntiBotTech::DDoSGuard,
                4 => AntiBotTech::DataDome,
                _ => return None,
            };
            return Some(tech);
        }
    }

    None
}

/// Detect the anti-bot technology.
#[inline]
pub fn detect_anti_bot_from_body(body: &[u8]) -> Option<AntiBotTech> {
    // Scan the first 30 KB for anti-bot fingerprints. Challenge pages are
    // almost always small, and most WAF interstitials inject markers near the
    // top. Scanning only the head avoids a linear scan over multi-MB bodies.
    let scan = if body.len() > 30_000 {
        &body[..30_000]
    } else {
        body
    };
    {
        if let Some(mat) = AC_BODY_SCAN.find(scan) {
            let tech = match mat.pattern().as_usize() {
                0..=2 | 10 | 11 | 21 => AntiBotTech::Cloudflare,
                3 => AntiBotTech::DataDome,
                4 | 13 => AntiBotTech::PerimeterX,
                5 => AntiBotTech::ArkoseLabs,
                6 | 18 => AntiBotTech::Imperva,
                7 | 8 => AntiBotTech::AlibabaTMD,
                9 => AntiBotTech::AkamaiBotManager,
                12 | 23 => AntiBotTech::DDoSGuard,
                14 | 15 => AntiBotTech::None, // Generic anti-bot signals
                16 => AntiBotTech::Sucuri,
                17 => AntiBotTech::Kasada,
                19 => AntiBotTech::Vercel,
                20 => AntiBotTech::Wordfence,
                22 => AntiBotTech::AwsWaf,
                _ => return None,
            };
            return Some(tech);
        }
    }

    None
}

/// Detect antibot from url
#[inline]
pub fn detect_antibot_from_url(url: &str) -> Option<AntiBotTech> {
    if let Some(mat) = AC_URL_SCAN.find(url) {
        let tech = match mat.pattern().as_usize() {
            0 | 22 => AntiBotTech::Cloudflare,
            1 | 2 => AntiBotTech::DataDome,
            3 | 4 => AntiBotTech::PerimeterX,
            5 | 6 => AntiBotTech::ArkoseLabs,
            7 => AntiBotTech::Kasada,
            8 | 9 => AntiBotTech::FingerprintJS,
            10 | 11 | 21 => AntiBotTech::Imperva,
            12 => AntiBotTech::RadwareBotManager,
            13 => AntiBotTech::Reblaze,
            14 => AntiBotTech::CHEQ,
            15 => AntiBotTech::AlibabaTMD,
            16 => AntiBotTech::HCaptcha,
            17 | 18 => AntiBotTech::GeeTest,
            19 => AntiBotTech::QueueIt,
            20 => AntiBotTech::DDoSGuard,
            23 => AntiBotTech::Sucuri,
            _ => return None,
        };
        Some(tech)
    } else {
        None
    }
}

/// Flip http -> https protocols.
pub fn flip_http_https(url: &str) -> Option<String> {
    if let Some(rest) = url.strip_prefix("http://") {
        let mut s = String::with_capacity(8 + rest.len());
        s.push_str("https://");
        s.push_str(rest);
        Some(s)
    } else if let Some(rest) = url.strip_prefix("https://") {
        let mut s = String::with_capacity(7 + rest.len());
        s.push_str("http://");
        s.push_str(rest);
        Some(s)
    } else {
        None
    }
}

/// Compiled custom antibot patterns for runtime matching.
/// Built from [`CustomAntibotPatterns`](crate::configuration::CustomAntibotPatterns) at crawl start.
#[derive(Clone)]
pub struct CompiledCustomAntibot {
    body_ac: Option<AhoCorasick>,
    url_ac: Option<AhoCorasick>,
    header_keys: Vec<crate::compact_str::CompactString>,
}

impl CompiledCustomAntibot {
    /// Compile user-supplied patterns into Aho-Corasick automatons.
    /// Returns `None` if all pattern lists are empty.
    pub fn compile(
        config: &crate::configuration::CustomAntibotPatterns,
    ) -> Option<CompiledCustomAntibot> {
        if config.body.is_empty() && config.url.is_empty() && config.header_keys.is_empty() {
            return None;
        }
        let body_ac = if config.body.is_empty() {
            None
        } else {
            AhoCorasick::builder()
                .match_kind(aho_corasick::MatchKind::LeftmostFirst)
                .build(&config.body)
                .ok()
        };
        let url_ac = if config.url.is_empty() {
            None
        } else {
            AhoCorasick::builder()
                .match_kind(aho_corasick::MatchKind::LeftmostFirst)
                .build(&config.url)
                .ok()
        };
        Some(CompiledCustomAntibot {
            body_ac,
            url_ac,
            header_keys: config.header_keys.clone(),
        })
    }

    /// Check body content for custom patterns (respects 30KB limit).
    #[inline]
    pub fn detect_body(&self, body: &[u8]) -> bool {
        if body.len() < 30_000 {
            if let Some(ref ac) = self.body_ac {
                return ac.is_match(body);
            }
        }
        false
    }

    /// Check URL for custom patterns.
    #[inline]
    pub fn detect_url(&self, url: &str) -> bool {
        if let Some(ref ac) = self.url_ac {
            return ac.is_match(url);
        }
        false
    }

    /// Check headers for custom key presence.
    #[inline]
    pub fn detect_headers(&self, headers: &HeaderSource) -> bool {
        self.header_keys.iter().any(|k| has_key(headers, k))
    }

    /// Check all custom patterns (body, URL, headers). Returns true if any match.
    #[inline]
    pub fn detect_any(&self, url: &str, headers: &HeaderSource, body: &[u8]) -> bool {
        self.detect_headers(headers) || self.detect_url(url) || self.detect_body(body)
    }
}

impl std::fmt::Debug for CompiledCustomAntibot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompiledCustomAntibot")
            .field("body_ac", &self.body_ac.is_some())
            .field("url_ac", &self.url_ac.is_some())
            .field("header_keys", &self.header_keys)
            .finish()
    }
}

/// Detect the anti-bot used from the request.
pub fn detect_anti_bot_tech_response(
    url: &str,
    headers: &HeaderSource,
    body: &[u8],
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

/// Detect the anti-bot used from the request, including custom user-supplied patterns.
pub fn detect_anti_bot_tech_response_custom(
    url: &str,
    headers: &HeaderSource,
    body: &[u8],
    subject_name: Option<&str>,
    custom: Option<&CompiledCustomAntibot>,
) -> AntiBotTech {
    let tech = detect_anti_bot_tech_response(url, headers, body, subject_name);
    if tech != AntiBotTech::None {
        return tech;
    }

    if let Some(custom) = custom {
        if custom.detect_any(url, headers, body) {
            return AntiBotTech::Custom;
        }
    }

    AntiBotTech::None
}

/// Extract to JsonResponse struct. This does nothing without 'openai' feature flag.
#[cfg(feature = "openai")]
pub fn handle_ai_data(js: &str) -> Option<JsonResponse> {
    serde_json::from_str::<JsonResponse>(js).ok()
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
impl ChromeHTTPReqRes {
    /// Is this an empty default
    pub fn is_empty(&self) -> bool {
        self.method.is_empty()
            && self.protocol.is_empty()
            && self.anti_bot_tech == crate::page::AntiBotTech::None
            && self.request_headers.is_empty()
            && self.response_headers.is_empty()
    }
}

#[cfg(feature = "chrome")]
/// Is a cyper mismatch.
fn is_cipher_mismatch(err: &chromiumoxide::error::CdpError) -> bool {
    match err {
        chromiumoxide::error::CdpError::ChromeMessage(msg) => {
            msg.contains("net::ERR_SSL_VERSION_OR_CIPHER_MISMATCH")
        }
        other => other
            .to_string()
            .contains("net::ERR_SSL_VERSION_OR_CIPHER_MISMATCH"),
    }
}

#[cfg(feature = "chrome")]
/// Is an SSL protocol error (e.g. multi-subdomain www. cert issues).
fn is_ssl_protocol_error(err: &chromiumoxide::error::CdpError) -> bool {
    match err {
        chromiumoxide::error::CdpError::ChromeMessage(msg) => {
            msg.contains("net::ERR_SSL_PROTOCOL_ERROR")
                || msg.contains("net::ERR_CERT_COMMON_NAME_INVALID")
                || msg.contains("net::ERR_CERT_AUTHORITY_INVALID")
        }
        other => {
            let s = other.to_string();
            s.contains("net::ERR_SSL_PROTOCOL_ERROR")
                || s.contains("net::ERR_CERT_COMMON_NAME_INVALID")
                || s.contains("net::ERR_CERT_AUTHORITY_INVALID")
        }
    }
}

/// Strip the `www.` prefix from a URL's host, if present.
/// Returns `None` if the URL has no `www.` prefix.
/// Example: `https://www.docs.github.com/foo` → `https://docs.github.com/foo`
pub fn strip_www(url: &str) -> Option<String> {
    // Find the scheme separator
    let after_scheme = if let Some(pos) = url.find("://") {
        pos + 3
    } else {
        return None;
    };
    let rest = &url[after_scheme..];
    if let Some(stripped) = rest.strip_prefix("www.") {
        let mut s = String::with_capacity(url.len() - 4);
        s.push_str(&url[..after_scheme]);
        s.push_str(stripped);
        Some(s)
    } else {
        None
    }
}

#[cfg(feature = "chrome")]
/// Perform a chrome http request.
pub async fn perform_chrome_http_request(
    page: &chromiumoxide::Page,
    source: &str,
    referrer: Option<String>,
) -> Result<ChromeHTTPReqRes, chromiumoxide::error::CdpError> {
    async fn attempt_once(
        page: &chromiumoxide::Page,
        source: &str,
        referrer: Option<String>,
    ) -> Result<ChromeHTTPReqRes, chromiumoxide::error::CdpError> {
        let mut waf_check = false;
        // Default to 599 (unknown) — only set 200 when we actually get a
        // valid HTTP response from Chrome.  This ensures callers see a
        // non-success status when navigation produces no response.
        let mut status_code = *crate::page::UNKNOWN_STATUS_ERROR;
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

                    if let Some(response) = &http_request.response {
                        if let Some(p) = &response.protocol {
                            protocol.clone_from(p);
                        }

                        if let Some(res_headers) = response.headers.inner().as_object() {
                            for (k, v) in res_headers {
                                response_headers.insert(k.to_string(), v.to_string());
                            }
                        }

                        let mut firewall = false;

                        waf_check = detect_antibot_from_url(&response.url).is_some();

                        // IMPORTANT: compare against the attempted URL (source param),
                        // so retries behave correctly.
                        if !response.url.starts_with(source) {
                            match &response.security_details {
                                Some(security_details) => {
                                    anti_bot_tech = detect_anti_bot_tech_response(
                                        &response.url,
                                        &HeaderSource::Map(&response_headers),
                                        &[],
                                        Some(&security_details.subject_name),
                                    );
                                    firewall = true;
                                }
                                _ => {
                                    anti_bot_tech = detect_anti_bot_tech_response(
                                        &response.url,
                                        &HeaderSource::Map(&response_headers),
                                        &[],
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

                            waf_check = waf_check
                                || firewall && !matches!(anti_bot_tech, AntiBotTech::None);

                            if !waf_check {
                                waf_check = match &response.protocol {
                                    Some(protocol) => protocol == "blob",
                                    _ => false,
                                }
                            }
                        }

                        status_code = StatusCode::from_u16(response.status as u16)
                            .unwrap_or(StatusCode::EXPECTATION_FAILED);
                    } else if let Some(failure_text) = &http_request.failure_text {
                        if failure_text == "net::ERR_FAILED" {
                            waf_check = true;
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

    match attempt_once(page, source, referrer.clone()).await {
        Ok(ok) => Ok(ok),
        Err(e) => {
            if is_cipher_mismatch(&e) {
                if let Some(flipped) = flip_http_https(source) {
                    return attempt_once(page, &flipped, referrer).await;
                }
            }
            if is_ssl_protocol_error(&e) {
                if let Some(no_www) = strip_www(source) {
                    return attempt_once(page, &no_www, referrer).await;
                }
            }
            Err(e)
        }
    }
}

#[cfg(all(feature = "chrome", feature = "chrome_remote_cache"))]
/// Perform a http future with chrome cached.
pub async fn perform_chrome_http_request_cache(
    page: &chromiumoxide::Page,
    source: &str,
    referrer: Option<String>,
    cache_options: &Option<CacheOptions>,
    cache_policy: &Option<BasicCachePolicy>,
    cache_namespace: Option<&str>,
) -> Result<ChromeHTTPReqRes, chromiumoxide::error::CdpError> {
    async fn attempt_once(
        page: &chromiumoxide::Page,
        source: &str,
        referrer: Option<String>,
        cache_options: &Option<CacheOptions>,
        cache_policy: &Option<BasicCachePolicy>,
        cache_namespace: Option<&str>,
    ) -> Result<ChromeHTTPReqRes, chromiumoxide::error::CdpError> {
        let mut waf_check = false;
        let mut status_code = *crate::page::UNKNOWN_STATUS_ERROR;
        let mut method = String::from("GET");
        let mut response_headers: std::collections::HashMap<String, String> =
            std::collections::HashMap::default();
        let mut request_headers = std::collections::HashMap::default();
        let mut protocol = String::from("http/1.1");
        let mut anti_bot_tech = AntiBotTech::default();

        let frame_id = page.mainframe().await?;

        let cmd = chromiumoxide::cdp::browser_protocol::page::NavigateParams {
            url: source.to_string(),
            transition_type: Some(
                chromiumoxide::cdp::browser_protocol::page::TransitionType::Other,
            ),
            frame_id,
            referrer,
            referrer_policy: None,
        };

        let auth_opt = cache_auth_token(cache_options);
        let cache_policy = chrome_cache_policy(cache_policy);
        let cache_strategy = None;
        let remote = None;

        let page_base = page.http_future_with_cache_intercept_enabled(
            cmd,
            auth_opt,
            cache_policy,
            cache_strategy,
            remote,
            cache_namespace,
        );

        match page_base.await {
            Ok(http_request) => {
                if let Some(http_method) = http_request.method.as_deref() {
                    method = http_method.into();
                }

                request_headers.clone_from(&http_request.headers);

                if let Some(response) = &http_request.response {
                    if let Some(p) = &response.protocol {
                        protocol.clone_from(p);
                    }

                    if let Some(res_headers) = response.headers.inner().as_object() {
                        for (k, v) in res_headers {
                            response_headers.insert(k.to_string(), v.to_string());
                        }
                    }

                    let mut firewall = false;

                    waf_check = detect_antibot_from_url(&response.url).is_some();

                    if !response.url.starts_with(source) {
                        match &response.security_details {
                            Some(security_details) => {
                                anti_bot_tech = detect_anti_bot_tech_response(
                                    &response.url,
                                    &HeaderSource::Map(&response_headers),
                                    &[],
                                    Some(&security_details.subject_name),
                                );
                                firewall = true;
                            }
                            _ => {
                                anti_bot_tech = detect_anti_bot_tech_response(
                                    &response.url,
                                    &HeaderSource::Map(&response_headers),
                                    &[],
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

                        waf_check =
                            waf_check || firewall && !matches!(anti_bot_tech, AntiBotTech::None);

                        if !waf_check {
                            waf_check = match &response.protocol {
                                Some(protocol) => protocol == "blob",
                                _ => false,
                            }
                        }
                    }

                    status_code = StatusCode::from_u16(response.status as u16)
                        .unwrap_or(StatusCode::EXPECTATION_FAILED);
                } else if let Some(failure_text) = &http_request.failure_text {
                    if failure_text == "net::ERR_FAILED" {
                        waf_check = true;
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

    match attempt_once(
        page,
        source,
        referrer.clone(),
        cache_options,
        cache_policy,
        cache_namespace,
    )
    .await
    {
        Ok(ok) => Ok(ok),
        Err(e) => {
            if is_cipher_mismatch(&e) {
                if let Some(flipped) = flip_http_https(source) {
                    return attempt_once(
                        page,
                        &flipped,
                        referrer.clone(),
                        cache_options,
                        cache_policy,
                        cache_namespace,
                    )
                    .await;
                }
            }
            if is_ssl_protocol_error(&e) {
                if let Some(no_www) = strip_www(source) {
                    return attempt_once(
                        page,
                        &no_www,
                        referrer,
                        cache_options,
                        cache_policy,
                        cache_namespace,
                    )
                    .await;
                }
            }
            Err(e)
        }
    }
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
    page_response: &mut PageResponse,
    ok: bool,
) {
    if let Some(gpt_configs) = openai_config {
        let gpt_configs = match &gpt_configs.prompt_url_map {
            Some(h) => {
                let c = h.get::<case_insensitive_string::CaseInsensitiveString>(&source.into());

                if c.is_none() && gpt_configs.paths_map {
                    h.get::<case_insensitive_string::CaseInsensitiveString>(
                        &get_path_from_url(source).into(),
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
                        source,
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
                handle_openai_credits(page_response, tokens_used);

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
                    let html: Option<Vec<u8>> = match page
                        .evaluate_function(string_concat!(
                            "async function() { ",
                            json_res.js,
                            "; return document.documentElement.outerHTML; }"
                        ))
                        .await
                    {
                        Ok(h) => h.into_value().ok(),
                        _ => None,
                    };

                    if html.is_some() {
                        page_wait(page, wait_for).await;
                        if json_res.js.len() <= 400 && json_res.js.contains("window.location") {
                            if let Ok(b) = page.outer_html_bytes().await {
                                page_response.content = Some(b);
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
#[cfg(any(feature = "cache_chrome_hybrid", feature = "cache_chrome_hybrid_mem"))]
pub struct HttpRequestLike {
    ///  The URI component of a request.
    pub uri: http::uri::Uri,
    /// The http method.
    pub method: reqwest::Method,
    /// The http headers.
    pub headers: http::HeaderMap,
}

#[cfg(any(feature = "cache_chrome_hybrid", feature = "cache_chrome_hybrid_mem"))]
/// A HTTP response type for caching.
pub struct HttpResponseLike {
    /// The http status code.
    pub status: StatusCode,
    /// The http headers.
    pub headers: http::HeaderMap,
}

#[cfg(any(feature = "cache_chrome_hybrid", feature = "cache_chrome_hybrid_mem"))]
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

#[cfg(any(feature = "cache_chrome_hybrid", feature = "cache_chrome_hybrid_mem"))]
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

#[cfg(any(feature = "cache_chrome_hybrid", feature = "cache_chrome_hybrid_mem"))]
/// Store the page to cache to be re-used across HTTP request.
pub async fn put_hybrid_cache(
    cache_key: &str,
    http_response: HttpResponse,
    method: &str,
    http_request_headers: std::collections::HashMap<String, String>,
) {
    use crate::http_cache_reqwest::CacheManager;
    use http_cache_semantics::CachePolicy;

    // Never cache empty or near-empty HTML responses.
    if is_cacheable_body_empty(&http_response.body) {
        return;
    }

    if let Ok(u) = http_response.url.as_str().parse::<http::uri::Uri>() {
        let req = HttpRequestLike {
            uri: u,
            method: reqwest::Method::from_bytes(method.as_bytes()).unwrap_or(reqwest::Method::GET),
            headers: convert_headers(&http_request_headers),
        };

        // Build policy headers: start from the real response headers but ensure
        // the CachePolicy has a usable max-age for Period-based staleness.
        //
        // Chrome-crawled pages commonly return no-cache / no-store / Set-Cookie
        // which makes CachePolicy.max_age()=0 → is_stale() always true.
        // We only override the headers used for the *policy* (not the stored response).
        //
        // Strategy:
        //   1. If the server provides last-modified, etag, expires, or a positive
        //      max-age → respect it (HTTP semantics work via the heuristic).
        //   2. If the server says no-cache, no-store, or provides no caching
        //      signals at all → inject a 2-day max-age so Period(now-2d) works.
        let mut policy_headers = http_response.headers.clone();
        let cc_lower = policy_headers
            .get("cache-control")
            .map(|v| v.to_lowercase());

        let has_no_cache = cc_lower
            .as_ref()
            .is_some_and(|v| v.contains("no-cache") || v.contains("no-store"));

        let has_positive_max_age = cc_lower.as_ref().is_some_and(|v| {
            v.split(',')
                .filter_map(|d| {
                    let d = d.trim();
                    d.strip_prefix("max-age=")
                        .or_else(|| d.strip_prefix("s-maxage="))
                })
                .any(|val| val.trim().parse::<u64>().unwrap_or(0) > 0)
        });

        let has_heuristic_signal =
            policy_headers.contains_key("last-modified") || policy_headers.contains_key("expires");

        // Override when: explicit no-cache/no-store, OR no caching signal at all
        if has_no_cache || (!has_positive_max_age && !has_heuristic_signal) {
            policy_headers.insert(
                "cache-control".to_string(),
                "public, max-age=172800".to_string(),
            );
            // Remove conflicting headers that would override max-age
            policy_headers.remove("pragma");
        }

        let res = HttpResponseLike {
            status: StatusCode::from_u16(http_response.status)
                .unwrap_or(StatusCode::EXPECTATION_FAILED),
            headers: convert_headers(&policy_headers),
        };

        // Use shared=false: this is a per-crawl cache, not a shared proxy.
        // Prevents Set-Cookie from forcing max_age=0 in shared-cache mode.
        let opts = http_cache_semantics::CacheOptions {
            shared: false,
            ..Default::default()
        };
        let policy = CachePolicy::new_options(&req, &res, std::time::SystemTime::now(), opts);

        let _ = crate::website::CACACHE_MANAGER
            .put(
                cache_key.into(),
                http_cache_reqwest::HttpResponse {
                    url: http_response.url,
                    body: http_response.body,
                    headers: http_cache::HttpHeaders::Modern(
                        http_response
                            .headers
                            .iter()
                            .map(|(k, v)| (k.clone(), vec![v.clone()]))
                            .collect(),
                    ),
                    version: match http_response.version {
                        HttpVersion::H2 => http_cache::HttpVersion::H2,
                        HttpVersion::Http10 => http_cache::HttpVersion::Http10,
                        HttpVersion::H3 => http_cache::HttpVersion::H3,
                        HttpVersion::Http09 => http_cache::HttpVersion::Http09,
                        HttpVersion::Http11 => http_cache::HttpVersion::Http11,
                    },
                    status: http_response.status,
                    metadata: None,
                },
                policy,
            )
            .await;
    }
}

#[cfg(not(any(feature = "cache_chrome_hybrid", feature = "cache_chrome_hybrid_mem")))]
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
    base_timeout.checked_sub(elapsed).unwrap_or_default()
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

/// Get the initial page headers of the page with navigation from the remote cache.
#[cfg(all(feature = "chrome", feature = "chrome_remote_cache"))]
async fn navigate_cache(
    page: &chromiumoxide::Page,
    url: &str,
    chrome_http_req_res: &mut ChromeHTTPReqRes,
    referrer: Option<String>,
    cache_options: &Option<CacheOptions>,
    cache_policy: &Option<BasicCachePolicy>,
    cache_namespace: Option<&str>,
) -> Result<(), chromiumoxide::error::CdpError> {
    *chrome_http_req_res = perform_chrome_http_request_cache(
        page,
        url,
        referrer,
        cache_options,
        cache_policy,
        cache_namespace,
    )
    .await?;
    Ok(())
}

#[cfg(all(feature = "real_browser", feature = "chrome"))]
/// Generate random mouse movement. This does nothing without the 'real_browser' flag enabled.
pub async fn perform_smart_mouse_movement(
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
/// Generate random mouse movement. This does nothing without the 'real_browser' flag enabled.
async fn perform_smart_mouse_movement(
    _page: &chromiumoxide::Page,
    _viewport: &Option<crate::configuration::Viewport>,
) {
}

/// Cache the chrome response
#[cfg(all(
    feature = "chrome",
    any(feature = "cache_chrome_hybrid", feature = "cache_chrome_hybrid_mem")
))]
pub async fn cache_chrome_response(
    target_url: &str,
    page_response: &PageResponse,
    chrome_http_req_res: ChromeHTTPReqRes,
    cache_options: &Option<CacheOptions>,
    namespace: Option<&str>,
) {
    // Skip caching empty content.
    let body = match page_response.content.as_ref() {
        Some(b) if !is_cacheable_body_empty(b) => b.to_vec(),
        _ => return,
    };

    let u = match url::Url::parse(target_url) {
        Ok(u) => u,
        Err(_) => return,
    };

    let chromey_version = match chrome_http_req_res.protocol.as_str() {
        "http/0.9" => HttpVersion::Http09,
        "http/1" | "http/1.0" => HttpVersion::Http10,
        "http/1.1" => HttpVersion::Http11,
        "http/2.0" | "http/2" => HttpVersion::H2,
        "http/3.0" | "http/3" => HttpVersion::H3,
        _ => HttpVersion::Http11,
    };

    let auth_opt = match cache_options {
        Some(CacheOptions::Yes) | Some(CacheOptions::SkipBrowser) => None,
        Some(CacheOptions::Authorized(token))
        | Some(CacheOptions::SkipBrowserAuthorized(token)) => Some(token),
        Some(CacheOptions::No) | None => None,
    };
    let cache_key = create_cache_key_raw(
        target_url,
        Some(&chrome_http_req_res.method),
        auth_opt.map(|token| token.as_ref()),
        namespace,
    );

    // Destructure chrome_http_req_res to avoid cloning fields consumed by both paths.
    let ChromeHTTPReqRes {
        method,
        response_headers,
        request_headers,
        status_code,
        ..
    } = chrome_http_req_res;

    // Prepare remote dump data BEFORE put_hybrid_cache consumes the HttpResponse.
    // Use the same body/headers — the worker batches, deduplicates, and uploads.
    #[cfg(feature = "chrome_remote_cache")]
    let remote_dump_data = {
        let cache_site =
            chromiumoxide::cache::manager::site_key_for_target_url(target_url, None, namespace);
        let remote_version = match chromey_version {
            HttpVersion::Http09 => spider_remote_cache::HttpVersion::Http09,
            HttpVersion::Http10 => spider_remote_cache::HttpVersion::Http10,
            HttpVersion::H2 => spider_remote_cache::HttpVersion::H2,
            HttpVersion::H3 => spider_remote_cache::HttpVersion::H3,
            _ => spider_remote_cache::HttpVersion::Http11,
        };
        // Clone only what the spawned task needs; body is cloned once (not twice).
        Some((
            cache_key.clone(),
            cache_site,
            body.clone(),
            status_code,
            request_headers.clone(),
            response_headers.clone(),
            remote_version,
            method.clone(),
            target_url.to_string(),
        ))
    };

    let http_response = HttpResponse {
        url: u,
        body,
        status: status_code.into(),
        version: chromey_version,
        headers: response_headers,
    };

    put_hybrid_cache(&cache_key, http_response, &method, request_headers).await;

    // Best-effort enqueue into the shared worker — batched, deduped, concurrent.
    #[cfg(feature = "chrome_remote_cache")]
    if let Some((key, site, body, _status, req_hdrs, resp_hdrs, remote_version, method, target)) =
        remote_dump_data
    {
        let job = spider_remote_cache::DumpJob {
            cache_key: key,
            cache_site: site,
            url: target,
            method,
            status: _status.into(),
            request_headers: req_hdrs,
            response_headers: resp_hdrs,
            body,
            http_version: remote_version,
            dump_remote: None,
        };

        if spider_remote_cache::worker_inited() {
            if !spider_remote_cache::try_enqueue(job) {
                #[cfg(feature = "tracing")]
                tracing::debug!("remote dump skipped (queue full)");
            }
        } else {
            // Worker should already be inited by spawn_cache_listener, but
            // enqueue() auto-inits as a fallback (uses default client).
            if let Err(_) = spider_remote_cache::enqueue(job).await {
                #[cfg(feature = "tracing")]
                tracing::debug!("remote dump skipped (queue full)");
            }
        }
    }
}

/// Cache the chrome response
#[cfg(all(
    feature = "chrome",
    not(any(feature = "cache_chrome_hybrid", feature = "cache_chrome_hybrid_mem"))
))]
pub async fn cache_chrome_response(
    _target_url: &str,
    _page_response: &PageResponse,
    _chrome_http_req_res: ChromeHTTPReqRes,
    _cache_options: &Option<CacheOptions>,
    _namespace: Option<&str>,
) {
}

/// 5 mins in ms
pub(crate) const FIVE_MINUTES: u32 = 300_000;

/// Max page timeout for events.
#[cfg(feature = "chrome")]
const MAX_PAGE_TIMEOUT: tokio::time::Duration =
    tokio::time::Duration::from_millis(FIVE_MINUTES as u64);
/// Half of the max timeout
#[cfg(feature = "chrome")]
const HALF_MAX_PAGE_TIMEOUT: tokio::time::Duration =
    tokio::time::Duration::from_millis(FIVE_MINUTES as u64 / 2);

#[cfg(all(feature = "chrome", feature = "headers"))]
/// Store the page headers. This does nothing without the 'headers' flag enabled.
fn store_headers(page_response: &PageResponse, chrome_http_req_res: &mut ChromeHTTPReqRes) {
    if let Some(response_headers) = &page_response.headers {
        chrome_http_req_res.response_headers =
            crate::utils::header_utils::header_map_to_hash_map(response_headers);
    }
}

#[cfg(all(feature = "chrome", not(feature = "headers")))]
/// Store the page headers. This does nothing without the 'headers' flag enabled.
fn store_headers(_page_response: &PageResponse, _chrome_http_req_res: &mut ChromeHTTPReqRes) {}

#[inline]
/// f64 to u64 floor.
#[cfg(feature = "chrome")]
fn f64_to_u64_floor(x: f64) -> u64 {
    if !x.is_finite() || x <= 0.0 {
        0
    } else if x >= u64::MAX as f64 {
        u64::MAX
    } else {
        x as u64
    }
}

#[cfg(all(feature = "chrome", feature = "cache_request"))]
/// Cache a chrome response from CDP body.
async fn cache_chrome_response_from_cdp_body(
    target_url: &str,
    body: &[u8],
    chrome_http_req_res: &ChromeHTTPReqRes,
    cache_options: &Option<CacheOptions>,
    namespace: Option<&str>,
) {
    use crate::utils::create_cache_key_raw;

    // Skip caching empty content.
    if is_cacheable_body_empty(body) {
        return;
    }

    if let Ok(u) = url::Url::parse(target_url) {
        let http_response = HttpResponse {
            url: u,
            body: body.to_vec(),
            status: chrome_http_req_res.status_code.into(),
            version: match chrome_http_req_res.protocol.as_str() {
                "http/0.9" => HttpVersion::Http09,
                "http/1" | "http/1.0" => HttpVersion::Http10,
                "http/1.1" => HttpVersion::Http11,
                "http/2.0" | "http/2" => HttpVersion::H2,
                "http/3.0" | "http/3" => HttpVersion::H3,
                _ => HttpVersion::Http11,
            },
            headers: chrome_http_req_res.response_headers.clone(),
        };

        let auth_opt = match cache_options {
            Some(CacheOptions::Yes) | Some(CacheOptions::SkipBrowser) => None,
            Some(CacheOptions::Authorized(token))
            | Some(CacheOptions::SkipBrowserAuthorized(token)) => Some(token),
            Some(CacheOptions::No) | None => None,
        };
        let cache_key = create_cache_key_raw(
            target_url,
            Some(&chrome_http_req_res.method),
            auth_opt.map(|x| x.as_str()),
            namespace,
        );

        put_hybrid_cache(
            &cache_key,
            http_response,
            &chrome_http_req_res.method,
            chrome_http_req_res.request_headers.clone(),
        )
        .await;
    }
}

#[derive(Debug, Clone, Default)]
#[cfg(feature = "chrome")]
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
#[cfg(feature = "chrome")]
struct ResponseBase {
    /// The map of the response.
    response_map: Option<hashbrown::HashMap<String, ResponseMap>>,
    /// The headers of request.
    headers: Option<chromiumoxide::cdp::browser_protocol::network::Headers>,
    /// The status code.
    status_code: Option<i64>,
    #[cfg(feature = "cache_request")]
    /// Is the main document cached?
    main_doc_from_cache: bool,
    #[cfg(feature = "remote_addr")]
    /// The remote IP address of the main document response (from CDP Network.responseReceived).
    /// Provides parity with the HTTP-mode `remote_addr` field.
    remote_ip_address: Option<String>,
    #[cfg(feature = "remote_addr")]
    /// The remote port of the main document response.
    remote_port: Option<i64>,
}

#[cfg(feature = "chrome")]
#[inline]
/// The log target.
fn log_target<'a>(source: &'a str, url_target: Option<&'a str>) -> &'a str {
    url_target.unwrap_or(source)
}

#[cfg(feature = "chrome")]
#[inline]
/// Is this a timeout error?
fn is_timeout(e: &chromiumoxide::error::CdpError) -> bool {
    matches!(e, chromiumoxide::error::CdpError::Timeout)
}

#[cfg(feature = "chrome")]
/// Go to the html with interception.
async fn goto_with_html_once(
    page: &chromiumoxide::Page,
    target_url: &str,
    html: &[u8],
    block_bytes: &mut bool,
    resp_headers: &Option<reqwest::header::HeaderMap<reqwest::header::HeaderValue>>,
    _chrome_intercept: &Option<&crate::features::chrome_common::RequestInterceptConfiguration>,
) -> Result<(), chromiumoxide::error::CdpError> {
    use base64::Engine;
    use chromiumoxide::cdp::browser_protocol::fetch::{
        DisableParams, EnableParams, EventRequestPaused, FulfillRequestParams, RequestPattern,
        RequestStage,
    };
    use chromiumoxide::cdp::browser_protocol::network::ResourceType;
    use tokio_stream::StreamExt;

    let fulfill_headers =
        chrome_fulfill_headers_from_reqwest(resp_headers.as_ref(), "text/html; charset=utf-8");

    // Tell the handler we will handle paused requests ourselves. This sets
    // `user_request_interception_enabled = !true = false` via the inverted
    // EnableInterception semantics. Combined with the listener registration
    // below (which sets `protocol_request_interception_enabled = true`),
    // the handler guard `user && protocol` evaluates to `false && true`,
    // so the handler still auto-processes events. We then flip it back with
    // set_request_interception(false) which sets `user = true`. With both
    // flags true the handler defers entirely to our listener — no race
    // between the handler's continueRequest and our fulfillRequest.
    let mut paused = page.event_listener::<EventRequestPaused>().await?;

    // Enable a narrow Document-only Fetch pattern. Only Document requests
    // will be paused by Chrome; subresource requests proceed unintercepted.
    // This prevents the old deadlock: with a wide "*" pattern, non-Document
    // events that nobody answers would hang forever.
    page.execute(EnableParams {
        patterns: Some(vec![RequestPattern {
            url_pattern: Some("*".into()),
            resource_type: Some(ResourceType::Document),
            request_stage: Some(RequestStage::Request),
        }]),
        handle_auth_requests: Some(false),
    })
    .await?;

    // Now make the handler defer to our listener. set_request_interception(false)
    // sends EnableInterception(false) → user_request_interception_enabled = !false = true.
    // With both user=true and protocol=true (from listener registration above),
    // the handler's guard returns early, letting ONLY our listener handle events.
    // This is safe because only Document events arrive (narrow pattern above).
    page.set_request_interception(false).await?;

    let mut did_goto = false;

    loop {
        tokio::select! {
            biased;
            res = page.goto(target_url), if !did_goto => {
                did_goto = true;
                if let Err(e) = res {
                    if matches!(e, chromiumoxide::error::CdpError::Timeout) {
                        *block_bytes = true;
                    }
                    let _ = page.execute(DisableParams {}).await;
                    let _ = page.set_request_interception(true).await;
                    return Err(e);
                }
            }
            maybe_ev = paused.next() => {
                let Some(ev) = maybe_ev else {
                    break;
                };

                if ev.resource_type != ResourceType::Document {
                    continue;
                }

                let body_b64 = base64::engine::general_purpose::STANDARD.encode(html);

                let res = page.execute(FulfillRequestParams {
                    request_id: ev.request_id.clone(),
                    response_code: 200,
                    response_phrase: None,
                    response_headers: Some(fulfill_headers.clone()),
                    body: Some(chromiumoxide::Binary(body_b64)),
                    binary_response_headers: None,
                }).await;

                let _ = page.execute(DisableParams {}).await;
                let _ = page.set_request_interception(true).await;

                match res {
                    Ok(_) => {
                        // Wait for the full load lifecycle (matching
                        // http_future's behavior in the normal Chrome path),
                        // then network idle so sub-resources and JS execute.
                        let _ = tokio::time::timeout(
                            tokio::time::Duration::from_secs(30),
                            page.wait_for_navigation(),
                        )
                        .await;
                        let _ = tokio::time::timeout(
                            tokio::time::Duration::from_secs(15),
                            page.wait_for_network_idle(),
                        )
                        .await;
                        return Ok(());
                    }
                    Err(e) => {
                        if matches!(e, chromiumoxide::error::CdpError::Timeout) {
                            *block_bytes = true;
                        }
                        return Err(e);
                    }
                }
            }
        }
    }

    let _ = page.execute(DisableParams {}).await;
    let _ = page.set_request_interception(true).await;

    Ok(())
}

#[cfg(feature = "chrome")]
/// Set the document if requested.
async fn set_document_content_if_requested(
    page: &chromiumoxide::Page,
    source: &[u8],
    url_target: Option<&str>,
    block_bytes: &mut bool,
    resp_headers: &Option<HeaderMap<HeaderValue>>,
    chrome_intercept: &Option<&crate::features::chrome_common::RequestInterceptConfiguration>,
) {
    if let Some(target_url) = url_target {
        let _ = goto_with_html_once(
            page,
            target_url,
            source,
            block_bytes,
            resp_headers,
            chrome_intercept,
        )
        .await;
    }
}

#[cfg(all(feature = "chrome", feature = "chrome_remote_cache"))]
/// Set the document if requested cached.
async fn set_document_content_if_requested_cached(
    page: &chromiumoxide::Page,
    source: &[u8],
    url_target: Option<&str>,
    block_bytes: &mut bool,
    cache_options: &Option<CacheOptions>,
    cache_policy: &Option<BasicCachePolicy>,
    resp_headers: &Option<HeaderMap<HeaderValue>>,
    chrome_intercept: &Option<&crate::features::chrome_common::RequestInterceptConfiguration>,
    namespace: Option<&str>,
) {
    let auth_opt = cache_auth_token(cache_options);
    let cache_policy = chrome_cache_policy(cache_policy);
    let cache_strategy = None;
    let remote = Some("true");
    let target_url = url_target.unwrap_or_default();
    let cache_site =
        chromiumoxide::cache::manager::site_key_for_target_url(target_url, auth_opt, namespace);

    let _ = page
        .set_cache_key((Some(cache_site.clone()), cache_policy.clone()))
        .await;

    let cache_future = async {
        if let Some(target_url) = url_target {
            let _ = goto_with_html_once(
                page,
                target_url,
                source,
                block_bytes,
                resp_headers,
                chrome_intercept,
            )
            .await;
        }
    };

    // Eagerly init the remote cache worker before the listener starts so
    // all uploads hit the fast try_enqueue path.
    #[cfg(feature = "chrome_remote_cache")]
    if remote.is_some() {
        #[cfg(feature = "chrome")]
        spider_remote_cache::set_client(chromiumoxide::browser::request_client().clone());
        #[cfg(not(feature = "chrome"))]
        spider_remote_cache::set_client(reqwest::Client::new());
        spider_remote_cache::init_default_worker().await;
    }

    let (_, __, _cache_future) = tokio::join!(
        page.spawn_cache_listener(
            &cache_site,
            auth_opt.map(|f| f.into()),
            cache_strategy,
            remote.map(|f| f.into()),
            namespace,
        ),
        page.seed_cache(target_url, auth_opt, remote, namespace),
        cache_future
    );

    let _ = page.clear_local_cache(&cache_site);
}

#[cfg(feature = "chrome")]
async fn navigate_if_requested(
    page: &chromiumoxide::Page,
    source: &str,
    url_target: Option<&str>,
    chrome_http_req_res: &mut ChromeHTTPReqRes,
    referrer: Option<String>,
    block_bytes: &mut bool,
) -> Result<(), chromiumoxide::error::CdpError> {
    if let Err(e) = navigate(page, source, chrome_http_req_res, referrer).await {
        log::info!(
            "Navigation Error({:?}) - {:?}",
            e,
            log_target(source, url_target)
        );
        if is_timeout(&e) {
            *block_bytes = true;
        }
        return Err(e);
    }
    Ok(())
}

#[cfg(all(feature = "chrome", feature = "chrome_remote_cache"))]
/// Navigate with the cache options.
async fn navigate_if_requested_cache(
    page: &chromiumoxide::Page,
    source: &str,
    url_target: Option<&str>,
    chrome_http_req_res: &mut ChromeHTTPReqRes,
    referrer: Option<String>,
    block_bytes: &mut bool,
    cache_options: &Option<CacheOptions>,
    cache_policy: &Option<BasicCachePolicy>,
    cache_namespace: Option<&str>,
) -> Result<(), chromiumoxide::error::CdpError> {
    if let Err(e) = navigate_cache(
        page,
        source,
        chrome_http_req_res,
        referrer,
        cache_options,
        cache_policy,
        cache_namespace,
    )
    .await
    {
        log::info!(
            "Navigation Error({:?}) - {:?}",
            e,
            log_target(source, url_target)
        );
        if is_timeout(&e) {
            *block_bytes = true;
        }
        return Err(e);
    }
    Ok(())
}

#[cfg(all(feature = "chrome", feature = "chrome_remote_cache"))]
/// Is cache enabled?
fn cache_enabled(cache_options: &Option<CacheOptions>) -> bool {
    matches!(
        cache_options,
        Some(CacheOptions::Yes | CacheOptions::Authorized(_))
    )
}

#[cfg(all(feature = "chrome", feature = "chrome_remote_cache"))]
/// The chrome cache policy
fn chrome_cache_policy(
    cache_policy: &Option<BasicCachePolicy>,
) -> Option<chromiumoxide::cache::BasicCachePolicy> {
    cache_policy.as_ref().map(BasicCachePolicy::from_basic)
}

#[cfg(all(feature = "chrome", not(feature = "chrome_remote_cache")))]
/// Core logic: either set document content or navigate.
///
/// Semantics preserved:
/// - If `page_set == true`: no-op.
/// - If `content == true`: tries SetDocumentContent; logs errors; sets `block_bytes` on timeout; does NOT return Err.
/// - Else: performs navigation; returns Err on failure; sets `block_bytes` on timeout.
pub async fn run_navigate_or_content_set_core(
    page: &chromiumoxide::Page,
    page_set: bool,
    content: bool,
    source: &[u8],
    url_target: Option<&str>,
    chrome_http_req_res: &mut ChromeHTTPReqRes,
    referrer: Option<String>,
    block_bytes: &mut bool,
    _cache_options: &Option<CacheOptions>,
    _cache_policy: &Option<BasicCachePolicy>,
    resp_headers: &Option<HeaderMap<HeaderValue>>,
    chrome_intercept: &Option<&crate::features::chrome_common::RequestInterceptConfiguration>,
    _namespace: Option<&str>,
) -> Result<(), chromiumoxide::error::CdpError> {
    if page_set {
        return Ok(());
    }

    if content {
        // check cf for the antibot
        if crate::features::solvers::detect_cf_turnstyle(source) {
            chrome_http_req_res.anti_bot_tech = AntiBotTech::Cloudflare;
        }
        set_document_content_if_requested(
            page,
            source,
            url_target,
            block_bytes,
            resp_headers,
            chrome_intercept,
        )
        .await;
        return Ok(());
    }

    // Navigate path: source is a URL — convert to &str for navigation APIs.
    let source_url = simdutf8::basic::from_utf8(source).unwrap_or_default();
    navigate_if_requested(
        page,
        source_url,
        url_target,
        chrome_http_req_res,
        referrer,
        block_bytes,
    )
    .await
}

#[cfg(all(feature = "chrome", feature = "chrome_remote_cache"))]
/// Core logic: either set document content or navigate.
///
/// Semantics preserved:
/// - If `page_set == true`: no-op.
/// - If `content == true`: tries SetDocumentContent; logs errors; sets `block_bytes` on timeout; does NOT return Err.
/// - Else: performs navigation; returns Err on failure; sets `block_bytes` on timeout.
pub async fn run_navigate_or_content_set_core(
    page: &chromiumoxide::Page,
    page_set: bool,
    content: bool,
    source: &[u8],
    url_target: Option<&str>,
    chrome_http_req_res: &mut ChromeHTTPReqRes,
    referrer: Option<String>,
    block_bytes: &mut bool,
    cache_options: &Option<CacheOptions>,
    cache_policy: &Option<BasicCachePolicy>,
    resp_headers: &Option<HeaderMap<HeaderValue>>,
    chrome_intercept: &Option<&crate::features::chrome_common::RequestInterceptConfiguration>,
    namespace: Option<&str>,
) -> Result<(), chromiumoxide::error::CdpError> {
    if page_set {
        return Ok(());
    }

    let cache = cache_enabled(cache_options);

    if content {
        // check cf for the antibot
        if crate::features::solvers::detect_cf_turnstyle(source) {
            chrome_http_req_res.anti_bot_tech = AntiBotTech::Cloudflare;
        }

        if cache {
            set_document_content_if_requested_cached(
                page,
                source,
                url_target,
                block_bytes,
                cache_options,
                cache_policy,
                resp_headers,
                chrome_intercept,
                namespace,
            )
            .await;
        } else {
            set_document_content_if_requested(
                page,
                source,
                url_target,
                block_bytes,
                resp_headers,
                chrome_intercept,
            )
            .await;
        }
        return Ok(());
    }

    // Navigate path: source is a URL — convert to &str for navigation APIs.
    let source_url = simdutf8::basic::from_utf8(source).unwrap_or_default();
    if cache {
        navigate_if_requested_cache(
            page,
            source_url,
            url_target,
            chrome_http_req_res,
            referrer,
            block_bytes,
            cache_options,
            cache_policy,
            namespace,
        )
        .await
    } else {
        navigate_if_requested(
            page,
            source_url,
            url_target,
            chrome_http_req_res,
            referrer,
            block_bytes,
        )
        .await
    }
}

#[cfg(feature = "chrome")]
/// Get the base redirect for the website.
pub async fn get_final_redirect(
    page: &chromiumoxide::Page,
    source: &str,
    base_timeout: Duration,
) -> Option<String> {
    let last_redirect = tokio::time::timeout(base_timeout, async {
        match page.wait_for_navigation_response().await {
            Ok(u) => get_last_redirect(source, &u, page).await,
            _ => None,
        }
    })
    .await;

    match last_redirect {
        Ok(final_url) => {
            if final_url.as_deref() == Some("about:blank")
                || final_url.as_deref() == Some("chrome-error://chromewebdata/")
            {
                None
            } else {
                final_url
            }
        }
        _ => None,
    }
}

#[cfg(feature = "chrome")]
/// Fullfil the headers.
pub fn chrome_fulfill_headers_from_reqwest(
    headers: Option<&reqwest::header::HeaderMap<reqwest::header::HeaderValue>>,
    default_content_type: &'static str,
) -> Vec<chromiumoxide::cdp::browser_protocol::fetch::HeaderEntry> {
    use chromiumoxide::cdp::browser_protocol::fetch::HeaderEntry;

    let mut out: Vec<HeaderEntry> =
        Vec::with_capacity(headers.as_ref().map_or(1, |hm| hm.len().min(32) + 1));

    // Convert reqwest headers -> CDP HeaderEntry (filter hop-by-hop)
    if let Some(hm) = headers {
        for (name, value) in hm.iter() {
            let k = name.as_str();

            // Hop-by-hop / unsafe in synthetic fulfill responses.
            // reqwest header names are already lowercase ASCII.
            if HOP_BY_HOP_HEADERS.contains(k) {
                continue;
            }

            let v = match value.to_str() {
                Ok(s) => s.to_string(),
                Err(_) => String::from_utf8_lossy(value.as_bytes()).into_owned(),
            };

            out.push(HeaderEntry {
                name: k.to_string(),
                value: v,
            });
        }
    }

    // Ensure Content-Type exists
    let has_ct = out
        .iter()
        .any(|h| h.name.eq_ignore_ascii_case("content-type"));
    if !has_ct {
        out.push(HeaderEntry {
            name: "Content-Type".into(),
            value: default_content_type.into(),
        });
    }

    // Good default for synthetic responses (avoid caching weirdness)
    if !out
        .iter()
        .any(|h| h.name.eq_ignore_ascii_case("cache-control"))
    {
        out.push(HeaderEntry {
            name: "Cache-Control".into(),
            value: "no-store".into(),
        });
    }

    out
}

#[cfg(feature = "chrome")]
/// Skip bytes tracker.
const SKIP_BYTES_AMOUNT: f64 = 17.0;

#[cfg(feature = "chrome")]
/// Perform a network request to a resource extracting all content as text streaming via chrome.
pub async fn fetch_page_html_chrome_base(
    source: &[u8],
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
    request_timeout: &Option<Duration>,
    track_events: &Option<crate::configuration::ChromeEventTracker>,
    referrer: Option<String>,
    max_page_bytes: Option<f64>,
    cache_options: Option<CacheOptions>,
    cache_policy: &Option<BasicCachePolicy>,
    resp_headers: &Option<HeaderMap<HeaderValue>>,
    chrome_intercept: &Option<&crate::features::chrome_common::RequestInterceptConfiguration>,
    jar: Option<&std::sync::Arc<crate::client::cookie::Jar>>,
    remote_multimodal: &Option<Box<RemoteMultimodalConfigs>>,
    cache_namespace: Option<&str>,
) -> Result<PageResponse, chromiumoxide::error::CdpError> {
    use crate::page::{is_asset_url, DOWNLOADABLE_MEDIA_TYPES, UNKNOWN_STATUS_ERROR};
    use chromiumoxide::{
        cdp::browser_protocol::network::{
            EventDataReceived, EventLoadingFailed, EventRequestWillBeSent, EventResponseReceived,
            GetResponseBodyParams, RequestId, ResourceType,
        },
        error::CdpError,
    };
    use tokio::{
        sync::{oneshot, OnceCell},
        time::Instant,
    };

    let duration = if cfg!(feature = "time") {
        Some(tokio::time::Instant::now())
    } else {
        None
    };

    let mut chrome_http_req_res = ChromeHTTPReqRes::default();
    let mut metadata: Option<Vec<crate::page::AutomationResults>> = None;
    let mut block_bytes = false;

    // the base networking timeout to prevent any hard hangs.
    // When request_timeout is None (user disabled it), use a generous 30-minute timeout
    // to allow long-running automation tasks while still preventing infinite hangs.
    let mut base_timeout = match request_timeout {
        Some(timeout) => (*timeout).min(MAX_PAGE_TIMEOUT),
        _ => tokio::time::Duration::from_secs(1800),
    };

    // track the initial base without modifying.
    let base_timeout_measurement = base_timeout;
    // When url_target is provided, use it directly. Otherwise source is a URL.
    let source_as_url;
    let target_url = match url_target {
        Some(u) => u,
        None => {
            source_as_url = simdutf8::basic::from_utf8(source).unwrap_or_default();
            source_as_url
        }
    };
    let asset = is_asset_url(target_url);

    let (tx1, rx1) = if asset {
        let c = oneshot::channel::<Option<RequestId>>();

        (Some(c.0), Some(c.1))
    } else {
        (None, None)
    };

    let should_block = max_page_bytes.is_some();

    let (track_requests, track_responses, track_automation) = match track_events {
        Some(tracker) => (tracker.requests, tracker.responses, tracker.automation),
        _ => (false, false, false),
    };

    let (
        event_loading_listener,
        cancel_listener,
        received_listener,
        event_sent_listener,
        event_data_received,
    ) = tokio::join!(
        page.event_listener::<chromiumoxide::cdp::browser_protocol::network::EventLoadingFinished>(
        ),
        page.event_listener::<EventLoadingFailed>(),
        page.event_listener::<EventResponseReceived>(),
        async {
            if track_requests {
                page.event_listener::<EventRequestWillBeSent>().await
            } else {
                Err(CdpError::NotFound)
            }
        },
        async {
            let chunk_idle = chunk_idle_timeout();
            if should_block || chunk_idle.is_some() {
                page.event_listener::<EventDataReceived>().await
            } else {
                Err(CdpError::NotFound)
            }
        }
    );

    #[cfg(feature = "cache_request")]
    let cache_request = match cache_options {
        Some(CacheOptions::No) => false,
        _ => true,
    };

    let (tx, rx) = oneshot::channel::<bool>();

    #[cfg(feature = "cache_request")]
    let (main_tx, main_rx) = if cache_request {
        let c = oneshot::channel::<RequestId>();
        (Some(c.0), Some(c.1))
    } else {
        (None, None)
    };

    let chunk_idle = chunk_idle_timeout();
    let page_clone = if should_block || chunk_idle.is_some() {
        Some(page.clone())
    } else {
        None
    };

    let html_source_size = source.len();

    // Lazily decode source to &str for internal APIs that require string
    // semantics (logging, URL comparison, cache keys). The hot content path
    // (set_document_content / base64 fulfillment) uses raw bytes directly.
    let source_str = simdutf8::basic::from_utf8(source).unwrap_or_default();

    // Shutdown signal for CDP event listeners. Sent after page work is done
    // so listeners exit deterministically instead of relying on stream closure.
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    // Listen for network events to track data transfer.
    // Spawning is always required here to collect network metrics in real-time.
    let bytes_collected_handle = tokio::spawn(async move {
        let finished_media: Option<OnceCell<RequestId>> =
            if asset { Some(OnceCell::new()) } else { None };

        let mut shutdown_f1 = shutdown_rx.clone();
        let f1 = async {
            let mut total = 0.0;

            let mut response_map: Option<HashMap<String, f64>> = if track_responses {
                Some(HashMap::new())
            } else {
                None
            };

            if let Ok(mut listener) = event_loading_listener {
                loop {
                    let event = tokio::select! {
                        biased;
                        _ = shutdown_f1.changed() => break,
                        ev = listener.next() => match ev {
                            Some(ev) => ev,
                            None => break,
                        },
                    };
                    total += event.encoded_data_length;
                    if let Some(response_map) = response_map.as_mut() {
                        response_map
                            .entry(event.request_id.inner().clone())
                            .and_modify(|e| *e += event.encoded_data_length)
                            .or_insert(event.encoded_data_length);
                    }
                    if asset {
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
                }
            }

            (total, response_map)
        };

        let mut shutdown_f2 = shutdown_rx.clone();
        let f2 = async {
            if let Ok(mut listener) = cancel_listener {
                let mut net_aborted = false;

                loop {
                    let event = tokio::select! {
                        biased;
                        _ = shutdown_f2.changed() => break,
                        ev = listener.next() => match ev {
                            Some(ev) => ev,
                            None => break,
                        },
                    };
                    if event.r#type == ResourceType::Document
                        && event.error_text == "net::ERR_ABORTED"
                    {
                        // canceled=true means Chrome intentionally aborted the
                        // request (e.g. following a 301/302 redirect). This is
                        // NOT a real failure — skip it so the navigation
                        // continues to the redirect target.
                        if event.canceled.unwrap_or_default() {
                            continue;
                        }
                        net_aborted = true;
                        break;
                    }
                }

                if net_aborted {
                    let _ = tx.send(true);
                }
            }
        };

        let mut shutdown_f3 = shutdown_rx.clone();
        let f3 = async {
            let mut response_map: Option<HashMap<String, ResponseMap>> = if track_responses {
                Some(HashMap::new())
            } else {
                None
            };

            let mut status_code = None;
            let mut headers = None;
            #[cfg(feature = "remote_addr")]
            let mut remote_ip_address = None;
            #[cfg(feature = "remote_addr")]
            let mut remote_port = None;
            #[cfg(feature = "cache_request")]
            let mut main_doc_request_id: Option<RequestId> = None;
            #[cfg(feature = "cache_request")]
            let mut main_doc_from_cache = false;

            let persist_event = asset || track_responses;

            if let Ok(mut listener) = received_listener {
                let mut initial_asset = false;
                let mut allow_download = false;
                let mut intial_request = false;

                loop {
                    let event = tokio::select! {
                        biased;
                        _ = shutdown_f3.changed() => break,
                        ev = listener.next() => match ev {
                            Some(ev) => ev,
                            None => break,
                        },
                    };
                    let document = event.r#type == ResourceType::Document;

                    if !intial_request && document {
                        // todo: capture the redirect code.
                        let redirect = event.response.status >= 300 && event.response.status <= 399;

                        if !redirect {
                            intial_request = true;
                            status_code = Some(event.response.status);
                            headers = Some(event.response.headers.clone());
                            #[cfg(feature = "remote_addr")]
                            {
                                remote_ip_address = event.response.remote_ip_address.clone();
                                remote_port = event.response.remote_port;
                            }
                            #[cfg(feature = "cache_request")]
                            {
                                main_doc_request_id = Some(event.request_id.clone());
                                // DevTools cache flags
                                let from_disk = event.response.from_disk_cache.unwrap_or(false);
                                let from_prefetch =
                                    event.response.from_prefetch_cache.unwrap_or(false);
                                let from_sw = event.response.from_service_worker.unwrap_or(false);
                                main_doc_from_cache = from_disk || from_prefetch || from_sw;
                            }

                            if !persist_event {
                                break;
                            }

                            if content {
                                if let Some(response_map) = response_map.as_mut() {
                                    response_map.insert(
                                        event.request_id.inner().clone(),
                                        ResponseMap {
                                            url: event.response.url.clone(),
                                            // encoded length should add 78.0 via chrome
                                            bytes_transferred: (html_source_size as f64)
                                                + event.response.encoded_data_length,
                                            skipped: true,
                                        },
                                    );
                                    continue;
                                }
                            }
                        }
                    }
                    // check if media asset needs to be downloaded ( this will trigger after the inital document )
                    else if asset {
                        if !initial_asset && document {
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
                                    && event.response.encoded_data_length <= SKIP_BYTES_AMOUNT,
                            },
                        );
                    }
                }
            }

            #[cfg(feature = "cache_request")]
            if let Some(request_id) = &main_doc_request_id {
                if let Some(tx) = main_tx {
                    let _ = tx.send(request_id.clone());
                }
            }

            ResponseBase {
                response_map,
                status_code,
                headers,
                #[cfg(feature = "cache_request")]
                main_doc_from_cache,
                #[cfg(feature = "remote_addr")]
                remote_ip_address,
                #[cfg(feature = "remote_addr")]
                remote_port,
            }
        };

        let mut shutdown_f4 = shutdown_rx.clone();
        let f4 = async {
            let mut request_map: Option<HashMap<String, f64>> = if track_requests {
                Some(HashMap::new())
            } else {
                None
            };

            if request_map.is_some() {
                if let Some(response_map) = request_map.as_mut() {
                    if let Ok(mut listener) = event_sent_listener {
                        loop {
                            let event = tokio::select! {
                                biased;
                                _ = shutdown_f4.changed() => break,
                                ev = listener.next() => match ev {
                                    Some(ev) => ev,
                                    None => break,
                                },
                            };
                            response_map
                                .insert(event.request.url.clone(), *event.timestamp.inner());
                        }
                    }
                }
            }

            request_map
        };

        let mut shutdown_f5 = shutdown_rx;
        let f5 = async {
            if let Some(page_clone) = &page_clone {
                if let Ok(mut listener) = event_data_received {
                    let mut total_bytes: u64 = 0;
                    let total_max = f64_to_u64_floor(max_page_bytes.unwrap_or_default());
                    let check_max = total_max > 0;

                    loop {
                        let event = match chunk_idle {
                            Some(timeout) => {
                                let next_event = listener.next();
                                tokio::select! {
                                    biased;
                                    _ = shutdown_f5.changed() => break,
                                    result = tokio::time::timeout(timeout, next_event) => {
                                        match result {
                                            Ok(Some(event)) => event,
                                            Ok(None) => break,
                                            Err(_elapsed) => {
                                                log::warn!(
                                                    "chrome network idle timeout ({timeout:?}), force-stopping page"
                                                );
                                                let _ = page_clone.force_stop_all().await;
                                                break;
                                            }
                                        }
                                    }
                                }
                            }
                            None => {
                                tokio::select! {
                                    biased;
                                    _ = shutdown_f5.changed() => break,
                                    ev = listener.next() => match ev {
                                        Some(event) => event,
                                        None => break,
                                    },
                                }
                            }
                        };

                        let encoded = event.encoded_data_length.max(0) as u64;
                        total_bytes = total_bytes.saturating_add(encoded);
                        if check_max && total_bytes > total_max {
                            let _ = page_clone.force_stop_all().await;
                            break;
                        }
                    }
                }
            }
        };

        let (t1, _, res_map, req_map, __) = tokio::join!(f1, f2, f3, f4, f5);

        (t1.0, t1.1, res_map, req_map)
    });

    let page_navigation = async {
        run_navigate_or_content_set_core(
            page,
            page_set,
            content,
            source,
            url_target,
            &mut chrome_http_req_res,
            referrer,
            &mut block_bytes,
            &cache_options,
            cache_policy,
            resp_headers,
            chrome_intercept,
            cache_namespace,
        )
        .await
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
                        _ = perform_smart_mouse_movement(page, viewport) => {
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

    // Content path: goto_with_html_once already waits for the `load` lifecycle
    // event, matching http_future()'s behavior. get_final_redirect resolves
    // instantly here (load already fired).
    let final_url = if wait_for_navigation && !request_cancelled && !block_bytes {
        let last_redirect = get_final_redirect(page, source_str, base_timeout).await;
        base_timeout = sub_duration(base_timeout_measurement, start_time.elapsed());
        last_redirect
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
        && !(chrome_http_req_res.is_empty() && !content)
        && (!chrome_http_req_res.status_code.is_server_error()
            && !chrome_http_req_res.status_code.is_client_error()
            || chrome_http_req_res.status_code == *UNKNOWN_STATUS_ERROR
            || chrome_http_req_res.status_code == 404
            || chrome_http_req_res.status_code == 403
            || chrome_http_req_res.status_code == 524
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
                if let Err(elasped) =
                    tokio::time::timeout(base_timeout, perform_smart_mouse_movement(page, viewport))
                        .await
                {
                    log::warn!("mouse movement timeout exceeded {elasped}");
                }
            }

            if wait_for.is_some() {
                let wait_budget = sub_duration(base_timeout_measurement, start_time.elapsed());
                #[cfg(feature = "wait_guard")]
                let wait_budget = crate::utils::wait_guard::global_wait_guard()
                    .adjusted_timeout(get_domain_from_url(source_str), wait_budget);
                if let Err(elasped) =
                    tokio::time::timeout(wait_budget, page_wait(page, wait_for)).await
                {
                    log::warn!("max wait for timeout {elasped}");
                }
            }

            base_timeout = sub_duration(base_timeout_measurement, start_time.elapsed());

            if execution_scripts.is_some() || automation_scripts.is_some() {
                let target_url = final_url
                    .as_deref()
                    .or(url_target)
                    .unwrap_or(source_str)
                    .to_string();

                if let Err(elasped) = tokio::time::timeout(base_timeout, async {
                    let mut _metadata = Vec::new();

                    if track_automation {
                        tokio::join!(
                            crate::features::chrome_common::eval_execution_scripts(
                                page,
                                &target_url,
                                execution_scripts
                            ),
                            crate::features::chrome_common::eval_automation_scripts_tracking(
                                page,
                                &target_url,
                                automation_scripts,
                                &mut _metadata
                            )
                        );
                        metadata = Some(_metadata);
                    } else {
                        tokio::join!(
                            crate::features::chrome_common::eval_execution_scripts(
                                page,
                                &target_url,
                                execution_scripts
                            ),
                            crate::features::chrome_common::eval_automation_scripts(
                                page,
                                &target_url,
                                automation_scripts
                            )
                        );
                    }
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
                if !xml_target {
                    return page.outer_html_bytes().await;
                }
                match page.content_bytes_xml().await {
                    Ok(b) if !b.is_empty() => Ok(b),
                    _ => page.outer_html_bytes().await,
                }
            };

            let results = tokio::time::timeout(base_timeout.max(HALF_MAX_PAGE_TIMEOUT), page_fn);

            let mut res: Vec<u8> = match results.await {
                Ok(v) => v.unwrap_or_default(),
                _ => Default::default(),
            };

            let forbidden = waf_check && res.starts_with(b"<html><head>\n    <style global=") && res.ends_with(b";</script><iframe height=\"1\" width=\"1\" style=\"position: absolute; top: 0px; left: 0px; border: none; visibility: hidden;\"></iframe>\n\n</body></html>");

            #[cfg(feature = "real_browser")]
            {
                // guard entry to real pages.
                if res.len() <= crate::page::TURNSTILE_WALL_PAGE_SIZE {
                    if anti_bot_tech == AntiBotTech::Cloudflare || waf_check {
                        if crate::features::solvers::detect_cf_turnstyle(&res) {
                            if let Err(_e) = tokio::time::timeout(base_timeout, async {
                                if let Ok(success) = crate::features::solvers::cf_handle(
                                    &mut res, page, target_url, viewport,
                                )
                                .await
                                {
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
                    } else if anti_bot_tech == AntiBotTech::Imperva {
                        if crate::features::solvers::looks_like_imperva_verify(res.len(), &res) {
                            if let Err(_e) = tokio::time::timeout(base_timeout, async {
                                if let Ok(success) = crate::features::solvers::imperva_handle(
                                    &mut res, page, target_url, viewport,
                                )
                                .await
                                {
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
                    } else if crate::features::solvers::detect_recaptcha(&res) {
                        if let Err(_e) = tokio::time::timeout(base_timeout, async {
                            if let Ok(solved) =
                                crate::features::solvers::recaptcha_handle(&mut res, page, viewport)
                                    .await
                            {
                                if solved {
                                    status_code = StatusCode::OK;
                                }
                            }
                        })
                        .await
                        {
                            validate_cf = true;
                        }
                    } else if crate::features::solvers::detect_geetest(&res) {
                        if let Err(_e) = tokio::time::timeout(base_timeout, async {
                            if let Ok(solved) =
                                crate::features::solvers::geetest_handle(&mut res, page, viewport)
                                    .await
                            {
                                if solved {
                                    status_code = StatusCode::OK;
                                }
                            }
                        })
                        .await
                        {
                            validate_cf = true;
                        }
                    } else if crate::features::solvers::detect_lemin(&res) {
                        if let Err(_e) = tokio::time::timeout(base_timeout, async {
                            if let Ok(solved) =
                                crate::features::solvers::lemin_handle(&mut res, page, viewport)
                                    .await
                            {
                                if solved {
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
            }

            let ok = !res.is_empty();

            #[cfg(feature = "real_browser")]
            if validate_cf
                && ok
                && !crate::features::solvers::detect_cf_turnstyle(&res)
                && status_code == StatusCode::FORBIDDEN
            {
                status_code = StatusCode::OK;
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

            let scope_url = if jar.is_some() {
                let scope_url = page_response
                    .final_url
                    .as_deref()
                    .filter(|u| !u.is_empty())
                    .or(url_target)
                    .unwrap_or(source_str);

                url::Url::parse(scope_url).ok()
            } else {
                None
            };

            let _ = tokio::time::timeout(
                base_timeout,
                set_page_response_cookies(&mut page_response, page, jar, scope_url.as_ref()),
            )
            .await;

            if openai_config.is_some() && !base_timeout.is_zero() {
                base_timeout = sub_duration(base_timeout_measurement, start_time.elapsed());

                let openai_request = run_openai_request(
                    match &url_target {
                        Some(ut) => ut,
                        _ => source_str,
                    },
                    page,
                    wait_for,
                    openai_config,
                    &mut page_response,
                    ok,
                );

                let _ = tokio::time::timeout(base_timeout, openai_request).await;
            }

            if remote_multimodal.is_some() && !base_timeout.is_zero() {
                use crate::features::automation::{
                    run_remote_multimodal_if_enabled, AutomationResultExt,
                };

                base_timeout = sub_duration(base_timeout_measurement, start_time.elapsed());

                // Use the user-configured automation timeout when available,
                // otherwise fall back to the remaining page-request budget.
                let automation_timeout = remote_multimodal
                    .as_ref()
                    .and_then(|mm| mm.automation_timeout())
                    .unwrap_or(base_timeout);

                let multi_modal_request = run_remote_multimodal_if_enabled(
                    remote_multimodal,
                    page,
                    match &url_target {
                        Some(ut) => ut,
                        _ => source_str,
                    },
                );

                let multimodal_success =
                    match tokio::time::timeout(automation_timeout, multi_modal_request).await {
                        Ok(Ok(Some(result))) => {
                            let success = result.success;

                            // Store usage on page_response
                            match page_response.remote_multimodal_usage.as_mut() {
                                Some(v) => v.push(result.usage.clone()),
                                None => {
                                    page_response.remote_multimodal_usage =
                                        Some(vec![result.usage.clone()])
                                }
                            }

                            // Store extracted data if available
                            if result.extracted.is_some() || result.screenshot.is_some() {
                                let automation_result = result.to_automation_results();
                                match page_response.extra_remote_multimodal_data.as_mut() {
                                    Some(v) => v.push(automation_result),
                                    None => {
                                        page_response.extra_remote_multimodal_data =
                                            Some(vec![automation_result])
                                    }
                                }
                            }

                            // Store spawn_pages for the caller to process with browser access
                            if !result.spawn_pages.is_empty() {
                                match page_response.spawn_pages.as_mut() {
                                    Some(v) => v.extend(result.spawn_pages.clone()),
                                    None => {
                                        page_response.spawn_pages = Some(result.spawn_pages.clone())
                                    }
                                }
                            }

                            success
                        }
                        Ok(Ok(None)) => false,
                        Ok(Err(_e)) => {
                            log::warn!("Remote multimodal automation error: {:?}", _e);
                            false
                        }
                        Err(_elapsed) => {
                            log::warn!("Remote multimodal automation timed out");
                            false
                        }
                    };

                if multimodal_success {
                    let next_content = tokio::time::timeout(base_timeout, page.outer_html_bytes())
                        .await
                        .ok()
                        .and_then(Result::ok)
                        .filter(|b| !b.is_empty());

                    if next_content.is_some() {
                        page_response.content = next_content;
                    }
                }
            }

            if cfg!(feature = "chrome_screenshot") || screenshot.is_some() {
                let _ = tokio::time::timeout(
                    base_timeout + tokio::time::Duration::from_secs(30),
                    perform_screenshot(source_str, page, screenshot, &mut page_response),
                )
                .await;
            }

            if metadata.is_some() {
                let mut default_metadata = Metadata::default();
                default_metadata.automation = metadata;
                page_response.metadata = Some(Box::new(default_metadata));
            }

            page_response
        } else {
            // Apply wait_for config (e.g. idle_network0) before HTML extraction.
            // The run_events branch already calls page_wait; this ensures the
            // non-content / empty-response path also honors the config, matching
            // smart mode behavior.
            if wait_for.is_some() && !block_bytes && !base_timeout.is_zero() {
                let idle_timeout = base_timeout.min(Duration::from_secs(15));
                #[cfg(feature = "wait_guard")]
                let idle_timeout = crate::utils::wait_guard::global_wait_guard()
                    .adjusted_timeout(get_domain_from_url(source_str), idle_timeout);
                if let Err(elapsed) =
                    tokio::time::timeout(idle_timeout, page_wait(page, wait_for)).await
                {
                    log::warn!("chrome wait_for timeout {elapsed}");
                }
                base_timeout = sub_duration(base_timeout_measurement, start_time.elapsed());
            }

            let res = if !block_bytes {
                let results = tokio::time::timeout(
                    base_timeout.max(HALF_MAX_PAGE_TIMEOUT),
                    page.outer_html_bytes(),
                );

                match results.await {
                    Ok(v) => v.unwrap_or_default(),
                    _ => Default::default(),
                }
            } else {
                Default::default()
            };

            let mut page_response = set_page_response(!res.is_empty(), res, status_code, final_url);

            if !block_bytes {
                let scope_url = if jar.is_some() {
                    let scope_url = page_response
                        .final_url
                        .as_deref()
                        .filter(|u| !u.is_empty())
                        .or(url_target)
                        .unwrap_or(source_str);

                    url::Url::parse(scope_url).ok()
                } else {
                    None
                };

                let _ = tokio::time::timeout(
                    base_timeout,
                    set_page_response_cookies(&mut page_response, page, jar, scope_url.as_ref()),
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
                if final_url.starts_with("about:blank") {
                    page_response.final_url = None;
                }
            }
        }

        page_response
    };

    let mut content: Option<Vec<u8>> = None;

    let page_response = match rx1 {
        Some(rx1) => {
            tokio::select! {
                v = tokio::time::timeout(base_timeout, run_page_response) => {
                    v.map_err(|_| CdpError::Timeout)
                }
                c = rx1 => {
                    if let Ok(c) = c {
                        if let Some(c) = c {
                            let params = GetResponseBodyParams::new(c);

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

                              if !media_file.is_empty() {
                                  content = Some(media_file);
                              }
                          }
                        }
                    }

            let mut page_response = PageResponse::default();

            let scope_url = if jar.is_some() {
            let scope_url = page_response
                .final_url
                .as_deref()
                .filter(|u| !u.is_empty())
                .or(url_target)
                .unwrap_or(source_str);

              url::Url::parse(scope_url).ok()
            } else {
                None
            };

                let _ = tokio::time::timeout(
                    base_timeout,
                    set_page_response_cookies(&mut page_response, page, jar, scope_url.as_ref()),
                )
                .await;

                    if let Some(mut chrome_http_req_res1) = chrome_http_req_res1 {
                        set_page_response_headers(&mut chrome_http_req_res1, &mut page_response);

                        page_response.status_code = chrome_http_req_res1.status_code;
                        page_response.waf_check = chrome_http_req_res1.waf_check;

                        #[cfg(feature = "cache_request")]
                        if !page_set && cache_request {
                            let _ = tokio::time::timeout(
                                base_timeout,
                                cache_chrome_response(source_str, &page_response, chrome_http_req_res1, &cache_options, cache_namespace),
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
    page_response.content = match content {
        Some(c) if !c.is_empty() => Some(c),
        _ => {
            let needs_fill = page_response.content.as_ref().is_none_or(|b| b.is_empty());

            if needs_fill {
                tokio::time::timeout(base_timeout, page.outer_html_bytes())
                    .await
                    .ok()
                    .and_then(Result::ok)
                    .filter(|b| !b.is_empty())
            } else {
                page_response.content
            }
        }
    };
    if page_response.status_code == *UNKNOWN_STATUS_ERROR && page_response.content.is_some() {
        page_response.status_code = StatusCode::OK;
    }
    // If the request was cancelled (timeout or net::ERR_ABORTED) and we ended
    // up with no usable content, upgrade to a retryable 504 so the outer crawl
    // loop will retry instead of silently accepting an empty page.
    if request_cancelled && page_response.content.is_none() {
        page_response.status_code = StatusCode::GATEWAY_TIMEOUT;
    }
    // If Chrome reported success (200) but produced no content, downgrade to
    // a retryable 504 so callers can distinguish real success from empty
    // responses and trigger retries appropriately.
    else if page_response.status_code == StatusCode::OK && page_response.content.is_none() {
        page_response.status_code = StatusCode::GATEWAY_TIMEOUT;
    }

    // Track bad wait-for outcomes per domain so future requests get reduced
    // timeouts, preventing one user's antibot-heavy crawl from hogging pages.
    #[cfg(feature = "wait_guard")]
    if wait_for.is_some() {
        let domain = get_domain_from_url(source_str);
        let guard = crate::utils::wait_guard::global_wait_guard();
        let no_useful_content = page_response
            .content
            .as_ref()
            .is_none_or(|b| b.is_empty() || is_cacheable_body_empty(b));
        let bad = match page_response.status_code.as_u16() {
            // Blocked / bot-detected / rate-limited: if antibot was
            // detected (waf_check) the response is challenge HTML —
            // not useful output regardless of size. Also flag when
            // content is genuinely empty.
            403 | 429 | 503 | 520..=530 => page_response.waf_check || no_useful_content,
            // Timeout with nothing to show.
            408 | 504 => page_response.content.is_none(),
            _ => false,
        };
        if bad {
            guard.record_bad(domain);
        } else {
            guard.record_good(domain);
        }
    }

    // run initial handling hidden anchors
    // if let Ok(new_links) = page.evaluate(crate::features::chrome::ANCHOR_EVENTS).await {
    //     if let Ok(results) = new_links.into_value::<hashbrown::HashSet<CaseInsensitiveString>>() {
    //         links.extend(page.extract_links_raw(&base, &results).await);
    //     }
    // }

    #[cfg(feature = "cache_request")]
    let mut modified_cache = false;

    #[cfg(feature = "cache_request")]
    if cache_request {
        if let Some(mut main_rx) = main_rx {
            if let Ok(doc_req_id) = &main_rx.try_recv() {
                let cache_url = match &page_response.final_url {
                    Some(final_url) if !final_url.is_empty() => final_url.as_str(),
                    _ => target_url,
                };

                match page
                    .execute(GetResponseBodyParams::new(doc_req_id.clone()))
                    .await
                {
                    Ok(body_result) => {
                        let raw_body: Vec<u8> = if body_result.base64_encoded {
                            chromiumoxide::utils::base64::decode(&body_result.body)
                                .unwrap_or_default()
                        } else {
                            body_result.body.clone().into_bytes()
                        };

                        if !raw_body.is_empty() {
                            let _ = tokio::time::timeout(
                                base_timeout,
                                cache_chrome_response_from_cdp_body(
                                    cache_url,
                                    &raw_body,
                                    &chrome_http_req_res,
                                    &cache_options,
                                    cache_namespace,
                                ),
                            )
                            .await;
                            modified_cache = true;
                        }
                    }
                    Err(e) => {
                        log::debug!("{:?}", e)
                    }
                }
            }
        }
    }

    if cfg!(not(feature = "chrome_store_page")) {
        let _ = page
            .send_command(chromiumoxide::cdp::browser_protocol::page::CloseParams::default())
            .await;

        // Signal CDP event listeners to exit now that the page is closed,
        // then give a brief grace period for final metric flush.
        let _ = shutdown_tx.send(true);

        let collect_timeout = base_timeout.min(Duration::from_secs(30));
        let collected = tokio::select! {
            result = bytes_collected_handle => Ok(result),
            _ = tokio::time::sleep(collect_timeout) => {
                Err(())
            }
        };

        if let Ok(Ok((mut transferred, bytes_map, mut rs, request_map))) = collected {
            let response_map = rs.response_map;

            if response_map.is_some() {
                // Cap to bound pre-allocation against pages with excessive subresources.
                let mut _response_map =
                    HashMap::with_capacity(response_map.as_ref().map_or(0, |r| r.len().min(1024)));

                if let Some(response_map) = response_map {
                    if let Some(bytes_map) = bytes_map {
                        let detect_anti_bots =
                            response_map.len() <= 4 && anti_bot_tech == AntiBotTech::None;

                        for item in response_map {
                            if detect_anti_bots && item.1.url.starts_with("/_Incapsula_Resource?") {
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

                if let Some(status) = rs
                    .status_code
                    .and_then(|s| s.try_into().ok())
                    .and_then(|u: u16| StatusCode::from_u16(u).ok())
                {
                    page_response.status_code = status;
                }

                set_page_response_headers_raw(&mut rs.headers, &mut page_response);
                store_headers(&page_response, &mut chrome_http_req_res);

                if anti_bot_tech == AntiBotTech::None {
                    let final_url = match &page_response.final_url {
                        Some(final_url)
                            if !final_url.is_empty()
                                && !final_url.starts_with("about:blank")
                                && !final_url.starts_with("chrome-error://chromewebdata") =>
                        {
                            final_url
                        }
                        _ => target_url,
                    };
                    if let Some(h) = &page_response.headers {
                        if let Some(content) = &page_response.content {
                            anti_bot_tech = detect_anti_bot_tech_response(
                                final_url,
                                &HeaderSource::HeaderMap(h),
                                content,
                                None,
                            );
                        }
                    }
                }

                #[cfg(feature = "real_browser")]
                if let Some(content) = &page_response.content {
                    // validate if the turnstile page is still open.
                    if anti_bot_tech == AntiBotTech::Cloudflare
                        && page_response.status_code == StatusCode::FORBIDDEN
                    {
                        let cf_turnstile = crate::features::solvers::detect_cf_turnstyle(content);

                        if !cf_turnstile {
                            page_response.status_code = StatusCode::OK;
                        }
                    }
                }
                #[cfg(feature = "cache_request")]
                if cache_request && !page_set && !rs.main_doc_from_cache && !modified_cache {
                    let _ = tokio::time::timeout(
                        base_timeout,
                        cache_chrome_response(
                            source_str,
                            &page_response,
                            chrome_http_req_res,
                            &cache_options,
                            cache_namespace,
                        ),
                    )
                    .await;
                }
            }
            if request_map.is_some() {
                page_response.request_map = request_map;
            }

            // Set remote address from CDP Network.responseReceived for parity
            // with HTTP-mode remote_addr.
            #[cfg(feature = "remote_addr")]
            if let Some(ref ip_str) = rs.remote_ip_address {
                let port = rs.remote_port.unwrap_or(0) as u16;
                if let Ok(ip) = ip_str.parse::<core::net::IpAddr>() {
                    page_response.remote_addr = Some(core::net::SocketAddr::new(ip, port));
                }
            }

            page_response.bytes_transferred = Some(transferred);
        }
    }

    page_response.anti_bot_tech = anti_bot_tech;

    set_page_response_duration(&mut page_response, duration);

    Ok(page_response)
}

#[cfg(feature = "time")]
/// Set the duration of time took for the page.
pub(crate) fn set_page_response_duration(
    page_response: &mut PageResponse,
    duration: Option<tokio::time::Instant>,
) {
    page_response.duration = duration;
}

#[cfg(not(feature = "time"))]
/// Set the duration of time took for the page.
pub(crate) fn set_page_response_duration(
    _page_response: &mut PageResponse,
    _duration: Option<tokio::time::Instant>,
) {
}

/// Set the page response.
#[cfg(feature = "chrome")]
fn set_page_response(
    ok: bool,
    res: Vec<u8>,
    status_code: StatusCode,
    final_url: Option<String>,
) -> PageResponse {
    PageResponse {
        content: if ok { Some(res) } else { None },
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

#[cfg(all(feature = "chrome", feature = "cookies"))]
async fn set_page_response_cookies(
    page_response: &mut PageResponse,
    page: &chromiumoxide::Page,
    jar: Option<&std::sync::Arc<crate::client::cookie::Jar>>,
    scope_url: Option<&url::Url>,
) {
    if let Ok(mut cookies) = page.get_cookies().await {
        // Cap to bound pre-allocation against malicious pages setting many cookies.
        let mut cookies_map: std::collections::HashMap<String, String> =
            std::collections::HashMap::with_capacity(cookies.len().min(256));

        for cookie in cookies.drain(..) {
            if let Some(scope_url) = scope_url {
                if let Some(jar) = jar {
                    let sc = format!("{}={}; Path=/", cookie.name, cookie.value);
                    jar.add_cookie_str(&sc, scope_url);
                }
            }
            cookies_map.insert(cookie.name, cookie.value);
        }

        let response_headers = convert_headers(&cookies_map);
        if !response_headers.is_empty() {
            page_response.cookies = Some(response_headers);
        }
    }
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

    match &screenshot {
        Some(ss) => {
            let output_format = string_concat!(
                ".",
                ss.params
                    .cdp_params
                    .format
                    .as_ref()
                    .unwrap_or(&crate::configuration::CaptureScreenshotFormat::Png)
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
                let _ = page.send_command(chromiumoxide::cdp::browser_protocol::emulation::SetDefaultBackgroundColorOverrideParams {
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
                Ok(b) => {
                    if let Ok(b) = STANDARD.decode(&b.data) {
                        if ss.save {
                            let output_path = create_output_path(
                                &ss.output_dir.clone().unwrap_or_else(|| "./storage/".into()),
                                target_url,
                                &output_format,
                            )
                            .await;
                            let _ = uring_fs::write_file(output_path, b.to_vec()).await;
                        }
                        if ss.bytes {
                            page_response.screenshot_bytes = Some(b);
                        }
                    }
                }
                Err(e) => {
                    log::error!("failed to take screenshot: {:?} - {:?}", e, target_url)
                }
            };

            if omit_background {
                let _ = page.send_command(chromiumoxide::cdp::browser_protocol::emulation::SetDefaultBackgroundColorOverrideParams { color: None })
                        .await;
            }
        }
        _ => {
            let output_path = create_output_path(
                &std::env::var("SCREENSHOT_DIRECTORY")
                    .unwrap_or_else(|_| "./storage/".to_string())
                    .into(),
                target_url,
                ".png",
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
    use std::str::FromStr;

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
pub fn get_cookies(_res: &Response) -> Option<crate::client::header::HeaderMap> {
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
    #[cfg(feature = "cookies")]
    let cookies = get_cookies(&res);

    let mut content: Option<Vec<u8>> = None;
    let mut anti_bot_tech = AntiBotTech::default();

    let limit = *MAX_SIZE_BYTES;

    if limit > 0 {
        let base = res
            .content_length()
            .and_then(|n| usize::try_from(n).ok())
            .unwrap_or(0);

        let hdr = res
            .headers()
            .get(CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(0);

        let current_size = base + hdr.saturating_sub(base);

        if current_size > limit {
            anti_bot_tech = detect_anti_bot_tech_response(
                target_url,
                &HeaderSource::HeaderMap(&headers),
                &[],
                None,
            );
            return PageResponse {
                headers: Some(headers),
                #[cfg(feature = "remote_addr")]
                remote_addr,
                #[cfg(feature = "cookies")]
                cookies,
                content: None,
                final_url: rd,
                status_code,
                anti_bot_tech,
                ..Default::default()
            };
        }
    }

    let mut content_truncated = false;

    if !block_streaming(&res, only_html) {
        let expected_len = res.content_length();
        let mut data = match expected_len {
            Some(cap) if cap > MAX_CONTENT_LENGTH => {
                log::warn!("{target_url} Content-Length {cap} exceeds 2 GB limit, rejecting");
                return PageResponse {
                    headers: Some(headers),
                    #[cfg(feature = "remote_addr")]
                    remote_addr,
                    #[cfg(feature = "cookies")]
                    cookies,
                    content: None,
                    final_url: rd,
                    status_code,
                    anti_bot_tech,
                    ..Default::default()
                };
            }
            Some(cap) if cap > 0 => Vec::with_capacity((cap as usize).min(MAX_PREALLOC)),
            _ => Vec::with_capacity(MAX_PRE_ALLOCATED_HTML_PAGE_SIZE_USIZE),
        };
        let mut stream = res.bytes_stream();
        let mut first_bytes = true;
        let chunk_idle_timeout = chunk_idle_timeout();

        loop {
            let next_chunk = async { stream.next().await };

            let item = match chunk_idle_timeout {
                Some(timeout) => match tokio::time::timeout(timeout, next_chunk).await {
                    Ok(Some(item)) => item,
                    Ok(None) => break,
                    Err(_elapsed) => {
                        log::warn!(
                            "chunk idle timeout ({timeout:?}) for {target_url}, returning {} bytes of partial content",
                            data.len()
                        );
                        content_truncated = true;
                        break;
                    }
                },
                None => match next_chunk.await {
                    Some(item) => item,
                    None => break,
                },
            };

            match item {
                Ok(text) => {
                    if only_html && first_bytes {
                        first_bytes = false;
                        if is_binary_file(&text) {
                            break;
                        }
                    }

                    if limit > 0 && data.len() + text.len() > limit {
                        content_truncated = true;
                        break;
                    }

                    data.extend_from_slice(&text)
                }
                Err(e) => {
                    log::error!("{e} in {}", target_url);
                    content_truncated = true;
                    break;
                }
            }
        }

        // Detect Content-Length mismatch (upstream sent fewer bytes than promised)
        if !content_truncated {
            if let Some(expected) = expected_len {
                let received = data.len() as u64;
                if received < expected {
                    log::warn!(
                        "Content-Length mismatch for {target_url}: expected {expected} bytes, received {received} bytes",
                    );
                    content_truncated = true;
                }
            }
        }

        if content_truncated && data.is_empty() {
            log::warn!("discarding empty truncated response for {target_url}");
        } else {
            anti_bot_tech = detect_anti_bot_tech_response(
                target_url,
                &HeaderSource::HeaderMap(&headers),
                &data,
                None,
            );
            content.replace(data);
        }
    }

    PageResponse {
        headers: Some(headers),
        #[cfg(feature = "remote_addr")]
        remote_addr,
        #[cfg(feature = "cookies")]
        cookies,
        content,
        final_url: rd,
        status_code,
        anti_bot_tech,
        content_truncated,
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
    #[cfg(feature = "cookies")]
    let cookies = get_cookies(&res);
    let mut anti_bot_tech = AntiBotTech::default();

    let mut rewrite_error = false;
    let mut content_truncated = false;

    if !block_streaming(&res, only_html) {
        let expected_len = res.content_length();
        let mut stream = res.bytes_stream();
        let mut first_bytes = true;
        let mut data_len = 0usize;
        let chunk_idle_timeout = chunk_idle_timeout();

        loop {
            let next_chunk = async { stream.next().await };

            let item = match chunk_idle_timeout {
                Some(timeout) => match tokio::time::timeout(timeout, next_chunk).await {
                    Ok(Some(item)) => item,
                    Ok(None) => break,
                    Err(_elapsed) => {
                        log::warn!(
                            "chunk idle timeout ({timeout:?}) for {target_url}, returning {data_len} bytes of partial content",
                        );
                        content_truncated = true;
                        break;
                    }
                },
                None => match next_chunk.await {
                    Some(item) => item,
                    None => break,
                },
            };

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
                        content_truncated = true;
                        break;
                    }

                    data_len += bytes_len;

                    if !rewrite_error && rewriter.write(&res_bytes).is_err() {
                        rewrite_error = true;
                    }

                    collected_bytes.extend_from_slice(&res_bytes);
                }
                Err(e) => {
                    log::error!("{e} in {}", target_url);
                    content_truncated = true;
                    break;
                }
            }
        }

        // Detect Content-Length mismatch
        if !content_truncated {
            if let Some(expected) = expected_len {
                if (data_len as u64) < expected {
                    log::warn!(
                        "Content-Length mismatch for {target_url}: expected {expected} bytes, received {data_len} bytes",
                    );
                    content_truncated = true;
                }
            }
        }

        anti_bot_tech = detect_anti_bot_tech_response(
            target_url,
            &HeaderSource::HeaderMap(&headers),
            collected_bytes,
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
            content_truncated,
            ..Default::default()
        },
        rewrite_error,
    )
}

/// Continue to parse a valid web page.
pub(crate) fn valid_parsing_status(res: &Response) -> bool {
    res.status().is_success() || res.status() == 404
}

/// Build the error page response.
fn build_error_page_response(target_url: &str, err: RequestError) -> PageResponse {
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

#[inline]
/// Build a cached page response from HTML.
pub(crate) fn build_cached_html_page_response(target_url: &str, html: &str) -> PageResponse {
    PageResponse {
        content: Some(html.as_bytes().to_vec()),
        status_code: StatusCode::OK,
        final_url: Some(target_url.to_string()),
        ..Default::default()
    }
}

/// Error chain handshake failure.
fn error_chain_contains_handshake_failure(err: &RequestError) -> bool {
    if err.to_string().to_lowercase().contains("handshake failure") {
        return true;
    }
    let mut cur: Option<&(dyn std::error::Error + 'static)> = err.source();

    while let Some(e) = cur {
        let s = e.to_string().to_lowercase();
        if s.contains("handshake failure") {
            return true;
        }
        cur = e.source();
    }

    false
}

/// Perform a network request to a resource extracting all content streaming.
async fn fetch_page_html_raw_base(
    target_url: &str,
    client: &Client,
    only_html: bool,
) -> PageResponse {
    async fn attempt_once(
        url: &str,
        client: &Client,
        only_html: bool,
    ) -> Result<PageResponse, RequestError> {
        let res = client.get(url).send().await?;
        Ok(handle_response_bytes(res, url, only_html).await)
    }

    let duration = if cfg!(feature = "time") {
        Some(tokio::time::Instant::now())
    } else {
        None
    };

    let mut page_response = match attempt_once(target_url, client, only_html).await {
        Ok(pr) => {
            // Retry once if the response was truncated (stream error or Content-Length mismatch)
            if pr.content_truncated {
                log::warn!("Response truncated for {target_url}, retrying once");
                match attempt_once(target_url, client, only_html).await {
                    Ok(pr2) => pr2,
                    Err(_) => pr, // fall back to original truncated response
                }
            } else {
                pr
            }
        }
        Err(err) => {
            let should_retry = error_chain_contains_handshake_failure(&err);
            if should_retry {
                if let Some(flipped) = flip_http_https(target_url) {
                    log::info!(
                        "TLS handshake failure for {}; retrying with flipped scheme: {}",
                        target_url,
                        flipped
                    );
                    match attempt_once(&flipped, client, only_html).await {
                        Ok(pr2) => pr2,
                        Err(err2) => build_error_page_response(&flipped, err2),
                    }
                } else if let Some(no_www) = strip_www(target_url) {
                    log::info!(
                        "TLS handshake failure for {}; retrying without www: {}",
                        target_url,
                        no_www
                    );
                    match attempt_once(&no_www, client, only_html).await {
                        Ok(pr2) => pr2,
                        Err(err2) => build_error_page_response(&no_www, err2),
                    }
                } else {
                    build_error_page_response(target_url, err)
                }
            } else {
                build_error_page_response(target_url, err)
            }
        }
    };

    set_page_response_duration(&mut page_response, duration);
    page_response
}

/// Perform a network request to a resource extracting all content streaming.
pub async fn fetch_page_html_raw(target_url: &str, client: &Client) -> PageResponse {
    fetch_page_html_raw_base(target_url, client, false).await
}

#[cfg(feature = "etag_cache")]
/// Perform a conditional network request using cached ETag / Last-Modified validators.
///
/// If the server responds with 304 Not Modified, returns a page response with
/// the original cached status (200) and empty body. The caller should use the
/// previously cached content. The `PageResponse.status_code` is set to 304 so
/// the caller can detect the not-modified case.
///
/// Also stores any new ETag / Last-Modified headers from 200 responses.
pub async fn fetch_page_html_raw_conditional(
    target_url: &str,
    client: &Client,
    etag_cache: &crate::utils::etag_cache::ETagCache,
) -> PageResponse {
    use reqwest::header::{HeaderName, HeaderValue};

    let duration = if cfg!(feature = "time") {
        Some(tokio::time::Instant::now())
    } else {
        None
    };

    // Build the request with conditional headers.
    let mut req = client.get(target_url);

    let conditional_headers = etag_cache.conditional_headers(target_url);
    for (name, value) in &conditional_headers {
        if let (Ok(hn), Ok(hv)) = (
            HeaderName::from_bytes(name.as_bytes()),
            HeaderValue::from_str(value.as_str()),
        ) {
            req = req.header(hn, hv);
        }
    }

    let mut page_response = match req.send().await {
        Ok(res) => {
            if res.status() == StatusCode::NOT_MODIFIED {
                // 304 — content unchanged, no body to process.
                PageResponse {
                    status_code: StatusCode::NOT_MODIFIED,
                    final_url: Some(target_url.to_string()),
                    ..Default::default()
                }
            } else {
                // Store validators from this response for future conditional requests.
                let etag = res
                    .headers()
                    .get("etag")
                    .and_then(|v| v.to_str().ok())
                    .map(|s| s.to_string());
                let last_modified = res
                    .headers()
                    .get("last-modified")
                    .and_then(|v| v.to_str().ok())
                    .map(|s| s.to_string());

                etag_cache.store(target_url, etag.as_deref(), last_modified.as_deref());

                handle_response_bytes(res, target_url, false).await
            }
        }
        Err(err) => build_error_page_response(target_url, err),
    };

    set_page_response_duration(&mut page_response, duration);
    page_response
}

/// Perform a network request to a resource and return a cached response immediately when available.
pub async fn fetch_page_html_raw_cached(
    target_url: &str,
    client: &Client,
    cache_options: Option<CacheOptions>,
    cache_policy: &Option<BasicCachePolicy>,
    cache_namespace: Option<&str>,
) -> PageResponse {
    let duration = if cfg!(feature = "time") {
        Some(tokio::time::Instant::now())
    } else {
        None
    };

    if let Some(cached_html) = get_cached_url(
        target_url,
        cache_options.as_ref(),
        cache_policy,
        cache_namespace,
    )
    .await
    {
        let mut response = build_cached_html_page_response(target_url, &cached_html);
        set_page_response_duration(&mut response, duration);
        return response;
    }

    fetch_page_html_raw_base(target_url, client, false).await
}

/// Perform a network request to a resource extracting all content streaming.
pub async fn fetch_page_html_raw_only_html(target_url: &str, client: &Client) -> PageResponse {
    fetch_page_html_raw_base(target_url, client, false).await
}

/// Fetch a single page via the spider.cloud REST API.
///
/// Response shape (all routes): `[{"content","costs","duration_elapsed_ms","error","metadata","status","url"}]`
///
/// Route selection via [`SpiderCloudConfig::fallback_route`]:
/// - `Smart` / `Unblocker` → `POST /unblocker`
/// - `Api` / `Fallback` / `Proxy` → `POST /crawl` (with `limit: 1`)
#[cfg(feature = "spider_cloud")]
pub async fn fetch_page_html_spider_cloud(
    target_url: &str,
    config: &crate::configuration::SpiderCloudConfig,
    client: &Client,
) -> PageResponse {
    let route = config.fallback_route();

    let multi = config.has_multiple_formats();

    let mut body = if multi {
        let mut formats: Vec<&str> =
            Vec::with_capacity(config.return_formats.as_ref().map_or(0, |v| v.len()));
        if let Some(fmts) = config.return_formats.as_ref() {
            for f in fmts {
                let s = f.as_str();
                if !formats.contains(&s) {
                    formats.push(s);
                }
            }
        }
        serde_json::json!({
            "url": target_url,
            "return_format": formats,
        })
    } else {
        serde_json::json!({
            "url": target_url,
            "return_format": config.return_format.as_str(),
        })
    };

    // /crawl needs limit: 1 to fetch a single page
    if route == "crawl" {
        body["limit"] = serde_json::json!(1);
    }

    // Merge extra_params into the body
    if let Some(ref extra) = config.extra_params {
        if let serde_json::Value::Object(ref mut map) = body {
            for (k, v) in extra {
                map.insert(k.clone(), v.clone());
            }
        }
    }

    let api_endpoint = format!("{}/{}", config.api_url, route);

    let result = client
        .post(&api_endpoint)
        .header("Authorization", format!("Bearer {}", config.api_key))
        .header("Content-Type", "application/json")
        .header("User-Agent", concat!("spider/", env!("CARGO_PKG_VERSION")))
        .body(match serde_json::to_vec(&body) {
            Ok(payload) => payload,
            Err(_) => {
                return PageResponse {
                    status_code: StatusCode::BAD_REQUEST,
                    ..Default::default()
                };
            }
        })
        .send()
        .await;

    match result {
        Ok(resp) => {
            let status = resp.status();
            match resp.bytes().await {
                Ok(bytes) => {
                    // spider.cloud returns: [{"content","costs","duration_elapsed_ms","error","metadata","status","url"}]
                    if let Ok(arr) = serde_json::from_slice::<Vec<serde_json::Value>>(&bytes) {
                        if let Some(first) = arr.into_iter().next() {
                            // Check for API-level error
                            if let Some(err) = first.get("error").and_then(|v| v.as_str()) {
                                log::warn!("spider.cloud error for {}: {}", target_url, err);
                            }

                            let item_status =
                                first.get("status").and_then(|v| v.as_u64()).unwrap_or(200) as u16;

                            let final_url = first
                                .get("url")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string());

                            // Multi-format: content is an object {"markdown": "...", "raw": "..."}
                            // Single format: content is a string
                            let content_val = first.get("content");

                            let (primary_content, content_map) = if multi {
                                if let Some(serde_json::Value::Object(obj)) = content_val {
                                    let primary_key = config.return_format.as_str();
                                    let primary = obj
                                        .get(primary_key)
                                        .and_then(|v| v.as_str())
                                        .unwrap_or_default()
                                        .to_string();

                                    let mut map = hashbrown::HashMap::with_capacity(obj.len());
                                    for (k, v) in obj {
                                        if let Some(s) = v.as_str() {
                                            map.insert(
                                                k.clone(),
                                                bytes::Bytes::from(s.as_bytes().to_vec()),
                                            );
                                        }
                                    }
                                    (primary, Some(map))
                                } else {
                                    // Fallback: API returned string even with multi-format
                                    let s = content_val
                                        .and_then(|v| v.as_str())
                                        .unwrap_or_default()
                                        .to_string();
                                    (s, None)
                                }
                            } else {
                                let s = content_val
                                    .and_then(|v| v.as_str())
                                    .unwrap_or_default()
                                    .to_string();
                                (s, None)
                            };

                            return PageResponse {
                                content: Some(primary_content.into_bytes()),
                                content_map,
                                status_code: StatusCode::from_u16(item_status)
                                    .unwrap_or(StatusCode::OK),
                                final_url,
                                ..Default::default()
                            };
                        }
                    }

                    // Fallback: treat entire body as content
                    PageResponse {
                        content: Some(bytes.to_vec()),
                        status_code: status,
                        ..Default::default()
                    }
                }
                Err(_) => PageResponse {
                    status_code: status,
                    ..Default::default()
                },
            }
        }
        Err(_) => PageResponse {
            status_code: StatusCode::BAD_GATEWAY,
            ..Default::default()
        },
    }
}

/// Fetch with spider.cloud fallback.
///
/// Tries a direct fetch first. Uses [`SpiderCloudConfig::should_fallback`] to
/// intelligently detect when to retry via the spider.cloud API — checking status
/// codes, bot protection markers, CAPTCHA challenges, and empty responses.
/// The fallback route (`/crawl` or `/unblocker`) is chosen by [`SpiderCloudConfig::fallback_route`].
#[cfg(feature = "spider_cloud")]
pub async fn fetch_page_html_with_fallback(
    target_url: &str,
    client: &Client,
    spider_cloud: &crate::configuration::SpiderCloudConfig,
    only_html: bool,
) -> PageResponse {
    let resp = fetch_page_html_raw_base(target_url, client, only_html).await;

    let body_bytes = resp.content.as_deref();
    let should_fallback = spider_cloud.should_fallback(resp.status_code.as_u16(), body_bytes);

    if should_fallback {
        log::info!(
            "spider_cloud fallback triggered for {} (status {})",
            target_url,
            resp.status_code
        );
        fetch_page_html_spider_cloud(target_url, spider_cloud, client).await
    } else {
        resp
    }
}

/// Perform a network request to a resource extracting all content as text.
#[cfg(feature = "decentralized")]
pub async fn fetch_page(target_url: &str, client: &Client) -> Option<Vec<u8>> {
    match client.get(target_url).send().await {
        Ok(res) if valid_parsing_status(&res) => match res.bytes().await {
            Ok(text) => Some(text.into()),
            Err(_) => {
                log("- error fetching {}", target_url);
                None
            }
        },
        Ok(_) => None,
        Err(_) => {
            log("- error parsing html bytes {}", target_url);
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
                Ok(text) => Some(text.to_vec()),
                Err(_) => {
                    log("- error fetching {}", target_url);
                    None
                }
            };
            FetchPageResult::Success(headers, b)
        }
        Ok(res) => FetchPageResult::NoSuccess(res.headers().clone()),
        Err(_) => {
            log("- error parsing html bytes {}", target_url);
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
    use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};

    let duration = if cfg!(feature = "time") {
        Some(tokio::time::Instant::now())
    } else {
        None
    };

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
            let mut writer: Option<uring_fs::StreamingWriter> = None;
            let mut file_path = String::new();

            while let Some(item) = stream.next().await {
                match item {
                    Ok(text) => {
                        let wrote_disk = writer.is_some();

                        // perform operations entire in memory to build resource
                        if !wrote_disk && data.capacity() < 8192 {
                            data.extend_from_slice(&text);
                        } else {
                            if !wrote_disk {
                                file_path = string_concat!(
                                    TMP_DIR,
                                    &utf8_percent_encode(target_url, NON_ALPHANUMERIC).to_string()
                                );
                                match uring_fs::StreamingWriter::create(file_path.clone()).await {
                                    Ok(w) => {
                                        data.extend_from_slice(&text);

                                        if w.write(data.as_ref()).await.is_ok() {
                                            data.clear();
                                        }
                                        writer = Some(w);
                                    }
                                    _ => data.extend_from_slice(&text),
                                };
                            } else {
                                if let Some(w) = writer.as_ref() {
                                    if let Err(_) = w.write(&text).await {
                                        data.extend_from_slice(&text)
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

            if let Some(w) = writer.take() {
                let _ = w.close().await;
            }

            PageResponse {
                #[cfg(feature = "time")]
                duration,
                #[cfg(feature = "headers")]
                headers: Some(headers),
                #[cfg(feature = "remote_addr")]
                remote_addr,
                #[cfg(feature = "cookies")]
                cookies,
                content: Some(if !file_path.is_empty() {
                    let buffer = if let Ok(b) = uring_fs::read_file(file_path.clone()).await {
                        let _ = uring_fs::remove_file(file_path).await;
                        b
                    } else {
                        vec![]
                    };

                    buffer
                } else {
                    data
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
                #[cfg(feature = "time")]
                duration,
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
    request_timeout: &Option<std::time::Duration>,
    track_events: &Option<crate::configuration::ChromeEventTracker>,
    referrer: Option<String>,
    max_page_bytes: Option<f64>,
    cache_options: Option<CacheOptions>,
    cache_policy: &Option<BasicCachePolicy>,
    #[cfg(feature = "cookies")] jar: Option<&std::sync::Arc<crate::client::cookie::Jar>>,
    cache_namespace: Option<&str>,
) -> PageResponse {
    use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};

    #[cfg(feature = "time")]
    let duration = Some(tokio::time::Instant::now());

    let skip_browser = cache_skip_browser(&cache_options);
    let cached_html = get_cached_url(
        target_url,
        cache_options.as_ref(),
        cache_policy,
        cache_namespace,
    )
    .await;
    let cached = cached_html.is_some();

    // Skip browser entirely if cached and skip_browser mode is enabled
    if skip_browser {
        if let Some(html) = cached_html {
            return PageResponse {
                content: Some(html.into_bytes()),
                status_code: StatusCode::OK,
                final_url: Some(target_url.to_string()),
                #[cfg(feature = "time")]
                duration,
                ..Default::default()
            };
        }
    }

    let mut page_response = match &page {
        page => {
            match fetch_page_html_chrome_base(
                if let Some(cached) = &cached_html {
                    cached.as_bytes()
                } else {
                    target_url.as_bytes()
                },
                page,
                cached,
                true,
                wait_for,
                screenshot,
                page_set,
                openai_config,
                if cached { Some(target_url) } else { None },
                execution_scripts,
                automation_scripts,
                viewport,
                request_timeout,
                track_events,
                referrer,
                max_page_bytes,
                cache_options,
                cache_policy,
                &None,
                &None,
                jar,
                &None,
                cache_namespace,
            )
            .await
            {
                Ok(page) => page,
                _ => {
                    log::info!(
                        "- error fetching chrome page defaulting to raw http request {}",
                        &target_url,
                    );

                    match client.get(target_url).send().await {
                        Ok(res) if valid_parsing_status(&res) => {
                            let headers = res.headers().clone();
                            let cookies = get_cookies(&res);
                            let status_code = res.status();
                            #[cfg(feature = "remote_addr")]
                            let remote_addr = res.remote_addr();
                            let mut stream = res.bytes_stream();
                            let mut data = Vec::new();

                            let mut writer: Option<uring_fs::StreamingWriter> = None;
                            let mut file_path = String::new();

                            while let Some(item) = stream.next().await {
                                match item {
                                    Ok(text) => {
                                        let wrote_disk = writer.is_some();

                                        // perform operations entire in memory to build resource
                                        if !wrote_disk && data.capacity() < 8192 {
                                            data.extend_from_slice(&text);
                                        } else if !wrote_disk {
                                            file_path = string_concat!(
                                                TMP_DIR,
                                                &utf8_percent_encode(target_url, NON_ALPHANUMERIC)
                                                    .to_string()
                                            );
                                            match uring_fs::StreamingWriter::create(
                                                file_path.clone(),
                                            )
                                            .await
                                            {
                                                Ok(w) => {
                                                    data.extend_from_slice(&text);

                                                    if w.write(data.as_ref()).await.is_ok() {
                                                        data.clear();
                                                    }
                                                    writer = Some(w);
                                                }
                                                _ => data.extend_from_slice(&text),
                                            };
                                        } else if let Some(w) = writer.as_ref() {
                                            if w.write(&text).await.is_ok() {
                                                data.extend_from_slice(&text)
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        log::error!("{e} in {}", target_url);
                                        break;
                                    }
                                }
                            }

                            if let Some(w) = writer.take() {
                                let _ = w.close().await;
                            }

                            PageResponse {
                                #[cfg(feature = "headers")]
                                headers: Some(headers),
                                #[cfg(feature = "remote_addr")]
                                remote_addr,
                                #[cfg(feature = "cookies")]
                                cookies,
                                content: Some(if !file_path.is_empty() {
                                    let buffer = if let Ok(b) =
                                        uring_fs::read_file(file_path.clone()).await
                                    {
                                        let _ = uring_fs::remove_file(file_path).await;
                                        b
                                    } else {
                                        vec![]
                                    };

                                    buffer
                                } else {
                                    data
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
    };
    set_page_response_duration(&mut page_response, duration);

    page_response
}

#[cfg(any(feature = "cache", feature = "cache_mem"))]
/// Create the cache key from string.
///
/// `namespace` is an opaque caller-supplied partition string so logically
/// distinct variants of the same URL (country, proxy pool, tenant, A/B bucket,
/// device profile, …) never collide on the same cached bytes. Passing `None`
/// produces the same key format as before this parameter existed, preserving
/// backward compatibility with existing cache stores.
pub fn create_cache_key_raw(
    uri: &str,
    override_method: Option<&str>,
    auth: Option<&str>,
    namespace: Option<&str>,
) -> String {
    let method = override_method.unwrap_or("GET");
    match (auth, namespace) {
        (Some(a), Some(ns)) => format!("{}:{}:{}:ns={}", method, uri, a, ns),
        (Some(a), None) => format!("{}:{}:{}", method, uri, a),
        (None, Some(ns)) => format!("{}:{}::ns={}", method, uri, ns),
        (None, None) => format!("{}:{}", method, uri),
    }
}

#[cfg(any(feature = "cache", feature = "cache_mem"))]
/// Create the cache key.
pub fn create_cache_key(
    parts: &http::request::Parts,
    override_method: Option<&str>,
    auth: Option<&str>,
    namespace: Option<&str>,
) -> String {
    create_cache_key_raw(
        &parts.uri.to_string(),
        Some(override_method.unwrap_or_else(|| parts.method.as_str())),
        auth,
        namespace,
    )
}

#[derive(Default, Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
/// Cache options to use for the request.
pub enum CacheOptions {
    /// Use cache without authentication.
    Yes,
    /// Use cache with authentication.
    Authorized(String),
    #[default]
    /// Do not use the memory cache.
    No,
    /// Skip browser entirely if cached response exists, return cached HTML directly.
    SkipBrowser,
    /// Skip browser with authentication token if cached response exists.
    SkipBrowserAuthorized(String),
}

#[inline]
/// Cache auth token.
pub fn cache_auth_token(cache_options: &std::option::Option<CacheOptions>) -> Option<&str> {
    cache_options.as_ref().and_then(|opt| match opt {
        CacheOptions::Authorized(token) | CacheOptions::SkipBrowserAuthorized(token) => {
            Some(token.as_str())
        }
        _ => None,
    })
}

#[inline]
/// Check if cache options indicate browser should be skipped when cached.
pub fn cache_skip_browser(cache_options: &std::option::Option<CacheOptions>) -> bool {
    matches!(
        cache_options,
        Some(CacheOptions::SkipBrowser) | Some(CacheOptions::SkipBrowserAuthorized(_))
    )
}

/// Basic cache policy.
#[derive(Debug, Default, Clone, Hash, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum BasicCachePolicy {
    /// Allow stale caches – responses may be used even if they *should* be revalidated.
    AllowStale,
    /// Use this `SystemTime` as the reference "now" for staleness checks.
    Period(std::time::SystemTime),
    #[default]
    /// Use the default system time.
    Normal,
}

#[cfg(feature = "chrome_remote_cache")]
impl BasicCachePolicy {
    /// Convert the cache policy to chrome.
    pub fn from_basic(&self) -> chromiumoxide::cache::BasicCachePolicy {
        match &self {
            BasicCachePolicy::AllowStale => chromiumoxide::cache::BasicCachePolicy::AllowStale,
            BasicCachePolicy::Normal => chromiumoxide::cache::BasicCachePolicy::Normal,
            BasicCachePolicy::Period(p) => chromiumoxide::cache::BasicCachePolicy::Period(*p),
        }
    }
}

#[cfg(any(
    feature = "cache",
    feature = "cache_mem",
    feature = "chrome_remote_cache"
))]
#[inline]
fn decode_cached_html_bytes(body: &[u8], accept_lang: Option<&str>) -> Option<String> {
    if is_binary_file(body) || is_cacheable_body_empty(body) {
        return None;
    }

    Some(match accept_lang {
        Some(lang) if !lang.is_empty() => auto_encoder::encode_bytes_from_language(body, lang),
        _ => auto_encoder::auto_encode_bytes(body),
    })
}

#[cfg(any(feature = "cache", feature = "cache_mem"))]
/// Perform a network request to a resource extracting all content as text streaming via chrome.
pub async fn get_cached_url_base(
    target_url: &str,
    cache_options: Option<CacheOptions>,
    cache_policy: &Option<BasicCachePolicy>, // optional override/behavior
    namespace: Option<&str>,
) -> Option<String> {
    use crate::http_cache_reqwest::CacheManager;

    let auth_opt = match cache_options {
        Some(CacheOptions::Yes) | Some(CacheOptions::SkipBrowser) => None,
        Some(CacheOptions::Authorized(token))
        | Some(CacheOptions::SkipBrowserAuthorized(token)) => Some(token),
        Some(CacheOptions::No) | None => return None,
    };

    // Override behavior:
    // - AllowStale: accept even stale entries
    // - Period(t): use t as "now" for staleness checks (entries stored after t appear fresh)
    // - Normal/None: use SystemTime::now()
    let allow_stale = matches!(cache_policy, Some(BasicCachePolicy::AllowStale));
    let now = match cache_policy {
        Some(BasicCachePolicy::Period(t)) => *t,
        _ => std::time::SystemTime::now(),
    };

    let auth_str = auth_opt.as_deref();

    // Helper: attempt CACACHE_MANAGER lookup for a given URL.
    let try_cacache = |url: &str| {
        let cache_url = create_cache_key_raw(url, None, auth_str, namespace);
        async move {
            let result = tokio::time::timeout(Duration::from_millis(60), async {
                crate::website::CACACHE_MANAGER.get(&cache_url).await
            })
            .await;
            if let Ok(Ok(Some((http_response, stored_policy)))) = result {
                if allow_stale || !stored_policy.is_stale(now) {
                    let http_cache::HttpHeaders::Modern(ref hdrs) = http_response.headers;
                    return decode_cached_html_bytes(
                        &http_response.body,
                        hdrs.get("accept-language").and_then(|vals| {
                            vals.first().and_then(|h| {
                                if h.is_empty() {
                                    None
                                } else {
                                    Some(h.as_str())
                                }
                            })
                        }),
                    );
                }
            }
            None
        }
    };

    if let Some(body) = try_cacache(target_url).await {
        return Some(body);
    }

    // Try alternate URL (with/without trailing slash) against local cache.
    let alt_url: Option<String> = if target_url.ends_with('/') {
        let trimmed = target_url.trim_end_matches('/');
        if trimmed.is_empty() || trimmed == target_url {
            None
        } else {
            Some(trimmed.to_string())
        }
    } else {
        let mut s = String::with_capacity(target_url.len() + 1);
        s.push_str(target_url);
        s.push('/');
        Some(s)
    };

    if let Some(ref alt) = alt_url {
        if let Some(body) = try_cacache(alt).await {
            return Some(body);
        }
    }

    // Fallback: query remote hybrid_cache_server when chrome_remote_cache is enabled.
    // The local CACACHE_MANAGER is in-memory (per-process), so it misses on first
    // request or after restart. The remote cache persists across processes and has
    // data populated by browser_server's CDP interception.
    #[cfg(feature = "chrome_remote_cache")]
    {
        let cache_site = chromiumoxide::cache::manager::site_key_for_target_url(
            target_url,
            auth_opt.as_deref(),
            namespace,
        );
        let make_session_key = |url: &str| format!("GET:{}", url);

        let try_session_get = |url: &str| {
            chromiumoxide::cache::remote::get_session_cache_item(
                &cache_site,
                &make_session_key(url),
            )
            .and_then(|(http_response, stored_policy)| {
                if allow_stale || !stored_policy.is_stale(now) {
                    let accept_lang = http_response
                        .headers
                        .get("accept-language")
                        .or_else(|| http_response.headers.get("Accept-Language"))
                        .map(|h| h.as_str());
                    decode_cached_html_bytes(&http_response.body, accept_lang)
                } else {
                    None
                }
            })
        };

        // Check chromiumoxide session cache (may have been seeded by a prior navigation).
        if let Some(body) = try_session_get(target_url) {
            return Some(body);
        }

        // Pull from the remote cache server, seed local session cache, then retry.
        // Timeout prevents blocking the critical path if the cache server is slow/down.
        let _ = tokio::time::timeout(
            Duration::from_secs(3),
            chromiumoxide::cache::remote::get_cache_site(
                target_url,
                auth_opt.as_deref(),
                Some("true"),
                namespace,
            ),
        )
        .await;

        if let Some(body) = try_session_get(target_url) {
            return Some(body);
        }

        // Reuse alt_url computed above for the CACACHE path.
        if let Some(ref alt) = alt_url {
            if let Some(body) = try_session_get(alt) {
                return Some(body);
            }
        }
    }

    None
}

#[cfg(all(
    feature = "chrome_remote_cache",
    not(any(feature = "cache", feature = "cache_mem"))
))]
/// Perform a network request to a resource extracting all content as text streaming via chrome.
pub async fn get_cached_url_base(
    target_url: &str,
    cache_options: Option<CacheOptions>,
    cache_policy: &Option<BasicCachePolicy>, // optional override/behavior
    namespace: Option<&str>,
) -> Option<String> {
    let auth_opt = match cache_options {
        Some(CacheOptions::Yes) | Some(CacheOptions::SkipBrowser) => None,
        Some(CacheOptions::Authorized(token))
        | Some(CacheOptions::SkipBrowserAuthorized(token)) => Some(token),
        Some(CacheOptions::No) | None => return None,
    };

    let allow_stale = matches!(cache_policy, Some(BasicCachePolicy::AllowStale));
    let now = match cache_policy {
        Some(BasicCachePolicy::Period(t)) => *t,
        _ => std::time::SystemTime::now(),
    };

    let cache_site = chromiumoxide::cache::manager::site_key_for_target_url(
        target_url,
        auth_opt.as_deref(),
        namespace,
    );
    let make_session_key = |url: &str| format!("GET:{}", url);

    let try_get = |url: &str| {
        chromiumoxide::cache::remote::get_session_cache_item(&cache_site, &make_session_key(url))
            .and_then(|(http_response, stored_policy)| {
                if allow_stale || !stored_policy.is_stale(now) {
                    let accept_lang = http_response
                        .headers
                        .get("accept-language")
                        .or_else(|| http_response.headers.get("Accept-Language"))
                        .map(|h| h.as_str());

                    decode_cached_html_bytes(&http_response.body, accept_lang)
                } else {
                    None
                }
            })
    };

    if let Some(body) = try_get(target_url) {
        return Some(body);
    }

    let alt_url: Option<String> = if target_url.ends_with('/') {
        let trimmed = target_url.trim_end_matches('/');
        if trimmed.is_empty() || trimmed == target_url {
            None
        } else {
            Some(trimmed.to_string())
        }
    } else {
        let mut s = String::with_capacity(target_url.len() + 1);
        s.push_str(target_url);
        s.push('/');
        Some(s)
    };

    if let Some(alt) = &alt_url {
        if let Some(body) = try_get(alt) {
            return Some(body);
        }
    }

    // Pull from the remote cache server once, then retry local session lookup.
    // Timeout prevents blocking the critical path if the cache server is slow/down.
    let _ = tokio::time::timeout(
        Duration::from_secs(3),
        chromiumoxide::cache::remote::get_cache_site(
            target_url,
            auth_opt.as_deref(),
            Some("true"),
            namespace,
        ),
    )
    .await;

    if let Some(body) = try_get(target_url) {
        return Some(body);
    }

    if let Some(alt) = alt_url {
        if let Some(body) = try_get(&alt) {
            return Some(body);
        }
    }

    None
}

#[cfg(any(
    feature = "cache",
    feature = "cache_mem",
    feature = "chrome_remote_cache"
))]
/// Perform a network request to a resource extracting all content as text streaming via chrome.
pub async fn get_cached_url(
    target_url: &str,
    cache_options: Option<&CacheOptions>,
    cache_policy: &Option<BasicCachePolicy>,
    namespace: Option<&str>,
) -> Option<String> {
    // get_cached_url_base already handles trailing-slash fallback internally
    // (both in the cache/cache_mem and chrome_remote_cache paths),
    // so no outer alt-URL retry is needed.
    get_cached_url_base(target_url, cache_options.cloned(), cache_policy, namespace).await
}

#[cfg(all(
    not(feature = "cache"),
    not(feature = "cache_mem"),
    not(feature = "chrome_remote_cache")
))]
/// Perform a network request to a resource extracting all content as text streaming via chrome.
pub async fn get_cached_url(
    _target_url: &str,
    _cache_options: Option<&CacheOptions>,
    _cache_policy: &Option<BasicCachePolicy>,
    _namespace: Option<&str>,
) -> Option<String> {
    None
}

#[cfg(all(not(feature = "fs"), feature = "chrome"))]
/// Perform a network request to a resource extracting all content as text streaming via chrome.
pub async fn fetch_page_html_base(
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
    request_timeout: &Option<std::time::Duration>,
    track_events: &Option<crate::configuration::ChromeEventTracker>,
    referrer: Option<String>,
    max_page_bytes: Option<f64>,
    cache_options: Option<CacheOptions>,
    cache_policy: &Option<BasicCachePolicy>,
    seeded_resource: Option<String>,
    jar: Option<&std::sync::Arc<crate::client::cookie::Jar>>,
    remote_multimodal: &Option<Box<RemoteMultimodalConfigs>>,
    cache_namespace: Option<&str>,
) -> PageResponse {
    let skip_browser = cache_skip_browser(&cache_options);
    let cached_html = if let Some(seeded) = seeded_resource {
        // Reject empty HTML shells from seeded resources too.
        if is_cacheable_body_empty(seeded.as_bytes()) {
            None
        } else {
            Some(seeded)
        }
    } else {
        get_cached_url(
            target_url,
            cache_options.as_ref(),
            cache_policy,
            cache_namespace,
        )
        .await
    };
    let cached = cached_html.is_some();

    // Skip browser entirely if cached and skip_browser mode is enabled
    if skip_browser {
        if let Some(html) = cached_html {
            return PageResponse {
                content: Some(html.into_bytes()),
                status_code: StatusCode::OK,
                final_url: Some(target_url.to_string()),
                ..Default::default()
            };
        }
    }

    match fetch_page_html_chrome_base(
        if let Some(cached) = &cached_html {
            cached.as_bytes()
        } else {
            target_url.as_bytes()
        },
        page,
        cached,
        true,
        wait_for,
        screenshot,
        page_set,
        openai_config,
        if cached { Some(target_url) } else { None },
        execution_scripts,
        automation_scripts,
        viewport,
        request_timeout,
        track_events,
        referrer,
        max_page_bytes,
        cache_options,
        cache_policy,
        &None,
        &None,
        jar,
        remote_multimodal,
        cache_namespace,
    )
    .await
    {
        Ok(page) => page,
        Err(err) => {
            log::error!("{:?}", err);
            fetch_page_html_raw(target_url, client).await
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
    request_timeout: &Option<std::time::Duration>,
    track_events: &Option<crate::configuration::ChromeEventTracker>,
    referrer: Option<String>,
    max_page_bytes: Option<f64>,
    cache_options: Option<CacheOptions>,
    cache_policy: &Option<BasicCachePolicy>,
    remote_multimodal: &Option<Box<RemoteMultimodalConfigs>>,
    cache_namespace: Option<&str>,
) -> PageResponse {
    fetch_page_html_base(
        target_url,
        client,
        page,
        wait_for,
        screenshot,
        page_set,
        openai_config,
        execution_scripts,
        automation_scripts,
        viewport,
        request_timeout,
        track_events,
        referrer,
        max_page_bytes,
        cache_options,
        cache_policy,
        None,
        None,
        remote_multimodal,
        cache_namespace,
    )
    .await
}

#[cfg(all(not(feature = "fs"), feature = "chrome"))]
/// Perform a network request to a resource extracting all content as text streaming via chrome.
pub async fn fetch_page_html_seeded(
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
    request_timeout: &Option<std::time::Duration>,
    track_events: &Option<crate::configuration::ChromeEventTracker>,
    referrer: Option<String>,
    max_page_bytes: Option<f64>,
    cache_options: Option<CacheOptions>,
    cache_policy: &Option<BasicCachePolicy>,
    seeded_resource: Option<String>,
    jar: Option<&std::sync::Arc<crate::client::cookie::Jar>>,
    remote_multimodal: &Option<Box<RemoteMultimodalConfigs>>,
    cache_namespace: Option<&str>,
) -> PageResponse {
    fetch_page_html_base(
        target_url,
        client,
        page,
        wait_for,
        screenshot,
        page_set,
        openai_config,
        execution_scripts,
        automation_scripts,
        viewport,
        request_timeout,
        track_events,
        referrer,
        max_page_bytes,
        cache_options,
        cache_policy,
        seeded_resource,
        jar,
        remote_multimodal,
        cache_namespace,
    )
    .await
}

#[cfg(feature = "chrome")]
/// Perform a network request to a resource extracting all content as text streaming via chrome.
async fn _fetch_page_html_chrome(
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
    request_timeout: &Option<std::time::Duration>,
    track_events: &Option<crate::configuration::ChromeEventTracker>,
    referrer: Option<String>,
    max_page_bytes: Option<f64>,
    cache_options: Option<CacheOptions>,
    cache_policy: &Option<BasicCachePolicy>,
    resource: Option<String>,
    jar: Option<&std::sync::Arc<crate::client::cookie::Jar>>,
    remote_multimodal: &Option<Box<RemoteMultimodalConfigs>>,
    cache_namespace: Option<&str>,
) -> PageResponse {
    let duration = if cfg!(feature = "time") {
        Some(tokio::time::Instant::now())
    } else {
        None
    };

    let skip_browser = cache_skip_browser(&cache_options);
    let cached_html = if resource.is_some() {
        resource
    } else {
        get_cached_url(
            target_url,
            cache_options.as_ref(),
            cache_policy,
            cache_namespace,
        )
        .await
    };

    if skip_browser {
        if let Some(html) = cached_html.as_deref() {
            let mut page_response = build_cached_html_page_response(target_url, html);
            set_page_response_duration(&mut page_response, duration);
            return page_response;
        }
    }

    let cached = cached_html.is_some();

    let mut page_response = match &page {
        page => {
            match fetch_page_html_chrome_base(
                if let Some(cached) = &cached_html {
                    cached.as_bytes()
                } else {
                    target_url.as_bytes()
                },
                page,
                cached,
                true,
                wait_for,
                screenshot,
                page_set,
                openai_config,
                if cached { Some(target_url) } else { None },
                execution_scripts,
                automation_scripts,
                viewport,
                request_timeout,
                track_events,
                referrer,
                max_page_bytes,
                cache_options,
                cache_policy,
                &None,
                &None,
                jar,
                remote_multimodal,
                cache_namespace,
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

                                        data.extend_from_slice(&text)
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
                                content: Some(data),
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
    };

    set_page_response_duration(&mut page_response, duration);

    page_response
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
    request_timeout: &Option<std::time::Duration>,
    track_events: &Option<crate::configuration::ChromeEventTracker>,
    referrer: Option<String>,
    max_page_bytes: Option<f64>,
    cache_options: Option<CacheOptions>,
    cache_policy: &Option<BasicCachePolicy>,
    jar: Option<&std::sync::Arc<crate::client::cookie::Jar>>,
    remote_multimodal: &Option<Box<RemoteMultimodalConfigs>>,
    cache_namespace: Option<&str>,
) -> PageResponse {
    _fetch_page_html_chrome(
        target_url,
        client,
        page,
        wait_for,
        screenshot,
        page_set,
        openai_config,
        execution_scripts,
        automation_scripts,
        viewport,
        request_timeout,
        track_events,
        referrer,
        max_page_bytes,
        cache_options,
        cache_policy,
        None,
        jar,
        remote_multimodal,
        cache_namespace,
    )
    .await
}

#[cfg(feature = "chrome")]
/// Perform a network request to a resource extracting all content as text streaming via chrome seeded.
pub async fn fetch_page_html_chrome_seeded(
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
    request_timeout: &Option<std::time::Duration>,
    track_events: &Option<crate::configuration::ChromeEventTracker>,
    referrer: Option<String>,
    max_page_bytes: Option<f64>,
    cache_options: Option<CacheOptions>,
    cache_policy: &Option<BasicCachePolicy>,
    resource: Option<String>,
    jar: Option<&std::sync::Arc<crate::client::cookie::Jar>>,
    remote_multimodal: &Option<Box<RemoteMultimodalConfigs>>,
    cache_namespace: Option<&str>,
) -> PageResponse {
    _fetch_page_html_chrome(
        target_url,
        client,
        page,
        wait_for,
        screenshot,
        page_set,
        openai_config,
        execution_scripts,
        automation_scripts,
        viewport,
        request_timeout,
        track_events,
        referrer,
        max_page_bytes,
        cache_options,
        cache_policy,
        resource,
        jar,
        remote_multimodal,
        cache_namespace,
    )
    .await
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
                async_openai::types::chat::CreateChatCompletionRequestArgs::default();
            let gpt_base = chat_completion_defaults
                .max_tokens(gpt_configs.max_tokens)
                .model(&gpt_configs.model);
            let gpt_base = match &gpt_configs.user {
                Some(user) => gpt_base.user(user),
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

            let core_bpe = tiktoken_rs::get_bpe_from_model(&gpt_configs.model).ok();

            let (tokens, prompt_tokens) = match &core_bpe {
                Some(core_bpe) => (
                    core_bpe.encode_with_special_tokens(&resource),
                    core_bpe.encode_with_special_tokens(prompt),
                ),
                _ => (
                    CORE_BPE_TOKEN_COUNT.encode_with_special_tokens(&resource),
                    CORE_BPE_TOKEN_COUNT.encode_with_special_tokens(prompt),
                ),
            };

            // // we can use the output count later to perform concurrent actions.
            let output_tokens_count = tokens.len() + prompt_tokens.len();

            let mut max_tokens = crate::features::openai::calculate_max_tokens(
                &gpt_configs.model,
                gpt_configs.max_tokens,
                &crate::features::openai::BROWSER_ACTIONS_SYSTEM_PROMPT_COMPLETION.clone(),
                &resource,
                prompt,
            );

            // we need to slim down the content to fit the window.
            let resource = if output_tokens_count > max_tokens {
                let r = clean_html(&resource);

                max_tokens = crate::features::openai::calculate_max_tokens(
                    &gpt_configs.model,
                    gpt_configs.max_tokens,
                    &crate::features::openai::BROWSER_ACTIONS_SYSTEM_PROMPT_COMPLETION.clone(),
                    &r,
                    prompt,
                );

                let (tokens, prompt_tokens) = match &core_bpe {
                    Some(core_bpe) => (
                        core_bpe.encode_with_special_tokens(&r),
                        core_bpe.encode_with_special_tokens(prompt),
                    ),
                    _ => (
                        CORE_BPE_TOKEN_COUNT.encode_with_special_tokens(&r),
                        CORE_BPE_TOKEN_COUNT.encode_with_special_tokens(prompt),
                    ),
                };

                let output_tokens_count = tokens.len() + prompt_tokens.len();

                if output_tokens_count > max_tokens {
                    let r = clean_html_slim(&r);

                    max_tokens = crate::features::openai::calculate_max_tokens(
                        &gpt_configs.model,
                        gpt_configs.max_tokens,
                        &crate::features::openai::BROWSER_ACTIONS_SYSTEM_PROMPT_COMPLETION.clone(),
                        &r,
                        prompt,
                    );

                    let (tokens, prompt_tokens) = match &core_bpe {
                        Some(core_bpe) => (
                            core_bpe.encode_with_special_tokens(&r),
                            core_bpe.encode_with_special_tokens(prompt),
                        ),
                        _ => (
                            CORE_BPE_TOKEN_COUNT.encode_with_special_tokens(&r),
                            CORE_BPE_TOKEN_COUNT.encode_with_special_tokens(prompt),
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
                    async_openai::types::chat::ResponseFormat::JsonObject
                } else {
                    async_openai::types::chat::ResponseFormat::Text
                };

                if let Some(structure) = &gpt_configs.json_schema {
                    if let Some(schema) = &structure.schema {
                        if let Ok(mut schema) =
                            crate::features::serde_json::from_str::<serde_json::Value>(schema)
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

                            mode = async_openai::types::chat::ResponseFormat::JsonSchema {
                                json_schema: async_openai::types::chat::ResponseFormatJsonSchema {
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

            match async_openai::types::chat::ChatCompletionRequestAssistantMessageArgs::default()
                .content(string_concat!("URL: ", url, "\n", "HTML: ", resource))
                .build()
            {
                Ok(resource_completion) => {
                    let mut messages: Vec<async_openai::types::chat::ChatCompletionRequestMessage> =
                        vec![crate::features::openai::BROWSER_ACTIONS_SYSTEM_PROMPT.clone()];

                    if json_mode {
                        messages.push(
                            crate::features::openai::BROWSER_ACTIONS_SYSTEM_EXTRA_PROMPT.clone(),
                        );
                    }

                    messages.push(resource_completion.into());

                    if !prompt.is_empty() {
                        messages.push(
                            match async_openai::types::chat::ChatCompletionRequestUserMessageArgs::default()
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
                            let res = match &gpt_configs.api_key {
                                Some(key) => {
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

#[cfg(any(feature = "gemini", feature = "real_browser"))]
lazy_static! {
    /// Semaphore for Gemini rate limiting
    pub static ref GEMINI_SEM: tokio::sync::Semaphore = {
        let sem_limit = (num_cpus::get() * 2).max(8);
        tokio::sync::Semaphore::const_new(sem_limit)
    };
}

#[cfg(not(feature = "gemini"))]
/// Perform a request to Gemini. This does nothing without the 'gemini' flag enabled.
pub async fn gemini_request(
    _gemini_configs: &crate::configuration::GeminiConfigs,
    _resource: String,
    _url: &str,
    _prompt: &str,
) -> crate::features::gemini_common::GeminiReturn {
    Default::default()
}

#[cfg(feature = "gemini")]
/// Perform a request to Gemini Chat.
pub async fn gemini_request_base(
    gemini_configs: &crate::configuration::GeminiConfigs,
    resource: String,
    url: &str,
    prompt: &str,
) -> crate::features::gemini_common::GeminiReturn {
    use crate::features::gemini_common::{GeminiReturn, GeminiUsage, DEFAULT_GEMINI_MODEL};

    match GEMINI_SEM.acquire().await {
        Ok(permit) => {
            // Get API key from config or environment
            let api_key = match &gemini_configs.api_key {
                Some(key) if !key.is_empty() => key.clone(),
                _ => match std::env::var("GEMINI_API_KEY") {
                    Ok(key) => key,
                    Err(_) => {
                        return GeminiReturn {
                            error: Some("GEMINI_API_KEY not set".to_string()),
                            ..Default::default()
                        };
                    }
                },
            };

            // Determine model to use
            let model = if gemini_configs.model.is_empty() {
                DEFAULT_GEMINI_MODEL.to_string()
            } else {
                gemini_configs.model.clone()
            };

            // Create Gemini client with model
            let client = match gemini_rust::Gemini::with_model(&api_key, model) {
                Ok(c) => c,
                Err(e) => {
                    drop(permit);
                    return GeminiReturn {
                        error: Some(format!("Failed to create Gemini client: {}", e)),
                        ..Default::default()
                    };
                }
            };

            // Clean HTML to reduce token usage
            let resource = clean_html(&resource);

            // Build the combined prompt
            let json_mode = gemini_configs.extra_ai_data;
            let system_prompt = if json_mode {
                format!(
                    "{}\n\n{}",
                    *crate::features::gemini::BROWSER_ACTIONS_SYSTEM_PROMPT,
                    *crate::features::gemini::BROWSER_ACTIONS_SYSTEM_EXTRA_PROMPT
                )
            } else {
                crate::features::gemini::BROWSER_ACTIONS_SYSTEM_PROMPT.clone()
            };

            let full_prompt = format!(
                "{}\n\nURL: {}\nHTML: {}\n\nUser Request: {}",
                system_prompt, url, resource, prompt
            );

            // Build generation config with JSON schema support
            let gen_config = gemini_rust::GenerationConfig {
                max_output_tokens: Some(gemini_configs.max_tokens as i32),
                temperature: gemini_configs.temperature,
                top_p: gemini_configs.top_p,
                top_k: gemini_configs.top_k,
                response_mime_type: if gemini_configs.json_schema.is_some() {
                    Some("application/json".to_string())
                } else {
                    None
                },
                response_schema: gemini_configs.json_schema.as_ref().and_then(|schema| {
                    schema
                        .schema
                        .as_ref()
                        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
                }),
                ..Default::default()
            };

            // Execute request
            let result = client
                .generate_content()
                .with_user_message(&full_prompt)
                .with_generation_config(gen_config)
                .execute()
                .await;

            drop(permit);

            match result {
                Ok(response) => {
                    let text = response.text();

                    // Extract usage metadata
                    let usage = if let Some(meta) = response.usage_metadata {
                        GeminiUsage {
                            prompt_tokens: meta.prompt_token_count.unwrap_or(0) as u32,
                            completion_tokens: meta.candidates_token_count.unwrap_or(0) as u32,
                            total_tokens: meta.total_token_count.unwrap_or(0) as u32,
                            cached: false,
                        }
                    } else {
                        GeminiUsage::default()
                    };

                    GeminiReturn {
                        response: text,
                        usage,
                        error: None,
                    }
                }
                Err(e) => {
                    log::error!("Gemini request failed: {:?}", e);
                    GeminiReturn {
                        error: Some(e.to_string()),
                        ..Default::default()
                    }
                }
            }
        }
        Err(e) => GeminiReturn {
            error: Some(e.to_string()),
            ..Default::default()
        },
    }
}

#[cfg(all(feature = "gemini", not(feature = "cache_gemini")))]
/// Perform a request to Gemini Chat.
pub async fn gemini_request(
    gemini_configs: &crate::configuration::GeminiConfigs,
    resource: String,
    url: &str,
    prompt: &str,
) -> crate::features::gemini_common::GeminiReturn {
    gemini_request_base(gemini_configs, resource, url, prompt).await
}

#[cfg(all(feature = "gemini", feature = "cache_gemini"))]
/// Perform a request to Gemini Chat with caching.
pub async fn gemini_request(
    gemini_configs: &crate::configuration::GeminiConfigs,
    resource: String,
    url: &str,
    prompt: &str,
) -> crate::features::gemini_common::GeminiReturn {
    match &gemini_configs.cache {
        Some(cache) => {
            use std::hash::{Hash, Hasher};
            let mut s = ahash::AHasher::default();

            url.hash(&mut s);
            prompt.hash(&mut s);
            gemini_configs.model.hash(&mut s);
            gemini_configs.max_tokens.hash(&mut s);
            gemini_configs.extra_ai_data.hash(&mut s);
            resource.hash(&mut s);

            let key = s.finish();

            match cache.get(&key).await {
                Some(cached) => {
                    let mut c = cached;
                    c.usage.cached = true;
                    c
                }
                _ => {
                    let r = gemini_request_base(gemini_configs, resource, url, prompt).await;
                    let _ = cache.insert(key, r.clone()).await;
                    r
                }
            }
        }
        _ => gemini_request_base(gemini_configs, resource, url, prompt).await,
    }
}

/// Clean the html removing css and js default (raw passthrough).
#[inline]
pub fn clean_html_raw(html: &str) -> String {
    html.to_string()
}

/// Clean the html removing css and js (base).
///
/// Uses `lol_html` to strip noisy elements and reduce prompt size.
pub fn clean_html_base(html: &str) -> String {
    use lol_html::{doc_comments, element, rewrite_str, RewriteStrSettings};

    // catch_unwind guards against lol_html's internal
    // `String::from_utf8(output).unwrap()` panic on malformed encodings.
    // AssertUnwindSafe avoids cloning html — zero overhead on success path.
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        rewrite_str(
            html,
            RewriteStrSettings {
                element_content_handlers: vec![
                    element!("script, style, link, iframe", |el| {
                        el.remove();
                        Ok(())
                    }),
                    element!(
                        "[style*='display:none'], [id*='ad'], [class*='ad'], [id*='tracking'], [class*='tracking']",
                        |el| {
                            el.remove();
                            Ok(())
                        }
                    ),
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
        )
    })) {
        Ok(Ok(r)) => r,
        _ => html.into(),
    }
}

/// Clean the HTML to slim-fit models. This removes base64 images and heavy nodes.
pub fn clean_html_slim(html: &str) -> String {
    use lol_html::{doc_comments, element, rewrite_str, RewriteStrSettings};

    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        rewrite_str(
            html,
            RewriteStrSettings {
                element_content_handlers: vec![
                    element!(
                        "script, style, svg, noscript, link, iframe, canvas, video",
                        |el| {
                            el.remove();
                            Ok(())
                        }
                    ),
                    element!("img, picture", |el| {
                        if let Some(src) = el.get_attribute("src") {
                            if src.starts_with("data:image") {
                                el.remove();
                            }
                        }
                        Ok(())
                    }),
                    element!(
                        "[style*='display:none'], [id*='ad'], [class*='ad'], [id*='tracking'], [class*='tracking']",
                        |el| {
                            el.remove();
                            Ok(())
                        }
                    ),
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
        )
    })) {
        Ok(Ok(r)) => r,
        _ => html.into(),
    }
}

/// Clean the most extra properties in the html to fit the context.
/// Removes nav/footer, trims meta, and prunes most attributes except id/class/data-*.
pub fn clean_html_full(html: &str) -> String {
    use lol_html::{doc_comments, element, rewrite_str, RewriteStrSettings};

    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        rewrite_str(
            html,
            RewriteStrSettings {
                element_content_handlers: vec![
                    element!("nav, footer", |el| {
                        el.remove();
                        Ok(())
                    }),
                    element!("meta", |el| {
                        let keep = el
                            .get_attribute("name")
                            .map(|n| {
                                n.eq_ignore_ascii_case("viewport")
                                    || n.eq_ignore_ascii_case("charset")
                            })
                            .unwrap_or(false);
                        if !keep {
                            el.remove();
                        }
                        Ok(())
                    }),
                    element!("*", |el| {
                        let attrs = el.attributes();
                        let mut to_remove: smallvec::SmallVec<[String; 16]> =
                            smallvec::SmallVec::new();
                        for attr in attrs.iter() {
                            let n = attr.name();
                            if n != "id" && n != "class" && !n.starts_with("data-") {
                                to_remove.push(n);
                            }
                        }
                        for attr in to_remove {
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
        )
    })) {
        Ok(Ok(r)) => r,
        _ => html.into(),
    }
}

/// Default cleaner used by the engine.
///
/// If you still want a “slim fit” toggle, keep the feature gate here (safe).
#[cfg(feature = "openai_slim_fit")]
#[inline]
pub fn clean_html(html: &str) -> String {
    clean_html_slim(html)
}

/// Default cleaner used by the engine (non-slim build).
#[cfg(not(feature = "openai_slim_fit"))]
#[inline]
pub fn clean_html(html: &str) -> String {
    clean_html_base(html)
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
    if let Err(e) = CONTROLLER
        .write()
        .await
        .0
        .send((target.into(), Handler::Pause))
    {
        log::error!("PAUSE: {:?}", e);
    }
}

#[cfg(feature = "control")]
/// Resume a target website crawl. The crawl_id is prepended directly to the domain and required if set. ex: d22323edsd-https://mydomain.com
pub async fn resume(target: &str) {
    if let Err(e) = CONTROLLER
        .write()
        .await
        .0
        .send((target.into(), Handler::Resume))
    {
        log::error!("RESUME: {:?}", e);
    }
}

#[cfg(feature = "control")]
/// Shutdown a target website crawl. The crawl_id is prepended directly to the domain and required if set. ex: d22323edsd-https://mydomain.com
pub async fn shutdown(target: &str) {
    if let Err(e) = CONTROLLER
        .write()
        .await
        .0
        .send((target.into(), Handler::Shutdown))
    {
        log::error!("SHUTDOWN: {:?}", e);
    }
}

#[cfg(feature = "control")]
/// Reset a target website crawl. The crawl_id is prepended directly to the domain and required if set. ex: d22323edsd-https://mydomain.com
pub async fn reset(target: &str) {
    if let Err(e) = CONTROLLER
        .write()
        .await
        .0
        .send((target.into(), Handler::Start))
    {
        log::error!("RESET: {:?}", e);
    }
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
    let bytes = url.as_bytes();
    if let Some(start_pos) = memchr::memmem::find(bytes, b"//") {
        let pos = start_pos + 2;

        if let Some(first_slash_pos) = memchr::memchr(b'/', &bytes[pos..]) {
            &url[pos..pos + first_slash_pos]
        } else {
            &url[pos..]
        }
    } else if let Some(first_slash_pos) = memchr::memchr(b'/', bytes) {
        &url[..first_slash_pos]
    } else {
        url
    }
}

/// Determine if networking is capable for a URL.
/// Uses first-byte dispatch to avoid 4 redundant prefix scans.
pub fn networking_capable(url: &str) -> bool {
    match url.as_bytes().first() {
        Some(b'h') => url.starts_with("https://") || url.starts_with("http://"),
        Some(b'f') => url.starts_with("file://") || url.starts_with("ftp://"),
        _ => false,
    }
}

/// Prepare the url for parsing if it fails. Use this method if the url does not start with http or https.
pub fn prepare_url(u: &str) -> String {
    if let Some(index) = memchr::memmem::find(u.as_bytes(), b"://") {
        let split_index = index + 3;
        let rest = if split_index < u.len() {
            &u[split_index..]
        } else {
            ""
        };
        let mut s = String::with_capacity(8 + rest.len());
        s.push_str("https://");
        s.push_str(rest);
        s
    } else {
        let mut s = String::with_capacity(8 + u.len());
        s.push_str("https://");
        s.push_str(u);
        s
    }
}

/// normalize the html markup to prevent Maliciousness.
pub(crate) async fn normalize_html(html: &[u8]) -> Vec<u8> {
    use lol_html::{element, send::Settings};

    // Pre-allocate: normalized output is typically smaller than input due to
    // removed elements/attributes, so 3/4 of input is a reasonable estimate.
    let mut output = Vec::with_capacity(html.len() * 3 / 4);

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
                    let attrs = el.attributes();
                    // SmallVec avoids heap alloc for elements with ≤16 attributes.
                    let mut remove_attr: smallvec::SmallVec<[String; 16]> =
                        smallvec::SmallVec::new();

                    for attr in attrs {
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

    let mut wrote_error = false;
    let should_yield = html.len() > REWRITER_YIELD_THRESHOLD;

    for (i, chunk) in html.chunks(*STREAMING_CHUNK_SIZE).enumerate() {
        if rewriter.write(chunk).is_err() {
            wrote_error = true;
            break;
        }
        if should_yield && i % REWRITER_YIELD_INTERVAL == REWRITER_YIELD_INTERVAL - 1 {
            tokio::task::yield_now().await;
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

        s.finish()
    } else {
        Default::default()
    }
}

#[allow(unused)]
/// Spawns a new asynchronous task.
pub(crate) fn spawn_task<F>(_task_name: &str, future: F) -> tokio::task::JoinHandle<F::Output>
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    tokio::task::spawn(future)
}

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
/// Takes the worse of CPU and process memory pressure to drive throttling.
#[cfg(feature = "balance")]
pub async fn get_semaphore(semaphore: &Arc<Semaphore>, detect: bool) -> &Arc<Semaphore> {
    let (cpu_load, mem_load) = if detect {
        (
            detect_system::get_global_cpu_state_sync(),
            detect_system::get_process_memory_state_sync(),
        )
    } else {
        (0, 0)
    };

    let load = cpu_load.max(mem_load);

    if load == 2 {
        tokio::time::sleep(REBALANCE_TIME).await;
    }

    if load >= 1 {
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

/// Check if the content is HTML.
///
/// Uses `memchr` SIMD-accelerated byte scanning to find `<` candidates, then
/// verifies the tag prefix with a case-insensitive comparison — O(n) with
/// SIMD vs the previous O(n*m) `.windows()` approach.
pub fn is_html_content_check(bytes: &[u8]) -> bool {
    const TAG_SUFFIXES: &[&[u8]] = &[b"!doctype html", b"html", b"document"];

    let check_bytes = if bytes.len() > 1024 {
        &bytes[..1024]
    } else {
        bytes
    };

    let mut offset = 0;
    while let Some(pos) = memchr::memchr(b'<', &check_bytes[offset..]) {
        let abs = offset + pos;
        let after = &check_bytes[abs + 1..]; // slice after '<'
        for suffix in TAG_SUFFIXES {
            if after.len() >= suffix.len() && after[..suffix.len()].eq_ignore_ascii_case(suffix) {
                return true;
            }
        }
        offset = abs + 1;
    }

    false
}

/// Return the semaphore that should be used.
#[cfg(not(feature = "balance"))]
pub async fn get_semaphore(semaphore: &Arc<Semaphore>, _detect: bool) -> &Arc<Semaphore> {
    semaphore
}

// #[derive(Debug)]
// /// Html output sink for the rewriter.
// #[cfg(feature = "smart")]
// pub(crate) struct HtmlOutputSink {
//     /// The bytes collected.
//     pub(crate) data: Vec<u8>,
//     /// The sender to send once finished.
//     pub(crate) sender: Option<tokio::sync::oneshot::Sender<Vec<u8>>>,
// }

// #[cfg(feature = "smart")]
// impl HtmlOutputSink {
//     /// A new output sink.
//     pub(crate) fn new(sender: tokio::sync::oneshot::Sender<Vec<u8>>) -> Self {
//         HtmlOutputSink {
//             data: Vec::new(),
//             sender: Some(sender),
//         }
//     }
// }

// #[cfg(feature = "smart")]
// impl OutputSink for HtmlOutputSink {
//     fn handle_chunk(&mut self, chunk: &[u8]) {
//         self.data.extend_from_slice(chunk);
//         if chunk.len() == 0 {
//             if let Some(sender) = self.sender.take() {
//                 let data_to_send = std::mem::take(&mut self.data);
//                 let _ = sender.send(data_to_send);
//             }
//         }
//     }
// }

/// Consumes `set` and returns (left, right), where `left` are items matching `pred`.
pub fn split_hashset_round_robin<T>(mut set: HashSet<T>, parts: usize) -> Vec<HashSet<T>>
where
    T: Eq + std::hash::Hash,
{
    if parts <= 1 {
        return vec![set];
    }
    let len = set.len();
    let mut buckets: Vec<HashSet<T>> = (0..parts)
        .map(|_| HashSet::with_capacity(len / parts + 1))
        .collect();

    let mut i = 0usize;
    for v in set.drain() {
        buckets[i % parts].insert(v);
        i += 1;
    }
    buckets
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_open_resty_forbidden() {
        let body = b"<html><head><title>403 Forbidden</title></head>\n<body>\n<center><h1>403 Forbidden</h1></center>\n<hr><center>openresty</center>";
        assert!(detect_open_resty_forbidden(body));
        assert!(!detect_open_resty_forbidden(
            b"<html><body>OK</body></html>"
        ));
    }

    #[test]
    fn test_detect_hard_forbidden_content() {
        // OpenResty forbidden
        let openresty = b"<html><head><title>403 Forbidden</title></head>\n<body>\n<center><h1>403 Forbidden</h1></center>\n<hr><center>openresty</center>";
        assert!(detect_hard_forbidden_content(openresty));
        // Normal content
        assert!(!detect_hard_forbidden_content(
            b"<html><body>Hello</body></html>"
        ));
    }

    #[test]
    fn test_detect_anti_bot_from_body() {
        // Large body with no pattern in first 30KB — returns None
        let large_body = vec![0u8; 40_000];
        assert!(detect_anti_bot_from_body(&large_body).is_none());

        // Large body with anti-bot pattern in the first 30KB — detected
        let mut large_with_pattern = b"<span class=\"cf-error-code\">1020</span>".to_vec();
        large_with_pattern.resize(40_000, b' ');
        assert_eq!(
            detect_anti_bot_from_body(&large_with_pattern),
            Some(AntiBotTech::Cloudflare)
        );

        // Large body with anti-bot pattern AFTER the 30KB boundary — not detected
        let mut large_pattern_late = vec![b' '; 30_001];
        large_pattern_late.extend_from_slice(b"cf-error-code");
        assert!(detect_anti_bot_from_body(&large_pattern_late).is_none());
        // Normal page - no match
        let normal = b"<html><body>Hello world</body></html>".to_vec();
        assert!(detect_anti_bot_from_body(&normal).is_none());

        // Pattern 0: cf-error-code → Cloudflare
        assert_eq!(
            detect_anti_bot_from_body(&b"<span class=\"cf-error-code\">1020</span>".to_vec()),
            Some(AntiBotTech::Cloudflare)
        );
        // Pattern 1: Access to this page has been denied → Cloudflare
        assert_eq!(
            detect_anti_bot_from_body(&b"<h1>Access to this page has been denied</h1>".to_vec()),
            Some(AntiBotTech::Cloudflare)
        );
        // Pattern 2: DataDome
        assert_eq!(
            detect_anti_bot_from_body(&b"<script src=\"https://js.DataDome.co/tags.js\">".to_vec()),
            Some(AntiBotTech::DataDome)
        );
        // Pattern 3: perimeterx → PerimeterX
        assert_eq!(
            detect_anti_bot_from_body(&b"<script>window._pxAppId='perimeterx';</script>".to_vec()),
            Some(AntiBotTech::PerimeterX)
        );
        // Pattern 4: funcaptcha → ArkoseLabs
        assert_eq!(
            detect_anti_bot_from_body(
                &b"<iframe src=\"https://client-api.arkoselabs.com/funcaptcha\">".to_vec()
            ),
            Some(AntiBotTech::ArkoseLabs)
        );
        // Pattern 5: Incapsula → Imperva
        assert_eq!(
            detect_anti_bot_from_body(
                &b"Request unsuccessful. Incapsula incident ID: 123".to_vec()
            ),
            Some(AntiBotTech::Imperva)
        );
        // Pattern 6: _____tmd_____ → AlibabaTMD
        assert_eq!(
            detect_anti_bot_from_body(
                &br#"<script>window.location.replace("https://example.com/_____tmd_____/punish?x5secdata=abc");</script>"#.to_vec()
            ),
            Some(AntiBotTech::AlibabaTMD)
        );
        // Pattern 7: x5secdata → AlibabaTMD
        assert_eq!(
            detect_anti_bot_from_body(
                &br#"<script>sessionStorage.x5referer=window.location.href;window.location.replace("https://example.com/punish?x5secdata=xyz&x5step=1");</script>"#.to_vec()
            ),
            Some(AntiBotTech::AlibabaTMD)
        );
        // Pattern 9: ak_bmsc → Akamai
        assert_eq!(
            detect_anti_bot_from_body(
                &b"<script>document.cookie=\"ak_bmsc=abc123\";</script>".to_vec()
            ),
            Some(AntiBotTech::AkamaiBotManager)
        );
        // Pattern 10: challenge-platform → Cloudflare
        assert_eq!(
            detect_anti_bot_from_body(
                &b"<script src=\"/cdn-cgi/challenge-platform/scripts/abc\"></script>".to_vec()
            ),
            Some(AntiBotTech::Cloudflare)
        );
        // Pattern 12: ddos-guard → DDoSGuard
        assert_eq!(
            detect_anti_bot_from_body(&b"<meta name=\"ddos-guard\">".to_vec()),
            Some(AntiBotTech::DDoSGuard)
        );
        // Pattern 13: px-captcha → PerimeterX
        assert_eq!(
            detect_anti_bot_from_body(&b"<div id=\"px-captcha\"></div>".to_vec()),
            Some(AntiBotTech::PerimeterX)
        );
        // Pattern 14: verify you are human → Generic
        assert_eq!(
            detect_anti_bot_from_body(&b"<p>Please verify you are human to continue</p>".to_vec()),
            Some(AntiBotTech::None)
        );
        // Pattern 15: prove you're not a robot → Generic
        assert_eq!(
            detect_anti_bot_from_body(&b"<p>Please prove you're not a robot</p>".to_vec()),
            Some(AntiBotTech::None)
        );
        // Pattern 16: Sucuri Website Firewall → Sucuri
        assert_eq!(
            detect_anti_bot_from_body(
                &b"<h1>Access Denied - Sucuri Website Firewall</h1>".to_vec()
            ),
            Some(AntiBotTech::Sucuri)
        );
        // Pattern 17: kpsdk → Kasada
        assert_eq!(
            detect_anti_bot_from_body(&b"<script src=\"/kpsdk/loader.js\"></script>".to_vec()),
            Some(AntiBotTech::Kasada)
        );
        // Pattern 18: _Incapsula_Resource → Imperva
        assert_eq!(
            detect_anti_bot_from_body(
                &b"<script src=\"/_Incapsula_Resource?SWKMTFSR=1\"></script>".to_vec()
            ),
            Some(AntiBotTech::Imperva)
        );
        // Pattern 19: Vercel Security Checkpoint → Vercel
        assert_eq!(
            detect_anti_bot_from_body(&b"<title>Vercel Security Checkpoint</title>".to_vec()),
            Some(AntiBotTech::Vercel)
        );
        // Pattern 20: Generated by Wordfence → Wordfence
        assert_eq!(
            detect_anti_bot_from_body(
                &b"<p>Generated by Wordfence at Sat, 22 Mar 2026</p>".to_vec()
            ),
            Some(AntiBotTech::Wordfence)
        );
        // Pattern 21: Attention Required! | Cloudflare → Cloudflare
        assert_eq!(
            detect_anti_bot_from_body(&b"<title>Attention Required! | Cloudflare</title>".to_vec()),
            Some(AntiBotTech::Cloudflare)
        );
        // Pattern 22: aws-waf-token → AwsWaf
        assert_eq!(
            detect_anti_bot_from_body(
                &b"<input name=\"aws-waf-token\" type=\"hidden\" value=\"abc\">".to_vec()
            ),
            Some(AntiBotTech::AwsWaf)
        );
        // Pattern 23: DDoS-Guard (capitalized) → DDoSGuard
        assert_eq!(
            detect_anti_bot_from_body(&b"<p>DDoS protection by DDoS-Guard</p>".to_vec()),
            Some(AntiBotTech::DDoSGuard)
        );
    }

    #[test]
    fn test_detect_antibot_from_url() {
        // No match
        assert!(detect_antibot_from_url("https://example.com/page").is_none());

        // Pattern 0: /cdn-cgi/challenge-platform → Cloudflare
        assert_eq!(
            detect_antibot_from_url("https://example.com/cdn-cgi/challenge-platform/h/b"),
            Some(AntiBotTech::Cloudflare)
        );
        // Pattern 1: datadome.co → DataDome
        assert_eq!(
            detect_antibot_from_url("https://api.datadome.co/validate"),
            Some(AntiBotTech::DataDome)
        );
        // Pattern 2: dd-api.io → DataDome
        assert_eq!(
            detect_antibot_from_url("https://dd-api.io/js/v1"),
            Some(AntiBotTech::DataDome)
        );
        // Pattern 3: perimeterx.net → PerimeterX
        assert_eq!(
            detect_antibot_from_url("https://client.perimeterx.net/main.min.js"),
            Some(AntiBotTech::PerimeterX)
        );
        // Pattern 4: px-captcha → PerimeterX
        assert_eq!(
            detect_antibot_from_url("https://example.com/px-captcha"),
            Some(AntiBotTech::PerimeterX)
        );
        // Pattern 5: arkoselabs.com → ArkoseLabs
        assert_eq!(
            detect_antibot_from_url("https://client-api.arkoselabs.com/fc/gt2/"),
            Some(AntiBotTech::ArkoseLabs)
        );
        // Pattern 6: funcaptcha → ArkoseLabs
        assert_eq!(
            detect_antibot_from_url("https://example.com/funcaptcha/verify"),
            Some(AntiBotTech::ArkoseLabs)
        );
        // Pattern 7: kasada.io → Kasada
        assert_eq!(
            detect_antibot_from_url("https://ips.kasada.io/149/script"),
            Some(AntiBotTech::Kasada)
        );
        // Pattern 8: fingerprint.com → FingerprintJS
        assert_eq!(
            detect_antibot_from_url("https://api.fingerprint.com/v3"),
            Some(AntiBotTech::FingerprintJS)
        );
        // Pattern 9: fpjs.io → FingerprintJS
        assert_eq!(
            detect_antibot_from_url("https://fpjs.io/agent"),
            Some(AntiBotTech::FingerprintJS)
        );
        // Pattern 10: incapsula → Imperva
        assert_eq!(
            detect_antibot_from_url("https://example.com/incapsula/resource"),
            Some(AntiBotTech::Imperva)
        );
        // Pattern 11: imperva → Imperva
        assert_eq!(
            detect_antibot_from_url("https://example.com/imperva/block"),
            Some(AntiBotTech::Imperva)
        );
        // Pattern 12: radwarebotmanager → RadwareBotManager
        assert_eq!(
            detect_antibot_from_url("https://example.com/radwarebotmanager/api"),
            Some(AntiBotTech::RadwareBotManager)
        );
        // Pattern 13: reblaze.com → Reblaze
        assert_eq!(
            detect_antibot_from_url("https://reblaze.com/check"),
            Some(AntiBotTech::Reblaze)
        );
        // Pattern 14: cheq.ai → CHEQ
        assert_eq!(
            detect_antibot_from_url("https://api.cheq.ai/verify"),
            Some(AntiBotTech::CHEQ)
        );
        // Pattern 15: _____tmd_____/punish → AlibabaTMD
        assert_eq!(
            detect_antibot_from_url(
                "https://www.miravia.es/p/i123/_____tmd_____/punish?x5secdata=abc"
            ),
            Some(AntiBotTech::AlibabaTMD)
        );
        // Pattern 16: hcaptcha.com → HCaptcha
        assert_eq!(
            detect_antibot_from_url("https://newassets.hcaptcha.com/captcha/v1/abc"),
            Some(AntiBotTech::HCaptcha)
        );
        // Pattern 17: api.geetest.com → GeeTest
        assert_eq!(
            detect_antibot_from_url("https://api.geetest.com/gettype.php"),
            Some(AntiBotTech::GeeTest)
        );
        // Pattern 18: geevisit.com → GeeTest
        assert_eq!(
            detect_antibot_from_url("https://api.geevisit.com/get.php"),
            Some(AntiBotTech::GeeTest)
        );
        // Pattern 19: queue-it.net → QueueIt
        assert_eq!(
            detect_antibot_from_url("https://myevent.queue-it.net/softblock"),
            Some(AntiBotTech::QueueIt)
        );
        // Pattern 20: ddos-guard.net → DDoSGuard
        assert_eq!(
            detect_antibot_from_url("https://ddos-guard.net/check-browser"),
            Some(AntiBotTech::DDoSGuard)
        );
        // Pattern 21: /_Incapsula_Resource → Imperva
        assert_eq!(
            detect_antibot_from_url("https://example.com/_Incapsula_Resource?SWKMTFSR=1&e=abc"),
            Some(AntiBotTech::Imperva)
        );
        // Pattern 22: /cdn-cgi/bm/cv/ → Cloudflare Bot Management
        assert_eq!(
            detect_antibot_from_url("https://example.com/cdn-cgi/bm/cv/result?req_id=abc"),
            Some(AntiBotTech::Cloudflare)
        );
        // Pattern 23: sucuri.net → Sucuri
        assert_eq!(
            detect_antibot_from_url("https://sucuri.net/verify"),
            Some(AntiBotTech::Sucuri)
        );
    }

    #[test]
    fn test_detect_anti_bot_from_headers() {
        use std::collections::HashMap;

        // No antibot headers → None
        let empty: HashMap<String, String> = HashMap::new();
        assert!(detect_anti_bot_from_headers(&HeaderSource::Map(&empty)).is_none());

        // Cloudflare via cf-ray
        let mut h = HashMap::new();
        h.insert("cf-ray".to_string(), "abc123".to_string());
        assert_eq!(
            detect_anti_bot_from_headers(&HeaderSource::Map(&h)),
            Some(AntiBotTech::Cloudflare)
        );

        // Cloudflare via cf-mitigated
        let mut h = HashMap::new();
        h.insert("cf-mitigated".to_string(), "challenge".to_string());
        assert_eq!(
            detect_anti_bot_from_headers(&HeaderSource::Map(&h)),
            Some(AntiBotTech::Cloudflare)
        );

        // DataDome via x-datadome
        let mut h = HashMap::new();
        h.insert("x-datadome".to_string(), "1".to_string());
        assert_eq!(
            detect_anti_bot_from_headers(&HeaderSource::Map(&h)),
            Some(AntiBotTech::DataDome)
        );

        // PerimeterX via pxhd
        let mut h = HashMap::new();
        h.insert("pxhd".to_string(), "token".to_string());
        assert_eq!(
            detect_anti_bot_from_headers(&HeaderSource::Map(&h)),
            Some(AntiBotTech::PerimeterX)
        );

        // Sucuri via x-sucuri-id
        let mut h = HashMap::new();
        h.insert("x-sucuri-id".to_string(), "12345".to_string());
        assert_eq!(
            detect_anti_bot_from_headers(&HeaderSource::Map(&h)),
            Some(AntiBotTech::Sucuri)
        );

        // Server: cloudflare
        let mut h = HashMap::new();
        h.insert("server".to_string(), "cloudflare".to_string());
        assert_eq!(
            detect_anti_bot_from_headers(&HeaderSource::Map(&h)),
            Some(AntiBotTech::Cloudflare)
        );

        // Server: AkamaiGHost (case-insensitive)
        let mut h = HashMap::new();
        h.insert("server".to_string(), "AkamaiGHost".to_string());
        assert_eq!(
            detect_anti_bot_from_headers(&HeaderSource::Map(&h)),
            Some(AntiBotTech::AkamaiBotManager)
        );

        // Server: Sucuri/Cloudproxy
        let mut h = HashMap::new();
        h.insert("server".to_string(), "Sucuri/Cloudproxy".to_string());
        assert_eq!(
            detect_anti_bot_from_headers(&HeaderSource::Map(&h)),
            Some(AntiBotTech::Sucuri)
        );

        // Server: DDoS-Guard
        let mut h = HashMap::new();
        h.insert("server".to_string(), "DDoS-Guard".to_string());
        assert_eq!(
            detect_anti_bot_from_headers(&HeaderSource::Map(&h)),
            Some(AntiBotTech::DDoSGuard)
        );

        // Server: DataDome
        let mut h = HashMap::new();
        h.insert("server".to_string(), "DataDome".to_string());
        assert_eq!(
            detect_anti_bot_from_headers(&HeaderSource::Map(&h)),
            Some(AntiBotTech::DataDome)
        );

        // Server: nginx (no match)
        let mut h = HashMap::new();
        h.insert("server".to_string(), "nginx/1.24".to_string());
        assert!(detect_anti_bot_from_headers(&HeaderSource::Map(&h)).is_none());
    }

    #[test]
    fn test_compiled_custom_antibot() {
        use crate::configuration::CustomAntibotPatterns;
        use crate::page::AntiBotTech;

        // Empty config → None
        let empty = CustomAntibotPatterns::default();
        assert!(CompiledCustomAntibot::compile(&empty).is_none());

        // Body pattern
        let cfg = CustomAntibotPatterns {
            body: vec!["my-custom-waf".into()],
            url: vec![],
            header_keys: vec![],
        };
        let compiled = CompiledCustomAntibot::compile(&cfg).unwrap();
        assert!(compiled.detect_body(b"<p>Blocked by my-custom-waf</p>"));
        assert!(!compiled.detect_body(b"<p>Normal page</p>"));
        // Body > 30KB skipped
        assert!(!compiled.detect_body(&vec![b'x'; 40_000]));

        // URL pattern
        let cfg = CustomAntibotPatterns {
            body: vec![],
            url: vec!["waf.example.com".into()],
            header_keys: vec![],
        };
        let compiled = CompiledCustomAntibot::compile(&cfg).unwrap();
        assert!(compiled.detect_url("https://waf.example.com/challenge"));
        assert!(!compiled.detect_url("https://example.com/page"));

        // Header key
        let cfg = CustomAntibotPatterns {
            body: vec![],
            url: vec![],
            header_keys: vec!["x-my-waf".into()],
        };
        let compiled = CompiledCustomAntibot::compile(&cfg).unwrap();
        let mut h = std::collections::HashMap::new();
        h.insert("x-my-waf".to_string(), "1".to_string());
        assert!(compiled.detect_headers(&HeaderSource::Map(&h)));
        let empty_h: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        assert!(!compiled.detect_headers(&HeaderSource::Map(&empty_h)));

        // detect_anti_bot_tech_response_custom
        let cfg = CustomAntibotPatterns {
            body: vec!["my-proprietary-bot-wall".into()],
            url: vec![],
            header_keys: vec![],
        };
        let compiled = CompiledCustomAntibot::compile(&cfg).unwrap();
        let empty_headers: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        assert_eq!(
            detect_anti_bot_tech_response_custom(
                "https://example.com",
                &HeaderSource::Map(&empty_headers),
                b"<p>Blocked by my-proprietary-bot-wall</p>",
                None,
                Some(&compiled),
            ),
            AntiBotTech::Custom
        );
        // Built-in match takes precedence over custom
        assert_eq!(
            detect_anti_bot_tech_response_custom(
                "https://example.com",
                &HeaderSource::Map(&empty_headers),
                b"<span class=\"cf-error-code\">1020</span>",
                None,
                Some(&compiled),
            ),
            AntiBotTech::Cloudflare
        );
        // No match → None
        assert_eq!(
            detect_anti_bot_tech_response_custom(
                "https://example.com",
                &HeaderSource::Map(&empty_headers),
                b"<p>Normal page</p>",
                None,
                Some(&compiled),
            ),
            AntiBotTech::None
        );
    }

    #[test]
    fn test_flip_http_https() {
        assert_eq!(
            flip_http_https("http://example.com"),
            Some("https://example.com".to_string())
        );
        assert_eq!(
            flip_http_https("https://example.com"),
            Some("http://example.com".to_string())
        );
        assert_eq!(flip_http_https("ftp://example.com"), None);
    }

    #[test]
    fn test_strip_www() {
        assert_eq!(
            strip_www("https://www.docs.github.com/foo"),
            Some("https://docs.github.com/foo".to_string())
        );
        assert_eq!(
            strip_www("https://www.example.com"),
            Some("https://example.com".to_string())
        );
        assert_eq!(
            strip_www("http://www.example.com/path?q=1"),
            Some("http://example.com/path?q=1".to_string())
        );
        // No www prefix → None
        assert_eq!(strip_www("https://example.com"), None);
        assert_eq!(strip_www("https://docs.github.com"), None);
        // No scheme → None
        assert_eq!(strip_www("www.example.com"), None);
    }

    #[test]
    fn test_clean_html_raw() {
        let html = "<html><body>Hello</body></html>";
        assert_eq!(clean_html_raw(html), html);
    }

    #[test]
    fn test_clean_html_base() {
        let html = r#"<html><head><script>alert(1)</script><style>.x{}</style></head><body><p>Hello</p></body></html>"#;
        let cleaned = clean_html_base(html);
        assert!(!cleaned.contains("alert(1)"));
        assert!(!cleaned.contains(".x{}"));
        assert!(cleaned.contains("Hello"));
    }

    #[test]
    fn test_clean_html_slim() {
        let html = r#"<html><body><p>Hello</p><svg><circle/></svg><noscript>No JS</noscript></body></html>"#;
        let cleaned = clean_html_slim(html);
        assert!(cleaned.contains("Hello"));
        assert!(!cleaned.contains("<svg>"));
        assert!(!cleaned.contains("No JS"));
    }

    #[test]
    fn test_clean_html_full() {
        let html = r#"<html><body><nav>Menu</nav><p id="main" class="content" onclick="foo()">Hello</p><footer>Foot</footer></body></html>"#;
        let cleaned = clean_html_full(html);
        assert!(cleaned.contains("Hello"));
        assert!(!cleaned.contains("Menu"));
        assert!(!cleaned.contains("Foot"));
    }

    #[test]
    fn test_get_last_segment() {
        assert_eq!(get_last_segment("/foo/bar/baz"), "baz");
        assert_eq!(get_last_segment("/foo/bar/"), "");
        assert_eq!(get_last_segment("nopath"), "nopath");
    }

    #[test]
    fn test_get_path_from_url() {
        assert_eq!(get_path_from_url("https://example.com/foo/bar"), "/foo/bar");
        assert_eq!(get_path_from_url("https://example.com"), "/");
    }

    #[test]
    fn test_get_domain_from_url() {
        assert_eq!(
            get_domain_from_url("https://example.com/path"),
            "example.com"
        );
        assert_eq!(
            get_domain_from_url("https://sub.example.com/path"),
            "sub.example.com"
        );
    }

    #[test]
    fn test_networking_capable() {
        assert!(networking_capable("https://example.com"));
        assert!(networking_capable("http://example.com"));
        assert!(networking_capable("ftp://files.example.com"));
        assert!(networking_capable("file:///local/path"));
        assert!(!networking_capable("mailto:user@example.com"));
        assert!(!networking_capable("javascript:void(0)"));
    }

    #[test]
    fn test_prepare_url_no_scheme() {
        assert_eq!(prepare_url("example.com"), "https://example.com");
    }

    #[test]
    fn test_prepare_url_http() {
        assert_eq!(
            prepare_url("http://example.com/path"),
            "https://example.com/path"
        );
    }

    #[test]
    fn test_prepare_url_ftp() {
        assert_eq!(
            prepare_url("ftp://files.example.com/data"),
            "https://files.example.com/data"
        );
    }

    #[test]
    fn test_prepare_url_https_passthrough() {
        assert_eq!(prepare_url("https://example.com"), "https://example.com");
    }

    #[test]
    fn test_prepare_url_with_port() {
        assert_eq!(
            prepare_url("http://localhost:8080/api"),
            "https://localhost:8080/api"
        );
    }

    #[test]
    fn test_prepare_url_empty() {
        assert_eq!(prepare_url(""), "https://");
    }

    #[test]
    fn test_prepare_url_bare_domain_with_path() {
        assert_eq!(prepare_url("example.com/page"), "https://example.com/page");
    }

    #[test]
    fn test_is_html_content_check() {
        assert!(is_html_content_check(b"<!DOCTYPE html><html>"));
        assert!(is_html_content_check(b"<html><body>"));
        assert!(!is_html_content_check(b"{ \"json\": true }"));
        assert!(!is_html_content_check(b"plain text content"));
    }

    #[test]
    fn test_crawl_duration_expired() {
        // None timeout → not expired
        assert!(!crawl_duration_expired(&None, &None));
        assert!(!crawl_duration_expired(
            &Some(Duration::from_secs(10)),
            &None
        ));
        assert!(!crawl_duration_expired(&None, &Some(Instant::now())));

        // Very long timeout → not expired
        let start = Some(Instant::now());
        assert!(!crawl_duration_expired(
            &Some(Duration::from_secs(3600)),
            &start
        ));

        // Zero timeout → expired immediately
        assert!(crawl_duration_expired(
            &Some(Duration::from_secs(0)),
            &start
        ));
    }

    #[test]
    fn test_split_hashset_round_robin() {
        let mut set = HashSet::new();
        for i in 0..10 {
            set.insert(i);
        }

        let buckets = split_hashset_round_robin(set, 3);
        assert_eq!(buckets.len(), 3);
        let total: usize = buckets.iter().map(|b| b.len()).sum();
        assert_eq!(total, 10);

        // Single part
        let mut set2 = HashSet::new();
        set2.insert(1);
        set2.insert(2);
        let buckets2 = split_hashset_round_robin(set2, 1);
        assert_eq!(buckets2.len(), 1);
        assert_eq!(buckets2[0].len(), 2);
    }

    #[cfg(any(feature = "cache", feature = "cache_mem"))]
    #[test]
    fn test_create_cache_key_raw() {
        // No namespace — format identical to the pre-namespace era,
        // preserving backward compatibility with existing cache stores.
        assert_eq!(
            create_cache_key_raw("https://example.com", None, None, None),
            "GET:https://example.com"
        );
        assert_eq!(
            create_cache_key_raw("https://example.com", Some("POST"), None, None),
            "POST:https://example.com"
        );
        assert_eq!(
            create_cache_key_raw("https://example.com", None, Some("token123"), None),
            "GET:https://example.com:token123"
        );

        // Namespaced variants — distinct from non-namespaced and from each other.
        assert_eq!(
            create_cache_key_raw("https://example.com", None, None, Some("us")),
            "GET:https://example.com::ns=us"
        );
        assert_eq!(
            create_cache_key_raw("https://example.com", None, Some("token123"), Some("us")),
            "GET:https://example.com:token123:ns=us"
        );
        assert_eq!(
            create_cache_key_raw("https://example.com", Some("POST"), None, Some("gb")),
            "POST:https://example.com::ns=gb"
        );

        // Different namespaces never collide on the same URL.
        assert_ne!(
            create_cache_key_raw("https://example.com", None, None, Some("us")),
            create_cache_key_raw("https://example.com", None, None, Some("gb")),
        );
    }

    #[cfg(feature = "cache_chrome_hybrid")]
    #[tokio::test]
    async fn test_fetch_page_html_raw_cached_uses_seeded_cache_entry() {
        use std::collections::HashMap;

        let target_url = "https://cache-unit-test.invalid/path";
        let cache_key = create_cache_key_raw(target_url, None, None, None);

        let mut response_headers = HashMap::new();
        response_headers.insert("accept-language".to_string(), "en-US".to_string());
        response_headers.insert("content-type".to_string(), "text/html".to_string());
        response_headers.insert(
            "cache-control".to_string(),
            "public, max-age=3600".to_string(),
        );

        let body = b"<html><body>cached-response</body></html>".to_vec();
        let http_response = HttpResponse {
            body,
            headers: response_headers.clone(),
            status: 200,
            url: Url::parse(target_url).expect("valid url"),
            version: HttpVersion::Http11,
        };

        let request_headers = HashMap::new();

        put_hybrid_cache(&cache_key, http_response, "GET", request_headers).await;

        let client = reqwest_middleware::ClientBuilder::new(
            reqwest::ClientBuilder::new()
                .build()
                .expect("build reqwest client"),
        )
        .build();

        let cache_options = Some(CacheOptions::Yes);
        let cache_policy = None;

        let page =
            fetch_page_html_raw_cached(target_url, &client, cache_options, &cache_policy, None)
                .await;
        assert_eq!(page.status_code, StatusCode::OK);

        let content = String::from_utf8_lossy(
            page.content
                .as_ref()
                .expect("cached response content")
                .as_ref(),
        );
        assert!(content.contains("cached-response"));
    }

    #[cfg(feature = "cache_chrome_hybrid")]
    #[tokio::test]
    async fn test_fetch_page_html_raw_cached_performance_seeded_vs_network() {
        use std::collections::HashMap;
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::time::Duration as StdDuration;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind local delayed server");
        let addr = listener.local_addr().expect("read local addr");

        let response_body = "<html><body>network-delayed-response</body></html>".to_string();
        let response_body_clone = response_body.clone();

        let server_thread = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept local connection");
            let mut request_buf = [0_u8; 1024];
            let _ = stream.read(&mut request_buf);
            std::thread::sleep(StdDuration::from_millis(350));

            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_body_clone.len(),
                response_body_clone
            );

            stream
                .write_all(response.as_bytes())
                .expect("write delayed response");
            stream.flush().expect("flush delayed response");
        });

        let target_url = format!("http://{}/perf-cache-test", addr);

        let client = reqwest_middleware::ClientBuilder::new(
            reqwest::ClientBuilder::new()
                .build()
                .expect("build reqwest client"),
        )
        .build();

        let network_start = tokio::time::Instant::now();
        let network_page = fetch_page_html_raw(&target_url, &client).await;
        let network_duration = network_start.elapsed();

        server_thread
            .join()
            .expect("join delayed local server thread");

        assert_eq!(network_page.status_code, StatusCode::OK);

        let cache_key = create_cache_key_raw(&target_url, None, None, None);
        let mut response_headers = HashMap::new();
        response_headers.insert("content-type".to_string(), "text/html".to_string());
        response_headers.insert(
            "cache-control".to_string(),
            "public, max-age=3600".to_string(),
        );

        let http_response = HttpResponse {
            body: response_body.into_bytes(),
            headers: response_headers,
            status: 200,
            url: Url::parse(&target_url).expect("valid cache url"),
            version: HttpVersion::Http11,
        };

        let request_headers = HashMap::new();

        put_hybrid_cache(&cache_key, http_response, "GET", request_headers).await;

        let cache_options = Some(CacheOptions::SkipBrowser);
        let cache_policy = None;

        let cached_start = tokio::time::Instant::now();
        let cached_page =
            fetch_page_html_raw_cached(&target_url, &client, cache_options, &cache_policy, None)
                .await;
        let cached_duration = cached_start.elapsed();

        assert_eq!(cached_page.status_code, StatusCode::OK);
        assert!(
            cached_duration < network_duration,
            "expected cached path to be faster (network={}ms cached={}ms)",
            network_duration.as_millis(),
            cached_duration.as_millis()
        );

        let cached_secs = cached_duration.as_secs_f64().max(0.000_001);
        let speedup = network_duration.as_secs_f64() / cached_secs;

        eprintln!(
            "cache performance: network={}ms cached={}ms speedup={:.2}x",
            network_duration.as_millis(),
            cached_duration.as_millis(),
            speedup
        );
    }

    /// Verify that Chrome-rendered pages with no-cache are still cacheable.
    /// put_hybrid_cache overrides no-cache → max-age=172800 for the policy,
    /// so Period(now - 2d) correctly treats recently-stored entries as fresh.
    #[cfg(any(feature = "cache_chrome_hybrid", feature = "cache_chrome_hybrid_mem"))]
    #[tokio::test]
    async fn test_put_hybrid_cache_overrides_no_cache_for_policy() {
        use std::collections::HashMap;

        let target_url = "https://no-cache-override.test/page";
        let cache_key = create_cache_key_raw(target_url, None, None, None);

        let mut response_headers = HashMap::new();
        response_headers.insert("content-type".to_string(), "text/html".to_string());
        response_headers.insert("cache-control".to_string(), "no-cache".to_string());

        let body = b"<html><body>no-cache-but-cacheable</body></html>".to_vec();
        let http_response = HttpResponse {
            body,
            headers: response_headers,
            status: 200,
            url: Url::parse(target_url).expect("valid url"),
            version: HttpVersion::Http11,
        };

        put_hybrid_cache(&cache_key, http_response, "GET", HashMap::new()).await;

        // Period(now - 2d): entry was just stored → age ≈ 0 < max_age(172800) → fresh
        let two_days_ago = std::time::SystemTime::now()
            .checked_sub(std::time::Duration::from_secs(2 * 24 * 3600))
            .unwrap();
        let cache_policy_period = Some(super::BasicCachePolicy::Period(two_days_ago));
        let result = get_cached_url_base(
            target_url,
            Some(CacheOptions::SkipBrowser),
            &cache_policy_period,
            None,
        )
        .await;
        assert!(
            result.is_some(),
            "no-cache response should be cached via policy override"
        );
        assert!(result.unwrap().contains("no-cache-but-cacheable"));

        // Normal policy also returns it (freshly stored, max-age=172800 > age≈0)
        let cache_policy_normal = Some(super::BasicCachePolicy::Normal);
        let result_normal = get_cached_url_base(
            target_url,
            Some(CacheOptions::SkipBrowser),
            &cache_policy_normal,
            None,
        )
        .await;
        assert!(
            result_normal.is_some(),
            "freshly stored entry should be fresh under Normal policy"
        );
    }

    /// Verify no-store is also overridden on write.
    #[cfg(any(feature = "cache_chrome_hybrid", feature = "cache_chrome_hybrid_mem"))]
    #[tokio::test]
    async fn test_put_hybrid_cache_overrides_no_store_for_policy() {
        use std::collections::HashMap;

        let target_url = "https://no-store-override.test/page";
        let cache_key = create_cache_key_raw(target_url, None, None, None);

        let mut response_headers = HashMap::new();
        response_headers.insert("content-type".to_string(), "text/html".to_string());
        response_headers.insert("cache-control".to_string(), "no-store".to_string());

        let body = b"<html><body>no-store-but-cacheable</body></html>".to_vec();
        let http_response = HttpResponse {
            body,
            headers: response_headers,
            status: 200,
            url: Url::parse(target_url).expect("valid url"),
            version: HttpVersion::Http11,
        };

        put_hybrid_cache(&cache_key, http_response, "GET", HashMap::new()).await;

        let two_days_ago = std::time::SystemTime::now()
            .checked_sub(std::time::Duration::from_secs(2 * 24 * 3600))
            .unwrap();
        let cache_policy_period = Some(super::BasicCachePolicy::Period(two_days_ago));
        let result = get_cached_url_base(
            target_url,
            Some(CacheOptions::SkipBrowser),
            &cache_policy_period,
            None,
        )
        .await;
        assert!(
            result.is_some(),
            "no-store response should be cached via policy override"
        );
        assert!(result.unwrap().contains("no-store-but-cacheable"));
    }

    /// Verify that last-modified heuristic is respected (no override needed).
    #[cfg(any(feature = "cache_chrome_hybrid", feature = "cache_chrome_hybrid_mem"))]
    #[tokio::test]
    async fn test_put_hybrid_cache_respects_last_modified_heuristic() {
        use std::collections::HashMap;

        let target_url = "https://last-modified-heuristic.test/page";
        let cache_key = create_cache_key_raw(target_url, None, None, None);

        let mut response_headers = HashMap::new();
        response_headers.insert("content-type".to_string(), "text/html".to_string());
        // No cache-control, but has last-modified → heuristic gives max_age
        response_headers.insert(
            "last-modified".to_string(),
            "Wed, 08 Feb 2023 21:02:33 GMT".to_string(),
        );

        let body = b"<html><body>heuristic-cached</body></html>".to_vec();
        let http_response = HttpResponse {
            body,
            headers: response_headers,
            status: 200,
            url: Url::parse(target_url).expect("valid url"),
            version: HttpVersion::Http11,
        };

        put_hybrid_cache(&cache_key, http_response, "GET", HashMap::new()).await;

        // last-modified from 2023 → heuristic max-age ≈ 109 days → fresh
        let two_days_ago = std::time::SystemTime::now()
            .checked_sub(std::time::Duration::from_secs(2 * 24 * 3600))
            .unwrap();
        let cache_policy_period = Some(super::BasicCachePolicy::Period(two_days_ago));
        let result = get_cached_url_base(
            target_url,
            Some(CacheOptions::SkipBrowser),
            &cache_policy_period,
            None,
        )
        .await;
        assert!(
            result.is_some(),
            "last-modified heuristic should make entry fresh"
        );
        assert!(result.unwrap().contains("heuristic-cached"));
    }

    /// Verify that Set-Cookie doesn't prevent caching (shared=false).
    #[cfg(any(feature = "cache_chrome_hybrid", feature = "cache_chrome_hybrid_mem"))]
    #[tokio::test]
    async fn test_put_hybrid_cache_set_cookie_does_not_block() {
        use std::collections::HashMap;

        let target_url = "https://set-cookie-cache.test/page";
        let cache_key = create_cache_key_raw(target_url, None, None, None);

        let mut response_headers = HashMap::new();
        response_headers.insert("content-type".to_string(), "text/html".to_string());
        response_headers.insert(
            "cache-control".to_string(),
            "public, max-age=3600".to_string(),
        );
        response_headers.insert(
            "set-cookie".to_string(),
            "session=abc123; Path=/".to_string(),
        );

        let body = b"<html><body>set-cookie-cached</body></html>".to_vec();
        let http_response = HttpResponse {
            body,
            headers: response_headers,
            status: 200,
            url: Url::parse(target_url).expect("valid url"),
            version: HttpVersion::Http11,
        };

        put_hybrid_cache(&cache_key, http_response, "GET", HashMap::new()).await;

        let two_days_ago = std::time::SystemTime::now()
            .checked_sub(std::time::Duration::from_secs(2 * 24 * 3600))
            .unwrap();
        let cache_policy_period = Some(super::BasicCachePolicy::Period(two_days_ago));
        let result = get_cached_url_base(
            target_url,
            Some(CacheOptions::SkipBrowser),
            &cache_policy_period,
            None,
        )
        .await;
        assert!(
            result.is_some(),
            "Set-Cookie should not block caching with shared=false"
        );
        assert!(result.unwrap().contains("set-cookie-cached"));
    }

    #[test]
    fn test_is_cacheable_body_empty_truly_empty() {
        assert!(is_cacheable_body_empty(b""));
        assert!(is_cacheable_body_empty(b"   "));
        assert!(is_cacheable_body_empty(b"\n\t  \r\n"));
    }

    #[test]
    fn test_is_cacheable_body_empty_skeleton_html() {
        assert!(is_cacheable_body_empty(
            b"<html><head></head><body></body></html>"
        ));
        assert!(is_cacheable_body_empty(b"<html></html>"));
    }

    #[test]
    fn test_is_cacheable_body_empty_html_empty_body() {
        assert!(is_cacheable_body_empty(
            b"<html><head><title>x</title></head><body>   </body></html>"
        ));
    }

    #[test]
    fn test_is_cacheable_body_empty_html_with_content() {
        assert!(!is_cacheable_body_empty(
            b"<html><body><p>Hello</p></body></html>"
        ));
    }

    #[test]
    fn test_is_cacheable_body_empty_json_skips_html_checks() {
        assert!(!is_cacheable_body_empty(b"{}"));
        assert!(!is_cacheable_body_empty(b"{\"key\": \"value\"}"));
        assert!(!is_cacheable_body_empty(b"[1,2,3]"));
        assert!(!is_cacheable_body_empty(b"null"));
    }

    #[test]
    fn test_is_cacheable_body_empty_css_js_skip_html_checks() {
        assert!(!is_cacheable_body_empty(b"body { color: red; }"));
        assert!(!is_cacheable_body_empty(b"function foo() { return 1; }"));
        assert!(!is_cacheable_body_empty(b"export default {}"));
    }

    #[test]
    fn test_is_cacheable_body_empty_binary_skip_html_checks() {
        // PNG header
        assert!(!is_cacheable_body_empty(&[
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A
        ]));
        // JPEG header
        assert!(!is_cacheable_body_empty(&[0xFF, 0xD8, 0xFF, 0xE0]));
        // Arbitrary binary
        assert!(!is_cacheable_body_empty(&[0x00, 0x01, 0x02, 0x03]));
    }

    #[cfg(any(
        feature = "cache",
        feature = "cache_mem",
        feature = "chrome_remote_cache"
    ))]
    #[test]
    fn test_decode_cached_html_bytes_rejects_empty_html() {
        // Empty shell HTML must be treated as a cache miss (returns None)
        assert!(
            decode_cached_html_bytes(b"<html><head></head><body></body></html>", None).is_none()
        );
        assert!(decode_cached_html_bytes(b"<html></html>", None).is_none());
        assert!(decode_cached_html_bytes(b"", None).is_none());
        assert!(decode_cached_html_bytes(b"   ", None).is_none());
        // Real content must still be returned
        assert!(
            decode_cached_html_bytes(b"<html><body><p>Hello</p></body></html>", None).is_some()
        );
    }

    /// Verify that the CDP event listener shutdown pattern (watch channel +
    /// tokio::select!) exits promptly when signaled, even if the underlying
    /// stream never closes.
    #[tokio::test]
    async fn test_cdp_listener_shutdown_exits_promptly() {
        use std::time::{Duration, Instant};

        let (shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(false);

        // Simulate an event stream that produces items slowly and never closes.
        let (event_tx, mut event_rx) = tokio::sync::mpsc::channel::<u64>(16);

        // Producer: sends one event per 100ms, runs for 60s (effectively forever).
        let producer = tokio::spawn(async move {
            for i in 0u64.. {
                if event_tx.send(i).await.is_err() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        });

        // Consumer: mirrors the real CDP listener loop pattern.
        let consumer = tokio::spawn(async move {
            let mut collected = Vec::new();
            loop {
                let event = tokio::select! {
                    biased;
                    _ = shutdown_rx.changed() => break,
                    ev = event_rx.recv() => match ev {
                        Some(ev) => ev,
                        None => break,
                    },
                };
                collected.push(event);
            }
            collected
        });

        // Let it collect a few events.
        tokio::time::sleep(Duration::from_millis(350)).await;

        // Signal shutdown and measure how long it takes to exit.
        let start = Instant::now();
        let _ = shutdown_tx.send(true);
        let collected = consumer.await.unwrap();
        let exit_time = start.elapsed();

        producer.abort();

        // Must have collected some events before shutdown.
        assert!(
            !collected.is_empty(),
            "should have collected events before shutdown"
        );
        // Must exit within 50ms of the signal (not waiting for stream to close).
        assert!(
            exit_time < Duration::from_millis(50),
            "listener should exit promptly after shutdown signal, took {:?}",
            exit_time
        );
    }

    /// Verify that without a shutdown signal the listener exits naturally
    /// when the stream closes (no regression on normal flow).
    #[tokio::test]
    async fn test_cdp_listener_exits_on_stream_close() {
        use std::time::{Duration, Instant};

        let (_shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(false);
        let (event_tx, mut event_rx) = tokio::sync::mpsc::channel::<u64>(16);

        let consumer = tokio::spawn(async move {
            let mut collected = Vec::new();
            loop {
                let event = tokio::select! {
                    biased;
                    _ = shutdown_rx.changed() => break,
                    ev = event_rx.recv() => match ev {
                        Some(ev) => ev,
                        None => break,
                    },
                };
                collected.push(event);
            }
            collected
        });

        // Send 5 events then drop the sender (closes the stream).
        for i in 0..5 {
            event_tx.send(i).await.unwrap();
        }
        drop(event_tx);

        let start = Instant::now();
        let collected = consumer.await.unwrap();
        let exit_time = start.elapsed();

        assert_eq!(
            collected,
            vec![0, 1, 2, 3, 4],
            "all events should be collected"
        );
        assert!(
            exit_time < Duration::from_millis(50),
            "should exit promptly on stream close, took {:?}",
            exit_time
        );
    }

    /// Verify the tokio::join! pattern with multiple listeners all exit
    /// when shutdown is signaled, even if only some streams close naturally.
    #[tokio::test]
    async fn test_cdp_listener_join_all_exit_on_shutdown() {
        use std::time::{Duration, Instant};

        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

        // 3 streams: first closes naturally, second and third never close.
        let (tx1, mut rx1) = tokio::sync::mpsc::channel::<u64>(4);
        let (_tx2, mut rx2) = tokio::sync::mpsc::channel::<u64>(4); // never sends
        let (_tx3, mut rx3) = tokio::sync::mpsc::channel::<u64>(4); // never sends

        let mut sr1 = shutdown_rx.clone();
        let mut sr2 = shutdown_rx.clone();
        let mut sr3 = shutdown_rx;

        let handle = tokio::spawn(async move {
            let f1 = async {
                let mut count = 0u64;
                loop {
                    tokio::select! {
                        biased;
                        _ = sr1.changed() => break,
                        ev = rx1.recv() => match ev {
                            Some(_) => count += 1,
                            None => break,
                        },
                    }
                }
                count
            };
            let f2 = async {
                loop {
                    tokio::select! {
                        biased;
                        _ = sr2.changed() => break,
                        ev = rx2.recv() => if ev.is_none() { break },
                    }
                }
            };
            let f3 = async {
                loop {
                    tokio::select! {
                        biased;
                        _ = sr3.changed() => break,
                        ev = rx3.recv() => if ev.is_none() { break },
                    }
                }
            };
            let (count, _, _) = tokio::join!(f1, f2, f3);
            count
        });

        // Send some events to f1, then close it.
        for i in 0..3 {
            tx1.send(i).await.unwrap();
        }
        drop(tx1);

        // f2 and f3 never get events — they'd hang without shutdown.
        tokio::time::sleep(Duration::from_millis(50)).await;

        let start = Instant::now();
        let _ = shutdown_tx.send(true);
        let count = handle.await.unwrap();
        let exit_time = start.elapsed();

        assert_eq!(count, 3, "f1 should have collected all events");
        assert!(
            exit_time < Duration::from_millis(50),
            "tokio::join! should complete promptly after shutdown, took {:?}",
            exit_time
        );
    }
}
