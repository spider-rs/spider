use page::Page;
use std::thread;
use std::thread::JoinHandle;

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
pub struct Website {
    /// this is a start URL given when instanciate with `new`
    domain: String,
    /// contains all non-visited URL
    links: Vec<String>,
    /// contains all visited URL
    links_visited: Vec<String>,
    /// contains page visited
    pages: Vec<Page>,
}

impl Website {
    /// Initialize Website object with a start link to scrawl.
    pub fn new(domain: &str) -> Self {
        // create home link
        let links: Vec<String> = vec![format!("{}/", domain)];

        Self {
            domain: domain.to_string(),
            links: links,
            links_visited: Vec::new(),
            pages: Vec::new(),
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
                if self.links_visited.contains(link) {
                    continue;
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
