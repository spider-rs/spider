use crate::configuration::{Configuration, SerializableHeaderMap};
use reqwest::header::{HeaderMap, REFERER};
pub use spider_fingerprint::spoof_headers::{
    is_title_case_browser_header, rewrite_headers_to_title_case,
};

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
    header_map.extend(spider_fingerprint::emulate_headers(
        user_agent,
        &headers.as_ref().map(|f| f.inner()),
        hostname,
        true,
        &viewport.map(|f| f.into()),
        domain_parsed,
        &Some(spider_fingerprint::spoof_headers::HeaderDetailLevel::Light),
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
