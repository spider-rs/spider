#![warn(missing_docs)]

//! Website crawling library that rapidly crawls all pages to
//! gather links via isolated contexts.
//!
//! Spider is multi-threaded crawler that can be configured
//! to scrape web pages. It has the ability to gather
//! millions of pages within seconds.
//!
//! # How to use Spider
//!
//! There are a couple of ways to use Spider:
//!
//! - **Crawl** starts crawling a web page and
//!   perform most work in isolation.
//!   - [`crawl`] is used to crawl concurrently.
//! - **Scrape** Scrape the page and hold onto the HTML raw string to parse.
//!   - [`scrape`] is used to gather the HTML.
//!
//! [`crawl`]: website/struct.Website.html#method.crawl
//! [`scrape`]: website/struct.Website.html#method.scrape
//!
//! # Examples
//!
//! A simple crawl to index a website:
//!
//! ```no_run
//! use spider::tokio;
//! use spider::website::Website;
//!
//! #[tokio::main]
//! async fn main() {
//!     let mut website: Website = Website::new("https://spider.cloud");
//!
//!     website.crawl().await;
//!
//!     let links = website.get_links();
//!
//!     for link in links {
//!         println!("- {:?}", link.as_ref());
//!     }
//! }
//! ```
//!
//! Subscribe to crawl events:
//!
//! ```no_run
//! use spider::tokio;
//! use spider::website::Website;
//! use tokio::io::AsyncWriteExt;
//!
//! #[tokio::main]
//! async fn main() {
//!     let mut website: Website = Website::new("https://spider.cloud");
//!     let mut rx2 = website.subscribe(16).unwrap();
//!
//!     tokio::spawn(async move {
//!         let mut stdout = tokio::io::stdout();
//!
//!         while let Ok(res) = rx2.recv().await {
//!             let _ = stdout
//!                 .write_all(format!("- {}\n", res.get_url()).as_bytes())
//!                 .await;
//!         }
//!     });
//!
//!     website.crawl().await;
//! }
//! ```
//!
//! ## Feature flags
//!
//! - `ua_generator`: Enables auto generating a random real User-Agent.
//! - `regex`: Enables blacklisting paths with regx
//! - `jemalloc`: Enables the [jemalloc](https://github.com/jemalloc/jemalloc) memory backend.
//! - `decentralized`: Enables decentralized processing of IO, requires the [spider_worker](https://docs.rs/crate/spider_worker/latest) startup before crawls.
//! - `sync`: Subscribe to changes for Page data processing async.
//! - `budget`: Allows setting a crawl budget per path with depth.
//! - `control`: Enables the ability to pause, start, and shutdown crawls on demand.
//! - `full_resources`: Enables gathering all content that relates to the domain like css,jss, and etc.
//! - `serde`: Enables serde serialization support.
//! - `socks`: Enables socks5 proxy support.
//! - `glob`: Enables [url glob](https://everything.curl.dev/cmdline/globbing) support.
//! - `fs`: Enables storing resources to disk for parsing (may greatly increases performance at the cost of temp storage). Enabled by default.
//! - `sitemap`: Include sitemap pages in results.
//! - `js`: Enables javascript parsing links created with the alpha [jsdom](https://github.com/a11ywatch/jsdom) crate.
//! - `time`: Enables duration tracking per page.
//! - `cache`: Enables HTTP caching request to disk.
//! - `cache_mem`: Enables HTTP caching request to persist in memory.
//! - `cache_openai`: Enables caching the OpenAI request. This can drastically save costs when developing AI workflows.
//! - `chrome`: Enables chrome headless rendering, use the env var `CHROME_URL` to connect remotely.
//! - `chrome_headed`: Enables chrome rendering headful rendering.
//! - `chrome_cpu`: Disable gpu usage for chrome browser.
//! - `chrome_stealth`: Enables stealth mode to make it harder to be detected as a bot.
//! - `chrome_store_page`: Store the page object to perform other actions like taking screenshots conditionally.
//! - `chrome_screenshot`: Enables storing a screenshot of each page on crawl. Defaults the screenshots to the ./storage/ directory. Use the env variable `SCREENSHOT_DIRECTORY` to adjust the directory.
//! - `chrome_intercept`: Allows intercepting network request to speed up processing.
//! - `chrome_headless_new`: Use headless=new to launch the chrome instance.
//! - `cookies`: Enables cookies storing and setting to use for request.
//! - `real_browser`: Enables the ability to bypass protected pages.
//! - `cron`: Enables the ability to start cron jobs for the website.
//! - `openai`: Enables OpenAI to generate dynamic browser executable scripts. Make sure to use the env var `OPENAI_API_KEY`.
//! - `smart`: Enables smart mode. This runs request as HTTP until JavaScript rendering is needed. This avoids sending multiple network request by re-using the content.
//! - `encoding`: Enables handling the content with different encodings like Shift_JIS.
//! - `headers`: Enables the extraction of header information on each retrieved page. Adds a `headers` field to the page struct.
//! - `decentralized_headers`: Enables the extraction of suppressed header information of the decentralized processing of IO. This is needed if `headers` is set in both [spider](https://docs.rs/spider/latest/spider/) and [spider_worker](https://docs.rs/crate/spider_worker/latest).
//!
//! Additional learning resources include:
//!
//! - [Spider Repository Examples](https://github.com/spider-rs/spider/tree/main/examples)

