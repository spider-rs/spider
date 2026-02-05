use crate::black_list::contains;
use crate::client::redirect::Policy;
use crate::compact_str::CompactString;
use crate::configuration::{
    self, get_ua, AutomationScriptsMap, Configuration, ExecutionScriptsMap, RedirectPolicy,
    SerializableHeaderMap,
};
use crate::{page::build, utils::PageResponse};

#[cfg(feature = "smart")]
use crate::features::chrome::OnceBrowser;
use crate::features::chrome_common::RequestInterceptConfiguration;
#[cfg(feature = "disk")]
use crate::features::disk::DatabaseHandler;
use crate::packages::robotparser::parser::RobotFileParser;
use crate::page::{
    AntiBotTech, Page, PageLinkBuildSettings, CHROME_UNKNOWN_STATUS_ERROR, UNKNOWN_STATUS_ERROR,
};
use crate::utils::abs::{convert_abs_url, parse_absolute_url};
use crate::utils::interner::ListBucket;
use crate::utils::{
    crawl_duration_expired, emit_log, emit_log_shutdown, get_path_from_url, get_semaphore,
    networking_capable, prepare_url, setup_website_selectors, spawn_set, AllowedDomainTypes,
};
use crate::{CaseInsensitiveString, Client, ClientBuilder, RelativeSelectors};
#[cfg(feature = "cron")]
use async_job::{async_trait, Job, Runner};
use hashbrown::{HashMap, HashSet};
use reqwest::header::REFERER;
use reqwest::StatusCode;
use std::fmt;
use std::net::IpAddr;
use std::sync::atomic::{AtomicBool, AtomicI8, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::{
    sync::{broadcast, Semaphore},
    task::JoinSet,
    time::Interval,
};
use tokio_stream::StreamExt;
use url::Url;

#[cfg(feature = "cache_request")]
use http_cache_reqwest::{Cache, CacheMode, HttpCache, HttpCacheOptions};

#[cfg(feature = "cache_request")]
pub use http_global_cache::CACACHE_MANAGER;

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

/// Javascript challenge pages.
static JS_SAFE_CHALLENGE_PATTERNS: &[&str] = &[
    r#"Enable JavaScript and cookies to continue"#, // Cloudflare
    r#"To continue, please enable JavaScript in your browser settings"#, // Akamai, F5
    r#"Please enable JavaScript to view the page content"#, // AWS WAF
];

/// check if the page is a javascript challenge
pub fn is_safe_javascript_challenge(page: &Page) -> bool {
    let page = page.get_html_bytes_u8();

    let page_size = page.len();

    if page_size == 0 || page_size > 10_000 {
        return false;
    }

    AC_JS_CHALLENGE.find(page).is_some()
}

#[cfg(all(
    any(
        target_os = "android",
        target_os = "fuchsia",
        target_os = "illumos",
        target_os = "ios",
        target_os = "linux",
        target_os = "macos",
        target_os = "solaris",
        target_os = "tvos",
        target_os = "visionos",
        target_os = "watchos",
    ),
    not(feature = "wreq")
))]
/// Bind connections only on the specified network interface.
pub fn set_interface(client: ClientBuilder, network_interface: &str) -> ClientBuilder {
    client.interface(&network_interface)
}

#[cfg(not(any(
    feature = "wreq",
    target_os = "android",
    target_os = "fuchsia",
    target_os = "illumos",
    target_os = "ios",
    target_os = "linux",
    target_os = "macos",
    target_os = "solaris",
    target_os = "tvos",
    target_os = "visionos",
    target_os = "watchos",
)))]
/// Bind connections only on the specified network interface.
pub fn set_interface(client: ClientBuilder, _interface: &str) -> ClientBuilder {
    client
}

lazy_static! {
    static ref AC_JS_CHALLENGE: aho_corasick::AhoCorasick =  aho_corasick::AhoCorasick::new(JS_SAFE_CHALLENGE_PATTERNS).expect("safe challenges");
    /// The default Semaphore limits.
    pub static ref DEFAULT_PERMITS: usize = calc_limits(1);
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
    /// The max links to store in memory.
    pub(crate) static ref LINKS_VISITED_MEMORY_LIMIT: usize = {
        const DEFAULT_LIMIT: usize = 15_000;

        match std::env::var("LINKS_VISITED_MEMORY_LIMIT") {
            Ok(limit) => limit.parse::<usize>().unwrap_or(DEFAULT_LIMIT),
            _ => DEFAULT_LIMIT
        }
    };
    static ref WILD_CARD_PATH: CaseInsensitiveString = CaseInsensitiveString::from("*");
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

// const INVALID_URL: &str = "The domain should be a valid URL, refer to <https://www.w3.org/TR/2011/WD-html5-20110525/urls.html#valid-url>.";

/// the active status of the crawl.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, strum::EnumString, strum::Display)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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
    /// Crawl blocked from spider firewall.
    FirewallBlocked,
    /// The crawl failed from a server error.
    ServerError,
    /// The crawl failed from a connection error with proxy or dns.
    ConnectError,
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
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, strum::EnumString, strum::Display)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, strum::EnumString, strum::Display)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum CronType {
    #[default]
    /// Crawl collecting links, page data, and etc.
    Crawl,
    /// Scrape collecting links, page data as bytes to store, and etc.
    Scrape,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, strum::EnumString, strum::Display)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
/// Generic website meta info for handling retries.
pub enum WebsiteMetaInfo {
    /// The page requires Javascript.
    RequiresJavascript,
    /// Standard apache 403 page that requires a special http header for access like a custom iframe server.
    Apache403,
    /// Standard Open Resty 403 page that requires a special http header for access like a custom iframe server.
    OpenResty403,
    /// No meta info.
    #[default]
    None,
}

/// On link find callback rewrite a url if it meets a condition.
pub type OnLinkFindCallback = Arc<
    dyn Fn(CaseInsensitiveString, Option<String>) -> (CaseInsensitiveString, Option<String>)
        + Send
        + Sync,
>;

/// Callback closure that determines if a link should be crawled or not.
pub trait OnShouldCrawlClosure: Fn(&Page) -> bool + Send + Sync + 'static {}
impl<F: Fn(&Page) -> bool + Send + Sync + 'static> OnShouldCrawlClosure for F {}

/// Callback closure or function pointer that determines if a link should be crawled or not.
#[derive(Clone)]
pub enum OnShouldCrawlCallback {
    /// Static function pointer.
    Fn(fn(&Page) -> bool),

    /// Closure.
    Closure(Arc<dyn OnShouldCrawlClosure>),
}
impl OnShouldCrawlCallback {
    fn call(&self, page: &Page) -> bool {
        match self {
            Self::Fn(func) => func(page),
            Self::Closure(closure) => closure(page),
        }
    }
}

/// Round-robin client rotator for proxy rotation.
/// Each client is built with a single proxy, and `next()` cycles through them.
#[derive(Clone)]
pub struct ClientRotator {
    clients: Vec<Client>,
    index: Arc<AtomicUsize>,
}

