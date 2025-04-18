use crate::configuration::{Configuration, SerializableHeaderMap};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, HOST, REFERER};

/// Setup the default headers for the request.
pub fn setup_default_headers(
    client_builder: crate::client::ClientBuilder,
    configuration: &Configuration,
    header_map: HeaderMap,
    url: &Option<Box<url::Url>>,
) -> crate::client::ClientBuilder {
    let mut headers = match configuration.headers {
        Some(ref h) => *h.clone(),
        None => crate::configuration::SerializableHeaderMap::default(),
    };

    if !headers.contains_key(HOST) && configuration.preserve_host_header {
        if let Some(u) = url {
            if let Some(host) = u.host_str() {
                if let Ok(ref_value) = HeaderValue::from_str(host) {
                    if !ref_value.is_empty() {
                        headers.insert(HOST, ref_value);
                    }
                }
            }
        }
    }

    headers.extend(header_map);

    client_builder.default_headers(headers.0)
}

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

/// Build the headers to use to act like a browser.
pub fn get_mimic_headers(
    user_agent: &str,
    header_map: &std::option::Option<Box<SerializableHeaderMap>>,
    contains_referer: bool,
    hostname: &Option<&str>,
    chrome: bool,
) -> reqwest::header::HeaderMap {
    use reqwest::header::{
        HeaderValue, ACCEPT, ACCEPT_ENCODING, ACCEPT_LANGUAGE, CACHE_CONTROL, CONNECTION, PRAGMA,
        TE, UPGRADE_INSECURE_REQUESTS, USER_AGENT,
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
        26
    } else {
        10
    };
    let mut headers = HeaderMap::with_capacity(cap);
    let binding = reqwest::header::HeaderMap::with_capacity(cap);
    let header_map = header_map.as_ref().map(|h| h.inner()).unwrap_or(&binding);

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
            insert_or_default!("sec-ch-ua-mobile", HeaderValue::from_static("?0"));
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
            if !contains_referer {
                insert_or_default!(
                    &refererer_header.as_header_name(),
                    HeaderValue::from_static(crate::features::spoof_referrer::spoof_referrer())
                );
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

            // 10. Optional behavior/diagnostic headers
            insert_or_default!(
                &cache_control_header.as_header_name(),
                HeaderValue::from_static("max-age=0")
            );

            insert_or_default!(
                &pragma_header.as_header_name(),
                HeaderValue::from_static("no-cache")
            );

            insert_or_default!("Priority", HeaderValue::from_static("u=0, i"));
            insert_or_default!("Downlink", HeaderValue::from_static("10"));
            insert_or_default!("Rtt", HeaderValue::from_static("50"));

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
            insert_or_default!(
                "sec-ch-ua-form-factors",
                HeaderValue::from_static(r#""Desktop""#)
            );
            insert_or_default!("sec-ch-ua-wow64", HeaderValue::from_static("?0"));
            insert_or_default!(
                "sec-ch-prefers-color-scheme",
                HeaderValue::from_static("light")
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
                if let Ok(ref_value) =
                    HeaderValue::from_str(crate::features::spoof_referrer::spoof_referrer())
                {
                    if !ref_value.is_empty() {
                        headers.insert(REFERER, ref_value);
                    }
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
                if let Ok(ref_value) =
                    HeaderValue::from_str(crate::features::spoof_referrer::spoof_referrer())
                {
                    if !ref_value.is_empty() {
                        headers.insert(REFERER, ref_value);
                    }
                }
            }

            insert_or_default!(UPGRADE_INSECURE_REQUESTS, HeaderValue::from_static("1"));
        }
        BrowserKind::Other => {}
    }

    headers
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
) {
    header_map.extend(crate::utils::header_utils::get_mimic_headers(
        user_agent,
        &headers,
        has_ref(&headers),
        hostname,
        true,
    ));
}

#[cfg(not(feature = "real_browser"))]
/// Extend the headers.
pub fn extend_headers(
    _header_map: &mut reqwest::header::HeaderMap,
    _user_agent: &str,
    _headers: &std::option::Option<Box<SerializableHeaderMap>>,
    _hostname: &Option<&str>,
) {
}

/// Headers has ref
pub fn has_ref(headers: &std::option::Option<Box<SerializableHeaderMap>>) -> bool {
    match headers {
        Some(headers) => headers.contains_key(REFERER),
        _ => false,
    }
}
