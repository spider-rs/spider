use crate::configuration::{Configuration, SerializableHeaderMap};
#[cfg(feature = "real_browser")]
use rand::{rng, Rng};
use reqwest::header::{HeaderMap, HeaderName, REFERER};

lazy_static::lazy_static! {
    /// The brand version of google chrome. Use the env var 'NOT_A_BRAND_VERSION'.
    static ref NOT_A_BRAND_VERSION: String = {
        std::env::var("NOT_A_BRAND_VERSION").unwrap_or_else(|_| "24".to_string())
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

/// Setup the default headers for the request.
pub fn setup_default_headers(
    client_builder: crate::client::ClientBuilder,
    configuration: &Configuration,
) -> crate::client::ClientBuilder {
    let headers = match configuration.headers {
        Some(ref h) => *h.clone(),
        None => crate::configuration::SerializableHeaderMap::default(),
    };

    client_builder.default_headers(headers.0)
}

#[cfg(feature = "real_browser")]
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
#[cfg(feature = "real_browser")]
/// sec ch user-agent platform
fn get_sec_ch_ua_platform() -> &'static str {
    "\"macOS\""
}

#[cfg(target_os = "windows")]
#[cfg(feature = "real_browser")]
/// sec ch user-agent platform
fn get_sec_ch_ua_platform() -> &'static str {
    "\"Windows\""
}

#[cfg(target_os = "linux")]
#[cfg(feature = "real_browser")]
/// sec ch user-agent platform
fn get_sec_ch_ua_platform() -> &'static str {
    "\"Linux\""
}

#[cfg(target_os = "android")]
#[cfg(feature = "real_browser")]
/// sec ch user-agent platform
fn get_sec_ch_ua_platform() -> &'static str {
    "\"Android\""
}

#[cfg(target_os = "ios")]
#[cfg(feature = "real_browser")]
/// sec ch user-agent platform
fn get_sec_ch_ua_platform() -> &'static str {
    "\"iOS\""
}

#[cfg(target_arch = "x86")]
#[cfg(feature = "real_browser")]
/// sec-ch-ua-arch: system architecture (32-bit x86)
fn get_sec_ch_ua_arch() -> &'static str {
    "\"x86\""
}

#[cfg(target_arch = "x86_64")]
#[cfg(feature = "real_browser")]
/// sec-ch-ua-arch: system architecture (64-bit x86_64)
fn get_sec_ch_ua_arch() -> &'static str {
    "\"x86_64\""
}

#[cfg(any(target_arch = "aarch64", target_arch = "arm"))]
#[cfg(feature = "real_browser")]
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
#[cfg(feature = "real_browser")]
/// sec-ch-ua-arch: unknown or unsupported architecture
fn get_sec_ch_ua_arch() -> &'static str {
    "\"unknown\""
}

#[cfg(feature = "real_browser")]
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
#[cfg(feature = "real_browser")]
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
#[cfg(feature = "real_browser")]
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

#[cfg(feature = "real_browser")]
/// Add the spoofed header from google or a real domain.
pub fn maybe_insert_spoofed_referer(
    domain_parsed: Option<&url::Url>,
    rng: &mut rand::rngs::ThreadRng,
) -> Option<reqwest::header::HeaderValue> {
    use crate::client::header::HeaderValue;

    if domain_parsed.is_some() && rng.random_bool(0.75) {
        domain_parsed
            .and_then(crate::features::spoof_referrer::spoof_referrer_google)
            .and_then(|s| HeaderValue::from_str(&s).ok())
            .or_else(|| {
                let fallback = crate::features::spoof_referrer::spoof_referrer();
                HeaderValue::from_static(fallback).into()
            })
    } else {
        let fallback = crate::features::spoof_referrer::spoof_referrer();
        HeaderValue::from_static(fallback).into()
    }
}