impl ClientRotator {
    /// Create a new rotator from a list of clients.
    pub fn new(clients: Vec<Client>) -> Self {
        Self {
            clients,
            index: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Get the next client in round-robin order.
    pub fn next(&self) -> &Client {
        let idx = self.index.fetch_add(1, Ordering::Relaxed) % self.clients.len();
        &self.clients[idx]
    }

    /// Number of clients in the rotator.
    pub fn len(&self) -> usize {
        self.clients.len()
    }

    /// Whether the rotator is empty.
    pub fn is_empty(&self) -> bool {
        self.clients.is_empty()
    }
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
#[derive(Clone, Default)]
pub struct Website {
    /// Configuration properties for website.
    pub configuration: Box<Configuration>,
    /// The callback when a link is found.
    pub on_link_find_callback: Option<OnLinkFindCallback>,
    /// The callback to use if a page should be ignored. Return false to ensure that the discovered links are not crawled.
    pub on_should_crawl_callback: Option<OnShouldCrawlCallback>,
    /// Set the crawl ID to track. This allows explicit targeting for shutdown, pause, and etc.
    pub crawl_id: Box<String>,
    #[cfg(feature = "extra_information")]
    /// Extra information to store.
    pub extra_info: Option<Box<String>>,
    /// Seed the initial html for crawling.
    seed_html: Option<String>,
    /// All URLs visited.
    links_visited: Box<ListBucket>,
    /// All signatures.
    signatures: Box<HashSet<u64>>,
    /// Extra links to crawl.
    extra_links: Box<HashSet<CaseInsensitiveString>>,
    /// Pages visited.
    pages: Option<Vec<Page>>,
    /// Robot.txt parser.
    robot_file_parser: Option<Box<RobotFileParser>>,
    /// Base url of the crawl.
    url: Box<CaseInsensitiveString>,
    /// The domain url parsed.
    domain_parsed: Option<Box<Url>>,
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
    /// The initial anti-bot tech found.
    initial_anti_bot_tech: AntiBotTech,
    /// The initial bytes size of the first request.
    initial_html_length: usize,
    /// The initial page had a waf detection.
    initial_page_waf_check: bool,
    /// The initial page should retry.
    initial_page_should_retry: bool,
    /// The website was manually stopped.
    shutdown: bool,
    /// The request client. Stored for re-use between runs.
    client: Option<Client>,
    /// Round-robin client rotator for proxy rotation. Built when 2+ proxies are configured.
    client_rotator: Option<Arc<ClientRotator>>,
    /// The disk handler to use.
    #[cfg(feature = "disk")]
    sqlite: Option<Box<DatabaseHandler>>,
    /// Configure sqlite on start
    #[cfg(feature = "disk")]
    enable_sqlite: bool,
    /// Was the setup already configured for sync sendable thread use?
    send_configured: bool,
    /// The website requires javascript to load. This will be sent as a hint when http request.
    website_meta_info: WebsiteMetaInfo,
    /// Skip the initial link?
    skip_initial: bool,
    #[cfg(feature = "cookies")]
    /// Cookie jar between request.
    pub cookie_jar: Arc<reqwest::cookie::Jar>,
}

impl fmt::Debug for Website {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let domain_str = self.domain_parsed.as_ref().map(|u| u.as_str().to_owned());
        let pages_len = self.pages.as_ref().map(|p| p.len()).unwrap_or(0);

        let mut ds = f.debug_struct("Website");

        ds.field("url", &self.url.as_ref())
            .field("crawl_id", &self.crawl_id)
            .field("domain_parsed", &domain_str)
            // callbacks – just show presence, avoids Fn: Debug bound
            .field(
                "on_link_find_callback",
                &self.on_link_find_callback.is_some(),
            )
            .field(
                "on_should_crawl_callback",
                &self.on_should_crawl_callback.is_some(),
            )
            // state + counters
            .field("status", &self.status)
            .field("shutdown", &self.shutdown)
            .field("extra_links_len", &self.extra_links.len())
            .field("signatures_len", &self.signatures.len())
            .field("pages_len", &pages_len)
            // channels / sqlite / client: just booleans
            .field("channel_present", &self.channel.is_some())
            .field("channel_queue_present", &self.channel_queue.is_some())
            .field("client_present", &self.client.is_some())
            // initial page info
            .field("initial_status_code", &self.initial_status_code)
            .field("initial_html_length", &self.initial_html_length)
            .field("initial_anti_bot_tech", &self.initial_anti_bot_tech)
            .field("initial_page_waf_check", &self.initial_page_waf_check)
            .field("initial_page_should_retry", &self.initial_page_should_retry)
            // misc flags/meta
            .field("send_configured", &self.send_configured)
            .field("website_meta_info", &self.website_meta_info)
            .field("skip_initial", &self.skip_initial);

        #[cfg(feature = "disk")]
        {
            ds.field("sqlite_present", &self.sqlite.is_some())
                .field("enable_sqlite", &self.enable_sqlite);
        }

        ds.finish()
    }
}

impl Website {
    /// Initialize the Website with a starting link to crawl and check the firewall base.
    fn _new(url: &str, check_firewall: bool) -> Self {
        let url = url.trim();
        let url: Box<CaseInsensitiveString> = if networking_capable(url) {
            CaseInsensitiveString::new(&url).into()
        } else {
            CaseInsensitiveString::new(&prepare_url(url)).into()
        };

        let domain_parsed: Option<Box<Url>> = parse_absolute_url(&url);
        let mut status = CrawlStatus::Start;

        if let Some(u) = &domain_parsed {
            if check_firewall && crate::utils::abs::block_website(&u) {
                status = CrawlStatus::FirewallBlocked;
            }
        }

        Self {
            configuration: Configuration::new().into(),
            status,
            domain_parsed,
            url,
            #[cfg(feature = "disk")]
            enable_sqlite: true,
            ..Default::default()
        }
    }

    /// Initialize the Website with a starting link to crawl.
    pub fn new(url: &str) -> Self {
        Website::_new(url, true)
    }

    /// Initialize the Website with a starting link to crawl and check the firewall.
    pub fn new_with_firewall(url: &str, check_firewall: bool) -> Self {
        Website::_new(url, check_firewall)
    }

    /// Setup a shared database.
    #[cfg(feature = "disk")]
    pub fn setup_database_handler(&self) -> Box<DatabaseHandler> {
        Box::new(DatabaseHandler::new(&Some(self.target_id())))
    }

    #[cfg(feature = "disk")]
    /// Setup the sqlist usage.
    pub fn setup_shared_db(&mut self, db: Box<DatabaseHandler>) {
        self.sqlite = Some(db)
    }

    #[cfg(feature = "disk")]
    /// Setup the sqlist usage.
    pub fn setup_sqlite(&mut self) {
        if self.sqlite.is_none() {
            self.sqlite = Some(self.setup_database_handler())
        }
    }

    /// Set the url of the website to re-use configuration and data.
    pub fn set_url(&mut self, url: &str) -> &mut Self {
        let url = if url.starts_with(' ') || url.ends_with(' ') {
            url.trim()
        } else {
            url
        };

        let domain: Box<CaseInsensitiveString> = if networking_capable(url) {
            CaseInsensitiveString::new(&url).into()
        } else {
            CaseInsensitiveString::new(&prepare_url(&url)).into()
        };

        self.domain_parsed = parse_absolute_url(&domain);
        self.url = domain;
        self
    }

    /// Set the direct url of the website to re-use configuration and data without parsing the domain.
    pub fn set_url_only(&mut self, url: &str) -> &mut Self {
        self.url = CaseInsensitiveString::new(&url).into();
        self
    }

    /// Get the target id for a crawl. This takes the crawl ID and the url and concats it without delimiters.
    pub fn target_id(&self) -> String {
        string_concat!(self.crawl_id, self.url.inner())
    }

    /// Single page request.
    pub fn single_page(&self) -> bool {
        match &self.configuration.inner_budget {
            Some(b) => match b.get(&*WILD_CARD_PATH) {
                Some(b) => b.eq(&1),
                _ => false,
            },
            _ => false,
        }
    }

    /// Setup SQLite. This does nothing with `disk` flag enabled.
    #[cfg(feature = "disk")]
    pub fn setup_disk(&mut self) {
        if self.enable_sqlite && self.sqlite.is_none() {
            self.setup_sqlite();
        }
        // run full on sqlite.
        if self.configuration.shared {
            if let Some(sqlite) = self.sqlite.as_mut() {
                sqlite.seeded = true;
                // sqlite.persist = true;
            }
        }
    }

    #[cfg(feature = "disk")]
    /// Set the sqlite disk persistance.
    pub fn set_disk_persistance(&mut self, persist: bool) -> &mut Self {
        if self.enable_sqlite {
            if !self.sqlite.is_none() {
                if let Some(sqlite) = self.sqlite.as_mut() {
                    sqlite.persist = persist;
                }
            }
        }
        self
    }

    /// Setup SQLite. This does nothing with `disk` flag enabled.
    #[cfg(not(feature = "disk"))]
    pub fn setup_disk(&mut self) {}

    /// Get the robots.txt parser.
    pub fn get_robots_parser(&self) -> &Option<Box<RobotFileParser>> {
        &self.robot_file_parser
    }

    /// Does the website require javascript to run?
    pub fn get_requires_javascript(&self) -> bool {
        self.website_meta_info == WebsiteMetaInfo::RequiresJavascript
    }

    /// Get the website meta information that can help with retry handling.
    pub fn get_website_meta_info(&self) -> &WebsiteMetaInfo {
        &self.website_meta_info
    }

    /// Check if URL exists (ignore case). This does nothing with `disk` flag enabled.
    #[cfg(feature = "disk")]
    pub async fn is_allowed_disk(&self, url_to_check: &str) -> bool {
        match &self.sqlite {
            Some(sqlite) => {
                if !sqlite.ready() {
                    true
                } else {
                    let db_pool = sqlite.get_db_pool().await;
                    let allowed = sqlite.url_exists(db_pool, url_to_check).await;

                    !allowed
                }
            }
            _ => true,
        }
    }

    /// Check if URL exists (ignore case). This does nothing with `disk` flag enabled.
    #[cfg(not(feature = "disk"))]
    pub async fn is_allowed_disk(&self, _url_to_check: &str) -> bool {
        true
    }

    /// Check if signature exists (ignore case). This does nothing with `disk` flag enabled.
    #[cfg(feature = "disk")]
    pub async fn is_allowed_signature_disk(&self, signature_to_check: u64) -> bool {
        match &self.sqlite {
            Some(sqlite) => {
                if !sqlite.ready() {
                    true
                } else {
                    let db_pool = sqlite.get_db_pool().await;

                    !sqlite.signature_exists(db_pool, signature_to_check).await
                }
            }
            _ => true,
        }
    }

    /// Check if signature exists (ignore case). This does nothing with `disk` flag enabled.
    #[cfg(not(feature = "disk"))]
    pub async fn is_allowed_signature_disk(&self, _signature_to_check: u64) -> bool {
        true
    }

    /// Is the signature allowed.
    pub async fn is_signature_allowed(&self, signature: u64) -> bool {
        !self.signatures.contains(&signature) || self.is_allowed_signature_disk(signature).await
    }

    /// Clear the disk. This does nothing with `disk` flag enabled.
    #[cfg(feature = "disk")]
    pub async fn clear_disk(&self) {
        if let Some(sqlite) = &self.sqlite {
            if sqlite.pool_inited() {
                let _ = DatabaseHandler::clear_table(sqlite.get_db_pool().await).await;
            }
        }
    }

    /// Clear the disk. This does nothing with `disk` flag enabled.
    #[cfg(not(feature = "disk"))]
    pub async fn clear_disk(&self) {}

    /// Check if the disk is enabled. This does nothing with `disk` flag enabled.
    #[cfg(not(feature = "disk"))]
    pub(crate) fn shared_disk_enabled(&self) -> bool {
        false
    }

    /// Check if the disk is enabled. This does nothing with `disk` flag enabled.
    #[cfg(feature = "disk")]
    pub(crate) fn shared_disk_enabled(&self) -> bool {
        self.configuration.shared && self.sqlite.is_some()
    }

    /// Insert a new URL to disk if it doesn't exist. This does nothing with `disk` flag enabled.
    #[cfg(feature = "disk")]
    pub async fn insert_url_disk(&self, new_url: &str) {
        if let Some(sqlite) = &self.sqlite {
            sqlite.insert_url(sqlite.get_db_pool().await, new_url).await
        }
    }

    /// Insert a new signature to disk if it doesn't exist. This does nothing with `disk` flag enabled.
    #[cfg(feature = "disk")]
    pub async fn insert_signature_disk(&self, signature: u64) {
        if let Some(sqlite) = &self.sqlite {
            sqlite
                .insert_signature(sqlite.get_db_pool().await, signature)
                .await
        }
    }

    /// Insert a new URL if it doesn't exist. This does nothing with `disk` flag enabled.
    #[cfg(feature = "disk")]
    pub async fn insert_link(&mut self, new_url: CaseInsensitiveString) {
        let mem_load = crate::utils::detect_system::get_global_memory_state().await;
        let beyond_memory_limits = self.links_visited.len() >= *LINKS_VISITED_MEMORY_LIMIT;
        let seed_check = mem_load == 2 || mem_load == 1 || beyond_memory_limits;

        if seed_check {
            let mut seeded = false;
            if let Some(sqlite) = &self.sqlite {
                if !sqlite.ready() {
                    let _ = self.seed().await;
                    seeded = true;
                }
            }
            if let Some(sqlite) = self.sqlite.as_mut() {
                sqlite.set_seeded(seeded);
            }
        }

        if mem_load == 2 || beyond_memory_limits || self.shared_disk_enabled() {
            self.insert_url_disk(&new_url).await
        } else if mem_load == 1 {
            if self.links_visited.len() <= 100 {
                self.links_visited.insert(new_url);
            } else {
                self.insert_url_disk(&new_url).await
            }
        } else {
            self.links_visited.insert(new_url);
        }
    }

    /// Insert a new URL if it doesn't exist. This does nothing with `disk` flag enabled.
    #[cfg(not(feature = "disk"))]
    pub async fn insert_link(&mut self, link: CaseInsensitiveString) {
        self.links_visited.insert(link);
    }

    /// Insert a new signature if it doesn't exist. This does nothing with `disk` flag enabled.
    #[cfg(feature = "disk")]
    pub async fn insert_signature(&mut self, new_signature: u64) {
        let mem_load = crate::utils::detect_system::get_global_memory_state().await;
        let beyond_memory_limits = self.signatures.len() >= *LINKS_VISITED_MEMORY_LIMIT;
        let seed_check = mem_load == 2 || mem_load == 1 || beyond_memory_limits;

        if seed_check {
            let mut seeded = false;
            if let Some(sqlite) = &self.sqlite {
                if !sqlite.ready() {
                    let _ = self.seed().await;
                    seeded = true;
                }
            }
            if let Some(sqlite) = self.sqlite.as_mut() {
                sqlite.set_seeded(seeded);
            }
        }

        if mem_load == 2 || beyond_memory_limits || self.shared_disk_enabled() {
            self.insert_signature_disk(new_signature).await
        } else if mem_load == 1 {
            if self.signatures.len() <= 100 {
                self.signatures.insert(new_signature);
            } else {
                self.insert_signature_disk(new_signature).await
            }
        } else {
            self.signatures.insert(new_signature);
        }
    }

    /// Insert a new signature if it doesn't exist. This does nothing with `disk` flag enabled.
    #[cfg(not(feature = "disk"))]
    pub async fn insert_signature(&mut self, new_signature: u64) {
        self.signatures.insert(new_signature);
    }

    /// Seed the DB and clear the Hashset. This does nothing with `disk` flag enabled.
    #[cfg(feature = "disk")]
    pub async fn seed(&mut self) -> Result<(), sqlx::Error> {
        let links = self.get_links();

        if let Some(sqlite) = &self.sqlite {
            if let Ok(links) = sqlite.seed(sqlite.get_db_pool().await, links).await {
                self.links_visited.clear();

                for link in links {
                    self.links_visited.insert(link);
                }

                if let Some(sqlite) = self.sqlite.as_mut() {
                    sqlite.seeded = true;
                }
            }
        }

        Ok(())
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
    /// - is not over depth
    /// - is not over crawl budget
    /// - is optionally whitelisted
    /// - is not blacklisted
    /// - is not forbidden in robot.txt file (if parameter is defined)
    #[inline]
    #[cfg(not(feature = "regex"))]
    pub fn is_allowed(&mut self, link: &CaseInsensitiveString) -> ProcessLinkStatus {
        let status = self.is_allowed_budgetless(link);

        if status.eq(&ProcessLinkStatus::Allowed) {
            if self.is_over_budget(link) {
                return ProcessLinkStatus::BudgetExceeded;
            }
        }

        status
    }

    /// return `true` if URL:
    ///
    /// - is not already crawled
    /// - is not over depth
    /// - is not over crawl budget
    /// - is optionally whitelisted
    /// - is not blacklisted
    /// - is not forbidden in robot.txt file (if parameter is defined)
    #[inline]
    #[cfg(feature = "regex")]
    pub fn is_allowed(&mut self, link: &CaseInsensitiveString) -> ProcessLinkStatus {
        let status = self.is_allowed_budgetless(link);

        if status.eq(&ProcessLinkStatus::Allowed) {
            if self.is_over_budget(link) {
                return ProcessLinkStatus::BudgetExceeded;
            }
        }
        status
    }

    /// return `true` if URL:
    ///
    /// - is not already crawled
    /// - is not over depth
    /// - is optionally whitelisted
    /// - is not blacklisted
    /// - is not forbidden in robot.txt file (if parameter is defined)
    #[inline]
    #[cfg(not(feature = "regex"))]
    pub fn is_allowed_budgetless(&mut self, link: &CaseInsensitiveString) -> ProcessLinkStatus {
        if self.links_visited.contains(link) {
            ProcessLinkStatus::Blocked
        } else {
            let status = self.is_allowed_default(link.inner());

            if status.eq(&ProcessLinkStatus::Allowed) {
                if self.is_over_depth(link) {
                    return ProcessLinkStatus::Blocked;
                }
            }

            status
        }
    }

    /// return `true` if URL:
    ///
    /// - is not already crawled
    /// - is not over depth
    /// - is optionally whitelisted
    /// - is not blacklisted
    /// - is not forbidden in robot.txt file (if parameter is defined)
    #[inline]
    #[cfg(feature = "regex")]
    pub fn is_allowed_budgetless(&mut self, link: &CaseInsensitiveString) -> ProcessLinkStatus {
        if self.links_visited.contains(link) {
            ProcessLinkStatus::Blocked
        } else {
            let status = self.is_allowed_default(link);
            if status.eq(&ProcessLinkStatus::Allowed) {
                if self.is_over_depth(link) {
                    return ProcessLinkStatus::Blocked;
                }
            }
            status
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
            if let Some(r) = &self.robot_file_parser {
                return r.can_fetch(
                    match &self.configuration.user_agent {
                        Some(ua) => ua,
                        _ => "*",
                    },
                    link,
                );
            }
        }

        true
    }

    /// Detect if the inner budget is exceeded
    pub(crate) fn is_over_inner_depth_budget(&mut self, link: &CaseInsensitiveString) -> bool {
        let mut over = false;

        if let Some(segments) = get_path_from_url(link)
            .strip_prefix('/')
            .map(|remainder| remainder.split('/'))
        {
            let mut depth: usize = 0;

            for _ in segments {
                depth = depth.saturating_add(1);
                if depth > self.configuration.depth_distance {
                    over = true;
                    break;
                }
            }
        }

        over
    }

    /// is over the wild card budget.
    #[cfg(feature = "sitemap")]
    pub(crate) fn is_over_wild_budget(
        &self,
        budget: &Option<hashbrown::HashMap<case_insensitive_string::CaseInsensitiveString, u32>>,
    ) -> bool {
        let exceeded_wild_budget = if self.configuration.wild_card_budgeting {
            match budget {
                Some(budget) => match budget.get(&*WILD_CARD_PATH) {
                    Some(budget) => {
                        if budget.abs_diff(0) == 1 {
                            true
                        } else {
                            false
                        }
                    }
                    _ => false,
                },
                _ => false,
            }
        } else {
            false
        };
        exceeded_wild_budget
    }

    /// Detect if the inner budget is exceeded
    pub(crate) fn is_over_inner_budget(&mut self, link: &CaseInsensitiveString) -> bool {
        match self.configuration.inner_budget.as_mut() {
            Some(budget) => {
                let exceeded_wild_budget = if self.configuration.wild_card_budgeting {
                    match budget.get_mut(&*WILD_CARD_PATH) {
                        Some(budget) => {
                            if budget.abs_diff(0) == 1 {
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
                let skip_paths = self.configuration.wild_card_budgeting && budget.len() == 1;
                let has_depth_control = self.configuration.depth_distance > 0;

                // check if paths pass
                if !skip_paths && !exceeded_wild_budget {
                    let path_segments = get_path_from_url(link)
                        .strip_prefix('/')
                        .map(|remainder| remainder.split('/'));

                    match path_segments {
                        Some(segments) => {
                            let mut joint_segment = CaseInsensitiveString::default();
                            let mut over = false;
                            let mut depth: usize = 0;

                            for seg in segments {
                                if has_depth_control {
                                    depth = depth.saturating_add(1);
                                    if depth > self.configuration.depth_distance {
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
                    }
                } else {
                    exceeded_wild_budget
                }
            }
            _ => false,
        }
    }

    /// Validate if url exceeds crawl depth and should be ignored.
    pub(crate) fn is_over_depth(&mut self, link: &CaseInsensitiveString) -> bool {
        self.configuration.depth_distance > 0 && self.is_over_inner_depth_budget(link)
    }

    /// Validate if url exceeds crawl budget and should not be handled.
    pub(crate) fn is_over_budget(&mut self, link: &CaseInsensitiveString) -> bool {
        self.is_over_inner_budget(link)
    }

    /// Restore one wildcard budget credit (for relevance-gated irrelevant pages).
    pub(crate) fn restore_wildcard_budget(&mut self) {
        if self.configuration.wild_card_budgeting {
            if let Some(budget) = self.configuration.inner_budget.as_mut() {
                if let Some(counter) = budget.get_mut(&*WILD_CARD_PATH) {
                    *counter = counter.saturating_add(1);
                }
            }
        }
    }

    /// Amount of pages crawled in memory only. Use get_size for full links between memory and disk.
    pub fn size(&self) -> usize {
        self.links_visited.len()
    }

    /// Get the amount of resources collected.
    #[cfg(not(feature = "disk"))]
    pub async fn get_size(&self) -> usize {
        self.links_visited.len()
    }

    /// Get the amount of resources collected.
    #[cfg(feature = "disk")]
    pub async fn get_size(&self) -> usize {
        let disk_count = if let Some(sqlite) = &self.sqlite {
            if sqlite.pool_inited() {
                let disk_count = DatabaseHandler::count_records(sqlite.get_db_pool().await).await;
                let disk_count = disk_count.unwrap_or_default() as usize;
                disk_count
            } else {
                0
            }
        } else {
            0
        };

        let mut mem_count = self.links_visited.len();

        if mem_count >= *LINKS_VISITED_MEMORY_LIMIT {
            mem_count -= *LINKS_VISITED_MEMORY_LIMIT;
        }

        disk_count + mem_count
    }

    /// Drain the extra links used for things like the sitemap.
    pub fn drain_extra_links(&mut self) -> hashbrown::hash_set::Drain<'_, CaseInsensitiveString> {
        self.extra_links.drain()
    }

    /// Set the initial status code of the request.
    pub fn set_initial_status_code(&mut self, initial_status_code: StatusCode) {
        self.initial_status_code = initial_status_code;
    }

    /// Get the initial status code of the request.
    pub fn get_initial_status_code(&self) -> &StatusCode {
        &self.initial_status_code
    }

    /// Set the initial html size of the request.
    pub fn set_initial_html_length(&mut self, initial_html_length: usize) {
        self.initial_html_length = initial_html_length;
    }

    /// Get the initial html size of the request.
    pub fn get_initial_html_length(&self) -> usize {
        self.initial_html_length
    }

    /// Set the initial anti-bot tech code used for the intitial request.
    pub fn set_initial_anti_bot_tech(&mut self, initial_anti_bot_tech: AntiBotTech) {
        self.initial_anti_bot_tech = initial_anti_bot_tech;
    }

    /// Get the initial anti-bot tech code used for the intitial request.
    pub fn get_initial_anti_bot_tech(&self) -> &AntiBotTech {
        &self.initial_anti_bot_tech
    }

    /// Set the initial waf detected used for the intitial request
    pub fn set_initial_page_waf_check(&mut self, initial_page_waf_check: bool) {
        self.initial_page_waf_check = initial_page_waf_check;
    }

    /// Get the initial waf detected used for the intitial request.
    pub fn get_initial_page_waf_check(&self) -> bool {
        self.initial_page_waf_check
    }

    /// Set the initial page should retry determination used for the intitial request.
    pub fn set_initial_page_should_retry(&mut self, initial_page_should_retry: bool) {
        self.initial_page_should_retry = initial_page_should_retry;
    }

    /// Get the initial page should retry determination used for the intitial request.
    pub fn get_initial_page_should_retry(&self) -> bool {
        self.initial_page_should_retry
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

    /// Drain the signatures visited.
    #[cfg(any(
        feature = "string_interner_bucket_backend",
        feature = "string_interner_string_backend",
        feature = "string_interner_buffer_backend",
    ))]
    pub fn drain_signatures(&mut self) -> hashbrown::hash_set::Drain<'_, u64> {
        self.signatures.drain()
    }

    #[cfg(not(any(
        feature = "string_interner_bucket_backend",
        feature = "string_interner_string_backend",
        feature = "string_interner_buffer_backend",
    )))]
    /// Drain the signatures visited.
    pub fn drain_signatures(&mut self) -> hashbrown::hash_set::Drain<'_, u64> {
        self.signatures.drain()
    }

    /// Set extra links to crawl. This could be used in conjuntion with 'website.persist_links' to extend the crawl on the next run.
    pub fn set_extra_links(
        &mut self,
        extra_links: HashSet<CaseInsensitiveString>,
    ) -> &HashSet<CaseInsensitiveString> {
        self.extra_links.extend(extra_links);
        &self.extra_links
    }

    /// Get the extra links.
    pub fn get_extra_links(&self) -> &HashSet<CaseInsensitiveString> {
        &self.extra_links
    }

    /// Clear all pages, disk, and links stored in memory.
    pub async fn clear_all(&mut self) {
        self.clear();
        self.clear_disk().await;
    }

    /// Clear all pages and links stored in memory.
    pub fn clear(&mut self) {
        self.links_visited.clear();
        self.signatures.clear();
        self.pages.take();
        self.extra_links.clear();
    }

    /// Get the HTTP request client. The client is set after the crawl has started.
    pub fn get_client(&self) -> &Option<Client> {
        &self.client
    }

    /// Page getter.
    pub fn get_pages(&self) -> Option<&Vec<Page>> {
        self.pages.as_ref()
    }

    /// Links visited getter for disk. This does nothing with `disk` flag enabled.
    #[cfg(not(feature = "disk"))]
    pub async fn get_links_disk(&self) -> HashSet<CaseInsensitiveString> {
        Default::default()
    }

    /// Links visited getter for disk. This does nothing with `disk` flag enabled.
    #[cfg(feature = "disk")]
    pub async fn get_links_disk(&self) -> HashSet<CaseInsensitiveString> {
        if let Some(sqlite) = &self.sqlite {
            if sqlite.pool_inited() {
                if let Ok(links) =
                    DatabaseHandler::get_all_resources(sqlite.get_db_pool().await).await
                {
                    links
                } else {
                    Default::default()
                }
            } else {
                Default::default()
            }
        } else {
            Default::default()
        }
    }

    /// Links all the links visited between memory and disk.
    #[cfg(feature = "disk")]
    pub async fn get_all_links_visited(&self) -> HashSet<CaseInsensitiveString> {
        let mut l = self.get_links_disk().await;
        let m = self.links_visited.get_links();

        l.extend(m);

        l
    }

    /// Links all the links visited between memory and disk.
    #[cfg(not(feature = "disk"))]
    pub async fn get_all_links_visited(&self) -> HashSet<CaseInsensitiveString> {
        self.get_links()
    }

    /// Links visited getter for memory resources.
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

    /// Set the active crawl status. This is helpful when chaining crawls concurrently.
    pub fn set_status(&mut self, status: CrawlStatus) -> &CrawlStatus {
        self.status = status;
        &self.status
    }

    /// Reset the active crawl status to bypass websites that are blocked.
    pub fn reset_status(&mut self) -> &CrawlStatus {
        self.status = CrawlStatus::Start;
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
            url::Url::parse(domain.unwrap_or_default())
                .ok()
                .map(|mut url| {
                    if let Ok(mut path) = url.path_segments_mut() {
                        path.clear();
                    }
                    url
                })
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
    pub fn start(&mut self) {
        self.shutdown = false;
    }

    /// configure the robots parser on initial crawl attempt and run.
    pub async fn configure_robots_parser(&mut self, client: &Client) {
        if self.configuration.respect_robots_txt {
            let robot_file_parser = self
                .robot_file_parser
                .get_or_insert_with(RobotFileParser::new);

            if robot_file_parser.mtime() <= 4000 {
                let host_str = match &self.domain_parsed {
                    Some(domain) => domain.as_str(),
                    _ => self.url.inner(),
                };

                if !host_str.is_empty() {
                    if host_str.ends_with('/') {
                        robot_file_parser.read(&client, host_str).await;
                    } else {
                        robot_file_parser
                            .read(&client, &string_concat!(host_str, "/"))
                            .await;
                    }
                }
                if let Some(delay) =
                    robot_file_parser.get_crawl_delay(&self.configuration.user_agent)
                {
                    self.configuration.delay = delay.as_millis().min(60000) as u64;
                }
            }
        }
    }

    /// Setup strict a strict redirect policy for request. All redirects need to match the host.
    pub fn setup_strict_policy(&self) -> Policy {
        use crate::client::redirect::Attempt;
        use crate::page::domain_name;
        use std::sync::atomic::AtomicU8;

        let default_policy = Policy::default();

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
                Policy::custom(custom_policy)
            }
            _ => default_policy,
        }
    }

    /// Setup redirect policy for reqwest.
    pub fn setup_redirect_policy(&self) -> Policy {
        match self.configuration.redirect_policy {
            RedirectPolicy::Loose => Policy::limited(*self.configuration.redirect_limit),
            RedirectPolicy::None => Policy::none(),
            RedirectPolicy::Strict => self.setup_strict_policy(),
        }
    }

    /// Configure the headers to use.
    pub fn configure_headers(&mut self) {
        let mut headers: reqwest::header::HeaderMap = reqwest::header::HeaderMap::new();

        let user_agent = match &self.configuration.user_agent {
            Some(ua) => ua.as_str(),
            _ => get_ua(self.configuration.only_chrome_agent()),
        };

        if self.configuration.modify_headers {
            crate::utils::header_utils::extend_headers(
                &mut headers,
                user_agent,
                &self.configuration.headers,
                &None,
                &self.configuration.viewport,
                &self.domain_parsed,
            );

            if !headers.is_empty() {
                // always remove the referer header.
                if let Some(referer) = headers.remove(REFERER) {
                    if let Ok(v) = referer.to_str() {
                        // modify the default referer
                        if self.configuration.referer.is_none() && !v.is_empty() {
                            self.configuration.referer = Some(v.into())
                        }
                    }
                }
                self.configuration
                    .headers
                    .replace(Box::new(SerializableHeaderMap::from(headers)));
            }
        }
    }

    #[cfg(all(not(feature = "wreq"), not(feature = "decentralized")))]
    /// Base client configuration.
    pub fn configure_base_client(&self) -> ClientBuilder {
        let policy = self.setup_redirect_policy();

        let user_agent = match &self.configuration.user_agent {
            Some(ua) => ua.as_str(),
            _ => get_ua(self.configuration.only_chrome_agent()),
        };

        // let missing_host =
        //     !headers.contains_key(crate::client::header::HOST) && !headers.contains_key("Host");
        let missing_agent = match &self.configuration.headers {
            Some(headers) => {
                !headers.contains_key(crate::client::header::USER_AGENT)
                    && !headers.contains_key("User-Agent")
            }
            _ => true,
        };

        let timeout_mult = if self.configuration.proxies.is_some() {
            2
        } else {
            1
        };

        let client = reqwest::Client::builder()
            .redirect(policy)
            .http09_responses()
            .http1_ignore_invalid_headers_in_responses(true)
            .referer(self.configuration.referer.is_none())
            .connect_timeout(
                self.configuration
                    .default_http_connect_timeout
                    .unwrap_or(Duration::from_secs(24 * timeout_mult)),
            )
            .read_timeout(
                self.configuration
                    .default_http_read_timeout
                    .unwrap_or(Duration::from_secs(42 * timeout_mult)),
            )
            .http1_title_case_headers()
            .http1_allow_obsolete_multiline_headers_in_responses(true)
            .http1_allow_spaces_after_header_name_in_responses(true)
            // .http1_preserve_header_order()
            // .http1_preserve_header_case()
            .danger_accept_invalid_certs(self.configuration.accept_invalid_certs);

        let client = if let Some(network_interface) = &self.configuration.network_interface {
            set_interface(client, &network_interface)
        } else {
            client
        };

        let client = if let Some(local_address) = &self.configuration.local_address {
            client.local_address(*local_address)
        } else {
            client
        };

        let client = if self.configuration.proxies.is_none() {
            client
        } else {
            client.tcp_keepalive(Duration::from_secs(30))
        };

        // check both casing for user-agent
        let client = if missing_agent {
            client.user_agent(user_agent)
        } else {
            client
        };

        let client = if self.configuration.http2_prior_knowledge {
            client.http2_prior_knowledge()
        } else {
            client
        };

        crate::utils::header_utils::setup_default_headers(client, &self.configuration)
    }

    #[cfg(all(feature = "wreq", not(feature = "decentralized")))]
    /// Base client configuration.
    pub fn configure_base_client(&self) -> ClientBuilder {
        let policy = self.setup_redirect_policy();

        let user_agent = match &self.configuration.user_agent {
            Some(ua) => ua.as_str(),
            _ => get_ua(self.configuration.only_chrome_agent()),
        };

        let missing_agent = match &self.configuration.headers {
            Some(headers) => {
                !headers.contains_key(crate::client::header::USER_AGENT)
                    && !headers.contains_key("User-Agent")
            }
            _ => true,
        };

        let timeout_mult = if self.configuration.proxies.is_some() {
            2
        } else {
            1
        };

        let client = Client::builder()
            .redirect(policy)
            .referer(self.configuration.referer.is_none())
            .connect_timeout(
                self.configuration
                    .default_http_connect_timeout
                    .unwrap_or(Duration::from_secs(24 * timeout_mult)),
            )
            .read_timeout(
                self.configuration
                    .default_http_read_timeout
                    .unwrap_or(Duration::from_secs(42 * timeout_mult)),
            );

        let client = if let Some(local_address) = &self.configuration.local_address {
            client.local_address(*local_address)
        } else {
            client
        };

        let client = if self.configuration.proxies.is_none() {
            client
        } else {
            client.tcp_keepalive(Duration::from_secs(30))
        };

        let client = if missing_agent {
            client.user_agent(user_agent)
        } else {
            client
        };

        let client = if let Some(emulation) = self.configuration.emulation {
            client.emulation(emulation)
        } else {
            client
        };

        crate::utils::header_utils::setup_default_headers(client, &self.configuration)
    }

    /// Build the HTTP client.
    #[cfg(all(not(feature = "decentralized"), not(feature = "cache_request")))]
    pub fn configure_http_client_builder(&self) -> ClientBuilder {
        let client = self.configure_base_client();

        let mut client = match &self.configuration.request_timeout {
            Some(t) => client.timeout(**t),
            _ => client,
        };

        let client = match &self.configuration.proxies {
            Some(proxies) => {
                let linux = cfg!(target_os = "linux");
                let ignore_plain_socks = proxies.len() >= 2 && linux;
                let replace_plain_socks = proxies.len() == 1 && linux;

                for proxie in proxies.iter() {
                    if proxie.ignore == crate::configuration::ProxyIgnore::Http {
                        continue;
                    }

                    let proxie = &proxie.addr;
                    let socks = proxie.starts_with("socks://");

                    // we can skip it and use another proxy from the list.
                    if ignore_plain_socks && socks {
                        continue;
                    }

                    // use HTTP instead as reqwest does not support the protocol on linux.
                    if replace_plain_socks && socks {
                        if let Ok(proxy) =
                            crate::client::Proxy::all(&proxie.replacen("socks://", "http://", 1))
                        {
                            client = client.proxy(proxy);
                        }
                    } else {
                        if let Ok(proxy) = crate::client::Proxy::all(proxie) {
                            client = client.proxy(proxy);
                        }
                    }
                }

                client
            }
            _ => client,
        };

        // Spider Cloud proxy injection (modes that use proxy transport)
        #[cfg(feature = "spider_cloud")]
        let client = if let Some(ref sc) = self.configuration.spider_cloud {
            if sc.uses_proxy() {
                match (crate::client::Proxy::all(&sc.proxy_url), reqwest::header::HeaderValue::from_str(&format!("Bearer {}", sc.api_key))) {
                    (Ok(proxy), Ok(auth_value)) => client.proxy(proxy.custom_http_auth(auth_value)),
                    _ => client,
                }
            } else {
                client
            }
        } else {
            client
        };

        let client = if crate::utils::connect::background_connect_threading() {
            client.connector_layer(crate::utils::connect::BackgroundProcessorLayer::new())
        } else {
            client
        };

        let client = match self.configuration.concurrency_limit {
            Some(limit) => {
                client.connector_layer(tower::limit::concurrency::ConcurrencyLimitLayer::new(limit))
            }
            _ => client,
        };

        self.configure_http_client_cookies(client)
    }

    /// Build the HTTP client with caching enabled.
    #[cfg(all(not(feature = "decentralized"), feature = "cache_request"))]
    pub fn configure_http_client_builder(&self) -> reqwest_middleware::ClientBuilder {
        use crate::utils::create_cache_key;
        let client = self.configure_base_client();

        let mut client = match &self.configuration.request_timeout {
            Some(t) => client.timeout(**t),
            _ => client,
        };

        let client = match &self.configuration.proxies {
            Some(proxies) => {
                let linux = cfg!(target_os = "linux");
                let ignore_plain_socks = proxies.len() >= 2 && linux;
                let replace_plain_socks = proxies.len() == 1 && linux;

                for proxie in proxies.iter() {
                    if proxie.ignore == crate::configuration::ProxyIgnore::Http {
                        continue;
                    }
                    let proxie = &proxie.addr;

                    let socks = proxie.starts_with("socks://");

                    // we can skip it and use another proxy from the list.
                    if ignore_plain_socks && socks {
                        continue;
                    }

                    // use HTTP instead as reqwest does not support the protocol on linux.
                    if replace_plain_socks && socks {
                        if let Ok(proxy) =
                            crate::client::Proxy::all(&proxie.replacen("socks://", "http://", 1))
                        {
                            client = client.proxy(proxy);
                        }
                    } else {
                        if let Ok(proxy) = crate::client::Proxy::all(proxie) {
                            client = client.proxy(proxy);
                        }
                    }
                }

                client
            }
            _ => client,
        };

        // Spider Cloud proxy injection (modes that use proxy transport)
        #[cfg(feature = "spider_cloud")]
        let client = if let Some(ref sc) = self.configuration.spider_cloud {
            if sc.uses_proxy() {
                match (crate::client::Proxy::all(&sc.proxy_url), reqwest::header::HeaderValue::from_str(&format!("Bearer {}", sc.api_key))) {
                    (Ok(proxy), Ok(auth_value)) => client.proxy(proxy.custom_http_auth(auth_value)),
                    _ => client,
                }
            } else {
                client
            }
        } else {
            client
        };

        let client = self.configure_http_client_cookies(client);

        let client = if crate::utils::connect::background_connect_threading() {
            client.connector_layer(crate::utils::connect::BackgroundProcessorLayer::new())
        } else {
            client
        };

        let client = match self.configuration.concurrency_limit {
            Some(limit) => {
                client.connector_layer(tower::limit::concurrency::ConcurrencyLimitLayer::new(limit))
            }
            _ => client,
        };

        let client =
            reqwest_middleware::ClientBuilder::new(unsafe { client.build().unwrap_unchecked() });

        if self.configuration.cache {
            let mut cache_options = HttpCacheOptions::default();

            cache_options.cache_key = Some(Arc::new(|req: &http::request::Parts| {
                let mut auth_token = None;
                if let Some(auth) = req.headers.get("authorization") {
                    if let Ok(token) = auth.to_str() {
                        if !token.is_empty() {
                            auth_token = Some(token);
                        }
                    }
                }
                create_cache_key(req, Some(req.method.as_str()), auth_token)
            }));
            client.with(Cache(HttpCache {
                mode: CacheMode::Default,
                manager: CACACHE_MANAGER.clone(),
                options: cache_options,
            }))
        } else {
            client
        }
    }

    /// Build the HTTP client with cookie configurations.
    #[cfg(all(not(feature = "decentralized"), feature = "cookies"))]
    pub fn configure_http_client_cookies(
        &self,
        client: crate::client::ClientBuilder,
    ) -> crate::client::ClientBuilder {
        let client = client.cookie_provider(self.cookie_jar.clone());

        if !self.configuration.cookie_str.is_empty() {
            if let Some(url) = self.domain_parsed.as_ref() {
                self.cookie_jar
                    .add_cookie_str(&self.configuration.cookie_str, url);
            }
        }

        client
    }

    /// Build the client with cookie configurations. This does nothing with [cookies] flag enabled.
    #[cfg(all(not(feature = "decentralized"), not(feature = "cookies")))]
    pub fn configure_http_client_cookies(
        &self,
        client: crate::client::ClientBuilder,
    ) -> crate::client::ClientBuilder {
        client
    }

    /// Set the HTTP client to use directly. This is helpful if you manually call 'website.configure_http_client' before the crawl.
    pub fn set_http_client(&mut self, client: Client) -> &Option<Client> {
        self.client = Some(client);
        &self.client
    }

    /// Build a client configured with a single proxy for use in rotation.
    #[cfg(all(not(feature = "decentralized"), not(feature = "cache_request")))]
    fn build_single_proxy_client(
        &self,
        proxy: &crate::configuration::RequestProxy,
    ) -> Option<Client> {
        if proxy.ignore == crate::configuration::ProxyIgnore::Http {
            return None;
        }

        let client = self.configure_base_client();

        let client = match &self.configuration.request_timeout {
            Some(t) => client.timeout(**t),
            _ => client,
        };

        let addr = &proxy.addr;
        let linux = cfg!(target_os = "linux");
        let socks = addr.starts_with("socks://");

        let client = if socks && linux {
            match crate::client::Proxy::all(&addr.replacen("socks://", "http://", 1)) {
                Ok(p) => client.proxy(p),
                Err(_) => return None,
            }
        } else {
            match crate::client::Proxy::all(addr) {
                Ok(p) => client.proxy(p),
                Err(_) => return None,
            }
        };

        #[cfg(feature = "spider_cloud")]
        let client = if let Some(ref sc) = self.configuration.spider_cloud {
            if sc.uses_proxy() {
                match (
                    crate::client::Proxy::all(&sc.proxy_url),
                    reqwest::header::HeaderValue::from_str(&format!("Bearer {}", sc.api_key)),
                ) {
                    (Ok(proxy), Ok(auth_value)) => {
                        client.proxy(proxy.custom_http_auth(auth_value))
                    }
                    _ => client,
                }
            } else {
                client
            }
        } else {
            client
        };

        let client = if crate::utils::connect::background_connect_threading() {
            client.connector_layer(crate::utils::connect::BackgroundProcessorLayer::new())
        } else {
            client
        };

        let client = match self.configuration.concurrency_limit {
            Some(limit) => client
                .connector_layer(tower::limit::concurrency::ConcurrencyLimitLayer::new(limit)),
            _ => client,
        };

        let client = self.configure_http_client_cookies(client);
        unsafe { Some(client.build().unwrap_unchecked()) }
    }

    /// Build a client configured with a single proxy for use in rotation (cache_request variant).
    #[cfg(all(not(feature = "decentralized"), feature = "cache_request"))]
    fn build_single_proxy_client(
        &self,
        proxy: &crate::configuration::RequestProxy,
    ) -> Option<Client> {
        use crate::utils::create_cache_key;

        if proxy.ignore == crate::configuration::ProxyIgnore::Http {
            return None;
        }

        let client = self.configure_base_client();

        let client = match &self.configuration.request_timeout {
            Some(t) => client.timeout(**t),
            _ => client,
        };

        let addr = &proxy.addr;
        let linux = cfg!(target_os = "linux");
        let socks = addr.starts_with("socks://");

        let client = if socks && linux {
            match crate::client::Proxy::all(&addr.replacen("socks://", "http://", 1)) {
                Ok(p) => client.proxy(p),
                Err(_) => return None,
            }
        } else {
            match crate::client::Proxy::all(addr) {
                Ok(p) => client.proxy(p),
                Err(_) => return None,
            }
        };

        #[cfg(feature = "spider_cloud")]
        let client = if let Some(ref sc) = self.configuration.spider_cloud {
            if sc.uses_proxy() {
                match (
                    crate::client::Proxy::all(&sc.proxy_url),
                    reqwest::header::HeaderValue::from_str(&format!("Bearer {}", sc.api_key)),
                ) {
                    (Ok(proxy), Ok(auth_value)) => {
                        client.proxy(proxy.custom_http_auth(auth_value))
                    }
                    _ => client,
                }
            } else {
                client
            }
        } else {
            client
        };

        let client = self.configure_http_client_cookies(client);

        let client = if crate::utils::connect::background_connect_threading() {
            client.connector_layer(crate::utils::connect::BackgroundProcessorLayer::new())
        } else {
            client
        };

        let client = match self.configuration.concurrency_limit {
            Some(limit) => client
                .connector_layer(tower::limit::concurrency::ConcurrencyLimitLayer::new(limit)),
            _ => client,
        };

        let client =
            reqwest_middleware::ClientBuilder::new(unsafe { client.build().unwrap_unchecked() });

        if self.configuration.cache {
            let mut cache_options = HttpCacheOptions::default();

            cache_options.cache_key = Some(Arc::new(|req: &http::request::Parts| {
                let mut auth_token = None;
                if let Some(auth) = req.headers.get("authorization") {
                    if let Ok(token) = auth.to_str() {
                        if !token.is_empty() {
                            auth_token = Some(token);
                        }
                    }
                }
                create_cache_key(req, Some(req.method.as_str()), auth_token)
            }));

            Some(
                client
                    .with(Cache(HttpCache {
                        mode: CacheMode::Default,
                        manager: CACACHE_MANAGER.clone(),
                        options: cache_options,
                    }))
                    .build(),
            )
        } else {
            Some(client.build())
        }
    }

    /// Build rotated clients from the proxy list. Returns None if fewer than 2 proxies.
    #[cfg(not(feature = "decentralized"))]
    fn build_rotated_clients(&self) -> Option<Arc<ClientRotator>> {
        let proxies = self.configuration.proxies.as_ref()?;
        if proxies.len() < 2 {
            return None;
        }
        let clients: Vec<Client> = proxies
            .iter()
            .filter_map(|proxy| self.build_single_proxy_client(proxy))
            .collect();
        if clients.len() < 2 {
            return None;
        }
        Some(Arc::new(ClientRotator::new(clients)))
    }

    /// Configure http client.
    #[cfg(all(not(feature = "decentralized"), not(feature = "cache_request")))]
    pub fn configure_http_client(&self) -> Client {
        let client = self.configure_http_client_builder();
        // should unwrap using native-tls-alpn
        unsafe { client.build().unwrap_unchecked() }
    }

    /// Configure http client.
    #[cfg(all(not(feature = "decentralized"), feature = "cache_request"))]
    pub fn configure_http_client(&self) -> Client {
        let client = self.configure_http_client_builder();
        client.build()
    }

    /// Configure http client for decentralization.
    #[cfg(all(feature = "decentralized", not(feature = "cache_request")))]
    pub fn configure_http_client(&self) -> Client {
        use reqwest::header::{HeaderMap, HeaderValue};

        let mut headers = HeaderMap::new();

        let policy = self.setup_redirect_policy();

        let mut client = Client::builder()
            .user_agent(match &self.configuration.user_agent {
                Some(ua) => ua.as_str(),
                _ => &get_ua(self.configuration.only_chrome_agent()),
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

        if let Some(h) = &self.configuration.headers {
            headers.extend(h.inner().clone());
        }

        if let Some(domain_url) = self.get_absolute_path(None) {
            let domain_url = domain_url.as_str();
            let domain_host = if domain_url.ends_with("/") {
                &domain_url[0..domain_url.len() - 1]
            } else {
                domain_url
            };
            if let Ok(value) = HeaderValue::from_str(domain_host) {
                headers.insert(reqwest::header::HOST, value);
            }
        }

        for worker in WORKERS.iter() {
            if let Ok(worker) = crate::client::Proxy::all(worker) {
                client = client.proxy(worker);
            }
        }

        if !self.configuration.modify_headers && self.configuration.modify_http_client_headers {
            if let Some(ua) = &self.configuration.user_agent {
                crate::utils::header_utils::extend_headers(
                    &mut headers,
                    ua,
                    &self.configuration.headers,
                    &None,
                    &self.configuration.viewport,
                    &self.domain_parsed,
                );
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
        use crate::utils::create_cache_key;
        use reqwest::header::{HeaderMap, HeaderValue};
        use reqwest_middleware::ClientBuilder;

        let mut headers = HeaderMap::new();

        let policy = self.setup_redirect_policy();

        let mut client = reqwest::Client::builder()
            .user_agent(match &self.configuration.user_agent {
                Some(ua) => ua.as_str(),
                _ => &get_ua(self.configuration.only_chrome_agent()),
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

        if let Some(h) = &self.configuration.headers {
            headers.extend(h.inner().clone());
        }

        if let Some(domain_url) = self.get_absolute_path(None) {
            let domain_url = domain_url.as_str();
            let domain_host = if domain_url.ends_with("/") {
                &domain_url[0..domain_url.len() - 1]
            } else {
                domain_url
            };
            if let Ok(value) = HeaderValue::from_str(domain_host) {
                headers.insert(reqwest::header::HOST, value);
            }
        }

        for worker in WORKERS.iter() {
            if let Ok(worker) = crate::client::Proxy::all(worker) {
                client = client.proxy(worker);
            }
        }

        let mut cache_options = HttpCacheOptions::default();

        cache_options.cache_key = Some(Arc::new(|req: &http::request::Parts| {
            let mut auth_token = None;
            if let Some(auth) = req.headers.get("authorization") {
                if let Ok(token) = auth.to_str() {
                    if !token.is_empty() {
                        auth_token = Some(token);
                    }
                }
            }
            create_cache_key(req, Some(req.method.as_str()), auth_token)
        }));

        if !self.configuration.modify_headers && self.configuration.modify_http_client_headers {
            if let Some(ua) = &self.configuration.user_agent {
                crate::utils::header_utils::extend_headers(
                    &mut headers,
                    ua,
                    &self.configuration.headers,
                    &None,
                    &self.configuration.viewport,
                    &self.domain_parsed,
                );
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
            options: cache_options,
        }));

        client.build()
    }

    /// Setup atomic controller. This does nothing without the 'control' feature flag enabled.
    #[cfg(feature = "control")]
    pub fn configure_handler(&self) -> Option<(Arc<AtomicI8>, tokio::task::JoinHandle<()>)> {
        use crate::utils::{Handler, CONTROLLER};

        if self.configuration.no_control_thread {
            None
        } else {
            let c: Arc<AtomicI8> = Arc::new(AtomicI8::new(0));
            let handle = c.clone();
            let target_id = self.target_id();

            let join_handle = crate::utils::spawn_task("control_handler", async move {
                let mut l = CONTROLLER.read().await.1.to_owned();

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

            Some((handle, join_handle))
        }
    }

    #[cfg(not(feature = "control"))]
    /// Setup atomic controller. This does nothing without the 'control' feature flag enabled.
    pub fn configure_handler(&self) -> Option<(Arc<AtomicI8>, tokio::task::JoinHandle<()>)> {
        None
    }

    /// Setup interception for chrome request.
    #[cfg(all(feature = "chrome", feature = "chrome_intercept"))]
    pub async fn setup_chrome_interception(
        &self,
        page: &chromiumoxide::Page,
    ) -> Option<tokio::task::JoinHandle<()>> {
        crate::features::chrome::setup_chrome_interception_base(
            page,
            self.configuration.chrome_intercept.enabled,
            &self.configuration.auth_challenge_response,
            self.configuration.chrome_intercept.block_visuals,
            self.url.inner(),
        )
        .await
    }

    /// Setup interception for chrome request
    #[cfg(all(feature = "chrome", not(feature = "chrome_intercept")))]
    pub async fn setup_chrome_interception(
        &self,
        _chrome_page: &chromiumoxide::Page,
    ) -> Option<tokio::task::JoinHandle<()>> {
        None
    }

    /// Setup selectors for handling link targets.
    pub fn setup_selectors(&self) -> RelativeSelectors {
        setup_website_selectors(
            self.get_url().inner(),
            AllowedDomainTypes::new(self.configuration.subdomains, self.configuration.tld),
        )
    }

    /// Base configuration setup.
    pub fn setup_base(&mut self) -> (Client, Option<(Arc<AtomicI8>, tokio::task::JoinHandle<()>)>) {
        self.determine_limits();
        self.setup_disk();
        self.configure_headers();

        crate::utils::connect::init_background_runtime();

        let client = match self.client.take() {
            Some(client) => client,
            _ => self.configure_http_client(),
        };

        #[cfg(not(feature = "decentralized"))]
        {
            self.client_rotator = self.build_rotated_clients();
        }

        (client, self.configure_handler())
    }

    /// Setup config for crawl.
    pub async fn setup(
        &mut self,
    ) -> (Client, Option<(Arc<AtomicI8>, tokio::task::JoinHandle<()>)>) {
        let setup = self.setup_base();
        if self.status != CrawlStatus::Active {
            self.clear_all().await;
        } else {
            self.skip_initial = !self.extra_links.is_empty();
        }
        self.configure_robots_parser(&setup.0).await;
        setup
    }

    /// Setup shared concurrent configs.
    pub fn setup_crawl(
        &self,
    ) -> (
        std::pin::Pin<Box<tokio::time::Interval>>,
        std::pin::Pin<Box<Duration>>,
    ) {
        let interval = Box::pin(tokio::time::interval(Duration::from_millis(10)));
        let throttle = Box::pin(self.get_delay());

        (interval, throttle)
    }

    /// Get all the expanded links.
    #[cfg(feature = "glob")]
    pub fn get_expanded_links(&self, domain_name: &str) -> Vec<CaseInsensitiveString> {
        let mut expanded = crate::features::glob::expand_url(&domain_name);

        if expanded.len() == 0 {
            if let Some(u) = self.get_absolute_path(Some(domain_name)) {
                expanded.push(u.as_str().into());
            }
        };

        expanded
    }

    /// Set the initial crawl status by page output.
    pub fn set_crawl_initial_status(
        &mut self,
        page: &crate::page::Page,
        links: &HashSet<CaseInsensitiveString>,
    ) {
        use crate::utils::{detect_open_resty_forbidden, APACHE_FORBIDDEN};

        if page.status_code == reqwest::StatusCode::FORBIDDEN && links.is_empty() {
            if is_safe_javascript_challenge(&page) {
                self.website_meta_info = WebsiteMetaInfo::RequiresJavascript;
            } else if page.get_html_bytes_u8() == *APACHE_FORBIDDEN {
                self.website_meta_info = WebsiteMetaInfo::Apache403;
            } else if detect_open_resty_forbidden(page.get_html_bytes_u8()) {
                self.website_meta_info = WebsiteMetaInfo::OpenResty403;
            }
            self.status = CrawlStatus::Blocked;
        } else if page.status_code == reqwest::StatusCode::TOO_MANY_REQUESTS {
            self.status = CrawlStatus::RateLimited;
        } else if page.status_code.is_server_error() {
            self.status = CrawlStatus::ServerError;
        } else if page.is_empty() {
            if page.status_code == *UNKNOWN_STATUS_ERROR
                || page.status_code == *CHROME_UNKNOWN_STATUS_ERROR
            {
                self.status = CrawlStatus::ConnectError;
            } else {
                self.status = CrawlStatus::Empty;
            }
        }
    }

    /// Expand links for crawl base establish using a **command-based fetch**.
    #[cfg(feature = "cmd")]
    pub async fn _crawl_establish_cmd(
        &mut self,
        cmd: std::path::PathBuf,
        cmd_args: Vec<String>,
        base: &mut RelativeSelectors,
        _ssg_build: bool,
    ) -> HashSet<CaseInsensitiveString> {
        if self.skip_initial {
            return Default::default();
        }

        if !self
            .is_allowed_default(self.get_base_link())
            .eq(&ProcessLinkStatus::Allowed)
        {
            return HashSet::new();
        }

        let url = self.url.inner();

        let mut links: HashSet<CaseInsensitiveString> = HashSet::new();
        let mut links_ssg = HashSet::new();
        let mut links_pages = if self.configuration.return_page_links {
            Some(HashSet::new())
        } else {
            None
        };

        let mut page_links_settings =
            PageLinkBuildSettings::new(true, self.configuration.full_resources);
        page_links_settings.subdomains = self.configuration.subdomains;
        page_links_settings.tld = self.configuration.tld;
        page_links_settings.normalize = self.configuration.normalize;

        let mut domain_parsed = self.domain_parsed.take();

        let mut retry_count = self.configuration.retry;
        let mut last_err: Option<std::io::Error> = None;

        let build_error_page = |status: StatusCode, _err: std::io::Error| {
            let mut p = Page::default();
            p.url = url.to_string();
            p.status_code = status;
            #[cfg(feature = "page_error_status_details")]
            {
                p.error_for_status = Some(Err(_err));
            }
            p
        };

        let mut page: Page = loop {
            let bytes = match Self::run_via_cmd(&cmd, &cmd_args, url).await {
                Ok(b) => {
                    if b.is_empty() {
                        last_err = Some(std::io::Error::new(
                            std::io::ErrorKind::UnexpectedEof,
                            "cmd returned empty stdout",
                        ));
                        None
                    } else {
                        Some(b)
                    }
                }
                Err(e) => {
                    last_err = Some(e);
                    None
                }
            };

            if let Some(bytes) = bytes.as_deref() {
                let mut domain_parsed_out = None;

                let page = Page::new_page_streaming_from_bytes(
                    url,
                    bytes,
                    base,
                    &self.configuration.external_domains_caseless,
                    &page_links_settings,
                    &mut links,
                    Some(&mut links_ssg),
                    &mut domain_parsed,
                    &mut domain_parsed_out,
                    &mut links_pages,
                )
                .await;

                if self.domain_parsed.is_none() {
                    if let Some(mut dp) = domain_parsed.take() {
                        convert_abs_url(&mut dp);
                        self.domain_parsed.replace(dp);
                    } else if let Some(mut dp) = domain_parsed_out.take() {
                        convert_abs_url(&mut dp);
                        self.domain_parsed.replace(dp);
                    }
                } else if self.domain_parsed.is_none() {
                    self.domain_parsed = domain_parsed_out;
                }

                if page.should_retry && retry_count > 0 {
                    retry_count -= 1;
                    if let Some(timeout) = page.get_timeout() {
                        tokio::time::sleep(timeout).await;
                    } else {
                        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                    }
                    continue;
                }

                break page;
            }

            if retry_count == 0 {
                let err = last_err.take().unwrap_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::Other,
                        "cmd fetch failed (unknown error)",
                    )
                });
                break build_error_page(StatusCode::BAD_GATEWAY, err);
            }

            retry_count -= 1;
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        };

        if page.get_html_bytes_u8().starts_with(b"<?xml") {
            page.links_stream_xml_links_stream_base(base, &page.get_html(), &mut links, &None)
                .await;
        }

        emit_log(url);

        if let Some(signature) = page.signature {
            if !self.is_signature_allowed(signature).await {
                return Default::default();
            }
            self.insert_signature(signature).await;
        }

        let url_ci = match &self.on_link_find_callback {
            Some(cb) => cb(*self.url.clone(), None).0,
            _ => *self.url.clone(),
        };
        self.insert_link(url_ci).await;

        if self.configuration.return_page_links {
            page.page_links = links_pages
                .filter(|pages: &HashSet<CaseInsensitiveString>| !pages.is_empty())
                .map(Box::new);
        }

        links.extend(links_ssg);

        self.initial_status_code = page.status_code;
        self.initial_html_length = page.get_html_bytes_u8().len();
        self.initial_anti_bot_tech = page.anti_bot_tech;
        self.initial_page_should_retry = page.should_retry;
        self.initial_page_waf_check = page.waf_check;

        self.set_crawl_initial_status(&page, &links);

        if let Some(ref cb) = self.on_should_crawl_callback {
            if !cb.call(&page) {
                page.blocked_crawl = true;
                channel_send_page(&self.channel, page, &self.channel_guard);
                return Default::default();
            }
        }

        channel_send_page(&self.channel, page, &self.channel_guard);

        links
    }

    /// Expand links for crawl base establish.
    #[cfg(not(feature = "glob"))]
    pub async fn _crawl_establish(
        &mut self,
        client: &Client,
        base: &mut RelativeSelectors,
        _: bool,
    ) -> HashSet<CaseInsensitiveString> {
        if self.skip_initial {
            return Default::default();
        }

        if self
            .is_allowed_default(self.get_base_link())
            .eq(&ProcessLinkStatus::Allowed)
        {
            let url = self.url.inner();

            let mut links: HashSet<CaseInsensitiveString> = HashSet::new();
            let mut links_ssg = HashSet::new();
            let mut links_pages = if self.configuration.return_page_links {
                Some(HashSet::new())
            } else {
                None
            };
            let mut page_links_settings =
                PageLinkBuildSettings::new(true, self.configuration.full_resources);

            page_links_settings.subdomains = self.configuration.subdomains;
            page_links_settings.tld = self.configuration.tld;
            page_links_settings.normalize = self.configuration.normalize;

            let mut domain_parsed = self.domain_parsed.take();

            let mut page = if let Some(mut seeded_page) = self.build_seed_page() {
                // Extract links and metadata from seeded HTML content if not binary
                #[cfg(not(feature = "decentralized"))]
                {
                    let html_bytes = seeded_page.get_html_bytes_u8();
                    if !html_bytes.is_empty() && !auto_encoder::is_binary_file(html_bytes) {
                        let html = seeded_page.get_html();
                        let extracted_links: HashSet<CaseInsensitiveString> = seeded_page
                            .links_stream_base_ssg(base, &html, client, &self.domain_parsed)
                            .await;
                        links.extend(extracted_links);
                    }
                }
                seeded_page
            } else {
                Page::new_page_streaming(
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
                .await
            };

            if page.get_html_bytes_u8().starts_with(b"<?xml") {
                page.links_stream_xml_links_stream_base(base, &page.get_html(), &mut links, &None)
                    .await;
            }

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

            if let Some(signature) = page.signature {
                if !self.is_signature_allowed(signature).await {
                    return Default::default();
                }
                self.insert_signature(signature).await;
            }

            let url = match &self.on_link_find_callback {
                Some(cb) => cb(*self.url.clone(), None).0,
                _ => *self.url.clone(),
            };

            self.insert_link(url).await;

            if self.configuration.return_page_links {
                page.page_links = links_pages
                    .filter(|pages: &HashSet<CaseInsensitiveString>| !pages.is_empty())
                    .map(Box::new);
            }

            links.extend(links_ssg);

            self.initial_status_code = page.status_code;
            self.initial_html_length = page.get_html_bytes_u8().len();
            self.initial_anti_bot_tech = page.anti_bot_tech;
            self.initial_page_should_retry = page.should_retry;
            self.initial_page_waf_check = page.waf_check;

            self.set_crawl_initial_status(&page, &links);

            if let Some(ref cb) = self.on_should_crawl_callback {
                if !cb.call(&page) {
                    page.blocked_crawl = true;
                    channel_send_page(&self.channel, page, &self.channel_guard);
                    return Default::default();
                }
            }

            channel_send_page(&self.channel, page, &self.channel_guard);

            links
        } else {
            HashSet::new()
        }
    }

    /// Run `cmd` and return stdout bytes.
    #[cfg(feature = "cmd")]
    pub async fn run_via_cmd(
        cmd: &std::path::Path,
        fixed_args: &[String],
        url: &str,
    ) -> std::io::Result<Vec<u8>> {
        use tokio::process::Command;
        let mut args: Vec<String> = Vec::with_capacity(fixed_args.len() + 1);
        let mut used_placeholder = false;

        for a in fixed_args {
            if a.contains("{url}") {
                used_placeholder = true;
                args.push(a.replace("{url}", url));
            } else {
                args.push(a.clone());
            }
        }

        if !used_placeholder {
            args.push(url.to_string());
        }

        let out = Command::new(cmd)
            .args(&args)
            .kill_on_drop(true)
            .output()
            .await?;

        if !out.status.success() {
            let code = out.status.code().unwrap_or(-1);
            let stderr = String::from_utf8_lossy(&out.stderr);

            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("cmd exit={code} stderr={stderr}"),
            ));
        }

        Ok(out.stdout)
    }

    /// Start to crawl website concurrently using a cmd executable.
    /// - `cmd` is the executable (absolute preferred)
    /// - `cmd_args` are fixed args; can include "{url}" placeholder, otherwise url is appended.
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    #[cfg(feature = "cmd")]
    pub async fn crawl_concurrent_cmd(
        &mut self,
        cmd: std::path::PathBuf,
        cmd_args: Vec<String>,
        handle: &Option<Arc<AtomicI8>>,
    ) {
        self.start();
        self.status = CrawlStatus::Active;

        let mut selector: (
            CompactString,
            smallvec::SmallVec<[CompactString; 2]>,
            CompactString,
        ) = self.setup_selectors();

        if self.single_page() {
            let mut links: HashSet<CaseInsensitiveString> = HashSet::new();
            let mut links_pages: Option<HashSet<CaseInsensitiveString>> =
                if self.configuration.return_page_links {
                    Some(HashSet::new())
                } else {
                    None
                };

            let mut relative_selectors = selector;
            let mut domain_parsed = None;

            let target = self
                .domain_parsed
                .as_ref()
                .map(|u| u.as_str())
                .unwrap_or(self.get_url());

            let bytes = match Self::run_via_cmd(&cmd, &cmd_args, target).await {
                Ok(b) => b,
                Err(e) => {
                    let mut page = Page::default();
                    page.url = target.to_string();
                    page.status_code = StatusCode::BAD_GATEWAY;
                    #[cfg(feature = "page_error_status_details")]
                    {
                        page.error_for_status = Some(Err(e));
                    }
                    channel_send_page(&self.channel, page, &self.channel_guard);
                    return;
                }
            };

            let page = Page::new_page_streaming_from_bytes(
                target,
                &bytes,
                &mut relative_selectors,
                &self.configuration.external_domains_caseless,
                &PageLinkBuildSettings::new_full(
                    false,
                    self.configuration.full_resources,
                    self.configuration.subdomains,
                    self.configuration.tld,
                    self.configuration.normalize,
                ),
                &mut links,
                None,
                &self.domain_parsed,
                &mut domain_parsed,
                &mut links_pages,
            )
            .await;

            channel_send_page(&self.channel, page, &self.channel_guard);
            return;
        }

        let on_should_crawl_callback = self.on_should_crawl_callback.clone();
        let return_page_links = self.configuration.return_page_links;
        let full_resources = self.configuration.full_resources;
        let mut q = self.channel_queue.as_ref().map(|q| q.0.subscribe());

        let (mut interval, throttle) = self.setup_crawl();
        let mut links: HashSet<CaseInsensitiveString> = self.drain_extra_links().collect();

        links.extend(
            self._crawl_establish_cmd(cmd.clone(), cmd_args.clone(), &mut selector, false)
                .await,
        );

        self.configuration.configure_allowlist();
        let semaphore = self.setup_semaphore();

        let shared = Arc::new((
            cmd,
            cmd_args,
            selector,
            self.channel.clone(),
            self.configuration.external_domains_caseless.clone(),
            self.channel_guard.clone(),
            self.configuration.retry,
            return_page_links,
            PageLinkBuildSettings::new_full(
                false,
                full_resources,
                self.configuration.subdomains,
                self.configuration.tld,
                self.configuration.normalize,
            ),
            self.domain_parsed.clone(),
            self.on_link_find_callback.clone(),
        ));

        let mut set: JoinSet<(HashSet<CaseInsensitiveString>, Option<u64>)> = JoinSet::new();

        let mut exceeded_budget = false;
        let concurrency = throttle.is_zero();

        self.dequeue(&mut q, &mut links, &mut exceeded_budget).await;

        if !concurrency && !links.is_empty() {
            tokio::time::sleep(*throttle).await;
        }

        let crawl_breaker = if self.configuration.crawl_timeout.is_some() {
            Some(Instant::now())
        } else {
            None
        };

        'outer: loop {
            #[cfg(all(feature = "agent", feature = "serde"))]
            self.apply_url_prefilter(&mut links).await;

            let mut stream =
                tokio_stream::iter::<HashSet<CaseInsensitiveString>>(links.drain().collect());

            loop {
                if !concurrency {
                    tokio::time::sleep(*throttle).await;
                }

                let semaphore = get_semaphore(&semaphore, !self.configuration.shared_queue).await;

                tokio::select! {
                    biased;

                    Some(link) = stream.next(),
                    if semaphore.available_permits() > 0
                        && !crawl_duration_expired(&self.configuration.crawl_timeout, &crawl_breaker) =>
                    {
                        if !self.handle_process(handle, &mut interval, async {
                            emit_log_shutdown(link.inner());
                            let permits = set.len();
                            set.shutdown().await;
                            semaphore.add_permits(permits);
                        }).await {
                            while let Some(links) = stream.next().await {
                                self.extra_links.insert(links);
                            }
                            break 'outer;
                        }

                        let allowed = self.is_allowed(&link);
                        if allowed.eq(&ProcessLinkStatus::BudgetExceeded) {
                            exceeded_budget = true;
                            break;
                        }
                        if allowed.eq(&ProcessLinkStatus::Blocked) || !self.is_allowed_disk(&link).await {
                            continue;
                        }

                        emit_log(link.inner());
                        self.insert_link(link.clone()).await;

                        if let Ok(permit) = semaphore.clone().acquire_owned().await {
                            let shared = shared.clone();
                            let on_should_crawl_callback = on_should_crawl_callback.clone();
                            spawn_set("page_fetch_cmd", &mut set, async move {
                                let link_result = match &shared.10 {
                                    Some(cb) => cb(link, None),
                                    _ => (link, None),
                                };

                                let mut out_links: HashSet<CaseInsensitiveString> = HashSet::new();
                                let mut links_pages = if shared.7 { Some(HashSet::new()) } else { None };

                                let mut relative_selectors = shared.2.clone();
                                let mut r_settings = shared.8;
                                r_settings.ssg_build = true;

                                let target_url = link_result.0.as_ref();

                                // Run cmd -> bytes with retry
                                let mut retry_count = shared.6;
                                let mut last_err: Option<std::io::Error> = None;

                                let bytes = loop {
                                    match Self::run_via_cmd(&shared.0, &shared.1, target_url).await {
                                        Ok(b) if !b.is_empty() => break Some(b),
                                        Ok(_) => {
                                            last_err = Some(std::io::Error::new(
                                                std::io::ErrorKind::UnexpectedEof,
                                                "cmd returned empty stdout",
                                            ));
                                        }
                                        Err(e) => {
                                            last_err = Some(e);
                                        }
                                    }

                                    if retry_count == 0 { break None; }
                                    retry_count -= 1;

                                    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                                };

                                let mut domain_parsed = None;

                                let mut page = if let Some(bytes) = bytes {
                                    Page::new_page_streaming_from_bytes(
                                        target_url,
                                        &bytes,
                                        &mut relative_selectors,
                                        &shared.4,
                                        &r_settings,
                                        &mut out_links,
                                        None,
                                        &shared.9,
                                        &mut domain_parsed,
                                        &mut links_pages,
                                    ).await
                                } else {
                                    // Build an error page
                                    let mut p = Page::default();
                                    p.url = target_url.to_string();
                                    p.status_code = StatusCode::BAD_GATEWAY;
                                    if let Some(e) = last_err {
                                                    #[cfg(feature = "page_error_status_details")]
                                                    {
                                                        p.error_for_status = Some(Err(e));
                                                    }
                                    }
                                    p
                                };

                                if shared.7 {
                                    page.page_links = links_pages
                                        .filter(|pages| !pages.is_empty())
                                        .map(Box::new);
                                }

                                if let Some(ref cb) = on_should_crawl_callback {
                                    if !cb.call(&page) {
                                        page.blocked_crawl = true;
                                        channel_send_page(&shared.3, page, &shared.5);
                                        drop(permit);
                                        return Default::default();
                                    }
                                }

                                let signature = page.signature;
                                channel_send_page(&shared.3, page, &shared.5);
                                drop(permit);

                                (out_links, signature)
                            });
                        }

                        self.dequeue(&mut q, &mut links, &mut exceeded_budget).await;
                    },

                    Some(result) = set.join_next(), if !set.is_empty() => {
                        if let Ok(res) = result {
                            match res.1 {
                                Some(signature) => {
                                    if self.is_signature_allowed(signature).await {
                                        self.insert_signature(signature).await;
                                        self.links_visited.extend_links(&mut links, res.0);
                                    }
                                }
                                _ => {
                                    self.links_visited.extend_links(&mut links, res.0);
                                }
                            }
                        } else {
                            break;
                        }
                    }

                    else => break,
                }

                self.dequeue(&mut q, &mut links, &mut exceeded_budget).await;

                if (links.is_empty() && set.is_empty()) || exceeded_budget {
                    if exceeded_budget {
                        while let Some(links) = stream.next().await {
                            self.extra_links.insert(links);
                        }
                        while let Some(links) = set.join_next().await {
                            if let Ok(links) = links {
                                self.extra_links.extend(links.0);
                            }
                        }
                    }
                    break 'outer;
                }
            }

            self.subscription_guard().await;
            self.dequeue(&mut q, &mut links, &mut exceeded_budget).await;

            if links.is_empty() && set.is_empty() {
                break;
            }
        }

        if !links.is_empty() {
            self.extra_links.extend(links);
        }
    }

    /// Build a page from a seed.
    #[allow(dead_code)]
    fn build_seed_page(&self) -> Option<Page> {
        if let Some(seeded_html) = self.get_seeded_html() {
            let mut page_response = PageResponse::default();
            page_response.content = Some(Box::new(seeded_html.as_bytes().to_vec()));
            Some(build(&self.url.inner(), page_response))
        } else {
            None
        }
    }

    /// Expand links for crawl.
    #[cfg(all(
        not(feature = "decentralized"),
        feature = "chrome",
        not(feature = "glob")
    ))]
    pub async fn crawl_establish(
        &mut self,
        client: &Client,
        base: &mut RelativeSelectors,
        _: bool,
        chrome_page: &chromiumoxide::Page,
    ) -> HashSet<CaseInsensitiveString> {
        if self.skip_initial {
            return Default::default();
        }

        if self
            .is_allowed_default(&self.get_base_link())
            .eq(&ProcessLinkStatus::Allowed)
        {
            let (_, intercept_handle) = tokio::join!(
                crate::features::chrome::setup_chrome_events(chrome_page, &self.configuration),
                self.setup_chrome_interception(&chrome_page)
            );

            let mut page = if let Some(seeded_html) = self.get_seeded_html() {
                Page::new_seeded(
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
                    &self.configuration.track_events,
                    self.configuration.referer.clone(),
                    self.configuration.max_page_bytes,
                    self.configuration.get_cache_options(),
                    &self.configuration.cache_policy,
                    Some(seeded_html.clone()),
                    Some(&self.cookie_jar),
                    &self.configuration.remote_multimodal,
                )
                .await
            } else {
                Page::new(
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
                    &self.configuration.track_events,
                    self.configuration.referer.clone(),
                    self.configuration.max_page_bytes,
                    self.configuration.get_cache_options(),
                    &self.configuration.cache_policy,
                    &self.configuration.remote_multimodal,
                )
                .await
            };

            let mut retry_count = self.configuration.retry;

            if let Some(final_redirect_destination) = &page.final_redirect_destination {
                if final_redirect_destination == "chrome-error://chromewebdata/"
                    && page.status_code.is_success()
                    && page.is_empty()
                    && self.configuration.proxies.is_some()
                {
                    page.error_status = Some("Invalid proxy configuration.".into());
                    page.should_retry = true;
                    page.status_code = *crate::page::CHROME_UNKNOWN_STATUS_ERROR;
                }
            }

            while page.should_retry && retry_count > 0 {
                retry_count -= 1;
                if let Some(timeout) = page.get_timeout() {
                    tokio::time::sleep(timeout).await;
                }
                if page.status_code == StatusCode::GATEWAY_TIMEOUT {
                    if let Err(elasped) = tokio::time::timeout(BACKOFF_MAX_DURATION, async {
                        let next_page = Page::new(
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
                            &self.configuration.track_events,
                            self.configuration.referer.clone(),
                            self.configuration.max_page_bytes,
                            self.configuration.get_cache_options(),
                            &self.configuration.cache_policy,
                            &self.configuration.remote_multimodal,
                        )
                        .await;
                        page.clone_from(&next_page);
                    })
                    .await
                    {
                        log::warn!("backoff timeout {elasped}");
                    }
                } else {
                    let next_page = Page::new(
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
                        &self.configuration.track_events,
                        self.configuration.referer.clone(),
                        self.configuration.max_page_bytes,
                        self.configuration.get_cache_options(),
                        &self.configuration.cache_policy,
                        &self.configuration.remote_multimodal,
                    )
                    .await;
                    page.clone_from(&next_page);
                }

                // check the page again for final.
                if let Some(final_redirect_destination) = &page.final_redirect_destination {
                    if final_redirect_destination == "chrome-error://chromewebdata/"
                        && page.status_code.is_success()
                        && page.is_empty()
                        && self.configuration.proxies.is_some()
                    {
                        page.error_status = Some("Invalid proxy configuration.".into());
                        page.should_retry = true;
                        page.status_code = *crate::page::CHROME_UNKNOWN_STATUS_ERROR;
                    }
                }
            }

            if let Some(h) = intercept_handle {
                let abort_handle = h.abort_handle();
                if let Err(elasped) =
                    tokio::time::timeout(tokio::time::Duration::from_secs(10), h).await
                {
                    log::warn!("Handler timeout exceeded {elasped}");
                    abort_handle.abort();
                }
            }

            if let Some(domain) = &page.final_redirect_destination {
                let domain: Box<CaseInsensitiveString> = CaseInsensitiveString::new(&domain).into();
                let prior_domain = self.domain_parsed.take();
                self.domain_parsed = parse_absolute_url(&domain);
                self.url = domain;

                let s = self.setup_selectors();
                base.0 = s.0;
                base.1 = s.1;

                if let Some(pdname) = prior_domain {
                    if let Some(dname) = pdname.host_str() {
                        base.2 = dname.into();
                    }
                }
            }

            emit_log(&self.url.inner());

            if let Some(sid) = page.signature {
                self.insert_signature(sid).await;
            }

            let url = match &self.on_link_find_callback {
                Some(cb) => cb(*self.url.clone(), None).0,
                _ => *self.url.clone(),
            };

            self.insert_link(url).await;

            // setup link tracking.
            if self.configuration.return_page_links && page.page_links.is_none() {
                page.page_links = Some(Box::new(Default::default()));
            }

            let xml_file = page.get_html_bytes_u8().starts_with(b"<?xml");

            let mut links = if !page.is_empty() && !xml_file {
                page.links_ssg(&base, &client, &self.domain_parsed).await
            } else {
                Default::default()
            };

            if xml_file {
                page.links_stream_xml_links_stream_base(base, &page.get_html(), &mut links, &None)
                    .await;
            }

            self.initial_status_code = page.status_code;
            self.initial_html_length = page.get_html_bytes_u8().len();
            self.initial_anti_bot_tech = page.anti_bot_tech;
            self.initial_page_should_retry = page.should_retry;
            self.initial_page_waf_check = page.waf_check;

            self.set_crawl_initial_status(&page, &links);

            if let Some(ref cb) = self.on_should_crawl_callback {
                if !cb.call(&page) {
                    page.blocked_crawl = true;
                    channel_send_page(&self.channel, page, &self.channel_guard);
                    return Default::default();
                }
            }

            channel_send_page(&self.channel, page, &self.channel_guard);

            links
        } else {
            HashSet::new()
        }
    }

