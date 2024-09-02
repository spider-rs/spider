use crate::black_list::contains;
use crate::compact_str::CompactString;
use crate::configuration::{
    self, get_ua, AutomationScriptsMap, Configuration, ExecutionScriptsMap, RedirectPolicy,
};
use crate::packages::robotparser::parser::RobotFileParser;
use crate::page::{get_page_selectors, Page};
use crate::utils::log;
use crate::CaseInsensitiveString;
use crate::Client;
use hashbrown::{HashMap, HashSet};
use reqwest::redirect::Policy;
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicI8, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::{
    runtime::Handle,
    sync::{broadcast, Semaphore},
    task::JoinSet,
    time::Interval,
};
use tokio_stream::StreamExt;
use url::Url;

#[cfg(feature = "chrome")]
use crate::features::chrome::{configure_browser, launch_browser};

#[cfg(feature = "cache")]
use http_cache_reqwest::{CACacheManager, Cache, CacheMode, HttpCache, HttpCacheOptions};

#[cfg(feature = "cron")]
use async_job::{async_trait, Job, Runner};

#[cfg(feature = "cache")]
lazy_static! {
    /// Cache manager for request.
    pub static ref CACACHE_MANAGER: CACacheManager = CACacheManager::default();
}

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
    static ref DEFAULT_PERMITS: usize = calc_limits(1);
    static ref SEM_SHARED: Arc<Semaphore> = {
        let base_limit = match std::env::var("SEMAPHORE_MULTIPLIER") {
            Ok(multiplier) => match multiplier.parse::<usize>() {
                Ok(parsed_value) => DEFAULT_PERMITS.wrapping_mul(parsed_value.max(1)),
                Err(_) => *DEFAULT_PERMITS,
            },
            _ => *DEFAULT_PERMITS,
        };
        Arc::new(Semaphore::const_new(base_limit))
    };
}

#[cfg(not(feature = "decentralized"))]
lazy_static! {
    static ref SEM: Semaphore = {
        let base_limit = calc_limits(1);

        let base_limit = match std::env::var("SEMAPHORE_MULTIPLIER") {
            Ok(multiplier) => match multiplier.parse::<usize>() {
                Ok(parsed_value) => base_limit * parsed_value.max(1),
                Err(_) => base_limit,
            },
            _ => base_limit,
        };

        Semaphore::const_new(base_limit)
    };
}

#[cfg(feature = "decentralized")]
lazy_static! {
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

/// Setup interception for chrome request.
#[cfg(all(
    feature = "chrome",
    feature = "chrome_intercept",
    not(feature = "adblock")
))]
async fn setup_chrome_interception_base(
    page: &chromiumoxide::Page,
    chrome_intercept: bool,
    auth_challenge_response: &Option<configuration::AuthChallengeResponse>,
    ignore_visuals: bool,
    host_name: &str,
) -> Option<tokio::task::JoinHandle<()>> {
    if chrome_intercept {
        use chromiumoxide::cdp::browser_protocol::network::ResourceType;

        match auth_challenge_response {
            Some(ref auth_challenge_response) => {
                match page
                        .event_listener::<chromiumoxide::cdp::browser_protocol::fetch::EventAuthRequired>()
                        .await
                        {
                            Ok(mut rp) => {
                                let intercept_page = page.clone();
                                let auth_challenge_response = auth_challenge_response.clone();

                                // we may need return for polling
                                tokio::task::spawn(async move {
                                    while let Some(event) = rp.next().await {
                                        let u = &event.request.url;
                                        let acr = chromiumoxide::cdp::browser_protocol::fetch::AuthChallengeResponse::from(auth_challenge_response.clone());

                                        match chromiumoxide::cdp::browser_protocol::fetch::ContinueWithAuthParams::builder()
                                        .request_id(event.request_id.clone())
                                        .auth_challenge_response(acr)
                                        .build() {
                                            Ok(c) => {
                                                if let Err(e) = intercept_page.execute(c).await
                                                {
                                                    log("Failed to fullfill auth challege request: ", e.to_string());
                                                }
                                            }
                                            _ => {
                                                log("Failed to get auth challege request handle ", &u);
                                            }
                                        }
                                    }
                                });
                            }
                            _ => (),
                        }
            }
            _ => (),
        }

        match page
            .event_listener::<chromiumoxide::cdp::browser_protocol::fetch::EventRequestPaused>()
            .await
        {
            Ok(mut rp) => {
                let mut host_name = host_name.to_string();
                let intercept_page = page.clone();

                let ih = tokio::task::spawn(async move {
                    let mut first_rq = true;
                    while let Some(event) = rp.next().await {
                        let u = &event.request.url;

                        if first_rq {
                            if ResourceType::Document == event.resource_type {
                                host_name = u.into();
                            }
                            first_rq = false;
                        }

                        if
                                    ignore_visuals && (ResourceType::Image == event.resource_type || ResourceType::Media == event.resource_type || ResourceType::Stylesheet == event.resource_type) ||
                                    ResourceType::Prefetch == event.resource_type ||
                                    ResourceType::Ping == event.resource_type ||
                                    ResourceType::Script == event.resource_type && !(u.starts_with('/') || u.starts_with(&host_name) || crate::page::JS_FRAMEWORK_ALLOW.contains(&u.as_str())) // add one off stripe framework check for now...
                                {
                                    match chromiumoxide::cdp::browser_protocol::fetch::FulfillRequestParams::builder()
                                    .request_id(event.request_id.clone())
                                    .response_code(200)
                                    .build() {
                                        Ok(c) => {
                                            if let Err(e) = intercept_page.execute(c).await
                                            {
                                                log("Failed to fullfill request: ", e.to_string());
                                            }
                                        }
                                        _ => {
                                            log("Failed to get request handle ", &host_name);
                                        }
                                    }
                            } else if let Err(e) = intercept_page
                                .execute(chromiumoxide::cdp::browser_protocol::fetch::ContinueRequestParams::new(event.request_id.clone()))
                                .await
                                {
                                    log("Failed to continue request: ", e.to_string());
                                }
                    }
                });

                Some(ih)
            }
            _ => None,
        }
    } else {
        None
    }
}

/// Setup interception for chrome request with advertisement blocking.
#[cfg(all(feature = "chrome", feature = "chrome_intercept", feature = "adblock"))]
async fn setup_chrome_interception_base(
    page: &chromiumoxide::Page,
    chrome_intercept: bool,
    auth_challenge_response: &Option<configuration::AuthChallengeResponse>,
    ignore_visuals: bool,
    host_name: &str,
) -> Option<tokio::task::JoinHandle<()>> {
    if chrome_intercept {
        use adblock::{
            lists::{FilterSet, ParseOptions},
            Engine,
        };
        use chromiumoxide::cdp::browser_protocol::network::ResourceType;

        lazy_static! {
            static ref AD_ENGINE: Engine = {
                let mut filter_set = FilterSet::new(false);
                filter_set.add_filters(
                    &vec![
                        String::from("-advertisement."),
                        String::from("-ads."),
                        String::from("-ad."),
                        String::from("-advertisement-icon."),
                        String::from("-advertisement-management/"),
                        String::from("-advertisement/script."),
                        String::from("-ads/script."),
                    ],
                    ParseOptions::default(),
                );
                Engine::from_filter_set(filter_set, true)
            };
        }

        match auth_challenge_response {
            Some(ref auth_challenge_response) => {
                match page
                        .event_listener::<chromiumoxide::cdp::browser_protocol::fetch::EventAuthRequired>()
                        .await
                        {
                            Ok(mut rp) => {
                                let intercept_page = page.clone();
                                let auth_challenge_response = auth_challenge_response.clone();

                                // we may need return for polling
                                tokio::task::spawn(async move {
                                    while let Some(event) = rp.next().await {
                                        let u = &event.request.url;
                                        let acr = chromiumoxide::cdp::browser_protocol::fetch::AuthChallengeResponse::from(auth_challenge_response.clone());

                                        match chromiumoxide::cdp::browser_protocol::fetch::ContinueWithAuthParams::builder()
                                        .request_id(event.request_id.clone())
                                        .auth_challenge_response(acr)
                                        .build() {
                                            Ok(c) => {
                                                if let Err(e) = intercept_page.execute(c).await
                                                {
                                                    log("Failed to fullfill auth challege request: ", e.to_string());
                                                }
                                            }
                                            _ => {
                                                log("Failed to get auth challege request handle ", &u);
                                            }
                                        }
                                    }
                                });
                            }
                            _ => (),
                        }
            }
            _ => (),
        }

        match page
            .event_listener::<chromiumoxide::cdp::browser_protocol::fetch::EventRequestPaused>()
            .await
        {
            Ok(mut rp) => {
                let mut host_name = host_name.to_string();
                let intercept_page = page.clone();

                let ih = tokio::task::spawn(async move {
                    let mut first_rq = true;
                    while let Some(event) = rp.next().await {
                        let u = &event.request.url;

                        if first_rq {
                            if ResourceType::Document == event.resource_type {
                                host_name = u.into();
                            }
                            first_rq = false;
                        }

                        let asset = ResourceType::Image == event.resource_type
                            || ResourceType::Media == event.resource_type
                            || ResourceType::Stylesheet == event.resource_type;

                        if
                                    ignore_visuals && asset ||
                                    ResourceType::Prefetch == event.resource_type ||
                                    ResourceType::Ping == event.resource_type ||
                                    ResourceType::Script == event.resource_type && !(u.starts_with('/') || u.starts_with(&host_name) || crate::page::JS_FRAMEWORK_ALLOW.contains(&u.as_str())) ||
                                    !ignore_visuals && (asset || event.resource_type == ResourceType::Fetch || event.resource_type == ResourceType::Xhr) && match adblock::request::Request::new(&u, &intercept_page.url().await.unwrap_or_default().unwrap_or_default(),  &event.resource_type.as_ref()) {
                                        Ok(adblock_request) => {
                                            AD_ENGINE.check_network_request(&adblock_request).matched
                                        }
                                        _ => false
                                    }
                                {
                                    match chromiumoxide::cdp::browser_protocol::fetch::FulfillRequestParams::builder()
                                    .request_id(event.request_id.clone())
                                    .response_code(200)
                                    .build() {
                                        Ok(c) => {
                                            if let Err(e) = intercept_page.execute(c).await
                                            {
                                                log("Failed to fullfill request: ", e.to_string());
                                            }
                                        }
                                        _ => {
                                            log("Failed to get request handle ", &host_name);
                                        }
                                    }
                            } else if let Err(e) = intercept_page
                                .execute(chromiumoxide::cdp::browser_protocol::fetch::ContinueRequestParams::new(event.request_id.clone()))
                                .await
                                {
                                    log("Failed to continue request: ", e.to_string());
                                }
                    }
                });

                Some(ih)
            }
            _ => None,
        }
    } else {
        None
    }
}

