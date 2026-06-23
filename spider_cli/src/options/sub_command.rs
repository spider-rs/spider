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
    /// Scrape a page, returning its content as markdown (jsonl). Use --return-format
    /// for another format, or --output-html for the raw HTML.
    SCRAPE {
        /// Include the page links in the output.
        #[clap(short, long)]
        output_links: bool,
        /// Return the raw HTML instead of the transformed (markdown) content.
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
    /// With no arguments it signs you in through your browser (OAuth) and provisions a key.
    /// Sign up at https://spider.cloud to get started.
    #[clap(alias = "auth", alias = "login")]
    AUTHENTICATE {
        /// Your Spider Cloud API key (e.g. sk-...). If omitted, sign in via the browser.
        api_key: Option<String>,
        /// Paste/read the API key from stdin instead of opening the browser.
        #[clap(long)]
        paste: bool,
    },
}
