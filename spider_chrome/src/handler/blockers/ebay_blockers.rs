use crate::handler::blockers::Trie;

lazy_static::lazy_static! {
        /// Ignore list of urls.
        static ref URL_IGNORE_TRIE: Trie = {
            let mut trie = Trie::new();
            let patterns = [
                "https://www.ebay.com/sch/ajax/autocomplete",
                "https://www.ebay.com/blueberry/v1/ads/identity/pixelUrls",
                "https://svcs.ebay.com/ufeservice/v1/events",
                "https://www.ebay.com/gh/useracquisition?",
                "https://vi.vipr.ebaydesc.com/",
                "https://srv.main.ebayrtm.com/",
                "https://www.ebay.com/nap/napkinapi/",
                "https://ir.ebaystatic.com/rs/c/scandal/ScandalJS-"
            ];
            for pattern in &patterns {
                trie.insert(pattern);
            }
            trie
        };
}

// Block ebay events that are not required
pub fn block_ebay(
    event: &chromiumoxide_cdp::cdp::browser_protocol::fetch::EventRequestPaused,
) -> bool {
    URL_IGNORE_TRIE.contains_prefix(&event.request.url)
}