    /// Expand links for crawl.
    #[cfg(all(not(feature = "decentralized"), feature = "chrome",))]
    pub async fn crawl_establish_chrome_one(
        &self,
        client: &Client,
        base: &mut RelativeSelectors,
        url: &Option<&str>,
        chrome_page: &chromiumoxide::Page,
    ) -> HashSet<CaseInsensitiveString> {
        if self
            .is_allowed_default(&self.get_base_link())
            .eq(&ProcessLinkStatus::Allowed)
        {
            let (_, intercept_handle) = tokio::join!(
                crate::features::chrome::setup_chrome_events(chrome_page, &self.configuration),
                self.setup_chrome_interception(&chrome_page)
            );

            let mut page = Page::new(
                url.unwrap_or(&self.url.inner()),
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
                &self.configuration.track_events,
                self.configuration.referer.clone(),
                self.configuration.max_page_bytes,
                self.configuration.get_cache_options(),
                &self.configuration.cache_policy,
                &self.configuration.remote_multimodal,
            )
            .await;

            let mut retry_count = self.configuration.retry;

            if let Some(final_redirect_destination) = &page.final_redirect_destination {
                if final_redirect_destination == "chrome-error://chromewebdata/"
                    && page.status_code.is_success()
                    && page.is_empty()
                    && self.configuration.proxies.is_some()
                {
                    page.error_status = Some("Invalid proxy configuration.".into());
                    page.should_retry = true;
                    page.status_code = *crate::page::CHROME_UNKNOWN_STATUS_ERROR;
                }
            }

            while page.should_retry && retry_count > 0 {
                retry_count -= 1;
                if let Some(timeout) = page.get_timeout() {
                    tokio::time::sleep(timeout).await;
                }
                if page.status_code == StatusCode::GATEWAY_TIMEOUT {
                    if let Err(elasped) = tokio::time::timeout(BACKOFF_MAX_DURATION, async {
                        let next_page = Page::new(
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
                            &self.configuration.track_events,
                            self.configuration.referer.clone(),
                            self.configuration.max_page_bytes,
                            self.configuration.get_cache_options(),
                            &self.configuration.cache_policy,
                            &self.configuration.remote_multimodal,
                        )
                        .await;
                        page.clone_from(&next_page);
                    })
                    .await
                    {
                        log::warn!("backoff timeout {elasped}");
                    }
                } else {
                    let next_page = Page::new(
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
                        &self.configuration.track_events,
                        self.configuration.referer.clone(),
                        self.configuration.max_page_bytes,
                        self.configuration.get_cache_options(),
                        &self.configuration.cache_policy,
                        &self.configuration.remote_multimodal,
                    )
                    .await;
                    page.clone_from(&next_page);
                }

                // check the page again for final.
                if let Some(final_redirect_destination) = &page.final_redirect_destination {
                    if final_redirect_destination == "chrome-error://chromewebdata/"
                        && page.status_code.is_success()
                        && page.is_empty()
                        && self.configuration.proxies.is_some()
                    {
                        page.error_status = Some("Invalid proxy configuration.".into());
                        page.should_retry = true;
                        page.status_code = *crate::page::CHROME_UNKNOWN_STATUS_ERROR;
                    }
                }
            }

            if let Some(h) = intercept_handle {
                let abort_handle = h.abort_handle();
                if let Err(elasped) =
                    tokio::time::timeout(tokio::time::Duration::from_secs(10), h).await
                {
                    log::warn!("Handler timeout exceeded {elasped}");
                    abort_handle.abort();
                }
            }

            if let Some(domain) = &page.final_redirect_destination {
                let domain: Box<CaseInsensitiveString> = CaseInsensitiveString::new(&domain).into();
                let s = self.setup_selectors();

                base.0 = s.0;
                base.1 = s.1;

                if let Some(pdname) = parse_absolute_url(&domain) {
                    if let Some(dname) = pdname.host_str() {
                        base.2 = dname.into();
                    }
                }
            }

            emit_log(&self.url.inner());

            if self.configuration.return_page_links && page.page_links.is_none() {
                page.page_links = Some(Box::new(Default::default()));
            }

            let xml_file = page.get_html_bytes_u8().starts_with(b"<?xml");

            let mut links = if !page.is_empty() && !xml_file {
                page.links_ssg(&base, &client, &self.domain_parsed).await
            } else {
                Default::default()
            };

            if xml_file {
                page.links_stream_xml_links_stream_base(base, &page.get_html(), &mut links, &None)
                    .await;
            }

            if let Some(ref cb) = self.on_should_crawl_callback {
                if !cb.call(&page) {
                    page.blocked_crawl = true;
                    channel_send_page(&self.channel, page, &self.channel_guard);
                    return Default::default();
                }
            }

            channel_send_page(&self.channel, page, &self.channel_guard);

            links
        } else {
            HashSet::new()
        }
    }

