use crate::black_list::contains;
use crate::compact_str::CompactString;
use crate::configuration::{
    self, get_ua, AutomationScriptsMap, Configuration, ExecutionScriptsMap, RedirectPolicy,
};
use crate::features::chrome_common::RequestInterceptConfiguration;
use crate::packages::robotparser::parser::RobotFileParser;
use crate::page::{Page, PageLinkBuildSettings};
use crate::utils::abs::{convert_abs_url, parse_absolute_url};
use crate::utils::{
    emit_log, emit_log_shutdown, get_semaphore, setup_website_selectors, spawn_set, spawn_task,
    AllowedDomainTypes,
};

use crate::utils::interner::ListBucket;
use crate::CaseInsensitiveString;
use crate::Client;
use crate::RelativeSelectors;
#[cfg(feature = "cron")]
use async_job::{async_trait, Job, Runner};
use hashbrown::{HashMap, HashSet};
use reqwest::redirect::Policy;
use reqwest::StatusCode;
use std::sync::atomic::{AtomicBool, AtomicI8, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::{
    sync::{broadcast, Semaphore},
    task::JoinSet,
    time::Interval,
};
use tokio_stream::StreamExt;
use url::Url;

#[cfg(feature = "cache_request")]
use http_cache_reqwest::{Cache, CacheMode, HttpCache, HttpCacheOptions};

// Use CACacheManager when cache_request and cache are set
#[cfg(all(
    feature = "cache_request",
    feature = "cache",
    not(feature = "cache_mem")
))]
use http_cache_reqwest::CACacheManager;
#[cfg(all(
    feature = "cache_request",
    feature = "cache",
    not(feature = "cache_mem")
))]
type CacheManager = CACacheManager;

// Use MokaManager when cache_request and cache_mem are set
#[cfg(all(
    feature = "cache_request",
    feature = "cache_mem",
    not(feature = "cache")
))]
use http_cache_reqwest::MokaManager;
#[cfg(all(
    feature = "cache_request",
    feature = "cache_mem",
    not(feature = "cache")
))]
type CacheManager = MokaManager;

// Default to CACacheManager if only cache_request is set, without cache or cache_mem
#[cfg(all(
    feature = "cache_request",
    not(feature = "cache"),
    not(feature = "cache_mem")
))]
use http_cache_reqwest::CACacheManager as DefaultCacheManager;
#[cfg(all(
    feature = "cache_request",
    not(feature = "cache"),
    not(feature = "cache_mem")
))]
type CacheManager = DefaultCacheManager;

#[cfg(feature = "cache_request")]
lazy_static! {
    /// Cache manager for request.
    pub static ref CACACHE_MANAGER: CacheManager = CacheManager::default();
}

/// The max backoff duration in seconds.
const BACKOFF_MAX_DURATION: tokio::time::Duration = tokio::time::Duration::from_secs(60);

/// calculate the base limits
pub fn calc_limits(multiplier: usize) -> usize {
    let logical = num_cpus::get();
    let physical = num_cpus::get_physical();

    let sem_limit = if logical > physical {
        (logical) / (physical)
    } else {
        logical
    };

    let (sem_limit, sem_max) = if logical == physical {
        (sem_limit * physical, 30 * multiplier)
    } else {
        (sem_limit * 2, 20 * multiplier)
    };

    sem_limit.max(sem_max)
}

lazy_static! {
    /// The default Semaphore limits.
    static ref DEFAULT_PERMITS: usize = calc_limits(1);
    /// The shared global Semaphore.
    pub(crate) static ref SEM_SHARED: Arc<Semaphore> = {
        let base_limit = match std::env::var("SEMAPHORE_MULTIPLIER") {
            Ok(multiplier) => match multiplier.parse::<isize>() {
                Ok(parsed_value) => (*DEFAULT_PERMITS as isize)
                    .wrapping_mul(parsed_value)
                    .max(1) as usize,
                Err(_) => *DEFAULT_PERMITS,
            },
            _ => *DEFAULT_PERMITS,
        };
        Arc::new(Semaphore::const_new(base_limit))
    };
}

#[cfg(not(feature = "decentralized"))]
lazy_static! {
    /// The global Semaphore.
    static ref SEM: Semaphore = {
        let base_limit = calc_limits(1);

        let base_limit = match std::env::var("SEMAPHORE_MULTIPLIER") {
            Ok(multiplier) => match multiplier.parse::<isize>() {
                Ok(parsed_value) => (base_limit as isize * parsed_value).max(1) as usize,
                Err(_) => base_limit,
            },
            _ => base_limit,
        };

        Semaphore::const_new(base_limit)
    };
}

#[cfg(feature = "decentralized")]
lazy_static! {
    /// The global worker count.
    static ref WORKERS: HashSet<String> = {
        let mut set: HashSet<_> = HashSet::new();

        for worker in std::env::var("SPIDER_WORKER_SCRAPER")
            .unwrap_or_else(|_| "http://127.0.0.1:3031".to_string())
            .split(",")
        {
            set.insert(worker.to_string());
        }

        for worker in std::env::var("SPIDER_WORKER")
            .unwrap_or_else(|_| "http://127.0.0.1:3030".to_string())
            .split(",")
        {
            set.insert(worker.to_string());
        }

        set
    };
    static ref SEM: Semaphore = {
        let sem_limit = calc_limits(3);
        Semaphore::const_new(sem_limit * WORKERS.len())
    };
}

lazy_static! {
    static ref WILD_CARD_PATH: CaseInsensitiveString = CaseInsensitiveString::from("*");
}

const INVALID_URL: &str = "The domain should be a valid URL, refer to <https://www.w3.org/TR/2011/WD-html5-20110525/urls.html#valid-url>.";

/// the active status of the crawl.
#[derive(Debug, Clone, Default, PartialEq, Eq, strum::EnumString, strum::Display)]
pub enum CrawlStatus {
    /// The crawl did not start yet.
    #[default]
    Start,
    /// The crawl is idle and has completed.
    Idle,
    /// The crawl is active.
    Active,
    /// The crawl blocked from network ratelimit, firewall, etc.
    Blocked,
    /// The crawl failed from a server error.
    ServerError,
    /// The crawl was rate limited.
    RateLimited,
    /// The initial request ran without returning html.
    Empty,
    /// The URL of the website is invalid. Crawl cannot commence.
    Invalid,
    #[cfg(feature = "control")]
    /// The crawl shutdown manually.
    Shutdown,
    #[cfg(feature = "control")]
    /// The crawl paused manually.
    Paused,
}

/// The link activity for the crawl.
#[derive(Debug, Clone, Default, PartialEq, Eq, strum::EnumString, strum::Display)]
pub enum ProcessLinkStatus {
    /// The link can process.
    #[default]
    Allowed,
    /// The link is blocked.
    Blocked,
    /// The budget is exceeded for the crawl.
    BudgetExceeded,
}

/// The type of cron job to run
#[derive(Debug, Clone, Default, PartialEq, Eq, strum::EnumString, strum::Display)]
pub enum CronType {
    #[default]
    /// Crawl collecting links, page data, and etc.
    Crawl,
    /// Scrape collecting links, page data as bytes to store, and etc.
    Scrape,
}

/// Represents a website to crawl and gather all links or page content.
/// ```rust
/// use spider::website::Website;
/// let mut website = Website::new("http://example.com");
/// website.crawl();
/// // `Website` will be filled with links or pages when crawled. If you need pages with the resource
/// // call the `website.scrape` method with `website.get_pages` instead.
/// for link in website.get_links() {
///     // do something
/// }
/// ```
#[derive(Debug, Clone, Default)]
pub struct Website {
    /// Configuration properties for website.
    pub configuration: Box<Configuration>,
    /// All URLs visited.
    links_visited: Box<ListBucket>,
    /// Extra links to crawl.
    extra_links: Box<HashSet<CaseInsensitiveString>>,
    /// Pages visited.
    pages: Option<Box<Vec<Page>>>,
    /// Robot.txt parser.
    robot_file_parser: Option<Box<RobotFileParser>>,
    /// Base url of the crawl.
    url: Box<CaseInsensitiveString>,
    /// The domain url parsed.
    domain_parsed: Option<Box<Url>>,
    /// The callback when a link is found.
    pub on_link_find_callback: Option<
        fn(CaseInsensitiveString, Option<String>) -> (CaseInsensitiveString, Option<String>),
    >,
    /// Subscribe and broadcast changes.
    channel: Option<(broadcast::Sender<Page>, Arc<broadcast::Receiver<Page>>)>,
    /// Guard counter for channel handling. This prevents things like the browser from closing after the crawl so that subscriptions can finalize events.
    channel_guard: Option<ChannelGuard>,
    /// Send links to process during the crawl.
    channel_queue: Option<(broadcast::Sender<String>, Arc<broadcast::Receiver<String>>)>,
    /// The status of the active crawl this is mapped to a general status and not the HTTP status code.
    status: CrawlStatus,
    /// The initial status code of the first request.
    initial_status_code: StatusCode,
    /// Set the crawl ID to track. This allows explicit targeting for shutdown, pause, and etc.
    #[cfg(feature = "control")]
    pub crawl_id: Box<String>,
    /// The website was manually stopped.
    shutdown: bool,
    /// The request client. Stored for re-use between runs.
    client: Option<Client>,
}

impl Website {
    /// Initialize Website object with a start link to crawl.
    pub fn new(url: &str) -> Self {
        let url = url.trim();
        let url: Box<CaseInsensitiveString> = if url.starts_with("http") {
            CaseInsensitiveString::new(&url).into()
        } else {
            CaseInsensitiveString::new(&string_concat!("https://", url)).into()
        };

        let domain_parsed: Option<Box<Url>> = parse_absolute_url(&url);

        Self {
            configuration: Configuration::new().into(),
            links_visited: Box::new(ListBucket::new()),
            pages: None,
            robot_file_parser: None,
            on_link_find_callback: None,
            channel: None,
            status: CrawlStatus::Start,
            shutdown: false,
            domain_parsed,
            url,
            ..Default::default()
        }
    }

    /// Set the url of the website to re-use configuration and data.
    pub fn set_url(&mut self, url: &str) -> &mut Self {
        let url = if url.starts_with(' ') || url.ends_with(' ') {
            url.trim()
        } else {
            url
        };
        let domain: Box<CaseInsensitiveString> = if url.starts_with("http") {
            CaseInsensitiveString::new(&url).into()
        } else {
            CaseInsensitiveString::new(&string_concat!("https://", url)).into()
        };
        self.domain_parsed = parse_absolute_url(&domain);
        self.url = domain;
        self
    }

    /// Return `false` if the crawl should shutdown. Process in between each link.
    async fn handle_process<T>(
        &self,
        handle: &Option<Arc<AtomicI8>>,
        interval: &mut Interval,
        shutdown: T,
    ) -> bool
    where
        T: std::future::Future<Output = ()>,
    {
        if self.shutdown {
            (shutdown).await;
            false
        } else {
            match handle.as_ref() {
                Some(handle) => {
                    while handle.load(Ordering::Relaxed) == 1 {
                        interval.tick().await;
                    }
                    if handle.load(Ordering::Relaxed) == 2 {
                        (shutdown).await;
                        false
                    } else {
                        true
                    }
                }
                _ => true,
            }
        }
    }

    /// return `true` if URL:
    ///
    /// - is not already crawled
    /// - is not over crawl budget
    /// - is optionally whitelisted
    /// - is not blacklisted
    /// - is not forbidden in robot.txt file (if parameter is defined)
    #[inline]
    #[cfg(not(feature = "regex"))]
    pub fn is_allowed(&mut self, link: &CaseInsensitiveString) -> ProcessLinkStatus {
        if self.links_visited.contains(link) {
            ProcessLinkStatus::Blocked
        } else {
            let status = self.is_allowed_default(link.inner());

            if status.eq(&ProcessLinkStatus::Allowed) && self.is_over_budget(link) {
                ProcessLinkStatus::BudgetExceeded
            } else {
                status
            }
        }
    }

    /// return `true` if URL:
    ///
    /// - is not already crawled
    /// - is not over crawl budget
    /// - is optionally whitelisted
    /// - is not blacklisted
    /// - is not forbidden in robot.txt file (if parameter is defined)
    #[inline]
    #[cfg(feature = "regex")]
    pub fn is_allowed(&mut self, link: &CaseInsensitiveString) -> ProcessLinkStatus {
        if self.links_visited.contains(link) {
            ProcessLinkStatus::Blocked
        } else {
            let status = self.is_allowed_default(link);
            if status.eq(&ProcessLinkStatus::Allowed) && self.is_over_budget(&link) {
                ProcessLinkStatus::BudgetExceeded
            } else {
                status
            }
        }
    }

    /// return `true` if URL:
    ///
    /// - is optionally whitelisted
    /// - is not blacklisted
    /// - is not forbidden in robot.txt file (if parameter is defined)
    #[inline]
    #[cfg(feature = "regex")]
    pub fn is_allowed_default(&self, link: &CaseInsensitiveString) -> ProcessLinkStatus {
        let blacklist = self.configuration.get_blacklist_compiled();
        let whitelist = self.configuration.get_whitelist_compiled();

        let blocked_whitelist = !whitelist.is_empty() && !contains(&whitelist, link.inner());
        let blocked_blacklist = !blacklist.is_empty() && contains(&blacklist, link.inner());

        if blocked_whitelist || blocked_blacklist || !self.is_allowed_robots(&link.as_ref()) {
            ProcessLinkStatus::Blocked
        } else {
            ProcessLinkStatus::Allowed
        }
    }

    /// return `true` if URL:
    ///
    /// - is optionally whitelisted
    /// - is not blacklisted
    /// - is not forbidden in robot.txt file (if parameter is defined)
    #[inline]
    #[cfg(not(feature = "regex"))]
    pub fn is_allowed_default(&self, link: &CompactString) -> ProcessLinkStatus {
        let whitelist = self.configuration.get_whitelist_compiled();
        let blacklist = self.configuration.get_blacklist_compiled();

        let blocked_whitelist = !whitelist.is_empty() && !contains(whitelist, link);
        let blocked_blacklist = !blacklist.is_empty() && contains(blacklist, link);

        if blocked_whitelist || blocked_blacklist || !self.is_allowed_robots(link) {
            ProcessLinkStatus::Blocked
        } else {
            ProcessLinkStatus::Allowed
        }
    }

    /// return `true` if URL:
    ///
    /// - is not forbidden in robot.txt file (if parameter is defined)
    pub fn is_allowed_robots(&self, link: &str) -> bool {
        if self.configuration.respect_robots_txt {
            match self.robot_file_parser.as_ref() {
                Some(r) => r.can_fetch(
                    match self.configuration.user_agent {
                        Some(ref ua) => ua,
                        _ => "*",
                    },
                    link,
                ),
                _ => true,
            }
        } else {
            true
        }
    }

    /// Detect if the inner budget is exceeded
    pub(crate) fn is_over_inner_depth_budget(&mut self, link: &CaseInsensitiveString) -> bool {
        match Url::parse(link.inner()) {
            Ok(r) => match r.path_segments() {
                Some(segments) => {
                    let mut over = false;
                    let mut depth: usize = 0;

                    for _ in segments {
                        depth = depth.saturating_add(1);
                        if depth >= self.configuration.depth_distance {
                            over = true;
                            break;
                        }
                    }

                    over
                }
                _ => false,
            },
            _ => false,
        }
    }

    /// Detect if the inner budget is exceeded
    pub(crate) fn is_over_inner_budget(&mut self, link: &CaseInsensitiveString) -> bool {
        match self.configuration.inner_budget.as_mut() {
            Some(budget) => {
                let exceeded_wild_budget = if self.configuration.wild_card_budgeting {
                    if let Some(budget) = budget.get_mut(&*WILD_CARD_PATH) {
                        if budget.abs_diff(0) == 1 {
                            true
                        } else {
                            *budget -= 1;
                            false
                        }
                    } else {
                        false
                    }
                } else {
                    false
                };

                // set this up prior to crawl to avoid checks per link.
                // If only the wild card budget is set we can safely skip all checks.
                let skip_paths = self.configuration.wild_card_budgeting && budget.len() == 1;
                let has_depth_control = self.configuration.depth_distance > 0;

                // check if paths pass
                if !skip_paths && !exceeded_wild_budget {
                    match Url::parse(link.inner()) {
                        Ok(r) => match r.path_segments() {
                            Some(segments) => {
                                let mut joint_segment = CaseInsensitiveString::default();
                                let mut over = false;
                                let mut depth: usize = 0;

                                for seg in segments {
                                    if has_depth_control {
                                        depth = depth.saturating_add(1);
                                        if depth >= self.configuration.depth_distance {
                                            over = true;
                                            break;
                                        }
                                    }

                                    joint_segment.push_str(seg);

                                    if budget.contains_key(&joint_segment) {
                                        if let Some(budget) = budget.get_mut(&joint_segment) {
                                            if budget.abs_diff(0) == 0 || *budget == 0 {
                                                over = true;
                                                break;
                                            } else {
                                                *budget -= 1;
                                                continue;
                                            }
                                        }
                                    }
                                }

                                over
                            }
                            _ => false,
                        },
                        _ => false,
                    }
                } else {
                    exceeded_wild_budget
                }
            }
            _ => false,
        }
    }

    /// Validate if url exceeds crawl budget and should not be handled.
    pub(crate) fn is_over_budget(&mut self, link: &CaseInsensitiveString) -> bool {
        let has_depth_control = self.configuration.depth_distance > 0;

        if self.configuration.inner_budget.is_some() || has_depth_control {
            if self.configuration.inner_budget.is_none() && has_depth_control {
                self.is_over_inner_depth_budget(link)
            } else {
                self.is_over_inner_budget(link)
            }
        } else {
            false
        }
    }

    /// Amount of pages crawled.
    pub fn size(&self) -> usize {
        self.links_visited.len()
    }

