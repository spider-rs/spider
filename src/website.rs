use page::Page;

/// Represent a website with many links to visit
#[derive(Debug)]
pub struct Website {
    domain: String,
    links: Vec<String>,
    links_visited: Vec<String>,
    pages: Vec<Page>,
}

impl Website {
    /// Initialize Website object with one link
    pub fn new(domain: &str) -> Self {
        // create home link
        let mut links: Vec<String> = Vec::new();
        links.push(format!("{}/", domain));

        Self {
            domain: domain.to_string(),
            links: links,
            links_visited: Vec::new(),
            pages: Vec::new(),
        }
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
