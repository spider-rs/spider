use scraper::{Html, Selector};
use reqwest;
use std::io::Read;

/// Represent a page visited. This page contains HTML scraped with [scraper](https://crates.io/crates/scraper).
///
/// **TODO**: store some usefull informations like code status, response time, headers, etc..
#[derive(Debug, Clone)]
pub struct Page {
    /// URL of this page
    url: String,
    /// HTML parsed with [scraper](https://crates.io/crates/scraper) lib
    html: Html,
}

impl Page {
    /// Instanciate a new page a start to scrape it.
    pub fn new(url: &str) -> Self {
        let html = Self::visit(url);

        Self {
            url: url.to_string(),
            html: html,
        }
    }

    /// URL getter
    pub fn get_url(&self) -> String {
        self.url.clone()
    }

    /// HTML getter
    pub fn get_html(&self) -> Html {
        self.html.clone()
    }

    /// Launch an HTTP GET query to te given URL & parse body response content
    fn visit(url: &str) -> Html {
        // TODO: handle uwrap here
        let mut res = reqwest::get(url).unwrap();
        let mut body = String::new();
        res.read_to_string(&mut body).unwrap();

        Html::parse_document(&body)
    }

    /// Find all href links and return them
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
