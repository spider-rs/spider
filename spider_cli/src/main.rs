extern crate env_logger;
extern crate serde_json;
extern crate spider;

pub mod options;

use clap::Parser;
use options::{Cli, Commands};
use spider::compact_str::CompactString;
use spider::page::get_page_selectors;
use spider::tokio;
use spider::website::Website;
use std::io::{self, Write};
use std::sync::Arc;

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    if cli.verbose {
        use env_logger::Env;
        let env = Env::default()
            .filter_or("RUST_LOG", "info")
            .write_style_or("RUST_LOG_STYLE", "always");

        env_logger::init_from_env(env);
    }

    let mut website: Website = Website::new(&cli.domain);

    let delay = cli.delay.unwrap_or_else(|| website.configuration.delay);
    let blacklist_url = cli.blacklist_url.unwrap_or_default();

    website.configuration.respect_robots_txt = cli.respect_robots_txt;
    website.configuration.delay = delay;
    website.configuration.subdomains = cli.subdomains;
    website.configuration.tld = cli.tld;

    if !blacklist_url.is_empty() {
        let blacklist_url: Vec<CompactString> =
            blacklist_url.split(',').map(|l| l.into()).collect();
        let blacklists = website
            .configuration
            .blacklist_url
            .insert(Default::default());

        blacklists.extend(blacklist_url);
    }

    match cli.user_agent {
        Some(user_agent) => {
            website.configuration.user_agent = Some(Box::new(user_agent.into()));
        }
        _ => {}
    }

    match &cli.command {
        Some(Commands::CRAWL { sync, output_links }) => {
            if *sync {
                website.crawl_sync().await;
            } else {
                website.crawl().await;
            }

            if *output_links {
                let links: Vec<_> = website.get_links().iter().collect();
                io::stdout()
                    .write_all(format!("{:?}", links).as_bytes())
                    .unwrap();
            }
        }
        Some(Commands::SCRAPE {
            output_html,
            output_links,
        }) => {
            use serde_json::json;

            website.scrape().await;

            let mut page_objects: Vec<_> = vec![];

            let selectors = Arc::new(get_page_selectors(&cli.domain, cli.subdomains, cli.tld));

            for page in website.get_pages() {
                let mut links: Vec<String> = vec![];
                let mut html: &String = &String::new();

                if *output_links {
                    let page_links = page.links(&*selectors, Some(true)).await;

                    for link in page_links {
                        links.push(link.as_ref().to_owned());
                    }
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
