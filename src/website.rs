use configuration::Configuration;
use page::Page;
use rayon::ThreadPoolBuilder;
use robotparser::RobotFileParser;
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
#[derive(Debug)]
pub struct Website<'a> {
    // configuration properies
    pub configuration: Configuration,
    /// this is a start URL given when instanciate with `new`
    domain: String,
    /// contains all non-visited URL
    links: Vec<String>,
    /// contains all visited URL
    links_visited: Vec<String>,
    /// contains page visited
    pages: Vec<Page>,
    /// Robot.txt parser holder
    robot_file_parser: RobotFileParser<'a>,
}

impl<'a> Website<'a> {
    /// Initialize Website object with a start link to scrawl.
    pub fn new(domain: &str) -> Self {
        // create home link
        let links: Vec<String> = vec![format!("{}/", domain)];
        // robots.txt parser
        let robot_txt_url = &format!("{}/robots.txt", domain);
        let parser = RobotFileParser::new(robot_txt_url);
        parser.read();

        Self {
            configuration: Configuration::new(),
            domain: domain.to_string(),
            links,
            links_visited: Vec::new(),
            pages: Vec::new(),
            robot_file_parser: parser,
        }
    }

    /// page getter
    pub fn get_pages(&self) -> Vec<Page> {
        self.pages.clone()
    }

    /// Start to crawl website
    pub fn crawl(&mut self) {
        let pool = ThreadPoolBuilder::new()
            .num_threads(self.configuration.concurrency)
            .build()
            .expect("Failed building thread pool.");

        // crawl while links exists
        while !self.links.is_empty() {
            let mut new_links: Vec<String> = Vec::new();
            let user_agent = self.configuration.user_agent;
            let delay = time::Duration::from_millis(self.configuration.delay);
            let (tx, rx) = sync::mpsc::channel();

            for link in &self.links {
                // extends visibility
                let thread_link: String = link.to_string();

                // verify that URL was not already crawled
                if !self.is_allowed(link) {
                    continue;
                }

                if self.configuration.verbose {
                    println!("- fetch {}", link);
                }

                let tx = tx.clone();

                pool.spawn(move || {
                    tx.send(Page::new(&thread_link, user_agent)).unwrap();
                    thread::sleep(delay);
                });
            }

            drop(tx);

            for page in rx {
                for link_found in page.links(&self.domain) {
                    // add only links not already vistited
                    if !self.links_visited.contains(&link_found) {
                        new_links.push(link_found);
                    }
                }

                // add page to crawled pages
                self.links_visited.push(page.get_url());
                self.pages.push(page);
            }

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
}

#[test]
fn crawl() {
    let mut website: Website = Website::new("https://choosealicense.com");
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
        format!("{:?}", website.links_visited)
    );
}

#[test]
fn test_get_robots_txt() {
    let mut website: Website = Website::new("https://stackoverflow.com");
    website.configuration.respect_robots_txt = true;
    assert!(!website.is_allowed(&"https://stackoverflow.com/posts/".to_string()));
}
