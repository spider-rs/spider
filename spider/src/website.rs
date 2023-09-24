use crate::black_list::contains;
use crate::configuration::{get_ua, Configuration};
use crate::packages::robotparser::parser::RobotFileParser;
use crate::page::{build, get_page_selectors, Page};
use crate::utils::log;
use crate::CaseInsensitiveString;
use compact_str::CompactString;

#[cfg(feature = "budget")]
use hashbrown::HashMap;

use hashbrown::HashSet;
use reqwest::Client;
use std::io::{Error, ErrorKind};
use std::sync::atomic::{AtomicI8, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::runtime::Handle;
use tokio::sync::{broadcast, Semaphore};
use tokio::task;
use tokio::task::JoinSet;
use tokio_stream::StreamExt;
use url::Url;

#[cfg(feature = "chrome")]
use crate::features::chrome::launch_browser;

#[cfg(not(feature = "decentralized"))]
lazy_static! {
    static ref SEM: Semaphore = {
        let logical = num_cpus::get();
        let physical = num_cpus::get_physical();

        let sem_limit = if logical > physical {
            (logical) / (physical) as usize
        } else {
            logical
        };

        let (sem_limit, sem_max) = if logical == physical {
            (sem_limit * physical, 50)
        } else {
            (sem_limit * 4, 25)
        };
        let sem_limit = if cfg!(feature = "chrome") {
            sem_limit / 2
        } else {
            sem_limit
        };
        Semaphore::const_new(sem_limit.max(sem_max))
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
        let logical = num_cpus::get();
        let physical = num_cpus::get_physical();

        let sem_limit = if logical > physical {
            (logical) / (physical) as usize
        } else {
            logical
        };

        let (sem_limit, sem_max) = if logical == physical {
            (sem_limit * physical, 75)
        } else {
            (sem_limit * 4, 33)
        };
        let (sem_limit, sem_max) = { (sem_limit * WORKERS.len(), sem_max * WORKERS.len()) };

        Semaphore::const_new(sem_limit.max(sem_max))
    };
}

/// the active status of the crawl.
#[derive(Debug, Clone, Default)]
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
    #[cfg(feature = "control")]
    /// The crawl shutdown manually.
    Shutdown,
    #[cfg(feature = "control")]
    /// The crawl paused manually.
    Paused,
}

/// Represents a website to crawl and gather all links.
/// ```rust
/// use spider::website::Website;
/// let mut website = Website::new("http://example.com");
/// website.crawl();
/// // `Website` will be filled with `Pages` when crawled. To get them, just use
/// while let Some(page) = website.get_pages() {
///     // do something
/// }
/// ```
#[derive(Debug, Clone, Default)]
pub struct Website {
    /// configuration properties for website.
    pub configuration: Box<Configuration>,
    /// contains all visited URL.
    links_visited: Box<HashSet<CaseInsensitiveString>>,
    /// contains page visited
    pages: Option<Box<Vec<Page>>>,
    /// Robot.txt parser holder.
    robot_file_parser: Option<Box<RobotFileParser>>,
    /// the base root domain of the crawl
    domain: Box<CaseInsensitiveString>,
    /// the domain url parsed
    domain_parsed: Option<Box<Url>>,
    /// callback when a link is found.
    pub on_link_find_callback: Option<
        fn(CaseInsensitiveString, Option<String>) -> (CaseInsensitiveString, Option<String>),
    >,
    /// subscribe and broadcast changes
    channel: Option<Arc<(broadcast::Sender<Page>, broadcast::Receiver<Page>)>>,
    /// the status of the active crawl
    status: CrawlStatus,
    /// external domains to include
    pub external_domains: Box<HashSet<String>>,
    /// external domains to include case-insensitive
    external_domains_caseless: Box<HashSet<CaseInsensitiveString>>,
    #[cfg(feature = "budget")]
    /// A crawl budget for the paths
    pub budget: Option<HashMap<CaseInsensitiveString, u32>>,
}

impl Website {
    /// Initialize Website object with a start link to crawl.
    pub fn new(domain: &str) -> Self {
        Self {
            configuration: Configuration::new().into(),
            links_visited: Box::new(HashSet::new()),
            pages: None,
            robot_file_parser: None,
            domain: CaseInsensitiveString::new(domain).into(),
            domain_parsed: match url::Url::parse(domain) {
                Ok(u) => Some(Box::new(crate::page::convert_abs_path(&u, "/"))),
                _ => None,
            },
            on_link_find_callback: None,
            channel: None,
            status: CrawlStatus::Start,
            ..Default::default()
        }
    }

    /// return `true` if URL:
    ///
    /// - is not already crawled
    /// - is not blacklisted
    /// - is not forbidden in robot.txt file (if parameter is defined)
    #[inline]
    #[cfg(all(not(feature = "regex"), not(feature = "budget")))]
    pub fn is_allowed(
        &self,
        link: &CaseInsensitiveString,
        blacklist_url: &Box<Vec<CompactString>>,
    ) -> bool {
        if self.links_visited.contains(link) {
            false
        } else {
            self.is_allowed_default(&link.inner(), blacklist_url)
        }
    }

    /// return `true` if URL:
    ///
    /// - is not already crawled
    /// - is not over crawl budget
    /// - is not blacklisted
    /// - is not forbidden in robot.txt file (if parameter is defined)
    #[inline]
    #[cfg(all(not(feature = "regex"), feature = "budget"))]
    pub fn is_allowed(
        &mut self,
        link: &CaseInsensitiveString,
        blacklist_url: &Box<Vec<CompactString>>,
    ) -> bool {
        if self.links_visited.contains(link) {
            false
        } else if self.is_over_budget(&link) {
            false
        } else {
            self.is_allowed_default(&link.inner(), blacklist_url)
        }
    }

    /// return `true` if URL:
    ///
    /// - is not already crawled
    /// - is not blacklisted
    /// - is not forbidden in robot.txt file (if parameter is defined)
    #[inline]
    #[cfg(all(feature = "regex", not(feature = "budget")))]
    pub fn is_allowed(
        &self,
        link: &CaseInsensitiveString,
        blacklist_url: &Box<regex::RegexSet>,
    ) -> bool {
        if self.links_visited.contains(link) {
            false
        } else {
            self.is_allowed_default(link, blacklist_url)
        }
    }

