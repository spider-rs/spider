#[cfg(not(feature = "decentralized"))]
use crate::packages::scraper::Html;
use crate::CaseInsensitiveString;
use bytes::Bytes;
use compact_str::CompactString;
use hashbrown::HashSet;
use reqwest::Client;
use smallvec::SmallVec;

#[cfg(all(feature = "time", not(feature = "decentralized")))]
use std::time::Duration;
#[cfg(all(feature = "time", not(feature = "decentralized")))]
use tokio::time::Instant;

#[cfg(not(feature = "decentralized"))]
use tokio_stream::StreamExt;
use url::Url;

lazy_static! {
    /// Wildcard match all domains.
    static ref CASELESS_WILD_CARD: CaseInsensitiveString = CaseInsensitiveString::new("*");
}

/// Represent a page visited. This page contains HTML scraped with [scraper](https://crates.io/crates/scraper).
#[derive(Debug, Clone)]
#[cfg(not(feature = "decentralized"))]
pub struct Page {
    /// The bytes of the resource.
    html: Option<Bytes>,
    /// Base absolute url for page.
    base: Url,
    /// The raw url for the page. Useful since Url::parse adds a trailing slash.
    url: String,
    #[cfg(feature = "time")]
    /// The duration from start of parsing to end of gathering links.
    duration: Instant,
    /// The external urls to group with the domain
    pub external_domains_caseless: Box<HashSet<CaseInsensitiveString>>,
    /// The final destination of the page if redirects were performed [Not implemented in the chrome feature].
    pub final_redirect_destination: Option<String>,
}

/// Represent a page visited. This page contains HTML scraped with [scraper](https://crates.io/crates/scraper).
#[cfg(feature = "decentralized")]
#[derive(Debug, Clone)]
pub struct Page {
    /// The bytes of the resource.
    html: Option<Bytes>,
    /// The current links for the page.
    pub links: HashSet<CaseInsensitiveString>,
    /// The external urls to group with the domain.
    pub external_domains_caseless: Box<HashSet<CaseInsensitiveString>>,
    /// The final destination of the page if redirects were performed [Unused].
    pub final_redirect_destination: Option<String>,
}

lazy_static! {
    /// include only list of resources
    static ref ONLY_RESOURCES: HashSet<CaseInsensitiveString> = {
        let mut m: HashSet<CaseInsensitiveString> = HashSet::with_capacity(16);

        m.extend([
            "html", "htm", "asp", "aspx", "php", "jps", "jpsx", "jsp",
            // handle .. prefix for urls ending with an extra ending
            ".html", ".htm", ".asp", ".aspx", ".php", ".jps", ".jpsx", ".jsp",
        ].map(|s| s.into()));

        m
    };
}