    /// Drain the extra links used for things like the sitemap.
    pub fn drain_extra_links(&mut self) -> hashbrown::hash_set::Drain<'_, CaseInsensitiveString> {
        self.extra_links.drain()
    }

    /// Get the initial status code of the request
    pub fn get_initial_status_code(&self) -> &StatusCode {
        &self.initial_status_code
    }

    /// Drain the links visited.
    #[cfg(any(
        feature = "string_interner_bucket_backend",
        feature = "string_interner_string_backend",
        feature = "string_interner_buffer_backend",
    ))]
    pub fn drain_links(
        &mut self,
    ) -> hashbrown::hash_set::Drain<'_, string_interner::symbol::SymbolUsize> {
        self.links_visited.drain()
    }

    #[cfg(not(any(
        feature = "string_interner_bucket_backend",
        feature = "string_interner_string_backend",
        feature = "string_interner_buffer_backend",
    )))]
    /// Drain the links visited.
    pub fn drain_links(&mut self) -> hashbrown::hash_set::Drain<'_, CaseInsensitiveString> {
        self.links_visited.drain()
    }

    /// Set extra links to crawl. This could be used in conjuntion with 'website.persist_links' to extend the crawl on the next run.
    pub fn set_extra_links(
        &mut self,
        extra_links: HashSet<CaseInsensitiveString>,
    ) -> &HashSet<CaseInsensitiveString> {
        self.extra_links.extend(extra_links);
        &self.extra_links
    }

    /// Clear all pages and links stored.
    pub fn clear(&mut self) {
        self.links_visited.clear();
        self.pages.take();
        self.extra_links.clear();
    }

    /// Get the HTTP request client. The client is set after the crawl has started.
    pub fn get_client(&self) -> &Option<Client> {
        &self.client
    }

    /// Page getter.
    pub fn get_pages(&self) -> Option<&Box<Vec<Page>>> {
        self.pages.as_ref()
    }

    /// Links visited getter.
    pub fn get_links(&self) -> HashSet<CaseInsensitiveString> {
        self.links_visited.get_links()
    }

    /// Domain parsed url getter.
    pub fn get_url_parsed(&self) -> &Option<Box<Url>> {
        &self.domain_parsed
    }

    /// Domain name getter.
    pub fn get_url(&self) -> &CaseInsensitiveString {
        &self.url
    }

    /// Crawl delay getter.
    pub fn get_delay(&self) -> Duration {
        Duration::from_millis(self.configuration.delay)
    }

    /// Get the active crawl status.
    pub fn get_status(&self) -> &CrawlStatus {
        &self.status
    }

    /// Set the crawl status to persist between the run.
    /// Example crawling a sitemap and all links after - website.crawl_sitemap().await.persist_links().crawl().await
    pub fn persist_links(&mut self) -> &mut Self {
        self.status = CrawlStatus::Active;
        self
    }

    /// Absolute base url of crawl.
    pub fn get_absolute_path(&self, domain: Option<&str>) -> Option<Url> {
        if domain.is_some() {
            match url::Url::parse(domain.unwrap_or_default()) {
                Ok(mut u) => {
                    if let Ok(mut path) = u.path_segments_mut() {
                        path.clear();
                    }
                    Some(u)
                }
                _ => None,
            }
        } else if let Some(mut d) = self.domain_parsed.as_deref().cloned() {
            if let Ok(mut path) = d.path_segments_mut() {
                path.clear();
            }
            Some(d)
        } else {
            None
        }
    }

    /// Stop all crawls for the website.
    pub fn stop(&mut self) {
        self.shutdown = true;
    }

    /// Crawls commenced from fresh run.
    fn start(&mut self) {
        self.shutdown = false;
    }

    /// configure the robots parser on initial crawl attempt and run.
    pub async fn configure_robots_parser(&mut self, client: Client) -> Client {
        if self.configuration.respect_robots_txt {
            let robot_file_parser = self
                .robot_file_parser
                .get_or_insert_with(RobotFileParser::new);

            if robot_file_parser.mtime() <= 4000 {
                let host_str = match &self.domain_parsed {
                    Some(domain) => domain.as_str(),
                    _ => self.url.inner(),
                };
                if host_str.ends_with('/') {
                    robot_file_parser.read(&client, host_str).await;
                } else {
                    robot_file_parser
                        .read(&client, &string_concat!(host_str, "/"))
                        .await;
                }

                if let Some(delay) =
                    robot_file_parser.get_crawl_delay(&self.configuration.user_agent)
                {
                    self.configuration.delay = delay.as_millis().min(60000) as u64;
                }
            }
        }

        client
    }

    /// Setup strict a strict redirect policy for request. All redirects need to match the host.
    fn setup_strict_policy(&self) -> Policy {
        use crate::page::domain_name;
        use reqwest::redirect::Attempt;
        use std::sync::atomic::AtomicU8;

        let default_policy = reqwest::redirect::Policy::default();

        match self.domain_parsed.as_deref().cloned() {
            Some(host_s) => {
                let initial_redirect_limit = if self.configuration.respect_robots_txt {
                    2
                } else {
                    1
                };
                let subdomains = self.configuration.subdomains;
                let tld = self.configuration.tld;
                let host_domain_name = if tld {
                    domain_name(&host_s).to_string()
                } else {
                    Default::default()
                };
                let redirect_limit = *self.configuration.redirect_limit;

                let custom_policy = {
                    let initial_redirect = Arc::new(AtomicU8::new(0));

                    move |attempt: Attempt| {
                        if tld && domain_name(attempt.url()) == host_domain_name
                            || subdomains
                                && attempt
                                    .url()
                                    .host_str()
                                    .unwrap_or_default()
                                    .ends_with(host_s.host_str().unwrap_or_default())
                            || attempt.url().host() == host_s.host()
                        {
                            default_policy.redirect(attempt)
                        } else if attempt.previous().len() > redirect_limit {
                            attempt.error("too many redirects")
                        } else if attempt.status().is_redirection()
                            && (0..initial_redirect_limit)
                                .contains(&initial_redirect.load(Ordering::Relaxed))
                        {
                            initial_redirect.fetch_add(1, Ordering::Relaxed);
                            default_policy.redirect(attempt)
                        } else {
                            attempt.stop()
                        }
                    }
                };
                reqwest::redirect::Policy::custom(custom_policy)
            }
            _ => default_policy,
        }
    }

    /// Setup redirect policy for reqwest.
    fn setup_redirect_policy(&self) -> Policy {
        match self.configuration.redirect_policy {
            RedirectPolicy::Loose => {
                reqwest::redirect::Policy::limited(*self.configuration.redirect_limit)
            }
            RedirectPolicy::Strict => self.setup_strict_policy(),
        }
    }

    /// Determine if the agent should be set to a Chrome Agent.
    #[cfg(not(feature = "chrome"))]
    fn only_chrome_agent(&self) -> bool {
        false
    }

    /// Determine if the agent should be set to a Chrome Agent.
    #[cfg(feature = "chrome")]
    fn only_chrome_agent(&self) -> bool {
        self.configuration.chrome_connection_url.is_some()
            || self.configuration.wait_for.is_some()
            || self.configuration.chrome_intercept.enabled
            || self.configuration.stealth_mode
            || self.configuration.fingerprint
    }

    /// Build the HTTP client.
    #[cfg(all(not(feature = "decentralized"), not(feature = "cache_request")))]
    fn configure_http_client_builder(&mut self) -> crate::ClientBuilder {
        use reqwest::header::HeaderMap;

        let policy = self.setup_redirect_policy();
        let mut headers: HeaderMap = HeaderMap::new();

        let user_agent = match &self.configuration.user_agent {
            Some(ua) => ua.as_str(),
            _ => get_ua(self.only_chrome_agent()),
        };

        if cfg!(feature = "real_browser") {
            headers.extend(crate::utils::header_utils::get_mimic_headers(user_agent));
        }

        let client = Client::builder()
            .user_agent(user_agent)
            .redirect(policy)
            .danger_accept_invalid_certs(self.configuration.accept_invalid_certs)
            .tcp_keepalive(Duration::from_millis(500));

        let client = if self.configuration.http2_prior_knowledge {
            client.http2_prior_knowledge()
        } else {
            client
        };

        let client = crate::utils::header_utils::setup_default_headers(
            client,
            &self.configuration,
            headers,
            self.get_url_parsed(),
        );

        let mut client = match &self.configuration.request_timeout {
            Some(t) => client.timeout(**t),
            _ => client,
        };

        let client = match &self.configuration.proxies {
            Some(proxies) => {
                for proxie in proxies.iter() {
                    if let Ok(proxy) = reqwest::Proxy::all(proxie) {
                        client = client.proxy(proxy);
                    }
                }
                client
            }
            _ => client,
        };

        self.configure_http_client_cookies(client)
    }

    /// Build the HTTP client with caching enabled.
    #[cfg(all(not(feature = "decentralized"), feature = "cache_request"))]
    fn configure_http_client_builder(&mut self) -> crate::ClientBuilder {
        use reqwest::header::HeaderMap;
        use reqwest_middleware::ClientBuilder;

        let mut headers = HeaderMap::new();

        let policy = self.setup_redirect_policy();
        let user_agent = match &self.configuration.user_agent {
            Some(ua) => ua.as_str(),
            _ => &get_ua(self.only_chrome_agent()),
        };

        if cfg!(feature = "real_browser") {
            headers.extend(crate::utils::header_utils::get_mimic_headers(user_agent));
        }

        let client = reqwest::Client::builder()
            .user_agent(user_agent)
            .danger_accept_invalid_certs(self.configuration.accept_invalid_certs)
            .redirect(policy)
            .tcp_keepalive(Duration::from_millis(500));

        let client = if self.configuration.http2_prior_knowledge {
            client.http2_prior_knowledge()
        } else {
            client
        };

        let client = crate::utils::header_utils::setup_default_headers(
            client,
            &self.configuration,
            headers,
            self.get_url_parsed(),
        );

        let mut client = match &self.configuration.request_timeout {
            Some(t) => client.timeout(**t),
            _ => client,
        };

        let client = match &self.configuration.proxies {
            Some(proxies) => {
                for proxie in proxies.iter() {
                    if let Ok(proxy) = reqwest::Proxy::all(proxie) {
                        client = client.proxy(proxy);
                    }
                }
                client
            }
            _ => client,
        };

        let client = self.configure_http_client_cookies(client);
        let client = ClientBuilder::new(unsafe { client.build().unwrap_unchecked() });

        if self.configuration.cache {
            client.with(Cache(HttpCache {
                mode: CacheMode::Default,
                manager: CACACHE_MANAGER.clone(),
                options: HttpCacheOptions::default(),
            }))
        } else {
            client
        }
    }

    /// Build the HTTP client with cookie configurations.
    #[cfg(all(not(feature = "decentralized"), feature = "cookies"))]
    fn configure_http_client_cookies(
        &mut self,
        client: reqwest::ClientBuilder,
    ) -> reqwest::ClientBuilder {
        let client = client.cookie_store(true);
        if !self.configuration.cookie_str.is_empty() && self.domain_parsed.is_some() {
            match self.domain_parsed.clone() {
                Some(p) => {
                    let cookie_store = reqwest::cookie::Jar::default();
                    cookie_store.add_cookie_str(&self.configuration.cookie_str, &p);
                    client.cookie_provider(cookie_store.into())
                }
                _ => client,
            }
        } else {
            client
        }
    }

    /// Build the client with cookie configurations. This does nothing with [cookies] flag enabled.
    #[cfg(all(not(feature = "decentralized"), not(feature = "cookies")))]
    fn configure_http_client_cookies(
        &mut self,
        client: reqwest::ClientBuilder,
    ) -> reqwest::ClientBuilder {
        client
    }

    /// Set the HTTP client to use directly. This is helpful if you manually call 'website.configure_http_client' before the crawl.
    pub fn set_http_client(&mut self, client: Client) -> &Option<Client> {
        self.client = Some(client);
        &self.client
    }

    /// Configure http client.
    #[cfg(all(not(feature = "decentralized"), not(feature = "cache_request")))]
    pub fn configure_http_client(&mut self) -> Client {
        let client = self.configure_http_client_builder();
        // should unwrap using native-tls-alpn
        unsafe { client.build().unwrap_unchecked() }
    }

    /// Configure http client.
    #[cfg(all(not(feature = "decentralized"), feature = "cache_request"))]
    pub fn configure_http_client(&mut self) -> Client {
        let client = self.configure_http_client_builder();
        client.build()
    }

    /// Configure http client for decentralization.
    #[cfg(all(feature = "decentralized", not(feature = "cache_request")))]
    pub fn configure_http_client(&mut self) -> Client {
        use reqwest::header::HeaderMap;
        use reqwest::header::HeaderValue;

        let mut headers = HeaderMap::new();

        let policy = self.setup_redirect_policy();

        let mut client = Client::builder()
            .user_agent(match &self.configuration.user_agent {
                Some(ua) => ua.as_str(),
                _ => &get_ua(self.only_chrome_agent()),
            })
            .redirect(policy)
            .tcp_keepalive(Duration::from_millis(500));

        let referer = if self.configuration.tld && self.configuration.subdomains {
            2
        } else if self.configuration.tld {
            2
        } else if self.configuration.subdomains {
            1
        } else {
            0
        };

        if referer > 0 {
            // use expected http headers for providers that drop invalid headers
            headers.insert(reqwest::header::REFERER, HeaderValue::from(referer));
        }

        match &self.configuration.headers {
            Some(h) => headers.extend(h.inner().clone()),
            _ => (),
        };

        if let Some(domain_url) = self.get_absolute_path(None) {
            let domain_url = domain_url.as_str();
            let domain_host = if domain_url.ends_with("/") {
                &domain_url[0..domain_url.len() - 1]
            } else {
                domain_url
            };
            match HeaderValue::from_str(domain_host) {
                Ok(value) => {
                    headers.insert(reqwest::header::HOST, value);
                }
                _ => (),
            }
        }

        for worker in WORKERS.iter() {
            if let Ok(worker) = reqwest::Proxy::all(worker) {
                client = client.proxy(worker);
            }
        }

        // should unwrap using native-tls-alpn
        unsafe {
            match &self.configuration.request_timeout {
                Some(t) => client.timeout(**t),
                _ => client,
            }
            .default_headers(headers)
            .build()
            .unwrap_unchecked()
        }
    }

    /// Configure http client for decentralization.
    #[cfg(all(feature = "decentralized", feature = "cache_request"))]
    pub fn configure_http_client(&mut self) -> Client {
        use reqwest::header::HeaderMap;
        use reqwest::header::HeaderValue;
        use reqwest_middleware::ClientBuilder;

        let mut headers = HeaderMap::new();

        let policy = self.setup_redirect_policy();

        let mut client = reqwest::Client::builder()
            .user_agent(match &self.configuration.user_agent {
                Some(ua) => ua.as_str(),
                _ => &get_ua(self.only_chrome_agent()),
            })
            .redirect(policy)
            .tcp_keepalive(Duration::from_millis(500));

        let referer = if self.configuration.tld && self.configuration.subdomains {
            2
        } else if self.configuration.tld {
            2
        } else if self.configuration.subdomains {
            1
        } else {
            0
        };

        if referer > 0 {
            // use expected http headers for providers that drop invalid headers
            headers.insert(reqwest::header::REFERER, HeaderValue::from(referer));
        }

        match &self.configuration.headers {
            Some(h) => headers.extend(h.inner().clone()),
            _ => (),
        };

        match self.get_absolute_path(None) {
            Some(domain_url) => {
                let domain_url = domain_url.as_str();
                let domain_host = if domain_url.ends_with("/") {
                    &domain_url[0..domain_url.len() - 1]
                } else {
                    domain_url
                };
                match HeaderValue::from_str(domain_host) {
                    Ok(value) => {
                        headers.insert(reqwest::header::HOST, value);
                    }
                    _ => (),
                }
            }
            _ => (),
        }

        for worker in WORKERS.iter() {
            if let Ok(worker) = reqwest::Proxy::all(worker) {
                client = client.proxy(worker);
            }
        }

        let client = ClientBuilder::new(unsafe {
            match &self.configuration.request_timeout {
                Some(t) => client.timeout(**t),
                _ => client,
            }
            .default_headers(headers)
            .build()
            .unwrap_unchecked()
        })
        .with(Cache(HttpCache {
            mode: CacheMode::Default,
            manager: CACACHE_MANAGER.clone(),
            options: HttpCacheOptions::default(),
        }));

        client.build()
    }

    /// Setup atomic controller.
    #[cfg(feature = "control")]
    fn configure_handler(&self) -> (Arc<AtomicI8>, tokio::task::JoinHandle<()>) {
        use crate::utils::{Handler, CONTROLLER};
        let c: Arc<AtomicI8> = Arc::new(AtomicI8::new(0));
        let handle = c.clone();
        let target_id = string_concat!(self.crawl_id, self.url.inner());
        let c_lock = CONTROLLER.clone();

        let join_handle = spawn_task("control_handler", async move {
            let mut l = c_lock.read().await.1.to_owned();

            while l.changed().await.is_ok() {
                let n = &*l.borrow();
                let (target, rest) = n;

                if target_id.eq_ignore_ascii_case(&target) {
                    if rest == &Handler::Resume {
                        c.store(0, Ordering::Relaxed);
                    }
                    if rest == &Handler::Pause {
                        c.store(1, Ordering::Relaxed);
                    }
                    if rest == &Handler::Shutdown {
                        c.store(2, Ordering::Relaxed);
                    }
                }
            }
        });

        (handle, join_handle)
    }

    /// Setup interception for chrome request.
    #[cfg(all(feature = "chrome", feature = "chrome_intercept"))]
    async fn setup_chrome_interception(
        &self,
        page: &chromiumoxide::Page,
    ) -> Option<tokio::task::JoinHandle<()>> {
        crate::features::chrome::setup_chrome_interception_base(
            page,
            self.configuration.chrome_intercept.enabled,
            &self.configuration.auth_challenge_response,
            self.configuration.chrome_intercept.block_visuals,
            &self.url.inner().to_string(),
        )
        .await
    }

    /// Setup interception for chrome request
    #[cfg(all(feature = "chrome", not(feature = "chrome_intercept")))]
    async fn setup_chrome_interception(
        &self,
        _chrome_page: &chromiumoxide::Page,
    ) -> Option<tokio::task::JoinHandle<()>> {
        None
    }

    /// Setup selectors for handling link targets.
    fn setup_selectors(&self) -> Option<RelativeSelectors> {
        setup_website_selectors(
            self.get_url_parsed(),
            self.get_url().inner(),
            AllowedDomainTypes::new(self.configuration.subdomains, self.configuration.tld),
        )
    }

    /// Setup config for crawl.
    #[cfg(feature = "control")]
    async fn setup(&mut self) -> (Client, Option<(Arc<AtomicI8>, tokio::task::JoinHandle<()>)>) {
        self.determine_limits();

        if self.status != CrawlStatus::Active {
            self.clear();
        }

        let client = match self.client.take() {
            Some(client) => client,
            _ => self.configure_http_client(),
        };

        (
            self.configure_robots_parser(client).await,
            Some(self.configure_handler()),
        )
    }

    /// Setup config for crawl.
    #[cfg(not(feature = "control"))]
    async fn setup(&mut self) -> (Client, Option<(Arc<AtomicI8>, tokio::task::JoinHandle<()>)>) {
        self.determine_limits();

        if self.status != CrawlStatus::Active {
            self.clear();
        }

        let client = match self.client.take() {
            Some(client) => client,
            _ => self.configure_http_client(),
        };

        (self.configure_robots_parser(client).await, None)
    }

    /// Setup shared concurrent configs.
    fn setup_crawl(
        &mut self,
    ) -> (
        std::pin::Pin<Box<tokio::time::Interval>>,
        std::pin::Pin<Box<Duration>>,
    ) {
        self.status = CrawlStatus::Active;
        let interval = Box::pin(tokio::time::interval(Duration::from_millis(10)));
        let throttle = Box::pin(self.get_delay());

        (interval, throttle)
    }

    /// Get all the expanded links.
    #[cfg(feature = "glob")]
    fn get_expanded_links(&self, domain_name: &str) -> Vec<CaseInsensitiveString> {
        let mut expanded = crate::features::glob::expand_url(&domain_name);

        if expanded.len() == 0 {
            match self.get_absolute_path(Some(domain_name)) {
                Some(u) => {
                    expanded.push(u.as_str().into());
                }
                _ => (),
            };
        };

        expanded
    }

    /// Expand links for crawl.
    async fn _crawl_establish(
        &mut self,
        client: &Client,
        base: &mut RelativeSelectors,
        _: bool,
    ) -> HashSet<CaseInsensitiveString> {
        if self
            .is_allowed_default(self.get_base_link())
            .eq(&ProcessLinkStatus::Allowed)
        {
            let url = self.url.inner();

            let mut links: HashSet<CaseInsensitiveString> = HashSet::new();
            let mut links_ssg = links.clone();
            let mut links_pages = if self.configuration.return_page_links {
                Some(links.clone())
            } else {
                None
            };
            let mut page_links_settings =
                PageLinkBuildSettings::new(true, self.configuration.full_resources);

            page_links_settings.subdomains = self.configuration.subdomains;
            page_links_settings.tld = self.configuration.tld;

            let mut domain_parsed = self.domain_parsed.take();

            let mut page = Page::new_page_streaming(
                url,
                client,
                false,
                base,
                &self.configuration.external_domains_caseless,
                &page_links_settings,
                &mut links,
                Some(&mut links_ssg),
                &mut domain_parsed, // original domain
                &mut self.domain_parsed,
                &mut links_pages,
            )
            .await;

            if self.domain_parsed.is_none() {
                if let Some(mut domain_parsed) = domain_parsed.take() {
                    convert_abs_url(&mut domain_parsed);
                    self.domain_parsed.replace(domain_parsed);
                }
            }

            let mut retry_count = self.configuration.retry;
            let domains_caseless = &self.configuration.external_domains_caseless;

            while page.should_retry && retry_count > 0 {
                retry_count -= 1;
                if let Some(timeout) = page.get_timeout() {
                    tokio::time::sleep(timeout).await;
                }

                if page.status_code == StatusCode::GATEWAY_TIMEOUT {
                    let mut domain_parsed_clone = self.domain_parsed.clone();

                    if let Err(elasped) = tokio::time::timeout(BACKOFF_MAX_DURATION, async {
                        page.clone_from(
                            &Page::new_page_streaming(
                                url,
                                client,
                                false,
                                base,
                                domains_caseless,
                                &page_links_settings,
                                &mut links,
                                Some(&mut links_ssg),
                                &mut domain_parsed,
                                &mut domain_parsed_clone,
                                &mut links_pages,
                            )
                            .await,
                        );
                    })
                    .await
                    {
                        log::info!("backoff gateway timeout exceeded {elasped}");
                    }

                    self.domain_parsed = domain_parsed_clone;
                } else {
                    page.clone_from(
                        &Page::new_page_streaming(
                            url,
                            client,
                            false,
                            base,
                            &self.configuration.external_domains_caseless,
                            &page_links_settings,
                            &mut links,
                            Some(&mut links_ssg),
                            &mut domain_parsed,
                            &mut self.domain_parsed,
                            &mut links_pages,
                        )
                        .await,
                    );
                }
            }

            emit_log(url);

            self.links_visited.insert(match self.on_link_find_callback {
                Some(cb) => {
                    let c = cb(*self.url.clone(), None);
                    c.0
                }
                _ => *self.url.clone(),
            });

            if page.is_empty() {
                self.status = CrawlStatus::Empty;
            }

            if self.configuration.return_page_links {
                page.page_links = links_pages.filter(|pages| !pages.is_empty()).map(Box::new);
            }

            links.extend(links_ssg);

            self.initial_status_code = page.status_code;

            if page.status_code == reqwest::StatusCode::FORBIDDEN && links.is_empty() {
                self.status = CrawlStatus::Blocked;
            } else if page.status_code == reqwest::StatusCode::TOO_MANY_REQUESTS {
                self.status = CrawlStatus::RateLimited;
            } else if page.status_code.is_server_error() {
                self.status = CrawlStatus::ServerError;
            }

            channel_send_page(&self.channel, page, &self.channel_guard);

            links
        } else {
            HashSet::new()
        }
    }

    /// Expand links for crawl.
    #[cfg(all(not(feature = "decentralized"), feature = "chrome"))]
    async fn crawl_establish(
        &mut self,
        client: &Client,
        base: &mut RelativeSelectors,
        _: bool,
        chrome_page: &chromiumoxide::Page,
    ) -> HashSet<CaseInsensitiveString> {
        if self
            .is_allowed_default(&self.get_base_link())
            .eq(&ProcessLinkStatus::Allowed)
        {
            crate::features::chrome::setup_chrome_events(chrome_page, &self.configuration).await;

            let intercept_handle = self.setup_chrome_interception(&chrome_page).await;

            let mut page = Page::new(
                &self.url.inner(),
                &client,
                &chrome_page,
                &self.configuration.wait_for,
                &self.configuration.screenshot,
                false, // we use the initial about:blank page.
                &self.configuration.openai_config,
                &self.configuration.execution_scripts,
                &self.configuration.automation_scripts,
                &self.configuration.viewport,
                &self.configuration.request_timeout,
            )
            .await;

            if let Some(h) = intercept_handle {
                let abort_handle = h.abort_handle();
                if let Err(elasped) =
                    tokio::time::timeout(tokio::time::Duration::from_secs(10), h).await
                {
                    log::warn!("Handler timeout exceeded {elasped}");
                    abort_handle.abort();
                }
            }

            if let Some(ref domain) = page.final_redirect_destination {
                let domain: Box<CaseInsensitiveString> = CaseInsensitiveString::new(&domain).into();
                let prior_domain = self.domain_parsed.take();
                self.domain_parsed = parse_absolute_url(&domain);
                self.url = domain;

                if let Some(s) = self.setup_selectors() {
                    base.0 = s.0;
                    base.1 = s.1;

                    if let Some(pdname) = prior_domain {
                        if let Some(dname) = pdname.host_str() {
                            base.2 = dname.into();
                        }
                    }
                }
            }

            emit_log(&self.url.inner());

            self.links_visited.insert(match self.on_link_find_callback {
                Some(cb) => {
                    let c = cb(*self.url.clone(), None);

                    c.0
                }
                _ => *self.url.clone(),
            });

            // setup link tracking.
            if self.configuration.return_page_links && page.page_links.is_none() {
                page.page_links = Some(Box::new(Default::default()));
            }

            let links = if !page.is_empty() {
                page.links_ssg(&base, &client).await
            } else {
                self.status = CrawlStatus::Empty;
                Default::default()
            };

            self.initial_status_code = page.status_code;

            if page.status_code == reqwest::StatusCode::FORBIDDEN && links.len() == 0 {
                self.status = CrawlStatus::Blocked;
            } else if page.status_code == reqwest::StatusCode::TOO_MANY_REQUESTS {
                self.status = CrawlStatus::RateLimited;
            } else if page.status_code.is_server_error() {
                self.status = CrawlStatus::ServerError;
            }

            channel_send_page(&self.channel, page, &self.channel_guard);

            links
        } else {
            HashSet::new()
        }
    }

    /// fetch the page with chrome
    #[cfg(all(
        not(feature = "glob"),
        not(feature = "decentralized"),
        feature = "smart"
    ))]
    async fn render_chrome_page(
        config: &Configuration,
        client: &Client,
        browser: &Arc<chromiumoxide::Browser>,
        context_id: &Option<chromiumoxide::cdp::browser_protocol::browser::BrowserContextId>,
        page: &mut Page,
        url: &str,
    ) {
        if let Ok(chrome_page) = crate::features::chrome::attempt_navigation(
            "about:blank",
            &browser,
            &config.request_timeout,
            &context_id,
            &config.viewport,
        )
        .await
        {
            crate::features::chrome::setup_chrome_events(&chrome_page, &config).await;
            let intercept_handle = crate::features::chrome::setup_chrome_interception_base(
                &chrome_page,
                config.chrome_intercept.enabled,
                &config.auth_challenge_response,
                config.chrome_intercept.block_visuals,
                &url,
            )
            .await;

            let next_page = Page::new(
                &url,
                &client,
                &chrome_page,
                &config.wait_for,
                &config.screenshot,
                false, // we use the initial about:blank page.
                &config.openai_config,
                &config.execution_scripts,
                &config.automation_scripts,
                &config.viewport,
                &config.request_timeout,
            )
            .await;

            page.clone_from(&next_page);

            if let Some(h) = intercept_handle {
                let abort_handle = h.abort_handle();
                if let Err(elasped) =
                    tokio::time::timeout(tokio::time::Duration::from_secs(10), h).await
                {
                    log::warn!("Handler timeout exceeded {elasped}");
                    abort_handle.abort();
                }
            }
        }
    }

    /// Expand links for crawl.
    #[cfg(all(
        not(feature = "glob"),
        not(feature = "decentralized"),
        feature = "smart"
    ))]
    async fn crawl_establish_smart(
        &mut self,
        client: &Client,
        mut base: &mut RelativeSelectors,
        _: bool,
        browser: &Arc<chromiumoxide::Browser>,
        context_id: &Option<chromiumoxide::cdp::browser_protocol::browser::BrowserContextId>,
    ) -> HashSet<CaseInsensitiveString> {
        let links: HashSet<CaseInsensitiveString> = if self
            .is_allowed_default(&self.get_base_link())
            .eq(&ProcessLinkStatus::Allowed)
        {
            let url = self.url.inner();

            let mut page = Page::new_page(&url, &client).await;

            let mut retry_count = self.configuration.retry;

            while page.should_retry && retry_count > 0 {
                retry_count -= 1;
                if let Some(timeout) = page.get_timeout() {
                    tokio::time::sleep(timeout).await;
                }
                if page.status_code == StatusCode::GATEWAY_TIMEOUT {
                    if let Err(elasped) = tokio::time::timeout(BACKOFF_MAX_DURATION, async {
                        if retry_count.is_power_of_two() {
                            Website::render_chrome_page(
                                &self.configuration,
                                client,
                                browser,
                                context_id,
                                &mut page,
                                url,
                            )
                            .await;
                        } else {
                            let next_page = Page::new_page(url, &client).await;
                            page.clone_from(&next_page);
                        };
                    })
                    .await
                    {
                        log::warn!("backoff timeout {elasped}");
                    }
                } else {
                    if retry_count.is_power_of_two() {
                        Website::render_chrome_page(
                            &self.configuration,
                            client,
                            browser,
                            context_id,
                            &mut page,
                            url,
                        )
                        .await
                    } else {
                        page.clone_from(&Page::new_page(url, &client).await);
                    }
                }
            }

            let page_links: HashSet<CaseInsensitiveString> = page
                .smart_links(&base, &browser, &self.configuration, &context_id)
                .await;

            if let Some(ref domain) = page.final_redirect_destination {
                let prior_domain = self.domain_parsed.take();
                crate::utils::modify_selectors(
                    &prior_domain,
                    domain,
                    &mut self.domain_parsed,
                    &mut self.url,
                    &mut base,
                    AllowedDomainTypes::new(self.configuration.subdomains, self.configuration.tld),
                );
            }

            emit_log(&self.url.inner());

            self.links_visited.insert(match self.on_link_find_callback {
                Some(cb) => {
                    let c = cb(*self.url.clone(), None);

                    c.0
                }
                _ => *self.url.clone(),
            });

            let links = if !page_links.is_empty() {
                page_links
            } else {
                self.status = CrawlStatus::Empty;
                Default::default()
            };

            self.initial_status_code = page.status_code;

            if page.status_code == reqwest::StatusCode::FORBIDDEN && links.len() == 0 {
                self.status = CrawlStatus::Blocked;
            } else if page.status_code == reqwest::StatusCode::TOO_MANY_REQUESTS {
                self.status = CrawlStatus::RateLimited;
            } else if page.status_code.is_server_error() {
                self.status = CrawlStatus::ServerError;
            }
            if self.configuration.return_page_links {
                page.page_links = if links.is_empty() {
                    None
                } else {
                    Some(Box::new(links.clone()))
                };
            }

            channel_send_page(&self.channel, page, &self.channel_guard);

            links
        } else {
            HashSet::new()
        };

        links
    }

    /// Expand links for crawl.
    #[cfg(all(not(feature = "glob"), feature = "decentralized"))]
    async fn crawl_establish(
        &mut self,
        client: &Client,
        _: &(CompactString, smallvec::SmallVec<[CompactString; 2]>),
        http_worker: bool,
    ) -> HashSet<CaseInsensitiveString> {
        // base_domain name passed here is for primary url determination and not subdomain.tld placement
        let links: HashSet<CaseInsensitiveString> = if self
            .is_allowed_default(&self.get_base_link())
            .eq(&ProcessLinkStatus::Allowed)
        {
            let link = self.url.inner();

            let mut page = Page::new(
                &if http_worker && link.starts_with("https") {
                    link.replacen("https", "http", 1)
                } else {
                    link.to_string()
                },
                &client,
            )
            .await;

            self.links_visited.insert(match self.on_link_find_callback {
                Some(cb) => {
                    let c = cb(*self.url.to_owned(), None);

                    c.0
                }
                _ => *self.url.to_owned(),
            });

            self.initial_status_code = page.status_code;

            if page.status_code == reqwest::StatusCode::FORBIDDEN {
                self.status = CrawlStatus::Blocked;
            } else if page.status_code == reqwest::StatusCode::TOO_MANY_REQUESTS {
                self.status = CrawlStatus::RateLimited;
            } else if page.status_code.is_server_error() {
                self.status = CrawlStatus::ServerError;
            }

            // todo: pass full links to the worker to return.
            if self.configuration.return_page_links {
                page.page_links = Some(page.links.clone().into());
            }

            let links = HashSet::from(page.links.clone());

            channel_send_page(&self.channel, page, &self.channel_guard);

            links
        } else {
            HashSet::new()
        };

        links
    }

    /// Expand links for crawl.
    #[cfg(all(feature = "glob", feature = "decentralized"))]
    async fn crawl_establish(
        &mut self,
        client: &Client,
        _: &(CompactString, smallvec::SmallVec<[CompactString; 2]>),
        http_worker: bool,
    ) -> HashSet<CaseInsensitiveString> {
        let mut links: HashSet<CaseInsensitiveString> = HashSet::new();
        let expanded = self.get_expanded_links(&self.url.inner().as_str());
        self.configuration.configure_allowlist();

        for link in expanded {
            let allowed = self.is_allowed(&link);

            if allowed.eq(&ProcessLinkStatus::BudgetExceeded) {
                break;
            }
            if allowed.eq(&ProcessLinkStatus::Blocked) {
                continue;
            }

            let mut page = Page::new(
                &if http_worker && link.as_ref().starts_with("https") {
                    link.inner().replacen("https", "http", 1).to_string()
                } else {
                    link.inner().to_string()
                },
                &client,
            )
            .await;

            let u = page.get_url();
            let u = if u.is_empty() { link } else { u.into() };

            let link_result = match self.on_link_find_callback {
                Some(cb) => cb(u, None),
                _ => (u, None),
            };

            self.links_visited.insert(link_result.0);

            if self.configuration.return_page_links {
                page.page_links = Some(Default::default());
            }

            channel_send_page(&self.channel, page.clone(), &self.channel_guard);

            let page_links = HashSet::from(page.links);

            links.extend(page_links);
        }

        links
    }

    /// Expand links for crawl.
    #[cfg(all(feature = "glob", feature = "chrome", not(feature = "decentralized")))]
    async fn crawl_establish(
        &mut self,
        client: &Client,
        base: &mut (CompactString, smallvec::SmallVec<[CompactString; 2]>),
        _: bool,
        page: &chromiumoxide::Page,
    ) -> HashSet<CaseInsensitiveString> {
        let mut links: HashSet<CaseInsensitiveString> = HashSet::new();
        let expanded = self.get_expanded_links(&self.url.inner().as_str());
        self.configuration.configure_allowlist();

        for link in expanded {
            let allowed = self.is_allowed(&link);

            if allowed.eq(&ProcessLinkStatus::BudgetExceeded) {
                break;
            }
            if allowed.eq(&ProcessLinkStatus::Blocked) {
                continue;
            }

            let mut page = Page::new(
                &link.inner().as_str(),
                &client,
                &page,
                &self.configuration.wait_for,
            )
            .await;
            let u = page.get_url();
            let u = if u.is_empty() { link } else { u.into() };

            let link_result = match self.on_link_find_callback {
                Some(cb) => cb(u, None),
                _ => (u, None),
            };

            self.links_visited.insert(link_result.0);

            if self.configuration.return_page_links {
                page.page_links = Some(Default::default());
                let next_links = HashSet::from(page.links(&base).await);

                channel_send_page(&self.channel, page.clone(), &self.channel_guard);

                links.extend(next_links);
            } else {
                channel_send_page(&self.channel, page.clone(), &self.channel_guard);
                let next_links = HashSet::from(page.links(&base).await);

                links.extend(next_links);
            }
        }

        links
    }

    /// Expand links for crawl.
    #[cfg(all(
        feature = "glob",
        not(feature = "chrome"),
        not(feature = "decentralized")
    ))]
    async fn crawl_establish(
        &mut self,
        client: &Client,
        base: &mut (CompactString, smallvec::SmallVec<[CompactString; 2]>),
        _: bool,
    ) -> HashSet<CaseInsensitiveString> {
        let mut links: HashSet<CaseInsensitiveString> = HashSet::new();
        let domain_name = self.url.inner();
        let expanded = self.get_expanded_links(&domain_name.as_str());

        self.configuration.configure_allowlist();

        for link in expanded {
            let allowed = self.is_allowed(&link);

            if allowed.eq(&ProcessLinkStatus::BudgetExceeded) {
                break;
            }

            if allowed.eq(&ProcessLinkStatus::Blocked) {
                continue;
            }

            let mut page = Page::new(&link.inner(), &client).await;

            if let Some(ref domain) = page.final_redirect_destination {
                let domain: Box<CaseInsensitiveString> = CaseInsensitiveString::new(&domain).into();
                let prior_domain = self.domain_parsed.take();
                self.domain_parsed = parse_absolute_url(&domain);
                self.url = domain;
                if let Some(s) = self.setup_selectors() {
                    base.0 = s.0;
                    base.1 = s.1;
                    if let Some(pd) = prior_domain {
                        if let Some(domain_name) = pd.host_str() {
                            base.2 = domain_name.into();
                        }
                    }
                }
            }

            let u = page.get_url().into();
            let link_result = match self.on_link_find_callback {
                Some(cb) => cb(u, None),
                _ => (u, None),
            };

            self.links_visited.insert(link_result.0);

            if !page.is_empty() {
                if self.configuration.return_page_links {
                    page.page_links = Some(Default::default());
                }
                let page_links = HashSet::from(page.links(&base).await);

                links.extend(page_links);
            } else {
                self.status = CrawlStatus::Empty;
            };

            channel_send_page(&self.channel, page, &self.channel_guard);
        }

        links
    }

    /// Set the crawl status depending on crawl state. The crawl that only changes if the state is Start or Active.
    fn set_crawl_status(&mut self) {
        if self.status == CrawlStatus::Start || self.status == CrawlStatus::Active {
            self.status = if self.domain_parsed.is_none() {
                CrawlStatus::Invalid
            } else {
                CrawlStatus::Idle
            };
        }
    }

    /// Setup the Semaphore for the crawl.
    fn setup_semaphore(&self) -> Arc<Semaphore> {
        if self.configuration.shared_queue {
            SEM_SHARED.clone()
        } else {
            Arc::new(Semaphore::const_new(
                self.configuration
                    .concurrency_limit
                    .unwrap_or(*DEFAULT_PERMITS),
            ))
        }
    }

    /// Start to crawl website with async concurrency.
    pub async fn crawl(&mut self) {
        self.start();
        let (client, handle) = self.setup().await;
        let (handle, join_handle) = match handle {
            Some(h) => (Some(h.0), Some(h.1)),
            _ => (None, None),
        };
        self.crawl_concurrent(&client, &handle).await;
        self.sitemap_crawl_chain(&client, &handle, false).await;
        self.set_crawl_status();
        if let Some(h) = join_handle {
            h.abort()
        }
        self.client.replace(client);
    }

    /// Start to crawl website with async concurrency using the sitemap. This does not page forward into the request. This does nothing without the `sitemap` flag enabled.
    pub async fn crawl_sitemap(&mut self) {
        self.start();
        let (client, handle) = self.setup().await;
        let (handle, join_handle) = match handle {
            Some(h) => (Some(h.0), Some(h.1)),
            _ => (None, None),
        };
        self.sitemap_crawl(&client, &handle, false).await;
        self.set_crawl_status();
        if let Some(h) = join_handle {
            h.abort()
        }
        self.client.replace(client);
    }

    #[cfg(all(feature = "decentralized", feature = "smart"))]
    /// Start to crawl website with async concurrency smart. Use HTTP first and JavaScript Rendering as needed. This has no effect without the `smart` flag enabled.
    pub async fn crawl_smart(&mut self) {
        self.crawl().await;
    }

    #[cfg(all(feature = "decentralized", not(feature = "smart")))]
    /// Start to crawl website with async concurrency smart. Use HTTP first and JavaScript Rendering as needed. This has no effect without the `smart` flag enabled.
    pub async fn crawl_smart(&mut self) {
        self.crawl().await;
    }

    #[cfg(all(not(feature = "decentralized"), feature = "smart"))]
    /// Start to crawl website with async concurrency smart. Use HTTP first and JavaScript Rendering as needed. This has no effect without the `smart` flag enabled.
    pub async fn crawl_smart(&mut self) {
        self.start();
        let (client, handle) = self.setup().await;
        let (handle, join_handle) = match handle {
            Some(h) => (Some(h.0), Some(h.1)),
            _ => (None, None),
        };
        self.crawl_concurrent_smart(&client, &handle).await;
        self.set_crawl_status();
        if let Some(h) = join_handle {
            h.abort()
        }
        self.client.replace(client);
    }

    #[cfg(all(not(feature = "decentralized"), not(feature = "smart")))]
    /// Start to crawl website with async concurrency smart. Use HTTP first and JavaScript Rendering as needed. This has no effect without the `smart` flag enabled.
    pub async fn crawl_smart(&mut self) {
        self.crawl().await
    }

    /// Start to crawl website with async concurrency using the base raw functionality. Useful when using the `chrome` feature and defaulting to the basic implementation.
    pub async fn crawl_raw(&mut self) {
        self.start();
        let (client, handle) = self.setup().await;
        let (handle, join_handle) = match handle {
            Some(h) => (Some(h.0), Some(h.1)),
            _ => (None, None),
        };
        self.crawl_concurrent_raw(&client, &handle).await;
        self.sitemap_crawl_chain(&client, &handle, false).await;
        self.set_crawl_status();
        if let Some(h) = join_handle {
            h.abort()
        }
        self.client.replace(client);
    }

    /// Start to scrape/download website with async concurrency.
    pub async fn scrape(&mut self) {
        let mut w = self.clone();
        let mut rx2 = w.subscribe(0).expect("receiver enabled");

        if self.pages.is_none() {
            self.pages = Some(Box::new(Vec::new()));
        }

        spawn_task("crawl", async move {
            w.crawl().await;
        });

        if let Some(p) = self.pages.as_mut() {
            while let Ok(res) = rx2.recv().await {
                self.links_visited.insert(res.get_url().into());
                p.push(res);
            }
        }
    }

    /// Start to crawl website with async concurrency using the base raw functionality. Useful when using the "chrome" feature and defaulting to the basic implementation.
    pub async fn scrape_raw(&mut self) {
        let mut w = self.clone();
        let mut rx2 = w.subscribe(0).expect("receiver enabled");

        if self.pages.is_none() {
            self.pages = Some(Box::new(Vec::new()));
        }

        spawn_task("crawl_raw", async move {
            w.crawl_raw().await;
        });

        if let Some(p) = self.pages.as_mut() {
            while let Ok(res) = rx2.recv().await {
                self.links_visited.insert(res.get_url().into());
                p.push(res);
            }
        }
    }

    /// Start to scrape website with async concurrency smart. Use HTTP first and JavaScript Rendering as needed. This has no effect without the `smart` flag enabled.
    pub async fn scrape_smart(&mut self) {
        let mut w = self.clone();
        let mut rx2 = w.subscribe(0).expect("receiver enabled");

        if self.pages.is_none() {
            self.pages = Some(Box::new(Vec::new()));
        }

        spawn_task("crawl_smart", async move {
            w.crawl_smart().await;
        });

        if let Some(p) = self.pages.as_mut() {
            while let Ok(res) = rx2.recv().await {
                self.links_visited.insert(res.get_url().into());
                p.push(res);
            }
        }
    }

    /// Start to scrape website sitemap with async concurrency. Use HTTP first and JavaScript Rendering as needed. This has no effect without the `sitemap` flag enabled.
    pub async fn scrape_sitemap(&mut self) {
        let mut w = self.clone();
        let mut rx2 = w.subscribe(0).expect("receiver enabled");

        if self.pages.is_none() {
            self.pages = Some(Box::new(Vec::new()));
        }

        spawn_task("crawl_sitemap", async move {
            w.crawl_sitemap().await;
        });

        if let Some(p) = self.pages.as_mut() {
            while let Ok(res) = rx2.recv().await {
                self.links_visited.insert(res.get_url().into());
                p.push(res);
            }
        }
    }

    /// Start to crawl website concurrently - used mainly for chrome instances to connect to default raw HTTP.
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    async fn crawl_concurrent_raw(&mut self, client: &Client, handle: &Option<Arc<AtomicI8>>) {
        self.start();
        match self.setup_selectors() {
            Some(mut selector) => {
                if match self.configuration.inner_budget {
                    Some(ref b) => match b.get(&*WILD_CARD_PATH) {
                        Some(b) => b.eq(&1),
                        _ => false,
                    },
                    _ => false,
                } {
                    self.status = CrawlStatus::Active;
                    self._crawl_establish(client, &mut selector, false).await;
                } else {
                    let on_link_find_callback = self.on_link_find_callback;
                    let full_resources = self.configuration.full_resources;
                    let return_page_links = self.configuration.return_page_links;
                    let only_html = self.configuration.only_html && !full_resources;

                    let (mut interval, throttle) = self.setup_crawl();

                    let mut links: HashSet<CaseInsensitiveString> =
                        self.drain_extra_links().collect();

                    links.extend(self._crawl_establish(client, &mut selector, false).await);

                    self.configuration.configure_allowlist();

                    let mut q = self.channel_queue.as_ref().map(|q| q.0.subscribe());

                    let semaphore = self.setup_semaphore();

                    let shared = Arc::new((
                        client.to_owned(),
                        selector,
                        self.channel.clone(),
                        self.configuration.external_domains_caseless.clone(),
                        self.channel_guard.clone(),
                        self.configuration.retry,
                        self.configuration.full_resources,
                        PageLinkBuildSettings::new_full(
                            false,
                            self.configuration.full_resources,
                            self.configuration.subdomains,
                            self.configuration.tld,
                        ),
                    ));

                    let mut set: JoinSet<HashSet<CaseInsensitiveString>> = JoinSet::new();

                    // track budgeting one time.
                    let mut exceeded_budget = false;
                    let concurrency = throttle.is_zero();

                    if !concurrency && !links.is_empty() {
                        tokio::time::sleep(*throttle).await;
                    }

                    'outer: loop {
                        let mut stream = tokio_stream::iter::<HashSet<CaseInsensitiveString>>(
                            links.drain().collect(),
                        );

                        loop {
                            if !concurrency {
                                tokio::time::sleep(*throttle).await;
                            }

                            let semaphore =
                                get_semaphore(&semaphore, !self.configuration.shared_queue).await;

                            tokio::select! {
                                biased;
                                Some(link) = stream.next(), if semaphore.available_permits() > 0 => {
                                    if !self.handle_process(handle, &mut interval, async {
                                        emit_log_shutdown(link.inner());
                                        let permits = set.len();
                                        set.shutdown().await;
                                        semaphore.add_permits(permits);
                                    }).await {
                                        break 'outer;
                                    }
                                    let allowed = self.is_allowed(&link);

                                    if allowed.eq(&ProcessLinkStatus::BudgetExceeded) {
                                        exceeded_budget = true;
                                        break;
                                    }

                                    if allowed.eq(&ProcessLinkStatus::Blocked) {
                                        continue;
                                    }

                                    emit_log(link.inner());

                                    self.links_visited.insert(link.clone());

                                    if let Ok(permit) = semaphore.clone().acquire_owned().await {
                                        let shared = shared.clone();

                                        spawn_set("page_fetch", &mut set, async move {
                                            let link_result = match on_link_find_callback {
                                                Some(cb) => cb(link, None),
                                                _ => (link, None),
                                            };

                                            let mut links: HashSet<CaseInsensitiveString> = HashSet::new();
                                            let mut links_pages = if return_page_links {
                                                Some(links.clone())
                                            } else {
                                                None
                                            };
                                            let mut relative_selectors = shared.1.clone();
                                            let mut r_settings = shared.7;
                                            r_settings.ssg_build = true;
                                            let target_url = link_result.0.as_ref();
                                            let external_domains_caseless = &shared.3;
                                            let client = &shared.0;

                                            let mut domain_parsed = None;

                                            let mut page = Page::new_page_streaming(
                                                target_url,
                                                client, only_html,
                                                &mut relative_selectors,
                                                external_domains_caseless,
                                                &r_settings,
                                                &mut links,
                                                None,
                                                &None,
                                                &mut domain_parsed,
                                                &mut links_pages).await;

                                            let mut retry_count = shared.5;

                                            while page.should_retry && retry_count > 0 {
                                                retry_count -= 1;

                                                if let Some(timeout) = page.get_timeout() {
                                                    tokio::time::sleep(timeout).await;
                                                }

                                                if page.status_code == StatusCode::GATEWAY_TIMEOUT {
                                                    if let Err(elasped) = tokio::time::timeout(BACKOFF_MAX_DURATION, async {
                                                        let mut domain_parsed = None;
                                                        let next_page = Page::new_page_streaming(
                                                            target_url,
                                                            client, only_html,
                                                            &mut relative_selectors.clone(),
                                                            external_domains_caseless,
                                                            &r_settings,
                                                            &mut links,
                                                            None,
                                                            &None,
                                                            &mut domain_parsed,
                                                            &mut links_pages).await;

                                                        page.clone_from(&next_page);

                                                    }).await
                                                {
                                                    log::warn!("Handler timeout exceeded {elasped}");
                                                }

                                                } else {
                                                    page.clone_from(&Page::new_page_streaming(
                                                        target_url,
                                                        client,
                                                        only_html,
                                                        &mut relative_selectors.clone(),
                                                        external_domains_caseless,
                                                        &r_settings,
                                                        &mut links,
                                                        None,
                                                        &None,
                                                        &mut domain_parsed,
                                                        &mut links_pages).await);
                                                }
                                            }

                                            if return_page_links {
                                                page.page_links = links_pages.filter(|pages| !pages.is_empty()).map(Box::new);
                                            }

                                            channel_send_page(&shared.2, page, &shared.4);
                                            drop(permit);

                                            links
                                        });
                                    }

                                    if let Some(q) = &mut q {
                                        while let Ok(link) = q.try_recv() {
                                            let s = link.into();
                                            let allowed = self.is_allowed(&s);

                                            if allowed.eq(&ProcessLinkStatus::BudgetExceeded) {
                                                exceeded_budget = true;
                                                break;
                                            }
                                            if allowed.eq(&ProcessLinkStatus::Blocked) {
                                                continue;
                                            }
                                            self.links_visited.extend_with_new_links(&mut links, s);
                                        }
                                    }
                                },
                                Some(result) = set.join_next(), if !set.is_empty() => {
                                    if let Ok(res) = result {
                                        // todo: add final url catching domains to make sure we do not add extra pages.
                                        self.links_visited.extend_links(&mut links, res);
                                    }
                                }
                                else => break,
                            }

                            if links.is_empty() && set.is_empty() || exceeded_budget {
                                // await for all tasks to complete.
                                if exceeded_budget {
                                    set.join_all().await;
                                }
                                break 'outer;
                            }
                        }

                        if links.is_empty() && set.is_empty() {
                            break;
                        }
                    }

                    self.subscription_guard();
                }
            }
            _ => log::info!("{} - {}", self.url, INVALID_URL),
        }
    }

    /// Start to crawl website concurrently.
    #[cfg(all(not(feature = "decentralized"), feature = "chrome"))]
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    async fn crawl_concurrent(&mut self, client: &Client, handle: &Option<Arc<AtomicI8>>) {
        use crate::features::chrome::attempt_navigation;
        self.start();
        match self.setup_selectors() {
            Some(mut selectors) => match self.setup_browser().await {
                Some((browser, browser_handle, mut context_id)) => {
                    match attempt_navigation(
                        "about:blank",
                        &browser,
                        &self.configuration.request_timeout,
                        &context_id,
                        &self.configuration.viewport,
                    )
                    .await
                    {
                        Ok(new_page) => {
                            if match self.configuration.inner_budget {
                                Some(ref b) => match b.get(&*WILD_CARD_PATH) {
                                    Some(b) => b.eq(&1),
                                    _ => false,
                                },
                                _ => false,
                            } {
                                self.status = CrawlStatus::Active;
                                self.crawl_establish(&client, &mut selectors, false, &new_page)
                                    .await;
                                self.subscription_guard();
                                crate::features::chrome::close_browser(
                                    browser_handle,
                                    &browser,
                                    &mut context_id,
                                )
                                .await;
                            } else {
                                let semaphore = self.setup_semaphore();

                                let mut q = self.channel_queue.as_ref().map(|q| q.0.subscribe());

                                let mut links: HashSet<CaseInsensitiveString> =
                                    self.drain_extra_links().collect();

                                let (mut interval, throttle) = self.setup_crawl();

                                links.extend(
                                    self.crawl_establish(&client, &mut selectors, false, &new_page)
                                        .await,
                                );

                                self.configuration.configure_allowlist();

                                let mut set: JoinSet<HashSet<CaseInsensitiveString>> =
                                    JoinSet::new();
                                let shared = Arc::new((
                                    client.to_owned(),
                                    selectors,
                                    self.channel.clone(),
                                    self.configuration.external_domains_caseless.clone(),
                                    self.channel_guard.clone(),
                                    browser,
                                    self.configuration.clone(),
                                    self.url.inner().to_string(),
                                    context_id.clone(),
                                    self.domain_parsed.clone(),
                                ));

                                let add_external = shared.3.len() > 0;
                                let on_link_find_callback = self.on_link_find_callback;
                                let full_resources = self.configuration.full_resources;
                                let return_page_links = self.configuration.return_page_links;
                                let mut exceeded_budget = false;
                                let concurrency = throttle.is_zero();

                                if !concurrency && !links.is_empty() {
                                    tokio::time::sleep(*throttle).await;
                                }

                                'outer: loop {
                                    let mut stream =
                                        tokio_stream::iter::<HashSet<CaseInsensitiveString>>(
                                            links.drain().collect(),
                                        );

                                    loop {
                                        if !concurrency {
                                            tokio::time::sleep(*throttle).await;
                                        }

                                        let semaphore = get_semaphore(
                                            &semaphore,
                                            !self.configuration.shared_queue,
                                        )
                                        .await;

                                        tokio::select! {
                                            biased;
                                            Some(link) = stream.next(), if semaphore.available_permits() > 0 => {
                                                if !self
                                                    .handle_process(
                                                        handle,
                                                        &mut interval,
                                                        async {
                                                            emit_log_shutdown(&link.inner());
                                                            let permits = set.len();
                                                            set.shutdown().await;
                                                            semaphore.add_permits(permits);
                                                        },
                                                    )
                                                    .await
                                                {
                                                    break 'outer;
                                                }

                                                let allowed = self.is_allowed(&link);

                                                if allowed
                                                    .eq(&ProcessLinkStatus::BudgetExceeded)
                                                {
                                                    exceeded_budget = true;
                                                    break;
                                                }
                                                if allowed.eq(&ProcessLinkStatus::Blocked) {
                                                    continue;
                                                }

                                                emit_log(&link.inner());

                                                self.links_visited.insert(link.clone());

                                                if let Ok(permit) = semaphore.clone().acquire_owned().await {
                                                    let shared = shared.clone();

                                                    spawn_set("page_fetch", &mut set, async move {
                                                        let results = match attempt_navigation("about:blank", &shared.5, &shared.6.request_timeout, &shared.8, &shared.6.viewport).await {
                                                            Ok(new_page) => {
                                                                crate::features::chrome::setup_chrome_events(&new_page, &shared.6).await;

                                                                let intercept_handle = crate::features::chrome::setup_chrome_interception_base(
                                                                    &new_page,
                                                                    shared.6.chrome_intercept.enabled,
                                                                    &shared.6.auth_challenge_response,
                                                                    shared.6.chrome_intercept.block_visuals,
                                                                    &shared.7,
                                                                )
                                                                .await;

                                                                let link_result =
                                                                    match on_link_find_callback {
                                                                        Some(cb) => cb(link, None),
                                                                        _ => (link, None),
                                                                    };

                                                                let target_url = link_result.0.as_ref();

                                                                let mut page = Page::new(
                                                                    &target_url,
                                                                    &shared.0,
                                                                    &new_page,
                                                                    &shared.6.wait_for,
                                                                    &shared.6.screenshot,
                                                                    false,
                                                                    &shared.6.openai_config,
                                                                    &shared.6.execution_scripts,
                                                                    &shared.6.automation_scripts,
                                                                    &shared.6.viewport,
                                                                    &shared.6.request_timeout
                                                                )
                                                                .await;

                                                                let mut retry_count = shared.6.retry;

                                                                while page.should_retry && retry_count > 0 {
                                                                    retry_count -= 1;
                                                                    if let Some(timeout) = page.get_timeout() {
                                                                        tokio::time::sleep(timeout).await;
                                                                    }
                                                                    if page.status_code == StatusCode::GATEWAY_TIMEOUT {
                                                                        if let Err(elasped) = tokio::time::timeout(BACKOFF_MAX_DURATION, async {
                                                                            let p = Page::new(
                                                                                &target_url,
                                                                                &shared.0,
                                                                                &new_page,
                                                                                &shared.6.wait_for,
                                                                                &shared.6.screenshot,
                                                                                false,
                                                                                &shared.6.openai_config,
                                                                                &shared.6.execution_scripts,
                                                                                &shared.6.automation_scripts,
                                                                                &shared.6.viewport,
                                                                                &shared.6.request_timeout
                                                                            ).await;
                                                                            page.clone_from(&p);

                                                                        }).await {
                                                                            log::info!("{target_url} backoff gateway timeout exceeded {elasped}");
                                                                        }
                                                                    } else {
                                                                        page.clone_from(
                                                                            &Page::new(
                                                                                &target_url,
                                                                                &shared.0,
                                                                                &new_page,
                                                                                &shared.6.wait_for,
                                                                                &shared.6.screenshot,
                                                                                false,
                                                                                &shared.6.openai_config,
                                                                                &shared.6.execution_scripts,
                                                                                &shared.6.automation_scripts,
                                                                                &shared.6.viewport,
                                                                                &shared.6.request_timeout,
                                                                            )
                                                                            .await,
                                                                        );
                                                                    }
                                                                }

                                                                if let Some(h) = intercept_handle {
                                                                    let abort_handle = h.abort_handle();
                                                                    if let Err(elasped) = tokio::time::timeout(tokio::time::Duration::from_secs(10), h).await {
                                                                        log::warn!("Handler timeout exceeded {elasped}");
                                                                        abort_handle.abort();
                                                                    }
                                                                }

                                                                if add_external {
                                                                    page.set_external(shared.3.clone());
                                                                }

                                                                let prev_domain = page.base;

                                                                page.base = shared.9.as_deref().cloned();

                                                                if return_page_links {
                                                                    page.page_links = Some(Default::default());
                                                                }

                                                                let links = if full_resources {
                                                                    page.links_full(&shared.1).await
                                                                } else {
                                                                    page.links(&shared.1).await
                                                                };

                                                                page.base = prev_domain;

                                                                channel_send_page(
                                                                    &shared.2, page, &shared.4,
                                                                );

                                                                links
                                                            }
                                                            _ => Default::default(),
                                                        };

                                                        drop(permit);

                                                        results
                                                    });
                                                }

                                                if let Some(q) = q.as_mut() {
                                                    while let Ok(link) = q.try_recv() {
                                                        let s = link.into();
                                                        let allowed = self.is_allowed(&s);

                                                        if allowed.eq(&ProcessLinkStatus::BudgetExceeded) {
                                                        exceeded_budget = true;
                                                        break;
                                                    }
                                                        if allowed
                                                            .eq(&ProcessLinkStatus::Blocked)
                                                        {
                                                            continue;
                                                        }

                                                        self.links_visited
                                                            .extend_with_new_links(
                                                                &mut links, s,
                                                            );
                                                    }
                                                }

                                            }
                                            Some(result) = set.join_next(), if !set.is_empty() => {
                                                match result {
                                                    Ok(res) => self.links_visited.extend_links(&mut links, res),
                                                    Err(_) => {
                                                        break
                                                    }
                                                }
                                            }
                                            else => break,
                                        };

                                        if links.is_empty() && set.is_empty() || exceeded_budget {
                                            if exceeded_budget {
                                                set.join_all().await;
                                            }
                                            break 'outer;
                                        }
                                    }

                                    if links.is_empty() && set.is_empty() {
                                        break;
                                    }
                                }

                                self.subscription_guard();

                                crate::features::chrome::close_browser(
                                    browser_handle,
                                    &shared.5,
                                    &mut context_id,
                                )
                                .await;
                            }
                        }
                        Err(err) => {
                            crate::features::chrome::close_browser(
                                browser_handle,
                                &browser,
                                &mut context_id,
                            )
                            .await;

                            log::error!("{}", err)
                        }
                    }
                }
                _ => log::info!("Chrome failed to start."),
            },
            _ => log::info!("{} - {}", self.url, INVALID_URL),
        }
    }

    /// Start to crawl website concurrently.
    #[cfg(all(not(feature = "decentralized"), not(feature = "chrome")))]
    async fn crawl_concurrent(&mut self, client: &Client, handle: &Option<Arc<AtomicI8>>) {
        self.crawl_concurrent_raw(client, handle).await
    }

    /// Start to crawl website concurrently.
    #[cfg(feature = "decentralized")]
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    async fn crawl_concurrent(&mut self, client: &Client, handle: &Option<Arc<AtomicI8>>) {
        match url::Url::parse(&self.url.inner()) {
            Ok(_) => {
                let mut q = self.channel_queue.as_ref().map(|q| q.0.subscribe());

                self.configuration.configure_allowlist();
                let domain = self.url.inner().as_str();
                let mut interval = Box::pin(tokio::time::interval(Duration::from_millis(10)));
                let throttle = Box::pin(self.get_delay());
                let on_link_find_callback = self.on_link_find_callback;
                // http worker verify
                let http_worker = std::env::var("SPIDER_WORKER")
                    .unwrap_or_else(|_| "http:".to_string())
                    .starts_with("http:");

                let mut links: HashSet<CaseInsensitiveString> = self
                    .crawl_establish(
                        &client,
                        &mut (domain.into(), Default::default()),
                        http_worker,
                    )
                    .await;

                let mut set: JoinSet<HashSet<CaseInsensitiveString>> = JoinSet::new();
                let mut exceeded_budget = false;

                'outer: loop {
                    let stream = tokio_stream::iter::<HashSet<CaseInsensitiveString>>(
                        links.drain().collect(),
                    )
                    .throttle(*throttle);
                    tokio::pin!(stream);

                    loop {
                        match stream.next().await {
                            Some(link) => {
                                if !self
                                    .handle_process(handle, &mut interval, async {
                                        emit_log_shutdown(&link.inner());
                                        set.shutdown().await;
                                    })
                                    .await
                                {
                                    break 'outer;
                                }

                                let allowed = self.is_allowed(&link);

                                if allowed.eq(&ProcessLinkStatus::BudgetExceeded) {
                                    exceeded_budget = true;
                                    break;
                                }
                                if allowed.eq(&ProcessLinkStatus::Blocked) {
                                    continue;
                                }

                                emit_log(&link.inner());

                                self.links_visited.insert(link.clone());

                                if let Ok(permit) = SEM.acquire().await {
                                    let client = client.clone();

                                    spawn_set("page_fetch", &mut set, async move {
                                        let link_results = match on_link_find_callback {
                                            Some(cb) => cb(link, None),
                                            _ => (link, None),
                                        };
                                        let link_results = link_results.0.as_ref();
                                        let page = Page::new_links_only(
                                            &if http_worker && link_results.starts_with("https") {
                                                link_results
                                                    .replacen("https", "http", 1)
                                                    .to_string()
                                            } else {
                                                link_results.to_string()
                                            },
                                            &client,
                                        )
                                        .await;

                                        drop(permit);

                                        page.links
                                    });

                                    if let Some(q) = q.as_mut() {
                                        while let Ok(link) = q.try_recv() {
                                            let s = link.into();
                                            let allowed = self.is_allowed(&s);

                                            if allowed.eq(&ProcessLinkStatus::BudgetExceeded) {
                                                exceeded_budget = true;
                                                break;
                                            }
                                            if allowed.eq(&ProcessLinkStatus::Blocked) {
                                                continue;
                                            }

                                            self.links_visited.extend_with_new_links(&mut links, s);
                                        }
                                    }
                                }
                            }
                            _ => break,
                        }
                        if exceeded_budget {
                            break;
                        }
                    }

                    while let Some(res) = set.join_next().await {
                        if let Ok(msg) = res {
                            self.links_visited.extend_links(&mut links, msg);
                        }
                    }

                    if links.is_empty() || exceeded_budget {
                        break;
                    }
                }
            }
            _ => (),
        }
    }

    /// Start to crawl website concurrently using HTTP by default and chrome Javascript Rendering as needed. The glob feature does not work with this at the moment.
    #[cfg(all(not(feature = "decentralized"), feature = "smart"))]
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    async fn crawl_concurrent_smart(&mut self, client: &Client, handle: &Option<Arc<AtomicI8>>) {
        self.start();
        match self.setup_selectors() {
            Some(mut selectors) => match self.setup_browser().await {
                Some((browser, browser_handle, mut context_id)) => {
                    if match self.configuration.inner_budget {
                        Some(ref b) => match b.get(&*WILD_CARD_PATH) {
                            Some(b) => b.eq(&1),
                            _ => false,
                        },
                        _ => false,
                    } {
                        self.status = CrawlStatus::Active;
                        self.crawl_establish_smart(
                            &client,
                            &mut selectors,
                            false,
                            &browser,
                            &context_id,
                        )
                        .await;
                        self.subscription_guard();
                        crate::features::chrome::close_browser(
                            browser_handle,
                            &browser,
                            &mut context_id,
                        )
                        .await;
                    } else {
                        let mut q = self.channel_queue.as_ref().map(|q| q.0.subscribe());

                        let mut links: HashSet<CaseInsensitiveString> =
                            self.drain_extra_links().collect();

                        let (mut interval, throttle) = self.setup_crawl();
                        let on_link_find_callback = self.on_link_find_callback;
                        let return_page_links = self.configuration.return_page_links;

                        links.extend(
                            self.crawl_establish_smart(
                                &client,
                                &mut selectors,
                                false,
                                &browser,
                                &context_id,
                            )
                            .await,
                        );
                        self.configuration.configure_allowlist();

                        let mut set: JoinSet<HashSet<CaseInsensitiveString>> = JoinSet::new();
                        let semaphore = self.setup_semaphore();

                        let shared = Arc::new((
                            client.to_owned(),
                            selectors,
                            self.channel.clone(),
                            self.channel_guard.clone(),
                            browser,
                            self.configuration.clone(),
                            context_id.clone(),
                            self.domain_parsed.clone(),
                        ));

                        let add_external = self.configuration.external_domains_caseless.len() > 0;
                        let mut exceeded_budget = false;
                        let concurrency = throttle.is_zero();

                        if !concurrency && !links.is_empty() {
                            tokio::time::sleep(*throttle).await;
                        }

                        'outer: loop {
                            let mut stream = tokio_stream::iter::<HashSet<CaseInsensitiveString>>(
                                links.drain().collect(),
                            );

                            loop {
                                if !concurrency {
                                    tokio::time::sleep(*throttle).await;
                                }

                                let semaphore =
                                    get_semaphore(&semaphore, !self.configuration.shared_queue)
                                        .await;

                                tokio::select! {
                                    biased;
                                    Some(link) = stream.next(), if semaphore.available_permits() > 0 => {
                                        if !self
                                            .handle_process(
                                                handle,
                                                &mut interval,
                                                async {
                                                    emit_log_shutdown(&link.inner());
                                                    let permits = set.len();
                                                    set.shutdown().await;
                                                    semaphore.add_permits(permits);

                                                },
                                            )
                                            .await
                                        {
                                            break 'outer;
                                        }

                                        let allowed = self.is_allowed(&link);

                                        if allowed.eq(&ProcessLinkStatus::BudgetExceeded) {
                                            exceeded_budget = true;
                                            break;
                                        }
                                        if allowed.eq(&ProcessLinkStatus::Blocked) {
                                            continue;
                                        }

                                        emit_log(&link.inner());
                                        self.links_visited.insert(link.clone());

                                        if let Ok(permit) = semaphore.clone().acquire_owned().await {
                                            let shared = shared.clone();

                                            spawn_set("page_fetch", &mut set, async move {
                                                let link_result = match on_link_find_callback {
                                                    Some(cb) => cb(link, None),
                                                    _ => (link, None),
                                                };

                                                let url = link_result.0.as_ref();
                                                let mut page =
                                                    Page::new_page(&url, &shared.0).await;

                                                let mut retry_count = shared.5.retry;

                                                while page.should_retry && retry_count > 0 {
                                                    retry_count -= 1;

                                                    if let Some(timeout) = page.get_timeout() {
                                                        tokio::time::sleep(timeout).await;
                                                    }

                                                    if page.status_code == StatusCode::GATEWAY_TIMEOUT {

                                                        if let Err(elasped) = tokio::time::timeout(BACKOFF_MAX_DURATION, async {
                                                            if retry_count.is_power_of_two() {
                                                                Website::render_chrome_page(
                                                                    &shared.5, &shared.0, &shared.4,
                                                                    &shared.6, &mut page, url,
                                                                )
                                                                .await;
                                                            } else {
                                                                let next_page =  Page::new_page(url, &shared.0).await;

                                                                page.clone_from(&next_page)
                                                            };

                                                        }).await
                                                    {
                                                        log::info!("backoff gateway timeout exceeded {elasped}");
                                                    }

                                                    } else {

                                                        if retry_count.is_power_of_two() {
                                                            Website::render_chrome_page(
                                                                &shared.5, &shared.0, &shared.4,
                                                                &shared.6, &mut page, url,
                                                            )
                                                            .await;
                                                        } else {
                                                            page.clone_from(
                                                                &Page::new_page(url, &shared.0)
                                                                    .await,
                                                            );
                                                        }
                                                    }
                                                }

                                                if add_external {
                                                    page.set_external(
                                                        shared
                                                            .5
                                                            .external_domains_caseless
                                                            .clone(),
                                                    );
                                                }

                                                let prev_domain = page.base;

                                                page.base = shared.7.as_deref().cloned();

                                                if return_page_links {
                                                    page.page_links = Some(Default::default());
                                                }

                                                let links = page
                                                    .smart_links(
                                                        &shared.1, &shared.4, &shared.5,
                                                        &shared.6,
                                                    )
                                                    .await;

                                                    page.base = prev_domain;

                                                channel_send_page(&shared.2, page, &shared.3);
                                                drop(permit);

                                                links
                                            });
                                        }

                                        if let Some(q) = q.as_mut() {
                                            while let Ok(link) = q.try_recv() {
                                                let s = link.into();
                                                let allowed = self.is_allowed(&s);

                                                if allowed
                                                    .eq(&ProcessLinkStatus::BudgetExceeded)
                                                {
                                                    exceeded_budget = true;
                                                    break;
                                                }
                                                if allowed.eq(&ProcessLinkStatus::Blocked) {
                                                    continue;
                                                }

                                                self.links_visited
                                                    .extend_with_new_links(&mut links, s);
                                            }
                                        }
                                    }
                                    Some(result) = set.join_next(), if !set.is_empty() => {
                                        match result {
                                            Ok(res) => self.links_visited.extend_links(&mut links, res),
                                            Err(_) => {
                                                break
                                            }
                                        }
                                    }
                                    else => break,
                                }

                                if links.is_empty() && set.is_empty() || exceeded_budget {
                                    if exceeded_budget {
                                        set.join_all().await;
                                    }
                                    break 'outer;
                                }
                            }

                            if links.is_empty() && set.is_empty() {
                                break;
                            }
                        }

                        self.subscription_guard();
                        crate::features::chrome::close_browser(
                            browser_handle,
                            &shared.4,
                            &mut context_id,
                        )
                        .await;
                    }
                }
                _ => log::info!("Chrome failed to start."),
            },
            _ => log::info!("{} - {}", self.url, INVALID_URL),
        }
    }

    /// Sitemap crawl entire lists. Note: this method does not re-crawl the links of the pages found on the sitemap. This does nothing without the `sitemap` flag.
    #[cfg(not(feature = "sitemap"))]
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub async fn sitemap_crawl(
        &mut self,
        _client: &Client,
        _handle: &Option<Arc<AtomicI8>>,
        _scrape: bool,
    ) {
    }

    /// Sitemap crawl entire lists chain. Note: this method does not re-crawl the links of the pages found on the sitemap. This does nothing without the [sitemap] flag.
    #[cfg(not(feature = "sitemap"))]
    async fn sitemap_crawl_chain(
        &mut self,
        _client: &Client,
        _handle: &Option<Arc<AtomicI8>>,
        _scrape: bool,
    ) {
    }

    /// Sitemap crawl entire lists. Note: this method does not re-crawl the links of the pages found on the sitemap. This does nothing without the `sitemap` flag.
    #[cfg(feature = "sitemap")]
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub async fn sitemap_crawl_raw(
        &mut self,
        client: &Client,
        handle: &Option<Arc<AtomicI8>>,
        scrape: bool,
    ) {
        use sitemap::reader::{SiteMapEntity, SiteMapReader};
        use sitemap::structs::Location;

        match self.setup_selectors() {
            Some(selectors) => {
                let mut q = self.channel_queue.as_ref().map(|q| q.0.subscribe());

                let domain = self.url.inner().as_str();
                self.domain_parsed = parse_absolute_url(&domain);

                let mut interval = tokio::time::interval(Duration::from_millis(15));
                let (sitemap_path, needs_trailing) = match &self.configuration.sitemap_url {
                    Some(sitemap_path) => {
                        let sitemap_path = sitemap_path.as_str();
                        if domain.ends_with('/') && sitemap_path.starts_with('/') {
                            (&sitemap_path[1..], false)
                        } else if !domain.ends_with('/')
                            && !sitemap_path.is_empty()
                            && !sitemap_path.starts_with('/')
                        {
                            (sitemap_path, true)
                        } else {
                            (sitemap_path, false)
                        }
                    }
                    _ => ("sitemap.xml", !domain.ends_with("/")),
                };

                self.configuration.sitemap_url = Some(Box::new(
                    string_concat!(domain, if needs_trailing { "/" } else { "" }, sitemap_path)
                        .into(),
                ));

                self.configuration.configure_allowlist();

                let shared = Arc::new((self.channel.clone(), self.channel_guard.clone()));
                let mut sitemaps = match self.configuration.sitemap_url {
                    Some(ref sitemap) => Vec::from([sitemap.to_owned()]),
                    _ => Default::default(),
                };

                let retry = self.configuration.retry;
                let mut exceeded_budget = false;

                'outer: loop {
                    let stream =
                        tokio_stream::iter::<Vec<Box<CompactString>>>(sitemaps.drain(..).collect());
                    tokio::pin!(stream);

                    while let Some(sitemap_url) = stream.next().await {
                        if !self.handle_process(handle, &mut interval, async {}).await {
                            break 'outer;
                        }
                        let (tx, mut rx) = tokio::sync::mpsc::channel::<Page>(100);

                        let shared = shared.clone();

                        let handles = spawn_task("page_fetch", async move {
                            let mut pages = Vec::new();

                            while let Some(page) = rx.recv().await {
                                if shared.0.is_some() {
                                    if scrape {
                                        pages.push(page.clone());
                                    };

                                    channel_send_page(&shared.0.clone(), page, &shared.1);
                                } else {
                                    pages.push(page);
                                }
                            }

                            pages
                        });

                        match client.get(sitemap_url.as_str()).send().await {
                            Ok(response) => {
                                match response.text().await {
                                    Ok(text) => {
                                        // <html><head><title>Invalid request</title></head><body><p>Blocked by WAF</p><
                                        let mut stream =
                                            tokio_stream::iter(SiteMapReader::new(text.as_bytes()));

                                        while let Some(entity) = stream.next().await {
                                            if !self
                                                .handle_process(handle, &mut interval, async {})
                                                .await
                                            {
                                                break;
                                            }

                                            match entity {
                                                SiteMapEntity::Url(url_entry) => {
                                                    match url_entry.loc {
                                                        Location::Url(url) => {
                                                            let link: CaseInsensitiveString =
                                                                url.as_str().into();

                                                            let allowed = self.is_allowed(&link);

                                                            if allowed.eq(
                                                                &ProcessLinkStatus::BudgetExceeded,
                                                            ) {
                                                                exceeded_budget = true;
                                                                break;
                                                            }
                                                            if allowed
                                                                .eq(&ProcessLinkStatus::Blocked)
                                                            {
                                                                continue;
                                                            }

                                                            self.links_visited.insert(link.clone());

                                                            let client = client.clone();
                                                            let tx = tx.clone();

                                                            spawn_task("page_fetch", async move {
                                                                let mut page = Page::new_page(
                                                                    &link.inner(),
                                                                    &client,
                                                                )
                                                                .await;

                                                                let mut retry_count = retry;

                                                                while page.should_retry
                                                                    && retry_count > 0
                                                                {
                                                                    if let Some(timeout) =
                                                                        page.get_timeout()
                                                                    {
                                                                        tokio::time::sleep(timeout)
                                                                            .await;
                                                                    }
                                                                    page.clone_from(
                                                                        &Page::new_page(
                                                                            link.inner(),
                                                                            &client,
                                                                        )
                                                                        .await,
                                                                    );
                                                                    retry_count -= 1;
                                                                }

                                                                if let Ok(permit) =
                                                                    tx.reserve().await
                                                                {
                                                                    permit.send(page);
                                                                }
                                                            });
                                                        }
                                                        Location::None | Location::ParseErr(_) => {
                                                            ()
                                                        }
                                                    }
                                                }
                                                SiteMapEntity::SiteMap(sitemap_entry) => {
                                                    match sitemap_entry.loc {
                                                        Location::Url(url) => {
                                                            sitemaps.push(Box::new(
                                                                CompactString::new(&url.as_str()),
                                                            ));
                                                        }
                                                        Location::None | Location::ParseErr(_) => {
                                                            ()
                                                        }
                                                    }
                                                }
                                                SiteMapEntity::Err(err) => {
                                                    log::info!(
                                                        "incorrect sitemap error: {:?}",
                                                        err.msg()
                                                    )
                                                }
                                            };
                                        }
                                    }
                                    Err(err) => {
                                        log::info!("http parse error: {:?}", err.to_string())
                                    }
                                };
                            }
                            Err(err) => log::info!("http network error: {}", err.to_string()),
                        };

                        drop(tx);

                        if let Ok(mut handle) = handles.await {
                            for mut page in handle.iter_mut() {
                                let prev_domain = page.base.take();
                                page.base = self.domain_parsed.as_deref().cloned();
                                if self.configuration.return_page_links {
                                    page.page_links = Some(Default::default());
                                }
                                let links = page.links(&selectors).await;
                                page.base = prev_domain;
                                self.extra_links.extend(links)
                            }
                            if scrape {
                                if let Some(p) = self.pages.as_mut() {
                                    p.extend(handle);
                                }
                            }

                            match q.as_mut() {
                                Some(q) => {
                                    while let Ok(link) = q.try_recv() {
                                        let s = link.into();
                                        let allowed = self.is_allowed(&s);

                                        if allowed.eq(&ProcessLinkStatus::BudgetExceeded) {
                                            exceeded_budget = true;
                                            break;
                                        }
                                        if allowed.eq(&ProcessLinkStatus::Blocked) {
                                            continue;
                                        }

                                        self.links_visited
                                            .extend_with_new_links(&mut self.extra_links, s);
                                    }
                                }
                                _ => (),
                            }
                        }

                        if exceeded_budget {
                            break;
                        }
                    }

                    if sitemaps.len() == 0 || exceeded_budget {
                        break;
                    }
                }
            }
            _ => (),
        }
    }

    /// Sitemap crawl entire lists using chrome. Note: this method does not re-crawl the links of the pages found on the sitemap. This does nothing without the `sitemap` flag.
    #[cfg(all(
        feature = "sitemap",
        feature = "chrome",
        not(feature = "decentralized")
    ))]
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub async fn sitemap_crawl_chrome(
        &mut self,
        client: &Client,
        handle: &Option<Arc<AtomicI8>>,
        scrape: bool,
    ) {
        use crate::features::chrome::attempt_navigation;
        use sitemap::reader::{SiteMapEntity, SiteMapReader};
        use sitemap::structs::Location;

        match self.setup_selectors() {
            Some(selectors) => {
                match self.setup_browser().await {
                    Some((browser, browser_handle, mut context_id)) => {
                        let domain = self.url.inner().as_str();
                        self.domain_parsed = parse_absolute_url(&domain);
                        let mut interval = tokio::time::interval(Duration::from_millis(15));
                        let (sitemap_path, needs_trailing) = match &self.configuration.sitemap_url {
                            Some(sitemap_path) => {
                                let sitemap_path = sitemap_path.as_str();
                                if domain.ends_with('/') && sitemap_path.starts_with('/') {
                                    (&sitemap_path[1..], false)
                                } else if !domain.ends_with('/')
                                    && !sitemap_path.is_empty()
                                    && !sitemap_path.starts_with('/')
                                {
                                    (sitemap_path, true)
                                } else {
                                    (sitemap_path, false)
                                }
                            }
                            _ => ("sitemap.xml", !domain.ends_with("/")),
                        };

                        self.configuration.sitemap_url = Some(Box::new(
                            string_concat!(
                                domain,
                                if needs_trailing { "/" } else { "" },
                                sitemap_path
                            )
                            .into(),
                        ));

                        self.configuration.configure_allowlist();

                        let shared = Arc::new((
                            self.channel.clone(),
                            self.channel_guard.clone(),
                            browser,
                            self.configuration.clone(),
                            self.url.inner().to_string(),
                            context_id.clone(),
                        ));

                        let mut sitemaps = match self.configuration.sitemap_url {
                            Some(ref sitemap) => Vec::from([sitemap.to_owned()]),
                            _ => Default::default(),
                        };

                        let mut exceeded_budget = false;

                        'outer: loop {
                            let stream = tokio_stream::iter::<Vec<Box<CompactString>>>(
                                sitemaps.drain(..).collect(),
                            );
                            tokio::pin!(stream);

                            while let Some(sitemap_url) = stream.next().await {
                                if !self.handle_process(handle, &mut interval, async {}).await {
                                    break 'outer;
                                }
                                let (tx, mut rx) = tokio::sync::mpsc::channel::<Page>(32);

                                let shared_1 = shared.clone();

                                let handles = spawn_task("page_fetch", async move {
                                    let mut pages = Vec::new();

                                    while let Some(page) = rx.recv().await {
                                        if shared_1.0.is_some() {
                                            if scrape {
                                                pages.push(page.clone());
                                            };
                                            channel_send_page(
                                                &shared_1.0.clone(),
                                                page,
                                                &shared_1.1,
                                            );
                                        } else {
                                            pages.push(page);
                                        }
                                    }

                                    pages
                                });

                                match client.get(sitemap_url.as_str()).send().await {
                                    Ok(response) => {
                                        match response.text().await {
                                            Ok(text) => {
                                                // <html><head><title>Invalid request</title></head><body><p>Blocked by WAF</p><
                                                let mut stream = tokio_stream::iter(
                                                    SiteMapReader::new(text.as_bytes()),
                                                );

                                                while let Some(entity) = stream.next().await {
                                                    if !self
                                                        .handle_process(
                                                            handle,
                                                            &mut interval,
                                                            async {},
                                                        )
                                                        .await
                                                    {
                                                        break;
                                                    }

                                                    match entity {
                                                        SiteMapEntity::Url(url_entry) => {
                                                            match url_entry.loc {
                                                                Location::Url(url) => {
                                                                    let link: CaseInsensitiveString =
                                                                        url.as_str().into();

                                                                    let allowed =
                                                                        self.is_allowed(&link);

                                                                    if allowed.eq(&ProcessLinkStatus::BudgetExceeded) {
                                                                        exceeded_budget = true;
                                                                        break;
                                                                    }
                                                                    if allowed.eq(
                                                                        &ProcessLinkStatus::Blocked,
                                                                    ) {
                                                                        continue;
                                                                    }

                                                                    self.links_visited
                                                                        .insert(link.clone());

                                                                    let client = client.clone();
                                                                    let tx = tx.clone();

                                                                    let shared = shared.clone();

                                                                    spawn_task(
                                                                        "page_fetch",
                                                                        async move {
                                                                            match attempt_navigation(
                                                                            "about:blank",
                                                                            &shared.2,
                                                                            &shared
                                                                                .3
                                                                                .request_timeout,
                                                                            &shared.5,
                                                                            &shared.3.viewport,
                                                                        )
                                                                        .await
                                                                        {
                                                                            Ok(new_page) => {
                                                                                let intercept_handle = crate::features::chrome::setup_chrome_interception_base(
                                                                                    &new_page,
                                                                                    shared.3.chrome_intercept.enabled,
                                                                                    &shared.3.auth_challenge_response,
                                                                                    shared.3.chrome_intercept.block_visuals,
                                                                                    &shared.4,
                                                                                )
                                                                                .await;

                                                                                crate::features::chrome::setup_chrome_events(&new_page, &shared.3).await;

                                                                                let page = Page::new(
                                                                                    &link.inner(),
                                                                                    &client,
                                                                                    &new_page,
                                                                                    &shared.3.wait_for,
                                                                                    &shared.3.screenshot,
                                                                                    false,
                                                                                    &shared.3.openai_config,
                                                                                    &shared.3.execution_scripts,
                                                                                    &shared.3.automation_scripts,
                                                                                    &shared.3.viewport,
                                                                                    &shared.3.request_timeout
                                                                                )
                                                                                .await;

                                                                                if let Some(intercept_handle) = intercept_handle {
                                                                                    let abort_handle = intercept_handle.abort_handle();

                                                                                    if let Err(elasped) = tokio::time::timeout(tokio::time::Duration::from_secs(10), async {
                                                                                        intercept_handle.await
                                                                                    }).await {
                                                                                        log::warn!("Handler timeout exceeded {elasped}");
                                                                                        abort_handle.abort();
                                                                                    }
                                                                                }

                                                                                if let Ok(permit) = tx.reserve().await {
                                                                                    permit.send(page);
                                                                                }
                                                                            }
                                                                            _ => (),
                                                                        }
                                                                        },
                                                                    );
                                                                }
                                                                Location::None
                                                                | Location::ParseErr(_) => (),
                                                            }
                                                        }
                                                        SiteMapEntity::SiteMap(sitemap_entry) => {
                                                            match sitemap_entry.loc {
                                                                Location::Url(url) => {
                                                                    sitemaps.push(Box::new(
                                                                        CompactString::new(
                                                                            &url.as_str(),
                                                                        ),
                                                                    ));
                                                                }
                                                                Location::None
                                                                | Location::ParseErr(_) => (),
                                                            }
                                                        }
                                                        SiteMapEntity::Err(err) => log::info!(
                                                            "incorrect sitemap error: {:?}",
                                                            err.msg(),
                                                        ),
                                                    };

                                                    if exceeded_budget {
                                                        break;
                                                    }
                                                }
                                            }
                                            Err(err) => log::info!(
                                                "http sitemap parse error: {}",
                                                err.to_string()
                                            ),
                                        };
                                    }
                                    Err(err) => log::info!(
                                        "http sitemap network error: {}",
                                        err.to_string()
                                    ),
                                };

                                drop(tx);

                                if let Ok(mut handle) = handles.await {
                                    for page in handle.iter_mut() {
                                        let prev_domain = page.base.take();
                                        page.base = self.domain_parsed.as_deref().cloned();
                                        self.extra_links.extend(page.links(&selectors).await);
                                        page.base = prev_domain;
                                    }
                                    if scrape {
                                        match self.pages.as_mut() {
                                            Some(p) => p.extend(handle),
                                            _ => (),
                                        };
                                    }
                                }
                            }

                            if sitemaps.len() == 0 || exceeded_budget {
                                break;
                            }
                        }

                        crate::features::chrome::close_browser(
                            browser_handle,
                            &shared.2,
                            &mut context_id,
                        )
                        .await;
                    }
                    _ => (),
                }
            }
            _ => (),
        }
    }

    /// Sitemap crawl entire lists. Note: this method does not re-crawl the links of the pages found on the sitemap. This does nothing without the [sitemap] flag.
    #[cfg(feature = "sitemap")]
    pub async fn sitemap_crawl(
        &mut self,
        client: &Client,
        handle: &Option<Arc<AtomicI8>>,
        scrape: bool,
    ) {
        self.sitemap_crawl_raw(client, handle, scrape).await
    }

    /// Sitemap crawl entire lists chain. Note: this method does not re-crawl the links of the pages found on the sitemap. This does nothing without the [sitemap] flag.
    #[cfg(all(
        feature = "sitemap",
        any(not(feature = "chrome"), feature = "decentralized")
    ))]
    async fn sitemap_crawl_chain(
        &mut self,
        client: &Client,
        handle: &Option<Arc<AtomicI8>>,
        scrape: bool,
    ) {
        if !self.configuration.ignore_sitemap {
            self.sitemap_crawl_raw(client, handle, scrape).await
        }
    }

    /// Sitemap crawl entire lists chain using chrome. Note: this method does not re-crawl the links of the pages found on the sitemap. This does nothing without the [sitemap] flag.
    #[cfg(all(
        feature = "sitemap",
        feature = "chrome",
        not(feature = "decentralized")
    ))]
    async fn sitemap_crawl_chain(
        &mut self,
        client: &Client,
        handle: &Option<Arc<AtomicI8>>,
        scrape: bool,
    ) {
        if !self.configuration.ignore_sitemap {
            self.sitemap_crawl_chrome(client, handle, scrape).await
        }
    }

    /// get base link for crawl establishing
    #[cfg(feature = "regex")]
    fn get_base_link(&self) -> &CaseInsensitiveString {
        &self.url
    }

    /// get base link for crawl establishing
    #[cfg(not(feature = "regex"))]
    fn get_base_link(&self) -> &CompactString {
        self.url.inner()
    }

    /// Guard the channel from closing until all subscription events complete.
    fn subscription_guard(&self) {
        if let Some(channel) = &self.channel {
            if !channel.1.is_empty() {
                if let Some(ref guard_counter) = self.channel_guard {
                    guard_counter.lock()
                }
            }
        }
    }

    /// Launch or connect to browser with setup
    #[cfg(feature = "chrome")]
    pub async fn setup_browser(
        &mut self,
    ) -> Option<(
        Arc<chromiumoxide::Browser>,
        tokio::task::JoinHandle<()>,
        Option<chromiumoxide::cdp::browser_protocol::browser::BrowserContextId>,
    )> {
        match crate::features::chrome::launch_browser(&self.configuration, self.get_url_parsed())
            .await
        {
            Some((browser, browser_handle, context_id)) => {
                let browser: Arc<chromiumoxide::Browser> = Arc::new(browser);

                Some((browser, browser_handle, context_id))
            }
            _ => None,
        }
    }

    /// Respect robots.txt file.
    pub fn with_respect_robots_txt(&mut self, respect_robots_txt: bool) -> &mut Self {
        self.configuration
            .with_respect_robots_txt(respect_robots_txt);
        self
    }

    /// Include subdomains detection.
    pub fn with_subdomains(&mut self, subdomains: bool) -> &mut Self {
        self.configuration.with_subdomains(subdomains);
        self
    }

    /// Include tld detection.
    pub fn with_tld(&mut self, tld: bool) -> &mut Self {
        self.configuration.with_tld(tld);
        self
    }

    /// Only use HTTP/2.
    pub fn with_http2_prior_knowledge(&mut self, http2_prior_knowledge: bool) -> &mut Self {
        self.configuration
            .with_http2_prior_knowledge(http2_prior_knowledge);
        self
    }

    /// Delay between request as ms.
    pub fn with_delay(&mut self, delay: u64) -> &mut Self {
        self.configuration.with_delay(delay);
        self
    }

    /// Max time to wait for request.
    pub fn with_request_timeout(&mut self, request_timeout: Option<Duration>) -> &mut Self {
        self.configuration.with_request_timeout(request_timeout);
        self
    }

    /// Dangerously accept invalid certificates - this should be used as a last resort.
    pub fn with_danger_accept_invalid_certs(&mut self, accept_invalid_certs: bool) -> &mut Self {
        self.configuration
            .with_danger_accept_invalid_certs(accept_invalid_certs);
        self
    }

    /// Add user agent to request.
    pub fn with_user_agent(&mut self, user_agent: Option<&str>) -> &mut Self {
        self.configuration.with_user_agent(user_agent);
        self
    }

    /// Preserve the HOST header.
    pub fn with_preserve_host_header(&mut self, preserve: bool) -> &mut Self {
        self.configuration.with_preserve_host_header(preserve);
        self
    }

    #[cfg(feature = "sitemap")]
    /// Add user agent to request. This does nothing without the `sitemap` flag enabled.
    pub fn with_sitemap(&mut self, sitemap_url: Option<&str>) -> &mut Self {
        self.configuration.with_sitemap(sitemap_url);
        self
    }

    #[cfg(not(feature = "sitemap"))]
    /// Add user agent to request. This does nothing without the `sitemap` flag enabled.
    pub fn with_sitemap(&mut self, _sitemap_url: Option<&str>) -> &mut Self {
        self
    }

    /// Use proxies for request.
    pub fn with_proxies(&mut self, proxies: Option<Vec<String>>) -> &mut Self {
        self.configuration.with_proxies(proxies);
        self
    }

    /// Set the concurrency limits. If you set the value to None to use the default limits using the system CPU cors * n.
    pub fn with_concurrency_limit(&mut self, limit: Option<usize>) -> &mut Self {
        self.configuration.with_concurrency_limit(limit);
        self
    }

    /// Set a crawl ID to use for tracking crawls. This does nothing without the `control` flag enabled.
    #[cfg(not(feature = "control"))]
    pub fn with_crawl_id(&mut self, _crawl_id: String) -> &mut Self {
        self
    }

    /// Set a crawl ID to use for tracking crawls. This does nothing without the [control] flag enabled.
    #[cfg(feature = "control")]
    pub fn with_crawl_id(&mut self, crawl_id: String) -> &mut Self {
        self.crawl_id = crawl_id.into();
        self
    }

    /// Add blacklist urls to ignore.
    pub fn with_blacklist_url<T>(&mut self, blacklist_url: Option<Vec<T>>) -> &mut Self
    where
        Vec<CompactString>: From<Vec<T>>,
    {
        self.configuration.with_blacklist_url(blacklist_url);
        self
    }

    /// Set the retry limit for request. Set the value to 0 for no retries. The default is 0.
    pub fn with_retry(&mut self, retry: u8) -> &mut Self {
        self.configuration.with_retry(retry);
        self
    }

    /// Add whitelist urls to allow.
    pub fn with_whitelist_url<T>(&mut self, blacklist_url: Option<Vec<T>>) -> &mut Self
    where
        Vec<CompactString>: From<Vec<T>>,
    {
        self.configuration.with_whitelist_url(blacklist_url);
        self
    }

    /// Set HTTP headers for request using [reqwest::header::HeaderMap](https://docs.rs/reqwest/latest/reqwest/header/struct.HeaderMap.html).
    pub fn with_headers(&mut self, headers: Option<reqwest::header::HeaderMap>) -> &mut Self {
        self.configuration.with_headers(headers);
        self
    }

    /// Set a crawl budget per path with levels support /a/b/c or for all paths with "*". This does nothing without the `budget` flag enabled.
    pub fn with_budget(&mut self, budget: Option<HashMap<&str, u32>>) -> &mut Self {
        self.configuration.with_budget(budget);
        self
    }

    /// Set the crawl budget directly. This does nothing without the `budget` flag enabled.
    pub fn set_crawl_budget(&mut self, budget: Option<HashMap<CaseInsensitiveString, u32>>) {
        self.configuration.budget = budget;
    }

    /// Set a crawl depth limit. If the value is 0 there is no limit. This does nothing without the feat flag `budget` enabled.
    pub fn with_depth(&mut self, depth: usize) -> &mut Self {
        self.configuration.with_depth(depth);
        self
    }

    /// Group external domains to treat the crawl as one. If None is passed this will clear all prior domains.
    pub fn with_external_domains<'a, 'b>(
        &mut self,
        external_domains: Option<impl Iterator<Item = String> + 'a>,
    ) -> &mut Self {
        self.configuration.with_external_domains(external_domains);
        self
    }

    /// Perform a callback to run on each link find.
    pub fn with_on_link_find_callback(
        &mut self,
        on_link_find_callback: Option<
            fn(CaseInsensitiveString, Option<String>) -> (CaseInsensitiveString, Option<String>),
        >,
    ) -> &mut Self {
        match on_link_find_callback {
            Some(callback) => self.on_link_find_callback = Some(callback),
            _ => self.on_link_find_callback = None,
        };
        self
    }

    /// Cookie string to use in request. This does nothing without the `cookies` flag enabled.
    pub fn with_cookies(&mut self, cookie_str: &str) -> &mut Self {
        self.configuration.with_cookies(cookie_str);
        self
    }

    /// Setup cron jobs to run. This does nothing without the `cron` flag enabled.
    pub fn with_cron(&mut self, cron_str: &str, cron_type: CronType) -> &mut Self {
        self.configuration.with_cron(cron_str, cron_type);
        self
    }

    /// Overrides default host system locale with the specified one. This does nothing without the `chrome` flag enabled.
    pub fn with_locale(&mut self, locale: Option<String>) -> &mut Self {
        self.configuration.with_locale(locale);
        self
    }

    /// Use stealth mode for the request. This does nothing without the `chrome` flag enabled.
    pub fn with_stealth(&mut self, stealth_mode: bool) -> &mut Self {
        self.configuration.with_stealth(stealth_mode);
        self
    }

    /// Use OpenAI to get dynamic javascript to drive the browser. This does nothing without the `openai` flag enabled.
    pub fn with_openai(&mut self, openai_configs: Option<configuration::GPTConfigs>) -> &mut Self {
        self.configuration.with_openai(openai_configs);
        self
    }

    /// Cache the page following HTTP rules. This method does nothing if the `cache` feature is not enabled.
    pub fn with_caching(&mut self, cache: bool) -> &mut Self {
        self.configuration.with_caching(cache);
        self
    }

    /// Setup custom fingerprinting for chrome. This method does nothing if the `chrome` feature is not enabled.
    pub fn with_fingerprint(&mut self, fingerprint: bool) -> &mut Self {
        self.configuration.with_fingerprint(fingerprint);
        self
    }

    /// Configures the viewport of the browser, which defaults to 800x600. This method does nothing if the `chrome` feature is not enabled.
    pub fn with_viewport(&mut self, viewport: Option<crate::configuration::Viewport>) -> &mut Self {
        self.configuration.with_viewport(viewport);
        self
    }

    /// Wait for idle network request. This method does nothing if the `chrome` feature is not enabled.
    pub fn with_wait_for_idle_network(
        &mut self,
        wait_for_idle_network: Option<crate::configuration::WaitForIdleNetwork>,
    ) -> &mut Self {
        self.configuration
            .with_wait_for_idle_network(wait_for_idle_network);
        self
    }

    /// Wait for a CSS query selector. This method does nothing if the `chrome` feature is not enabled.
    pub fn with_wait_for_selector(
        &mut self,
        wait_for_selector: Option<crate::configuration::WaitForSelector>,
    ) -> &mut Self {
        self.configuration.with_wait_for_selector(wait_for_selector);
        self
    }

    /// Wait for idle dom mutations for target element. This method does nothing if the [chrome] feature is not enabled.
    pub fn with_wait_for_idle_dom(
        &mut self,
        wait_for_selector: Option<crate::configuration::WaitForSelector>,
    ) -> &mut Self {
        self.configuration.with_wait_for_idle_dom(wait_for_selector);
        self
    }

    /// Wait for a delay. Should only be used for testing. This method does nothing if the `chrome` feature is not enabled.
    pub fn with_wait_for_delay(
        &mut self,
        wait_for_delay: Option<crate::configuration::WaitForDelay>,
    ) -> &mut Self {
        self.configuration.with_wait_for_delay(wait_for_delay);
        self
    }

    /// Set the max redirects allowed for request.
    pub fn with_redirect_limit(&mut self, redirect_limit: usize) -> &mut Self {
        self.configuration.with_redirect_limit(redirect_limit);
        self
    }

    /// Set the redirect policy to use, either Strict or Loose by default.
    pub fn with_redirect_policy(&mut self, policy: RedirectPolicy) -> &mut Self {
        self.configuration.with_redirect_policy(policy);
        self
    }

    /// Use request intercept for the request to only allow content that matches the host. If the content is from a 3rd party it needs to be part of our include list. This method does nothing if the `chrome_intercept` flag is not enabled.
    pub fn with_chrome_intercept(
        &mut self,
        chrome_intercept: RequestInterceptConfiguration,
    ) -> &mut Self {
        self.configuration
            .with_chrome_intercept(chrome_intercept, &self.url);
        self
    }

    /// Determine whether to collect all the resources found on pages.
    pub fn with_full_resources(&mut self, full_resources: bool) -> &mut Self {
        self.configuration.with_full_resources(full_resources);
        self
    }

    /// Ignore the sitemap when crawling. This method does nothing if the `sitemap` flag is not enabled.
    pub fn with_ignore_sitemap(&mut self, ignore_sitemap: bool) -> &mut Self {
        self.configuration.with_ignore_sitemap(ignore_sitemap);
        self
    }

    /// Overrides default host system timezone with the specified one. This does nothing without the `chrome` flag enabled.
    pub fn with_timezone_id(&mut self, timezone_id: Option<String>) -> &mut Self {
        self.configuration.with_timezone_id(timezone_id);
        self
    }

    /// Set a custom script to evaluate on new document creation. This does nothing without the feat flag `chrome` enabled.
    pub fn with_evaluate_on_new_document(
        &mut self,
        evaluate_on_new_document: Option<Box<String>>,
    ) -> &mut Self {
        self.configuration
            .with_evaluate_on_new_document(evaluate_on_new_document);

        self
    }

    /// Set a crawl page limit. If the value is 0 there is no limit. This does nothing without the feat flag `budget` enabled.
    pub fn with_limit(&mut self, limit: u32) -> &mut Self {
        self.configuration.with_limit(limit);
        self
    }

    /// Set the chrome screenshot configuration. This does nothing without the `chrome` flag enabled.
    pub fn with_screenshot(
        &mut self,
        screenshot_config: Option<configuration::ScreenShotConfig>,
    ) -> &mut Self {
        self.configuration.with_screenshot(screenshot_config);
        self
    }

    /// Use a shared semaphore to evenly handle workloads. The default is false.
    pub fn with_shared_queue(&mut self, shared_queue: bool) -> &mut Self {
        self.configuration.with_shared_queue(shared_queue);
        self
    }

    /// Set the authentiation challenge response. This does nothing without the feat flag `chrome` enabled.
    pub fn with_auth_challenge_response(
        &mut self,
        auth_challenge_response: Option<configuration::AuthChallengeResponse>,
    ) -> &mut Self {
        self.configuration
            .with_auth_challenge_response(auth_challenge_response);
        self
    }

    /// Return the links found on the page in the channel subscriptions. This method does nothing if the `decentralized` is enabled.
    pub fn with_return_page_links(&mut self, return_page_links: bool) -> &mut Self {
        self.configuration.with_return_page_links(return_page_links);
        self
    }

    /// Set the connection url for the chrome instance. This method does nothing if the `chrome` is not enabled.
    pub fn with_chrome_connection(&mut self, chrome_connection_url: Option<String>) -> &mut Self {
        self.configuration
            .with_chrome_connection(chrome_connection_url);
        self
    }

    /// Set JS to run on certain pages. This method does nothing if the `chrome` is not enabled.
    pub fn with_execution_scripts(
        &mut self,
        execution_scripts: Option<ExecutionScriptsMap>,
    ) -> &mut Self {
        self.configuration.with_execution_scripts(execution_scripts);
        self
    }

    /// Run web automated actions on certain pages. This method does nothing if the `chrome` is not enabled.
    pub fn with_automation_scripts(
        &mut self,
        automation_scripts: Option<AutomationScriptsMap>,
    ) -> &mut Self {
        self.configuration
            .with_automation_scripts(automation_scripts);
        self
    }

    /// Block assets from loading from the network. Focus primarly on HTML documents.
    pub fn with_block_assets(&mut self, only_html: bool) -> &mut Self {
        self.configuration.with_block_assets(only_html);
        self
    }

    /// Set the configuration for the website directly.
    pub fn with_config(&mut self, config: Configuration) -> &mut Self {
        self.configuration = config.into();
        self
    }

    /// Build the website configuration when using with_builder.
    pub fn build(&self) -> Result<Self, Self> {
        if self.domain_parsed.is_none() {
            Err(self.to_owned())
        } else {
            Ok(self.to_owned())
        }
    }

    /// Determine if the budget has a wildcard path and the depth limit distance. This does nothing without the `budget` flag enabled.
    fn determine_limits(&mut self) {
        self.configuration.configure_budget();
        if self.configuration.inner_budget.is_some() {
            let wild_card_budget = match &self.configuration.inner_budget {
                Some(budget) => budget.contains_key(&*WILD_CARD_PATH),
                _ => false,
            };
            self.configuration.wild_card_budgeting = wild_card_budget;
        }
        if self.configuration.depth > 0 && self.domain_parsed.is_some() {
            if let Some(ref domain) = self.domain_parsed {
                if let Some(segments) = domain.path_segments() {
                    let segments_cnt = segments.count();

                    if segments_cnt > self.configuration.depth {
                        self.configuration.depth_distance = self.configuration.depth
                            + self.configuration.depth.abs_diff(segments_cnt);
                    } else {
                        self.configuration.depth_distance = self.configuration.depth;
                    }
                }
            }
        }
    }

    #[cfg(not(feature = "sync"))]
    /// Sets up a subscription to receive concurrent data. This will panic if it is larger than `usize::MAX / 2`.
    /// Set the value to `0` to use the semaphore permits. If the subscription is going to block or use async methods,
    /// make sure to spawn a task to avoid losing messages. This does nothing unless the `sync` flag is enabled.
    ///
    /// # Examples
    ///
    /// Subscribe and receive messages using an async tokio environment:
    ///
    /// ```rust
    /// use spider::{tokio, website::Website};
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let mut website = Website::new("http://example.com");
    ///     let mut rx = website.subscribe(0).unwrap();
    ///
    ///     tokio::spawn(async move {
    ///         while let Ok(page) = rx.recv().await {
    ///             tokio::spawn(async move {
    ///                 // Process the received page.
    ///                 // If performing non-blocking tasks or managing a high subscription count, configure accordingly.
    ///             });
    ///         }
    ///     });
    ///
    ///     website.crawl().await;
    /// }
    /// ```
    pub fn subscribe(&mut self, capacity: usize) -> Option<broadcast::Receiver<Page>> {
        None
    }

    /// Sets up a subscription to receive concurrent data. This will panic if it is larger than `usize::MAX / 2`.
    /// Set the value to `0` to use the semaphore permits. If the subscription is going to block or use async methods,
    /// make sure to spawn a task to avoid losing messages. This does nothing unless the `sync` flag is enabled.
    ///
    /// # Examples
    ///
    /// Subscribe and receive messages using an async tokio environment:
    ///
    /// ```rust
    /// use spider::{tokio, website::Website};
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let mut website = Website::new("http://example.com");
    ///     let mut rx = website.subscribe(0).unwrap();
    ///
    ///     tokio::spawn(async move {
    ///         while let Ok(page) = rx.recv().await {
    ///             tokio::spawn(async move {
    ///                 // Process the received page.
    ///                 // If performing non-blocking tasks or managing a high subscription count, configure accordingly.
    ///             });
    ///         }
    ///     });
    ///
    ///     website.crawl().await;
    /// }
    /// ```
    #[cfg(feature = "sync")]
    pub fn subscribe(&mut self, capacity: usize) -> Option<broadcast::Receiver<Page>> {
        let channel = self.channel.get_or_insert_with(|| {
            let (tx, rx) = broadcast::channel(
                (if capacity == 0 {
                    *DEFAULT_PERMITS
                } else {
                    capacity
                })
                .max(1),
            );
            (tx, Arc::new(rx))
        });

        let rx2 = channel.0.subscribe();

        Some(rx2)
    }

    /// Get a sender for queueing extra links mid crawl. This does nothing unless the `sync` flag is enabled.
    #[cfg(feature = "sync")]
    pub fn queue(&mut self, capacity: usize) -> Option<broadcast::Sender<String>> {
        let channel = self.channel_queue.get_or_insert_with(|| {
            let (tx, rx) = broadcast::channel(capacity);
            (tx, Arc::new(rx))
        });

        Some(channel.0.to_owned())
    }

    /// Get a sender for queueing extra links mid crawl. This does nothing unless the `sync` flag is enabled.
    #[cfg(not(feature = "sync"))]
    pub fn queue(
        &mut self,
        capacity: usize,
    ) -> Option<Arc<(broadcast::Sender<Page>, broadcast::Receiver<Page>)>> {
        None
    }

    /// Remove subscriptions for data. This is useful for auto droping subscriptions that are running on another thread. This does nothing without the `sync` flag enabled.
    #[cfg(not(feature = "sync"))]
    pub fn unsubscribe(&mut self) {}

    /// Remove subscriptions for data. This is useful for auto droping subscriptions that are running on another thread. This does nothing without the `sync` flag enabled.
    #[cfg(feature = "sync")]
    pub fn unsubscribe(&mut self) {
        self.channel.take();
    }

    /// Setup subscription counter to track concurrent operation completions.
    /// This helps keep a chrome instance active until all operations are completed from all threads to safely take screenshots and other actions.
    /// Make sure to call `inc` if you take a guard. Without calling `inc` in the subscription receiver the crawl will stay in a infinite loop.
    /// This does nothing without the `sync` flag enabled. You also need to use the 'chrome_store_page' to keep the page alive between request.
    ///
    /// # Example
    ///
    /// ```
    /// use spider::tokio;
    /// use spider::website::Website;
    /// #[tokio::main]
    ///
    /// async fn main() {
    ///     let mut website: Website = Website::new("http://example.com");
    ///     let mut rx2 = website.subscribe(18).unwrap();
    ///     let mut rxg = website.subscribe_guard().unwrap();
    ///
    ///     tokio::spawn(async move {
    ///         while let Ok(page) = rx2.recv().await {
    ///             println!("📸 - {:?}", page.get_url());
    ///             page
    ///                 .screenshot(
    ///                     true,
    ///                     true,
    ///                     spider::configuration::CaptureScreenshotFormat::Png,
    ///                     Some(75),
    ///                     None::<std::path::PathBuf>,
    ///                     None,
    ///                 )
    ///                 .await;
    ///             rxg.inc();
    ///         }
    ///     });
    ///     website.crawl().await;
    /// }
    /// ```
    #[cfg(not(feature = "sync"))]
    pub fn subscribe_guard(&mut self) -> Option<ChannelGuard> {
        None
    }

    /// Setup subscription counter to track concurrent operation completions.
    /// This helps keep a chrome instance active until all operations are completed from all threads to safely take screenshots and other actions.
    /// Make sure to call `inc` if you take a guard. Without calling `inc` in the subscription receiver the crawl will stay in a infinite loop.
    /// This does nothing without the `sync` flag enabled. You also need to use the 'chrome_store_page' to keep the page alive between request.
    ///
    /// # Example
    ///
    /// ```
    /// use spider::tokio;
    /// use spider::website::Website;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let mut website: Website = Website::new("http://example.com");
    ///     let mut rx2 = website.subscribe(18).unwrap();
    ///     let mut rxg = website.subscribe_guard().unwrap();
    ///
    ///     tokio::spawn(async move {
    ///         while let Ok(page) = rx2.recv().await {
    ///             println!("📸 - {:?}", page.get_url());
    ///             page
    ///                 .screenshot(
    ///                     true,
    ///                     true,
    ///                     spider::configuration::CaptureScreenshotFormat::Png,
    ///                     Some(75),
    ///                     None::<std::path::PathBuf>,
    ///                     None,
    ///                 )
    ///                 .await;
    ///             rxg.inc();
    ///         }
    ///     });
    ///     website.crawl().await;
    /// }
    /// ```
    #[cfg(feature = "sync")]
    pub fn subscribe_guard(&mut self) -> Option<ChannelGuard> {
        // *note*: it would be better to handle this on page drop if the subscription is used automatically. For now we add the API upfront.
        let channel_guard = self.channel_guard.get_or_insert_with(ChannelGuard::new);
        Some(channel_guard.clone())
    }

    #[cfg(feature = "cron")]
    /// Start a cron job - if you use subscribe on another thread you need to abort the handle in conjuction with runner.stop.
    pub async fn run_cron(&self) -> Runner {
        async_job::Runner::new()
            .add(Box::new(self.clone()))
            .run()
            .await
    }

    #[cfg(not(feature = "control"))]
    /// Get the attached crawl id.
    pub fn get_crawl_id(&self) -> Option<&Box<String>> {
        None
    }

    #[cfg(feature = "control")]
    /// Get the attached crawl id.
    pub fn get_crawl_id(&self) -> Option<&Box<String>> {
        if self.crawl_id.is_empty() {
            None
        } else {
            Some(&self.crawl_id)
        }
    }
}

