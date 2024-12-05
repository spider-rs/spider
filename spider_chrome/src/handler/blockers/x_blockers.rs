use crate::handler::blockers::Trie;

lazy_static::lazy_static! {
        /// Ignore list of urls.
        static ref URL_IGNORE_TRIE: Trie = {
            let mut trie = Trie::new();
            let patterns = [
                "https://accounts.google.com/gsi/",
                "https://appleid.cdn-apple.com/appleauth/static/jsapi/appleid/1/en_US/appleid.auth.js",
                "https://api.x.com/1.1/onboarding/sso_init.json",
                "https://api.x.com/1.1/jot/client_event.json",
                "https://api.x.com/1.1/jot/error_log.json",
                "https://api.x.com/1.1/hashflags.json",
                // "https://abs.twimg.com/responsive-web/client-web/"
            ];
            for pattern in &patterns {
                trie.insert(pattern);
            }
            trie
        };
}

// Block x events that are not required
pub fn block_x(
    event: &chromiumoxide_cdp::cdp::browser_protocol::fetch::EventRequestPaused,
) -> bool {
    URL_IGNORE_TRIE.contains_prefix(&event.request.url)
}
