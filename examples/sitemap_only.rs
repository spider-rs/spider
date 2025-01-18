//! `cargo run --example sitemap_only --features sitemap`
extern crate spider;

use spider::tokio;
use spider::website::Website;
use std::time::Instant;

#[tokio::main]
async fn main() {
    let mut website: Website = Website::new("https://spider.cloud");

    website
        .configuration
        .with_respect_robots_txt(true)
        .with_user_agent(Some("SpiderBot"));

    let start = Instant::now();

    website.crawl_sitemap().await;

    let duration = start.elapsed();

    let links = website.get_all_links_visited().await;

    for link in links.iter() {
        println!("- {:?}", link.as_ref());
    }

    println!(
        "Time elapsed in website.crawl() is: {:?} for total pages: {:?}",
        duration,
        links.len()
    )
}