/// Channel broadcast send the Page to receivers.
fn channel_send_page(
    channel: &Option<(
        tokio::sync::broadcast::Sender<Page>,
        std::sync::Arc<tokio::sync::broadcast::Receiver<Page>>,
    )>,
    page: Page,
    channel_guard: &Option<ChannelGuard>,
) {
    if let Some(c) = channel {
        if c.0.send(page).is_ok() {
            if let Some(guard) = channel_guard {
                ChannelGuard::inc_guard(&guard.0 .1)
            }
        }
    }
}

/// Guard a channel from closing until all concurrent operations are done.
#[derive(Debug, Clone)]
pub struct ChannelGuard(Arc<(AtomicBool, AtomicUsize)>);

impl ChannelGuard {
    /// Create a new channel guard. The tuple has the guard control and the counter.
    pub(crate) fn new() -> ChannelGuard {
        ChannelGuard(Arc::new((AtomicBool::new(true), AtomicUsize::new(0))))
    }
    /// Lock the channel until complete. This is only used for when storing the chrome page outside.
    pub(crate) fn lock(&self) {
        if self.0 .0.load(Ordering::Relaxed) {
            while self
                .0
                 .1
                .compare_exchange_weak(0, 0, Ordering::Acquire, Ordering::Relaxed)
                .is_err()
            {
                std::hint::spin_loop();
            }
        }
        std::sync::atomic::fence(Ordering::Acquire);
    }

