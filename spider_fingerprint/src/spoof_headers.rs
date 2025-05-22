use http::header::{
    HeaderValue, ACCEPT, ACCEPT_ENCODING, ACCEPT_LANGUAGE, CACHE_CONTROL, CONNECTION, HOST, PRAGMA,
    REFERER, UPGRADE_INSECURE_REQUESTS, USER_AGENT,
};
use http::{HeaderMap, HeaderName};
use rand::{rng, Rng};

use crate::configs::AgentOs;
use crate::get_agent_os;

lazy_static::lazy_static! {
    /// The brand version of google chrome. Use the env var 'NOT_A_BRAND_VERSION'.
    static ref NOT_A_BRAND_VERSION: String = {
       crate::CHROME_NOT_A_BRAND_VERSION.split('.').next().unwrap_or("99".into()).into()
    };
    /// The chrome platform linux version of google chrome. Use the env var 'NOT_A_BRAND_VERSION'.
    static ref CHROME_PLATFORM_LINUX_VERSION: String = {
        std::env::var("CHROME_PLATFORM_LINUX_VERSION").unwrap_or_else(|_| {
            "\"6.12.10\"".to_string()
        })
    };
    /// The chrome platform version of google chrome. Use the env var 'NOT_A_BRAND_VERSION'.
    static ref CHROME_PLATFORM_VERSION: String = {
        std::env::var("CHROME_PLATFORM_VERSION").unwrap_or_else(|_| {
            #[cfg(target_os = "linux")]
            {
                CHROME_PLATFORM_LINUX_VERSION.to_string()
            }

            #[cfg(not(target_os = "linux"))]
            {
                "\"14.6.1\"".to_string()
            }
        })
    };
}

fn parse_user_agent_to_ch_ua(ua: &str, dec: bool, linux: bool) -> String {
    let mut parts = Vec::with_capacity(3);

    if let Some(version) = ua
        .split("Chrome/")
        .nth(1)
        .and_then(|s| s.split_whitespace().next())
    {
        if let Some(major_version) = version.split('.').next() {
            parts.push(format!(
                r#""Chromium";v="{}{}""#,
                major_version,
                if dec {
                    if linux {
                        ".0.0.0"
                    } else {
                        ".0.0"
                    }
                } else {
                    ""
                }
            ));
            parts.push(format!(
                r#""Not:A-Brand";v="{}{}""#,
                *NOT_A_BRAND_VERSION,
                if dec {
                    if linux {
                        ".0.0.0"
                    } else {
                        ".0.0"
                    }
                } else {
                    ""
                }
            ));
            parts.push(format!(r#""Google Chrome";v="{}""#, major_version));
        }
    }

    parts.join(", ").trim_end().into()
}

#[cfg(target_os = "macos")]
/// sec ch user-agent platform
fn get_sec_ch_ua_platform() -> &'static str {
    "\"macOS\""
}

#[cfg(target_os = "windows")]
/// sec ch user-agent platform
fn get_sec_ch_ua_platform() -> &'static str {
    "\"Windows\""
}

#[cfg(target_os = "linux")]
/// sec ch user-agent platform
fn get_sec_ch_ua_platform() -> &'static str {
    "\"Linux\""
}

#[cfg(target_os = "android")]
/// sec ch user-agent platform
fn get_sec_ch_ua_platform() -> &'static str {
    "\"Android\""
}

#[cfg(target_os = "ios")]
/// sec ch user-agent platform
fn get_sec_ch_ua_platform() -> &'static str {
    "\"iOS\""
}

#[cfg(target_arch = "x86")]
/// sec-ch-ua-arch: system architecture (32-bit x86)
fn get_sec_ch_ua_arch() -> &'static str {
    "\"x86\""
}

#[cfg(target_arch = "x86_64")]
/// sec-ch-ua-arch: system architecture (64-bit x86_64)
fn get_sec_ch_ua_arch() -> &'static str {
    "\"x86_64\""
}

