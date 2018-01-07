use std::io::Read;
use scraper::{Html, Selector};
use reqwest;

/// Represent a website with many links to visit
#[derive(Debug)]
pub struct Website {
    domain: String,
    links: Vec<Link>,
}

impl Website {
    /// Initialize Website object with one link
    pub fn new(domain: &str) -> Self {
        // create home link
        let mut links: Vec<Link> = Vec::new();
        links.push(Link::new(domain, "/"));

        Self {
            domain: domain.to_string(),
            links: links,
        }
    }

    pub fn crawl(&mut self) {

        let mut new_links: Vec<Link> = Vec::new();

        for mut link in &self.links {
            let body: Html = link.visit();
            let selector = Selector::parse("a").unwrap();

            for element in body.select(&selector) {

                match element.value().attr("href") {
                    Some(href) => {

                        // Keep only links for this domains
                        match href.find('/') {
                            Some(0) => new_links.push(Link::new(&self.domain, href)),
                            Some(_) => (),
                            None => (),
                        };
                    }
                    None => (),
                };
            }

        }

        self.links.append(&mut new_links);
    }
}

/// Represent a link who can be visited
#[derive(Debug)]
struct Link {
    url: String,
    visited: bool,
}

impl Link {
    fn new(domain: &str, url: &str) -> Self {
        Self {
            url: format!("{}{}", domain, url),
            visited: false,
        }
    }

    /// Launch an HTTP GET query to te given URL & parse body response content
    fn visit(&self) -> Html {
        let mut res = reqwest::get(&self.url).unwrap();
        let mut body = String::new();
        res.read_to_string(&mut body).unwrap();

        // todo: set link visited

        Html::parse_document(&body)
    }
}
