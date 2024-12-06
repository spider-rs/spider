lazy_static::lazy_static! {
    pub static ref ADBLOCK_PATTERNS: Vec<&'static str> = {
        let adblock_patterns = vec![
            // Advertisement patterns
            "-advertisement.",
            "-advertisement-icon.",
            "-advertisement-management/",
            "-advertisement/script.",
            "-ads.",
            "-ads/script.",
            "-ad.",
            "ads.js",
            "gtm.js?",
            "googletagmanager.com",
            "ssl.google-analytics.com",
            // Tracking patterns
            "-tracking.",
            "-tracking/script.",
            ".tracking",
            ".snowplowanalytics.snowplow",
            ".mountain.com",
            "tracking.js",
            "track.js",
            "/upi/jslogger",
            "otBannerSdk.js",
            // Analytics scripts
            "analytics.js",
            "ob.cityrobotflower.com",
            "siteintercept.qualtrics.com",
            "iesnare.com",
            "iovation.com",
            "googletagmanager.com",
            "forter.com",
            "/first.iovation.com",
            // Specific ad and tracking domains
            "googlesyndication.com",
            ".googlesyndication.com/safeframe/",
            "adsafeprotected.com",
            "cxense.com/",
            ".sharethis.com",
            "amazon-adsystem.com",
            "g.doubleclick.net",
            // Explicit ignore for common scripts
            "privacy-notice.js",
            "insight.min.js",
        ];

        adblock_patterns
    };
}
