pub use crate::features::chrome_common::{
    AuthChallengeResponse, AuthChallengeResponseResponse, CaptureScreenshotFormat, ClipViewport,
    ScreenShotConfig, Viewport, WaitFor, WaitForDelay, WaitForIdleNetwork, WaitForSelector,
};
pub use crate::features::openai_common::GPTConfigs;
use crate::website::CronType;
use compact_str::CompactString;
use std::time::Duration;

/// Redirect policy configuration for request
#[derive(Debug, Default, Clone)]
pub enum RedirectPolicy {
    #[default]
    /// A loose policy that allows all request up to the redirect limit.
    Loose,
    /// A strict policy only allowing request that match the domain set for crawling.
    Strict,
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
pub struct Configuration {
    /// Respect robots.txt file and not scrape not allowed files. This may slow down crawls if robots.txt file has a delay included.
    pub respect_robots_txt: bool,
    /// Allow sub-domains.
    pub subdomains: bool,
    /// Allow all tlds for domain.
    pub tld: bool,
    /// List of pages to not crawl. [optional: regex pattern matching]
    pub blacklist_url: Option<Box<Vec<CompactString>>>,
    /// User-Agent for request.
    pub user_agent: Option<Box<CompactString>>,
    /// Polite crawling delay in milli seconds.
    pub delay: u64,
    /// Request max timeout per page. By default the request times out in 15s. Set to None to disable.
    pub request_timeout: Option<Box<Duration>>,
    /// Use HTTP2 for connection. Enable if you know the website has http2 support.
    pub http2_prior_knowledge: bool,
    /// Use proxy list for performing network request.
    pub proxies: Option<Box<Vec<String>>>,
    /// Headers to include with request.
    pub headers: Option<Box<reqwest::header::HeaderMap>>,
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
    #[cfg(feature = "cron")]
    /// Cron string to perform crawls - use <https://crontab.guru/> to help generate a valid cron for needs.
    pub cron_str: String,
    #[cfg(feature = "cron")]
    /// The type of cron to run either crawl or scrape.
    pub cron_type: CronType,
    #[cfg(feature = "budget")]
    /// The max depth to crawl for a website.
    pub depth: usize,
    #[cfg(feature = "budget")]
    /// The depth to crawl pertaining to the root.
    pub depth_distance: usize,
    /// Cache the page following HTTP caching rules.
    #[cfg(any(feature = "cache", feature = "chrome"))]
    pub cache: bool,
    #[cfg(feature = "chrome")]
    /// Use stealth mode for requests.
    pub stealth_mode: bool,
    /// Setup network interception for request. This does nothing without the flag `chrome_intercept` enabled.
    #[cfg(feature = "chrome")]
    pub chrome_intercept: bool,
    /// Configure the viewport for chrome. This does nothing without the flag `chrome` enabled.
    #[cfg(feature = "chrome")]
    pub viewport: Option<chromiumoxide::handler::viewport::Viewport>,
    /// Block all images from rendering in Chrome. This does nothing without the flag `chrome_intercept` enabled
    #[cfg(feature = "chrome")]
    pub chrome_intercept_block_visuals: bool,
    /// Overrides default host system timezone with the specified one. This does nothing without the flag `chrome` enabled.
    #[cfg(feature = "chrome")]
    pub timezone_id: Option<Box<String>>,
    /// Overrides default host system locale with the specified one. This does nothing without the flag `chrome` enabled.
    #[cfg(feature = "chrome")]
    pub locale: Option<Box<String>>,
    /// Set a custom script to eval on each new document. This does nothing without the flag `chrome` enabled.
    #[cfg(feature = "chrome")]
    pub evaluate_on_new_document: Option<Box<String>>,
    #[cfg(feature = "budget")]
    /// Crawl budget for the paths. This helps prevent crawling extra pages and limiting the amount.
    pub budget: Option<hashbrown::HashMap<case_insensitive_string::CaseInsensitiveString, u32>>,
    #[cfg(feature = "budget")]
    /// If wild card budgeting is found for the website.
    pub wild_card_budgeting: bool,
    /// External domains to include case-insensitive.
    pub external_domains_caseless:
        Box<hashbrown::HashSet<case_insensitive_string::CaseInsensitiveString>>,
    /// Collect all the resources found on the page.
    pub full_resources: bool,
    #[cfg(feature = "chrome")]
    /// Wait for options for the page.
    pub wait_for: Option<WaitFor>,
    #[cfg(feature = "chrome")]
    /// Take a screenshot of the page.
    pub screenshot: Option<ScreenShotConfig>,
    /// Dangerously accept invalid certficates.
    pub accept_invalid_certs: bool,
    /// The auth challenge response. The 'chrome_intercept' flag is also required in order to intercept the response.
    pub auth_challenge_response: Option<AuthChallengeResponse>,
    /// The OpenAI configs to use to help drive the chrome browser. This does nothing without the 'openai' flag.
    pub openai_config: Option<GPTConfigs>,
    /// Setup fingerprint ID on each document. This does nothing without the flag `chrome` enabled.
    #[cfg(feature = "chrome")]
    pub fingerprint: bool,
}

/// Get the user agent from the top agent list randomly.
#[cfg(any(feature = "ua_generator"))]
pub fn get_ua() -> &'static str {
    ua_generator::ua::spoof_ua()
}

