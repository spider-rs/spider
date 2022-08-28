//! `cargo run --example debug`
//! example to demonstate enabling logging within the crate
extern crate env_logger;
extern crate spider;

use env_logger::Env;
use spider::website::Website;
use spider::tokio;

#[tokio::main]
async fn main() {
    // enable with RUST_LOG env_logger crate
    let env = Env::default()
        .filter_or("RUST_LOG", "info")
        .write_style_or("RUST_LOG_STYLE", "always");

    env_logger::init_from_env(env);

    let mut website: Website = Website::new("https://rsseau.fr");

    website.crawl().await;
}
