use crate::handler::blockers::Trie;

lazy_static::lazy_static! {
        /// Ignore list of urls.
        static ref URL_IGNORE_TRIE: Trie = {
            let mut trie = Trie::new();
            let patterns = [
                "/log",
                "https://assets.nflxext.com/web/",
                "https://ae.nflximg.net/monet/scripts/adtech_iframe",
            ];
            for pattern in &patterns {
                trie.insert(pattern);
            }
            trie
        };
}

// Block netflix events that are not required
pub fn block_netflix(
    event: &chromiumoxide_cdp::cdp::browser_protocol::fetch::EventRequestPaused,
) -> bool {
    URL_IGNORE_TRIE.contains_prefix(&event.request.url)
}
