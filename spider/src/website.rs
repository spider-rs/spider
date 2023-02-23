use crate::black_list::contains;
use crate::configuration::{get_ua, Configuration};
use crate::packages::robotparser::RobotFileParser;
use crate::page::{build, get_page_selectors, Page};
use crate::utils::{log, Handler, CONTROLLER};
use hashbrown::HashSet;
use scraper::Selector;
use std::sync::atomic::{AtomicI8, Ordering};
use std::sync::Arc;
use std::time::Duration;

use reqwest::header;
use reqwest::header::CONNECTION;
use reqwest::Client;
use std::hash::{Hash, Hasher};
use tokio::sync::mpsc::{channel, Receiver, Sender};
use tokio::task;
use tokio::time::sleep;
use tokio_stream::StreamExt;

/// case-insensitive string handling
#[derive(Debug, Clone)]
pub struct CaseInsensitiveString(String);

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
        for c in self.0.as_bytes() {
            c.to_ascii_lowercase().hash(state)
        }
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
        CaseInsensitiveString { 0: s }
    }
}

impl AsRef<str> for CaseInsensitiveString {
    #[inline]
    fn as_ref(&self) -> &str {
        &self.0
    }
}

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
    pub configuration: Configuration,
    /// contains all visited URL.
    links_visited: Box<HashSet<CaseInsensitiveString>>,
    /// contains page visited
    pages: Option<Vec<Page>>,
    /// callback when a link is found.
    pub on_link_find_callback: fn(String) -> String,
    /// Robot.txt parser holder.
    robot_file_parser: Option<RobotFileParser>,
    /// the base root domain of the crawl
    domain: String,
}

type Message = HashSet<CaseInsensitiveString>;

impl Website {
    /// Initialize Website object with a start link to crawl.
    pub fn new(domain: &str) -> Self {
        let domain = if domain.ends_with('/') {
            domain.into()
        } else {
            string_concat!(domain, "/")
        };

        Self {
            configuration: Configuration::new(),
            links_visited: Box::new(HashSet::new()),
            pages: None,
            robot_file_parser: None,
            on_link_find_callback: |s| s,
            domain,
        }
    }

    /// return `true` if URL:
    ///
    /// - is not already crawled
    /// - is not blacklisted
    /// - is not forbidden in robot.txt file (if parameter is defined)
    pub fn is_allowed(&self, link: &CaseInsensitiveString) -> bool {
        if self.links_visited.contains(link)
            || contains(&self.configuration.blacklist_url, &link.0)
            || self.configuration.respect_robots_txt && !self.is_allowed_robots(&link.0)
        {
            return false;
        }

        true
    }

    /// return `true` if URL:
    ///
    /// - is not forbidden in robot.txt file (if parameter is defined)
    pub fn is_allowed_robots(&self, link: &str) -> bool {
        if self.configuration.respect_robots_txt {
            let robot_file_parser = self.robot_file_parser.as_ref().unwrap(); // unwrap will always return

            robot_file_parser.can_fetch("*", &link)
        } else {
            true
        }
    }

    /// page getter
    pub fn get_pages(&self) -> Vec<Page> {
        if !self.pages.is_none() {
            self.pages.as_ref().unwrap().clone()
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
            let mut robot_file_parser: RobotFileParser = match &self.robot_file_parser {
                Some(parser) => parser.to_owned(),
                _ => {
                    let mut robot_file_parser = RobotFileParser::new();
                    robot_file_parser.user_agent = self.configuration.user_agent.to_owned();

                    robot_file_parser
                }
            };

            // get the latest robots todo determine time elaspe
            if robot_file_parser.mtime() == 0 || robot_file_parser.mtime() >= 4000 {
                robot_file_parser.read(&client, &self.domain).await;
                self.configuration.delay = robot_file_parser
                    .get_crawl_delay(&robot_file_parser.user_agent) // returns the crawl delay in seconds
                    .unwrap_or_else(|| self.get_delay())
                    .as_millis() as u64;
            }

            self.robot_file_parser = Some(robot_file_parser);
        }

        client
    }

