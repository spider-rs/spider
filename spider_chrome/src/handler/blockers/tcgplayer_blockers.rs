use crate::handler::blockers::Trie;

lazy_static::lazy_static! {
        /// Ignore list of urls.
        static ref URL_IGNORE_TRIE: Trie = {
            let mut trie = Trie::new();
            let patterns = [
                "https://data.tcgplayer.com/suggestions/trending",
                "https://mpapi.tcgplayer.com/v2/kickbacks?active=true",
                "https://homepage.marketplace.tcgplayer.com/sitealert.json",
                "https://infinite-api.tcgplayer.com/signup/?",
                "https://features.tcgplayer.com/v1/optimizely/Variation/",
                "https://mpapi.tcgplayer.com/v2/address/countryCodes?mpfev=3031"
            ];
            for pattern in &patterns {
                trie.insert(pattern);
            }
            trie
        };
}

// Block tcgplayer events that are not required
pub fn block_tcgplayer(
    event: &chromiumoxide_cdp::cdp::browser_protocol::fetch::EventRequestPaused,
) -> bool {
    URL_IGNORE_TRIE.contains_prefix(&event.request.url)
}
