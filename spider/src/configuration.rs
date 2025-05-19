use crate::compact_str::CompactString;
use crate::features::chrome_common::RequestInterceptConfiguration;
pub use crate::features::chrome_common::{
    AuthChallengeResponse, AuthChallengeResponseResponse, AutomationScripts, AutomationScriptsMap,
    CaptureScreenshotFormat, CaptureScreenshotParams, ClipViewport, ExecutionScripts,
    ExecutionScriptsMap, ScreenShotConfig, ScreenshotParams, Viewport, WaitFor, WaitForDelay,
    WaitForIdleNetwork, WaitForSelector, WebAutomation,
};
pub use crate::features::openai_common::GPTConfigs;
use crate::utils::get_domain_from_url;
use crate::website::CronType;
use reqwest::header::{AsHeaderName, HeaderMap, HeaderName, HeaderValue, IntoHeaderName};
use std::time::Duration;

#[cfg(feature = "chrome")]
pub use spider_fingerprint::Fingerprint;

/// Redirect policy configuration for request
#[derive(Debug, Default, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum RedirectPolicy {
    #[default]
    #[cfg_attr(
        feature = "serde",
        serde(alias = "Loose", alias = "loose", alias = "LOOSE",)
    )]
    /// A loose policy that allows all request up to the redirect limit.
    Loose,
    #[cfg_attr(
        feature = "serde",
        serde(alias = "Strict", alias = "strict", alias = "STRICT",)
    )]
    /// A strict policy only allowing request that match the domain set for crawling.
    Strict,
    #[cfg_attr(
        feature = "serde",
        serde(alias = "None", alias = "none", alias = "NONE",)
    )]
    /// Prevent all redirects.
    None,
}

#[cfg(not(feature = "regex"))]
/// Allow list normal matching paths.
pub type AllowList = Vec<CompactString>;

#[cfg(feature = "regex")]
/// Allow list regex.
pub type AllowList = Box<regex::RegexSet>;

/// Whitelist or Blacklist
#[derive(Debug, Default, Clone)]
#[cfg_attr(not(feature = "regex"), derive(PartialEq, Eq))]
pub struct AllowListSet(pub AllowList);

#[cfg(feature = "chrome")]
/// Track the events made via chrome.
#[derive(Debug, PartialEq, Eq, Clone, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ChromeEventTracker {
    /// Track the responses.
    pub responses: bool,
    /// Track the requests.
    pub requests: bool,
}

#[cfg(feature = "chrome")]
impl ChromeEventTracker {
    /// Create a new chrome event tracker
    pub fn new(requests: bool, responses: bool) -> Self {
        ChromeEventTracker {
            requests,
            responses,
        }
    }
}

#[cfg(feature = "sitemap")]
#[derive(Debug, Default)]
/// Determine if the sitemap modified to the whitelist.
pub(crate) struct SitemapWhitelistChanges {
    /// Added the default sitemap.xml whitelist.
    pub added_default: bool,
    /// Added the custom whitelist path.
    pub added_custom: bool,
}

#[cfg(feature = "sitemap")]
impl SitemapWhitelistChanges {
    /// Was the whitelist modified?
    pub(crate) fn modified(&self) -> bool {
        self.added_default || self.added_custom
    }
}

/// Determine allow proxy
#[derive(Debug, Default, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ProxyIgnore {
    /// Chrome proxy.
    Chrome,
    /// HTTP proxy.
    Http,
    #[default]
    /// Do not ignore
    No,
}

/// The networking proxy to use.
#[derive(Debug, Default, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RequestProxy {
    /// The proxy address.
    pub addr: String,
    /// Ignore the proxy when running a request type.
    pub ignore: ProxyIgnore,
}

