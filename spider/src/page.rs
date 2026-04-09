#[cfg(all(feature = "chrome", not(feature = "decentralized")))]
use crate::configuration::{AutomationScripts, ExecutionScripts};
#[cfg(all(not(feature = "decentralized"), feature = "chrome"))]
use crate::features::automation::RemoteMultimodalConfigs;
use crate::utils::abs::convert_abs_path;
use crate::utils::templates::EMPTY_HTML_BASIC;
#[cfg(not(feature = "decentralized"))]
use crate::utils::RequestError;
use crate::utils::{
    css_selectors::{compiled_base_element_selector, compiled_selector, compiled_xml_selector},
    get_domain_from_url, hash_html, networking_capable, BasicCachePolicy, CacheOptions,
    PageResponse,
};
use crate::CaseInsensitiveString;
use crate::Client;
use crate::RelativeSelectors;
use crate::{compact_str::CompactString, utils::templates::EMPTY_HTML};
use auto_encoder::auto_encode_bytes;
use hashbrown::HashSet;
use lol_html::AsciiCompatibleEncoding;
use phf::phf_set;
use regex::bytes::Regex;
use reqwest::StatusCode;
use std::sync::Arc;
use tokio::time::Duration;
#[cfg(feature = "time")]
use tokio::time::Instant;

#[cfg(all(feature = "decentralized", feature = "headers"))]
use crate::utils::FetchPageResult;
use lazy_static::lazy_static;
use url::Url;

/// Construct an element content handler tuple using a pre-compiled `&'static Selector`.
/// Same as `lol_html::element!` but uses `Cow::Borrowed` to avoid re-parsing.
macro_rules! element_precompiled {
    ($selector:expr, $handler:expr) => {{
        #[inline(always)]
        const fn type_hint<'h, T, H: lol_html::HandlerTypes>(h: T) -> T
        where
            T: FnMut(&mut lol_html::html_content::Element<'_, '_, H>) -> lol_html::HandlerResult
                + 'h,
        {
            h
        }
        (
            std::borrow::Cow::Borrowed($selector),
            lol_html::send::ElementContentHandlers::default().element(type_hint($handler)),
        )
    }};
}

/// Allocate up to 128kb upfront for small pages.
pub(crate) const MAX_PRE_ALLOCATED_HTML_PAGE_SIZE: u64 = 128 * 1024;
/// Allocate up to 128kb upfront for small pages.
pub(crate) const MAX_PRE_ALLOCATED_HTML_PAGE_SIZE_USIZE: usize =
    MAX_PRE_ALLOCATED_HTML_PAGE_SIZE as usize;

/// Allocate up to 16kb * 4 upfront for small pages.
#[cfg(feature = "chrome")]
pub(crate) const TURNSTILE_WALL_PAGE_SIZE: usize = MAX_PRE_ALLOCATED_HTML_PAGE_SIZE_USIZE * 4;

lazy_static! {
    /// Wildcard match all domains.
    static ref CASELESS_WILD_CARD: CaseInsensitiveString = CaseInsensitiveString::new("*");
    static ref SSG_CAPTURE: Regex =  Regex::new(r#""(.*?)""#).unwrap();
    /// Gatsby
    static ref GATSBY: Option<&'static str> =  Some("gatsby-chunk-mapping");
    /// Nuxt.
    static ref NUXT_DATA: Option<&'static str> =  Some("__NUXT_DATA__");
    /// Nuxt.
    static ref NUXT: Option<&'static str> =  Some("__nuxt");
    /// React ssr app.
    static ref REACT_SSR: Option<&'static str> =  Some("react-app.embeddedData");
    /// Unknown status (generic fallback)
    pub(crate) static ref UNKNOWN_STATUS_ERROR: StatusCode =
        StatusCode::from_u16(599).expect("valid status code");
    /// Chrome-style timeout / network unknown
    pub(crate) static ref CHROME_UNKNOWN_STATUS_ERROR: StatusCode =
        StatusCode::from_u16(598).expect("valid status code");
    /// Connection timeout
    pub(crate) static ref CONNECTION_TIMEOUT_ERROR: StatusCode =
        StatusCode::from_u16(524).expect("valid status code");
    /// Connection refused (origin down)
    pub(crate) static ref CONNECTION_REFUSED_ERROR: StatusCode =
        StatusCode::from_u16(521).expect("valid status code");
    /// Connection aborted
    pub(crate) static ref CONNECTION_ABORTED_ERROR: StatusCode =
        StatusCode::from_u16(522).expect("valid status code");
    /// Connection reset
    pub(crate) static ref CONNECTION_RESET_ERROR: StatusCode =
        StatusCode::from_u16(523).expect("valid status code");
    /// DNS failure
    pub(crate) static ref DNS_RESOLVE_ERROR: StatusCode =
        StatusCode::from_u16(525).expect("valid status code");
    /// Body decode failure
    pub(crate) static ref BODY_DECODE_ERROR: StatusCode =
        StatusCode::from_u16(400).expect("valid status code");
    /// Request malformed or unreachable
    pub(crate) static ref UNREACHABLE_REQUEST_ERROR: StatusCode =
        StatusCode::from_u16(503).expect("valid status code");
}

lazy_static! {
    /// Aho-Corasick automaton for DNS error detection — single O(n) scan.
    static ref DNS_ERROR_AC: aho_corasick::AhoCorasick = aho_corasick::AhoCorasick::new([
        "dns error",
        "failed to lookup address",
        "Name or service not known",
        "No address associated with hostname",
        "ENOTFOUND",
    ]).expect("valid patterns");
}

/// Check if a connect error is a DNS resolution failure.
/// Zero-alloc fast path via `downcast_ref`, single-alloc fallback via Aho-Corasick O(n) scan.
/// Only called when `err.is_connect()` is already true.
fn is_dns_error(err: &crate::client::Error) -> bool {
    use std::error::Error;

    // Fast path: walk source chain looking for io::Error with specific kinds.
    // Hickory DNS surfaces as: reqwest → hyper → io::Error(Custom("dns error: ..."))
    let mut source: Option<&(dyn Error + 'static)> = err.source();
    let mut depth = 0u8;
    while let Some(e) = source {
        if depth >= 6 {
            break;
        }
        if let Some(io_err) = e.downcast_ref::<std::io::Error>() {
            if matches!(io_err.kind(), std::io::ErrorKind::NotFound) {
                return true;
            }
        }
        source = e.source();
        depth += 1;
    }

    // Slow fallback: single to_string() + single-pass Aho-Corasick scan.
    DNS_ERROR_AC.is_match(&err.to_string())
}

/// Whether a status code is retryable (transient server/network errors).
/// DNS errors (525) are permanent and excluded.
#[inline]
pub fn is_retryable_status(status: StatusCode) -> bool {
    status != *DNS_RESOLVE_ERROR
        && (status.is_server_error()
            || matches!(
                status,
                StatusCode::TOO_MANY_REQUESTS | StatusCode::REQUEST_TIMEOUT
            ))
}

/// Get the HTTP status code of errors.

pub(crate) fn get_error_http_status_code(err: &crate::client::Error) -> StatusCode {
    use std::error::Error;
    use std::io;

    if let Some(status) = err.status() {
        return status;
    }

    if err.is_timeout() {
        return *CONNECTION_TIMEOUT_ERROR;
    }

    if err.is_connect() {
        // DNS resolution failure — never retried
        if is_dns_error(err) {
            return *DNS_RESOLVE_ERROR;
        }
        if let Some(io_err) = err.source().and_then(|e| e.downcast_ref::<io::Error>()) {
            match io_err.kind() {
                io::ErrorKind::ConnectionRefused => return *CONNECTION_REFUSED_ERROR,
                io::ErrorKind::ConnectionAborted => return *CONNECTION_ABORTED_ERROR,
                io::ErrorKind::ConnectionReset => return *CONNECTION_RESET_ERROR,
                io::ErrorKind::NotFound => return *UNREACHABLE_REQUEST_ERROR,
                io::ErrorKind::HostUnreachable | io::ErrorKind::NetworkUnreachable => {
                    return *UNREACHABLE_REQUEST_ERROR
                }
                io::ErrorKind::TimedOut => return *CONNECTION_TIMEOUT_ERROR,
                _ => (),
            }
        }
        return *UNREACHABLE_REQUEST_ERROR;
    }

    if err.is_body() {
        return *BODY_DECODE_ERROR;
    }

    if err.is_request() {
        return StatusCode::BAD_REQUEST;
    }

    *UNKNOWN_STATUS_ERROR
}

/// Check if a script src URL is a known ad, tracker, or analytics script.
/// Uses `spider_network_blocker` trie for absolute URLs and `ADBLOCK_PATTERNS`
/// substring matching for relative paths. Returns `true` if the script should
/// be excluded from upgrade scoring (i.e., it is a tracker/ad, not app logic).
#[cfg(all(not(feature = "decentralized"), feature = "smart"))]
#[inline]
fn is_tracker_script(src: &str) -> bool {
    use chromiumoxide::spider_network_blocker;
    if src.starts_with("http") {
        // Absolute URL — trie prefix match against known tracker domains.
        spider_network_blocker::scripts::URL_IGNORE_TRIE.contains_prefix(src)
    } else {
        // Relative path — substring match against adblock patterns.
        spider_network_blocker::adblock::ADBLOCK_PATTERNS
            .iter()
            .any(|p| src.contains(p))
    }
}

#[cfg(all(not(feature = "decentralized"), feature = "smart"))]
lazy_static! {
    /// General no script patterns.
    static ref NO_SCRIPT_JS_REQUIRED: aho_corasick::AhoCorasick = {
        let patterns = &[
            // JS-required / SPA shell markers
            "enable javascript", "requires javascript", "turn on javascript",
        ];
        aho_corasick::AhoCorasick::new(patterns).expect("valid dom script  patterns")
    };

    /// Methods that cause the dom to mutate.
    static ref DOM_SCRIPT_WATCH_METHODS: aho_corasick::AhoCorasick = {
        let patterns = &[
            ".createElementNS", ".removeChild", ".insertBefore", ".createElement(",
            ".createTextNode", ".replaceChildren(", ".prepend(",
            ".appendChild(", "document.write(", "window.location.href",
            // DOM mutation hot paths
            ".innerHTML", ".outerHTML", ".insertAdjacentHTML(", ".insertAdjacentElement(",
            ".replaceWith(", ".replaceChild(", ".cloneNode(",
            "new DOMParser",
            // SPA routing
            "history.pushState", "history.replaceState",
            "location.assign(", "location.replace(",
            "window.location=", "document.location=",
            // Fetching required
            "fetch(", "new XMLHttpRequest",
            // APPS
            "window.__NUXT__"
        ];
        aho_corasick::AhoCorasick::new(patterns).expect("valid dom script patterns")
    };

    /// Attributes for JS requirements.
    static ref DOM_WATCH_ATTRIBUTE_PATTERNS: [&'static str; 5] = [
        "__NEXT_DATA__", "__NUXT__", "data-reactroot",
        "ng-version", "data-v-app",
    ];

    /// Hydration required
    pub(crate) static ref HYDRATION_IDS: phf::Set<&'static str> = phf_set! {
        "__nuxt",
        "__nuxt-loader",
        "__NUXT_DATA__",
        "__next",
        "__NEXT_DATA__",
        "___gatsby",
        "redwood-app",
        "sapper"
    };
}

lazy_static! {
    /// Downloadable media types [https://developer.mozilla.org/en-US/docs/Web/HTTP/Guides/MIME_types/Common_types].
    pub(crate) static ref DOWNLOADABLE_MEDIA_TYPES: phf::Set<&'static str> = phf_set! {
        // --- Audio ---
        "audio/mpeg",        // mp3
        "audio/wav",         // wav
        "audio/ogg",         // ogg/oga/opus
        "audio/flac",        // flac
        "audio/aac",         // aac
        "audio/webm",        // weba
        "audio/midi",        // mid/midi
        "audio/x-midi",      // mid/midi (common)
        "audio/mp4",         // m4a (common)
        "audio/x-m4a",       // m4a (common)
        "audio/aiff",        // aiff (common)
        "audio/x-aiff",      // aiff (common)
        "audio/3gpp",        // 3gp (audio-only)
        "audio/3gpp2",       // 3g2 (audio-only)
        // --- Video ---
        "video/mp4",         // mp4/m4v
        "video/webm",        // webm
        "video/ogg",         // ogv
        "video/x-matroska",  // mkv
        "video/x-msvideo",   // avi
        "video/quicktime",   // mov
        "video/x-ms-wmv",    // wmv
        "video/x-flv",       // flv
        "video/mpeg",        // mpeg
        "video/mp2t",        // ts
        "video/3gpp",        // 3gp (video)
        "video/3gpp2",       // 3g2 (video)
        // --- Images ---
        "image/jpeg",        // jpg/jpeg
        "image/png",         // png
        "image/gif",         // gif
        "image/webp",        // webp
        "image/svg+xml",     // svg
        "image/bmp",         // bmp
        "image/tiff",        // tif/tiff
        "image/vnd.microsoft.icon", // ico
        "image/apng",        // apng
        "image/avif",        // avif
        "image/heic",        // heic (common)
        "image/heif",        // heif (common)
        // --- Fonts ---
        "font/woff",         // woff
        "font/woff2",        // woff2
        "font/ttf",          // ttf
        "font/otf",          // otf
        "application/vnd.ms-fontobject", // eot
        // --- Documents / ebooks ---
        "application/pdf",   // pdf
        "application/rtf",   // rtf
        "text/plain",        // txt
        "text/csv",          // csv
        "text/markdown",     // md
        "text/calendar",     // ics
        "application/msword", // doc
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document", // docx
        "application/vnd.ms-powerpoint", // ppt
        "application/vnd.openxmlformats-officedocument.presentationml.presentation", // pptx
        "application/vnd.ms-excel", // xls
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet", // xlsx
        "application/vnd.oasis.opendocument.text", // odt
        "application/vnd.oasis.opendocument.spreadsheet", // ods
        "application/vnd.oasis.opendocument.presentation", // odp
        "application/vnd.visio", // vsd
        "application/epub+zip", // epub
        "application/vnd.amazon.ebook", // azw
        "application/x-abiword", // abw
        // --- Archives / binaries / installers ---
        "application/octet-stream", // generic binary downloads
        "application/zip",          // zip
        "application/x-zip-compressed", // zip (common non-standard)
        "application/vnd.rar",      // rar
        "application/x-rar-compressed", // rar (common)
        "application/x-7z-compressed", // 7z
        "application/x-tar",        // tar
        "application/gzip",         // gz
        "application/x-gzip",       // gz (common)
        "application/x-bzip",       // bz
        "application/x-bzip2",      // bz2
        "application/java-archive", // jar
        "application/x-freearc",    // arc
        "application/vnd.apple.installer+xml", // mpkg
        // --- Ogg container ---
        "application/ogg",          // ogx
    };

    /// Visual assets to ignore.
    pub(crate) static ref IGNORE_ASSETS: HashSet<CaseInsensitiveString> = {
        let mut m: HashSet<CaseInsensitiveString> = HashSet::with_capacity(90);

        m.extend([
            "jpg", "jpeg", "png", "gif", "svg", "webp",      // Image files
            "mp4", "avi", "mov", "wmv", "flv",               // Video files
            "mp3", "wav", "ogg",                             // Audio files
            "woff", "woff2", "ttf", "otf",                   // Font files
            "swf", "xap",                                    // Flash/Silverlight files
            "ico", "eot",                                    // Other resource files
            "bmp", "tiff", "tif", "heic", "heif",            // Additional Image files
            "mkv", "webm", "m4v",                            // Additional Video files
            "aac", "flac", "m4a", "aiff",                    // Additional Audio files
            "pdf", "eps", "yaml", "yml", "rtf", "txt",       // Other additional files
            "doc", "docx", "csv", "eot", "epub", "gz",
            "ics", "md", "webmanifest",
            "apng", "avif",
            "cda", "mid", "midi", "oga", "ogv", "ogx", "opus", "weba", "mpeg", "ts", "3gp", "3g2",
            "arc", "bin", "bz", "bz2", "jar", "mpkg", "rar", "tar", "zip", "7z",
            "abw", "azw", "odt", "ods", "odp", "ppt", "pptx", "xls", "xlsx", "vsd",
            ".jpg", ".jpeg", ".png", ".gif", ".svg", ".webp",
            ".mp4", ".avi", ".mov", ".wmv", ".flv",
            ".mp3", ".wav", ".ogg",
            ".woff", ".woff2", ".ttf", ".otf",
            ".swf", ".xap",
            ".ico", ".eot",
            ".bmp", ".tiff", ".tif", ".heic", ".heif",
            ".mkv", ".webm", ".m4v",
            ".aac", ".flac", ".m4a", ".aiff",
            ".pdf", ".eps", ".yaml", ".yml", ".rtf", ".txt",
            ".doc", ".docx", ".csv", ".eot", ".epub", ".gz",
            ".apng", ".avif", ".ics", ".md", ".webmanifest",
            ".cda", ".mid", ".midi", ".oga", ".ogv", ".ogx", ".opus", ".weba", ".mpeg", ".ts", ".3gp", ".3g2",
            ".arc", ".bin", ".bz", ".bz2", ".jar", ".mpkg", ".rar", ".tar", ".zip", ".7z",
            ".abw", ".azw", ".odt", ".ods", ".odp", ".ppt", ".pptx", ".xls", ".xlsx", ".vsd",
        ].map(|s| s.into()));

        m
    };

    /// The chunk size for the rewriter. Can be adjusted using the env var "SPIDER_STREAMING_CHUNK_SIZE".
    pub(crate) static ref STREAMING_CHUNK_SIZE: usize = {
        let default_streaming_chunk_size: usize = (8192 * num_cpus::get_physical().min(64)).min(65536);
        let min_streaming_chunk_size: usize = default_streaming_chunk_size * 2 / 3;

        std::env::var("SPIDER_STREAMING_CHUNK_SIZE")
            .ok()
            .and_then(|val| val.parse::<usize>().ok())
            .map(|val| {
                if val < min_streaming_chunk_size {
                    min_streaming_chunk_size
                } else {
                    val
                }
            })
            .unwrap_or(default_streaming_chunk_size)
    };
}

/// Global EMA of links-per-page. Used to pre-size extraction HashSets and
/// avoid repeated rehashing on link-dense sites. Lock-free, race-safe:
/// worst case a slightly stale hint, which is still better than a static 32.
static LINK_CAPACITY_HINT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(32);

/// Read the current link capacity hint (minimum 32).
#[inline(always)]
fn link_set_capacity() -> usize {
    LINK_CAPACITY_HINT
        .load(std::sync::atomic::Ordering::Relaxed)
        .max(32)
}

/// Update the EMA after a page extraction. Uses 3:1 weighting (75% old, 25% new).
#[inline(always)]
fn update_link_capacity_hint(count: usize) {
    let prev = LINK_CAPACITY_HINT.load(std::sync::atomic::Ordering::Relaxed);
    let next = if prev == 0 {
        count.max(32)
    } else {
        ((prev * 3 + count) / 4).max(32)
    };
    LINK_CAPACITY_HINT.store(next, std::sync::atomic::Ordering::Relaxed);
}

// Thread-local reusable buffer for XML sitemap parsing.
// Uses Cell take/replace pattern — safe across `.await` points because the
// Vec is taken out (owned) before any await and put back after.
// If the task migrates threads between take and replace, the buffer simply
// lands on a different thread — harmless. Re-entrant calls see an empty Vec
// (no panic, just no reuse for that nested invocation).
thread_local! {
    static XML_PARSE_BUF: std::cell::Cell<Vec<u8>> = const { std::cell::Cell::new(Vec::new()) };
}

/// Byte threshold above which rewriter loops yield to the async runtime.
/// Pages smaller than this are processed without yielding (zero overhead).
pub const REWRITER_YIELD_THRESHOLD: usize = 512 * 1024;

/// How many chunks between yield points for large pages.
pub const REWRITER_YIELD_INTERVAL: usize = 8;

/// The AI data returned from a GPT.
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AIResults {
    /// The prompt used for the GPT.
    pub input: String,
    /// The js output of the GPT response.
    pub js_output: String,
    /// The content output returned from the GPT response that is not a browser script, example: extracted data from the markup.
    pub content_output: Vec<String>,
    /// The image of the page.
    pub screenshot_output: Option<Vec<u8>>,
    /// The error that occured if any.
    pub error: Option<String>,
}

/// Results from automation operations (extraction, observation, etc.).
///
/// This struct stores the output from remote multimodal automation
/// and can be used with both Chrome and HTTP-only crawls.
#[cfg(feature = "serde")]
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct AutomationResults {
    /// The prompt used for the GPT/LLM.
    pub input: String,
    /// The content output returned from the automation.
    pub content_output: serde_json::Value,
    /// The base64 image of the page (if captured).
    pub screenshot_output: Option<String>,
    /// The error that occurred if any.
    pub error: Option<String>,
    /// Token usage for this automation result.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<crate::features::automation::AutomationUsage>,
    /// Whether the page is relevant to crawl goals.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relevant: Option<bool>,
    /// Number of automation steps executed, when available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub steps_executed: Option<usize>,
    /// Optional reasoning text returned by the model.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
}

/// Results from automation operations (extraction, observation, etc.).
#[cfg(not(feature = "serde"))]
#[derive(Debug, Clone, Default)]
pub struct AutomationResults {
    /// The prompt used for the GPT/LLM.
    pub input: String,
    /// The content output returned from the automation (as string without serde).
    pub content_output: String,
    /// The base64 image of the page (if captured).
    pub screenshot_output: Option<String>,
    /// The error that occurred if any.
    pub error: Option<String>,
    /// Token usage for this automation result.
    pub usage: Option<crate::features::automation::AutomationUsage>,
    /// Whether the page is relevant to crawl goals.
    pub relevant: Option<bool>,
    /// Number of automation steps executed, when available.
    pub steps_executed: Option<usize>,
    /// Optional reasoning text returned by the model.
    pub reasoning: Option<String>,
}

/// Page-level metadata extracted from HTML.
#[derive(Debug, Default, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Metadata {
    /// The `<title>` text from the page.
    pub title: Option<CompactString>,
    /// The `<meta name="description">` content.
    pub description: Option<CompactString>,
    /// The Open Graph image URL (`og:image`).
    pub image: Option<CompactString>,
    #[cfg(feature = "chrome")]
    /// The web automation metadata:
    pub automation: Option<Vec<AutomationResults>>, // /// Optional Open Graph metadata (`<meta property="og:*">`) extracted from the page.
                                                    // #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
                                                    // pub og: Option<Box<OpenGraph>>,
}

impl Metadata {
    /// Does metadata exist?
    pub fn exist(&self) -> bool {
        self.title.is_some() || self.description.is_some() || self.image.is_some()
    }
}

// /// Open Graph metadata extracted from `<meta property="og:*">` tags.
// #[derive(Debug, Default, Clone)]
// #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
// pub struct OpenGraph {
//     /// The Open Graph title (`og:title`). NOT USED.
//     #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
//     pub title: Option<CompactString>,
//     /// The Open Graph description (`og:description`). NOT USED.
//     #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
//     pub description: Option<CompactString>,
//     /// The Open Graph image URL (`og:image`).
//     #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
//     pub image: Option<CompactString>,
//     /// The canonical page URL (`og:url`). NOT USED.
//     #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
//     pub url: Option<CompactString>,
//     /// The content type (`og:type`, e.g., "article", "website"). NOT USED.
//     #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
//     pub content_type: Option<CompactString>,
//     /// The site name (`og:site_name`). NOT USED.
//     #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
//     pub site_name: Option<CompactString>,
//     /// The locale of the content (`og:locale`, e.g., "en_US"). NOT USED.
//     #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
//     pub locale: Option<CompactString>,
//     /// The author's name (`article:author` or `og:author`). NOT USED.
//     #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
//     pub author: Option<CompactString>,
//     /// The time the content was first published (`article:published_time`). NOT USED.
//     #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
//     pub published_time: Option<CompactString>,
//     /// The time the content was last modified (`article:modified_time`). NOT USED.
//     #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
//     pub modified_time: Option<CompactString>,
// }

/// Enumeration of known anti-bot and fraud prevention technologies.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum AntiBotTech {
    /// Cloudflare Bot Management - integrated with CDN/WAF, provides behavioral and ML detection.
    Cloudflare,
    /// DataDome - bot protection focused on e-commerce, travel, and classifieds.
    DataDome,
    /// HUMAN (formerly White Ops) - advanced bot mitigation with a focus on ad fraud and Satori threat intelligence.
    HUMAN,
    /// PerimeterX - offers Bot Defender and Code Defender with a strong focus on e-commerce and credential stuffing.
    PerimeterX,
    /// Kasada - bot defense using client-side interrogation and cryptographic challenges.
    Kasada,
    /// FingerprintJS - device fingerprinting and fraud detection via browser and device signal intelligence.
    FingerprintJS,
    /// Arkose Labs - bot mitigation through interactive challenges and risk scoring, used by large enterprises.
    ArkoseLabs,
    /// Imperva - offers bot management as part of a broader WAF/CDN suite with threat intelligence integration.
    Imperva,
    /// F5 - legacy enterprise security vendor offering bot protection through traffic inspection and WAF.
    F5,
    /// Queue-it - virtual waiting room technology to manage surges in traffic and prevent bot scalping.
    QueueIt,
    /// Netacea - behavioral analysis and intent-based detection via server-side interception.
    Netacea,
    /// AppsFlyer - primarily a mobile attribution platform with anti-fraud capabilities.
    AppsFlyer,
    /// Adjust - mobile measurement and analytics provider with fraud prevention modules.
    Adjust,
    /// AppTrana - Indusface is a leading application security SaaS company that secures critical Web, Mobil
    AppTrana,
    /// Akamai Bot Manager - enterprise-grade bot management as part of Akamai’s edge security stack.
    AkamaiBotManager,
    /// Radware Bot Manager - bot detection via intent-based algorithms and real-time profiling.
    RadwareBotManager,
    /// Reblaze - cloud-based web security suite including bot management, WAF, and DDoS mitigation.
    Reblaze,
    /// CHEQ - marketing security and fraud prevention focused on ad spend protection and invalid traffic.
    CHEQ,
    /// Incode - identity verification and fraud prevention with biometric and document validation.
    Incode,
    /// Singula - AI-based marketing and user protection, offering user behavior analysis and fraud insights.
    Singula,
    /// Alibaba TMD (TMall Defense) - anti-bot system used by Alibaba group sites (Taobao, Tmall, Lazada, Miravia, AliExpress).
    AlibabaTMD,
    /// Sucuri Website Firewall - cloud-based WAF and DDoS protection.
    Sucuri,
    /// DDoS-Guard - DDoS protection and CDN service.
    DDoSGuard,
    /// Vercel Firewall - edge protection and security checkpoint for Vercel-hosted sites.
    Vercel,
    /// AWS WAF - Amazon Web Services Web Application Firewall, often paired with CloudFront.
    AwsWaf,
    /// Wordfence - WordPress security plugin with WAF and bot blocking.
    Wordfence,
    /// GeeTest - CAPTCHA provider using slide, click, and behavioral challenges.
    GeeTest,
    /// hCaptcha - privacy-focused CAPTCHA service used as reCAPTCHA alternative.
    HCaptcha,
    /// Custom user-supplied antibot pattern matched (no specific provider identified).
    Custom,
    /// Fallback value if none match or detection failed.
    #[default]
    None,
}

