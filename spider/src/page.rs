use crate::utils::fetch_page_html;
use hashbrown::HashSet;
use reqwest::Client;
use scraper::{Html, Selector};
use url::Url;

/// Represent a page visited. This page contains HTML scraped with [scraper](https://crates.io/crates/scraper).
#[derive(Debug, Clone)]
pub struct Page {
    /// URL of this page.
    url: String,
    /// HTML parsed with [scraper](https://crates.io/crates/scraper) lib. The html is not stored and only used to parse links.
    html: String,
    /// Base absolute url for page.
    base: Url,
}

/// CSS query selector to ignore all resources that are not valid web pages.
const MEDIA_IGNORE_SELECTOR: &str = r#":not([href$=".ico"]):not([href$=".png"]):not([href$=".jpg"]):not([href$=".jpeg"]):not([href$=".svg"]):not([href$=".webp"]):not([href$=".gif"]):not([href$=".pdf"]):not([href$=".tiff"]):not([href$=".wav"]):not([href$=".mp3"]):not([href$=".mp4"]):not([href$=".ogg"]):not([href$=".webm"]):not([href$=".sql"]):not([href$=".zip"]):not([href$=".docx"]):not([href$=".git"]):not([href$=".json"]):not([href$=".xml"]):not([href$=".css"]):not([href$=".md"]):not([href$=".txt"]):not([href$=".js"]):not([href$=".jsx"]):not([href$=".csv"])"#;
/// CSS query selector for all relative links that includes MEDIA_IGNORE_SELECTOR
const MEDIA_SELECTOR_RELATIVE: &str = r#"a[href^="/"]:not([href$=".ico"]):not([href$=".png"]):not([href$=".jpg"]):not([href$=".jpeg"]):not([href$=".svg"]):not([href$=".webp"]):not([href$=".gif"]):not([href$=".pdf"]):not([href$=".tiff"]):not([href$=".wav"]):not([href$=".mp3"]):not([href$=".mp4"]):not([href$=".ogg"]):not([href$=".webm"]):not([href$=".sql"]):not([href$=".zip"]):not([href$=".docx"]):not([href$=".git"]):not([href$=".json"]):not([href$=".xml"]):not([href$=".css"]):not([href$=".md"]):not([href$=".txt"]):not([href$=".js"]):not([href$=".jsx"]):not([href$=".csv"])"#;
/// CSS query selector for all common static MIME types.
const MEDIA_SELECTOR_STATIC: &str = r#"[href$=".html"] [href$=".htm"] [href$=".asp"] [href$=".aspx"] [href$=".php"] [href$=".jps"] [href$=".jpsx"]"#;

