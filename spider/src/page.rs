use crate::compact_str::CompactString;

#[cfg(all(feature = "chrome", not(feature = "decentralized")))]
use crate::configuration::{AutomationScripts, ExecutionScripts};
use crate::utils::log;
use crate::utils::PageResponse;
use crate::CaseInsensitiveString;
use crate::Client;
use crate::RelativeSelectors;
use auto_encoder::auto_encode_bytes;
use bytes::Bytes;
use hashbrown::HashSet;
use lol_html::Settings;
use regex::bytes::Regex;
use reqwest::StatusCode;
use tokio::time::Duration;

#[cfg(all(feature = "time", not(feature = "decentralized")))]
use tokio::time::Instant;

#[cfg(all(feature = "decentralized", feature = "headers"))]
use crate::utils::FetchPageResult;
#[cfg(not(feature = "decentralized"))]
use tokio_stream::StreamExt;
use url::Url;

lazy_static! {
    /// Wildcard match all domains.
    static ref CASELESS_WILD_CARD: CaseInsensitiveString = CaseInsensitiveString::new("*");
    static ref SSG_CAPTURE: Regex =  Regex::new(r#""(.*?)""#).unwrap();
    static ref GATSBY: Option<String> =  Some("gatsby-chunk-mapping".into());
}

#[cfg(any(feature = "smart", feature = "chrome_intercept"))]
lazy_static! {
    /// popular js frameworks and libs
    pub static ref JS_FRAMEWORK_ASSETS: phf::Set<&'static str> = {
        phf::phf_set! {
            "jquery.min.js", "jquery.qtip.min.js", "jquery.js", "angular.js", "jquery.slim.js", "react.development.js", "react-dom.development.js", "react.production.min.js", "react-dom.production.min.js",
            "vue.global.js", "vue.global.prod.js", "vue.esm-browser.js", "vue.js", "bootstrap.min.js", "bootstrap.bundle.min.js", "bootstrap.esm.min.js", "d3.min.js", "d3.js", "material-components-web.min.js",
            "otSDKStub.js", "clipboard.min.js", "moment.js", "moment.min.js", "dexie.js", "layui.js"
        }
    };
}

