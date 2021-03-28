use reqwest;
use scraper::{Html, Selector};
use url::Url;

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
    /// Instanciate a new page and start to scrape it.
    pub fn new(url: &str, user_agent: &str) -> Self {
        let mut body = String::new();

        let client = reqwest::blocking::Client::builder()
            .user_agent(user_agent)
            .build()
            .unwrap();

        match client.get(url).send() {
            Ok(res) if res.status() == reqwest::StatusCode::OK => match res.text() {
                Ok(text) => body = text,
                Err(e) => eprintln!("[error] {}: {}", url, e),
            },
            Ok(_) => (),
            Err(e) => eprintln!("[error] {}: {}", url, e),
        }

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

        let html = self.get_html();
        let anchors = html
            .select(&selector)
            .filter(|a| a.value().attrs().any(|attr| attr.0 == "href"));

        for anchor in anchors {
            match anchor.value().attr("href") {
                Some(href) => {
                    let abs_path = self.abs_path(href);

                    if abs_path.as_str().starts_with(domain) {
                        urls.push(format!("{}", abs_path));
                    }
                }
                None => (),
            };
        }

        urls
    }

    fn abs_path(&self, href: &str) -> Url {
        let base = Url::parse(&self.url.to_string()).expect("Invalid page URL");
        let mut joined = base.join(href).unwrap_or(base);

        joined.set_fragment(None);

        joined
    }
}

#[test]
fn parse_links() {
    let page: Page = Page::new("https://choosealicense.com/", "spider/1.1.2");

    assert!(
        page.links("https://choosealicense.com")
            .contains(&"https://choosealicense.com/about/".to_string()),
        format!(
            "Could not find {}. Theses URLs was found {:?}",
            page.url,
            page.links("https://choosealicense.com")
        )
    );
}
