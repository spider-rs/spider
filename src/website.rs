use crate::configuration::Configuration;
use crate::page::Page;
use rayon::ThreadPoolBuilder;
use regex::Regex;
use robotparser::RobotFileParser;
use std::collections::{HashMap, HashSet};
use std::{sync, thread, time};

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
//#[derive(Debug)]
pub struct Website<'a, F: Fn(&Page)> {
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
    /// Robot.txt parser holder
    robot_file_parser: RobotFileParser<'a>,
    /// Parsers to run against matched routes
    parsers: HashMap<String, F>,
}

impl<'a, F: Fn(&Page)> Website<'a, F> {
    /// Initialize Website object with a start link to scrawl.
    pub fn new(domain: &str) -> Self {
        // create home link
        let links: HashSet<String> = vec![format!("{}/", domain)].into_iter().collect();
        // robots.txt parser
        let robot_txt_url = &format!("{}/robots.txt", domain);
        let parser = RobotFileParser::new(robot_txt_url);
        parser.read();

        Self {
            configuration: Configuration::new(),
            domain: domain.to_string(),
            links,
            links_visited: HashSet::new(),
            pages: Vec::new(),
            robot_file_parser: parser,
            parsers: HashMap::new(),
        }
    }

    /// page getter
    pub fn get_pages(&self) -> Vec<Page> {
        self.pages.clone()
    }

    /// Start to crawl website
    pub fn crawl(&mut self) {
        let delay = time::Duration::from_millis(self.configuration.delay);
        let user_agent = self.configuration.user_agent;
        let pool = ThreadPoolBuilder::new()
            .num_threads(self.configuration.concurrency)
            .build()
            .expect("Failed building thread pool.");

        // crawl while links exists
        while !self.links.is_empty() {
            let mut new_links: HashSet<String> = HashSet::new();
            let (tx, rx) = sync::mpsc::channel();

            self.links
                .iter()
                .filter(|link| self.is_allowed(link))
                .for_each(|link| {
                    let thread_link: String = link.to_string();

                    if self.configuration.verbose {
                        println!("- fetch {}", link);
                    }

                    let tx = tx.clone();

                    pool.spawn(move || {
                        tx.send(Page::new(&thread_link, user_agent)).unwrap();
                        thread::sleep(delay);
                    });
                });

            drop(tx);

            rx.into_iter().for_each(|page| {
                let page_url = page.get_url();

                if self.configuration.verbose {
                    println!("- parse {}", page_url);
                }

                for (route, parser) in &self.parsers {
                    let routex = Regex::new(&route).unwrap();
                    if routex.is_match(&page_url) {
                        parser(&page);
                    }
                }

                new_links.extend(page.links(&self.domain));
                self.links_visited.insert(page.get_url());
                self.pages.push(page);
            });

            self.links = new_links.clone();
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

    pub fn when(&mut self, route: String, parser: F) {
        self.parsers.insert(route, parser);
    }
}

#[test]
fn crawl() {
    let mut website: Website<F> = Website::new("https://choosealicense.com");
    website.crawl();
    assert!(
        website
            .links_visited
            .contains(&"https://choosealicense.com/licenses/".to_string()),
        format!("{:?}", website.links_visited)
    );
}

#[test]
fn not_crawl_blacklist() {
    let mut website: Website<F> = Website::new("https://choosealicense.com");
    website
        .configuration
        .blacklist_url
        .push("https://choosealicense.com/licenses/".to_string());
    website.crawl();
    assert!(
        !website
            .links_visited
            .contains(&"https://choosealicense.com/licenses/".to_string()),
        format!("{:?}", website.links_visited)
    );
}

#[test]
fn test_get_robots_txt() {
    let mut website: Website<F> = Website::new("https://stackoverflow.com");
    website.configuration.respect_robots_txt = true;
    assert!(!website.is_allowed(&"https://stackoverflow.com/posts/".to_string()));
}

#[test]
fn test_link_duplicates() {
    fn has_unique_elements<T>(iter: T) -> bool
    where
        T: IntoIterator,
        T::Item: Eq + std::hash::Hash,
    {
        let mut uniq = std::collections::HashSet::new();
        iter.into_iter().all(move |x| uniq.insert(x))
    }

    let mut website: Website<F> = Website::new("http://0.0.0.0:8000");
    website.crawl();

    assert!(has_unique_elements(website.links_visited));
}
