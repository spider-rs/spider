use crate::compact_str::CompactString;

#[cfg(all(feature = "chrome", not(feature = "decentralized")))]
use crate::configuration::{AutomationScripts, ExecutionScripts};

#[cfg(not(feature = "decentralized"))]
use crate::packages::scraper::Html;

use crate::utils::log;
use crate::utils::PageResponse;
use crate::CaseInsensitiveString;
use crate::Client;
use crate::RelativeSelectors;
use bytes::Bytes;
use hashbrown::HashSet;
use reqwest::StatusCode;

#[cfg(all(feature = "time", not(feature = "decentralized")))]
use std::time::Duration;
#[cfg(all(feature = "time", not(feature = "decentralized")))]
use tokio::time::Instant;

#[cfg(all(feature = "decentralized", feature = "headers"))]
use crate::utils::FetchPageResult;
use tokio_stream::StreamExt;
use url::Url;

lazy_static! {
    /// Wildcard match all domains.
    static ref CASELESS_WILD_CARD: CaseInsensitiveString = CaseInsensitiveString::new("*");
}

#[cfg(any(feature = "smart", feature = "chrome_intercept"))]
lazy_static! {
    /// popular js frameworks and libs
    pub static ref JS_FRAMEWORK_ASSETS: phf::Set<&'static str> = {
        phf::phf_set! {
            "jquery.min.js", "jquery.qtip.min.js", "jquery.js", "angular.js", "jquery.slim.js", "react.development.js", "react-dom.development.js", "react.production.min.js", "react-dom.production.min.js",
            "vue.global.js", "vue.esm-browser.js", "vue.js", "bootstrap.min.js", "bootstrap.bundle.min.js", "bootstrap.esm.min.js", "d3.min.js", "d3.js", "material-components-web.min.js",
            "otSDKStub.js", "clipboard.min.js", "moment.js", "moment.min.js", "dexie.js"
        }
    };
}

#[cfg(any(feature = "chrome_intercept"))]
lazy_static! {
    /// allowed js frameworks and libs excluding some and adding additional URLs
    pub static ref JS_FRAMEWORK_ALLOW: phf::Set<&'static str> = {
        phf::phf_set! {
            // Add allowed assets from JS_FRAMEWORK_ASSETS except the excluded ones
            "jquery.min.js", "jquery.qtip.min.js", "jquery.js", "angular.js", "jquery.slim.js",
            "react.development.js", "react-dom.development.js", "react.production.min.js",
            "react-dom.production.min.js", "vue.global.js", "vue.esm-browser.js", "vue.js",
            "bootstrap.min.js", "bootstrap.bundle.min.js", "bootstrap.esm.min.js", "d3.min.js",
            "d3.js",
            // Verified 3rd parties for request
            "https://m.stripe.network/inner.html",
            "https://m.stripe.network/out-4.5.43.js",
            "https://challenges.cloudflare.com/turnstile",
            "https://js.stripe.com/v3/"
        }
    };
}

lazy_static! {
    /// include only list of resources
    static ref ONLY_RESOURCES: HashSet<CaseInsensitiveString> = {
        let mut m: HashSet<CaseInsensitiveString> = HashSet::with_capacity(20);

        m.extend([
            "html", "htm", "shtml", "asp", "aspx", "php", "jps", "jpsx", "jsp", "cfm",
            // handle .. prefix for urls ending with an extra ending
            ".html", ".htm", ".shtml", ".asp", ".aspx", ".php", ".jps", ".jpsx", ".jsp", ".cfm"
        ].map(|s| s.into()));

        m
    };
}

/// The AI data returned from a GPT.
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AIResults {
    /// The prompt used for the GPT.
    pub input: String,
    /// The js output of the GPT response.
    pub js_output: String,
    /// The content output returned from the GPT response that is not a browser script, example: extracted data from the markup.
    pub content_output: Vec<String>,
    /// The base64 image of the page.
    pub screenshot_output: Option<Vec<u8>>,
    /// The error of the occured if any.
    pub error: Option<String>,
}

/// Represent a page visited. This page contains HTML scraped with [scraper](https://crates.io/crates/scraper).
#[derive(Debug, Clone)]
#[cfg(not(feature = "decentralized"))]
pub struct Page {
    /// The bytes of the resource.
    html: Option<Bytes>,
    /// Base absolute url for page.
    base: Option<Url>,
    /// The raw url for the page. Useful since Url::parse adds a trailing slash.
    url: String,
    #[cfg(feature = "headers")]
    /// The headers of the page request response.
    pub headers: Option<reqwest::header::HeaderMap>,
    #[cfg(feature = "cookies")]
    /// The cookies of the page request response.
    pub cookies: Option<reqwest::header::HeaderMap>,
    /// The status code of the page request.
    pub status_code: StatusCode,
    /// The error of the request if any.
    pub error_status: Option<String>,
    /// The external urls to group with the domain
    pub external_domains_caseless: Box<HashSet<CaseInsensitiveString>>,
    /// The final destination of the page if redirects were performed [Not implemented in the chrome feature].
    pub final_redirect_destination: Option<String>,
    #[cfg(feature = "time")]
    /// The duration from start of parsing to end of gathering links.
    duration: Instant,
    #[cfg(feature = "chrome")]
    /// Page object for chrome. The page may be closed when accessing it on another thread from concurrency.
    chrome_page: Option<chromiumoxide::Page>,
    #[cfg(feature = "chrome")]
    /// The screenshot bytes of the page.
    pub screenshot_bytes: Option<Vec<u8>>,
    #[cfg(feature = "openai")]
    /// The credits used from OpenAI in order.
    pub openai_credits_used: Option<Vec<crate::features::openai_common::OpenAIUsage>>,
    #[cfg(feature = "openai")]
    /// The extra data from the AI, example extracting data etc...
    pub extra_ai_data: Option<Vec<AIResults>>,
    /// The links found on the page.
    pub page_links: Option<Box<HashSet<CaseInsensitiveString>>>,
    /// The language for the page.
    pub lang: Option<String>,
}

