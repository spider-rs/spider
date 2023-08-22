//! `cargo run --example callback`
extern crate spider;

use spider::tokio;
use spider::website::Website;

#[tokio::main]
async fn main() {
    let mut website: Website = Website::new("https://rsseau.fr");
    website.on_link_find_callback = Some(|s, ss| {
        println!("link target: {:?}", s);
        (s, ss)
    });
    website.crawl().await;
}
