use clap::Subcommand;

#[derive(Subcommand)]
pub enum Commands {
    /// crawl the website.
    CRAWL {
        /// sequentially one by one crawl pages
        #[clap(short, long)]
        sync: bool,
        /// stdout all links crawled
        #[clap(short, long)]
        output_links: bool,
    },
}
