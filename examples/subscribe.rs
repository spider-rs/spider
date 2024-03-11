//! `cargo run --example subscribe`
extern crate spider;

use spider::tokio;
use spider::website::Website;

#[tokio::main]
async fn main() {
    let mut website: Website = Website::new("https://rsseau.fr");
    let mut rx2 = website.subscribe(16).unwrap();

    tokio::spawn(async move {
        while let Ok(res) = rx2.recv().await {
            println!("{:?}", res.get_url());
        }
    });
    let start = std::time::Instant::now();
    website.crawl().await;
    let duration = start.elapsed();

    println!(
        "Time elapsed in website.crawl() is: {:?} for total pages: {:?}",
        duration,
        website.get_links().len()
    )
}
