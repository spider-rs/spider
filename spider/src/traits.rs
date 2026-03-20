//! Trait abstractions for spider's core types.
//!
//! These traits provide polymorphic access to [`Page`](crate::page::Page) and
//! [`Website`](crate::website::Website), enabling generic code, mocking in tests,
//! and alternative implementations.

use hashbrown::HashSet;
use reqwest::header::HeaderMap;
use reqwest::StatusCode;

use crate::CaseInsensitiveString;
use crate::Client;

/// Read-only access to a crawled page's content.
pub trait PageData {
    /// The page URL as originally requested.
    fn url(&self) -> &str;
    /// The final URL after any redirects.
    fn url_final(&self) -> &str;
    /// The raw response bytes, if available.
    fn bytes(&self) -> Option<&[u8]>;
    /// The page content decoded as a UTF-8 string.
    fn html(&self) -> String;
    /// The raw HTML bytes (empty slice if none).
    fn html_bytes_u8(&self) -> &[u8];
    /// The HTTP status code of the response.
    fn status_code(&self) -> StatusCode;
    /// The HTTP response headers, if available.
    fn headers(&self) -> Option<&HeaderMap>;
    /// Whether the page has no meaningful content.
    fn is_empty(&self) -> bool;
}

/// Timing information for a crawled page. Requires the `time` feature.
#[cfg(feature = "time")]
#[cfg_attr(docsrs, doc(cfg(feature = "time")))]
pub trait PageTimingExt: PageData {
    /// How long since the page request started.
    fn duration_elapsed(&self) -> tokio::time::Duration;
}

/// Chrome-specific page data. Requires the `chrome` feature.
#[cfg(feature = "chrome")]
#[cfg_attr(docsrs, doc(cfg(feature = "chrome")))]
pub trait PageChromeExt: PageData {
    /// The underlying Chrome DevTools Protocol page handle, if available.
    fn chrome_page(&self) -> Option<&chromiumoxide::Page>;
    /// A screenshot of the page as raw bytes, if captured.
    fn screenshot_bytes(&self) -> Option<&[u8]>;
}

/// Core crawl orchestration.
pub trait Crawler {
    /// The page type produced by this crawler.
    type Page: PageData;

    /// The base URL being crawled.
    fn url(&self) -> &str;
    /// The current crawl status.
    fn status(&self) -> &crate::website::CrawlStatus;
    /// All links discovered so far.
    fn links(&self) -> HashSet<CaseInsensitiveString>;
    /// The collected pages, if page storage is enabled.
    fn pages(&self) -> Option<&[Self::Page]>;
    /// The HTTP client, if one has been built.
    fn client(&self) -> &Option<Client>;

    /// Start a standard crawl.
    fn crawl(&mut self) -> impl std::future::Future<Output = ()> + Send;
    /// Start a raw crawl (useful with the `chrome` feature).
    fn crawl_raw(&mut self) -> impl std::future::Future<Output = ()> + Send;
}

/// Subscribe to page events during a crawl. Requires the `sync` feature.
#[cfg(feature = "sync")]
#[cfg_attr(docsrs, doc(cfg(feature = "sync")))]
pub trait CrawlerSubscription: Crawler {
    /// Subscribe to receive pages as they are crawled.
    fn subscribe(
        &mut self,
        capacity: usize,
    ) -> Option<tokio::sync::broadcast::Receiver<Self::Page>>;
}
