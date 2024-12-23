//! `cargo run --example sitemap --features sitemap`
extern crate spider;

use spider::tokio;
use spider::website::Website;
use std::time::Instant;

#[tokio::main]
async fn main() {
    let mut website: Website = Website::new("https://rsseau.fr/en");

    website
        .configuration
        .with_respect_robots_txt(true)
        .with_user_agent(Some("SpiderBot"))
        .with_ignore_sitemap(true) // ignore running the sitemap on base crawl/scape methods. Remove or set to true to include the sitemap with the crawl.
        .with_sitemap(Some("/sitemap/sitemap-0.xml"));

    let start = Instant::now();

    // crawl the sitemap first
    website.crawl_sitemap().await;
    // persist links to the next crawl
    website.persist_links();
    // crawl normal with links found in the sitemap extended.
    website.crawl().await;

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
