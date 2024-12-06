use crate::handler::blockers::Trie;

lazy_static::lazy_static! {
        /// Ignore list of urls.
        static ref URL_IGNORE_TRIE: Trie = {
            let mut trie = Trie::new();
            let patterns = [
                "/log",
                "https://www.linkedin.com/li/track",
                "https://li.protechts.net",
                "https://www.linkedin.com/platform-telemetry/li",
                "https://www.linkedin.com/organization-guest/api/feedUpdates/",
                "https://www.linkedin.com/feedcontent-guest/api/ingraphs/gauge",
                "https://www.linkedin.com/voyager/api/",
                "https://platform.linkedin.com/litms/allowlist/voyager-web-global"
            ];
            for pattern in &patterns {
                trie.insert(pattern);
            }
            trie
        };
}

// Block linkedin events that are not required
pub fn block_linkedin(
    event: &chromiumoxide_cdp::cdp::browser_protocol::fetch::EventRequestPaused,
) -> bool {
    URL_IGNORE_TRIE.contains_prefix(&event.request.url)
}
