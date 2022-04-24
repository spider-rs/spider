use clap::Subcommand;

#[derive(Subcommand)]
pub enum Commands {
    /// crawl the website.
    CRAWL {},
}
