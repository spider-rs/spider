use super::blockers::{
    block_websites::block_xhr, ignore_script_embedded, ignore_script_xhr, ignore_script_xhr_media,
    xhr::IGNORE_XHR_ASSETS,
};
use crate::auth::Credentials;
use crate::cmd::CommandChain;
use crate::handler::http::HttpRequest;
use aho_corasick::AhoCorasick;
use case_insensitive_string::CaseInsensitiveString;
use chromiumoxide_cdp::cdp::browser_protocol::network::{
    EmulateNetworkConditionsParams, EventLoadingFailed, EventLoadingFinished,
    EventRequestServedFromCache, EventRequestWillBeSent, EventResponseReceived, Headers,
    InterceptionId, RequestId, ResourceType, Response, SetCacheDisabledParams,
    SetExtraHttpHeadersParams,
};
use chromiumoxide_cdp::cdp::browser_protocol::{
    fetch::{
        self, AuthChallengeResponse, AuthChallengeResponseResponse, ContinueRequestParams,
        ContinueWithAuthParams, DisableParams, EventAuthRequired, EventRequestPaused,
        RequestPattern,
    },
    network::SetBypassServiceWorkerParams,
};
use chromiumoxide_cdp::cdp::browser_protocol::{
    network::EnableParams, security::SetIgnoreCertificateErrorsParams,
};
use chromiumoxide_types::{Command, Method, MethodId};
use hashbrown::{HashMap, HashSet};
use lazy_static::lazy_static;
use reqwest::header::PROXY_AUTHORIZATION;
use spider_network_blocker::intercept_manager::NetworkInterceptManager;
pub use spider_network_blocker::scripts::{
    URL_IGNORE_SCRIPT_BASE_PATHS, URL_IGNORE_SCRIPT_STYLES_PATHS, URL_IGNORE_TRIE_PATHS,
};
use std::collections::VecDeque;
use std::time::Duration;

lazy_static! {
    /// General patterns for popular libraries and resources
    static ref JS_FRAMEWORK_ALLOW: Vec<&'static str> = vec![
        "jquery",           // Covers jquery.min.js, jquery.js, etc.
        "angular",
        "react",            // Covers all React-related patterns
        "vue",              // Covers all Vue-related patterns
        "bootstrap",
        "d3",
        "lodash",
        "ajax",
        "application",
        "app",              // Covers general app scripts like app.js
        "main",
        "index",
        "bundle",
        "vendor",
        "runtime",
        "polyfill",
        "scripts",
        "es2015.",
        "es2020.",
        "webpack",
        "/wp-content/js/",  // Covers Wordpress content
        // Verified 3rd parties for request
        "https://m.stripe.network/",
        "https://challenges.cloudflare.com/",
        "https://www.google.com/recaptcha/api.js",
        "https://google.com/recaptcha/api.js",
        "https://js.stripe.com/",
        "https://cdn.prod.website-files.com/", // webflow cdn scripts
        "https://cdnjs.cloudflare.com/",        // cloudflare cdn scripts
        "https://code.jquery.com/jquery-"
    ];

    /// Determine if a script should be rendered in the browser by name.
    pub static ref ALLOWED_MATCHER: AhoCorasick = AhoCorasick::new(JS_FRAMEWORK_ALLOW.iter()).unwrap();

    /// path of a js framework
    pub static ref JS_FRAMEWORK_PATH: phf::Set<&'static str> = {
        phf::phf_set! {
            // Add allowed assets from JS_FRAMEWORK_ASSETS except the excluded ones
            "_astro/", "_app/immutable"
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
        "application/rtf",
    };

    /// Ignore the resources for visual content types.
    pub static ref IGNORE_VISUAL_RESOURCE_MAP: phf::Set<&'static str> = phf::phf_set! {
        "Image",
        "Media",
        "Font"
    };

    /// Ignore the resources for visual content types.
    pub static ref IGNORE_NETWORKING_RESOURCE_MAP: phf::Set<&'static str> = phf::phf_set! {
        "CspViolationReport",
        "Manifest",
        "Other",
        "Prefetch",
        "Ping",
    };

    /// Case insenstive css matching
    pub static ref CSS_EXTENSION: CaseInsensitiveString = CaseInsensitiveString::from("css");

    /// The command chain.
    pub static ref INIT_CHAIN: Vec<(std::borrow::Cow<'static, str>, serde_json::Value)>  = {
        let enable = EnableParams::default();

        if let Ok(c) = serde_json::to_value(&enable) {
            vec![(enable.identifier(), c)]
        } else {
            vec![]
        }
    };

    /// The command chain with https ignore.
    pub static ref INIT_CHAIN_IGNORE_HTTP_ERRORS: Vec<(std::borrow::Cow<'static, str>, serde_json::Value)>  = {
        let enable = EnableParams::default();
        let mut v = vec![];
        if let Ok(c) = serde_json::to_value(&enable) {
            v.push((enable.identifier(), c));
        }
        let ignore = SetIgnoreCertificateErrorsParams::new(true);
        if let Ok(ignored) = serde_json::to_value(&ignore) {
            v.push((ignore.identifier(), ignored));
        }

        v
    };

    /// Enable the fetch intercept command
    pub static ref ENABLE_FETCH: chromiumoxide_cdp::cdp::browser_protocol::fetch::EnableParams = {
        fetch::EnableParams::builder()
        .handle_auth_requests(true)
        .pattern(RequestPattern::builder().url_pattern("*").build())
        .build()
    };
}