/// Represent a page visited.
#[derive(Debug, Clone, Default)]
#[cfg(not(feature = "decentralized"))]
pub struct Page {
    /// The bytes of the resource.
    pub(crate) html: Option<bytes::Bytes>,
    /// Base absolute url for page.
    pub(crate) base: Option<Url>,
    /// The raw url for the page. Useful since Url::parse adds a trailing slash.
    pub(crate) url: String,
    /// The headers of the page request response.
    pub headers: Option<reqwest::header::HeaderMap>,
    #[cfg(feature = "remote_addr")]
    /// The remote address of the page.
    pub remote_addr: Option<core::net::SocketAddr>,
    #[cfg(feature = "cookies")]
    /// The cookies of the page request response.
    pub cookies: Option<reqwest::header::HeaderMap>,
    /// The status code of the page request.
    pub status_code: StatusCode,
    #[cfg(not(feature = "page_error_status_details"))]
    /// The error of the request if any.
    pub error_status: Option<String>,
    #[cfg(feature = "page_error_status_details")]
    /// The error of the request if any.
    pub error_status: Option<std::sync::Arc<reqwest::Error>>,
    /// The external urls to group with the domain
    pub external_domains_caseless: Arc<HashSet<CaseInsensitiveString>>,
    /// The final destination of the page if redirects were performed [Not implemented in the chrome feature].
    pub final_redirect_destination: Option<String>,
    #[cfg(feature = "time")]
    /// The duration from start of parsing to end of gathering links.
    duration: Option<Instant>,
    #[cfg(feature = "chrome")]
    /// Page object for chrome. The page may be closed when accessing it on another thread from concurrency.
    chrome_page: Option<chromiumoxide::Page>,
    #[cfg(feature = "chrome")]
    /// The screenshot bytes of the page.
    pub screenshot_bytes: Option<Vec<u8>>,
    #[cfg(feature = "openai")]
    /// The credits used from OpenAI in order.
    pub openai_credits_used: Option<Vec<crate::features::openai_common::OpenAIUsage>>,
    #[cfg(feature = "openai")]
    /// The extra data from the AI, example extracting data etc...
    pub extra_ai_data: Option<Vec<AIResults>>,
    #[cfg(feature = "gemini")]
    /// The credits used from Gemini in order.
    pub gemini_credits_used: Option<Vec<crate::features::gemini_common::GeminiUsage>>,
    #[cfg(feature = "gemini")]
    /// The extra data from the Gemini AI.
    pub extra_gemini_data: Option<Vec<AIResults>>,
    /// The usage from remote multimodal automation (extraction, etc.).
    /// Works with both Chrome and HTTP-only crawls.
    pub remote_multimodal_usage: Option<Vec<crate::features::automation::AutomationUsage>>,
    /// The extra data from the remote multimodal automation (extraction results, etc.).
    /// Works with both Chrome and HTTP-only crawls.
    pub extra_remote_multimodal_data: Option<Vec<AutomationResults>>,
    /// URLs requested by automation to spawn as additional pages.
    pub spawn_pages: Option<Vec<String>>,
    /// Additional content keyed by return format (e.g. `"markdown"`, `"text"`).
    /// Populated when multiple formats are requested via
    /// [`SpiderCloudConfig::with_return_formats`](crate::configuration::SpiderCloudConfig::with_return_formats).
    #[cfg(feature = "spider_cloud")]
    pub content_map: Option<hashbrown::HashMap<String, bytes::Bytes>>,
    /// The links found on the page. This includes all links that have an href url.
    pub page_links: Option<Box<HashSet<CaseInsensitiveString>>>,
    /// The request should retry.
    pub should_retry: bool,
    /// A WAF was found on the page.
    pub waf_check: bool,
    /// The total byte transferred for the page. Mainly used for chrome events. Inspect the content for bytes when using http instead.
    pub bytes_transferred: Option<f64>,
    /// The page was blocked from crawling usual from using website::on_should_crawl_callback.
    pub blocked_crawl: bool,
    /// The signature of the page to de-duplicate content.
    pub signature: Option<u64>,
    #[cfg(feature = "chrome")]
    /// All of the response events mapped with the amount of bytes used.
    pub response_map: Option<hashbrown::HashMap<String, f64>>,
    #[cfg(feature = "chrome")]
    /// All of the request events mapped with the time period of the event sent.
    pub request_map: Option<hashbrown::HashMap<String, f64>>,
    /// The anti-bot tech used.
    pub anti_bot_tech: AntiBotTech,
    /// Page metadata.
    pub metadata: Option<Box<Metadata>>,
    /// Whether the response content was truncated due to a stream error,
    /// chunk idle timeout, or Content-Length mismatch.
    pub content_truncated: bool,
    /// Whether a proxy was configured for this request.
    /// When true, 401 responses are retried (proxy rotation may fix auth).
    pub proxy_configured: bool,
    #[cfg(feature = "parallel_backends")]
    /// Identifies which backend produced this page (e.g. "primary",
    /// "lightpanda", "servo"). `None` when parallel backends are not active.
    pub backend_source: Option<crate::compact_str::CompactString>,
}

/// Represent a page visited.
#[cfg(feature = "decentralized")]
#[derive(Debug, Clone, Default)]
pub struct Page {
    /// The bytes of the resource.
    pub(crate) html: Option<bytes::Bytes>,
    /// Base absolute url for page.
    pub(crate) base: Option<Url>,
    /// The raw url for the page. Useful since Url::parse adds a trailing slash.
    pub(crate) url: String,
    /// The headers of the page request response.
    pub headers: Option<reqwest::header::HeaderMap>,
    #[cfg(feature = "remote_addr")]
    /// The remote address of the page.
    pub remote_addr: Option<core::net::SocketAddr>,
    #[cfg(feature = "cookies")]
    /// The cookies of the page request response.
    pub cookies: Option<reqwest::header::HeaderMap>,
    /// The status code of the page request.
    pub status_code: StatusCode,
    /// The error of the request if any.
    pub error_status: Option<String>,
    /// The current links for the page.
    pub links: HashSet<CaseInsensitiveString>,
    /// The external urls to group with the domain.
    pub external_domains_caseless: Arc<HashSet<CaseInsensitiveString>>,
    /// The final destination of the page if redirects were performed [Unused].
    pub final_redirect_destination: Option<String>,
    #[cfg(feature = "time")]
    /// The duration from start of parsing to end of gathering links.
    duration: Option<Instant>,
    #[cfg(feature = "chrome")]
    /// The screenshot bytes of the page.
    pub screenshot_bytes: Option<Vec<u8>>,
    #[cfg(feature = "openai")]
    /// The credits used from OpenAI in order.
    pub openai_credits_used: Option<Vec<crate::features::openai_common::OpenAIUsage>>,
    #[cfg(feature = "openai")]
    /// The extra data from the AI, example extracting data etc...
    pub extra_ai_data: Option<Vec<AIResults>>,
    #[cfg(feature = "gemini")]
    /// The credits used from Gemini in order.
    pub gemini_credits_used: Option<Vec<crate::features::gemini_common::GeminiUsage>>,
    #[cfg(feature = "gemini")]
    /// The extra data from the Gemini AI.
    pub extra_gemini_data: Option<Vec<AIResults>>,
    /// The usage from remote multimodal automation (extraction, etc.).
    /// Works with both Chrome and HTTP-only crawls.
    pub remote_multimodal_usage: Option<Vec<crate::features::automation::AutomationUsage>>,
    /// The extra data from the remote multimodal automation (extraction results, etc.).
    /// Works with both Chrome and HTTP-only crawls.
    pub extra_remote_multimodal_data: Option<Vec<AutomationResults>>,
    /// URLs requested by automation to spawn as additional pages.
    pub spawn_pages: Option<Vec<String>>,
    /// Additional content keyed by return format (e.g. `"markdown"`, `"text"`).
    /// Populated when multiple formats are requested via
    /// [`SpiderCloudConfig::with_return_formats`](crate::configuration::SpiderCloudConfig::with_return_formats).
    #[cfg(feature = "spider_cloud")]
    pub content_map: Option<hashbrown::HashMap<String, bytes::Bytes>>,
    /// The links found on the page. Unused until we can structure the buffers to match.
    pub page_links: Option<Box<HashSet<CaseInsensitiveString>>>,
    /// The request should retry.
    pub should_retry: bool,
    /// A WAF was found on the page.
    pub waf_check: bool,
    /// The total byte transferred for the page. Mainly used for chrome events.
    pub bytes_transferred: Option<f64>,
    /// The page was blocked from crawling usual from using website::on_should_crawl_callback.
    pub blocked_crawl: bool,
    /// The signature of the page to de-duplicate content.
    pub signature: Option<u64>,
    #[cfg(feature = "chrome")]
    /// All of the response events mapped with the amount of bytes used.
    pub response_map: Option<hashbrown::HashMap<String, f64>>,
    #[cfg(feature = "chrome")]
    /// All of the request events mapped with the time period of the event sent.
    pub request_map: Option<hashbrown::HashMap<String, f64>>,
    /// The anti-bot tech used.
    pub anti_bot_tech: AntiBotTech,
    /// Page metadata.
    pub metadata: Option<Box<Metadata>>,
    /// Whether the response content was truncated due to a stream error,
    /// chunk idle timeout, or Content-Length mismatch.
    pub content_truncated: bool,
    /// Whether a proxy was configured for this request.
    /// When true, 401 responses are retried (proxy rotation may fix auth).
    pub proxy_configured: bool,
    #[cfg(feature = "parallel_backends")]
    /// Identifies which backend produced this page (e.g. "primary",
    /// "lightpanda", "servo"). `None` when parallel backends are not active.
    pub backend_source: Option<crate::compact_str::CompactString>,
}

/// Assign properties from a new page.
#[cfg(feature = "smart")]
pub fn page_assign(page: &mut Page, new_page: Page) {
    if let Some(s) = new_page.final_redirect_destination.as_deref() {
        let bad = match s.as_bytes().first().copied() {
            None => true,
            Some(b'a') => s.starts_with("about:blank"),
            Some(b'c') => s.starts_with("chrome-error://chromewebdata"),
            _ => false,
        };
        if !bad {
            page.final_redirect_destination = Some(s.into());
        }
    }

    let chrome_default_empty_200 =
        new_page.status_code == 200 && new_page.bytes_transferred.is_none() && new_page.is_empty();

    page.anti_bot_tech = new_page.anti_bot_tech;
    page.base = new_page.base;
    page.blocked_crawl = new_page.blocked_crawl;

    if !chrome_default_empty_200 {
        page.status_code = new_page.status_code;
        page.bytes_transferred = new_page.bytes_transferred;
        if new_page.html.is_some() {
            page.html = new_page.html;
        }
    } else {
        // Chrome returned 200 with no content — mark for retry so the outer
        // loop re-fetches instead of silently accepting an empty page.
        page.should_retry = true;
    }

    #[cfg(feature = "remote_addr")]
    {
        page.remote_addr = new_page.remote_addr;
    }
    #[cfg(feature = "time")]
    {
        page.duration = new_page.duration;
    }
    #[cfg(feature = "page_error_status_details")]
    {
        page.error_status = new_page.error_status;
    }

    page.request_map = new_page.request_map;
    page.response_map = new_page.response_map;

    #[cfg(feature = "cookies")]
    {
        if new_page.cookies.is_some() {
            page.cookies = new_page.cookies;
        }
    }
    if new_page.headers.is_some() {
        page.headers = new_page.headers;
    }

    page.waf_check = new_page.waf_check;
    page.should_retry = new_page.should_retry;
    page.signature = new_page.signature;
    if let Some(mut new_spawn_pages) = new_page.spawn_pages {
        /// Max URLs a single page can accumulate via automation spawn_pages.
        const MAX_SPAWN_PAGES: usize = 1000;
        match page.spawn_pages.as_mut() {
            Some(existing) => {
                let remaining = MAX_SPAWN_PAGES.saturating_sub(existing.len());
                new_spawn_pages.truncate(remaining);
                existing.append(&mut new_spawn_pages);
            }
            None => {
                new_spawn_pages.truncate(MAX_SPAWN_PAGES);
                page.spawn_pages = Some(new_spawn_pages);
            }
        }
    }
    page.metadata = new_page.metadata;
}

/// Validate link and push into the map
pub(crate) fn validate_link<
    A: PartialEq + Eq + std::hash::Hash + From<String> + for<'a> From<&'a str>,
>(
    base: &Option<&Url>,
    href: &str,
    base_domain: &CompactString,
    parent_host: &CompactString,
    base_input_domain: &CompactString,
    sub_matcher: &CompactString,
    external_domains_caseless: &Arc<HashSet<CaseInsensitiveString>>,
    links_pages: &mut Option<HashSet<A>>,
) -> Option<Url> {
    if let Some(b) = base {
        let abs = convert_abs_path(b, href);

        if let Some(link_map) = links_pages {
            link_map.insert(A::from(href));
        }

        let scheme = abs.scheme();

        if scheme == "https" || scheme == "http" {
            let host_name = abs.host_str();

            let mut can_process = parent_host_match(
                host_name,
                base_domain,
                parent_host,
                base_input_domain,
                sub_matcher,
            );

            // attempt to check if domain matches with port.
            if !can_process && host_name.is_some() && abs.port().is_some() {
                if let Some(host) = host_name {
                    let hname =
                        string_concat!(host, ":", abs.port().unwrap_or_default().to_string());
                    can_process = parent_host_match(
                        Some(&hname),
                        base_domain,
                        parent_host,
                        base_input_domain,
                        sub_matcher,
                    );
                }
            }

            if !can_process && host_name.is_some() && !external_domains_caseless.is_empty() {
                can_process = external_domains_caseless
                    .contains::<CaseInsensitiveString>(&host_name.unwrap_or_default().into())
                    || external_domains_caseless
                        .contains::<CaseInsensitiveString>(&CASELESS_WILD_CARD);
            }
            if can_process {
                return Some(abs);
            }
        }
    }
    None
}

/// determine a url is relative page
pub(crate) fn relative_directory_url(href: &str) -> bool {
    if href.starts_with("./") || href.starts_with("//") || href.starts_with("../") {
        true
    } else {
        let network_capable = networking_capable(href);

        if network_capable {
            false
        } else {
            !href.starts_with("/")
        }
    }
}

/// Validate link and push into the map without extended verify.
pub(crate) fn push_link<
    A: PartialEq + Eq + std::hash::Hash + From<String> + for<'a> From<&'a str>,
>(
    base: &Option<&Url>,
    href: &str,
    map: &mut HashSet<A>,
    base_domain: &CompactString,
    parent_host: &CompactString,
    parent_host_scheme: &CompactString,
    base_input_domain: &CompactString,
    sub_matcher: &CompactString,
    external_domains_caseless: &Arc<HashSet<CaseInsensitiveString>>,
    links_pages: &mut Option<HashSet<A>>,
) {
    let abs = validate_link(
        base,
        href,
        base_domain,
        parent_host,
        base_input_domain,
        sub_matcher,
        external_domains_caseless,
        links_pages,
    );

    if let Some(mut abs) = abs {
        if abs.scheme() != parent_host_scheme.as_str() {
            let _ = abs.set_scheme(parent_host_scheme.as_str());
        }
        map.insert(A::from(abs.as_str()));
    }
}

/// Validate link and push into the map
pub(crate) fn push_link_verify<
    A: PartialEq + Eq + std::hash::Hash + From<String> + for<'a> From<&'a str>,
>(
    base: &Option<&Url>,
    href: &str,
    map: &mut HashSet<A>,
    base_domain: &CompactString,
    parent_host: &CompactString,
    parent_host_scheme: &CompactString,
    base_input_domain: &CompactString,
    sub_matcher: &CompactString,
    external_domains_caseless: &Arc<HashSet<CaseInsensitiveString>>,
    full_resources: bool,
    links_pages: &mut Option<HashSet<A>>,
    verify: bool,
) {
    let abs = validate_link(
        base,
        href,
        base_domain,
        parent_host,
        base_input_domain,
        sub_matcher,
        external_domains_caseless,
        links_pages,
    );
    if let Some(mut abs) = abs {
        if abs.scheme() != parent_host_scheme.as_str() {
            let _ = abs.set_scheme(parent_host_scheme.as_str());
        }
        if verify {
            push_link_check(&mut abs, map, full_resources, &mut true);
        } else {
            map.insert(A::from(abs.as_str()));
        }
    }
}

/// Determine if a url is an asset.
pub fn is_asset_url(url: &str) -> bool {
    let mut asset = false;
    if let Some(position) = url.rfind('.') {
        if url.len() - position >= 3 {
            asset = IGNORE_ASSETS.contains::<CaseInsensitiveString>(&url[position + 1..].into());
        }
    }
    asset
}

/// Validate link and push into the map checking if asset
pub(crate) fn push_link_check<
    A: PartialEq + Eq + std::hash::Hash + From<String> + for<'a> From<&'a str>,
>(
    abs: &mut Url,
    map: &mut HashSet<A>,
    full_resources: bool,
    can_process: &mut bool,
) {
    let hchars = abs.path();

    // check if the file is a resource and block if it is
    if let Some(position) = hchars.rfind('.') {
        let hlen = hchars.len();
        let has_asset = hlen - position;

        if has_asset >= 3 {
            let next_position = position + 1;

            if !full_resources
                && IGNORE_ASSETS.contains::<CaseInsensitiveString>(&hchars[next_position..].into())
            {
                *can_process = false;
            }
        }
    }

    if *can_process {
        map.insert(A::from(abs.as_str()));
    }
}

/// get the clean domain name
pub(crate) fn domain_name(domain: &Url) -> &str {
    domain.host_str().unwrap_or_default()
}

/// Extract the root domain from a hostname in a single pass.
/// "sub.example.com" → "example.com", "example.com" → "example", "localhost" → "localhost"
#[inline]
fn extract_root_domain(domain: &str) -> &str {
    // Single-pass: find the second-to-last dot position by scanning from the end.
    let bytes = domain.as_bytes();
    let mut dot_count = 0u8;
    let mut second_last_dot = 0;

    for i in (0..bytes.len()).rev() {
        if bytes[i] == b'.' {
            dot_count += 1;
            if dot_count == 2 {
                second_last_dot = i + 1;
                break;
            }
        }
    }

    if dot_count >= 2 {
        &domain[second_last_dot..]
    } else if dot_count == 1 {
        // "example.com" → "example"
        let dot_pos = memchr::memchr(b'.', bytes).unwrap_or(domain.len());
        &domain[..dot_pos]
    } else {
        domain
    }
}

/// Check for subdomain matches by comparing root domains.
#[inline]
#[cfg_attr(not(test), allow(dead_code))]
fn is_subdomain(subdomain: &str, domain: &str) -> bool {
    extract_root_domain(subdomain) == extract_root_domain(domain)
}

/// Validation to match a domain to parent host and the top level redirect
/// for the crawl. Short-circuits on exact match (most common case).
pub(crate) fn parent_host_match(
    host_name: Option<&str>,
    base_domain: &str,
    parent_host: &CompactString,
    base_host: &CompactString,
    sub_matcher: &CompactString,
) -> bool {
    match host_name {
        Some(host) => {
            // Fast path: exact match (covers ~80% of same-domain links).
            if parent_host.eq(&host) || base_host.eq(&host) {
                return true;
            }

            if base_domain.is_empty() {
                return false;
            }

            // Extract host's root domain once, compare against both targets.
            let host_root = extract_root_domain(host);
            extract_root_domain(parent_host) == host_root
                || extract_root_domain(sub_matcher) == host_root
        }
        _ => false,
    }
}

/// html selector for valid web pages for domain.
pub(crate) fn get_page_selectors_base(u: &str, subdomains: bool, tld: bool) -> RelativeSelectors {
    let dname = get_domain_from_url(u);
    let host_name = CompactString::from(dname);

    let scheme = if u.starts_with("https://") {
        "https"
    } else if u.starts_with("http://") {
        "http"
    } else if u.starts_with("file://") {
        "file"
    } else if u.starts_with("wss://") {
        "wss"
    } else if u.starts_with("ws://") {
        "ws"
    } else {
        // default to https
        "https"
    };

    if tld || subdomains {
        let dname = if tld {
            extract_root_domain(dname)
        } else {
            dname
        };

        (
            dname.into(), // match for tlds or subdomains
            smallvec::SmallVec::from([host_name, CompactString::from(scheme)]),
            CompactString::default(),
        )
    } else {
        (
            CompactString::default(),
            smallvec::SmallVec::from([host_name, CompactString::from(scheme)]),
            CompactString::default(),
        )
    }
}

/// html selector for valid web pages for domain.
pub fn get_page_selectors(url: &str, subdomains: bool, tld: bool) -> RelativeSelectors {
    get_page_selectors_base(url, subdomains, tld)
}

#[cfg(not(feature = "decentralized"))]
/// Is the resource valid?
pub fn validate_empty(content: &Option<Vec<u8>>, is_success: bool) -> bool {
    match &content {
        Some(content) => {
            // is_success && content.starts_with(br#"<html style=\"height:100%\"><head><META NAME=\"ROBOTS\" CONTENT=\"NOINDEX, NOFOLLOW\"><meta name=\"format-detection\" content=\"telephone=no\"><meta name=\"viewport\" content=\"initial-scale=1.0\"><meta http-equiv=\"X-UA-Compatible\" content=\"IE=edge,chrome=1\"></head><body style=\"margin:0px;height:100%\"><iframe id=\"main-iframe\" src=\"/_Incapsula_"#)
            !( content.is_empty() || content.starts_with(b"<html><head></head><body></body></html>") || is_success &&
                     content.starts_with(b"<html>\r\n<head>\r\n<META NAME=\"robots\" CONTENT=\"noindex,nofollow\">\r\n<script src=\"/") &&
                      content.ends_with(b"\">\r\n</script>\r\n<body>\r\n</body></html>\r\n")
                || is_chrome_error_page(content))
        }
        _ => false,
    }
}

/// Returns `true` if the HTML content is a Chrome browser error page
/// (e.g. ERR_TUNNEL_CONNECTION_FAILED, ERR_NAME_NOT_RESOLVED, etc.).
///
/// Chrome renders these pages locally when proxy/network errors occur.
/// They arrive with HTTP 200 and ~157KB of content (CSS, dino game JS,
/// base64 error images). Detection uses the structural tail: Chrome error
/// pages always end with `};</script></html>` (no `</body>` tag) and
/// contain `"errorCode":"ERR` in the `loadTimeDataRaw` JSON blob.
///
/// Two checks, both on the last 4KB only — O(1) for all non-error pages:
/// 1. `ends_with(b"};</script></html>")` — Chrome error page structure
/// 2. `"errorCode":"ERR` present — Chrome-internal JSON key
///
/// Zero false positives: real websites always have `</body>` before
/// `</html>`, and never contain `loadTimeDataRaw` with `errorCode`.
#[cfg(not(feature = "decentralized"))]
#[inline]
pub fn is_chrome_error_page(content: &[u8]) -> bool {
    const TAIL: &[u8] = b"};</script></html>";
    const NEEDLE: &[u8] = b"\"errorCode\":\"ERR";

    if content.len() < 500 {
        return false;
    }

    // Trim trailing whitespace for ends_with
    let mut end = content.len();
    while end > 0 && content[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    let trimmed = &content[..end];

    if !trimmed.ends_with(TAIL) {
        return false;
    }

    // Confirm errorCode in the last 4KB
    let region = if trimmed.len() > 4096 {
        &trimmed[trimmed.len() - 4096..]
    } else {
        trimmed
    };
    memchr::memmem::find(region, NEEDLE).is_some()
}

/// Extract a specific type of error from a chain of errors.
#[cfg(not(feature = "decentralized"))]
fn extract_specific_error<'a, T: std::error::Error + 'static>(
    error: &'a (dyn std::error::Error + 'static),
) -> Option<&'a T> {
    let mut current_error = Some(error);
    while let Some(err) = current_error {
        if let Some(desired_error) = err.downcast_ref::<T>() {
            return Some(desired_error);
        }
        current_error = err.source();
    }
    None
}

/// Determine if the response is goaway and should retry.
/// Covers transient HTTP/2 errors where a retry is likely to succeed:
/// - GO_AWAY + NO_ERROR: graceful server shutdown (load balancer rotation)
/// - REFUSED_STREAM: server rejected the stream before processing
/// - ENHANCE_YOUR_CALM: server-side rate limiting (HTTP/2 equivalent of 429)
/// - INTERNAL_ERROR: transient server error
#[cfg(not(feature = "decentralized"))]
fn should_attempt_retry(error: &(dyn std::error::Error + 'static)) -> bool {
    if let Some(e) = extract_specific_error::<h2::Error>(error) {
        if e.is_go_away() && e.is_remote() && e.reason() == Some(h2::Reason::NO_ERROR) {
            return true;
        }
        if e.is_remote() {
            if let Some(reason) = e.reason() {
                return matches!(
                    reason,
                    h2::Reason::REFUSED_STREAM
                        | h2::Reason::ENHANCE_YOUR_CALM
                        | h2::Reason::INTERNAL_ERROR
                );
            }
        }
    }
    false
}

/// Get the error status of the page base.
#[cfg(not(feature = "decentralized"))]
fn get_error_status_base(
    should_retry: &mut bool,
    error_for_status: Option<Result<crate::utils::RequestResponse, RequestError>>,
) -> Option<RequestError> {
    match error_for_status {
        Some(e) => match e {
            Ok(_) => None,
            Err(er) => {
                if er.is_timeout() || (er.is_connect() && !is_dns_error(&er)) {
                    *should_retry = true;
                }
                if !*should_retry && should_attempt_retry(&er) {
                    *should_retry = true;
                }
                if let Some(status_code) = er.status() {
                    let retry = match status_code {
                        StatusCode::TOO_MANY_REQUESTS
                        | StatusCode::INTERNAL_SERVER_ERROR
                        | StatusCode::BAD_GATEWAY
                        | StatusCode::SERVICE_UNAVAILABLE
                        | StatusCode::GATEWAY_TIMEOUT => true,
                        _ => false,
                    };

                    if retry {
                        *should_retry = true;
                    }
                }
                // Ensure errors that map to retryable status codes (599, 598, 5xx,
                // 429, 408) always set should_retry, even when the error type
                // didn't match is_status/is_connect/is_timeout above.
                if !*should_retry {
                    let mapped = get_error_http_status_code(&er);
                    if is_retryable_status(mapped) {
                        *should_retry = true;
                    }
                }
                Some(er)
            }
        },
        _ => None,
    }
}

#[cfg(all(
    not(feature = "page_error_status_details"),
    not(feature = "decentralized")
))]
/// Get the error status of the page.
fn get_error_status(
    should_retry: &mut bool,
    error_for_status: Option<Result<crate::utils::RequestResponse, RequestError>>,
) -> Option<String> {
    get_error_status_base(should_retry, error_for_status).map(|e| e.to_string())
}

#[cfg(all(feature = "page_error_status_details", not(feature = "decentralized")))]
/// Get the error status of the page.
fn get_error_status(
    should_retry: &mut bool,
    error_for_status: Option<Result<crate::utils::RequestResponse, RequestError>>,
) -> Option<std::sync::Arc<reqwest::Error>> {
    get_error_status_base(should_retry, error_for_status).map(std::sync::Arc::new)
}

#[cfg(not(feature = "decentralized"))]
/// Instantiate a new page without scraping it and with the base URL parsed (used for testing purposes).
pub fn build_with_parse(url: &str, res: PageResponse) -> Page {
    let mut page = build(url, res);
    page.set_url_parsed_direct_empty();
    page
}

/// Instantiate a new page without scraping it and with the base URL parsed (used for testing purposes).
#[cfg(feature = "decentralized")]
pub fn build_with_parse(url: &str, res: PageResponse) -> Page {
    build(url, res)
}

/// Instantiate a new page without scraping it (used for testing purposes).
#[cfg(not(feature = "decentralized"))]
pub fn build(url: &str, mut res: PageResponse) -> Page {
    use crate::utils::validation::is_false_403;

    // Chrome error pages (ERR_CONNECTION_RESET, ERR_TUNNEL_CONNECTION_FAILED, etc.)
    // return HTTP 200 with ~157KB of error page content. Reclassify to 599
    // (spider internal error) so all retry paths treat it as a failed crawl.
    // Content is preserved for debugging but status signals failure.
    let chrome_error =
        res.status_code.is_success() && res.content.as_deref().is_some_and(is_chrome_error_page);
    if chrome_error {
        res.status_code = StatusCode::from_u16(599).unwrap_or(StatusCode::BAD_GATEWAY);
    }

    let success = res.status_code.is_success() || res.status_code == StatusCode::NOT_FOUND;
    let resource_found = validate_empty(&res.content, success);

    let status = res.status_code;

    // DNS resolve error (525) is permanent — never retry
    let should_retry_status = status != *DNS_RESOLVE_ERROR
        && (status.is_server_error()
            || matches!(
                status,
                StatusCode::TOO_MANY_REQUESTS | StatusCode::FORBIDDEN | StatusCode::REQUEST_TIMEOUT
            ));

    let should_retry_resource = resource_found && !success && status != StatusCode::UNAUTHORIZED;

    // Success status but no usable content — the crawl likely failed silently
    // (e.g. upstream returned 200 with empty body or blank HTML shell).
    let should_retry_empty_success = success && !resource_found && !res.content_truncated;

    let should_retry_antibot_false_403 = res.anti_bot_tech != AntiBotTech::None
        && res.status_code.is_success()
        && is_false_403(
            res.content.as_deref(),
            res.headers
                .as_ref()
                .and_then(|h| h.get(reqwest::header::CONTENT_LANGUAGE))
                .and_then(|v| v.to_str().ok()),
        );

    let mut should_retry = should_retry_resource
        || should_retry_status
        || should_retry_empty_success
        || should_retry_antibot_false_403;

    let mut empty_page = false;

    if let Some(final_url) = &res.final_url {
        if final_url.starts_with("chrome-error://chromewebdata")
            || final_url.starts_with("about:blank")
        {
            should_retry = false;
            empty_page = true;
        }
    }

    // Cancel retry for legitimate 403s (Apache/nginx access denied) but keep
    // retrying when antibot tech is detected — proxy/browser rotation can bypass
    // WAF blocks (Cloudflare Bot Management, DataDome, Imperva, etc.).
    if should_retry
        && !resource_found
        && res.status_code == StatusCode::FORBIDDEN
        && res.headers.is_some()
        && res.anti_bot_tech == AntiBotTech::None
    {
        should_retry = false;
    }

    Page {
        html: res.content.map(bytes::Bytes::from),
        headers: res.headers,
        #[cfg(feature = "remote_addr")]
        remote_addr: res.remote_addr,
        #[cfg(feature = "cookies")]
        cookies: res.cookies,
        url: url.into(),
        #[cfg(feature = "time")]
        duration: res.duration,
        status_code: res.status_code,
        error_status: get_error_status(&mut should_retry, res.error_for_status),
        final_redirect_destination: if empty_page { None } else { res.final_url },
        #[cfg(feature = "chrome")]
        chrome_page: None,
        #[cfg(feature = "chrome")]
        screenshot_bytes: res.screenshot_bytes,
        #[cfg(feature = "openai")]
        openai_credits_used: res.openai_credits_used,
        #[cfg(feature = "openai")]
        extra_ai_data: res.extra_ai_data,
        #[cfg(feature = "gemini")]
        gemini_credits_used: res.gemini_credits_used,
        #[cfg(feature = "gemini")]
        extra_gemini_data: res.extra_gemini_data,
        remote_multimodal_usage: res.remote_multimodal_usage,
        extra_remote_multimodal_data: res.extra_remote_multimodal_data,
        spawn_pages: res.spawn_pages,
        #[cfg(feature = "spider_cloud")]
        content_map: res.content_map,
        should_retry,
        waf_check: res.waf_check,
        bytes_transferred: res.bytes_transferred,
        blocked_crawl: false,
        signature: res.signature,
        #[cfg(feature = "chrome")]
        response_map: res.response_map,
        #[cfg(feature = "chrome")]
        request_map: res.request_map,
        anti_bot_tech: res.anti_bot_tech,
        metadata: res.metadata,
        content_truncated: res.content_truncated,
        ..Default::default()
    }
}

