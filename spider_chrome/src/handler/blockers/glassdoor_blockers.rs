use crate::handler::blockers::Trie;

lazy_static::lazy_static! {
        /// Ignore list of urls.
        static ref URL_IGNORE_TRIE: Trie = {
            let mut trie = Trie::new();
            let patterns = [
                "https://www.glassdoor.com/garnish/static/js/gd-sw-register.",
                "https://cdnjs.cloudflare.com/ajax/libs/prop-types/15.7.2/prop-types.min.js",
                "https://www.glassdoor.com/autocomplete/location?",
            ];
            for pattern in &patterns {
                trie.insert(pattern);
            }
            trie
        };

        /// Ignore list of urls styles.
        static ref URL_IGNORE_TRIE_STYLES: Trie = {
            let mut trie = Trie::new();
            let patterns = [
                "https://www.glassdoor.com/sam-global-nav/static/",
                "https://www.glassdoor.com/garnish/static/js/gd-",
                "https://unpkg.com/@dotlottie/player-component@",
                "https://www.glassdoor.com/job-search-next/assets/_next/static/",
                "https://www.glassdoor.com/ei-overview-next/assets/_next/static/",
                "https://www.glassdoor.com/occ-salaries-web/assets/_next/static/"
            ];
            for pattern in &patterns {
                trie.insert(pattern);
            }
            trie
        };
}

// Block glassdoor events that are not required
pub fn block_glassdoor_styles(
    event: &chromiumoxide_cdp::cdp::browser_protocol::fetch::EventRequestPaused,
) -> bool {
    URL_IGNORE_TRIE_STYLES.contains_prefix(&event.request.url)
}

// Block glassdoor events that are not required
pub fn block_glassdoor(
    event: &chromiumoxide_cdp::cdp::browser_protocol::fetch::EventRequestPaused,
    ignore_visuals: bool,
) -> bool {
    let blocked = URL_IGNORE_TRIE.contains_prefix(&event.request.url);
    if !blocked && ignore_visuals {
        block_glassdoor_styles(event)
    } else {
        blocked
    }
}