// performance reasons jemalloc memory backend for dedicated work and large crawls
#[cfg(all(
    not(windows),
    not(target_os = "android"),
    not(target_env = "musl"),
    feature = "jemalloc"
))]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

pub extern crate bytes;
pub extern crate case_insensitive_string;
pub extern crate compact_str;
pub extern crate hashbrown;
extern crate log;
pub extern crate percent_encoding;
pub extern crate quick_xml;
pub extern crate reqwest;
pub extern crate smallvec;
pub extern crate tokio;
pub extern crate tokio_stream;
pub extern crate url;

#[cfg(feature = "cron")]
pub extern crate async_job;
#[cfg(feature = "openai")]
pub extern crate async_openai;
#[cfg(feature = "flexbuffers")]
pub extern crate flexbuffers;
#[cfg(feature = "cache")]
pub extern crate http_cache_reqwest;
#[cfg(feature = "cache_openai")]
pub extern crate moka;
#[cfg(feature = "cache")]
pub extern crate reqwest_middleware;
#[cfg(feature = "serde")]
pub extern crate serde;
#[cfg(feature = "ua_generator")]
pub extern crate ua_generator;
#[macro_use]
pub extern crate string_concat;
pub extern crate strum;
#[macro_use]
pub extern crate lazy_static;
#[macro_use]
pub extern crate fast_html5ever;

/// Configuration structure for `Website`.
pub mod configuration;
/// Optional features to use.
pub mod features;
/// Internal packages customized.
pub mod packages;
/// A page scraped.
pub mod page;
/// Application utils.
pub mod utils;
/// A website to crawl.
pub mod website;

pub use case_insensitive_string::CaseInsensitiveString;

#[cfg(feature = "chrome")]
pub use chromiumoxide;

#[cfg(feature = "regex")]
/// Black list checking url exist with Regex.
pub mod black_list {
    use compact_str::CompactString;
    /// check if link exist in blacklists with regex.
    pub fn contains(blacklist_url: &regex::RegexSet, link: &CompactString) -> bool {
        blacklist_url.is_match(link)
    }
}

#[cfg(not(feature = "regex"))]
/// Black list checking url exist.
pub mod black_list {
    use compact_str::CompactString;
    /// check if link exist in blacklists.
    pub fn contains(blacklist_url: &Vec<CompactString>, link: &CompactString) -> bool {
        blacklist_url.contains(link)
    }
}

/// The asynchronous Client to make requests with.
#[cfg(not(feature = "cache"))]
pub type Client = reqwest::Client;
#[cfg(not(feature = "cache"))]
/// The asynchronous Client Builder.
pub type ClientBuilder = reqwest::ClientBuilder;

/// The asynchronous Client to make requests with HTTP Cache.
#[cfg(feature = "cache")]
pub type Client = reqwest_middleware::ClientWithMiddleware;
#[cfg(feature = "cache")]
/// The asynchronous Client Builder.
pub type ClientBuilder = reqwest_middleware::ClientBuilder;