/// Get the user agent via cargo package + version.
#[cfg(not(any(feature = "ua_generator")))]
pub fn get_ua() -> &'static str {
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
            redirect_limit: Box::new(7),
            request_timeout: Some(Box::new(Duration::from_secs(15))),
            ..Default::default()
        }
    }

    /// Represents crawl configuration for a website.
    #[cfg(feature = "chrome")]
    pub fn new() -> Self {
        Self {
            delay: 0,
            redirect_limit: Box::new(7),
            request_timeout: Some(Box::new(Duration::from_secs(15))),
            chrome_intercept: cfg!(feature = "chrome_intercept"),
            ..Default::default()
        }
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
    pub fn get_blacklist(&self) -> Box<Vec<CompactString>> {
        match &self.blacklist_url {
            Some(blacklist) => blacklist.to_owned(),
            _ => Default::default(),
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
            Some(timeout) => {
                self.request_timeout = Some(timeout.into());
            }
            _ => {
                self.request_timeout = None;
            }
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

    #[cfg(not(feature = "openai"))]
    /// The OpenAI configs to use to drive the browser. This method does nothing if the `openai` is not enabled.
    pub fn with_openai(&mut self, _openai_config: Option<GPTConfigs>) -> &mut Self {
        self
    }

    /// The OpenAI configs to use to drive the browser. This method does nothing if the `openai` is not enabled.
    #[cfg(feature = "openai")]
    pub fn with_openai(&mut self, openai_config: Option<GPTConfigs>) -> &mut Self {
        match openai_config {
            Some(openai_config) => self.openai_config = Some(openai_config),
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
        match proxies {
            Some(p) => self.proxies = Some(p.into()),
            _ => self.proxies = None,
        };
        self
    }

    /// Add blacklist urls to ignore.
    pub fn with_blacklist_url<T>(&mut self, blacklist_url: Option<Vec<T>>) -> &mut Self
    where
        Vec<CompactString>: From<Vec<T>>,
    {
        match blacklist_url {
            Some(p) => self.blacklist_url = Some(Box::new(p.into())),
            _ => self.blacklist_url = None,
        };
        self
    }

    /// Set HTTP headers for request using [reqwest::header::HeaderMap](https://docs.rs/reqwest/latest/reqwest/header/struct.HeaderMap.html).
    pub fn with_headers(&mut self, headers: Option<reqwest::header::HeaderMap>) -> &mut Self {
        match headers {
            Some(m) => self.headers = Some(m.into()),
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

    #[cfg(feature = "budget")]
    /// Set a crawl page limit. If the value is 0 there is no limit. This does nothing without the feat flag `budget` enabled.
    pub fn with_limit(&mut self, limit: u32) -> &mut Self {
        self.with_budget(Some(hashbrown::HashMap::from([("*", limit)])));
        self
    }

    #[cfg(not(feature = "budget"))]
    /// Set a crawl page limit. If the value is 0 there is no limit. This does nothing without the feat flag `budget` enabled.
    pub fn with_limit(&mut self, _limit: u32) -> &mut Self {
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

    #[cfg(feature = "budget")]
    /// Set a crawl depth limit. If the value is 0 there is no limit. This does nothing without the feat flag `budget` enabled.
    pub fn with_depth(&mut self, depth: usize) -> &mut Self {
        self.depth = depth;
        self
    }

    #[cfg(not(feature = "budget"))]
    /// Set a crawl depth limit. If the value is 0 there is no limit. This does nothing without the feat flag `budget` enabled.
    pub fn with_depth(&mut self, _depth: usize) -> &mut Self {
        self
    }

    #[cfg(feature = "cache")]
    /// Cache the page following HTTP rules. This method does nothing if the `cache` feature is not enabled.
    pub fn with_caching(&mut self, cache: bool) -> &mut Self {
        self.cache = cache;
        self
    }

    #[cfg(not(feature = "cache"))]
    /// Cache the page following HTTP rules. This method does nothing if the `cache` feature is not enabled.
    pub fn with_caching(&mut self, _cache: bool) -> &mut Self {
        self
    }

    /// Configures the view port for chrome. This method does nothing if the `chrome` feature is not enabled.
    #[cfg(not(feature = "chrome"))]
    pub fn with_viewport(
        &mut self,
        _viewport: Option<crate::configuration::Viewport>,
    ) -> &mut Self {
        self
    }

    /// Configures the viewport of the browser, which defaults to 800x600. This method does nothing if the [chrome] feature is not enabled.
    #[cfg(feature = "chrome")]
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
    /// Wait for a selector. This method does nothing if the [chrome] feature is not enabled.
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
    /// Wait for with delay. Should only be used for testing. This method does nothing if the [chrome] feature is not enabled.
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
    /// Wait for with delay. Should only be used for testing. This method does nothing if the [chrome] feature is not enabled.
    pub fn with_wait_for_delay(&mut self, _wait_for_delay: Option<WaitForDelay>) -> &mut Self {
        self
    }

    #[cfg(feature = "chrome_intercept")]
    /// Use request intercept for the request to only allow content that matches the host. If the content is from a 3rd party it needs to be part of our include list. This method does nothing if the `chrome_intercept` is not enabled.
    pub fn with_chrome_intercept(
        &mut self,
        chrome_intercept: bool,
        block_images: bool,
    ) -> &mut Self {
        self.chrome_intercept = chrome_intercept;
        self.chrome_intercept_block_visuals = block_images;
        self
    }

    #[cfg(not(feature = "chrome_intercept"))]
    /// Use request intercept for the request to only allow content required for the page that matches the host. If the content is from a 3rd party it needs to be part of our include list. This method does nothing if the `chrome_intercept` is not enabled.
    pub fn with_chrome_intercept(
        &mut self,
        _chrome_intercept: bool,
        _block_images: bool,
    ) -> &mut Self {
        self
    }

    #[cfg(feature = "budget")]
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

    #[cfg(not(feature = "budget"))]
    /// Set a crawl budget per path with levels support /a/b/c or for all paths with "*". This does nothing without the `budget` flag enabled.
    pub fn with_budget(&mut self, _budget: Option<hashbrown::HashMap<&str, u32>>) -> &mut Self {
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
                            match url::Url::parse(&d) {
                                Ok(d) => Some(d.host_str().unwrap_or_default().into()),
                                _ => None,
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

    /// Build the website configuration when using with_builder.
    pub fn build(&self) -> Self {
        self.to_owned()
    }
}
