use std::io::Read;
use scraper::{Html, Selector};
use reqwest;

#[derive(Debug)]
pub struct Website {
    domain: String,
    links: Vec<Link>,
}

impl Website {
    pub fn new(domain: &str) -> Self {

        let links Vec<Link>= Vec::new(){
            Link::new(domain, "/")
        }; 


        Self {
            domain: domain.to_string(),
            links: links,
        }
    }

    pub fn crawl(&mut self) {
        let body: Html = Self::get(&self.domain);
        let selector = Selector::parse("a").unwrap();

        for element in body.select(&selector) {

            match element.value().attr("href") {
                Some(href) => {

                    // Keep only links for this domains
                    match href.find('/') {
                        Some(0) => self.links.push(Link::new(&self.domain, href)),
                        Some(_) => (),
                        None => (),
                    };
                }
                None => (),
            };
        }
    }

    /// Launch an HTTP GET query to te given URL & parse body response content
    fn get(url: &str) -> Html {
        let mut res = reqwest::get(url).unwrap();
        let mut body = String::new();
        res.read_to_string(&mut body).unwrap();

        Html::parse_document(&body)
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
}
