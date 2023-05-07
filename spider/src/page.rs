#[cfg(not(feature = "decentralized"))]
use crate::packages::scraper::Html;
use crate::CaseInsensitiveString;
use compact_str::CompactString;
use hashbrown::HashSet;
use reqwest::Client;
use smallvec::SmallVec;
#[cfg(not(feature = "decentralized"))]
use tokio_stream::StreamExt;
use url::Url;

/// Represent a page visited. This page contains HTML scraped with [scraper](https://crates.io/crates/scraper).
#[derive(Debug, Clone)]
#[cfg(not(feature = "decentralized"))]
pub struct Page {
    /// HTML parsed with [scraper](https://crates.io/crates/scraper) lib. The html is not stored and only used to parse links.
    html: Option<String>,
    /// Base absolute url for page.
    base: Url,
}

/// Represent a page visited. This page contains HTML scraped with [scraper](https://crates.io/crates/scraper).
#[cfg(feature = "decentralized")]
#[derive(Debug, Clone)]
pub struct Page {
    /// HTML parsed with [scraper](https://crates.io/crates/scraper) lib. The html is not stored and only used to parse links.
    html: Option<String>,
    /// The current links for the page.
    pub links: HashSet<CaseInsensitiveString>,
}

lazy_static! {
    /// include only list of resources
    static ref ONLY_RESOURCES: HashSet<CaseInsensitiveString> = {
        let mut m: HashSet<CaseInsensitiveString> = HashSet::with_capacity(14);

        m.extend([
            "html", "htm", "asp", "aspx", "php", "jps", "jpsx",
            // handle .. prefix for urls ending with an extra ending
            ".html", ".htm", ".asp", ".aspx", ".php", ".jps", ".jpsx",
        ].map(|s| s.into()));

        m
    };
}

/// get the clean domain name
pub fn domain_name(domain: &Url) -> &str {
    match domain.host_str() {
        Some(b) => {
            let b = b.split('.').collect::<Vec<&str>>();

            if b.len() > 2 {
                b[1]
            } else if b.len() == 2 {
                b[0]
            } else {
                b[b.len() - 2]
            }
        }
        _ => "",
    }
}

/// convert to absolute path
#[inline]
pub fn convert_abs_path(base: &Url, href: &str) -> Url {
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
) -> Option<(CompactString, SmallVec<[CompactString; 2]>)> {
    match Url::parse(&url) {
        Ok(host) => {
            let host_name = CompactString::from(
                match convert_abs_path(&host, Default::default()).host_str() {
                    Some(host) => host.to_ascii_lowercase(),
                    _ => Default::default(),
                },
            );
            let scheme = host.scheme();

            Some(if tld || subdomains {
                let base = Url::parse(&url).expect("Invalid page URL");
                let dname = domain_name(&base);
                let scheme = base.scheme();

                // static html group parse
                (
                    dname.into(),
                    smallvec::SmallVec::from([host_name, CompactString::from(scheme)]),
                )
            } else {
                (
                    CompactString::default(),
                    smallvec::SmallVec::from([host_name, CompactString::from(scheme)]),
                )
            })
        }
        _ => None,
    }
}

/// Instantiate a new page without scraping it (used for testing purposes).
#[cfg(not(feature = "decentralized"))]
pub fn build(url: &str, html: Option<String>) -> Page {
    Page {
        html: if html.is_some() { html } else { None },
        base: Url::parse(&url).expect("Invalid page URL"),
    }
}

/// Instantiate a new page without scraping it (used for testing purposes).
#[cfg(feature = "decentralized")]
pub fn build(_: &str, html: Option<String>) -> Page {
    Page {
        html: if html.is_some() { html } else { None },
        links: Default::default(),
    }
}

impl Page {
    /// Instantiate a new page and gather the html.
    #[cfg(not(feature = "decentralized"))]
    pub async fn new(url: &str, client: &Client) -> Self {
        build(url, crate::utils::fetch_page_html(&url, &client).await)
    }

    /// Instantiate a new page and gather the links.
    #[cfg(feature = "decentralized")]
    pub async fn new(url: &str, client: &Client) -> Self {
        use crate::serde::Deserialize;
        let links = match crate::utils::fetch_page(&url, &client).await {
            Some(b) => {
                use bytes::Buf;
                match flexbuffers::Reader::get_root(b.chunk()) {
                    Ok(buf) => match HashSet::<CaseInsensitiveString>::deserialize(buf) {
                        Ok(link) => link,
                        _ => Default::default(),
                    },
                    _ => Default::default(),
                }
            }
            _ => Default::default(),
        };

        Page { html: None, links }
    }

    /// URL getter for page.
    #[cfg(not(feature = "decentralized"))]
    pub fn get_url(&self) -> &str {
        self.base.as_str()
    }

    #[cfg(feature = "decentralized")]
    /// URL getter for page.
    pub fn get_url(&self) -> &str {
        ""
    }

    /// Html getter for page.
    pub fn get_html(&self) -> &str {
        match self.html.as_deref() {
            Some(html) => html,
            _ => "",
        }
    }

