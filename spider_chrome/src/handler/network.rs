use super::blockers::{
    scripts::{
        URL_IGNORE_EMBEDED_TRIE, URL_IGNORE_SCRIPT_BASE_PATHS, URL_IGNORE_SCRIPT_STYLES_PATHS,
        URL_IGNORE_TRIE,
    },
    xhr::{IGNORE_XHR_ASSETS, URL_IGNORE_XHR_MEDIA_TRIE, URL_IGNORE_XHR_TRIE},
    Trie,
};
use crate::auth::Credentials;
use crate::cmd::CommandChain;
use crate::handler::http::HttpRequest;
use case_insensitive_string::CaseInsensitiveString;
use chromiumoxide_cdp::cdp::browser_protocol::fetch::{
    self, AuthChallengeResponse, AuthChallengeResponseResponse, ContinueRequestParams,
    ContinueWithAuthParams, DisableParams, EventAuthRequired, EventRequestPaused, RequestPattern,
};
use chromiumoxide_cdp::cdp::browser_protocol::network::{
    EmulateNetworkConditionsParams, EventLoadingFailed, EventLoadingFinished,
    EventRequestServedFromCache, EventRequestWillBeSent, EventResponseReceived, Headers,
    InterceptionId, RequestId, ResourceType, Response, SetCacheDisabledParams,
    SetExtraHttpHeadersParams,
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
        ];
        for pattern in &patterns {
            trie.insert(pattern);
        }
        trie
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
    /// medium.com
    Medium,
    /// upwork.com,
    Upwork,
    /// glassdoor.com
    Glassdoor,
    /// ebay.com
    Ebay,
    /// nytimes.com
    Nytimes,
    #[default]
    /// Unknown
    Unknown,
}

lazy_static! {
    /// Top tier list of the most common websites visited.
    pub static ref TOP_TIER_LIST: [(&'static str, NetworkInterceptManager); 20] = [
        ("https://www.tiktok.com", NetworkInterceptManager::TikTok),
        ("https://tiktok.com", NetworkInterceptManager::TikTok),
        ("https://www.amazon.", NetworkInterceptManager::Amazon),
        ("https://amazon.", NetworkInterceptManager::Amazon),
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
        ("https://www.glassdoor.", NetworkInterceptManager::Glassdoor),
        ("https://glassdoor.", NetworkInterceptManager::Glassdoor),
        ("https://www.medium.com", NetworkInterceptManager::Medium),
        ("https://medium.com", NetworkInterceptManager::Medium),
        ("https://www.ebay.", NetworkInterceptManager::Ebay),
        ("https://ebay.", NetworkInterceptManager::Ebay),
        ("https://www.nytimes.com", NetworkInterceptManager::Nytimes),
        ("https://nytimes.com", NetworkInterceptManager::Nytimes),
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

                            if !ignore_script
                                && URL_IGNORE_SCRIPT_BASE_PATHS.contains_prefix(new_url)
                            {
                                ignore_script = true;
                            }

                            if !ignore_script
                                && self.ignore_visuals
                                && URL_IGNORE_SCRIPT_STYLES_PATHS.contains_prefix(new_url)
                            {
                                ignore_script = true;
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
                            NetworkInterceptManager::Medium => {
                                super::blockers::medium_blockers::block_medium(event)
                            }
                            NetworkInterceptManager::Ebay => {
                                super::blockers::ebay_blockers::block_ebay(event)
                            }
                            NetworkInterceptManager::Nytimes => {
                                super::blockers::nytimes_blockers::block_nytimes(
                                    event,
                                    self.ignore_visuals,
                                )
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
                            NetworkInterceptManager::Medium => {
                                super::blockers::medium_blockers::block_medium(event)
                            }
                            NetworkInterceptManager::Ebay => {
                                super::blockers::ebay_blockers::block_ebay(event)
                            }
                            NetworkInterceptManager::Nytimes => {
                                super::blockers::nytimes_blockers::block_nytimes(
                                    event,
                                    self.ignore_visuals,
                                )
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
