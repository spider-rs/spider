use crate::utils::fetch_page_html;
use hashbrown::HashSet;
use reqwest::Client;
use scraper::{Html, Selector};
use url::Url;

/// Represent a page visited. This page contains HTML scraped with [scraper](https://crates.io/crates/scraper).
#[derive(Debug, Clone)]
pub struct Page {
    /// HTML parsed with [scraper](https://crates.io/crates/scraper) lib. The html is not stored and only used to parse links.
    html: String,
    /// Base absolute url for page.
    base: Url,
}

/// CSS query selector to ignore all resources that are not valid web pages.
const MEDIA_IGNORE_SELECTOR: &str = r#":not([style*="display: none"]):not([style*="visibility: hidden"]):not([href$=".ico"]):not([href$=".png"]):not([href$=".jpg"]):not([href$=".jpeg"]):not([href$=".svg"]):not([href$=".xlsx"]):not([href$=".img"]):not([href$=".webp"]):not([href$=".gif"]):not([href$=".pdf"]):not([href$=".tiff"]):not([href$=".mov"]):not([href$=".wav"]):not([href$=".mp3"]):not([href$=".mp4"]):not([href$=".ogg"]):not([href$=".webm"]):not([href$=".sql"]):not([href$=".zip"]):not([href$=".docx"]):not([href$=".git"]):not([href$=".json"]):not([href$=".xml"]):not([href$=".css"]):not([href$=".md"]):not([href$=".txt"]):not([href$=".js"]):not([href$=".jsx"]):not([href$=".csv"])"#;
/// CSS query selector for all relative links that includes MEDIA_IGNORE_SELECTOR
const MEDIA_SELECTOR_RELATIVE: &str = r#"a[href^="/"]:not([href$=".ico"]):not([href$=".png"]):not([href$=".jpg"]):not([href$=".jpeg"]):not([href$=".svg"]):not([href$=".xlsx"]):not([href$=".img"]):not([href$=".webp"]):not([href$=".gif"]):not([href$=".pdf"]):not([href$=".tiff"]):not([href$=".mov"]):not([href$=".wav"]):not([href$=".mp3"]):not([href$=".mp4"]):not([href$=".ogg"]):not([href$=".webm"]):not([href$=".sql"]):not([href$=".zip"]):not([href$=".docx"]):not([href$=".git"]):not([href$=".json"]):not([href$=".xml"]):not([href$=".css"]):not([href$=".md"]):not([href$=".txt"]):not([href$=".js"]):not([href$=".jsx"]):not([href$=".csv"])"#;
/// CSS query selector for all common static MIME types.
const MEDIA_SELECTOR_STATIC: &str = r#"[href$=".html"] [href$=".htm"] [href$=".asp"] [href$=".aspx"] [href$=".php"] [href$=".jps"] [href$=".jpsx"]"#;

/// build absolute page selectors
fn build_absolute_selectors(url: &str) -> String {
    // handle unsecure and secure transports
    string_concat::string_concat!(
        "a[href^=",
        r#"""#,
        if url.starts_with("https") {
            url.replacen("https://", "http://", 1)
        } else {
            url.replacen("http://", "https://", 1)
        },
        r#"""#,
        "i ],",
        "a[href^=",
        r#"""#,
        url,
        r#"""#,
        "i ]",
        MEDIA_IGNORE_SELECTOR
    )
}

/// get the host name for url without tld
pub fn domain_name(domain: &Url) -> &str {
    let b = domain.host_str().unwrap_or_default();
    let b = b.split('.').collect::<Vec<&str>>();

    if b.len() > 2 {
        b[1]
    } else if b.len() == 2 {
        b[0]
    } else {
        b[b.len() - 2]
    }
}

/// html selector for valid web pages for domain.
pub fn get_page_selectors(url: &str, subdomains: bool, tld: bool) -> Selector {
    if tld || subdomains {
        let base = Url::parse(&url).expect("Invalid page URL");
        let dname = domain_name(&base);
        let scheme = base.scheme();
        // . extension
        let tlds = if tld {
            string_concat::string_concat!(
                "a[href^=",
                r#"""#,
                scheme,
                "://",
                dname,
                r#"""#,
                "i ]",
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
                "i ]",
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

/// Instanciate a new page without scraping it (used for testing purposes).
pub fn build(url: &str, html: &str) -> Page {
    Page {
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
    pub fn get_url(&self) -> String {
        self.base.to_string()
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

    /// Find all href links and return them using CSS selectors.
    pub fn links(&self, selector: &Selector, subdomains: bool, _tld: bool) -> HashSet<String> {
        let html = self.parse_html();
        let anchors = html.select(&selector);

        if subdomains {
            let base_domain = domain_name(&self.base);

            anchors
                .filter_map(|a| {
                    let abs = self.abs_path(a.value().attr("href").unwrap_or_default());

                    if base_domain == domain_name(&abs) {
                        Some(abs.to_string().to_lowercase())
                    } else {
                        None
                    }
                })
                .collect()
        } else {
            anchors
                .map(|a| {
                    self.abs_path(a.value().attr("href").unwrap_or_default())
                        .to_string().to_lowercase()
                })
                .collect()
        }
    }

    /// Convert a URL to its absolute path without any fragments or params.
    fn abs_path(&self, href: &str) -> Url {
        match self.base.join(href) {
            Ok(mut joined) => {
                joined.set_fragment(None);
                joined
            }
            Err(_) => Url::parse(&self.get_url()).expect("Invalid page URL"),
        }
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
    let selectors: Selector = get_page_selectors(&link_result, false, false);
    let links = page.links(&selectors, false, false);

    assert!(
        links.contains(&"https://choosealicense.com/about/".to_string()),
        "Could not find {}. Theses URLs was found {:?}",
        page.get_url(),
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
