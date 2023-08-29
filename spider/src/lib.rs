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
//! - `decentralized`: Enables decentralized processing of IO,
//!         requires the [spider_worker] startup before crawls.
//! - `control`: Enables the ability to pause, start, and shutdown crawls on demand.
//! - `full_resources`: Enables gathering all content that relates to the domain.
//! - `serde`: Enables serde serialization support.

pub extern crate compact_str;
pub extern crate hashbrown;
extern crate log;
pub extern crate reqwest;
pub extern crate tokio;
pub extern crate bytes;

#[cfg(feature = "ua_generator")]
extern crate ua_generator;

#[cfg(feature = "flexbuffers")]
pub extern crate bytes;

#[cfg(feature = "flexbuffers")]
pub extern crate flexbuffers;

#[cfg(feature = "serde")]
pub extern crate serde;

pub extern crate case_insensitive_string;
pub extern crate url;

#[macro_use]
pub extern crate string_concat;
#[macro_use]
pub extern crate lazy_static;
#[macro_use]
pub extern crate fast_html5ever;
#[macro_use]
pub extern crate matches;

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
mod features;
/// Internal packages customized.
pub mod packages;
/// A page scraped.
pub mod page;
/// Application utils.
pub mod utils;
/// A website to crawl.
pub mod website;

pub use case_insensitive_string::CaseInsensitiveString;

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
