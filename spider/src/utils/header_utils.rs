use crate::configuration::Configuration;
use reqwest::header::{HOST, REFERER};
use reqwest::{
    header::{HeaderMap, HeaderValue},
    ClientBuilder,
};

/// Setup the default headers for the request.
pub fn setup_default_headers(
    client_builder: ClientBuilder,
    configuration: &Configuration,
    header_map: HeaderMap,
    url: &Option<Box<url::Url>>,
) -> ClientBuilder {
    let mut headers = match configuration.headers {
        Some(ref h) => *h.clone(),
        None => crate::configuration::SerializableHeaderMap::default(),
    };

    if !headers.contains_key(REFERER) {
        if let Ok(ref_value) =
            HeaderValue::from_str(crate::features::spoof_referrer::spoof_referrer())
        {
            if !ref_value.is_empty() {
                headers.insert(REFERER, ref_value);
            }
        }
    }

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

/// Build the headers to use to act like a browser
pub fn get_mimic_headers(user_agent: &str) -> reqwest::header::HeaderMap {
    use reqwest::header::{ACCEPT, CACHE_CONTROL, TE, UPGRADE_INSECURE_REQUESTS};

    let mut headers = HeaderMap::new();

    if user_agent.contains("Chrome/") {
        headers.insert(ACCEPT, HeaderValue::from_static("text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.9"));
        headers.insert("Sec-Fetch-Site", HeaderValue::from_static("none"));
        headers.insert("Sec-Fetch-Mode", HeaderValue::from_static("navigate"));
        headers.insert("Sec-Fetch-User", HeaderValue::from_static("?1"));
        headers.insert("Sec-Fetch-Dest", HeaderValue::from_static("document"));
        headers.insert(UPGRADE_INSECURE_REQUESTS, HeaderValue::from_static("1"));
    } else if user_agent.contains("Firefox/") {
        headers.insert(
            ACCEPT,
            HeaderValue::from_static(
                "text/html,application/xhtml+xml,application/xml;q=0.9,image/webp,*/*;q=0.8",
            ),
        );
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
        headers.insert(UPGRADE_INSECURE_REQUESTS, HeaderValue::from_static("1"));
    } else if user_agent.contains("Edge/") {
        headers.insert(
            ACCEPT,
            HeaderValue::from_static(
                "text/html,application/xhtml+xml,application/xml;q=0.9,image/webp,*/*;q=0.8",
            ),
        );
        headers.insert(UPGRADE_INSECURE_REQUESTS, HeaderValue::from_static("1"));
    }

    headers
}

/// convert the headermap to hashmap
pub fn header_map_to_hash_map(header_map: &HeaderMap) -> std::collections::HashMap<String, String> {
    let mut hash_map = std::collections::HashMap::new();

    for (key, value) in header_map.iter() {
        let key_string = key.as_str().to_string();
        if let Ok(value_string) = value.to_str() {
            hash_map.insert(key_string, value_string.to_string());
        }
    }

    hash_map
}
