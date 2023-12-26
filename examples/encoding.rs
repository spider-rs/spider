//! `cargo run --example encoding --features encoding`
extern crate spider;

use spider::hashbrown::HashMap;
use spider::tokio;
use spider::website::Website;

#[tokio::main]
async fn main() {
    let mut website: Website =
        Website::new("https://hoken.kakaku.com/health_check/blood_pressure/")
            .with_budget(Some(HashMap::from([("*", 2)])))
            .build()
            .unwrap();
    let mut rx2 = website.subscribe(16).unwrap();

    tokio::spawn(async move {
        while let Ok(res) = rx2.recv().await {
            println!("{:?}", res.get_url());
            println!("{:?}", res.get_html_encoded("SHIFT_JIS"));
        }
    });

    website.crawl().await;

    println!("Links found {:?}", website.get_links().len());
}