    /// Set the guard control manually. If this is set to false the loop will not enter.
    pub fn guard(&mut self, guard: bool) {
        self.0 .0.store(guard, Ordering::Release);
    }

    /// Increment the guard channel completions.
    // rename on next major since logic is now flow-controlled.
    pub fn inc(&mut self) {
        self.0 .1.fetch_sub(1, std::sync::atomic::Ordering::Release);
    }

    /// Increment a guard channel completions.
    pub(crate) fn inc_guard(guard: &AtomicUsize) {
        guard.fetch_add(1, std::sync::atomic::Ordering::Release);
    }
}

impl Drop for ChannelGuard {
    fn drop(&mut self) {
        self.0 .0.store(false, Ordering::Release);
    }
}

#[cfg(feature = "cron")]
/// Start a cron job taking ownership of the website
pub async fn run_cron(website: Website) -> Runner {
    async_job::Runner::new().add(Box::new(website)).run().await
}

#[cfg(feature = "cron")]
#[async_trait]
impl Job for Website {
    fn schedule(&self) -> Option<async_job::Schedule> {
        match self.configuration.cron_str.parse() {
            Ok(schedule) => Some(schedule),
            Err(e) => {
                log::error!("{:?}", e);
                None
            }
        }
    }
    async fn handle(&mut self) {
        log::info!(
            "CRON: {} - cron job running {}",
            self.get_url().as_ref(),
            self.now()
        );
        if self.configuration.cron_type == CronType::Crawl {
            self.crawl().await;
        } else {
            self.scrape().await;
        }
    }
}