/// Represent a page visited. This page contains HTML scraped with [scraper](https://crates.io/crates/scraper).
#[cfg(feature = "decentralized")]
#[derive(Debug, Clone, Default)]
pub struct Page {
    /// The bytes of the resource.
    html: Option<Bytes>,
    #[cfg(feature = "headers")]
    /// The headers of the page request response.
    pub headers: Option<reqwest::header::HeaderMap>,
    #[cfg(feature = "cookies")]
    /// The cookies of the page request response.
    pub cookies: Option<reqwest::header::HeaderMap>,
    /// The status code of the page request.
    pub status_code: StatusCode,
    /// The error of the request if any.
    pub error_status: Option<String>,
    /// The current links for the page.
    pub links: HashSet<CaseInsensitiveString>,
    /// The external urls to group with the domain.
    pub external_domains_caseless: Box<HashSet<CaseInsensitiveString>>,
    /// The final destination of the page if redirects were performed [Unused].
    pub final_redirect_destination: Option<String>,
    #[cfg(feature = "chrome")]
    /// The screenshot bytes of the page.
    pub screenshot_bytes: Option<Vec<u8>>,
    #[cfg(feature = "openai")]
    /// The credits used from OpenAI in order.
    pub openai_credits_used: Option<Vec<crate::features::openai_common::OpenAIUsage>>,
    #[cfg(feature = "openai")]
    /// The extra data from the AI, example extracting data etc...
    pub extra_ai_data: Option<Vec<AIResults>>,
    /// The links found on the page. Unused until we can structure the buffers to match.
    pub page_links: Option<Box<HashSet<CaseInsensitiveString>>>,
    /// The language for the page.
    pub lang: Option<String>,
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
        Err(e) => {
            log("URL Parse Error: ", e.to_string());
            // todo: we may want to repair the url being passed in.
            base.clone()
        }
    }
}

/// validation to match a domain to parent host and the top level redirect for the crawl 'parent_host' and 'base_host' being the input start domain.
pub fn parent_host_match(
    host_name: Option<&str>,
    base_domain: &str,
    parent_host: &CompactString,
    base_host: &CompactString,
) -> bool {
    match host_name {
        Some(host) => {
            if base_domain.is_empty() {
                parent_host.eq(&host) || base_host.eq(&host)
            } else {
                host.ends_with(parent_host.as_str()) || host.ends_with(base_host.as_str())
            }
        }
        _ => false,
    }
}

/// html selector for valid web pages for domain.
pub fn get_page_selectors_base(u: &Url, subdomains: bool, tld: bool) -> Option<RelativeSelectors> {
    let host_name =
        CompactString::from(match convert_abs_path(&u, Default::default()).host_str() {
            Some(host) => host.to_ascii_lowercase(),
            _ => Default::default(),
        });
    let scheme = u.scheme();

    Some(if tld || subdomains {
        let dname = domain_name(&u);

        (
            dname.into(),
            smallvec::SmallVec::from([host_name, CompactString::from(scheme)]),
            CompactString::default(),
        )
    } else {
        (
            CompactString::default(),
            smallvec::SmallVec::from([host_name, CompactString::from(scheme)]),
            CompactString::default(),
        )
    })
}

/// html selector for valid web pages for domain.
pub fn get_page_selectors(url: &str, subdomains: bool, tld: bool) -> Option<RelativeSelectors> {
    match Url::parse(url) {
        Ok(host) => get_page_selectors_base(&host, subdomains, tld),
        _ => None,
    }
}

/// Instantiate a new page without scraping it (used for testing purposes).
#[cfg(not(feature = "decentralized"))]
pub fn build(url: &str, res: PageResponse) -> Page {
    Page {
        html: if res.content.is_some() {
            res.content
        } else {
            None
        },
        #[cfg(feature = "headers")]
        headers: res.headers,
        #[cfg(feature = "cookies")]
        cookies: res.cookies,
        base: match Url::parse(url) {
            Ok(u) => Some(u),
            _ => None,
        },
        url: url.into(),
        #[cfg(feature = "time")]
        duration: Instant::now(),
        external_domains_caseless: Default::default(),
        final_redirect_destination: res.final_url,
        status_code: res.status_code,
        error_status: match res.error_for_status {
            Some(e) => match e {
                Ok(_) => None,
                Err(er) => Some(er.to_string()),
            },
            _ => None,
        },
        #[cfg(feature = "chrome")]
        chrome_page: None,
        #[cfg(feature = "chrome")]
        screenshot_bytes: res.screenshot_bytes,
        #[cfg(feature = "openai")]
        openai_credits_used: res.openai_credits_used,
        #[cfg(feature = "openai")]
        extra_ai_data: res.extra_ai_data,
        page_links: None,
        lang: None,
    }
}

/// Instantiate a new page without scraping it (used for testing purposes).
#[cfg(feature = "decentralized")]
pub fn build(_: &str, res: PageResponse) -> Page {
    Page {
        html: if res.content.is_some() {
            res.content
        } else {
            None
        },
        #[cfg(feature = "headers")]
        headers: res.headers,
        #[cfg(feature = "cookies")]
        cookies: res.cookies,
        final_redirect_destination: res.final_url,
        status_code: res.status_code,
        error_status: match res.error_for_status {
            Some(e) => match e {
                Ok(_) => None,
                Err(er) => Some(er.to_string()),
            },
            _ => None,
        },
        ..Default::default()
    }
}

