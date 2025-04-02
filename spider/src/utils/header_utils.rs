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
}

fn parse_user_agent_to_ch_ua(ua: &str) -> String {
    let mut parts = Vec::with_capacity(3);

    if ua.contains("Chrome/") {
        if let Some(version) = ua
            .split("Chrome/")
            .nth(1)
            .and_then(|s| s.split_whitespace().next())
        {
            if let Some(major_version) = version.split('.').next() {
                parts.push(format!(r#""Chromium";v="{}""#, major_version));
                parts.push(format!(r#""Not:A-Brand";v="{}""#, *NOT_A_BRAND_VERSION));
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

/// Build the headers to use to act like a browser
pub fn get_mimic_headers(
    user_agent: &str,
    chrome_entry: bool,
    contains_referer: bool,
) -> reqwest::header::HeaderMap {
    use reqwest::header::{ACCEPT, ACCEPT_ENCODING, CACHE_CONTROL, TE, UPGRADE_INSECURE_REQUESTS};
    let mut headers = HeaderMap::new();
    let add_ref = !contains_referer && cfg!(feature = "spoof");

    if user_agent.contains("Chrome/") {
        headers.insert(ACCEPT, HeaderValue::from_static("text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.9"));

        headers.insert(
            ACCEPT_ENCODING,
            HeaderValue::from_static("gzip, deflate, br, zstd"),
        );

        if !chrome_entry {
            headers.insert("Accept-Language", HeaderValue::from_static("*"));
        }

        if add_ref {
            if let Ok(ref_value) =
                HeaderValue::from_str(crate::features::spoof_referrer::spoof_referrer())
            {
                if !ref_value.is_empty() {
                    headers.insert(REFERER, ref_value);
                }
            }
        }

        if let Ok(ch) = HeaderValue::from_str(&parse_user_agent_to_ch_ua(user_agent)) {
            headers.insert("Sec-CH-UA", ch);
        }

        headers.insert("Sec-CH-UA-Mobile", HeaderValue::from_static("?0"));
        headers.insert(
            "Sec-CH-UA-Platform",
            HeaderValue::from_static(get_sec_ch_ua_platform()),
        );
        headers.insert("Sec-Fetch-Dest", HeaderValue::from_static("document"));
        headers.insert("Sec-Fetch-Mode", HeaderValue::from_static("navigate"));
        headers.insert("Sec-Fetch-Site", HeaderValue::from_static("none"));
        headers.insert("Sec-Fetch-User", HeaderValue::from_static("?1"));

        headers.insert(UPGRADE_INSECURE_REQUESTS, HeaderValue::from_static("1"));
    } else if user_agent.contains("Firefox/") {
        headers.insert(
            ACCEPT,
            HeaderValue::from_static(
                "text/html,application/xhtml+xml,application/xml;q=0.9,image/webp,*/*;q=0.8",
            ),
        );
        if add_ref {
            if let Ok(ref_value) =
                HeaderValue::from_str(crate::features::spoof_referrer::spoof_referrer())
            {
                if !ref_value.is_empty() {
                    headers.insert(REFERER, ref_value);
                }
            }
        }
        headers.insert(UPGRADE_INSECURE_REQUESTS, HeaderValue::from_static("1"));
        headers.insert(CACHE_CONTROL, HeaderValue::from_static("max-age=0"));
        headers.insert(TE, HeaderValue::from_static("trailers"));
    } else if user_agent.contains("Safari/") && !user_agent.contains("Chrome/") {
        headers.insert(
            ACCEPT,
            HeaderValue::from_static(
                "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
            ),
        );
        if add_ref {
            if let Ok(ref_value) =
                HeaderValue::from_str(crate::features::spoof_referrer::spoof_referrer())
            {
                if !ref_value.is_empty() {
                    headers.insert(REFERER, ref_value);
                }
            }
        }
        headers.insert(UPGRADE_INSECURE_REQUESTS, HeaderValue::from_static("1"));
    } else if user_agent.contains("Edge/") {
        headers.insert(
            ACCEPT,
            HeaderValue::from_static(
                "text/html,application/xhtml+xml,application/xml;q=0.9,image/webp,*/*;q=0.8",
            ),
        );
        if add_ref {
            if let Ok(ref_value) =
                HeaderValue::from_str(crate::features::spoof_referrer::spoof_referrer())
            {
                if !ref_value.is_empty() {
                    headers.insert(REFERER, ref_value);
                }
            }
        }
        headers.insert(UPGRADE_INSECURE_REQUESTS, HeaderValue::from_static("1"));
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
        false,
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
