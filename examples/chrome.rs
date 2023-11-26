//! `cargo run --example chrome --features chrome_headed`
extern crate spider;

use spider::tokio;
use spider::website::Website;

#[tokio::main]
async fn main() {
    let mut website: Website = Website::new("https://rsseau.fr");
    let mut rx2 = website.subscribe(16).unwrap();

    tokio::spawn(async move {
        while let Ok(page) = rx2.recv().await {
            println!("{:?}", page.get_url());
        }
    });

    website.crawl().await;

    println!("Links found {:?}", website.get_links().len());
}