impl Page {
    /// Instantiate a new page and gather the html repro of standard fetch_page_html.
    pub async fn new_page(url: &str, client: &Client) -> Self {
        let page_resource = crate::utils::fetch_page_html_raw(url, client).await;
        build(url, page_resource)
    }

    /// Instantiate a new page and gather the html.
    #[cfg(all(not(feature = "decentralized"), not(feature = "chrome")))]
    pub async fn new(url: &str, client: &Client) -> Self {
        let page_resource = crate::utils::fetch_page_html(url, client).await;
        build(url, page_resource)
    }

    #[cfg(all(not(feature = "decentralized"), feature = "chrome"))]
    /// Instantiate a new page and gather the html.
    pub async fn new(
        url: &str,
        client: &Client,
        page: &chromiumoxide::Page,
        wait_for: &Option<crate::configuration::WaitFor>,
        screenshot: &Option<crate::configuration::ScreenShotConfig>,
        page_set: bool,
        openai_config: &Option<crate::configuration::GPTConfigs>,
        execution_scripts: &Option<ExecutionScripts>,
        automation_scripts: &Option<AutomationScripts>,
    ) -> Self {
        let page_resource = crate::utils::fetch_page_html(
            &url,
            &client,
            &page,
            wait_for,
            screenshot,
            page_set,
            openai_config,
            execution_scripts,
            automation_scripts,
        )
        .await;
        let mut p = build(url, page_resource);

        // store the chrome page to perform actions like screenshots etc.
        if cfg!(feature = "chrome_store_page") {
            p.chrome_page = Some(page.clone());
        }

        p
    }

    /// Instantiate a new page and gather the links.
    #[cfg(all(feature = "decentralized", not(feature = "headers")))]
    pub async fn new(url: &str, client: &Client) -> Self {
        Self::new_links_only(url, client).await
    }

    /// Instantiate a new page and gather the headers and links.
    #[cfg(all(feature = "decentralized", feature = "headers"))]
    pub async fn new(url: &str, client: &Client) -> Self {
        use crate::serde::Deserialize;
        use bytes::Buf;

        match crate::utils::fetch_page_and_headers(&url, &client).await {
            FetchPageResult::Success(headers, page_content) => {
                let links = match page_content {
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
                    headers: Some(headers),
                    links,
                    ..Default::default()
                }
            }
            FetchPageResult::NoSuccess(headers) => Page {
                headers: Some(headers),
                ..Default::default()
            },
            FetchPageResult::FetchError => Default::default(),
        }
    }

