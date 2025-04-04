use crate::configuration::{Configuration, SerializableHeaderMap};
use reqwest::header::{HeaderMap, HeaderValue, HOST, REFERER};

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
    /// The chrome platform version of google chrome. Use the env var 'NOT_A_BRAND_VERSION'.
    static ref CHROME_PLATFORM_VERSION: String = {
        std::env::var("CHROME_PLATFORM_VERSION").unwrap_or_else(|_| "\"14.6.1\"".to_string())
    };
}

fn parse_user_agent_to_ch_ua(ua: &str, dec: bool) -> String {
    let mut parts = Vec::with_capacity(3);

    if ua.contains("Chrome/") {
        if let Some(version) = ua
            .split("Chrome/")
            .nth(1)
            .and_then(|s| s.split_whitespace().next())
        {
            if let Some(major_version) = version.split('.').next() {
                parts.push(format!(
                    r#""Chromium";v="{}{}""#,
                    major_version,
                    if dec { ".0.0" } else { "" }
                ));
                parts.push(format!(
                    r#""Not:A-Brand";v="{}{}""#,
                    *NOT_A_BRAND_VERSION,
                    if dec { ".0.0" } else { "" }
                ));
                parts.push(format!(r#""Google Chrome";v="{}""#, major_version));
            }
        }
    }

    parts.join(", ")
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

#[cfg(target_arch = "aarch64")]
/// sec-ch-ua-arch: system architecture (ARM 64-bit)
fn get_sec_ch_ua_arch() -> &'static str {
    "\"arm64\""
}

#[cfg(target_arch = "arm")]
/// sec-ch-ua-arch: system architecture (ARM 32-bit)
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

/// Build the headers to use to act like a browser
pub fn get_mimic_headers(
    user_agent: &str,
    header_map: &std::option::Option<Box<SerializableHeaderMap>>,
    contains_referer: bool,
) -> reqwest::header::HeaderMap {
    use reqwest::header::{
        HeaderValue, ACCEPT, ACCEPT_ENCODING, CACHE_CONTROL, REFERER, TE, UPGRADE_INSECURE_REQUESTS,
    };

    let mut headers = HeaderMap::new();
    let add_ref = !contains_referer && cfg!(feature = "spoof");

    let binding = reqwest::header::HeaderMap::new();
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

    if user_agent.contains("Chrome/") {
        insert_or_default!(
            ACCEPT,
            HeaderValue::from_static("text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.9")
        );
        insert_or_default!(
            ACCEPT_ENCODING,
            HeaderValue::from_static("gzip, deflate, br, zstd")
        );
        insert_or_default!("Accept-Language", HeaderValue::from_static("*"));

        if add_ref {
            if !header_map.contains_key(REFERER) {
                if let Ok(ref_value) =
                    HeaderValue::from_str(crate::features::spoof_referrer::spoof_referrer())
                {
                    if !ref_value.is_empty() {
                        headers.insert(REFERER, ref_value);
                    }
                }
            }
        }

        if let Ok(ch) = HeaderValue::from_str(&parse_user_agent_to_ch_ua(user_agent, false)) {
            insert_or_default!("sec-ch-ua", ch);
        }

        if let Ok(ch) = HeaderValue::from_str(&parse_user_agent_to_ch_ua(user_agent, true)) {
            insert_or_default!("sec-ch-ua-full-version-list", ch);
        }

        insert_or_default!(
            "Sec-CH-UA-Arc",
            HeaderValue::from_static(get_sec_ch_ua_arch())
        );
        insert_or_default!("Sec-CH-UA-Mobile", HeaderValue::from_static("?0"));
        insert_or_default!(
            "sec-ch-ua-platform",
            HeaderValue::from_static(get_sec_ch_ua_platform())
        );

        #[cfg(not(target_os = "linux"))]
        insert_or_default!(
            "sec-ch-ua-platform-version",
            HeaderValue::from_str(&CHROME_PLATFORM_VERSION).unwrap()
        );

        insert_or_default!("Sec-Fetch-Dest", HeaderValue::from_static("document"));
        insert_or_default!("Sec-Fetch-Mode", HeaderValue::from_static("navigate"));
        insert_or_default!("Sec-Fetch-Site", HeaderValue::from_static("none"));
        insert_or_default!("Sec-Fetch-User", HeaderValue::from_static("?1"));

        insert_or_default!(UPGRADE_INSECURE_REQUESTS, HeaderValue::from_static("1"));
    } else if user_agent.contains("Firefox/") {
        insert_or_default!(
            ACCEPT,
            HeaderValue::from_static(
                "text/html,application/xhtml+xml,application/xml;q=0.9,image/webp,*/*;q=0.8"
            )
        );

        if add_ref {
            if !header_map.contains_key(REFERER) {
                if let Ok(ref_value) =
                    HeaderValue::from_str(crate::features::spoof_referrer::spoof_referrer())
                {
                    if !ref_value.is_empty() {
                        headers.insert(REFERER, ref_value);
                    }
                }
            }
        }

        insert_or_default!(UPGRADE_INSECURE_REQUESTS, HeaderValue::from_static("1"));
        insert_or_default!(CACHE_CONTROL, HeaderValue::from_static("max-age=0"));
        insert_or_default!(TE, HeaderValue::from_static("trailers"));
    } else if user_agent.contains("Safari/") && !user_agent.contains("Chrome/") {
        insert_or_default!(
            ACCEPT,
            HeaderValue::from_static(
                "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"
            )
        );

        if add_ref {
            if !header_map.contains_key(REFERER) {
                if let Ok(ref_value) =
                    HeaderValue::from_str(crate::features::spoof_referrer::spoof_referrer())
                {
                    if !ref_value.is_empty() {
                        headers.insert(REFERER, ref_value);
                    }
                }
            }
        }

        insert_or_default!(UPGRADE_INSECURE_REQUESTS, HeaderValue::from_static("1"));
    } else if user_agent.contains("Edge/") {
        insert_or_default!(
            ACCEPT,
            HeaderValue::from_static(
                "text/html,application/xhtml+xml,application/xml;q=0.9,image/webp,*/*;q=0.8"
            )
        );

        if add_ref {
            if !header_map.contains_key(REFERER) {
                if let Ok(ref_value) =
                    HeaderValue::from_str(crate::features::spoof_referrer::spoof_referrer())
                {
                    if !ref_value.is_empty() {
                        headers.insert(REFERER, ref_value);
                    }
                }
            }
        }

        insert_or_default!(UPGRADE_INSECURE_REQUESTS, HeaderValue::from_static("1"));
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
) {
    header_map.extend(crate::utils::header_utils::get_mimic_headers(
        user_agent,
        &headers,
        has_ref(&headers),
    ));
}

#[cfg(not(feature = "real_browser"))]
/// Extend the headers.
pub fn extend_headers(
    _header_map: &mut reqwest::header::HeaderMap,
    _user_agent: &str,
    _headers: &std::option::Option<Box<SerializableHeaderMap>>,
) {
}

/// Headers has ref
pub fn has_ref(headers: &std::option::Option<Box<SerializableHeaderMap>>) -> bool {
    match headers {
        Some(headers) => headers.contains_key(REFERER),
        _ => false,
    }
}