#[cfg(any(target_arch = "aarch64", target_arch = "arm"))]
/// sec-ch-ua-arch: general CPU family for Chrome
fn get_sec_ch_ua_arch() -> &'static str {
    "\"arm\""
}

#[cfg(not(any(
    target_arch = "x86",
    target_arch = "x86_64",
    target_arch = "aarch64",
    target_arch = "arm"
)))]
/// sec-ch-ua-arch: unknown or unsupported architecture
fn get_sec_ch_ua_arch() -> &'static str {
    "\"unknown\""
}

fn get_sec_ch_ua_bitness() -> &'static str {
    #[cfg(target_pointer_width = "64")]
    {
        "64"
    }

    #[cfg(target_pointer_width = "32")]
    {
        "32"
    }
}
fn get_accept_language() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "en-US,en;q=0.9"
    }

    #[cfg(target_os = "macos")]
    {
        "en-US,en;q=0.9"
    }

    #[cfg(target_os = "linux")]
    {
        "en-US,en;q=0.9"
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        "en"
    }
}

/// The kind of browser.
#[derive(PartialEq, Eq)]
enum BrowserKind {
    /// Chrome
    Chrome,
    /// Firefox
    Firefox,
    /// Safari
    Safari,
    /// Edge
    Edge,
    /// Other
    Other,
}

#[derive(Clone)]
/// Header key value.
pub enum HeaderKey {
    /// The name of the header.
    Name(HeaderName),
    /// The static str.
    Str(&'static str),
}

impl HeaderKey {
    /// Return HeaderName if valid
    pub fn as_header_name(&self) -> HeaderName {
        match self {
            HeaderKey::Name(h) => h.clone(),
            HeaderKey::Str(s) => HeaderName::from_bytes(s.as_bytes()).expect("valid header"),
        }
    }
}

/// Add a spoofed Referer header from Google or a realistic domain,
/// or sometimes intentionally omit it entirely for privacy realism.
pub fn maybe_insert_spoofed_referer(
    domain_parsed: Option<&url::Url>,
    rng: &mut rand::rngs::ThreadRng,
) -> Option<HeaderValue> {
    use crate::spoof_refererer::{spoof_referrer, spoof_referrer_google};
    let chance: f64 = rng.random();

    if chance < 0.50 {
        if rng.random_bool(0.25) {
            if let Some(parsed_domain) = domain_parsed {
                // prioritize spoofed Google referer if available
                if let Some(google_ref) = spoof_referrer_google(parsed_domain) {
                    return HeaderValue::from_str(&google_ref).ok();
                }
            }
        }

        if rng.random_bool(0.50) {
            HeaderValue::from_static("https://google.com/").into()
        } else {
            HeaderValue::from_static(spoof_referrer()).into()
        }
    } else {
        None
    }
}

/// Add a spoofed Referer header from Google or a realistic domain,
/// or sometimes intentionally omit it entirely for privacy realism.
pub fn maybe_insert_spoofed_referer_simple(rng: &mut rand::rngs::ThreadRng) -> Option<HeaderValue> {
    use crate::spoof_refererer::spoof_referrer;
    let chance: f64 = rng.random();

    if chance < 0.50 {
        if rng.random_bool(0.35) {
            HeaderValue::from_static("https://google.com/").into()
        } else {
            HeaderValue::from_static(spoof_referrer()).into()
        }
    } else {
        None
    }
}

/// The extent of emulation to build.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum HeaderDetailLevel {
    /// Include only basic headers.
    Light,
    /// Include a moderate set of headers.
    Mild,
    /// Include a moderate set of headers without the referrer header.
    MildNoRef,
    /// Include the full, extensive set of headers.
    Extensive,
    #[default]
    /// Include the full, extensive set of headers without the referrer header.
    ExtensiveNoRef,
    /// Return nothing.
    Empty,
}

