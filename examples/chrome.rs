//! `cargo run --example chrome --features chrome`
extern crate spider;

use spider::tokio;
use spider::website::Website;

#[tokio::main]
async fn main() {
    let mut website: Website = Website::new("https://rsseau.fr")
        .with_chrome_intercept(cfg!(feature = "chrome_intercept"), true)
        .build()
        .unwrap();
    let mut rx2 = website.subscribe(16).unwrap();

    tokio::spawn(async move {
        while let Ok(page) = rx2.recv().await {
            println!("{:?}", page.get_url());
        }
    });

    website.crawl().await;

    println!("Links found {:?}", website.get_links().len());
}
