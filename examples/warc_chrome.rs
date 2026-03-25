//! WARC output with Chrome rendering — archives JavaScript-rendered pages.
//!
//!   cargo run --example warc_chrome --features "spider/sync,spider/warc,spider/chrome"
//!
//! The WARC writer hooks into the same broadcast channel as HTTP crawls, so
//! Chrome-rendered pages are archived identically. The response records contain
//! the fully rendered HTML (post-JS execution).

extern crate spider;

use spider::tokio;
use spider::utils::warc::WarcConfig;
use spider::website::Website;
use std::time::Instant;

#[tokio::main]
async fn main() {
    let url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "https://choosealicense.com".to_string());

    let output_path = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "output_chrome.warc".to_string());

    let mut website: Website = Website::new(&url);

    website.configuration.with_warc(WarcConfig {
        path: output_path.clone(),
        write_warcinfo: true,
        ..Default::default()
    });

    website.configuration.respect_robots_txt = true;

    // Limit pages for the example.
    website
        .configuration
        .with_limit(10);

    let start = Instant::now();
    website.crawl().await;
    let duration = start.elapsed();

    let links = website.get_all_links_visited().await;
    let warc_records = website.warc_record_count();

    println!("Crawled {} pages (Chrome) in {:?}", links.len(), duration);
    println!("WARC records written: {warc_records}");
    println!("Output: {output_path}");
}
