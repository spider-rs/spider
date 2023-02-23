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
//! There are two ways to use Spider:
//!
//! - **Concurrent** is the fastest way to start crawling a web page and
//!   typically the most efficient.
//!   - [`crawl`] is used to crawl concurrently :blocking.
//! - **Sequential** lets you crawl the web pages one after another respecting delay sequences.
//!   - [`crawl_sync`] is used to crawl in sync :blocking.
//!
//! [`crawl`]: website/struct.Website.html#method.crawl
//! [`crawl_sync`]: website/struct.Website.html#method.crawl_sync
//!
//! # Basic usage
//!
//! First, you will need to add `spider` to your `Cargo.toml`.
//!
//! Next, simply add the website url in the struct of website and crawl,
//! you can also crawl sequentially.

extern crate hashbrown;
extern crate log;
extern crate reqwest;
extern crate scraper;
pub extern crate tokio;

#[cfg(feature = "ua_generator")]
extern crate ua_generator;
pub extern crate url;
#[macro_use]
extern crate string_concat;
#[macro_use]
extern crate lazy_static;
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

#[cfg(feature = "regex")]
/// Black list checking url exist with Regex.
pub mod black_list {
    use regex::Regex;
    /// check if link exist in blacklists with regex.
    pub fn contains(blacklist_url: &Vec<String>, link: &String) -> bool {
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
    /// check if link exist in blacklists.
    pub fn contains(blacklist_url: &Vec<String>, link: &String) -> bool {
        blacklist_url.contains(&link)
    }
}
