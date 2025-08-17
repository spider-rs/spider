use crate::compact_str::CompactString;
#[cfg(all(feature = "chrome", not(feature = "decentralized")))]
use crate::configuration::{AutomationScripts, ExecutionScripts};
use crate::utils::abs::convert_abs_path;
use crate::utils::{
    css_selectors::{BASE_CSS_SELECTORS, BASE_CSS_SELECTORS_WITH_XML},
    get_domain_from_url, hash_html, networking_capable, PageResponse, RequestError,
};
use crate::CaseInsensitiveString;
use crate::Client;
use crate::RelativeSelectors;
use auto_encoder::auto_encode_bytes;
use hashbrown::HashSet;
use lol_html::AsciiCompatibleEncoding;
use phf::phf_set;
use regex::bytes::Regex;
use reqwest::StatusCode;
use tokio::time::Duration;

#[cfg(all(feature = "time", not(feature = "decentralized")))]
use tokio::time::Instant;

#[cfg(all(feature = "decentralized", feature = "headers"))]
use crate::utils::FetchPageResult;
#[cfg(not(feature = "decentralized"))]
use tokio_stream::StreamExt;
use url::Url;

/// Allocate up to 16kb upfront for small pages.
pub(crate) const MAX_PRE_ALLOCATED_HTML_PAGE_SIZE: u64 = 16 * 1024;
/// Allocate up to 16kb upfront for small pages.
pub(crate) const MAX_PRE_ALLOCATED_HTML_PAGE_SIZE_USIZE: usize =
    MAX_PRE_ALLOCATED_HTML_PAGE_SIZE as usize;

/// Allocate up to 16kb * 4 upfront for small pages.
#[cfg(feature = "chrome")]
pub(crate) const TURNSTILE_WALL_PAGE_SIZE: usize = MAX_PRE_ALLOCATED_HTML_PAGE_SIZE_USIZE * 4;