impl std::fmt::Display for Website {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(
            f,
            "Website:\n  URL: {}\n ID: {:?}\n Configuration: {:?}",
            self.get_url(),
            self.get_crawl_id(),
            self.configuration
        )
    }
}

impl std::error::Error for Website {}

#[cfg(not(feature = "decentralized"))]
#[tokio::test]
async fn crawl() {
    let url = "https://choosealicense.com";
    let mut website: Website = Website::new(url);
    website.crawl().await;
    assert!(
        website
            .links_visited
            .contains(&"https://choosealicense.com/licenses/".into()),
        "{:?}",
        website.links_visited
    );
}

#[cfg(feature = "cron")]
#[tokio::test]
async fn crawl_cron() {
    let url = "https://choosealicense.com";
    let mut website: Website = Website::new(&url)
        .with_cron("1/5 * * * * *", Default::default())
        .build()
        .unwrap();
    let mut rx2 = website.subscribe(16).unwrap();

    // handle an event on every cron
    let join_handle = tokio::spawn(async move {
        let mut links_visited = HashSet::new();
        while let Ok(res) = rx2.recv().await {
            let url = res.get_url();
            links_visited.insert(CaseInsensitiveString::new(url));
        }
        assert!(
            links_visited.contains(&CaseInsensitiveString::from(
                "https://choosealicense.com/licenses/"
            )),
            "{:?}",
            links_visited
        );
    });

    let mut runner = website.run_cron().await;
    log::debug!("Starting the Runner for 10 seconds");
    tokio::time::sleep(Duration::from_secs(10)).await;
    runner.stop().await;
    join_handle.abort();
    let _ = join_handle.await;
}