/// Instantiate a new page without scraping it (used for testing purposes).
#[cfg(feature = "decentralized")]
pub fn build(_: &str, res: PageResponse) -> Page {
    Page {
        html: res.content.map(bytes::Bytes::from),
        headers: res.headers,
        #[cfg(feature = "remote_addr")]
        remote_addr: res.remote_addr,
        #[cfg(feature = "cookies")]
        cookies: res.cookies,
        final_redirect_destination: res.final_url,
        status_code: res.status_code,
        metadata: res.metadata,
        spawn_pages: res.spawn_pages,
        content_truncated: res.content_truncated,
        error_status: match res.error_for_status {
            Some(e) => match e {
                Ok(_) => None,
                Err(er) => Some(er.to_string()),
            },
            _ => None,
        },
        ..Default::default()
    }
}

#[cfg(all(feature = "headers", feature = "cookies"))]
/// Re build the cookies.
pub fn build_cookie_header_from_set_cookie(page: &Page) -> Option<reqwest::header::HeaderValue> {
    use reqwest::header::HeaderValue;
    let mut cookie_pairs = Vec::with_capacity(8);

    if let Some(headers) = &page.headers {
        for cookie in headers.get_all(crate::client::header::SET_COOKIE).iter() {
            if let Ok(cookie_str) = cookie.to_str() {
                if let Ok(parsed) = cookie::Cookie::parse(cookie_str) {
                    cookie_pairs.push(format!("{}={}", parsed.name(), parsed.value()));
                }
            }
        }
    }

    if cookie_pairs.is_empty() {
        None
    } else {
        let cookie_header_str = cookie_pairs.join("; ");
        HeaderValue::from_str(&cookie_header_str).ok()
    }
}

#[cfg(not(all(feature = "headers", feature = "cookies")))]
/// Re build the cookies.
pub fn build_cookie_header_from_set_cookie(_page: &Page) -> Option<reqwest::header::HeaderValue> {
    None
}

/// Settings for streaming rewriter
#[derive(Debug, Default, Clone, Copy)]
pub struct PageLinkBuildSettings {
    /// If the SSG build is in progress.
    pub ssg_build: bool,
    /// If full resources should be included.
    pub full_resources: bool,
    /// TLD handling resources.
    pub tld: bool,
    /// Subdomain handling resources.
    pub subdomains: bool,
    /// De-duplication signature.
    pub normalize: bool,
}

impl PageLinkBuildSettings {
    /// New build link settings.
    pub(crate) fn new(ssg_build: bool, full_resources: bool) -> Self {
        Self {
            ssg_build,
            full_resources,
            ..Default::default()
        }
    }

    /// New build full link settings.
    pub(crate) fn new_full(
        ssg_build: bool,
        full_resources: bool,
        subdomains: bool,
        tld: bool,
        normalize: bool,
    ) -> Self {
        Self {
            ssg_build,
            full_resources,
            subdomains,
            tld,
            normalize,
        }
    }
}

/// Get the content type from the responses
pub(crate) fn get_charset_from_content_type(
    headers: &reqwest::header::HeaderMap,
) -> Option<AsciiCompatibleEncoding> {
    use auto_encoder::encoding_rs;

    if let Some(content_type) = headers.get(reqwest::header::CONTENT_TYPE) {
        if let Ok(content_type_str) = content_type.to_str() {
            for part in content_type_str.split(';') {
                let part = part.trim();
                if part.len() >= 8 && part.as_bytes()[..8].eq_ignore_ascii_case(b"charset=") {
                    let stripped = &part[8..];
                    if let Some(encoding) = encoding_rs::Encoding::for_label(stripped.as_bytes()) {
                        if let Some(ascii_encoding) = AsciiCompatibleEncoding::new(encoding) {
                            return Some(ascii_encoding);
                        }
                    }
                }
            }
        }
    }

    None
}

#[cfg(feature = "chrome")]
/// Set the metadata found on the page.

pub(crate) fn set_metadata(mdata: &Option<Box<Metadata>>, metadata: &mut Metadata) {
    if let Some(mdata) = &mdata {
        if mdata.automation.is_some() {
            metadata.automation = mdata.automation.clone();
        }
    }
}

/// Set the metadata found on the page.

#[cfg(not(feature = "chrome"))]
pub(crate) fn set_metadata(_mdata: &Option<Box<Metadata>>, _metadata: &mut Metadata) {}

/// Check if urls are the same without the trailing slashes.
fn exact_url_match(url: &str, target_url: &str) -> bool {
    let end_target_slash = target_url.ends_with('/');
    let main_slash = url.ends_with('/');

    if end_target_slash && !main_slash {
        strip_trailing_slash(target_url) == url
    } else if !end_target_slash && main_slash {
        url == strip_trailing_slash(target_url)
    } else {
        url == target_url
    }
}

/// Strip end matching
fn strip_trailing_slash(s: &str) -> &str {
    if s.ends_with('/') {
        s.trim_end_matches('/')
    } else {
        s
    }
}

/// metadata handlers
pub(crate) fn metadata_handlers<'h>(
    meta_title: &'h mut Option<CompactString>,
    meta_description: &'h mut Option<CompactString>,
    meta_og_image: &'h mut Option<CompactString>,
) -> Vec<(
    std::borrow::Cow<'static, lol_html::Selector>,
    lol_html::send::ElementContentHandlers<'h>,
)> {
    vec![
        lol_html::text!("head title", |el| {
            let t = el.as_str();
            if !t.is_empty() {
                *meta_title = Some(t.into());
            }

            Ok(())
        }),
        lol_html::element!(r#"meta[name="description"]"#, |el| {
            if let Some(content) = el.get_attribute("content") {
                if !content.is_empty() {
                    *meta_description = Some(content.into());
                }
            }
            Ok(())
        }),
        lol_html::element!(r#"meta[property="og:image"]"#, |el| {
            if let Some(content) = el.get_attribute("content") {
                if !content.is_empty() {
                    *meta_og_image = Some(content.into());
                }
            }
            Ok(())
        }),
    ]
}

impl Page {
    /// Whether the page needs a retry based on `should_retry`, a retryable status code,
    /// a truncated response (upstream stream ended prematurely), or a proxy-retryable
    /// 401 (when `proxy_configured` is set, proxy rotation may resolve the auth failure).
    #[inline]
    pub fn needs_retry(&self) -> bool {
        self.should_retry
            || self.content_truncated
            || is_retryable_status(self.status_code)
            || (self.proxy_configured && self.status_code == StatusCode::UNAUTHORIZED)
    }

    /// Instantiate a new page and gather the html repro of standard fetch_page_html.
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub async fn new_page(url: &str, client: &Client) -> Self {
        let page_resource: PageResponse = crate::utils::fetch_page_html_raw(url, client).await;

        build(url, page_resource)
    }

    /// Instantiate a new page using cache options when available.
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub async fn new_page_with_cache(
        url: &str,
        client: &Client,
        cache_options: Option<CacheOptions>,
        cache_policy: &Option<BasicCachePolicy>,
        cache_namespace: Option<&str>,
    ) -> Self {
        let page_resource: PageResponse = crate::utils::fetch_page_html_raw_cached(
            url,
            client,
            cache_options,
            cache_policy,
            cache_namespace,
        )
        .await;

        build(url, page_resource)
    }

    /// Create a new page from WebDriver content.
    #[cfg(feature = "webdriver")]
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub fn new_webdriver(url: &str, html: String, status_code: StatusCode) -> Self {
        Page {
            html: Some(bytes::Bytes::from(html.into_bytes())),
            url: url.into(),
            status_code,
            ..Default::default()
        }
    }

    /// Create a new page from WebDriver with full response.
    #[cfg(feature = "webdriver")]
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub async fn new_page_webdriver(
        url: &str,
        driver: &std::sync::Arc<thirtyfour::WebDriver>,
        timeout: Option<std::time::Duration>,
    ) -> Self {
        use crate::features::webdriver::{attempt_navigation, get_current_url, get_page_content};

        // Navigate to the URL
        if let Err(e) = attempt_navigation(url, driver, &timeout).await {
            log::error!("WebDriver navigation failed: {:?}", e);
            return Page {
                url: url.into(),
                status_code: *UNKNOWN_STATUS_ERROR,
                #[cfg(not(feature = "page_error_status_details"))]
                error_status: Some(format!("WebDriver navigation failed: {:?}", e)),
                ..Default::default()
            };
        }

        // Get current URL (may have redirected)
        let final_url = get_current_url(driver).await.ok();

        // Get page content
        match get_page_content(driver).await {
            Ok(content) => Page {
                html: Some(bytes::Bytes::from(content.into_bytes())),
                url: url.into(),
                status_code: StatusCode::OK,
                final_redirect_destination: final_url,
                ..Default::default()
            },
            Err(e) => {
                log::error!("Failed to get WebDriver page content: {:?}", e);
                Page {
                    url: url.into(),
                    status_code: *UNKNOWN_STATUS_ERROR,
                    #[cfg(not(feature = "page_error_status_details"))]
                    error_status: Some(format!("Failed to get page content: {:?}", e)),
                    ..Default::default()
                }
            }
        }
    }

    /// Create a new page from WebDriver with full response and automation support.
    #[cfg(all(feature = "webdriver", feature = "chrome"))]
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub async fn new_page_webdriver_full(
        url: &str,
        driver: &std::sync::Arc<thirtyfour::WebDriver>,
        timeout: Option<std::time::Duration>,
        wait_for: &Option<crate::configuration::WaitFor>,
        execution_scripts: &Option<crate::features::chrome_common::ExecutionScripts>,
        automation_scripts: &Option<crate::features::chrome_common::AutomationScripts>,
    ) -> Self {
        use crate::features::webdriver::{
            attempt_navigation, get_current_url, get_page_content, run_execution_scripts,
            run_url_automation_scripts,
        };

        // Navigate to the URL
        if let Err(e) = attempt_navigation(url, driver, &timeout).await {
            log::error!("WebDriver navigation failed: {:?}", e);
            return Page {
                url: url.into(),
                status_code: *UNKNOWN_STATUS_ERROR,
                #[cfg(not(feature = "page_error_status_details"))]
                error_status: Some(format!("WebDriver navigation failed: {:?}", e)),
                ..Default::default()
            };
        }

        // Run execution scripts for this URL
        run_execution_scripts(driver, url, execution_scripts).await;

        // Run automation scripts for this URL
        run_url_automation_scripts(driver, url, automation_scripts).await;

        // Handle wait_for configuration
        if let Some(wait_config) = wait_for {
            // Handle delay wait
            if let Some(ref delay) = wait_config.delay {
                if let Some(timeout_duration) = delay.timeout {
                    tokio::time::sleep(timeout_duration).await;
                }
            }
            // Handle selector wait
            if let Some(ref selector_wait) = wait_config.selector {
                let wait_timeout = selector_wait
                    .timeout
                    .unwrap_or(std::time::Duration::from_secs(30));
                let _ = crate::features::webdriver::wait_for_element(
                    driver,
                    &selector_wait.selector,
                    wait_timeout,
                )
                .await;
            }
            // Handle idle network wait (approximate with delay)
            if let Some(ref idle) = wait_config.idle_network {
                let wait_time = idle.timeout.unwrap_or(std::time::Duration::from_secs(5));
                tokio::time::sleep(wait_time).await;
            }
        }

        // Get current URL (may have redirected)
        let final_url = get_current_url(driver).await.ok();

        // Get page content
        match get_page_content(driver).await {
            Ok(content) => Page {
                html: Some(bytes::Bytes::from(content.into_bytes())),
                url: url.into(),
                status_code: StatusCode::OK,
                final_redirect_destination: final_url,
                ..Default::default()
            },
            Err(e) => {
                log::error!("Failed to get WebDriver page content: {:?}", e);
                Page {
                    url: url.into(),
                    status_code: *UNKNOWN_STATUS_ERROR,
                    #[cfg(not(feature = "page_error_status_details"))]
                    error_status: Some(format!("Failed to get page content: {:?}", e)),
                    ..Default::default()
                }
            }
        }
    }

