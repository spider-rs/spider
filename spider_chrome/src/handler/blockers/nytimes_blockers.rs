use crate::handler::blockers::Trie;

lazy_static::lazy_static! {
    /// Ignore list of urls.
    static ref URL_IGNORE_TRIE: Trie = {
        let mut trie = Trie::new();
        let patterns = [
            "https://purr.nytimes.com/v1/purr-cache",
            "https://static01.nyt.com/ads/tpc-check.html",
            "https://www.nytimes.com/vi-assets/static-assets/adslot",
            "https://purr.nytimes.com/v2/tcf",
            "https://a.et.nytimes.com//.status",
            "https://www.nytimes.com/fides/api/v1/privacy-experience?",
            "https://o82024.ingest.us.sentry.io/",
            "https://a.nytimes.com/svc/nyt/data-layer?",
            "https://www.nytimes.com/ads/prebid9.11.0.js"
        ];
        for pattern in &patterns {
            trie.insert(pattern);
        }
        trie
    };
    /// Ignore list of urls.
    static ref URL_IGNORE_TRIE_VISUALS: Trie = {
        let mut trie = Trie::new();
        let patterns = [
            "https://static01.nyt.com/video-static/vhs3/vhs.min.js",
            "https://www.nytimes.com/vi-assets/static-assets/vendors~",
            "https://als-svc.nytimes.com/als"
        ];
        for pattern in &patterns {
            trie.insert(pattern);
        }
        trie
    };
}

// Block nytimes events that are not required
pub fn block_nytimes(
    event: &chromiumoxide_cdp::cdp::browser_protocol::fetch::EventRequestPaused,
    ignore_visuals: bool,
) -> bool {
    let mut allowed = URL_IGNORE_TRIE.contains_prefix(&event.request.url);

    if !allowed && ignore_visuals {
        allowed = URL_IGNORE_TRIE_VISUALS.contains_prefix(&event.request.url)
    }

    allowed
}
