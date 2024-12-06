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
                "https://aax-us-east-retail-direct.amazon.com/e/xsp/getAd",
                "https://fls-na.amazon.com/1/batch/1/OE/",
                "https://unagi.amazon.com/1/events/",
                "https://images-na.ssl-images-amazon.com/images/S/apesafeframe/ape/sf/desktop/",
                // ads
                "https://m.media-amazon.com/images/G/01/csm/showads",
                // we can prob search for rum subs uptop instead.
                "https://dataplane.rum",
                "https://client.rum",
                ".amazon-adsystem.com",
                "SearchPartnerAssets",
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
        let u = &event.request.url;

        if u.ends_with("?pageViewLogging=1")
            || u.starts_with("https://s.amazon-adsystem.com/")
            || u.ends_with("inner-host.min.js")
            || u.ends_with(".js?xcp")
            || u.contains(".amazon-adsystem.com/")
        {
            block_request = true;
        }
    }

    block_request
}
