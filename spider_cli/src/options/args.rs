use crate::options::sub_command::Commands;
use clap::{ArgAction, Parser};

/// program to crawl a website and gather valid web urls.
#[derive(Parser)]
#[clap(author, version, about, long_about = None)]
#[command(arg_required_else_help = true)]
pub struct Cli {
    /// Build main sub commands
    #[clap(subcommand)]
    pub command: Option<Commands>,
    /// The website URL to crawl.
    #[clap(short, long)]
    pub url: String,
    /// Respect robots.txt file
    #[clap(short, long)]
    pub respect_robots_txt: bool,
    /// Allow sub-domain crawling.
    #[clap(short, long)]
    pub subdomains: bool,
    /// Allow all tlds for domain.
    #[clap(short, long)]
    pub tld: bool,
    #[clap(short = 'H', long)]
    /// Return the headers of the page.  Requires the `headers` flag enabled.
    pub return_headers: bool,
    /// Print page visited on standard output
    #[clap(short, long)]
    pub verbose: bool,
    /// Polite crawling delay in milli seconds
    #[clap(short = 'D', long)]
    pub delay: Option<u64>,
    /// The max pages allowed to crawl.
    #[clap(long)]
    pub limit: Option<u32>,
    /// Comma seperated string list of pages to not crawl or regex with feature enabled
    #[clap(long)]
    pub blacklist_url: Option<String>,
    /// User-Agent
    #[clap(short, long)]
    pub agent: Option<String>,
    /// Crawl Budget preventing extra paths from being crawled. Use commas to split the path followed by the limit ex: "*,1" - to only allow one page.
    #[clap(short = 'B', long)]
    pub budget: Option<String>,
    /// Set external domains to group with crawl.
    #[clap(short = 'E', long)]
    pub external_domains: Option<Vec<String>>,
    #[clap(short = 'b', long)]
    /// Block Images from rendering when using Chrome. Requires the `chrome_intercept` flag enabled.
    pub block_images: bool,
    /// The crawl depth limits.
    #[clap(short, long)]
    pub depth: Option<usize>,
    /// Dangerously accept invalid certficates
    #[clap(long)]
    pub accept_invalid_certs: bool,
    /// Gather all content that relates to the domain like css,jss, and etc.
    #[clap(long)]
    pub full_resources: bool,
    /// Use browser rendering mode (headless) for crawl/scrape/download. Requires the `chrome` feature.
    #[clap(long, conflicts_with = "http")]
    pub headless: bool,
    /// Force HTTP-only mode (no browser rendering), even when built with `chrome`.
    #[clap(long, action = ArgAction::SetTrue, conflicts_with = "headless")]
    pub http: bool,
    /// The proxy url to use.
    #[clap(short, long)]
    pub proxy_url: Option<String>,
    /// Spider Cloud API key. Sign up at https://spider.cloud for an API key.
    #[clap(long)]
    pub spider_cloud_key: Option<String>,
    /// Spider Cloud mode: proxy (default), api, unblocker, fallback, or smart.
    #[clap(long, default_value = "proxy")]
    pub spider_cloud_mode: Option<String>,
    /// Wait for network request to be idle within a time frame period (500ms no network connections) with an optional timeout in milliseconds.
    #[clap(long)]
    pub wait_for_idle_network: Option<u64>,
    /// Wait for network request with a max timeout (0 connections) with an optional timeout in milliseconds.
    #[clap(long)]
    pub wait_for_idle_network0: Option<u64>,
    /// Wait for network to be almost idle with a max timeout (max 2 connections) with an optional timeout in milliseconds.
    #[clap(long)]
    pub wait_for_almost_idle_network0: Option<u64>,
    /// Wait for idle dom mutations for target element (defaults to "body") with a 30s timeout.
    #[clap(long)]
    pub wait_for_idle_dom: Option<String>,
    /// Wait for a specific CSS selector to appear with a 60s timeout.
    #[clap(long)]
    pub wait_for_selector: Option<String>,
    /// Wait for a fixed delay in milliseconds.
    #[clap(long)]
    pub wait_for_delay: Option<u64>,
}
