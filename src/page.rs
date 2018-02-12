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
    html: String,
}

impl Page {
    /// Instanciate a new page a start to scrape it.
    pub fn new(url: &str) -> Self {
        // TODO: handle uwrap here
        let mut res = reqwest::get(url).unwrap();
        let mut body = String::new();
        res.read_to_string(&mut body).unwrap();

        Self {
            url: url.to_string(),
            html: body,
        }
    }

    /// Instanciate a new page without scraping it (used for testing purposes)
    pub fn build(url: &str, html: &str) -> Self {
        Self {
            url: url.to_string(),
            html: html.to_string(),
        }
    }

    /// URL getter
    pub fn get_url(&self) -> String {
        self.url.clone()
    }

    /// HTML parser
    pub fn get_html(&self) -> Html {
        Html::parse_document(&self.html)
    }

    pub fn get_plain_html(&self) -> String {
        self.html.clone()
    }

    /// Find all href links and return them
    pub fn links(&self, domain: &str) -> Vec<String> {
        let mut urls: Vec<String> = Vec::new();
        let selector = Selector::parse("a").unwrap();

        for element in self.get_html().select(&selector) {
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

#[test]
fn parse_links() {
    let page : Page = Page::new("https://choosealicense.com/");

    assert!(
        page.links("https://choosealicense.com").contains(
            &"https://choosealicense.com/about/".to_string()),
        format!(
            "Could not find {}. Theses URLs was found {:?}", 
            page.url,
            page.links("https://choosealicense.com")
        )
    );
}