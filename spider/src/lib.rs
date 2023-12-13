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
//!     let mut website: Website = Website::new("https://rsseau.fr");
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
//! ## Feature flags
//!
//! - `ua_generator`: Enables auto generating a random real User-Agent.
//! - `regex`: Enables blacklisting paths with regx
//! - `jemalloc`: Enables the [jemalloc](https://github.com/jemalloc/jemalloc) memory backend.
//! - `decentralized`: Enables decentralized processing of IO, requires the [spider_worker](../spider_worker/README.md) startup before crawls.
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
//! - `chrome`: Enables chrome headless rendering, use the env var `CHROME_URL` to connect remotely [experimental].
//! - `chrome_headed`: Enables chrome rendering headful rendering [experimental].
//! - `chrome_cpu`: Disable gpu usage for chrome browser.
//! - `chrome_stealth`: Enables stealth mode to make it harder to be detected as a bot.
//! - `chrome_store_page`: Store the page object to perform other actions like taking screenshots conditionally.
//! - `chrome_screenshot`: Enables storing a screenshot of each page on crawl. Defaults the screenshots to the ./storage/ directory. Use the env variable `SCREENSHOT_DIRECTORY` to adjust the directory.
//! - `cookies`: Enables cookies storing and setting to use for request.
//! - `cron`: Enables the ability to start cron jobs for the website.
//! - `http3`: Enables experimental HTTP/3 client.
//! - `smart`: Enables smart mode. This runs request as HTTP until JavaScript rendering is needed. This avoids sending multiple network request by re-using the content.

pub extern crate bytes;
pub extern crate compact_str;
pub extern crate hashbrown;
extern crate log;
pub extern crate reqwest;
pub extern crate tokio;
pub extern crate tokio_stream;

#[cfg(feature = "ua_generator")]
pub extern crate ua_generator;

#[cfg(feature = "cron")]
pub extern crate async_job;

#[cfg(feature = "flexbuffers")]
pub extern crate flexbuffers;

#[cfg(feature = "serde")]
pub extern crate serde;

pub extern crate case_insensitive_string;
pub extern crate smallvec;
pub extern crate url;

#[macro_use]
pub extern crate string_concat;
#[macro_use]
pub extern crate lazy_static;
#[macro_use]
pub extern crate fast_html5ever;

// performance reasons jemalloc memory backend for dedicated work and large crawls
#[cfg(all(
    not(windows),
    not(target_os = "android"),
    not(target_env = "musl"),
    feature = "jemalloc"
))]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

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
        blacklist_url.contains(&link)
    }
}
