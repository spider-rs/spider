include!(concat!(env!("OUT_DIR"), "/bad_websites.rs"));

/// The url is in the bad website.
pub fn is_bad_website_url(host: &str) -> bool {
    BAD_WEBSITES.contains(&host)
}