    /// Instantiate a new page and gather the links.
    #[cfg(all(feature = "decentralized"))]
    pub async fn new_links_only(url: &str, client: &Client) -> Self {
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
            ..Default::default()
        }
    }

    #[cfg(not(all(not(feature = "decentralized"), feature = "chrome")))]
    /// Take a screenshot of the page. If the output path is set to None the screenshot will not be saved.
    /// The feature flag `chrome_store_page` is required.
    pub async fn screenshot(
        &self,
        _full_page: bool,
        _omit_background: bool,
        _format: crate::configuration::CaptureScreenshotFormat,
        _quality: Option<i64>,
        _output_path: Option<impl AsRef<std::path::Path>>,
        _clip: Option<crate::configuration::ClipViewport>,
    ) -> Vec<u8> {
        Default::default()
    }

    #[cfg(all(not(feature = "decentralized"), feature = "chrome"))]
    /// Take a screenshot of the page. If the output path is set to None the screenshot will not be saved.
    /// The feature flag `chrome_store_page` is required.
    pub async fn take_screenshot(
        page: &Page,
        full_page: bool,
        omit_background: bool,
        format: crate::configuration::CaptureScreenshotFormat,
        quality: Option<i64>,
        output_path: Option<impl AsRef<std::path::Path>>,
        clip: Option<crate::configuration::ClipViewport>,
    ) -> Vec<u8> {
        match &page.chrome_page {
            Some(chrome_page) => {
                let format =
                    chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat::from(
                        format,
                    );

                let screenshot_configs = chromiumoxide::page::ScreenshotParams::builder()
                    .format(format)
                    .full_page(full_page)
                    .omit_background(omit_background);

                let screenshot_configs = match quality {
                    Some(q) => screenshot_configs.quality(q),
                    _ => screenshot_configs,
                };

                let screenshot_configs = match clip {
                    Some(vp) => screenshot_configs.clip(
                        chromiumoxide::cdp::browser_protocol::page::Viewport::from(vp),
                    ),
                    _ => screenshot_configs,
                };

                if output_path.is_none() {
                    match chrome_page.screenshot(screenshot_configs.build()).await {
                        Ok(v) => {
                            log::debug!("took screenshot: {:?}", page.url);
                            v
                        }
                        Err(e) => {
                            log::error!("failed to took screenshot: {:?} - {:?}", e, page.url);
                            Default::default()
                        }
                    }
                } else {
                    let output_path = match output_path {
                        Some(out) => out.as_ref().to_path_buf(),
                        _ => Default::default(),
                    };

                    match chrome_page
                        .save_screenshot(screenshot_configs.build(), &output_path)
                        .await
                    {
                        Ok(v) => {
                            log::debug!("saved screenshot: {:?}", output_path);
                            v
                        }
                        Err(e) => {
                            log::error!("failed to save screenshot: {:?} - {:?}", e, output_path);
                            Default::default()
                        }
                    }
                }
            }
            _ => Default::default(),
        }
    }

    #[cfg(all(not(feature = "decentralized"), feature = "chrome"))]
    /// Take a screenshot of the page. If the output path is set to None the screenshot will not be saved. The feature flag `chrome_store_page` is required.
    pub async fn screenshot(
        &self,
        full_page: bool,
        omit_background: bool,
        format: crate::configuration::CaptureScreenshotFormat,
        quality: Option<i64>,
        output_path: Option<impl AsRef<std::path::Path>>,
        clip: Option<crate::configuration::ClipViewport>,
    ) -> Vec<u8> {
        // prevent screenshot hangs
        let screenshot_result = tokio::time::timeout(
            tokio::time::Duration::from_secs(30),
            Page::take_screenshot(
                &self,
                full_page,
                omit_background,
                format,
                quality,
                output_path,
                clip,
            ),
        )
        .await;
        match screenshot_result {
            Ok(sb) => sb,
            _ => Default::default(),
        }
    }

    #[cfg(all(feature = "chrome", not(feature = "decentralized")))]
    /// Get the chrome page used. The feature flag `chrome` is required.
    pub fn get_chrome_page(&self) -> Option<&chromiumoxide::Page> {
        self.chrome_page.as_ref()
    }

    #[cfg(all(feature = "chrome", feature = "decentralized"))]
    /// Get the chrome page used. The feature flag `chrome` is required.
    pub fn get_chrome_page(&self) -> Option<&chromiumoxide::Page> {
        None
    }

    #[cfg(all(not(feature = "decentralized"), feature = "chrome"))]
    /// Close the chrome page used. Useful when storing the page with subscription usage. The feature flag `chrome_store_page` is required.
    pub async fn close_page(&mut self) {
        match self.chrome_page.as_mut() {
            Some(page) => {
                let _ = page
                    .execute(chromiumoxide::cdp::browser_protocol::page::CloseParams::default())
                    .await;
            }
            _ => (),
        }
    }

    #[cfg(all(feature = "decentralized", feature = "chrome"))]
    /// Close the chrome page used. Useful when storing the page for subscription usage. The feature flag `chrome_store_page` is required.
    pub async fn close_page(&mut self) {}

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
            Some(u) => u,
            _ => &self.url,
        }
    }

    /// Set the external domains to treat as one
    pub fn set_external(&mut self, external_domains_caseless: Box<HashSet<CaseInsensitiveString>>) {
        self.external_domains_caseless = external_domains_caseless;
    }

    /// Set the html directly of the page
    pub fn set_html_bytes(&mut self, html: Option<Bytes>) {
        self.html = html;
    }

    /// Set the url directly of the page. Useful for transforming the content and rewriting the url.
    #[cfg(not(feature = "decentralized"))]
    pub fn set_url(&mut self, url: String) {
        self.url = url;
    }

    /// Set the url directly parsed url of the page. Useful for transforming the content and rewriting the url.
    #[cfg(not(feature = "decentralized"))]
    pub fn set_url_parsed(&mut self, url_parsed: Url) {
        self.base = Some(url_parsed);
    }

    /// Parsed URL getter for page.
    #[cfg(not(feature = "decentralized"))]
    pub fn get_url_parsed(&self) -> &Option<Url> {
        &self.base
    }

    /// Parsed URL getter for page.
    #[cfg(feature = "decentralized")]
    pub fn get_url_parsed(&self) -> &Option<Url> {
        &None
    }

    /// Take the parsed url.
    #[cfg(not(feature = "decentralized"))]
    pub fn take_url(&mut self) -> Option<Url> {
        self.base.take()
    }

    /// Take the parsed url.
    #[cfg(feature = "decentralized")]
    pub fn take_url(&mut self) -> Option<Url> {
        None
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

    /// Set the language for the page.
    pub fn detect_language(&mut self) {
        if self.lang.is_none() {
            match self.html.as_ref() {
                Some(html) => {
                    if !html.is_empty() {
                        match auto_encoder::detect_language(html) {
                            Some(lang) => {
                                self.lang.replace(lang);
                            }
                            _ => (),
                        }
                    }
                }
                _ => (),
            }
        }
    }

    /// Html getter for bytes on the page as string.
    pub fn get_html(&self) -> String {
        match self.html.as_ref() {
            Some(html) => {
                if html.is_empty() {
                    Default::default()
                } else {
                    let language = self.lang.as_ref();
                    match language {
                        Some(l) => encode_bytes_from_language(html, &l),
                        _ => encode_bytes_from_language(html, &""),
                    }
                }
            }
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

    /// Html getter for getting the content with proper encoding. Pass in a proper encoding label like SHIFT_JIS. This fallsback to get_html without the [encoding] flag enabled.
    #[cfg(feature = "encoding")]
    pub fn get_html_encoded(&self, label: &str) -> String {
        get_html_encoded(&self.html, label)
    }

    /// Html getter for getting the content with proper encoding. Pass in a proper encoding label like SHIFT_JIS. This fallsback to get_html without the `encoding` flag enabled.
    #[cfg(not(feature = "encoding"))]
    pub fn get_html_encoded(&self, _label: &str) -> String {
        self.get_html()
    }

    /// Get the elasped duration of the page since scraped.
    #[cfg(all(feature = "time", not(feature = "decentralized")))]
    pub fn get_duration_elasped(&self) -> Duration {
        self.duration.elapsed()
    }

    /// Validate link and push into the map
    pub fn push_link<A: PartialEq + Eq + std::hash::Hash + From<String>>(
        &self,
        href: &str,
        map: &mut HashSet<A>,
        base_domain: &CompactString,
        parent_host: &CompactString,
        parent_host_scheme: &CompactString,
        base_input_domain: &CompactString,
    ) {
        match self.abs_path(href) {
            Some(mut abs) => {
                let host_name = abs.host_str();
                let mut can_process =
                    parent_host_match(host_name, base_domain, parent_host, base_input_domain);
                let mut external_domain = false;

                if !can_process && host_name.is_some() && !self.external_domains_caseless.is_empty()
                {
                    can_process = self
                        .external_domains_caseless
                        .contains::<CaseInsensitiveString>(&host_name.unwrap_or_default().into())
                        || self
                            .external_domains_caseless
                            .contains::<CaseInsensitiveString>(&CASELESS_WILD_CARD);
                    external_domain = can_process;
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
                        && (base_domain.is_empty()
                            || external_domain
                            || base_domain.as_str() == domain_name(&abs))
                    {
                        map.insert(abs.as_str().to_string().into());
                    }
                }
            }
            _ => (),
        }
    }

    /// Find the links as a stream using string resource validation for XML files
    pub async fn links_stream_xml_links_stream_base<
        A: PartialEq + Eq + std::hash::Hash + From<String>,
    >(
        &self,
        selectors: &RelativeSelectors,
        xml: &str,
        map: &mut HashSet<A>,
    ) {
        use quick_xml::events::Event;
        use quick_xml::reader::NsReader;

        let mut reader = NsReader::from_reader(xml.as_bytes());

        reader.config_mut().trim_text(true);

        let mut buf = Vec::new();

        let parent_host = &selectors.1[0];
        let parent_host_scheme = &selectors.1[1];
        let base_input_domain = &selectors.2;

        let mut is_link_tag = false;

        loop {
            match reader.read_event_into_async(&mut buf).await {
                Ok(e) => match e {
                    Event::Start(e) => {
                        let (_, local) = reader.resolve_element(e.name());
                        match local.as_ref() {
                            b"link" => {
                                is_link_tag = true;
                            }
                            _ => (),
                        }
                    }
                    Event::Text(e) => {
                        if is_link_tag {
                            match e.unescape() {
                                Ok(v) => {
                                    self.push_link(
                                        &v,
                                        map,
                                        &selectors.0,
                                        parent_host,
                                        parent_host_scheme,
                                        base_input_domain,
                                    );
                                }
                                _ => (),
                            }
                        }
                    }
                    Event::End(ref e) => {
                        let (_, local) = reader.resolve_element(e.name());
                        match local.as_ref() {
                            b"link" => {
                                is_link_tag = false;
                            }
                            _ => (),
                        }
                    }
                    Event::Eof => {
                        break;
                    }
                    _ => (),
                },
                _ => break,
            }
            buf.clear();
        }
    }

    /// Find the links as a stream using string resource validation
    #[inline(always)]
    #[cfg(all(not(feature = "decentralized")))]
    pub async fn links_stream_base<A: PartialEq + Eq + std::hash::Hash + From<String>>(
        &self,
        selectors: &RelativeSelectors,
        html: &str,
    ) -> HashSet<A> {
        let mut map = HashSet::new();

        if html.starts_with("<?xml") {
            self.links_stream_xml_links_stream_base(selectors, html, &mut map)
                .await;
        } else {
            let html = Box::new(Html::parse_fragment(html));
            let mut stream = tokio_stream::iter(html.tree);

            let parent_host = &selectors.1[0];
            let parent_host_scheme = &selectors.1[1];
            let base_input_domain = &selectors.2;

            while let Some(node) = stream.next().await {
                if let Some(element) = node.as_element() {
                    let element_name = element.name();

                    if element_name == "a" {
                        match element.attr("href") {
                            Some(href) => {
                                self.push_link(
                                    href,
                                    &mut map,
                                    &selectors.0,
                                    parent_host,
                                    parent_host_scheme,
                                    base_input_domain,
                                );
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
    #[cfg(all(not(feature = "decentralized"), not(feature = "full_resources"),))]
    pub async fn links_stream<A: PartialEq + Eq + std::hash::Hash + From<String>>(
        &self,
        selectors: &RelativeSelectors,
    ) -> HashSet<A> {
        if auto_encoder::is_binary_file(self.get_html_bytes_u8()) {
            return Default::default();
        }
        self.links_stream_base(selectors, &self.get_html()).await
    }

    /// Find the links as a stream using string resource validation
    #[cfg(all(
        not(feature = "decentralized"),
        not(feature = "full_resources"),
        feature = "smart"
    ))]
    #[inline(always)]
    pub async fn links_stream_smart<
        A: PartialEq + std::fmt::Debug + Eq + std::hash::Hash + From<String>,
    >(
        &self,
        selectors: &RelativeSelectors,
        browser: &std::sync::Arc<chromiumoxide::Browser>,
        configuration: &crate::configuration::Configuration,
        context_id: &Option<chromiumoxide::cdp::browser_protocol::browser::BrowserContextId>,
    ) -> HashSet<A> {
        let mut map = HashSet::new();
        let html = self.get_html();

        if html.starts_with("<?xml") {
            self.links_stream_xml_links_stream_base(selectors, &html, &mut map)
                .await;
        } else {
            let base_domain = &selectors.0;
            let parent_frags = &selectors.1; // todo: allow mix match tpt
            let base_input_domain = &selectors.2;

            let parent_host = &parent_frags[0];
            let parent_host_scheme = &parent_frags[1];

            let html = Box::new(Html::parse_document(&html));
            let (tx, rx) = tokio::sync::oneshot::channel();

            let mut stream = tokio_stream::iter(html.tree);
            let mut rerender = false;
            let mut static_app = false;

            while let Some(node) = stream.next().await {
                if let Some(element) = node.as_element() {
                    let element_name = element.name();

                    // check scripts for non SSR/SSG pages. We need to check for lazy loading elements done by the static app for re-rendering.
                    if !static_app && element_name == "script" {
                        match element.attr("src") {
                            Some(src) => {
                                if src.starts_with("/") {
                                    if src.starts_with("/_next/static/chunks/pages/")
                                        || src.starts_with("/webpack-runtime-")
                                        || element.attr("id") == Some("gatsby-chunk-mapping")
                                    {
                                        static_app = true;
                                        continue;
                                    }

                                    match self.abs_path(src) {
                                        Some(abs) => {
                                            match abs
                                                .path_segments()
                                                .ok_or_else(|| "cannot be base")
                                            {
                                                Ok(mut paths) => {
                                                    while let Some(p) = paths.next() {
                                                        // todo: get the path last before None instead of checking for ends_with
                                                        if p.ends_with(".js")
                                                            && JS_FRAMEWORK_ASSETS.contains(&p)
                                                        {
                                                            rerender = true;
                                                        } else {
                                                            match node.as_text() {
                                                                Some(text) => {
                                                                    lazy_static! {
                                                                        static ref DOM_WATCH_METHODS: regex::RegexSet = {
                                                                            let set = unsafe {
                                                                                regex::RegexSet::new(&[
                                                                                r"/.createElementNS/gm",
                                                                                r"/.removeChild/gm",
                                                                                r"/.insertBefore/gm",
                                                                                r"/.createElement/gm",
                                                                                r"/.setAttribute/gm",
                                                                                r"/.createTextNode/gm",
                                                                                r"/.replaceChildren/gm",
                                                                                r"/.prepend/gm",
                                                                                r"/.append/gm",
                                                                                r"/.appendChild/gm",
                                                                                r"/.write/gm",
                                                                            ])
                                                                            .unwrap_unchecked()
                                                                            };

                                                                            set
                                                                        };
                                                                    }
                                                                    rerender = DOM_WATCH_METHODS
                                                                        .is_match(text);
                                                                }
                                                                _ => (),
                                                            }
                                                        }
                                                    }
                                                }
                                                _ => (),
                                            };

                                            if rerender {
                                                // we should re-use the html content instead with events.
                                                let uu = self.get_html();
                                                let browser = browser.to_owned();
                                                let configuration = configuration.clone();
                                                let target_url = self.url.clone();
                                                let context_id = context_id.clone();

                                                tokio::task::spawn(async move {
                                                    // we need to use about:blank here since we set the HTML content directly
                                                    match crate::features::chrome::attempt_navigation("about:blank", &browser, &configuration.request_timeout, &context_id).await {
                                                        Ok(new_page) => {
                                                            match configuration
                                                                .evaluate_on_new_document
                                                            {
                                                                Some(ref script) => {
                                                                    if configuration.fingerprint {
                                                                        let _ = new_page
                                                                            .evaluate_on_new_document(string_concat!(
                                                                                crate::features::chrome::FP_JS,
                                                                                script.as_str()
                                                                            ))
                                                                            .await;
                                                                    } else {
                                                                        let _ =
                                                                            new_page.evaluate_on_new_document(script.as_str()).await;
                                                                    }
                                                                }
                                                                _ => {
                                                                    if configuration.fingerprint {
                                                                        let _ = new_page
                                                                            .evaluate_on_new_document(crate::features::chrome::FP_JS)
                                                                            .await;
                                                                    }
                                                                }
                                                            }

                                                            let new_page =
                                                            crate::features::chrome::configure_browser(
                                                                new_page,
                                                                &configuration,
                                                            )
                                                            .await;

                                                            if cfg!(feature = "chrome_stealth")
                                                                || configuration.stealth_mode
                                                            {
                                                                let _ = new_page
                                                                    .enable_stealth_mode_with_agent(
                                                                        &if configuration
                                                                            .user_agent
                                                                            .is_some()
                                                                        {
                                                                            match configuration
                                                                            .user_agent
                                                                            .as_ref() {
                                                                                Some(agent) => {
                                                                                    if !agent.contains("Chrome")  {
                                                                                        if cfg!(feature = "ua_generator") {
                                                                                            crate::configuration::get_ua(true)
                                                                                        } else {
                                                                                           ""
                                                                                        }
                                                                                    } else {
                                                                                        agent
                                                                                    }
                                                                                }
                                                                                _ => ""
                                                                            }
                                                                        } else {
                                                                            ""
                                                                        },
                                                                    );
                                                            }

                                                            let page_resource =
                                                            crate::utils::fetch_page_html_chrome_base(
                                                                &uu,
                                                                &new_page,
                                                                true,
                                                                false,
                                                                &Some(crate::configuration::WaitFor::new(
                                                                    Some(
                                                                        core::time::Duration::from_secs(
                                                                            120,
                                                                        ), // default a duration for smart handling. (maybe expose later on.)
                                                                    ),
                                                                    None,
                                                                    true,
                                                                    true,
                                                                    None,
                                                                )),
                                                                &configuration.screenshot,
                                                                false,
                                                                &configuration.openai_config,
                                                                Some(&target_url),
                                                                &configuration
                                                                        .execution_scripts,
                                                                        &configuration
                                                                        .automation_scripts
                                                            )
                                                            .await;

                                                            match page_resource {
                                                                Ok(resource) => {
                                                                    if let Err(_) =
                                                                        tx.send(resource)
                                                                    {
                                                                        crate::utils::log(
                                                                            "the receiver dropped",
                                                                            "",
                                                                        );
                                                                    }
                                                                }
                                                                _ => (),
                                                            };
                                                        }
                                                        _ => (),
                                                    }
                                                });

                                                break;
                                            }
                                        }
                                        _ => (),
                                    }
                                }
                            }
                            _ => (),
                        }
                    }

                    if element_name == "a" {
                        // add fullresources?
                        match element.attr("href") {
                            Some(href) => match self.abs_path(href) {
                                Some(mut abs) => {
                                    let host_name = abs.host_str();
                                    let mut can_process = parent_host_match(
                                        host_name,
                                        &base_domain,
                                        parent_host,
                                        base_input_domain,
                                    );

                                    if can_process {
                                        if abs.scheme() != parent_host_scheme.as_str() {
                                            let _ = abs.set_scheme(parent_host_scheme.as_str());
                                        }
                                        let hchars = abs.path();

                                        if let Some(position) = hchars.rfind('.') {
                                            let resource_ext = &hchars[position + 1..hchars.len()];

                                            if !ONLY_RESOURCES.contains::<CaseInsensitiveString>(
                                                &resource_ext.into(),
                                            ) {
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
                            },
                            _ => (),
                        };
                    }
                }
            }

            if rerender {
                drop(stream);
                match rx.await {
                    Ok(v) => {
                        let extended_map = self
                            .links_stream_base::<A>(
                                selectors,
                                &match v.content {
                                    Some(h) => String::from_utf8_lossy(&h).to_string(),
                                    _ => Default::default(),
                                },
                            )
                            .await;
                        map.extend(extended_map)
                    }
                    Err(e) => {
                        crate::utils::log("receiver error", e.to_string());
                    }
                };
            }
        }

        map
    }

    /// Find the links as a stream using string resource validation
    #[inline(always)]
    pub async fn links_stream_full_resource<A: PartialEq + Eq + std::hash::Hash + From<String>>(
        &self,
        selectors: &RelativeSelectors,
    ) -> HashSet<A> {
        let mut map = HashSet::new();
        let html = self.get_html();

        if html.starts_with("<?xml") {
            self.links_stream_xml_links_stream_base(selectors, &html, &mut map)
                .await;
        } else {
            let html = Box::new(crate::packages::scraper::Html::parse_document(&html));
            let mut stream = tokio_stream::iter(html.tree);

            let base_domain = &selectors.0;
            let base_input_domain = &selectors.2;
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
                        Some(href) => match self.abs_path(href) {
                            Some(mut abs) => {
                                let host_name = abs.host_str();
                                let mut can_process = parent_host_match(
                                    host_name,
                                    base_domain,
                                    parent_host,
                                    base_input_domain,
                                );

                                let mut external_domain = false;

                                if !can_process
                                    && host_name.is_some()
                                    && !self.external_domains_caseless.is_empty()
                                {
                                    can_process = self
                                        .external_domains_caseless
                                        .contains::<CaseInsensitiveString>(
                                        &host_name.unwrap_or_default().into(),
                                    ) || self
                                        .external_domains_caseless
                                        .contains::<CaseInsensitiveString>(
                                        &CASELESS_WILD_CARD,
                                    );
                                    external_domain = can_process;
                                }

                                if can_process {
                                    if abs.scheme() != parent_host_scheme.as_str() {
                                        let _ = abs.set_scheme(parent_host_scheme.as_str());
                                    }

                                    let h = abs.as_str();

                                    if can_process
                                        && (base_domain.is_empty()
                                            || external_domain
                                            || base_domain.as_str() == domain_name(&abs))
                                    {
                                        map.insert(h.to_string().into());
                                    }
                                }
                            }
                            _ => (),
                        },
                        _ => (),
                    };
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
        selectors: &RelativeSelectors,
    ) -> HashSet<A> {
        if auto_encoder::is_binary_file(self.get_html_bytes_u8()) {
            return Default::default();
        }
        self.links_stream_full_resource(selectors).await
    }

    #[inline(always)]
    #[cfg(feature = "decentralized")]
    /// Find the links as a stream using string resource validation
    pub async fn links_stream<A: PartialEq + Eq + std::hash::Hash + From<String>>(
        &self,
        _: &RelativeSelectors,
    ) -> HashSet<A> {
        Default::default()
    }

    /// Find all href links and return them using CSS selectors.
    #[cfg(not(feature = "decentralized"))]
    #[inline(always)]
    pub async fn links(&self, selectors: &RelativeSelectors) -> HashSet<CaseInsensitiveString> {
        match self.html.is_some() {
            false => Default::default(),
            true => self.links_stream::<CaseInsensitiveString>(selectors).await,
        }
    }

    /// Find all href links and return them using CSS selectors gathering all resources.
    #[inline(always)]
    pub async fn links_full(
        &self,
        selectors: &RelativeSelectors,
    ) -> HashSet<CaseInsensitiveString> {
        match self.html.is_some() {
            false => Default::default(),
            true => {
                if auto_encoder::is_binary_file(self.get_html_bytes_u8()) {
                    return Default::default();
                }
                self.links_stream_full_resource::<CaseInsensitiveString>(&selectors)
                    .await
            }
        }
    }

    /// Find all href links and return them using CSS selectors.
    #[cfg(all(not(feature = "decentralized"), feature = "smart"))]
    #[inline(always)]
    pub async fn smart_links(
        &self,
        selectors: &RelativeSelectors,
        page: &std::sync::Arc<chromiumoxide::Browser>,
        configuration: &crate::configuration::Configuration,
        context_id: &Option<chromiumoxide::cdp::browser_protocol::browser::BrowserContextId>,
    ) -> HashSet<CaseInsensitiveString> {
        match self.html.is_some() {
            false => Default::default(),
            true => {
                if auto_encoder::is_binary_file(self.get_html_bytes_u8()) {
                    return Default::default();
                }
                self.links_stream_smart::<CaseInsensitiveString>(
                    &selectors,
                    page,
                    configuration,
                    context_id,
                )
                .await
            }
        }
    }

    /// Find all href links and return them using CSS selectors.
    #[cfg(feature = "decentralized")]
    #[inline(always)]
    pub async fn links(&self, _: &RelativeSelectors) -> HashSet<CaseInsensitiveString> {
        self.links.to_owned()
    }

    /// Convert a URL to its absolute path without any fragments or params.
    #[inline]
    #[cfg(not(feature = "decentralized"))]
    fn abs_path(&self, href: &str) -> Option<Url> {
        match &self.base {
            Some(b) => Some(convert_abs_path(&b, href)),
            _ => None,
        }
    }

    /// Convert a URL to its absolute path without any fragments or params. [unused in the worker atm by default all is returned]
    #[inline(never)]
    #[cfg(feature = "decentralized")]
    fn abs_path(&self, href: &str) -> Option<Url> {
        match Url::parse(&href) {
            Ok(u) => Some(convert_abs_path(&u, href)),
            _ => None,
        }
    }
}

/// Get the content with proper encoding. Pass in a proper encoding label like SHIFT_JIS.
#[cfg(feature = "encoding")]
fn encode_bytes(html: &Bytes, label: &str) -> String {
    auto_encoder::encode_bytes(html, label)
}

#[cfg(feature = "encoding")]
/// Get the content with proper encoding from a language. Pass in a proper language like "jp". This does nothing without the "encoding" flag.
pub fn encode_bytes_from_language(html: &[u8], language: &str) -> String {
    auto_encoder::encode_bytes_from_language(html, language)
}

/// Get the content with proper encoding from a language. Pass in a proper language like "jp". This does nothing without the "encoding" flag.
#[cfg(not(feature = "encoding"))]
pub fn encode_bytes_from_language(html: &[u8], _language: &str) -> String {
    String::from_utf8_lossy(html).to_string()
}

/// Get the content with proper encoding. Pass in a proper encoding label like SHIFT_JIS.
#[cfg(feature = "encoding")]
pub fn get_html_encoded(html: &Option<Bytes>, label: &str) -> String {
    match html.as_ref() {
        Some(html) => encode_bytes(html, label),
        _ => Default::default(),
    }
}

#[cfg(not(feature = "encoding"))]
/// Get the content with proper encoding. Pass in a proper encoding label like SHIFT_JIS.
pub fn get_html_encoded(html: &Option<Bytes>, _label: &str) -> String {
    match html {
        Some(b) => String::from_utf8_lossy(b).to_string(),
        _ => Default::default(),
    }
}

#[cfg(test)]
pub const TEST_AGENT_NAME: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));

#[cfg(all(
    feature = "headers",
    not(feature = "decentralized"),
    not(feature = "cache"),
))]
#[tokio::test]
async fn test_headers() {
    use reqwest::header::HeaderName;
    use reqwest::header::HeaderValue;

    let client = Client::builder()
        .user_agent(TEST_AGENT_NAME)
        .build()
        .unwrap();

    let link_result = "https://choosealicense.com/";
    let page: Page = Page::new_page(link_result, &client).await;

    let headers = page.headers.expect("There should be some headers!");

    assert_eq!(
        headers
            .get(HeaderName::from_static("server"))
            .expect("There should be a server header value!"),
        HeaderValue::from_static("GitHub.com")
    );

    assert_eq!(
        headers
            .get(HeaderName::from_static("content-type"))
            .expect("There should be a content-type value!"),
        HeaderValue::from_static("text/html; charset=utf-8")
    );
}

#[cfg(all(
    not(feature = "decentralized"),
    not(feature = "chrome"),
    not(feature = "cache")
))]
#[tokio::test]
async fn parse_links() {
    let client = Client::builder()
        .user_agent(TEST_AGENT_NAME)
        .build()
        .unwrap();

    let link_result = "https://choosealicense.com/";
    let page = Page::new(link_result, &client).await;
    let selector = get_page_selectors(link_result, false, false);
    let links = page.links(&selector.unwrap()).await;

    assert!(
        links.contains::<CaseInsensitiveString>(&"https://choosealicense.com/about/".into()),
        "Could not find {}. Theses URLs was found {:?}",
        page.get_url(),
        &links
    );
}

#[cfg(all(
    not(feature = "decentralized"),
    not(feature = "chrome"),
    not(feature = "cache")
))]
#[tokio::test]
async fn test_status_code() {
    let client = Client::builder()
        .user_agent(TEST_AGENT_NAME)
        .build()
        .unwrap();
    let link_result = "https://choosealicense.com/does-not-exist";
    let page: Page = Page::new(link_result, &client).await;

    assert_eq!(page.status_code.as_u16(), 404);
}

