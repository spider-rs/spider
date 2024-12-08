use super::blockers::Trie;
use crate::auth::Credentials;
use crate::cmd::CommandChain;
use crate::handler::http::HttpRequest;
use case_insensitive_string::CaseInsensitiveString;
use chromiumoxide_cdp::cdp::browser_protocol::fetch::{
    self, AuthChallengeResponse, AuthChallengeResponseResponse, ContinueRequestParams,
    ContinueWithAuthParams, DisableParams, EventAuthRequired, EventRequestPaused, RequestPattern,
};
use chromiumoxide_cdp::cdp::browser_protocol::network::ResourceType;
use chromiumoxide_cdp::cdp::browser_protocol::network::{
    EmulateNetworkConditionsParams, EventLoadingFailed, EventLoadingFinished,
    EventRequestServedFromCache, EventRequestWillBeSent, EventResponseReceived, Headers,
    InterceptionId, RequestId, Response, SetCacheDisabledParams, SetExtraHttpHeadersParams,
};
use chromiumoxide_cdp::cdp::browser_protocol::{
    network::EnableParams, security::SetIgnoreCertificateErrorsParams,
};
use chromiumoxide_types::{Command, Method, MethodId};
use hashbrown::{HashMap, HashSet};
use lazy_static::lazy_static;
use std::collections::VecDeque;
use std::time::Duration;

