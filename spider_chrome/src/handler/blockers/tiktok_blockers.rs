use crate::handler::blockers::Trie;

lazy_static::lazy_static! {
        /// Ignore list of urls.
        static ref URL_IGNORE_TRIE: Trie = {
            let mut trie = Trie::new();
            let patterns = [
                "https://mcs.tiktokw.us/v1/list",
                "https://www.tiktok.com/ttwid/check",
                "https://www.tiktok.com/api/share/settings",
                "https://webcast.us.tiktok.com/",
                "https://www.tiktok.com/api/ba/business/suite/permission/list",
                "https://www.tiktok.com/api/policy/notice/",
                "https://www.tiktok.com/api/v1/web-cookie-privacy",
                "https://www.tiktok.com/aweme/v1/report/inbox/notice",
                "https://www.tiktok.com/api/inbox/notice_count/",
                "https://mcs.tiktokv.us/v1/user/webid",
                "https://mon16-normal-useast5.tiktokv.us/monitor_browser/collect/batch/?bid=tiktok_pns_web_runtime",
                "https://webcast.tiktok.com/webcast/wallet_api/fs/diamond_buy",
                "https://lf16-tiktok-web.tiktokcdn-us.com/obj/tiktok-web-tx/tiktok_privacy_protection_framework/loader/",
                "https://lf16-tiktok-web.tiktokcdn-us.com/obj/tiktok-web-tx/tiktok/webapp/main/webapp-desktop/npm-async-bric_verify_sec_sdk_build_captcha",
                "/tiktok_privacy_protection_framework/loader",
                "/obj/tiktok-web-tx/tiktok_privacy_protection_framework/loader",
                "/service/2/abtest_config/",
                "collect/batch/?bid=tiktok_pns_web_runtime",
                // "https://libraweb.tiktokw.us/service/2/abtest_config/",
                // "https://lf16-cdn-tos.tiktokcdn-us.com/obj/static-tx/secsdk/secsdk-lastest.umd.js",
                "monitor_browser/collect/batch/?bid=tiktok_pns_web_runtime",
                "/tiktok-cookie-banner/",
                // custom framework
                "/tiktok/webapp/main/webapp-desktop-islands/npm-async-bric_verify_sec_sdk_build_captcha_",
            ];
            for pattern in &patterns {
                trie.insert(pattern);
            }
            trie
        };
}

// Block tiktok events that are not required
pub fn block_tiktok(
    event: &chromiumoxide_cdp::cdp::browser_protocol::fetch::EventRequestPaused,
) -> bool {
    URL_IGNORE_TRIE.contains_prefix(&event.request.url)
}
