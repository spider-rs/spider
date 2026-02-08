#![warn(missing_docs)]
#![allow(clippy::perf)]
#![allow(clippy::borrowed_box)]
#![allow(clippy::collapsible_match)]
#![allow(clippy::empty_docs)]
#![allow(clippy::empty_line_after_doc_comments)]
#![allow(clippy::explicit_counter_loop)]
#![allow(clippy::field_reassign_with_default)]
#![allow(clippy::if_same_then_else)]
#![allow(clippy::just_underscores_and_digits)]
#![allow(clippy::let_underscore_future)]
#![allow(clippy::manual_clamp)]
#![allow(clippy::manual_strip)]
#![allow(clippy::manual_unwrap_or_default)]
#![allow(clippy::map_identity)]
#![allow(clippy::match_like_matches_macro)]
#![allow(clippy::match_single_binding)]
#![allow(clippy::mixed_attributes_style)]
#![allow(clippy::non_minimal_cfg)]
#![allow(clippy::nonminimal_bool)]
#![allow(clippy::ptr_arg)]
#![allow(clippy::redundant_pattern_matching)]
#![allow(clippy::should_implement_trait)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::type_complexity)]
#![allow(clippy::vec_box)]
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
//! - [`crawl`]: start concurrently crawling a site. Can be used to send each page (including URL
//!   and HTML) to a subscriber for processing, or just to gather links.
//!
//! - [`scrape`]: like `crawl`, but saves the HTML raw strings to parse after scraping is complete.
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
//!
//! #[tokio::main]
//! async fn main() {
//!     let mut website: Website = Website::new("https://spider.cloud");
//!     let mut rx2 = website.subscribe(16).unwrap();
//!
//!     tokio::spawn(async move {
//!         while let Ok(res) = rx2.recv().await {
//!             println!("- {}", res.get_url());
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
//! - `disk`: Enables SQLite hybrid disk storage to balance memory usage with no tls.
//! - `disk_native_tls`: Enables SQLite hybrid disk storage to balance memory usage with native tls.
//! - `disk_aws`: Enables SQLite hybrid disk storage to balance memory usage with aws_tls.
//! - `balance`: Enables balancing the CPU and memory to scale more efficiently.
//! - `regex`: Enables blacklisting paths with regx.
//! - `firewall`: Enables spider_firewall crate to prevent bad websites from crawling.
//! - `decentralized`: Enables decentralized processing of IO, requires the [spider_worker](https://docs.rs/crate/spider_worker/latest) startup before crawls.
//! - `sync`: Subscribe to changes for Page data processing async.
//! - `control`: Enables the ability to pause, start, and shutdown crawls on demand.
//! - `full_resources`: Enables gathering all content that relates to the domain like css,jss, and etc.
//! - `serde`: Enables serde serialization support.
//! - `socks`: Enables socks5 proxy support.
//! - `glob`: Enables [url glob](https://everything.curl.dev/cmdline/globbing) support.
//! - `fs`: Enables storing resources to disk for parsing (may greatly increases performance at the cost of temp storage). Enabled by default.
//! - `sitemap`: Include sitemap pages in results.
//! - `time`: Enables duration tracking per page.
//! - `cache`: Enables HTTP caching request to disk.
//! - `cache_mem`: Enables HTTP caching request to persist in memory.
//! - `cache_chrome_hybrid`: Enables hybrid chrome request caching between HTTP.
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
//! - `gemini`: Enables Gemini AI to generate dynamic browser executable scripts. Make sure to use the env var `GEMINI_API_KEY`.
//! - `smart`: Enables smart mode. This runs request as HTTP until JavaScript rendering is needed. This avoids sending multiple network request by re-using the content.
//! - `encoding`: Enables handling the content with different encodings like Shift_JIS.
//! - `spoof`: Spoof HTTP headers for the request.
//! - `headers`: Enables the extraction of header information on each retrieved page. Adds a `headers` field to the page struct.
//! - `decentralized_headers`: Enables the extraction of suppressed header information of the decentralized processing of IO. This is needed if `headers` is set in both [spider](https://docs.rs/spider/latest/spider/) and [spider_worker](https://docs.rs/crate/spider_worker/latest).
//!
//! Additional learning resources include:
//!
//! - [Spider Repository Examples](https://github.com/spider-rs/spider/tree/main/examples)
pub extern crate bytes;
pub extern crate case_insensitive_string;
pub extern crate hashbrown;
extern crate log;
pub extern crate percent_encoding;
pub extern crate quick_xml;
pub extern crate reqwest;
pub extern crate smallvec;
pub extern crate spider_fingerprint;
pub extern crate tokio;
pub extern crate tokio_stream;
pub extern crate url;