    /// Expand links for crawl using WebDriver.
    #[cfg(all(feature = "webdriver", not(feature = "decentralized"), not(feature = "chrome")))]
    pub async fn crawl_establish_webdriver_one(
        &self,
        client: &Client,
        base: &mut RelativeSelectors,
        url: &Option<&str>,
        driver: &std::sync::Arc<thirtyfour::WebDriver>,
    ) -> HashSet<CaseInsensitiveString> {
        if self
            .is_allowed_default(&self.get_base_link())
            .eq(&ProcessLinkStatus::Allowed)
        {
            let timeout = self
                .configuration
                .webdriver_config
                .as_ref()
                .and_then(|c| c.timeout);

            // Setup stealth events
            crate::features::webdriver::setup_driver_events(driver, &self.configuration).await;

            let mut page = Page::new_page_webdriver(
                url.unwrap_or(&self.url.inner()),
                driver,
                timeout,
            )
            .await;

            let mut retry_count = self.configuration.retry;

            while page.should_retry && retry_count > 0 {
                retry_count -= 1;
                if let Some(timeout_duration) = page.get_timeout() {
                    tokio::time::sleep(timeout_duration).await;
                }
                if page.status_code == StatusCode::GATEWAY_TIMEOUT {
                    if let Err(elapsed) = tokio::time::timeout(BACKOFF_MAX_DURATION, async {
                        let next_page = Page::new_page_webdriver(
                            &self.url.inner(),
                            driver,
                            timeout,
                        )
                        .await;
                        page.clone_from(&next_page);
                    })
                    .await
                    {
                        log::warn!("backoff timeout {elapsed}");
                    }
                } else {
                    let next_page = Page::new_page_webdriver(
                        &self.url.inner(),
                        driver,
                        timeout,
                    )
                    .await;
                    page.clone_from(&next_page);
                }
            }

            if let Some(domain) = &page.final_redirect_destination {
                let domain: Box<CaseInsensitiveString> = CaseInsensitiveString::new(&domain).into();
                let s = self.setup_selectors();

                base.0 = s.0;
                base.1 = s.1;

                if let Some(pdname) = parse_absolute_url(&domain) {
                    if let Some(dname) = pdname.host_str() {
                        base.2 = dname.into();
                    }
                }
            }

            emit_log(&self.url.inner());

            if self.configuration.return_page_links && page.page_links.is_none() {
                page.page_links = Some(Box::new(Default::default()));
            }

            let xml_file = page.get_html_bytes_u8().starts_with(b"<?xml");

            let mut links = if !page.is_empty() && !xml_file {
                page.links_ssg(&base, &client, &self.domain_parsed).await
            } else {
                Default::default()
            };

            if xml_file {
                page.links_stream_xml_links_stream_base(base, &page.get_html(), &mut links, &None)
                    .await;
            }

            if let Some(ref cb) = self.on_should_crawl_callback {
                if !cb.call(&page) {
                    page.blocked_crawl = true;
                    channel_send_page(&self.channel, page, &self.channel_guard);
                    return Default::default();
                }
            }

            channel_send_page(&self.channel, page, &self.channel_guard);

            links
        } else {
            HashSet::new()
        }
    }

    /// Expand links for crawl.
    #[cfg(all(not(feature = "glob"), feature = "decentralized"))]
    pub async fn crawl_establish(
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

            if let Some(sid) = page.signature {
                self.insert_signature(sid).await;
            }

            self.insert_link(match &self.on_link_find_callback {
                Some(cb) => cb(*self.url.to_owned(), None).0,
                _ => *self.url.to_owned(),
            })
            .await;

            self.initial_status_code = page.status_code;
            self.initial_html_length = page.get_html_bytes_u8().len();
            self.initial_anti_bot_tech = page.anti_bot_tech;
            self.initial_page_should_retry = page.should_retry;
            self.initial_page_waf_check = page.waf_check;

            // todo: pass full links to the worker to return.
            if self.configuration.return_page_links {
                page.page_links = Some(page.links.clone().into());
            }

            let links = HashSet::from(page.links.clone());

            self.set_crawl_initial_status(&page, &links);

            channel_send_page(&self.channel, page, &self.channel_guard);

            links
        } else {
            HashSet::new()
        };

