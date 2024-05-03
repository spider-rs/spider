
use reqwest::{header::{HeaderValue, HeaderMap}, ClientBuilder};
use crate::configuration::Configuration;
use reqwest::header::REFERER;

/// Setup the default headers for the request.
pub fn setup_default_headers(
    client_builder: ClientBuilder, 
    configuration: &Configuration,
) -> ClientBuilder {
    let mut headers = match configuration.headers.clone() {
        Some(h) => *h,
        None => HeaderMap::new(),
    };

    if !headers.contains_key(REFERER) {
        if let Ok(ref_value) = HeaderValue::from_str(&crate::features::spoof_referrer::spoof_referrer()) {
            if !ref_value.is_empty() {
                headers.insert(REFERER, ref_value);
            }
        }
    }

    client_builder.default_headers(headers)
}
