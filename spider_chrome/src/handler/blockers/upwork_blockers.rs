use crate::handler::blockers::Trie;

lazy_static::lazy_static! {
        /// Ignore list of urls.
        static ref URL_IGNORE_TRIE: Trie = {
            let mut trie = Trie::new();
            let patterns = [
                "https://www.upwork.com/shitake/suit",
                "https://www.upwork.com/upi/jslogger",
                "https://mpsnare.iesnare.com/5.8.1/logo.js",
                "https://first.iovation.com/",
                "https://zn0izjiulta2j2t4o-upwork.siteintercept.qualtrics.com/",
                "https://cdn123.forter.com/",
                "https://www.upwork.com/static/assets/TopNavSsi/visitor-v2/js/manifest.",
                "https://www.upwork.com/iojs/general5/static_wdp.js",
                "https://www.upwork.com/static/suit2-tracker/",
                "https://www.upwork.com/api/graphql/v1?alias=spellCheck",
                "https://www.upwork.com/api/graphql/v1?alias=relatedSuggestions",
                "https://www.upwork.com/api/graphql/v1?alias=autoSuggestions",
                ".siteintercept.qualtrics.com/",
                ".forter.com",
            ];
            for pattern in &patterns {
                trie.insert(pattern);
            }
            trie
        };

        /// Ignore list of urls.
        static ref URL_IGNORE_TRIE_STYLES: Trie = {
            let mut trie = Trie::new();
            let patterns = [
                "https://www.upwork.com/static/assets/TopNavSsi/visitor-v2/",
                // 1 missing link needs further looking into for each of the styles
                "https://www.upwork.com/static/assets/UniversalSearchNuxt/styles~",
                "https://www.upwork.com/static/assets/Brontes/styles",
                "https://www.upwork.com/static/assets/Brontes/google-one-tap.6226625d.js"

            ];
            for pattern in &patterns {
                trie.insert(pattern);
            }
            trie
        };
}

// Block upwork events that are not required
pub fn block_upwork_styles(
    event: &chromiumoxide_cdp::cdp::browser_protocol::fetch::EventRequestPaused,
) -> bool {
    URL_IGNORE_TRIE_STYLES.contains_prefix(&event.request.url)
}

// Block upwork events that are not required
pub fn block_upwork(
    event: &chromiumoxide_cdp::cdp::browser_protocol::fetch::EventRequestPaused,
    ignore_visuals: bool,
) -> bool {
    let blocked = URL_IGNORE_TRIE.contains_prefix(&event.request.url);
    if !blocked && ignore_visuals {
        block_upwork_styles(event)
    } else {
        blocked
    }
}
