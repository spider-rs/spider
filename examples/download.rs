//! `cargo run --example download`
extern crate env_logger;
extern crate spider;

use spider::utils::log;
use spider::website::Website;

use env_logger::Env;
use std::fs::OpenOptions;
use std::io::Write;
use spider::tokio;

#[tokio::main]
async fn main() {
    // view the target dist for the downloads
    std::fs::create_dir_all("./target/downloads").unwrap_or_default();

    let env = Env::default()
        .filter_or("RUST_LOG", "info")
        .write_style_or("RUST_LOG_STYLE", "always");

    env_logger::init_from_env(env);

    let website_name = "https://rsseau.fr";

    let mut website: Website = Website::new(website_name);
    website.configuration.respect_robots_txt = true;
    website.configuration.delay = 0;

    website.scrape().await;

    for page in website.get_pages() {
        let download_file = page.get_url().clone();
        let download_file = download_file.replace(website_name, "");
        let download_file = download_file.replace(".", "-");
        let download_file = download_file.replace("/", "-");

        let download_file = if download_file.starts_with("-") {
            &download_file[1..]
        } else {
            &download_file
        };

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

        let html = page.get_html();
        let html = html.as_bytes();

        file.write_all(html).unwrap();

        log("downloaded", download_file)
    }
}