    /// New page with rewriter
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub async fn new_page_streaming<
        A: PartialEq
            + Eq
            + Sync
            + Send
            + Clone
            + Default
            + std::hash::Hash
            + From<String>
            + for<'a> From<&'a str>,
    >(
        url: &str,
        client: &Client,
        only_html: bool,
        selectors: &mut RelativeSelectors,
        external_domains_caseless: &Arc<HashSet<CaseInsensitiveString>>,
        r_settings: &PageLinkBuildSettings,
        map: &mut hashbrown::HashSet<A>,
        ssg_map: Option<&mut hashbrown::HashSet<A>>,
        prior_domain: &Option<Box<Url>>,
        domain_parsed: &mut Option<Box<Url>>,
        links_pages: &mut Option<hashbrown::HashSet<A>>,
    ) -> Self {
        use crate::utils::{
            handle_response_bytes, handle_response_bytes_writer, modify_selectors,
            AllowedDomainTypes,
        };

        let mut metadata: Option<Box<Metadata>> = None;
        let mut meta_title: Option<_> = None;
        let mut meta_description: Option<_> = None;
        let mut meta_og_image: Option<_> = None;

        let duration = if cfg!(feature = "time") {
            Some(tokio::time::Instant::now())
        } else {
            None
        };

        let mut page_response: PageResponse = match client.get(url).send().await {
            Ok(res)
                if crate::utils::valid_parsing_status(&res)
                    && !crate::utils::block_streaming(&res, only_html) =>
            {
                let cell = if r_settings.ssg_build {
                    Some(tokio::sync::OnceCell::new())
                } else {
                    None
                };

                let base_input_url = tokio::sync::OnceCell::new();

                let (encoding, adjust_charset_on_meta_tag) =
                    match get_charset_from_content_type(res.headers()) {
                        Some(h) => (h, false),
                        _ => (AsciiCompatibleEncoding::utf_8(), true),
                    };

                let target_url = res.url().as_str();

                // handle initial redirects
                if ssg_map.is_some() && url != target_url && !exact_url_match(url, target_url) {
                    let mut url = Box::new(CaseInsensitiveString::new(&url));

                    modify_selectors(
                        prior_domain,
                        target_url,
                        domain_parsed,
                        &mut url,
                        selectors,
                        AllowedDomainTypes::new(r_settings.subdomains, r_settings.tld),
                    );
                };

                let base = if domain_parsed.is_none() {
                    prior_domain
                } else {
                    domain_parsed
                };

                let original_page = Url::parse(url).ok();

                let parent_host = &selectors.1[0];
                // the host schemes
                let parent_host_scheme = &selectors.1[1];
                let base_input_domain = &selectors.2; // the domain after redirects
                let sub_matcher = &selectors.0;
                let xml_file = target_url.ends_with(".xml");

                let base_links_settings = if r_settings.full_resources {
                    lol_html::element!(
                        "a[href]:not([aria-hidden=\"true\"]),script[src],link[href]",
                        |el| {
                            let tag_name = el.tag_name();

                            let attribute = if tag_name == "script" { "src" } else { "href" };

                            if let Some(href) = el.get_attribute(attribute) {
                                let base = if relative_directory_url(&href) || base.is_none() {
                                    original_page.as_ref()
                                } else {
                                    base.as_deref()
                                };
                                let base = if base_input_url.initialized() {
                                    base_input_url.get()
                                } else {
                                    base
                                };

                                push_link(
                                    &base,
                                    &href,
                                    map,
                                    &selectors.0,
                                    parent_host,
                                    parent_host_scheme,
                                    base_input_domain,
                                    sub_matcher,
                                    external_domains_caseless,
                                    links_pages,
                                );
                            }

                            Ok(())
                        }
                    )
                } else {
                    element_precompiled!(
                        if xml_file {
                            compiled_xml_selector()
                        } else {
                            compiled_selector()
                        },
                        |el| {
                            if let Some(href) = el.get_attribute("href") {
                                let base = if relative_directory_url(&href) || base.is_none() {
                                    original_page.as_ref()
                                } else {
                                    base.as_deref()
                                };
                                let base = if base_input_url.initialized() {
                                    base_input_url.get()
                                } else {
                                    base
                                };
                                push_link(
                                    &base,
                                    &href,
                                    map,
                                    &selectors.0,
                                    parent_host,
                                    parent_host_scheme,
                                    base_input_domain,
                                    sub_matcher,
                                    external_domains_caseless,
                                    links_pages,
                                );
                            }
                            Ok(())
                        }
                    )
                };

                let mut element_content_handlers =
                    Vec::with_capacity(if r_settings.ssg_build { 2 } else { 1 } + 4);

                element_content_handlers.push(element_precompiled!(
                    compiled_base_element_selector(),
                    |el| {
                        if let Some(href) = el.get_attribute("href") {
                            if let Ok(parsed_base) = Url::parse(&href) {
                                let _ = base_input_url.set(parsed_base);
                            }
                        }

                        Ok(())
                    }
                ));
                element_content_handlers.push(base_links_settings);

                element_content_handlers.extend(metadata_handlers(
                    &mut meta_title,
                    &mut meta_description,
                    &mut meta_og_image,
                ));

                if r_settings.ssg_build {
                    element_content_handlers.push(lol_html::element!("script", |el| {
                        if let Some(build_path) = el.get_attribute("src") {
                            if build_path.starts_with("/_next/static/")
                                && build_path.ends_with("/_ssgManifest.js")
                            {
                                if let Some(ref cell) = cell {
                                    let _ = cell.set(build_path.to_string());
                                }
                            }
                        }
                        Ok(())
                    }));
                }

                let settings = lol_html::send::Settings {
                    element_content_handlers,
                    adjust_charset_on_meta_tag,
                    encoding,
                    ..lol_html::send::Settings::new_for_handler_types()
                };

                let mut rewriter = lol_html::send::HtmlRewriter::new(settings, |_c: &[u8]| {});

                let mut collected_bytes = match res.content_length() {
                    Some(cap) if cap >= MAX_PRE_ALLOCATED_HTML_PAGE_SIZE => {
                        Vec::with_capacity(cap as usize)
                    }
                    Some(cap) if cap > 0 => Vec::with_capacity(cap as usize),
                    _ => Vec::with_capacity(MAX_PRE_ALLOCATED_HTML_PAGE_SIZE_USIZE),
                };

                let mut response = handle_response_bytes_writer(
                    res,
                    url,
                    only_html,
                    &mut rewriter,
                    &mut collected_bytes,
                )
                .await;

                let rewrite_error = response.1;

                if !rewrite_error {
                    let _ = rewriter.end();
                }

                if r_settings.normalize {
                    response.0.signature = Some(hash_html(&collected_bytes).await);
                }

                response.0.content = if collected_bytes.is_empty() {
                    None
                } else {
                    Some(collected_bytes)
                };

                if r_settings.ssg_build {
                    if let Some(ssg_map) = ssg_map {
                        if let Some(cell) = &cell {
                            if let Some(source) = cell.get() {
                                if let Some(url_base) = &base {
                                    let build_ssg_path = convert_abs_path(url_base, source);
                                    let build_page =
                                        Page::new_page(build_ssg_path.as_str(), client).await;

                                    for cap in
                                        SSG_CAPTURE.captures_iter(build_page.get_html_bytes_u8())
                                    {
                                        if let Some(matched) = cap.get(1) {
                                            let href = auto_encode_bytes(matched.as_bytes())
                                                .replace(r#"\u002F"#, "/");

                                            let last_segment =
                                                crate::utils::get_last_segment(&href);

                                            // we can pass in a static map of the dynamic SSG routes pre-hand, custom API endpoint to seed, or etc later.
                                            if !(last_segment.starts_with("[")
                                                && last_segment.ends_with("]"))
                                            {
                                                let base = if relative_directory_url(&href)
                                                    || base.is_none()
                                                {
                                                    original_page.as_ref()
                                                } else {
                                                    base.as_deref()
                                                };
                                                let base = if base_input_url.initialized() {
                                                    base_input_url.get()
                                                } else {
                                                    base
                                                };
                                                push_link(
                                                    &base,
                                                    &href,
                                                    ssg_map,
                                                    &selectors.0,
                                                    parent_host,
                                                    parent_host_scheme,
                                                    base_input_domain,
                                                    sub_matcher,
                                                    external_domains_caseless,
                                                    &mut None,
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                response.0
            }
            Ok(res) => {
                let pr = handle_response_bytes(res, url, only_html).await;
                if pr.content_truncated {
                    log::warn!("Response truncated for {url}, retrying once");
                    match client.get(url).send().await {
                        Ok(res2) => handle_response_bytes(res2, url, only_html).await,
                        Err(_) => pr,
                    }
                } else {
                    pr
                }
            }
            Err(err) => {
                log::info!("error fetching {}", url);

                let mut page_response = PageResponse::default();

                if let Some(status_code) = err.status() {
                    page_response.status_code = status_code;
                } else {
                    page_response.status_code = crate::page::get_error_http_status_code(&err);
                }

                page_response.error_for_status = Some(Err(err));

                page_response
            }
        };

        let valid_meta = meta_title.is_some()
            || meta_description.is_some()
            || meta_og_image.is_some()
            || metadata.is_some();

        if valid_meta {
            let mut metadata_inner = Metadata::default();
            metadata_inner.title = meta_title;
            metadata_inner.description = meta_description;
            metadata_inner.image = meta_og_image;

            if metadata_inner.exist() {
                // Preserve automation results from existing metadata if present
                set_metadata(&metadata, &mut metadata_inner);
                metadata.replace(Box::new(metadata_inner));
            }

            if metadata.is_some() {
                page_response.metadata = metadata;
            }
        }

        crate::utils::set_page_response_duration(&mut page_response, duration);

        build(url, page_response)
    }

    /// Instantiate a new page and gather the html repro of standard fetch_page_html only gathering resources to crawl.
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub async fn new_page_only_html(url: &str, client: &Client) -> Self {
        let page_resource = crate::utils::fetch_page_html_raw_only_html(url, client).await;
        build(url, page_resource)
    }

    /// Instantiate a new page and gather the html.
    #[cfg(all(not(feature = "decentralized"), not(feature = "chrome")))]
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub async fn new(url: &str, client: &Client) -> Self {
        let page_resource = crate::utils::fetch_page_html(url, client).await;
        build(url, page_resource)
    }

    /// Instantiate a new page and gather the links from input bytes.
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    #[cfg(feature = "cmd")]
    pub async fn new_page_streaming_from_bytes<
        A: PartialEq
            + Eq
            + Sync
            + Send
            + Clone
            + Default
            + std::hash::Hash
            + From<String>
            + for<'a> From<&'a str>,
    >(
        url: &str,
        input_bytes: &[u8],
        selectors: &mut RelativeSelectors,
        external_domains_caseless: &Arc<HashSet<CaseInsensitiveString>>,
        r_settings: &PageLinkBuildSettings,
        map: &mut hashbrown::HashSet<A>,
        ssg_map: Option<&mut hashbrown::HashSet<A>>,
        prior_domain: &Option<Box<Url>>,
        domain_parsed: &mut Option<Box<Url>>,
        links_pages: &mut Option<hashbrown::HashSet<A>>,
    ) -> Self {
        use crate::utils::{modify_selectors, AllowedDomainTypes};

        let mut metadata: Option<Box<Metadata>> = None;
        let mut meta_title: Option<_> = None;
        let mut meta_description: Option<_> = None;
        let mut meta_og_image: Option<_> = None;

        let duration = if cfg!(feature = "time") {
            Some(tokio::time::Instant::now())
        } else {
            None
        };

        let encoding = AsciiCompatibleEncoding::utf_8();
        let adjust_charset_on_meta_tag = true;

        let base_input_url = tokio::sync::OnceCell::new();

        let original_page = Url::parse(url).ok();

        if ssg_map.is_some() {
            let mut ci_url = Box::new(CaseInsensitiveString::new(url));
            modify_selectors(
                prior_domain,
                url,
                domain_parsed,
                &mut ci_url,
                selectors,
                AllowedDomainTypes::new(r_settings.subdomains, r_settings.tld),
            );
        }

        let base = if domain_parsed.is_none() {
            prior_domain
        } else {
            domain_parsed
        };

        let parent_host = &selectors.1[0];
        let parent_host_scheme = &selectors.1[1];
        let base_input_domain = &selectors.2;
        let sub_matcher = &selectors.0;

        let xml_file = url.ends_with(".xml");

        let base_links_settings = if r_settings.full_resources {
            lol_html::element!(
                "a[href]:not([aria-hidden=\"true\"]),script[src],link[href]",
                |el| {
                    let tag_name = el.tag_name();
                    let attribute = if tag_name == "script" { "src" } else { "href" };

                    if let Some(href) = el.get_attribute(attribute) {
                        let base = if relative_directory_url(&href) || base.is_none() {
                            original_page.as_ref()
                        } else {
                            base.as_deref()
                        };
                        let base = if base_input_url.initialized() {
                            base_input_url.get()
                        } else {
                            base
                        };

                        push_link(
                            &base,
                            &href,
                            map,
                            &selectors.0,
                            parent_host,
                            parent_host_scheme,
                            base_input_domain,
                            sub_matcher,
                            external_domains_caseless,
                            links_pages,
                        );
                    }

                    Ok(())
                }
            )
        } else {
            element_precompiled!(
                if xml_file {
                    compiled_xml_selector()
                } else {
                    compiled_selector()
                },
                |el| {
                    if let Some(href) = el.get_attribute("href") {
                        let base = if relative_directory_url(&href) || base.is_none() {
                            original_page.as_ref()
                        } else {
                            base.as_deref()
                        };
                        let base = if base_input_url.initialized() {
                            base_input_url.get()
                        } else {
                            base
                        };

                        push_link(
                            &base,
                            &href,
                            map,
                            &selectors.0,
                            parent_host,
                            parent_host_scheme,
                            base_input_domain,
                            sub_matcher,
                            external_domains_caseless,
                            links_pages,
                        );
                    }
                    Ok(())
                }
            )
        };

        let mut element_content_handlers =
            Vec::with_capacity(if r_settings.ssg_build { 2 } else { 1 } + 4);

        element_content_handlers.push(element_precompiled!(
            compiled_base_element_selector(),
            |el| {
                if let Some(href) = el.get_attribute("href") {
                    if let Ok(parsed_base) = Url::parse(&href) {
                        let _ = base_input_url.set(parsed_base);
                    }
                }
                Ok(())
            }
        ));

        element_content_handlers.push(base_links_settings);

        element_content_handlers.extend(metadata_handlers(
            &mut meta_title,
            &mut meta_description,
            &mut meta_og_image,
        ));

        let settings = lol_html::send::Settings {
            element_content_handlers,
            adjust_charset_on_meta_tag,
            encoding,
            ..lol_html::send::Settings::new_for_handler_types()
        };

        let mut collected_bytes: Vec<u8> = match input_bytes.len() {
            n if n >= MAX_PRE_ALLOCATED_HTML_PAGE_SIZE_USIZE => Vec::with_capacity(n),
            n if n > 0 => Vec::with_capacity(n),
            _ => Vec::with_capacity(MAX_PRE_ALLOCATED_HTML_PAGE_SIZE_USIZE),
        };

        let mut rewriter = lol_html::send::HtmlRewriter::new(settings, |c: &[u8]| {
            collected_bytes.extend_from_slice(c);
        });

        let _ = rewriter.write(input_bytes);
        let _ = rewriter.end();

        let mut page_response = PageResponse::default();
        page_response.status_code = StatusCode::OK;

        if r_settings.normalize {
            page_response.signature = Some(hash_html(&collected_bytes).await);
        }

        if !collected_bytes.is_empty() {
            page_response.content = Some(collected_bytes);
        }

        let valid_meta = meta_title.is_some()
            || meta_description.is_some()
            || meta_og_image.is_some()
            || metadata.is_some();

        if valid_meta {
            let mut metadata_inner = Metadata::default();
            metadata_inner.title = meta_title;
            metadata_inner.description = meta_description;
            metadata_inner.image = meta_og_image;

            if metadata_inner.exist() {
                // Preserve automation results from existing metadata if present
                set_metadata(&metadata, &mut metadata_inner);
                metadata.replace(Box::new(metadata_inner));
            }

            if metadata.is_some() {
                page_response.metadata = metadata;
            }
        }

        crate::utils::set_page_response_duration(&mut page_response, duration);

        build(url, page_response)
    }

    #[cfg(all(not(feature = "decentralized"), feature = "chrome"))]
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    /// Instantiate a new page and gather the html.
    pub(crate) async fn new_base(
        url: &str,
        client: &Client,
        page: &chromiumoxide::Page,
        wait_for: &Option<crate::configuration::WaitFor>,
        screenshot: &Option<crate::configuration::ScreenShotConfig>,
        page_set: bool,
        openai_config: &Option<Box<crate::configuration::GPTConfigs>>,
        execution_scripts: &Option<ExecutionScripts>,
        automation_scripts: &Option<AutomationScripts>,
        viewport: &Option<crate::configuration::Viewport>,
        request_timeout: &Option<Duration>,
        track_events: &Option<crate::configuration::ChromeEventTracker>,
        referrer: Option<String>,
        max_page_bytes: Option<f64>,
        cache_options: Option<CacheOptions>,
        cache_policy: &Option<BasicCachePolicy>,
        seeded_resource: Option<String>,
        jar: Option<&std::sync::Arc<crate::client::cookie::Jar>>,
        remote_multimodal: &Option<Box<RemoteMultimodalConfigs>>,
        cache_namespace: Option<&str>,
    ) -> Self {
        let page_resource = if seeded_resource.is_some() {
            crate::utils::fetch_page_html_seeded(
                url,
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
        } else {
            crate::utils::fetch_page_html(
                url,
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
                remote_multimodal,
                cache_namespace,
            )
            .await
        };
        let mut p = build(url, page_resource);

        // store the chrome page to perform actions like screenshots etc.
        if cfg!(feature = "chrome_store_page") {
            p.chrome_page = Some(page.clone());
        }

        p
    }

    #[cfg(all(not(feature = "decentralized"), feature = "chrome"))]
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    /// Instantiate a new page and gather the html.
    pub async fn new(
        url: &str,
        client: &Client,
        page: &chromiumoxide::Page,
        wait_for: &Option<crate::configuration::WaitFor>,
        screenshot: &Option<crate::configuration::ScreenShotConfig>,
        page_set: bool,
        openai_config: &Option<Box<crate::configuration::GPTConfigs>>,
        execution_scripts: &Option<ExecutionScripts>,
        automation_scripts: &Option<AutomationScripts>,
        viewport: &Option<crate::configuration::Viewport>,
        request_timeout: &Option<Duration>,
        track_events: &Option<crate::configuration::ChromeEventTracker>,
        referrer: Option<String>,
        max_page_bytes: Option<f64>,
        cache_options: Option<CacheOptions>,
        cache_policy: &Option<BasicCachePolicy>,
        remote_multimodal: &Option<Box<RemoteMultimodalConfigs>>,
        cache_namespace: Option<&str>,
    ) -> Self {
        Self::new_base(
            url,
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

    #[cfg(all(not(feature = "decentralized"), feature = "chrome"))]
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    /// Instantiate a new page and gather the html seeded.
    pub async fn new_seeded(
        url: &str,
        client: &Client,
        page: &chromiumoxide::Page,
        wait_for: &Option<crate::configuration::WaitFor>,
        screenshot: &Option<crate::configuration::ScreenShotConfig>,
        page_set: bool,
        openai_config: &Option<Box<crate::configuration::GPTConfigs>>,
        execution_scripts: &Option<ExecutionScripts>,
        automation_scripts: &Option<AutomationScripts>,
        viewport: &Option<crate::configuration::Viewport>,
        request_timeout: &Option<Duration>,
        track_events: &Option<crate::configuration::ChromeEventTracker>,
        referrer: Option<String>,
        max_page_bytes: Option<f64>,
        cache_options: Option<CacheOptions>,
        cache_policy: &Option<BasicCachePolicy>,
        seeded_resource: Option<String>,
        jar: Option<&std::sync::Arc<crate::client::cookie::Jar>>,
        remote_multimodal: &Option<Box<RemoteMultimodalConfigs>>,
        cache_namespace: Option<&str>,
    ) -> Self {
        Self::new_base(
            url,
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

    /// Instantiate a new page and gather the links.
    #[cfg(all(feature = "decentralized", not(feature = "headers")))]
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub async fn new(url: &str, client: &Client) -> Self {
        Self::new_links_only(url, client).await
    }

    /// Instantiate a new page and gather the headers and links.
    #[cfg(all(feature = "decentralized", feature = "headers"))]
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all,))]
    pub async fn new(url: &str, client: &Client) -> Self {
        use crate::serde::Deserialize;

        match crate::utils::fetch_page_and_headers(url, client).await {
            FetchPageResult::Success(headers, page_content) => {
                let links = match page_content {
                    Some(b) => match flexbuffers::Reader::get_root(b.as_slice()) {
                        Ok(buf) => match HashSet::<CaseInsensitiveString>::deserialize(buf) {
                            Ok(link) => link,
                            _ => Default::default(),
                        },
                        _ => Default::default(),
                    },
                    _ => Default::default(),
                };
                Page {
                    html: None,
                    headers: Some(headers),
                    links,
                    ..Default::default()
                }
            }
            FetchPageResult::NoSuccess(headers) => Page {
                headers: Some(headers),
                ..Default::default()
            },
            FetchPageResult::FetchError => Default::default(),
        }
    }

    /// Instantiate a new page and gather the links.
    #[cfg(feature = "decentralized")]
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all,))]
    pub async fn new_links_only(url: &str, client: &Client) -> Self {
        use crate::serde::Deserialize;

        let links = match crate::utils::fetch_page(url, client).await {
            Some(b) => match flexbuffers::Reader::get_root(b.as_slice()) {
                Ok(buf) => match HashSet::<CaseInsensitiveString>::deserialize(buf) {
                    Ok(link) => link,
                    _ => Default::default(),
                },
                _ => Default::default(),
            },
            _ => Default::default(),
        };

        Page {
            html: None,
            links,
            ..Default::default()
        }
    }

    #[cfg(not(all(not(feature = "decentralized"), feature = "chrome")))]
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all,))]
    /// Take a screenshot of the page. If the output path is set to None the screenshot will not be saved.
    /// The feature flag `chrome_store_page` is required.
    pub async fn screenshot(
        &self,
        _full_page: bool,
        _omit_background: bool,
        _format: crate::configuration::CaptureScreenshotFormat,
        _quality: Option<i64>,
        _output_path: Option<impl AsRef<std::path::Path>>,
        _clip: Option<crate::configuration::ClipViewport>,
    ) -> Vec<u8> {
        Default::default()
    }

    #[cfg(all(not(feature = "decentralized"), feature = "chrome"))]
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    /// Take a screenshot of the page. If the output path is set to None the screenshot will not be saved.
    /// The feature flag `chrome_store_page` is required.
    pub async fn take_screenshot(
        page: &Page,
        full_page: bool,
        omit_background: bool,
        format: crate::configuration::CaptureScreenshotFormat,
        quality: Option<i64>,
        output_path: Option<impl AsRef<std::path::Path>>,
        clip: Option<crate::configuration::ClipViewport>,
    ) -> Vec<u8> {
        match &page.chrome_page {
            Some(chrome_page) => {
                let format: chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat =
                    chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat::from(
                        format,
                    );

                let screenshot_configs = chromiumoxide::page::ScreenshotParams::builder()
                    .format(format)
                    .full_page(full_page)
                    .omit_background(omit_background);

                let screenshot_configs = match quality {
                    Some(q) => screenshot_configs.quality(q),
                    _ => screenshot_configs,
                };

                let screenshot_configs = match clip {
                    Some(vp) => screenshot_configs.clip(
                        chromiumoxide::cdp::browser_protocol::page::Viewport::from(vp),
                    ),
                    _ => screenshot_configs,
                };

                if output_path.is_none() {
                    match chrome_page.screenshot(screenshot_configs.build()).await {
                        Ok(v) => {
                            log::debug!("took screenshot: {:?}", page.url);
                            v
                        }
                        Err(e) => {
                            log::error!("failed to took screenshot: {:?} - {:?}", e, page.url);
                            Default::default()
                        }
                    }
                } else {
                    let output_path = match output_path {
                        Some(out) => out.as_ref().to_path_buf(),
                        _ => Default::default(),
                    };

                    match chrome_page
                        .save_screenshot(screenshot_configs.build(), &output_path)
                        .await
                    {
                        Ok(v) => {
                            log::debug!("saved screenshot: {:?}", output_path);
                            v
                        }
                        Err(e) => {
                            log::error!("failed to save screenshot: {:?} - {:?}", e, output_path);
                            Default::default()
                        }
                    }
                }
            }
            _ => Default::default(),
        }
    }

    #[cfg(all(not(feature = "decentralized"), feature = "chrome"))]
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    /// Take a screenshot of the page. If the output path is set to None the screenshot will not be saved. The feature flag `chrome_store_page` is required.
    pub async fn screenshot(
        &self,
        full_page: bool,
        omit_background: bool,
        format: crate::configuration::CaptureScreenshotFormat,
        quality: Option<i64>,
        output_path: Option<impl AsRef<std::path::Path>>,
        clip: Option<crate::configuration::ClipViewport>,
    ) -> Vec<u8> {
        // prevent screenshot hangs
        let screenshot_result = tokio::time::timeout(
            tokio::time::Duration::from_secs(30),
            Page::take_screenshot(
                self,
                full_page,
                omit_background,
                format,
                quality,
                output_path,
                clip,
            ),
        )
        .await;
        match screenshot_result {
            Ok(sb) => sb,
            _ => Default::default(),
        }
    }

    #[cfg(all(feature = "chrome", not(feature = "decentralized")))]
    /// Get the chrome page used. The feature flag `chrome` is required.
    pub fn get_chrome_page(&self) -> Option<&chromiumoxide::Page> {
        self.chrome_page.as_ref()
    }

    #[cfg(all(feature = "chrome", feature = "decentralized"))]
    /// Get the chrome page used. The feature flag `chrome` is required.
    pub fn get_chrome_page(&self) -> Option<&chromiumoxide::Page> {
        None
    }

    #[cfg(all(not(feature = "decentralized"), feature = "chrome"))]
    /// Close the chrome page used. Useful when storing the page with subscription usage. The feature flag `chrome_store_page` is required.
    pub async fn close_page(&mut self) {
        if let Some(page) = self.chrome_page.as_mut() {
            let _ = page
                .send_command(chromiumoxide::cdp::browser_protocol::page::CloseParams::default())
                .await;
        }
    }

    #[cfg(all(feature = "decentralized", feature = "chrome"))]
    /// Close the chrome page used. Useful when storing the page for subscription usage. The feature flag `chrome_store_page` is required.
    pub async fn close_page(&mut self) {}

    /// Page request is empty. On chrome an empty page has bare html markup.
    #[inline]
    pub fn is_empty(&self) -> bool {
        match self.html.as_deref() {
            None => true,
            Some(html) => {
                let html = html.trim_ascii();
                html.is_empty() || html.eq(*EMPTY_HTML) || html.eq(*EMPTY_HTML_BASIC)
            }
        }
    }

    /// Url getter for page.
    #[cfg(not(feature = "decentralized"))]
    pub fn get_url(&self) -> &str {
        &self.url
    }

    #[cfg(not(feature = "headers"))]
    /// Get the timeout required for rate limiting. The max duration is 30 seconds for delay respecting. Requires the feature flag `headers`.
    pub fn get_timeout(&self) -> Option<Duration> {
        if self.status_code == 429 {
            return Some(Duration::from_millis(2_500));
        } else if self.status_code == StatusCode::GATEWAY_TIMEOUT {
            return Some(Duration::from_millis(1_500));
        } else if self.status_code.as_u16() >= 598 {
            // Proxy/unknown connection errors (599, 598) - brief backoff before
            // same-proxy retry to avoid hammering a transiently broken tunnel.
            return Some(Duration::from_millis(500));
        }

        None
    }

    #[cfg(feature = "headers")]
    /// Get the timeout required for rate limiting. The max duration is 30 seconds for delay respecting. Requires the feature flag `headers`.
    pub fn get_timeout(&self) -> Option<Duration> {
        if self.status_code == 429 {
            const MAX_TIMEOUT: Duration = Duration::from_secs(30);
            if let Some(headers) = &self.headers {
                if let Some(retry_after) = headers.get(reqwest::header::RETRY_AFTER) {
                    if let Ok(retry_after_str) = retry_after.to_str() {
                        if let Ok(seconds) = retry_after_str.parse::<u64>() {
                            return Some(Duration::from_secs(seconds).min(MAX_TIMEOUT));
                        }
                        if let Ok(date) = httpdate::parse_http_date(retry_after_str) {
                            if let Ok(duration) = date.duration_since(std::time::SystemTime::now())
                            {
                                return Some(duration.min(MAX_TIMEOUT));
                            }
                        }
                    }
                }
            };
        } else if self.status_code == StatusCode::GATEWAY_TIMEOUT {
            return Some(Duration::from_millis(1_500));
        } else if self.status_code.as_u16() >= 598 {
            // Proxy/unknown connection errors (599, 598) - brief backoff before
            // same-proxy retry to avoid hammering a transiently broken tunnel.
            return Some(Duration::from_millis(500));
        }

        None
    }

    /// Url getter for page after redirects.
    #[cfg(not(feature = "decentralized"))]
    pub fn get_url_final(&self) -> &str {
        match self.final_redirect_destination.as_ref() {
            Some(u) => u,
            _ => &self.url,
        }
    }

    /// Set the external domains to treat as one
    pub fn set_external(&mut self, external_domains_caseless: Arc<HashSet<CaseInsensitiveString>>) {
        self.external_domains_caseless = external_domains_caseless;
    }

    /// Set the html directly of the page
    pub fn set_html_bytes(&mut self, html: Option<Vec<u8>>) {
        self.html = html.map(bytes::Bytes::from);
    }

    /// Set the url directly of the page. Useful for transforming the content and rewriting the url.
    #[cfg(not(feature = "decentralized"))]
    pub fn set_url(&mut self, url: String) {
        self.url = url;
    }

    /// Set the url directly parsed url of the page.
    /// Uses the final redirect destination when available, so that relative link
    /// resolution and parent_host_match use the actual origin after redirects.
    #[cfg(not(feature = "decentralized"))]
    pub fn set_url_parsed_direct(&mut self) {
        let effective_url = match &self.final_redirect_destination {
            Some(u) => u.as_str(),
            None => &self.url,
        };
        if let Ok(base) = Url::parse(effective_url) {
            self.base = Some(base);
        }
    }

    /// Set the url directly parsed url of the page. Useful for transforming the content and rewriting the url.
    #[cfg(not(feature = "decentralized"))]
    pub fn set_url_parsed_direct_empty(&mut self) {
        if !self.base.is_some() && !self.url.is_empty() {
            self.set_url_parsed_direct()
        }
    }

    /// Set the url directly parsed url of the page.
    #[cfg(feature = "decentralized")]
    pub fn set_url_parsed_direct(&mut self) {}

    /// Set the url directly parsed url of the page. Useful for transforming the content and rewriting the url.
    #[cfg(feature = "decentralized")]
    pub fn set_url_parsed_direct_empty(&mut self) {}

    /// Set the url directly parsed url of the page. Useful for transforming the content and rewriting the url.
    #[cfg(not(feature = "decentralized"))]
    pub fn set_url_parsed(&mut self, url_parsed: Url) {
        self.base = Some(url_parsed);
    }

    /// Parsed URL getter for page.
    #[cfg(not(feature = "decentralized"))]
    pub fn get_url_parsed_ref(&self) -> &Option<Url> {
        &self.base
    }

    /// Parsed URL getter for page.
    #[cfg(not(feature = "decentralized"))]
    pub fn get_url_parsed(&mut self) -> &Option<Url> {
        if self.base.is_none() && !self.url.is_empty() {
            self.base = Url::parse(&self.url).ok();
        }
        &self.base
    }

    /// Parsed URL getter for page.
    #[cfg(feature = "decentralized")]
    pub fn get_url_parsed(&self) -> &Option<Url> {
        &None
    }

    /// Parsed URL getter for page.
    #[cfg(feature = "decentralized")]
    pub fn get_url_parsed_ref(&self) -> &Option<Url> {
        &None
    }

    /// Take the parsed url.
    #[cfg(not(feature = "decentralized"))]
    pub fn take_url(&mut self) -> Option<Url> {
        self.base.take()
    }

    /// Take the parsed url.
    #[cfg(feature = "decentralized")]
    pub fn take_url(&mut self) -> Option<Url> {
        None
    }

    #[cfg(feature = "decentralized")]
    /// URL getter for page.
    pub fn get_url(&self) -> &str {
        &self.url
    }

    /// Html getter for bytes on the page.
    pub fn get_bytes(&self) -> Option<&[u8]> {
        self.html.as_deref()
    }

    /// Html getter for bytes on the page as string.
    pub fn get_html(&self) -> String {
        match self.get_html_cow() {
            std::borrow::Cow::Borrowed(s) => s.to_string(),
            std::borrow::Cow::Owned(s) => s,
        }
    }

    /// Content getter — returns the page body as a string.
    ///
    /// This is an alias for [`get_html`](Self::get_html) that works with any
    /// return format (HTML, markdown, text, etc.) set via
    /// [`SpiderCloudConfig::with_return_format`](crate::configuration::SpiderCloudConfig::with_return_format)
    /// or transformed locally with `spider_transformations`.
    #[inline]
    pub fn get_content(&self) -> String {
        self.get_html()
    }

    /// Html getter that avoids allocation when the content is already valid UTF-8.
    /// Returns `Cow::Borrowed` for UTF-8 content (common case), `Cow::Owned` when
    /// encoding conversion is needed.
    pub fn get_html_cow(&self) -> std::borrow::Cow<'_, str> {
        match self.html.as_deref() {
            Some(bytes) => match std::str::from_utf8(bytes) {
                Ok(s) => std::borrow::Cow::Borrowed(s),
                Err(_) => std::borrow::Cow::Owned(auto_encoder::auto_encode_bytes(bytes)),
            },
            None => std::borrow::Cow::Borrowed(""),
        }
    }

    /// Html getter for page to u8.
    pub fn get_html_bytes_u8(&self) -> &[u8] {
        match self.html.as_deref() {
            Some(html) => html,
            _ => Default::default(),
        }
    }

    /// Content getter as raw bytes — alias for [`get_html_bytes_u8`](Self::get_html_bytes_u8).
    ///
    /// Works with any return format (HTML, markdown, text, etc.).
    #[inline]
    pub fn get_content_bytes(&self) -> &[u8] {
        self.get_html_bytes_u8()
    }

    /// Get content for a specific return format from a multi-format response.
    ///
    /// Returns `None` if multi-format was not requested or the format is not present.
    /// Use [`with_return_formats`](crate::configuration::SpiderCloudConfig::with_return_formats)
    /// on `SpiderCloudConfig` to request multiple formats.
    #[cfg(feature = "spider_cloud")]
    #[inline]
    pub fn get_content_for(&self, format: &str) -> Option<String> {
        self.content_map.as_ref().and_then(|map| {
            map.get(format).map(|b| {
                std::str::from_utf8(b)
                    .map(|s| s.to_string())
                    .unwrap_or_else(|_| auto_encoder::auto_encode_bytes(b))
            })
        })
    }

    /// Get content for a specific return format as raw bytes.
    ///
    /// Returns `None` if multi-format was not requested or the format is not present.
    #[cfg(feature = "spider_cloud")]
    #[inline]
    pub fn get_content_bytes_for(&self, format: &str) -> Option<&[u8]> {
        self.content_map
            .as_ref()
            .and_then(|map| map.get(format).map(|b| b.as_ref()))
    }

    /// Check if this page has multi-format content available.
    #[cfg(feature = "spider_cloud")]
    #[inline]
    pub fn has_content_map(&self) -> bool {
        self.content_map.as_ref().is_some_and(|m| !m.is_empty())
    }

    /// Compute an HTML quality score (0–100) for this page.
    ///
    /// Uses status code, content length, structural HTML checks,
    /// and anti-bot detection to score the response.
    #[cfg(feature = "parallel_backends")]
    #[inline]
    pub fn quality_score(&self) -> u16 {
        crate::utils::parallel_backends::html_quality_score(
            self.html.as_deref(),
            self.status_code,
            &self.anti_bot_tech,
        )
    }

    /// Modify xml - html.
    #[cfg(all(
        feature = "sitemap",
        feature = "chrome",
        not(feature = "decentralized")
    ))]
    pub(crate) fn modify_xml_html(&mut self) -> &[u8] {
        if let Some(html_bytes) = self.html.take() {
            const XML_DECL: &str = r#"<?xml version="1.0" encoding="UTF-8"?>"#;

            let xml = html_bytes.as_ref();
            if let Ok(xml_str) = std::str::from_utf8(xml) {
                let stripped = xml_str
                    .strip_prefix(XML_DECL)
                    .map(|f| f.trim_start())
                    .unwrap_or(xml_str);
                // Zero-copy: Bytes::slice shares the same Arc backing buffer.
                let offset = stripped.as_ptr() as usize - xml.as_ptr() as usize;
                self.html = Some(html_bytes.slice(offset..offset + stripped.len()));
            } else {
                self.html = Some(html_bytes);
            }
        }

        self.html.as_deref().unwrap_or_default()
    }

    /// Get the response events mapped.
    #[cfg(feature = "chrome")]
    pub fn get_responses(&self) -> &Option<hashbrown::HashMap<String, f64>> {
        &self.response_map
    }

    /// Get the metadata found on the page.
    pub fn get_metadata(&self) -> &Option<Box<Metadata>> {
        &self.metadata
    }

    /// Get the response events mapped.
    #[cfg(feature = "chrome")]
    pub fn get_request(&self) -> &Option<hashbrown::HashMap<String, f64>> {
        &self.request_map
    }

    /// Html getter for getting the content with proper encoding. Pass in a proper encoding label like SHIFT_JIS. This fallsback to get_html without the `encoding` flag enabled.
    #[cfg(feature = "encoding")]
    pub fn get_html_encoded(&self, label: &str) -> String {
        get_html_encoded(&self.html, label)
    }

    /// Html getter for getting the content with proper encoding. Pass in a proper encoding label like SHIFT_JIS. This fallsback to get_html without the `encoding` flag enabled.
    #[cfg(not(feature = "encoding"))]
    pub fn get_html_encoded(&self, _label: &str) -> String {
        self.get_html()
    }

    /// Set the elasped duration of the page since scraped from duration.
    #[inline]
    #[cfg(all(feature = "time", not(feature = "decentralized")))]
    pub fn set_duration_elapsed(&mut self, scraped_at: Option<Instant>) {
        self.duration = scraped_at;
    }

    /// Set the elasped duration of the page since scraped from duration.
    #[inline]
    #[cfg(all(feature = "time", not(feature = "decentralized")))]
    pub fn set_duration_elapsed_from_duration(&mut self, elapsed: Option<std::time::Duration>) {
        self.duration = elapsed.map(|d| Instant::now().checked_sub(d).unwrap_or_else(Instant::now));
    }

    /// Get the elasped duration of the page since scraped.
    #[cfg(all(feature = "time", not(feature = "decentralized")))]
    pub fn get_duration_elapsed(&self) -> Duration {
        self.duration
            .as_ref()
            .map(|t| t.elapsed())
            .unwrap_or_default()
    }

    /// Set the elapsed duration of the page since scraped from duration.
    #[inline]
    #[cfg(all(feature = "time", feature = "decentralized"))]
    pub fn set_duration_elapsed(&mut self, scraped_at: Option<Instant>) {
        self.duration = scraped_at;
    }

    /// Set the elapsed duration of the page since scraped from duration.
    #[inline]
    #[cfg(all(feature = "time", feature = "decentralized"))]
    pub fn set_duration_elapsed_from_duration(&mut self, elapsed: Option<std::time::Duration>) {
        self.duration = elapsed.map(|d| Instant::now().checked_sub(d).unwrap_or_else(Instant::now));
    }

    /// Get the elapsed duration of the page since scraped.
    #[cfg(all(feature = "time", feature = "decentralized"))]
    pub fn get_duration_elapsed(&self) -> Duration {
        self.duration
            .as_ref()
            .map(|t| t.elapsed())
            .unwrap_or_default()
    }

    /// Find the links as a stream using string resource validation for XML files
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub async fn links_stream_xml_links_stream_base<
        A: PartialEq
            + Eq
            + Sync
            + Send
            + Clone
            + Default
            + ToString
            + std::hash::Hash
            + From<String>
            + Into<CaseInsensitiveString>
            + for<'a> From<&'a str>,
    >(
        &mut self,
        selectors: &RelativeSelectors,
        xml: &str,
        map: &mut HashSet<A>,
        base: &Option<Box<Url>>,
    ) {
        use quick_xml::events::Event;
        use quick_xml::reader::NsReader;

        let mut reader = NsReader::from_reader(xml.as_bytes());

        reader.config_mut().trim_text(true);

        // Take the thread-local buffer (owned, safe across awaits).
        // If re-entered or migrated, we just get an empty Vec — no panic.
        let mut buf = XML_PARSE_BUF.with(|c| c.take());

        let parent_host = &selectors.1[0];
        let parent_host_scheme = &selectors.1[1];
        let base_input_domain = &selectors.2;
        let sub_matcher = &selectors.0;

        let mut is_link_tag = false;
        let mut links_pages: Option<HashSet<A>> = if self.page_links.is_some() {
            Some(HashSet::new())
        } else {
            None
        };

        let base = if base.is_some() {
            base.as_deref()
        } else {
            self.set_url_parsed_direct_empty();
            let base = self.get_url_parsed_ref().as_ref();
            base
        };

        loop {
            match reader.read_event_into_async(&mut buf).await {
                Ok(e) => match e {
                    Event::Start(e) => {
                        let (_, local) = reader.resolver().resolve_element(e.name());

                        if local.as_ref() == b"link" {
                            is_link_tag = true;
                        }
                    }
                    Event::Text(e) => {
                        if is_link_tag {
                            if let Ok(v) = e.decode() {
                                push_link_verify(
                                    &base,
                                    &v,
                                    map,
                                    &selectors.0,
                                    parent_host,
                                    parent_host_scheme,
                                    base_input_domain,
                                    sub_matcher,
                                    &self.external_domains_caseless,
                                    false,
                                    &mut links_pages,
                                    true,
                                );
                            }
                        }
                    }
                    Event::End(ref e) => {
                        let (_, local) = reader.resolver().resolve_element(e.name());

                        if local.as_ref() == b"link" {
                            is_link_tag = false;
                        }
                    }
                    Event::Eof => {
                        break;
                    }
                    _ => (),
                },
                _ => break,
            }
            buf.clear();
        }

        // Return the buffer to the thread-local pool (retains capacity for reuse).
        buf.clear();
        XML_PARSE_BUF.with(|c| c.set(buf));

        if let Some(lp) = links_pages {
            let page_links = self.page_links.get_or_insert_with(Default::default);
            page_links.extend(lp.into_iter().map(Into::into));
        }
    }

    /// Find the links as a stream using string resource validation
    #[inline(always)]
    #[cfg(not(feature = "decentralized"))]
    pub async fn links_stream_base<
        A: PartialEq
            + Eq
            + Sync
            + Send
            + Clone
            + Default
            + ToString
            + std::hash::Hash
            + From<String>
            + Into<CaseInsensitiveString>
            + for<'a> From<&'a str>,
    >(
        &mut self,
        selectors: &RelativeSelectors,
        html: &str,
        base: &Option<Box<Url>>,
    ) -> HashSet<A> {
        let mut map: HashSet<A> = HashSet::with_capacity(link_set_capacity());
        let mut links_pages: Option<HashSet<A>> = if self.page_links.is_some() {
            Some(HashSet::new())
        } else {
            None
        };

        let mut metadata: Option<Box<Metadata>> = None;
        let mut meta_title: Option<_> = None;
        let mut meta_description: Option<_> = None;
        let mut meta_og_image: Option<_> = None;

        if !html.is_empty() {
            if html.starts_with("<?xml") {
                self.links_stream_xml_links_stream_base(selectors, html, &mut map, base)
                    .await;
            } else {
                let base_input_url = tokio::sync::OnceCell::new();

                let parent_host = &selectors.1[0];
                // the host schemes
                let parent_host_scheme = &selectors.1[1];
                let base_input_domain = &selectors.2; // the domain after redirects
                let sub_matcher = &selectors.0;

                let base = base.as_deref();

                // original domain to match local pages.
                let original_page = {
                    self.set_url_parsed_direct_empty();
                    self.get_url_parsed_ref().as_ref()
                };

                let xml_file = self.get_url().ends_with(".xml");

                let mut element_content_handlers =
                    metadata_handlers(&mut meta_title, &mut meta_description, &mut meta_og_image);

                element_content_handlers.push(element_precompiled!(
                    compiled_base_element_selector(),
                    |el| {
                        if let Some(href) = el.get_attribute("href") {
                            if let Ok(parsed_base) = Url::parse(&href) {
                                let _ = base_input_url.set(parsed_base);
                            }
                        }

                        Ok(())
                    }
                ));

                element_content_handlers.push(element_precompiled!(
                    if xml_file {
                        compiled_xml_selector()
                    } else {
                        compiled_selector()
                    },
                    |el| {
                        if let Some(href) = el.get_attribute("href") {
                            let base = if relative_directory_url(&href) || base.is_none() {
                                original_page
                            } else {
                                base
                            };
                            let base = if base_input_url.initialized() {
                                base_input_url.get()
                            } else {
                                base
                            };

                            push_link(
                                &base,
                                &href,
                                &mut map,
                                &selectors.0,
                                parent_host,
                                parent_host_scheme,
                                base_input_domain,
                                sub_matcher,
                                &self.external_domains_caseless,
                                &mut links_pages,
                            );
                        }
                        Ok(())
                    }
                ));

                let rewriter_settings = lol_html::Settings {
                    element_content_handlers,
                    adjust_charset_on_meta_tag: true,
                    ..lol_html::send::Settings::new_for_handler_types()
                };

                let mut wrote_error = false;

                let mut rewriter =
                    lol_html::send::HtmlRewriter::new(rewriter_settings, |_c: &[u8]| {});

                let html_bytes = html.as_bytes();
                let should_yield = html_bytes.len() > REWRITER_YIELD_THRESHOLD;

                for (i, chunk) in html_bytes.chunks(*STREAMING_CHUNK_SIZE).enumerate() {
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
            }
        }

        if let Some(lp) = links_pages {
            let page_links = self.page_links.get_or_insert_with(Default::default);
            page_links.extend(lp.into_iter().map(Into::into));
        }

        let valid_meta =
            meta_title.is_some() || meta_description.is_some() || meta_og_image.is_some();

        if valid_meta {
            let mut metadata_inner = Metadata::default();
            metadata_inner.title = meta_title;
            metadata_inner.description = meta_description;
            metadata_inner.image = meta_og_image;

            if metadata_inner.exist() {
                metadata.replace(Box::new(metadata_inner));
            }

            if metadata.is_some() {
                self.metadata = metadata;
            }
        }

        update_link_capacity_hint(map.len());

        map
    }

    /// Find the links as a stream using string resource validation
    #[inline(always)]
    #[cfg(not(feature = "decentralized"))]
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub async fn links_stream_base_ssg<
        A: PartialEq
            + Eq
            + Sync
            + Send
            + Clone
            + Default
            + ToString
            + std::hash::Hash
            + From<String>
            + Into<CaseInsensitiveString>
            + for<'a> From<&'a str>,
    >(
        &mut self,
        selectors: &RelativeSelectors,
        html: &str,
        client: &Client,
        base: &Option<Box<Url>>,
    ) -> HashSet<A> {
        use auto_encoder::auto_encode_bytes;

        let mut map: HashSet<A> = HashSet::with_capacity(link_set_capacity());
        let mut map_ssg: HashSet<A> = HashSet::new();
        let mut links_pages: Option<HashSet<A>> = if self.page_links.is_some() {
            Some(HashSet::new())
        } else {
            None
        };

        let mut metadata: Option<Box<Metadata>> = None;
        let mut meta_title: Option<_> = None;
        let mut meta_description: Option<_> = None;
        let mut meta_og_image: Option<_> = None;

        if !html.is_empty() {
            if html.starts_with("<?xml") {
                self.links_stream_xml_links_stream_base(selectors, html, &mut map, base)
                    .await;
            } else {
                let cell = tokio::sync::OnceCell::new();
                let base_input_url = tokio::sync::OnceCell::new();

                // the original url
                let parent_host = &selectors.1[0];
                // the host schemes
                let parent_host_scheme = &selectors.1[1];
                let base_input_domain = &selectors.2; // the domain after redirects
                let sub_matcher = &selectors.0;

                let base = base.as_deref();

                // original domain to match local pages.
                let original_page = {
                    self.set_url_parsed_direct_empty();
                    self.get_url_parsed_ref().as_ref()
                };

                let xml_file = self.get_url().ends_with(".xml");

                let mut element_content_handlers =
                    metadata_handlers(&mut meta_title, &mut meta_description, &mut meta_og_image);

                element_content_handlers.push(element_precompiled!(
                    compiled_base_element_selector(),
                    |el| {
                        if let Some(href) = el.get_attribute("href") {
                            if let Ok(parsed_base) = Url::parse(&href) {
                                let _ = base_input_url.set(parsed_base);
                            }
                        }

                        Ok(())
                    }
                ));

                element_content_handlers.push(element_precompiled!(
                    if xml_file {
                        compiled_xml_selector()
                    } else {
                        compiled_selector()
                    },
                    |el| {
                        if let Some(href) = el.get_attribute("href") {
                            let base = if relative_directory_url(&href) || base.is_none() {
                                original_page
                            } else {
                                base
                            };
                            let base = if base_input_url.initialized() {
                                base_input_url.get()
                            } else {
                                base
                            };

                            push_link(
                                &base,
                                &href,
                                &mut map,
                                &selectors.0,
                                parent_host,
                                parent_host_scheme,
                                base_input_domain,
                                sub_matcher,
                                &self.external_domains_caseless,
                                &mut links_pages,
                            );
                        }
                        Ok(())
                    }
                ));

                element_content_handlers.push(lol_html::element!("script[src]", |el| {
                    if let Some(source) = el.get_attribute("src") {
                        if source.starts_with("/_next/static/")
                            && source.ends_with("/_ssgManifest.js")
                        {
                            if let Some(build_path) = base.map(|b| convert_abs_path(b, &source)) {
                                let _ = cell.set(build_path.to_string());
                            }
                        }
                    }
                    Ok(())
                }));

                let rewriter_settings = lol_html::Settings {
                    element_content_handlers,
                    adjust_charset_on_meta_tag: true,
                    ..lol_html::send::Settings::new_for_handler_types()
                };

                let mut rewriter =
                    lol_html::send::HtmlRewriter::new(rewriter_settings, |_c: &[u8]| {});

                let html_bytes = html.as_bytes();
                let mut wrote_error = false;
                let should_yield = html_bytes.len() > REWRITER_YIELD_THRESHOLD;

                for (i, chunk) in html_bytes.chunks(*STREAMING_CHUNK_SIZE).enumerate() {
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

                if let Some(build_ssg_path) = cell.get() {
                    if !build_ssg_path.is_empty() {
                        let build_page = Page::new_page(build_ssg_path, client).await;

                        for cap in SSG_CAPTURE.captures_iter(build_page.get_html_bytes_u8()) {
                            if let Some(matched) = cap.get(1) {
                                let href =
                                    auto_encode_bytes(matched.as_bytes()).replace(r#"\u002F"#, "/");

                                let last_segment = crate::utils::get_last_segment(&href);

                                // we can pass in a static map of the dynamic SSG routes pre-hand, custom API endpoint to seed, or etc later.
                                if !(last_segment.starts_with("[") && last_segment.ends_with("]")) {
                                    let base = if relative_directory_url(&href) || base.is_none() {
                                        original_page
                                    } else {
                                        base
                                    };
                                    let base = if base_input_url.initialized() {
                                        base_input_url.get()
                                    } else {
                                        base
                                    };

                                    push_link(
                                        &base,
                                        &href,
                                        &mut map_ssg,
                                        &selectors.0,
                                        parent_host,
                                        parent_host_scheme,
                                        base_input_domain,
                                        sub_matcher,
                                        &self.external_domains_caseless,
                                        &mut None,
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }

        if let Some(lp) = links_pages {
            let page_links = self.page_links.get_or_insert_with(Default::default);
            page_links.extend(lp.into_iter().map(Into::into));
        }

        let valid_meta = meta_title.is_some()
            || meta_description.is_some()
            || meta_og_image.is_some()
            || self.get_metadata().is_some();

        if valid_meta {
            let mut metadata_inner = Metadata::default();
            metadata_inner.title = meta_title;
            metadata_inner.description = meta_description;
            metadata_inner.image = meta_og_image;

            if metadata_inner.exist() && self.get_metadata().is_some() {
                set_metadata(self.get_metadata(), &mut metadata_inner);
            }

            if metadata_inner.exist() {
                metadata.replace(Box::new(metadata_inner));
            }

            if metadata.is_some() {
                self.metadata = metadata;
            }
        }

        map.extend(map_ssg);

        update_link_capacity_hint(map.len());

        map
    }

    /// Find the links as a stream using string resource validation and parsing the script for nextjs initial SSG paths.
    #[cfg(not(feature = "decentralized"))]
    pub async fn links_stream_ssg<
        A: PartialEq
            + Eq
            + Sync
            + Send
            + Clone
            + Default
            + ToString
            + std::hash::Hash
            + From<String>
            + Into<CaseInsensitiveString>
            + for<'a> From<&'a str>,
    >(
        &mut self,
        selectors: &RelativeSelectors,
        client: &Client,
        prior_domain: &Option<Box<Url>>,
    ) -> HashSet<A> {
        if auto_encoder::is_binary_file(self.get_html_bytes_u8()) {
            Default::default()
        } else if let Some(html_bytes) = self.html.take() {
            let html = match std::str::from_utf8(&html_bytes) {
                Ok(s) => std::borrow::Cow::Borrowed(s),
                Err(_) => std::borrow::Cow::Owned(auto_encoder::auto_encode_bytes(&html_bytes)),
            };
            let result = self
                .links_stream_base_ssg(selectors, &html, client, prior_domain)
                .await;
            self.html = Some(html_bytes);
            result
        } else {
            Default::default()
        }
    }

    /// Find all href links and return them using CSS selectors.
    #[inline(always)]
    #[cfg(not(feature = "decentralized"))]
    pub async fn links_ssg(
        &mut self,
        selectors: &RelativeSelectors,
        client: &Client,
        prior_domain: &Option<Box<Url>>,
    ) -> HashSet<CaseInsensitiveString> {
        match self.html.is_some() {
            false => Default::default(),
            true => {
                self.links_stream_ssg::<CaseInsensitiveString>(selectors, client, prior_domain)
                    .await
            }
        }
    }

    /// Find the links as a stream using string resource validation
    #[inline(always)]
    #[cfg(all(not(feature = "decentralized"), not(feature = "full_resources")))]
    pub async fn links_stream<
        A: PartialEq
            + Eq
            + Sync
            + Send
            + Clone
            + Default
            + ToString
            + std::hash::Hash
            + From<String>
            + Into<CaseInsensitiveString>
            + for<'a> From<&'a str>,
    >(
        &mut self,
        selectors: &RelativeSelectors,
        base: &Option<Box<Url>>,
    ) -> HashSet<A> {
        if auto_encoder::is_binary_file(self.get_html_bytes_u8()) {
            Default::default()
        } else if let Some(html_bytes) = self.html.take() {
            let html = match std::str::from_utf8(&html_bytes) {
                Ok(s) => std::borrow::Cow::Borrowed(s),
                Err(_) => std::borrow::Cow::Owned(auto_encoder::auto_encode_bytes(&html_bytes)),
            };
            let result = self.links_stream_base(selectors, &html, base).await;
            self.html = Some(html_bytes);
            result
        } else {
            Default::default()
        }
    }

    /// Find the links as a stream using string resource validation
    #[cfg(all(
        not(feature = "decentralized"),
        not(feature = "full_resources"),
        feature = "smart"
    ))]
    #[inline(always)]
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub(crate) async fn links_stream_smart<
        A: PartialEq
            + Eq
            + Sync
            + Send
            + Clone
            + Default
            + ToString
            + std::hash::Hash
            + From<String>
            + Into<CaseInsensitiveString>
            + for<'a> From<&'a str>,
    >(
        &mut self,
        selectors: &RelativeSelectors,
        configuration: &crate::configuration::Configuration,
        base: &Option<Box<Url>>,
        browser: &crate::features::chrome::OnceBrowser,
        jar: Option<&std::sync::Arc<crate::client::cookie::Jar>>,
    ) -> (HashSet<A>, Option<f64>) {
        use auto_encoder::auto_encode_bytes;
        use lol_html::{element, text};
        use std::sync::atomic::Ordering;

        let mut bytes_transferred: Option<f64> = None;
        let mut map: HashSet<A> = HashSet::with_capacity(link_set_capacity());
        let mut inner_map: HashSet<A> = HashSet::new();
        let mut links_pages: Option<HashSet<A>> = if self.page_links.is_some() {
            Some(HashSet::new())
        } else {
            None
        };

        let mut metadata: Option<Box<Metadata>> = None;
        let mut meta_title: Option<_> = None;
        let mut meta_description: Option<_> = None;
        let mut meta_og_image: Option<_> = None;

        if !self.is_empty() {
            let html_bytes_taken = self.html.take();
            let html_resource = match html_bytes_taken.as_deref() {
                Some(b) => match std::str::from_utf8(b) {
                    Ok(s) => std::borrow::Cow::Borrowed(s),
                    Err(_) => std::borrow::Cow::Owned(auto_encode_bytes(b)),
                },
                None => std::borrow::Cow::Borrowed(""),
            };

            if html_resource.starts_with("<?xml") {
                self.links_stream_xml_links_stream_base(selectors, &html_resource, &mut map, &base)
                    .await;
            } else {
                let base_input_url = tokio::sync::OnceCell::new();

                let base_input_domain = &selectors.2;
                let parent_frags = &selectors.1; // todo: allow mix match tpt
                let parent_host = &parent_frags[0];
                let parent_host_scheme = &parent_frags[1];
                let sub_matcher = &selectors.0;

                let external_domains_caseless = self.external_domains_caseless.clone();

                let base1 = base.as_deref();

                // original domain to match local pages.
                let original_page = {
                    self.set_url_parsed_direct_empty();
                    self.get_url_parsed_ref().as_ref().cloned()
                };

                // Weighted upgrade score: avoids Chrome on a single weak signal.
                // Strong signals (framework markers, hydration IDs) set the score
                // above the threshold immediately. Weak signals (script src) accumulate.
                const SMART_UPGRADE_THRESHOLD: u8 = 10;
                let upgrade_score = std::sync::atomic::AtomicU8::new(0);

                let mut static_app = false;
                let mut script_src_count: u8 = 0;
                let xml_file = self.get_url().ends_with(".xml");

                let mut element_content_handlers =
                    metadata_handlers(&mut meta_title, &mut meta_description, &mut meta_og_image);

                element_content_handlers.push(element_precompiled!(
                    compiled_base_element_selector(),
                    |el| {
                        if let Some(href) = el.get_attribute("href") {
                            if let Ok(parsed_base) = Url::parse(&href) {
                                let _ = base_input_url.set(parsed_base);
                            }
                        }

                        Ok(())
                    }
                ));

                element_content_handlers.push(element!("script", |el| {
                    if static_app
                        || upgrade_score.load(Ordering::Relaxed) >= SMART_UPGRADE_THRESHOLD
                    {
                        return Ok(());
                    }

                    let id = el.get_attribute("id");

                    if id.as_deref() == *NUXT_DATA {
                        static_app = true;
                        upgrade_score.store(SMART_UPGRADE_THRESHOLD, Ordering::Relaxed);
                        return Ok(());
                    }

                    if el.get_attribute("data-target").as_deref() == *REACT_SSR {
                        upgrade_score.store(SMART_UPGRADE_THRESHOLD, Ordering::Relaxed);
                        return Ok(());
                    }

                    let Some(src) = el.get_attribute("src") else {
                        return Ok(());
                    };

                    // Skip known ad/tracker scripts — they don't indicate an SPA.
                    if !is_tracker_script(&src) {
                        script_src_count = script_src_count.saturating_add(1);
                        if script_src_count >= 4 {
                            let _ = upgrade_score.fetch_update(
                                Ordering::Relaxed,
                                Ordering::Relaxed,
                                |v| Some(v.saturating_add(SMART_UPGRADE_THRESHOLD)),
                            );
                        }
                    }

                    if !src.starts_with('/') {
                        return Ok(());
                    }

                    let is_next = src.starts_with("/_next/static/chunks/pages/")
                        || src.starts_with("/webpack-runtime-");
                    let is_gatsby = id.as_deref() == *GATSBY;

                    let is_nuxt_asset = src.starts_with("/_nuxt/");

                    if is_next || is_gatsby || is_nuxt_asset {
                        static_app = true;
                    }

                    if is_nuxt_asset {
                        upgrade_score.store(SMART_UPGRADE_THRESHOLD, Ordering::Relaxed);
                        return Ok(());
                    }

                    if let Some(base) = base1.as_ref() {
                        let abs = convert_abs_path(base, &src);

                        if abs.path_segments().is_some_and(|mut segs| {
                            segs.any(|p| {
                                chromiumoxide::handler::network::ALLOWED_MATCHER.is_match(p)
                            })
                        }) {
                            upgrade_score.store(SMART_UPGRADE_THRESHOLD, Ordering::Relaxed);
                        }
                    }

                    Ok(())
                }));

                element_content_handlers.push(element_precompiled!(
                    if xml_file {
                        compiled_xml_selector()
                    } else {
                        compiled_selector()
                    },
                    |el| {
                        if let Some(href) = el.get_attribute("href") {
                            let base = if relative_directory_url(&href) || base.is_none() {
                                original_page.as_ref()
                            } else {
                                base.as_deref()
                            };

                            let base = if base_input_url.initialized() {
                                base_input_url.get()
                            } else {
                                base
                            };

                            push_link(
                                &base,
                                &href,
                                &mut inner_map,
                                &selectors.0,
                                parent_host,
                                parent_host_scheme,
                                base_input_domain,
                                sub_matcher,
                                &external_domains_caseless,
                                &mut links_pages,
                            );
                        }

                        Ok(())
                    }
                ));

                element_content_handlers.push(text!("noscript", |el| {
                    if upgrade_score.load(Ordering::Relaxed) < SMART_UPGRADE_THRESHOLD {
                        if NO_SCRIPT_JS_REQUIRED.find(el.as_str()).is_some() {
                            upgrade_score.store(SMART_UPGRADE_THRESHOLD, Ordering::Relaxed);
                        }
                    }
                    Ok(())
                }));

                element_content_handlers.push(text!("script", |el| {
                    let s = el.as_str();
                    if !s.is_empty()
                        && upgrade_score.load(Ordering::Relaxed) < SMART_UPGRADE_THRESHOLD
                    {
                        if DOM_SCRIPT_WATCH_METHODS.find(s).is_some() {
                            // Inline DOM mutation is a medium signal (7 points).
                            // Combined with script srcs it crosses the threshold.
                            let _ = upgrade_score.fetch_update(
                                Ordering::Relaxed,
                                Ordering::Relaxed,
                                |v| Some(v.saturating_add(7)),
                            );
                        }
                    }
                    Ok(())
                }));

                element_content_handlers.push(element!("body", |el| {
                    if upgrade_score.load(Ordering::Relaxed) < SMART_UPGRADE_THRESHOLD {
                        let mut matched = false;

                        if let Some(id) = el.get_attribute("id") {
                            if HYDRATION_IDS.contains(&id) {
                                upgrade_score.store(SMART_UPGRADE_THRESHOLD, Ordering::Relaxed);
                                matched = true;
                            }
                        }

                        if !matched {
                            for attr in DOM_WATCH_ATTRIBUTE_PATTERNS.iter() {
                                if el.has_attribute(attr) {
                                    upgrade_score.store(SMART_UPGRADE_THRESHOLD, Ordering::Relaxed);
                                    break;
                                }
                            }
                        }
                    }
                    Ok(())
                }));

                let rewriter_settings = lol_html::Settings {
                    element_content_handlers,
                    adjust_charset_on_meta_tag: true,
                    ..lol_html::send::Settings::new_for_handler_types()
                };

                let mut rewriter =
                    lol_html::send::HtmlRewriter::new(rewriter_settings.into(), |_c: &[u8]| {});

                let html_bytes = html_resource.as_bytes();
                let mut wrote_error = false;
                let should_yield = html_bytes.len() > REWRITER_YIELD_THRESHOLD;

                for (i, chunk) in html_bytes.chunks(*STREAMING_CHUNK_SIZE).enumerate() {
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

                // Anti-bot detection is a strong signal (immediate upgrade).
                let mut score = upgrade_score.load(Ordering::Relaxed);
                if score < SMART_UPGRADE_THRESHOLD {
                    if crate::utils::detect_anti_bot_from_body(html_resource.as_bytes()).is_some() {
                        score = SMART_UPGRADE_THRESHOLD;
                    }
                }

                if score >= SMART_UPGRADE_THRESHOLD {
                    if let Some(browser_controller) = browser
                        .get_or_init(|| {
                            crate::website::Website::setup_browser_base(&configuration, &base, jar)
                        })
                        .await
                    {
                        if let Ok(new_page) = crate::features::chrome::attempt_navigation(
                            "about:blank",
                            &browser_controller.browser.0,
                            &configuration.request_timeout,
                            &browser_controller.browser.2,
                            &configuration.viewport,
                        )
                        .await
                        {
                            let (intercept_handle, _) = tokio::join!(
                                crate::features::chrome::setup_chrome_interception_base(
                                    &new_page,
                                    configuration.chrome_intercept.enabled,
                                    &configuration.auth_challenge_response,
                                    configuration.chrome_intercept.block_visuals,
                                    &parent_host,
                                ),
                                crate::features::chrome::setup_chrome_events(
                                    &new_page,
                                    &configuration,
                                ),
                            );

                            if let Some(cookie_jar) = jar {
                                if let Some(u) = &original_page {
                                    if !configuration.cookie_str.is_empty() {
                                        let _ =
                                            crate::features::chrome::seed_jar_from_cookie_header(
                                                cookie_jar,
                                                &configuration.cookie_str,
                                                &u,
                                            );
                                    }

                                    if let Ok(cps) = crate::features::chrome::cookie_params_from_jar(
                                        cookie_jar, &u,
                                    ) {
                                        let _ = crate::features::chrome::set_page_cookies(
                                            &new_page, cps,
                                        )
                                        .await;
                                    }
                                }
                            }

                            let page_resource = crate::utils::fetch_page_html_chrome_base(
                                &html_resource,
                                &new_page,
                                true,
                                true,
                                &configuration.wait_for,
                                &configuration.screenshot,
                                false,
                                &configuration.openai_config,
                                Some(&self.url),
                                &configuration.execution_scripts,
                                &configuration.automation_scripts,
                                &configuration.viewport,
                                &configuration.request_timeout,
                                &configuration.track_events,
                                configuration.referer.clone(),
                                configuration.max_page_bytes,
                                configuration.get_cache_options(),
                                &configuration.cache_policy,
                                {
                                    #[cfg(feature = "headers")]
                                    {
                                        &self.headers
                                    }
                                    #[cfg(not(feature = "headers"))]
                                    {
                                        &None
                                    }
                                },
                                &Some(&configuration.chrome_intercept),
                                jar,
                                &configuration.remote_multimodal,
                                configuration.cache_namespace_str(),
                            )
                            .await;

                            if let Some(h) = intercept_handle {
                                let abort_handle = h.abort_handle();
                                if let Err(elasped) =
                                    tokio::time::timeout(tokio::time::Duration::from_secs(15), h)
                                        .await
                                {
                                    log::warn!("Handler timeout exceeded {elasped}");
                                    abort_handle.abort();
                                }
                            }

                            if let Ok(resource) = page_resource {
                                let base = if base_input_url.initialized() {
                                    base_input_url.get().cloned().map(Box::new)
                                } else {
                                    base1.as_deref().cloned().map(Box::new)
                                };

                                let page_resource = match &resource.content {
                                    Some(h) => auto_encode_bytes(&h),
                                    _ => Default::default(),
                                };

                                let extended_map = self
                                    .links_stream_base::<A>(selectors, &page_resource, &base)
                                    .await;

                                bytes_transferred = resource.bytes_transferred;

                                let new_page = build(&self.url, resource);

                                page_assign(self, new_page);

                                map.extend(extended_map);
                            };
                        }
                    }
                }
            }

            map.extend(inner_map);
            drop(html_resource);
            self.html = html_bytes_taken;
        }

        if let Some(lp) = links_pages {
            let page_links = self.page_links.get_or_insert_with(Default::default);
            page_links.extend(lp.into_iter().map(Into::into));
            page_links.extend(map.iter().map(|item| item.clone().into()));
        }

        let valid_meta =
            meta_title.is_some() || meta_description.is_some() || meta_og_image.is_some();

        if valid_meta {
            let mut metadata_inner = Metadata::default();
            metadata_inner.title = meta_title;
            metadata_inner.description = meta_description;
            metadata_inner.image = meta_og_image;

            if metadata_inner.exist() {
                metadata.replace(Box::new(metadata_inner));
            }

            if metadata.is_some() {
                self.metadata = metadata;
            }
        }

        update_link_capacity_hint(map.len());

        (map, bytes_transferred)
    }

    /// Find all the links as a stream using string resource validation.
    #[cfg(all(
        not(feature = "decentralized"),
        feature = "full_resources",
        feature = "smart"
    ))]
    #[inline(always)]
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub async fn links_stream_smart<
        A: PartialEq
            + Eq
            + Sync
            + Send
            + Clone
            + Default
            + ToString
            + std::hash::Hash
            + From<String>
            + Into<CaseInsensitiveString>
            + for<'a> From<&'a str>,
    >(
        &mut self,
        selectors: &RelativeSelectors,
        configuration: &crate::configuration::Configuration,
        base: &Option<Box<Url>>,
        browser: &crate::features::chrome::OnceBrowser,
        jar: Option<&std::sync::Arc<crate::client::cookie::Jar>>,
    ) -> (HashSet<A>, Option<f64>) {
        use auto_encoder::auto_encode_bytes;
        use lol_html::{element, text};
        use std::sync::atomic::Ordering;

        let mut bytes_transferred: Option<f64> = None;
        let mut map: HashSet<A> = HashSet::with_capacity(link_set_capacity());
        let mut inner_map: HashSet<A> = HashSet::new();
        let mut links_pages: Option<HashSet<A>> = if self.page_links.is_some() {
            Some(HashSet::new())
        } else {
            None
        };

        let mut metadata: Option<Box<Metadata>> = None;
        let mut meta_title: Option<_> = None;
        let mut meta_description: Option<_> = None;
        let mut meta_og_image: Option<_> = None;

        if !self.is_empty() {
            let html_bytes_taken = self.html.take();
            let html_resource = match html_bytes_taken.as_deref() {
                Some(b) => match std::str::from_utf8(b) {
                    Ok(s) => std::borrow::Cow::Borrowed(s),
                    Err(_) => std::borrow::Cow::Owned(auto_encode_bytes(b)),
                },
                None => std::borrow::Cow::Borrowed(""),
            };

            if html_resource.starts_with("<?xml") {
                self.links_stream_xml_links_stream_base(selectors, &html_resource, &mut map, &base)
                    .await;
            } else {
                let base_input_url = tokio::sync::OnceCell::new();

                let base_input_domain = &selectors.2;
                let parent_frags = &selectors.1; // todo: allow mix match tpt
                let parent_host = &parent_frags[0];
                let parent_host_scheme = &parent_frags[1];
                let sub_matcher = &selectors.0;

                let external_domains_caseless = self.external_domains_caseless.clone();

                let base1 = base.as_deref();

                // original domain to match local pages.
                let original_page = {
                    self.set_url_parsed_direct_empty();
                    self.get_url_parsed_ref().as_ref().cloned()
                };

                const SMART_UPGRADE_THRESHOLD: u8 = 10;
                let upgrade_score = std::sync::atomic::AtomicU8::new(0);

                let mut static_app = false;
                let mut script_src_count: u8 = 0;

                let mut element_content_handlers = vec![
                    element_precompiled!(compiled_base_element_selector(), |el| {
                        if let Some(href) = el.get_attribute("href") {
                            if let Ok(parsed_base) = Url::parse(&href) {
                                let _ = base_input_url.set(parsed_base);
                            }
                        }

                        Ok(())
                    }),
                    element!("script", |el| {
                        if static_app
                            || upgrade_score.load(Ordering::Relaxed) >= SMART_UPGRADE_THRESHOLD
                        {
                            return Ok(());
                        }

                        let id = el.get_attribute("id");

                        if id.as_deref() == *NUXT_DATA {
                            static_app = true;
                            upgrade_score.store(SMART_UPGRADE_THRESHOLD, Ordering::Relaxed);
                            return Ok(());
                        }

                        if el.get_attribute("data-target").as_deref() == *REACT_SSR {
                            upgrade_score.store(SMART_UPGRADE_THRESHOLD, Ordering::Relaxed);
                            return Ok(());
                        }

                        let Some(src) = el.get_attribute("src") else {
                            return Ok(());
                        };

                        if !src.starts_with('/') {
                            return Ok(());
                        }

                        let is_next = src.starts_with("/_next/static/chunks/pages/")
                            || src.starts_with("/webpack-runtime-");
                        let is_gatsby = id.as_deref() == *GATSBY;

                        let is_nuxt_asset = src.starts_with("/_nuxt/");

                        if is_next || is_gatsby || is_nuxt_asset {
                            static_app = true;
                        }

                        if is_nuxt_asset {
                            upgrade_score.store(SMART_UPGRADE_THRESHOLD, Ordering::Relaxed);
                            return Ok(());
                        }

                        if let Some(base) = base1.as_ref() {
                            let abs = convert_abs_path(base, &src);

                            if abs.path_segments().is_some_and(|mut segs| {
                                segs.any(|p| {
                                    chromiumoxide::handler::network::ALLOWED_MATCHER.is_match(p)
                                })
                            }) {
                                upgrade_score.store(SMART_UPGRADE_THRESHOLD, Ordering::Relaxed);
                            }
                        }

                        Ok(())
                    }),
                    element!(
                        "a[href]:not([aria-hidden=\"true\"]),script[src],link[href]",
                        |el| {
                            let attribute = if el.tag_name() == "script" {
                                if let Some(src) = el.get_attribute("src") {
                                    if !is_tracker_script(&src) {
                                        script_src_count = script_src_count.saturating_add(1);
                                        if script_src_count >= 4 {
                                            let _ = upgrade_score.fetch_update(
                                                Ordering::Relaxed,
                                                Ordering::Relaxed,
                                                |v| Some(v.saturating_add(SMART_UPGRADE_THRESHOLD)),
                                            );
                                        }
                                    }
                                }
                                "src"
                            } else {
                                "href"
                            };
                            if let Some(href) = el.get_attribute(attribute) {
                                let base = if relative_directory_url(&href) || base.is_none() {
                                    original_page.as_ref()
                                } else {
                                    base.as_deref()
                                };

                                let base = if base_input_url.initialized() {
                                    base_input_url.get()
                                } else {
                                    base
                                };

                                push_link(
                                    &base,
                                    &href,
                                    &mut inner_map,
                                    &selectors.0,
                                    parent_host,
                                    parent_host_scheme,
                                    base_input_domain,
                                    sub_matcher,
                                    &external_domains_caseless,
                                    &mut links_pages,
                                );
                            }

                            Ok(())
                        }
                    ),
                    text!("noscript", |el| {
                        if upgrade_score.load(Ordering::Relaxed) < SMART_UPGRADE_THRESHOLD {
                            if NO_SCRIPT_JS_REQUIRED.find(el.as_str()).is_some() {
                                upgrade_score.store(SMART_UPGRADE_THRESHOLD, Ordering::Relaxed);
                            }
                        }
                        Ok(())
                    }),
                    text!("script", |el| {
                        let s = el.as_str();
                        if !s.is_empty()
                            && upgrade_score.load(Ordering::Relaxed) < SMART_UPGRADE_THRESHOLD
                        {
                            if DOM_SCRIPT_WATCH_METHODS.find(s).is_some() {
                                let _ = upgrade_score.fetch_update(
                                    Ordering::Relaxed,
                                    Ordering::Relaxed,
                                    |v| Some(v.saturating_add(7)),
                                );
                            }
                        }
                        Ok(())
                    }),
                    element!("body", |el| {
                        if upgrade_score.load(Ordering::Relaxed) < SMART_UPGRADE_THRESHOLD {
                            let mut matched = false;

                            if let Some(id) = el.get_attribute("id") {
                                if HYDRATION_IDS.contains(&id) {
                                    upgrade_score.store(SMART_UPGRADE_THRESHOLD, Ordering::Relaxed);
                                    matched = true;
                                }
                            }

                            if !matched {
                                for attr in DOM_WATCH_ATTRIBUTE_PATTERNS.iter() {
                                    if el.has_attribute(attr) {
                                        upgrade_score
                                            .store(SMART_UPGRADE_THRESHOLD, Ordering::Relaxed);
                                        break;
                                    }
                                }
                            }
                        }
                        Ok(())
                    }),
                ];

                element_content_handlers.extend(metadata_handlers(
                    &mut meta_title,
                    &mut meta_description,
                    &mut meta_og_image,
                ));

                let rewriter_settings = lol_html::Settings {
                    element_content_handlers,
                    adjust_charset_on_meta_tag: true,
                    ..lol_html::send::Settings::new_for_handler_types()
                };

                let mut rewriter =
                    lol_html::send::HtmlRewriter::new(rewriter_settings.into(), |_c: &[u8]| {});

                let html_bytes = html_resource.as_bytes();
                let mut wrote_error = false;
                let should_yield = html_bytes.len() > REWRITER_YIELD_THRESHOLD;

                for (i, chunk) in html_bytes.chunks(*STREAMING_CHUNK_SIZE).enumerate() {
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

                // Anti-bot detection is a strong signal (immediate upgrade).
                let mut score = upgrade_score.load(Ordering::Relaxed);
                if score < SMART_UPGRADE_THRESHOLD {
                    if crate::utils::detect_anti_bot_from_body(html_resource.as_bytes()).is_some() {
                        score = SMART_UPGRADE_THRESHOLD;
                    }
                }

                if score >= SMART_UPGRADE_THRESHOLD {
                    if let Some(browser_controller) = browser
                        .get_or_init(|| {
                            crate::website::Website::setup_browser_base(&configuration, &base, jar)
                        })
                        .await
                    {
                        if let Ok(new_page) = crate::features::chrome::attempt_navigation(
                            "about:blank",
                            &browser_controller.browser.0,
                            &configuration.request_timeout,
                            &browser_controller.browser.2,
                            &configuration.viewport,
                        )
                        .await
                        {
                            let (intercept_handle, _) = tokio::join!(
                                crate::features::chrome::setup_chrome_interception_base(
                                    &new_page,
                                    configuration.chrome_intercept.enabled,
                                    &configuration.auth_challenge_response,
                                    configuration.chrome_intercept.block_visuals,
                                    &parent_host,
                                ),
                                crate::features::chrome::setup_chrome_events(
                                    &new_page,
                                    &configuration,
                                )
                            );

                            if let Some(cookie_jar) = jar {
                                if let Some(u) = &original_page {
                                    if !configuration.cookie_str.is_empty() {
                                        let _ =
                                            crate::features::chrome::seed_jar_from_cookie_header(
                                                cookie_jar,
                                                &configuration.cookie_str,
                                                &u,
                                            );
                                    }

                                    if let Ok(cps) = crate::features::chrome::cookie_params_from_jar(
                                        cookie_jar, &u,
                                    ) {
                                        let _ = crate::features::chrome::set_page_cookies(
                                            &new_page, cps,
                                        )
                                        .await;
                                    }
                                }
                            }

                            let page_resource = crate::utils::fetch_page_html_chrome_base(
                                &html_resource,
                                &new_page,
                                true,
                                true,
                                &configuration.wait_for,
                                &configuration.screenshot,
                                false,
                                &configuration.openai_config,
                                Some(&self.url),
                                &configuration.execution_scripts,
                                &configuration.automation_scripts,
                                &configuration.viewport,
                                &configuration.request_timeout,
                                &configuration.track_events,
                                configuration.referer.clone(),
                                configuration.max_page_bytes,
                                configuration.get_cache_options(),
                                &configuration.cache_policy,
                                {
                                    #[cfg(feature = "headers")]
                                    {
                                        &self.headers
                                    }
                                    #[cfg(not(feature = "headers"))]
                                    {
                                        &None
                                    }
                                },
                                &Some(&configuration.chrome_intercept),
                                jar,
                                &configuration.remote_multimodal,
                                configuration.cache_namespace_str(),
                            )
                            .await;

                            if let Some(h) = intercept_handle {
                                let abort_handle = h.abort_handle();
                                if let Err(elasped) =
                                    tokio::time::timeout(tokio::time::Duration::from_secs(15), h)
                                        .await
                                {
                                    log::warn!("Handler timeout exceeded {elasped}");
                                    abort_handle.abort();
                                }
                            }

                            if let Ok(v) = page_resource {
                                let resource = match &v.content {
                                    Some(h) => auto_encode_bytes(&h),
                                    _ => Default::default(),
                                };

                                let extended_map = self
                                    .links_stream_base::<A>(
                                        selectors,
                                        &resource,
                                        &base.as_deref().cloned().map(Box::new),
                                    )
                                    .await;

                                bytes_transferred = v.bytes_transferred;

                                let new_page = build(&self.url, v);

                                page_assign(self, new_page);

                                map.extend(extended_map)
                            }
                        }
                    }
                }
            }

            map.extend(inner_map);
            drop(html_resource);
            self.html = html_bytes_taken;
        }

        if let Some(lp) = links_pages {
            let page_links = self.page_links.get_or_insert_with(Default::default);
            page_links.extend(lp.into_iter().map(Into::into));
            page_links.extend(map.iter().map(|item| item.clone().into()));
        }

        let valid_meta =
            meta_title.is_some() || meta_description.is_some() || meta_og_image.is_some();

        if valid_meta {
            let mut metadata_inner = Metadata::default();
            metadata_inner.title = meta_title;
            metadata_inner.description = meta_description;
            metadata_inner.image = meta_og_image;

            if metadata_inner.exist() {
                metadata.replace(Box::new(metadata_inner));
            }

            if metadata.is_some() {
                self.metadata = metadata;
            }
        }

        update_link_capacity_hint(map.len());

        (map, bytes_transferred)
    }

    /// Find the links as a stream using string resource validation
    #[inline(always)]
    #[cfg(not(feature = "decentralized"))]
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all,))]
    pub async fn links_stream_full_resource<
        A: PartialEq
            + Eq
            + Sync
            + Send
            + Clone
            + Default
            + ToString
            + std::hash::Hash
            + From<String>
            + Into<CaseInsensitiveString>
            + for<'a> From<&'a str>,
    >(
        &mut self,
        selectors: &RelativeSelectors,
        base: &Option<Box<Url>>,
    ) -> HashSet<A> {
        let mut map: HashSet<A> = HashSet::with_capacity(link_set_capacity());
        let mut links_pages: Option<HashSet<A>> = if self.page_links.is_some() {
            Some(HashSet::new())
        } else {
            None
        };

        let mut metadata: Option<Box<Metadata>> = None;
        let mut meta_title: Option<_> = None;
        let mut meta_description: Option<_> = None;
        let mut meta_og_image: Option<_> = None;

        if !self.is_empty() {
            let html_bytes_taken = self.html.take();
            let html = match html_bytes_taken.as_deref() {
                Some(b) => match std::str::from_utf8(b) {
                    Ok(s) => std::borrow::Cow::Borrowed(s),
                    Err(_) => std::borrow::Cow::Owned(auto_encoder::auto_encode_bytes(b)),
                },
                None => std::borrow::Cow::Borrowed(""),
            };

            if html.starts_with("<?xml") {
                self.links_stream_xml_links_stream_base(selectors, &html, &mut map, base)
                    .await;
            } else {
                // let base_domain = &selectors.0;
                let parent_host = &selectors.1[0];
                // the host schemes
                let parent_host_scheme = &selectors.1[1];
                let base_input_domain = &selectors.2; // the domain after redirects
                let sub_matcher = &selectors.0;
                let base_input_url = tokio::sync::OnceCell::new();

                let base = base.as_deref();

                // original domain to match local pages.
                let original_page = {
                    self.set_url_parsed_direct_empty();
                    self.get_url_parsed_ref().as_ref().cloned()
                };

                let external_domains_caseless = self.external_domains_caseless.clone();

                let base_links_settings = lol_html::element!(
                    "a[href]:not([aria-hidden=\"true\"]),script[src],link[href]",
                    |el| {
                        let attribute = if el.tag_name() == "script" {
                            "src"
                        } else {
                            "href"
                        };
                        if let Some(href) = el.get_attribute(attribute) {
                            let base = if relative_directory_url(&href) || base.is_none() {
                                original_page.as_ref()
                            } else {
                                base
                            };
                            let base = if base_input_url.initialized() {
                                base_input_url.get()
                            } else {
                                base
                            };

                            push_link(
                                &base,
                                &href,
                                &mut map,
                                &selectors.0,
                                parent_host,
                                parent_host_scheme,
                                base_input_domain,
                                sub_matcher,
                                &external_domains_caseless,
                                &mut links_pages,
                            );
                        }
                        Ok(())
                    }
                );

                let mut element_content_handlers =
                    metadata_handlers(&mut meta_title, &mut meta_description, &mut meta_og_image);

                element_content_handlers.push(element_precompiled!(
                    compiled_base_element_selector(),
                    |el| {
                        if let Some(href) = el.get_attribute("href") {
                            if let Ok(parsed_base) = Url::parse(&href) {
                                let _ = base_input_url.set(parsed_base);
                            }
                        }

                        Ok(())
                    }
                ));

                element_content_handlers.push(base_links_settings);

                let settings = lol_html::send::Settings {
                    element_content_handlers,
                    adjust_charset_on_meta_tag: true,
                    ..lol_html::send::Settings::new_for_handler_types()
                };

                let mut rewriter = lol_html::send::HtmlRewriter::new(settings, |_c: &[u8]| {});

                let html_bytes = html.as_bytes();
                let mut wrote_error = false;
                let should_yield = html_bytes.len() > REWRITER_YIELD_THRESHOLD;

                for (i, chunk) in html_bytes.chunks(*STREAMING_CHUNK_SIZE).enumerate() {
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
            }

            drop(html);
            self.html = html_bytes_taken;
        }

        let valid_meta =
            meta_title.is_some() || meta_description.is_some() || meta_og_image.is_some();

        if valid_meta {
            let mut metadata_inner = Metadata::default();
            metadata_inner.title = meta_title;
            metadata_inner.description = meta_description;
            metadata_inner.image = meta_og_image;

            if metadata_inner.exist() {
                metadata.replace(Box::new(metadata_inner));
            }

            if metadata.is_some() {
                self.metadata = metadata;
            }
        }

        update_link_capacity_hint(map.len());

        map
    }

    /// Find the links as a stream using string resource validation
    #[inline(always)]
    #[cfg(all(not(feature = "decentralized"), feature = "full_resources"))]
    pub async fn links_stream<
        A: PartialEq
            + Eq
            + Sync
            + Send
            + Clone
            + Default
            + ToString
            + std::hash::Hash
            + From<String>
            + Into<CaseInsensitiveString>
            + for<'a> From<&'a str>,
    >(
        &mut self,
        selectors: &RelativeSelectors,
        base: &Option<Box<Url>>,
    ) -> HashSet<A> {
        if auto_encoder::is_binary_file(self.get_html_bytes_u8()) {
            Default::default()
        } else {
            self.links_stream_full_resource(selectors, base).await
        }
    }

    #[inline(always)]
    #[cfg(feature = "decentralized")]
    /// Find the links as a stream using string resource validation
    pub async fn links_stream<
        A: PartialEq
            + Eq
            + Sync
            + Send
            + Clone
            + Default
            + ToString
            + std::hash::Hash
            + From<String>
            + Into<CaseInsensitiveString>
            + for<'a> From<&'a str>,
    >(
        &mut self,
        _: &RelativeSelectors,
    ) -> HashSet<A> {
        Default::default()
    }

    /// Find all href links and return them using CSS selectors.
    #[cfg(not(feature = "decentralized"))]
    #[inline(always)]
    pub async fn links(
        &mut self,
        selectors: &RelativeSelectors,
        base: &Option<Box<Url>>,
    ) -> HashSet<CaseInsensitiveString> {
        match self.html.is_some() {
            false => Default::default(),
            true => {
                self.links_stream::<CaseInsensitiveString>(selectors, base)
                    .await
            }
        }
    }

    /// Find all href links and return them using CSS selectors gathering all resources.
    #[inline(always)]
    #[cfg(not(feature = "decentralized"))]
    pub async fn links_full(
        &mut self,
        selectors: &RelativeSelectors,
        base: &Option<Box<Url>>,
    ) -> HashSet<CaseInsensitiveString> {
        match self.html.is_some() {
            false => Default::default(),
            true => {
                if auto_encoder::is_binary_file(self.get_html_bytes_u8()) {
                    return Default::default();
                }
                self.links_stream_full_resource::<CaseInsensitiveString>(selectors, base)
                    .await
            }
        }
    }

    /// Find all href links and return them using CSS selectors.
    #[cfg(all(not(feature = "decentralized"), feature = "smart"))]
    #[inline(always)]
    pub(crate) async fn smart_links(
        &mut self,
        selectors: &RelativeSelectors,
        configuration: &crate::configuration::Configuration,
        base: &Option<Box<Url>>,
        page: &crate::features::chrome::OnceBrowser,
        jar: Option<&std::sync::Arc<crate::client::cookie::Jar>>,
    ) -> (HashSet<CaseInsensitiveString>, Option<f64>) {
        match self.html.is_some() {
            false => Default::default(),
            true => {
                if auto_encoder::is_binary_file(self.get_html_bytes_u8()) {
                    return Default::default();
                }
                self.links_stream_smart::<CaseInsensitiveString>(
                    &selectors,
                    configuration,
                    base,
                    page,
                    jar,
                )
                .await
            }
        }
    }

    /// Find all href links and return them using CSS selectors.
    #[cfg(feature = "decentralized")]
    #[inline(always)]
    pub async fn links(
        &self,
        _: &RelativeSelectors,
        _: &Option<Box<Url>>,
    ) -> HashSet<CaseInsensitiveString> {
        self.links.to_owned()
    }

    /// Find all href links and return them using CSS selectors gathering all resources.
    #[cfg(feature = "decentralized")]
    #[inline(always)]
    pub async fn links_full(
        &self,
        _: &RelativeSelectors,
        _: &Option<Box<Url>>,
    ) -> HashSet<CaseInsensitiveString> {
        self.links.to_owned()
    }
}

/// Get the content with proper encoding. Pass in a proper encoding label like SHIFT_JIS.
pub fn encode_bytes(html: &[u8], label: &str) -> String {
    auto_encoder::encode_bytes(html, label)
}

/// Get the content with proper encoding. Pass in a proper encoding label like SHIFT_JIS.
#[cfg(feature = "encoding")]
pub fn get_html_encoded(html: &Option<bytes::Bytes>, label: &str) -> String {
    match html.as_ref() {
        Some(html) => encode_bytes(html, label),
        _ => Default::default(),
    }
}

#[cfg(not(feature = "encoding"))]
/// Get the content with proper encoding. Pass in a proper encoding label like SHIFT_JIS.
pub fn get_html_encoded(html: &Option<bytes::Bytes>, _label: &str) -> String {
    match html {
        Some(b) => String::from_utf8_lossy(b).into_owned(),
        _ => Default::default(),
    }
}

#[cfg(all(test, not(feature = "decentralized"), feature = "smart"))]
mod smart_tests {
    use super::is_tracker_script;

    #[test]
    fn tracker_absolute_urls() {
        // Known tracker scripts (absolute) should be detected.
        assert!(is_tracker_script(
            "https://www.googletagmanager.com/gtm.js?id=GTM-ABC"
        ));
        assert!(is_tracker_script(
            "https://www.google-analytics.com/analytics.js"
        ));
        assert!(is_tracker_script(
            "https://static.hotjar.com/c/hotjar-123.js"
        ));
        assert!(is_tracker_script(
            "https://connect.facebook.net/en_US/fbevents.js"
        ));
    }

    #[test]
    fn non_tracker_absolute_urls() {
        // Application scripts should not be flagged.
        assert!(!is_tracker_script("https://cdn.example.com/app.js"));
        assert!(!is_tracker_script(
            "https://unpkg.com/react@18/umd/react.production.min.js"
        ));
    }

    #[test]
    fn tracker_relative_paths() {
        // Relative paths matching adblock patterns.
        assert!(is_tracker_script("/js/analytics.js"));
        assert!(is_tracker_script("/scripts/gtm.js?id=GTM-XYZ"));
    }

    #[test]
    fn non_tracker_relative_paths() {
        // Normal app scripts should pass through.
        assert!(!is_tracker_script("/assets/app.bundle.js"));
        assert!(!is_tracker_script("/_next/static/chunks/pages/index.js"));
        assert!(!is_tracker_script("/main.js"));
    }
}

#[cfg(test)]
/// The test user agent.
pub const TEST_AGENT_NAME: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));

#[cfg(all(
    feature = "headers",
    not(feature = "decentralized"),
    not(feature = "cache_request"),
))]
#[tokio::test]
async fn test_headers() {
    use crate::utils::PageResponse;
    use reqwest::header::HeaderName;
    use reqwest::header::HeaderValue;

    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        HeaderName::from_static("server"),
        HeaderValue::from_static("GitHub.com"),
    );
    headers.insert(
        HeaderName::from_static("content-type"),
        HeaderValue::from_static("text/html; charset=utf-8"),
    );

