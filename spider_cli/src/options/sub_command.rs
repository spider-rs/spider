use clap::Subcommand;

#[derive(Subcommand)]
pub enum Commands {
    /// crawl the website.
    CRAWL {
        /// use the stack to crawl
        #[clap(short, long)]
        stack: bool,
    },
}
