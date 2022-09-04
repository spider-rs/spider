use crate::black_list::contains;
use crate::configuration::{get_ua, Configuration};
use crate::packages::robotparser::RobotFileParser;
use crate::page::Page;
use crate::utils::log;
use hashbrown::HashSet;
use std::time::Duration;

use rayon::prelude::*;
use reqwest::header;
use reqwest::header::CONNECTION;
use reqwest::Client;
use tokio::sync::mpsc::{channel, Receiver, Sender};
use tokio::time::sleep;

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
    /// contains all non-visited URL.
    links: HashSet<String>,
    /// contains all visited URL.
    links_visited: HashSet<String>,
    /// contains page visited
    pages: Vec<Page>,
    /// callback when a link is found.
    pub on_link_find_callback: fn(String) -> String,
    /// Robot.txt parser holder.
    robot_file_parser: Option<RobotFileParser>,
}

type Message = HashSet<String>;

impl Website {
    /// Initialize Website object with a start link to crawl.
    pub fn new(domain: &str) -> Self {
        Self {
            configuration: Configuration::new(),
            links_visited: HashSet::new(),
            pages: Vec::new(),
            robot_file_parser: None,
            links: HashSet::from([format!("{}/", &domain)]),
            on_link_find_callback: |s| s,
        }
    }

    /// return `true` if URL:
    ///
    /// - is not already crawled
    /// - is not blacklisted
    /// - is not forbidden in robot.txt file (if parameter is defined)  
    pub fn is_allowed(&self, link: &String) -> bool {
        if self.links_visited.contains(link) {
            return false;
        }
        if contains(&self.configuration.blacklist_url, link) {
            return false;
        }
        if self.configuration.respect_robots_txt && !self.is_allowed_robots(link) {
            return false;
        }

        true
    }

    /// return `true` if URL:
    ///
    /// - is not forbidden in robot.txt file (if parameter is defined)  
    pub fn is_allowed_robots(&self, link: &String) -> bool {
        if self.configuration.respect_robots_txt {
            let robot_file_parser = self.robot_file_parser.as_ref().unwrap(); // unwrap will always return

            robot_file_parser.can_fetch("*", link)
        } else {
            true
        }
    }

    /// page getter
    pub fn get_pages(&self) -> Vec<Page> {
        if !self.pages.is_empty() {
            self.pages.clone()
        } else {
            self.links_visited
                .par_iter()
                .map(|l| Page::build(l, ""))
                .collect()
        }
    }

    /// links visited getter
    pub fn get_links(&self) -> &HashSet<String> {
        &self.links_visited
    }

    /// crawl delay getter
    fn get_delay(&self) -> Duration {
        Duration::from_millis(self.configuration.delay)
    }

    /// configure the robots parser on initial crawl attempt and run.
    pub async fn configure_robots_parser(&mut self, client: &Client) {
        if self.configuration.respect_robots_txt {
            let mut robot_file_parser: RobotFileParser = match &self.robot_file_parser {
                Some(parser) => parser.to_owned(),
                _ => {
                    let mut domain = String::from("");
                    // the first link upon initial config is always the domain
                    for links in self.links.iter() {
                        domain = links.clone();
                    }
                    let mut robot_file_parser =
                        RobotFileParser::new(&format!("{}robots.txt", &domain));
                    robot_file_parser.user_agent = self.configuration.user_agent.to_owned();

                    robot_file_parser
                }
            };

            // get the latest robots todo determine time elaspe
            if robot_file_parser.mtime() == 0 || robot_file_parser.mtime() >= 4000 {
                robot_file_parser.read(client).await;
                self.configuration.delay = robot_file_parser
                    .get_crawl_delay(&robot_file_parser.user_agent) // returns the crawl delay in seconds
                    .unwrap_or(self.get_delay())
                    .as_millis() as u64;
            }

            self.robot_file_parser = Some(robot_file_parser);
        }
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
            .build()
            .unwrap()
    }

    /// setup config for crawl
    pub async fn setup(&mut self) -> Client {
        self.configure_agent();
        let client = self.configure_http_client();
        self.configure_robots_parser(&client).await;

        client
    }

    /// Start to crawl website with async parallelization
    pub async fn crawl(&mut self) {
        let client = self.setup().await;

        self.crawl_concurrent(&client).await;
    }

    /// Start to crawl website in sync
    pub async fn crawl_sync(&mut self) {
        let client = self.setup().await;

        self.crawl_sequential(&client).await;
    }

    /// Start to scrape/download website with async parallelization
    pub async fn scrape(&mut self) {
        let client = self.setup().await;

        self.scrape_concurrent(&client).await;
    }

    /// Start to crawl website concurrently
    async fn crawl_concurrent(&mut self, client: &Client) {
        let delay = self.configuration.delay;
        let subdomains = self.configuration.subdomains;
        let tld = self.configuration.tld;
        let delay_enabled = delay > 0;
        let on_link_find_callback = self.on_link_find_callback;

        let mut start_url = String::new(); // base crawl index

        // crawl while links exists
        while !self.links.is_empty() {
            let (tx, mut rx): (Sender<Message>, Receiver<Message>) = channel(50);

            for link in self.links.iter() {
                if !self.is_allowed(link) {
                    continue;
                }
                log("fetch", link);
                if start_url.is_empty() {
                    start_url = link.clone();
                }
                self.links_visited.insert(link.into());

                let tx = tx.clone();
                let client = client.clone();
                let link = link.clone();

                tokio::spawn(async move {
                    if delay_enabled {
                        sleep(Duration::from_millis(delay)).await;
                    }
                    let link_result = on_link_find_callback(link);
                    let page = Page::new(&link_result, &client).await;
                    let links = page.links(subdomains, tld);

                    drop(client);
                    drop(link_result);
                    drop(page);

                    if let Err(_) = tx.send(links).await {
                        log("receiver dropped", "");
                    }
                });
            }

            drop(tx);

            let mut new_links: HashSet<String> = HashSet::new();

            while let Some(msg) = rx.recv().await {
                new_links.par_extend(msg);
            }

            self.links = &new_links - &self.links_visited;
        }

        self.links.insert(start_url);
    }