#[cfg(all(
    not(feature = "decentralized"),
    not(feature = "chrome"),
    not(feature = "cache")
))]
#[tokio::test]
async fn test_abs_path() {
    let client = Client::builder()
        .user_agent(TEST_AGENT_NAME)
        .build()
        .expect("a valid agent");
    let link_result = "https://choosealicense.com/";
    let page: Page = Page::new(link_result, &client).await;

    assert_eq!(
        page.abs_path("/page").expect("a valid url"),
        Url::parse("https://choosealicense.com/page").expect("a valid url")
    );
    assert_eq!(
        page.abs_path("/page?query=keyword").expect("a valid url"),
        Url::parse("https://choosealicense.com/page?query=keyword").expect("a valid url")
    );
    assert_eq!(
        page.abs_path("/page#hash").expect("a valid url"),
        Url::parse("https://choosealicense.com/page").expect("a valid url")
    );
    assert_eq!(
        page.abs_path("/page?query=keyword#hash")
            .expect("a valid url"),
        Url::parse("https://choosealicense.com/page?query=keyword").unwrap()
    );
    assert_eq!(
        page.abs_path("#hash").unwrap(),
        Url::parse("https://choosealicense.com/").expect("a valid url")
    );
    assert_eq!(
        page.abs_path("tel://+212 3456").unwrap(),
        Url::parse("https://choosealicense.com/").expect("a valid url")
    );
}

#[cfg(all(feature = "time", not(feature = "decentralized")))]
#[tokio::test]
async fn test_duration() {
    let client = Client::builder()
        .user_agent(TEST_AGENT_NAME)
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
