use crate::black_list::contains;
use crate::configuration::{get_ua, Configuration};
use crate::packages::robotparser::parser::RobotFileParser;
use crate::page::{build, get_page_selectors, Page};
use crate::utils::log;
use crate::CaseInsensitiveString;
use compact_str::CompactString;
use hashbrown::HashSet;
use reqwest::Client;
use std::sync::atomic::{AtomicI8, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::runtime::Handle;
use tokio::sync::Semaphore;
use tokio::task;
use tokio::task::JoinSet;
use tokio_stream::StreamExt;
use url::Url;

#[cfg(feature = "sync")]
use tokio::sync::broadcast;

#[cfg(feature = "chrome")]
use chromiumoxide::BrowserConfig;

/// get chrome configuration
#[cfg(all(feature = "chrome", not(feature = "chrome_headed")))]
pub fn get_browser_config() -> Result<BrowserConfig, String> {
    BrowserConfig::builder().build()
}

/// get chrome configuration headful
#[cfg(all(feature = "chrome", feature = "chrome_headed"))]
pub fn get_browser_config() -> Result<BrowserConfig, String> {
    BrowserConfig::builder().with_head().build()
}

/// shutdown the chrome instance by process id
#[cfg(feature = "chrome")]
#[cfg(target_os = "windows")]
fn shutdown_chrome(pid: &u32) {
    let _ = std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/F"])
        .spawn();
}

/// shutdown the chrome instance by process id
#[cfg(feature = "chrome")]
#[cfg(not(target_os = "windows"))]
fn shutdown_chrome(pid: &u32) {
    let _ = std::process::Command::new("kill")
        .args(["-9", &pid.to_string()])
        .spawn();
}

#[cfg(feature = "chrome")]
/// launch a chromium browser and wait until the instance is up blocking the thread.
pub fn launch_browser() -> (
    std::option::Option<chromiumoxide::Browser>,
    tokio::task::JoinHandle<()>,
    u32,
) {
    let (tx, mut rx) = tokio::sync::oneshot::channel();
    use chromiumoxide::Browser;

    let handle = tokio::task::spawn(async move {
        match get_browser_config() {
            Ok(c) => match Browser::launch(c).await {
                Ok(x) => {
                    let (mut browser, mut handler) = x;
                    let id = browser.get_mut_child().unwrap().as_mut_inner().id();

                    if let Err(_) = tx.send((browser, id)) {
                        println!("the receiver dropped");
                    }

                    while let Some(h) = handler.next().await {
                        if h.is_err() {
                            break;
                        }
                    }
                }
                _ => (),
            },
            _ => (),
        };
    });

    loop {
        let x = match rx.try_recv() {
            Ok(v) => (Some(v.0), v.1),
            Err(_) => (None, Default::default()),
        };

        if x.0.is_some() {
            break (x.0, handle, x.1.unwrap_or_default());
        }
    }
}

#[cfg(feature = "chrome")]
lazy_static! {
    /// a chrome instance
    pub static ref CHROME: (
        std::option::Option<chromiumoxide::Browser>,
        tokio::task::JoinHandle<()>,
        u32
    ) = {
        launch_browser()
    };
}

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
#[derive(Debug, Clone)]
pub enum CrawlStatus {
    /// The crawl did not start yet.
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
#[derive(Debug, Clone)]
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
        }
    }

    /// return `true` if URL:
    ///
    /// - is not already crawled
    /// - is not blacklisted
    /// - is not forbidden in robot.txt file (if parameter is defined)
    #[inline]
    #[cfg(not(feature = "regex"))]
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
    /// - is not blacklisted
    /// - is not forbidden in robot.txt file (if parameter is defined)
    #[inline]
    #[cfg(feature = "regex")]
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
    #[cfg(all(not(feature = "glob"), not(feature = "decentralized")))]
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
                        Some(cb) => cb(u),
                        _ => u,
                    };

                    self.links_visited.insert(link_result);
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

    /// Start to crawl website with async conccurency
    pub async fn crawl(&mut self) {
        let (client, handle) = self.setup().await;

        self.crawl_concurrent(client, handle).await;
    }

    /// Start to crawl website in sync
    pub async fn crawl_sync(&mut self) {
        let (client, handle) = self.setup().await;

        self.crawl_sequential(&client, handle).await;
    }

    /// Start to scrape/download website with async conccurency
    pub async fn scrape(&mut self) {
        let (client, handle) = self.setup().await;

        self.scrape_concurrent(&client, handle).await;
    }

    /// Start to crawl website concurrently
    #[cfg(not(feature = "decentralized"))]
    async fn crawl_concurrent(&mut self, client: Client, handle: Option<Arc<AtomicI8>>) {
        let selectors = get_page_selectors(
            &self.domain.inner(),
            self.configuration.subdomains,
            self.configuration.tld,
        );

        // crawl if valid selector
        if selectors.is_some() {
            self.status = CrawlStatus::Active;
            let blacklist_url = self.configuration.get_blacklist();
            let mut interval = Box::pin(tokio::time::interval(Duration::from_millis(10)));
            let throttle = Box::pin(self.get_delay());
            let on_link_find_callback = self.on_link_find_callback;
            let shared = Arc::new((
                client,
                unsafe { selectors.unwrap_unchecked() },
                self.channel.clone(),
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
                                        let page =
                                            Page::new(&link_result.0.as_ref(), &shared.0).await;
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
    async fn crawl_concurrent(&mut self, client: Client, handle: Option<Arc<AtomicI8>>) {
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

    /// Start to crawl website sequential
    async fn crawl_sequential(&mut self, client: &Client, handle: Option<Arc<AtomicI8>>) {
        let selectors = get_page_selectors(
            &self.domain.inner(),
            self.configuration.subdomains,
            self.configuration.tld,
        );

        if selectors.is_some() {
            self.status = CrawlStatus::Active;
            let blacklist_url = self.configuration.get_blacklist();
            let selectors = unsafe { selectors.unwrap_unchecked() };
            let delay = Box::from(self.configuration.delay);
            let delay_enabled = self.configuration.delay > 0;
            let on_link_find_callback = self.on_link_find_callback;
            let mut interval = tokio::time::interval(Duration::from_millis(10));

            let mut new_links: HashSet<CaseInsensitiveString> = HashSet::new();
            let mut links: HashSet<CaseInsensitiveString> =
                self.crawl_establish(&client, &selectors, false).await;

            let channel = self.channel.clone();

            // crawl while links exists
            loop {
                for link in links.iter() {
                    match handle.as_ref() {
                        Some(handle) => {
                            while handle.load(Ordering::Relaxed) == 1 {
                                interval.tick().await;
                            }
                            if handle.load(Ordering::Relaxed) == 2 {
                                links.clear();
                                break;
                            }
                        }
                        None => (),
                    }
                    if !self.is_allowed(&link, &blacklist_url) {
                        continue;
                    }
                    self.links_visited.insert(link.clone());
                    log("fetch", link);
                    if delay_enabled {
                        tokio::time::sleep(Duration::from_millis(*delay)).await;
                    }
                    let link = link.clone();
                    let link_result = match on_link_find_callback {
                        Some(cb) => cb(link, None),
                        _ => (link, None),
                    };
                    let page = Page::new(&link_result.0.as_ref(), &client).await;
                    let page_links = page.links(&selectors).await;
                    task::yield_now().await;
                    new_links.extend(page_links);
                    task::yield_now().await;

                    match &channel {
                        Some(c) => {
                            match c.0.send(page) {
                                _ => (),
                            };
                        }
                        _ => (),
                    };
                }

                links.clone_from(&(&new_links - &self.links_visited));
                new_links.clear();
                if new_links.capacity() >= 1500 {
                    new_links.shrink_to_fit();
                }
                task::yield_now().await;
                if links.is_empty() {
                    break;
                }
            }

            self.status = CrawlStatus::Idle;
        }
    }

    /// Start to scape website concurrently and store html
    async fn scrape_concurrent(&mut self, client: &Client, handle: Option<Arc<AtomicI8>>) {
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
                    let client = client.clone();
                    let permit = SEM.acquire().await.unwrap();
                    let channel = self.channel.clone();
                    let selectors = selectors.clone();

                    set.spawn(async move {
                        drop(permit);
                        let page = crate::utils::fetch_page_html(&link.as_ref(), &client).await;
                        let page = build(&link.as_ref(), page);

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
    pub fn with_user_agent(&mut self, user_agent: Option<CompactString>) -> &mut Self {
        self.configuration.with_user_agent(user_agent);
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

#[cfg(feature = "chrome")]
impl Drop for Website {
    /// check if chrome instances are alive if not drop the browser
    fn drop(&mut self) {
        match &CHROME.0 {
            Some(browser) => {
                tokio::spawn(async move {
                    match browser.pages().await {
                        Ok(pages) => {
                            if pages.len() == 0 {
                                shutdown_chrome(&CHROME.2);
                            }
                        }
                        _ => (),
                    };
                });
            }
            _ => (),
        }
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