#[cfg(feature = "cron")]
#[tokio::test]
async fn crawl_cron_own() {
    let url = "https://choosealicense.com";
    let mut website: Website = Website::new(&url)
        .with_cron("1/5 * * * * *", Default::default())
        .build()
        .unwrap();
    let mut rx2 = website.subscribe(16).unwrap();

    // handle an event on every cron
    let join_handle = tokio::spawn(async move {
        let mut links_visited = HashSet::new();
        while let Ok(res) = rx2.recv().await {
            let url = res.get_url();
            links_visited.insert(CaseInsensitiveString::new(url));
        }
        assert!(
            links_visited.contains(&CaseInsensitiveString::from(
                "https://choosealicense.com/licenses/"
            )),
            "{:?}",
            links_visited
        );
    });

    let mut runner = run_cron(website).await;
    log::debug!("Starting the Runner for 10 seconds");
    tokio::time::sleep(Duration::from_secs(10)).await;
    let _ = tokio::join!(runner.stop(), join_handle);
}

#[cfg(not(feature = "decentralized"))]
#[tokio::test]
async fn scrape() {
    let mut website: Website = Website::new("https://choosealicense.com");
    website.scrape().await;
    assert!(
        website
            .links_visited
            .contains(&"https://choosealicense.com/licenses/".into()),
        "{:?}",
        website.links_visited
    );

    assert!(!website.get_pages().unwrap()[0].get_html().is_empty());
}

