//! `cargo run --example download_to_react`
extern crate convert_case;
extern crate env_logger;
extern crate htr;
extern crate spider;

use spider::percent_encoding::{percent_encode, NON_ALPHANUMERIC};
use spider::utils::log;
use spider::website::Website;

use convert_case::{Case, Casing};
use env_logger::Env;
use htr::convert_to_react;
use spider::tokio;
use std::env;
use std::fs::OpenOptions;
use std::io::Write;

#[tokio::main]
async fn main() {
    let target_dir = env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| "./target".to_string());

    // view the target dist for the downloads
    std::fs::create_dir_all(format!("{}/downloads", target_dir)).unwrap_or_default();

    let env = Env::default()
        .filter_or("RUST_LOG", "info")
        .write_style_or("RUST_LOG_STYLE", "always");

    env_logger::init_from_env(env);

    let website_name = "https://rsseau.fr";

    let mut website: Website = Website::new(website_name);
    website.configuration.respect_robots_txt = true;
    website.configuration.delay = 0;

    website.scrape().await;

    match website.get_pages() {
        Some(pages) => {
            for page in pages.iter() {
                let download_file =
                    percent_encode(page.get_url().as_bytes(), NON_ALPHANUMERIC).to_string();

                let download_file = if download_file.is_empty() {
                    "index"
                } else {
                    &download_file
                };

                let mut file = OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .open(&format!("{}/downloads/{}.tsx", target_dir, download_file))
                    .expect("Unable to open file");

                let download_file = download_file.to_case(Case::Camel);
                let download_file = download_file[0..1].to_uppercase() + &download_file[1..];

                let react_component = convert_to_react(&page.get_html(), download_file.to_string());
                let react_component = react_component.as_bytes();

                file.write_all(react_component).unwrap_or_default();

                log("downloaded", download_file);
            }
        }
        _ => (),
    };
}