/// Structure to configure `Website` crawler
/// ```rust
/// use spider::website::Website;
/// let mut website: Website = Website::new("https://choosealicense.com");
/// website.configuration.blacklist_url.insert(Default::default()).push("https://choosealicense.com/licenses/".to_string().into());
/// website.configuration.respect_robots_txt = true;
/// website.configuration.subdomains = true;
/// website.configuration.tld = true;
/// ```
#[derive(Debug, Default, Clone)]
#[cfg_attr(
    all(
        not(feature = "regex"),
        not(feature = "openai"),
        not(feature = "cache_openai")
    ),
    derive(PartialEq)
)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct Configuration {
    /// Respect robots.txt file and not scrape not allowed files. This may slow down crawls if robots.txt file has a delay included.
    pub respect_robots_txt: bool,
    /// Allow sub-domains.
    pub subdomains: bool,
    /// Allow all tlds for domain.
    pub tld: bool,
    /// The max timeout for the crawl.
    pub crawl_timeout: Option<Duration>,
    /// Preserve the HTTP host header from being included.
    pub preserve_host_header: bool,
    /// List of pages to not crawl. [optional: regex pattern matching]
    pub blacklist_url: Option<Vec<CompactString>>,
    /// List of pages to only crawl. [optional: regex pattern matching]
    pub whitelist_url: Option<Vec<CompactString>>,
    /// User-Agent for request.
    pub user_agent: Option<Box<CompactString>>,
    /// Polite crawling delay in milli seconds.
    pub delay: u64,
    /// Request max timeout per page. By default the request times out in 15s. Set to None to disable.
    pub request_timeout: Option<Box<Duration>>,
    /// Use HTTP2 for connection. Enable if you know the website has http2 support.
    pub http2_prior_knowledge: bool,
    /// Use proxy list for performing network request.
    pub proxies: Option<Vec<RequestProxy>>,
    /// Headers to include with request.
    pub headers: Option<Box<SerializableHeaderMap>>,
    #[cfg(feature = "sitemap")]
    /// Include a sitemap in response of the crawl.
    pub sitemap_url: Option<Box<CompactString>>,
    #[cfg(feature = "sitemap")]
    /// Prevent including the sitemap links with the crawl.
    pub ignore_sitemap: bool,
    /// The max redirections allowed for request.
    pub redirect_limit: Box<usize>,
    /// The redirect policy type to use.
    pub redirect_policy: RedirectPolicy,
    #[cfg(feature = "cookies")]
    /// Cookie string to use for network requests ex: "foo=bar; Domain=blog.spider"
    pub cookie_str: Box<String>,
    #[cfg(feature = "rquest")]
    /// The type of request emulation. This does nothing without the flag `sync` enabled.
    pub emulation: Option<rquest_util::Emulation>,
    #[cfg(feature = "cron")]
    /// Cron string to perform crawls - use <https://crontab.guru/> to help generate a valid cron for needs.
    pub cron_str: String,
    #[cfg(feature = "cron")]
    /// The type of cron to run either crawl or scrape.
    pub cron_type: CronType,
    /// The max depth to crawl for a website. Defaults to 25 to help prevent infinite recursion.
    pub depth: usize,
    /// The depth to crawl pertaining to the root.
    pub depth_distance: usize,
    /// Use stealth mode for requests.
    pub stealth_mode: spider_fingerprint::configs::Tier,
    /// Configure the viewport for chrome and viewport headers.
    pub viewport: Option<Viewport>,
    /// Crawl budget for the paths. This helps prevent crawling extra pages and limiting the amount.
    pub budget: Option<hashbrown::HashMap<case_insensitive_string::CaseInsensitiveString, u32>>,
    /// If wild card budgeting is found for the website.
    pub wild_card_budgeting: bool,
    /// External domains to include case-insensitive.
    pub external_domains_caseless:
        Box<hashbrown::HashSet<case_insensitive_string::CaseInsensitiveString>>,
    /// Collect all the resources found on the page.
    pub full_resources: bool,
    /// Dangerously accept invalid certficates.
    pub accept_invalid_certs: bool,
    /// The auth challenge response. The 'chrome_intercept' flag is also required in order to intercept the response.
    pub auth_challenge_response: Option<AuthChallengeResponse>,
    /// The OpenAI configs to use to help drive the chrome browser. This does nothing without the 'openai' flag.
    pub openai_config: Option<Box<GPTConfigs>>,
    /// Use a shared queue strategy when crawling. This can scale workloads evenly that do not need priority.
    pub shared_queue: bool,
    /// Return the page links in the subscription channels. This does nothing without the flag `sync` enabled.
    pub return_page_links: bool,
    /// Retry count to attempt to swap proxies etc.
    pub retry: u8,
    /// Skip spawning a control thread that can pause, start, and shutdown the crawl.
    pub no_control_thread: bool,
    /// The blacklist urls.
    blacklist: AllowListSet,
    /// The whitelist urls.
    whitelist: AllowListSet,
    /// Crawl budget for the paths. This helps prevent crawling extra pages and limiting the amount.
    pub(crate) inner_budget:
        Option<hashbrown::HashMap<case_insensitive_string::CaseInsensitiveString, u32>>,
    /// Expect only to handle HTML to save on resources. This mainly only blocks the crawling and returning of resources from the server.
    pub only_html: bool,
    /// The concurrency limits to apply.
    pub concurrency_limit: Option<usize>,
    /// Normalize the html de-deplucating the content.
    pub normalize: bool,
    /// Modify the headers to act like a real-browser
    pub modify_headers: bool,
    /// Cache the page following HTTP caching rules.
    #[cfg(any(feature = "cache_request", feature = "chrome"))]
    pub cache: bool,
    #[cfg(feature = "chrome")]
    /// Enable or disable service workers. Enabled by default.
    pub service_worker_enabled: bool,
    #[cfg(feature = "chrome")]
    /// Overrides default host system timezone with the specified one.
    #[cfg(feature = "chrome")]
    pub timezone_id: Option<Box<String>>,
    /// Overrides default host system locale with the specified one.
    #[cfg(feature = "chrome")]
    pub locale: Option<Box<String>>,
    /// Set a custom script to eval on each new document.
    #[cfg(feature = "chrome")]
    pub evaluate_on_new_document: Option<Box<String>>,
    #[cfg(feature = "chrome")]
    /// Dismiss dialogs.
    pub dismiss_dialogs: Option<bool>,
    #[cfg(feature = "chrome")]
    /// Wait for options for the page.
    pub wait_for: Option<WaitFor>,
    #[cfg(feature = "chrome")]
    /// Take a screenshot of the page.
    pub screenshot: Option<ScreenShotConfig>,
    #[cfg(feature = "chrome")]
    /// Track the events made via chrome.
    pub track_events: Option<ChromeEventTracker>,
    #[cfg(feature = "chrome")]
    /// Setup fingerprint ID on each document. This does nothing without the flag `chrome` enabled.
    pub fingerprint: Fingerprint,
    #[cfg(feature = "chrome")]
    /// The chrome connection url. Useful for targeting different headless instances. Defaults to using the env CHROME_URL.
    pub chrome_connection_url: Option<String>,
    /// Scripts to execute for individual pages, the full path of the url is required for an exact match. This is useful for running one off JS on pages like performing custom login actions.
    #[cfg(feature = "chrome")]
    pub execution_scripts: Option<ExecutionScripts>,
    /// Web automation scripts to run up to a duration of 60 seconds.
    #[cfg(feature = "chrome")]
    pub automation_scripts: Option<AutomationScripts>,
    /// Setup network interception for request. This does nothing without the flag `chrome_intercept` enabled.
    #[cfg(feature = "chrome")]
    pub chrome_intercept: RequestInterceptConfiguration,
}