/// Emulate real HTTP chrome headers.
pub fn emulate_headers(
    user_agent: &str,
    header_map: &std::option::Option<&HeaderMap>,
    hostname: &Option<&str>,
    chrome: bool, // incase HeaderMap allows case handling and ignoring the referer handling.
    viewport: &Option<crate::spoof_viewport::Viewport>,
    domain_parsed: &Option<Box<url::Url>>,
    detail_level: &Option<HeaderDetailLevel>,
) -> HeaderMap {
    let empty = matches!(detail_level, Some(HeaderDetailLevel::Empty));

    if empty {
        return Default::default();
    }

    let browser = if user_agent.contains("Chrome/") {
        BrowserKind::Chrome
    } else if user_agent.contains("Firefox/") {
        BrowserKind::Firefox
    } else if user_agent.contains("Safari/") {
        BrowserKind::Safari
    } else if user_agent.contains("Edge/") {
        BrowserKind::Edge
    } else {
        BrowserKind::Other
    };

    let cap = if browser == BrowserKind::Chrome {
        31
    } else {
        10
    };

    let mild = matches!(
        detail_level,
        Some(HeaderDetailLevel::Mild)
            | Some(HeaderDetailLevel::MildNoRef)
            | Some(HeaderDetailLevel::Extensive)
            | None
    );
    let extensive = matches!(
        detail_level,
        Some(HeaderDetailLevel::Extensive) | Some(HeaderDetailLevel::ExtensiveNoRef) | None
    );

    let mut headers = HeaderMap::with_capacity(cap);
    let binding = HeaderMap::with_capacity(cap);
    let mut map_exist = false;

    let header_map = match header_map {
        Some(h) => {
            let m = h;
            map_exist = !m.is_empty();
            m
        }
        _ => &binding,
    };

    let add_ref = !header_map.contains_key(REFERER)
        && !matches!(
            detail_level,
            Some(HeaderDetailLevel::ExtensiveNoRef) | Some(HeaderDetailLevel::MildNoRef)
        );

    macro_rules! insert_or_default {
        ($key:expr, $default:expr) => {
            if let Some(val) = header_map.get($key) {
                headers.insert($key, val.clone());
            } else {
                headers.insert($key, $default);
            }
        };
        ($name:literal, $default:expr) => {
            if let Some(val) = header_map.get($name) {
                headers.insert($name, val.clone());
            } else {
                headers.insert($name, $default);
            }
        };
    }

    match browser {
        BrowserKind::Chrome => {
            let agent_os = get_agent_os(user_agent);

            let linux_agent = agent_os == AgentOs::Linux;

            let mut thread_rng = rng();

            // if not a chrome request we should stick to the headers from request to prevent duplications.
            let (
                host_header,
                connection_header,
                useragent_header,
                accept_header,
                refererer_header,
                upgrade_request_header,
                cache_control_header,
                pragma_header,
                accept_encoding,
                accept_language,
            ) = if !chrome {
                (
                    HeaderKey::Name(HOST),
                    HeaderKey::Name(CONNECTION),
                    HeaderKey::Name(USER_AGENT),
                    HeaderKey::Name(ACCEPT),
                    HeaderKey::Name(REFERER),
                    HeaderKey::Name(UPGRADE_INSECURE_REQUESTS),
                    HeaderKey::Name(CACHE_CONTROL),
                    HeaderKey::Name(PRAGMA),
                    HeaderKey::Name(ACCEPT_ENCODING),
                    HeaderKey::Name(ACCEPT_LANGUAGE),
                )
            } else {
                (
                    HeaderKey::Str("Host"),
                    HeaderKey::Str("Connection"),
                    HeaderKey::Str("User-Agent"),
                    HeaderKey::Str("Accept"),
                    HeaderKey::Str("Referer"),
                    HeaderKey::Str("Upgrade-Insecure-Requests"),
                    HeaderKey::Str("Cache-Control"),
                    HeaderKey::Str("Pragma"),
                    HeaderKey::Str("Accept-Encoding"),
                    HeaderKey::Str("Accept-Language"),
                )
            };

            let memory_levels = [0.25, 0.5, 1.0, 2.0, 4.0, 8.0];
            let device_memory = memory_levels[thread_rng.random_range(0..memory_levels.len())];
            let device_memory_str = format!("{}", device_memory);
            let downlink_mbps = thread_rng.random_range(0.1..=10.0);
            let downlink_str = format!("{:.1}", downlink_mbps);
            let is_mobile = if let Some(vp) = viewport {
                vp.emulating_mobile
            } else {
                false
            };

            // 1. Host
            // Note: do not set the host header for the client in case of redirects to prevent mismatches.
            if chrome {
                if let Some(host) = &hostname {
                    if !host.is_empty() {
                        if let Ok(host_value) = HeaderValue::from_str(host) {
                            insert_or_default!(&host_header.as_header_name(), host_value);
                        }
                    }
                }
            }

            // 2. Connection
            insert_or_default!(
                &connection_header.as_header_name(),
                HeaderValue::from_static("keep-alive")
            );

            // 3. sec-ch-ua group
            if let Ok(sec_ch_ua) =
                HeaderValue::from_str(&parse_user_agent_to_ch_ua(user_agent, false, linux_agent))
            {
                insert_or_default!("sec-ch-ua", sec_ch_ua);
            }
            insert_or_default!(
                "sec-ch-ua-mobile",
                HeaderValue::from_static(if is_mobile { "?1" } else { "?0" })
            );
            insert_or_default!(
                "sec-ch-ua-platform",
                HeaderValue::from_static(if linux_agent {
                    "\"Linux\""
                } else if agent_os == AgentOs::Mac {
                    "\"macOS\""
                } else if agent_os == AgentOs::Windows {
                    "\"Windows\""
                } else if agent_os == AgentOs::Android {
                    "\"Android\""
                } else {
                    get_sec_ch_ua_platform()
                })
            );
            // 4. Upgrade-Insecure-Requests
            insert_or_default!(
                &upgrade_request_header.as_header_name(),
                HeaderValue::from_static("1")
            );
            // 5. User-Agent
            if let Ok(ua) = HeaderValue::from_str(user_agent) {
                insert_or_default!(&useragent_header.as_header_name(), ua);
            }
            // 6. Accept
            insert_or_default!(
               &accept_header.as_header_name(),
                HeaderValue::from_static("text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.7")
            );
            // 7. Sec-Fetch group
            insert_or_default!("Sec-Fetch-Site", HeaderValue::from_static("none"));
            insert_or_default!("Sec-Fetch-Mode", HeaderValue::from_static("navigate"));
            insert_or_default!("Sec-Fetch-User", HeaderValue::from_static("?1"));
            insert_or_default!("Sec-Fetch-Dest", HeaderValue::from_static("document"));

            // 8. Referer (if spoofing enabled and missing)
            // If this is coming from chrome we do not want to inject the ref for all resources.
            // We want to set the referer with our page navigation or on interception remove the referer headers.
            // For now it is better for chrome not to use the referer unless you can re-order the headers with a http proxy.
            if add_ref {
                if chrome {
                    if let Some(ref_header) = maybe_insert_spoofed_referer_simple(&mut thread_rng) {
                        insert_or_default!(&refererer_header.as_header_name(), ref_header);
                    }
                } else {
                    if let Some(ref_header) =
                        maybe_insert_spoofed_referer(domain_parsed.as_deref(), &mut thread_rng)
                    {
                        insert_or_default!(&refererer_header.as_header_name(), ref_header);
                    }
                }
            }

            // 9. Accept-Encoding and Accept-Language
            insert_or_default!(
                &accept_encoding.as_header_name(),
                HeaderValue::from_static("gzip, deflate, br, zstd")
            );
            insert_or_default!(
                &accept_language.as_header_name(),
                HeaderValue::from_static(get_accept_language())
            );
            insert_or_default!(
                &pragma_header.as_header_name(),
                HeaderValue::from_static("no-cache")
            );

            if extensive || linux_agent {
                if let Ok(device_memory_str) = HeaderValue::from_str(&device_memory_str) {
                    insert_or_default!("Device-Memory", device_memory_str);
                }
            }

            // 10. Optional behavior/diagnostic headers
            insert_or_default!(
                &cache_control_header.as_header_name(),
                HeaderValue::from_static("max-age=0")
            );

            if extensive || linux_agent {
                insert_or_default!("Dpr", HeaderValue::from_static("2"));
                if let Some(vp) = viewport {
                    let width = if vp.width > 0 {
                        format!("{}", vp.width)
                    } else {
                        format!(
                            "{}",
                            crate::spoof_viewport::randomize_viewport_rng(
                                &crate::spoof_viewport::DeviceType::Desktop,
                                &mut thread_rng
                            )
                            .width
                        )
                    };

                    if let Ok(width) = HeaderValue::from_str(&width) {
                        insert_or_default!("Viewport-Width", width);
                    }
                }
            }

            insert_or_default!("Priority", HeaderValue::from_static("u=0, i"));

            if mild {
                insert_or_default!("Ect", HeaderValue::from_static("4g"));
                insert_or_default!("Rtt", HeaderValue::from_static("50"));
                if let Ok(dl) = HeaderValue::from_str(&downlink_str) {
                    insert_or_default!("Downlink", dl);
                }
            }

            if extensive || linux_agent {
                // 11. Extra client hints (real Chrome includes some of these)
                if let Ok(ua_full_list) =
                    HeaderValue::from_str(&parse_user_agent_to_ch_ua(user_agent, true, linux_agent))
                {
                    insert_or_default!("sec-ch-ua-full-version-list", ua_full_list);
                }
                if let Ok(sec_ch_platform) = HeaderValue::from_str(if linux_agent {
                    &CHROME_PLATFORM_LINUX_VERSION
                } else {
                    &CHROME_PLATFORM_VERSION
                }) {
                    insert_or_default!("sec-ch-ua-platform-version", sec_ch_platform);
                }
            }

            if mild || linux_agent {
                insert_or_default!("sec-ch-ua-model", HeaderValue::from_static("\"\""));
                insert_or_default!(
                    "sec-ch-ua-arc",
                    HeaderValue::from_static(if linux_agent {
                        "x86_64"
                    } else {
                        get_sec_ch_ua_arch()
                    })
                );
                insert_or_default!(
                    "sec-ch-ua-bitness",
                    HeaderValue::from_static(get_sec_ch_ua_bitness())
                );
                // TODO: parse the user-agent for mobile or desktop
                insert_or_default!(
                    "sec-ch-ua-form-factors",
                    HeaderValue::from_static(if is_mobile {
                        r#""Mobile""#
                    } else {
                        r#""Desktop""#
                    })
                );
            }

            if extensive || linux_agent {
                insert_or_default!("sec-ch-ua-wow64", HeaderValue::from_static("?0"));
                insert_or_default!(
                    "sec-ch-prefers-reduced-motion",
                    HeaderValue::from_static(if thread_rng.random() {
                        "no-preference"
                    } else {
                        "reduced"
                    })
                );
                insert_or_default!(
                    "sec-ch-prefers-color-scheme",
                    HeaderValue::from_static(if thread_rng.random() { "light" } else { "dark" })
                );
            }
        }
        BrowserKind::Firefox => {
            if let Ok(ua) = HeaderValue::from_str(user_agent) {
                insert_or_default!(USER_AGENT, ua);
            }

            insert_or_default!(
                ACCEPT,
                HeaderValue::from_static(
                    "text/html,application/xhtml+xml,application/xml;q=0.9,image/webp,*/*;q=0.8"
                )
            );

            if add_ref {
                if let Some(ref_header) =
                    maybe_insert_spoofed_referer(domain_parsed.as_deref(), &mut rand::rng())
                {
                    insert_or_default!(REFERER, ref_header);
                }
            }

            insert_or_default!(ACCEPT_LANGUAGE, HeaderValue::from_static("en-US,en;q=0.5"));
            insert_or_default!(
                ACCEPT_ENCODING,
                HeaderValue::from_static("gzip, deflate, br, zstd")
            );
            insert_or_default!(CONNECTION, HeaderValue::from_static("keep-alive"));
            insert_or_default!(UPGRADE_INSECURE_REQUESTS, HeaderValue::from_static("1"));
            insert_or_default!(CACHE_CONTROL, HeaderValue::from_static("max-age=0"));
            insert_or_default!("Sec-Fetch-Dest", HeaderValue::from_static("document"));
            insert_or_default!("Sec-Fetch-Mode", HeaderValue::from_static("navigate"));
            insert_or_default!("Sec-Fetch-Site", HeaderValue::from_static("none"));
            insert_or_default!("Sec-Fetch-User", HeaderValue::from_static("?1"));
        }
        BrowserKind::Safari => {
            insert_or_default!(
                ACCEPT,
                HeaderValue::from_static(
                    "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"
                )
            );

            if add_ref {
                if let Some(ref_header) =
                    maybe_insert_spoofed_referer(domain_parsed.as_deref(), &mut rng())
                {
                    insert_or_default!(REFERER, ref_header);
                }
            }

            insert_or_default!(UPGRADE_INSECURE_REQUESTS, HeaderValue::from_static("1"));

            if let Ok(ua) = HeaderValue::from_str(user_agent) {
                insert_or_default!(USER_AGENT, ua);
            }
        }
        BrowserKind::Edge | BrowserKind::Other => {
            insert_or_default!(
                ACCEPT,
                HeaderValue::from_static(
                    "text/html,application/xhtml+xml,application/xml;q=0.9,image/webp,*/*;q=0.8"
                )
            );

            if add_ref {
                if let Some(ref_header) =
                    maybe_insert_spoofed_referer(domain_parsed.as_deref(), &mut rng())
                {
                    insert_or_default!(REFERER, ref_header);
                }
            }

            insert_or_default!(UPGRADE_INSECURE_REQUESTS, HeaderValue::from_static("1"));

            if let Ok(ua) = HeaderValue::from_str(user_agent) {
                insert_or_default!(USER_AGENT, ua);
            }
        }
    }

    // re-merge the existing keys.
    if map_exist {
        for h in header_map.iter() {
            if !headers.contains_key(h.0) {
                headers.insert(h.0, h.1.clone());
            }
        }
    }

    if let Some(val) = headers.get(REFERER) {
        if val.as_bytes().is_empty() {
            headers.remove(REFERER);
        }
    }

    headers
}

