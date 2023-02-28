use crate::packages::scraper::{ElementRef, Html, Selector};
use crate::utils::fetch_page_html;
use crate::website::CaseInsensitiveString;
use compact_str::CompactString;
use hashbrown::HashSet;
use reqwest::Client;
use tokio_stream::StreamExt;
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

lazy_static! {
    /// ignore list of resources
    static ref IGNORE_RESOURCES: HashSet<CaseInsensitiveString> = {
        let mut m: HashSet<CaseInsensitiveString> = HashSet::with_capacity(58);

        m.extend([
            "css", "csv", "docx", "gif", "git", "ico", "js", "jsx", "json", "jpg", "jpeg",
            "md", "mp3", "mp4", "ogg", "png", "pdf", "txt", "tiff", "srt", "svg", "sql", "wave",
            "webm", "woff2", "webp", "xml", "xlsx", "zip",
            // handle .. prefix for urls ending with an extra ending
            ".css", ".csv", ".docx", ".gif", ".git", ".ico", ".js", ".jsx", ".json", ".jpg", ".jpeg",
            ".md", ".mp3", ".mp4", ".ogg", ".png", ".pdf", ".txt", ".tiff", ".srt", ".svg", ".sql", ".wave",
            ".webm", ".woff2", ".webp", ".xml", ".xlsx", ".zip",
        ].map(|s| s.into()));

        m
    };
}

/// build absolute page selectors
fn build_absolute_selectors(url: &str) -> (String, String) {
    let off_target = if url.starts_with("https") {
        url.replacen("https://", "http://", 1)
    } else {
        url.replacen("http://", "https://", 1)
    };

    // handle unsecure and secure transports
    let css_base = string_concat::string_concat!(
        "a[href^=",
        r#"""#,
        off_target,
        r#"""#,
        "i ],",
        "a[href^=",
        r#"""#,
        url,
        r#"""#,
        "i ]",
        MEDIA_IGNORE_SELECTOR
    );

    (css_base, off_target)
}

/// get the clean domain name
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

/// convert to absolute path
fn convert_abs_path(base: &Url, href: &str) -> Url {
    match base.join(href) {
        Ok(mut joined) => {
            joined.set_fragment(None);
            joined
        }
        Err(_) => base.clone(),
    }
}

/// html selector for valid web pages for domain.
pub fn get_page_selectors(
    url: &str,
    subdomains: bool,
    tld: bool,
) -> (Selector, CompactString, (String, String)) {
    let host = Url::parse(&url).expect("Invalid page URL");
    let host_name = match convert_abs_path(&host, &"").host_str() {
        Some(host) => host,
        _ => "",
    }
    .to_ascii_lowercase();

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

        let (absolute_selector, off_target) = build_absolute_selectors(url);

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
        (
            unsafe {
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
                .unwrap_unchecked()
            },
            dname.into(),
            (host_name, off_target),
        )
    } else {
        let (absolute_selector, off_target) = build_absolute_selectors(url);
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

        (
            unsafe {
                Selector::parse(&string_concat::string_concat!(
                    MEDIA_SELECTOR_RELATIVE,
                    ",",
                    absolute_selector,
                    ",",
                    static_html_selector
                ))
                .unwrap_unchecked()
            },
            CompactString::default(),
            (host_name, off_target),
        )
    }
}

/// Instantiate a new page without scraping it (used for testing purposes).
pub fn build(url: &str, html: String) -> Page {
    Page {
        html,
        base: Url::parse(&url).expect("Invalid page URL"),
    }
}

impl Page {
    /// Instantiate a new page and gather the html.
    pub async fn new(url: &str, client: &Client) -> Self {
        let html = fetch_page_html(&url, &client).await; // TODO: remove heavy cpu / network from new

        build(url, html)
    }

    /// URL getter for page.
    pub fn get_url(&self) -> &str {
        self.base.as_str()
    }

    /// Html getter for page.
    pub fn get_html(&self) -> &String {
        &self.html
    }

    /// Clear the html for the page.
    pub fn clear_html(&mut self) {
        self.html.clear();
    }

    /// Find all link hrefs using the ego tree extremely imp useful when concurrency is low
    pub async fn links_ego(
        &self,
        selectors: &(Selector, CompactString, (String, String)),
    ) -> HashSet<CaseInsensitiveString> {
        let base_domain = &selectors.1;
        let mut map: HashSet<CaseInsensitiveString> = HashSet::new();
        let html = Html::parse_document(self.html.as_str());
        tokio::task::yield_now().await;

        // extremely fast ego tree handling
        for node in html.tree.root().traverse() {
            match node {
                ego_tree::iter::Edge::Open(node_ref) => {
                    if let Some(element) = ElementRef::wrap(node_ref) {
                        if element.parent().is_some() && selectors.0.matches(&element) {
                            match element.value().attr("href") {
                                Some(val) => {
                                    let abs = self.abs_path(val);

                                    if base_domain.is_empty()
                                        || base_domain.as_str() == domain_name(&abs)
                                    {
                                        let h = abs.as_str();
                                        let hlen = h.len();
                                        let hchars = &h[hlen - 5..hlen];

                                        // validte non fragments
                                        if let Some(position) = hchars.find('.') {
                                            if !IGNORE_RESOURCES
                                                .contains(&hchars[position + 1..hchars.len()])
                                            {
                                                map.insert(h.into());
                                            }
                                        } else {
                                            map.insert(h.into());
                                        }
                                    }
                                }
                                None => (),
                            }
                        }
                    }
                }
                _ => (),
            }
        }

        map
    }

    /// Find all href links and return them using CSS selectors.
    pub async fn links(
        &self,
        selectors: &(Selector, CompactString, (String, String)),
        streamed: Option<bool>,
    ) -> HashSet<CaseInsensitiveString> {
        let base_domain = &selectors.1;

        match streamed {
            None | Some(false) => self.links_ego(&(selectors)).await,
            Some(_) => {
                let mut map: HashSet<CaseInsensitiveString> = HashSet::new();
                let html = Box::new(Html::parse_document(self.html.as_str()));
                tokio::task::yield_now().await;

                let mut stream = tokio_stream::iter(html.tree);
                let (tmp, _) = &selectors.2; // todo: allow mix match tpt

                while let Some(node) = stream.next().await {
                    if let Some(element) = node.as_element() {
                        match element.attr("href") {
                            Some(href) => {
                                let abs = self.abs_path(href);

                                let mut can_process = match abs.host_str() {
                                    Some(host) => host == tmp,
                                    _ => false,
                                };

                                if can_process {
                                    let h = abs.as_str();
                                    let hlen = h.len();

                                    if hlen > 4 {
                                        let hchars = &h[hlen - 5..hlen];
                                        if let Some(position) = hchars.find('.') {
                                            if IGNORE_RESOURCES
                                                .contains(&hchars[position + 1..hchars.len()])
                                            {
                                                can_process = false;
                                            }
                                        }
                                    }

                                    if can_process {
                                        if base_domain.is_empty()
                                            || base_domain.as_str() == domain_name(&abs)
                                        {
                                            map.insert(h.into());
                                        }
                                    }
                                }
                            }
                            _ => (),
                        };
                    }
                }

                map
            }
        }
    }

    /// Convert a URL to its absolute path without any fragments or params.
    fn abs_path(&self, href: &str) -> Url {
        convert_abs_path(&self.base, href)
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
    let links = page
        .links(&get_page_selectors(&link_result, false, false), None)
        .await;

    assert!(
        links.contains::<CaseInsensitiveString>(&"https://choosealicense.com/about/".into()),
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
