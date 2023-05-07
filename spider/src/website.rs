use crate::black_list::contains;
use crate::configuration::{get_ua, Configuration};
use crate::packages::robotparser::parser::RobotFileParser;
use crate::page::{build, get_page_selectors, Page};

use crate::utils::log;
use crate::CaseInsensitiveString;
use compact_str::CompactString;
use hashbrown::HashSet;
use reqwest::header;
use reqwest::header::CONNECTION;
use reqwest::Client;
use std::sync::atomic::{AtomicI8, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::runtime::Handle;
use tokio::sync::Semaphore;
use tokio::task;
use tokio::task::JoinSet;
use tokio_stream::StreamExt;

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
    /// callback when a link is found.
    pub on_link_find_callback: Option<fn(CompactString) -> CompactString>,
    /// Robot.txt parser holder.
    robot_file_parser: Option<Box<RobotFileParser>>,
    /// the base root domain of the crawl
    domain: Box<CompactString>,
}

impl Website {
    /// Initialize Website object with a start link to crawl.
    pub fn new(domain: &str) -> Self {
        let domain = if domain.ends_with('/') {
            domain.into()
        } else {
            string_concat!(domain, "/")
        };

        Self {
            configuration: Configuration::new().into(),
            links_visited: Box::new(HashSet::new()),
            pages: None,
            robot_file_parser: None,
            on_link_find_callback: None,
            domain: CompactString::new(domain).into(),
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
        blacklist_url: &Box<Vec<regex::Regex>>,
    ) -> bool {
        if self.links_visited.contains(link) {
            false
        } else {
            self.is_allowed_default(&link.0, blacklist_url)
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
        link: &CompactString,
        blacklist_url: &Box<Vec<regex::Regex>>,
    ) -> bool {
        if !blacklist_url.is_empty() {
            !contains(blacklist_url, &link)
        } else {
            self.is_allowed_robots(&link)
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
            self.is_allowed_default(&link.0, blacklist_url)
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

    /// crawl delay getter
    fn get_delay(&self) -> Duration {
        Duration::from_millis(self.configuration.delay)
    }

    /// configure the robots parser on initial crawl attempt and run.
    pub async fn configure_robots_parser(&mut self, client: Client) -> Client {
        if self.configuration.respect_robots_txt {
            let robot_file_parser = self
                .robot_file_parser
                .get_or_insert_with(|| RobotFileParser::new());

            if robot_file_parser.mtime() <= 4000 {
                robot_file_parser.read(&client, &self.domain).await;
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
    pub fn configure_http_client(&mut self, _: bool) -> Client {
        let client = Client::builder()
            .user_agent(match &self.configuration.user_agent {
                Some(ua) => ua.as_str(),
                _ => &get_ua(),
            })
            .brotli(true)
            .gzip(true)
            .tcp_keepalive(Duration::from_millis(500))
            .pool_idle_timeout(None);

        let client = if self.configuration.http2_prior_knowledge {
            client.http2_prior_knowledge()
        } else {
            let mut headers = header::HeaderMap::new();
            headers.insert(CONNECTION, header::HeaderValue::from_static("keep-alive"));

            client.default_headers(headers)
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
    pub fn configure_http_client(&mut self, scraping: bool) -> Client {
        let mut headers = header::HeaderMap::new();
        headers.insert(CONNECTION, header::HeaderValue::from_static("keep-alive"));

        let mut client = Client::builder()
            .user_agent(match &self.configuration.user_agent {
                Some(ua) => ua.as_str(),
                _ => &get_ua(),
            })
            .brotli(true)
            .gzip(true)
            .tcp_keepalive(Duration::from_millis(500))
            .pool_idle_timeout(None);

        let worker_url = if scraping {
            std::env::var("SPIDER_WORKER_SCRAPER")
                .unwrap_or_else(|_| "http://127.0.0.1:3031".to_string())
        } else {
            std::env::var("SPIDER_WORKER").unwrap_or_else(|_| "http://127.0.0.1:3030".to_string())
        };
        let worker_url = worker_url.split(",");

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
            headers.insert(reqwest::header::REFERER, header::HeaderValue::from(referer));
        }

        for worker in worker_url {
            client = client.proxy(reqwest::Proxy::all(worker).unwrap());
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
        let domain = self.domain.clone();

        tokio::spawn(async move {
            let mut l = CONTROLLER.lock().await.1.to_owned();

            while l.changed().await.is_ok() {
                let n = &*l.borrow();
                let (name, rest) = n;

                let url = if name.ends_with('/') {
                    name.into()
                } else {
                    string_concat!(name.clone(), "/")
                };

                if domain.eq_ignore_ascii_case(&url) {
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
    pub async fn setup(&mut self, scraping: bool) -> (Client, Option<Arc<AtomicI8>>) {
        let client = self.configure_http_client(scraping);

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
    pub async fn setup<T>(&mut self, scraping: bool) -> (Client, Option<T>) {
        let client = self.configure_http_client(scraping);

        // allow fresh crawls to run fully
        if !self.links_visited.is_empty() {
            self.links_visited.clear();
        }

        (self.configure_robots_parser(client).await, None)
    }

    /// Start to crawl website with async conccurency
    pub async fn crawl(&mut self) {
        let (client, handle) = self.setup(false).await;

        self.crawl_concurrent(client, handle).await;
    }

    /// Start to crawl website in sync
    pub async fn crawl_sync(&mut self) {
        let (client, handle) = self.setup(false).await;

        self.crawl_sequential(&client, handle).await;
    }

    /// Start to scrape/download website with async conccurency
    pub async fn scrape(&mut self) {
        let (client, handle) = self.setup(true).await;

        self.scrape_concurrent(&client, handle).await;
    }

    /// expand links for crawl
    #[cfg(all(not(feature = "glob"), not(feature = "decentralized")))]
    async fn crawl_establish(
        &mut self,
        client: &Client,
        base: &(CompactString, smallvec::SmallVec<[CompactString; 2]>),
    ) -> HashSet<CaseInsensitiveString> {
        let links: HashSet<CaseInsensitiveString> =
            if self.is_allowed_default(&self.domain, &self.configuration.get_blacklist()) {
                let page = Page::new(&self.domain, &client).await;
                let u = page.get_url().into();

                let link_result = match self.on_link_find_callback {
                    Some(cb) => cb(u),
                    _ => u,
                };

                self.links_visited
                    .insert(CaseInsensitiveString { 0: link_result });

                HashSet::from(page.links(&base).await)
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
        base: &(CompactString, smallvec::SmallVec<[CompactString; 2]>),
    ) -> HashSet<CaseInsensitiveString> {
        // base_domain name passed here is for primary url determination and not subdomain.tld placement
        let (domain_name, _) = base;
        let domain_name = if domain_name.is_empty() {
            &*self.domain
        } else {
            domain_name
        };
        let links: HashSet<CaseInsensitiveString> =
            if self.is_allowed_default(&domain_name, &self.configuration.get_blacklist()) {
                let page = Page::new(&domain_name, &client).await;
                let link = domain_name.clone();

                let link_result = match self.on_link_find_callback {
                    Some(cb) => cb(link),
                    _ => link,
                };

                self.links_visited
                    .insert(CaseInsensitiveString { 0: link_result });

                HashSet::from(page.links)
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
        base: &(CompactString, smallvec::SmallVec<[CompactString; 2]>),
    ) -> HashSet<CaseInsensitiveString> {
        use crate::features::glob::expand_url;
        use url::Url;

        let mut links: HashSet<CaseInsensitiveString> = HashSet::new();
        let (domain_name, _) = base;
        let domain_name = if domain_name.is_empty() {
            &*self.domain
        } else {
            domain_name
        };
        let mut expanded = expand_url(&domain_name.as_str());

        if expanded.len() == 0 {
            match Url::parse(domain_name) {
                Ok(u) => {
                    expanded.push(crate::page::convert_abs_path(&u, "/").as_str().into());
                }
                _ => (),
            };
        };

        let blacklist_url = self.configuration.get_blacklist();

        for link in expanded {
            if self.is_allowed_default(&link, &blacklist_url) {
                let page = Page::new(&link, &client).await;
                let u = page.get_url();

                let u = if u.is_empty() { link } else { u.into() };

                let link_result = match self.on_link_find_callback {
                    Some(cb) => cb(u),
                    _ => u,
                };

                self.links_visited
                    .insert(CaseInsensitiveString { 0: link_result });

                links.extend(HashSet::from(page.links));
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
    ) -> HashSet<CaseInsensitiveString> {
        use crate::features::glob::expand_url;
        use url::Url;

        let mut links: HashSet<CaseInsensitiveString> = HashSet::new();
        let (domain_name, _) = base;
        let domain_name = if domain_name.is_empty() {
            &*self.domain
        } else {
            domain_name
        };

        let mut expanded = expand_url(&domain_name.as_str());

        if expanded.len() == 0 {
            match Url::parse(domain_name) {
                Ok(u) => {
                    expanded.push(crate::page::convert_abs_path(&u, "/").as_str().into());
                }
                _ => (),
            };
        };

        let blacklist_url = self.configuration.get_blacklist();

        for link in expanded {
            if self.is_allowed_default(&link, &blacklist_url) {
                let page = Page::new(&link, &client).await;
                let u = page.get_url().into();

                let link_result = match self.on_link_find_callback {
                    Some(cb) => cb(u),
                    _ => u,
                };

                self.links_visited
                    .insert(CaseInsensitiveString { 0: link_result });

                links.extend(HashSet::from(page.links(&base).await));
            }
        }

        links
    }

    /// Start to crawl website concurrently
    #[cfg(not(feature = "decentralized"))]
    async fn crawl_concurrent(&mut self, client: Client, handle: Option<Arc<AtomicI8>>) {
        let selectors = get_page_selectors(
            &self.domain,
            self.configuration.subdomains,
            self.configuration.tld,
        );

        let blacklist_url = self.configuration.get_blacklist();

        // crawl if valid selector
        if selectors.is_some() {
            let on_link_find_callback = self.on_link_find_callback;
            let mut interval = Box::pin(tokio::time::interval(Duration::from_millis(10)));
            let throttle = Box::pin(self.get_delay());
            let shared = Arc::new((client, unsafe { selectors.unwrap_unchecked() }));

            let mut links: HashSet<CaseInsensitiveString> =
                self.crawl_establish(&shared.0, &shared.1).await;

            let mut set: JoinSet<HashSet<CaseInsensitiveString>> = JoinSet::new();
            let chandle = Handle::current();

            // crawl while links exists
            loop {
                let stream =
                    tokio_stream::iter::<HashSet<CaseInsensitiveString>>(links.drain().collect())
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
                                        Some(cb) => cb(link.0),
                                        _ => link.0,
                                    };
                                    let page = Page::new(&link_result, &shared.0).await;
                                    let page_links = page.links(&shared.1).await;

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
    }

    /// Start to crawl website concurrently
    #[cfg(feature = "decentralized")]
    async fn crawl_concurrent(&mut self, client: Client, handle: Option<Arc<AtomicI8>>) {
        match url::Url::parse(&self.domain) {
            Ok(_) => {
                let blacklist_url = self.configuration.get_blacklist();
                let domain = self.domain.as_str();
                let on_link_find_callback = self.on_link_find_callback;
                let mut interval = Box::pin(tokio::time::interval(Duration::from_millis(10)));
                let throttle = Box::pin(self.get_delay());

                // http worker verify
                let http_worker = std::env::var("SPIDER_WORKER")
                    .unwrap_or_else(|_| "http:".to_string())
                    .starts_with("http:");

                let domain = if http_worker && domain.starts_with("https") {
                    domain.replacen("https", "http", 1)
                } else {
                    domain.to_string()
                };

                let mut links: HashSet<CaseInsensitiveString> = self
                    .crawl_establish(&client, &(domain.clone().into(), Default::default()))
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
                                        let link_results =
                                            if http_worker && link.0.starts_with("https") {
                                                link.0.replacen("https", "http", 1).into()
                                            } else {
                                                link.0
                                            };

                                        let link_results = match on_link_find_callback {
                                            Some(cb) => cb(link_results),
                                            _ => link_results,
                                        };

                                        let page = Page::new(&link_results, &client).await;

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
            &self.domain,
            self.configuration.subdomains,
            self.configuration.tld,
        );
        let blacklist_url = self.configuration.get_blacklist();

        if selectors.is_some() {
            let selectors = unsafe { selectors.unwrap_unchecked() };
            let delay = Box::from(self.configuration.delay);
            let delay_enabled = self.configuration.delay > 0;
            let on_link_find_callback = self.on_link_find_callback;
            let mut interval = tokio::time::interval(Duration::from_millis(10));

            let mut new_links: HashSet<CaseInsensitiveString> = HashSet::new();
            let mut links: HashSet<CaseInsensitiveString> =
                self.crawl_establish(&client, &selectors).await;

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
                        Some(cb) => cb(link.0),
                        _ => link.0,
                    };

                    let page = Page::new(&link_result, &client).await;
                    let page_links = page.links(&selectors).await;
                    task::yield_now().await;
                    new_links.extend(page_links);
                    task::yield_now().await;
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
        }
    }

    /// Start to scape website concurrently and store html
    async fn scrape_concurrent(&mut self, client: &Client, handle: Option<Arc<AtomicI8>>) {
        let selectors = get_page_selectors(
            &self.domain,
            self.configuration.subdomains,
            self.configuration.tld,
        );
        let blacklist_url = self.configuration.get_blacklist();

        if selectors.is_some() {
            self.pages = Some(Box::new(Vec::new()));
            let delay = self.configuration.delay;
            let on_link_find_callback = self.on_link_find_callback;
            let mut interval = tokio::time::interval(Duration::from_millis(10));
            let selectors = Arc::new(unsafe { selectors.unwrap_unchecked() });
            let throttle = Duration::from_millis(delay);

            let mut links: HashSet<CaseInsensitiveString> = self
                .crawl_establish(&client, &(selectors.0.clone(), selectors.1.clone()))
                .await;

            let mut set: JoinSet<(CompactString, Option<String>)> = JoinSet::new();

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

                    set.spawn(async move {
                        let link_result = match on_link_find_callback {
                            Some(cb) => cb(link.0),
                            _ => link.0,
                        };

                        drop(permit);

                        let page = crate::utils::fetch_page_html(&link_result, &client).await;

                        (link_result, page)
                    });
                }

                task::yield_now().await;

                if links.capacity() >= 1500 {
                    links.shrink_to_fit();
                }

                while let Some(res) = set.join_next().await {
                    match res {
                        Ok(msg) => {
                            if msg.1.is_some() {
                                let page = build(&msg.0, msg.1);
                                let page_links = page.links(&*selectors).await;
                                links.extend(&page_links - &self.links_visited);
                                task::yield_now().await;
                                self.pages.as_mut().unwrap().push(page);
                            }
                        }
                        _ => (),
                    };
                }

                task::yield_now().await;
                if links.is_empty() {
                    break;
                }
            }
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
    let url = "https://w.com";
    let mut website: Website = Website::new(url);
    website.crawl().await;
    let mut uniq: Box<HashSet<CaseInsensitiveString>> = Box::new(HashSet::new());
    uniq.insert(format!("{}/", url.to_string()).into()); // TODO: remove trailing slash mutate

    assert_eq!(website.links_visited, uniq); // only the target url should exist
}

#[tokio::test]
#[cfg(feature = "decentralized")]
async fn crawl_invalid() {
    let url = "https://w.com";
    let mut website: Website = Website::new(url);
    website.crawl().await;
    let mut uniq: Box<HashSet<CaseInsensitiveString>> = Box::new(HashSet::new());
    uniq.insert(format!("{}/", url.to_string().replace("https:", "http:")).into()); // TODO: remove trailing slash mutate

    assert_eq!(website.links_visited, uniq); // only the target url should exist
}

#[tokio::test]
#[cfg(not(feature = "decentralized"))]
async fn crawl_link_callback() {
    let mut website: Website = Website::new("https://choosealicense.com");
    website.on_link_find_callback = Some(|s| {
        log("callback link target: {}", &s);
        s
    });
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
    website
        .configuration
        .blacklist_url
        .insert(Default::default())
        .push(CompactString::from("/choosealicense.com/"));
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

    let (client, _): (Client, Option<Arc<AtomicI8>>) = website.setup(false).await;

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

    let (client_second, _): (Client, Option<Arc<AtomicI8>>) = website_second.setup(false).await;
    website_second.configure_robots_parser(client_second).await;

    assert_eq!(website_second.configuration.delay, 60000); // should equal one minute in ms

    // test crawl delay with wildcard agent [DOES not work when using set agent]
    let mut website_third: Website = Website::new("https://www.mongodb.com");
    website_third.configuration.respect_robots_txt = true;
    let (client_third, _): (Client, Option<Arc<AtomicI8>>) = website_third.setup(false).await;

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

#[cfg(feature = "glob")]
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

    println!("{:?}", website.links_visited);

    assert!(
        website
            .links_visited
            .contains::<CaseInsensitiveString>(&"https://choosealicense.com/licenses/".into()),
        "{:?}",
        website.links_visited
    );
}

#[cfg(feature = "socks")]
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
        if links_visited.0.as_str().contains("/licenses/") {
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

    let url = "https://choosealicense.com/";
    let mut website: Website = Website::new(&url);

    let start = tokio::time::Instant::now();

    tokio::spawn(async move {
        pause(url).await;
        // static website test pause/resume - scan will never take longer than 5secs for target website choosealicense
        tokio::time::sleep(Duration::from_millis(5000)).await;
        resume(url).await;
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
    let url = "https://rsseau.fr/";
    let mut website: Website = Website::new(&url);

    tokio::spawn(async move {
        shutdown(url).await;
    });

    website.crawl().await;

    assert_eq!(website.links_visited.len(), 1);
}