/// Should title the case headers.
pub fn is_title_case_browser_header(header: &str) -> bool {
    match header {
        "user-agent"
        | "accept"
        | "accept-language"
        | "accept-encoding"
        | "access-control-allow-origin"
        | "connection"
        | "device-memory"
        | "host"
        | "referer"
        | "upgrade-insecure-requests"
        | "cache-control"
        | "pragma"
        | "dpr"
        | "viewport-width"
        | "priority"
        | "rtt"
        | "ect"
        | "downlink" => true,
        _ => false,
    }
}

/// Capitalizes each part of a hyphenated header: `user-agent` → `User-Agent`
pub fn title_case_header(key: &str) -> String {
    let is_leading_hyphen = key.starts_with('-');

    key.split('-')
        .enumerate()
        .map(|(i, part)| {
            if part.is_empty() && i == 0 && is_leading_hyphen {
                // Preserve leading hyphen
                String::new()
            } else {
                let mut chars = part.chars();
                match chars.next() {
                    Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
                    None => String::new(),
                }
            }
        })
        .collect::<Vec<_>>()
        .join("-")
}

/// Modify a reqwest::Request to title-case eligible headers
pub fn rewrite_headers_to_title_case(headers: &mut std::collections::HashMap<String, String>) {
    let mut new_headers = std::collections::HashMap::with_capacity(headers.len());

    for (key, value) in headers.iter() {
        let raw = key.as_str();

        if is_title_case_browser_header(raw) {
            let new_key = title_case_header(raw);
            new_headers.insert(new_key, value.clone());
        } else {
            new_headers.insert(key.clone(), value.clone());
        }
    }

    *headers = new_headers;
}