    /// configure the user agent for the request
    fn configure_agent(&mut self) {
        if self.configuration.user_agent.is_empty() {
            self.configuration.user_agent = get_ua();
        }
    }

    /// configure http client
    fn configure_http_client(&mut self) -> Client {
        let mut headers = header::HeaderMap::new();
        headers.insert(CONNECTION, header::HeaderValue::from_static("keep-alive"));

        Client::builder()
            .default_headers(headers)
            .user_agent(&self.configuration.user_agent)
            .brotli(true)
            .gzip(true)
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

                if domain == url {
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
        self.configure_agent();
        let client = self.configure_http_client();

        (
            self.configure_robots_parser(client).await,
            self.configure_handler(),
        )
    }

    /// Start to crawl website with async conccurency
    pub async fn crawl(&mut self) {
        let (client, handle) = self.setup().await;

        self.crawl_concurrent(&client, handle).await;
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
    async fn crawl_concurrent(&mut self, client: &Client, handle: Arc<AtomicI8>) {
        let delay = self.configuration.delay;
        let on_link_find_callback = self.on_link_find_callback;
        let channel_buffer = self.configuration.channel_buffer as usize;
        let mut interval = tokio::time::interval(Duration::from_millis(10));
        let throttle = Duration::from_millis(delay);
        let selector: Arc<(Selector, String)> = Arc::new(get_page_selectors(
            &self.domain,
            self.configuration.subdomains,
            self.configuration.tld,
        ));
        let mut links: HashSet<CaseInsensitiveString> =
            HashSet::from([self.domain.to_owned().into()]);
        let mut new_links: HashSet<CaseInsensitiveString> = HashSet::new();

        // crawl while links exists
        loop {
            let (tx, mut rx): (Sender<Message>, Receiver<Message>) = channel(channel_buffer);
            let stream = tokio_stream::iter(&links).throttle(throttle);
            tokio::pin!(stream);

            while let Some(link) = stream.next().await {
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
                log("fetch", &link);
                let tx = tx.clone();
                let client = client.clone();
                let link = link.clone();
                let selector = selector.clone();

                task::yield_now().await;

                task::spawn(async move {
                    {
                        let link_result = on_link_find_callback(link.0);
                        task::yield_now().await;
                        let page = Page::new(&link_result, &client).await;
                        let page_links = page.links(&*selector);
                        task::yield_now().await;

                        if let Err(_) = tx.send(page_links).await {
                            log("receiver dropped", "");
                        }
                    }
                    task::yield_now().await;
                });
            }

            drop(tx);

            while let Some(msg) = rx.recv().await {
                new_links.extend(msg);
                task::yield_now().await;
            }

            links.clone_from(&(&new_links - &self.links_visited));

            task::yield_now().await;
            new_links.clear();

            if new_links.capacity() > channel_buffer {
                new_links.shrink_to_fit();
            }

            task::yield_now().await;
            if links.is_empty() {
                break;
            }
        }
    }

    /// Start to crawl website sequential
    async fn crawl_sequential(&mut self, client: &Client, handle: Arc<AtomicI8>) {
        let delay = self.configuration.delay;
        let delay_enabled = delay > 0;
        let on_link_find_callback = self.on_link_find_callback;
        let channel_buffer = self.configuration.channel_buffer as usize;
        let mut interval = tokio::time::interval(Duration::from_millis(10));
        let selectors = get_page_selectors(
            &self.domain,
            self.configuration.subdomains,
            self.configuration.tld,
        );
        let mut links: HashSet<CaseInsensitiveString> =
            HashSet::from([self.domain.to_owned().into()]);
        let mut new_links: HashSet<CaseInsensitiveString> = HashSet::new();

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
                log("fetch", link);
                self.links_visited.insert(link.clone());
                if delay_enabled {
                    sleep(Duration::from_millis(delay)).await;
                }
                let link = link.clone();
                let link_result = on_link_find_callback(link.0);
                let page = Page::new(&link_result, &client).await;
                let page_links = page.links(&selectors);
                task::yield_now().await;
                new_links.extend(page_links);
                task::yield_now().await;
            }

            links.clone_from(&(&new_links - &self.links_visited));
            new_links.clear();
            if new_links.capacity() > channel_buffer {
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
        self.pages = Some(Vec::new());
        let delay = self.configuration.delay;
        let on_link_find_callback = self.on_link_find_callback;
        let channel_buffer = self.configuration.channel_buffer as usize;
        let mut interval = tokio::time::interval(Duration::from_millis(10));
        let selectors: Arc<(Selector, String)> = Arc::new(get_page_selectors(
            &self.domain,
            self.configuration.subdomains,
            self.configuration.tld,
        ));
        let throttle = Duration::from_millis(delay);
        let mut links: HashSet<CaseInsensitiveString> =
            HashSet::from([self.domain.to_owned().into()]);
        let mut new_links: HashSet<CaseInsensitiveString> = HashSet::new();

        // crawl while links exists
        loop {
            let (tx, mut rx): (Sender<Page>, Receiver<Page>) = channel(channel_buffer);
            let stream = tokio_stream::iter(&links).throttle(throttle);
            tokio::pin!(stream);

            while let Some(link) = stream.next().await {
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
                log("fetch", &link);
                self.links_visited.insert(link.clone());

                let tx = tx.clone();
                let client = client.clone();
                let link = link.clone();

                task::spawn(async move {
                    {
                        let link_result = on_link_find_callback(link.0);
                        let page = Page::new(&link_result, &client).await;

                        if let Err(_) = tx.send(page).await {
                            log("receiver dropped", "");
                        }
                    }
                    task::yield_now().await;
                });
            }

            drop(tx);

            while let Some(msg) = rx.recv().await {
                let page_links = msg.links(&*selectors);
                task::yield_now().await;
                new_links.extend(page_links);
                task::yield_now().await;
                self.pages.as_mut().unwrap().push(msg);
                task::yield_now().await;
            }

            links.clone_from(&(&new_links - &self.links_visited));

            new_links.clear();

            if new_links.capacity() > channel_buffer {
                new_links.shrink_to_fit();
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
    website.configuration.delay = 250;
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
    website.on_link_find_callback = |s| {
        log("callback link target: {}", &s);
        s
    };
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
    website
        .configuration
        .blacklist_url
        .push("https://choosealicense.com/licenses/".to_string());
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
        .push("/choosealicense.com/".to_string());
    website.crawl().await;
    assert_eq!(website.links_visited.len(), 0);
}

#[test]
#[cfg(feature = "ua_generator")]
fn randomize_website_agent() {
    let mut website: Website = Website::new("https://choosealicense.com");
    website.configure_agent();

    assert_eq!(website.configuration.user_agent.is_empty(), false);
}

#[tokio::test]
async fn test_respect_robots_txt() {
    let mut website: Website = Website::new("https://stackoverflow.com");
    website.configuration.respect_robots_txt = true;
    website.configuration.user_agent = "*".into();

    let (client, _) = website.setup().await;
    website.configure_robots_parser(client).await;

    assert_eq!(website.configuration.delay, 0);

    assert!(!website.is_allowed(&"https://stackoverflow.com/posts/".into()));

    // test match for bing bot
    let mut website_second: Website = Website::new("https://www.mongodb.com");
    website_second.configuration.respect_robots_txt = true;
    website_second.configuration.user_agent = "bingbot".into();

    let (client_second, _) = website_second.setup().await;
    website_second.configure_robots_parser(client_second).await;

    assert_eq!(
        website_second.configuration.user_agent,
        website_second
            .robot_file_parser
            .as_ref()
            .unwrap()
            .user_agent
    );
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