    let page = build(
        "https://choosealicense.com/",
        PageResponse {
            content: Some(b"<html></html>".to_vec()),
            headers: Some(headers),
            status_code: StatusCode::OK,
            ..Default::default()
        },
    );

    let headers = page.headers.expect("There should be some headers");

    assert_eq!(
        headers
            .get(HeaderName::from_static("server"))
            .expect("There should be a server header value"),
        HeaderValue::from_static("GitHub.com")
    );

    assert_eq!(
        headers
            .get(HeaderName::from_static("content-type"))
            .expect("There should be a content-type value"),
        HeaderValue::from_static("text/html; charset=utf-8")
    );
}

#[tokio::test]
#[cfg(all(
    not(feature = "decentralized"),
    not(feature = "chrome"),
    not(feature = "cache_request")
))]
async fn parse_links() {
    use crate::utils::PageResponse;

    let link_result = "https://choosealicense.com/";
    let html = br#"<html><body><a href="/about/">About</a></body></html>"#;
    let mut page = build_with_parse(
        link_result,
        PageResponse {
            content: Some(html.to_vec()),
            status_code: StatusCode::OK,
            ..Default::default()
        },
    );
    let selector = get_page_selectors(link_result, false, false);

    let links = page.links(&selector, &None).await;

    let about_page = "https://choosealicense.com/about/".into();

    assert!(
        links.contains::<CaseInsensitiveString>(&about_page),
        "Could not find {}. Theses URLs was found {:?}",
        about_page,
        &links
    );
}

