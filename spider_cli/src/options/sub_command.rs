use clap::Subcommand;

#[derive(Subcommand)]
pub enum Commands {
    /// Crawl the website extracting links.
    CRAWL {
        /// sequentially one by one crawl pages
        #[clap(short, long)]
        sync: bool,
        /// stdout all links crawled
        #[clap(short, long)]
        output_links: bool,
    },
    /// Scrape the website extracting html and links returning the output as jsonl.
    SCRAPE {
        /// stdout all pages links crawled
        #[clap(short, long)]
        output_links: bool,
        /// stdout all pages html crawled
        #[clap(long)]
        output_html: bool,
    },
    /// Download html markup to destination.
    DOWNLOAD {
        /// store files at target destination
        #[clap(short, long)]
        target_destination: Option<String>,
    },
    /// Authenticate with the Spider Cloud service. Stores your API key locally for remote crawls.
    /// Sign up at https://spider.cloud to get an API key.
    #[clap(alias = "auth", alias = "login")]
    AUTHENTICATE {
        /// Your Spider Cloud API key (e.g. sk-...). If omitted, reads from stdin.
        api_key: Option<String>,
    },
}
