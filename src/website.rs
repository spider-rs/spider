use std::io::Read;
use scraper::{Html, Selector};
use reqwest;

/// Represent a website with many links to visit
#[derive(Debug)]
pub struct Website {
    domain: String,
    links: Vec<String>,
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
            pages: Vec::new(),
        }
    }

    pub fn crawl(&mut self) {
        let mut new_links: Vec<String> = Vec::new();

        for link in &self.links {
            let page = Page::new(link);
            let mut links_founded = page.links(&self.domain);

            new_links.append(&mut links_founded);

            self.pages.push(page);
        }

        self.links = new_links;
    }
}

/// Represent a link who can be visited
#[derive(Debug)]
struct Page {
    url: String,
    html: Html,
}

impl Page {
    fn new(url: &str) -> Self {
        let html = Self::visit(url);

        Self {
            url: url.to_string(),
            html: html,
        }
    }

    /// Launch an HTTP GET query to te given URL & parse body response content
    fn visit(url: &str) -> Html {
        let mut res = reqwest::get(url).unwrap();
        let mut body = String::new();
        res.read_to_string(&mut body).unwrap();

        Html::parse_document(&body)
    }

    fn links(&self, domain: &str) -> Vec<String> {
        let mut urls: Vec<String> = Vec::new();

        let selector = Selector::parse("a").unwrap();

        for element in self.html.select(&selector) {

            match element.value().attr("href") {
                Some(href) => {

                    // Keep only links for this domains
                    match href.find('/') {
                        Some(0) => urls.push(format!("{}{}", domain, href)),
                        Some(_) => (),
                        None => (),
                    };
                }
                None => (),
            };
        }

        urls
    }
}