#[tokio::test]
#[cfg(all(
    not(feature = "decentralized"),
    not(feature = "chrome"),
    not(feature = "cache_request")
))]
async fn test_status_code() {
    use crate::utils::PageResponse;
    let page = build(
        "https://choosealicense.com/does-not-exist",
        PageResponse {
            status_code: StatusCode::NOT_FOUND,
            ..Default::default()
        },
    );

    assert_eq!(page.status_code.as_u16(), 404);
}

#[tokio::test]
#[cfg(all(feature = "time", not(feature = "decentralized")))]
async fn test_duration() {
    let client = Client::default();
    let link_result = "https://choosealicense.com/";
    let page: Page = Page::new_page(&link_result, &client).await;
    let duration_elasped = page.get_duration_elapsed().as_millis();

    assert!(
        duration_elasped < 6000,
        "Duration took longer than expected {}.",
        duration_elasped,
    );
}

// ============================================================================
// Metadata Tests
// ============================================================================

/// Test that Metadata::exist returns false when all fields are None.
#[test]
fn test_metadata_exist_empty() {
    let metadata = Metadata::default();
    assert!(!metadata.exist(), "Empty metadata should not exist");
}

/// Test that Metadata::exist returns true when title is set.
#[test]
fn test_metadata_exist_with_title() {
    let metadata = Metadata {
        title: Some(CompactString::from("Test Title")),
        ..Default::default()
    };
    assert!(metadata.exist(), "Metadata with title should exist");
}

/// Test that Metadata::exist returns true when description is set.
#[test]
fn test_metadata_exist_with_description() {
    let metadata = Metadata {
        description: Some(CompactString::from("Test Description")),
        ..Default::default()
    };
    assert!(metadata.exist(), "Metadata with description should exist");
}

/// Test that Metadata::exist returns true when image is set.
#[test]
fn test_metadata_exist_with_image() {
    let metadata = Metadata {
        image: Some(CompactString::from("https://example.com/image.png")),
        ..Default::default()
    };
    assert!(metadata.exist(), "Metadata with image should exist");
}

/// Test that Metadata::exist returns true when all fields are set.
#[test]
fn test_metadata_exist_all_fields() {
    let metadata = Metadata {
        title: Some(CompactString::from("Test Title")),
        description: Some(CompactString::from("Test Description")),
        image: Some(CompactString::from("https://example.com/image.png")),
        #[cfg(feature = "chrome")]
        automation: None,
    };
    assert!(metadata.exist(), "Metadata with all fields should exist");
}

/// Test metadata extraction via build function with PageResponse.
#[test]
#[cfg(not(feature = "decentralized"))]
fn test_metadata_via_build() {
    use crate::utils::PageResponse;

    let metadata = Metadata {
        title: Some(CompactString::from("Build Test Title")),
        description: Some(CompactString::from("Build Test Description")),
        image: Some(CompactString::from("https://example.com/build-image.png")),
        #[cfg(feature = "chrome")]
        automation: None,
    };

    let page_response = PageResponse {
        content: Some(b"<html></html>".to_vec()),
        status_code: StatusCode::OK,
        metadata: Some(Box::new(metadata)),
        ..Default::default()
    };

    let page = build("https://example.com", page_response);
    let page_metadata = page.get_metadata();

    assert!(page_metadata.is_some(), "Page should have metadata");

    let meta = page_metadata.as_ref().unwrap();
    assert_eq!(
        meta.title.as_deref(),
        Some("Build Test Title"),
        "Title should match"
    );
    assert_eq!(
        meta.description.as_deref(),
        Some("Build Test Description"),
        "Description should match"
    );
    assert_eq!(
        meta.image.as_deref(),
        Some("https://example.com/build-image.png"),
        "Image should match"
    );
}

/// Test metadata extraction via build_with_parse function.
#[test]
#[cfg(not(feature = "decentralized"))]
fn test_metadata_via_build_with_parse() {
    use crate::utils::PageResponse;

    let metadata = Metadata {
        title: Some(CompactString::from("Parse Test Title")),
        description: Some(CompactString::from("Parse Test Description")),
        image: Some(CompactString::from("https://example.com/parse-image.png")),
        #[cfg(feature = "chrome")]
        automation: None,
    };

    let page_response = PageResponse {
        content: Some(b"<html></html>".to_vec()),
        status_code: StatusCode::OK,
        metadata: Some(Box::new(metadata)),
        ..Default::default()
    };

    let page = build_with_parse("https://example.com/page", page_response);
    let page_metadata = page.get_metadata();

    assert!(
        page_metadata.is_some(),
        "Page should have metadata after build_with_parse"
    );

    let meta = page_metadata.as_ref().unwrap();
    assert_eq!(
        meta.title.as_deref(),
        Some("Parse Test Title"),
        "Title should match after build_with_parse"
    );
}

