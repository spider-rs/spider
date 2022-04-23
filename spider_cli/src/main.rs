extern crate spider;

pub mod options;

use clap::Parser;
use options::{Cli, Commands};
use spider::website::Website;

fn main() {
    let cli = Cli::parse();
    let mut website: Website = Website::new(&cli.domain);
    let delay = cli.delay.unwrap_or_default();
    let concurrency = cli.concurrency.unwrap_or_default();
    let user_agent = cli.user_agent.unwrap_or_default();
    let blacklist_url = cli.blacklist_url.unwrap_or_default();

    website.configuration.respect_robots_txt = cli.respect_robots_txt;
    website.configuration.verbose = cli.verbose;
    website.configuration.delay = delay;
    website.configuration.concurrency = concurrency;
    website.page_store_ignore = true;

    if !blacklist_url.is_empty() {
        website.configuration.blacklist_url.push(blacklist_url);
    }

    if !user_agent.is_empty() {
        website.configuration.user_agent = Box::leak(user_agent.to_owned().into_boxed_str());
    }

    match &cli.command {
        Some(Commands::CRAWL { stack }) => {
            if *stack {
                website.crawl_stack(None);
            } else {
                website.crawl();
            }
        }
        None => {}
    }
}
