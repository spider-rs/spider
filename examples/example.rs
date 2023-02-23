//! `cargo run --example example`
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
    website.configuration.respect_robots_txt = true;
    website.configuration.subdomains = false;
    website.configuration.delay = 0; // Defaults to 0 ms
    website.configuration.user_agent = "SpiderBot".into(); // Defaults to spider/x.y.z, where x.y.z is the library version

    let start = Instant::now();
    website.crawl().await;
    let duration = start.elapsed();

    let pages = website.get_pages();

    for page in &pages {
        println!("- {:?}", page.get_url());
    }

    println!(
        "Time elapsed in website.crawl() is: {:?} for total pages: {:?}",
        duration,
        pages.len()
    )
}
