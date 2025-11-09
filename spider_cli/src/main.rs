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

pub mod build_folders;
pub mod options;

use clap::Parser;
use options::{Cli, Commands};
use tokio::io::AsyncWriteExt;

use serde_json::{json, Value};

use spider::client::header::{HeaderMap, HeaderValue};
use spider::features::chrome_common::RequestInterceptConfiguration;
use spider::hashbrown::HashMap;
use spider::page::Page;
use spider::string_concat::{string_concat, string_concat_impl};
use spider::tokio;
use spider::utils::header_utils::header_map_to_hash_map;
use spider::utils::log;
use spider::website::Website;
use std::path::{Path, PathBuf};

use crate::build_folders::build_local_path;

/// convert the headers to json
fn headers_to_json(headers: &Option<HeaderMap<HeaderValue>>) -> Value {
    if let Some(headers) = &headers {
        serde_json::to_value(&header_map_to_hash_map(headers)).unwrap_or_default()
    } else {
        Value::Null
    }
}

/// handle the headers
#[cfg(feature = "headers")]
fn handle_headers(res: &Page) -> Value {
    headers_to_json(&res.headers)
}

/// handle the headers
#[cfg(not(feature = "headers"))]
fn handle_headers(_res: &Page) -> Value {
    headers_to_json(&None)
}

/// handle the duration elaspsed to milliseconds.
#[cfg(feature = "time")]
fn handle_time(res: &Page, mut json: Value) -> Value {
    json["duration_elapsed_ms"] = json!(res.get_duration_elapsed().as_millis());
    json
}

/// handle the duration elaspsed to milliseconds.
#[cfg(not(feature = "time"))]
fn handle_time(_res: &Page, mut _json: Value) -> Value {
    _json
}

/// handle the HTTP status code.
#[cfg(feature = "status_code")]
fn handle_status_code(res: &Page, mut json: Value) -> Value {
    json["status_code"] = res.status_code.as_u16().into();
    json
}

/// handle the HTTP status code.
#[cfg(not(feature = "status_code"))]
fn handle_status_code(_res: &Page, mut _json: Value) -> Value {
    _json
}

/// handle the remote address.
#[cfg(feature = "remote_addr")]
fn handle_remote_address(res: &Page, mut json: Value) -> Value {
    json["remote_address"] = json!(res.remote_addr);
    json
}

/// handle the remote address.
#[cfg(not(feature = "remote_addr"))]
fn handle_remote_address(_res: &Page, mut _json: Value) -> Value {
    _json
}

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
        .with_full_resources(cli.full_resources)
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

    let return_headers = cli.return_headers;

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
                            if return_headers {
                                let headers_json = handle_headers(&res);

                                let _ = stdout
                                    .write_all(format!("{} - {}\n", res.get_url(), headers_json).as_bytes())
                                    .await;
                            } else {
                                let _ = stdout.write_all(string_concat!(res.get_url(), "\n").as_bytes()).await;
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
                        let _ = tokio::fs::create_dir_all(tmp_path).await;
                    }

                    let download_path = PathBuf::from(tmp_path);

                    tokio::spawn(async move {
                        website.crawl().await;
                    });

                    while let Ok(mut res) = rx2.recv().await {
                        if let Some(parsed_url) = res.get_url_parsed() {
                            log("Storing", parsed_url);
                            let mut url_path = parsed_url.path().to_string();

                            if url_path.is_empty() {
                                url_path = "/".into();
                            }

                            let final_path = build_local_path(&download_path, &url_path);

                            if let Some(parent) = final_path.parent() {
                                if !parent.exists() {
                                    if let Err(e) = tokio::fs::create_dir_all(parent).await {
                                        eprintln!("Failed to create dirs {:?}: {e}", parent);
                                        continue;
                                    }
                                }
                            }

                            if let Some(bytes) = res.get_bytes() {
                                match tokio::fs::OpenOptions::new()
                                    .write(true)
                                    .create(true)
                                    .truncate(true)
                                    .open(&final_path)
                                    .await
                                {
                                    Ok(mut file) => {
                                        if let Err(e) = file.write_all(bytes).await {
                                            eprintln!("Failed to write {:?}: {e}", final_path);
                                        }
                                    }
                                    Err(e) => {
                                        eprintln!("Unable to open file {:?}: {e}", final_path);
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
                            },
                            "headers": if return_headers {
                               handle_headers(&res)
                            } else {
                                Default::default()
                            }
                        });

                        let page_json = handle_time(&res, page_json);
                        let page_json = handle_status_code(&res, page_json);
                        let page_json = handle_remote_address(&res, page_json);

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
