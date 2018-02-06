use page::Page;

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
            let mut new_links: Vec<String> = Vec::new();
            for link in &self.links {
                // verify that URL was not already scrawled
                if self.links_visited.contains(link) {
                    continue;
                }

                // scrape page & found links
                let page = Page::new(link);
                for link_founded in page.links(&self.domain) {
                    // add only links not already vistited
                    if !self.links_visited.contains(&link_founded) {
                        new_links.push(link_founded);
                    }
                }
                // add page to scrawled pages

                self.pages.push(page);
                self.links_visited.push(link.to_string());
            }

            self.links = new_links.clone();
        }
    }
}

#[test]
fn crawl() {
    let mut website: Website = Website::new("http://rousseau-alexandre.fr");
    website.crawl();
    assert!(website.links_visited.contains(
        &"http://rousseau-alexandre.fr/blog".to_string(),
    ));
}