#[tokio::test]
#[cfg(not(feature = "decentralized"))]
async fn crawl_invalid() {
    let mut website: Website = Website::new("https://w.com");
    website.crawl().await;
    assert!(website.links_visited.len() <= 1); // only the target url should exist
}

#[tokio::test]
#[cfg(feature = "decentralized")]
async fn crawl_invalid() {
    let domain = "https://w.com";
    let mut website: Website = Website::new(domain);
    website.crawl().await;
    let mut uniq: Box<HashSet<CaseInsensitiveString>> = Box::new(HashSet::new());
    uniq.insert(format!("{}/", domain.to_string()).into()); // TODO: remove trailing slash mutate

    assert_eq!(website.links_visited.get_links(), *uniq); // only the target url should exist
}

#[tokio::test]
async fn not_crawl_blacklist() {
    let mut website: Website = Website::new("https://choosealicense.com");
    website.configuration.blacklist_url = Some(Box::new(Vec::from([CompactString::from(
        "https://choosealicense.com/licenses/",
    )])));

    website.crawl().await;
    assert!(
        !website
            .links_visited
            .contains(&"https://choosealicense.com/licenses/".into()),
        "{:?}",
        website.links_visited
    );
}

#[tokio::test]
#[cfg(feature = "regex")]
async fn not_crawl_blacklist_regex() {
    let mut website: Website = Website::new("https://choosealicense.com");
    website.with_blacklist_url(Some(Vec::from(["choosealicense.com".into()])));
    website.crawl().await;
    assert_eq!(website.links_visited.len(), 0);
}