lazy_static! {
    /// Wildcard match all domains.
    static ref CASELESS_WILD_CARD: CaseInsensitiveString = CaseInsensitiveString::new("*");
    static ref SSG_CAPTURE: Regex =  Regex::new(r#""(.*?)""#).unwrap();
    static ref GATSBY: Option<String> =  Some("gatsby-chunk-mapping".into());
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
        StatusCode::from_u16(524).expect("valid status code");
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
        if let Some(io_err) = err.source().and_then(|e| e.downcast_ref::<io::Error>()) {
            match io_err.kind() {
                io::ErrorKind::ConnectionRefused => return *CONNECTION_REFUSED_ERROR,
                io::ErrorKind::ConnectionAborted => return *CONNECTION_ABORTED_ERROR,
                io::ErrorKind::ConnectionReset => return *CONNECTION_RESET_ERROR,
                io::ErrorKind::NotFound => return *DNS_RESOLVE_ERROR,
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
        return *UNREACHABLE_REQUEST_ERROR;
    }

    *UNKNOWN_STATUS_ERROR
}

#[cfg(all(not(feature = "decentralized"), feature = "smart"))]
lazy_static! {
    static ref DOM_WATCH_METHODS: aho_corasick::AhoCorasick = {
        let patterns = &[
            ".createElementNS",
            ".removeChild",
            ".insertBefore",
            ".createElement",
            ".setAttribute",
            ".createTextNode",
            ".replaceChildren",
            ".prepend",
            ".append",
            ".appendChild",
            ".write",
        ];

        aho_corasick::AhoCorasick::new(patterns).unwrap()
    };
}

lazy_static! {
    /// Downloadable media types.
    pub(crate) static ref DOWNLOADABLE_MEDIA_TYPES: phf::Set<&'static str> = phf_set! {
        "audio/mpeg",    // mp3
        "audio/wav",     // wav
        "audio/ogg",     // ogg
        "audio/flac",    // flac
        "audio/aac",     // aac
        "video/mp4",     // mp4
        "video/webm",    // webm
        "video/ogg",     // ogv
        "video/x-matroska",    // mkv
        "application/ogg",     // ogx for Ogg
        "application/octet-stream", // general binary data, often used for downloads
        "application/zip",     // zip archives
        "application/x-rar-compressed", // rar archives
        "application/x-7z-compressed",   // 7z archives
        "application/x-tar",   // tar archives
        "application/pdf",     // pdf
        "application/rtf"     // rtf
    };

    /// Visual assets to ignore.
    pub(crate) static ref IGNORE_ASSETS: HashSet<CaseInsensitiveString> = {
        let mut m: HashSet<CaseInsensitiveString> = HashSet::with_capacity(62);

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
            "pdf", "eps", "yaml", "yml", "rtf",              // Other additional files

            // Including extensions with extra dot
            ".jpg", ".jpeg", ".png", ".gif", ".svg", ".webp",
            ".mp4", ".avi", ".mov", ".wmv", ".flv",
            ".mp3", ".wav", ".ogg",
            ".woff", ".woff2", ".ttf", ".otf",
            ".swf", ".xap",
            ".ico", ".eot",
            ".bmp", ".tiff", ".tif", ".heic", ".heif",
            ".mkv", ".webm", ".m4v",
            ".aac", ".flac", ".m4a", ".aiff",
            ".pdf", ".eps", ".yaml", ".yml", ".rtf"
        ].map(|s| s.into()));

        m
    };

    /// The chunk size for the rewriter. Can be adjusted using the env var "SPIDER_STREAMING_CHUNK_SIZE".
    pub(crate) static ref STREAMING_CHUNK_SIZE: usize = {
        let default_streaming_chunk_size: usize = 8192 * num_cpus::get_physical().min(64);
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

/// The automation results..
#[cfg(feature = "chrome")]
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AutomationResults {
    /// The prompt used for the GPT.
    pub input: String,
    /// The content output returned from observing the changes.
    pub content_output: serde_json::Value,
    /// The base64 image of the page.
    pub screenshot_output: Option<String>,
    /// The error that occured if any.
    pub error: Option<String>,
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
    /// Fallback value if none match or detection failed.
    #[default]
    None,
}

/// Represent a page visited.
#[derive(Debug, Clone, Default)]
#[cfg(not(feature = "decentralized"))]
pub struct Page {
    /// The bytes of the resource.
    html: Option<Box<Vec<u8>>>,
    /// Base absolute url for page.
    pub(crate) base: Option<Url>,
    /// The raw url for the page. Useful since Url::parse adds a trailing slash.
    url: String,
    #[cfg(feature = "headers")]
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
    pub external_domains_caseless: Box<HashSet<CaseInsensitiveString>>,
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
}

/// Represent a page visited.
#[cfg(feature = "decentralized")]
#[derive(Debug, Clone, Default)]
pub struct Page {
    /// The bytes of the resource.
    html: Option<Box<Vec<u8>>>,
    #[cfg(feature = "headers")]
    /// The headers of the page request response.
    pub headers: Option<reqwest::header::HeaderMap>,
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
    pub external_domains_caseless: Box<HashSet<CaseInsensitiveString>>,
    /// The final destination of the page if redirects were performed [Unused].
    pub final_redirect_destination: Option<String>,
    #[cfg(feature = "chrome")]
    /// The screenshot bytes of the page.
    pub screenshot_bytes: Option<Vec<u8>>,
    #[cfg(feature = "openai")]
    /// The credits used from OpenAI in order.
    pub openai_credits_used: Option<Vec<crate::features::openai_common::OpenAIUsage>>,
    #[cfg(feature = "openai")]
    /// The extra data from the AI, example extracting data etc...
    pub extra_ai_data: Option<Vec<AIResults>>,
    /// The links found on the page. Unused until we can structure the buffers to match.
    pub page_links: Option<Box<HashSet<CaseInsensitiveString>>>,
    /// The request should retry.
    pub should_retry: bool,
    /// A WAF was found on the page.
    pub waf_check: bool,
    /// The page was blocked from crawling usual from using website::on_should_crawl_callback.
    pub blocked_crawl: bool,
    /// The signature of the page to de-duplicate content.
    pub signature: Option<u64>,
    /// The anti-bot tech used.
    pub anti_bot_tech: AntiBotTech,
    /// Page metadata.
    pub metadata: Option<Box<Metadata>>,
}

/// Validate link and push into the map
pub(crate) fn validate_link<A: PartialEq + Eq + std::hash::Hash + From<String>>(
    base: &Option<&Url>,
    href: &str,
    base_domain: &CompactString,
    parent_host: &CompactString,
    base_input_domain: &CompactString,
    sub_matcher: &CompactString,
    external_domains_caseless: &Box<HashSet<CaseInsensitiveString>>,
    links_pages: &mut Option<HashSet<A>>,
) -> Option<Url> {
    if let Some(b) = base {
        let abs = convert_abs_path(b, href);

        if let Some(link_map) = links_pages {
            link_map.insert(A::from(href.to_string()));
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
pub(crate) fn push_link<A: PartialEq + Eq + std::hash::Hash + From<String>>(
    base: &Option<&Url>,
    href: &str,
    map: &mut HashSet<A>,
    base_domain: &CompactString,
    parent_host: &CompactString,
    parent_host_scheme: &CompactString,
    base_input_domain: &CompactString,
    sub_matcher: &CompactString,
    external_domains_caseless: &Box<HashSet<CaseInsensitiveString>>,
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
        map.insert(abs.as_str().to_string().into());
    }
}

/// Validate link and push into the map
pub(crate) fn push_link_verify<A: PartialEq + Eq + std::hash::Hash + From<String>>(
    base: &Option<&Url>,
    href: &str,
    map: &mut HashSet<A>,
    base_domain: &CompactString,
    parent_host: &CompactString,
    parent_host_scheme: &CompactString,
    base_input_domain: &CompactString,
    sub_matcher: &CompactString,
    external_domains_caseless: &Box<HashSet<CaseInsensitiveString>>,
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
            map.insert(abs.as_str().to_string().into());
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
pub(crate) fn push_link_check<A: PartialEq + Eq + std::hash::Hash + From<String>>(
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
        map.insert(abs.as_str().to_string().into());
    }
}

/// get the clean domain name
pub(crate) fn domain_name(domain: &Url) -> &str {
    domain.host_str().unwrap_or_default()
}

/// extract the valid domains from a url.
fn extract_root_domain(domain: &str) -> &str {
    let parts: Vec<&str> = domain.split('.').collect();

    if parts.len() >= 3 {
        let start_index = parts.len() - 2;
        if let Some(start_pos) = domain.find(parts[start_index]) {
            &domain[start_pos..]
        } else {
            domain
        }
    } else if parts.len() == 2 {
        parts[0]
    } else {
        domain
    }
}

/// check for subdomain matches
fn is_subdomain(subdomain: &str, domain: &str) -> bool {
    extract_root_domain(subdomain) == extract_root_domain(domain)
}

/// validation to match a domain to parent host and the top level redirect for the crawl 'parent_host' and 'base_host' being the input start domain.
pub(crate) fn parent_host_match(
    host_name: Option<&str>,
    base_domain: &str,           // the base domain input
    parent_host: &CompactString, // the main parent host
    base_host: &CompactString,   // the host before any redirections - entered in Website::new()
    sub_matcher: &CompactString, // matches TLDS or subdomains. If tlds the domain is stripped.
) -> bool {
    match host_name {
        Some(host) => {
            let exact_match = parent_host.eq(&host) || base_host.eq(&host);

            if base_domain.is_empty() {
                exact_match
            } else {
                exact_match || is_subdomain(host, parent_host) || is_subdomain(host, sub_matcher)
            }
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
    get_page_selectors_base(&url, subdomains, tld)
}

#[cfg(not(feature = "decentralized"))]
/// Is the resource valid?
pub fn validate_empty(content: &Option<Box<Vec<u8>>>, is_success: bool) -> bool {
    match content {
        Some(ref content) => {
            // is_success && content.starts_with(br#"<html style=\"height:100%\"><head><META NAME=\"ROBOTS\" CONTENT=\"NOINDEX, NOFOLLOW\"><meta name=\"format-detection\" content=\"telephone=no\"><meta name=\"viewport\" content=\"initial-scale=1.0\"><meta http-equiv=\"X-UA-Compatible\" content=\"IE=edge,chrome=1\"></head><body style=\"margin:0px;height:100%\"><iframe id=\"main-iframe\" src=\"/_Incapsula_"#)
            !( content.is_empty() || content.starts_with(b"<html><head></head><body></body></html>") || is_success &&
                     content.starts_with(b"<html>\r\n<head>\r\n<META NAME=\"robots\" CONTENT=\"noindex,nofollow\">\r\n<script src=\"/") &&
                      content.ends_with(b"\">\r\n</script>\r\n<body>\r\n</body></html>\r\n"))
        }
        _ => false,
    }
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
#[cfg(not(feature = "decentralized"))]
fn should_attempt_retry(error: &(dyn std::error::Error + 'static)) -> bool {
    if let Some(e) = extract_specific_error::<h2::Error>(error) {
        if e.is_go_away() && e.is_remote() && e.reason() == Some(h2::Reason::NO_ERROR) {
            return true;
        }
        if e.is_remote() && e.reason() == Some(h2::Reason::REFUSED_STREAM) {
            return true;
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
                if er.is_status() || er.is_connect() || er.is_timeout() {
                    *should_retry = !er.to_string().contains("ENOTFOUND");
                }
                if !*should_retry && should_attempt_retry(&er) {
                    *should_retry = true;
                }
                if let Some(status_code) = er.status() {
                    let retry = match status_code {
                        StatusCode::TOO_MANY_REQUESTS
                        | StatusCode::UNAUTHORIZED
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
pub fn build(url: &str, res: PageResponse) -> Page {
    let success = res.status_code.is_success() || res.status_code == StatusCode::NOT_FOUND;
    let resource_found = validate_empty(&res.content, success);

    let mut should_retry = resource_found && !success
        || res.status_code.is_server_error()
        || res.status_code == StatusCode::TOO_MANY_REQUESTS
        || res.status_code == StatusCode::FORBIDDEN
        || res.status_code == StatusCode::REQUEST_TIMEOUT;

    Page {
        html: res.content,
        #[cfg(feature = "headers")]
        headers: res.headers,
        #[cfg(feature = "remote_addr")]
        remote_addr: res.remote_addr,
        #[cfg(feature = "cookies")]
        cookies: res.cookies,
        url: url.into(),
        #[cfg(feature = "time")]
        duration: res.duration,
        final_redirect_destination: res.final_url,
        status_code: res.status_code,
        error_status: get_error_status(&mut should_retry, res.error_for_status),
        #[cfg(feature = "chrome")]
        chrome_page: None,
        #[cfg(feature = "chrome")]
        screenshot_bytes: res.screenshot_bytes,
        #[cfg(feature = "openai")]
        openai_credits_used: res.openai_credits_used,
        #[cfg(feature = "openai")]
        extra_ai_data: res.extra_ai_data,
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
        ..Default::default()
    }
}

/// Instantiate a new page without scraping it (used for testing purposes).
#[cfg(feature = "decentralized")]
pub fn build(_: &str, res: PageResponse) -> Page {
    Page {
        html: res.content,
        #[cfg(feature = "headers")]
        headers: res.headers,
        #[cfg(feature = "remote_addr")]
        remote_addr: res.remote_addr,
        #[cfg(feature = "cookies")]
        cookies: res.cookies,
        final_redirect_destination: res.final_url,
        status_code: res.status_code,
        metadata: res.metadata,
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

#[cfg(feature = "headers")]
/// Re build the cookies.
pub fn build_cookie_header_from_set_cookie(page: &Page) -> Option<reqwest::header::HeaderValue> {
    use reqwest::header::HeaderValue;
    let mut cookie_pairs = Vec::new();

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

#[cfg(not(feature = "headers"))]
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
            let parts: Vec<&str> = content_type_str.split(';').collect();
            for part in parts {
                let part = part.trim().to_lowercase();
                if let Some(stripped) = part.strip_prefix("charset=") {
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
    /// Instantiate a new page and gather the html repro of standard fetch_page_html.
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub async fn new_page(url: &str, client: &Client) -> Self {
        let page_resource: PageResponse = crate::utils::fetch_page_html_raw(url, client).await;

        build(url, page_resource)
    }

    /// New page with rewriter
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub async fn new_page_streaming<
        A: PartialEq + Eq + Sync + Send + Clone + Default + std::hash::Hash + From<String>,
    >(
        url: &str,
        client: &Client,
        only_html: bool,
        selectors: &mut RelativeSelectors,
        external_domains_caseless: &Box<HashSet<CaseInsensitiveString>>,
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

                let (encoding, adjust_charset_on_meta_tag) =
                    match get_charset_from_content_type(res.headers()) {
                        Some(h) => (h, false),
                        _ => (AsciiCompatibleEncoding::utf_8(), true),
                    };

                let target_url = res.url().as_str();

                // handle initial redirects
                if ssg_map.is_some() && url != target_url && !exact_url_match(&url, &target_url) {
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

                let original_page = match Url::parse(url) {
                    Ok(u) => Some(u),
                    _ => None,
                };

                let parent_host = &selectors.1[0];
                // the host schemes
                let parent_host_scheme = &selectors.1[1];
                let base_input_domain = &selectors.2; // the domain after redirects
                let sub_matcher = &selectors.0;
                let xml_file = target_url.ends_with(".xml");

                let base_links_settings = if r_settings.full_resources {
                    lol_html::element!("a[href],script[src],link[href]", |el| {
                        let tag_name = el.tag_name();
                        let attribute = if tag_name == "script" { "src" } else { "href" };

                        if let Some(href) = el.get_attribute(attribute) {
                            let base = if relative_directory_url(&href) || base.is_none() {
                                original_page.as_ref()
                            } else {
                                base.as_deref()
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
                                &external_domains_caseless,
                                links_pages,
                            );
                        }

                        Ok(())
                    })
                } else {
                    lol_html::element!(
                        if xml_file {
                            BASE_CSS_SELECTORS_WITH_XML
                        } else {
                            BASE_CSS_SELECTORS
                        },
                        |el| {
                            if let Some(href) = el.get_attribute("href") {
                                let base = if relative_directory_url(&href) || base.is_none() {
                                    original_page.as_ref()
                                } else {
                                    base.as_deref()
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
                                    &external_domains_caseless,
                                    links_pages,
                                );
                            }
                            Ok(())
                        }
                    )
                };

                let mut element_content_handlers =
                    Vec::with_capacity(if r_settings.ssg_build { 2 } else { 1 } + 3);

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
                        Vec::with_capacity(cap.max(MAX_PRE_ALLOCATED_HTML_PAGE_SIZE) as usize)
                    }
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

                let response_bytes = Box::new(collected_bytes);

                response.0.content = if response_bytes.is_empty() {
                    None
                } else {
                    Some(response_bytes)
                };

                if r_settings.ssg_build {
                    if let Some(ssg_map) = ssg_map {
                        if let Some(ref cell) = cell {
                            if let Some(source) = cell.get() {
                                if let Some(ref url_base) = base {
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
                                                push_link(
                                                    &base,
                                                    &href,
                                                    ssg_map,
                                                    &selectors.0,
                                                    parent_host,
                                                    parent_host_scheme,
                                                    base_input_domain,
                                                    sub_matcher,
                                                    &external_domains_caseless,
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
            Ok(res) => handle_response_bytes(res, url, only_html).await,
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

            if metadata_inner.exist() && metadata.is_some() {
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
        request_timeout: &Option<Box<Duration>>,
        track_events: &Option<crate::configuration::ChromeEventTracker>,
        referrer: Option<String>,
        max_page_bytes: Option<f64>,
    ) -> Self {
        let page_resource = crate::utils::fetch_page_html(
            &url,
            &client,
            &page,
            wait_for,
            screenshot,
            page_set,
            openai_config,
            execution_scripts,
            automation_scripts,
            viewport,
            &request_timeout,
            track_events,
            referrer,
            max_page_bytes,
        )
        .await;
        let mut p = build(url, page_resource);

        // store the chrome page to perform actions like screenshots etc.
        if cfg!(feature = "chrome_store_page") {
            p.chrome_page = Some(page.clone());
        }

        p
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
        use bytes::Buf;

        match crate::utils::fetch_page_and_headers(&url, &client).await {
            FetchPageResult::Success(headers, page_content) => {
                let links = match page_content {
                    Some(b) => match flexbuffers::Reader::get_root(b) {
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
    #[cfg(all(feature = "decentralized"))]
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all,))]
    pub async fn new_links_only(url: &str, client: &Client) -> Self {
        use crate::serde::Deserialize;

        let links = match crate::utils::fetch_page(&url, &client).await {
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
                &self,
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
                .execute(chromiumoxide::cdp::browser_protocol::page::CloseParams::default())
                .await;
        }
    }

    #[cfg(all(feature = "decentralized", feature = "chrome"))]
    /// Close the chrome page used. Useful when storing the page for subscription usage. The feature flag `chrome_store_page` is required.
    pub async fn close_page(&mut self) {}

    /// Page request fulfilled.
    pub fn is_empty(&self) -> bool {
        self.html.is_none()
    }

    /// Url getter for page.
    #[cfg(not(feature = "decentralized"))]
    pub fn get_url(&self) -> &str {
        &self.url
    }

    #[cfg(not(feature = "headers"))]
    /// Get the timeout required for rate limiting. The max duration is 30 seconds for delay respecting. Requires the feature flag `headers`.
    pub fn get_timeout(&self) -> Option<Duration> {
        None
    }

    #[cfg(feature = "headers")]
    /// Get the timeout required for rate limiting. The max duration is 30 seconds for delay respecting. Requires the feature flag `headers`.
    pub fn get_timeout(&self) -> Option<Duration> {
        if self.status_code == 429 {
            const MAX_TIMEOUT: Duration = Duration::from_secs(30);
            if let Some(ref headers) = self.headers {
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
    pub fn set_external(&mut self, external_domains_caseless: Box<HashSet<CaseInsensitiveString>>) {
        self.external_domains_caseless = external_domains_caseless;
    }

    /// Set the html directly of the page
    pub fn set_html_bytes(&mut self, html: Option<Vec<u8>>) {
        self.html = html.map(Box::new);
    }

    /// Set the url directly of the page. Useful for transforming the content and rewriting the url.
    #[cfg(not(feature = "decentralized"))]
    pub fn set_url(&mut self, url: String) {
        self.url = url;
    }

    /// Set the url directly parsed url of the page. Useful for transforming the content and rewriting the url.
    #[cfg(not(feature = "decentralized"))]
    pub fn set_url_parsed_direct(&mut self) {
        if let Ok(base) = Url::parse(&self.url) {
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
        ""
    }

    /// Html getter for bytes on the page.
    pub fn get_bytes(&self) -> Option<&Vec<u8>> {
        self.html.as_deref()
    }

    /// Html getter for bytes on the page as string.
    pub fn get_html(&self) -> String {
        self.html
            .as_ref()
            .map(|v| auto_encoder::auto_encode_bytes(v))
            .unwrap_or_default()
    }

    /// Html getter for page to u8.
    pub fn get_html_bytes_u8(&self) -> &[u8] {
        match self.html.as_deref() {
            Some(html) => html,
            _ => Default::default(),
        }
    }

    /// Modify xml - html.
    #[cfg(all(
        feature = "sitemap",
        feature = "chrome",
        not(feature = "decentralized")
    ))]
    pub(crate) fn modify_xml_html(&mut self) -> &[u8] {
        if let Some(xml) = self.html.as_deref() {
            const XML_DECL: &str = r#"<?xml version="1.0" encoding="UTF-8"?>"#;

            let stripped = if let Ok(xml_str) = std::str::from_utf8(xml) {
                xml_str
                    .strip_prefix(XML_DECL)
                    .map(|f| f.trim_start())
                    .unwrap_or(xml_str)
                    .as_bytes()
                    .to_vec()
            } else {
                xml.to_vec()
            };

            self.html = Some(Box::new(stripped));
        }

        self.html.as_deref().map(Vec::as_slice).unwrap_or_default()
    }

    /// Get the response events mapped.
    #[cfg(all(feature = "chrome", not(feature = "decentralized")))]
    pub fn get_responses(&self) -> &Option<hashbrown::HashMap<String, f64>> {
        &self.response_map
    }

    /// Get the metadata found on the page.
    pub fn get_metadata(&self) -> &Option<Box<Metadata>> {
        &self.metadata
    }

    /// Get the response events mapped.
    #[cfg(all(feature = "chrome", not(feature = "decentralized")))]
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

    /// Get the elasped duration of the page since scraped.
    #[cfg(all(feature = "time", not(feature = "decentralized")))]
    pub fn get_duration_elapsed(&self) -> Duration {
        self.duration
            .as_ref()
            .map(|t| t.elapsed())
            .unwrap_or_default()
    }

    /// Find the links as a stream using string resource validation for XML files
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub async fn links_stream_xml_links_stream_base<
        A: PartialEq + Eq + Sync + Send + Clone + Default + ToString + std::hash::Hash + From<String>,
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

        let mut buf = Vec::new();

        let parent_host = &selectors.1[0];
        let parent_host_scheme = &selectors.1[1];
        let base_input_domain = &selectors.2;
        let sub_matcher = &selectors.0;

        let mut is_link_tag = false;
        let mut links_pages = if self.page_links.is_some() {
            Some(map.clone())
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
                        let (_, local) = reader.resolve_element(e.name());

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
                        let (_, local) = reader.resolve_element(e.name());

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

        if let Some(lp) = links_pages {
            let page_links = self.page_links.get_or_insert_with(Default::default);
            page_links.extend(
                lp.into_iter()
                    .map(|item| CaseInsensitiveString::from(item.to_string())),
            );
        }
    }

    /// Find the links as a stream using string resource validation
    #[inline(always)]
    #[cfg(all(not(feature = "decentralized")))]
    pub async fn links_stream_base<
        A: PartialEq + Eq + Sync + Send + Clone + Default + ToString + std::hash::Hash + From<String>,
    >(
        &mut self,
        selectors: &RelativeSelectors,
        html: &str,
        base: &Option<Box<Url>>,
    ) -> HashSet<A> {
        let mut map: HashSet<A> = HashSet::new();
        let mut links_pages = if self.page_links.is_some() {
            Some(map.clone())
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

                element_content_handlers.push(lol_html::element!(
                    if xml_file {
                        BASE_CSS_SELECTORS_WITH_XML
                    } else {
                        BASE_CSS_SELECTORS
                    },
                    |el| {
                        if let Some(href) = el.get_attribute("href") {
                            let base = if relative_directory_url(&href) || base.is_none() {
                                original_page
                            } else {
                                base.as_deref()
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
                let chunks = html_bytes.chunks(*STREAMING_CHUNK_SIZE);

                let mut stream = tokio_stream::iter(chunks);

                while let Some(chunk) = stream.next().await {
                    if rewriter.write(chunk).is_err() {
                        wrote_error = true;
                        break;
                    }
                }

                if !wrote_error {
                    let _ = rewriter.end();
                }
            }
        }

        if let Some(lp) = links_pages {
            let page_links = self.page_links.get_or_insert_with(Default::default);
            page_links.extend(
                lp.into_iter()
                    .map(|item| CaseInsensitiveString::from(item.to_string())),
            );
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

        map
    }

    /// Find the links as a stream using string resource validation
    #[inline(always)]
    #[cfg(all(not(feature = "decentralized")))]
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub async fn links_stream_base_ssg<
        A: PartialEq + Eq + Sync + Send + Clone + Default + ToString + std::hash::Hash + From<String>,
    >(
        &mut self,
        selectors: &RelativeSelectors,
        html: &str,
        client: &Client,
        base: &Option<Box<Url>>,
    ) -> HashSet<A> {
        use auto_encoder::auto_encode_bytes;

        let mut map: HashSet<A> = HashSet::new();
        let mut map_ssg: HashSet<A> = HashSet::new();
        let mut links_pages = if self.page_links.is_some() {
            Some(map.clone())
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

                element_content_handlers.push(lol_html::element!(
                    if xml_file {
                        BASE_CSS_SELECTORS_WITH_XML
                    } else {
                        BASE_CSS_SELECTORS
                    },
                    |el| {
                        if let Some(href) = el.get_attribute("href") {
                            let base = if relative_directory_url(&href) || base.is_none() {
                                original_page
                            } else {
                                base.as_deref()
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
                            if let Some(build_path) = base.map(|b| convert_abs_path(&b, &source)) {
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
                let chunks = html_bytes.chunks(*STREAMING_CHUNK_SIZE);
                let mut wrote_error = false;

                let mut stream = tokio_stream::iter(chunks).map(Ok::<&[u8], A>);

                while let Some(chunk) = stream.next().await {
                    if let Ok(chunk) = chunk {
                        if rewriter.write(chunk).is_err() {
                            wrote_error = true;
                            break;
                        }
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
                                        base.as_deref()
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
            page_links.extend(
                lp.into_iter()
                    .map(|item| CaseInsensitiveString::from(item.to_string())),
            );
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

        map
    }

    /// Find the links as a stream using string resource validation and parsing the script for nextjs initial SSG paths.
    #[cfg(all(not(feature = "decentralized")))]
    pub async fn links_stream_ssg<
        A: PartialEq + Eq + Sync + Send + Clone + Default + ToString + std::hash::Hash + From<String>,
    >(
        &mut self,
        selectors: &RelativeSelectors,
        client: &Client,
        prior_domain: &Option<Box<Url>>,
    ) -> HashSet<A> {
        if auto_encoder::is_binary_file(self.get_html_bytes_u8()) {
            Default::default()
        } else {
            self.links_stream_base_ssg(selectors, &Box::new(self.get_html()), client, prior_domain)
                .await
        }
    }

    /// Find all href links and return them using CSS selectors.
    #[inline(always)]
    #[cfg(all(not(feature = "decentralized")))]
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
        A: PartialEq + Eq + Sync + Send + Clone + Default + ToString + std::hash::Hash + From<String>,
    >(
        &mut self,
        selectors: &RelativeSelectors,
        base: &Option<Box<Url>>,
    ) -> HashSet<A> {
        if auto_encoder::is_binary_file(self.get_html_bytes_u8()) {
            Default::default()
        } else {
            self.links_stream_base(selectors, &Box::new(self.get_html()), base)
                .await
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
        A: PartialEq + Eq + Sync + Send + Clone + Default + ToString + std::hash::Hash + From<String>,
    >(
        &mut self,
        selectors: &RelativeSelectors,
        configuration: &crate::configuration::Configuration,
        base: &Option<Box<Url>>,
        browser: &crate::features::chrome::OnceBrowser,
    ) -> (HashSet<A>, Option<f64>) {
        use auto_encoder::auto_encode_bytes;
        use chromiumoxide::error::CdpError;
        use lol_html::{doc_comments, element};
        use std::sync::atomic::{AtomicBool, Ordering};

        let mut bytes_transferred: Option<f64> = None;
        let mut map = HashSet::new();
        let mut inner_map: HashSet<A> = map.clone();
        let mut links_pages = if self.page_links.is_some() {
            Some(map.clone())
        } else {
            None
        };

        let mut metadata: Option<Box<Metadata>> = None;
        let mut meta_title: Option<_> = None;
        let mut meta_description: Option<_> = None;
        let mut meta_og_image: Option<_> = None;

        if !self.is_empty() {
            let html_resource = Box::new(self.get_html());

            if html_resource.starts_with("<?xml") {
                self.links_stream_xml_links_stream_base(selectors, &html_resource, &mut map, &base)
                    .await;
            } else {
                let (tx, rx) = tokio::sync::oneshot::channel();

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

                let rerender = AtomicBool::new(false);

                let mut static_app = false;
                let xml_file = self.get_url().ends_with(".xml");

                let mut element_content_handlers =
                    metadata_handlers(&mut meta_title, &mut meta_description, &mut meta_og_image);

                element_content_handlers.push(element!("script", |element| {
                    if !static_app {
                        if let Some(src) = element.get_attribute("src") {
                            if src.starts_with("/") {
                                if src.starts_with("/_next/static/chunks/pages/")
                                    || src.starts_with("/webpack-runtime-")
                                    || element.get_attribute("id").eq(&*GATSBY)
                                {
                                    static_app = true;
                                }

                                if let Some(ref base) = base1 {
                                    let abs = convert_abs_path(&base, &src);

                                    if let Ok(mut paths) =
                                        abs.path_segments().ok_or_else(|| "cannot be base")
                                    {
                                        while let Some(p) = paths.next() {
                                            if chromiumoxide::handler::network::ALLOWED_MATCHER
                                                .is_match(&p)
                                            {
                                                rerender.swap(true, Ordering::Relaxed);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Ok(())
                }));

                element_content_handlers.push(element!(
                    if xml_file {
                        BASE_CSS_SELECTORS_WITH_XML
                    } else {
                        BASE_CSS_SELECTORS
                    },
                    |el| {
                        if let Some(href) = el.get_attribute("href") {
                            let base = if relative_directory_url(&href) || base.is_none() {
                                original_page.as_ref()
                            } else {
                                base.as_deref()
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

                        el.remove();

                        Ok(())
                    }
                ));

                element_content_handlers.push(element!(
                    "*:not(script):not(a):not(body):not(head):not(html)",
                    |el| {
                        el.remove();
                        Ok(())
                    }
                ));

                let rewriter_settings = lol_html::Settings {
                    element_content_handlers,
                    document_content_handlers: vec![doc_comments!(|c| {
                        c.remove();
                        Ok(())
                    })],
                    adjust_charset_on_meta_tag: true,
                    ..lol_html::send::Settings::new_for_handler_types()
                };

                let (dtx, rdx) = tokio::sync::oneshot::channel();
                let output_sink = crate::utils::HtmlOutputSink::new(dtx);

                let mut rewriter =
                    lol_html::send::HtmlRewriter::new(rewriter_settings.into(), output_sink);

                let html_bytes = html_resource.as_bytes();
                let chunks = html_bytes.chunks(*STREAMING_CHUNK_SIZE);
                let mut wrote_error = false;

                let mut stream = tokio_stream::iter(chunks);

                while let Some(chunk) = stream.next().await {
                    if let Err(_) = rewriter.write(chunk) {
                        wrote_error = true;
                        break;
                    }
                }

                if !wrote_error {
                    let _ = rewriter.end();
                }

                let rewrited_bytes = if let Ok(c) = rdx.await { c } else { Vec::new() };

                let mut rerender = rerender.load(Ordering::Relaxed);

                if !rerender {
                    if let Some(_) = DOM_WATCH_METHODS.find(&rewrited_bytes) {
                        rerender = true;
                    }
                }

                if rerender {
                    if let Some(browser_controller) = browser
                        .get_or_init(|| {
                            crate::website::Website::setup_browser_base(&configuration, &base)
                        })
                        .await
                    {
                        let browser = browser_controller.browser.0.clone();
                        let browser_id = browser_controller.browser.2.clone();
                        let configuration = configuration.clone();
                        // we should re-use the html content instead with events.
                        let target_url = self.url.clone();
                        let parent_host = parent_host.clone();

                        crate::utils::spawn_task("page_render_fetch", async move {
                            if let Ok(new_page) = crate::features::chrome::attempt_navigation(
                                "about:blank",
                                &browser,
                                &configuration.request_timeout,
                                &browser_id,
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

                                let page_resource = crate::utils::fetch_page_html_chrome_base(
                                    &html_resource,
                                    &new_page,
                                    true,
                                    true,
                                    &configuration.wait_for,
                                    &configuration.screenshot,
                                    false,
                                    &configuration.openai_config,
                                    Some(&target_url),
                                    &configuration.execution_scripts,
                                    &configuration.automation_scripts,
                                    &configuration.viewport,
                                    &configuration.request_timeout,
                                    &configuration.track_events,
                                    configuration.referer.clone(),
                                    configuration.max_page_bytes,
                                )
                                .await;

                                if let Some(h) = intercept_handle {
                                    let abort_handle = h.abort_handle();
                                    if let Err(elasped) = tokio::time::timeout(
                                        tokio::time::Duration::from_secs(15),
                                        h,
                                    )
                                    .await
                                    {
                                        log::warn!("Handler timeout exceeded {elasped}");
                                        abort_handle.abort();
                                    }
                                }

                                match page_resource {
                                    Ok(resource) => {
                                        if let Err(_) = tx.send(resource) {
                                            log::info!("the receiver dropped - {target_url}");
                                        }
                                    }
                                    Err(e) => {
                                        let mut default_response: PageResponse = Default::default();

                                        default_response.final_url = Some(target_url.clone());

                                        match e {
                                            CdpError::NotFound => {
                                                default_response.status_code =
                                                    StatusCode::NOT_FOUND;
                                            }
                                            CdpError::NoResponse => {
                                                default_response.status_code =
                                                    StatusCode::GATEWAY_TIMEOUT;
                                            }
                                            CdpError::LaunchTimeout(_) => {
                                                default_response.status_code =
                                                    StatusCode::REQUEST_TIMEOUT;
                                            }
                                            _ => (),
                                        }

                                        if let Err(_) = tx.send(default_response) {
                                            log::info!("the receiver dropped - {target_url}");
                                        }
                                    }
                                }
                            }
                        });
                    }

                    match rx.await {
                        Ok(v) => {
                            let extended_map = self
                                .links_stream_base::<A>(
                                    selectors,
                                    &match v.content {
                                        Some(h) => auto_encode_bytes(&h),
                                        _ => Default::default(),
                                    },
                                    &base1.as_deref().cloned().map(Box::new),
                                )
                                .await;

                            bytes_transferred = v.bytes_transferred;

                            map.extend(extended_map)
                        }
                        Err(e) => {
                            crate::utils::log("receiver error", e.to_string());
                        }
                    };
                }
            }

            map.extend(inner_map);
        }

        if let Some(lp) = links_pages {
            let page_links = self.page_links.get_or_insert_with(Default::default);
            page_links.extend(
                lp.into_iter()
                    .map(|item| CaseInsensitiveString::from(item.to_string())),
            );
            page_links.extend(
                map.iter()
                    .map(|item| CaseInsensitiveString::from(item.to_string())),
            );
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
        A: PartialEq + Eq + Sync + Send + Clone + Default + ToString + std::hash::Hash + From<String>,
    >(
        &mut self,
        selectors: &RelativeSelectors,
        configuration: &crate::configuration::Configuration,
        base: &Option<Box<Url>>,
        browser: &crate::features::chrome::OnceBrowser,
    ) -> (HashSet<A>, Option<f64>) {
        use auto_encoder::auto_encode_bytes;
        use chromiumoxide::error::CdpError;
        use lol_html::{doc_comments, element};
        use std::sync::atomic::{AtomicBool, Ordering};

        let mut bytes_transferred: Option<f64> = None;
        let mut map = HashSet::new();
        let mut inner_map: HashSet<A> = map.clone();
        let mut links_pages = if self.page_links.is_some() {
            Some(map.clone())
        } else {
            None
        };

        let mut metadata: Option<Box<Metadata>> = None;
        let mut meta_title: Option<_> = None;
        let mut meta_description: Option<_> = None;
        let mut meta_og_image: Option<_> = None;

        if !self.is_empty() {
            let html_resource = Box::new(self.get_html());

            if html_resource.starts_with("<?xml") {
                self.links_stream_xml_links_stream_base(selectors, &html_resource, &mut map, &base)
                    .await;
            } else {
                let (tx, rx) = tokio::sync::oneshot::channel();

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

                let rerender = AtomicBool::new(false);

                let mut static_app = false;

                let mut element_content_handlers = vec![
                    element!("script", |element| {
                        if !static_app {
                            if let Some(src) = element.get_attribute("src") {
                                if src.starts_with("/") {
                                    if src.starts_with("/_next/static/chunks/pages/")
                                        || src.starts_with("/webpack-runtime-")
                                        || element.get_attribute("id").eq(&*GATSBY)
                                    {
                                        static_app = true;
                                    }

                                    if let Some(ref base) = base1 {
                                        let abs = convert_abs_path(&base, &src);

                                        if let Ok(mut paths) =
                                            abs.path_segments().ok_or_else(|| "cannot be base")
                                        {
                                            while let Some(p) = paths.next() {
                                                if chromiumoxide::handler::network::ALLOWED_MATCHER
                                                    .is_match(&p)
                                                {
                                                    rerender.swap(true, Ordering::Relaxed);
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        Ok(())
                    }),
                    element!("a[href],script[src],link[href]", |el| {
                        let attribute = if el.tag_name() == "script" {
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

                        el.remove();

                        Ok(())
                    }),
                    element!("*:not(script):not(a):not(body):not(head):not(html)", |el| {
                        el.remove();
                        Ok(())
                    }),
                ];

                element_content_handlers.extend(&metadata_handlers(
                    &mut meta_title,
                    &mut meta_description,
                    &mut meta_og_image,
                ));

                let rewriter_settings = lol_html::Settings {
                    element_content_handlers,
                    document_content_handlers: vec![doc_comments!(|c| {
                        c.remove();
                        Ok(())
                    })],
                    adjust_charset_on_meta_tag: true,
                    ..lol_html::send::Settings::new_for_handler_types()
                };

                let (dtx, rdx) = tokio::sync::oneshot::channel();
                let output_sink = crate::utils::HtmlOutputSink::new(dtx);

                let mut rewriter =
                    lol_html::send::HtmlRewriter::new(rewriter_settings.into(), output_sink);

                let html_bytes = html_resource.as_bytes();
                let chunks = html_bytes.chunks(*STREAMING_CHUNK_SIZE);
                let mut wrote_error = false;

                let mut stream = tokio_stream::iter(chunks);

                while let Some(chunk) = stream.next().await {
                    if let Err(_) = rewriter.write(chunk) {
                        wrote_error = true;
                        break;
                    }
                }

                if !wrote_error {
                    let _ = rewriter.end();
                }

                let rewrited_bytes = if let Ok(c) = rdx.await { c } else { Vec::new() };

                let mut rerender = rerender.load(Ordering::Relaxed);

                if !rerender {
                    if let Some(_) = DOM_WATCH_METHODS.find(&rewrited_bytes) {
                        rerender = true;
                    }
                }

                if rerender {
                    if let Some(browser_controller) = browser
                        .get_or_init(|| {
                            crate::website::Website::setup_browser_base(&configuration, &base)
                        })
                        .await
                    {
                        let browser = browser_controller.browser.0.clone();
                        let browser_id = browser_controller.browser.2.clone();
                        let configuration = configuration.clone();
                        // we should re-use the html content instead with events.
                        let target_url = self.url.clone();
                        // let context_id = context_id.clone();
                        let parent_host = parent_host.clone();

                        crate::utils::spawn_task("page_render_fetch", async move {
                            if let Ok(new_page) = crate::features::chrome::attempt_navigation(
                                "about:blank",
                                &browser,
                                &configuration.request_timeout,
                                &browser_id,
                                &configuration.viewport,
                            )
                            .await
                            {
                                let (_, intercept_handle) = tokio::join!(
                                    crate::features::chrome::setup_chrome_events(
                                        &new_page,
                                        &configuration,
                                    ),
                                    crate::features::chrome::setup_chrome_interception_base(
                                        &new_page,
                                        configuration.chrome_intercept.enabled,
                                        &configuration.auth_challenge_response,
                                        configuration.chrome_intercept.block_visuals,
                                        &parent_host,
                                    )
                                );

                                let page_resource = crate::utils::fetch_page_html_chrome_base(
                                    &html_resource,
                                    &new_page,
                                    true,
                                    true,
                                    &configuration.wait_for,
                                    &configuration.screenshot,
                                    false,
                                    &configuration.openai_config,
                                    Some(&target_url),
                                    &configuration.execution_scripts,
                                    &configuration.automation_scripts,
                                    &configuration.viewport,
                                    &configuration.request_timeout,
                                    &configuration.track_events,
                                    configuration.referer.clone(),
                                    configuration.max_page_bytes,
                                )
                                .await;

                                if let Some(h) = intercept_handle {
                                    let abort_handle = h.abort_handle();
                                    if let Err(elasped) = tokio::time::timeout(
                                        tokio::time::Duration::from_secs(15),
                                        h,
                                    )
                                    .await
                                    {
                                        log::warn!("Handler timeout exceeded {elasped}");
                                        abort_handle.abort();
                                    }
                                }

                                match page_resource {
                                    Ok(resource) => {
                                        if let Err(_) = tx.send(resource) {
                                            log::info!("the receiver dropped - {target_url}");
                                        }
                                    }
                                    Err(e) => {
                                        let mut default_response: PageResponse = Default::default();

                                        default_response.final_url = Some(target_url.clone());

                                        match e {
                                            CdpError::NotFound => {
                                                default_response.status_code =
                                                    StatusCode::NOT_FOUND;
                                            }
                                            CdpError::NoResponse => {
                                                default_response.status_code =
                                                    StatusCode::GATEWAY_TIMEOUT;
                                            }
                                            CdpError::LaunchTimeout(_) => {
                                                default_response.status_code =
                                                    StatusCode::REQUEST_TIMEOUT;
                                            }
                                            _ => (),
                                        }

                                        if let Err(_) = tx.send(default_response) {
                                            log::info!("the receiver dropped - {target_url}");
                                        }
                                    }
                                }
                            }
                        });
                    }

                    match rx.await {
                        Ok(v) => {
                            let extended_map = self
                                .links_stream_base::<A>(
                                    selectors,
                                    &match v.content {
                                        Some(h) => auto_encode_bytes(&h),
                                        _ => Default::default(),
                                    },
                                    &base.as_deref().cloned().map(Box::new),
                                )
                                .await;

                            bytes_transferred = v.bytes_transferred;
                            map.extend(extended_map)
                        }
                        Err(e) => {
                            crate::utils::log("receiver error", e.to_string());
                        }
                    };
                }
            }

            map.extend(inner_map);
        }

        if let Some(lp) = links_pages {
            let page_links = self.page_links.get_or_insert_with(Default::default);
            page_links.extend(
                lp.into_iter()
                    .map(|item| CaseInsensitiveString::from(item.to_string())),
            );
            page_links.extend(
                map.iter()
                    .map(|item| CaseInsensitiveString::from(item.to_string())),
            );
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

        (map, bytes_transferred)
    }

    /// Find the links as a stream using string resource validation
    #[inline(always)]
    #[cfg(all(not(feature = "decentralized")))]
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all,))]
    pub async fn links_stream_full_resource<
        A: PartialEq + Eq + Sync + Send + Clone + Default + ToString + std::hash::Hash + From<String>,
    >(
        &mut self,
        selectors: &RelativeSelectors,
        base: &Option<Box<Url>>,
    ) -> HashSet<A> {
        let mut map = HashSet::new();
        let mut links_pages = if self.page_links.is_some() {
            Some(map.clone())
        } else {
            None
        };

        let mut metadata: Option<Box<Metadata>> = None;
        let mut meta_title: Option<_> = None;
        let mut meta_description: Option<_> = None;
        let mut meta_og_image: Option<_> = None;

        if !self.is_empty() {
            let html = Box::new(self.get_html());

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

                let base = base.as_deref();

                // original domain to match local pages.
                let original_page = {
                    self.set_url_parsed_direct_empty();
                    self.get_url_parsed_ref().as_ref().cloned()
                };

                let external_domains_caseless = self.external_domains_caseless.clone();

                let base_links_settings =
                    lol_html::element!("a[href],script[src],link[href]", |el| {
                        let attribute = if el.tag_name() == "script" {
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
                    });

                let mut element_content_handlers =
                    metadata_handlers(&mut meta_title, &mut meta_description, &mut meta_og_image);

                element_content_handlers.push(base_links_settings);

                let settings = lol_html::send::Settings {
                    element_content_handlers,
                    adjust_charset_on_meta_tag: true,
                    ..lol_html::send::Settings::new_for_handler_types()
                };

                let mut rewriter = lol_html::send::HtmlRewriter::new(settings, |_c: &[u8]| {});

                let html_bytes = html.as_bytes();
                let chunks = html_bytes.chunks(*STREAMING_CHUNK_SIZE);
                let mut wrote_error = false;

                let mut stream = tokio_stream::iter(chunks);

                while let Some(chunk) = stream.next().await {
                    if rewriter.write(chunk).is_err() {
                        wrote_error = true;
                        break;
                    }
                }

                if !wrote_error {
                    let _ = rewriter.end();
                }
            }
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

        map
    }

    /// Find the links as a stream using string resource validation
    #[inline(always)]
    #[cfg(all(not(feature = "decentralized"), feature = "full_resources"))]
    pub async fn links_stream<
        A: PartialEq + Eq + Sync + Send + Clone + Default + ToString + std::hash::Hash + From<String>,
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
        A: PartialEq + Eq + Sync + Send + Clone + Default + ToString + std::hash::Hash + From<String>,
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
    #[cfg(all(not(feature = "decentralized")))]
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
                )
                .await
            }
        }
    }

    /// Find all href links and return them using CSS selectors.
    #[cfg(feature = "decentralized")]
    #[inline(always)]
    pub async fn links(&self, _: &RelativeSelectors) -> HashSet<CaseInsensitiveString> {
        self.links.to_owned()
    }
}

/// Get the content with proper encoding. Pass in a proper encoding label like SHIFT_JIS.
pub fn encode_bytes(html: &Vec<u8>, label: &str) -> String {
    auto_encoder::encode_bytes(html, label)
}

/// Get the content with proper encoding. Pass in a proper encoding label like SHIFT_JIS.
#[cfg(feature = "encoding")]
pub fn get_html_encoded(html: &Option<Box<Vec<u8>>>, label: &str) -> String {
    match html.as_ref() {
        Some(html) => encode_bytes(html, label),
        _ => Default::default(),
    }
}

#[cfg(not(feature = "encoding"))]
/// Get the content with proper encoding. Pass in a proper encoding label like SHIFT_JIS.
pub fn get_html_encoded(html: &Option<Box<Vec<u8>>>, _label: &str) -> String {
    match html {
        Some(b) => String::from_utf8_lossy(b).to_string(),
        _ => Default::default(),
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
    use reqwest::header::HeaderName;
    use reqwest::header::HeaderValue;

    let client = Client::builder()
        .user_agent(TEST_AGENT_NAME)
        .build()
        .unwrap();

    let link_result = "https://choosealicense.com/";
    let page: Page = Page::new_page(link_result, &client).await;

    let headers = page.headers.expect("There should be some headers!");

    assert_eq!(
        headers
            .get(HeaderName::from_static("server"))
            .expect("There should be a server header value!"),
        HeaderValue::from_static("GitHub.com")
    );

    assert_eq!(
        headers
            .get(HeaderName::from_static("content-type"))
            .expect("There should be a content-type value!"),
        HeaderValue::from_static("text/html; charset=utf-8")
    );
}

#[cfg(all(
    not(feature = "decentralized"),
    not(feature = "chrome"),
    not(feature = "cache_request")
))]
#[tokio::test]
async fn parse_links() {
    let client = Client::builder()
        .user_agent(TEST_AGENT_NAME)
        .build()
        .unwrap();

    let link_result = "https://choosealicense.com/";
    let mut page = Page::new(link_result, &client).await;
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

#[cfg(all(
    not(feature = "decentralized"),
    not(feature = "chrome"),
    not(feature = "cache_request")
))]
#[tokio::test]
async fn test_status_code() {
    let client = Client::builder()
        .user_agent(TEST_AGENT_NAME)
        .build()
        .unwrap();
    let link_result = "https://choosealicense.com/does-not-exist";
    let page: Page = Page::new(link_result, &client).await;

    assert_eq!(page.status_code.as_u16(), 404);
}

#[cfg(all(feature = "time", not(feature = "decentralized")))]
#[tokio::test]
async fn test_duration() {
    let client = Client::default();
    let link_result = "https://choosealicense.com/";
    let page: Page = Page::new_page(&link_result, &client).await;
    let duration_elasped = page.get_duration_elasped().as_millis();

    assert!(
        duration_elasped < 6000,
        "Duration took longer than expected {}.",
        duration_elasped,
    );
}
