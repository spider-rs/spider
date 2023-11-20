extern crate env_logger;
extern crate serde_json;
extern crate spider;

pub mod options;

use clap::Parser;
use options::{Cli, Commands};
use spider::hashbrown::HashMap;
use spider::page::get_page_selectors;
use spider::string_concat::string_concat;
use spider::string_concat::string_concat_impl;
use spider::tokio;
use spider::url::Url;
use spider::utils::log;
use spider::website::Website;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[cfg(feature = "chrome")]
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

    let mut website: Website = Website::new(&cli.domain)
        .with_respect_robots_txt(cli.respect_robots_txt)
        .with_delay(cli.delay.unwrap_or_default())
        .with_subdomains(cli.subdomains)
        .with_tld(cli.tld)
        .with_user_agent(cli.user_agent.as_deref())
        .with_budget(match cli.budget {
            Some(ref budget) => Some(
                budget
                    .split(",")
                    .collect::<Vec<_>>()
                    .chunks(2)
                    .map(|x| (x[0], x[1].parse::<u32>().unwrap_or_default()))
                    .collect::<HashMap<&str, u32>>(),
            ),
            _ => None,
        })
        .with_blacklist_url(match cli.blacklist_url {
            Some(blacklist_url) => Some(blacklist_url.split(',').map(|l| l.into()).collect()),
            _ => None,
        })
        .with_external_domains(Some(cli.external_domains.unwrap_or_default().into_iter()))
        .build()
        .unwrap();

    match &cli.command {
        Some(Commands::CRAWL {
            sync: _,
            output_links,
        }) => {
            if cli.headless {
                website.crawl_chrome().await;
            } else {
                website.crawl().await;
            }

            if *output_links {
                let links: Vec<String> = website
                    .get_links()
                    .iter()
                    .map(|l| l.inner().to_string())
                    .collect();
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

            if cli.headless {
                website.scrape_chrome().await;
            } else {
                website.scrape().await;
            }

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

                                    let mut download_path = PathBuf::from(tmp_path);

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
                                                .open(&download_path.join(if p.contains(".") {
                                                    p.to_string()
                                                } else {
                                                    string_concat!(
                                                        if p.is_empty() { "index" } else { p },
                                                        ".html"
                                                    )
                                                }))
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
                                    links.push(link.as_ref().to_string());
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

#[cfg(not(feature = "chrome"))]
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

    let mut website: Website = Website::new(&cli.domain)
        .with_respect_robots_txt(cli.respect_robots_txt)
        .with_delay(cli.delay.unwrap_or_default())
        .with_subdomains(cli.subdomains)
        .with_tld(cli.tld)
        .with_user_agent(cli.user_agent.as_deref())
        .with_budget(match cli.budget {
            Some(ref budget) => Some(
                budget
                    .split(",")
                    .collect::<Vec<_>>()
                    .chunks(2)
                    .map(|x| (x[0], x[1].parse::<u32>().unwrap_or_default()))
                    .collect::<HashMap<&str, u32>>(),
            ),
            _ => None,
        })
        .with_blacklist_url(match cli.blacklist_url {
            Some(blacklist_url) => Some(blacklist_url.split(',').map(|l| l.into()).collect()),
            _ => None,
        })
        .with_external_domains(Some(cli.external_domains.unwrap_or_default().into_iter()))
        .build()
        .unwrap();

    match &cli.command {
        Some(Commands::CRAWL {
            sync: _,
            output_links,
        }) => {
            website.crawl().await;

            if *output_links {
                let links: Vec<String> = website
                    .get_links()
                    .iter()
                    .map(|l| l.inner().to_string())
                    .collect();
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

                                    let mut download_path = PathBuf::from(tmp_path);

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
                                                .open(&download_path.join(if p.contains(".") {
                                                    p.to_string()
                                                } else {
                                                    string_concat!(
                                                        if p.is_empty() { "index" } else { p },
                                                        ".html"
                                                    )
                                                }))
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
                                    links.push(link.as_ref().to_string());
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