/// Semaphore low priority tasks to run
#[cfg(not(feature = "cowboy"))]
async fn run_task<F, Fut>(
    semaphore: Arc<Semaphore>,
    task: F,
) -> hashbrown::HashSet<CaseInsensitiveString>
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = hashbrown::HashSet<CaseInsensitiveString>> + Send + 'static,
{
    match semaphore.acquire_owned().await {
        Ok(_permit) => task().await,
        _ => {
            task().await;
            Default::default()
        }
    }
}

/// Semaphore low priority tasks to run
#[cfg(feature = "cowboy")]
async fn run_task<F, Fut>(
    _semaphore: Arc<Semaphore>,
    task: F,
) -> hashbrown::HashSet<CaseInsensitiveString>
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = hashbrown::HashSet<CaseInsensitiveString>> + Send + 'static,
{
    task().await
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
    links_visited: Box<HashSet<CaseInsensitiveString>>,
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
    /// The status of the active crawl.
    status: CrawlStatus,
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
        let url = if url.starts_with(' ') || url.ends_with(' ') {
            url.trim()
        } else {
            url
        };
        let url: Box<CaseInsensitiveString> = if url.starts_with("http") {
            CaseInsensitiveString::new(&url).into()
        } else {
            CaseInsensitiveString::new(&string_concat!("https://", url)).into()
        };
        Self {
            configuration: Configuration::new().into(),
            links_visited: Box::new(HashSet::new()),
            pages: None,
            robot_file_parser: None,
            on_link_find_callback: None,
            channel: None,
            status: CrawlStatus::Start,
            shutdown: false,
            domain_parsed: match url::Url::parse(url.inner()) {
                Ok(u) => Some(Box::new(crate::page::convert_abs_path(&u, "/"))),
                _ => None,
            },
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
        self.domain_parsed = match url::Url::parse(domain.inner()) {
            Ok(u) => Some(Box::new(crate::page::convert_abs_path(&u, "/"))),
            _ => None,
        };
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
        } else if self.is_over_budget(link) {
            ProcessLinkStatus::BudgetExceeded
        } else {
            self.is_allowed_default(link.inner())
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
        } else if self.is_over_budget(&link) {
            ProcessLinkStatus::BudgetExceeded
        } else if self
            .is_allowed_default(link)
            .eq(&ProcessLinkStatus::Allowed)
        {
            ProcessLinkStatus::Allowed
        } else {
            ProcessLinkStatus::Blocked
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

        if !whitelist.is_empty() && !contains(&whitelist, link.inner()) {
            ProcessLinkStatus::Blocked
        } else if !blacklist.is_empty() {
            if !contains(&blacklist, link.inner()) {
                ProcessLinkStatus::Allowed
            } else {
                ProcessLinkStatus::Blocked
            }
        } else if self.is_allowed_robots(&link.as_ref()) {
            ProcessLinkStatus::Allowed
        } else {
            ProcessLinkStatus::Blocked
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

        if !whitelist.is_empty() && !contains(&whitelist, link) {
            ProcessLinkStatus::Blocked
        } else if contains(&self.configuration.get_blacklist_compiled(), link) {
            ProcessLinkStatus::Blocked
        } else if self.is_allowed_robots(link) {
            ProcessLinkStatus::Allowed
        } else {
            ProcessLinkStatus::Blocked
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

    /// Validate if url exceeds crawl budget and should not be handled.
    pub fn is_over_budget(&mut self, link: &CaseInsensitiveString) -> bool {
        if self.configuration.inner_budget.is_some() || self.configuration.depth_distance > 0 {
            match Url::parse(link.inner()) {
                Ok(r) => {
                    let has_depth_control = self.configuration.depth_distance > 0;

                    if self.configuration.inner_budget.is_none() {
                        match r.path_segments() {
                            Some(segments) => {
                                let mut over = false;
                                let mut depth: usize = 0;

                                for _ in segments {
                                    if has_depth_control {
                                        depth = depth.saturating_add(1);
                                        if depth >= self.configuration.depth_distance {
                                            over = true;
                                            break;
                                        }
                                    }
                                }

                                over
                            }
                            _ => false,
                        }
                    } else {
                        match self.configuration.inner_budget.as_mut() {
                            Some(budget) => {
                                let exceeded_wild_budget = if self.configuration.wild_card_budgeting
                                {
                                    match budget.get_mut(&*WILD_CARD_PATH) {
                                        Some(budget) => {
                                            if budget.abs_diff(0) == 1 {
                                                true
                                            } else if budget == &0 {
                                                true
                                            } else {
                                                *budget -= 1;
                                                false
                                            }
                                        }
                                        _ => false,
                                    }
                                } else {
                                    false
                                };

                                // set this up prior to crawl to avoid checks per link.
                                // If only the wild card budget is set we can safely skip all checks.
                                let skip_paths =
                                    self.configuration.wild_card_budgeting && budget.len() == 1;

                                // check if paths pass
                                if !skip_paths && !exceeded_wild_budget {
                                    match r.path_segments() {
                                        Some(segments) => {
                                            let mut joint_segment =
                                                CaseInsensitiveString::default();
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
                                                    match budget.get_mut(&joint_segment) {
                                                        Some(budget) => {
                                                            if budget.abs_diff(0) == 0
                                                                || *budget == 0
                                                            {
                                                                over = true;
                                                                break;
                                                            } else {
                                                                *budget -= 1;
                                                                continue;
                                                            }
                                                        }
                                                        _ => (),
                                                    };
                                                }
                                            }

                                            over
                                        }
                                        _ => false,
                                    }
                                } else {
                                    exceeded_wild_budget
                                }
                            }
                            _ => false,
                        }
                    }
                }
                _ => false,
            }
        } else {
            false
        }
    }

    /// Amount of pages crawled.
    pub fn size(&self) -> usize {
        self.links_visited.len()
    }

    /// Drain the links visited.
    pub fn drain_links(&mut self) -> hashbrown::hash_set::Drain<'_, CaseInsensitiveString> {
        self.links_visited.drain()
    }

    /// Drain the extra links used for things like the sitemap.
    pub fn drain_extra_links(&mut self) -> hashbrown::hash_set::Drain<'_, CaseInsensitiveString> {
        self.extra_links.drain()
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
    pub fn get_links(&self) -> &HashSet<CaseInsensitiveString> {
        &self.links_visited
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
    fn get_delay(&self) -> Duration {
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
                Ok(u) => Some(crate::page::convert_abs_path(&u, "/")),
                _ => None,
            }
        } else {
            self.domain_parsed.as_deref().cloned()
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

                match robot_file_parser.get_crawl_delay(&self.configuration.user_agent) {
                    Some(delay) => {
                        // 60 seconds should be the longest to respect for efficiency.
                        self.configuration.delay = delay.as_millis().min(60000) as u64;
                    }
                    _ => (),
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
                let initial_redirect = Arc::new(AtomicU8::new(0));
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
            || self.configuration.chrome_intercept
            || self.configuration.stealth_mode
            || self.configuration.fingerprint
    }

    /// Build the HTTP client.
    #[cfg(all(not(feature = "decentralized"), not(feature = "cache")))]
    fn configure_http_client_builder(&mut self) -> crate::ClientBuilder {
        use reqwest::header::HeaderMap;

        let policy = self.setup_redirect_policy();
        let mut headers = HeaderMap::new();

        let user_agent = match &self.configuration.user_agent {
            Some(ua) => ua.as_str(),
            _ => &get_ua(self.only_chrome_agent()),
        };

        if cfg!(feature = "real_browser") {
            headers.extend(crate::utils::header_utils::get_mimic_headers(user_agent));
        }

        let client = Client::builder()
            .user_agent(user_agent)
            .redirect(policy)
            .danger_accept_invalid_certs(self.configuration.accept_invalid_certs)
            .tcp_keepalive(Duration::from_millis(500))
            .pool_idle_timeout(None);

        let client = if self.configuration.http2_prior_knowledge {
            client.http2_prior_knowledge()
        } else {
            client
        };

        let client =
            crate::utils::header_utils::setup_default_headers(client, &self.configuration, headers);

        let mut client = match &self.configuration.request_timeout {
            Some(t) => client.timeout(**t),
            _ => client,
        };

        let client = match &self.configuration.proxies {
            Some(proxies) => {
                for proxie in proxies.iter() {
                    match reqwest::Proxy::all(proxie) {
                        Ok(proxy) => client = client.proxy(proxy),
                        _ => (),
                    }
                }
                client
            }
            _ => client,
        };

        self.configure_http_client_cookies(client)
    }

    /// Build the HTTP client with caching enabled.
    #[cfg(all(not(feature = "decentralized"), feature = "cache"))]
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
            .tcp_keepalive(Duration::from_millis(500))
            .pool_idle_timeout(None);

        let client = if self.configuration.http2_prior_knowledge {
            client.http2_prior_knowledge()
        } else {
            client
        };

        let client =
            crate::utils::header_utils::setup_default_headers(client, &self.configuration, headers);

        let mut client = match &self.configuration.request_timeout {
            Some(t) => client.timeout(**t),
            _ => client,
        };

        let client = match &self.configuration.proxies {
            Some(proxies) => {
                for proxie in proxies.iter() {
                    match reqwest::Proxy::all(proxie) {
                        Ok(proxy) => client = client.proxy(proxy),
                        _ => (),
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
        let client = if !self.configuration.cookie_str.is_empty() && self.domain_parsed.is_some() {
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
        };
        client
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
    #[cfg(all(not(feature = "decentralized"), not(feature = "cache")))]
    pub fn configure_http_client(&mut self) -> Client {
        let client = self.configure_http_client_builder();
        // should unwrap using native-tls-alpn
        unsafe { client.build().unwrap_unchecked() }
    }

    /// Configure http client.
    #[cfg(all(not(feature = "decentralized"), feature = "cache"))]
    pub fn configure_http_client(&mut self) -> Client {
        let client = self.configure_http_client_builder();
        client.build()
    }

    /// Configure http client for decentralization.
    #[cfg(all(feature = "decentralized", not(feature = "cache")))]
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
            .tcp_keepalive(Duration::from_millis(500))
            .pool_idle_timeout(None);

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
            Some(h) => headers.extend(*h.to_owned()),
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
            match reqwest::Proxy::all(worker) {
                Ok(worker) => {
                    client = client.proxy(worker);
                }
                _ => (),
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
    #[cfg(all(feature = "decentralized", feature = "cache"))]
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
            .tcp_keepalive(Duration::from_millis(500))
            .pool_idle_timeout(None);

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
            Some(h) => headers.extend(*h.to_owned()),
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
            match reqwest::Proxy::all(worker) {
                Ok(worker) => {
                    client = client.proxy(worker);
                }
                _ => (),
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

        let join_handle = tokio::spawn(async move {
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
        setup_chrome_interception_base(
            page,
            self.configuration.chrome_intercept,
            &self.configuration.auth_challenge_response,
            self.configuration.chrome_intercept_block_visuals,
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
    fn setup_selectors(&self) -> Option<(CompactString, smallvec::SmallVec<[CompactString; 2]>)> {
        get_page_selectors(
            self.url.inner(),
            self.configuration.subdomains,
            self.configuration.tld,
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
        base: &mut (CompactString, smallvec::SmallVec<[CompactString; 2]>),
        _: bool,
        scrape: bool,
    ) -> HashSet<CaseInsensitiveString> {
        if self
            .is_allowed_default(self.get_base_link())
            .eq(&ProcessLinkStatus::Allowed)
        {
            let url = self.url.inner();
            let mut page = Page::new_page(url, client).await;
            log("fetch", &url);

            // allow initial page mutation
            match page.final_redirect_destination.as_deref() {
                Some(domain) => {
                    self.domain_parsed = match url::Url::parse(domain) {
                        Ok(u) => Some(Box::new(crate::page::convert_abs_path(&u, "/"))),
                        _ => None,
                    };
                    self.url = Box::new(domain.into());
                    match self.setup_selectors() {
                        Some(s) => {
                            base.0 = s.0;
                            base.1 = s.1;
                        }
                        _ => (),
                    }
                }
                _ => (),
            };

            let links = if !page.is_empty() {
                self.links_visited.insert(match self.on_link_find_callback {
                    Some(cb) => {
                        let c = cb(*self.url.clone(), None);
                        c.0
                    }
                    _ => *self.url.clone(),
                });
                page.detect_language();
                page.links(base).await
            } else {
                self.status = CrawlStatus::Empty;
                Default::default()
            };

            if scrape {
                match self.pages.as_mut() {
                    Some(p) => p.push(page.clone()),
                    _ => (),
                };
            }

            if page.status_code == reqwest::StatusCode::FORBIDDEN && links.len() == 0 {
                self.status = CrawlStatus::Blocked;
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
        }
    }

    /// Expand links for crawl.
    #[cfg(all(not(feature = "decentralized"), feature = "chrome"))]
    async fn crawl_establish(
        &mut self,
        client: &Client,
        base: &mut (CompactString, smallvec::SmallVec<[CompactString; 2]>),
        _: bool,
        chrome_page: &chromiumoxide::Page,
        scrape: bool,
    ) -> HashSet<CaseInsensitiveString> {
        if self
            .is_allowed_default(&self.get_base_link())
            .eq(&ProcessLinkStatus::Allowed)
        {
            if cfg!(feature = "chrome_stealth") || self.configuration.stealth_mode {
                match self.configuration.user_agent.as_ref() {
                    Some(agent) => {
                        let _ = chrome_page.enable_stealth_mode_with_agent(agent).await;
                    }
                    _ => {
                        let _ = chrome_page.enable_stealth_mode().await;
                    }
                }
            }

            match self.configuration.evaluate_on_new_document {
                Some(ref script) => {
                    if self.configuration.fingerprint {
                        let _ = chrome_page
                            .evaluate_on_new_document(string_concat!(
                                crate::features::chrome::FP_JS,
                                script.as_str()
                            ))
                            .await;
                    } else {
                        let _ = chrome_page.evaluate_on_new_document(script.as_str()).await;
                    }
                }
                _ => {
                    if self.configuration.fingerprint {
                        let _ = chrome_page
                            .evaluate_on_new_document(crate::features::chrome::FP_JS)
                            .await;
                    }
                }
            }

            let _ = self.setup_chrome_interception(&chrome_page).await;

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
            )
            .await;

            match page.final_redirect_destination {
                Some(ref domain) => {
                    let domain: Box<CaseInsensitiveString> =
                        CaseInsensitiveString::new(&domain).into();
                    self.domain_parsed = match url::Url::parse(&domain.inner()) {
                        Ok(u) => Some(Box::new(crate::page::convert_abs_path(&u, "/"))),
                        _ => None,
                    };
                    self.url = domain;
                    match self.setup_selectors() {
                        Some(s) => {
                            base.0 = s.0;
                            base.1 = s.1;
                        }
                        _ => (),
                    }
                }
                _ => (),
            }

            let links = if !page.is_empty() {
                self.links_visited.insert(match self.on_link_find_callback {
                    Some(cb) => {
                        let c = cb(*self.url.clone(), None);

                        c.0
                    }
                    _ => *self.url.clone(),
                });
                page.detect_language();

                let links = HashSet::from(page.links(&base).await);

                links
            } else {
                self.status = CrawlStatus::Empty;
                Default::default()
            };

            if page.status_code == reqwest::StatusCode::FORBIDDEN && links.len() == 0 {
                self.status = CrawlStatus::Blocked;
            }

            if scrape {
                match self.pages.as_mut() {
                    Some(p) => p.push(page.clone()),
                    _ => (),
                };
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
        base: &mut (CompactString, smallvec::SmallVec<[CompactString; 2]>),
        _: bool,
        browser: &Arc<chromiumoxide::Browser>,
        scrape: bool,
    ) -> HashSet<CaseInsensitiveString> {
        let links: HashSet<CaseInsensitiveString> = if self
            .is_allowed_default(&self.get_base_link())
            .eq(&ProcessLinkStatus::Allowed)
        {
            let mut page = Page::new_page(&self.url.inner(), &client).await;

            let page_links: HashSet<CaseInsensitiveString> =
                page.smart_links(&base, &browser, &self.configuration).await;

            match page.final_redirect_destination {
                Some(ref domain) => {
                    let domain: Box<CaseInsensitiveString> =
                        CaseInsensitiveString::new(&domain).into();
                    self.domain_parsed = match url::Url::parse(&domain.inner()) {
                        Ok(u) => Some(Box::new(crate::page::convert_abs_path(&u, "/"))),
                        _ => None,
                    };
                    self.url = domain;
                    match self.setup_selectors() {
                        Some(s) => {
                            base.0 = s.0;
                            base.1 = s.1;
                        }
                        _ => (),
                    }
                }
                _ => (),
            }

            let links = if !page_links.is_empty() {
                self.links_visited.insert(match self.on_link_find_callback {
                    Some(cb) => {
                        let c = cb(*self.url.clone(), None);

                        c.0
                    }
                    _ => *self.url.clone(),
                });

                page_links
            } else {
                self.status = CrawlStatus::Empty;
                Default::default()
            };

            if page.status_code == reqwest::StatusCode::FORBIDDEN && links.len() == 0 {
                self.status = CrawlStatus::Blocked;
            }

            if scrape {
                match self.pages.as_mut() {
                    Some(p) => p.push(page.clone()),
                    _ => (),
                };
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
        scrape: bool,
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

            if page.status_code == reqwest::StatusCode::FORBIDDEN && page.links.len() == 0 {
                self.status = CrawlStatus::Blocked;
            }

            let links = HashSet::from(page.links.clone());

            if scrape {
                match self.pages.as_mut() {
                    Some(p) => p.push(page.clone()),
                    _ => (),
                };
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
    #[cfg(all(feature = "glob", feature = "decentralized"))]
    async fn crawl_establish(
        &mut self,
        client: &Client,
        _: &(CompactString, smallvec::SmallVec<[CompactString; 2]>),
        http_worker: bool,
        scrape: bool,
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

            if scrape {
                match self.pages.as_mut() {
                    Some(p) => p.push(page.clone()),
                    _ => (),
                };
            }

            if self.configuration.return_page_links {
                page.page_links = if links.is_empty() {
                    None
                } else {
                    Some(Box::new(links.clone()))
                };
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
        scrape: bool,
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

            if scrape {
                match self.pages.as_mut() {
                    Some(p) => p.push(page.clone()),
                    _ => (),
                };
            }

            page.detect_language();

            if self.configuration.return_page_links {
                let links = HashSet::from(page.links(&base).await);

                page.page_links = if links.is_empty() {
                    None
                } else {
                    Some(Box::new(links.clone()))
                };

                channel_send_page(&self.channel, page.clone(), &self.channel_guard);

                links.extend(links);
            } else {
                channel_send_page(&self.channel, page.clone(), &self.channel_guard);

                let links = HashSet::from(page.links(&base).await);

                links.extend(links);
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
        scrape: bool,
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

            match page.final_redirect_destination {
                Some(ref domain) => {
                    let domain: Box<CaseInsensitiveString> =
                        CaseInsensitiveString::new(&domain).into();
                    self.domain_parsed = match url::Url::parse(&domain.inner()) {
                        Ok(u) => Some(Box::new(crate::page::convert_abs_path(&u, "/"))),
                        _ => None,
                    };
                    self.url = domain;
                    match self.setup_selectors() {
                        Some(s) => {
                            base.0 = s.0;
                            base.1 = s.1;
                        }
                        _ => (),
                    }
                }
                _ => (),
            }

            if !page.is_empty() {
                page.detect_language();

                let u = page.get_url().into();
                let link_result = match self.on_link_find_callback {
                    Some(cb) => cb(u, None),
                    _ => (u, None),
                };

                self.links_visited.insert(link_result.0);
                let page_links = HashSet::from(page.links(&base).await);

                links.extend(page_links);
            } else {
                self.status = CrawlStatus::Empty;
            };

            if scrape {
                match self.pages.as_mut() {
                    Some(p) => p.push(page.clone()),
                    _ => (),
                };
            }

            if self.configuration.return_page_links {
                page.page_links = if links.is_empty() {
                    None
                } else {
                    Some(Box::new(links.clone()))
                };
            }

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

        tokio::spawn(async move {
            w.crawl().await;
        });

        match self.pages.as_mut() {
            Some(p) => {
                while let Ok(res) = rx2.recv().await {
                    self.links_visited.insert(res.get_url().into());
                    p.push(res);
                }
            }
            _ => (),
        };
    }

    /// Start to crawl website with async concurrency using the base raw functionality. Useful when using the "chrome" feature and defaulting to the basic implementation.
    pub async fn scrape_raw(&mut self) {
        let mut w = self.clone();
        let mut rx2 = w.subscribe(0).expect("receiver enabled");

        if self.pages.is_none() {
            self.pages = Some(Box::new(Vec::new()));
        }

        tokio::spawn(async move {
            w.crawl_raw().await;
        });

        match self.pages.as_mut() {
            Some(p) => {
                while let Ok(res) = rx2.recv().await {
                    self.links_visited.insert(res.get_url().into());
                    p.push(res);
                }
            }
            _ => (),
        };
    }

    /// Start to scrape website with async concurrency smart. Use HTTP first and JavaScript Rendering as needed. This has no effect without the `smart` flag enabled.
    pub async fn scrape_smart(&mut self) {
        let mut w = self.clone();
        let mut rx2 = w.subscribe(0).expect("receiver enabled");

        if self.pages.is_none() {
            self.pages = Some(Box::new(Vec::new()));
        }

        tokio::spawn(async move {
            w.crawl_smart().await;
        });

        match self.pages.as_mut() {
            Some(p) => {
                while let Ok(res) = rx2.recv().await {
                    self.links_visited.insert(res.get_url().into());
                    p.push(res);
                }
            }
            _ => (),
        };
    }

    /// Start to scrape website sitemap with async concurrency. Use HTTP first and JavaScript Rendering as needed. This has no effect without the `sitemap` flag enabled.
    pub async fn scrape_sitemap(&mut self) {
        let mut w = self.clone();
        let mut rx2 = w.subscribe(0).expect("receiver enabled");

        if self.pages.is_none() {
            self.pages = Some(Box::new(Vec::new()));
        }

        tokio::spawn(async move {
            w.crawl_sitemap().await;
        });

        match self.pages.as_mut() {
            Some(p) => {
                while let Ok(res) = rx2.recv().await {
                    self.links_visited.insert(res.get_url().into());
                    p.push(res);
                }
            }
            _ => (),
        };
    }

    /// Start to crawl website concurrently - used mainly for chrome instances to connect to default raw HTTP.
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
                    self._crawl_establish(client, &mut selector, false, false)
                        .await;
                } else {
                    let mut links: HashSet<CaseInsensitiveString> =
                        self.drain_extra_links().collect();
                    let (mut interval, throttle) = self.setup_crawl();
                    let semaphore = if self.configuration.shared_queue {
                        SEM_SHARED.clone()
                    } else {
                        Arc::new(Semaphore::const_new(*DEFAULT_PERMITS))
                    };

                    links.extend(
                        self._crawl_establish(client, &mut selector, false, false)
                            .await,
                    );
                    self.configuration.configure_allowlist();
                    let on_link_find_callback = self.on_link_find_callback;
                    let full_resources = self.configuration.full_resources;
                    let return_page_links = self.configuration.return_page_links;
                    let mut q = match &self.channel_queue {
                        Some(q) => Some(q.0.subscribe()),
                        _ => None,
                    };

                    let shared = Arc::new((
                        client.to_owned(),
                        selector,
                        self.channel.clone(),
                        self.configuration.external_domains_caseless.clone(),
                        self.channel_guard.clone(),
                    ));

                    let mut set: JoinSet<HashSet<CaseInsensitiveString>> = JoinSet::new();
                    let chandle = Handle::current();

                    while !links.is_empty() {
                        loop {
                            let stream = tokio_stream::iter::<HashSet<CaseInsensitiveString>>(
                                links.drain().collect(),
                            )
                            .throttle(*throttle);

                            tokio::pin!(stream);

                            loop {
                                match stream.next().await {
                                    Some(link) => {
                                        if !self
                                            .handle_process(handle, &mut interval, set.shutdown())
                                            .await
                                        {
                                            break;
                                        }

                                        let allowed = self.is_allowed(&link);

                                        if allowed.eq(&ProcessLinkStatus::BudgetExceeded) {
                                            break;
                                        }
                                        if allowed.eq(&ProcessLinkStatus::Blocked) {
                                            continue;
                                        }

                                        log("fetch", &link);
                                        self.links_visited.insert(link.clone());

                                        let shared = shared.clone();
                                        let semaphore = semaphore.clone();

                                        set.spawn_on(
                                            run_task(semaphore, move || async move {
                                                let link_result = match on_link_find_callback {
                                                    Some(cb) => cb(link, None),
                                                    _ => (link, None),
                                                };
                                                let mut page = Page::new_page(
                                                    link_result.0.as_ref(),
                                                    &shared.0,
                                                )
                                                .await;
                                                page.set_external(shared.3.to_owned());
                                                page.detect_language();

                                                let links = if full_resources {
                                                    page.links_full(&shared.1).await
                                                } else {
                                                    page.links(&shared.1).await
                                                };

                                                if return_page_links {
                                                    page.page_links = if links.is_empty() {
                                                        None
                                                    } else {
                                                        Some(Box::new(links.clone()))
                                                    };
                                                }

                                                channel_send_page(&shared.2, page, &shared.4);

                                                links
                                            }),
                                            &chandle,
                                        );

                                        match q.as_mut() {
                                            Some(q) => {
                                                while let Ok(link) = q.try_recv() {
                                                    let s = link.into();
                                                    let allowed = self.is_allowed(&s);

                                                    if allowed
                                                        .eq(&ProcessLinkStatus::BudgetExceeded)
                                                    {
                                                        break;
                                                    }
                                                    if allowed.eq(&ProcessLinkStatus::Blocked) {
                                                        continue;
                                                    }
                                                    links.extend(
                                                        &HashSet::from([s]) - &self.links_visited,
                                                    );
                                                }
                                            }
                                            _ => (),
                                        }
                                    }
                                    _ => break,
                                }
                            }

                            while let Some(res) = set.join_next().await {
                                match res {
                                    Ok(msg) => links.extend(&msg - &self.links_visited),
                                    _ => (),
                                };
                            }

                            if links.is_empty() {
                                break;
                            }
                        }

                        self.subscription_guard();
                    }
                }
            }
            _ => log("", INVALID_URL),
        }
    }

    /// Start to crawl website concurrently.
    #[cfg(all(
        not(feature = "decentralized"),
        not(feature = "chrome_intercept"),
        feature = "chrome",
    ))]
    async fn crawl_concurrent(&mut self, client: &Client, handle: &Option<Arc<AtomicI8>>) {
        use crate::features::chrome::attempt_navigation;

        self.start();
        match self.setup_selectors() {
            Some(mut selectors) => match self.setup_browser().await {
                Some((browser, browser_handle, mut context_id)) => match attempt_navigation(
                    "about:blank",
                    &browser,
                    &self.configuration.request_timeout,
                )
                .await
                {
                    Ok(new_page) => {
                        let new_page = configure_browser(new_page, &self.configuration).await;
                        let semaphore = if self.configuration.shared_queue {
                            SEM_SHARED.clone()
                        } else {
                            Arc::new(Semaphore::const_new(*DEFAULT_PERMITS))
                        };
                        let mut q = match &self.channel_queue {
                            Some(q) => Some(q.0.subscribe()),
                            _ => None,
                        };

                        if match self.configuration.inner_budget {
                            Some(ref b) => match b.get(&*WILD_CARD_PATH) {
                                Some(b) => b.eq(&1),
                                _ => false,
                            },
                            _ => false,
                        } {
                            self.status = CrawlStatus::Active;
                            self.crawl_establish(&client, &mut selectors, false, &new_page, false)
                                .await;
                            self.subscription_guard();
                            crate::features::chrome::close_browser(
                                browser_handle,
                                &browser,
                                &mut context_id,
                            )
                            .await;
                        } else {
                            let mut links: HashSet<CaseInsensitiveString> =
                                self.drain_extra_links().collect();

                            let (mut interval, throttle) = self.setup_crawl();

                            links.extend(
                                self.crawl_establish(
                                    &client,
                                    &mut selectors,
                                    false,
                                    &new_page,
                                    false,
                                )
                                .await,
                            );

                            let mut set: JoinSet<HashSet<CaseInsensitiveString>> = JoinSet::new();
                            let chandle = Handle::current();

                            let shared = Arc::new((
                                client.to_owned(),
                                selectors,
                                self.channel.clone(),
                                self.channel_guard.clone(),
                                browser,
                                self.configuration.clone(), // we may just want to share explicit config instead.
                            ));

                            let add_external =
                                self.configuration.external_domains_caseless.len() > 0;
                            self.configuration.configure_allowlist();
                            let on_link_find_callback = self.on_link_find_callback;
                            let full_resources = self.configuration.full_resources;
                            let return_page_links = self.configuration.return_page_links;

                            while !links.is_empty() {
                                loop {
                                    let stream =
                                        tokio_stream::iter::<HashSet<CaseInsensitiveString>>(
                                            links.drain().collect(),
                                        )
                                        .throttle(*throttle);
                                    tokio::pin!(stream);

                                    loop {
                                        match stream.next().await {
                                            Some(link) => {
                                                if !self
                                                    .handle_process(
                                                        handle,
                                                        &mut interval,
                                                        set.shutdown(),
                                                    )
                                                    .await
                                                {
                                                    break;
                                                }

                                                let allowed = self.is_allowed(&link);

                                                if allowed.eq(&ProcessLinkStatus::BudgetExceeded) {
                                                    break;
                                                }
                                                if allowed.eq(&ProcessLinkStatus::Blocked) {
                                                    continue;
                                                }

                                                log("fetch", &link);
                                                self.links_visited.insert(link.clone());

                                                let shared = shared.clone();

                                                set.spawn_on(
                                                    run_task(
                                                        semaphore.clone(),
                                                        move || async move {
                                                            let link_result = match on_link_find_callback {
                                                                Some(cb) => cb(link, None),
                                                                _ => (link, None),
                                                            };
                                                            let target_url = link_result.0.as_ref();
                                                            let next = match attempt_navigation(target_url, &shared.4, &shared.5.request_timeout).await {
                                                                Ok(new_page) => {

                                                                    match shared.5.evaluate_on_new_document
                                                                    {
                                                                        Some(ref script) => {
                                                                            let _ = new_page
                                                                                .evaluate_on_new_document(
                                                                                    script.as_str(),
                                                                                )
                                                                                .await;
                                                                        }
                                                                        _ => (),
                                                                    }

                                                                    let new_page = configure_browser(
                                                                        new_page, &shared.5,
                                                                    )
                                                                    .await;

                                                                    if cfg!(feature = "chrome_stealth")
                                                                        || shared.5.stealth_mode
                                                                    {
                                                                        match shared.5.user_agent.as_ref() {
                                                                            Some(agent) => {
                                                                                let _ = new_page.enable_stealth_mode_with_agent(agent).await;
                                                                            },
                                                                            _ => {
                                                                                let _ = new_page.enable_stealth_mode().await;
                                                                            },
                                                                        }
                                                                    }

                                                                    let mut page = Page::new(
                                                                        &target_url,
                                                                        &shared.0,
                                                                        &new_page,
                                                                        &shared.5.wait_for,
                                                                        &shared.5.screenshot,
                                                                        true,
                                                                        &shared.5.openai_config,
                                                                        &shared.5.execution_scripts,
                                                                        &shared.5.automation_scripts,
                                                                    )
                                                                    .await;

                                                                    if add_external {
                                                                        page.set_external(
                                                                            shared
                                                                                .5
                                                                                .external_domains_caseless
                                                                                .clone(),
                                                                        );
                                                                    }
                                                                    page.detect_language();

                                                                    let links = if full_resources {
                                                                        page.links_full(&shared.1).await
                                                                    } else {
                                                                        page.links(&shared.1).await
                                                                    };

                                                                    if return_page_links {
                                                                        page.page_links = if links.is_empty() {
                                                                            None
                                                                        } else {
                                                                            Some(Box::new(links.clone()))
                                                                        };
                                                                    }

                                                                    channel_send_page(
                                                                        &shared.2, page, &shared.3,
                                                                    );

                                                                    links
                                                                }
                                                                _ => Default::default(),
                                                            };

                                                            next
                                                        },
                                                    ),
                                                    &chandle,
                                                );

                                                match q.as_mut() {
                                                    Some(q) => {
                                                        while let Ok(link) = q.try_recv() {
                                                            let s = link.into();
                                                            let allowed = self.is_allowed(&s);

                                                            if allowed.eq(
                                                                &ProcessLinkStatus::BudgetExceeded,
                                                            ) {
                                                                break;
                                                            }
                                                            if allowed
                                                                .eq(&ProcessLinkStatus::Blocked)
                                                            {
                                                                continue;
                                                            }
                                                            links.extend(
                                                                &HashSet::from([s])
                                                                    - &self.links_visited,
                                                            );
                                                        }
                                                    }
                                                    _ => (),
                                                }
                                            }
                                            _ => break,
                                        }
                                    }

                                    while let Some(res) = set.join_next().await {
                                        match res {
                                            Ok(msg) => links.extend(&msg - &self.links_visited),
                                            Err(e) => {
                                                if set.is_empty() {
                                                    break;
                                                } else {
                                                    if e.is_panic() {
                                                        set.shutdown().await;
                                                    }
                                                    continue;
                                                }
                                            }
                                        };
                                    }

                                    if links.is_empty() {
                                        break;
                                    }
                                }

                                self.subscription_guard();
                            }
                            crate::features::chrome::close_browser(
                                browser_handle,
                                &shared.4,
                                &mut context_id,
                            )
                            .await;
                        }
                    }
                    _ => log("", "Chrome failed to open page."),
                },
                _ => log("", "Chrome failed to start."),
            },
            _ => log("", INVALID_URL),
        }
    }

    /// Start to crawl website concurrently.
    #[cfg(all(
        not(feature = "decentralized"),
        feature = "chrome",
        feature = "chrome_intercept"
    ))]
    async fn crawl_concurrent(&mut self, client: &Client, handle: &Option<Arc<AtomicI8>>) {
        use crate::features::chrome::attempt_navigation;

        self.start();
        match self.setup_selectors() {
            Some(mut selectors) => match self.setup_browser().await {
                Some((browser, browser_handle, mut context_id)) => match attempt_navigation(
                    "about:blank",
                    &browser,
                    &self.configuration.request_timeout,
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
                            self.crawl_establish(&client, &mut selectors, false, &new_page, false)
                                .await;
                            self.subscription_guard();
                            crate::features::chrome::close_browser(
                                browser_handle,
                                &browser,
                                &mut context_id,
                            )
                            .await;
                        } else {
                            let semaphore = if self.configuration.shared_queue {
                                SEM_SHARED.clone()
                            } else {
                                Arc::new(Semaphore::const_new(*DEFAULT_PERMITS))
                            };
                            let new_page = configure_browser(new_page, &self.configuration).await;
                            let mut q = match &self.channel_queue {
                                Some(q) => Some(q.0.subscribe()),
                                _ => None,
                            };
                            let mut links: HashSet<CaseInsensitiveString> =
                                self.drain_extra_links().collect();

                            let (mut interval, throttle) = self.setup_crawl();

                            links.extend(
                                self.crawl_establish(
                                    &client,
                                    &mut selectors,
                                    false,
                                    &new_page,
                                    false,
                                )
                                .await,
                            );
                            let mut set: JoinSet<HashSet<CaseInsensitiveString>> = JoinSet::new();
                            let chandle = Handle::current();

                            let shared = Arc::new((
                                client.to_owned(),
                                selectors,
                                self.channel.clone(),
                                self.configuration.external_domains_caseless.clone(),
                                self.channel_guard.clone(),
                                browser,
                                self.configuration.clone(),
                                self.url.inner().to_string(),
                            ));

                            let add_external = shared.3.len() > 0;
                            self.configuration.configure_allowlist();
                            let on_link_find_callback = self.on_link_find_callback;
                            let full_resources = self.configuration.full_resources;
                            let return_page_links = self.configuration.return_page_links;

                            while !links.is_empty() {
                                loop {
                                    let stream =
                                        tokio_stream::iter::<HashSet<CaseInsensitiveString>>(
                                            links.drain().collect(),
                                        )
                                        .throttle(*throttle);
                                    tokio::pin!(stream);

                                    loop {
                                        match stream.next().await {
                                            Some(link) => {
                                                if !self
                                                    .handle_process(
                                                        handle,
                                                        &mut interval,
                                                        set.shutdown(),
                                                    )
                                                    .await
                                                {
                                                    break;
                                                }

                                                let allowed = self.is_allowed(&link);

                                                if allowed.eq(&ProcessLinkStatus::BudgetExceeded) {
                                                    break;
                                                }
                                                if allowed.eq(&ProcessLinkStatus::Blocked) {
                                                    continue;
                                                }

                                                log("fetch", &link);
                                                self.links_visited.insert(link.clone());

                                                let shared = shared.clone();

                                                set.spawn_on(
                                                    run_task(semaphore.clone(), move || async move {
                                                        match attempt_navigation("about:blank", &shared.5, &shared.6.request_timeout).await {
                                                            Ok(new_page) => {
                                                                let _ = setup_chrome_interception_base(
                                                                    &new_page,
                                                                    shared.6.chrome_intercept,
                                                                    &shared.6.auth_challenge_response,
                                                                    shared.6.chrome_intercept_block_visuals,
                                                                    &shared.7
                                                                )
                                                                .await;

                                                                let link_result =
                                                                    match on_link_find_callback {
                                                                        Some(cb) => cb(link, None),
                                                                        _ => (link, None),
                                                                    };

                                                                let target_url = link_result.0.as_ref();

                                                                let new_page = configure_browser(
                                                                    new_page, &shared.6,
                                                                )
                                                                .await;

                                                                if cfg!(feature = "chrome_stealth")
                                                                    || shared.6.stealth_mode
                                                                {
                                                                    match shared.6.user_agent.as_ref() {
                                                                        Some(agent) => {
                                                                            let _ = new_page.enable_stealth_mode_with_agent(agent).await;
                                                                        },
                                                                        _ => {
                                                                            let _ = new_page.enable_stealth_mode().await;
                                                                        },
                                                                    }
                                                                }

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
                                                                )
                                                                .await;

                                                                if add_external {
                                                                    page.set_external(shared.3.clone());
                                                                }
                                                                page.detect_language();

                                                                let links = if full_resources {
                                                                    page.links_full(&shared.1).await
                                                                } else {
                                                                    page.links(&shared.1).await
                                                                };

                                                                if return_page_links {
                                                                    page.page_links = if links.is_empty() {
                                                                        None
                                                                    } else {
                                                                        Some(Box::new(links.clone()))
                                                                    };
                                                                }

                                                                channel_send_page(
                                                                    &shared.2, page, &shared.4,
                                                                );

                                                                links
                                                            }
                                                            _ => Default::default(),
                                                        }
                                                        }),
                                                    &chandle,
                                                );

                                                match q.as_mut() {
                                                    Some(q) => {
                                                        while let Ok(link) = q.try_recv() {
                                                            let s = link.into();
                                                            let allowed = self.is_allowed(&s);

                                                            if allowed.eq(
                                                                &ProcessLinkStatus::BudgetExceeded,
                                                            ) {
                                                                break;
                                                            }
                                                            if allowed
                                                                .eq(&ProcessLinkStatus::Blocked)
                                                            {
                                                                continue;
                                                            }
                                                            links.extend(
                                                                &HashSet::from([s])
                                                                    - &self.links_visited,
                                                            );
                                                        }
                                                    }
                                                    _ => (),
                                                }
                                            }
                                            _ => break,
                                        }
                                    }

                                    while let Some(res) = set.join_next().await {
                                        match res {
                                            Ok(msg) => links.extend(&msg - &self.links_visited),
                                            _ => (),
                                        };
                                    }

                                    if links.is_empty() {
                                        break;
                                    }
                                }
                                self.subscription_guard();
                            }

                            crate::features::chrome::close_browser(
                                browser_handle,
                                &shared.5,
                                &mut context_id,
                            )
                            .await;
                        }
                    }
                    _ => log("", "Chrome failed to open page."),
                },
                _ => log("", "Chrome failed to start."),
            },
            _ => log("", INVALID_URL),
        }
    }

    /// Start to crawl website concurrently.
    #[cfg(all(not(feature = "decentralized"), not(feature = "chrome")))]
    async fn crawl_concurrent(&mut self, client: &Client, handle: &Option<Arc<AtomicI8>>) {
        self.crawl_concurrent_raw(client, handle).await
    }

    /// Start to crawl website concurrently.
    #[cfg(feature = "decentralized")]
    async fn crawl_concurrent(&mut self, client: &Client, handle: &Option<Arc<AtomicI8>>) {
        match url::Url::parse(&self.url.inner()) {
            Ok(_) => {
                let mut q = match &self.channel_queue {
                    Some(q) => Some(q.0.subscribe()),
                    _ => None,
                };
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
                        false,
                    )
                    .await;

                let mut set: JoinSet<HashSet<CaseInsensitiveString>> = JoinSet::new();
                let chandle = Handle::current();

                loop {
                    let stream = tokio_stream::iter::<HashSet<CaseInsensitiveString>>(
                        links.drain().collect(),
                    )
                    .throttle(*throttle);
                    tokio::pin!(stream);

                    loop {
                        match stream.next().await {
                            Some(link) => {
                                if !self
                                    .handle_process(handle, &mut interval, set.shutdown())
                                    .await
                                {
                                    break;
                                }

                                let allowed = self.is_allowed(&link);

                                if allowed.eq(&ProcessLinkStatus::BudgetExceeded) {
                                    break;
                                }
                                if allowed.eq(&ProcessLinkStatus::Blocked) {
                                    continue;
                                }

                                log("fetch", &link);

                                self.links_visited.insert(link.clone());

                                match SEM.acquire().await {
                                    Ok(permit) => {
                                        let client = client.clone();
                                        tokio::task::yield_now().await;

                                        set.spawn_on(
                                            async move {
                                                let link_results = match on_link_find_callback {
                                                    Some(cb) => cb(link, None),
                                                    _ => (link, None),
                                                };
                                                let link_results = link_results.0.as_ref();
                                                let page = Page::new_links_only(
                                                    &if http_worker
                                                        && link_results.starts_with("https")
                                                    {
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
                                            },
                                            &chandle,
                                        );

                                        match q.as_mut() {
                                            Some(q) => {
                                                while let Ok(link) = q.try_recv() {
                                                    let s = link.into();
                                                    let allowed = self.is_allowed(&s);

                                                    if allowed
                                                        .eq(&ProcessLinkStatus::BudgetExceeded)
                                                    {
                                                        break;
                                                    }
                                                    if allowed.eq(&ProcessLinkStatus::Blocked) {
                                                        continue;
                                                    }
                                                    links.extend(
                                                        &HashSet::from([s]) - &self.links_visited,
                                                    );
                                                }
                                            }
                                            _ => (),
                                        }
                                    }
                                    _ => (),
                                }
                            }
                            _ => break,
                        }
                    }

                    while let Some(res) = set.join_next().await {
                        match res {
                            Ok(msg) => {
                                links.extend(&msg - &self.links_visited);
                            }
                            _ => (),
                        };
                    }

                    if links.is_empty() {
                        break;
                    }
                }
            }
            _ => (),
        }
    }

    /// Start to crawl website concurrently using HTTP by default and chrome Javascript Rendering as needed. The glob feature does not work with this at the moment.
    #[cfg(all(not(feature = "decentralized"), feature = "smart"))]
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
                        self.crawl_establish_smart(&client, &mut selectors, false, &browser, false)
                            .await;
                        self.subscription_guard();
                        crate::features::chrome::close_browser(
                            browser_handle,
                            &browser,
                            &mut context_id,
                        )
                        .await;
                    } else {
                        let mut q = match &self.channel_queue {
                            Some(q) => Some(q.0.subscribe()),
                            _ => None,
                        };
                        let mut links: HashSet<CaseInsensitiveString> =
                            self.drain_extra_links().collect();

                        let (mut interval, throttle) = self.setup_crawl();
                        self.configuration.configure_allowlist();
                        let on_link_find_callback = self.on_link_find_callback;
                        let return_page_links = self.configuration.return_page_links;

                        let semaphore = if self.configuration.shared_queue {
                            SEM_SHARED.clone()
                        } else {
                            Arc::new(Semaphore::const_new(*DEFAULT_PERMITS))
                        };
                        links.extend(
                            self.crawl_establish_smart(
                                &client,
                                &mut selectors,
                                false,
                                &browser,
                                false,
                            )
                            .await,
                        );

                        let mut set: JoinSet<HashSet<CaseInsensitiveString>> = JoinSet::new();
                        let chandle = Handle::current();

                        let shared = Arc::new((
                            client.to_owned(),
                            selectors,
                            self.channel.clone(),
                            self.channel_guard.clone(),
                            browser,
                            self.configuration.clone(),
                        ));

                        let add_external = self.configuration.external_domains_caseless.len() > 0;

                        while !links.is_empty() {
                            loop {
                                let stream = tokio_stream::iter::<HashSet<CaseInsensitiveString>>(
                                    links.drain().collect(),
                                )
                                .throttle(*throttle);
                                tokio::pin!(stream);

                                loop {
                                    match stream.next().await {
                                        Some(link) => {
                                            if !self
                                                .handle_process(
                                                    handle,
                                                    &mut interval,
                                                    set.shutdown(),
                                                )
                                                .await
                                            {
                                                break;
                                            }

                                            let allowed = self.is_allowed(&link);

                                            if allowed.eq(&ProcessLinkStatus::BudgetExceeded) {
                                                break;
                                            }
                                            if allowed.eq(&ProcessLinkStatus::Blocked) {
                                                continue;
                                            }

                                            log("fetch", &link);
                                            self.links_visited.insert(link.clone());
                                            let shared = shared.clone();

                                            set.spawn_on(
                                                run_task(semaphore.clone(), move || async move {
                                                    let link_result = match on_link_find_callback {
                                                        Some(cb) => cb(link, None),
                                                        _ => (link, None),
                                                    };

                                                    let mut page = Page::new_page(
                                                        &link_result.0.as_ref(),
                                                        &shared.0,
                                                    )
                                                    .await;

                                                    if add_external {
                                                        page.set_external(
                                                            shared
                                                                .5
                                                                .external_domains_caseless
                                                                .clone(),
                                                        );
                                                    }

                                                    let links = page
                                                        .smart_links(
                                                            &shared.1, &shared.4, &shared.5,
                                                        )
                                                        .await;

                                                    if return_page_links {
                                                        page.page_links = if links.is_empty() {
                                                            None
                                                        } else {
                                                            Some(Box::new(links.clone()))
                                                        };
                                                    }

                                                    channel_send_page(&shared.2, page, &shared.3);

                                                    links
                                                }),
                                                &chandle,
                                            );

                                            match q.as_mut() {
                                                Some(q) => {
                                                    while let Ok(link) = q.try_recv() {
                                                        let s = link.into();
                                                        let allowed = self.is_allowed(&s);

                                                        if allowed
                                                            .eq(&ProcessLinkStatus::BudgetExceeded)
                                                        {
                                                            break;
                                                        }
                                                        if allowed.eq(&ProcessLinkStatus::Blocked) {
                                                            continue;
                                                        }
                                                        links.extend(
                                                            &HashSet::from([s])
                                                                - &self.links_visited,
                                                        );
                                                    }
                                                }
                                                _ => (),
                                            }
                                        }
                                        _ => break,
                                    }
                                }

                                while let Some(res) = set.join_next().await {
                                    match res {
                                        Ok(msg) => links.extend(&msg - &self.links_visited),
                                        _ => (),
                                    };
                                }

                                if links.is_empty() {
                                    break;
                                }
                            }

                            self.subscription_guard();
                        }
                        crate::features::chrome::close_browser(
                            browser_handle,
                            &shared.4,
                            &mut context_id,
                        )
                        .await;
                    }
                }
                _ => log("", "Chrome failed to start."),
            },
            _ => log("", INVALID_URL),
        }
    }

    #[cfg(not(feature = "sitemap"))]
    /// Sitemap crawl entire lists. Note: this method does not re-crawl the links of the pages found on the sitemap. This does nothing without the `sitemap` flag.
    pub async fn sitemap_crawl(
        &mut self,
        _client: &Client,
        _handle: &Option<Arc<AtomicI8>>,
        _scrape: bool,
    ) {
    }

    #[cfg(not(feature = "sitemap"))]
    /// Sitemap crawl entire lists chain. Note: this method does not re-crawl the links of the pages found on the sitemap. This does nothing without the [sitemap] flag.
    async fn sitemap_crawl_chain(
        &mut self,
        _client: &Client,
        _handle: &Option<Arc<AtomicI8>>,
        _scrape: bool,
    ) {
    }

    /// Sitemap crawl entire lists. Note: this method does not re-crawl the links of the pages found on the sitemap. This does nothing without the `sitemap` flag.
    #[cfg(feature = "sitemap")]
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
                let mut q = match &self.channel_queue {
                    Some(q) => Some(q.0.subscribe()),
                    _ => None,
                };

                let domain = self.url.inner().as_str();
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

                loop {
                    let stream =
                        tokio_stream::iter::<Vec<Box<CompactString>>>(sitemaps.drain(..).collect());
                    tokio::pin!(stream);

                    while let Some(sitemap_url) = stream.next().await {
                        if !self.handle_process(handle, &mut interval, async {}).await {
                            break;
                        }
                        let (tx, mut rx) = tokio::sync::mpsc::channel::<Page>(32);

                        let shared = shared.clone();

                        let handles = tokio::spawn(async move {
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

                                                            tokio::spawn(async move {
                                                                let page = Page::new_page(
                                                                    &link.inner(),
                                                                    &client,
                                                                )
                                                                .await;

                                                                match tx.reserve().await {
                                                                    Ok(permit) => {
                                                                        permit.send(page);
                                                                    }
                                                                    _ => (),
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
                                                    log("incorrect sitemap error: ", err.msg())
                                                }
                                            };
                                        }
                                    }
                                    Err(err) => log("http parse error: ", err.to_string()),
                                };
                            }
                            Err(err) => log("http network error: ", err.to_string()),
                        };

                        drop(tx);

                        if let Ok(mut handle) = handles.await {
                            for page in handle.iter_mut() {
                                page.detect_language();
                                let links = page.links(&selectors).await;
                                self.extra_links.extend(links)
                            }
                            if scrape {
                                match self.pages.as_mut() {
                                    Some(p) => p.extend(handle),
                                    _ => (),
                                };
                            }

                            match q.as_mut() {
                                Some(q) => {
                                    while let Ok(link) = q.try_recv() {
                                        let s = link.into();
                                        let allowed = self.is_allowed(&s);

                                        if allowed.eq(&ProcessLinkStatus::BudgetExceeded) {
                                            break;
                                        }
                                        if allowed.eq(&ProcessLinkStatus::Blocked) {
                                            continue;
                                        }
                                        self.extra_links
                                            .extend(&HashSet::from([s]) - &self.links_visited);
                                    }
                                }
                                _ => (),
                            }
                        }
                    }

                    if sitemaps.len() == 0 {
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
                        ));

                        let mut sitemaps = match self.configuration.sitemap_url {
                            Some(ref sitemap) => Vec::from([sitemap.to_owned()]),
                            _ => Default::default(),
                        };

                        loop {
                            let stream = tokio_stream::iter::<Vec<Box<CompactString>>>(
                                sitemaps.drain(..).collect(),
                            );
                            tokio::pin!(stream);

                            while let Some(sitemap_url) = stream.next().await {
                                if !self.handle_process(handle, &mut interval, async {}).await {
                                    break;
                                }
                                let (tx, mut rx) = tokio::sync::mpsc::channel::<Page>(32);

                                let shared_1 = shared.clone();

                                let handles = tokio::spawn(async move {
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

                                                                    tokio::spawn(async move {
                                                                        match attempt_navigation(
                                                                            link.inner().as_str(),
                                                                            &shared.2,
                                                                            &shared
                                                                                .3
                                                                                .request_timeout,
                                                                        )
                                                                        .await
                                                                        {
                                                                            Ok(new_page) => {
                                                                                let new_page = configure_browser(new_page, &shared.3).await;
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
                                                                                )
                                                                                .await;

                                                                                match tx
                                                                                    .reserve()
                                                                                    .await
                                                                                {
                                                                                    Ok(permit) => {
                                                                                        permit
                                                                                            .send(
                                                                                            page,
                                                                                        );
                                                                                    }
                                                                                    _ => (),
                                                                                }
                                                                            }
                                                                            _ => (),
                                                                        }
                                                                    });
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
                                                        SiteMapEntity::Err(err) => log(
                                                            "incorrect sitemap error: ",
                                                            err.msg(),
                                                        ),
                                                    };
                                                }
                                            }
                                            Err(err) => log("http parse error: ", err.to_string()),
                                        };
                                    }
                                    Err(err) => log("http network error: ", err.to_string()),
                                };

                                drop(tx);

                                if let Ok(mut handle) = handles.await {
                                    for page in handle.iter_mut() {
                                        page.detect_language();
                                        self.extra_links.extend(page.links(&selectors).await)
                                    }
                                    if scrape {
                                        match self.pages.as_mut() {
                                            Some(p) => p.extend(handle),
                                            _ => (),
                                        };
                                    }
                                }
                            }

                            if sitemaps.len() == 0 {
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
    #[cfg(feature = "chrome_store_page")]
    fn subscription_guard(&self) {
        match &self.channel {
            Some(channel) => {
                if !channel.1.is_empty() {
                    match &self.channel_guard {
                        Some(guard_counter) => guard_counter.lock(),
                        _ => (),
                    }
                }
            }
            _ => (),
        }
    }

    /// Guard the channel from closing until all subscription events complete.
    #[cfg(not(feature = "chrome_store_page"))]
    fn subscription_guard(&self) {}

    /// Launch or connect to browser with setup
    #[cfg(feature = "chrome")]
    pub async fn setup_browser(
        &self,
    ) -> Option<(
        Arc<chromiumoxide::Browser>,
        tokio::task::JoinHandle<()>,
        Option<chromiumoxide::cdp::browser_protocol::browser::BrowserContextId>,
    )> {
        match launch_browser(&self.configuration).await {
            Some((browser, browser_handle, context_id)) => {
                let browser = Arc::new(browser);

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
        chrome_intercept: bool,
        block_images: bool,
    ) -> &mut Self {
        self.configuration
            .with_chrome_intercept(chrome_intercept, block_images);
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
            match &self.domain_parsed {
                Some(domain) => match domain.path_segments() {
                    Some(segments) => {
                        let segments_cnt = segments.count();

                        if segments_cnt > self.configuration.depth {
                            self.configuration.depth_distance = self.configuration.depth
                                + self.configuration.depth.abs_diff(segments_cnt);
                        } else {
                            self.configuration.depth_distance = self.configuration.depth;
                        }
                    }
                    _ => (),
                },
                _ => (),
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
                    DEFAULT_PERMITS.clone()
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
    match channel {
        Some(c) => {
            match c.0.send(page) {
                Ok(_) => match channel_guard {
                    Some(guard) => ChannelGuard::inc_guard(&guard.0 .1),
                    _ => (),
                },
                _ => (),
            };
        }
        _ => (),
    };
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
    #[cfg(feature = "chrome_store_page")]
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
        write!(f, "`{}`", self)
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
            .contains::<CaseInsensitiveString>(&"https://choosealicense.com/licenses/".into()),
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
            links_visited
                .contains::<CaseInsensitiveString>(&"https://choosealicense.com/licenses/".into()),
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
            links_visited
                .contains::<CaseInsensitiveString>(&"https://choosealicense.com/licenses/".into()),
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
            .contains::<CaseInsensitiveString>(&"https://choosealicense.com/licenses/".into()),
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

    assert_eq!(website.links_visited, uniq); // only the target url should exist
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
            .contains::<CaseInsensitiveString>(&"https://choosealicense.com/licenses/".into()),
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
            .contains::<CaseInsensitiveString>(&"https://choosealicense.com/licenses/".into()),
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
            .contains::<CaseInsensitiveString>(&"https://choosealicense.com/licenses/".into())
            || website
                .links_visited
                .contains::<CaseInsensitiveString>(&"http://choosealicense.com/licenses/".into()),
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
            .contains::<CaseInsensitiveString>(&"https://choosealicense.com/licenses/".into()),
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

    for links_visited in website.links_visited.iter() {
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

    assert!(has_unique_elements(&*website.links_visited));
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
            .contains::<CaseInsensitiveString>(&"https://choosealicense.com/licenses/".into()),
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
#[cfg(all(feature = "cache", not(feature = "decentralized")))]
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
