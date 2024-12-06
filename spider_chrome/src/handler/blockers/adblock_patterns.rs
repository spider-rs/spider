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
            // Tracking patterns
            "-tracking.",
            "-tracking/script.",
            ".tracking",
            ".snowplowanalytics.snowplow",
            "dx.mountain.com",
            "tracking.js",
            "track.js",

            // Analytics scripts
            "analytics.js",

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
