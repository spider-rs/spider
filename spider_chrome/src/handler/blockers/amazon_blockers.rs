use crate::handler::blockers::Trie;

lazy_static::lazy_static! {
        /// Ignore list of urls.
        static ref URL_IGNORE_TRIE: Trie = {
            let mut trie = Trie::new();
            let patterns = [
                // images
                // "https://m.media-amazon.com",
                // "https://images-na.ssl-images-amazon.com/images/",
                // analytics and ads
                "https://cognito-identity.us-east-1.amazonaws.com",
                "https://completion.amazon.com/api/2017/suggestions",
                "https://sts.us-east-1.amazonaws.com/",
                "https://www.amazon.com/cross_border_interstitial_sp/render",

                "https://fls-na.amazon.com/1/batch/1/OE/",
                "https://unagi.amazon.com/1/events/",
                // ads
                "https://m.media-amazon.com/images/G/01/csm/showads",
                // we can prob search for rum subs uptop instead.
                "https://dataplane.rum",
                "https://client.rum",
                ".amazon-adsystem.com"
            ];
            for pattern in &patterns {
                trie.insert(pattern);
            }
            trie
        };
}

// Block amazon events that are not required
pub fn block_amazon(
    event: &chromiumoxide_cdp::cdp::browser_protocol::fetch::EventRequestPaused,
) -> bool {
    let mut block_request = URL_IGNORE_TRIE.contains_prefix(&event.request.url);

    if !block_request {
        if event.request.url.ends_with("?pageViewLogging=1")
            || event
                .request
                .url
                .starts_with("https://s.amazon-adsystem.com/")
            || event.request.url.contains(".amazon-adsystem.com/")
        {
            block_request = true;
        }
    }

    block_request
}