/// Determine if a redirect is true.
pub(crate) fn is_redirect_status(status: i64) -> bool {
    matches!(status, 301 | 302 | 303 | 307 | 308)
}

#[derive(Debug)]
/// The base network manager.
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
    // unused atm for remote connections, needs to be used for self launches.
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
    /// Is xml document?
    pub xml_document: bool,
    /// The custom intercept handle logic to run on the website.
    pub intercept_manager: NetworkInterceptManager,
    /// Track the amount of times the document reloaded.
    pub document_reload_tracker: u8,
    /// The initial target domain.
    pub document_target_domain: String,
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
            xml_document: false,
            intercept_manager: NetworkInterceptManager::Unknown,
            document_reload_tracker: 0,
            document_target_domain: String::new(),
        }
    }

    pub fn init_commands(&self) -> CommandChain {
        let cmds = if self.ignore_httpserrors {
            INIT_CHAIN_IGNORE_HTTP_ERRORS.clone()
        } else {
            INIT_CHAIN.clone()
        };

        CommandChain::new(cmds, self.request_timeout)
    }

    pub(crate) fn push_cdp_request<T: Command>(&mut self, cmd: T) {
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
        self.extra_headers.remove(PROXY_AUTHORIZATION.as_str());
        self.extra_headers.remove("Proxy-Authorization");
        if let Ok(headers) = serde_json::to_value(&self.extra_headers) {
            self.push_cdp_request(SetExtraHttpHeadersParams::new(Headers::new(headers)));
        }
    }

    pub fn set_service_worker_enabled(&mut self, bypass: bool) {
        self.push_cdp_request(SetBypassServiceWorkerParams::new(bypass));
    }

    pub fn set_request_interception(&mut self, enabled: bool) {
        self.user_request_interception_enabled = enabled;
        self.update_protocol_request_interception();
    }

    pub fn set_cache_enabled(&mut self, enabled: bool) {
        let run = self.user_cache_disabled != !enabled;
        self.user_cache_disabled = !enabled;
        if run {
            self.update_protocol_cache_disabled();
        }
    }

    pub fn disable_request_intercept(&mut self) {
        self.protocol_request_interception_enabled = true;
    }

    pub fn update_protocol_cache_disabled(&mut self) {
        self.push_cdp_request(SetCacheDisabledParams::new(self.user_cache_disabled));
    }

    pub fn authenticate(&mut self, credentials: Credentials) {
        self.credentials = Some(credentials);
        self.update_protocol_request_interception();
        self.protocol_request_interception_enabled = true;
    }

    fn update_protocol_request_interception(&mut self) {
        let enabled = self.user_request_interception_enabled || self.credentials.is_some();

        if enabled == self.protocol_request_interception_enabled {
            return;
        }

        if enabled {
            self.push_cdp_request(ENABLE_FETCH.clone())
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
        let mut ignore_script = block_analytics
            && spider_network_blocker::scripts::URL_IGNORE_TRIE.contains_prefix(url);

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
        if !ignore_script && block_analytics {
            ignore_script = URL_IGNORE_TRIE_PATHS.contains_prefix(url);
        }

        ignore_script
    }

    /// Determine if the request should be skipped.
    fn skip_xhr(
        &self,
        skip_networking: bool,
        event: &EventRequestPaused,
        network_event: bool,
    ) -> bool {
        // XHR check
        if !skip_networking && network_event {
            let request_url = event.request.url.as_str();

            // check if part of ignore scripts.
            let skip_analytics =
                self.block_analytics && (ignore_script_xhr(request_url) || block_xhr(request_url));

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
        use super::blockers::block_websites::block_website;

        if self.user_request_interception_enabled && self.protocol_request_interception_enabled {
            return;
        }

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
                    let document_resource = event.resource_type == ResourceType::Document;
                    let network_resource = !document_resource
                        && (event.resource_type == ResourceType::Xhr
                            || event.resource_type == ResourceType::Fetch
                            || event.resource_type == ResourceType::WebSocket);

                    let skip_networking =
                        IGNORE_NETWORKING_RESOURCE_MAP.contains(event.resource_type.as_ref());

                    let skip_networking = skip_networking || self.document_reload_tracker >= 3;
                    let mut replacer = None;

                    if document_resource {
                        if self.document_target_domain == current_url {
                            // this will prevent the domain from looping (3 times is enough).
                            self.document_reload_tracker += 1;
                        } else if !self.document_target_domain.is_empty()
                            && event.redirected_request_id.is_some()
                        {
                            let (http_document_replacement, mut https_document_replacement) =
                                if self.document_target_domain.starts_with("http://") {
                                    (
                                        self.document_target_domain.replace("http://", "http//"),
                                        self.document_target_domain.replace("http://", "https://"),
                                    )
                                } else {
                                    (
                                        self.document_target_domain.replace("https://", "https//"),
                                        self.document_target_domain.replace("https://", "http://"),
                                    )
                                };

                            let trailing = https_document_replacement.ends_with('/');

                            if trailing {
                                https_document_replacement.pop();
                            }

                            if https_document_replacement.ends_with('/') {
                                https_document_replacement.pop();
                            }

                            let redirect_mask = format!(
                                "{}{}",
                                https_document_replacement, http_document_replacement
                            );

                            // handle redirect masking
                            if current_url == redirect_mask {
                                replacer = Some(if trailing {
                                    format!("{}/", https_document_replacement)
                                } else {
                                    https_document_replacement
                                });
                            }
                        }

                        if self.document_target_domain.is_empty() && current_url.ends_with(".xml") {
                            self.xml_document = true;
                        }

                        self.document_target_domain = event.request.url.clone();
                    }

                    let current_url = match &replacer {
                        Some(r) => r,
                        _ => &event.request.url,
                    }
                    .as_str();

                    // main initial check
                    let skip_networking = if !skip_networking {
                        // allow sitemap xml building xsl
                        if self.xml_document && current_url.ends_with(".xsl") {
                            false
                        } else {
                            self.ignore_visuals
                                && (IGNORE_VISUAL_RESOURCE_MAP
                                    .contains(event.resource_type.as_ref()))
                                || self.block_stylesheets
                                    && ResourceType::Stylesheet == event.resource_type
                                || self.block_javascript
                                    && javascript_resource
                                    && self.intercept_manager == NetworkInterceptManager::Unknown
                                    && !ALLOWED_MATCHER.is_match(current_url)
                        }
                    } else {
                        skip_networking
                    };

                    let skip_networking = if !skip_networking
                        && (self.only_html || self.ignore_visuals)
                        && (javascript_resource || document_resource)
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
                    let skip_networking = self.skip_xhr(skip_networking, &event, network_resource);

                    // custom interception layer.
                    let skip_networking = if !skip_networking
                        && (javascript_resource || network_resource || document_resource)
                    {
                        self.intercept_manager.intercept_detection(
                            &current_url,
                            self.ignore_visuals,
                            network_resource,
                        )
                    } else {
                        skip_networking
                    };

                    let skip_networking =
                        if !skip_networking && (javascript_resource || network_resource) {
                            block_website(&current_url)
                        } else {
                            skip_networking
                        };

                    if skip_networking {
                        tracing::debug!("Blocked: {:?} - {}", event.resource_type, current_url);
                        let fullfill_params =
                            crate::handler::network::fetch::FulfillRequestParams::new(
                                event.request_id.clone(),
                                200,
                            );
                        self.push_cdp_request(fullfill_params);
                    } else {
                        tracing::debug!("Allowed: {:?} - {}", event.resource_type, current_url);
                        let mut continue_params =
                            ContinueRequestParams::new(event.request_id.clone());

                        if replacer.is_some() {
                            continue_params.url = Some(current_url.into());
                            continue_params.intercept_response = Some(false);
                        }

                        self.push_cdp_request(continue_params)
                    }
                }
            } else {
                self.push_cdp_request(ContinueRequestParams::new(event.request_id.clone()))
            }
        }
    }

    #[cfg(feature = "adblock")]
    pub fn on_fetch_request_paused(&mut self, event: &EventRequestPaused) {
        if self.user_request_interception_enabled && self.protocol_request_interception_enabled {
            return;
        }

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
                    let document_resource = event.resource_type == ResourceType::Document;
                    let network_resource = !document_resource
                        && (event.resource_type == ResourceType::Xhr
                            || event.resource_type == ResourceType::Fetch
                            || event.resource_type == ResourceType::WebSocket);
                    let mut replacer = None;

                    // block all of these events.
                    let skip_networking =
                        IGNORE_NETWORKING_RESOURCE_MAP.contains(event.resource_type.as_ref());

                    let skip_networking = skip_networking || self.document_reload_tracker >= 3;

                    if document_resource {
                        if self.document_target_domain == current_url {
                            // this will prevent the domain from looping (3 times is enough).
                            self.document_reload_tracker += 1;
                        } else if !self.document_target_domain.is_empty()
                            && event.redirected_request_id.is_some()
                        {
                            let (http_document_replacement, mut https_document_replacement) =
                                if self.document_target_domain.starts_with("http://") {
                                    (
                                        self.document_target_domain.replace("http://", "http//"),
                                        self.document_target_domain.replace("http://", "https://"),
                                    )
                                } else {
                                    (
                                        self.document_target_domain.replace("https://", "https//"),
                                        self.document_target_domain.replace("https://", "http://"),
                                    )
                                };

                            let trailing = https_document_replacement.ends_with('/');

                            if trailing {
                                https_document_replacement.pop();
                            }

                            if https_document_replacement.ends_with('/') {
                                https_document_replacement.pop();
                            }

                            let redirect_mask = format!(
                                "{}{}",
                                https_document_replacement, http_document_replacement
                            );

                            // handle redirect masking
                            if current_url == redirect_mask {
                                replacer = Some(if trailing {
                                    format!("{}/", https_document_replacement)
                                } else {
                                    https_document_replacement
                                });
                            }
                        }

                        if self.document_target_domain.is_empty() && current_url.ends_with(".xml") {
                            self.xml_document = true;
                        }

                        self.document_target_domain = event.request.url.clone();
                    }

                    let current_url = match &replacer {
                        Some(r) => r,
                        _ => &event.request.url,
                    }
                    .as_str();

                    // main initial check
                    let skip_networking = if !skip_networking {
                        // allow sitemap xml building xsl
                        if self.xml_document && current_url.ends_with(".xsl") {
                            false
                        } else {
                            self.ignore_visuals
                                && (IGNORE_VISUAL_RESOURCE_MAP
                                    .contains(event.resource_type.as_ref()))
                                || self.block_stylesheets
                                    && ResourceType::Stylesheet == event.resource_type
                                || self.block_javascript
                                    && javascript_resource
                                    && self.intercept_manager == NetworkInterceptManager::Unknown
                                    && !ALLOWED_MATCHER.is_match(current_url)
                        }
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
                        && (javascript_resource || document_resource)
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
                    let skip_networking = self.skip_xhr(skip_networking, &event, network_resource);

                    // custom interception layer.
                    let skip_networking = if !skip_networking
                        && (javascript_resource || network_resource || document_resource)
                    {
                        self.intercept_manager.intercept_detection(
                            &event.request.url,
                            self.ignore_visuals,
                            network_resource,
                        )
                    } else {
                        skip_networking
                    };

                    let skip_networking = if !skip_networking
                        && (javascript_resource || network_resource)
                    {
                        crate::handler::blockers::block_websites::block_website(&event.request.url)
                    } else {
                        skip_networking
                    };

                    if skip_networking {
                        tracing::debug!("Blocked: {:?} - {}", event.resource_type, current_url);

                        let fullfill_params =
                            crate::handler::network::fetch::FulfillRequestParams::new(
                                event.request_id.clone(),
                                200,
                            );
                        self.push_cdp_request(fullfill_params);
                    } else {
                        tracing::debug!("Allowed: {:?} - {}", event.resource_type, current_url);

                        let mut continue_params =
                            ContinueRequestParams::new(event.request_id.clone());

                        if replacer.is_some() {
                            continue_params.url = Some(current_url.into());
                            continue_params.intercept_response = Some(false);
                        }
                    }
                }
            } else {
                self.push_cdp_request(ContinueRequestParams::new(event.request_id.clone()))
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
                    &*spider_network_blocker::adblock::ADBLOCK_PATTERNS,
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
        let mut redirect_location = None;

        if let Some(redirect_resp) = &event.redirect_response {
            if let Some(mut request) = self.requests.remove(event.request_id.as_ref()) {
                if is_redirect_status(redirect_resp.status) {
                    if let Some(location) = redirect_resp.headers.inner()["Location"].as_str() {
                        if redirect_resp.url != location {
                            let fixed_location = location.replace(&redirect_resp.url, "");

                            request.response.as_mut().map(|resp| {
                                resp.headers.0["Location"] =
                                    serde_json::Value::String(fixed_location.clone());
                            });

                            redirect_location = Some(fixed_location);
                        }
                    }
                }

                self.handle_request_redirect(
                    &mut request,
                    if let Some(redirect_location) = redirect_location {
                        let mut redirect_resp = redirect_resp.clone();

                        redirect_resp.headers.0["Location"] =
                            serde_json::Value::String(redirect_location);

                        redirect_resp
                    } else {
                        redirect_resp.clone()
                    },
                );

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