    /// return `true` if URL:
    ///
    /// - is not already crawled
    /// - is not over crawl budget
    /// - is not blacklisted
    /// - is not forbidden in robot.txt file (if parameter is defined)
    #[inline]
    #[cfg(all(feature = "regex", feature = "budget"))]
    pub fn is_allowed(
        &mut self,
        link: &CaseInsensitiveString,
        blacklist_url: &Box<regex::RegexSet>,
    ) -> bool {
        if self.links_visited.contains(link) {
            false
        } else if self.is_over_budget(&link) {
            false
        } else {
            self.is_allowed_default(link, blacklist_url)
        }
    }

    /// return `true` if URL:
    ///
    /// - is not blacklisted
    /// - is not forbidden in robot.txt file (if parameter is defined)
    #[inline]
    #[cfg(feature = "regex")]
    pub fn is_allowed_default(
        &self,
        link: &CaseInsensitiveString,
        blacklist_url: &Box<regex::RegexSet>,
    ) -> bool {
        if !blacklist_url.is_empty() {
            !contains(blacklist_url, &link.inner())
        } else {
            self.is_allowed_robots(&link.as_ref())
        }
    }

    /// return `true` if URL:
    ///
    /// - is not blacklisted
    /// - is not forbidden in robot.txt file (if parameter is defined)
    #[inline]
    #[cfg(not(feature = "regex"))]
    pub fn is_allowed_default(
        &self,
        link: &CompactString,
        blacklist_url: &Box<Vec<CompactString>>,
    ) -> bool {
        if contains(blacklist_url, &link) {
            false
        } else {
            self.is_allowed_robots(&link)
        }
    }

    /// return `true` if URL:
    ///
    /// - is not forbidden in robot.txt file (if parameter is defined)
    pub fn is_allowed_robots(&self, link: &str) -> bool {
        if self.configuration.respect_robots_txt {
            unsafe {
                self.robot_file_parser
                    .as_ref()
                    .unwrap_unchecked()
                    .can_fetch("*", &link)
            } // unwrap will always return
        } else {
            true
        }
    }