lazy_static! {
    /// allowed js frameworks and libs excluding some and adding additional URLs
    pub static ref JS_FRAMEWORK_ALLOW: phf::Set<&'static str> = {
        phf::phf_set! {
            // Add allowed assets from JS_FRAMEWORK_ASSETS except the excluded ones
            "jquery.min.js", "jquery.qtip.min.js", "jquery.js", "angular.js", "jquery.slim.js",
            "react.development.js", "react-dom.development.js", "react.production.min.js",
            "react-dom.production.min.js", "vue.global.js", "vue.esm-browser.js", "vue.js",
            "bootstrap.min.js", "bootstrap.bundle.min.js", "bootstrap.esm.min.js", "d3.min.js",
            "d3.js", "lodash.min.js", "lodash.js",
            "app.js", "main.js", "index.js", "bundle.js", "vendor.js",
            // Verified 3rd parties for request
            "https://m.stripe.network/inner.html",
            "https://m.stripe.network/out-4.5.43.js",
            "https://challenges.cloudflare.com/turnstile",
            "https://js.stripe.com/v3/"
        }
    };

    /// path of a js framework
    pub static ref JS_FRAMEWORK_PATH: phf::Set<&'static str> = {
        phf::phf_set! {
            // Add allowed assets from JS_FRAMEWORK_ASSETS except the excluded ones
            "_next/static/", "_astro/",
        }
    };

    /// Ignore the content types.
    pub static ref IGNORE_CONTENT_TYPES: phf::Set<&'static str> = phf::phf_set! {
        "application/pdf",
        "application/zip",
        "application/x-rar-compressed",
        "application/x-tar",
        "image/png",
        "image/jpeg",
        "image/gif",
        "image/bmp",
        "image/svg+xml",
        "video/mp4",
        "video/x-msvideo",
        "video/x-matroska",
        "video/webm",
        "audio/mpeg",
        "audio/ogg",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "application/vnd.ms-excel",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "application/vnd.ms-powerpoint",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "application/x-7z-compressed",
        "application/x-rpm",
        "application/x-shockwave-flash",
    };

    /// Ignore the resources for visual content types.
    pub static ref IGNORE_VISUAL_RESOURCE_MAP: phf::Set<&'static str> = phf::phf_set! {
        "Image",
        "Media",
        "Font"
    };

    /// Ignore the resources for visual content types.
    pub static ref IGNORE_NETWORKING_RESOURCE_MAP: phf::Set<&'static str> = phf::phf_set! {
        "Prefetch",
        "Ping",
    };
    /// Ignore list of scripts.
    static ref URL_IGNORE_TRIE: Trie = {
        let mut trie = Trie::new();
        let patterns = [
            "https://www.googletagservices.com/tag/",
            "https://js.hs-analytics.net/analytics/",
            "https://www.googletagmanager.com/gtag",
            "https://www.googletagmanager.com/gtm.js",
            "https://js.hsadspixel.net",
            "https://www.google.com/adsense/",
            "https://www.googleadservices.com",
            "https://static.cloudflareinsights.com/",
            "https://adservice.google.com",
            "https://www.gstatic.com/cv/js/sender/",
            "https://googleads.g.doubleclick.net",
            "https://www.google-analytics.com",
            "https://iabusprivacy.pmc.com/geo-info.js",
            "https://cookie-cdn.cookiepro.com/consent",
            "https://w.usabilla.com/",
            "https://consentcdn.cookiebot.com/",
            "https://plausible.io/api/event",
            "https://sentry.io/api/",
            "https://cdn.onesignal.com",
            "https://cdn.cookielaw.org/",
            "https://static.doubleclick.net",
            "https://cdn.piano.io",
            "https://px.ads.linkedin.com",
            "https://connect.facebook.net",
            "https://tags.tiqcdn.com",
            "https://tr.snapchat.com",
            "https://ads.twitter.com",
            "https://cdn.segment.com",
            "https://stats.wp.com",
            "https://analytics.",
            "http://analytics.",
            "https://cdn.cxense.com",
            "https://cdn.tinypass.com",
            "https://cd.connatix.com",
            "https://platform-api.sharethis.com/js/sharethis.js",
            "https://js.hsforms.net/forms/embed/v2.js",
            "https://static.parastorage.com/services/wix-thunderbolt/dist/",
            "https://static.parastorage.com/services/tag-manager-client/",
            "https://www.datadoghq-browser-agent.com/datadog-rum-slim-v4.js",
            "https://cdn.rudderlabs.com",
            "https://script.hotjar.com/",
            "https://static.hotjar.com/",
            "https://cdn.insurads.com/",
            "https://cdn-ukwest.onetrust.com",
            "https://cdn.onetrust.com",
            "https://services.insurads.com/",
            "https://platform.iteratehq.com/loader.js",
            "https://acdn.adnxs.com/ast/ast.js",
            "https://schibsted-cdn.relevant-digital.com/static/tags/",
            "https://bat.bing.net",
            "https://static.addtoany.com/menu/",
            "https://www.b2i.us/b2i/",
            "https://acsbapp.com/apps/app/dist/js/app.js",
            ".sharethis.com",
            ".newrelic.com",
            ".googlesyndication.com",
            ".amazon-adsystem.com",
            ".onetrust.com",
            "sc.omtrdc.net",
            "doubleclick.net",
            "hotjar.com",
            "datadome.com",
            "datadog-logs-us.js",
            "tinypass.min.js",
            ".airship.com",
            ".adlightning.com",
            ".lab.amplitude.",
            // explicit ignore tracking.js and ad files
            "privacy-notice.js",
            "tracking.js",
            "ads.js",
            "insight.min.js",
            "https://ads.",
            "http://ads.",
            "https://tracking.",
            "http://tracking.",
            // exp testin
            // used for possible location outside
            "https://geo.privacymanager.io/",
            // "https://www.recaptcha.net/recaptcha/",
            // "https://www.google.com/recaptcha/",
            // "https://www.gstatic.com/recaptcha/",
        ];
        for pattern in &patterns {
            trie.insert(pattern);
        }
        trie
    };

     /// Ignore list of scripts paths.
     static ref URL_IGNORE_TRIE_PATHS: Trie = {
        let mut trie = Trie::new();
        let patterns = [
            // explicit ignore tracking.js and ad files
            "privacy-notice.js",
            "tracking.js",
            "track.js",
            "ads.js",
            "analytics.js",
            "otSDKStub.js",
            "otBannerSdk.js",
            "_vercel/insights/script.js",
            "analytics.",
            "cookie-law-info-ccpa.js"
        ];
        for pattern in &patterns {
            trie.insert(pattern);
        }
        trie
    };


    /// Ignore list of XHR urls.
    static ref URL_IGNORE_XHR_TRIE: Trie = {
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
            ".wixapps.net/api/v1/bulklog",
            // video embeddings
            "https://video.squarespace-cdn.com/content/",
            "googlesyndication.com",
            ".doubleclick.net",
            ".piano.io/",
            ".browsiprod.com",
            ".onetrust.",
            "https://logs.",
            "/track.php",
            "/api/v1/bulklog",
            "cookieconsentpub",
            "cookie-law-info",
            "mediaelement-and-player.min.j"
        ];
        for pattern in &patterns {
            trie.insert(pattern);
        }
        trie
    };

    /// Ignore list of scripts embedded or font extra.
    static ref URL_IGNORE_EMBEDED_TRIE: Trie = {
        let mut trie = Trie::new();
        let patterns = [
            "https://www.youtube.com/embed/",      // YouTube video embeds
            "https://www.google.com/maps/embed?",  // Google Maps embeds
            "https://player.vimeo.com/video/",     // Vimeo video embeds
            "https://open.spotify.com/embed/",     // Spotify music embeds
            "https://w.soundcloud.com/player/",    // SoundCloud embeds
            "https://platform.twitter.com/embed/", // Twitter embedded tweets
            "https://www.instagram.com/embed.js",  // Instagram embeds
            "https://www.facebook.com/plugins/",   // Facebook embeds (like posts and videos)
            "https://cdn.embedly.com/widgets/",    // Embedly embeds
            "https://player.twitch.tv/",           // Twitch video player embeds
            "https://maps.googleapis.com/maps/", // Google map embeds
            "https://www.youtube.com/player_api", // Youtube player.
            "https://www.googletagmanager.com/ns.html", // Google tag manager.
            "https://consentcdn.cookiebot.com", // Cookie bot
            "https://www.youtube.com/iframe_api", // Youtube iframes.
            "https://f.vimeocdn.com", // Vimeo EMBEDDINGS
            "https://i.vimeocdn.com/",
            // "https://www.youtube.com/s/player/", // Youtube player not needed usually since iframe_api is used mainly
            // vercel live
            "https://vercel.live/api/",

            // extra CDN scripts
            "https://cdn.readme.io/public/",
            // insight tracker
            "https://insight.adsrvr.org/track/",
            "cxense.com/",
            // snapchat tracker
            "https://tr.snapchat.com/",
            "https://buy.tinypass.com",
            "https://nimbleplot.com/",
            "https://my.actiondata.co/js/tracker.php",
            // ignore font extras
            "https://kit.fontawesome.com/",
            "https://use.typekit.net",
            ".amplitude.com",
            ".rudderstack.com",
            // ignore tailwind cdn
            "https://cdn.tailwindcss.com",
            // ignore extra ads
            ".sharethis.com",
            "amazon-adsystem.com",
            ".vimeocdn.com",
            "g.doubleclick.net",
            "https://securepubads.g.doubleclick.net",
            "googlesyndication.com",
            "adsafeprotected.com",
            // more google tracking
            ".googlesyndication.com/safeframe/",
            // repeat consent js
            "/ccpa/user-consent.min.js",
            "/cookiebanner/js/",
            "cookielaw.org",
            // privacy
            "otBannerSdk.js",
            "privacy-notice.js",
            ".ingest.sentry.io/api",
            // ignore amazon scripts for media
            ".ssl-images-amazon.com/images/"
        ];
        for pattern in &patterns {
            trie.insert(pattern);
        }
        trie
    };

    /// Ignore list of XHR urls for media.
    static ref URL_IGNORE_XHR_MEDIA_TRIE: Trie = {
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

    /// Case insenstive css matching
    pub static ref CSS_EXTENSION: CaseInsensitiveString = CaseInsensitiveString::from("css");

}

