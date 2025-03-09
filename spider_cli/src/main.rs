// performance reasons jemalloc memory backend for dedicated work and large crawls
#[cfg(all(
    not(windows),
    not(target_os = "android"),
    not(target_env = "musl"),
    feature = "jemalloc"
))]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

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

    if let Some(ref agent) = cli.agent {
        website.with_user_agent(Some(agent));
    }
    if let Some(delay) = cli.delay {
        website.with_delay(delay);
    }
    if let Some(limit) = cli.limit {
        website.with_limit(limit);
    }
    if let Some(depth) = cli.depth {
        website.with_depth(depth);
    }
    if let Some(domains) = cli.external_domains {
        website.with_external_domains(Some(domains.into_iter()));
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
                            let _ = stdout.write_all(string_concat!(res.get_url(), "\n").as_bytes()).await;
                        }
                    }
                }
                Some(Commands::DOWNLOAD { target_destination }) => {
                    let tmp_dir = target_destination
                        .to_owned()
                        .unwrap_or(String::from("./_temp_spider_downloads/"));

                    let tmp_path = Path::new(&tmp_dir);

                    if !Path::new(&tmp_path).exists() {
                        let _ = tokio::fs::create_dir_all(tmp_path).await;
                    }

                    let download_path = PathBuf::from(tmp_path);

                    tokio::spawn(async move {
                        website.crawl().await;
                    });

                    while let Ok(res) = rx2.recv().await {
                        if let Some(parsed_url) = res.get_url_parsed_ref() {
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
                                            let _ = tokio::fs::create_dir_all(&download_path).await;
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
                                                if let Some(b) = res.get_bytes() {
                                                    let _ = file.write_all(b).await;
                                                }
                                            }
                                            _ => {
                                                eprintln!("Unable to open file.")
                                            }
                                        }
                                    }
                                }
                        }
                    }
                }
                Some(Commands::SCRAPE {
                    output_html,
                    output_links,
                }) => {
                    let mut stdout = tokio::io::stdout();

                    if output_links {
                        website.configuration.return_page_links = true;
                    }

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
                            "links": match res.page_links {
                                Some(ref s) => s.iter().map(|i| i.inner().to_string()).collect::<serde_json::Value>(),
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