        links
    }

    /// Expand links for crawl.
    #[cfg(all(feature = "glob", feature = "decentralized"))]
    pub async fn crawl_establish(
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
            if allowed.eq(&ProcessLinkStatus::Blocked) || !self.is_allowed_disk(&link).await {
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

            let link_result = match &self.on_link_find_callback {
                Some(cb) => cb(u, None),
                _ => (u, None),
            };

            if let Some(sid) = page.signature {
                self.insert_signature(sid).await;
            }

            self.insert_link(link_result.0).await;

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
    pub async fn crawl_establish(
        &mut self,
        client: &Client,
        base: &mut RelativeSelectors,
        _: bool,
        page: &chromiumoxide::Page,
    ) -> HashSet<CaseInsensitiveString> {
        if self.skip_initial {
            return Default::default();
        }
        let mut links: HashSet<CaseInsensitiveString> = HashSet::new();
        let expanded = self.get_expanded_links(&self.url.inner().as_str());
        self.configuration.configure_allowlist();

        for link in expanded {
            let allowed = self.is_allowed(&link);

            if allowed.eq(&ProcessLinkStatus::BudgetExceeded) {
                break;
            }
            if allowed.eq(&ProcessLinkStatus::Blocked) || !self.is_allowed_disk(&link).await {
                continue;
            }

            let mut page = Page::new(
                &link.inner().as_str(),
                &client,
                &page,
                &self.configuration.wait_for,
                &self.configuration.screenshot,
                false, // we use the initial about:blank page.
                &self.configuration.openai_config,
                &self.configuration.execution_scripts,
                &self.configuration.automation_scripts,
                &self.configuration.viewport,
                &self.configuration.request_timeout,
                &self.configuration.track_events,
                self.configuration.referer.clone(),
                self.configuration.max_page_bytes,
                self.configuration.get_cache_options(),
                &self.configuration.cache_policy,
                &self.configuration.remote_multimodal,
            )
            .await;

            let u = page.get_url();
            let u = if u.is_empty() { link } else { u.into() };

            let link_result = match &self.on_link_find_callback {
                Some(cb) => cb(u, None),
                _ => (u, None),
            };

            if let Some(sid) = page.signature {
                self.insert_signature(sid).await;
            }

            self.insert_link(link_result.0).await;

            if self.configuration.return_page_links {
                page.page_links = Some(Default::default());
                let next_links = HashSet::from(page.links(&base, &self.domain_parsed).await);

                channel_send_page(&self.channel, page.clone(), &self.channel_guard);

                links.extend(next_links);
            } else {
                channel_send_page(&self.channel, page.clone(), &self.channel_guard);
                let next_links = HashSet::from(page.links(&base, &self.domain_parsed).await);

                links.extend(next_links);
            }
        }

        links
    }

    /// Expand links for crawl.
    #[cfg(feature = "glob")]
    async fn _crawl_establish(
        &mut self,
        client: &Client,
        base: &mut RelativeSelectors,
        _: bool,
    ) -> HashSet<CaseInsensitiveString> {
        if self.skip_initial {
            return Default::default();
        }
        let mut links: HashSet<CaseInsensitiveString> = HashSet::new();
        let domain_name = self.url.inner();
        let expanded = self.get_expanded_links(&domain_name.as_str());

        self.configuration.configure_allowlist();

        for url in expanded {
            #[cfg(feature = "regex")]
            let url_ref: &CaseInsensitiveString = &url;
            #[cfg(not(feature = "regex"))]
            let url_ref: &CompactString = url.inner();
            if self
                .is_allowed_default(url_ref)
                .eq(&ProcessLinkStatus::Allowed)
            {
                let mut links_ssg = HashSet::new();
                let mut links_pages = if self.configuration.return_page_links {
                    Some(HashSet::new())
                } else {
                    None
                };
                let mut page_links_settings =
                    PageLinkBuildSettings::new(true, self.configuration.full_resources);

                page_links_settings.subdomains = self.configuration.subdomains;
                page_links_settings.tld = self.configuration.tld;
                page_links_settings.normalize = self.configuration.normalize;

                let mut domain_parsed = self.domain_parsed.take();

                let mut page = Page::new_page_streaming(
                    &url,
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
                                    &url,
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
                                &url,
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

                emit_log(&url);

                if let Some(signature) = page.signature {
                    if !self.is_signature_allowed(signature).await {
                        return Default::default();
                    }
                    self.insert_signature(signature).await;
                }

                self.insert_link(
                    self.on_link_find_callback
                        .as_ref()
                        .map(|cb| cb(*self.url.clone(), None).0)
                        .unwrap_or_else(|| *self.url.clone()),
                )
                .await;

                if self.configuration.return_page_links {
                    page.page_links = links_pages.filter(|pages| !pages.is_empty()).map(Box::new);
                }

                links.extend(links_ssg);

                self.initial_status_code = page.status_code;
                self.initial_html_length = page.get_html_bytes_u8().len();
                self.initial_anti_bot_tech = page.anti_bot_tech;
                self.initial_page_should_retry = page.should_retry;
                self.initial_page_waf_check = page.waf_check;

                self.set_crawl_initial_status(&page, &links);

                if let Some(ref cb) = self.on_should_crawl_callback {
                    if !cb.call(&page) {
                        page.blocked_crawl = true;
                        channel_send_page(&self.channel, page, &self.channel_guard);
                        return Default::default();
                    }
                }

                channel_send_page(&self.channel, page, &self.channel_guard);
            }
        }

        links
    }

    /// Expand links for crawl.
    #[cfg(all(not(feature = "decentralized"), feature = "smart"))]
    pub async fn crawl_establish_smart(
        &mut self,
        client: &Client,
        mut base: &mut RelativeSelectors,
        browser: &crate::features::chrome::OnceBrowser,
    ) -> HashSet<CaseInsensitiveString> {
        if self.skip_initial {
            return Default::default();
        }

        let links: HashSet<CaseInsensitiveString> = if self
            .is_allowed_default(&self.get_base_link())
            .eq(&ProcessLinkStatus::Allowed)
        {
            let url = self.url.inner();

            let mut page = if let Some(seeded_page) = self.build_seed_page() {
                seeded_page
            } else {
                Page::new_page(&url, &client).await
            };

            let mut retry_count = self.configuration.retry;

            while page.should_retry && retry_count > 0 {
                retry_count -= 1;
                if let Some(timeout) = page.get_timeout() {
                    tokio::time::sleep(timeout).await;
                }
                let client_error = page.status_code.is_client_error();

                if page.status_code == StatusCode::GATEWAY_TIMEOUT {
                    if let Err(elasped) = tokio::time::timeout(BACKOFF_MAX_DURATION, async {
                        if retry_count.is_power_of_two() {
                            Website::render_chrome_page(
                                &self.configuration,
                                client,
                                &mut page,
                                url,
                                &self.domain_parsed,
                                browser,
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
                    if retry_count.is_power_of_two() || client_error {
                        Website::render_chrome_page(
                            &self.configuration,
                            client,
                            &mut page,
                            url,
                            &self.domain_parsed,
                            browser,
                        )
                        .await
                    } else {
                        page.clone_from(&Page::new_page(url, &client).await);
                    }
                }
            }

            let (page_links, bytes_transferred): (HashSet<CaseInsensitiveString>, Option<f64>) =
                page.smart_links(
                    &base,
                    &self.configuration,
                    &self.domain_parsed,
                    &browser,
                    Some(&self.cookie_jar),
                )
                .await;

            if let Some(domain) = &page.final_redirect_destination {
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

            if let Some(sid) = page.signature {
                self.insert_signature(sid).await;
            }

            self.insert_link(
                self.on_link_find_callback
                    .as_ref()
                    .map(|cb| cb(*self.url.clone(), None).0)
                    .unwrap_or_else(|| *self.url.clone()),
            )
            .await;

            let links = if !page_links.is_empty() {
                page_links
            } else {
                Default::default()
            };

            page.bytes_transferred = bytes_transferred;

            self.initial_status_code = page.status_code;
            self.initial_html_length = page.get_html_bytes_u8().len();
            self.initial_anti_bot_tech = page.anti_bot_tech;
            self.initial_page_should_retry = page.should_retry;
            self.initial_page_waf_check = page.waf_check;

            self.set_crawl_initial_status(&page, &links);

            if self.configuration.return_page_links {
                page.page_links = if links.is_empty() {
                    None
                } else {
                    Some(Box::new(links.clone()))
                };
            }

            if let Some(cb) = &mut self.on_should_crawl_callback {
                if !cb.call(&page) {
                    page.blocked_crawl = true;
                    channel_send_page(&self.channel, page, &self.channel_guard);
                    return Default::default();
                }
            }

            channel_send_page(&self.channel, page, &self.channel_guard);

            links
        } else {
            HashSet::new()
        };

        links
    }

    /// fetch the page with chrome
    #[cfg(all(not(feature = "decentralized"), feature = "smart"))]
    pub async fn render_chrome_page(
        config: &Configuration,
        client: &Client,
        page: &mut Page,
        url: &str,
        base: &Option<Box<Url>>,
        browser: &crate::features::chrome::OnceBrowser,
    ) {
        if let Some(browser_controller) = browser
            .get_or_init(|| crate::website::Website::setup_browser_base(&config, &base, None))
            .await
        {
            if let Ok(chrome_page) = crate::features::chrome::attempt_navigation(
                "about:blank",
                &browser_controller.browser.0,
                &config.request_timeout,
                &browser_controller.browser.2,
                &config.viewport,
            )
            .await
            {
                let (_, intercept_handle) = tokio::join!(
                    crate::features::chrome::setup_chrome_events(&chrome_page, &config),
                    crate::features::chrome::setup_chrome_interception_base(
                        &chrome_page,
                        config.chrome_intercept.enabled,
                        &config.auth_challenge_response,
                        config.chrome_intercept.block_visuals,
                        &url,
                    )
                );

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
                    &config.track_events,
                    config.referer.clone(),
                    config.max_page_bytes,
                    config.get_cache_options(),
                    &config.cache_policy,
                    &config.remote_multimodal,
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
    }

    /// Set the crawl status depending on crawl state. The crawl that only changes if the state is Start or Active.
    pub fn set_crawl_status(&mut self) {
        if self.status == CrawlStatus::Start || self.status == CrawlStatus::Active {
            self.status = if self.domain_parsed.is_none() {
                CrawlStatus::Invalid
            } else {
                CrawlStatus::Idle
            };
        }
    }

    /// Setup the Semaphore for the crawl.
    pub fn setup_semaphore(&self) -> Arc<Semaphore> {
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
        if !self.status.eq(&CrawlStatus::FirewallBlocked) {
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
    }

    /// Start to crawl website with async concurrency using the sitemap. This does not page forward into the request. This does nothing without the `sitemap` flag enabled.
    pub async fn crawl_sitemap(&mut self) {
        if !self.status.eq(&CrawlStatus::FirewallBlocked) {
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
    }

    /// Start to crawl website with async concurrency using the sitemap. This does not page forward into the request. This does nothing without the `sitemap` and the `chrome` flag enabled.
    #[cfg(all(
        feature = "sitemap",
        feature = "chrome",
        not(feature = "decentralized")
    ))]
    pub async fn crawl_sitemap_chrome(&mut self) {
        if !self.status.eq(&CrawlStatus::FirewallBlocked) {
            self.start();
            let (client, handle) = self.setup().await;
            let (handle, join_handle) = match handle {
                Some(h) => (Some(h.0), Some(h.1)),
                _ => (None, None),
            };
            self.sitemap_crawl_chrome(&client, &handle, false).await;
            self.set_crawl_status();
            if let Some(h) = join_handle {
                h.abort()
            }
            self.client.replace(client);
        }
    }

    /// Configures the website crawling process for concurrent execution with the ability to send it across threads for subscriptions.
    pub async fn configure_setup(&mut self) {
        self.status = CrawlStatus::Active;
        self.start();
        self.setup().await;
        self.configuration.configure_allowlist();
        self.send_configured = true;
    }

    /// Configures the website crawling process for concurrent execution with the ability to send it across threads for subscriptions without robot protection.
    /// You can manually call `website.configure_robots_parser` after.
    pub fn configure_setup_norobots(&mut self) {
        self.status = CrawlStatus::Active;
        self.start();
        self.setup_base();
        self.configuration.configure_allowlist();
        self.send_configured = true;
    }

    #[cfg(not(feature = "decentralized"))]
    /// Initiates the website crawling http process concurrently with the ability to send it across threads for subscriptions.
    /// Ensure that `website.configure_setup()` has been called before executing this function.
    /// It checks the status to ensure it is not firewall-blocked before proceeding with concurrent crawling.
    /// You can pass in a manual url in order to setup a new crawl directly with pre-configurations ready.
    pub async fn crawl_raw_send(&self, url: Option<&str>) {
        if !self.status.eq(&CrawlStatus::FirewallBlocked) {
            let (client, handle) = (
                match &self.client {
                    Some(c) => c.to_owned(),
                    _ => self.configure_http_client(),
                },
                self.configure_handler(),
            );
            let (handle, join_handle) = match handle {
                Some(h) => (Some(h.0), Some(h.1)),
                _ => (None, None),
            };
            self.crawl_concurrent_raw_send(&client, &handle, &url).await;
            if let Some(h) = join_handle {
                h.abort()
            }
        }
    }

    #[cfg(all(feature = "chrome", not(feature = "decentralized")))]
    /// Initiates the website crawling process concurrently with the ability to send it across threads for subscriptions.
    /// Use `website.configure_setup().await` before executing this function to re-use the initial setup.
    /// You can pass in a manual url in order to setup a new crawl directly with pre-configurations ready.
    pub async fn crawl_chrome_send(&self, url: Option<&str>) {
        if !self.status.eq(&CrawlStatus::FirewallBlocked) {
            let (client, handle) = (
                match &self.client {
                    Some(c) => c.to_owned(),
                    _ => self.configure_http_client(),
                },
                self.configure_handler(),
            );
            let (handle, join_handle) = match handle {
                Some(h) => (Some(h.0), Some(h.1)),
                _ => (None, None),
            };
            self.crawl_concurrent_send(&client, &handle, &url).await;
            if let Some(h) = join_handle {
                h.abort()
            }
        }
    }

    #[cfg(all(feature = "chrome", not(feature = "decentralized")))]
    /// Initiates a single fetch with chrome for one page with the ability to send it across threads for subscriptions.
    pub async fn fetch_chrome(&self, url: Option<&str>) {
        if !self.status.eq(&CrawlStatus::FirewallBlocked) {
            let (client, handle) = (
                match &self.client {
                    Some(c) => c.to_owned(),
                    _ => self.configure_http_client(),
                },
                self.configure_handler(),
            );
            let (_handle, join_handle) = match handle {
                Some(h) => (Some(h.0), Some(h.1)),
                _ => (None, None),
            };
            self._fetch_chrome(&client, &url).await;
            if let Some(h) = join_handle {
                h.abort()
            }
        }
    }

    #[cfg(all(feature = "chrome", not(feature = "decentralized")))]
    /// Initiates a single fetch with chrome without closing the browser for one page with the ability to send it across threads for subscriptions.
    pub async fn fetch_chrome_persisted(
        &self,
        url: Option<&str>,
        browser: &crate::features::chrome::BrowserController,
    ) {
        if !self.status.eq(&CrawlStatus::FirewallBlocked) {
            let (client, handle) = (
                match &self.client {
                    Some(c) => c.to_owned(),
                    _ => self.configure_http_client(),
                },
                self.configure_handler(),
            );
            let (_handle, join_handle) = match handle {
                Some(h) => (Some(h.0), Some(h.1)),
                _ => (None, None),
            };
            self._fetch_chrome_persisted(&client, &url, &browser).await;
            if let Some(h) = join_handle {
                h.abort()
            }
        }
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
        if !self.status.eq(&CrawlStatus::FirewallBlocked) {
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
    }

    #[cfg(all(not(feature = "decentralized"), not(feature = "smart")))]
    /// Start to crawl website with async concurrency smart. Use HTTP first and JavaScript Rendering as needed. This has no effect without the `smart` flag enabled.
    pub async fn crawl_smart(&mut self) {
        self.crawl().await
    }

    /// Start to crawl website with async concurrency using the base raw functionality. Useful when using the `chrome` feature and defaulting to the basic implementation.
    pub async fn crawl_raw(&mut self) {
        if !self.status.eq(&CrawlStatus::FirewallBlocked) {
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
    }

    /// Start to scrape/download website with async concurrency.
    pub async fn scrape(&mut self) {
        if !self.status.eq(&CrawlStatus::FirewallBlocked) {
            let mut w = self.clone();
            let mut rx2 = w.subscribe(0).expect("receiver enabled");

            if self.pages.is_none() {
                self.pages = Some(Vec::new());
            }

            // Signal channel to notify when crawl is done
            let (done_tx, mut done_rx) = tokio::sync::oneshot::channel::<()>();

            let crawl = async move {
                w.crawl().await;
                w.unsubscribe();
                // Signal that crawl is complete
                let _ = done_tx.send(());
            };

            let sub = async {
                loop {
                    tokio::select! {
                        biased;
                        // Check if crawl is done first
                        _ = &mut done_rx => {
                            break;
                        }
                        result = rx2.recv() => {
                            if let Ok(page) = result {
                                if let Some(sid) = page.signature {
                                    self.insert_signature(sid).await;
                                }
                                self.insert_link(page.get_url().into()).await;
                                if let Some(p) = self.pages.as_mut() {
                                    p.push(page);
                                }
                            } else {
                                break;
                            }
                        }
                    }
                }
            };

            tokio::join!(sub, crawl);
            // Unsubscribe from self to close the original channel for any external subscribers
            self.unsubscribe();
        }
    }

    /// Start to crawl website with async concurrency using the base raw functionality. Useful when using the "chrome" feature and defaulting to the basic implementation.
    pub async fn scrape_raw(&mut self) {
        if !self.status.eq(&CrawlStatus::FirewallBlocked) {
            let mut w = self.clone();
            let mut rx2 = w.subscribe(0).expect("receiver enabled");

            if self.pages.is_none() {
                self.pages = Some(Vec::new());
            }

            let (done_tx, mut done_rx) = tokio::sync::oneshot::channel::<()>();

            let crawl = async move {
                w.crawl_raw().await;
                w.unsubscribe();
                let _ = done_tx.send(());
            };

            let sub = async {
                loop {
                    tokio::select! {
                        biased;
                        _ = &mut done_rx => break,
                        result = rx2.recv() => {
                            if let Ok(page) = result {
                                if let Some(sid) = page.signature {
                                    self.insert_signature(sid).await;
                                }
                                self.insert_link(page.get_url().into()).await;
                                if let Some(p) = self.pages.as_mut() {
                                    p.push(page);
                                }
                            } else {
                                break;
                            }
                        }
                    }
                }
            };

            tokio::join!(sub, crawl);
            self.unsubscribe();
        }
    }

    /// Start to scrape website with async concurrency smart. Use HTTP first and JavaScript Rendering as needed. This has no effect without the `smart` flag enabled.
    pub async fn scrape_smart(&mut self) {
        if !self.status.eq(&CrawlStatus::FirewallBlocked) {
            let mut w = self.clone();
            let mut rx2 = w.subscribe(0).expect("receiver enabled");

            if self.pages.is_none() {
                self.pages = Some(Vec::new());
            }

            let (done_tx, mut done_rx) = tokio::sync::oneshot::channel::<()>();

            let crawl = async move {
                w.crawl_smart().await;
                w.unsubscribe();
                let _ = done_tx.send(());
            };

            let sub = async {
                loop {
                    tokio::select! {
                        biased;
                        _ = &mut done_rx => break,
                        result = rx2.recv() => {
                            if let Ok(page) = result {
                                if let Some(sid) = page.signature {
                                    self.insert_signature(sid).await;
                                }
                                self.insert_link(page.get_url().into()).await;
                                if let Some(p) = self.pages.as_mut() {
                                    p.push(page);
                                }
                            } else {
                                break;
                            }
                        }
                    }
                }
            };

            tokio::join!(sub, crawl);
            self.unsubscribe();
        }
    }

    /// Start to scrape website sitemap with async concurrency. Use HTTP first and JavaScript Rendering as needed. This has no effect without the `sitemap` flag enabled.
    pub async fn scrape_sitemap(&mut self) {
        if !self.status.eq(&CrawlStatus::FirewallBlocked) {
            let mut w = self.clone();
            let mut rx2 = w.subscribe(0).expect("receiver enabled");

            if self.pages.is_none() {
                self.pages = Some(Vec::new());
            }

            let (done_tx, mut done_rx) = tokio::sync::oneshot::channel::<()>();

            let crawl = async move {
                w.crawl_sitemap().await;
                w.unsubscribe();
                let _ = done_tx.send(());
            };

            let sub = async {
                loop {
                    tokio::select! {
                        biased;
                        _ = &mut done_rx => break,
                        result = rx2.recv() => {
                            if let Ok(page) = result {
                                if let Some(sid) = page.signature {
                                    self.insert_signature(sid).await;
                                }
                                self.insert_link(page.get_url().into()).await;
                                if let Some(p) = self.pages.as_mut() {
                                    p.push(page);
                                }
                            } else {
                                break;
                            }
                        }
                    }
                }
            };

            tokio::join!(sub, crawl);
            self.unsubscribe();
        }
    }

    /// Dequeue the links to a set
    async fn dequeue(
        &mut self,
        q: &mut Option<tokio::sync::broadcast::Receiver<String>>,
        links: &mut HashSet<CaseInsensitiveString>,
        exceeded_budget: &mut bool,
    ) {
        // Drain relevance credits: restore wildcard budget for irrelevant pages
        #[cfg(all(feature = "agent", feature = "serde"))]
        if let Some(ref cfgs) = self.configuration.remote_multimodal {
            let credits = cfgs.relevance_credits.swap(0, std::sync::atomic::Ordering::Relaxed);
            for _ in 0..credits {
                self.restore_wildcard_budget();
            }
        }

        if let Some(q) = q {
            while let Ok(link) = q.try_recv() {
                let s = link.into();
                let allowed = self.is_allowed_budgetless(&s);

                if allowed.eq(&ProcessLinkStatus::BudgetExceeded) {
                    *exceeded_budget = true;
                    break;
                }

                if allowed.eq(&ProcessLinkStatus::Blocked) || !self.is_allowed_disk(&s).await {
                    continue;
                }

                self.links_visited.extend_with_new_links(links, s);
            }
        }
    }

    /// Apply URL pre-filter to links if configured. Returns filtered links.
    #[cfg(all(feature = "agent", feature = "serde"))]
    async fn apply_url_prefilter(&self, links: &mut HashSet<CaseInsensitiveString>) {
        if let Some(ref cfgs) = self.configuration.remote_multimodal {
            if cfgs.cfg.url_prefilter && cfgs.cfg.relevance_gate && !links.is_empty() {
                *links = crate::features::automation::prefilter_urls(cfgs, links).await;
            }
        }
    }

    /// Start to crawl website concurrently - used mainly for chrome instances to connect to default raw HTTP.
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    async fn crawl_concurrent_raw(&mut self, client: &Client, handle: &Option<Arc<AtomicI8>>) {
        self.start();
        self.status = CrawlStatus::Active;
        let client_rotator = self.client_rotator.clone();
        let mut selector: (
            CompactString,
            smallvec::SmallVec<[CompactString; 2]>,
            CompactString,
        ) = self.setup_selectors();
        if self.single_page() {
            self._crawl_establish(client, &mut selector, false).await;
        } else {
            let on_should_crawl_callback = self.on_should_crawl_callback.clone();
            let full_resources = self.configuration.full_resources;
            let return_page_links = self.configuration.return_page_links;
            let only_html = self.configuration.only_html && !full_resources;
            let mut q = self.channel_queue.as_ref().map(|q| q.0.subscribe());

            let (mut interval, throttle) = self.setup_crawl();

            let mut links: HashSet<CaseInsensitiveString> = self.drain_extra_links().collect();

            links.extend(self._crawl_establish(client, &mut selector, false).await);

            self.configuration.configure_allowlist();

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
                    self.configuration.normalize,
                ),
                self.domain_parsed.clone(),
                self.on_link_find_callback.clone(),
                self.configuration.remote_multimodal.clone(),
            ));

            let mut set: JoinSet<(HashSet<CaseInsensitiveString>, Option<u64>)> = JoinSet::new();

            // track budgeting one time.
            let mut exceeded_budget = false;
            let concurrency = throttle.is_zero();

            self.dequeue(&mut q, &mut links, &mut exceeded_budget).await;

            if !concurrency && !links.is_empty() {
                tokio::time::sleep(*throttle).await;
            }

            let crawl_breaker = if self.configuration.crawl_timeout.is_some() {
                Some(Instant::now())
            } else {
                None
            };

            'outer: loop {
                #[cfg(all(feature = "agent", feature = "serde"))]
                self.apply_url_prefilter(&mut links).await;

                let mut stream =
                    tokio_stream::iter::<HashSet<CaseInsensitiveString>>(links.drain().collect());

                loop {
                    if !concurrency {
                        tokio::time::sleep(*throttle).await;
                    }

                    let semaphore =
                        get_semaphore(&semaphore, !self.configuration.shared_queue).await;

                    tokio::select! {
                        biased;
                        Some(link) = stream.next(), if semaphore.available_permits() > 0 && !crawl_duration_expired(&self.configuration.crawl_timeout, &crawl_breaker) => {
                            if !self.handle_process(handle, &mut interval, async {
                                emit_log_shutdown(link.inner());
                                let permits = set.len();
                                set.shutdown().await;
                                semaphore.add_permits(permits);
                            }).await {
                                while let Some(links) = stream.next().await {
                                    self.extra_links.insert(links);
                                }
                                break 'outer;
                            }
                            let allowed = self.is_allowed(&link);

                            if allowed.eq(&ProcessLinkStatus::BudgetExceeded) {
                                exceeded_budget = true;
                                break;
                            }

                            if allowed.eq(&ProcessLinkStatus::Blocked) || !self.is_allowed_disk(&link).await {
                                continue;
                            }

                            emit_log(link.inner());

                            self.insert_link(link.clone()).await;

                            if let Ok(permit) = semaphore.clone().acquire_owned().await {
                                let shared = shared.clone();
                                let on_should_crawl_callback = on_should_crawl_callback.clone();
                                let rotator = client_rotator.clone();
                                spawn_set("page_fetch", &mut set, async move {
                                    let link_result = match &shared.9 {
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
                                    let client = match &rotator {
                                        Some(r) => r.next(),
                                        None => &shared.0,
                                    };

                                    let mut domain_parsed = None;

                                    let mut page = Page::new_page_streaming(
                                        target_url,
                                        client, only_html,
                                        &mut relative_selectors,
                                        external_domains_caseless,
                                        &r_settings,
                                        &mut links,
                                        None,
                                        &shared.8,
                                        &mut domain_parsed,
                                        &mut links_pages).await;

                                    let mut retry_count = shared.5;

                                    while page.should_retry && retry_count > 0 {
                                        retry_count -= 1;

                                        if let Some(timeout) = page.get_timeout() {
                                            tokio::time::sleep(timeout).await;
                                        }

                                        let retry_client = match &rotator {
                                            Some(r) => r.next(),
                                            None => &shared.0,
                                        };

                                        if page.status_code == StatusCode::GATEWAY_TIMEOUT {
                                            if let Err(elasped) = tokio::time::timeout(BACKOFF_MAX_DURATION, async {
                                                let mut domain_parsed = None;
                                                let next_page = Page::new_page_streaming(
                                                    target_url,
                                                    retry_client, only_html,
                                                    &mut relative_selectors.clone(),
                                                    external_domains_caseless,
                                                    &r_settings,
                                                    &mut links,
                                                    None,
                                                    &shared.8,
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
                                                retry_client,
                                                only_html,
                                                &mut relative_selectors.clone(),
                                                external_domains_caseless,
                                                &r_settings,
                                                &mut links,
                                                None,
                                                &shared.8,
                                                &mut domain_parsed,
                                                &mut links_pages).await);
                                        }
                                    }

                                    if return_page_links {
                                        page.page_links = links_pages.filter(|pages| !pages.is_empty()).map(Box::new);
                                    }

                                    // Run remote multimodal extraction if configured (HTTP-only path)
                                    #[cfg(all(feature = "agent", feature = "serde"))]
                                    if shared.10.is_some() {
                                        let html = page.get_html();
                                        if !html.is_empty() {
                                            use crate::features::automation::{run_remote_multimodal_extraction, AutomationResultExt};
                                            let title = page.metadata.as_ref().and_then(|m| m.title.as_ref()).map(|t| t.as_str());
                                            if let Ok(Some(result)) = run_remote_multimodal_extraction(
                                                &shared.10,
                                                &html,
                                                target_url,
                                                title,
                                            ).await {
                                                // Store usage on page
                                                match page.remote_multimodal_usage.as_mut() {
                                                    Some(v) => v.push(result.usage.clone()),
                                                    None => page.remote_multimodal_usage = Some(vec![result.usage.clone()]),
                                                }
                                                // Store extracted data if available
                                                if result.extracted.is_some() || result.screenshot.is_some() {
                                                    let automation_result = result.to_automation_results();
                                                    match page.extra_remote_multimodal_data.as_mut() {
                                                        Some(v) => v.push(automation_result),
                                                        None => page.extra_remote_multimodal_data = Some(vec![automation_result]),
                                                    }
                                                }
                                            }
                                        }
                                    }

                                    if let Some(ref cb) = on_should_crawl_callback {
                                        if !cb.call(&page) {
                                            page.blocked_crawl = true;
                                            channel_send_page(&shared.2, page, &shared.4);
                                            drop(permit);
                                            return Default::default()
                                        }
                                    }

                                    let signature = page.signature;

                                    channel_send_page(&shared.2, page, &shared.4);

                                    drop(permit);

                                    (links, signature)
                                });
                            }

                            self.dequeue(&mut q, &mut links, &mut exceeded_budget).await;
                        },
                        Some(result) = set.join_next(), if !set.is_empty() => {
                            if let Ok(res) = result {
                                match res.1 {
                                    Some(signature) => {
                                        if self.is_signature_allowed(signature).await {
                                            self.insert_signature(signature).await;
                                            self.links_visited.extend_links(&mut links, res.0);
                                        }
                                    }
                                    _ => {
                                        self.links_visited.extend_links(&mut links, res.0);
                                    }
                                }
                            } else {
                                break;
                            }
                        }
                        else => break,
                    }

                    self.dequeue(&mut q, &mut links, &mut exceeded_budget).await;

                    if links.is_empty() && set.is_empty() || exceeded_budget {
                        // await for all tasks to complete.
                        if exceeded_budget {
                            while let Some(links) = stream.next().await {
                                self.extra_links.insert(links);
                            }
                            while let Some(links) = set.join_next().await {
                                if let Ok(links) = links {
                                    self.extra_links.extend(links.0);
                                }
                            }
                        }
                        break 'outer;
                    }
                }

                self.subscription_guard().await;
                self.dequeue(&mut q, &mut links, &mut exceeded_budget).await;

                if links.is_empty() && set.is_empty() {
                    break;
                }
            }

            // store the extra links.
            if !links.is_empty() {
                self.extra_links.extend(links);
            }
        }
    }

    /// Start to crawl website concurrently.
    #[cfg(all(not(feature = "decentralized"), feature = "chrome"))]
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    async fn crawl_concurrent(&mut self, client: &Client, handle: &Option<Arc<AtomicI8>>) {
        use crate::features::chrome::attempt_navigation;
        self.start();

        match self.setup_browser().await {
            Some(mut b) => {
                match attempt_navigation(
                    "about:blank",
                    &b.browser.0,
                    &self.configuration.request_timeout,
                    &b.browser.2,
                    &self.configuration.viewport,
                )
                .await
                {
                    Ok(new_page) => {
                        let mut selectors = self.setup_selectors();
                        self.status = CrawlStatus::Active;

                        if self.single_page() {
                            self.crawl_establish(&client, &mut selectors, false, &new_page)
                                .await;
                            drop(new_page);
                            self.subscription_guard().await;
                            b.dispose();
                        } else {
                            let semaphore: Arc<Semaphore> = self.setup_semaphore();
                            let (mut interval, throttle) = self.setup_crawl();

                            let mut q = self.channel_queue.as_ref().map(|q| q.0.subscribe());

                            let base_links = self
                                .crawl_establish(&client, &mut selectors, false, &new_page)
                                .await;

                            drop(new_page);

                            let mut links: HashSet<CaseInsensitiveString> =
                                self.drain_extra_links().collect();

                            links.extend(base_links);

                            self.configuration.configure_allowlist();

                            let mut set: JoinSet<(HashSet<CaseInsensitiveString>, Option<u64>)> =
                                JoinSet::new();

                            let shared = Arc::new((
                                client.to_owned(),
                                selectors,
                                self.channel.clone(),
                                self.configuration.external_domains_caseless.clone(),
                                self.channel_guard.clone(),
                                b.browser.0.clone(),
                                self.configuration.clone(),
                                self.url.inner().to_string(),
                                b.browser.2.clone(),
                                self.domain_parsed.clone(),
                                self.on_link_find_callback.clone(),
                            ));

                            let add_external = shared.3.len() > 0;
                            let on_should_crawl_callback = self.on_should_crawl_callback.clone();
                            let full_resources = self.configuration.full_resources;
                            let return_page_links = self.configuration.return_page_links;
                            let mut exceeded_budget = false;
                            let concurrency = throttle.is_zero();

                            self.dequeue(&mut q, &mut links, &mut exceeded_budget).await;

                            if !concurrency && !links.is_empty() {
                                tokio::time::sleep(*throttle).await;
                            }

                            let crawl_breaker = if self.configuration.crawl_timeout.is_some() {
                                Some(Instant::now())
                            } else {
                                None
                            };

                            'outer: loop {
                                #[cfg(all(feature = "agent", feature = "serde"))]
                                self.apply_url_prefilter(&mut links).await;

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
                                        Some(link) = stream.next(), if semaphore.available_permits() > 0 && !crawl_duration_expired(&self.configuration.crawl_timeout, &crawl_breaker)  => {
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
                                            if allowed.eq(&ProcessLinkStatus::Blocked) || !self.is_allowed_disk(&link).await {
                                                continue;
                                            }

                                            emit_log(&link.inner());

                                            self.insert_link(link.clone()).await;

                                            if let Ok(permit) = semaphore.clone().acquire_owned().await {
                                                let shared = shared.clone();
                                                let on_should_crawl_callback = on_should_crawl_callback.clone();
                                                spawn_set("page_fetch", &mut set, async move {
                                                    let results = match attempt_navigation("about:blank", &shared.5, &shared.6.request_timeout, &shared.8, &shared.6.viewport).await {
                                                        Ok(new_page) => {
                                                            let (_, intercept_handle) = tokio::join!(
                                                                crate::features::chrome::setup_chrome_events(&new_page, &shared.6),
                                                                crate::features::chrome::setup_chrome_interception_base(
                                                                    &new_page,
                                                                    shared.6.chrome_intercept.enabled,
                                                                    &shared.6.auth_challenge_response,
                                                                    shared.6.chrome_intercept.block_visuals,
                                                                    &shared.7,
                                                                )
                                                            );

                                                            let link_result =
                                                                match  &shared.10 {
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
                                                                &shared.6.request_timeout,
                                                                &shared.6.track_events,
                                                                shared.6.referer.clone(),
                                                                shared.6.max_page_bytes,
                                                                shared.6.get_cache_options(),
                                                                &shared.6.cache_policy,
                                                                &shared.6.remote_multimodal,
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
                                                                            &shared.6.request_timeout,
                                                                            &shared.6.track_events,
                                                                            shared.6.referer.clone(),
                                                                            shared.6.max_page_bytes,
                                                                            shared.6.get_cache_options(),
                                                                            &shared.6.cache_policy,
                                                                            &shared.6.remote_multimodal,
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
                                                                            &shared.6.track_events,
                                                                            shared.6.referer.clone(),
                                                                            shared.6.max_page_bytes,
                                                                            shared.6.get_cache_options(),
                                                                            &shared.6.cache_policy,
                                                                            &shared.6.remote_multimodal,
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
                                                                page.links_full(&shared.1, &shared.9).await
                                                            } else {
                                                                page.links(&shared.1, &shared.9).await
                                                            };

                                                            page.base = prev_domain;

                                                            if shared.6.normalize {
                                                                page.signature.replace(crate::utils::hash_html(&page.get_html_bytes_u8()).await);
                                                            }

                                                            if let Some(ref cb) = on_should_crawl_callback {
                                                                if !cb.call(&page) {
                                                                    page.blocked_crawl = true;
                                                                    channel_send_page(&shared.2, page, &shared.4);
                                                                    drop(permit);
                                                                    return Default::default()
                                                                }
                                                            }

                                                            let signature = page.signature;

                                                            channel_send_page(
                                                                &shared.2, page, &shared.4,
                                                            );

                                                            (links, signature)
                                                        }
                                                        _ => Default::default(),
                                                    };


                                                    drop(permit);

                                                    results
                                                });
                                            }

                                            self.dequeue(&mut q, &mut links, &mut exceeded_budget).await;
                                        }
                                        Some(result) = set.join_next(), if !set.is_empty() => {
                                            if let Ok(res) = result {
                                                match res.1 {
                                                    Some(signature) => {
                                                        if self.is_signature_allowed(signature).await {
                                                            self.insert_signature(signature).await;
                                                            self.links_visited.extend_links(&mut links, res.0);
                                                        }
                                                    }
                                                    _ => {
                                                        self.links_visited.extend_links(&mut links, res.0);
                                                    }
                                                }
                                            } else{
                                                break
                                            }
                                        }
                                        else => break,
                                    };

                                    if links.is_empty() && set.is_empty() || exceeded_budget {
                                        if exceeded_budget {
                                            while set.join_next().await.is_some() {}
                                        }
                                        break 'outer;
                                    }
                                }

                                self.dequeue(&mut q, &mut links, &mut exceeded_budget).await;

                                if links.is_empty() && set.is_empty() {
                                    break;
                                }
                            }

                            self.subscription_guard().await;
                            b.dispose();
                            // store the extra links.
                            if !links.is_empty() {
                                self.extra_links.extend(links);
                            }
                        }
                    }
                    Err(err) => {
                        b.dispose();
                        log::error!("{}", err)
                    }
                }
            }
            _ => log::error!("Chrome initialization failed."),
        }
    }

    /// Start to crawl website concurrently using chrome with the ability to send it across threads for subscriptions.
    #[cfg_attr(
        all(feature = "tracing", not(feature = "decentralized")),
        tracing::instrument(skip_all)
    )]
    async fn crawl_concurrent_raw_send(
        &self,
        client: &Client,
        handle: &Option<Arc<AtomicI8>>,
        url: &Option<&str>,
    ) -> Website {
        let mut selector: (
            CompactString,
            smallvec::SmallVec<[CompactString; 2]>,
            CompactString,
        ) = self.setup_selectors();

        let mut website = self.clone();

        if let Some(u) = url {
            match &website.domain_parsed {
                Some(domain_url) => {
                    if domain_url.as_str().starts_with(u) {
                        website.set_url_only(u);
                    } else {
                        website.set_url(u);
                    }
                }
                _ => {
                    website.set_url(u);
                }
            }
        }

        if !website.send_configured {
            website.configure_setup().await;
        }

        if self.single_page() {
            website._crawl_establish(client, &mut selector, false).await;
            website
        } else {
            let client_rotator = self.client_rotator.clone();
            let on_should_crawl_callback = self.on_should_crawl_callback.clone();
            let full_resources = self.configuration.full_resources;
            let return_page_links = self.configuration.return_page_links;
            let only_html = self.configuration.only_html && !full_resources;
            let mut q = self.channel_queue.as_ref().map(|q| q.0.subscribe());

            let (mut interval, throttle) = self.setup_crawl();

            let mut links: HashSet<CaseInsensitiveString> = website.drain_extra_links().collect();

            links.extend(website._crawl_establish(client, &mut selector, false).await);

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
                    self.configuration.normalize,
                ),
                self.domain_parsed.clone(),
                self.on_link_find_callback.clone(),
                self.configuration.remote_multimodal.clone(),
            ));

            let mut set: JoinSet<(HashSet<CaseInsensitiveString>, Option<u64>)> = JoinSet::new();

            // track budgeting one time.
            let mut exceeded_budget = false;
            let concurrency = throttle.is_zero();

            website
                .dequeue(&mut q, &mut links, &mut exceeded_budget)
                .await;

            if !concurrency && !links.is_empty() {
                tokio::time::sleep(*throttle).await;
            }

            let crawl_breaker = if self.configuration.crawl_timeout.is_some() {
                Some(Instant::now())
            } else {
                None
            };

            'outer: loop {
                #[cfg(all(feature = "agent", feature = "serde"))]
                self.apply_url_prefilter(&mut links).await;

                let mut stream =
                    tokio_stream::iter::<HashSet<CaseInsensitiveString>>(links.drain().collect());

                loop {
                    if !concurrency {
                        tokio::time::sleep(*throttle).await;
                    }

                    let semaphore =
                        get_semaphore(&semaphore, !self.configuration.shared_queue).await;

                    tokio::select! {
                        biased;
                        Some(link) = stream.next(), if semaphore.available_permits() > 0 && !crawl_duration_expired(&self.configuration.crawl_timeout, &crawl_breaker)   => {
                            if !self.handle_process(handle, &mut interval, async {
                                emit_log_shutdown(link.inner());
                                let permits = set.len();
                                set.shutdown().await;
                                semaphore.add_permits(permits);
                            }).await {
                                break 'outer;
                            }
                            let allowed = website.is_allowed(&link);

                            if allowed.eq(&ProcessLinkStatus::BudgetExceeded) {
                                exceeded_budget = true;
                                break;
                            }

                            if allowed.eq(&ProcessLinkStatus::Blocked) || !self.is_allowed_disk(&link).await {
                                continue;
                            }

                            emit_log(link.inner());

                            website.insert_link(link.clone()).await;

                            if let Ok(permit) = semaphore.clone().acquire_owned().await {
                                let shared = shared.clone();
                                let on_should_crawl_callback = on_should_crawl_callback.clone();
                                let rotator = client_rotator.clone();
                                spawn_set("page_fetch", &mut set, async move {
                                    let link_result = match &shared.9 {
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
                                    let client = match &rotator {
                                        Some(r) => r.next(),
                                        None => &shared.0,
                                    };

                                    let mut domain_parsed = None;

                                    let mut page = Page::new_page_streaming(
                                        target_url,
                                        client, only_html,
                                        &mut relative_selectors,
                                        external_domains_caseless,
                                        &r_settings,
                                        &mut links,
                                        None,
                                        &shared.8,
                                        &mut domain_parsed,
                                        &mut links_pages).await;

                                    let mut retry_count = shared.5;

                                    while page.should_retry && retry_count > 0 {
                                        retry_count -= 1;

                                        if let Some(timeout) = page.get_timeout() {
                                            tokio::time::sleep(timeout).await;
                                        }

                                        let retry_client = match &rotator {
                                            Some(r) => r.next(),
                                            None => &shared.0,
                                        };

                                        if page.status_code == StatusCode::GATEWAY_TIMEOUT {
                                            if let Err(elasped) = tokio::time::timeout(BACKOFF_MAX_DURATION, async {
                                                let mut domain_parsed = None;
                                                let next_page = Page::new_page_streaming(
                                                    target_url,
                                                    retry_client, only_html,
                                                    &mut relative_selectors.clone(),
                                                    external_domains_caseless,
                                                    &r_settings,
                                                    &mut links,
                                                    None,
                                                    &shared.8,
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
                                                retry_client,
                                                only_html,
                                                &mut relative_selectors.clone(),
                                                external_domains_caseless,
                                                &r_settings,
                                                &mut links,
                                                None,
                                                &shared.8,
                                                &mut domain_parsed,
                                                &mut links_pages).await);
                                        }
                                    }

                                    if return_page_links {
                                        page.page_links = links_pages.filter(|pages| !pages.is_empty()).map(Box::new);
                                    }

                                    // Run remote multimodal extraction if configured (HTTP-only path)
                                    #[cfg(all(feature = "agent", feature = "serde"))]
                                    if shared.10.is_some() {
                                        let html = page.get_html();
                                        if !html.is_empty() {
                                            use crate::features::automation::{run_remote_multimodal_extraction, AutomationResultExt};
                                            let title = page.metadata.as_ref().and_then(|m| m.title.as_ref()).map(|t| t.as_str());
                                            if let Ok(Some(result)) = run_remote_multimodal_extraction(
                                                &shared.10,
                                                &html,
                                                target_url,
                                                title,
                                            ).await {
                                                // Store usage on page
                                                match page.remote_multimodal_usage.as_mut() {
                                                    Some(v) => v.push(result.usage.clone()),
                                                    None => page.remote_multimodal_usage = Some(vec![result.usage.clone()]),
                                                }
                                                // Store extracted data if available
                                                if result.extracted.is_some() || result.screenshot.is_some() {
                                                    let automation_result = result.to_automation_results();
                                                    match page.extra_remote_multimodal_data.as_mut() {
                                                        Some(v) => v.push(automation_result),
                                                        None => page.extra_remote_multimodal_data = Some(vec![automation_result]),
                                                    }
                                                }
                                            }
                                        }
                                    }

                                    if let Some(ref cb) = on_should_crawl_callback {
                                        if !cb.call(&page) {
                                            page.blocked_crawl = true;
                                            channel_send_page(&shared.2, page, &shared.4);
                                            drop(permit);
                                            return Default::default()
                                        }
                                    }

                                    let signature = page.signature;

                                    channel_send_page(&shared.2, page, &shared.4);

                                    drop(permit);

                                    (links, signature)
                                });
                            }

                            website.dequeue(&mut q, &mut links, &mut exceeded_budget).await;
                        },
                        Some(result) = set.join_next(), if !set.is_empty() => {
                            if let Ok(res) = result {
                                match res.1 {
                                    Some(signature) => {
                                        if website.is_signature_allowed(signature).await {
                                            website.insert_signature(signature).await;
                                            website.links_visited.extend_links(&mut links, res.0);
                                        }
                                    }
                                    _ => {
                                        website.links_visited.extend_links(&mut links, res.0);
                                    }
                                }
                            } else {
                                break;
                            }
                        }
                        else => break,
                    }

                    website
                        .dequeue(&mut q, &mut links, &mut exceeded_budget)
                        .await;

                    if links.is_empty() && set.is_empty() || exceeded_budget {
                        // await for all tasks to complete.
                        if exceeded_budget {
                            while set.join_next().await.is_some() {}
                        }
                        break 'outer;
                    }
                }

                website.subscription_guard().await;
                website
                    .dequeue(&mut q, &mut links, &mut exceeded_budget)
                    .await;

                if links.is_empty() && set.is_empty() {
                    break;
                }
            }
            website
        }
    }

    /// Start to crawl website concurrently with the ability to send it across threads for subscriptions.
    #[cfg(all(not(feature = "decentralized"), feature = "chrome"))]
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    async fn crawl_concurrent_send(
        &self,
        client: &Client,
        handle: &Option<Arc<AtomicI8>>,
        url: &Option<&str>,
    ) -> Website {
        use crate::features::chrome::attempt_navigation;

        match self.setup_browser().await {
            Some(mut b) => {
                match attempt_navigation(
                    "about:blank",
                    &b.browser.0,
                    &self.configuration.request_timeout,
                    &b.browser.2,
                    &self.configuration.viewport,
                )
                .await
                {
                    Ok(new_page) => {
                        let mut selectors = self.setup_selectors();
                        let mut website = self.to_owned();

                        if let Some(u) = url {
                            match &website.domain_parsed {
                                Some(domain_url) => {
                                    if domain_url.as_str().starts_with(u) {
                                        website.set_url_only(u);
                                    } else {
                                        website.set_url(u);
                                    }
                                }
                                _ => {
                                    website.set_url(u);
                                }
                            }
                        }

                        if !website.send_configured {
                            website.configure_setup().await;
                        }

                        let base_links = website
                            .crawl_establish(&client, &mut selectors, false, &new_page)
                            .await;

                        drop(new_page);

                        if self.single_page() {
                            website.subscription_guard().await;
                            b.dispose();
                            website
                        } else {
                            let semaphore: Arc<Semaphore> = self.setup_semaphore();
                            let (mut interval, throttle) = self.setup_crawl();

                            let mut q = self.channel_queue.as_ref().map(|q| q.0.subscribe());

                            let mut links: HashSet<CaseInsensitiveString> =
                                *self.extra_links.clone();

                            links.extend(base_links);

                            let mut set: JoinSet<(HashSet<CaseInsensitiveString>, Option<u64>)> =
                                JoinSet::new();

                            let shared = Arc::new((
                                client.to_owned(),
                                selectors,
                                self.channel.clone(),
                                self.configuration.external_domains_caseless.clone(),
                                self.channel_guard.clone(),
                                b.browser.0.clone(),
                                self.configuration.clone(),
                                self.url.inner().to_string(),
                                b.browser.2.clone(),
                                self.domain_parsed.clone(),
                                self.on_link_find_callback.clone(),
                            ));

                            let add_external = shared.3.len() > 0;
                            let on_should_crawl_callback = self.on_should_crawl_callback.clone();
                            let full_resources = self.configuration.full_resources;
                            let return_page_links = self.configuration.return_page_links;
                            let mut exceeded_budget = false;
                            let concurrency = throttle.is_zero();

                            website
                                .dequeue(&mut q, &mut links, &mut exceeded_budget)
                                .await;

                            if !concurrency && !links.is_empty() {
                                tokio::time::sleep(*throttle).await;
                            }

                            let crawl_breaker = if self.configuration.crawl_timeout.is_some() {
                                Some(Instant::now())
                            } else {
                                None
                            };

                            'outer: loop {
                                #[cfg(all(feature = "agent", feature = "serde"))]
                                self.apply_url_prefilter(&mut links).await;

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
                                        Some(link) = stream.next(), if semaphore.available_permits() > 0 && !crawl_duration_expired(&self.configuration.crawl_timeout, &crawl_breaker)  => {
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

                                            let allowed = website.is_allowed(&link);

                                            if allowed
                                                .eq(&ProcessLinkStatus::BudgetExceeded)
                                            {
                                                exceeded_budget = true;
                                                break;
                                            }
                                            if allowed.eq(&ProcessLinkStatus::Blocked) || !self.is_allowed_disk(&link).await {
                                                continue;
                                            }

                                            emit_log(&link.inner());

                                            website.insert_link(link.clone()).await;

                                            if let Ok(permit) = semaphore.clone().acquire_owned().await {
                                                let shared = shared.clone();
                                                let on_should_crawl_callback = on_should_crawl_callback.clone();
                                                spawn_set("page_fetch", &mut set, async move {
                                                    let results = match attempt_navigation("about:blank", &shared.5, &shared.6.request_timeout, &shared.8, &shared.6.viewport).await {
                                                        Ok(new_page) => {
                                                            let (_, intercept_handle) = tokio::join!(
                                                                crate::features::chrome::setup_chrome_events(&new_page, &shared.6),
                                                                crate::features::chrome::setup_chrome_interception_base(
                                                                    &new_page,
                                                                    shared.6.chrome_intercept.enabled,
                                                                    &shared.6.auth_challenge_response,
                                                                    shared.6.chrome_intercept.block_visuals,
                                                                    &shared.7,
                                                                )
                                                            );

                                                            let link_result =
                                                                match &shared.10 {
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
                                                                &shared.6.request_timeout,
                                                                &shared.6.track_events,
                                                                shared.6.referer.clone(),
                                                                shared.6.max_page_bytes,
                                                                shared.6.get_cache_options(),
                                                                &shared.6.cache_policy,
                                                                &shared.6.remote_multimodal,
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
                                                                            &shared.6.request_timeout,
                                                                            &shared.6.track_events,
                                                                            shared.6.referer.clone(),
                                                                            shared.6.max_page_bytes,
                                                                            shared.6.get_cache_options(),
                                                                            &shared.6.cache_policy,
                                                                            &shared.6.remote_multimodal,
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
                                                                            &shared.6.track_events,
                                                                            shared.6.referer.clone(),
                                                                            shared.6.max_page_bytes,
                                                                            shared.6.get_cache_options(),
                                                                            &shared.6.cache_policy,
                                                                            &shared.6.remote_multimodal,
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
                                                                page.links_full(&shared.1, &shared.9).await
                                                            } else {
                                                                page.links(&shared.1, &shared.9).await
                                                            };

                                                            page.base = prev_domain;

                                                            if shared.6.normalize {
                                                                page.signature.replace(crate::utils::hash_html(&page.get_html_bytes_u8()).await);
                                                            }

                                                            if let Some(ref cb) = on_should_crawl_callback {
                                                                if !cb.call(&page) {
                                                                    page.blocked_crawl = true;
                                                                    channel_send_page(&shared.2, page, &shared.4);
                                                                    drop(permit);
                                                                    return Default::default()
                                                                }
                                                            }

                                                            let signature = page.signature;

                                                            channel_send_page(
                                                                &shared.2, page, &shared.4,
                                                            );

                                                            (links, signature)
                                                        }
                                                        _ => Default::default(),
                                                    };


                                                    drop(permit);

                                                    results
                                                });
                                            }

                                            website.dequeue(&mut q, &mut links, &mut exceeded_budget).await;
                                        }
                                        Some(result) = set.join_next(), if !set.is_empty() => {
                                            if let Ok(res) = result {
                                                match res.1 {
                                                    Some(signature) => {
                                                        if website.is_signature_allowed(signature).await {
                                                            website.insert_signature(signature).await;
                                                            website.links_visited.extend_links(&mut links, res.0);
                                                        }
                                                    }
                                                    _ => {
                                                        website.links_visited.extend_links(&mut links, res.0);
                                                    }
                                                }
                                            } else{
                                                break
                                            }
                                        }
                                        else => break,
                                    };

                                    if links.is_empty() && set.is_empty() || exceeded_budget {
                                        if exceeded_budget {
                                            while set.join_next().await.is_some() {}
                                        }
                                        break 'outer;
                                    }
                                }

                                website
                                    .dequeue(&mut q, &mut links, &mut exceeded_budget)
                                    .await;

                                if links.is_empty() && set.is_empty() {
                                    break;
                                }
                            }

                            website.subscription_guard().await;
                            b.dispose();

                            website
                        }
                    }
                    Err(err) => {
                        b.dispose();
                        log::error!("{}", err);
                        self.clone()
                    }
                }
            }
            _ => {
                log::error!("Chrome initialization failed.");
                self.clone()
            }
        }
    }

    /// Start to crawl website concurrently with the ability to send it across threads for subscriptions for one page.
    #[cfg(all(not(feature = "decentralized"), feature = "chrome"))]
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    async fn _fetch_chrome(&self, client: &Client, url: &Option<&str>) {
        use crate::features::chrome::attempt_navigation;

        match self.setup_browser().await {
            Some(mut b) => {
                match attempt_navigation(
                    "about:blank",
                    &b.browser.0,
                    &self.configuration.request_timeout,
                    &b.browser.2,
                    &self.configuration.viewport,
                )
                .await
                {
                    Ok(new_page) => {
                        let mut selectors = self.setup_selectors();
                        self.crawl_establish_chrome_one(&client, &mut selectors, url, &new_page)
                            .await;
                        self.subscription_guard().await;
                        b.dispose();
                    }
                    Err(err) => {
                        b.dispose();
                        log::error!("{}", err);
                    }
                }
            }
            _ => {
                log::error!("Chrome initialization failed.");
            }
        }
    }

    /// Start to crawl website concurrently with the ability to send it across threads for subscriptions for one page.
    #[cfg(all(not(feature = "decentralized"), feature = "chrome"))]
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    async fn _fetch_chrome_persisted(
        &self,
        client: &Client,
        url: &Option<&str>,
        b: &crate::features::chrome::BrowserController,
    ) {
        use crate::features::chrome::attempt_navigation;
        match attempt_navigation(
            "about:blank",
            &b.browser.0,
            &self.configuration.request_timeout,
            &b.browser.2,
            &self.configuration.viewport,
        )
        .await
        {
            Ok(new_page) => {
                let mut selectors = self.setup_selectors();
                self.crawl_establish_chrome_one(&client, &mut selectors, url, &new_page)
                    .await;
                self.subscription_guard().await;
            }
            Err(err) => {
                log::error!("{}", err);
            }
        }
    }

    /// Start to crawl website concurrently using WebDriver.
    #[cfg(all(not(feature = "decentralized"), not(feature = "chrome"), feature = "webdriver"))]
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    async fn crawl_concurrent_webdriver(&mut self, client: &Client, handle: &Option<Arc<AtomicI8>>) {
        self.start();

        match self.setup_webdriver().await {
            Some(mut controller) => {
                let driver = controller.driver();
                let mut selectors = self.setup_selectors();
                self.status = CrawlStatus::Active;

                if self.single_page() {
                    self.crawl_establish_webdriver_one(&client, &mut selectors, &None, driver)
                        .await;
                    self.subscription_guard().await;
                    controller.dispose();
                } else {
                    let semaphore: Arc<Semaphore> = self.setup_semaphore();
                    let (mut interval, throttle) = self.setup_crawl();

                    let mut q = self.channel_queue.as_ref().map(|q| q.0.subscribe());

                    let base_links = self
                        .crawl_establish_webdriver_one(&client, &mut selectors, &None, driver)
                        .await;

                    let mut links: HashSet<CaseInsensitiveString> =
                        self.drain_extra_links().collect();

                    links.extend(base_links);

                    self.configuration.configure_allowlist();

                    let timeout = self
                        .configuration
                        .webdriver_config
                        .as_ref()
                        .and_then(|c| c.timeout);

                    let mut set: JoinSet<(HashSet<CaseInsensitiveString>, Option<u64>)> =
                        JoinSet::new();

                    let shared = Arc::new((
                        client.to_owned(),
                        selectors,
                        self.channel.clone(),
                        self.configuration.external_domains_caseless.clone(),
                        self.channel_guard.clone(),
                        driver.clone(),
                        self.configuration.clone(),
                        self.url.inner().to_string(),
                        self.domain_parsed.clone(),
                        self.on_link_find_callback.clone(),
                        timeout,
                    ));

                    let add_external = shared.3.len() > 0;
                    let on_should_crawl_callback = self.on_should_crawl_callback.clone();
                    let full_resources = self.configuration.full_resources;
                    let return_page_links = self.configuration.return_page_links;
                    let mut exceeded_budget = false;
                    let concurrency = throttle.is_zero();

                    self.dequeue(&mut q, &mut links, &mut exceeded_budget).await;

                    if !concurrency && !links.is_empty() {
                        tokio::time::sleep(*throttle).await;
                    }

                    let crawl_breaker = if self.configuration.crawl_timeout.is_some() {
                        Some(Instant::now())
                    } else {
                        None
                    };

                    'outer: loop {
                        #[cfg(all(feature = "agent", feature = "serde"))]
                        self.apply_url_prefilter(&mut links).await;

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
                                Some(link) = stream.next(), if semaphore.available_permits() > 0 && !crawl_duration_expired(&self.configuration.crawl_timeout, &crawl_breaker) => {
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
                                    if allowed.eq(&ProcessLinkStatus::Blocked) || !self.is_allowed_disk(&link).await {
                                        continue;
                                    }

                                    emit_log(&link.inner());

                                    self.insert_link(link.clone()).await;

                                    if let Ok(permit) = semaphore.clone().acquire_owned().await {
                                        let shared = shared.clone();
                                        let on_should_crawl_callback = on_should_crawl_callback.clone();

                                        spawn_set("page_fetch_webdriver", &mut set, async move {
                                            let link_result = match &shared.9 {
                                                Some(cb) => cb(link, None),
                                                _ => (link, None),
                                            };

                                            let target_url = link_result.0.as_ref();

                                            // Setup stealth events before navigation
                                            crate::features::webdriver::setup_driver_events(&shared.5, &shared.6).await;

                                            let mut page = Page::new_page_webdriver(
                                                target_url,
                                                &shared.5,
                                                shared.10,
                                            )
                                            .await;

                                            let mut retry_count = shared.6.retry;

                                            while page.should_retry && retry_count > 0 {
                                                retry_count -= 1;
                                                if let Some(timeout_duration) = page.get_timeout() {
                                                    tokio::time::sleep(timeout_duration).await;
                                                }
                                                if page.status_code == StatusCode::GATEWAY_TIMEOUT {
                                                    if let Err(elapsed) = tokio::time::timeout(BACKOFF_MAX_DURATION, async {
                                                        let p = Page::new_page_webdriver(
                                                            target_url,
                                                            &shared.5,
                                                            shared.10,
                                                        ).await;
                                                        page.clone_from(&p);
                                                    }).await {
                                                        log::info!("{target_url} backoff gateway timeout exceeded {elapsed}");
                                                    }
                                                } else {
                                                    page.clone_from(
                                                        &Page::new_page_webdriver(
                                                            target_url,
                                                            &shared.5,
                                                            shared.10,
                                                        )
                                                        .await,
                                                    );
                                                }
                                            }

                                            if add_external {
                                                page.set_external(shared.3.clone());
                                            }

                                            let prev_domain = page.base;
                                            page.base = shared.8.as_deref().cloned();

                                            if return_page_links {
                                                page.page_links = Some(Default::default());
                                            }

                                            let links = if full_resources {
                                                page.links_full(&shared.1, &shared.8).await
                                            } else {
                                                page.links(&shared.1, &shared.8).await
                                            };

                                            page.base = prev_domain;

                                            if shared.6.normalize {
                                                page.signature.replace(crate::utils::hash_html(&page.get_html_bytes_u8()).await);
                                            }

                                            if let Some(ref cb) = on_should_crawl_callback {
                                                if !cb.call(&page) {
                                                    page.blocked_crawl = true;
                                                    channel_send_page(&shared.2, page, &shared.4);
                                                    drop(permit);
                                                    return Default::default();
                                                }
                                            }

                                            let signature = page.signature;

                                            channel_send_page(&shared.2, page, &shared.4);

                                            drop(permit);

                                            (links, signature)
                                        });
                                    }

                                    self.dequeue(&mut q, &mut links, &mut exceeded_budget).await;
                                }
                                Some(result) = set.join_next(), if !set.is_empty() => {
                                    if let Ok(res) = result {
                                        match res.1 {
                                            Some(signature) => {
                                                if self.is_signature_allowed(signature).await {
                                                    self.insert_signature(signature).await;
                                                    self.links_visited.extend_links(&mut links, res.0);
                                                }
                                            }
                                            _ => {
                                                self.links_visited.extend_links(&mut links, res.0);
                                            }
                                        }
                                    } else {
                                        break
                                    }

                                    if links.is_empty() && set.is_empty() || exceeded_budget {
                                        if exceeded_budget {
                                            while set.join_next().await.is_some() {}
                                        }
                                        break 'outer;
                                    }
                                }
                                else => break,
                            };

                            if links.is_empty() && set.is_empty() || exceeded_budget {
                                if exceeded_budget {
                                    while set.join_next().await.is_some() {}
                                }
                                break 'outer;
                            }
                        }

                        self.dequeue(&mut q, &mut links, &mut exceeded_budget).await;

                        if links.is_empty() && set.is_empty() {
                            break;
                        }
                    }

                    self.subscription_guard().await;
                    controller.dispose();

                    if !links.is_empty() {
                        self.extra_links.extend(links);
                    }
                }
            }
            None => {
                log::error!("WebDriver initialization failed.");
            }
        }
    }

    /// Start to crawl website concurrently.
    #[cfg(all(not(feature = "decentralized"), not(feature = "chrome"), feature = "webdriver"))]
    pub async fn crawl_concurrent(&mut self, client: &Client, handle: &Option<Arc<AtomicI8>>) {
        // Use WebDriver if configured, otherwise fall back to raw HTTP
        if self.configuration.webdriver_config.is_some() {
            self.crawl_concurrent_webdriver(client, handle).await
        } else {
            self.crawl_concurrent_raw(client, handle).await
        }
    }

    /// Start to crawl website concurrently.
    #[cfg(all(not(feature = "decentralized"), not(feature = "chrome"), not(feature = "webdriver")))]
    pub async fn crawl_concurrent(&mut self, client: &Client, handle: &Option<Arc<AtomicI8>>) {
        self.crawl_concurrent_raw(client, handle).await
    }

    /// Start to crawl website concurrently.
    #[cfg(feature = "decentralized")]
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub async fn crawl_concurrent(&mut self, client: &Client, handle: &Option<Arc<AtomicI8>>) {
        let mut q = self.channel_queue.as_ref().map(|q| q.0.subscribe());

        self.configuration.configure_allowlist();
        let domain = self.url.inner().as_str();
        let mut interval = Box::pin(tokio::time::interval(Duration::from_millis(10)));
        let throttle = Box::pin(self.get_delay());
        let on_link_find_callback = self.on_link_find_callback.clone();
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
            #[cfg(all(feature = "agent", feature = "serde"))]
            self.apply_url_prefilter(&mut links).await;

            let stream =
                tokio_stream::iter::<HashSet<CaseInsensitiveString>>(links.drain().collect())
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
                        if allowed.eq(&ProcessLinkStatus::Blocked)
                            || !self.is_allowed_disk(&link).await
                        {
                            continue;
                        }

                        emit_log(&link.inner());

                        self.insert_link(link.clone()).await;

                        if let Ok(permit) = SEM.acquire().await {
                            let client = client.clone();
                            let on_link_find_callback = on_link_find_callback.clone();

                            spawn_set("page_fetch", &mut set, async move {
                                let link_results = match &on_link_find_callback.clone() {
                                    Some(cb) => cb(link, None),
                                    _ => (link, None),
                                };
                                let link_results = link_results.0.as_ref();
                                let page = Page::new_links_only(
                                    &if http_worker && link_results.starts_with("https") {
                                        link_results.replacen("https", "http", 1).to_string()
                                    } else {
                                        link_results.to_string()
                                    },
                                    &client,
                                )
                                .await;

                                drop(permit);

                                page.links
                            });

                            self.dequeue(&mut q, &mut links, &mut exceeded_budget).await;
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

            self.dequeue(&mut q, &mut links, &mut exceeded_budget).await;

            if links.is_empty() || exceeded_budget {
                break;
            }
        }

        if !links.is_empty() {
            self.extra_links.extend(links);
        }
    }

    #[cfg(all(feature = "chrome", feature = "real_browser"))]
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    /// Warm up the gemini model.
    pub async fn warm_up_gemini(&mut self) {
        use crate::features::chrome::attempt_navigation;

        if let Some(mut b) = self.setup_browser().await {
            if let Ok(page) = attempt_navigation(
                "about:blank",
                &b.browser.0,
                &self.configuration.request_timeout,
                &b.browser.2,
                &self.configuration.viewport,
            )
            .await
            {
                let _ = crate::features::solvers::warm_gemini_model(&page).await;
                b.dispose();
            }
        }
    }

    /// Start to crawl website concurrently using HTTP by default and chrome Javascript Rendering as needed. The glob feature does not work with this at the moment.
    #[cfg(all(not(feature = "decentralized"), feature = "smart"))]
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub async fn crawl_concurrent_smart(
        &mut self,
        client: &Client,
        handle: &Option<Arc<AtomicI8>>,
    ) {
        use tokio::sync::OnceCell;
        self.start();
        self.status = CrawlStatus::Active;
        let browser: OnceBrowser = OnceCell::new();

        let mut selectors: (
            CompactString,
            smallvec::SmallVec<[CompactString; 2]>,
            CompactString,
        ) = self.setup_selectors();

        if self.single_page() {
            self.subscription_guard().await;
            self.crawl_establish_smart(&client, &mut selectors, &browser)
                .await;
        } else {
            let mut q = self.channel_queue.as_ref().map(|q| q.0.subscribe());

            let mut links: HashSet<CaseInsensitiveString> = self.drain_extra_links().collect();

            let (mut interval, throttle) = self.setup_crawl();
            let on_should_crawl_callback = self.on_should_crawl_callback.clone();
            let return_page_links = self.configuration.return_page_links;

            links.extend(
                self.crawl_establish_smart(&client, &mut selectors, &browser)
                    .await,
            );

            self.configuration.configure_allowlist();

            let mut set: JoinSet<(HashSet<CaseInsensitiveString>, Option<u64>)> = JoinSet::new();
            let semaphore = self.setup_semaphore();

            let shared = Arc::new((
                client.to_owned(),
                selectors,
                self.channel.clone(),
                self.channel_guard.clone(),
                self.configuration.clone(),
                self.domain_parsed.clone(),
                browser,
                self.on_link_find_callback.clone(),
                self.cookie_jar.clone(),
            ));

            let add_external = self.configuration.external_domains_caseless.len() > 0;
            let mut exceeded_budget = false;
            let concurrency = throttle.is_zero();

            self.dequeue(&mut q, &mut links, &mut exceeded_budget).await;

            if !concurrency && !links.is_empty() {
                tokio::time::sleep(*throttle).await;
            }

            let crawl_breaker = if self.configuration.crawl_timeout.is_some() {
                Some(Instant::now())
            } else {
                None
            };

            'outer: loop {
                #[cfg(all(feature = "agent", feature = "serde"))]
                self.apply_url_prefilter(&mut links).await;

                let mut stream =
                    tokio_stream::iter::<HashSet<CaseInsensitiveString>>(links.drain().collect());

                loop {
                    if !concurrency {
                        tokio::time::sleep(*throttle).await;
                    }

                    let semaphore =
                        get_semaphore(&semaphore, !self.configuration.shared_queue).await;

                    tokio::select! {
                        biased;
                        Some(link) = stream.next(), if semaphore.available_permits() > 0 && !crawl_duration_expired(&self.configuration.crawl_timeout, &crawl_breaker)  => {
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
                            if allowed.eq(&ProcessLinkStatus::Blocked) || !self.is_allowed_disk(&link).await {
                                continue;
                            }

                            emit_log(&link.inner());
                            self.insert_link(link.clone()).await;

                            if let Ok(permit) = semaphore.clone().acquire_owned().await {
                                let shared = shared.clone();
                                let on_should_crawl_callback = on_should_crawl_callback.clone();
                                spawn_set("page_fetch", &mut set, async move {
                                    let link_result = match &shared.7 {
                                        Some(cb) => cb(link, None),
                                        _ => (link, None),
                                    };

                                    let url = link_result.0.as_ref();
                                    let mut page =
                                        Page::new_page(&url, &shared.0).await;

                                    let mut retry_count = shared.4.retry;

                                    while page.should_retry && retry_count > 0 {
                                        retry_count -= 1;

                                        if let Some(timeout) = page.get_timeout() {
                                            tokio::time::sleep(timeout).await;
                                        }

                                        if page.status_code == StatusCode::GATEWAY_TIMEOUT {

                                            if let Err(elasped) = tokio::time::timeout(BACKOFF_MAX_DURATION, async {
                                                if retry_count.is_power_of_two() {
                                                    Website::render_chrome_page(
                                                        &shared.4, &shared.0,
                                                         &mut page, url,
                                                         &shared.5,
                                                         &shared.6,
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
                                                    &shared.4, &shared.0,
                                                    &mut page, url,
                                                    &shared.5,
                                                    &shared.6,
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
                                                .4
                                                .external_domains_caseless
                                                .clone(),
                                        );
                                    }

                                    let prev_domain = page.base;

                                    page.base = shared.5.as_deref().cloned();

                                    if return_page_links {
                                        page.page_links = Some(Default::default());
                                    }

                                    let (links, bytes_transferred ) = page
                                        .smart_links(
                                            &shared.1, &shared.4, &shared.5, &shared.6, Some(&shared.8)
                                        )
                                        .await;

                                    page.base = prev_domain;
                                    page.bytes_transferred = bytes_transferred;

                                    if shared.4.normalize {
                                        page.signature.replace(crate::utils::hash_html(&page.get_html_bytes_u8()).await);
                                    }

                                    // Run remote multimodal extraction if configured (smart HTTP path)
                                    #[cfg(all(feature = "agent", feature = "serde"))]
                                    if shared.4.remote_multimodal.is_some() {
                                        let html = page.get_html();
                                        if !html.is_empty() {
                                            use crate::features::automation::{run_remote_multimodal_extraction, AutomationResultExt};
                                            let title = page.metadata.as_ref().and_then(|m| m.title.as_ref()).map(|t| t.as_str());
                                            if let Ok(Some(result)) = run_remote_multimodal_extraction(
                                                &shared.4.remote_multimodal,
                                                &html,
                                                url,
                                                title,
                                            ).await {
                                                // Store usage on page
                                                match page.remote_multimodal_usage.as_mut() {
                                                    Some(v) => v.push(result.usage.clone()),
                                                    None => page.remote_multimodal_usage = Some(vec![result.usage.clone()]),
                                                }
                                                // Store extracted data if available
                                                if result.extracted.is_some() || result.screenshot.is_some() {
                                                    let automation_result = result.to_automation_results();
                                                    match page.extra_remote_multimodal_data.as_mut() {
                                                        Some(v) => v.push(automation_result),
                                                        None => page.extra_remote_multimodal_data = Some(vec![automation_result]),
                                                    }
                                                }
                                            }
                                        }
                                    }

                                    if let Some(ref cb) = on_should_crawl_callback {
                                        if !cb.call(&page) {
                                            page.blocked_crawl = true;
                                            channel_send_page(&shared.2, page, &shared.3);
                                            drop(permit);
                                            return Default::default()
                                        }
                                    }

                                    let signature = page.signature;

                                    channel_send_page(&shared.2, page, &shared.3);

                                    drop(permit);

                                    (links, signature)
                                });
                            }

                            self.dequeue(&mut q, &mut links, &mut exceeded_budget).await;
                        }
                        Some(result) = set.join_next(), if !set.is_empty() => {
                            if let Ok(res) = result {
                                match res.1 {
                                    Some(signature) => {
                                        if self.is_signature_allowed(signature).await {
                                            self.insert_signature(signature).await;
                                            self.links_visited.extend_links(&mut links, res.0);
                                        }
                                    }
                                    _ => {
                                        self.links_visited.extend_links(&mut links, res.0);
                                    }
                                }
                            } else{
                                break
                            }
                        }
                        else => break,
                    }

                    if links.is_empty() && set.is_empty() || exceeded_budget {
                        if exceeded_budget {
                            while set.join_next().await.is_some() {}
                        }
                        break 'outer;
                    }
                }

                self.subscription_guard().await;
                self.dequeue(&mut q, &mut links, &mut exceeded_budget).await;

                if links.is_empty() && set.is_empty() {
                    break;
                }
            }

            if !links.is_empty() {
                self.extra_links.extend(links);
            }
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

    /// Sitemap crawl entire lists chain. Note: this method does not re-crawl the links of the pages found on the sitemap. This does nothing without the `sitemap` flag.
    #[cfg(not(feature = "sitemap"))]
    pub async fn sitemap_crawl_chain(
        &mut self,
        _client: &Client,
        _handle: &Option<Arc<AtomicI8>>,
        _scrape: bool,
    ) {
    }

    /// Setup the sitemap path
    #[cfg(feature = "sitemap")]
    pub(crate) fn get_sitemap_setup(&self, domain: &str) -> (&str, bool) {
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

        (sitemap_path, needs_trailing)
    }

    /// Sitemap crawl entire lists. Note: this method does not re-crawl the links of the pages found on the sitemap. This does nothing without the `sitemap` flag.
    #[cfg(feature = "sitemap")]
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub(crate) async fn sitemap_crawl_raw(
        &mut self,
        client: &Client,
        handle: &Option<Arc<AtomicI8>>,
        scrape: bool,
    ) {
        let mut exceeded_budget = self.is_over_wild_budget(&self.configuration.budget);

        if !exceeded_budget {
            let selectors = self.setup_selectors();
            let mut q = self.channel_queue.as_ref().map(|q| q.0.subscribe());
            let domain = self.url.inner().as_str();
            self.domain_parsed = parse_absolute_url(&domain);

            let persist_links = self.status == CrawlStatus::Start;

            let mut interval: Interval = tokio::time::interval(Duration::from_millis(15));

            let (sitemap_path, needs_trailing) = self.get_sitemap_setup(&domain);

            self.configuration.sitemap_url = Some(Box::new(
                string_concat!(domain, if needs_trailing { "/" } else { "" }, sitemap_path).into(),
            ));

            self.configuration.configure_allowlist();

            let domain_parsed_ref = self.domain_parsed.as_deref().cloned().map(Box::new);

            let shared = Arc::new((
                self.channel.clone(),
                self.channel_guard.clone(),
                selectors,
                domain_parsed_ref,
            ));
            let mut sitemaps = match &self.configuration.sitemap_url {
                Some(sitemap) => Vec::from([sitemap.to_owned()]),
                _ => Default::default(),
            };

            let return_page_links = self.configuration.return_page_links;

            let mut extra_links = self.extra_links.clone();
            self.dequeue(&mut q, &mut *extra_links, &mut exceeded_budget)
                .await;
            self.extra_links.clone_from(&extra_links);

            let whitelist_changes = self.configuration.add_sitemap_to_whitelist();

            if whitelist_changes.modified() {
                self.configuration.set_whitelist();
            }

            'outer: loop {
                let stream =
                    tokio_stream::iter::<Vec<Box<CompactString>>>(sitemaps.drain(..).collect());
                tokio::pin!(stream);

                let mut first_request = false;
                let mut attempted_correct = false;

                while let Some(mut sitemap_url) = stream.next().await {
                    if !self.handle_process(handle, &mut interval, async {}).await {
                        break 'outer;
                    }

                    let link = <CompactString as Clone>::clone(&(*sitemap_url)).into();

                    let allowed = self.is_allowed_budgetless(&link);

                    if allowed.eq(&ProcessLinkStatus::Blocked) {
                        continue;
                    }

                    self.insert_link(link).await;

                    let (tx, mut rx) = tokio::sync::mpsc::channel::<Page>(100);

                    let shared = shared.clone();

                    let handles = crate::utils::spawn_task("page_fetch", async move {
                        let mut pages = Vec::new();

                        while let Some(mut page) = rx.recv().await {
                            if page.page_links.is_none() {
                                let links = page.links(&shared.2, &shared.3).await;
                                page.page_links = Some(links.into());
                            }

                            if scrape || persist_links {
                                pages.push(page.clone());
                            };

                            // reset the page links before sending to the main subscriber.
                            if !return_page_links {
                                page.page_links = None;
                            }

                            if shared.0.is_some() {
                                channel_send_page(&shared.0, page, &shared.1);
                            }
                        }

                        pages
                    });

                    while !first_request {
                        // try to get the original sitemap if it had an error on the first request make a request to the root html and parse out the sitemap path.
                        match client.get(sitemap_url.as_str()).send().await {
                            Ok(response) => {
                                let limit = *crate::utils::MAX_SIZE_BYTES as u64;

                                if let Some(response_content_length) = response.content_length() {
                                    if limit > 0 && response_content_length >= limit {
                                        // we need a error here
                                        first_request = true;
                                        log::info!(
                                            "{} exceeded parse limit: {:?}",
                                            sitemap_url,
                                            limit
                                        );
                                        break;
                                    }
                                }

                                if response.status() == 404 {
                                    if !self
                                        .sitemap_parse(
                                            client,
                                            &mut first_request,
                                            &mut sitemap_url,
                                            &mut attempted_correct,
                                        )
                                        .await
                                    {
                                        break;
                                    }
                                } else {
                                    match response.bytes().await {
                                        Ok(b) => {
                                            first_request = true;
                                            self.sitemap_parse_crawl(
                                                client,
                                                handle,
                                                b,
                                                &mut interval,
                                                &mut exceeded_budget,
                                                &tx,
                                                &mut sitemaps,
                                                true,
                                            )
                                            .await;
                                        }
                                        Err(err) => {
                                            first_request = true;
                                            log::info!("http parse error: {:?}", err.to_string())
                                        }
                                    };
                                }
                            }
                            Err(err) => {
                                // do not retry error again.
                                if attempted_correct {
                                    first_request = true;
                                    break;
                                }

                                log::info!("attempting to find sitemap path: {}", err.to_string());

                                if !self
                                    .sitemap_parse(
                                        client,
                                        &mut first_request,
                                        &mut sitemap_url,
                                        &mut attempted_correct,
                                    )
                                    .await
                                {
                                    break;
                                }
                            }
                        };
                    }

                    drop(tx);

                    if let Ok(mut handle) = handles.await {
                        for page in handle.iter_mut() {
                            if let Some(mut links) = page.page_links.clone() {
                                self.dequeue(&mut q, &mut links, &mut exceeded_budget).await;
                                self.extra_links.extend(*links)
                            }
                        }
                        if scrape {
                            if let Some(p) = self.pages.as_mut() {
                                p.extend(handle);
                            }
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

            self.configuration
                .remove_sitemap_from_whitelist(whitelist_changes);
        }
    }

    /// Sitemap crawl entire lists using chrome. Note: this method does not re-crawl the links of the pages found on the sitemap. This does nothing without the `sitemap` flag.
    #[cfg(all(
        feature = "sitemap",
        feature = "chrome",
        not(feature = "decentralized")
    ))]
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub(crate) async fn sitemap_crawl_chrome(
        &mut self,
        client: &Client,
        handle: &Option<Arc<AtomicI8>>,
        scrape: bool,
    ) {
        use crate::features::chrome::attempt_navigation;
        use sitemap::{
            reader::{SiteMapEntity, SiteMapReader},
            structs::Location,
        };

        let mut exceeded_budget = self.is_over_wild_budget(&self.configuration.budget);

        if !exceeded_budget {
            if let Some(mut b) = self.setup_browser().await {
                let selectors = self.setup_selectors();
                let semaphore: Arc<Semaphore> = self.setup_semaphore();
                let mut q = self.channel_queue.as_ref().map(|q| q.0.subscribe());
                let domain = self.url.inner().as_str();
                self.domain_parsed = parse_absolute_url(&domain);
                let persist_links = self.status == CrawlStatus::Start;

                let mut interval = tokio::time::interval(Duration::from_millis(15));

                let (sitemap_path, needs_trailing) = self.get_sitemap_setup(&domain);

                self.configuration.sitemap_url = Some(Box::new(
                    string_concat!(domain, if needs_trailing { "/" } else { "" }, sitemap_path)
                        .into(),
                ));

                self.configuration.configure_allowlist();

                let domain_parsed_ref = self.domain_parsed.as_deref().cloned().map(Box::new);

                let shared = Arc::new((
                    self.channel.clone(),
                    self.channel_guard.clone(),
                    b.browser.0.clone(),
                    self.configuration.clone(),
                    self.url.inner().to_string(),
                    b.browser.2.clone(),
                    selectors.clone(),
                    domain_parsed_ref,
                ));

                let mut sitemaps = match &self.configuration.sitemap_url {
                    Some(sitemap) => Vec::from([sitemap.to_owned()]),
                    _ => Default::default(),
                };

                let crawl_breaker = if self.configuration.crawl_timeout.is_some() {
                    Some(Instant::now())
                } else {
                    None
                };

                let mut extra_links = self.extra_links.clone();
                self.dequeue(&mut q, &mut *extra_links, &mut exceeded_budget)
                    .await;
                self.extra_links.clone_from(&extra_links);
                let mut set: JoinSet<Option<Page>> = JoinSet::new();

                let whitelist_changes = self.configuration.add_sitemap_to_whitelist();

                if whitelist_changes.modified() {
                    self.configuration.set_whitelist();
                }

                'outer: loop {
                    let stream: tokio_stream::Iter<std::vec::IntoIter<Box<CompactString>>> =
                        tokio_stream::iter::<Vec<Box<CompactString>>>(sitemaps.drain(..).collect());
                    tokio::pin!(stream);

                    tokio::select! {
                        biased;
                        Some(sitemap_url) = stream.next(), if semaphore.available_permits() > 0 && !crawl_duration_expired(&self.configuration.crawl_timeout, &crawl_breaker)  => {
                            if !self.handle_process(handle, &mut interval, async {}).await {
                                break 'outer;
                            }

                            let link = <CompactString as Clone>::clone(&(*sitemap_url)).into();

                            let allowed = self.is_allowed_budgetless(&link);

                            if allowed.eq(&ProcessLinkStatus::Blocked) {
                                continue;
                            }

                            self.insert_link(link).await;

                            match attempt_navigation(
                                "about:blank",
                                &shared.2,
                                &self.configuration.request_timeout,
                                &shared.5,
                                &self.configuration.viewport,
                            )
                            .await {
                                Ok(new_page) => {
                                    let (_, intercept_handle) = tokio::join!(
                                        crate::features::chrome::setup_chrome_events(
                                            &new_page,
                                            &self.configuration
                                        ),
                                        self.setup_chrome_interception(&new_page)
                                    );

                                    let mut page = Page::new(
                                        &sitemap_url,
                                        &client,
                                        &new_page,
                                        &self.configuration.wait_for,
                                        &self.configuration.screenshot,
                                        false, // we use the initial about:blank page.
                                        &self.configuration.openai_config,
                                        &self.configuration.execution_scripts,
                                        &self.configuration.automation_scripts,
                                        &self.configuration.viewport,
                                        &self.configuration.request_timeout,
                                        &self.configuration.track_events,
                                        self.configuration.referer.clone(),
                                        self.configuration.max_page_bytes,
                                        self.configuration.get_cache_options(),
                                        &self.configuration.cache_policy,
                                        &self.configuration.remote_multimodal,
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

                                    drop(new_page);

                                    let is_xml_entry = page.get_html_bytes_u8().starts_with(b"<?xml");
                                    let is_xml = is_xml_entry
                                        && !page.get_html_bytes_u8().ends_with(b"</html>");

                                    if is_xml {
                                        let reader = SiteMapReader::new(&*page.get_html_bytes_u8());
                                        let mut stream = tokio_stream::iter(reader);

                                        while let Some(entity) = stream.next().await {
                                            if !self.handle_process(handle, &mut interval, async {}).await {
                                                break;
                                            }
                                            match entity {
                                                SiteMapEntity::Url(url_entry) => match url_entry.loc {
                                                    Location::Url(url) => {
                                                        let link: CaseInsensitiveString = url.as_str().into();

                                                        let allowed = self.is_allowed(&link);

                                                        if allowed.eq(&ProcessLinkStatus::Blocked) {
                                                            continue;
                                                        }
                                                        if allowed.eq(&ProcessLinkStatus::BudgetExceeded) {
                                                            exceeded_budget = true;
                                                            break;
                                                        }

                                                        self.insert_link(link.clone()).await;

                                                        let client = client.clone();
                                                        let shared = shared.clone();

                                                        spawn_set("page_fetch", &mut set, async move {
                                                            if let Ok(new_page) = attempt_navigation(
                                                                "about:blank",
                                                                &shared.2,
                                                                &shared.3.request_timeout,
                                                                &shared.5,
                                                                &shared.3.viewport,
                                                            )
                                                            .await
                                                            {
                                                                let (_, intercept_handle) = tokio::join!(
                                                                    crate::features::chrome::setup_chrome_events(
                                                                        &new_page, &shared.3,
                                                                    ),
                                                                    crate::features::chrome::setup_chrome_interception_base(
                                                                        &new_page,
                                                                        shared.3.chrome_intercept.enabled,
                                                                        &shared.3.auth_challenge_response,
                                                                        shared.3.chrome_intercept.block_visuals,
                                                                        &shared.4,
                                                                    )
                                                                );

                                                                let mut page = Page::new(
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
                                                                    &shared.3.request_timeout,
                                                                    &shared.3.track_events,
                                                                    shared.3.referer.clone(),
                                                                    shared.3.max_page_bytes,
                                                                    shared.3.get_cache_options(),
                                                                    &shared.3.cache_policy,
                                                                    &shared.3.remote_multimodal,
                                                                )
                                                                .await;

                                                                if let Some(intercept_handle) = intercept_handle
                                                                {
                                                                    let abort_handle =
                                                                        intercept_handle.abort_handle();

                                                                    if let Err(elasped) = tokio::time::timeout(
                                                                        tokio::time::Duration::from_secs(10),
                                                                        async { intercept_handle.await },
                                                                    )
                                                                    .await
                                                                    {
                                                                        log::warn!("Handler timeout exceeded {elasped}");
                                                                        abort_handle.abort();
                                                                    }
                                                                }

                                                                if page.page_links.is_none() {
                                                                    let links =
                                                                        page.links(&shared.6, &shared.7).await;
                                                                    page.page_links = Some(links.into());
                                                                }

                                                                Some(page)
                                                            } else {
                                                                None
                                                            }
                                                        });
                                                    }
                                                    Location::None | Location::ParseErr(_) => (),
                                                },
                                                SiteMapEntity::SiteMap(sitemap_entry) => {
                                                    match sitemap_entry.loc {
                                                        Location::Url(url) => {
                                                            sitemaps.push(Box::new(CompactString::new(
                                                                &url.as_str(),
                                                            )));
                                                        }
                                                        Location::None | Location::ParseErr(_) => (),
                                                    }
                                                }
                                                SiteMapEntity::Err(err) => {
                                                    log::info!("incorrect sitemap error: {:?}", err.msg(),)
                                                }
                                            };

                                            if exceeded_budget {
                                                break;
                                            }
                                        }
                                    } else {

                                        if is_xml_entry {
                                            page.modify_xml_html();
                                        }

                                        let links = page.links(&shared.6, &shared.7).await;

                                        let mut stream = tokio_stream::iter(links);

                                        while let Some(link) = stream.next().await {
                                            if !self.handle_process(handle, &mut interval, async {}).await {
                                                break;
                                            }

                                            if link.ends_with(".xml") {
                                                sitemaps.push(Box::new(link.inner().clone()));
                                                continue;
                                            }

                                            let allowed = self.is_allowed(&link);

                                            if allowed.eq(&ProcessLinkStatus::BudgetExceeded) {
                                                exceeded_budget = true;
                                                break;
                                            }
                                            if allowed.eq(&ProcessLinkStatus::Blocked) {
                                                continue;
                                            }

                                            self.insert_link(link.clone()).await;

                                            let client = client.clone();
                                            let shared = shared.clone();

                                            spawn_set("page_fetch", &mut set, async move {
                                                match attempt_navigation(
                                                    "about:blank",
                                                    &shared.2,
                                                    &shared.3.request_timeout,
                                                    &shared.5,
                                                    &shared.3.viewport,
                                                )
                                                .await {
                                                    Ok(new_page) => {
                                                        let (_, intercept_handle) = tokio::join!(
                                                            crate::features::chrome::setup_chrome_events(
                                                                &new_page, &shared.3,
                                                            ),
                                                            crate::features::chrome::setup_chrome_interception_base(
                                                                &new_page,
                                                                shared.3.chrome_intercept.enabled,
                                                                &shared.3.auth_challenge_response,
                                                                shared.3.chrome_intercept.block_visuals,
                                                                &shared.4,
                                                            )
                                                        );

                                                        let mut page = Page::new(
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
                                                            &shared.3.request_timeout,
                                                            &shared.3.track_events,
                                                            shared.3.referer.clone(),
                                                            shared.3.max_page_bytes,
                                                            shared.3.get_cache_options(),
                                                            &shared.3.cache_policy,
                                                            &shared.3.remote_multimodal,
                                                        )
                                                        .await;

                                                        if let Some(intercept_handle) = intercept_handle {
                                                            let abort_handle = intercept_handle.abort_handle();

                                                            if let Err(elasped) = tokio::time::timeout(
                                                                tokio::time::Duration::from_secs(10),
                                                                async { intercept_handle.await },
                                                            )
                                                            .await
                                                            {
                                                                log::warn!("Handler timeout exceeded {elasped}");
                                                                abort_handle.abort();
                                                            }
                                                        }

                                                        if page.page_links.is_none() {
                                                            let links = page.links(&shared.6, &shared.7).await;
                                                            page.page_links = Some(links.into());
                                                        }

                                                        Some(page)
                                                    }
                                                    Err(err) => {
                                                        log::error!("chrome failed to open: {:?}", err);
                                                        None
                                                    }
                                                }
                                            });

                                            if exceeded_budget {
                                                break;
                                            }
                                        }
                                    }
                                }
                                Err(err) => {
                                    log::error!("chrome failed to open: {:?}", err);
                                }
                            }


                        },
                        Some(result) = set.join_next(), if !set.is_empty() => {
                            if let Ok(res) = result {
                                match res {
                                    Some(page) => {
                                        if let Some(signature) = page.signature {
                                            if self.is_signature_allowed(signature).await {
                                                if let Some(mut links) = page.page_links.clone() {
                                                    self.dequeue(&mut q, &mut links, &mut exceeded_budget).await;
                                                    self.extra_links.extend(*links)
                                                }
                                                self.insert_signature(signature).await;

                                                channel_send_page(
                                                    &shared.0, page.clone(), &shared.1,
                                                );

                                                if scrape || persist_links {
                                                    if let Some(p) = self.pages.as_mut() {
                                                        p.push(page);
                                                    }
                                                }
                                            }
                                        } else {
                                            if let Some(mut links) = page.page_links.clone() {
                                                self.dequeue(&mut q, &mut links, &mut exceeded_budget).await;
                                                self.extra_links.extend(*links)
                                            }
                                            channel_send_page(
                                                &shared.0, page.clone(), &shared.1,
                                            );
                                            if scrape || persist_links {
                                                if let Some(p) = self.pages.as_mut() {
                                                    p.push(page);
                                                }
                                            }
                                        }
                                    }
                                    _ => ()
                                }
                            } else {
                                break;
                            }
                        }
                        else => break,
                    }

                    if sitemaps.len() == 0 || exceeded_budget {
                        break;
                    }
                }

                while let Some(result) = set.join_next().await {
                    if let Ok(res) = result {
                        match res {
                            Some(page) => {
                                if let Some(signature) = page.signature {
                                    if self.is_signature_allowed(signature).await {
                                        if let Some(mut links) = page.page_links.clone() {
                                            self.dequeue(&mut q, &mut links, &mut exceeded_budget)
                                                .await;
                                            self.extra_links.extend(*links)
                                        }
                                        self.insert_signature(signature).await;
                                        channel_send_page(&shared.0, page.clone(), &shared.1);
                                        if scrape || persist_links {
                                            if let Some(p) = self.pages.as_mut() {
                                                p.push(page);
                                            }
                                        }
                                    }
                                } else {
                                    if let Some(mut links) = page.page_links.clone() {
                                        self.dequeue(&mut q, &mut links, &mut exceeded_budget)
                                            .await;
                                        self.extra_links.extend(*links)
                                    }
                                    channel_send_page(&shared.0, page.clone(), &shared.1);
                                    if scrape || persist_links {
                                        if let Some(p) = self.pages.as_mut() {
                                            p.push(page);
                                        }
                                    }
                                }
                            }
                            _ => (),
                        }
                    }
                }
                b.dispose();
                self.configuration
                    .remove_sitemap_from_whitelist(whitelist_changes);
            }
        }
    }

    /// Sitemap crawl entire lists. Note: this method does not re-crawl the links of the pages found on the sitemap. This does nothing without the `sitemap` flag.
    #[cfg(feature = "sitemap")]
    pub async fn sitemap_crawl(
        &mut self,
        client: &Client,
        handle: &Option<Arc<AtomicI8>>,
        scrape: bool,
    ) {
        self.sitemap_crawl_raw(client, handle, scrape).await
    }

    /// Sitemap crawl entire lists chain. Note: this method does not re-crawl the links of the pages found on the sitemap. This does nothing without the `sitemap` flag.
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

    /// Sitemap crawl entire lists chain using chrome. Note: this method does not re-crawl the links of the pages found on the sitemap. This does nothing without the `sitemap` flag.
    #[cfg(all(
        feature = "sitemap",
        feature = "chrome",
        not(feature = "decentralized")
    ))]
    pub async fn sitemap_crawl_chain(
        &mut self,
        client: &Client,
        handle: &Option<Arc<AtomicI8>>,
        scrape: bool,
    ) {
        if !self.configuration.ignore_sitemap {
            self.sitemap_crawl_chrome(client, handle, scrape).await
        }
    }

    /// Sitemap parse entire lists. Note: this method does not re-crawl the links of the pages found on the sitemap. This does nothing without the `sitemap` flag.
    #[cfg(feature = "sitemap")]
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub async fn sitemap_parse(
        &mut self,
        client: &Client,
        first_request: &mut bool,
        sitemap_url: &mut Box<CompactString>,
        attempted_correct: &mut bool,
    ) -> bool {
        let mut valid = *attempted_correct == false;

        if valid {
            if let Some(domain) = &self.domain_parsed {
                // attempt to parse the sitemap from the html.
                match client.get(domain.as_str()).send().await {
                    Ok(response) => {
                        let limit = *crate::utils::MAX_SIZE_BYTES as u64;

                        if let Some(response_content_length) = response.content_length() {
                            if limit > 0 && response_content_length >= limit {
                                log::info!("{} exceeded parse limit: {:?}", domain, limit);
                                *first_request = true;
                                *attempted_correct = true;
                                valid = false;
                            }
                        }

                        if valid {
                            // stream the bytes to lol_html to parse the sitemap from the path.
                            let cell = tokio::sync::OnceCell::new();

                            let rewriter_settings = lol_html::Settings {
                                element_content_handlers: vec![lol_html::element!(
                                    r#"link[rel="sitemap"]"#,
                                    |el| {
                                        if let Some(href) = el.get_attribute("href") {
                                            let _ = cell.set(href);
                                        }
                                        Ok(())
                                    }
                                )],
                                adjust_charset_on_meta_tag: false,
                                ..lol_html::send::Settings::new_for_handler_types()
                            };

                            let mut rewriter = lol_html::send::HtmlRewriter::new(
                                rewriter_settings,
                                |_c: &[u8]| {},
                            );

                            let mut wrote_error = false;
                            let mut stream = response.bytes_stream();

                            while let Some(chunk) = stream.next().await {
                                if let Ok(chunk) = chunk {
                                    if rewriter.write(&chunk).is_err() {
                                        wrote_error = true;
                                        break;
                                    }
                                }
                                if cell.initialized() {
                                    break;
                                }
                            }

                            if !wrote_error {
                                let _ = rewriter.end();
                            }

                            if let Some(sitemap) = cell.get() {
                                if sitemap.is_empty() {
                                    *first_request = true;
                                }

                                if let Err(_) = domain.join(sitemap) {
                                    *first_request = true;
                                }
                                // if we retried the request here it should succeed.
                                *sitemap_url = Box::new(sitemap.into());
                                *attempted_correct = true;
                            } else {
                                *first_request = true;
                            }
                        }
                    }
                    Err(err) => {
                        *first_request = true;
                        valid = false;
                        log::info!("http parse error: {:?}", err.to_string())
                    }
                };
            }
        }

        valid
    }
    /// Sitemap parse entire lists. Note: this method does not re-crawl the links of the pages found on the sitemap. This does nothing without the `sitemap` flag.
    #[cfg(feature = "sitemap")]
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    async fn sitemap_parse_crawl(
        &mut self,
        client: &Client,
        handle: &Option<Arc<AtomicI8>>,
        b: bytes::Bytes,
        mut interval: &mut Interval,
        exceeded_budget: &mut bool,
        tx: &tokio::sync::mpsc::Sender<Page>,
        sitemaps: &mut Vec<Box<CompactString>>,
        crawl: bool,
    ) {
        use sitemap::reader::{SiteMapEntity, SiteMapReader};
        use sitemap::structs::Location;

        if !b.is_empty() && b.starts_with(b"<?xml") {
            let mut stream = tokio_stream::iter(SiteMapReader::new(&*b));

            let retry = self.configuration.retry;

            while let Some(entity) = stream.next().await {
                if !self.handle_process(handle, &mut interval, async {}).await {
                    break;
                }
                match entity {
                    SiteMapEntity::Url(url_entry) => match url_entry.loc {
                        Location::Url(url) => {
                            let link: CaseInsensitiveString = url.as_str().into();

                            let allowed = self.is_allowed(&link);

                            if allowed.eq(&ProcessLinkStatus::Blocked) {
                                continue;
                            }

                            if allowed.eq(&ProcessLinkStatus::BudgetExceeded) {
                                *exceeded_budget = true;
                                break;
                            }

                            self.insert_link(link.clone()).await;

                            if crawl {
                                let client = client.clone();
                                let tx = tx.clone();

                                crate::utils::spawn_task("page_fetch", async move {
                                    let mut page = Page::new_page(&link.inner(), &client).await;

                                    let mut retry_count = retry;

                                    while page.should_retry && retry_count > 0 {
                                        if let Some(timeout) = page.get_timeout() {
                                            tokio::time::sleep(timeout).await;
                                        }
                                        page.clone_from(
                                            &Page::new_page(link.inner(), &client).await,
                                        );
                                        retry_count -= 1;
                                    }

                                    if let Ok(permit) = tx.reserve().await {
                                        permit.send(page);
                                    }
                                });
                            }
                        }
                        Location::None | Location::ParseErr(_) => (),
                    },
                    SiteMapEntity::SiteMap(sitemap_entry) => match sitemap_entry.loc {
                        Location::Url(url) => {
                            sitemaps.push(Box::new(CompactString::new(&url.as_str())));
                        }
                        Location::None | Location::ParseErr(_) => (),
                    },
                    SiteMapEntity::Err(err) => {
                        log::info!("incorrect sitemap error: {:?}", err.msg())
                    }
                };

                if *exceeded_budget {
                    break;
                }
            }
        }
    }

    /// get base link for crawl establishing.
    #[cfg(feature = "regex")]
    pub fn get_base_link(&self) -> &CaseInsensitiveString {
        &self.url
    }

    /// get base link for crawl establishing.
    #[cfg(not(feature = "regex"))]
    pub fn get_base_link(&self) -> &CompactString {
        self.url.inner()
    }

    /// Guard the channel from closing until all subscription events complete.
    pub async fn subscription_guard(&self) {
        if let Some(channel) = &self.channel {
            if !channel.1.is_empty() {
                if let Some(guard_counter) = &self.channel_guard {
                    guard_counter.lock().await
                }
            }
        }
    }

    /// Launch or connect to browser with setup.
    #[cfg(feature = "chrome")]
    pub async fn setup_browser_base(
        config: &Configuration,
        url_parsed: &Option<Box<Url>>,
        jar: Option<&Arc<reqwest::cookie::Jar>>,
    ) -> Option<crate::features::chrome::BrowserController> {
        match crate::features::chrome::launch_browser_cookies(&config, url_parsed, jar).await {
            Some((browser, browser_handle, context_id)) => {
                let browser: Arc<chromiumoxide::Browser> = Arc::new(browser);
                let b = (browser, Some(browser_handle), context_id);

                Some(crate::features::chrome::BrowserController::new(b))
            }
            _ => None,
        }
    }

    /// Launch or connect to browser with setup.
    #[cfg(feature = "chrome")]
    pub async fn setup_browser(&self) -> Option<crate::features::chrome::BrowserController> {
        Website::setup_browser_base(
            &self.configuration,
            self.get_url_parsed(),
            Some(&self.cookie_jar),
        )
        .await
    }

    /// Launch or connect to WebDriver with setup.
    #[cfg(feature = "webdriver")]
    pub async fn setup_webdriver(&self) -> Option<crate::features::webdriver::WebDriverController> {
        crate::features::webdriver::launch_driver(&self.configuration).await
    }

    /// Render a page using WebDriver.
    #[cfg(feature = "webdriver")]
    pub async fn render_webdriver_page(
        &self,
        url: &str,
        driver: &std::sync::Arc<thirtyfour::WebDriver>,
    ) -> Option<String> {
        use crate::features::webdriver::{attempt_navigation, get_page_content, setup_driver_events};

        let timeout = self
            .configuration
            .webdriver_config
            .as_ref()
            .and_then(|c| c.timeout);

        // Navigate to the URL
        if let Err(e) = attempt_navigation(url, driver, &timeout).await {
            log::error!("WebDriver navigation failed: {:?}", e);
            return None;
        }

        // Setup events (stealth injection)
        setup_driver_events(driver, &self.configuration).await;

        // Get page content
        match get_page_content(driver).await {
            Ok(content) => Some(content),
            Err(e) => {
                log::error!("Failed to get WebDriver page content: {:?}", e);
                None
            }
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

    /// Bypass CSP protection detection. This does nothing without the feat flag `chrome` enabled.
    pub fn with_csp_bypass(&mut self, enabled: bool) -> &mut Self {
        self.configuration.with_csp_bypass(enabled);
        self
    }

    /// Configure WebDriver for browser automation. This does nothing without the `webdriver` feature flag enabled.
    /// When configured, the `crawl()` function will automatically use WebDriver instead of raw HTTP.
    #[cfg(feature = "webdriver")]
    pub fn with_webdriver(
        &mut self,
        webdriver_config: crate::features::webdriver_common::WebDriverConfig,
    ) -> &mut Self {
        self.configuration
            .with_webdriver_config(Some(webdriver_config));
        self
    }

    /// Configure WebDriver for browser automation. This does nothing without the `webdriver` feature flag enabled.
    #[cfg(not(feature = "webdriver"))]
    pub fn with_webdriver(&mut self, _webdriver_config: ()) -> &mut Self {
        self
    }

    /// Use sqlite to store data and track large crawls. This does nothing without the `disk` flag enabled.
    #[cfg(feature = "disk")]
    pub fn with_sqlite(&mut self, sqlite: bool) -> &mut Self {
        if sqlite {
            self.enable_sqlite = true;
        } else {
            self.enable_sqlite = false;
            self.sqlite = None;
        };
        self
    }

    /// Use sqlite to store data and track large crawls.
    #[cfg(not(feature = "disk"))]
    pub fn with_sqlite(&mut self, _sqlite: bool) -> &mut Self {
        self
    }

    /// Include tld detection.
    pub fn with_tld(&mut self, tld: bool) -> &mut Self {
        self.configuration.with_tld(tld);
        self
    }

    /// The max duration for the crawl. This is useful when websites use a robots.txt with long durations and throttle the timeout removing the full concurrency.
    pub fn with_crawl_timeout(&mut self, crawl_timeout: Option<Duration>) -> &mut Self {
        self.configuration.with_crawl_timeout(crawl_timeout);
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

    /// Use proxies for request with control between chrome and http.
    pub fn with_proxies_direct(
        &mut self,
        proxies: Option<Vec<crate::configuration::RequestProxy>>,
    ) -> &mut Self {
        self.configuration.with_proxies_direct(proxies);
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

    /// Skip setting up a control thread for pause, start, and shutdown programmatic handling. This does nothing without the 'control' flag enabled.
    pub fn with_no_control_thread(&mut self, no_control_thread: bool) -> &mut Self {
        self.configuration.with_no_control_thread(no_control_thread);
        self
    }

    /// Add whitelist urls to allow.
    pub fn with_whitelist_url<T>(&mut self, whitelist_url: Option<Vec<T>>) -> &mut Self
    where
        Vec<CompactString>: From<Vec<T>>,
    {
        self.configuration.with_whitelist_url(whitelist_url);
        self
    }

    #[cfg(feature = "chrome")]
    /// Track the events made via chrome.
    pub fn with_event_tracker(
        &mut self,
        track_events: Option<crate::configuration::ChromeEventTracker>,
    ) -> &mut Self {
        self.configuration.with_event_tracker(track_events);
        self
    }

    /// Set HTTP headers for request using [reqwest::header::HeaderMap](https://docs.rs/reqwest/latest/reqwest/header/struct.HeaderMap.html).
    pub fn with_headers(&mut self, headers: Option<reqwest::header::HeaderMap>) -> &mut Self {
        self.configuration.with_headers(headers);
        self
    }

    /// Modify the headers to mimic a real browser.
    pub fn with_modify_headers(&mut self, modify_headers: bool) -> &mut Self {
        self.configuration.with_modify_headers(modify_headers);
        self
    }

    /// Modify the HTTP client headers to mimic a real browser.
    pub fn with_modify_http_client_headers(
        &mut self,
        modify_http_client_headers: bool,
    ) -> &mut Self {
        self.configuration
            .with_modify_http_client_headers(modify_http_client_headers);
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

    /// Set a crawl depth limit. If the value is 0 there is no limit.
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
        on_link_find_callback: Option<OnLinkFindCallback>,
    ) -> &mut Self {
        match on_link_find_callback {
            Some(callback) => self.on_link_find_callback = Some(callback),
            _ => self.on_link_find_callback = None,
        };
        self
    }

    /// Perform a callback to run on each link find shorthand.
    pub fn set_on_link_find<F>(&mut self, f: F)
    where
        F: Fn(CaseInsensitiveString, Option<String>) -> (CaseInsensitiveString, Option<String>)
            + Send
            + Sync
            + 'static,
    {
        self.on_link_find_callback = Some(Arc::new(f));
    }

    /// Use a callback to determine if a page should be ignored. Return false to ensure that the discovered links are not crawled.
    pub fn with_on_should_crawl_callback(
        &mut self,
        on_should_crawl_callback: Option<fn(&Page) -> bool>,
    ) -> &mut Self {
        match on_should_crawl_callback {
            Some(callback) => {
                self.on_should_crawl_callback = Some(OnShouldCrawlCallback::Fn(callback))
            }
            _ => self.on_should_crawl_callback = None,
        };
        self
    }

    /// Use an immutable closure to determine if a page should be ignored. Return false to ensure that the discovered links are not crawled.
    ///
    /// Slightly slower than [`Self::with_on_should_crawl_callback`].
    pub fn with_on_should_crawl_callback_closure<F: OnShouldCrawlClosure>(
        &mut self,
        on_should_crawl_closure: Option<F>,
    ) -> &mut Self {
        match on_should_crawl_closure {
            Some(callback) => {
                self.on_should_crawl_callback =
                    Some(OnShouldCrawlCallback::Closure(Arc::new(callback)))
            }
            _ => self.on_should_crawl_callback = None,
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

    /// Use stealth mode for the request. This does nothing without the `chrome` flag enabled.
    #[cfg(feature = "chrome")]
    pub fn with_stealth_advanced(
        &mut self,
        stealth_mode: spider_fingerprint::configs::Tier,
    ) -> &mut Self {
        self.configuration.with_stealth_advanced(stealth_mode);
        self
    }

    /// Set the cache policy.
    pub fn with_cache_policy(
        &mut self,
        cache_policy: Option<crate::utils::BasicCachePolicy>,
    ) -> &mut Self {
        self.configuration.with_cache_policy(cache_policy);

        self
    }

    /// Use OpenAI to get dynamic javascript to drive the browser. This does nothing without the `openai` flag enabled.
    pub fn with_openai(&mut self, openai_configs: Option<configuration::GPTConfigs>) -> &mut Self {
        self.configuration.with_openai(openai_configs);
        self
    }

    /// Use a remote multimodal model (vision + HTML + URL) to drive browser automation.
    ///
    /// When enabled, Spider can ask an OpenAI-compatible “chat completions” endpoint to
    /// generate a JSON plan (a list of `WebAutomation` steps), execute those steps against a
    /// live Chrome page, then re-capture state and iterate until the model reports it is done
    /// (or the configured limits are hit). The default system prompt is set to handle web challenges that can be adjusted if required.
    /// Take a look at `DEFAULT_SYSTEM_PROMPT` at spider::features::automation::DEFAULT_SYSTEM_PROMPT for a base line.
    ///
    /// This is useful for:
    /// - handling captchas,
    /// - dismissing popups / cookie banners,
    /// - navigating to a target page (pricing, docs, etc.),
    /// - clicking through multi-step UI flows,
    /// - recovering from dynamic page state that plain HTML scraping can’t handle.
    ///
    /// # Feature gate
    /// This method only has an effect when the crate is built with `feature="chrome"`.
    /// Without `chrome`, the method is not available.
    ///
    /// # Parameters
    /// - `cfg`: The remote multimodal configuration bundle (endpoint, model, prompts, and runtime knobs).
    ///   Pass `None` to disable remote multimodal automation.
    ///
    /// # Example
    /// ```no_run
    /// # #[cfg(feature = "chrome")]
    /// # async fn demo() -> Result<(), Box<dyn std::error::Error>> {
    /// use spider::website::Website;
    /// use spider::configuration::Configuration;
    /// use spider::features::automation::{RemoteMultimodalConfigs, RemoteMultimodalConfig};
    ///
    /// // Build the engine configs (similar to GPTConfigs::new(...))
    /// let mm_cfgs = RemoteMultimodalConfigs::new(
    ///     "http://localhost:11434/v1/chat/completions",
    ///     "qwen2.5-vl", // any OpenAI-compatible model id your endpoint understands
    /// )
    /// .with_api_key(None)
    /// .with_system_prompt_extra(Some("Never log in. Prefer closing popups and continuing."))
    /// .with_user_message_extra(Some("Goal: reach the pricing page, then stop."))
    /// .with_cfg(RemoteMultimodalConfig {
    ///     // keep HTML smaller if you want lower token usage
    ///     include_html: true,
    ///     html_max_bytes: 24_000,
    ///     include_url: true,
    ///     include_title: true,
    ///     // loop controls
    ///     max_rounds: 6,
    ///     post_plan_wait_ms: 400,
    ///     ..Default::default()
    /// })
    /// .with_concurrency_limit(Some(8));
    ///
    /// // Attach to the crawler configuration
    /// let mut cfg = Configuration::new();
    /// cfg.with_remote_multimodal(Some(mm_cfgs));
    ///
    /// // Use the configuration in a Website (example)
    /// let mut site = Website::new("https://example.com");
    /// site.configuration = cfg;
    ///
    /// // Start crawling/scraping as you normally would...
    /// // site.crawl().await?;
    ///
    /// Ok(())
    /// # }
    /// ```
    ///
    /// # Notes
    /// - Remote multimodal automation typically requires `feature="serde"` if you deserialize model
    ///   steps into `WebAutomation`.
    /// - If your endpoint does not support `response_format: {"type":"json_object"}`, disable that
    ///   in `RemoteMultimodalConfig` (`request_json_object = false`).
    #[cfg(feature = "chrome")]
    pub fn with_remote_multimodal(
        &mut self,
        cfg: Option<crate::features::automation::RemoteMultimodalConfigs>,
    ) -> &mut Self {
        self.configuration.with_remote_multimodal(cfg);
        self
    }

    /// Use Gemini to get dynamic javascript to drive the browser. This does nothing without the `gemini` flag enabled.
    pub fn with_gemini(
        &mut self,
        gemini_configs: Option<configuration::GeminiConfigs>,
    ) -> &mut Self {
        self.configuration.with_gemini(gemini_configs);
        self
    }

    /// Cache the page following HTTP rules. This method does nothing if the `cache` feature is not enabled.
    pub fn with_caching(&mut self, cache: bool) -> &mut Self {
        self.configuration.with_caching(cache);
        self
    }

    /// Enable or disable Service Workers. This method does nothing if the `chrome` feature is not enabled.
    pub fn with_service_worker_enabled(&mut self, enabled: bool) -> &mut Self {
        self.configuration.with_service_worker_enabled(enabled);
        self
    }

    /// Automatically setup geo-location configurations when using a proxy. This method does nothing if the `chrome` feature is not enabled.
    pub fn with_auto_geolocation(&mut self, enabled: bool) -> &mut Self {
        self.configuration.with_auto_geolocation(enabled);
        self
    }

    #[cfg(feature = "chrome")]
    /// Set custom fingerprint ID for request. This does nothing without the `chrome` flag enabled.
    pub fn with_fingerprint_advanced(
        &mut self,
        fingerprint: crate::configuration::Fingerprint,
    ) -> &mut Self {
        self.configuration.with_fingerprint_advanced(fingerprint);
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

    /// Wait for network request to be idle within a time frame period (500ms no network connections). This does nothing without the `chrome` flag enabled.
    pub fn with_wait_for_idle_network(
        &mut self,
        wait_for_idle_network: Option<crate::configuration::WaitForIdleNetwork>,
    ) -> &mut Self {
        self.configuration
            .with_wait_for_idle_network(wait_for_idle_network);
        self
    }

    /// Wait for network request with a max timeout. This does nothing without the `chrome` flag enabled.
    pub fn with_wait_for_idle_network0(
        &mut self,
        wait_for_idle_network: Option<crate::configuration::WaitForIdleNetwork>,
    ) -> &mut Self {
        self.configuration
            .with_wait_for_idle_network0(wait_for_idle_network);
        self
    }

    /// Wait for network to be almost idle with a max timeout. This does nothing without the `chrome` flag enabled.
    pub fn with_wait_for_almost_idle_network0(
        &mut self,
        wait_for_idle_network: Option<crate::configuration::WaitForIdleNetwork>,
    ) -> &mut Self {
        self.configuration
            .with_wait_for_almost_idle_network0(wait_for_idle_network);
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

    /// Wait for idle dom mutations for target element. This method does nothing if the `chrome` feature is not enabled.
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

    /// The default http connect timeout.
    pub fn with_default_http_connect_timeout(
        &mut self,
        default_http_connect_timeout: Option<Duration>,
    ) -> &mut Self {
        self.configuration
            .with_default_http_connect_timeout(default_http_connect_timeout);

        self
    }

    /// The default http read timeout.
    pub fn with_default_http_read_timeout(
        &mut self,
        default_http_read_timeout: Option<Duration>,
    ) -> &mut Self {
        self.configuration
            .with_default_http_read_timeout(default_http_read_timeout);

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
            .with_chrome_intercept(chrome_intercept, &self.domain_parsed);
        self
    }

    /// Add a referer to the request.
    pub fn with_referer(&mut self, referer: Option<String>) -> &mut Self {
        self.configuration.with_referer(referer);
        self
    }

    /// Add a referer to the request.
    pub fn with_referrer(&mut self, referer: Option<String>) -> &mut Self {
        self.configuration.with_referrer(referer);
        self
    }

    /// Determine whether to collect all the resources found on pages.
    pub fn with_full_resources(&mut self, full_resources: bool) -> &mut Self {
        self.configuration.with_full_resources(full_resources);
        self
    }

    /// Dismiss all dialogs on the page. This method does nothing if the `chrome` feature is not enabled.
    pub fn with_dismiss_dialogs(&mut self, full_resources: bool) -> &mut Self {
        self.configuration.with_dismiss_dialogs(full_resources);
        self
    }

    /// Set the request emuluation. This method does nothing if the `wreq` flag is not enabled.
    #[cfg(feature = "wreq")]
    pub fn with_emulation(&mut self, emulation: Option<wreq_util::Emulation>) -> &mut Self {
        self.configuration.with_emulation(emulation);
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

    /// Set a crawl page limit. If the value is 0 there is no limit.
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

    /// Bind the connections only on the network interface.
    pub fn with_network_interface(&mut self, network_interface: Option<String>) -> &mut Self {
        self.configuration.with_network_interface(network_interface);
        self
    }

    /// Bind to a local IP Address.
    pub fn with_local_address(&mut self, local_address: Option<IpAddr>) -> &mut Self {
        self.configuration.with_local_address(local_address);
        self
    }

    /// Block assets from loading from the network. Focus primarly on HTML documents.
    pub fn with_block_assets(&mut self, only_html: bool) -> &mut Self {
        self.configuration.with_block_assets(only_html);
        self
    }

    /// Normalize the content de-duplicating trailing slash pages and other pages that can be duplicated. This may initially show the link in your links_visited or subscription calls but, the following links will not be crawled.
    pub fn with_normalize(&mut self, normalize: bool) -> &mut Self {
        self.configuration.with_normalize(normalize);
        self
    }

    /// Store all the links found on the disk to share the state. This does nothing without the `disk` flag enabled.
    pub fn with_shared_state(&mut self, shared: bool) -> &mut Self {
        self.configuration.with_shared_state(shared);
        self
    }

    /// Set the max amount of bytes to collect per page. Only used for chrome atm.
    pub fn with_max_page_bytes(&mut self, max_page_bytes: Option<f64>) -> &mut Self {
        self.configuration.with_max_page_bytes(max_page_bytes);
        self
    }

    /// Set the max amount of bytes to collected for the browser context. Only used for chrome atm.
    pub fn with_max_bytes_allowed(&mut self, max_bytes_allowed: Option<u64>) -> &mut Self {
        self.configuration.with_max_bytes_allowed(max_bytes_allowed);
        self
    }

    /// Set the configuration for the website directly.
    pub fn with_config(&mut self, config: Configuration) -> &mut Self {
        self.configuration = config.into();
        self
    }

    /// Set a [spider.cloud](https://spider.cloud) API key (Proxy mode).
    #[cfg(feature = "spider_cloud")]
    pub fn with_spider_cloud(&mut self, api_key: &str) -> &mut Self {
        self.configuration.with_spider_cloud(api_key);
        self
    }

    /// Set a [spider.cloud](https://spider.cloud) API key (no-op without `spider_cloud` feature).
    #[cfg(not(feature = "spider_cloud"))]
    pub fn with_spider_cloud(&mut self, _api_key: &str) -> &mut Self {
        self
    }

    /// Set a [spider.cloud](https://spider.cloud) config.
    #[cfg(feature = "spider_cloud")]
    pub fn with_spider_cloud_config(
        &mut self,
        config: crate::configuration::SpiderCloudConfig,
    ) -> &mut Self {
        self.configuration.with_spider_cloud_config(config);
        self
    }

    /// Set a [spider.cloud](https://spider.cloud) config (no-op without `spider_cloud` feature).
    #[cfg(not(feature = "spider_cloud"))]
    pub fn with_spider_cloud_config(&mut self, _config: ()) -> &mut Self {
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

    /// Clear the HTTP headers for the requests.
    pub fn clear_headers(&mut self) {
        if let Some(headers) = self.configuration.headers.as_mut() {
            headers.0.clear();
        }
    }

    /// Determine if the budget has a wildcard path and the depth limit distance. This does nothing without the `budget` flag enabled.
    pub fn determine_limits(&mut self) {
        self.configuration.configure_budget();
        if self.configuration.inner_budget.is_some() {
            let wild_card_budget = match &self.configuration.inner_budget {
                Some(budget) => budget.contains_key(&*WILD_CARD_PATH),
                _ => false,
            };
            self.configuration.wild_card_budgeting = wild_card_budget;
        }
        if self.configuration.depth > 0 && self.domain_parsed.is_some() {
            if let Some(domain) = &self.domain_parsed {
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
        _capacity: usize,
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

    /// Get the channel sender to send manual subscriptions.
    pub fn get_channel(
        &self,
    ) -> &Option<(broadcast::Sender<Page>, Arc<broadcast::Receiver<Page>>)> {
        &self.channel
    }

    /// Get the channel guard to send manual subscriptions from closing.
    pub fn get_channel_guard(&self) -> &Option<ChannelGuard> {
        &self.channel_guard
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

    /// Set extra useful information.
    #[cfg(feature = "extra_information")]
    pub fn set_extra_info(&mut self, info: Option<String>) {
        self.extra_info = info.map(|f| f.into());
    }

    /// Get extra information stored.
    #[cfg(feature = "extra_information")]
    pub fn get_extra_info(&self) -> Option<&Box<String>> {
        self.extra_info.as_ref()
    }

    /// Set the initial HTML page instead of firing a request to the URL.
    pub fn set_seeded_html(&mut self, html: Option<String>) {
        self.seed_html = html;
    }

    /// Get the initial seeded html.
    pub fn get_seeded_html(&self) -> &Option<String> {
        &self.seed_html
    }

    /// Apply configuration from a `PromptConfiguration` generated by an LLM.
    ///
    /// This method takes a configuration object produced by
    /// `RemoteMultimodalEngine::configure_from_prompt()` and applies the
    /// settings to this website.
    ///
    /// # Example
    /// ```ignore
    /// use spider::features::automation::{RemoteMultimodalEngine, configure_crawler_from_prompt};
    ///
    /// let config = configure_crawler_from_prompt(
    ///     "http://localhost:11434/v1/chat/completions",
    ///     "llama3",
    ///     None,
    ///     "Crawl blog posts only, respect robots.txt, max 100 pages, 200ms delay"
    /// ).await?;
    ///
    /// let mut website = Website::new("https://example.com");
    /// website.apply_prompt_configuration(&config);
    /// ```
    #[cfg(feature = "serde")]
    pub fn apply_prompt_configuration(
        &mut self,
        config: &crate::features::automation::PromptConfiguration,
    ) -> &mut Self {
        // Core crawling
        if let Some(v) = config.respect_robots_txt {
            self.configuration.respect_robots_txt = v;
        }
        if let Some(v) = config.subdomains {
            self.configuration.subdomains = v;
        }
        if let Some(v) = config.tld {
            self.configuration.tld = v;
        }
        if let Some(v) = config.depth {
            self.configuration.depth = v;
        }
        if let Some(v) = config.delay {
            self.configuration.delay = v;
        }
        if let Some(ms) = config.request_timeout_ms {
            self.configuration.request_timeout =
                Some(Box::new(std::time::Duration::from_millis(ms)));
        }
        if let Some(ms) = config.crawl_timeout_ms {
            self.configuration.crawl_timeout = Some(std::time::Duration::from_millis(ms));
        }

        // URL filtering
        if let Some(ref urls) = config.blacklist_url {
            self.configuration.blacklist_url =
                Some(urls.iter().map(|s| s.as_str().into()).collect());
        }
        if let Some(ref urls) = config.whitelist_url {
            self.configuration.whitelist_url =
                Some(urls.iter().map(|s| s.as_str().into()).collect());
        }
        if let Some(ref domains) = config.external_domains {
            for domain in domains {
                self.configuration
                    .external_domains_caseless
                    .insert(case_insensitive_string::CaseInsensitiveString::new(domain));
            }
        }

        // Request settings
        if let Some(ref ua) = config.user_agent {
            self.configuration.user_agent = Some(Box::new(ua.as_str().into()));
        }
        if let Some(v) = config.http2_prior_knowledge {
            self.configuration.http2_prior_knowledge = v;
        }
        if let Some(v) = config.accept_invalid_certs {
            self.configuration.accept_invalid_certs = v;
        }

        // Limits
        if let Some(v) = config.redirect_limit {
            self.configuration.redirect_limit = Box::new(v);
        }
        if let Some(ref budget_map) = config.budget {
            let mut budget = hashbrown::HashMap::new();
            for (k, v) in budget_map {
                budget.insert(
                    case_insensitive_string::CaseInsensitiveString::new(k),
                    *v,
                );
            }
            self.configuration.budget = Some(budget);
        }
        if let Some(v) = config.max_page_bytes {
            self.configuration.max_page_bytes = Some(v);
        }

        // Content
        if let Some(v) = config.full_resources {
            self.configuration.full_resources = v;
        }
        if let Some(v) = config.only_html {
            self.configuration.only_html = v;
        }
        if let Some(v) = config.return_page_links {
            self.configuration.return_page_links = v;
        }

        // Chrome options
        #[cfg(feature = "chrome")]
        if let Some(true) = config.use_chrome {
            // Chrome is enabled via feature flag, this is a hint for the user
        }
        if let Some(ref mode) = config.stealth_mode {
            self.configuration.stealth_mode = match mode.to_lowercase().as_str() {
                "basic" => spider_fingerprint::configs::Tier::Basic,
                "low" => spider_fingerprint::configs::Tier::Low,
                "mid" => spider_fingerprint::configs::Tier::Mid,
                "full" => spider_fingerprint::configs::Tier::Full,
                _ => spider_fingerprint::configs::Tier::None,
            };
        }
        if config.viewport_width.is_some() || config.viewport_height.is_some() {
            let width = config.viewport_width.unwrap_or(800);
            let height = config.viewport_height.unwrap_or(600);
            self.configuration.viewport = Some(crate::configuration::Viewport::new(width, height));
        }
        #[cfg(feature = "chrome")]
        {
            let mut wait_for = self
                .configuration
                .wait_for
                .take()
                .unwrap_or_default();

            if let Some(true) = config.wait_for_idle_network {
                wait_for.idle_network = Some(
                    crate::features::chrome_common::WaitForIdleNetwork::new(Some(
                        std::time::Duration::from_secs(30),
                    )),
                );
            }
            if let Some(ms) = config.wait_for_delay_ms {
                wait_for.delay = Some(crate::features::chrome_common::WaitForDelay::new(Some(
                    std::time::Duration::from_millis(ms),
                )));
            }
            if let Some(ref selector) = config.wait_for_selector {
                wait_for.selector = Some(crate::features::chrome_common::WaitForSelector::new(
                    Some(std::time::Duration::from_secs(30)),
                    selector.clone(),
                ));
            }

            if wait_for.idle_network.is_some()
                || wait_for.delay.is_some()
                || wait_for.selector.is_some()
            {
                self.configuration.wait_for = Some(wait_for);
            }
        }
        #[cfg(feature = "chrome")]
        if let Some(ref js) = config.evaluate_on_new_document {
            self.configuration.evaluate_on_new_document = Some(Box::new(js.clone()));
        }

        // Performance
        if let Some(v) = config.shared_queue {
            self.configuration.shared_queue = v;
        }
        if let Some(v) = config.retry {
            self.configuration.retry = v;
        }

        self
    }

    /// Configure the website from a natural language prompt using an LLM.
    ///
    /// This is a convenience method that calls the LLM to generate configuration
    /// and applies it to the website in one step.
    ///
    /// # Arguments
    /// * `api_url` - OpenAI-compatible chat completions endpoint
    /// * `model_name` - Model identifier (e.g., "gpt-4", "llama3", "qwen2.5")
    /// * `api_key` - Optional API key for authenticated endpoints
    /// * `prompt` - Natural language description of crawling requirements
    ///
    /// # Example
    /// ```ignore
    /// let mut website = Website::new("https://example.com");
    /// website.configure_from_prompt(
    ///     "http://localhost:11434/v1/chat/completions",
    ///     "llama3",
    ///     None,
    ///     "Only crawl product pages, use 100ms delay, max depth 5, respect robots.txt"
    /// ).await?;
    ///
    /// website.crawl().await;
    /// ```
    /// Requires the `agent` and `serde` features.
    #[cfg(all(feature = "agent", feature = "serde"))]
    pub async fn configure_from_prompt(
        &mut self,
        api_url: &str,
        model_name: &str,
        api_key: Option<&str>,
        prompt: &str,
    ) -> Result<&mut Self, crate::features::automation::EngineError> {
        let config = crate::features::automation::configure_crawler_from_prompt(
            api_url, model_name, api_key, prompt,
        )
        .await?;
        Ok(self.apply_prompt_configuration(&config))
    }
}

/// Channel broadcast send the Page to receivers.
pub fn channel_send_page(
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
pub struct ChannelGuard(Arc<(AtomicBool, AtomicUsize, AtomicUsize)>);

impl ChannelGuard {
    /// Create a new channel guard. The tuple has the guard control and the counter.
    pub(crate) fn new() -> ChannelGuard {
        ChannelGuard(Arc::new((
            AtomicBool::new(true),
            AtomicUsize::new(0),
            AtomicUsize::new(0),
        )))
    }
    /// Lock the channel until complete. This is only used for when storing the chrome page outside.
    pub(crate) async fn lock(&self) {
        if self.0 .0.load(Ordering::Relaxed) {
            let old = self.0 .1.load(Ordering::Relaxed);

            while self
                .0
                 .2
                .compare_exchange_weak(old, 0, Ordering::Acquire, Ordering::Relaxed)
                .is_err()
            {
                tokio::task::yield_now().await;
            }
            std::sync::atomic::fence(Ordering::Acquire);
        }
    }

    /// Set the guard control manually. If this is set to false the loop will not enter.
    pub fn guard(&mut self, guard: bool) {
        self.0 .0.store(guard, Ordering::Release);
    }

    /// Increment the guard channel completions.
    // rename on next major since logic is now flow-controlled.
    pub fn inc(&mut self) {
        self.0 .2.fetch_add(1, std::sync::atomic::Ordering::Release);
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

#[tokio::test]
#[cfg(not(feature = "decentralized"))]
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

#[tokio::test]
#[cfg(feature = "cron")]
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

#[tokio::test]
#[cfg(feature = "cron")]
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

#[tokio::test]
#[cfg(not(feature = "decentralized"))]
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
    website.configuration.blacklist_url = Some(Vec::from([CompactString::from(
        "https://choosealicense.com/licenses/",
    )]));

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

    website.configure_robots_parser(&client).await;

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
    website_second.configure_robots_parser(&client_second).await;

    assert!(!&website
        .is_allowed(&"https://www.mongodb.com/community/forums/auth/".into())
        .eq(&ProcessLinkStatus::Allowed));

    // assert_eq!(website_second.configuration.delay, 60000); // should equal one minute in ms
}

#[tokio::test]
#[cfg(not(feature = "decentralized"))]
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
#[cfg(all(
    not(feature = "regex"),
    not(feature = "openai"),
    not(feature = "gemini")
))]
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

#[tokio::test]
#[cfg(all(feature = "glob", not(feature = "decentralized")))]
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

#[tokio::test]
#[ignore]
#[cfg(not(feature = "decentralized"))]
async fn test_crawl_tld() {
    let mut website: Website = Website::new("https://choosealicense.com");
    website.configuration.tld = true;
    website.with_limit(10);
    website.crawl().await;

    assert!(
        website.links_visited.len() > 1,
        "expected more than 1 link visited with tld enabled, got {:?}",
        website.links_visited
    );
}

#[tokio::test]
#[cfg(all(feature = "sync", not(feature = "decentralized")))]
async fn test_crawl_subscription() {
    let mut website: Website = Website::new("https://choosealicense.com");
    let mut rx2 = website.subscribe(100).unwrap();

    let join_handle = tokio::spawn(async move {
        let mut count = 0;

        while let Ok(_) = rx2.recv().await {
            count += 1;
        }
        count
    });

    website.crawl().await;
    website.unsubscribe();
    let website_links = website.get_links().len();
    let count = join_handle.await.unwrap();

    // no subscription if did not fulfill. The root page is always captured in links.
    assert!(count == website_links, "{:?}", true);
}

#[tokio::test]
#[cfg(all(feature = "socks", not(feature = "decentralized")))]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(feature = "decentralized"))]
    #[test]
    fn test_client_rotator_round_robin() {
        // Build 3 simple clients to verify round-robin cycling.
        let clients: Vec<Client> = (0..3)
            .map(|_| {
                #[cfg(not(feature = "cache_request"))]
                {
                    unsafe { crate::ClientBuilder::new().build().unwrap_unchecked() }
                }
                #[cfg(feature = "cache_request")]
                {
                    reqwest_middleware::ClientBuilder::new(unsafe {
                        reqwest::ClientBuilder::new().build().unwrap_unchecked()
                    })
                    .build()
                }
            })
            .collect();

        let rotator = ClientRotator::new(clients);
        assert_eq!(rotator.len(), 3);
        assert!(!rotator.is_empty());

        // Each call to next() should advance the index.
        // We verify the pattern cycles by checking the internal index.
        let _ = rotator.next(); // index 0
        let _ = rotator.next(); // index 1
        let _ = rotator.next(); // index 2
        let _ = rotator.next(); // index 3 -> wraps to 0

        // After 4 calls, the atomic index should be 4.
        let current_idx = rotator.index.load(Ordering::Relaxed);
        assert_eq!(current_idx, 4);
    }

    #[cfg(not(feature = "decentralized"))]
    #[test]
    fn test_build_rotated_clients_with_multiple_proxies() {
        let mut website = Website::new("http://example.com");
        website.configuration.with_proxies(Some(vec![
            "http://proxy1.example.com:8080".to_string(),
            "http://proxy2.example.com:8080".to_string(),
            "http://proxy3.example.com:8080".to_string(),
        ]));

        let rotator = website.build_rotated_clients();
        assert!(rotator.is_some(), "Should build rotator with 3 proxies");
        let rotator = rotator.unwrap();
        assert_eq!(rotator.len(), 3);
    }

    #[cfg(not(feature = "decentralized"))]
    #[test]
    fn test_build_rotated_clients_single_proxy_returns_none() {
        let mut website = Website::new("http://example.com");
        website.configuration.with_proxies(Some(vec![
            "http://proxy1.example.com:8080".to_string(),
        ]));

        let rotator = website.build_rotated_clients();
        assert!(
            rotator.is_none(),
            "Should not build rotator with only 1 proxy"
        );
    }

    #[cfg(not(feature = "decentralized"))]
    #[test]
    fn test_build_rotated_clients_no_proxies_returns_none() {
        let website = Website::new("http://example.com");
        let rotator = website.build_rotated_clients();
        assert!(
            rotator.is_none(),
            "Should not build rotator with no proxies"
        );
    }
}