#[test]
#[cfg(feature = "ua_generator")]
fn randomize_website_agent() {
    assert_eq!(get_ua(false).is_empty(), false);
}

#[tokio::test]
#[cfg(not(feature = "decentralized"))]
async fn test_respect_robots_txt() {
    let mut website: Website = Website::new("https://stackoverflow.com");
    website.configuration.respect_robots_txt = true;
    website.configuration.user_agent = Some(Box::new("*".into()));

    let (client, _): (Client, Option<(Arc<AtomicI8>, tokio::task::JoinHandle<()>)>) =
        website.setup().await;

    website.configure_robots_parser(client).await;

    assert_eq!(website.configuration.delay, 0);

    assert!(!&website
        .is_allowed(&"https://stackoverflow.com/posts/".into())
        .eq(&ProcessLinkStatus::Allowed));

    // test match for bing bot
    let mut website_second: Website = Website::new("https://www.mongodb.com");
    website_second.configuration.respect_robots_txt = true;
    website_second.configuration.user_agent = Some(Box::new("bingbot".into()));

    let (client_second, _): (Client, Option<(Arc<AtomicI8>, tokio::task::JoinHandle<()>)>) =
        website_second.setup().await;
    website_second.configure_robots_parser(client_second).await;

    assert_eq!(website_second.configuration.delay, 60000); // should equal one minute in ms

    // test crawl delay with wildcard agent [DOES not work when using set agent]
    let mut website_third: Website = Website::new("https://www.mongodb.com");
    website_third.configuration.respect_robots_txt = true;
    let (client_third, _): (Client, Option<(Arc<AtomicI8>, tokio::task::JoinHandle<()>)>) =
        website_third.setup().await;

    website_third.configure_robots_parser(client_third).await;

    assert_eq!(website_third.configuration.delay, 10000); // should equal 10 seconds in ms
}

#[cfg(not(feature = "decentralized"))]
#[tokio::test]
async fn test_crawl_subdomains() {
    let mut website: Website = Website::new("https://choosealicense.com");
    website.configuration.subdomains = true;
    website.crawl().await;
    assert!(
        website
            .links_visited
            .contains(&"https://choosealicense.com/licenses/".into()),
        "{:?}",
        website.links_visited
    );
}

#[tokio::test]
#[cfg(all(not(feature = "regex"), not(feature = "openai")))]
async fn test_with_configuration() {
    let mut website = Website::new("https://choosealicense.com");

    website
        .with_respect_robots_txt(true)
        .with_subdomains(true)
        .with_tld(false)
        .with_delay(0)
        .with_request_timeout(None)
        .with_http2_prior_knowledge(false)
        .with_user_agent(Some(crate::page::TEST_AGENT_NAME))
        .with_headers(None)
        .with_proxies(None);

    let mut configuration = Box::new(configuration::Configuration::new());

    configuration.respect_robots_txt = true;
    configuration.subdomains = true;
    configuration.tld = false;
    configuration.delay = 0;
    configuration.request_timeout = None;
    configuration.http2_prior_knowledge = false;
    configuration.user_agent = Some(Box::new(CompactString::new(crate::page::TEST_AGENT_NAME)));
    configuration.headers = None;
    configuration.proxies = None;

    assert!(
        website.configuration == configuration,
        "Left\n{:?}\n\nRight\n{:?}",
        website.configuration,
        configuration
    );
}

#[cfg(all(feature = "glob", not(feature = "decentralized")))]
#[tokio::test]
async fn test_crawl_glob() {
    let mut website: Website =
        Website::new("https://choosealicense.com/licenses/{mit,apache-2.0,mpl-2.0}/");
    website.crawl().await;

    // check for either https/http in collection
    assert!(
        website
            .links_visited
            .contains(&"https://choosealicense.com/licenses/".into())
            || website
                .links_visited
                .contains(&"http://choosealicense.com/licenses/".into()),
        "{:?}",
        website.links_visited
    );
}

#[cfg(not(feature = "decentralized"))]
#[tokio::test]
async fn test_crawl_tld() {
    let mut website: Website = Website::new("https://choosealicense.com");
    website.configuration.tld = true;
    website.crawl().await;

    assert!(
        website
            .links_visited
            .contains(&"https://choosealicense.com/licenses/".into()),
        "{:?}",
        website.links_visited
    );
}

#[tokio::test]
#[cfg(all(feature = "sync", not(feature = "decentralized")))]
async fn test_crawl_subscription() {
    let mut website: Website = Website::new("https://choosealicense.com");
    let mut rx2 = website.subscribe(100).unwrap();
    let count = Arc::new(tokio::sync::Mutex::new(0));
    let count1 = count.clone();

    tokio::spawn(async move {
        while let Ok(_) = rx2.recv().await {
            let mut lock = count1.lock().await;
            *lock += 1;
        }
    });

    website.crawl().await;
    let website_links = website.get_links().len();
    let count = *count.lock().await;

    // no subscription if did not fulfill. The root page is always captured in links.
    assert!(count == website_links, "{:?}", true);
}

#[cfg(all(feature = "socks", not(feature = "decentralized")))]
#[tokio::test]
async fn test_crawl_proxy() {
    let mut website: Website = Website::new("https://choosealicense.com");
    website
        .configuration
        .proxies
        .get_or_insert(Default::default())
        .push("socks5://127.0.0.1:1080".into());

    website.crawl().await;

    let mut license_found = false;

    for links_visited in website.get_links() {
        // Proxy may return http or https in socks5 per platform.
        // We may want to replace the protocol with the host of the platform regardless of proxy response.
        if links_visited.as_ref().contains("/licenses/") {
            license_found = true;
        };
    }

    assert!(license_found, "{:?}", website.links_visited);
}

#[tokio::test]
async fn test_link_duplicates() {
    fn has_unique_elements<T>(iter: T) -> bool
    where
        T: IntoIterator,
        T::Item: Eq + std::hash::Hash,
    {
        let mut uniq = HashSet::new();
        iter.into_iter().all(move |x| uniq.insert(x))
    }

    let mut website: Website = Website::new("http://0.0.0.0:8000");
    website.crawl().await;

    assert!(has_unique_elements(website.links_visited.get_links()));
}

#[tokio::test]
async fn test_crawl_budget() {
    let mut website: Website = Website::new("https://choosealicense.com");
    website.with_budget(Some(HashMap::from([("*", 1), ("/licenses", 1)])));
    website.crawl().await;

    assert!(website.links_visited.len() <= 1);
}

#[tokio::test]
#[cfg(feature = "control")]
#[ignore]
async fn test_crawl_pause_resume() {
    use crate::utils::{pause, resume};

    let domain = "https://choosealicense.com/";
    let mut website: Website = Website::new(&domain);

    let start = tokio::time::Instant::now();

    tokio::spawn(async move {
        pause(domain).await;
        // static website test pause/resume - scan will never take longer than 5secs for target website choosealicense
        tokio::time::sleep(Duration::from_millis(5000)).await;
        resume(domain).await;
    });

    website.crawl().await;

    let duration = start.elapsed();

    assert!(duration.as_secs() >= 5, "{:?}", duration);

    assert!(
        website
            .links_visited
            .contains(&"https://choosealicense.com/licenses/".into()),
        "{:?}",
        website.links_visited
    );
}

#[cfg(feature = "control")]
#[ignore]
#[tokio::test]
async fn test_crawl_shutdown() {
    use crate::utils::shutdown;

    // use target blog to prevent shutdown of prior crawler
    let domain = "https://spider.cloud/";
    let mut website: Website = Website::new(&domain);

    tokio::spawn(async move {
        shutdown(domain).await;
    });

    website.crawl().await;
    let links_visited_count = website.links_visited.len();

    assert!(links_visited_count <= 1, "{:?}", links_visited_count);
}

#[tokio::test]
#[cfg(all(feature = "cache_request", not(feature = "decentralized")))]
async fn test_cache() {
    let domain = "https://choosealicense.com/";
    let mut website: Website = Website::new(&domain);
    website.configuration.cache = true;

    let fresh_start = tokio::time::Instant::now();
    website.crawl().await;
    let fresh_duration = fresh_start.elapsed();

    let cached_start = tokio::time::Instant::now();
    website.crawl().await;
    let cached_duration = cached_start.elapsed();

    // cache should be faster at least 5x.
    assert!(
        fresh_duration.as_millis() > cached_duration.as_millis() * 5,
        "{:?}",
        cached_duration
    );
}
