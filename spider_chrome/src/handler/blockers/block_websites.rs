/// Firewall protection. This does nothing without the [firewall] flag.
#[cfg(feature = "firewall")]
pub(crate) fn block_website(u: &str) -> bool {
    spider_firewall::is_bad_website_url_clean(&u)
}

/// Firewall protection. This does nothing without the [firewall] flag.
#[cfg(not(feature = "firewall"))]
pub(crate) fn block_website(_u: &str) -> bool {
    false
}