    #[cfg(feature = "budget")]
    /// Validate if url exceeds crawl budget and should not be handled.
    pub fn is_over_budget(&mut self, link: &CaseInsensitiveString) -> bool {
        if self.budget.is_some() {
            match Url::parse(&link.inner()) {
                Ok(r) => {
                    match self.budget.as_mut() {
                        Some(budget) => {
                            let wild = CaseInsensitiveString::from("*");
                            let has_wildpath = budget.contains_key(&wild);

                            let exceeded_wild_budget = if has_wildpath {
                                let budget = budget.get_mut(&wild).unwrap();
                                if budget.abs_diff(0) == 1 {
                                    true
                                } else {
                                    *budget -= 1;
                                    false
                                }
                            } else {
                                false
                            };

                            // set this up prior to crawl to avoid checks per link
                            let skip_paths = has_wildpath && budget.len() == 1;

                            // check if paths pass
                            if !skip_paths && !exceeded_wild_budget {
                                match r.path_segments() {
                                    Some(mut segments) => {
                                        let mut joint_segment = String::new();
                                        let mut over = false;

                                        while let Some(seg) = segments.next() {
                                            let next_segment = string_concat!(joint_segment, seg);
                                            let caseless_segment =
                                                CaseInsensitiveString::from(next_segment);

                                            if budget.contains_key(&caseless_segment) {
                                                let budget =
                                                    budget.get_mut(&caseless_segment).unwrap();

                                                if budget.abs_diff(0) == 0 {
                                                    over = true;
                                                    break;
                                                } else {
                                                    *budget -= 1;
                                                    continue;
                                                }
                                            }

                                            joint_segment = joint_segment;
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
                _ => false,
            }
        } else {
            false
        }
    }

    /// page getter
    pub fn get_pages(&self) -> Option<&Box<Vec<Page>>> {
        self.pages.as_ref()
    }

    /// links visited getter
    pub fn get_links(&self) -> &HashSet<CaseInsensitiveString> {
        &self.links_visited
    }

    /// domain parsed url getter
    pub fn get_domain_parsed(&self) -> &Option<Box<Url>> {
        &self.domain_parsed
    }

    /// domain name getter
    pub fn get_domain(&self) -> &CaseInsensitiveString {
        &self.domain
    }

    /// crawl delay getter
    fn get_delay(&self) -> Duration {
        Duration::from_millis(self.configuration.delay)
    }

    /// get the active crawl status
    pub fn get_status(&self) -> &CrawlStatus {
        &self.status
    }

    /// absolute base url of crawl
    pub fn get_absolute_path(&self, domain: Option<&str>) -> Option<Url> {
        if domain.is_some() {
            match url::Url::parse(domain.unwrap()) {
                Ok(u) => Some(crate::page::convert_abs_path(&u, "/")),
                _ => None,
            }
        } else {
            self.domain_parsed.as_deref().cloned()
        }
    }

    /// configure the robots parser on initial crawl attempt and run.
    pub async fn configure_robots_parser(&mut self, client: Client) -> Client {
        if self.configuration.respect_robots_txt {
            let robot_file_parser = self
                .robot_file_parser
                .get_or_insert_with(|| RobotFileParser::new());

            if robot_file_parser.mtime() <= 4000 {
                let host_str = match &self.domain_parsed {
                    Some(domain) => &*domain.as_str(),
                    _ => &self.domain.inner(),
                };
                if host_str.ends_with("/") {
                    robot_file_parser.read(&client, &host_str).await;
                } else {
                    robot_file_parser
                        .read(&client, &string_concat!(host_str, "/"))
                        .await;
                }
                self.configuration.delay = robot_file_parser
                    .get_crawl_delay(&self.configuration.user_agent) // returns the crawl delay in seconds
                    .unwrap_or_else(|| self.get_delay())
                    .as_millis() as u64;
            }
        }

        client
    }

    /// configure http client
    #[cfg(not(feature = "decentralized"))]
    pub fn configure_http_client(&mut self) -> Client {
        let host_str = self.domain_parsed.take();
        let default_policy = reqwest::redirect::Policy::default();
        let policy = match host_str {
            Some(host_s) => reqwest::redirect::Policy::custom(move |attempt| {
                if attempt.url().host_str() != host_s.host_str() {
                    attempt.stop()
                } else {
                    default_policy.redirect(attempt)
                }
            }),
            _ => default_policy,
        };

        let client = Client::builder()
            .user_agent(match &self.configuration.user_agent {
                Some(ua) => ua.as_str(),
                _ => &get_ua(),
            })
            .redirect(policy)
            .tcp_keepalive(Duration::from_millis(500))
            .pool_idle_timeout(None);

        let client = if self.configuration.http2_prior_knowledge {
            client.http2_prior_knowledge()
        } else {
            client
        };

        let client = match &self.configuration.headers {
            Some(headers) => client.default_headers(*headers.to_owned()),
            _ => client,
        };

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

        // should unwrap using native-tls-alpn
        unsafe { client.build().unwrap_unchecked() }
    }

    /// configure http client for decentralization
    #[cfg(feature = "decentralized")]
    pub fn configure_http_client(&mut self) -> Client {
        use reqwest::header::HeaderMap;
        use reqwest::header::HeaderValue;

        let mut headers = HeaderMap::new();

        let host_str = self.domain_parsed.take();
        let default_policy = reqwest::redirect::Policy::default();
        let policy = match host_str {
            Some(host_s) => reqwest::redirect::Policy::custom(move |attempt| {
                if attempt.url().host_str() != host_s.host_str() {
                    attempt.stop()
                } else {
                    default_policy.redirect(attempt)
                }
            }),
            _ => default_policy,
        };

        let mut client = Client::builder()
            .user_agent(match &self.configuration.user_agent {
                Some(ua) => ua.as_str(),
                _ => &get_ua(),
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

    /// setup atomic controller
    #[cfg(feature = "control")]
    fn configure_handler(&self) -> Arc<AtomicI8> {
        use crate::utils::{Handler, CONTROLLER};

        let paused = Arc::new(AtomicI8::new(0));
        let handle = paused.clone();
        let domain = self.domain.inner().clone();

        tokio::spawn(async move {
            let mut l = CONTROLLER.lock().await.1.to_owned();

            while l.changed().await.is_ok() {
                let n = &*l.borrow();
                let (target, rest) = n;

                if domain.eq_ignore_ascii_case(&target) {
                    if rest == &Handler::Resume {
                        paused.store(0, Ordering::Relaxed);
                    }
                    if rest == &Handler::Pause {
                        paused.store(1, Ordering::Relaxed);
                    }
                    if rest == &Handler::Shutdown {
                        paused.store(2, Ordering::Relaxed);
                    }
                }
            }
        });

        handle
    }

    /// setup config for crawl
    #[cfg(feature = "control")]
    pub async fn setup(&mut self) -> (Client, Option<Arc<AtomicI8>>) {
        let client = self.configure_http_client();

        // allow fresh crawls to run fully
        if !self.links_visited.is_empty() {
            self.links_visited.clear();
        }

        (
            self.configure_robots_parser(client).await,
            Some(self.configure_handler()),
        )
    }

    /// setup config for crawl
    #[cfg(not(feature = "control"))]
    pub async fn setup<T>(&mut self) -> (Client, Option<T>) {
        let client = self.configure_http_client();

        // allow fresh crawls to run fully
        if !self.links_visited.is_empty() {
            self.links_visited.clear();
        }

        (self.configure_robots_parser(client).await, None)
    }

    /// setup selectors for handling link targets
    #[cfg(not(feature = "decentralized"))]
    fn setup_selectors(&self) -> Option<(CompactString, smallvec::SmallVec<[CompactString; 2]>)> {
        get_page_selectors(
            &self.domain.inner(),
            self.configuration.subdomains,
            self.configuration.tld,
        )
    }

    /// setup shared concurrent configs
    #[cfg(not(feature = "decentralized"))]
    fn setup_crawl(
        &mut self,
    ) -> (
        Box<Vec<CompactString>>,
        std::pin::Pin<Box<tokio::time::Interval>>,
        std::pin::Pin<Box<Duration>>,
    ) {
        self.status = CrawlStatus::Active;
        let blacklist_url = self.configuration.get_blacklist();
        let interval = Box::pin(tokio::time::interval(Duration::from_millis(10)));
        let throttle = Box::pin(self.get_delay());

        (blacklist_url, interval, throttle)
    }

    /// get base link for crawl establishing
    #[cfg(all(not(feature = "glob"), feature = "regex"))]
    fn get_base_link(&self) -> &CaseInsensitiveString {
        &self.domain
    }

    /// get base link for crawl establishing
    #[cfg(all(not(feature = "glob"), not(feature = "regex")))]
    fn get_base_link(&self) -> &CompactString {
        self.domain.inner()
    }

    /// expand links for crawl
    #[cfg(all(
        not(feature = "glob"),
        not(feature = "decentralized"),
        not(feature = "chrome")
    ))]
    async fn crawl_establish(
        &mut self,
        client: &Client,
        base: &(CompactString, smallvec::SmallVec<[CompactString; 2]>),
        _: bool,
    ) -> HashSet<CaseInsensitiveString> {
        let links: HashSet<CaseInsensitiveString> = if self
            .is_allowed_default(&self.get_base_link(), &self.configuration.get_blacklist())
        {
            let page = Page::new(&self.domain.inner(), &client).await;

            if !self.external_domains.is_empty() {
                self.external_domains_caseless = self
                    .external_domains
                    .iter()
                    .filter_map(|d| match Url::parse(d) {
                        Ok(d) => Some(d.host_str().unwrap_or_default().into()),
                        _ => None,
                    })
                    .collect::<HashSet<CaseInsensitiveString>>()
                    .into();
            }

            let links = if !page.is_empty() {
                self.links_visited.insert(match self.on_link_find_callback {
                    Some(cb) => {
                        let c = cb(*self.domain.clone(), None);

                        c.0
                    }
                    _ => *self.domain.clone(),
                });

                let links = HashSet::from(page.links(&base).await);

                links
            } else {
                self.status = CrawlStatus::Empty;
                Default::default()
            };

            match &self.channel {
                Some(c) => {
                    match c.0.send(page) {
                        _ => (),
                    };
                }
                _ => (),
            };

            links
        } else {
            HashSet::new()
        };

        links
    }

    /// expand links for crawl
    #[cfg(all(
        not(feature = "glob"),
        not(feature = "decentralized"),
        feature = "chrome"
    ))]
    async fn crawl_establish(
        &mut self,
        client: &Client,
        base: &(CompactString, smallvec::SmallVec<[CompactString; 2]>),
        _: bool,
        page: &chromiumoxide_fork::Page,
    ) -> HashSet<CaseInsensitiveString> {
        let links: HashSet<CaseInsensitiveString> = if self
            .is_allowed_default(&self.get_base_link(), &self.configuration.get_blacklist())
        {
            let page = Page::new(&self.domain.inner(), &client, &page).await;

            if !self.external_domains.is_empty() {
                self.external_domains_caseless = self
                    .external_domains
                    .iter()
                    .filter_map(|d| match Url::parse(d) {
                        Ok(d) => Some(d.host_str().unwrap_or_default().into()),
                        _ => None,
                    })
                    .collect::<HashSet<CaseInsensitiveString>>()
                    .into();
            }

            let links = if !page.is_empty() {
                self.links_visited.insert(match self.on_link_find_callback {
                    Some(cb) => {
                        let c = cb(*self.domain.clone(), None);

                        c.0
                    }
                    _ => *self.domain.clone(),
                });

                let links = HashSet::from(page.links(&base).await);

                links
            } else {
                self.status = CrawlStatus::Empty;
                Default::default()
            };

            match &self.channel {
                Some(c) => {
                    match c.0.send(page) {
                        _ => (),
                    };
                }
                _ => (),
            };

            links
        } else {
            HashSet::new()
        };

        links
    }

    /// expand links for crawl
    #[cfg(all(not(feature = "glob"), feature = "decentralized"))]
    async fn crawl_establish(
        &mut self,
        client: &Client,
        _: &(CompactString, smallvec::SmallVec<[CompactString; 2]>),
        http_worker: bool,
    ) -> HashSet<CaseInsensitiveString> {
        // base_domain name passed here is for primary url determination and not subdomain.tld placement
        let links: HashSet<CaseInsensitiveString> = if self
            .is_allowed_default(&self.get_base_link(), &self.configuration.get_blacklist())
        {
            let link = self.domain.inner();

            let page = Page::new(
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
                    let c = cb(*self.domain.to_owned(), None);

                    c.0
                }
                _ => *self.domain.to_owned(),
            });

            match &self.channel {
                Some(c) => {
                    match c.0.send(page.clone()) {
                        _ => (),
                    };
                }
                _ => (),
            };

            let page_links = HashSet::from(page.links);

            page_links
        } else {
            HashSet::new()
        };

        links
    }

    /// expand links for crawl
    #[cfg(all(feature = "glob", feature = "decentralized"))]
    async fn crawl_establish(
        &mut self,
        client: &Client,
        _: &(CompactString, smallvec::SmallVec<[CompactString; 2]>),
        http_worker: bool,
    ) -> HashSet<CaseInsensitiveString> {
        let mut links: HashSet<CaseInsensitiveString> = HashSet::new();
        let domain_name = self.domain.inner();
        let mut expanded = crate::features::glob::expand_url(&domain_name.as_str());

        if expanded.len() == 0 {
            match self.get_absolute_path(Some(domain_name)) {
                Some(u) => {
                    expanded.push(u.as_str().into());
                }
                _ => (),
            };
        };

        let blacklist_url = self.configuration.get_blacklist();

        for link in expanded {
            if self.is_allowed_default(&link, &blacklist_url) {
                let page = Page::new(
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
                match &self.channel {
                    Some(c) => {
                        match c.0.send(page.clone()) {
                            _ => (),
                        };
                    }
                    _ => (),
                };

                let page_links = HashSet::from(page.links);

                links.extend(page_links);
            }
        }

        links
    }

    /// expand links for crawl
    #[cfg(all(feature = "glob", not(feature = "decentralized")))]
    async fn crawl_establish(
        &mut self,
        client: &Client,
        base: &(CompactString, smallvec::SmallVec<[CompactString; 2]>),
        _: bool,
    ) -> HashSet<CaseInsensitiveString> {
        let mut links: HashSet<CaseInsensitiveString> = HashSet::new();
        let domain_name = self.domain.inner();
        let mut expanded = crate::features::glob::expand_url(&domain_name.as_str());

        if expanded.len() == 0 {
            match self.get_absolute_path(Some(domain_name)) {
                Some(u) => {
                    expanded.push(u.as_str().into());
                }
                _ => (),
            };
        };

        let blacklist_url = self.configuration.get_blacklist();

        for link in expanded {
            if self.is_allowed_default(&link.inner(), &blacklist_url) {
                let page = Page::new(&link.inner(), &client).await;

                if !page.is_empty() {
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

                match &self.channel {
                    Some(c) => {
                        match c.0.send(page) {
                            _ => (),
                        };
                    }
                    _ => (),
                };
            }
        }

        links
    }

    #[cfg(not(feature = "sitemap"))]
    /// Start to crawl website with async concurrency
    pub async fn crawl(&mut self) {
        let (client, handle) = self.setup().await;

        self.crawl_concurrent(&client, &handle).await;
    }

    #[cfg(not(feature = "sitemap"))]
    /// Start to scrape/download website with async concurrency
    pub async fn scrape(&mut self) {
        let (client, handle) = self.setup().await;

        self.scrape_concurrent(&client, &handle).await;
    }

    #[cfg(feature = "sitemap")]
    /// Start to crawl website and include sitemap links
    pub async fn crawl(&mut self) {
        let (client, handle) = self.setup().await;

        self.crawl_concurrent(&client, &handle).await;
        self.sitemap_crawl(&client, &handle, false).await;
    }

    #[cfg(feature = "sitemap")]
    /// Start to scrape/download website with async concurrency
    pub async fn scrape(&mut self) {
        let (client, handle) = self.setup().await;

        self.scrape_concurrent(&client, &handle).await;
        self.sitemap_crawl(&client, &handle, true).await;
    }

    /// Start to crawl website concurrently
    #[cfg(all(not(feature = "decentralized"), feature = "chrome"))]
    async fn crawl_concurrent(&mut self, client: &Client, handle: &Option<Arc<AtomicI8>>) {
        let selectors = self.setup_selectors();

        // crawl if valid selector
        if selectors.is_some() {
            let (blacklist_url, mut interval, throttle) = self.setup_crawl();
            let on_link_find_callback = self.on_link_find_callback;
            let (mut browser, browser_handle) = launch_browser(&self.configuration.proxies).await;

            let new_page = browser.new_page("about:blank").await.unwrap();

            if cfg!(feature = "chrome_stealth") {
                let _ = new_page.enable_stealth_mode(&if self.configuration.user_agent.is_some() {
                    &self.configuration.user_agent.as_ref().unwrap().as_str()
                } else {
                    ""
                });
            }

            let shared = Arc::new((
                client.to_owned(),
                unsafe { selectors.unwrap_unchecked() },
                self.channel.clone(),
                new_page.clone(),
                self.external_domains_caseless.clone(),
            ));

            let mut links: HashSet<CaseInsensitiveString> = self
                .crawl_establish(&shared.0, &shared.1, false, &shared.3)
                .await;

            if !links.is_empty() {
                let mut set: JoinSet<HashSet<CaseInsensitiveString>> = JoinSet::new();
                let chandle = Handle::current();

                // crawl while links exists
                loop {
                    let stream = tokio_stream::iter::<HashSet<CaseInsensitiveString>>(
                        links.drain().collect(),
                    )
                    .throttle(*throttle);
                    tokio::pin!(stream);

                    loop {
                        match stream.next().await {
                            Some(link) => {
                                match handle.as_ref() {
                                    Some(handle) => {
                                        while handle.load(Ordering::Relaxed) == 1 {
                                            interval.tick().await;
                                        }
                                        if handle.load(Ordering::Relaxed) == 2 {
                                            set.shutdown().await;
                                            break;
                                        }
                                    }
                                    None => (),
                                }

                                if !self.is_allowed(&link, &blacklist_url) {
                                    continue;
                                }

                                log("fetch", &link);
                                self.links_visited.insert(link.clone());
                                let permit = SEM.acquire().await.unwrap();
                                let shared = shared.clone();
                                task::yield_now().await;

                                set.spawn_on(
                                    async move {
                                        let link_result = match on_link_find_callback {
                                            Some(cb) => cb(link, None),
                                            _ => (link, None),
                                        };
                                        let mut page = Page::new(
                                            &link_result.0.as_ref(),
                                            &shared.0,
                                            &shared.3,
                                        )
                                        .await;

                                        page.set_external(shared.4.clone());

                                        let page_links = page.links(&shared.1).await;

                                        match &shared.2 {
                                            Some(c) => {
                                                match c.0.send(page) {
                                                    _ => (),
                                                };
                                            }
                                            _ => (),
                                        };

                                        drop(permit);

                                        page_links
                                    },
                                    &chandle,
                                );
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
            }

            self.status = CrawlStatus::Idle;
            if !std::env::var("CHROME_URL").is_ok() {
                let _ = browser.close().await;
                let _ = browser_handle.await;
            } else {
                let _ = new_page.close().await;
            }
        }
    }

    /// Start to crawl website concurrently
    #[cfg(all(not(feature = "decentralized"), not(feature = "chrome")))]
    async fn crawl_concurrent(&mut self, client: &Client, handle: &Option<Arc<AtomicI8>>) {
        let selectors = self.setup_selectors();

        // crawl if valid selector
        if selectors.is_some() {
            let (blacklist_url, mut interval, throttle) = self.setup_crawl();
            let on_link_find_callback = self.on_link_find_callback;

            let shared = Arc::new((
                client.to_owned(),
                unsafe { selectors.unwrap_unchecked() },
                self.channel.clone(),
                self.external_domains_caseless.clone(),
            ));

            let mut links: HashSet<CaseInsensitiveString> =
                self.crawl_establish(&shared.0, &shared.1, false).await;

            if !links.is_empty() {
                let mut set: JoinSet<HashSet<CaseInsensitiveString>> = JoinSet::new();
                let chandle = Handle::current();

                // crawl while links exists
                loop {
                    let stream = tokio_stream::iter::<HashSet<CaseInsensitiveString>>(
                        links.drain().collect(),
                    )
                    .throttle(*throttle);
                    tokio::pin!(stream);

                    loop {
                        match stream.next().await {
                            Some(link) => {
                                match handle.as_ref() {
                                    Some(handle) => {
                                        while handle.load(Ordering::Relaxed) == 1 {
                                            interval.tick().await;
                                        }
                                        if handle.load(Ordering::Relaxed) == 2 {
                                            set.shutdown().await;
                                            break;
                                        }
                                    }
                                    None => (),
                                }

                                if !self.is_allowed(&link, &blacklist_url) {
                                    continue;
                                }

                                log("fetch", &link);
                                self.links_visited.insert(link.clone());
                                let permit = SEM.acquire().await.unwrap();
                                let shared = shared.clone();
                                task::yield_now().await;

                                set.spawn_on(
                                    async move {
                                        let link_result = match on_link_find_callback {
                                            Some(cb) => cb(link, None),
                                            _ => (link, None),
                                        };
                                        let mut page =
                                            Page::new(&link_result.0.as_ref(), &shared.0).await;
                                        page.set_external(shared.3.to_owned());

                                        let page_links = page.links(&shared.1).await;

                                        match &shared.2 {
                                            Some(c) => {
                                                match c.0.send(page) {
                                                    _ => (),
                                                };
                                            }
                                            _ => (),
                                        };

                                        drop(permit);

                                        page_links
                                    },
                                    &chandle,
                                );
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

            self.status = CrawlStatus::Idle;
        }
    }

    /// Start to crawl website concurrently
    #[cfg(feature = "decentralized")]
    async fn crawl_concurrent(&mut self, client: &Client, handle: &Option<Arc<AtomicI8>>) {
        match url::Url::parse(&self.domain.inner()) {
            Ok(_) => {
                let blacklist_url = self.configuration.get_blacklist();
                let domain = self.domain.inner().as_str();
                let mut interval = Box::pin(tokio::time::interval(Duration::from_millis(10)));
                let throttle = Box::pin(self.get_delay());
                let on_link_find_callback = self.on_link_find_callback;
                // http worker verify
                let http_worker = std::env::var("SPIDER_WORKER")
                    .unwrap_or_else(|_| "http:".to_string())
                    .starts_with("http:");

                let mut links: HashSet<CaseInsensitiveString> = self
                    .crawl_establish(&client, &(domain.into(), Default::default()), http_worker)
                    .await;

                let mut set: JoinSet<HashSet<CaseInsensitiveString>> = JoinSet::new();
                let chandle = Handle::current();

                // crawl while links exists
                loop {
                    let stream = tokio_stream::iter::<HashSet<CaseInsensitiveString>>(
                        links.drain().collect(),
                    )
                    .throttle(*throttle);
                    tokio::pin!(stream);

                    loop {
                        match stream.next().await {
                            Some(link) => {
                                match handle.as_ref() {
                                    Some(handle) => {
                                        while handle.load(Ordering::Relaxed) == 1 {
                                            interval.tick().await;
                                        }
                                        if handle.load(Ordering::Relaxed) == 2 {
                                            set.shutdown().await;
                                            break;
                                        }
                                    }
                                    None => (),
                                }

                                if !self.is_allowed(&link, &blacklist_url) {
                                    continue;
                                }

                                log("fetch", &link);

                                self.links_visited.insert(link.clone());
                                let permit = SEM.acquire().await.unwrap();
                                let client = client.clone();
                                task::yield_now().await;

                                set.spawn_on(
                                    async move {
                                        let link_results = match on_link_find_callback {
                                            Some(cb) => cb(link, None),
                                            _ => (link, None),
                                        };
                                        let link_results = link_results.0.as_ref();
                                        let page = Page::new(
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
                                    },
                                    &chandle,
                                );
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

    #[cfg(not(feature = "chrome"))]
    /// Start to scape website concurrently and store html
    async fn scrape_concurrent(&mut self, client: &Client, handle: &Option<Arc<AtomicI8>>) {
        let selectors = get_page_selectors(
            &self.domain.inner(),
            self.configuration.subdomains,
            self.configuration.tld,
        );

        if selectors.is_some() {
            self.status = CrawlStatus::Active;
            let blacklist_url = self.configuration.get_blacklist();
            self.pages = Some(Box::new(Vec::new()));
            let delay = self.configuration.delay;
            let on_link_find_callback = self.on_link_find_callback;
            let mut interval = tokio::time::interval(Duration::from_millis(10));
            let selectors = Arc::new(unsafe { selectors.unwrap_unchecked() });
            let throttle = Duration::from_millis(delay);

            let mut links: HashSet<CaseInsensitiveString> = HashSet::from([*self.domain.clone()]);
            let mut set: JoinSet<(CaseInsensitiveString, Page, HashSet<CaseInsensitiveString>)> =
                JoinSet::new();

            // crawl while links exists
            loop {
                let stream =
                    tokio_stream::iter::<HashSet<CaseInsensitiveString>>(links.drain().collect())
                        .throttle(throttle);
                tokio::pin!(stream);

                while let Some(link) = stream.next().await {
                    match handle.as_ref() {
                        Some(handle) => {
                            while handle.load(Ordering::Relaxed) == 1 {
                                interval.tick().await;
                            }
                            if handle.load(Ordering::Relaxed) == 2 {
                                set.shutdown().await;
                                break;
                            }
                        }
                        None => (),
                    }
                    if !self.is_allowed(&link, &blacklist_url) {
                        continue;
                    }
                    self.links_visited.insert(link.clone());
                    log("fetch", &link);
                    let permit = SEM.acquire().await.unwrap();
                    // these clones should move into a single arc
                    let client = client.clone();
                    let channel = self.channel.clone();
                    let selectors = selectors.clone();
                    let external_domains_caseless = self.external_domains_caseless.clone();

                    set.spawn(async move {
                        drop(permit);
                        let page_resource =
                            crate::utils::fetch_page_html(&link.as_ref(), &client).await;
                        let mut page = build(&link.as_ref(), page_resource.0);
                        page.set_final_redirect(page_resource.1);

                        let (link, _) = match on_link_find_callback {
                            Some(cb) => {
                                let c = cb(link, Some(page.get_html()));

                                c
                            }
                            _ => (link, None),
                        };

                        match &channel {
                            Some(c) => {
                                match c.0.send(page.clone()) {
                                    _ => (),
                                };
                            }
                            _ => (),
                        };

                        page.set_external(external_domains_caseless);

                        let page_links = page.links(&*selectors).await;

                        (link, page, page_links)
                    });
                }

                task::yield_now().await;

                if links.capacity() >= 1500 {
                    links.shrink_to_fit();
                }

                while let Some(res) = set.join_next().await {
                    match res {
                        Ok(msg) => {
                            let page = msg.1;
                            links.extend(&msg.2 - &self.links_visited);
                            task::yield_now().await;
                            match self.pages.as_mut() {
                                Some(p) => p.push(page.clone()),
                                _ => (),
                            };
                        }
                        _ => (),
                    };
                }

                task::yield_now().await;
                if links.is_empty() {
                    break;
                }
            }

            self.status = CrawlStatus::Idle;
        }
    }

    #[cfg(feature = "chrome")]
    /// Start to scape website concurrently and store html
    async fn scrape_concurrent(&mut self, client: &Client, handle: &Option<Arc<AtomicI8>>) {
        let selectors = get_page_selectors(
            &self.domain.inner(),
            self.configuration.subdomains,
            self.configuration.tld,
        );

        if selectors.is_some() {
            self.status = CrawlStatus::Active;
            let blacklist_url = self.configuration.get_blacklist();
            self.pages = Some(Box::new(Vec::new()));
            let delay = self.configuration.delay;
            let on_link_find_callback = self.on_link_find_callback;
            let mut interval = tokio::time::interval(Duration::from_millis(10));
            let selectors = Arc::new(unsafe { selectors.unwrap_unchecked() });
            let throttle = Duration::from_millis(delay);

            let mut links: HashSet<CaseInsensitiveString> = HashSet::from([*self.domain.clone()]);
            let mut set: JoinSet<(CaseInsensitiveString, Page, HashSet<CaseInsensitiveString>)> =
                JoinSet::new();
            let (mut browser, _) = launch_browser(&self.configuration.proxies).await;
            let new_page = browser.new_page("about:blank").await.unwrap();
            if cfg!(feature = "chrome_stealth") {
                let _ = new_page.enable_stealth_mode(&if self.configuration.user_agent.is_some() {
                    &self.configuration.user_agent.as_ref().unwrap().as_str()
                } else {
                    ""
                });
            }
            let page = Arc::new(new_page.clone());
            // crawl while links exists
            loop {
                let stream =
                    tokio_stream::iter::<HashSet<CaseInsensitiveString>>(links.drain().collect())
                        .throttle(throttle);
                tokio::pin!(stream);

                while let Some(link) = stream.next().await {
                    match handle.as_ref() {
                        Some(handle) => {
                            while handle.load(Ordering::Relaxed) == 1 {
                                interval.tick().await;
                            }
                            if handle.load(Ordering::Relaxed) == 2 {
                                set.shutdown().await;
                                break;
                            }
                        }
                        None => (),
                    }
                    if !self.is_allowed(&link, &blacklist_url) {
                        continue;
                    }
                    self.links_visited.insert(link.clone());
                    log("fetch", &link);
                    let client = client.clone();
                    let permit = SEM.acquire().await.unwrap();
                    let channel = self.channel.clone();
                    let selectors = selectors.clone();
                    let page = page.clone();
                    let external_domains_caseless = self.external_domains_caseless.clone();

                    set.spawn(async move {
                        drop(permit);
                        let page =
                            crate::utils::fetch_page_html_chrome(&link.as_ref(), &client, &page)
                                .await;
                        let mut page = build(&link.as_ref(), page);

                        let (link, _) = match on_link_find_callback {
                            Some(cb) => {
                                let c = cb(link, Some(page.get_html()));

                                c
                            }
                            _ => (link, None),
                        };

                        match &channel {
                            Some(c) => {
                                match c.0.send(page.clone()) {
                                    _ => (),
                                };
                            }
                            _ => (),
                        };

                        page.set_external(external_domains_caseless);
                        let page_links = page.links(&*selectors).await;

                        (link, page, page_links)
                    });
                }

                task::yield_now().await;

                if links.capacity() >= 1500 {
                    links.shrink_to_fit();
                }

                while let Some(res) = set.join_next().await {
                    match res {
                        Ok(msg) => {
                            let page = msg.1;
                            links.extend(&msg.2 - &self.links_visited);
                            task::yield_now().await;
                            match self.pages.as_mut() {
                                Some(p) => p.push(page.clone()),
                                _ => (),
                            };
                        }
                        _ => (),
                    };
                }

                task::yield_now().await;
                if links.is_empty() {
                    break;
                }
            }

            self.status = CrawlStatus::Idle;

            if !std::env::var("CHROME_URL").is_ok() {
                let _ = browser.close().await;
            } else {
                let _ = new_page.close().await;
            }
        }
    }

    /// Sitemap crawl entire lists. Note: this method does not re-crawl the links of the pages found on the sitemap.
    #[cfg(feature = "sitemap")]
    pub async fn sitemap_crawl(
        &mut self,
        client: &Client,
        handle: &Option<Arc<AtomicI8>>,
        scrape: bool,
    ) {
        use sitemap::reader::{SiteMapEntity, SiteMapReader};
        use sitemap::structs::Location;
        let domain = self.domain.inner().as_str();
        let handle = handle.clone().unwrap_or_default();

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
            string_concat!(domain, if needs_trailing { "/" } else { "" }, sitemap_path).into(),
        ));

        let blacklist_url = self.configuration.get_blacklist();

        while let Some(site) = &self.configuration.sitemap_url {
            if !handle.load(Ordering::Relaxed) == 2 {
                break;
            }

            let mut sitemap_added = false;
            let (tx, mut rx) = tokio::sync::mpsc::channel::<Page>(32);
            let client = client.clone();

            let channel = self.channel.clone();

            let handles = tokio::spawn(async move {
                let mut pages = Vec::new();

                while let Some(page) = rx.recv().await {
                    if scrape {
                        pages.push(page.clone());
                    };
                    match &channel {
                        Some(c) => {
                            match c.0.send(page) {
                                _ => (),
                            };
                        }
                        _ => (),
                    };
                }

                pages
            });

            match client.get(site.as_str()).send().await {
                Ok(response) => {
                    match response.text().await {
                        Ok(text) => {
                            // <html><head><title>Invalid request</title></head><body><p>Blocked by WAF</p><
                            let mut stream =
                                tokio_stream::iter(SiteMapReader::new(text.as_bytes()));

                            while let Some(entity) = stream.next().await {
                                while handle.load(Ordering::Relaxed) == 1 {
                                    interval.tick().await;
                                }
                                // shutdown all links
                                if handle.load(Ordering::Relaxed) == 2 {
                                    break;
                                }

                                match entity {
                                    SiteMapEntity::Url(url_entry) => match url_entry.loc {
                                        Location::Url(url) => {
                                            let link: CaseInsensitiveString = url.as_str().into();

                                            if !self.is_allowed(&link, &blacklist_url) {
                                                continue;
                                            }

                                            self.links_visited.insert(link.clone());

                                            let client = client.clone();
                                            let tx = tx.clone();

                                            tokio::spawn(async move {
                                                let page = Page::new(&link.inner(), &client).await;

                                                match tx.reserve().await {
                                                    Ok(permit) => {
                                                        permit.send(page);
                                                    }
                                                    _ => (),
                                                }
                                            });
                                        }
                                        Location::None | Location::ParseErr(_) => (),
                                    },
                                    SiteMapEntity::SiteMap(sitemap_entry) => {
                                        match sitemap_entry.loc {
                                            Location::Url(url) => {
                                                self.configuration
                                                    .sitemap_url
                                                    .replace(Box::new(url.as_str().into()));
                                                sitemap_added = true;
                                            }
                                            Location::None | Location::ParseErr(_) => (),
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

            if let Ok(handle) = handles.await {
                match self.pages.as_mut() {
                    Some(p) => p.extend(handle),
                    _ => (),
                };
            }

            if !sitemap_added {
                self.configuration.sitemap_url = None;
            };
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

    /// Add user agent to request.
    pub fn with_user_agent(&mut self, user_agent: Option<&str>) -> &mut Self {
        self.configuration.with_user_agent(user_agent);
        self
    }

    #[cfg(feature = "sitemap")]
    /// Add user agent to request.
    pub fn with_sitemap(&mut self, sitemap_url: Option<&str>) -> &mut Self {
        self.configuration.with_sitemap(sitemap_url);
        self
    }

    /// Use proxies for request.
    pub fn with_proxies(&mut self, proxies: Option<Vec<String>>) -> &mut Self {
        self.configuration.with_proxies(proxies);
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

    /// Set HTTP headers for request using [reqwest::header::HeaderMap](https://docs.rs/reqwest/latest/reqwest/header/struct.HeaderMap.html).
    pub fn with_headers(&mut self, headers: Option<reqwest::header::HeaderMap>) -> &mut Self {
        self.configuration.with_headers(headers);
        self
    }

    #[cfg(feature = "budget")]
    /// Set a crawl budget per path with levels support /a/b/c or for all paths with "*".
    pub fn with_budget(&mut self, budget: Option<HashMap<&str, u32>>) -> &mut Self {
        self.budget = match budget {
            Some(budget) => {
                let mut crawl_budget: HashMap<CaseInsensitiveString, u32> = HashMap::new();

                for b in budget.into_iter() {
                    crawl_budget.insert(CaseInsensitiveString::from(b.0), b.1);
                }

                Some(crawl_budget)
            }
            _ => None,
        };
        self
    }

    #[cfg(feature = "budget")]
    /// Set the crawl budget directly.
    pub fn set_crawl_budget(&mut self, budget: Option<HashMap<CaseInsensitiveString, u32>>) {
        self.budget = budget;
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
                    .filter_map(|d| match Url::parse(&d) {
                        Ok(d) => Some(d.host_str().unwrap_or_default().into()),
                        _ => None,
                    })
                    .collect::<HashSet<CaseInsensitiveString>>()
                    .into();
            }
            _ => self.external_domains_caseless.clear(),
        }

        self
    }

    /// Build the website configuration when using with_builder
    pub fn build(&self) -> Result<Self, Error> {
        if self.domain_parsed.is_none() {
            Err(ErrorKind::NotFound.into())
        } else {
            Ok(self.to_owned())
        }
    }

    /// Setup subscription for data.
    #[cfg(not(feature = "sync"))]
    pub fn subscribe(
        &mut self,
        capacity: usize,
    ) -> Option<Arc<(broadcast::Sender<Page>, broadcast::Receiver<Page>)>> {
        None
    }

    /// Setup subscription for data.
    #[cfg(feature = "sync")]
    pub fn subscribe(&mut self, capacity: usize) -> Option<broadcast::Receiver<Page>> {
        let channel = self
            .channel
            .get_or_insert(Arc::new(broadcast::channel(capacity.max(1))));
        let channel = channel.clone();

        let rx2 = channel.0.subscribe();

        Some(rx2)
    }
}

#[cfg(not(feature = "decentralized"))]
#[tokio::test]
async fn crawl() {
    let url = "https://choosealicense.com";
    let mut website: Website = Website::new(&url);
    website.crawl().await;
    assert!(
        website
            .links_visited
            .contains::<CaseInsensitiveString>(&"https://choosealicense.com/licenses/".into()),
        "{:?}",
        website.links_visited
    );
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

    assert_eq!(website.get_pages().unwrap()[0].get_html().is_empty(), false);
}

#[tokio::test]
#[cfg(not(feature = "decentralized"))]
async fn crawl_invalid() {
    let domain = "https://w.com";
    let mut website: Website = Website::new(domain);
    website.crawl().await;
    let mut uniq: Box<HashSet<CaseInsensitiveString>> = Box::new(HashSet::new());
    uniq.insert(format!("{}", domain.to_string()).into()); // TODO: remove trailing slash mutate

    assert_eq!(website.links_visited, uniq); // only the target url should exist
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
    assert_eq!(get_ua().is_empty(), false);
}

#[tokio::test]
#[cfg(not(feature = "decentralized"))]
async fn test_respect_robots_txt() {
    let mut website: Website = Website::new("https://stackoverflow.com");
    website.configuration.respect_robots_txt = true;
    website.configuration.user_agent = Some(Box::new("*".into()));

    let (client, _): (Client, Option<Arc<AtomicI8>>) = website.setup().await;

    website.configure_robots_parser(client).await;

    assert_eq!(website.configuration.delay, 0);

    assert!(!&website.is_allowed(
        &"https://stackoverflow.com/posts/".into(),
        &Default::default()
    ));

    // test match for bing bot
    let mut website_second: Website = Website::new("https://www.mongodb.com");
    website_second.configuration.respect_robots_txt = true;
    website_second.configuration.user_agent = Some(Box::new("bingbot".into()));

    let (client_second, _): (Client, Option<Arc<AtomicI8>>) = website_second.setup().await;
    website_second.configure_robots_parser(client_second).await;

    assert_eq!(website_second.configuration.delay, 60000); // should equal one minute in ms

    // test crawl delay with wildcard agent [DOES not work when using set agent]
    let mut website_third: Website = Website::new("https://www.mongodb.com");
    website_third.configuration.respect_robots_txt = true;
    let (client_third, _): (Client, Option<Arc<AtomicI8>>) = website_third.setup().await;

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
async fn test_with_configuration() {
    let mut website = Website::new("https://choosealicense.com");

    website
        .with_respect_robots_txt(true)
        .with_subdomains(true)
        .with_tld(false)
        .with_delay(0)
        .with_request_timeout(None)
        .with_http2_prior_knowledge(false)
        .with_user_agent(Some("myapp/version".into()))
        .with_headers(None)
        .with_proxies(None);

    website.crawl().await;

    assert!(
        !website
            .links_visited
            .contains::<CaseInsensitiveString>(&"https://choosealicense.com/licenses/".into()),
        "{:?}",
        website.links_visited
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
#[cfg(feature = "budget")]
async fn test_crawl_budget() {
    let mut website: Website = Website::new("https://choosealicense.com");
    website.with_budget(Some(HashMap::from([("*", 1), ("/licenses", 1)])));
    website.crawl().await;

    assert!(website.links_visited.len() <= 1);
}

#[cfg(feature = "control")]
#[tokio::test]
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

    assert!(duration.as_secs() > 5, "{:?}", duration);

    assert!(
        website
            .links_visited
            .contains::<CaseInsensitiveString>(&"https://choosealicense.com/licenses/".into()),
        "{:?}",
        website.links_visited
    );
}

#[cfg(feature = "control")]
#[tokio::test]
#[ignore]
async fn test_crawl_shutdown() {
    use crate::utils::shutdown;

    // use target blog to prevent shutdown of prior crawler
    let domain = "https://rsseau.fr/";
    let mut website: Website = Website::new(&domain);

    tokio::spawn(async move {
        shutdown(domain).await;
    });

    website.crawl().await;

    assert_eq!(website.links_visited.len(), 1);
}
