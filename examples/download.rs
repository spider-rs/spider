//! `cargo run --example download`
extern crate env_logger;
extern crate spider;

use spider::percent_encoding::{percent_encode, NON_ALPHANUMERIC};
use spider::utils::log;
use spider::website::Website;

use env_logger::Env;
use spider::tokio;
use std::fs::OpenOptions;
use std::io::Write;

#[tokio::main]
async fn main() {
    // view the target dist for the downloads
    std::fs::create_dir_all("./target/downloads").unwrap_or_default();

    let env = Env::default()
        .filter_or("RUST_LOG", "info")
        .write_style_or("RUST_LOG_STYLE", "always");

    env_logger::init_from_env(env);

    let website_name = "https://rsseau.fr/en";

    let mut website: Website = Website::new(website_name);

    website.scrape().await;

    for page in website.get_pages().unwrap().iter() {
        let download_file = percent_encode(page.get_url().as_bytes(), NON_ALPHANUMERIC).to_string();

        let download_file = if download_file.is_empty() {
            "index"
        } else {
            &download_file
        };

        let download_file = format!("./target/downloads/{}.html", download_file);

        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&download_file)
            .expect("Unable to open file");

        match page.get_bytes() {
            Some(b) => {
                file.write_all(b).unwrap_or_default();
            }
            _ => (),
        }

        log("downloaded", download_file)
    }
}