    /// Find the links as a stream using string resource validation
    #[inline(always)]
    #[cfg(all(not(feature = "decentralized"), not(feature = "full_resources")))]
    pub async fn links_stream<A: PartialEq + Eq + std::hash::Hash + From<String>>(
        &self,
        selectors: &(&CompactString, &SmallVec<[CompactString; 2]>),
    ) -> HashSet<A> {
        let html = Box::new(Html::parse_document(self.get_html()));
        tokio::task::yield_now().await;

        let mut stream = tokio_stream::iter(html.tree);
        let mut map = HashSet::new();

        let base_domain = &selectors.0;
        let parent_frags = &selectors.1; // todo: allow mix match tpt
        let parent_host = &parent_frags[0];
        let parent_host_scheme = &parent_frags[1];

        while let Some(node) = stream.next().await {
            if let Some(element) = node.as_element() {
                match element.attr("href") {
                    Some(href) => {
                        let mut abs = self.abs_path(href);

                        // determine if the crawl can continue based on host match
                        let mut can_process = match abs.host_str() {
                            Some(host) => parent_host.ends_with(host),
                            _ => false,
                        };

                        if can_process {
                            if abs.scheme() != parent_host_scheme.as_str() {
                                let _ = abs.set_scheme(parent_host_scheme.as_str());
                            }

                            let h = abs.as_str();
                            let hlen = h.len();

                            if hlen > 4 {
                                let hchars = &h[hlen - 5..hlen];
                                if let Some(position) = hchars.find('.') {
                                    let resource_ext = &hchars[position + 1..hchars.len()];

                                    if !ONLY_RESOURCES
                                        .contains(&CaseInsensitiveString::from(resource_ext))
                                    {
                                        can_process = false;
                                    }
                                }
                            }

                            if can_process && base_domain.is_empty()
                                || base_domain.as_str() == domain_name(&abs)
                            {
                                map.insert(h.to_string().into());
                            }
                        }
                    }
                    _ => (),
                };
            }
        }

        map
    }

    /// Find the links as a stream using string resource validation
    #[inline(always)]
    #[cfg(all(not(feature = "decentralized"), feature = "full_resources"))]
    pub async fn links_stream<A: PartialEq + Eq + std::hash::Hash + From<String>>(
        &self,
        selectors: &(&CompactString, &SmallVec<[CompactString; 2]>),
    ) -> HashSet<A> {
        let html = Box::new(Html::parse_document(self.get_html()));
        tokio::task::yield_now().await;

        let mut stream = tokio_stream::iter(html.tree);
        let mut map = HashSet::new();

        let base_domain = &selectors.0;
        let parent_frags = &selectors.1; // todo: allow mix match tpt
        let parent_host = &parent_frags[0];
        let parent_host_scheme = &parent_frags[1];

        while let Some(node) = stream.next().await {
            if let Some(element) = node.as_element() {
                match element.attr("href") {
                    Some(href) => {
                        let mut abs = self.abs_path(href);

                        let can_process = match abs.host_str() {
                            Some(host) => parent_host.ends_with(host),
                            _ => false,
                        };

                        if can_process {
                            if abs.scheme() != parent_host_scheme.as_str() {
                                let _ = abs.set_scheme(parent_host_scheme.as_str());
                            }

                            let h = abs.as_str();

                            if can_process && base_domain.is_empty()
                                || base_domain.as_str() == domain_name(&abs)
                            {
                                map.insert(h.to_string().into());
                            }
                        }
                    }
                    _ => (),
                };
            }
        }

        map
    }

    #[inline(always)]
    #[cfg(feature = "decentralized")]
    /// Find the links as a stream using string resource validation
    pub async fn links_stream<A: PartialEq + Eq + std::hash::Hash + From<String>>(
        &self,
        _: &(&CompactString, &SmallVec<[CompactString; 2]>),
    ) -> HashSet<A> {
        Default::default()
    }

    /// Find all href links and return them using CSS selectors.
    #[cfg(not(feature = "decentralized"))]
    #[inline(never)]
    pub async fn links(
        &self,
        selectors: &(CompactString, SmallVec<[CompactString; 2]>),
    ) -> HashSet<CaseInsensitiveString> {
        match self.html.is_some() {
            false => Default::default(),
            true => {
                self.links_stream::<CaseInsensitiveString>(&(&selectors.0, &selectors.1))
                    .await
            }
        }
    }

    /// Find all href links and return them using CSS selectors.
    #[cfg(feature = "decentralized")]
    #[inline(never)]
    pub async fn links(
        &self,
        _: &(CompactString, smallvec::SmallVec<[CompactString; 2]>),
    ) -> HashSet<CaseInsensitiveString> {
        self.links.to_owned()
    }

    /// Convert a URL to its absolute path without any fragments or params.
    #[inline]
    #[cfg(not(feature = "decentralized"))]
    fn abs_path(&self, href: &str) -> Url {
        convert_abs_path(&self.base, href)
    }
}

#[cfg(not(feature = "decentralized"))]
#[tokio::test]
async fn parse_links() {
    let client = Client::builder()
        .user_agent("spider/1.1.2")
        .build()
        .unwrap();

    let link_result = "https://choosealicense.com/";
    let page: Page = Page::new(&link_result, &client).await;
    let selector = get_page_selectors(&link_result, false, false);

    let links = page.links(&selector.unwrap()).await;

    assert!(
        links.contains::<CaseInsensitiveString>(&"https://choosealicense.com/about/".into()),
        "Could not find {}. Theses URLs was found {:?}",
        page.get_url(),
        &links
    );
}

#[cfg(not(feature = "decentralized"))]
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