    /// Start to crawl website sequential
    async fn crawl_sequential(&mut self, client: &Client) {
        let delay = self.configuration.delay;
        let subdomains = self.configuration.subdomains;
        let tld = self.configuration.tld;
        let delay_enabled = delay > 0;
        let on_link_find_callback = self.on_link_find_callback;

        let mut start_url = String::new(); // base crawl index

        // crawl while links exists
        while !self.links.is_empty() {
            let mut new_links: HashSet<String> = HashSet::new();

            for link in self.links.iter() {
                if !self.is_allowed(link) {
                    continue;
                }
                log("fetch", link);
                if start_url.is_empty() {
                    start_url = link.clone();
                }
                self.links_visited.insert(link.into());
                if delay_enabled {
                    sleep(Duration::from_millis(delay)).await;
                }

                let link = link.clone();
                let link_result = on_link_find_callback(link);
                let page = Page::new(&link_result, &client).await;
                let links = page.links(subdomains, tld);

                new_links.par_extend(links);
            }

            self.links = &new_links - &self.links_visited;
        }

        self.links.insert(start_url);
    }

    /// Start to scape website concurrently and store html
    async fn scrape_concurrent(&mut self, client: &Client) {
        let delay = self.configuration.delay;
        let delay_enabled = delay > 0;
        let on_link_find_callback = self.on_link_find_callback;

        let mut start_url = String::new(); // base crawl index

        // crawl while links exists
        while !self.links.is_empty() {
            let (tx, mut rx): (Sender<Page>, Receiver<Page>) = channel(50);

            for link in self.links.iter() {
                if !self.is_allowed(link) {
                    continue;
                }
                log("fetch", link);
                if start_url.is_empty() {
                    start_url = link.clone();
                }
                self.links_visited.insert(link.into());

                let tx = tx.clone();
                let client = client.clone();
                let link = link.clone();

                tokio::spawn(async move {
                    if delay_enabled {
                        sleep(Duration::from_millis(delay)).await;
                    }
                    let link_result = on_link_find_callback(link);
                    let page = Page::new(&link_result, &client).await;

                    drop(client);
                    drop(link_result);

                    if let Err(_) = tx.send(page).await {
                        log("receiver dropped", "");
                        return;
                    }
                });
            }

            drop(tx);

            let mut new_links: HashSet<String> = HashSet::new();

            while let Some(msg) = rx.recv().await {
                let links = msg.links(self.configuration.subdomains, self.configuration.tld);
                new_links.par_extend(links);
                self.pages.push(msg);
            }

            self.links = &new_links - &self.links_visited;
        }

        self.links.insert(start_url);
    }
}

#[tokio::test]
async fn crawl() {
    let mut website: Website = Website::new("https://choosealicense.com");
    website.crawl().await;
    assert!(
        website
            .links_visited
            .contains(&"https://choosealicense.com/licenses/".to_string()),
        "{:?}",
        website.links_visited
    );
    // resets base link for crawling
    assert!(
        website
            .links
            .contains(&"https://choosealicense.com/".to_string()),
        "{:?}",
        website.links
    );
}

#[tokio::test]
async fn scrape() {
    let mut website: Website = Website::new("https://choosealicense.com");
    website.scrape().await;
    assert!(
        website
            .links_visited
            .contains(&"https://choosealicense.com/licenses/".to_string()),
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
            .contains(&"https://choosealicense.com/licenses/".to_string()),
        "{:?}",
        website.links_visited
    );
}

#[tokio::test]
async fn crawl_invalid() {
    let url = "https://w.com";
    let mut website: Website = Website::new(url);
    website.crawl().await;
    let mut uniq = HashSet::new();
    uniq.insert(format!("{}/", url.to_string())); // TODO: remove trailing slash mutate

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
            .contains(&"https://choosealicense.com/licenses/".to_string()),
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
            .contains(&"https://choosealicense.com/licenses/".to_string()),
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

    let client = website.setup().await;
    website.configure_robots_parser(&client).await;

    assert_eq!(website.configuration.delay, 250);

    assert!(!website.is_allowed(&"https://stackoverflow.com/posts/".to_string()));

    // test match for bing bot
    let mut website_second: Website = Website::new("https://www.mongodb.com");
    website_second.configuration.respect_robots_txt = true;
    website_second.configuration.user_agent = "bingbot".into();

    let client_second = website_second.setup().await;
    website_second.configure_robots_parser(&client_second).await;

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
    let client_third = website_third.setup().await;

    website_third.configure_robots_parser(&client_third).await;

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
            .contains(&"https://choosealicense.com/licenses/".to_string()),
        "{:?}",
        website.links_visited
    );
}

#[tokio::test]
async fn test_crawl_tld() {
    let mut website: Website = Website::new("https://choosealicense.com");
    website.configuration.subdomains = true;
    website.crawl().await;
    assert!(
        website
            .links_visited
            .contains(&"https://choosealicense.com/licenses/".to_string()),
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

    assert!(has_unique_elements(&website.links_visited));
}
