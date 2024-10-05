extern crate env_logger;
extern crate serde_json;
extern crate spider;

pub mod options;

use crate::spider::tokio::io::AsyncWriteExt;
use clap::Parser;
use options::{Cli, Commands};
use serde_json::json;
use spider::features::chrome_common::RequestInterceptConfiguration;
use spider::hashbrown::HashMap;
use spider::page::get_page_selectors;
use spider::string_concat::string_concat;
use spider::string_concat::string_concat_impl;
use spider::tokio;
use spider::utils::log;
use spider::website::Website;
use std::path::{Path, PathBuf};

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

    let url = if cli.url.starts_with("http") {
        cli.url
    } else {
        string_concat!("https://", cli.url)
    };

    let mut website = Website::new(&url);

    website
        .with_respect_robots_txt(cli.respect_robots_txt)
        .with_subdomains(cli.subdomains)
        .with_chrome_intercept(RequestInterceptConfiguration::new(cli.block_images))
        .with_danger_accept_invalid_certs(cli.accept_invalid_certs)
        .with_tld(cli.tld)
        .with_blacklist_url(
            cli.blacklist_url
                .map(|blacklist_url| blacklist_url.split(',').map(|l| l.into()).collect()),
        )
        .with_budget(cli.budget.as_ref().map(|budget| {
            budget
                .split(',')
                .collect::<Vec<_>>()
                .chunks(2)
                .map(|x| (x[0], x[1].parse::<u32>().unwrap_or_default()))
                .collect::<HashMap<&str, u32>>()
        }));

    match cli.agent {
        Some(agent) => {
            website.with_user_agent(Some(&agent));
        }
        _ => (),
    }
    match cli.delay {
        Some(delay) => {
            website.with_delay(delay);
        }
        _ => (),
    }
    match cli.limit {
        Some(limit) => {
            website.with_limit(limit);
        }
        _ => (),
    }
    match cli.depth {
        Some(depth) => {
            website.with_depth(depth);
        }
        _ => (),
    }
    match cli.external_domains {
        Some(domains) => {
            website.with_external_domains(Some(domains.into_iter()));
        }
        _ => (),
    }

    match website
        .build()
    {
        Ok(mut website) => {
            let mut rx2 = website.subscribe(0).expect("sync feature required");

            match cli.command {
                Some(Commands::CRAWL {
                    sync,
                    output_links,
                }) => {
                    if sync {
                        // remove concurrency
                        website.with_delay(1);
                    }

                    let mut stdout = tokio::io::stdout();

                    tokio::spawn(async move {
                        website.crawl().await;
                    });

                    if output_links {
                        while let Ok(res) = rx2.recv().await {
                            match stdout.write_all(string_concat!(res.get_url(), "\n").as_bytes()).await {
                                _ => ()
                            }
                        }
                    }
                }
                Some(Commands::DOWNLOAD { target_destination }) => {
                    let tmp_dir = target_destination
                        .to_owned()
                        .unwrap_or(String::from("./_temp_spider_downloads/"));

                    let tmp_path = Path::new(&tmp_dir);

                    if !Path::new(&tmp_path).exists() {
                        match tokio::fs::create_dir_all(tmp_path).await {
                            _ => (),
                        };
                    }

                    let download_path = PathBuf::from(tmp_path);

                    tokio::spawn(async move {
                        website.crawl().await;
                    });

                    while let Ok(res) = rx2.recv().await {
                        match res.get_url_parsed() {
                            Some(parsed_url) => {
                                log("Storing", parsed_url);

                                let url_path = parsed_url.path();

                                let split_paths: Vec<&str> = url_path.split('/').collect();
                                let it = split_paths.iter();
                                let last_item = split_paths.last().unwrap_or(&"");
                                let mut download_path = download_path.clone();

                                for p in it {
                                    if p != last_item {
                                        download_path.push(p);

                                        if !Path::new(&download_path).exists() {
                                            match tokio::fs::create_dir_all(&download_path).await {
                                                _ => (),
                                            };
                                        }
                                    } else {
                                        match tokio::fs::OpenOptions::new()
                                        .write(true)
                                        .create(true)
                                        .truncate(true)
                                        .open(&download_path.join(if p.contains('.') {
                                            p.to_string()
                                        } else {
                                            string_concat!(
                                                if p.is_empty() { "index" } else { p },
                                                ".html"
                                            )
                                        })).await {
                                            Ok(mut file) => {
                                                match res.get_bytes() {
                                                    Some(b) => {
                                                        match file.write_all(b).await {
                                                            _ => ()
                                                        }
                                                    }
                                                    _ => (),
                                                }
                                            }
                                            _ => {
                                                eprintln!("Unable to open file.")
                                            }
                                        }
                                    }
                                }
                            }
                            _ => ()
                        }

                    }
                }
                Some(Commands::SCRAPE {
                    output_html,
                    output_links,
                }) => {
                    let mut stdout = tokio::io::stdout();

                    let selectors: Option<spider::RelativeSelectors> = if output_links {
                        get_page_selectors(&url, cli.subdomains, cli.tld)
                    } else {
                        None
                    };

                    tokio::spawn(async move {
                        website.crawl().await;
                    });

                    while let Ok(res) = rx2.recv().await {
                        let page_json = json!({
                            "url": res.get_url(),
                            "html": if output_html {
                                res.get_html()
                            } else {
                                Default::default()
                            },
                            "links": match selectors {
                                Some(ref s) => res.links(&s).await.iter().map(|i| i.inner().to_string()).collect::<serde_json::Value>(),
                                _ => Default::default()
                            }
                        });

                        match serde_json::to_string_pretty(&page_json) {
                            Ok(j) => {
                               if let Err(e) = stdout.write_all(string_concat!(j, "\n").as_bytes()).await {
                                    eprintln!("{:?}", e)
                               }
                            }
                            Err(e) =>  eprintln!("{:?}", e)
                        }
                    }
                }
                None => ()
            }
        }
        _ =>  println!("Invalid website URL passed in. The url should start with http:// or https:// following the website domain ex: https://example.com.")
    }
}
