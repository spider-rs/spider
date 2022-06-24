extern crate spider;
extern crate env_logger;
extern crate serde_json;

pub mod options;

use clap::Parser;
use options::{Cli, Commands};
use spider::website::Website;
use std::io::{self, Write};

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
    website.configuration.subdomains = cli.subdomains;

    if !blacklist_url.is_empty() {
        let blacklist_url: Vec<String> = blacklist_url.split(",").map(|l| l.to_string()).collect();
        website.configuration.blacklist_url.extend(blacklist_url);
    }

    if !user_agent.is_empty() {
        website.configuration.user_agent = user_agent;
    }

    match &cli.command {
        Some(Commands::CRAWL { sync, output_links }) => {
            if *sync {
                website.crawl_sync();
            } else {
                website.crawl();
            }

            if *output_links {
                let links: Vec<_> = website.get_links().iter().collect();
                io::stdout().write_all(format!("{:?}", links).as_bytes()).unwrap();
            }

        }
        Some(Commands::SCRAPE { output_html, output_links }) => {
            use serde_json::{json};

            website.scrape();

            let mut page_objects: Vec<_> = vec![];

            for page in website.get_pages() {
                let mut links: Vec<String> = vec![];
                let mut html: &String = &String::new();

                if *output_links {
                    let page_links = page.links(cli.subdomains);
                    links.extend(page_links);
                }

                if *output_html {
                    html = page.get_html();
                }

                let page_json = json!({
                    "url": page.get_url(),
                    "links": links,
                    "html": html,
                });

                page_objects.push(page_json);
            }
            
            let j = serde_json::to_string_pretty(&page_objects).unwrap();

            io::stdout().write_all(j.as_bytes()).unwrap();

        }
        None => {}
    }
}