/// get the clean domain name
pub fn domain_name(domain: &Url) -> &str {
    match domain.host_str() {
        Some(b) => {
            let b = b.split('.').collect::<Vec<&str>>();
            let bsize = b.len();

            if bsize > 0 {
                b[bsize - 1]
            } else {
                ""
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
                let dname = domain_name(&host);
                let scheme = host.scheme();

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
pub fn build(url: &str, html: Option<bytes::Bytes>) -> Page {
    Page {
        html: if html.is_some() { html } else { None },
        base: Url::parse(&url).expect("Invalid page URL"),
        url: url.into(),
        #[cfg(feature = "time")]
        duration: Instant::now(),
        external_domains_caseless: Default::default(),
        final_redirect_destination: Default::default(),
    }
}

/// Instantiate a new page without scraping it (used for testing purposes).
#[cfg(feature = "decentralized")]
pub fn build(_: &str, html: Option<bytes::Bytes>) -> Page {
    Page {
        html: if html.is_some() { html } else { None },
        links: Default::default(),
        external_domains_caseless: Default::default(),
        final_redirect_destination: Default::default(),
    }
}

impl Page {
    #[cfg(all(not(feature = "decentralized"), feature = "chrome"))]
    /// Instantiate a new page and gather the html.
    pub async fn new(url: &str, client: &Client, page: &chromiumoxide_fork::Page) -> Self {
        let page_resource = crate::utils::fetch_page_html(&url, &client, &page).await;
        let mut page = build(url, page_resource.0);
        page.set_final_redirect(page_resource.1);
        page
    }

    #[cfg(not(feature = "decentralized"))]
    /// Instantiate a new page and gather the html repro of standard fetch_page_html.
    pub async fn new_page(url: &str, client: &Client) -> Self {
        let page_resource = crate::utils::fetch_page_html_raw(&url, &client).await;
        let mut page = build(url, page_resource.0);
        page.set_final_redirect(page_resource.1);
        page
    }

    /// Instantiate a new page and gather the html.
    #[cfg(all(not(feature = "decentralized"), not(feature = "chrome")))]
    pub async fn new(url: &str, client: &Client) -> Self {
        let page_resource = crate::utils::fetch_page_html(&url, &client).await;
        let mut page = build(url, page_resource.0);
        page.set_final_redirect(page_resource.1);
        page
    }

    /// Instantiate a new page and gather the links.
    #[cfg(feature = "decentralized")]
    pub async fn new(url: &str, client: &Client) -> Self {
        use crate::serde::Deserialize;
        use bytes::Buf;
        let links = match crate::utils::fetch_page(&url, &client).await {
            Some(b) => match flexbuffers::Reader::get_root(b.chunk()) {
                Ok(buf) => match HashSet::<CaseInsensitiveString>::deserialize(buf) {
                    Ok(link) => link,
                    _ => Default::default(),
                },
                _ => Default::default(),
            },
            _ => Default::default(),
        };

        Page {
            html: None,
            links,
            external_domains_caseless: Default::default(),
            final_redirect_destination: Default::default(),
        }
    }

    /// Page request fulfilled.
    pub fn is_empty(&self) -> bool {
        self.html.is_none()
    }

    /// Url getter for page.
    #[cfg(not(feature = "decentralized"))]
    pub fn get_url(&self) -> &str {
        &self.url
    }

    /// Url getter for page after redirects.
    #[cfg(not(feature = "decentralized"))]
    pub fn get_url_final(&self) -> &str {
        match self.final_redirect_destination.as_ref() {
            Some(u) => &u,
            _ => &self.url,
        }
    }

    /// Set the external domains to treat as one
    pub fn set_external(&mut self, external_domains_caseless: Box<HashSet<CaseInsensitiveString>>) {
        self.external_domains_caseless = external_domains_caseless;
    }

    /// Set final redirect destination
    pub fn set_final_redirect(&mut self, final_redirect_destination: Option<String>) {
        self.final_redirect_destination = final_redirect_destination;
    }

    /// Parsed URL getter for page.
    #[cfg(not(feature = "decentralized"))]
    pub fn get_url_parsed(&self) -> &Url {
        &self.base
    }

    #[cfg(feature = "decentralized")]
    /// URL getter for page.
    pub fn get_url(&self) -> &str {
        ""
    }

    /// Html getter for bytes on the page.
    pub fn get_bytes(&self) -> Option<&Bytes> {
        match self.html.as_ref() {
            Some(html) => Some(html),
            _ => None,
        }
    }

    /// Html getter for bytes on the page as string.
    pub fn get_html(&self) -> String {
        match self.html.as_ref() {
            Some(html) => String::from_utf8_lossy(&html).to_string(),
            _ => Default::default(),
        }
    }

    /// Html getter for page to u8.
    pub fn get_html_bytes_u8(&self) -> &[u8] {
        match self.html.as_deref() {
            Some(html) => html,
            _ => Default::default(),
        }
    }

    /// Get the elasped duration of the page since scraped.
    #[cfg(all(feature = "time", not(feature = "decentralized")))]
    pub fn get_duration_elasped(&self) -> Duration {
        self.duration.elapsed()
    }

    /// Find the links as a stream using string resource validation
    #[inline(always)]
    #[cfg(all(
        not(feature = "decentralized"),
        not(feature = "full_resources"),
        not(feature = "js")
    ))]
    pub async fn links_stream<A: PartialEq + Eq + std::hash::Hash + From<String>>(
        &self,
        selectors: &(&CompactString, &SmallVec<[CompactString; 2]>),
    ) -> HashSet<A> {
        let html = Box::new(Html::parse_fragment(&self.get_html()));
        tokio::task::yield_now().await;

        let mut stream = tokio_stream::iter(html.tree);
        let mut map = HashSet::new();

        let base_domain = &selectors.0;
        let parent_frags = &selectors.1; // todo: allow mix match tpt
        let parent_host = &parent_frags[0];
        let parent_host_scheme = &parent_frags[1];

        while let Some(node) = stream.next().await {
            if let Some(element) = node.as_element() {
                if element.name() == "a" {
                    match element.attr("href") {
                        Some(href) => {
                            let mut abs = self.abs_path(href);
                            let host_name = abs.host_str();
                            let mut can_process = match host_name {
                                Some(host) => {
                                    if base_domain.is_empty() {
                                        parent_host.eq(&host)
                                    } else {
                                        parent_host.ends_with(host)
                                    }
                                }
                                _ => false,
                            };
                            if !can_process
                                && host_name.is_some()
                                && !self.external_domains_caseless.is_empty()
                            {
                                can_process = self
                                    .external_domains_caseless
                                    .contains::<CaseInsensitiveString>(
                                        &host_name.unwrap_or_default().into(),
                                    )
                                    || self
                                        .external_domains_caseless
                                        .contains::<CaseInsensitiveString>(&CASELESS_WILD_CARD)
                            }

                            if can_process {
                                if abs.scheme() != parent_host_scheme.as_str() {
                                    let _ = abs.set_scheme(parent_host_scheme.as_str());
                                }

                                let hchars = abs.path();

                                if let Some(position) = hchars.rfind('.') {
                                    let resource_ext = &hchars[position + 1..hchars.len()];

                                    if !ONLY_RESOURCES
                                        .contains::<CaseInsensitiveString>(&resource_ext.into())
                                    {
                                        can_process = false;
                                    }
                                }

                                if can_process
                                    && (base_domain.is_empty()
                                        || base_domain.as_str() == domain_name(&abs))
                                {
                                    map.insert(abs.as_str().to_string().into());
                                }
                            }
                        }
                        _ => (),
                    };
                }
            }
        }

        map
    }

    /// Find the links as a stream using string resource validation
    #[inline(always)]
    #[cfg(all(
        not(feature = "decentralized"),
        not(feature = "full_resources"),
        feature = "js"
    ))]
    pub async fn links_stream<
        A: PartialEq + std::fmt::Debug + Eq + std::hash::Hash + From<String>,
    >(
        &self,
        selectors: &(&CompactString, &SmallVec<[CompactString; 2]>),
    ) -> HashSet<A> {
        use jsdom::extract::extract_links;

        lazy_static! {
            /// include only list of resources
            static ref IGNORE_ASSETS: HashSet<&'static str> = {
                let mut m: HashSet<&'static str> = HashSet::with_capacity(23);

                m.extend::<[&'static str; 23]>([
                    "jquery.min.js", "jquery.qtip.min.js", "jquery.js", "angular.js", "jquery.slim.js", "react.development.js", "react-dom.development.js", "react.production.min.js", "react-dom.production.min.js",
                    "vue.global.js", "vue.esm-browser.js", "vue.js", "bootstrap.min.js", "bootstrap.bundle.min.js", "bootstrap.esm.min.js", "d3.min.js", "d3.js", "material-components-web.min.js",
                    "otSDKStub.js", "clipboard.min.js", "moment.js", "moment.min.js", "dexie.js",
                ].map(|s| s.into()));

                m
            };
        }

        let base_domain = &selectors.0;
        let parent_frags = &selectors.1; // todo: allow mix match tpt
        let parent_host = &parent_frags[0];
        let parent_host_scheme = &parent_frags[1];

        let mut map = HashSet::new();
        let html = Box::new(self.get_html());

        if !base_domain.is_empty() && !html.starts_with("<") {
            let links: HashSet<CaseInsensitiveString> = extract_links(&html).await;
            let mut stream = tokio_stream::iter(&links);

            while let Some(href) = stream.next().await {
                let mut abs = self.abs_path(href.inner());
                let host_name = abs.host_str();
                let mut can_process = match host_name {
                    Some(host) => {
                        if base_domain.is_empty() {
                            parent_host.eq(&host)
                        } else {
                            parent_host.ends_with(host)
                        }
                    }
                    _ => false,
                };
                if !can_process && host_name.is_some() && !self.external_domains_caseless.is_empty()
                {
                    can_process = self
                        .external_domains_caseless
                        .contains::<CaseInsensitiveString>(&host_name.unwrap_or_default().into())
                }

                if can_process {
                    if abs.scheme() != parent_host_scheme.as_str() {
                        let _ = abs.set_scheme(parent_host_scheme.as_str());
                    }
                    let hchars = abs.path();

                    if let Some(position) = hchars.rfind('.') {
                        let resource_ext = &hchars[position + 1..hchars.len()];

                        if !ONLY_RESOURCES.contains::<CaseInsensitiveString>(&resource_ext.into()) {
                            can_process = false;
                        }
                    }

                    if can_process
                        && (base_domain.is_empty() || base_domain.as_str() == domain_name(&abs))
                    {
                        map.insert(abs.as_str().to_string().into());
                    }
                }
            }
        } else {
            let html = Box::new(Html::parse_document(&html));
            tokio::task::yield_now().await;
            let mut stream = tokio_stream::iter(html.tree);

            while let Some(node) = stream.next().await {
                if let Some(element) = node.as_element() {
                    let element_name = element.name();

                    if element_name == "script" {
                        match element.attr("src") {
                            Some(src) => {
                                if src.starts_with("/")
                                    && element.attr("id") != Some("gatsby-chunk-mapping")
                                {
                                    // check special framework paths todo: customize path segments to build for framework
                                    // IGNORE: next.js pre-rendering pages since html is already rendered
                                    if !src.starts_with("/_next/static/chunks/pages/")
                                        && !src.starts_with("/webpack-runtime-")
                                    {
                                        let abs = self.abs_path(src);
                                        // determine if script can run
                                        let mut insertable = true;

                                        match abs.path_segments().ok_or_else(|| "cannot be base") {
                                            Ok(mut paths) => {
                                                while let Some(p) = paths.next() {
                                                    // todo: get the path last before None instead of checking for ends_with
                                                    if p.ends_with(".js")
                                                        && IGNORE_ASSETS.contains(&p)
                                                    {
                                                        insertable = false;
                                                    }
                                                }
                                            }
                                            _ => (),
                                        };

                                        if insertable {
                                            map.insert(abs.as_str().to_string().into());
                                        }
                                    }
                                }
                            }
                            _ => (),
                        }
                    }
                    if element_name == "a" {
                        match element.attr("href") {
                            Some(href) => {
                                let mut abs = self.abs_path(href);

                                // determine if the crawl can continue based on host match
                                let mut can_process = match abs.host_str() {
                                    Some(host) => {
                                        if base_domain.is_empty() {
                                            parent_host.eq(&host)
                                        } else {
                                            parent_host.ends_with(host)
                                        }
                                    }
                                    _ => false,
                                };

                                if can_process {
                                    if abs.scheme() != parent_host_scheme.as_str() {
                                        let _ = abs.set_scheme(parent_host_scheme.as_str());
                                    }
                                    let hchars = abs.path();

                                    if let Some(position) = hchars.find('.') {
                                        let resource_ext = &hchars[position + 1..hchars.len()];

                                        if !ONLY_RESOURCES
                                            .contains::<CaseInsensitiveString>(&resource_ext.into())
                                        {
                                            can_process = false;
                                        }
                                    }

                                    if can_process
                                        && (base_domain.is_empty()
                                            || base_domain.as_str() == domain_name(&abs))
                                    {
                                        map.insert(abs.as_str().to_string().into());
                                    }
                                }
                            }
                            _ => (),
                        };
                    }
                }
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
        let html = Box::new(Html::parse_document(&self.get_html()));
        tokio::task::yield_now().await;

        let mut stream = tokio_stream::iter(html.tree);
        let mut map = HashSet::new();

        let base_domain = &selectors.0;
        let parent_frags = &selectors.1; // todo: allow mix match tpt
        let parent_host = &parent_frags[0];
        let parent_host_scheme = &parent_frags[1];

        while let Some(node) = stream.next().await {
            if let Some(element) = node.as_element() {
                let element_name = element.name();

                let ele_attribute = if element_name == "a" || element_name == "link" {
                    "href"
                } else if element_name == "script" {
                    "src"
                } else {
                    "href"
                };

                match element.attr(ele_attribute) {
                    Some(href) => {
                        let mut abs = self.abs_path(href);

                        let can_process = match abs.host_str() {
                            Some(host) => {
                                if base_domain.is_empty() {
                                    parent_host.eq(&host)
                                } else {
                                    parent_host.ends_with(host)
                                }
                            }
                            _ => false,
                        };

                        if can_process {
                            if abs.scheme() != parent_host_scheme.as_str() {
                                let _ = abs.set_scheme(parent_host_scheme.as_str());
                            }

                            let h = abs.as_str();

                            if can_process
                                && (base_domain.is_empty()
                                    || base_domain.as_str() == domain_name(&abs))
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

#[cfg(all(not(feature = "decentralized"), not(feature = "chrome")))]
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

#[cfg(all(not(feature = "decentralized"), not(feature = "chrome")))]
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

#[cfg(all(feature = "time", not(feature = "decentralized")))]
#[tokio::test]
async fn test_duration() {
    let client = Client::builder()
        .user_agent("spider/1.1.2")
        .build()
        .unwrap();

    let link_result = "https://choosealicense.com/";
    let page: Page = Page::new(&link_result, &client).await;
    let duration_elasped = page.get_duration_elasped().as_millis();

    assert!(
        duration_elasped < 6000,
        "Duration took longer than expected {}.",
        duration_elasped,
    );
}
