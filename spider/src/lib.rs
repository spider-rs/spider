#![warn(missing_docs)]

//! Website crawling library that rapidly crawls all pages to
//! gather links via isolated contexts.
//!
//! Spider is multi-threaded crawler that can be configured
//! to scrape web pages. It has the ability to gather
//! tens of thousands of pages within seconds.
//!
//! # How to use Spider
//!
//! There are a couple of ways to use Spider:
//!
//! - **Concurrent** is the fastest way to start crawling a web page and
//!   typically the most efficient.
//!   - [`crawl`] is used to crawl concurrently.
//! - **Sequential** lets you crawl the web pages one after another respecting delay sequences.
//!   - [`crawl_sync`] is used to crawl in sync.
//! - **Scrape** Scrape the page and hold onto the HTML raw string to parse.
//!   - [`scrape`] is used to gather the HTML.
//!
//! [`crawl`]: website/struct.Website.html#method.crawl
//! [`crawl_sync`]: website/struct.Website.html#method.crawl_sync
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
//! - `ua_generator`: Enables auto generating a random real User-Agent. Enabled by default.
//! - `regex`: Enables blacklisting paths with regx
//! - `jemalloc`: Enables the jemalloc memory backend.
//! - `decentralized`: Enables decentralized processing of IO
//!         requires the [spider_worker] startup before crawls.
//! - `control`: Enabled the ability to pause, start, and shutdown crawls on demand.

use compact_str::CompactString;

pub extern crate compact_str;
pub extern crate hashbrown;
extern crate log;
pub extern crate reqwest;
pub extern crate tokio;

#[cfg(feature = "ua_generator")]
extern crate ua_generator;

#[cfg(feature = "serde")]
pub extern crate serde;

pub extern crate url;
#[macro_use]
pub extern crate string_concat;
#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate fast_html5ever;
#[macro_use]
extern crate matches;
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
/// Internal packages customized.
pub mod packages;
/// A page scraped.
pub mod page;
/// Application utils.
pub mod utils;
/// A website to crawl.
pub mod website;
/// Optional features to use.
mod features;

#[cfg(feature = "regex")]
/// Black list checking url exist with Regex.
pub mod black_list {
    use compact_str::CompactString;
    use regex::Regex;
    /// check if link exist in blacklists with regex.
    pub fn contains(blacklist_url: &Vec<CompactString>, link: &CompactString) -> bool {
        for pattern in blacklist_url {
            let re = Regex::new(pattern).unwrap();
            if re.is_match(link) {
                return true;
            }
        }

        return false;
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

/// case-insensitive string handling
#[derive(Debug, Clone)]
#[repr(transparent)]
pub struct CaseInsensitiveString(CompactString);

impl PartialEq for CaseInsensitiveString {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.0.eq_ignore_ascii_case(&other.0)
    }
}

impl Eq for CaseInsensitiveString {}

impl std::hash::Hash for CaseInsensitiveString {
    #[inline]
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.to_ascii_lowercase().hash(state)
    }
}

impl From<&str> for CaseInsensitiveString {
    #[inline]
    fn from(s: &str) -> Self {
        CaseInsensitiveString { 0: s.into() }
    }
}

impl From<String> for CaseInsensitiveString {
    fn from(s: String) -> Self {
        CaseInsensitiveString { 0: s.into() }
    }
}

impl AsRef<str> for CaseInsensitiveString {
    #[inline]
    fn as_ref(&self) -> &str {
        &self.0
    }
}