/// Build the headers to use to act like a browser.
#[cfg(feature = "real_browser")]
pub fn get_mimic_headers(
    user_agent: &str,
    header_map: &std::option::Option<Box<SerializableHeaderMap>>,
    contains_referer: bool,
    hostname: &Option<&str>,
    chrome: bool,
    viewport: &Option<crate::features::chrome_common::Viewport>,
    domain_parsed: &Option<Box<url::Url>>,
) -> reqwest::header::HeaderMap {
    use reqwest::header::{
        HeaderValue, ACCEPT, ACCEPT_ENCODING, ACCEPT_LANGUAGE, CACHE_CONTROL, CONNECTION, HOST,
        PRAGMA, TE, UPGRADE_INSECURE_REQUESTS, USER_AGENT,
    };

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

    let add_ref = !contains_referer && cfg!(feature = "spoof");
    let cap = if browser == BrowserKind::Chrome {
        31
    } else {
        10
    };
    let mut headers = HeaderMap::with_capacity(cap);
    let binding = reqwest::header::HeaderMap::with_capacity(cap);
    let mut map_exist = false;

    let header_map = header_map
        .as_ref()
        .map(|h| {
            let m = h.inner();
            map_exist = !m.is_empty();
            m
        })
        .unwrap_or(&binding);

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
            let linux_agent = user_agent.contains("Linux");
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
            if add_ref && !header_map.contains_key(REFERER) {
                if let Some(ref_header) =
                    maybe_insert_spoofed_referer(domain_parsed.as_deref(), &mut thread_rng)
                {
                    insert_or_default!(&refererer_header.as_header_name(), ref_header);
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

            if let Ok(device_memory_str) = HeaderValue::from_str(&device_memory_str) {
                insert_or_default!("Device-Memory", device_memory_str);
            }

            // 10. Optional behavior/diagnostic headers
            insert_or_default!(
                &cache_control_header.as_header_name(),
                HeaderValue::from_static("max-age=0")
            );

            insert_or_default!("Dpr", HeaderValue::from_static("2"));

            if let Some(vp) = viewport {
                let width = if vp.width > 0 {
                    format!("{}", vp.width)
                } else {
                    format!(
                        "{}",
                        crate::features::chrome_viewport::randomize_viewport_rng(
                            &crate::features::chrome_viewport::DeviceType::Desktop,
                            &mut thread_rng
                        )
                        .width
                    )
                };

                if let Ok(width) = HeaderValue::from_str(&width) {
                    insert_or_default!("Viewport-Width", width);
                }
            }

            insert_or_default!("Priority", HeaderValue::from_static("u=0, i"));
            insert_or_default!("Ect", HeaderValue::from_static("4g"));
            insert_or_default!("Rtt", HeaderValue::from_static("50"));

            if let Ok(dl) = HeaderValue::from_str(&downlink_str) {
                insert_or_default!("Downlink", dl);
            }
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
        BrowserKind::Firefox => {
            insert_or_default!(
                ACCEPT,
                HeaderValue::from_static(
                    "text/html,application/xhtml+xml,application/xml;q=0.9,image/webp,*/*;q=0.8"
                )
            );

            if add_ref && !header_map.contains_key(REFERER) {
                if let Ok(ref_value) =
                    HeaderValue::from_str(crate::features::spoof_referrer::spoof_referrer())
                {
                    if !ref_value.is_empty() {
                        headers.insert(REFERER, ref_value);
                    }
                }
            }
            insert_or_default!(UPGRADE_INSECURE_REQUESTS, HeaderValue::from_static("1"));
            insert_or_default!(CACHE_CONTROL, HeaderValue::from_static("max-age=0"));
            insert_or_default!(TE, HeaderValue::from_static("trailers"));
        }
        BrowserKind::Safari => {
            insert_or_default!(
                ACCEPT,
                HeaderValue::from_static(
                    "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"
                )
            );

            if add_ref && !header_map.contains_key(REFERER) {
                if let Some(ref_header) =
                    maybe_insert_spoofed_referer(domain_parsed.as_deref(), &mut rng())
                {
                    insert_or_default!(REFERER, ref_header);
                }
            }

            insert_or_default!(UPGRADE_INSECURE_REQUESTS, HeaderValue::from_static("1"));
        }
        BrowserKind::Edge => {
            insert_or_default!(
                ACCEPT,
                HeaderValue::from_static(
                    "text/html,application/xhtml+xml,application/xml;q=0.9,image/webp,*/*;q=0.8"
                )
            );

            if add_ref && !header_map.contains_key(REFERER) {
                if let Some(ref_header) =
                    maybe_insert_spoofed_referer(domain_parsed.as_deref(), &mut rng())
                {
                    insert_or_default!(REFERER, ref_header);
                }
            }

            insert_or_default!(UPGRADE_INSECURE_REQUESTS, HeaderValue::from_static("1"));
        }
        BrowserKind::Other => (),
    }

    // re-merge the existing keys.
    if map_exist {
        for h in header_map {
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

#[cfg(not(feature = "real_browser"))]
/// Build the headers to use to act like a browser.
pub fn get_mimic_headers(
    _user_agent: &str,
    _header_map: &std::option::Option<Box<SerializableHeaderMap>>,
    _contains_referer: bool,
    _hostname: &Option<&str>,
    _chrome: bool,
    _viewport: &Option<crate::features::chrome_common::Viewport>,
    _domain_parsed: &Option<Box<url::Url>>,
) -> reqwest::header::HeaderMap {
    Default::default()
}

/// convert the headermap to hashmap
pub fn header_map_to_hash_map(header_map: &HeaderMap) -> std::collections::HashMap<String, String> {
    let mut hash_map = std::collections::HashMap::with_capacity(header_map.len());

    for (key, value) in header_map.iter() {
        let key_string = key.as_str().to_string();
        if let Ok(value_string) = value.to_str() {
            hash_map.insert(key_string, value_string.to_string());
        }
    }

    hash_map
}

#[cfg(feature = "real_browser")]
/// Extend the headers.
pub fn extend_headers(
    header_map: &mut reqwest::header::HeaderMap,
    user_agent: &str,
    headers: &std::option::Option<Box<SerializableHeaderMap>>,
    hostname: &Option<&str>,
    viewport: &Option<crate::features::chrome_common::Viewport>,
    domain_parsed: &Option<Box<url::Url>>,
) {
    header_map.extend(crate::utils::header_utils::get_mimic_headers(
        user_agent,
        &headers,
        has_ref(&headers),
        hostname,
        true,
        viewport,
        domain_parsed,
    ));
}

#[cfg(not(feature = "real_browser"))]
/// Extend the headers.
pub fn extend_headers(
    header_map: &mut reqwest::header::HeaderMap,
    _user_agent: &str,
    headers: &std::option::Option<Box<SerializableHeaderMap>>,
    _hostname: &Option<&str>,
    _viewport: &Option<crate::features::chrome_common::Viewport>,
    _domain_parsed: &Option<Box<url::Url>>,
) {
    if let Some(_headers) = headers {
        header_map.extend(_headers.0.clone());
    }
}

/// Headers has ref
pub fn has_ref(headers: &std::option::Option<Box<SerializableHeaderMap>>) -> bool {
    match headers {
        Some(headers) => headers.contains_key(REFERER),
        _ => false,
    }
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
fn title_case_header(key: &str) -> String {
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
