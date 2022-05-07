use scraper::{Html, Selector};
use url::Url;
use crate::utils::{fetch_page_html};
use reqwest::blocking::{Client};
use hashbrown::HashSet;

/// Represent a page visited. This page contains HTML scraped with [scraper](https://crates.io/crates/scraper).
#[derive(Debug, Clone)]
pub struct Page {
    /// URL of this page.
    url: String,
    /// HTML parsed with [scraper](https://crates.io/crates/scraper) lib. The html is cleared when crawling concurrently before sending to main thread.
    html: String,
    /// Base absolute url for domain.
    base: Url
}

/// Macro to get all media selectors that should be ignored for link gathering.
macro_rules! media_ignore_selector {
    () => ( 
        concat!(
        r#":not([href$=".png"]):not([href$=".jpg"]):not([href$=".jpeg"]):not([href$=".svg"]):not([href$=".webp"]):not([href$=".gif"]):not([href$=".pdf"]):not([href$=".tiff"])"#, // images
        r#":not([href$=".mp3"]):not([href$=".mp4"]):not([href$=".ogg"]):not([href$=".webm"])"#, // videos
        r#":not([href$=".git"]):not([href$=".json"]):not([href$=".xml"]):not([href$=".css"]):not([href$=".md"]):not([href$=".txt"]):not([href$=".js"]):not([href$=".jsx"]):not([href$=".csv"])"#) // else
    )
}

lazy_static! {
    /// CSS query selector to ignore all resources that are not valid web pages.
    static ref MEDIA_IGNORE_SELECTOR: &'static str = media_ignore_selector!();
    /// CSS query selector for all relative links
    static ref MEDIA_SELECTOR_RELATIVE: &'static str = concat!(r#"a[href^="/"]"#, media_ignore_selector!());
    /// CSS query selector for all common static MIME types.
    static ref MEDIA_SELECTOR_STATIC: &'static str = r#"[href$=".html"] [href$=".htm"] [href$=".asp"] [href$=".aspx"] [href$=".php"] [href$=".jps"] [href$=".jpsx"]"#;
}

impl Page {
    /// Instantiate a new page and start to scrape it.
    pub fn new(url: &str, client: &Client) -> Self {
        let html = fetch_page_html(&url, &client); // TODO: remove heavy cpu / network from new

        Page::build(url, &html)
    }

    /// Instanciate a new page without scraping it (used for testing purposes).
    pub fn build(url: &str, html: &str) -> Self {
        Self {
            url: url.to_string(),
            html: html.to_string(),
            base: Url::parse(&url).expect("Invalid page URL")
        }
    }

    /// URL getter for page.
    pub fn get_url(&self) -> &String {
        &self.url
    }

    /// HTML returned from Scraper.
    pub fn get_html(&self) -> Html {
        Html::parse_document(&self.html)
    }

    /// Clear the html for the page.
    pub fn clear_html(&mut self) {
        self.html.clear();
    }

    /// html selector for valid web pages for domain.
    pub fn get_page_selectors(&self, domain: &str) -> Selector {
        // select all absolute links
        let absolute_selector = &format!(
            r#"a[href^="{}"]{}"#,
            domain,
            *MEDIA_IGNORE_SELECTOR,
        );
        // allow relative and absolute .html files
        let static_html_selector = &format!(
            r#"{} {}, {} {}"#,
            *MEDIA_SELECTOR_RELATIVE,
            *MEDIA_SELECTOR_STATIC,
            absolute_selector,
            *MEDIA_SELECTOR_STATIC
        );
        // select all relative links, absolute, and static .html files for a domain
        Selector::parse(&format!(
            "{},{},{}",
            *MEDIA_SELECTOR_RELATIVE,
            absolute_selector,
            static_html_selector
        ))
        .unwrap()
    }

    /// Find all href links and return them using CSS selectors.
    pub fn links(&self) -> HashSet<String> {
        let selector = self.get_page_selectors(&self.url);
        let html = self.get_html();
        
        html.select(&selector)
            .map(|a| self.abs_path(a.value().attr("href").unwrap_or_default()).to_string())
            .collect()
    }

    /// Convert a URL to its absolute path without any fragments or params.
    fn abs_path(&self, href: &str) -> Url {
        let mut joined = self.base.join(href).unwrap_or(Url::parse(&self.url.to_string()).expect("Invalid page URL"));

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
    let page: Page = Page::new(&link_result, &client);
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