use crate::black_list::contains;
use crate::configuration::{get_ua, Configuration};
use crate::packages::robotparser::parser::RobotFileParser;
use crate::page::{build, get_page_selectors, Page};
use crate::utils::{log, Handler, CONTROLLER};
use compact_str::CompactString;
use hashbrown::HashSet;
use reqwest::header;
use reqwest::header::CONNECTION;
use reqwest::Client;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicI8, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;
use tokio::task;
use tokio::task::JoinSet;
use tokio::time::sleep;
use tokio_stream::StreamExt;

/// case-insensitive string handling
#[derive(Debug, Clone)]
pub struct CaseInsensitiveString(CompactString);

impl PartialEq for CaseInsensitiveString {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.0.eq_ignore_ascii_case(&other.0)
    }
}

impl Eq for CaseInsensitiveString {}

impl Hash for CaseInsensitiveString {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.to_ascii_lowercase().hash(state)
    }
}

impl From<&str> for CaseInsensitiveString {
    #[inline]
    fn from(s: &str) -> Self {
        CaseInsensitiveString { 0: s.into() }
    }
}

impl From<String> for CaseInsensitiveString {
    fn from(s: String) -> Self {
        CaseInsensitiveString { 0: s.into() }
    }
}

impl AsRef<str> for CaseInsensitiveString {
    #[inline]
    fn as_ref(&self) -> &str {
        &self.0
    }
}

static SEM: Semaphore = Semaphore::const_new(124);

