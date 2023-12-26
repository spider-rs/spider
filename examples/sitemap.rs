//! `cargo run --example sitemap`
extern crate spider;

use spider::tokio;
use spider::website::Website;
use std::time::Instant;

#[tokio::main]
async fn main() {
    let mut website: Website = Website::new("https://rsseau.fr");
    website
        .configuration
        .blacklist_url
        .insert(Default::default())
        .push("https://rsseau.fr/resume".into());
    website
        .configuration
        .with_respect_robots_txt(true)
        .with_user_agent(Some("SpiderBot"))
        .with_ignore_sitemap(true) // the default website.crawl will not run the sitemap
        .with_sitemap(Some("/sitemap/sitemap-0.xml"));
    let start = Instant::now();
    // crawl all the links on the sitemap and links after.
    website.crawl_sitemap().await.persist_links().crawl().await;
    let duration = start.elapsed();

    let links = website.get_links();

    for link in links {
        println!("- {:?}", link.as_ref());
    }

    println!(
        "Time elapsed in website.crawl() is: {:?} for total pages: {:?}",
        duration,
        links.len()
    )
}