/// Test that Page without metadata returns None from get_metadata.
#[test]
#[cfg(not(feature = "decentralized"))]
fn test_page_without_metadata() {
    use crate::utils::PageResponse;

    let page_response = PageResponse {
        content: Some(b"<html></html>".to_vec()),
        status_code: StatusCode::OK,
        metadata: None,
        ..Default::default()
    };

    let page = build("https://example.com", page_response);
    let page_metadata = page.get_metadata();

    assert!(
        page_metadata.is_none(),
        "Page without metadata should return None"
    );
}

/// Test metadata extraction using new_page_streaming_from_bytes with crafted HTML.
#[tokio::test]
#[cfg(all(feature = "cmd", not(feature = "decentralized")))]
async fn test_metadata_from_streaming_bytes() {
    let html = br#"<!DOCTYPE html>
<html>
<head>
    <title>Streaming Test Title</title>
    <meta name="description" content="Streaming Test Description">
    <meta property="og:image" content="https://example.com/streaming-image.png">
</head>
<body>
    <a href="/page1">Link 1</a>
    <a href="/page2">Link 2</a>
</body>
</html>"#;

    let url = "https://example.com/test";
    let mut selectors = get_page_selectors(url, false, false);
    let external_domains: Arc<HashSet<CaseInsensitiveString>> = Default::default();
    let r_settings = PageLinkBuildSettings::default();
    let mut map: HashSet<CaseInsensitiveString> = HashSet::with_capacity(32);
    let prior_domain: Option<Box<Url>> = None;
    let mut domain_parsed: Option<Box<Url>> = None;
    let mut links_pages: Option<HashSet<CaseInsensitiveString>> = None;

    let page = Page::new_page_streaming_from_bytes::<CaseInsensitiveString>(
        url,
        html,
        &mut selectors,
        &external_domains,
        &r_settings,
        &mut map,
        None,
        &prior_domain,
        &mut domain_parsed,
        &mut links_pages,
    )
    .await;

    let page_metadata = page.get_metadata();

    assert!(
        page_metadata.is_some(),
        "Page from streaming bytes should have metadata"
    );

    let meta = page_metadata.as_ref().unwrap();

    assert_eq!(
        meta.title.as_deref(),
        Some("Streaming Test Title"),
        "Title should be extracted from streaming bytes"
    );
    assert_eq!(
        meta.description.as_deref(),
        Some("Streaming Test Description"),
        "Description should be extracted from streaming bytes"
    );
    assert_eq!(
        meta.image.as_deref(),
        Some("https://example.com/streaming-image.png"),
        "OG image should be extracted from streaming bytes"
    );
}

/// Test metadata extraction with partial metadata (only title).
#[tokio::test]
#[cfg(all(feature = "cmd", not(feature = "decentralized")))]
async fn test_metadata_partial_title_only() {
    let html = br#"<!DOCTYPE html>
<html>
<head>
    <title>Only Title Here</title>
</head>
<body></body>
</html>"#;

    let url = "https://example.com/test";
    let mut selectors = get_page_selectors(url, false, false);
    let external_domains: Arc<HashSet<CaseInsensitiveString>> = Default::default();
    let r_settings = PageLinkBuildSettings::default();
    let mut map: HashSet<CaseInsensitiveString> = HashSet::with_capacity(32);
    let prior_domain: Option<Box<Url>> = None;
    let mut domain_parsed: Option<Box<Url>> = None;
    let mut links_pages: Option<HashSet<CaseInsensitiveString>> = None;

    let page = Page::new_page_streaming_from_bytes::<CaseInsensitiveString>(
        url,
        html,
        &mut selectors,
        &external_domains,
        &r_settings,
        &mut map,
        None,
        &prior_domain,
        &mut domain_parsed,
        &mut links_pages,
    )
    .await;

    let page_metadata = page.get_metadata();

    assert!(
        page_metadata.is_some(),
        "Page with only title should have metadata"
    );

    let meta = page_metadata.as_ref().unwrap();

    assert_eq!(
        meta.title.as_deref(),
        Some("Only Title Here"),
        "Title should be extracted"
    );
    assert!(meta.description.is_none(), "Description should be None");
    assert!(meta.image.is_none(), "Image should be None");
}

/// Test metadata extraction with partial metadata (only description).
#[tokio::test]
#[cfg(all(feature = "cmd", not(feature = "decentralized")))]
async fn test_metadata_partial_description_only() {
    let html = br#"<!DOCTYPE html>
<html>
<head>
    <meta name="description" content="Only Description Here">
</head>
<body></body>
</html>"#;

    let url = "https://example.com/test";
    let mut selectors = get_page_selectors(url, false, false);
    let external_domains: Arc<HashSet<CaseInsensitiveString>> = Default::default();
    let r_settings = PageLinkBuildSettings::default();
    let mut map: HashSet<CaseInsensitiveString> = HashSet::with_capacity(32);
    let prior_domain: Option<Box<Url>> = None;
    let mut domain_parsed: Option<Box<Url>> = None;
    let mut links_pages: Option<HashSet<CaseInsensitiveString>> = None;

    let page = Page::new_page_streaming_from_bytes::<CaseInsensitiveString>(
        url,
        html,
        &mut selectors,
        &external_domains,
        &r_settings,
        &mut map,
        None,
        &prior_domain,
        &mut domain_parsed,
        &mut links_pages,
    )
    .await;

    let page_metadata = page.get_metadata();

    assert!(
        page_metadata.is_some(),
        "Page with only description should have metadata"
    );

    let meta = page_metadata.as_ref().unwrap();

    assert!(meta.title.is_none(), "Title should be None");
    assert_eq!(
        meta.description.as_deref(),
        Some("Only Description Here"),
        "Description should be extracted"
    );
    assert!(meta.image.is_none(), "Image should be None");
}

/// Test metadata extraction with partial metadata (only og:image).
#[tokio::test]
#[cfg(all(feature = "cmd", not(feature = "decentralized")))]
async fn test_metadata_partial_image_only() {
    let html = br#"<!DOCTYPE html>
<html>
<head>
    <meta property="og:image" content="https://example.com/only-image.png">
</head>
<body></body>
</html>"#;

    let url = "https://example.com/test";
    let mut selectors = get_page_selectors(url, false, false);
    let external_domains: Arc<HashSet<CaseInsensitiveString>> = Default::default();
    let r_settings = PageLinkBuildSettings::default();
    let mut map: HashSet<CaseInsensitiveString> = HashSet::with_capacity(32);
    let prior_domain: Option<Box<Url>> = None;
    let mut domain_parsed: Option<Box<Url>> = None;
    let mut links_pages: Option<HashSet<CaseInsensitiveString>> = None;

    let page = Page::new_page_streaming_from_bytes::<CaseInsensitiveString>(
        url,
        html,
        &mut selectors,
        &external_domains,
        &r_settings,
        &mut map,
        None,
        &prior_domain,
        &mut domain_parsed,
        &mut links_pages,
    )
    .await;

    let page_metadata = page.get_metadata();

    assert!(
        page_metadata.is_some(),
        "Page with only og:image should have metadata"
    );

    let meta = page_metadata.as_ref().unwrap();

    assert!(meta.title.is_none(), "Title should be None");
    assert!(meta.description.is_none(), "Description should be None");
    assert_eq!(
        meta.image.as_deref(),
        Some("https://example.com/only-image.png"),
        "OG image should be extracted"
    );
}

/// Test that page with no metadata tags returns None.
#[tokio::test]
#[cfg(all(feature = "cmd", not(feature = "decentralized")))]
async fn test_metadata_empty_html() {
    let html = br#"<!DOCTYPE html>
<html>
<head></head>
<body><p>No metadata here</p></body>
</html>"#;

    let url = "https://example.com/test";
    let mut selectors = get_page_selectors(url, false, false);
    let external_domains: Arc<HashSet<CaseInsensitiveString>> = Default::default();
    let r_settings = PageLinkBuildSettings::default();
    let mut map: HashSet<CaseInsensitiveString> = HashSet::with_capacity(32);
    let prior_domain: Option<Box<Url>> = None;
    let mut domain_parsed: Option<Box<Url>> = None;
    let mut links_pages: Option<HashSet<CaseInsensitiveString>> = None;

    let page = Page::new_page_streaming_from_bytes::<CaseInsensitiveString>(
        url,
        html,
        &mut selectors,
        &external_domains,
        &r_settings,
        &mut map,
        None,
        &prior_domain,
        &mut domain_parsed,
        &mut links_pages,
    )
    .await;

    let page_metadata = page.get_metadata();

    assert!(
        page_metadata.is_none(),
        "Page without any metadata tags should return None"
    );
}

/// Test metadata with special characters in content.
#[tokio::test]
#[cfg(all(feature = "cmd", not(feature = "decentralized")))]
async fn test_metadata_special_characters() {
    let html = br#"<!DOCTYPE html>
<html>
<head>
    <title>Title with &amp; special &lt;characters&gt;</title>
    <meta name="description" content="Description with &quot;quotes&quot; and 'apostrophes'">
    <meta property="og:image" content="https://example.com/image?param=value&amp;other=1">
</head>
<body></body>
</html>"#;

    let url = "https://example.com/test";
    let mut selectors = get_page_selectors(url, false, false);
    let external_domains: Arc<HashSet<CaseInsensitiveString>> = Default::default();
    let r_settings = PageLinkBuildSettings::default();
    let mut map: HashSet<CaseInsensitiveString> = HashSet::with_capacity(32);
    let prior_domain: Option<Box<Url>> = None;
    let mut domain_parsed: Option<Box<Url>> = None;
    let mut links_pages: Option<HashSet<CaseInsensitiveString>> = None;

    let page = Page::new_page_streaming_from_bytes::<CaseInsensitiveString>(
        url,
        html,
        &mut selectors,
        &external_domains,
        &r_settings,
        &mut map,
        None,
        &prior_domain,
        &mut domain_parsed,
        &mut links_pages,
    )
    .await;

    let page_metadata = page.get_metadata();

    assert!(
        page_metadata.is_some(),
        "Page with special characters should have metadata"
    );

    let meta = page_metadata.as_ref().unwrap();
    assert!(
        meta.title.is_some(),
        "Title with special chars should be extracted"
    );
    assert!(
        meta.description.is_some(),
        "Description with special chars should be extracted"
    );
    assert!(
        meta.image.is_some(),
        "Image URL with special chars should be extracted"
    );
}

/// Test metadata with unicode content.
#[tokio::test]
#[cfg(all(feature = "cmd", not(feature = "decentralized")))]
async fn test_metadata_unicode() {
    let html = r#"<!DOCTYPE html>
<html>
<head>
    <title>日本語タイトル - Japanese Title</title>
    <meta name="description" content="中文描述 - Chinese Description - Описание на русском">
    <meta property="og:image" content="https://example.com/画像.png">
</head>
<body></body>
</html>"#
        .as_bytes();

    let url = "https://example.com/test";
    let mut selectors = get_page_selectors(url, false, false);
    let external_domains: Arc<HashSet<CaseInsensitiveString>> = Default::default();
    let r_settings = PageLinkBuildSettings::default();
    let mut map: HashSet<CaseInsensitiveString> = HashSet::with_capacity(32);
    let prior_domain: Option<Box<Url>> = None;
    let mut domain_parsed: Option<Box<Url>> = None;
    let mut links_pages: Option<HashSet<CaseInsensitiveString>> = None;

    let page = Page::new_page_streaming_from_bytes::<CaseInsensitiveString>(
        url,
        html,
        &mut selectors,
        &external_domains,
        &r_settings,
        &mut map,
        None,
        &prior_domain,
        &mut domain_parsed,
        &mut links_pages,
    )
    .await;

    let page_metadata = page.get_metadata();

    assert!(
        page_metadata.is_some(),
        "Page with unicode content should have metadata"
    );

    let meta = page_metadata.as_ref().unwrap();
    assert!(
        meta.title
            .as_ref()
            .map(|t| t.contains("日本語"))
            .unwrap_or(false),
        "Title should contain Japanese characters"
    );
    assert!(
        meta.description
            .as_ref()
            .map(|d| d.contains("中文"))
            .unwrap_or(false),
        "Description should contain Chinese characters"
    );
}

/// Test metadata with chrome feature - AutomationResults structure.
#[test]
#[cfg(feature = "chrome")]
fn test_automation_results_structure() {
    let automation_result = AutomationResults {
        input: "Test prompt".to_string(),
        content_output: serde_json::json!({"result": "test"}),
        screenshot_output: Some("base64_screenshot_data".to_string()),
        error: None,
        usage: None,
        relevant: None,
        steps_executed: None,
        reasoning: None,
    };

    assert_eq!(automation_result.input, "Test prompt");
    assert!(automation_result.screenshot_output.is_some());
    assert!(automation_result.error.is_none());
}

/// Test metadata with automation results (chrome feature).
#[test]
#[cfg(feature = "chrome")]
fn test_metadata_with_automation() {
    let automation_results = vec![AutomationResults {
        input: "Click the button".to_string(),
        content_output: serde_json::json!({"clicked": true}),
        screenshot_output: None,
        error: None,
        usage: None,
        relevant: None,
        steps_executed: None,
        reasoning: None,
    }];

    let metadata = Metadata {
        title: Some(CompactString::from("Automation Test")),
        description: None,
        image: None,
        automation: Some(automation_results),
    };

    assert!(metadata.exist(), "Metadata with title should exist");
    assert!(
        metadata.automation.is_some(),
        "Automation results should be present"
    );
    assert_eq!(
        metadata.automation.as_ref().unwrap().len(),
        1,
        "Should have one automation result"
    );
}

/// Test set_metadata function preserves automation results (chrome feature).
#[test]
#[cfg(all(feature = "chrome", not(feature = "decentralized")))]
fn test_set_metadata_preserves_automation() {
    let automation_results = vec![AutomationResults {
        input: "Original automation".to_string(),
        content_output: serde_json::json!({"original": true}),
        screenshot_output: None,
        error: None,
        usage: None,
        relevant: None,
        steps_executed: None,
        reasoning: None,
    }];

    let existing_metadata = Metadata {
        title: Some(CompactString::from("Original Title")),
        description: None,
        image: None,
        automation: Some(automation_results),
    };

    let existing = Some(Box::new(existing_metadata));

    let mut new_metadata = Metadata {
        title: Some(CompactString::from("New Title")),
        description: Some(CompactString::from("New Description")),
        image: None,
        automation: None,
    };

    set_metadata(&existing, &mut new_metadata);

    assert!(
        new_metadata.automation.is_some(),
        "Automation should be preserved from existing metadata"
    );
}

/// Test metadata via Page::new with chrome feature.
#[tokio::test]
#[cfg(all(
    feature = "chrome",
    not(feature = "decentralized"),
    not(feature = "cache_request")
))]
async fn test_metadata_chrome_real_page() {
    use crate::utils::PageResponse;

    // Keep this deterministic: verify chrome-gated metadata plumbing without external network.
    let automation_results = vec![AutomationResults {
        input: "Extract CTA".to_string(),
        content_output: serde_json::json!({"cta": "Sign up"}),
        screenshot_output: Some("base64_screenshot_data".to_string()),
        error: None,
        usage: None,
        relevant: Some(true),
        steps_executed: Some(1),
        reasoning: Some("CTA extracted from main hero section".to_string()),
    }];

    let metadata = Metadata {
        title: Some(CompactString::from("Chrome Metadata Test")),
        description: Some(CompactString::from("Description available")),
        image: Some(CompactString::from("https://example.com/image.png")),
        automation: Some(automation_results),
    };

    let page = build(
        "https://example.com",
        PageResponse {
            content: Some(b"<html></html>".to_vec()),
            status_code: StatusCode::OK,
            metadata: Some(Box::new(metadata)),
            ..Default::default()
        },
    );

    let meta = page
        .get_metadata()
        .as_ref()
        .expect("metadata should be present for chrome feature test");
    assert!(meta.title.as_deref() == Some("Chrome Metadata Test"));
    assert!(
        meta.automation.is_some(),
        "automation metadata should be present"
    );
    assert_eq!(meta.automation.as_ref().expect("automation data").len(), 1);
}

// ============================================================================
// Feature-specific Tests
// ============================================================================

/// Test encoding feature - get_html_encoded function.
#[test]
#[cfg(feature = "encoding")]
fn test_encoding_get_html_encoded() {
    // Test with UTF-8 content
    let html_bytes = "こんにちは世界".as_bytes().to_vec();
    let encoded = encode_bytes(&html_bytes, "UTF-8");
    assert!(
        encoded.contains("こんにちは"),
        "UTF-8 encoding should preserve Japanese characters"
    );
}

/// Test encoding feature - get_html_encoded with Page.
#[test]
#[cfg(all(feature = "encoding", not(feature = "decentralized")))]
fn test_encoding_page_get_html_encoded() {
    use crate::utils::PageResponse;

    let html_content = "Hello World - テスト";
    let page_response = PageResponse {
        content: Some(html_content.as_bytes().to_vec()),
        status_code: StatusCode::OK,
        ..Default::default()
    };

    let page = build("https://example.com", page_response);
    let encoded = page.get_html_encoded("UTF-8");

    assert!(
        encoded.contains("Hello World"),
        "Encoded content should contain ASCII text"
    );
    assert!(
        encoded.contains("テスト"),
        "Encoded content should contain Japanese text"
    );
}

/// Test remote_addr feature - Page struct has remote_addr field.
#[test]
#[cfg(all(feature = "remote_addr", not(feature = "decentralized")))]
fn test_remote_addr_field() {
    use crate::utils::PageResponse;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    let page_response = PageResponse {
        content: Some(b"<html></html>".to_vec()),
        status_code: StatusCode::OK,
        remote_addr: Some(SocketAddr::new(
            IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            8080,
        )),
        ..Default::default()
    };

    let page = build("https://example.com", page_response);

    assert!(
        page.remote_addr.is_some(),
        "Page should have remote_addr when feature is enabled"
    );

    let addr = page.remote_addr.unwrap();
    assert_eq!(addr.ip(), IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)));
    assert_eq!(addr.port(), 8080);
}

/// Test page_error_status_details feature - error_status is Arc<reqwest::Error>.
#[test]
#[cfg(all(feature = "page_error_status_details", not(feature = "decentralized")))]
fn test_page_error_status_details() {
    use crate::utils::PageResponse;

    // When page_error_status_details is enabled, error_status should be Option<Arc<reqwest::Error>>
    let page_response = PageResponse {
        content: None,
        status_code: StatusCode::INTERNAL_SERVER_ERROR,
        ..Default::default()
    };

    let page = build("https://example.com", page_response);

    // The error_status type should be Option<Arc<reqwest::Error>> with this feature
    // We just verify it compiles and can be accessed
    let _error: &Option<std::sync::Arc<reqwest::Error>> = &page.error_status;
}

/// Test page_error_status_details feature disabled - error_status is String.
#[test]
#[cfg(all(
    not(feature = "page_error_status_details"),
    not(feature = "decentralized")
))]
fn test_page_error_status_string() {
    use crate::utils::PageResponse;

    let page_response = PageResponse {
        content: None,
        status_code: StatusCode::INTERNAL_SERVER_ERROR,
        ..Default::default()
    };

    let page = build("https://example.com", page_response);

    // The error_status type should be Option<String> without the feature
    let _error: &Option<String> = &page.error_status;
}

/// Test cookies feature - Page struct has cookies field.
#[test]
#[cfg(all(feature = "cookies", not(feature = "decentralized")))]
fn test_cookies_field() {
    use crate::utils::PageResponse;

    let page_response = PageResponse {
        content: Some(b"<html></html>".to_vec()),
        status_code: StatusCode::OK,
        ..Default::default()
    };

    let page = build("https://example.com", page_response);

    // Verify cookies field exists and is accessible
    let _cookies: &Option<reqwest::header::HeaderMap> = &page.cookies;
}

/// Test chrome feature - screenshot_bytes field exists.
#[test]
#[cfg(all(feature = "chrome", not(feature = "decentralized")))]
fn test_chrome_screenshot_bytes_field() {
    use crate::utils::PageResponse;

    let screenshot_data = vec![0x89, 0x50, 0x4E, 0x47]; // PNG header bytes

    let page_response = PageResponse {
        content: Some(b"<html></html>".to_vec()),
        status_code: StatusCode::OK,
        screenshot_bytes: Some(screenshot_data.clone()),
        ..Default::default()
    };

    let page = build("https://example.com", page_response);

    assert!(
        page.screenshot_bytes.is_some(),
        "Page should have screenshot_bytes when chrome feature is enabled"
    );
    assert_eq!(
        page.screenshot_bytes.as_ref().unwrap(),
        &screenshot_data,
        "Screenshot bytes should match"
    );
}

/// Test time feature - duration field and get_duration_elapsed.
#[test]
#[cfg(all(feature = "time", not(feature = "decentralized")))]
fn test_time_duration_field() {
    use crate::utils::PageResponse;

    let page_response = PageResponse {
        content: Some(b"<html></html>".to_vec()),
        status_code: StatusCode::OK,
        duration: Some(tokio::time::Instant::now()),
        ..Default::default()
    };

    let page = build("https://example.com", page_response);
    let duration = page.get_duration_elapsed();

    // Duration should be very small since we just created the page
    assert!(
        duration.as_millis() < 1000,
        "Duration should be less than 1 second"
    );
}

/// Test openai feature - Page struct has openai_credits_used and extra_ai_data fields.
#[test]
#[cfg(all(feature = "openai", not(feature = "decentralized")))]
fn test_openai_fields() {
    use crate::features::openai_common::OpenAIUsage;
    use crate::utils::PageResponse;

    let page_response = PageResponse {
        content: Some(b"<html></html>".to_vec()),
        status_code: StatusCode::OK,
        ..Default::default()
    };

    let mut page = build("https://example.com", page_response);

    // Test openai_credits_used field
    assert!(
        page.openai_credits_used.is_none(),
        "openai_credits_used should be None initially"
    );

    // Set openai_credits_used
    page.openai_credits_used = Some(vec![OpenAIUsage::default()]);
    assert!(
        page.openai_credits_used.is_some(),
        "openai_credits_used should be set"
    );

    // Test extra_ai_data field
    assert!(
        page.extra_ai_data.is_none(),
        "extra_ai_data should be None initially"
    );

    // Set extra_ai_data
    page.extra_ai_data = Some(vec![AIResults::default()]);
    assert!(page.extra_ai_data.is_some(), "extra_ai_data should be set");
}

/// Test gemini feature - Page struct has gemini_credits_used and extra_gemini_data fields.
#[test]
#[cfg(all(feature = "gemini", not(feature = "decentralized")))]
fn test_gemini_fields() {
    use crate::features::gemini_common::GeminiUsage;
    use crate::utils::PageResponse;

    let page_response = PageResponse {
        content: Some(b"<html></html>".to_vec()),
        status_code: StatusCode::OK,
        ..Default::default()
    };

    let mut page = build("https://example.com", page_response);

    // Test gemini_credits_used field
    assert!(
        page.gemini_credits_used.is_none(),
        "gemini_credits_used should be None initially"
    );

    // Set gemini_credits_used
    page.gemini_credits_used = Some(vec![GeminiUsage::default()]);
    assert!(
        page.gemini_credits_used.is_some(),
        "gemini_credits_used should be set"
    );

    // Test extra_gemini_data field
    assert!(
        page.extra_gemini_data.is_none(),
        "extra_gemini_data should be None initially"
    );

    // Set extra_gemini_data
    page.extra_gemini_data = Some(vec![AIResults::default()]);
    assert!(
        page.extra_gemini_data.is_some(),
        "extra_gemini_data should be set"
    );
}

/// Test serde feature - Metadata can be serialized and deserialized.
#[test]
#[cfg(feature = "serde")]
fn test_metadata_serde() {
    let metadata = Metadata {
        title: Some("Test Title".into()),
        description: Some("Test Description".into()),
        image: Some("https://example.com/image.png".into()),
        #[cfg(feature = "chrome")]
        automation: None,
    };

    // Serialize to JSON
    let json = serde_json::to_string(&metadata).expect("Failed to serialize metadata");
    assert!(json.contains("Test Title"), "JSON should contain title");

    // Deserialize from JSON
    let deserialized: Metadata =
        serde_json::from_str(&json).expect("Failed to deserialize metadata");
    assert_eq!(
        metadata.title, deserialized.title,
        "Title should match after deserialization"
    );
    assert_eq!(
        metadata.description, deserialized.description,
        "Description should match after deserialization"
    );
    assert_eq!(
        metadata.image, deserialized.image,
        "Image should match after deserialization"
    );
}

/// Test serde feature - AIResults can be serialized and deserialized.
#[test]
#[cfg(feature = "serde")]
fn test_airesults_serde() {
    let ai_results = AIResults {
        input: "Test prompt".to_string(),
        js_output: "console.log('test');".to_string(),
        content_output: vec!["Result 1".to_string(), "Result 2".to_string()],
        screenshot_output: None,
        error: None,
    };

    // Serialize to JSON
    let json = serde_json::to_string(&ai_results).expect("Failed to serialize AIResults");
    assert!(json.contains("Test prompt"), "JSON should contain input");

    // Deserialize from JSON
    let deserialized: AIResults =
        serde_json::from_str(&json).expect("Failed to deserialize AIResults");
    assert_eq!(
        ai_results.input, deserialized.input,
        "Input should match after deserialization"
    );
    assert_eq!(
        ai_results.js_output, deserialized.js_output,
        "JS output should match after deserialization"
    );
    assert_eq!(
        ai_results.content_output.len(),
        deserialized.content_output.len(),
        "Content output length should match"
    );
}

/// Test decentralized feature - Page struct has links field.
#[test]
#[cfg(feature = "decentralized")]
fn test_decentralized_page() {
    // Decentralized Page struct has a links field for distributed crawling
    let page = Page::default();

    // Default Page should have empty links
    assert!(
        page.links.is_empty(),
        "Default Page should have empty links"
    );

    // Decentralized Page has external_domains_caseless field
    assert!(
        page.external_domains_caseless.is_empty(),
        "Default Page should have empty external_domains_caseless"
    );
}

/// Test smart feature implies chrome and chrome_intercept.
#[test]
#[cfg(all(feature = "smart", not(feature = "decentralized")))]
fn test_smart_feature() {
    use crate::utils::PageResponse;

    let page_response = PageResponse {
        content: Some(b"<html></html>".to_vec()),
        status_code: StatusCode::OK,
        ..Default::default()
    };

    let page = build("https://example.com", page_response);

    // Smart feature includes chrome, so screenshot_bytes should exist
    assert!(
        page.screenshot_bytes.is_none(),
        "screenshot_bytes should be None initially"
    );
}

#[test]
#[cfg(not(feature = "decentralized"))]
fn test_build_preserves_spawn_pages() {
    use crate::utils::PageResponse;

    let page = build(
        "https://example.com",
        PageResponse {
            status_code: StatusCode::OK,
            spawn_pages: Some(vec![
                "https://example.com/a".to_string(),
                "https://example.com/b".to_string(),
            ]),
            ..Default::default()
        },
    );

    let spawn_pages = page.spawn_pages.expect("spawn_pages should be preserved");
    assert_eq!(spawn_pages.len(), 2);
    assert_eq!(spawn_pages[0], "https://example.com/a");
    assert_eq!(spawn_pages[1], "https://example.com/b");
}

#[test]
#[cfg(all(feature = "smart", not(feature = "decentralized")))]
fn test_page_assign_merges_spawn_pages() {
    use crate::utils::PageResponse;

    let mut page = build(
        "https://example.com",
        PageResponse {
            status_code: StatusCode::OK,
            spawn_pages: Some(vec!["https://example.com/root".to_string()]),
            ..Default::default()
        },
    );

    let new_page = build(
        "https://example.com",
        PageResponse {
            status_code: StatusCode::OK,
            spawn_pages: Some(vec![
                "https://example.com/x".to_string(),
                "https://example.com/y".to_string(),
            ]),
            ..Default::default()
        },
    );

    page_assign(&mut page, new_page);

    let spawn_pages = page.spawn_pages.expect("spawn_pages should be merged");
    assert_eq!(spawn_pages.len(), 3);
    assert!(spawn_pages.contains(&"https://example.com/root".to_string()));
    assert!(spawn_pages.contains(&"https://example.com/x".to_string()));
    assert!(spawn_pages.contains(&"https://example.com/y".to_string()));
}

/// Test page_links field exists and works correctly.
#[test]
#[cfg(not(feature = "decentralized"))]
fn test_page_links_field() {
    use crate::utils::PageResponse;

    let page_response = PageResponse {
        content: Some(b"<html></html>".to_vec()),
        status_code: StatusCode::OK,
        ..Default::default()
    };

    let mut page = build("https://example.com", page_response);

    // page_links should be None initially
    assert!(
        page.page_links.is_none(),
        "page_links should be None initially"
    );

    // Set page_links
    let mut links = HashSet::new();
    links.insert(CaseInsensitiveString::new("https://example.com/page1"));
    page.page_links = Some(Box::new(links));

    assert!(page.page_links.is_some(), "page_links should be set");
    assert_eq!(
        page.page_links.as_ref().unwrap().len(),
        1,
        "page_links should have 1 link"
    );
}

