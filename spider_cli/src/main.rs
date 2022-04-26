extern crate spider;
extern crate env_logger;

pub mod options;

use clap::Parser;
use options::{Cli, Commands};
use spider::website::Website;

fn main() {
    let cli = Cli::parse();

    if cli.verbose {
        use env_logger::Env;
        let env = Env::default()
            .filter_or("RUST_LOG", "info")
            .write_style_or("RUST_LOG_STYLE", "always");

        env_logger::init_from_env(env);
    }

    let mut website: Website = Website::new(&cli.domain);
    
    let delay = cli.delay.unwrap_or(website.configuration.delay);
    let concurrency = cli.concurrency.unwrap_or(website.configuration.concurrency);
    let user_agent = cli.user_agent.unwrap_or(website.configuration.user_agent.to_string());
    let blacklist_url = cli.blacklist_url.unwrap_or_default();

    website.configuration.respect_robots_txt = cli.respect_robots_txt;
    website.configuration.delay = delay;
    website.configuration.concurrency = concurrency;

    if !blacklist_url.is_empty() {
        let blacklist_url: Vec<String> = blacklist_url.split(",").map(|l| l.to_string()).collect();
        website.configuration.blacklist_url.extend(blacklist_url);
    }

    if !user_agent.is_empty() {
        website.configuration.user_agent = Box::leak(user_agent.to_owned().into_boxed_str());
    }

    match &cli.command {
        Some(Commands::CRAWL { sync }) => {
            if *sync {
                website.crawl_sync();
            } else {
                website.crawl();
            }
        }
        None => {}
    }
}