/// Url matches analytics that we want to ignore or trackers.
pub(crate) fn ignore_script_embedded(url: &str) -> bool {
    URL_IGNORE_EMBEDED_TRIE.contains_prefix(url)
}

/// Url matches analytics that we want to ignore or trackers.
pub(crate) fn ignore_script_xhr(url: &str) -> bool {
    URL_IGNORE_XHR_TRIE.contains_prefix(url)
}

/// Url matches media that we want to ignore.
pub(crate) fn ignore_script_xhr_media(url: &str) -> bool {
    URL_IGNORE_XHR_MEDIA_TRIE.contains_prefix(url)
}

/// Custom network intercept types to expect on a domain
#[derive(Debug, Default, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq)]
pub enum NetworkInterceptManager {
    /// tiktok.com
    TikTok,
    /// facebook.com
    Facebook,
    /// amazon.com
    Amazon,
    /// x.com
    X,
    /// LinkedIn,
    LinkedIn,
    /// netflix.com
    Netflix,
    /// upwork.com,
    Upwork,
    /// glassdoor.com
    Glassdoor,
    #[default]
    /// Unknown
    Unknown,
}

lazy_static! {
    /// Top tier list of the most common websites visited.
    pub static ref TOP_TIER_LIST: [(&'static str, NetworkInterceptManager); 14] = [
        ("https://www.tiktok.com", NetworkInterceptManager::TikTok),
        ("https://tiktok.com", NetworkInterceptManager::TikTok),
        ("https://www.amazon.com", NetworkInterceptManager::Amazon),
        ("https://amazon.com", NetworkInterceptManager::Amazon),
        ("https://www.x.com", NetworkInterceptManager::X),
        ("https://x.com", NetworkInterceptManager::X),
        ("https://www.netflix.com", NetworkInterceptManager::Netflix),
        ("https://netflix.com", NetworkInterceptManager::Netflix),
        (
            "https://www.linkedin.com",
            NetworkInterceptManager::LinkedIn
        ),
        ("https://linkedin.com", NetworkInterceptManager::LinkedIn),
        ("https://www.upwork.com", NetworkInterceptManager::Upwork),
        ("https://upwork.com", NetworkInterceptManager::Upwork),
        ("https://www.glassdoor.com", NetworkInterceptManager::Glassdoor),
        ("https://glassdoor.com", NetworkInterceptManager::Glassdoor),
    ];
}

impl NetworkInterceptManager {
    /// a custom intercept handle.
    pub fn new(url: &str) -> NetworkInterceptManager {
        TOP_TIER_LIST
            .iter()
            .find(|&(pattern, _)| url.starts_with(pattern))
            .map(|&(_, manager_type)| manager_type)
            .unwrap_or(NetworkInterceptManager::Unknown)
    }
    /// Setup the intercept handle
    pub fn setup(&mut self, url: &str) -> Self {
        NetworkInterceptManager::new(url)
    }
}

#[derive(Debug)]
pub struct NetworkManager {
    queued_events: VecDeque<NetworkEvent>,
    ignore_httpserrors: bool,
    requests: HashMap<RequestId, HttpRequest>,
    // TODO put event in an Arc?
    requests_will_be_sent: HashMap<RequestId, EventRequestWillBeSent>,
    extra_headers: std::collections::HashMap<String, String>,
    request_id_to_interception_id: HashMap<RequestId, InterceptionId>,
    user_cache_disabled: bool,
    attempted_authentications: HashSet<RequestId>,
    credentials: Option<Credentials>,
    user_request_interception_enabled: bool,
    protocol_request_interception_enabled: bool,
    offline: bool,
    request_timeout: Duration,
    // made_request: bool,
    /// Ignore visuals (no pings, prefetching, and etc).
    pub ignore_visuals: bool,
    /// Block CSS stylesheets.
    pub block_stylesheets: bool,
    /// Block javascript that is not critical to rendering.
    pub block_javascript: bool,
    /// Block analytics from rendering
    pub block_analytics: bool,
    /// Only html from loading.
    pub only_html: bool,
    /// The custom intercept handle logic to run on the website.
    pub intercept_manager: NetworkInterceptManager,
}

impl NetworkManager {
    pub fn new(ignore_httpserrors: bool, request_timeout: Duration) -> Self {
        Self {
            queued_events: Default::default(),
            ignore_httpserrors,
            requests: Default::default(),
            requests_will_be_sent: Default::default(),
            extra_headers: Default::default(),
            request_id_to_interception_id: Default::default(),
            user_cache_disabled: false,
            attempted_authentications: Default::default(),
            credentials: None,
            user_request_interception_enabled: false,
            protocol_request_interception_enabled: false,
            offline: false,
            request_timeout,
            ignore_visuals: false,
            block_javascript: false,
            block_stylesheets: false,
            block_analytics: true,
            only_html: false,
            intercept_manager: NetworkInterceptManager::Unknown,
        }
    }

    pub fn init_commands(&self) -> CommandChain {
        let enable = EnableParams::default();
        let mut v = vec![];

        if let Ok(c) = serde_json::to_value(&enable) {
            v.push((enable.identifier(), c));
        }

        let cmds = if self.ignore_httpserrors {
            let ignore = SetIgnoreCertificateErrorsParams::new(true);

            if let Ok(ignored) = serde_json::to_value(&ignore) {
                v.push((ignore.identifier(), ignored));
            }

            v
        } else {
            v
        };

        CommandChain::new(cmds, self.request_timeout)
    }

    fn push_cdp_request<T: Command>(&mut self, cmd: T) {
        let method = cmd.identifier();
        if let Ok(params) = serde_json::to_value(cmd) {
            self.queued_events
                .push_back(NetworkEvent::SendCdpRequest((method, params)));
        }
    }

    /// The next event to handle
    pub fn poll(&mut self) -> Option<NetworkEvent> {
        self.queued_events.pop_front()
    }

    pub fn extra_headers(&self) -> &std::collections::HashMap<String, String> {
        &self.extra_headers
    }

    pub fn set_extra_headers(&mut self, headers: std::collections::HashMap<String, String>) {
        self.extra_headers = headers;
        self.extra_headers.remove("proxy-authorization");
        if let Ok(headers) = serde_json::to_value(&self.extra_headers) {
            self.push_cdp_request(SetExtraHttpHeadersParams::new(Headers::new(headers)));
        }
    }

    pub fn set_request_interception(&mut self, enabled: bool) {
        self.user_request_interception_enabled = enabled;
        self.update_protocol_request_interception();
    }

    pub fn set_cache_enabled(&mut self, enabled: bool) {
        self.user_cache_disabled = !enabled;
        self.update_protocol_cache_disabled();
    }

    pub fn update_protocol_cache_disabled(&mut self) {
        self.push_cdp_request(SetCacheDisabledParams::new(
            self.user_cache_disabled || self.protocol_request_interception_enabled,
        ));
    }

    pub fn authenticate(&mut self, credentials: Credentials) {
        self.credentials = Some(credentials);
        self.update_protocol_request_interception()
    }

    fn update_protocol_request_interception(&mut self) {
        let enabled = self.user_request_interception_enabled || self.credentials.is_some();

        if enabled == self.protocol_request_interception_enabled {
            return;
        }
        self.update_protocol_cache_disabled();

        if enabled {
            self.push_cdp_request(
                fetch::EnableParams::builder()
                    .handle_auth_requests(true)
                    .pattern(RequestPattern::builder().url_pattern("*").build())
                    .build(),
            )
        } else {
            self.push_cdp_request(DisableParams::default())
        }
    }

    /// Url matches analytics that we want to ignore or trackers.
    pub(crate) fn ignore_script(
        &self,
        url: &str,
        block_analytics: bool,
        intercept_manager: NetworkInterceptManager,
    ) -> bool {
        let mut ignore_script = block_analytics && URL_IGNORE_TRIE.contains_prefix(url);

        if !ignore_script {
            if let Some(index) = url.find("//") {
                let pos = index + 2;

                // Ensure there is something after `//`
                if pos < url.len() {
                    // Find the first slash after the `//`
                    if let Some(slash_index) = url[pos..].find('/') {
                        let base_path_index = pos + slash_index + 1;

                        if url.len() > base_path_index {
                            let new_url: &str = &url[base_path_index..];
                            ignore_script = URL_IGNORE_TRIE_PATHS.contains_prefix(new_url);

                            // ignore assets we do not need for frameworks
                            if !ignore_script
                                && intercept_manager == NetworkInterceptManager::Unknown
                            {
                                let hydration_file =
                                    JS_FRAMEWORK_PATH.iter().any(|p| new_url.starts_with(p));

                                // ignore astro paths
                                if hydration_file && new_url.ends_with(".js") {
                                    ignore_script = true;
                                }
                            }
                        }
                    }
                }
            }
        }

        // fallback for file ending in analytics.js
        if !ignore_script {
            ignore_script = url.ends_with("analytics.js")
                || url.ends_with("ads.js")
                || url.ends_with("tracking.js")
                || url.ends_with("track.js");
        }

        ignore_script
    }

    /// Determine if the request should be skipped.
    fn skip_xhr(&self, skip_networking: bool, event: &EventRequestPaused) -> bool {
        // XHR check
        if !skip_networking
            && (event.resource_type == ResourceType::Xhr
                || event.resource_type == ResourceType::WebSocket
                || event.resource_type == ResourceType::Fetch)
        {
            let request_url = event.request.url.as_str();

            // check if part of ignore scripts.
            let skip_analytics = self.block_analytics && ignore_script_xhr(request_url);

            if skip_analytics {
                true
            } else if self.block_stylesheets || self.ignore_visuals {
                let block_css = self.block_stylesheets;
                let block_media = self.ignore_visuals;

                let mut block_request = false;

                if let Some(position) = request_url.rfind('.') {
                    let hlen = request_url.len();
                    let has_asset = hlen - position;

                    if has_asset >= 3 {
                        let next_position = position + 1;

                        if block_media
                            && IGNORE_XHR_ASSETS.contains::<CaseInsensitiveString>(
                                &request_url[next_position..].into(),
                            )
                        {
                            block_request = true;
                        } else if block_css {
                            block_request =
                                CaseInsensitiveString::from(request_url[next_position..].as_bytes())
                                    .contains(&**CSS_EXTENSION)
                        }
                    }
                }

                if !block_request {
                    block_request = ignore_script_xhr_media(request_url);
                }

                block_request
            } else {
                skip_networking
            }
        } else {
            skip_networking
        }
    }

    #[cfg(not(feature = "adblock"))]
    pub fn on_fetch_request_paused(&mut self, event: &EventRequestPaused) {
        if !self.user_request_interception_enabled && self.protocol_request_interception_enabled {
            self.push_cdp_request(ContinueRequestParams::new(event.request_id.clone()))
        } else {
            if let Some(network_id) = event.network_id.as_ref() {
                if let Some(request_will_be_sent) =
                    self.requests_will_be_sent.remove(network_id.as_ref())
                {
                    self.on_request(&request_will_be_sent, Some(event.request_id.clone().into()));
                } else {
                    let current_url = event.request.url.as_str();
                    let javascript_resource = event.resource_type == ResourceType::Script;
                    let skip_networking = event.resource_type == ResourceType::Other
                        || event.resource_type == ResourceType::Manifest
                        || event.resource_type == ResourceType::CspViolationReport
                        || event.resource_type == ResourceType::Ping
                        || event.resource_type == ResourceType::Prefetch;
                    let network_resource = event.resource_type == ResourceType::Xhr
                        || event.resource_type == ResourceType::Fetch
                        || event.resource_type == ResourceType::WebSocket;

                    // main initial check
                    let skip_networking = if !skip_networking {
                        IGNORE_NETWORKING_RESOURCE_MAP.contains(event.resource_type.as_ref())
                            || self.ignore_visuals
                                && (IGNORE_VISUAL_RESOURCE_MAP
                                    .contains(event.resource_type.as_ref()))
                            || self.block_stylesheets
                                && ResourceType::Stylesheet == event.resource_type
                            || self.block_javascript
                                && javascript_resource
                                && !JS_FRAMEWORK_ALLOW.contains(current_url)
                    } else {
                        skip_networking
                    };

                    let skip_networking = if !skip_networking
                        && (self.only_html || self.ignore_visuals)
                        && (javascript_resource || event.resource_type == ResourceType::Document)
                    {
                        ignore_script_embedded(current_url)
                    } else {
                        skip_networking
                    };

                    // analytics check
                    let skip_networking = if !skip_networking && javascript_resource {
                        self.ignore_script(
                            current_url,
                            self.block_analytics,
                            self.intercept_manager,
                        )
                    } else {
                        skip_networking
                    };

                    // XHR check
                    let skip_networking = self.skip_xhr(skip_networking, &event);

                    // custom interception layer.
                    let skip_networking = if !skip_networking
                        && (javascript_resource
                            || network_resource
                            || event.resource_type == ResourceType::Document)
                    {
                        match self.intercept_manager {
                            NetworkInterceptManager::TikTok => {
                                super::blockers::tiktok_blockers::block_tiktok(event)
                            }
                            NetworkInterceptManager::Amazon => {
                                super::blockers::amazon_blockers::block_amazon(event)
                            }
                            NetworkInterceptManager::X => {
                                super::blockers::x_blockers::block_x(event)
                            }
                            NetworkInterceptManager::Netflix => {
                                super::blockers::netflix_blockers::block_netflix(event)
                            }
                            NetworkInterceptManager::LinkedIn => {
                                super::blockers::linkedin_blockers::block_linkedin(event)
                            }
                            NetworkInterceptManager::Glassdoor => {
                                super::blockers::glassdoor_blockers::block_glassdoor(
                                    event,
                                    self.ignore_visuals,
                                )
                            }
                            NetworkInterceptManager::Upwork => {
                                super::blockers::upwork_blockers::block_upwork(
                                    event,
                                    self.ignore_visuals,
                                )
                            }
                            _ => skip_networking,
                        }
                    } else {
                        skip_networking
                    };

                    if skip_networking {
                        let fullfill_params =
                            crate::handler::network::fetch::FulfillRequestParams::new(
                                event.request_id.clone(),
                                200,
                            );
                        self.push_cdp_request(fullfill_params);
                    } else {
                        tracing::debug!(
                            "Network Allowed: {:?} - {}",
                            event.resource_type,
                            event.request.url
                        );
                        self.push_cdp_request(ContinueRequestParams::new(event.request_id.clone()))
                    }
                }
            } else {
                self.push_cdp_request(ContinueRequestParams::new(event.request_id.clone()))
            }
        }
    }

    #[cfg(feature = "adblock")]
    pub fn on_fetch_request_paused(&mut self, event: &EventRequestPaused) {
        if !self.user_request_interception_enabled && self.protocol_request_interception_enabled {
            self.push_cdp_request(ContinueRequestParams::new(event.request_id.clone()))
        } else {
            if let Some(network_id) = event.network_id.as_ref() {
                if let Some(request_will_be_sent) =
                    self.requests_will_be_sent.remove(network_id.as_ref())
                {
                    self.on_request(&request_will_be_sent, Some(event.request_id.clone().into()));
                } else {
                    let current_url = event.request.url.as_str();
                    let javascript_resource = event.resource_type == ResourceType::Script;
                    let skip_networking = event.resource_type == ResourceType::Other
                        || event.resource_type == ResourceType::Manifest
                        || event.resource_type == ResourceType::CspViolationReport
                        || event.resource_type == ResourceType::Ping
                        || event.resource_type == ResourceType::Prefetch;
                    let network_resource = event.resource_type == ResourceType::Xhr
                        || event.resource_type == ResourceType::Fetch
                        || event.resource_type == ResourceType::WebSocket;

                    // main initial check
                    let skip_networking = if !skip_networking {
                        IGNORE_NETWORKING_RESOURCE_MAP.contains(event.resource_type.as_ref())
                            || self.ignore_visuals
                                && (IGNORE_VISUAL_RESOURCE_MAP
                                    .contains(event.resource_type.as_ref()))
                            || self.block_stylesheets
                                && ResourceType::Stylesheet == event.resource_type
                            || self.block_javascript
                                && javascript_resource
                                && !JS_FRAMEWORK_ALLOW.contains(current_url)
                    } else {
                        skip_networking
                    };

                    let skip_networking = if !skip_networking {
                        self.detect_ad(event)
                    } else {
                        skip_networking
                    };

                    let skip_networking = if !skip_networking
                        && (self.only_html || self.ignore_visuals)
                        && (javascript_resource || event.resource_type == ResourceType::Document)
                    {
                        ignore_script_embedded(current_url)
                    } else {
                        skip_networking
                    };

                    // analytics check
                    let skip_networking = if !skip_networking && javascript_resource {
                        self.ignore_script(
                            current_url,
                            self.block_analytics,
                            self.intercept_manager,
                        )
                    } else {
                        skip_networking
                    };

                    // XHR check
                    let skip_networking = self.skip_xhr(skip_networking, &event);

                    // custom interception layer.
                    let skip_networking = if !skip_networking
                        && (javascript_resource
                            || network_resource
                            || event.resource_type == ResourceType::Document)
                    {
                        match self.intercept_manager {
                            NetworkInterceptManager::TikTok => {
                                super::blockers::tiktok_blockers::block_tiktok(event)
                            }
                            NetworkInterceptManager::Amazon => {
                                super::blockers::amazon_blockers::block_amazon(event)
                            }
                            NetworkInterceptManager::X => {
                                super::blockers::x_blockers::block_x(event)
                            }
                            NetworkInterceptManager::Netflix => {
                                super::blockers::netflix_blockers::block_netflix(event)
                            }
                            NetworkInterceptManager::LinkedIn => {
                                super::blockers::linkedin_blockers::block_linkedin(event)
                            }
                            NetworkInterceptManager::Glassdoor => {
                                super::blockers::glassdoor_blockers::block_glassdoor(
                                    event,
                                    self.ignore_visuals,
                                )
                            }
                            NetworkInterceptManager::Upwork => {
                                super::blockers::upwork_blockers::block_upwork(
                                    event,
                                    self.ignore_visuals,
                                )
                            }
                            _ => skip_networking,
                        }
                    } else {
                        skip_networking
                    };

                    if skip_networking {
                        let fullfill_params =
                            crate::handler::network::fetch::FulfillRequestParams::new(
                                event.request_id.clone(),
                                200,
                            );
                        self.push_cdp_request(fullfill_params);
                    } else {
                        self.push_cdp_request(ContinueRequestParams::new(event.request_id.clone()))
                    }
                }
            }
        }

        // if self.only_html {
        //     self.made_request = true;
        // }
    }

    /// Perform a page intercept for chrome
    #[cfg(feature = "adblock")]
    pub fn detect_ad(&self, event: &EventRequestPaused) -> bool {
        use adblock::{
            lists::{FilterSet, ParseOptions, RuleTypes},
            Engine,
        };

        lazy_static::lazy_static! {
            static ref AD_ENGINE: Engine = {
                let mut filter_set = FilterSet::new(false);
                let mut rules = ParseOptions::default();
                rules.rule_types = RuleTypes::All;

                filter_set.add_filters(
                    &*crate::handler::blockers::adblock_patterns::ADBLOCK_PATTERNS,
                    rules,
                );

                Engine::from_filter_set(filter_set, true)
            };
        };

        let blockable = ResourceType::Image == event.resource_type
            || event.resource_type == ResourceType::Media
            || event.resource_type == ResourceType::Stylesheet
            || event.resource_type == ResourceType::Document
            || event.resource_type == ResourceType::Fetch
            || event.resource_type == ResourceType::Xhr;

        let u = &event.request.url;

        let block_request = blockable
            // set it to example.com for 3rd party handling is_same_site
        && {
            let request = adblock::request::Request::preparsed(
                 &u,
                 "example.com",
                 "example.com",
                 &event.resource_type.as_ref().to_lowercase(),
                 !event.request.is_same_site.unwrap_or_default());

            AD_ENGINE.check_network_request(&request).matched
        };

        block_request
    }

    pub fn on_fetch_auth_required(&mut self, event: &EventAuthRequired) {
        let response = if self
            .attempted_authentications
            .contains(event.request_id.as_ref())
        {
            AuthChallengeResponseResponse::CancelAuth
        } else if self.credentials.is_some() {
            self.attempted_authentications
                .insert(event.request_id.clone().into());
            AuthChallengeResponseResponse::ProvideCredentials
        } else {
            AuthChallengeResponseResponse::Default
        };

        let mut auth = AuthChallengeResponse::new(response);
        if let Some(creds) = self.credentials.clone() {
            auth.username = Some(creds.username);
            auth.password = Some(creds.password);
        }
        self.push_cdp_request(ContinueWithAuthParams::new(event.request_id.clone(), auth));
    }

    pub fn set_offline_mode(&mut self, value: bool) {
        if self.offline == value {
            return;
        }
        self.offline = value;
        if let Ok(network) = EmulateNetworkConditionsParams::builder()
            .offline(self.offline)
            .latency(0)
            .download_throughput(-1.)
            .upload_throughput(-1.)
            .build()
        {
            self.push_cdp_request(network);
        }
    }

    /// Request interception doesn't happen for data URLs with Network Service.
    pub fn on_request_will_be_sent(&mut self, event: &EventRequestWillBeSent) {
        if self.protocol_request_interception_enabled && !event.request.url.starts_with("data:") {
            if let Some(interception_id) = self
                .request_id_to_interception_id
                .remove(event.request_id.as_ref())
            {
                self.on_request(event, Some(interception_id));
            } else {
                // TODO remove the clone for event
                self.requests_will_be_sent
                    .insert(event.request_id.clone(), event.clone());
            }
        } else {
            self.on_request(event, None);
        }
    }

    pub fn on_request_served_from_cache(&mut self, event: &EventRequestServedFromCache) {
        if let Some(request) = self.requests.get_mut(event.request_id.as_ref()) {
            request.from_memory_cache = true;
        }
    }

    pub fn on_response_received(&mut self, event: &EventResponseReceived) {
        if let Some(mut request) = self.requests.remove(event.request_id.as_ref()) {
            request.set_response(event.response.clone());
            self.queued_events
                .push_back(NetworkEvent::RequestFinished(request))
        }
    }

    pub fn on_network_loading_finished(&mut self, event: &EventLoadingFinished) {
        if let Some(request) = self.requests.remove(event.request_id.as_ref()) {
            if let Some(interception_id) = request.interception_id.as_ref() {
                self.attempted_authentications
                    .remove(interception_id.as_ref());
            }
            self.queued_events
                .push_back(NetworkEvent::RequestFinished(request));
        }
    }

    pub fn on_network_loading_failed(&mut self, event: &EventLoadingFailed) {
        if let Some(mut request) = self.requests.remove(event.request_id.as_ref()) {
            request.failure_text = Some(event.error_text.clone());
            if let Some(interception_id) = request.interception_id.as_ref() {
                self.attempted_authentications
                    .remove(interception_id.as_ref());
            }
            self.queued_events
                .push_back(NetworkEvent::RequestFailed(request));
        }
    }

    fn on_request(
        &mut self,
        event: &EventRequestWillBeSent,
        interception_id: Option<InterceptionId>,
    ) {
        let mut redirect_chain = Vec::new();
        if let Some(redirect_resp) = event.redirect_response.as_ref() {
            if let Some(mut request) = self.requests.remove(event.request_id.as_ref()) {
                self.handle_request_redirect(&mut request, redirect_resp.clone());
                redirect_chain = std::mem::take(&mut request.redirect_chain);
                redirect_chain.push(request);
            }
        }
        let request = HttpRequest::new(
            event.request_id.clone(),
            event.frame_id.clone(),
            interception_id,
            self.user_request_interception_enabled,
            redirect_chain,
        );

        self.requests.insert(event.request_id.clone(), request);
        self.queued_events
            .push_back(NetworkEvent::Request(event.request_id.clone()));
    }

    fn handle_request_redirect(&mut self, request: &mut HttpRequest, response: Response) {
        request.set_response(response);
        if let Some(interception_id) = request.interception_id.as_ref() {
            self.attempted_authentications
                .remove(interception_id.as_ref());
        }
    }
}

#[derive(Debug)]
pub enum NetworkEvent {
    SendCdpRequest((MethodId, serde_json::Value)),
    Request(RequestId),
    Response(RequestId),
    RequestFailed(HttpRequest),
    RequestFinished(HttpRequest),
}