/// Test bytes_transferred field exists and works correctly.
#[test]
#[cfg(not(feature = "decentralized"))]
fn test_bytes_transferred_field() {
    use crate::utils::PageResponse;

    let page_response = PageResponse {
        content: Some(b"<html></html>".to_vec()),
        status_code: StatusCode::OK,
        ..Default::default()
    };

    let mut page = build("https://example.com", page_response);

    // bytes_transferred should be None initially
    assert!(
        page.bytes_transferred.is_none(),
        "bytes_transferred should be None initially"
    );

    // Set bytes_transferred
    page.bytes_transferred = Some(1024.0);
    assert_eq!(
        page.bytes_transferred,
        Some(1024.0),
        "bytes_transferred should be 1024.0"
    );
}

/// Test waf_check and should_retry fields exist and work correctly.
#[test]
#[cfg(not(feature = "decentralized"))]
fn test_waf_and_retry_fields() {
    use crate::utils::PageResponse;

    let page_response = PageResponse {
        content: Some(b"<html></html>".to_vec()),
        status_code: StatusCode::OK,
        ..Default::default()
    };

    let mut page = build("https://example.com", page_response);

    // waf_check and should_retry should be false initially
    assert!(!page.waf_check, "waf_check should be false initially");
    assert!(!page.should_retry, "should_retry should be false initially");

    // Set waf_check
    page.waf_check = true;
    assert!(page.waf_check, "waf_check should be true");

    // Set should_retry
    page.should_retry = true;
    assert!(page.should_retry, "should_retry should be true");
}

/// Test that is_retryable_status excludes DNS_RESOLVE_ERROR but includes other 5xx codes.
#[test]
fn test_is_retryable_status_excludes_dns() {
    assert!(
        !is_retryable_status(*DNS_RESOLVE_ERROR),
        "DNS_RESOLVE_ERROR (525) must not be retryable"
    );
    assert!(
        is_retryable_status(StatusCode::INTERNAL_SERVER_ERROR),
        "500 should be retryable"
    );
    assert!(
        is_retryable_status(StatusCode::BAD_GATEWAY),
        "502 should be retryable"
    );
    assert!(
        is_retryable_status(StatusCode::SERVICE_UNAVAILABLE),
        "503 should be retryable"
    );
    assert!(
        is_retryable_status(StatusCode::GATEWAY_TIMEOUT),
        "504 should be retryable"
    );
    assert!(
        is_retryable_status(StatusCode::TOO_MANY_REQUESTS),
        "429 should be retryable"
    );
    assert!(
        !is_retryable_status(StatusCode::OK),
        "200 should not be retryable"
    );
}

/// Test blocked_crawl field exists and works correctly.
#[test]
#[cfg(not(feature = "decentralized"))]
fn test_blocked_crawl_field() {
    use crate::utils::PageResponse;

    let page_response = PageResponse {
        content: Some(b"<html></html>".to_vec()),
        status_code: StatusCode::OK,
        ..Default::default()
    };

    let mut page = build("https://example.com", page_response);

    // blocked_crawl should be false initially
    assert!(
        !page.blocked_crawl,
        "blocked_crawl should be false initially"
    );

    // Set blocked_crawl
    page.blocked_crawl = true;
    assert!(page.blocked_crawl, "blocked_crawl should be true");
}

/// Test extract_root_domain strips TLD correctly.
#[test]
fn test_extract_root_domain() {
    assert_eq!(extract_root_domain("example.com"), "example");
    assert_eq!(extract_root_domain("example.org"), "example");
    assert_eq!(extract_root_domain("sub.example.com"), "example.com");
    assert_eq!(extract_root_domain("deep.sub.example.co.uk"), "co.uk");
    assert_eq!(extract_root_domain("localhost"), "localhost");
}

/// Test is_subdomain matches across different TLDs.
#[test]
fn test_is_subdomain_tld_matching() {
    // Same root domain, different TLDs — should match (both 2-part → compare first part)
    assert!(is_subdomain("example.com", "example.org"));
    assert!(is_subdomain("example.net", "example.com"));

    // Both 3-part with same last two parts — should match
    assert!(is_subdomain("a.example.com", "b.example.com"));

    // 3-part vs 2-part extracts differently (example.com vs example) — won't match
    assert!(!is_subdomain("sub.example.com", "example.com"));

    // Different root domains — should NOT match
    assert!(!is_subdomain("example.com", "other.com"));
    assert!(!is_subdomain("myexample.com", "example.com"));
}

/// Test get_page_selectors_base with tld=true produces a root-domain matcher.
#[test]
fn test_get_page_selectors_base_tld() {
    let selectors = get_page_selectors_base("https://example.com/page", false, true);
    // First element is the sub_matcher — should be the root domain without TLD
    assert_eq!(selectors.0.as_str(), "example");

    let selectors_no_tld = get_page_selectors_base("https://example.com/page", false, false);
    // Without tld, sub_matcher should be empty
    assert!(selectors_no_tld.0.is_empty());
}

/// Test parent_host_match allows different TLDs when sub_matcher is root domain.
#[test]
fn test_parent_host_match_tld() {
    let parent_host = CompactString::from("example.com");
    let base_host = CompactString::from("example.com");
    // sub_matcher is "example" (what extract_root_domain returns for tld mode)
    let sub_matcher = CompactString::from("example");

    // Same host — always allowed
    assert!(parent_host_match(
        Some("example.com"),
        "example",
        &parent_host,
        &base_host,
        &sub_matcher,
    ));

    // Different TLD — allowed via is_subdomain through sub_matcher
    assert!(parent_host_match(
        Some("example.org"),
        "example",
        &parent_host,
        &base_host,
        &sub_matcher,
    ));

    // Completely different domain — NOT allowed
    assert!(!parent_host_match(
        Some("other.com"),
        "example",
        &parent_host,
        &base_host,
        &sub_matcher,
    ));
}

/// Test that validate_link resolves relative URLs against the subdomain host,
/// not the original crawl domain. This is the core fix for subdomain crawling.
#[test]
fn test_validate_link_subdomain_relative_resolution() {
    // Simulate: crawling www.example.com with subdomains=true,
    // currently on a page at sub.example.com
    let selectors = get_page_selectors("https://www.example.com/", true, false);

    let external_domains: Arc<HashSet<CaseInsensitiveString>> = Arc::new(HashSet::new());

    // With the fix: base is the page's own URL (sub.example.com)
    let subdomain_base = url::Url::parse("https://sub.example.com/page").unwrap();
    let mut no_page_links: Option<HashSet<CaseInsensitiveString>> = None;

    let result = validate_link(
        &Some(&subdomain_base),
        "/about",
        &selectors.0,
        &selectors.1[0],
        &selectors.2,
        &selectors.0,
        &external_domains,
        &mut no_page_links,
    );

    assert!(
        result.is_some(),
        "Relative link on subdomain page should be accepted"
    );
    assert_eq!(
        result.unwrap().as_str(),
        "https://sub.example.com/about",
        "Relative link should resolve against subdomain host, not crawl origin"
    );

    // Before the fix: base was the crawl origin (www.example.com)
    // This would incorrectly resolve /about to www.example.com/about
    let crawl_origin_base = url::Url::parse("https://www.example.com/").unwrap();
    let mut no_page_links2: Option<HashSet<CaseInsensitiveString>> = None;

    let result_old = validate_link(
        &Some(&crawl_origin_base),
        "/about",
        &selectors.0,
        &selectors.1[0],
        &selectors.2,
        &selectors.0,
        &external_domains,
        &mut no_page_links2,
    );

    assert!(result_old.is_some());
    assert_eq!(
        result_old.unwrap().as_str(),
        "https://www.example.com/about",
        "With crawl origin as base, link resolves against wrong host"
    );
}

/// Test that validate_link still works correctly for same-domain pages
/// (no regression from the subdomain fix).
#[test]
fn test_validate_link_same_domain_resolution() {
    let selectors = get_page_selectors("https://www.example.com/", false, false);

    let external_domains: Arc<HashSet<CaseInsensitiveString>> = Arc::new(HashSet::new());

    let page_base = url::Url::parse("https://www.example.com/some-page").unwrap();
    let mut no_page_links: Option<HashSet<CaseInsensitiveString>> = None;

    let result = validate_link(
        &Some(&page_base),
        "/about",
        &selectors.0,
        &selectors.1[0],
        &selectors.2,
        &selectors.0,
        &external_domains,
        &mut no_page_links,
    );

    assert!(result.is_some());
    assert_eq!(
        result.unwrap().as_str(),
        "https://www.example.com/about",
        "Same-domain relative link should resolve correctly"
    );
}

/// Integration test: build a page at a subdomain URL with relative links,
/// verify links() resolves them against the subdomain.
#[tokio::test]
#[cfg(all(
    not(feature = "decentralized"),
    not(feature = "chrome"),
    not(feature = "cache_request")
))]
async fn test_subdomain_page_links_resolution() {
    use crate::utils::PageResponse;

    let html = br#"<html><body>
        <a href="/about">About</a>
        <a href="/contact">Contact</a>
        <a href="https://sub.example.com/absolute">Absolute</a>
    </body></html>"#;

    let mut page = build_with_parse(
        "https://sub.example.com/page",
        PageResponse {
            content: Some(html.to_vec()),
            status_code: reqwest::StatusCode::OK,
            ..Default::default()
        },
    );

    // Selectors for crawl origin www.example.com with subdomains=true
    let selectors = get_page_selectors("https://www.example.com/", true, false);

    // Simulate the fix: base is derived from the page's own URL
    let page_base = url::Url::parse("https://sub.example.com/page")
        .ok()
        .map(Box::new);

    let links = page.links(&selectors, &page_base).await;

    let expected_about: CaseInsensitiveString = "https://sub.example.com/about".into();
    let expected_contact: CaseInsensitiveString = "https://sub.example.com/contact".into();
    let expected_absolute: CaseInsensitiveString = "https://sub.example.com/absolute".into();
    let wrong_about: CaseInsensitiveString = "https://www.example.com/about".into();

    assert!(
        links.contains(&expected_about),
        "Relative /about should resolve to sub.example.com/about, got: {:?}",
        &links
    );
    assert!(
        links.contains(&expected_contact),
        "Relative /contact should resolve to sub.example.com/contact, got: {:?}",
        &links
    );
    assert!(
        links.contains(&expected_absolute),
        "Absolute link should be preserved, got: {:?}",
        &links
    );
    assert!(
        !links.contains(&wrong_about),
        "Links should NOT resolve against crawl origin www.example.com"
    );
}

/// Test that same-domain page links still resolve correctly after the fix.
#[tokio::test]
#[cfg(all(
    not(feature = "decentralized"),
    not(feature = "chrome"),
    not(feature = "cache_request")
))]
async fn test_same_domain_page_links_resolution() {
    use crate::utils::PageResponse;

    let html = br#"<html><body><a href="/about">About</a></body></html>"#;

    let mut page = build_with_parse(
        "https://www.example.com/page",
        PageResponse {
            content: Some(html.to_vec()),
            status_code: reqwest::StatusCode::OK,
            ..Default::default()
        },
    );

    let selectors = get_page_selectors("https://www.example.com/", false, false);

    // Base is the page's own URL (same domain as crawl origin)
    let page_base = url::Url::parse("https://www.example.com/page")
        .ok()
        .map(Box::new);

    let links = page.links(&selectors, &page_base).await;

    let expected: CaseInsensitiveString = "https://www.example.com/about".into();
    assert!(
        links.contains(&expected),
        "Same-domain relative link should resolve correctly, got: {:?}",
        &links
    );
}

/// DNS resolve error (525) should not be retried and needs_retry() must be false.
#[cfg(not(feature = "decentralized"))]
#[test]
fn test_dns_error_no_retry() {
    let res = PageResponse {
        status_code: StatusCode::from_u16(525).unwrap(),
        content: None,
        ..Default::default()
    };
    let page = build("https://nonexistent.invalid", res);
    assert!(
        !page.should_retry,
        "DNS resolve errors (525) must not be retried"
    );
    assert!(
        !page.needs_retry(),
        "DNS resolve errors (525) — needs_retry() must be false"
    );
}

/// Normal server errors should still be retried.
#[cfg(not(feature = "decentralized"))]
#[test]
fn test_server_error_still_retries() {
    let res = PageResponse {
        status_code: StatusCode::INTERNAL_SERVER_ERROR,
        content: Some(Default::default()),
        ..Default::default()
    };
    let page = build("https://example.com", res);
    assert!(page.should_retry, "500 errors should still be retried");
}

#[cfg(not(feature = "decentralized"))]
#[test]
fn test_chrome_error_page_detected_as_empty() {
    // Realistic Chrome error page: ~157KB with dino game CSS/JS, no </body>,
    // ends with loadTimeDataRaw JSON blob containing errorCode.
    let padding = "x".repeat(1000); // enough to exceed 500 byte minimum
    let chrome_error_html_str = format!(
        "<html lang=\"en\" dir=\"ltr\">\n\
         <style>{padding}</style>\n\
         <div id=\"main-frame-error\" class=\"interstitial-wrapper\">\n\
         <h1><span>This site can\u{2019}t be reached</span></h1>\n\
         <div class=\"error-code\">ERR_TUNNEL_CONNECTION_FAILED</div>\n\
         </div>\n\
         <script>var loadTimeDataRaw = {{\"errorCode\":\"ERR_TUNNEL_CONNECTION_FAILED\",\
         \"heading\":{{\"msg\":\"This site can't be reached\"}},\
         \"title\":\"www.example.com\"}};</script></html>"
    );
    let chrome_error_html = chrome_error_html_str.as_bytes();

    assert!(
        is_chrome_error_page(chrome_error_html),
        "should detect Chrome error page by structural tail match"
    );
    assert!(
        !validate_empty(&Some(chrome_error_html.to_vec()), true),
        "Chrome error page should be treated as empty/invalid content"
    );

    let res = PageResponse {
        status_code: StatusCode::OK,
        content: Some(chrome_error_html.to_vec()),
        ..Default::default()
    };
    let page = build("https://www.example.com", res);
    assert!(
        page.should_retry,
        "Chrome error page with 200 status should trigger retry"
    );
    assert_eq!(
        page.status_code,
        StatusCode::from_u16(599).unwrap(),
        "Chrome error page should be reclassified to 599"
    );
    assert!(
        !page.get_html().is_empty(),
        "Chrome error page content should be preserved for debugging"
    );
}

#[cfg(not(feature = "decentralized"))]
#[test]
fn test_normal_page_not_detected_as_chrome_error() {
    let normal_html =
        b"<html><head><title>My Blog</title></head><body><p>Hello world</p></body></html>";
    assert!(!is_chrome_error_page(normal_html));
    assert!(validate_empty(&Some(normal_html.to_vec()), true));
}

// ---------------------------------------------------------------------------
// is_retryable_status — exhaustive coverage
// ---------------------------------------------------------------------------

#[test]
fn test_retryable_status_server_errors() {
    // All 5xx except 525 are retryable
    for code in [500, 501, 502, 503, 504, 521, 522, 523, 524, 598, 599] {
        let status = StatusCode::from_u16(code).unwrap();
        assert!(is_retryable_status(status), "{code} should be retryable");
    }
}

#[test]
fn test_retryable_status_rate_limit_and_timeout() {
    assert!(
        is_retryable_status(StatusCode::TOO_MANY_REQUESTS),
        "429 retryable"
    );
    assert!(
        is_retryable_status(StatusCode::REQUEST_TIMEOUT),
        "408 retryable"
    );
}

#[test]
fn test_non_retryable_status_dns_error() {
    let dns = StatusCode::from_u16(525).unwrap();
    assert!(!is_retryable_status(dns), "525 DNS must never be retried");
}

#[test]
fn test_non_retryable_client_errors() {
    for code in [400, 401, 403, 404, 405, 409, 410, 422, 451] {
        let status = StatusCode::from_u16(code).unwrap();
        assert!(
            !is_retryable_status(status),
            "{code} should NOT be retryable"
        );
    }
}

#[test]
fn test_non_retryable_success_codes() {
    for code in [200, 201, 204, 301, 302, 304] {
        let status = StatusCode::from_u16(code).unwrap();
        assert!(
            !is_retryable_status(status),
            "{code} should NOT be retryable"
        );
    }
}

// ---------------------------------------------------------------------------
// needs_retry — combinatorial coverage
// ---------------------------------------------------------------------------

#[cfg(not(feature = "decentralized"))]
#[test]
fn test_needs_retry_should_retry_flag_alone() {
    let res = PageResponse {
        status_code: StatusCode::OK,
        content: Some(b"<html><body>ok</body></html>".to_vec()),
        ..Default::default()
    };
    let mut page = build("https://example.com", res);
    assert!(!page.needs_retry(), "clean 200 page should not need retry");

    page.should_retry = true;
    assert!(page.needs_retry(), "should_retry flag forces retry");
}

#[cfg(not(feature = "decentralized"))]
#[test]
fn test_needs_retry_content_truncated_alone() {
    let res = PageResponse {
        status_code: StatusCode::OK,
        content: Some(b"<html><body>ok</body></html>".to_vec()),
        ..Default::default()
    };
    let mut page = build("https://example.com", res);
    page.content_truncated = true;
    assert!(page.needs_retry(), "truncated content forces retry");
}

#[cfg(not(feature = "decentralized"))]
#[test]
fn test_needs_retry_retryable_status_alone() {
    let res = PageResponse {
        status_code: StatusCode::BAD_GATEWAY,
        content: Some(Default::default()),
        ..Default::default()
    };
    let page = build("https://example.com", res);
    // Even if should_retry wasn't explicitly set by build(), needs_retry
    // catches it via is_retryable_status.
    assert!(page.needs_retry(), "502 status triggers needs_retry");
}

#[cfg(not(feature = "decentralized"))]
#[test]
fn test_needs_retry_dns_error_not_retried() {
    let res = PageResponse {
        status_code: StatusCode::from_u16(525).unwrap(),
        content: None,
        ..Default::default()
    };
    let page = build("https://nonexistent.invalid", res);
    // DNS errors (525) are explicitly excluded from is_retryable_status AND
    // should not set should_retry either.
    assert!(!page.needs_retry(), "DNS 525 must never trigger retry");
}

#[cfg(not(feature = "decentralized"))]
#[test]
fn test_needs_retry_client_error_no_flags() {
    let res = PageResponse {
        status_code: StatusCode::NOT_FOUND,
        content: Some(b"<html>not found</html>".to_vec()),
        ..Default::default()
    };
    let page = build("https://example.com/missing", res);
    assert!(!page.needs_retry(), "404 should not need retry");
}

#[cfg(not(feature = "decentralized"))]
#[test]
fn test_needs_retry_multiple_flags_combined() {
    let res = PageResponse {
        status_code: StatusCode::SERVICE_UNAVAILABLE,
        content: Some(Default::default()),
        ..Default::default()
    };
    let mut page = build("https://example.com", res);
    page.content_truncated = true;
    // Both retryable status AND truncated — still just needs_retry = true
    assert!(
        page.needs_retry(),
        "multiple retry signals still returns true"
    );
}

// ---------------------------------------------------------------------------
// is_chrome_error_page — edge cases
// ---------------------------------------------------------------------------

#[cfg(not(feature = "decentralized"))]
#[test]
fn test_chrome_error_page_under_500_bytes() {
    let short = b"<script>var loadTimeDataRaw = {\"errorCode\":\"ERR_FAIL\"};</script></html>";
    assert!(
        !is_chrome_error_page(short),
        "content < 500 bytes should be rejected"
    );
}

#[cfg(not(feature = "decentralized"))]
#[test]
fn test_chrome_error_page_missing_tail() {
    let padding = "x".repeat(1000);
    let html = format!(
        "<html><style>{padding}</style>\
         <script>var loadTimeDataRaw = {{\"errorCode\":\"ERR_FAIL\"}};</script></body></html>"
    );
    assert!(
        !is_chrome_error_page(html.as_bytes()),
        "wrong tail (has </body>) should not match"
    );
}

#[cfg(not(feature = "decentralized"))]
#[test]
fn test_chrome_error_page_missing_error_code_needle() {
    let padding = "x".repeat(1000);
    let html = format!(
        "<html><style>{padding}</style>\
         <script>var loadTimeDataRaw = {{\"someKey\":\"value\"}};</script></html>"
    );
    assert!(
        !is_chrome_error_page(html.as_bytes()),
        "missing errorCode needle should not match"
    );
}

#[cfg(not(feature = "decentralized"))]
#[test]
fn test_chrome_error_page_trailing_whitespace() {
    let padding = "x".repeat(1000);
    let html = format!(
        "<html><style>{padding}</style>\
         <script>var loadTimeDataRaw = {{\"errorCode\":\"ERR_TUNNEL_CONNECTION_FAILED\"}};</script></html>\n\r\n  "
    );
    assert!(
        is_chrome_error_page(html.as_bytes()),
        "trailing whitespace should be trimmed"
    );
}

#[cfg(not(feature = "decentralized"))]
#[test]
fn test_chrome_error_page_needle_outside_4kb_window() {
    // Put the errorCode at the start, then pad >4KB before the tail.
    // The needle should NOT be found because we only scan the last 4KB.
    let error_part = r#"<script>var loadTimeDataRaw = {"errorCode":"ERR_FAIL"};</script>"#;
    let padding = "x".repeat(5000); // >4KB of padding after needle
    let html = format!(
        "<html>{error_part}<style>{padding}</style>\
         <script>var more = {{}};</script></html>"
    );
    assert!(
        !is_chrome_error_page(html.as_bytes()),
        "needle outside last 4KB window should not match"
    );
}

// ---------------------------------------------------------------------------
// get_timeout — status-specific delay values
// ---------------------------------------------------------------------------

#[cfg(not(feature = "decentralized"))]
#[test]
fn test_get_timeout_rate_limit() {
    let res = PageResponse {
        status_code: StatusCode::TOO_MANY_REQUESTS,
        content: None,
        ..Default::default()
    };
    let page = build("https://example.com", res);
    let timeout = page.get_timeout();
    assert_eq!(
        timeout,
        Some(std::time::Duration::from_millis(2_500)),
        "429 → 2500ms"
    );
}

#[cfg(not(feature = "decentralized"))]
#[test]
fn test_get_timeout_gateway_timeout() {
    let res = PageResponse {
        status_code: StatusCode::GATEWAY_TIMEOUT,
        content: None,
        ..Default::default()
    };
    let page = build("https://example.com", res);
    let timeout = page.get_timeout();
    assert_eq!(
        timeout,
        Some(std::time::Duration::from_millis(1_500)),
        "504 → 1500ms"
    );
}

#[cfg(not(feature = "decentralized"))]
#[test]
fn test_get_timeout_proxy_errors() {
    for code in [598u16, 599] {
        let res = PageResponse {
            status_code: StatusCode::from_u16(code).unwrap(),
            content: None,
            ..Default::default()
        };
        let page = build("https://example.com", res);
        let timeout = page.get_timeout();
        assert_eq!(
            timeout,
            Some(std::time::Duration::from_millis(500)),
            "{code} → 500ms"
        );
    }
}

#[cfg(not(feature = "decentralized"))]
#[test]
fn test_get_timeout_normal_status_none() {
    for code in [200u16, 301, 404, 500, 502, 503] {
        let res = PageResponse {
            status_code: StatusCode::from_u16(code).unwrap(),
            content: Some(b"<html></html>".to_vec()),
            ..Default::default()
        };
        let page = build("https://example.com", res);
        let timeout = page.get_timeout();
        assert_eq!(timeout, None, "{code} should have no special timeout");
    }
}

// ---------------------------------------------------------------------------
// validate_empty — edge cases
// ---------------------------------------------------------------------------

#[cfg(not(feature = "decentralized"))]
#[test]
fn test_validate_empty_none_content() {
    assert!(!validate_empty(&None, true), "None content is empty");
    assert!(
        !validate_empty(&None, false),
        "None content is empty regardless of success"
    );
}

#[cfg(not(feature = "decentralized"))]
#[test]
fn test_validate_empty_zero_length() {
    assert!(!validate_empty(&Some(vec![]), true), "empty vec is empty");
    assert!(
        !validate_empty(&Some(vec![]), false),
        "empty vec is empty regardless of success"
    );
}

#[cfg(not(feature = "decentralized"))]
#[test]
fn test_validate_empty_html_shell() {
    let shell = b"<html><head></head><body></body></html>".to_vec();
    assert!(
        !validate_empty(&Some(shell), true),
        "empty HTML shell should be rejected"
    );
}

#[cfg(not(feature = "decentralized"))]
#[test]
fn test_validate_empty_valid_content() {
    let valid = b"<html><head><title>Test</title></head><body><p>Hello</p></body></html>".to_vec();
    assert!(validate_empty(&Some(valid), true), "valid HTML should pass");
}

// ---------------------------------------------------------------------------
// 401 proxy-conditional retry
// ---------------------------------------------------------------------------

#[cfg(not(feature = "decentralized"))]
#[test]
fn test_401_not_retried_without_proxy() {
    let res = PageResponse {
        status_code: StatusCode::UNAUTHORIZED,
        content: Some(b"unauthorized".to_vec()),
        ..Default::default()
    };
    let page = build("https://example.com", res);
    assert!(
        !page.needs_retry(),
        "401 without proxy_configured should NOT trigger retry"
    );
}

#[cfg(not(feature = "decentralized"))]
#[test]
fn test_401_retried_with_proxy() {
    let res = PageResponse {
        status_code: StatusCode::UNAUTHORIZED,
        content: Some(b"unauthorized".to_vec()),
        ..Default::default()
    };
    let mut page = build("https://example.com", res);
    page.proxy_configured = true;
    assert!(
        page.needs_retry(),
        "401 with proxy_configured should trigger retry"
    );
}

#[cfg(not(feature = "decentralized"))]
#[test]
fn test_401_proxy_flag_does_not_affect_other_client_errors() {
    // 404 (with content) should NOT be retried even with proxy
    let res = PageResponse {
        status_code: StatusCode::NOT_FOUND,
        content: Some(b"<html><body>Not Found</body></html>".to_vec()),
        ..Default::default()
    };
    let mut page = build("https://example.com/missing", res);
    page.proxy_configured = true;
    assert!(
        !page.needs_retry(),
        "404 should NOT be retried even with proxy"
    );
}

// ---------------------------------------------------------------------------
// needs_retry integration: proxy + other flags
// ---------------------------------------------------------------------------

#[cfg(not(feature = "decentralized"))]
#[test]
fn test_needs_retry_server_error_regardless_of_proxy() {
    let res = PageResponse {
        status_code: StatusCode::INTERNAL_SERVER_ERROR,
        content: Some(Default::default()),
        ..Default::default()
    };
    let page = build("https://example.com", res);
    // 500 is retryable regardless of proxy config
    assert!(page.needs_retry(), "500 retried without proxy");
}

// ---------------------------------------------------------------------------
// Trait implementations
// ---------------------------------------------------------------------------

impl crate::traits::PageData for Page {
    #[inline]
    fn url(&self) -> &str {
        self.get_url()
    }

    #[inline]
    fn url_final(&self) -> &str {
        match self.final_redirect_destination.as_deref() {
            Some(u) => u,
            _ => &self.url,
        }
    }

    #[inline]
    fn bytes(&self) -> Option<&[u8]> {
        self.get_bytes()
    }

    #[inline]
    fn html(&self) -> String {
        self.get_html()
    }

    #[inline]
    fn html_bytes_u8(&self) -> &[u8] {
        self.get_html_bytes_u8()
    }

    #[inline]
    fn status_code(&self) -> StatusCode {
        self.status_code
    }

    #[inline]
    fn headers(&self) -> Option<&reqwest::header::HeaderMap> {
        self.headers.as_ref()
    }

    #[inline]
    fn is_empty(&self) -> bool {
        self.is_empty()
    }
}

#[cfg(feature = "time")]
impl crate::traits::PageTimingExt for Page {
    #[inline]
    fn duration_elapsed(&self) -> tokio::time::Duration {
        self.get_duration_elapsed()
    }
}

#[cfg(feature = "chrome")]
impl crate::traits::PageChromeExt for Page {
    #[inline]
    fn chrome_page(&self) -> Option<&chromiumoxide::Page> {
        self.get_chrome_page()
    }

    #[inline]
    fn screenshot_bytes(&self) -> Option<&[u8]> {
        self.screenshot_bytes.as_deref()
    }
}
