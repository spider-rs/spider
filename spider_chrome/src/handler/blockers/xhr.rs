use crate::handler::blockers::Trie;
use case_insensitive_string::CaseInsensitiveString;
use hashbrown::HashSet;

lazy_static::lazy_static! {
    /// Ignore list of XHR urls for media.
    pub (crate) static ref URL_IGNORE_XHR_MEDIA_TRIE: Trie = {
        let mut trie = Trie::new();
        let patterns = [
            "https://www.youtube.com/s/player/",
            "https://www.vimeo.com/player/",
            "https://soundcloud.com/player/",
            "https://open.spotify.com/",
            "https://api.spotify.com/v1/",
            "https://music.apple.com/"
        ];
        for pattern in &patterns {
            trie.insert(pattern);
        }
        trie
    };


    /// Visual assets to ignore for XHR request.
    pub(crate) static ref IGNORE_XHR_ASSETS: HashSet<CaseInsensitiveString> = {
        let mut m: HashSet<CaseInsensitiveString> = HashSet::with_capacity(36);

        m.extend([
            "jpg", "jpeg", "png", "gif", "svg", "webp",       // Image files
            "mp4", "avi", "mov", "wmv", "flv",               // Video files
            "mp3", "wav", "ogg",                             // Audio files
            "woff", "woff2", "ttf", "otf",                   // Font files
            "swf", "xap",                                    // Flash/Silverlight files
            "ico", "eot",                                    // Other resource files

            // Including extensions with extra dot
            ".jpg", ".jpeg", ".png", ".gif", ".svg", ".webp",
            ".mp4", ".avi", ".mov", ".wmv", ".flv",
            ".mp3", ".wav", ".ogg",
            ".woff", ".woff2", ".ttf", ".otf",
            ".swf", ".xap",
            ".ico", ".eot"
        ].map(|s| s.into()));

        m
    };


    /// Ignore list of XHR urls.
    pub(crate) static ref URL_IGNORE_XHR_TRIE: Trie = {
        let mut trie = Trie::new();
        let patterns = [
            "https://play.google.com/log?",
            "https://googleads.g.doubleclick.net/pagead/id",
            "https://js.monitor.azure.com/scripts",
            "https://securepubads.g.doubleclick.net",
            "https://pixel-config.reddit.com/pixels",
            // amazon product feedback
            "https://www.amazon.com/af/feedback-link?",
            "https://www.google.com/ads/ga-audiences",
            "https://player.vimeo.com/video/",
            "https://www.youtube.com/iframe_api",
            "https://tr.snapchat.com/config/",
            "https://collect.tealiumiq.com/",
            "https://cdn.acsbapp.com/config/",
            "https://s.yimg.com/wi",
            "https://disney.my.sentry.io/api/",
            "https://www.redditstatic.com/ads",
            "https://sentry.io/api/",
            "https://buy.tinypass.com/",
            "https://idx.liadm.com",
            "https://geo.privacymanager.io/",
            "https://nimbleplot.com",
            "https://api.lab.amplitude.com/",
            "https://flag.lab.amplitude.com/sdk/v2/flags",
            "https://cdn-ukwest.onetrust.com/",
            "https://cdn.onetrust.com/",
            "https://geolocation.onetrust.com/",
            "https://assets.adobedtm.com/",
            "https://sdkconfig.pulse.",
            "https://bat.bing.net",
            "https://api.reviews.io/",
            "https://ads.rubiconproject.com/",
            "https://api.config-security.com/event",
            "https://conf.config-security.com/model",
            "https://sumome.com/api/load/",
            ".wixapps.net/api/v1/bulklog",
            "https://error-analytics-sessions-production.shopifysvc.com/",
            "https://static-forms.",
            // video embeddings
            "https://video.squarespace-cdn.com/content/",
            "googlesyndication.com",
            ".doubleclick.net",
            ".doofinder.com",
            ".piano.io/",
            ".browsiprod.com",
            ".onetrust.",
            "https://logs.",
            "/track.php",
            "/api/v1/bulklog",
            "cookieconsentpub",
            "cookie-law-info",
            "mediaelement-and-player.min.j",
        ];
        for pattern in &patterns {
            trie.insert(pattern);
        }
        trie
    };

}
