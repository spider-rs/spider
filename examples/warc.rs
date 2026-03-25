//! WARC output example — writes crawled pages to a WARC 1.1 archive file.
//!
//! HTTP mode:
//!   cargo run --example warc --features "spider/sync,spider/warc"
//!
//! Chrome mode (works identically — WARC hooks into the broadcast channel):
//!   cargo run --example warc_chrome --features "spider/sync,spider/warc,spider/chrome"

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
        .unwrap_or_else(|| "output.warc".to_string());

    let mut website: Website = Website::new(&url);

    // Configure WARC output — the crawl will automatically write all fetched
    // pages as WARC response records to this file.
    website.configuration.with_warc(WarcConfig {
        path: output_path.clone(),
        write_warcinfo: true,
        ..Default::default()
    });

    website.configuration.respect_robots_txt = true;

    let start = Instant::now();
    website.crawl().await;
    let duration = start.elapsed();

    let links = website.get_all_links_visited().await;
    let warc_records = website.warc_record_count();

    println!("Crawled {} pages in {:?}", links.len(), duration);
    println!("WARC records written: {warc_records}");
    println!("Output: {output_path}");
}
