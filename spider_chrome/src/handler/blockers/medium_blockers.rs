use crate::handler::blockers::Trie;

lazy_static::lazy_static! {
        /// Ignore list of urls.
        static ref URL_IGNORE_TRIE: Trie = {
            let mut trie = Trie::new();
            let patterns = [
                "https://cdn-client.medium.com/lite/static/js/instrumentation.",
                "https://medium.com/_/clientele/reports/performance/",
                "https://cdn-client.medium.com/lite/static/js/reporting.f",
                "https://medium.com/_/clientele/reports/performance/",
                "https://cdn-client.medium.com/lite/static/js/manifest.",
                "clientele/reports/performance/",
                "https://www.google.com/js/bg/",
                "https://chitaranjanbiswal93.medium.com/_/clientele/reports/performance/"
            ];
            for pattern in &patterns {
                trie.insert(pattern);
            }
            trie
        };
}

// Block medium events that are not required
pub fn block_medium(
    event: &chromiumoxide_cdp::cdp::browser_protocol::fetch::EventRequestPaused,
) -> bool {
    URL_IGNORE_TRIE.contains_prefix(&event.request.url)
}