#[derive(Default, Debug, Clone, PartialEq, Eq)]
/// Serializable HTTP headers.
pub struct SerializableHeaderMap(pub HeaderMap);

impl SerializableHeaderMap {
    /// Innter HeaderMap.
    pub fn inner(&self) -> &HeaderMap {
        &self.0
    }
    /// Returns true if the map contains a value for the specified key.
    pub fn contains_key<K>(&self, key: K) -> bool
    where
        K: AsHeaderName,
    {
        self.0.contains_key(key)
    }
    /// Inserts a key-value pair into the map.
    pub fn insert<K>(
        &mut self,
        key: K,
        val: reqwest::header::HeaderValue,
    ) -> Option<reqwest::header::HeaderValue>
    where
        K: IntoHeaderName,
    {
        self.0.insert(key, val)
    }
    /// Extend a `HeaderMap` with the contents of another `HeaderMap`.
    pub fn extend<I>(&mut self, iter: I)
    where
        I: IntoIterator<Item = (Option<HeaderName>, HeaderValue)>,
    {
        self.0.extend(iter);
    }
}

impl From<HeaderMap> for SerializableHeaderMap {
    fn from(header_map: HeaderMap) -> Self {
        SerializableHeaderMap(header_map)
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for SerializableHeaderMap {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let map: std::collections::BTreeMap<String, String> = self
            .0
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
            .collect();
        map.serialize(serializer)
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for SerializableHeaderMap {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use reqwest::header::{HeaderName, HeaderValue};
        use std::collections::BTreeMap;
        let map: BTreeMap<String, String> = BTreeMap::deserialize(deserializer)?;
        let mut headers = HeaderMap::with_capacity(map.len());
        for (k, v) in map {
            let key = HeaderName::from_bytes(k.as_bytes()).map_err(serde::de::Error::custom)?;
            let value = HeaderValue::from_str(&v).map_err(serde::de::Error::custom)?;
            headers.insert(key, value);
        }
        Ok(SerializableHeaderMap(headers))
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for AllowListSet {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        #[cfg(not(feature = "regex"))]
        {
            self.0.serialize(serializer)
        }

        #[cfg(feature = "regex")]
        {
            self.0
                .patterns()
                .into_iter()
                .collect::<Vec<&String>>()
                .serialize(serializer)
        }
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for AllowListSet {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[cfg(not(feature = "regex"))]
        {
            let vec = Vec::<CompactString>::deserialize(deserializer)?;
            Ok(AllowListSet(vec.into()))
        }

        #[cfg(feature = "regex")]
        {
            let patterns = Vec::<String>::deserialize(deserializer)?;
            let regex_set = regex::RegexSet::new(&patterns).map_err(serde::de::Error::custom)?;
            Ok(AllowListSet(regex_set.into()))
        }
    }
}

/// Get the user agent from the top agent list randomly.
#[cfg(any(feature = "ua_generator"))]
pub fn get_ua(chrome: bool) -> &'static str {
    if chrome {
        ua_generator::ua::spoof_chrome_ua()
    } else {
        ua_generator::ua::spoof_ua()
    }
}

/// Get the user agent via cargo package + version.
#[cfg(not(any(feature = "ua_generator")))]
pub fn get_ua(_chrome: bool) -> &'static str {
    use std::env;

    lazy_static! {
        static ref AGENT: &'static str =
            concat!(env!("CARGO_PKG_NAME"), '/', env!("CARGO_PKG_VERSION"));
    };

    AGENT.as_ref()
}

impl Configuration {
    /// Represents crawl configuration for a website.
    #[cfg(not(feature = "chrome"))]
    pub fn new() -> Self {
        Self {
            delay: 0,
            depth: 25,
            redirect_limit: Box::new(7),
            request_timeout: Some(Box::new(Duration::from_secs(60))),
            only_html: true,
            modify_headers: true,
            ..Default::default()
        }
    }

    /// Represents crawl configuration for a website.
    #[cfg(feature = "chrome")]
    pub fn new() -> Self {
        Self {
            delay: 0,
            depth: 25,
            redirect_limit: Box::new(7),
            request_timeout: Some(Box::new(Duration::from_secs(60))),
            chrome_intercept: RequestInterceptConfiguration::new(cfg!(
                feature = "chrome_intercept"
            )),
            user_agent: Some(Box::new(get_ua(true).into())),
            only_html: true,
            cache: true,
            modify_headers: true,
            service_worker_enabled: true,
            fingerprint: Fingerprint::Basic,
            ..Default::default()
        }
    }

    /// Determine if the agent should be set to a Chrome Agent.
    #[cfg(not(feature = "chrome"))]
    pub(crate) fn only_chrome_agent(&self) -> bool {
        false
    }

    /// Determine if the agent should be set to a Chrome Agent.
    #[cfg(feature = "chrome")]
    pub(crate) fn only_chrome_agent(&self) -> bool {
        self.chrome_connection_url.is_some()
            || self.wait_for.is_some()
            || self.chrome_intercept.enabled
            || self.stealth_mode.stealth()
            || self.fingerprint.valid()
    }

    #[cfg(feature = "regex")]
    /// Compile the regex for the blacklist.
    pub fn get_blacklist(&self) -> Box<regex::RegexSet> {
        match &self.blacklist_url {
            Some(blacklist) => match regex::RegexSet::new(&**blacklist) {
                Ok(s) => Box::new(s),
                _ => Default::default(),
            },
            _ => Default::default(),
        }
    }

    #[cfg(not(feature = "regex"))]
    /// Handle the blacklist options.
    pub fn get_blacklist(&self) -> AllowList {
        match &self.blacklist_url {
            Some(blacklist) => blacklist.to_owned(),
            _ => Default::default(),
        }
    }

    /// Set the blacklist
    pub(crate) fn set_blacklist(&mut self) {
        self.blacklist = AllowListSet(self.get_blacklist());
    }

    /// Set the whitelist
    pub(crate) fn set_whitelist(&mut self) {
        self.whitelist = AllowListSet(self.get_whitelist());
    }

    /// Configure the allow list.
    pub(crate) fn configure_allowlist(&mut self) {
        self.set_whitelist();
        self.set_blacklist();
    }

    /// Get the blacklist compiled.
    pub(crate) fn get_blacklist_compiled(&self) -> &AllowList {
        &self.blacklist.0
    }

    /// Setup the budget for crawling.
    pub(crate) fn configure_budget(&mut self) {
        self.inner_budget.clone_from(&self.budget);
    }

    /// Get the whitelist compiled.
    pub(crate) fn get_whitelist_compiled(&self) -> &AllowList {
        &self.whitelist.0
    }

    #[cfg(feature = "regex")]
    /// Compile the regex for the whitelist.
    pub fn get_whitelist(&self) -> Box<regex::RegexSet> {
        match &self.whitelist_url {
            Some(whitelist) => match regex::RegexSet::new(&**whitelist) {
                Ok(s) => Box::new(s),
                _ => Default::default(),
            },
            _ => Default::default(),
        }
    }

    #[cfg(not(feature = "regex"))]
    /// Handle the whitelist options.
    pub fn get_whitelist(&self) -> AllowList {
        match &self.whitelist_url {
            Some(whitelist) => whitelist.to_owned(),
            _ => Default::default(),
        }
    }

    #[cfg(feature = "sitemap")]
    /// Add sitemap paths to the whitelist and track what was added.
    pub(crate) fn add_sitemap_to_whitelist(&mut self) -> SitemapWhitelistChanges {
        let mut changes = SitemapWhitelistChanges::default();

        if self.ignore_sitemap && !self.whitelist_url.is_some() {
            return changes;
        }

        if let Some(list) = self.whitelist_url.as_mut() {
            if list.is_empty() {
                return changes;
            }

            let default = CompactString::from("sitemap.xml");

            if !list.contains(&default) {
                list.push(default);
                changes.added_default = true;
            }

            if let Some(custom) = &self.sitemap_url {
                if !list.contains(custom) {
                    list.push(*custom.clone());
                    changes.added_custom = true;
                }
            }
        }

        changes
    }

    #[cfg(feature = "sitemap")]
    /// Revert any changes made to the whitelist by `add_sitemap_to_whitelist`.
    pub(crate) fn remove_sitemap_from_whitelist(&mut self, changes: SitemapWhitelistChanges) {
        if let Some(list) = self.whitelist_url.as_mut() {
            if changes.added_default {
                let default = CompactString::from("sitemap.xml");
                if let Some(pos) = list.iter().position(|s| s == &default) {
                    list.remove(pos);
                }
            }
            if changes.added_custom {
                if let Some(custom) = &self.sitemap_url {
                    if let Some(pos) = list.iter().position(|s| *s == **custom) {
                        list.remove(pos);
                    }
                }
            }
            if list.is_empty() {
                self.whitelist_url = None;
            }
        }
    }

    /// Respect robots.txt file.
    pub fn with_respect_robots_txt(&mut self, respect_robots_txt: bool) -> &mut Self {
        self.respect_robots_txt = respect_robots_txt;
        self
    }

    /// Include subdomains detection.
    pub fn with_subdomains(&mut self, subdomains: bool) -> &mut Self {
        self.subdomains = subdomains;
        self
    }

    /// Include tld detection.
    pub fn with_tld(&mut self, tld: bool) -> &mut Self {
        self.tld = tld;
        self
    }

    /// The max duration for the crawl. This is useful when websites use a robots.txt with long durations and throttle the timeout removing the full concurrency.
    pub fn with_crawl_timeout(&mut self, crawl_timeout: Option<Duration>) -> &mut Self {
        self.crawl_timeout = crawl_timeout;
        self
    }

    /// Delay between request as ms.
    pub fn with_delay(&mut self, delay: u64) -> &mut Self {
        self.delay = delay;
        self
    }

    /// Only use HTTP/2.
    pub fn with_http2_prior_knowledge(&mut self, http2_prior_knowledge: bool) -> &mut Self {
        self.http2_prior_knowledge = http2_prior_knowledge;
        self
    }

    /// Max time to wait for request. By default request times out in 15s. Set to None to disable.
    pub fn with_request_timeout(&mut self, request_timeout: Option<Duration>) -> &mut Self {
        match request_timeout {
            Some(timeout) => self.request_timeout = Some(timeout.into()),
            _ => self.request_timeout = None,
        };

        self
    }

    #[cfg(feature = "sitemap")]
    /// Set the sitemap url. This does nothing without the `sitemap` feature flag.
    pub fn with_sitemap(&mut self, sitemap_url: Option<&str>) -> &mut Self {
        match sitemap_url {
            Some(sitemap_url) => {
                self.sitemap_url = Some(CompactString::new(sitemap_url.to_string()).into())
            }
            _ => self.sitemap_url = None,
        };
        self
    }

    #[cfg(not(feature = "sitemap"))]
    /// Set the sitemap url. This does nothing without the `sitemap` feature flag.
    pub fn with_sitemap(&mut self, _sitemap_url: Option<&str>) -> &mut Self {
        self
    }

    #[cfg(feature = "sitemap")]
    /// Ignore the sitemap when crawling. This method does nothing if the `sitemap` is not enabled.
    pub fn with_ignore_sitemap(&mut self, ignore_sitemap: bool) -> &mut Self {
        self.ignore_sitemap = ignore_sitemap;
        self
    }

    #[cfg(not(feature = "sitemap"))]
    /// Ignore the sitemap when crawling. This method does nothing if the `sitemap` is not enabled.
    pub fn with_ignore_sitemap(&mut self, _ignore_sitemap: bool) -> &mut Self {
        self
    }

    /// Add user agent to request.
    pub fn with_user_agent(&mut self, user_agent: Option<&str>) -> &mut Self {
        match user_agent {
            Some(agent) => self.user_agent = Some(CompactString::new(agent).into()),
            _ => self.user_agent = None,
        };
        self
    }

    /// Preserve the HOST header.
    pub fn with_preserve_host_header(&mut self, preserve: bool) -> &mut Self {
        self.preserve_host_header = preserve;
        self
    }

    #[cfg(not(feature = "openai"))]
    /// The OpenAI configs to use to drive the browser. This method does nothing if the `openai` is not enabled.
    pub fn with_openai(&mut self, _openai_config: Option<GPTConfigs>) -> &mut Self {
        self
    }

    /// The OpenAI configs to use to drive the browser. This method does nothing if the `openai` is not enabled.
    #[cfg(feature = "openai")]
    pub fn with_openai(&mut self, openai_config: Option<GPTConfigs>) -> &mut Self {
        match openai_config {
            Some(openai_config) => self.openai_config = Some(Box::new(openai_config)),
            _ => self.openai_config = None,
        };
        self
    }

    #[cfg(feature = "cookies")]
    /// Cookie string to use in request. This does nothing without the `cookies` flag enabled.
    pub fn with_cookies(&mut self, cookie_str: &str) -> &mut Self {
        self.cookie_str = Box::new(cookie_str.into());
        self
    }

    #[cfg(not(feature = "cookies"))]
    /// Cookie string to use in request. This does nothing without the `cookies` flag enabled.
    pub fn with_cookies(&mut self, _cookie_str: &str) -> &mut Self {
        self
    }

    #[cfg(feature = "chrome")]
    /// Set custom fingerprint ID for request. This does nothing without the `chrome` flag enabled.
    pub fn with_fingerprint(&mut self, fingerprint: bool) -> &mut Self {
        if fingerprint {
            self.fingerprint = Fingerprint::Basic;
        } else {
            self.fingerprint = Fingerprint::None;
        }
        self
    }

    #[cfg(feature = "chrome")]
    /// Set custom fingerprint ID for request. This does nothing without the `chrome` flag enabled.
    pub fn with_fingerprint_advanced(&mut self, fingerprint: Fingerprint) -> &mut Self {
        self.fingerprint = fingerprint;
        self
    }

    #[cfg(not(feature = "chrome"))]
    /// Set custom fingerprint ID for request. This does nothing without the `chrome` flag enabled.
    pub fn with_fingerprint(&mut self, _fingerprint: bool) -> &mut Self {
        self
    }

    /// Use proxies for request.
    pub fn with_proxies(&mut self, proxies: Option<Vec<String>>) -> &mut Self {
        self.proxies = proxies.map(|p| {
            p.iter()
                .map(|addr| RequestProxy {
                    addr: addr.to_owned(),
                    ..Default::default()
                })
                .collect::<Vec<RequestProxy>>()
        });
        self
    }

    /// Use proxies for request with control between chrome and http.
    pub fn with_proxies_direct(&mut self, proxies: Option<Vec<RequestProxy>>) -> &mut Self {
        self.proxies = proxies;
        self
    }

    /// Use a shared semaphore to evenly handle workloads. The default is false.
    pub fn with_shared_queue(&mut self, shared_queue: bool) -> &mut Self {
        self.shared_queue = shared_queue;
        self
    }

    /// Add blacklist urls to ignore.
    pub fn with_blacklist_url<T>(&mut self, blacklist_url: Option<Vec<T>>) -> &mut Self
    where
        Vec<CompactString>: From<Vec<T>>,
    {
        match blacklist_url {
            Some(p) => self.blacklist_url = Some(p.into()),
            _ => self.blacklist_url = None,
        };
        self
    }

    /// Add whitelist urls to allow.
    pub fn with_whitelist_url<T>(&mut self, whitelist_url: Option<Vec<T>>) -> &mut Self
    where
        Vec<CompactString>: From<Vec<T>>,
    {
        match whitelist_url {
            Some(p) => self.whitelist_url = Some(p.into()),
            _ => self.whitelist_url = None,
        };
        self
    }

    /// Return the links found on the page in the channel subscriptions. This method does nothing if the `decentralized` is enabled.
    pub fn with_return_page_links(&mut self, return_page_links: bool) -> &mut Self {
        self.return_page_links = return_page_links;
        self
    }

    /// Set HTTP headers for request using [reqwest::header::HeaderMap](https://docs.rs/reqwest/latest/reqwest/header/struct.HeaderMap.html).
    pub fn with_headers(&mut self, headers: Option<reqwest::header::HeaderMap>) -> &mut Self {
        match headers {
            Some(m) => self.headers = Some(SerializableHeaderMap::from(m).into()),
            _ => self.headers = None,
        };
        self
    }

    /// Set the max redirects allowed for request.
    pub fn with_redirect_limit(&mut self, redirect_limit: usize) -> &mut Self {
        self.redirect_limit = redirect_limit.into();
        self
    }

    /// Set the redirect policy to use.
    pub fn with_redirect_policy(&mut self, policy: RedirectPolicy) -> &mut Self {
        self.redirect_policy = policy;
        self
    }

    /// Determine whether to collect all the resources found on pages.
    pub fn with_full_resources(&mut self, full_resources: bool) -> &mut Self {
        self.full_resources = full_resources;
        self
    }

    /// Determine whether to dismiss dialogs. This method does nothing if the `chrome` is enabled.
    #[cfg(feature = "chrome")]
    pub fn with_dismiss_dialogs(&mut self, dismiss_dialogs: bool) -> &mut Self {
        self.dismiss_dialogs = Some(dismiss_dialogs);
        self
    }

    /// Determine whether to dismiss dialogs. This method does nothing if the `chrome` is enabled.
    #[cfg(not(feature = "chrome"))]
    pub fn with_dismiss_dialogs(&mut self, _dismiss_dialogs: bool) -> &mut Self {
        self
    }

    /// Set the request emuluation. This method does nothing if the `rquest` flag is not enabled.
    #[cfg(feature = "rquest")]
    pub fn with_emulation(&mut self, emulation: Option<rquest_util::Emulation>) -> &mut Self {
        self.emulation = emulation;
        self
    }

    #[cfg(feature = "cron")]
    /// Setup cron jobs to run. This does nothing without the `cron` flag enabled.
    pub fn with_cron(&mut self, cron_str: &str, cron_type: CronType) -> &mut Self {
        self.cron_str = cron_str.into();
        self.cron_type = cron_type;
        self
    }

    #[cfg(not(feature = "cron"))]
    /// Setup cron jobs to run. This does nothing without the `cron` flag enabled.
    pub fn with_cron(&mut self, _cron_str: &str, _cron_type: CronType) -> &mut Self {
        self
    }

    /// Set a crawl page limit. If the value is 0 there is no limit.
    pub fn with_limit(&mut self, limit: u32) -> &mut Self {
        self.with_budget(Some(hashbrown::HashMap::from([("*", limit)])));
        self
    }

    /// Set the concurrency limits. If you set the value to None to use the default limits using the system CPU cors * n.
    pub fn with_concurrency_limit(&mut self, limit: Option<usize>) -> &mut Self {
        self.concurrency_limit = limit;
        self
    }

    #[cfg(feature = "chrome")]
    /// Set the authentiation challenge response. This does nothing without the feat flag `chrome` enabled.
    pub fn with_auth_challenge_response(
        &mut self,
        auth_challenge_response: Option<AuthChallengeResponse>,
    ) -> &mut Self {
        self.auth_challenge_response = auth_challenge_response;
        self
    }

    #[cfg(feature = "chrome")]
    /// Set a custom script to evaluate on new document creation. This does nothing without the feat flag `chrome` enabled.
    pub fn with_evaluate_on_new_document(
        &mut self,
        evaluate_on_new_document: Option<Box<String>>,
    ) -> &mut Self {
        self.evaluate_on_new_document = evaluate_on_new_document;
        self
    }

    #[cfg(not(feature = "chrome"))]
    /// Set a custom script to evaluate on new document creation. This does nothing without the feat flag `chrome` enabled.
    pub fn with_evaluate_on_new_document(
        &mut self,
        _evaluate_on_new_document: Option<Box<String>>,
    ) -> &mut Self {
        self
    }

    #[cfg(not(feature = "chrome"))]
    /// Set the authentiation challenge response. This does nothing without the feat flag `chrome` enabled.
    pub fn with_auth_challenge_response(
        &mut self,
        _auth_challenge_response: Option<AuthChallengeResponse>,
    ) -> &mut Self {
        self
    }

    /// Set a crawl depth limit. If the value is 0 there is no limit.
    pub fn with_depth(&mut self, depth: usize) -> &mut Self {
        self.depth = depth;
        self
    }

    #[cfg(feature = "cache_request")]
    /// Cache the page following HTTP rules. This method does nothing if the `cache` feature is not enabled.
    pub fn with_caching(&mut self, cache: bool) -> &mut Self {
        self.cache = cache;
        self
    }

    #[cfg(not(feature = "cache_request"))]
    /// Cache the page following HTTP rules. This method does nothing if the `cache` feature is not enabled.
    pub fn with_caching(&mut self, _cache: bool) -> &mut Self {
        self
    }

    #[cfg(feature = "chrome")]
    /// Enable or disable Service Workers. This method does nothing if the `chrome` feature is not enabled.
    pub fn with_service_worker_enabled(&mut self, enabled: bool) -> &mut Self {
        self.service_worker_enabled = enabled;
        self
    }

    #[cfg(not(feature = "chrome"))]
    /// Enable or disable Service Workers. This method does nothing if the `chrome` feature is not enabled.
    pub fn with_service_worker_enabled(&mut self, _enabled: bool) -> &mut Self {
        self
    }

    /// Set the retry limit for request. Set the value to 0 for no retries. The default is 0.
    pub fn with_retry(&mut self, retry: u8) -> &mut Self {
        self.retry = retry;
        self
    }

    /// Skip setting up a control thread for pause, start, and shutdown programmatic handling. This does nothing without the [control] flag enabled.
    pub fn with_no_control_thread(&mut self, no_control_thread: bool) -> &mut Self {
        self.no_control_thread = no_control_thread;
        self
    }

    /// Configures the viewport of the browser, which defaults to 800x600. This method does nothing if the [chrome] feature is not enabled.
    pub fn with_viewport(&mut self, viewport: Option<crate::configuration::Viewport>) -> &mut Self {
        self.viewport = match viewport {
            Some(vp) => Some(vp.into()),
            _ => None,
        };
        self
    }

    #[cfg(feature = "chrome")]
    /// Use stealth mode for the request. This does nothing without the `chrome` flag enabled.
    pub fn with_stealth(&mut self, stealth_mode: bool) -> &mut Self {
        if stealth_mode {
            self.stealth_mode = spider_fingerprint::configs::Tier::Basic;
        } else {
            self.stealth_mode = spider_fingerprint::configs::Tier::None;
        }
        self
    }

    #[cfg(feature = "chrome")]
    /// Use stealth mode for the request. This does nothing without the `chrome` flag enabled.
    pub fn with_stealth_advanced(
        &mut self,
        stealth_mode: spider_fingerprint::configs::Tier,
    ) -> &mut Self {
        self.stealth_mode = stealth_mode;
        self
    }

    #[cfg(not(feature = "chrome"))]
    /// Use stealth mode for the request. This does nothing without the `chrome` flag enabled.
    pub fn with_stealth(&mut self, _stealth_mode: bool) -> &mut Self {
        self
    }

    #[cfg(feature = "chrome")]
    /// Wait for idle network request. This method does nothing if the [chrome] feature is not enabled.
    pub fn with_wait_for_idle_network(
        &mut self,
        wait_for_idle_network: Option<WaitForIdleNetwork>,
    ) -> &mut Self {
        match self.wait_for.as_mut() {
            Some(wait_for) => wait_for.idle_network = wait_for_idle_network,
            _ => {
                let mut wait_for = WaitFor::default();
                wait_for.idle_network = wait_for_idle_network;
                self.wait_for = Some(wait_for);
            }
        }
        self
    }

    #[cfg(not(feature = "chrome"))]
    /// Wait for idle network request. This method does nothing if the `chrome` feature is not enabled.
    pub fn with_wait_for_idle_network(
        &mut self,
        _wait_for_idle_network: Option<WaitForIdleNetwork>,
    ) -> &mut Self {
        self
    }

    #[cfg(feature = "chrome")]
    /// Wait for idle dom mutations for target element. This method does nothing if the [chrome] feature is not enabled.
    pub fn with_wait_for_idle_dom(
        &mut self,
        wait_for_idle_dom: Option<WaitForSelector>,
    ) -> &mut Self {
        match self.wait_for.as_mut() {
            Some(wait_for) => wait_for.dom = wait_for_idle_dom,
            _ => {
                let mut wait_for = WaitFor::default();
                wait_for.dom = wait_for_idle_dom;
                self.wait_for = Some(wait_for);
            }
        }
        self
    }

    #[cfg(not(feature = "chrome"))]
    /// Wait for idle dom mutations for target element. This method does nothing if the `chrome` feature is not enabled.
    pub fn with_wait_for_idle_dom(
        &mut self,
        _wait_for_idle_dom: Option<WaitForSelector>,
    ) -> &mut Self {
        self
    }

    #[cfg(feature = "chrome")]
    /// Wait for a selector. This method does nothing if the `chrome` feature is not enabled.
    pub fn with_wait_for_selector(
        &mut self,
        wait_for_selector: Option<WaitForSelector>,
    ) -> &mut Self {
        match self.wait_for.as_mut() {
            Some(wait_for) => wait_for.selector = wait_for_selector,
            _ => {
                let mut wait_for = WaitFor::default();
                wait_for.selector = wait_for_selector;
                self.wait_for = Some(wait_for);
            }
        }
        self
    }

    #[cfg(not(feature = "chrome"))]
    /// Wait for a selector. This method does nothing if the `chrome` feature is not enabled.
    pub fn with_wait_for_selector(
        &mut self,
        _wait_for_selector: Option<WaitForSelector>,
    ) -> &mut Self {
        self
    }

    #[cfg(feature = "chrome")]
    /// Wait for with delay. Should only be used for testing. This method does nothing if the 'chrome' feature is not enabled.
    pub fn with_wait_for_delay(&mut self, wait_for_delay: Option<WaitForDelay>) -> &mut Self {
        match self.wait_for.as_mut() {
            Some(wait_for) => wait_for.delay = wait_for_delay,
            _ => {
                let mut wait_for = WaitFor::default();
                wait_for.delay = wait_for_delay;
                self.wait_for = Some(wait_for);
            }
        }
        self
    }

    #[cfg(not(feature = "chrome"))]
    /// Wait for with delay. Should only be used for testing. This method does nothing if the 'chrome' feature is not enabled.
    pub fn with_wait_for_delay(&mut self, _wait_for_delay: Option<WaitForDelay>) -> &mut Self {
        self
    }

    #[cfg(feature = "chrome_intercept")]
    /// Use request intercept for the request to only allow content that matches the host. If the content is from a 3rd party it needs to be part of our include list. This method does nothing if the `chrome_intercept` is not enabled.
    pub fn with_chrome_intercept(
        &mut self,
        chrome_intercept: RequestInterceptConfiguration,
        url: &Option<Box<url::Url>>,
    ) -> &mut Self {
        self.chrome_intercept = chrome_intercept;
        self.chrome_intercept.setup_intercept_manager(url);
        self
    }

    #[cfg(not(feature = "chrome_intercept"))]
    /// Use request intercept for the request to only allow content required for the page that matches the host. If the content is from a 3rd party it needs to be part of our include list. This method does nothing if the `chrome_intercept` is not enabled.
    pub fn with_chrome_intercept(
        &mut self,
        _chrome_intercept: RequestInterceptConfiguration,
        _url: &Option<Box<url::Url>>,
    ) -> &mut Self {
        self
    }

    #[cfg(feature = "chrome")]
    /// Set the connection url for the chrome instance. This method does nothing if the `chrome` is not enabled.
    pub fn with_chrome_connection(&mut self, chrome_connection_url: Option<String>) -> &mut Self {
        self.chrome_connection_url = chrome_connection_url;
        self
    }

    #[cfg(not(feature = "chrome"))]
    /// Set the connection url for the chrome instance. This method does nothing if the `chrome` is not enabled.
    pub fn with_chrome_connection(&mut self, _chrome_connection_url: Option<String>) -> &mut Self {
        self
    }

    #[cfg(not(feature = "chrome"))]
    /// Set JS to run on certain pages. This method does nothing if the `chrome` is not enabled.
    pub fn with_execution_scripts(
        &mut self,
        _execution_scripts: Option<ExecutionScriptsMap>,
    ) -> &mut Self {
        self
    }

    #[cfg(feature = "chrome")]
    /// Set JS to run on certain pages. This method does nothing if the `chrome` is not enabled.
    pub fn with_execution_scripts(
        &mut self,
        execution_scripts: Option<ExecutionScriptsMap>,
    ) -> &mut Self {
        self.execution_scripts =
            crate::features::chrome_common::convert_to_trie_execution_scripts(&execution_scripts);
        self
    }

    #[cfg(not(feature = "chrome"))]
    /// Run web automated actions on certain pages. This method does nothing if the `chrome` is not enabled.
    pub fn with_automation_scripts(
        &mut self,
        _automation_scripts: Option<AutomationScriptsMap>,
    ) -> &mut Self {
        self
    }

    #[cfg(feature = "chrome")]
    /// Run web automated actions on certain pages. This method does nothing if the `chrome` is not enabled.
    pub fn with_automation_scripts(
        &mut self,
        automation_scripts: Option<AutomationScriptsMap>,
    ) -> &mut Self {
        self.automation_scripts =
            crate::features::chrome_common::convert_to_trie_automation_scripts(&automation_scripts);
        self
    }

    /// Set a crawl budget per path with levels support /a/b/c or for all paths with "*". This does nothing without the `budget` flag enabled.
    pub fn with_budget(&mut self, budget: Option<hashbrown::HashMap<&str, u32>>) -> &mut Self {
        self.budget = match budget {
            Some(budget) => {
                let mut crawl_budget: hashbrown::HashMap<
                    case_insensitive_string::CaseInsensitiveString,
                    u32,
                > = hashbrown::HashMap::new();

                for b in budget.into_iter() {
                    crawl_budget.insert(
                        case_insensitive_string::CaseInsensitiveString::from(b.0),
                        b.1,
                    );
                }

                Some(crawl_budget)
            }
            _ => None,
        };
        self
    }

    /// Group external domains to treat the crawl as one. If None is passed this will clear all prior domains.
    pub fn with_external_domains<'a, 'b>(
        &mut self,
        external_domains: Option<impl Iterator<Item = String> + 'a>,
    ) -> &mut Self {
        match external_domains {
            Some(external_domains) => {
                self.external_domains_caseless = external_domains
                    .into_iter()
                    .filter_map(|d| {
                        if d == "*" {
                            Some("*".into())
                        } else {
                            let host = get_domain_from_url(&d);

                            if !host.is_empty() {
                                Some(host.into())
                            } else {
                                None
                            }
                        }
                    })
                    .collect::<hashbrown::HashSet<case_insensitive_string::CaseInsensitiveString>>()
                    .into();
            }
            _ => self.external_domains_caseless.clear(),
        }

        self
    }

    /// Dangerously accept invalid certificates - this should be used as a last resort.
    pub fn with_danger_accept_invalid_certs(&mut self, accept_invalid_certs: bool) -> &mut Self {
        self.accept_invalid_certs = accept_invalid_certs;
        self
    }

    /// Normalize the content de-duplicating trailing slash pages and other pages that can be duplicated. This may initially show the link in your links_visited or subscription calls but, the following links will not be crawled.
    pub fn with_normalize(&mut self, normalize: bool) -> &mut Self {
        self.normalize = normalize;
        self
    }

    #[cfg(not(feature = "chrome"))]
    /// Overrides default host system timezone with the specified one. This does nothing without the `chrome` flag enabled.
    pub fn with_timezone_id(&mut self, _timezone_id: Option<String>) -> &mut Self {
        self
    }

    #[cfg(feature = "chrome")]
    /// Overrides default host system timezone with the specified one. This does nothing without the `chrome` flag enabled.
    pub fn with_timezone_id(&mut self, timezone_id: Option<String>) -> &mut Self {
        self.timezone_id = match timezone_id {
            Some(timezone_id) => Some(timezone_id.into()),
            _ => None,
        };
        self
    }

    #[cfg(not(feature = "chrome"))]
    /// Overrides default host system locale with the specified one. This does nothing without the `chrome` flag enabled.
    pub fn with_locale(&mut self, _locale: Option<String>) -> &mut Self {
        self
    }

    #[cfg(feature = "chrome")]
    /// Overrides default host system locale with the specified one. This does nothing without the `chrome` flag enabled.
    pub fn with_locale(&mut self, locale: Option<String>) -> &mut Self {
        self.locale = match locale {
            Some(locale) => Some(locale.into()),
            _ => None,
        };
        self
    }

    #[cfg(feature = "chrome")]
    /// Track the events made via chrome.
    pub fn with_event_tracker(&mut self, track_events: Option<ChromeEventTracker>) -> &mut Self {
        self.track_events = track_events;
        self
    }

    /// Set the chrome screenshot configuration. This does nothing without the `chrome` flag enabled.
    #[cfg(not(feature = "chrome"))]
    pub fn with_screenshot(&mut self, _screenshot_config: Option<ScreenShotConfig>) -> &mut Self {
        self
    }

    /// Set the chrome screenshot configuration. This does nothing without the `chrome` flag enabled.
    #[cfg(feature = "chrome")]
    pub fn with_screenshot(&mut self, screenshot_config: Option<ScreenShotConfig>) -> &mut Self {
        self.screenshot = screenshot_config;
        self
    }

    /// Block assets from loading from the network.
    pub fn with_block_assets(&mut self, only_html: bool) -> &mut Self {
        self.only_html = only_html;
        self
    }

    /// Modify the headers to mimic a real browser.
    pub fn with_modify_headers(&mut self, modify_headers: bool) -> &mut Self {
        self.modify_headers = modify_headers;
        self
    }

    /// Build the website configuration when using with_builder.
    pub fn build(&self) -> Self {
        self.to_owned()
    }
}
