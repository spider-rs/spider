//! `cargo run --example example`
extern crate spider;

use spider::tokio;
use spider::website::Website;
use std::time::Instant;

#[tokio::main]
async fn main() {
    let mut website: Website = Website::new("https://www.va.gov"); // todo: use stdin
    website.configuration.user_agent = Some(Box::new("usasearch".into())); // Defaults to spider/x.y.z, where x.y.z is the library version
    website.configuration.respect_robots_txt = true;
    let start = Instant::now();
    website.crawl().await;
    let duration = start.elapsed();
    let links = website.get_links();

    println!(
        "Time elapsed in website.crawl() is: {:?} for total pages: {:?}",
        duration,
        links.len()
    )
}