/// Detect the browser type.
fn detect_browser(ua: &str) -> &'static str {
    let ua = ua.to_ascii_lowercase();
    if ua.contains("chrome") && !ua.contains("edge") {
        "chrome"
    } else if ua.contains("safari") && !ua.contains("chrome") {
        "safari"
    } else if ua.contains("firefox") {
        "firefox"
    } else {
        "chrome" // default fallback
    }
}

/// Real header order map.
pub static HEADER_ORDER_MAP: phf::Map<&'static str, &'static [&'static str]> = phf::phf_map! {
    "safari" => &[
        "Referer", "Origin", "Content-Type", "Accept", "Upgrade-Insecure-Requests",
        "User-Agent", "Content-Length", "Accept-Encoding", "Accept-Language", "Connection",
        "Host", "Cookie", "Sec-Fetch-Dest", "Sec-Fetch-Mode", "Sec-Fetch-Site", ":method",
        ":scheme", ":authority", ":path", "referer", "origin", "content-type", "accept",
        "user-agent", "content-length", "accept-encoding", "accept-language", "cookie",
        "sec-fetch-dest", "sec-fetch-mode", "sec-fetch-site",
    ],
    "chrome" => &[
        "Content-Type",
        "Content-Length",
        "Host",
        "Pragma",
        "Cache-Control",
        "Device-Memory",
        "DPR",
        "Viewport-Width",
        "RTT",
        "Downlink",
        "ECT",
        "sec-ch-ua",
        "sec-ch-ua-mobile",
        "Sec-CH-UA-Full-Version",
        "Sec-CH-UA-Arch",
        "sec-ch-ua-platform",
        "Sec-CH-UA-Platform-Version",
        "Sec-CH-UA-Model",
        "Sec-CH-Prefers-Color-Scheme",
        "Sec-CH-Prefers-Reduced-Motion",
        "Upgrade-Insecure-Requests",
        "Origin",
        "User-Agent",
        "Accept",
        "Sec-Fetch-Site",
        "Sec-Fetch-Mode",
        "Sec-Fetch-User",
        "Sec-Fetch-Dest",
        "Referer",
        "Accept-Encoding",
        "Accept-Language",
        "Priority",
        "Cookie",
        ":method",
        ":authority",
        ":scheme",
        ":path",
        "content-length",
        "cache-control",
        "sec-ch-ua",
        "sec-ch-ua-mobile",
        "sec-ch-ua-platform",
        "upgrade-insecure-requests",
        "origin",
        "content-type",
        "user-agent",
        "accept",
        "sec-fetch-site",
        "sec-fetch-mode",
        "sec-fetch-user",
        "sec-fetch-dest",
        "referer",
        "accept-encoding",
        "accept-language",
        "cookie",
    ],
    "firefox" => &[
        "Host", "User-Agent", "Accept", "Accept-Language", "Accept-Encoding", "Content-Type",
        "Content-Length", "Origin", "Connection", "Referer", "Cookie",
        "Upgrade-Insecure-Requests", "Sec-Fetch-Dest", "Sec-Fetch-Mode", "Sec-Fetch-Site",
        "Sec-Fetch-User", ":method", ":path", ":authority", ":scheme", "user-agent", "accept",
        "accept-language", "accept-encoding", "content-type", "content-length", "origin",
        "referer", "cookie", "upgrade-insecure-requests", "sec-fetch-dest", "sec-fetch-mode",
        "sec-fetch-site", "sec-fetch-user", "te",
    ],
};

