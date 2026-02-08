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
//! ## Spider Cloud Integration
//!
//! Use [Spider Cloud](https://spider.cloud) for anti-bot bypass, proxy rotation, and high-throughput
//! data collection. Enable the `spider_cloud` feature and set your API key:
//!
//! ```no_run
//! use spider::tokio;
//! use spider::website::Website;
//!
//! #[tokio::main]
//! async fn main() {
//!     let mut website: Website = Website::new("https://example.com")
//!         .with_spider_cloud("YOUR_API_KEY")
//!         .with_limit(10)
//!         .build()
//!         .unwrap();
//!
//!     website.crawl().await;
//!
//!     for link in website.get_links() {
//!         println!("- {:?}", link.as_ref());
//!     }
//! }
//! ```
//!
//! ## Chrome Rendering
//!
//! Enable the `chrome` feature to render JavaScript-heavy pages. Use the env var
//! `CHROME_URL` to connect to a remote instance:
//!
//! ```no_run
//! use spider::tokio;
//! use spider::website::Website;
//!
//! #[tokio::main]
//! async fn main() {
//!     let mut website: Website = Website::new("https://spider.cloud")
//!         .with_limit(10)
//!         .with_chrome_intercept(Default::default())
//!         .build()
//!         .unwrap();
//!
//!     let mut rx = website.subscribe(16).unwrap();
//!
//!     tokio::spawn(async move {
//!         while let Ok(page) = rx.recv().await {
//!             println!("{} - {}", page.get_url(), page.get_html_bytes_u8().len());
//!         }
//!     });
//!
//!     website.crawl().await;
//! }
//! ```
//!
//! ## Feature flags
//!
//! ### Core
//!
//! - `ua_generator`: Enables auto generating a random real User-Agent.
//! - `regex`: Enables blacklisting paths with regex.
//! - `glob`: Enables [url glob](https://everything.curl.dev/cmdline/globbing) support.
//! - `fs`: Enables storing resources to disk for parsing (may greatly increase performance at the cost of temp storage). Enabled by default.
//! - `sitemap`: Include sitemap pages in results.
//! - `time`: Enables duration tracking per page.
//! - `encoding`: Enables handling the content with different encodings like Shift_JIS.
//! - `serde`: Enables serde serialization support.
//! - `sync`: Subscribe to changes for Page data processing async.
//! - `control`: Enables the ability to pause, start, and shutdown crawls on demand.
//! - `full_resources`: Enables gathering all content that relates to the domain like CSS, JS, and etc.
//! - `cookies`: Enables cookies storing and setting to use for request.
//! - `spoof`: Spoof HTTP headers for the request.
//! - `headers`: Enables the extraction of header information on each retrieved page. Adds a `headers` field to the page struct.
//! - `balance`: Enables balancing the CPU and memory to scale more efficiently.
//! - `cron`: Enables the ability to start cron jobs for the website.
//! - `tracing`: Enables tokio tracing support for diagnostics.
//! - `cowboy`: Enables full concurrency mode with no throttle.
//! - `llm_json`: Enables LLM-friendly JSON parsing.
//! - `page_error_status_details`: Enables storing detailed error status information on pages.
//! - `extra_information`: Enables extra page metadata collection.
//! - `cmd`: Enables tokio process support.
//! - `io_uring`: Enables Linux io_uring support for async I/O (default on Linux).
//! - `simd`: Enables SIMD-accelerated JSON parsing.
//! - `inline-more`: More aggressive function inlining for performance (may increase compile times).
//!
//! ### Storage
//!
//! - `disk`: Enables SQLite hybrid disk storage to balance memory usage with no TLS.
//! - `disk_native_tls`: Enables SQLite hybrid disk storage to balance memory usage with native TLS.
//! - `disk_aws`: Enables SQLite hybrid disk storage to balance memory usage with AWS TLS.
//!
//! ### Caching
//!
//! - `cache`: Enables HTTP caching request to disk.
//! - `cache_mem`: Enables HTTP caching request to persist in memory.
//! - `cache_openai`: Enables caching the OpenAI request. This can drastically save costs when developing AI workflows.
//! - `cache_gemini`: Enables caching Gemini AI requests.
//! - `cache_chrome_hybrid`: Enables hybrid Chrome + HTTP caching to disk.
//! - `cache_chrome_hybrid_mem`: Enables hybrid Chrome + HTTP caching in memory.
//!
//! ### Chrome / Browser
//!
//! - `chrome`: Enables Chrome headless rendering, use the env var `CHROME_URL` to connect remotely.
//! - `chrome_headed`: Enables Chrome headful rendering.
//! - `chrome_cpu`: Disable GPU usage for Chrome browser.
//! - `chrome_stealth`: Enables stealth mode to make it harder to be detected as a bot.
//! - `chrome_store_page`: Store the page object to perform other actions like taking screenshots conditionally.
//! - `chrome_screenshot`: Enables storing a screenshot of each page on crawl. Defaults the screenshots to the `./storage/` directory. Use the env variable `SCREENSHOT_DIRECTORY` to adjust the directory.
//! - `chrome_intercept`: Allows intercepting network requests to speed up processing.
//! - `chrome_headless_new`: Use `headless=new` to launch the Chrome instance.
//! - `chrome_simd`: Enables SIMD optimizations for Chrome message parsing.
//! - `chrome_tls_connection`: Enables TLS connection support for Chrome.
//! - `chrome_serde_stacker`: Enables serde stacker for deeply nested Chrome messages.
//! - `chrome_remote_cache`: Enables remote Chrome caching in memory.
//! - `chrome_remote_cache_disk`: Enables remote Chrome caching to disk.
//! - `chrome_remote_cache_mem`: Enables remote Chrome caching in memory only.
//! - `adblock`: Enables adblock support for Chrome to block ads during rendering.
//! - `real_browser`: Enables the ability to bypass protected pages.
//! - `smart`: Enables smart mode. This runs request as HTTP until JavaScript rendering is needed. This avoids sending multiple network requests by re-using the content.
//!
//! ### WebDriver
//!
//! - `webdriver`: Enables WebDriver support via [thirtyfour](https://docs.rs/thirtyfour). Use with chromedriver, geckodriver, or Selenium.
//! - `webdriver_headed`: Enables WebDriver headful mode.
//! - `webdriver_stealth`: Enables stealth mode for WebDriver.
//! - `webdriver_chrome`: WebDriver with Chrome browser.
//! - `webdriver_firefox`: WebDriver with Firefox browser.
//! - `webdriver_edge`: WebDriver with Edge browser.
//! - `webdriver_screenshot`: Enables screenshots via WebDriver.
//!
//! ### AI / LLM
//!
//! - `openai`: Enables OpenAI to generate dynamic browser executable scripts. Make sure to use the env var `OPENAI_API_KEY`.
//! - `gemini`: Enables Gemini AI to generate dynamic browser executable scripts. Make sure to use the env var `GEMINI_API_KEY`.
//!
//! ### Spider Cloud
//!
//! - `spider_cloud`: Enables [Spider Cloud](https://spider.cloud) integration for anti-bot bypass, proxy rotation, and API-based crawling.
//!
//! ### Agent
//!
//! - `agent`: Enables the [spider_agent](https://docs.rs/spider_agent) multimodal autonomous agent.
//! - `agent_openai`: Agent with OpenAI provider.
//! - `agent_chrome`: Agent with Chrome browser context.
//! - `agent_webdriver`: Agent with WebDriver context.
//! - `agent_skills`: Agent with dynamic skill system for web automation challenges.
//! - `agent_skills_s3`: Agent skills with S3 storage.
//! - `agent_fs`: Agent with filesystem support for temp storage.
//! - `agent_search_serper`: Agent with [Serper](https://serper.dev) search integration.
//! - `agent_search_brave`: Agent with [Brave Search](https://brave.com/search/api/) integration.
//! - `agent_search_bing`: Agent with [Bing Search](https://www.microsoft.com/en-us/bing/apis/bing-web-search-api) integration.
//! - `agent_search_tavily`: Agent with [Tavily](https://tavily.com) search integration.
//! - `agent_full`: Full agent with all features enabled.
//!
//! ### Search
//!
//! - `search`: Enables search provider base.
//! - `search_serper`: Enables [Serper](https://serper.dev) search integration.
//! - `search_brave`: Enables [Brave Search](https://brave.com/search/api/) integration.
//! - `search_bing`: Enables [Bing Search](https://www.microsoft.com/en-us/bing/apis/bing-web-search-api) integration.
//! - `search_tavily`: Enables [Tavily](https://tavily.com) search integration.
//!
//! ### Networking
//!
//! - `socks`: Enables SOCKS5 proxy support.
//! - `wreq`: Enables the [wreq](https://docs.rs/wreq) HTTP client alternative with built-in impersonation.
//!
//! ### Distributed
//!
//! - `decentralized`: Enables decentralized processing of IO, requires the [spider_worker](https://docs.rs/crate/spider_worker/latest) startup before crawls.
//! - `decentralized_headers`: Enables the extraction of suppressed header information of the decentralized processing of IO. This is needed if `headers` is set in both [spider](https://docs.rs/spider/latest/spider/) and [spider_worker](https://docs.rs/crate/spider_worker/latest).
//! - `firewall`: Enables spider_firewall crate to prevent bad websites from crawling.
//!
//! Additional learning resources include:
//!
//! - [Spider Repository Examples](https://github.com/spider-rs/spider/tree/main/examples)
//! - [Spider Cloud](https://spider.cloud)
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
