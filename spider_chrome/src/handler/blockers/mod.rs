/// Block websites from spider_firewall list
pub mod block_websites;
/// xhr blockers
pub mod xhr;

pub use spider_network_blocker::intercept_manager::NetworkInterceptManager;

/// Url matches analytics that we want to ignore or trackers.
pub(crate) fn ignore_script_embedded(url: &str) -> bool {
    spider_network_blocker::scripts::URL_IGNORE_EMBEDED_TRIE.contains_prefix(url)
}

/// Url matches analytics that we want to ignore or trackers.
pub(crate) fn ignore_script_xhr(url: &str) -> bool {
    spider_network_blocker::xhr::URL_IGNORE_XHR_TRIE.contains_prefix(url)
}

/// Url matches media that we want to ignore.
pub(crate) fn ignore_script_xhr_media(url: &str) -> bool {
    spider_network_blocker::xhr::URL_IGNORE_XHR_MEDIA_TRIE.contains_prefix(url)
}
