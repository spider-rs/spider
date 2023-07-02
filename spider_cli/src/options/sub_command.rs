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
    /// Scrape the website extracting html and links.
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
}
