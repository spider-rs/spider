use crate::configuration::Configuration;
use crate::page::Page;
use rayon::ThreadPoolBuilder;
use robotparser_fork::RobotFileParser;

use std::collections::HashSet;
use std::{sync, thread, time::Duration};
use crate::utils::{fetch_page_html, Client};

/// Represent a website to scrawl. To start crawling, instanciate a new `struct` using
/// <pre>
/// let mut localhost = Website::new("http://example.com");
/// localhost.crawl();
/// </pre>
/// `Website` will be filled with `Pages` when crawled. To get them, just use
/// <pre>
/// for page in localhost.get_pages() {
///     // do something
/// }
/// </pre>
#[derive(Debug)]
pub struct Website<'a> {
    // configuration properies
    pub configuration: Configuration,
    /// this is a start URL given when instanciate with `new`
    domain: String,
    /// contains all non-visited URL
    links: HashSet<String>,
    /// contains all visited URL
    links_visited: HashSet<String>,
    /// contains page visited
    pages: Vec<Page>,
    /// callback when a link is found
    pub on_link_find_callback: fn(String) -> String,
    /// Robot.txt parser holder
    robot_file_parser: RobotFileParser<'a>,
    // configured the robots parser
    configured_robots_parser: bool,
    // fetch client
    client: Client,
}

impl<'a> Website<'a> {
    /// Initialize Website object with a start link to scrawl.
    pub fn new(domain: &str) -> Self {
        // create home link
        let links: HashSet<String> = vec![format!("{}/", domain)].into_iter().collect();

        Self {
            configuration: Configuration::new(),
            configured_robots_parser: false,
            domain: domain.to_string(),
            links,
            links_visited: HashSet::new(),
            pages: Vec::new(),
            robot_file_parser: RobotFileParser::new(&format!("{}/robots.txt", domain)), // TODO: lazy establish
            on_link_find_callback: |s| s,
            client: Client::new()
        }
    }

    /// page getter
    pub fn get_pages(&self) -> &Vec<Page> {
        &self.pages
    }
    
    /// configure the robots parser on initial crawl attempt and run
    pub fn configure_robots_parser(&mut self) {
        if self.configuration.respect_robots_txt && !self.configured_robots_parser {
            self.configured_robots_parser = true;
            self.robot_file_parser.user_agent = self.configuration.user_agent.to_string();
            self.robot_file_parser.read();

            // returns the crawl delay in seconds
            let ms = self.robot_file_parser.get_crawl_delay(&self.configuration.user_agent)
                .unwrap_or_else(|| Duration::from_millis(self.configuration.delay))
                .as_millis();

            self.configuration.delay = ms as u64;
        }
    }
    
    /// Start to crawl website
    pub fn crawl(&mut self) {
        self.configure_robots_parser();
        let delay = Duration::from_millis(self.configuration.delay);
        let user_agent = self.configuration.user_agent;
        let pool = ThreadPoolBuilder::new()
            .num_threads(self.configuration.concurrency)
            .build()
            .expect("Failed building thread pool.");
        self.client = Client::builder()
            .user_agent(user_agent)
            .pool_max_idle_per_host(0)
            .build()
            .expect("Failed building client.");

        // crawl while links exists
        while !self.links.is_empty() {
            let mut new_links: HashSet<String> = HashSet::new();
            let (tx, rx) = sync::mpsc::channel();
            let on_link_find_callback = self.on_link_find_callback;

            self.links
                .iter()
                .filter(|link| self.is_allowed(link))
                .for_each(|link| {
                    let thread_link: String = link.to_string();

                    if self.configuration.verbose {
                        println!("- fetch {}", link);
                    }

                    let tx = tx.clone();
                    let cx = self.client.clone();

                    pool.spawn(move || {
                        let link_result = on_link_find_callback(thread_link);
                        let html = fetch_page_html(&link_result, &cx).unwrap_or("".to_string());
                        tx.send(Page::new(&link_result, &html)).unwrap();
                        thread::sleep(delay);
                    });
                });

            drop(tx);

            rx.into_iter().for_each(|page| {
                let url = page.get_url();
                if self.configuration.verbose {
                    println!("- parse {}", &url);
                }

                new_links.extend(page.links(&self.domain));

                self.links_visited.insert(String::from(url));
                self.pages.push(page);
            });

            self.links = new_links;
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

        if self.configuration.blacklist_url.contains(link) {
            return false;
        }

        if self.configuration.respect_robots_txt {
            let path: String = str::replace(link, &self.domain, "");
            if !self.robot_file_parser.can_fetch("*", &path) {
                return false;
            }
        }

        true
    }
}

impl<'a> Drop for Website<'a> {
  fn drop(&mut self)  {}  
} 

#[test]
fn crawl() {
    let mut website: Website = Website::new("https://choosealicense.com");
    website.crawl();
    assert!(
        website
            .links_visited
            .contains(&"https://choosealicense.com/licenses/".to_string()),
        "{:?}", website.links_visited
    );
}

#[test]
fn crawl_invalid() {
    let url = "https://w.com";
    let mut website: Website = Website::new(url);
    website.crawl();
    let mut uniq = HashSet::new();
    uniq.insert(format!("{}/", url.to_string())); // TODO: remove trailing slash mutate

    assert_eq!(website.links_visited, uniq); // only the target url should exist
}

#[test]
fn crawl_link_callback() {
    let mut website: Website = Website::new("https://choosealicense.com");
    website.on_link_find_callback = |s| { 
        println!("callback link target: {}", s); 
        s 
    };
    website.crawl();
    assert!(
        website
            .links_visited
            .contains(&"https://choosealicense.com/licenses/".to_string()),
        "{:?}", website.links_visited
    );
}

#[test]
fn not_crawl_blacklist() {
    let mut website: Website = Website::new("https://choosealicense.com");
    website
        .configuration
        .blacklist_url
        .push("https://choosealicense.com/licenses/".to_string());
    website.crawl();
    assert!(
        !website
            .links_visited
            .contains(&"https://choosealicense.com/licenses/".to_string()),
        "{:?}", website.links_visited
    );
}

#[test]
fn test_respect_robots_txt() {
    let mut website: Website = Website::new("https://stackoverflow.com");
    website.configuration.respect_robots_txt = true;
    assert_eq!(website.configuration.delay, 250);
    assert!(!website.is_allowed(&"https://stackoverflow.com/posts/".to_string()));

    // test match for bing bot
    let mut website_second: Website = Website::new("https://www.mongodb.com");
    website_second.configuration.respect_robots_txt = true;
    website_second.configuration.user_agent = "bingbot";
    website_second.configure_robots_parser();
    assert_eq!(website_second.configuration.user_agent, website_second.robot_file_parser.user_agent);
    assert_eq!(website_second.configuration.delay, 60000); // should equal one minute in ms

    // test crawl delay with wildcard agent [DOES not work when using set agent]
    let mut website_third: Website = Website::new("https://www.mongodb.com");
    website_third.configuration.respect_robots_txt = true;
    website_third.configure_robots_parser();

    assert_eq!(website_third.configuration.delay, 10000); // should equal 10 seconds in ms
}

#[test]
fn test_link_duplicates() {
    fn has_unique_elements<T>(iter: T) -> bool
    where
        T: IntoIterator,
        T::Item: Eq + std::hash::Hash,
    {
        let mut uniq = HashSet::new();
        iter.into_iter().all(move |x| uniq.insert(x))
    }

    let mut website: Website = Website::new("http://0.0.0.0:8000");
    website.crawl();

    assert!(has_unique_elements(&website.links_visited));
}