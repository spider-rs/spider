use reqwest;
use scraper::{Html, Selector};
use url::Url;
use reqwest::Error;

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

// TODO: RE-EXPORTING RUNTIME FROM RAYON instead install matching
#[tokio::main]
pub async fn fetch_page_html(url: &str, user_agent: &str) -> Result<String, Error> {
    let client = reqwest::Client::builder()
        .user_agent(user_agent)
        .build()
        .unwrap();

    let mut body = String::new();

	let res = client
		.get(url)
		.send()
		.await;

    match res {
        Ok(result) => body = result.text().await?,
        Err(e) => eprintln!("[error] {}: {}", url, e),
    }

    Ok(body)
}

impl Page {
    /// Instanciate a new page and start to scrape it.
    pub fn new(url: &str, user_agent: &str) -> Self {
        Page::build(url, &fetch_page_html(url, user_agent).unwrap())
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

    /// Find all href links and return them
    pub fn links(&self, domain: &str) -> Vec<String> {
        let mut urls: Vec<String> = Vec::new();
        let selector = Selector::parse("a").unwrap();

        let html = self.get_html();
        let anchors = html
            .select(&selector)
            .filter(|a| a.value().attrs().any(|attr| attr.0 == "href"));

        for anchor in anchors {
            if let Some(href) = anchor.value().attr("href") {
                let abs_path = self.abs_path(href);

                if abs_path.as_str().starts_with(domain) {
                    urls.push(abs_path.to_string());
                }
            }
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
            "Could not find {}. Theses URLs was found {:?}",
            page.url,
            page.links("https://choosealicense.com")
    );
}

#[test]
fn test_abs_path() {
    let page: Page = Page::new("https://choosealicense.com/", "spider/1.1.2");

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
