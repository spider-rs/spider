use scraper::{Html, Selector};
use reqwest;
use std::io::Read;

/// Represent a link who can be visited
#[derive(Debug)]

pub struct Page {
    url: String,
    html: Html,
}

impl Page {
    pub fn new(url: &str) -> Self {
        println!("[x] Fetch {}", url);

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

    pub fn links(&self, domain: &str) -> Vec<String> {
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