#[cfg(feature = "cron")]
pub extern crate async_job;
#[cfg(feature = "openai")]
pub extern crate async_openai;
pub extern crate auto_encoder;
#[cfg(feature = "flexbuffers")]
pub extern crate flexbuffers;
#[cfg(feature = "gemini")]
pub extern crate gemini_rust;
#[cfg(feature = "cache_request")]
pub extern crate http_cache_reqwest;
#[cfg(feature = "cache_openai")]
pub extern crate moka;
#[cfg(feature = "cache_request")]
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
#[cfg(feature = "agent")]
pub extern crate spider_agent;
#[cfg(feature = "firewall")]
pub extern crate spider_firewall;

/// Re-export agent types from spider_agent crate.
#[cfg(feature = "agent")]
pub mod agent {
    //! Agent module re-exports from spider_agent crate.
    //!
    //! This provides convenient access to the multimodal agent functionality.
    pub use spider_agent::{
        Agent,
        AgentBuilder,
        AgentConfig,
        AgentError,
        AgentMemory,
        AgentResult,
        // Custom tool types
        AuthConfig,
        CustomTool,
        CustomToolRegistry,
        CustomToolResult,
        FetchResult,
        HtmlCleaningMode,
        HttpMethod,
        LimitType,
        Message,
        RetryConfig,
        SpiderCloudToolConfig,
        UsageLimits,
        UsageSnapshot,
        UsageStats,
    };

    #[cfg(feature = "agent_openai")]
    pub use spider_agent::OpenAIProvider;

    #[cfg(feature = "agent_chrome")]
    pub use spider_agent::BrowserContext;

    #[cfg(feature = "agent_webdriver")]
    pub use spider_agent::WebDriverContext;

    #[cfg(feature = "agent_fs")]
    pub use spider_agent::{TempFile, TempStorage};

    #[cfg(any(
        feature = "agent_search_serper",
        feature = "agent_search_brave",
        feature = "agent_search_bing",
        feature = "agent_search_tavily"
    ))]
    pub use spider_agent::{
        ResearchOptions, ResearchResult, SearchOptions, SearchProvider, SearchResult,
        SearchResults, TimeRange,
    };

    #[cfg(feature = "agent_search_serper")]
    pub use spider_agent::SerperProvider;

    #[cfg(feature = "agent_search_brave")]
    pub use spider_agent::BraveProvider;

    #[cfg(feature = "agent_search_bing")]
    pub use spider_agent::BingProvider;

    #[cfg(feature = "agent_search_tavily")]
    pub use spider_agent::TavilyProvider;
}

/// Client interface.
pub mod client;
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

pub use case_insensitive_string::compact_str;
pub use case_insensitive_string::CaseInsensitiveString;
pub use client::{Client, ClientBuilder};

#[cfg(feature = "chrome")]
pub use chromiumoxide;

#[cfg(feature = "search")]
pub use features::search;
#[cfg(feature = "search")]
pub use features::search_providers;

#[cfg(feature = "regex")]
/// Black list checking url exist with Regex.
pub mod black_list {
    use crate::compact_str::CompactString;
    /// check if link exist in blacklists with regex.
    pub fn contains(blacklist_url: &regex::RegexSet, link: &CompactString) -> bool {
        blacklist_url.is_match(link)
    }
}

#[cfg(not(feature = "regex"))]
/// Black list checking url exist.
pub mod black_list {
    use crate::compact_str::CompactString;
    /// check if link exist in blacklists.
    pub fn contains(blacklist_url: &[CompactString], link: &CompactString) -> bool {
        blacklist_url.contains(link)
    }
}

/// The selectors type. The values are held to make sure the relative domain can be crawled upon base redirects.
pub type RelativeSelectors = (
    // base domain
    compact_str::CompactString,
    smallvec::SmallVec<[compact_str::CompactString; 2]>,
    // redirected domain
    compact_str::CompactString,
);
