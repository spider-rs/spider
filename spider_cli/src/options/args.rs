use crate::options::sub_command::Commands;
use clap::Parser;

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
    #[clap(short='H', long)]
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
}
