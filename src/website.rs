use page::Page;
use std::thread;
use std::thread::JoinHandle;
use configuration::Configuration;
use robotparser::RobotFileParser;

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
    robot_file_parser : RobotFileParser<'a>

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
            links: links,
            links_visited: Vec::new(),
            pages: Vec::new(),
            robot_file_parser: parser
        }
    }

    /// page getter
    pub fn get_pages(&self) -> Vec<Page> {
        self.pages.clone()
    }

    /// Start to crawl website
    pub fn crawl(&mut self) {
        // scrawl while links exists
        while self.links.len() > 0 {
            let mut workers: Vec<JoinHandle<Page>> = Vec::new();
            let mut new_links: Vec<String> = Vec::new();
            for link in &self.links {
                // extends visibility
                let thread_link: String = link.to_string();

                // verify that URL was not already scrawled
                if !self.is_allowed(link) {
                    continue;
                }

                if self.configuration.verbose {
                    println!("- fetch {}", link);
                }

                workers.push(thread::spawn(move || Page::new(&thread_link)));
            }

            for worker in workers {
                match worker.join() {
                    Ok(page) => {
                        // get links founded on
                        for link_founded in page.links(&self.domain) {
                            // add only links not already vistited
                            if !self.links_visited.contains(&link_founded) {
                                new_links.push(link_founded);
                            }
                        }
                        // add page to scrawled pages

                        self.links_visited.push(page.get_url());
                        self.pages.push(page);

                    }
                    Err(_) => (),
                }
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
            let path : String = str::replace(link, &self.domain, "");
            if !self.robot_file_parser.can_fetch("*", &path) {
                return false;
            }
        }

        return true;
    }
}


#[test]
fn crawl() {
    let mut website: Website = Website::new("https://choosealicense.com");
    website.crawl();
    assert!(
        website.links_visited.contains(
            &"https://choosealicense.com/licenses/".to_string()),
        format!("{:?}", website.links_visited)
    );
}

#[test]
fn not_crawl_blacklist() {
    let mut website: Website = Website::new("https://choosealicense.com");
    website.configuration.blacklist_url.push("https://choosealicense.com/licenses/".to_string());
    website.crawl();
    assert!(
        !website.links_visited.contains(
            &"https://choosealicense.com/licenses/".to_string()),
        format!("{:?}", website.links_visited)
    );
}

#[test]
fn test_get_robots_txt() {
    let mut website: Website = Website::new("https://stackoverflow.com");
    website.configuration.respect_robots_txt = true;
    assert!(!website.is_allowed(&"https://stackoverflow.com/posts/".to_string()));
}