#[cfg(all(
    not(feature = "decentralized"),
    not(feature = "full_resources"),
    feature = "smart"
))]
lazy_static! {
    static ref DOM_WATCH_METHODS: aho_corasick::AhoCorasick = {
        let patterns = &[
            ".createElementNS",
            ".removeChild",
            ".insertBefore",
            ".createElement",
            ".setAttribute",
            ".createTextNode",
            ".replaceChildren",
            ".prepend",
            ".append",
            ".appendChild",
            ".write",
        ];

        aho_corasick::AhoCorasick::new(patterns).unwrap()
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
            "react-dom.production.min.js", "vue.global.js", "vue.global.prod.js", "vue.esm-browser.js", "vue.js",
            "bootstrap.min.js", "bootstrap.bundle.min.js", "bootstrap.esm.min.js", "d3.min.js",
            "d3.js", "layui.js",
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
        let mut m: HashSet<CaseInsensitiveString> = HashSet::with_capacity(28);

        m.extend([
            "html", "htm", "shtml", "asp", "aspx", "php", "jps", "jpsx", "jsp", "cfm", "xhtml", "rhtml", "phtml", "erb",
            // handle .. prefix for urls ending with an extra ending
            ".html", ".htm", ".shtml", ".asp", ".aspx", ".php", ".jps", ".jpsx", ".jsp", ".cfm", ".xhtml", ".rhtml", ".phtml", ".erb",
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
    html: Option<Box<Bytes>>,
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
    /// The request should retry
    pub should_retry: bool,
    /// A WAF was found on the page.
    pub waf_check: bool,
}

/// Represent a page visited. This page contains HTML scraped with [scraper](https://crates.io/crates/scraper).
#[cfg(feature = "decentralized")]
#[derive(Debug, Clone, Default)]
pub struct Page {
    /// The bytes of the resource.
    html: Option<Box<Bytes>>,
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
    /// The request should retry
    pub should_retry: bool,
    /// A WAF was found on the page.
    pub waf_check: bool,
}

/// Validate link and push into the map
pub fn push_link<A: PartialEq + Eq + std::hash::Hash + From<String>>(
    base: &Option<Url>,
    href: &str,
    map: &mut HashSet<A>,
    base_domain: &CompactString,
    parent_host: &CompactString,
    parent_host_scheme: &CompactString,
    base_input_domain: &CompactString,
    sub_matcher: &CompactString,
    external_domains_caseless: &Box<HashSet<CaseInsensitiveString>>,
) {
    if let Some(b) = base {
        let mut abs = convert_abs_path(&b, href);
        let scheme = abs.scheme();

        if scheme == "https" || scheme == "http" {
            let host_name = abs.host_str();
            let mut can_process = parent_host_match(
                host_name,
                base_domain,
                parent_host,
                base_input_domain,
                sub_matcher,
            );

            if !can_process && host_name.is_some() && !external_domains_caseless.is_empty() {
                can_process = external_domains_caseless
                    .contains::<CaseInsensitiveString>(&host_name.unwrap_or_default().into())
                    || external_domains_caseless
                        .contains::<CaseInsensitiveString>(&CASELESS_WILD_CARD);
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

                if can_process {
                    map.insert(abs.as_str().to_string().into());
                }
            }
        }
    }
}

/// get the clean domain name
pub fn domain_name(domain: &Url) -> &str {
    match domain.host_str() {
        Some(host) => host,
        _ => "",
    }
}

/// Convert to absolute path
#[inline]
pub fn convert_abs_path(base: &Url, href: &str) -> Url {
    match base.join(&href) {
        Ok(mut joined) => {
            joined.set_fragment(None);
            joined
        }
        Err(e) => {
            log("URL Parse Error: ", e.to_string());
            base.clone()
        }
    }
}

/// extract the valid domains from a url.
fn extract_root_domain(domain: &str) -> &str {
    let parts: Vec<&str> = domain.split('.').collect();

    if parts.len() >= 3 {
        let start_index = parts.len() - 2;
        if let Some(start_pos) = domain.find(parts[start_index]) {
            &domain[start_pos..]
        } else {
            domain
        }
    } else if parts.len() == 2 {
        &parts[0]
    } else {
        domain
    }
}

/// check for subdomain matches
fn is_subdomain(subdomain: &str, domain: &str) -> bool {
    extract_root_domain(subdomain) == extract_root_domain(domain)
}

/// validation to match a domain to parent host and the top level redirect for the crawl 'parent_host' and 'base_host' being the input start domain.
pub fn parent_host_match(
    host_name: Option<&str>,
    base_domain: &str,           // the base domain input
    parent_host: &CompactString, // the main parent host
    base_host: &CompactString,   // the host before any redirections - entered in Website::new()
    sub_matcher: &CompactString, // matches TLDS or subdomains. If tlds the domain is stripped.
) -> bool {
    match host_name {
        Some(host) => {
            let exact_match = parent_host.eq(&host) || base_host.eq(&host);

            if base_domain.is_empty() {
                exact_match
            } else {
                let valid = exact_match
                    || is_subdomain(host, &parent_host)
                    || is_subdomain(host, &sub_matcher);

                valid
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

        let dname = if tld {
            extract_root_domain(dname)
        } else {
            dname
        };

        (
            dname.into(), // match for tlds or subdomains
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

#[cfg(not(feature = "decentralized"))]
/// Is the resource valid?
pub fn validate_empty(content: &Option<Box<Bytes>>, is_success: bool) -> bool {
    match content {
        Some(ref content) => {
            if content.is_empty() || content.starts_with(b"<html><head></head><body></body></html>") || is_success &&
            content.starts_with(b"<html>\r\n<head>\r\n<META NAME=\"robots\" CONTENT=\"noindex,nofollow\">\r\n<script src=\"/") && 
            content.ends_with(b"\">\r\n</script>\r\n<body>\r\n</body></html>\r\n") {
                false
            } else {
                true
            }
        }
        _ => false,
    }
}

/// Instantiate a new page without scraping it (used for testing purposes).
#[cfg(not(feature = "decentralized"))]
pub fn build(url: &str, res: PageResponse) -> Page {
    let success = res.status_code.is_success();
    let resource_found = validate_empty(&res.content, success);
    let mut should_retry = resource_found && !success
        || res.status_code.is_server_error()
        || res.status_code == StatusCode::TOO_MANY_REQUESTS
        || res.status_code == StatusCode::FORBIDDEN
        || res.status_code == StatusCode::REQUEST_TIMEOUT;

    Page {
        html: if resource_found { res.content } else { None },
        #[cfg(feature = "headers")]
        headers: res.headers,
        #[cfg(feature = "cookies")]
        cookies: res.cookies,
        base: if !url.is_empty() {
            match Url::parse(url) {
                Ok(u) => Some(u),
                _ => None,
            }
        } else {
            None
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
                Err(er) => {
                    if er.is_status() || er.is_connect() || er.is_timeout() {
                        if er.to_string().contains("ENOTFOUND") {
                            should_retry = false;
                        } else {
                            should_retry = true;
                        }
                    }
                    Some(er.to_string())
                }
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
        should_retry,
        waf_check: res.waf_check,
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

/// Settings for streaming rewriter
#[derive(Debug, Default, Clone, Copy)]
pub struct PageLinkBuildSettings {
    /// If the SSG build is in progress.
    pub ssg_build: bool,
    /// If full resources should be included.
    pub full_resources: bool,
    /// TLD handling resources.
    pub tld: bool,
    /// Subdomain handling resources.
    pub subdomains: bool,
}

impl PageLinkBuildSettings {
    /// New build link settings.
    pub fn new(ssg_build: bool, full_resources: bool) -> Self {
        Self {
            ssg_build,
            full_resources,
            ..Default::default()
        }
    }

    /// New build full link settings.
    pub fn new_full(ssg_build: bool, full_resources: bool, subdomains: bool, tld: bool) -> Self {
        Self {
            ssg_build,
            full_resources,
            subdomains,
            tld,
        }
    }
}

impl Page {
    /// Instantiate a new page and gather the html repro of standard fetch_page_html.
    pub async fn new_page(url: &str, client: &Client) -> Self {
        let page_resource: PageResponse = crate::utils::fetch_page_html_raw(url, client).await;
        build(url, page_resource)
    }

    /// New page with rewriter
    pub async fn new_page_streaming<
        A: PartialEq + Eq + Sync + Send + Clone + Default + std::hash::Hash + From<String>,
    >(
        url: &str,
        client: &Client,
        only_html: bool,
        mut selectors: &mut RelativeSelectors,
        external_domains_caseless: &Box<HashSet<CaseInsensitiveString>>,
        r_settings: &PageLinkBuildSettings,
        mut map: &mut hashbrown::HashSet<A>,
        ssg_map: Option<&mut hashbrown::HashSet<A>>,
        prior_domain: &Option<Box<Url>>,
        mut domain_parsed: &mut Option<Box<Url>>,
    ) -> Self {
        use crate::utils::{
            handle_response_bytes_writer, modify_selectors, setup_default_response,
            AllowedDomainTypes,
        };

        let page_response = match client.get(url).send().await {
            Ok(res) if res.status().is_success() => {
                let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
                let mut senders: Option<(
                    tokio::sync::mpsc::UnboundedSender<String>,
                    tokio::sync::mpsc::UnboundedReceiver<String>,
                )> = None;

                let base = match Url::parse(url) {
                    Ok(u) => Some(u),
                    _ => None,
                };

                if url != res.url().as_str() {
                    let domain = res.url().as_str();
                    let mut url = Box::new(CaseInsensitiveString::new(&url));

                    modify_selectors(
                        &prior_domain,
                        domain,
                        &mut domain_parsed,
                        &mut url,
                        &mut selectors,
                        AllowedDomainTypes::new(r_settings.subdomains, r_settings.tld),
                    );
                };

                let parent_host = &selectors.1[0];
                // the host schemes
                let parent_host_scheme = &selectors.1[1];
                let base_input_domain = &selectors.2; // the domain after redirects
                let sub_matcher = &selectors.0;

                // let prior_domain = self.domain_parsed.take();

                let external_domains_caseless = external_domains_caseless.clone();

                let base_links_settings = if r_settings.full_resources {
                    lol_html::element!("a[href],script[src],link[href]", |el| {
                        let attribute = if el.tag_name() == "script" {
                            "src"
                        } else {
                            "href"
                        };
                        if let Some(href) = el.get_attribute(attribute) {
                            push_link(
                                &base,
                                &href,
                                &mut map,
                                &selectors.0,
                                parent_host,
                                parent_host_scheme,
                                base_input_domain,
                                sub_matcher,
                                &external_domains_caseless,
                            );
                        }
                        Ok(())
                    })
                } else {
                    lol_html::element!("a[href]", |el| {
                        if let Some(href) = el.get_attribute("href") {
                            push_link(
                                &base,
                                &href,
                                &mut map,
                                &selectors.0,
                                parent_host,
                                parent_host_scheme,
                                base_input_domain,
                                sub_matcher,
                                &external_domains_caseless,
                            );
                        }
                        Ok(())
                    })
                };

                let mut element_content_handlers = vec![base_links_settings];

                if r_settings.ssg_build {
                    let c = tokio::sync::mpsc::unbounded_channel();
                    let ctx = c.0.clone();

                    element_content_handlers.push(lol_html::element!("script", move |el| {
                        if let Some(source) = el.get_attribute("src") {
                            if source.starts_with("/_next/static/")
                                && source.ends_with("/_ssgManifest.js")
                            {
                                let _ = ctx.send(source);
                            }
                        }
                        Ok(())
                    }));

                    senders.replace(c);
                }

                let settings = lol_html::send::Settings {
                    element_content_handlers,
                    adjust_charset_on_meta_tag: true,
                    ..lol_html::send::Settings::new_for_handler_types()
                };

                let mut rewriter =
                    lol_html::send::HtmlRewriter::new(settings.into(), move |c: &[u8]| {
                        let _ = tx.send(c.to_vec());
                    });

                let mut response =
                    handle_response_bytes_writer(res, url, only_html, &mut rewriter).await;

                let rewrite_error = response.1;

                if !rewrite_error {
                    let _ = rewriter.end();
                }

                let mut collected_bytes: Vec<u8> = Vec::new();

                while let Some(c) = rx.recv().await {
                    collected_bytes.extend_from_slice(&c);
                }

                response.0.content.replace(Box::new(collected_bytes.into()));

                drop(rx);

                if let Some(ctx) = senders {
                    let mut rtx = ctx.1;
                    drop(ctx.0);

                    if r_settings.ssg_build {
                        if let Some(mut ssg_map) = ssg_map {
                            let rc = rtx.recv().await;

                            if let Some(source) = rc {
                                if let Some(ref url_base) = base {
                                    let build_ssg_path = convert_abs_path(&url_base, &source);
                                    let build_page =
                                        Page::new_page(build_ssg_path.as_str(), &client).await;

                                    for cap in
                                        SSG_CAPTURE.captures_iter(build_page.get_html_bytes_u8())
                                    {
                                        if let Some(matched) = cap.get(1) {
                                            let href = auto_encode_bytes(&matched.as_bytes())
                                                .replace(r#"\u002F"#, "/");

                                            fn get_last_segment(path: &str) -> &str {
                                                if let Some(pos) = path.rfind('/') {
                                                    &path[pos + 1..]
                                                } else {
                                                    path
                                                }
                                            }

                                            let last_segment = get_last_segment(&href);

                                            // we can pass in a static map of the dynamic SSG routes pre-hand, custom API endpoint to seed, or etc later.
                                            if !(last_segment.starts_with("[")
                                                && last_segment.ends_with("]"))
                                            {
                                                push_link(
                                                    &base,
                                                    &href,
                                                    &mut ssg_map,
                                                    &selectors.0,
                                                    parent_host,
                                                    parent_host_scheme,
                                                    base_input_domain,
                                                    sub_matcher,
                                                    &external_domains_caseless,
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                response.0
            }
            Ok(res) => setup_default_response(&res),
            Err(_) => {
                log("- error parsing html text {}", url);
                let mut page_response = PageResponse::default();
                if let Ok(status_code) = StatusCode::from_u16(599) {
                    page_response.status_code = status_code;
                }
                page_response
            }
        };

        build(url, page_response)
    }

    /// Instantiate a new page and gather the html repro of standard fetch_page_html only gathering resources to crawl.
    pub async fn new_page_only_html(url: &str, client: &Client) -> Self {
        let page_resource = crate::utils::fetch_page_html_raw_only_html(url, client).await;
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
        viewport: &Option<crate::configuration::Viewport>,
        request_timeout: &Option<Box<Duration>>,
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
            viewport,
            request_timeout,
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

    #[cfg(not(feature = "headers"))]
    /// Get the timeout required for rate limiting. The max duration is 30 seconds for delay respecting. Requires the feature flag `headers`.
    pub fn get_timeout(&self) -> Option<Duration> {
        None
    }

    #[cfg(feature = "headers")]
    /// Get the timeout required for rate limiting. The max duration is 30 seconds for delay respecting. Requires the feature flag `headers`.
    pub fn get_timeout(&self) -> Option<Duration> {
        if self.status_code == 429 {
            const MAX_TIMEOUT: Duration = Duration::from_secs(30);
            if let Some(ref headers) = self.headers {
                if let Some(retry_after) = headers.get(reqwest::header::RETRY_AFTER) {
                    if let Ok(retry_after_str) = retry_after.to_str() {
                        if let Ok(seconds) = retry_after_str.parse::<u64>() {
                            return Some(Duration::from_secs(seconds).min(MAX_TIMEOUT));
                        }
                        if let Ok(date) = httpdate::parse_http_date(retry_after_str) {
                            if let Ok(duration) = date.duration_since(std::time::SystemTime::now())
                            {
                                return Some(duration.min(MAX_TIMEOUT));
                            }
                        }
                    }
                }
            };
        }
        None
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
        self.html = match html {
            Some(html) => Some(Box::new(html)),
            _ => None,
        };
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

    /// Html getter for bytes on the page as string.
    pub fn get_html(&self) -> String {
        match self.html.as_ref() {
            Some(html) => {
                if html.is_empty() {
                    Default::default()
                } else {
                    auto_encoder::auto_encode_bytes(html)
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

    /// Html getter for getting the content with proper encoding. Pass in a proper encoding label like SHIFT_JIS. This fallsback to get_html without the `encoding` flag enabled.
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

    /// Find the links as a stream using string resource validation for XML files
    #[cfg(all(not(feature = "decentralized")))]
    pub async fn links_stream_xml_links_stream_base<
        A: PartialEq + Eq + Sync + Send + Clone + Default + std::hash::Hash + From<String>,
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
        let sub_matcher = &selectors.0;

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
                                    push_link(
                                        &self.base,
                                        &v,
                                        map,
                                        &selectors.0,
                                        parent_host,
                                        parent_host_scheme,
                                        base_input_domain,
                                        sub_matcher,
                                        &self.external_domains_caseless,
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
    pub async fn links_stream_base<
        A: PartialEq + Eq + Sync + Send + Clone + Default + std::hash::Hash + From<String>,
    >(
        &self,
        selectors: &RelativeSelectors,
        html: &str,
    ) -> HashSet<A> {
        let mut map = HashSet::new();

        if !html.is_empty() {
            if html.starts_with("<?xml") {
                self.links_stream_xml_links_stream_base(selectors, html, &mut map)
                    .await;
            } else {
                let parent_host = &selectors.1[0];
                // the host schemes
                let parent_host_scheme = &selectors.1[1];
                let base_input_domain = &selectors.2; // the domain after redirects
                let sub_matcher = &selectors.0;

                let rewriter_settings = Settings {
                    element_content_handlers: vec![lol_html::element!("a", |el| {
                        if let Some(href) = el.get_attribute("href") {
                            push_link(
                                &self.base,
                                &href,
                                &mut map,
                                &selectors.0,
                                parent_host,
                                parent_host_scheme,
                                base_input_domain,
                                sub_matcher,
                                &self.external_domains_caseless,
                            );
                        }
                        Ok(())
                    })],
                    adjust_charset_on_meta_tag: true,
                    ..lol_html::send::Settings::new_for_handler_types()
                };

                let mut wrote_error = false;

                let mut rewriter =
                    lol_html::send::HtmlRewriter::new(rewriter_settings.into(), |_c: &[u8]| {});

                let html_bytes = html.as_bytes();
                let chunk_size = 8192;
                let chunks = html_bytes.chunks(chunk_size);

                let mut stream = tokio_stream::iter(chunks).map(Ok::<&[u8], A>);

                while let Some(chunk) = stream.next().await {
                    if let Ok(chunk) = chunk {
                        if let Err(_) = rewriter.write(chunk) {
                            wrote_error = true;
                            break;
                        }
                    }
                }

                if !wrote_error {
                    let _ = rewriter.end();
                }
            }
        }

        map
    }

    /// Find the links as a stream using string resource validation
    #[inline(always)]
    #[cfg(all(not(feature = "decentralized")))]
    pub async fn links_stream_base_ssg<
        A: PartialEq + Eq + Sync + Send + Clone + Default + std::hash::Hash + From<String>,
    >(
        &self,
        selectors: &RelativeSelectors,
        html: &str,
        client: &Client,
    ) -> HashSet<A> {
        use auto_encoder::auto_encode_bytes;

        let mut map = HashSet::new();
        let mut map_ssg = HashSet::new();

        if !html.is_empty() {
            if html.starts_with("<?xml") {
                self.links_stream_xml_links_stream_base(selectors, html, &mut map)
                    .await;
            } else {
                let c = tokio::sync::mpsc::unbounded_channel();

                // the original url
                let parent_host = &selectors.1[0];
                // the host schemes
                let parent_host_scheme = &selectors.1[1];
                let base_input_domain = &selectors.2; // the domain after redirects
                let sub_matcher = &selectors.0;

                let txx = c.0.clone();

                let rewriter_settings = Settings {
                    element_content_handlers: vec![
                        lol_html::element!("a", |el| {
                            if let Some(href) = el.get_attribute("href") {
                                push_link(
                                    &self.base,
                                    &href,
                                    &mut map,
                                    &selectors.0,
                                    parent_host,
                                    parent_host_scheme,
                                    base_input_domain,
                                    sub_matcher,
                                    &self.external_domains_caseless,
                                );
                            }
                            Ok(())
                        }),
                        lol_html::element!("script", move |el| {
                            if let Some(source) = el.get_attribute("src") {
                                if source.starts_with("/_next/static/")
                                    && source.ends_with("/_ssgManifest.js")
                                {
                                    if let Some(build_path) = self.abs_path(&source) {
                                        let _ = txx.send(build_path.as_str().to_string());
                                    }
                                }
                            }
                            Ok(())
                        }),
                    ],
                    adjust_charset_on_meta_tag: true,
                    ..lol_html::send::Settings::new_for_handler_types()
                };

                let mut rewriter =
                    lol_html::send::HtmlRewriter::new(rewriter_settings.into(), |_c: &[u8]| {});

                let html_bytes = html.as_bytes();
                let chunk_size = 8192;
                let chunks = html_bytes.chunks(chunk_size);
                let mut wrote_error = false;

                let mut stream = tokio_stream::iter(chunks).map(Ok::<&[u8], A>);

                while let Some(chunk) = stream.next().await {
                    if let Ok(chunk) = chunk {
                        if let Err(_) = rewriter.write(chunk) {
                            wrote_error = true;
                            break;
                        }
                    }
                }

                if !wrote_error {
                    let _ = rewriter.end();
                }

                drop(c.0);
                let mut rx = c.1;

                if let Some(build_ssg_path) = rx.recv().await {
                    let build_page = Page::new_page(&build_ssg_path, &client).await;

                    for cap in SSG_CAPTURE.captures_iter(build_page.get_html_bytes_u8()) {
                        if let Some(matched) = cap.get(1) {
                            let href =
                                auto_encode_bytes(&matched.as_bytes()).replace(r#"\u002F"#, "/");

                            fn get_last_segment(path: &str) -> &str {
                                if let Some(pos) = path.rfind('/') {
                                    &path[pos + 1..]
                                } else {
                                    path
                                }
                            }

                            let last_segment = get_last_segment(&href);

                            // we can pass in a static map of the dynamic SSG routes pre-hand, custom API endpoint to seed, or etc later.
                            if !(last_segment.starts_with("[") && last_segment.ends_with("]")) {
                                push_link(
                                    &self.base,
                                    &href,
                                    &mut map_ssg,
                                    &selectors.0,
                                    parent_host,
                                    parent_host_scheme,
                                    base_input_domain,
                                    sub_matcher,
                                    &self.external_domains_caseless,
                                );
                            }
                        }
                    }
                }
            }
        }

        map.extend(map_ssg);

        map
    }

    /// Find the links as a stream using string resource validation and parsing the script for nextjs initial SSG paths.
    #[cfg(all(not(feature = "decentralized")))]
    pub async fn links_stream_ssg<
        A: PartialEq + Eq + Sync + Send + Clone + Default + std::hash::Hash + From<String>,
    >(
        &self,
        selectors: &RelativeSelectors,
        client: &Client,
    ) -> HashSet<A> {
        if auto_encoder::is_binary_file(self.get_html_bytes_u8()) {
            Default::default()
        } else {
            self.links_stream_base_ssg(selectors, &Box::new(self.get_html()), client)
                .await
        }
    }

    /// Find all href links and return them using CSS selectors.
    #[inline(always)]
    #[cfg(all(not(feature = "decentralized")))]
    pub async fn links_ssg(
        &self,
        selectors: &RelativeSelectors,
        client: &Client,
    ) -> HashSet<CaseInsensitiveString> {
        match self.html.is_some() {
            false => Default::default(),
            true => {
                self.links_stream_ssg::<CaseInsensitiveString>(selectors, client)
                    .await
            }
        }
    }

    /// Find the links as a stream using string resource validation
    #[inline(always)]
    #[cfg(all(not(feature = "decentralized"), not(feature = "full_resources")))]
    pub async fn links_stream<
        A: PartialEq + Eq + Sync + Send + Clone + Default + std::hash::Hash + From<String>,
    >(
        &self,
        selectors: &RelativeSelectors,
    ) -> HashSet<A> {
        if auto_encoder::is_binary_file(self.get_html_bytes_u8()) {
            Default::default()
        } else {
            self.links_stream_base(selectors, &Box::new(self.get_html()))
                .await
        }
    }

    /// Find the links as a stream using string resource validation
    #[cfg(all(
        not(feature = "decentralized"),
        not(feature = "full_resources"),
        feature = "smart"
    ))]
    #[inline(always)]
    pub async fn links_stream_smart<
        A: PartialEq + Eq + Sync + Send + Clone + Default + std::hash::Hash + From<String>,
    >(
        &self,
        selectors: &RelativeSelectors,
        browser: &std::sync::Arc<chromiumoxide::Browser>,
        configuration: &crate::configuration::Configuration,
        context_id: &Option<chromiumoxide::cdp::browser_protocol::browser::BrowserContextId>,
    ) -> HashSet<A> {
        use auto_encoder::auto_encode_bytes;
        use lol_html::{doc_comments, element};

        let mut map = HashSet::new();
        let mut inner_map: HashSet<A> = map.clone();

        if !self.is_empty() {
            let html_resource = Box::new(self.get_html());

            if html_resource.starts_with("<?xml") {
                self.links_stream_xml_links_stream_base(selectors, &html_resource, &mut map)
                    .await;
            } else {
                let (tx, rx) = tokio::sync::oneshot::channel();

                let (txx, mut rxx) = tokio::sync::mpsc::unbounded_channel();
                let (txxx, mut rxxx) = tokio::sync::mpsc::unbounded_channel();

                let base_input_domain = &selectors.2;
                let parent_frags = &selectors.1; // todo: allow mix match tpt
                let parent_host = &parent_frags[0];
                let parent_host_scheme = &parent_frags[1];
                let sub_matcher = &selectors.0;

                let external_domains_caseless = self.external_domains_caseless.clone();

                let base = self.base.clone();
                let base1 = base.clone();

                let txxx2 = txxx.clone();

                let mut static_app = false;

                let rewriter_settings = Settings {
                    element_content_handlers: vec![
                        element!("script", move |element| {
                            if !static_app {
                                if let Some(src) = element.get_attribute("src") {
                                    if src.starts_with("/") {
                                        if src.starts_with("/_next/static/chunks/pages/")
                                            || src.starts_with("/webpack-runtime-")
                                            || element.get_attribute("id").eq(&*GATSBY)
                                        {
                                            static_app = true;
                                        }

                                        if let Some(ref base) = base1 {
                                            let abs = convert_abs_path(&base, &src);

                                            if let Ok(mut paths) =
                                                abs.path_segments().ok_or_else(|| "cannot be base")
                                            {
                                                while let Some(p) = paths.next() {
                                                    // todo: get the path last before None instead of checking for ends_with
                                                    if p.ends_with(".js")
                                                        && JS_FRAMEWORK_ASSETS.contains(&p)
                                                    {
                                                        let _ = txxx2.send(true);
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            Ok(())
                        }),
                        element!("a", |el| {
                            if let Some(href) = el.get_attribute("href") {
                                push_link(
                                    &base,
                                    &href,
                                    &mut inner_map,
                                    &selectors.0,
                                    parent_host,
                                    parent_host_scheme,
                                    base_input_domain,
                                    sub_matcher,
                                    &external_domains_caseless,
                                );
                            }

                            el.remove();

                            Ok(())
                        }),
                        element!("*:not(script):not(a):not(body):not(head):not(html)", |el| {
                            el.remove();
                            Ok(())
                        }),
                    ],
                    document_content_handlers: vec![doc_comments!(|c| {
                        c.remove();
                        Ok(())
                    })],
                    adjust_charset_on_meta_tag: true,
                    ..lol_html::send::Settings::new_for_handler_types()
                };

                let mut rewriter =
                    lol_html::send::HtmlRewriter::new(rewriter_settings.into(), |c: &[u8]| {
                        let _ = txx.send(c.to_vec());
                    });

                let html_bytes = html_resource.as_bytes();
                let chunk_size = 8192;
                let chunks = html_bytes.chunks(chunk_size);
                let mut wrote_error = false;

                let mut stream = tokio_stream::iter(chunks).map(Ok::<&[u8], A>);

                while let Some(chunk) = stream.next().await {
                    if let Ok(chunk) = chunk {
                        if let Err(_) = rewriter.write(chunk) {
                            wrote_error = true;
                            break;
                        }
                    }
                }

                if !wrote_error {
                    let _ = rewriter.end();
                }

                drop(txxx);
                drop(txx);

                let mut rewrited_bytes: Vec<u8> = Vec::new();

                while let Some(c) = rxx.recv().await {
                    rewrited_bytes.extend_from_slice(&c);
                }

                let mut rerender = rxxx.recv().await.unwrap_or_default();

                if !rerender {
                    if let Some(_) = DOM_WATCH_METHODS.find(&rewrited_bytes) {
                        rerender = true;
                    }
                }

                if rerender {
                    // we should re-use the html content instead with events.
                    let browser = browser.to_owned();
                    let configuration = configuration.clone();
                    let target_url = self.url.clone();
                    let context_id = context_id.clone();
                    let parent_host = parent_host.clone();

                    tokio::task::spawn(async move {
                        if let Ok(new_page) = crate::features::chrome::attempt_navigation(
                            "about:blank",
                            &browser,
                            &configuration.request_timeout,
                            &context_id,
                        )
                        .await
                        {
                            let intercept_handle =
                                crate::features::chrome::setup_chrome_interception_base(
                                    &new_page,
                                    configuration.chrome_intercept.enabled,
                                    &configuration.auth_challenge_response,
                                    configuration.chrome_intercept.block_visuals,
                                    &parent_host,
                                )
                                .await;

                            crate::features::chrome::setup_chrome_events(&new_page, &configuration)
                                .await;

                            let page_resource = crate::utils::fetch_page_html_chrome_base(
                                &html_resource,
                                &new_page,
                                true,
                                true,
                                &Some(crate::configuration::WaitFor::new(
                                    Some(
                                        core::time::Duration::from_secs(120), // default a duration for smart handling. (maybe expose later on.)
                                    ),
                                    None,
                                    true,
                                    true,
                                    None,
                                    Some(crate::configuration::WaitForSelector::new(
                                        Some(core::time::Duration::from_millis(500)),
                                        "body".into(),
                                    )),
                                )),
                                &configuration.screenshot,
                                false,
                                &configuration.openai_config,
                                Some(&target_url),
                                &configuration.execution_scripts,
                                &configuration.automation_scripts,
                                &configuration.viewport,
                                &configuration.request_timeout,
                            )
                            .await;

                            if let Some(h) = intercept_handle {
                                let _ = h.await;
                            }

                            if let Ok(resource) = page_resource {
                                if let Err(_) = tx.send(resource) {
                                    crate::utils::log("the receiver dropped", "");
                                }
                            }
                        }
                    });

                    match rx.await {
                        Ok(v) => {
                            let extended_map = self
                                .links_stream_base::<A>(
                                    selectors,
                                    &match v.content {
                                        Some(h) => auto_encode_bytes(&h),
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
            map.extend(inner_map);
        }

        map
    }

    /// Find the links as a stream using string resource validation
    #[inline(always)]
    #[cfg(all(not(feature = "decentralized")))]
    pub async fn links_stream_full_resource<
        A: PartialEq + Eq + Sync + Send + Clone + Default + std::hash::Hash + From<String>,
    >(
        &self,
        selectors: &RelativeSelectors,
    ) -> HashSet<A> {
        let mut map = HashSet::new();

        if !self.is_empty() {
            let html = Box::new(self.get_html());

            if html.starts_with("<?xml") {
                self.links_stream_xml_links_stream_base(selectors, &html, &mut map)
                    .await;
            } else {
                let base_domain = &selectors.0;
                let base_input_domain = &selectors.2;
                let parent_frags = &selectors.1; // todo: allow mix match tpt
                let parent_host = &parent_frags[0];
                let parent_host_scheme = &parent_frags[1];
                let sub_matcher = &selectors.0;

                let html = Box::new(crate::packages::scraper::Html::parse_document(&html));
                let mut stream = tokio_stream::iter(html.tree);

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
                                        sub_matcher,
                                    );

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
                                    }

                                    if can_process {
                                        if abs.scheme() != parent_host_scheme.as_str() {
                                            let _ = abs.set_scheme(parent_host_scheme.as_str());
                                        }

                                        let h = abs.as_str();

                                        if can_process {
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
        }
        map
    }

    /// Find the links as a stream using string resource validation
    #[inline(always)]
    #[cfg(all(not(feature = "decentralized"), feature = "full_resources"))]
    pub async fn links_stream<
        A: PartialEq + Eq + Sync + Send + Clone + Default + std::hash::Hash + From<String>,
    >(
        &self,
        selectors: &RelativeSelectors,
    ) -> HashSet<A> {
        if auto_encoder::is_binary_file(self.get_html_bytes_u8()) {
            Default::default()
        } else {
            self.links_stream_full_resource(selectors).await
        }
    }

    #[inline(always)]
    #[cfg(feature = "decentralized")]
    /// Find the links as a stream using string resource validation
    pub async fn links_stream<
        A: PartialEq + Eq + Sync + Send + Clone + Default + std::hash::Hash + From<String>,
    >(
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
    #[cfg(all(not(feature = "decentralized")))]
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
pub fn encode_bytes(html: &Bytes, label: &str) -> String {
    auto_encoder::encode_bytes(html, label)
}

/// Get the content with proper encoding. Pass in a proper encoding label like SHIFT_JIS.
#[cfg(feature = "encoding")]
pub fn get_html_encoded(html: &Option<Box<Bytes>>, label: &str) -> String {
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

/// Rewrite a string without encoding it.
#[cfg(all(
    not(feature = "decentralized"),
    not(feature = "full_resources"),
    feature = "smart"
))]
pub fn rewrite_str_as_bytes<'h, 's>(
    html: &str,
    settings: impl Into<lol_html::Settings<'h, 's>>,
) -> Result<Vec<u8>, lol_html::errors::RewritingError> {
    let mut output = vec![];

    let mut rewriter = lol_html::HtmlRewriter::new(settings.into(), |c: &[u8]| {
        output.extend_from_slice(c);
    });

    rewriter.write(html.as_bytes())?;
    rewriter.end()?;

    Ok(output)
}

/// Basic rewriter without rewriting.
pub fn rewrite_str_empty<'h, 's>(
    html: &str,
    settings: impl Into<lol_html::Settings<'h, 's>>,
) -> Result<(), lol_html::errors::RewritingError> {
    // we should use this in our chunks to rewrite while streaming.
    let mut rewriter = lol_html::HtmlRewriter::new(settings.into(), |_c: &[u8]| {});

    rewriter.write(html.as_bytes())?;
    rewriter.end()?;

    Ok(())
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

#[tokio::test]
async fn test_abs_path() {
    let link_result = "https://choosealicense.com/";
    let page: Page = build(&link_result, Default::default());

    assert_eq!(
        page.abs_path("?query=keyword").expect("a valid url"),
        Url::parse("https://choosealicense.com?query=keyword").expect("a valid url")
    );

    assert_eq!(
        page.abs_path("#query=keyword").expect("a valid url"),
        Url::parse("https://choosealicense.com").expect("a valid url")
    );

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

    let page: Page = build(&format!("{}index.php", link_result), Default::default());

    assert_eq!(
        page.abs_path("index.html").expect("a valid url"),
        Url::parse("https://choosealicense.com/index.html").expect("a valid url")
    );
}

#[cfg(all(feature = "time", not(feature = "decentralized")))]
#[tokio::test]
async fn test_duration() {
    let client = Client::default();
    let link_result = "https://choosealicense.com/";
    let page: Page = Page::new_page(&link_result, &client).await;
    let duration_elasped = page.get_duration_elasped().as_millis();

    assert!(
        duration_elasped < 6000,
        "Duration took longer than expected {}.",
        duration_elasped,
    );
}
