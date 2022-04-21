use scraper::{Html, Selector};
use url::Url;
use crate::utils::{fetch_page_html, Client};

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
    pub fn new(url: &str, client: &Client) -> Self {
        let html = fetch_page_html(&url, &client);

        Page::build(url, &html)
    }

    /// Instanciate a new page without scraping it (used for testing purposes)
    pub fn build(url: &str, html: &str) -> Self {
        Self {
            url: url.to_string(),
            html: html.to_string(),
        }
    }

    /// URL getter
    pub fn get_url(&self) -> &String {
        &self.url
    }

    /// HTML parser
    pub fn get_html(&self) -> Html {
        Html::parse_document(&self.html)
    }

    /// html selector for valid web pages for domain
    pub fn get_page_selectors(&self, domain: &str) -> Selector {
        let media_ignore_selector = r#":not([href$=".png"]):not([href$=".jpg"]):not([href$=".mp4"]):not([href$=".mp3"]):not([href$=".gif"]):not([href$=".pdf"])"#;
        let relative_selector = &format!(
            r#"a[href^="/"]{}"#,
            media_ignore_selector,
        );
        let absolute_selector = &format!(
            r#"a[href^="{}"]{}"#,
            domain,
            media_ignore_selector,
        );
        let static_html_selector = &format!(
            r#"{} [href$=".html"], {} [href$=".html"]"#,
            relative_selector,
            absolute_selector,
        );
        Selector::parse(&format!(
            "{},{},{}",
            relative_selector,
            absolute_selector,
            static_html_selector
        ))
        .unwrap()
    }

    /// Find all href links and return them: this also clears the set html for the page
    pub fn links(&mut self) -> Vec<String> {
        let selector = self.get_page_selectors(&self.url);
        let html = self.get_html();

        self.html.clear();
        
        html.select(&selector)
            .map(|a| self.abs_path(a.value().attr("href").unwrap_or_default()).to_string())
            .collect()
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
    let client = Client::builder()
        .user_agent("spider/1.1.2")
        .build()
        .unwrap();

    let link_result = "https://choosealicense.com/";
    let mut page: Page = Page::new(&link_result, &client);

    let links = page.links();

    assert!(
        links
            .contains(&"https://choosealicense.com/about/".to_string()),
        "Could not find {}. Theses URLs was found {:?}",
        page.url,
        &links
    );
}

#[test]
fn test_abs_path() {
    let client = Client::builder()
        .user_agent("spider/1.1.2")
        .build()
        .unwrap();
    let link_result = "https://choosealicense.com/";
    let page: Page = Page::new(&link_result, &client);

    assert_eq!(
        page.abs_path("/page"),
        Url::parse("https://choosealicense.com/page").unwrap()
    );
    assert_eq!(
        page.abs_path("/page?query=keyword"),
        Url::parse("https://choosealicense.com/page?query=keyword").unwrap()
    );
    assert_eq!(
        page.abs_path("/page#hash"),
        Url::parse("https://choosealicense.com/page").unwrap()
    );
    assert_eq!(
        page.abs_path("/page?query=keyword#hash"),
        Url::parse("https://choosealicense.com/page?query=keyword").unwrap()
    );
    assert_eq!(
        page.abs_path("#hash"),
        Url::parse("https://choosealicense.com/").unwrap()
    );
    assert_eq!(
        page.abs_path("tel://+212 3456"),
        Url::parse("https://choosealicense.com/").unwrap()
    );
}
