use std::io::Read;
use scraper::{Html, Selector};
use reqwest;

#[derive(Debug)]
pub struct Website {
    domain: String,
    links: Vec<String>,
}

impl Website {
    pub fn new(domain: &str) -> Self {
        Self {
            domain: domain.to_string(),
            links: Vec::new(),
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
                        Some(0) => {
                            self.links.push(
                                format!("{}{}", self.domain, href.to_string()),
                            )
                        }
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
