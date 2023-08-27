//! `cargo run --example subscribe`
extern crate spider;

use spider::tokio;
use spider::website::Website;

#[tokio::main]
async fn main() {
    let mut website: Website = Website::new("https://rsseau.fr");
    let mut rx2 = website.subscribe(16).unwrap();

    let join_handle = tokio::spawn(async move {
        while let Ok(res) = rx2.recv().await {
            println!("{:?}", res.get_url());
        }
    });

    website.crawl().await;

    let _ = join_handle.await.unwrap();
}
