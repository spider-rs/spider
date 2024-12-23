//! `cargo run --example subscribe_download`
extern crate env_logger;
extern crate spider;

use spider::percent_encoding::{percent_encode, NON_ALPHANUMERIC};
use spider::utils::log;
use spider::website::Website;

use env_logger::Env;
use spider::tokio;
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;

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
    let mut rx2 = website.subscribe(888).unwrap();

    tokio::spawn(async move {
        while let Ok(page) = rx2.recv().await {
            let download_file =
                percent_encode(page.get_url().as_bytes(), NON_ALPHANUMERIC).to_string();

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
                .await
                .expect("Unable to open file");

            if let Some(b) = page.get_bytes() {
                file.write_all(b).await.unwrap_or_default();
            }

            log("downloaded", download_file)
        }
    });

    website.crawl().await;
}
