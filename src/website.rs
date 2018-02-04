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

    pub fn crawl(&mut self) {
        let mut new_links: Vec<String> = Vec::new();

        for link in &self.links {

            if self.links_visited.contains(link) {
                continue;
            }

            let page = Page::new(link);
            let mut links_founded = page.links(&self.domain);

            new_links.append(&mut links_founded);

            self.pages.push(page);

            self.links_visited.push(link.to_string());
        }

        self.links.append(&mut new_links);
    }
}