/// build absolute page selectors
fn build_absolute_selectors(url: &str) -> String {
    string_concat::string_concat!("a[href^=", r#"""#, url, r#"""#, "]", MEDIA_IGNORE_SELECTOR)
}

/// get the host name for url without tld
fn domain_name(domain: &Url) -> String {
    let b = domain.host_str().unwrap_or("").to_string();
    let mut b = b.split(".").collect::<Vec<&str>>();
    if b.len() >= 2 {
        b.pop(); // remove the tld
    }
    let b = b[b.len() - 1];

    b.to_string()
}

/// Instanciate a new page without scraping it (used for testing purposes).
pub fn build(url: &str, html: &str) -> Page {
    Page {
        url: url.into(),
        html: html.into(),
        base: Url::parse(&url).expect("Invalid page URL"),
    }
}

impl Page {
    /// Instantiate a new page and start to scrape it.
    pub async fn new(url: &str, client: &Client) -> Self {
        let html = fetch_page_html(&url, &client).await; // TODO: remove heavy cpu / network from new

        build(url, &html)
    }

    /// URL getter for page.
    pub fn get_url(&self) -> &String {
        &self.url
    }

    /// Html getter for page.
    pub fn get_html(&self) -> &String {
        &self.html
    }

    /// HTML returned from Scraper.
    fn parse_html(&self) -> Html {
        Html::parse_document(&self.html)
    }

    /// Clear the html for the page.
    pub fn clear_html(&mut self) {
        self.html.clear();
    }

    /// html selector for valid web pages for domain.
    pub fn get_page_selectors(&self, url: &str, subdomains: bool, tld: bool) -> Selector {
        if tld || subdomains {
            let dname = domain_name(&self.base);
            let scheme = self.base.scheme();
            // . extension
            let tlds = if tld {
                string_concat::string_concat!(
                    "a[href^=",
                    r#"""#,
                    scheme,
                    "://",
                    dname,
                    r#"""#,
                    "]",
                    MEDIA_IGNORE_SELECTOR,
                    ","
                )
            // match everything that follows the base.
            } else {
                "".to_string()
            };

            let absolute_selector = build_absolute_selectors(url);
            // absolute urls with subdomains
            let absolute_selector = &if subdomains {
                string_concat::string_concat!(
                    absolute_selector,
                    MEDIA_IGNORE_SELECTOR,
                    ",",
                    tlds,
                    "a[href^=",
                    r#"""#,
                    scheme,
                    r#"""#,
                    "]",
                    "[href*=",
                    r#"""#,
                    ".",
                    dname,
                    ".",
                    r#"""#,
                    "]",
                    MEDIA_IGNORE_SELECTOR
                )
            } else {
                absolute_selector
            };

            // static html group parse
            Selector::parse(&string_concat::string_concat!(
                tlds,
                MEDIA_SELECTOR_RELATIVE,
                ",",
                absolute_selector,
                ",",
                MEDIA_SELECTOR_RELATIVE,
                " ",
                MEDIA_SELECTOR_STATIC,
                ", ",
                absolute_selector,
                " ",
                MEDIA_SELECTOR_STATIC
            ))
            .unwrap()
        } else {
            let absolute_selector = build_absolute_selectors(url);
            let static_html_selector = string_concat::string_concat!(
                MEDIA_SELECTOR_RELATIVE,
                " ",
                MEDIA_SELECTOR_STATIC,
                ",",
                " ",
                absolute_selector,
                " ",
                MEDIA_SELECTOR_STATIC
            );

            Selector::parse(&string_concat::string_concat!(
                MEDIA_SELECTOR_RELATIVE,
                ",",
                absolute_selector,
                ",",
                static_html_selector
            ))
            .unwrap()
        }
    }

    /// Find all href links and return them using CSS selectors.
    pub fn links(&self, subdomains: bool, tld: bool) -> HashSet<String> {
        let selector = self.get_page_selectors(&self.url, subdomains, tld);
        let html = self.parse_html();
        let anchors = html.select(&selector);

        if subdomains {
            let base_domain = domain_name(&self.base);

            anchors
                .filter_map(|a| {
                    let abs = self
                        .abs_path(a.value().attr("href").unwrap_or_default())
                        .to_string();
                    let url_domain = domain_name(&Url::parse(&abs).unwrap());

                    if base_domain == url_domain {
                        Some(abs)
                    } else {
                        None
                    }
                })
                .collect()
        } else {
            anchors
                .map(|a| {
                    self.abs_path(a.value().attr("href").unwrap_or(""))
                        .to_string()
                })
                .collect()
        }
    }

    /// Convert a URL to its absolute path without any fragments or params.
    fn abs_path(&self, href: &str) -> Url {
        let mut joined = self
            .base
            .join(href)
            .unwrap_or(Url::parse(&self.url.to_string()).expect("Invalid page URL"));

        joined.set_fragment(None);

        joined
    }
}

#[tokio::test]
async fn parse_links() {
    let client = Client::builder()
        .user_agent("spider/1.1.2")
        .build()
        .unwrap();

    let link_result = "https://choosealicense.com/";
    let page: Page = Page::new(&link_result, &client).await;
    let links = page.links(false, false);

    assert!(
        links.contains(&"https://choosealicense.com/about/".to_string()),
        "Could not find {}. Theses URLs was found {:?}",
        page.url,
        &links
    );
}

#[tokio::test]
async fn test_abs_path() {
    let client = Client::builder()
        .user_agent("spider/1.1.2")
        .build()
        .unwrap();
    let link_result = "https://choosealicense.com/";
    let page: Page = Page::new(&link_result, &client).await;

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