/// Sort the headers in custom order based on detected browser.
pub fn sort_headers_by_custom_order(user_agent: &str, original: &HeaderMap) -> HeaderMap {
    let mut sorted = HeaderMap::with_capacity(original.capacity());
    let browser = detect_browser(user_agent);

    if let Some(sort_order) = HEADER_ORDER_MAP.get(browser) {
        for &key in *sort_order {
            if let Ok(header_name) = key.parse::<HeaderName>() {
                if let Some(value) = original.get(&header_name) {
                    sorted.insert(header_name, value.clone());
                }
            }
        }

        // Add remaining headers not already inserted
        for (k, v) in original.iter() {
            if !sorted.contains_key(k) {
                sorted.insert(k.clone(), v.clone());
            }
        }
    }

    sorted
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spoof_viewport::Viewport;
    use http::header::{ACCEPT, HOST, USER_AGENT};
    use http::HeaderMap;
    use url::Url;

    #[test]
    fn test_emulate_headers_chrome_basic() {
        let user_agent = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/119.0.0.0 Safari/537.36";
        let hostname = Some("example.com");
        let viewport = Some(Viewport {
            width: 1920,
            height: 1080,
            emulating_mobile: false,
            ..Viewport::default()
        });

        let domain_parsed = Url::parse("https://example.com").ok().map(Box::new);

        let headers = emulate_headers(
            user_agent,
            &None, // No initial headers
            &hostname,
            true, // chrome headers enabled
            &viewport,
            &domain_parsed,
            &None, // default to extensive headers
        );

        // Check host header
        assert_eq!(headers.get(HOST).unwrap(), "example.com");

        // Check user-agent header
        assert_eq!(headers.get(USER_AGENT).unwrap(), user_agent);

        // Check Accept header exists with a reasonable default
        assert!(headers.contains_key(ACCEPT));

        // Check Sec-Fetch headers exist
        assert!(headers.contains_key("Sec-Fetch-Site"));
        assert!(headers.contains_key("Sec-Fetch-Mode"));
        assert!(headers.contains_key("Sec-Fetch-User"));
        assert!(headers.contains_key("Sec-Fetch-Dest"));

        // Check Chrome-specific headers like sec-ch-ua
        assert!(headers.contains_key("sec-ch-ua"));
        assert!(headers.contains_key("sec-ch-ua-mobile"));
        assert!(headers.contains_key("sec-ch-ua-platform"));

        // Ensure viewport headers are set
        assert_eq!(headers.get("Viewport-Width").unwrap(), "1920");
    }

    #[test]
    fn test_emulate_headers_existing_headers_merging() {
        let user_agent = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.2 Safari/605.1.15";
        let hostname = Some("example.org");
        let viewport = Some(Viewport {
            width: 1024,
            height: 768,
            emulating_mobile: false,
            ..Viewport::default()
        });
        let existing_url = Url::parse("https://example.org").ok().map(Box::new);

        // Existing headers supplied by the user
        let mut existing_headers = HeaderMap::new();
        existing_headers.insert("Accept-Language", HeaderValue::from_static("en-US"));
        existing_headers.insert("Custom-Header", HeaderValue::from_static("CustomValue"));

        let headers = emulate_headers(
            user_agent,
            &Some(&existing_headers),
            &hostname,
            false, // chrome=false, use existing naming
            &viewport,
            &existing_url,
            &Some(HeaderDetailLevel::Mild),
        );

        // Existing headers retained or merged correctly
        assert_eq!(headers.get("Accept-Language").unwrap(), "en-US");
        assert_eq!(headers.get("Custom-Header").unwrap(), "CustomValue");

        // Default headers added
        assert_eq!(headers.get(USER_AGENT).unwrap(), user_agent);
        assert!(headers.contains_key(ACCEPT));
    }

    #[test]
    fn test_emulate_headers_default_handling() {
        // This tests emulate_headers behavior with minimum parameters to ensure graceful defaults
        let user_agent = "";
        let headers = emulate_headers(user_agent, &None, &None, false, &None, &None, &None);

        // Check that empty or default values handled gracefully without crashing
        assert!(!headers.is_empty()); // Should produce something safely without panics
    }
}