/// Represents a website to crawl and gather all links.
/// ```rust
/// use spider::website::Website;
/// let mut website = Website::new("http://example.com");
/// website.crawl();
/// // `Website` will be filled with `Pages` when crawled. To get them, just use
/// for page in website.get_pages() {
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
    pub fn is_allowed(&self, link: &CaseInsensitiveString) -> bool {
        if self.links_visited.contains(link) {
            false
        } else {
            self.is_allowed_default(&link.0)
        }
    }

    /// return `true` if URL:
    ///
    /// - is not blacklisted
    /// - is not forbidden in robot.txt file (if parameter is defined)
    #[inline]
    pub fn is_allowed_default(&self, link: &CompactString) -> bool {
        if !self.configuration.blacklist_url.is_none() {
            match &self.configuration.blacklist_url {
                Some(v) => !contains(v, &link),
                _ => true,
            }
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
    pub fn get_pages(&self) -> Vec<Page> {
        if !self.pages.is_none() {
            unsafe { *self.pages.as_ref().unwrap_unchecked().clone() }
        } else {
            self.links_visited
                .iter()
                .map(|l| build(&l.0, Default::default()))
                .collect::<Vec<Page>>()
        }
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
    fn configure_http_client(&mut self) -> Client {
        let mut headers = header::HeaderMap::new();
        headers.insert(CONNECTION, header::HeaderValue::from_static("keep-alive"));

        let client = Client::builder()
            .default_headers(headers)
            .user_agent(match &self.configuration.user_agent {
                Some(ua) => ua.as_str(),
                _ => &get_ua(),
            })
            .brotli(true)
            .gzip(true)
            .tcp_keepalive(Duration::from_millis(500))
            .pool_idle_timeout(None);

        match &self.configuration.request_timeout {
            Some(t) => client.timeout(**t),
            _ => client,
        }
        .build()
        .unwrap()
    }

    /// setup atomic controller
    fn configure_handler(&self) -> Arc<AtomicI8> {
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
    pub async fn setup(&mut self) -> (Client, Arc<AtomicI8>) {
        let client = self.configure_http_client();

        // allow fresh crawls to run fully
        if !self.links_visited.is_empty() {
            self.links_visited.clear();
        }

        (
            self.configure_robots_parser(client).await,
            self.configure_handler(),
        )
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
    async fn crawl_concurrent(&mut self, client: Client, handle: Arc<AtomicI8>) {
        let on_link_find_callback = self.on_link_find_callback;
        let mut interval = Box::pin(tokio::time::interval(Duration::from_millis(10)));
        let throttle = Box::pin(self.get_delay());

        let channel_buffer = if self.configuration.channel_buffer > 224 {
            self.configuration.channel_buffer as usize
        } else {
            224
        };
        let channel_unload_limit = channel_buffer - 1;

        let shared = Arc::new((
            client,
            get_page_selectors(
                &self.domain,
                self.configuration.subdomains,
                self.configuration.tld,
            ),
        ));

        let mut links: HashSet<CaseInsensitiveString> =
            if self.is_allowed_default(&CompactString::new(&self.domain.as_str())) {
                let page = Page::new(&self.domain, &shared.0).await;
                let u = page.get_url().into();
                let link_result = match on_link_find_callback {
                    Some(cb) => cb(u),
                    _ => u,
                };

                self.links_visited
                    .insert(CaseInsensitiveString { 0: link_result });
                HashSet::from(page.links(&shared.1, None).await)
            } else {
                HashSet::new()
            };

        let mut set: JoinSet<HashSet<CaseInsensitiveString>> = JoinSet::new();

        // crawl while links exists
        loop {
            let stream =
                tokio_stream::iter::<HashSet<CaseInsensitiveString>>(links.drain().collect())
                    .throttle(*throttle);
            tokio::pin!(stream);

            loop {
                match stream.next().await {
                    Some(link) => {
                        while handle.load(Ordering::Relaxed) == 1 {
                            interval.tick().await;
                        }
                        if handle.load(Ordering::Relaxed) == 2 {
                            set.shutdown().await;
                            break;
                        }
                        if !self.is_allowed(&link) {
                            continue;
                        }
                        log("fetch", &link);
                        self.links_visited.insert(link.clone());
                        let shared = shared.clone();
                        let permit = SEM.acquire().await.unwrap();
                        task::yield_now().await;

                        set.spawn(async move {
                            let link_result = match on_link_find_callback {
                                Some(cb) => cb(link.0),
                                _ => link.0,
                            };
                            let page = Page::new(&link_result, &shared.0).await;
                            let page_links = page
                                .links(
                                    &shared.1,
                                    Some(SEM.available_permits() < channel_unload_limit),
                                )
                                .await;

                            drop(permit);

                            page_links
                        });

                        task::yield_now().await;
                    }
                    _ => {
                        break;
                    }
                }
            }

            task::yield_now().await;

            if links.capacity() >= 1500 {
                links.shrink_to_fit();
            }

            while let Some(res) = set.join_next().await {
                let msg = res.unwrap();
                links.extend(&msg - &self.links_visited);
                task::yield_now().await;
            }

            if links.is_empty() {
                break;
            }
        }
    }

    /// Start to crawl website sequential
    async fn crawl_sequential(&mut self, client: &Client, handle: Arc<AtomicI8>) {
        let delay = Box::from(self.configuration.delay);
        let delay_enabled = self.configuration.delay > 0;
        let on_link_find_callback = self.on_link_find_callback;
        let mut interval = tokio::time::interval(Duration::from_millis(10));
        let selectors = get_page_selectors(
            &self.domain,
            self.configuration.subdomains,
            self.configuration.tld,
        );
        let mut new_links: HashSet<CaseInsensitiveString> = HashSet::new();

        let mut links: HashSet<CaseInsensitiveString> =
            if self.is_allowed_default(&CompactString::new(&self.domain.as_str())) {
                let page = Page::new(&self.domain, &client).await;
                let link_result = match on_link_find_callback {
                    Some(cb) => cb(page.get_url().into()),
                    _ => page.get_url().into(),
                };
                self.links_visited
                    .insert(CaseInsensitiveString { 0: link_result });
                HashSet::from(page.links(&selectors, None).await)
            } else {
                HashSet::new()
            };

        // crawl while links exists
        loop {
            for link in links.iter() {
                while handle.load(Ordering::Relaxed) == 1 {
                    interval.tick().await;
                }
                if handle.load(Ordering::Relaxed) == 2 {
                    links.clear();
                    break;
                }
                if !self.is_allowed(&link) {
                    continue;
                }
                self.links_visited.insert(link.clone());
                log("fetch", link);
                if delay_enabled {
                    sleep(Duration::from_millis(*delay)).await;
                }
                let link = link.clone();
                let link_result = match on_link_find_callback {
                    Some(cb) => cb(link.0),
                    _ => link.0,
                };

                let page = Page::new(&link_result, &client).await;
                let page_links = page.links(&selectors, None).await;
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

    /// Start to scape website concurrently and store html
    async fn scrape_concurrent(&mut self, client: &Client, handle: Arc<AtomicI8>) {
        self.pages = Some(Box::new(Vec::new()));
        let delay = self.configuration.delay;
        let on_link_find_callback = self.on_link_find_callback;
        let mut interval = tokio::time::interval(Duration::from_millis(10));
        let selectors = Arc::new(get_page_selectors(
            &self.domain,
            self.configuration.subdomains,
            self.configuration.tld,
        ));
        let throttle = Duration::from_millis(delay);
        let mut links: HashSet<CaseInsensitiveString> =
            if self.is_allowed_default(&CompactString::new(&self.domain.as_str())) {
                let page = Page::new(&self.domain, &client).await;
                let u = page.get_url().into();

                let link_result = match on_link_find_callback {
                    Some(cb) => cb(u),
                    _ => u,
                };

                self.links_visited
                    .insert(CaseInsensitiveString { 0: link_result });

                HashSet::from(page.links(&selectors, None).await)
            } else {
                HashSet::new()
            };

        let mut set: JoinSet<Page> = JoinSet::new();

        // crawl while links exists
        loop {
            let stream =
                tokio_stream::iter::<HashSet<CaseInsensitiveString>>(links.drain().collect())
                    .throttle(throttle);
            tokio::pin!(stream);

            while let Some(l) = stream.next().await {
                while handle.load(Ordering::Relaxed) == 1 {
                    interval.tick().await;
                }
                if handle.load(Ordering::Relaxed) == 2 {
                    set.shutdown().await;
                    break;
                }
                if !self.is_allowed(&l) {
                    continue;
                }
                let link = l.clone();
                self.links_visited.insert(l);
                log("fetch", &link);
                let client = client.clone();
                let permit = SEM.acquire().await.unwrap();

                set.spawn(async move {
                    let link_result = match on_link_find_callback {
                        Some(cb) => cb(link.0),
                        _ => link.0,
                    };

                    drop(permit);

                    let page = Page::new(&link_result, &client).await;

                    page
                });
            }

            task::yield_now().await;

            if links.capacity() >= 1500 {
                links.shrink_to_fit();
            }

            while let Some(res) = set.join_next().await {
                let msg = res.unwrap();
                let page_links = msg.links(&*selectors, None).await;
                task::yield_now().await;
                links.extend(&page_links - &self.links_visited);
                task::yield_now().await;
                self.pages.as_mut().unwrap().push(msg);
                task::yield_now().await;
            }

            task::yield_now().await;
            if links.is_empty() {
                break;
            }
        }
    }
}

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

    assert_eq!(website.get_pages()[0].get_html().is_empty(), false);
}

#[tokio::test]
async fn crawl_subsequential() {
    let mut website: Website = Website::new("https://choosealicense.com");
    website.configuration.delay = 50;
    website.crawl_sync().await;
    assert!(
        website
            .links_visited
            .contains::<CaseInsensitiveString>(&"https://choosealicense.com/licenses/".into()),
        "{:?}",
        website.links_visited
    );
}

#[tokio::test]
async fn crawl_invalid() {
    let url = "https://w.com";
    let mut website: Website = Website::new(url);
    website.crawl().await;
    let mut uniq: Box<HashSet<CaseInsensitiveString>> = Box::new(HashSet::new());
    uniq.insert(format!("{}/", url.to_string()).into()); // TODO: remove trailing slash mutate

    assert_eq!(website.links_visited, uniq); // only the target url should exist
}

#[tokio::test]
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
async fn test_respect_robots_txt() {
    let mut website: Website = Website::new("https://stackoverflow.com");
    website.configuration.respect_robots_txt = true;
    website.configuration.user_agent = Some(Box::new("*".into()));

    let (client, _) = website.setup().await;
    website.configure_robots_parser(client).await;

    assert_eq!(website.configuration.delay, 0);

    assert!(!website.is_allowed(&"https://stackoverflow.com/posts/".into()));

    // test match for bing bot
    let mut website_second: Website = Website::new("https://www.mongodb.com");
    website_second.configuration.respect_robots_txt = true;
    website_second.configuration.user_agent = Some(Box::new("bingbot".into()));

    let (client_second, _) = website_second.setup().await;
    website_second.configure_robots_parser(client_second).await;

    assert_eq!(website_second.configuration.delay, 60000); // should equal one minute in ms

    // test crawl delay with wildcard agent [DOES not work when using set agent]
    let mut website_third: Website = Website::new("https://www.mongodb.com");
    website_third.configuration.respect_robots_txt = true;
    let (client_third, _) = website_third.setup().await;

    website_third.configure_robots_parser(client_third).await;

    assert_eq!(website_third.configuration.delay, 10000); // should equal 10 seconds in ms
}

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
#[ignore]
async fn test_crawl_pause_resume() {
    use crate::utils::{pause, resume};

    let url = "https://choosealicense.com/";
    let mut website: Website = Website::new(&url);

    let start = tokio::time::Instant::now();

    tokio::spawn(async move {
        pause(url).await;
        // static website test pause/resume - scan will never take longer than 5secs for target website choosealicense
        sleep(Duration::from_millis(5000)).await;
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
