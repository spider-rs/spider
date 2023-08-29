extern crate env_logger;
extern crate serde_json;
extern crate spider;

pub mod options;

use clap::Parser;
use options::{Cli, Commands};
use spider::compact_str::CompactString;
use spider::page::get_page_selectors;
use spider::tokio;
use spider::url::Url;
use spider::utils::log;
use spider::website::Website;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
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
        Some(Commands::CRAWL {
            sync: _,
            output_links,
        }) => {
            website.crawl().await;

            if *output_links {
                let links: Vec<_> = website.get_links().iter().collect();
                io::stdout()
                    .write_all(format!("{:?}", links).as_bytes())
                    .unwrap();
            }
        }
        Some(Commands::DOWNLOAD { target_destination }) => {
            let tmp_dir: String = target_destination
                .to_owned()
                .unwrap_or(String::from("./_temp_spider_downloads/"));
            let tmp_path = Path::new(&tmp_dir);

            if !Path::new(&tmp_path).exists() {
                match std::fs::create_dir_all(&tmp_path) {
                    _ => (),
                };
            }

            website.scrape().await;
            let selectors = get_page_selectors(&cli.domain, cli.subdomains, cli.tld);

            if selectors.is_some() {
                match website.get_pages() {
                    Some(pages) => {
                        for page in pages.iter() {
                            let page_url = page.get_url();

                            match Url::parse(page_url) {
                                Ok(parsed_url) => {
                                    let url_path = parsed_url.path();

                                    log("- ", page_url);

                                    let split_paths: Vec<&str> = url_path.split("/").collect();
                                    let it = split_paths.iter();
                                    let last_item = split_paths.last().unwrap_or(&"");

                                    let mut download_path = PathBuf::from(tmp_path.clone());

                                    for p in it {
                                        if p != last_item {
                                            download_path.push(p);

                                            if !Path::new(&download_path).exists() {
                                                match std::fs::create_dir_all(&download_path) {
                                                    _ => (),
                                                };
                                            }
                                        } else {
                                            let mut file = std::fs::OpenOptions::new()
                                                .write(true)
                                                .create(true)
                                                .truncate(true)
                                                .open(&download_path.join(format!(
                                                    "{}.html",
                                                    if p.is_empty() { "index" } else { p }
                                                )))
                                                .expect("Unable to open file");

                                            match page.get_bytes() {
                                                Some(b) => {
                                                    file.write_all(b).unwrap_or_default();
                                                }
                                                _ => (),
                                            }
                                        }
                                    }
                                }
                                _ => (),
                            }
                        }
                    }
                    None => {}
                }
            }
        }
        Some(Commands::SCRAPE {
            output_html,
            output_links,
        }) => {
            use serde_json::json;

            website.scrape().await;

            let mut page_objects: Vec<_> = vec![];

            let selectors = get_page_selectors(&cli.domain, cli.subdomains, cli.tld);

            if selectors.is_some() {
                let selectors = Arc::new(unsafe { selectors.unwrap_unchecked() });

                match website.get_pages() {
                    Some(pages) => {
                        for page in pages.iter() {
                            let mut links: Vec<String> = vec![];

                            if *output_links {
                                let page_links = page.links(&*selectors).await;

                                for link in page_links {
                                    links.push(link.as_ref().to_owned());
                                }
                            }

                            let page_json = json!({
                                "url": page.get_url(),
                                "links": links,
                                "html": if *output_html {
                                    page.get_html()
                                } else {
                                    Default::default()
                                },
                            });

                            page_objects.push(page_json);
                        }
                    }
                    _ => (),
                }
            }

            let j = serde_json::to_string_pretty(&page_objects).unwrap();

            io::stdout().write_all(j.as_bytes()).unwrap();
        }
        None => {}
    }
}
