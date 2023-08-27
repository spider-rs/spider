//! `cargo run --example subscribe`
extern crate spider;

use spider::tokio;
use spider::website::Website;

#[tokio::main]
async fn main() {
    let mut website: Website = Website::new("https://rsseau.fr");
    let sub = website.subscribe().unwrap();
    let mut rx2 = sub.0.subscribe();

    let join_handle = tokio::spawn(async move {
        while let Ok(res) = rx2.recv().await {
            println!("{:?}", res.get_url());
        }
    });

    website.crawl().await;

    let _ = join_handle.await.unwrap();
}
