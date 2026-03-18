//! Spider Browser Cloud — remote CDP browser via `wss://browser.spider.cloud`.
//!
//! Connects to a managed Rust-based browser instance with stealth, proxies,
//! and AI extraction built in.  Uses the same `chrome_connection_url` path
//! as any local/remote Chrome instance.
//!
//! ```bash
//! SPIDER_CLOUD_API_KEY=your-key cargo run --example spider_browser_cloud --features="chrome spider_cloud"
//! ```
//!
//! Optional env vars:
//! - `SPIDER_BROWSER_STEALTH=1` — enable stealth/anti-fingerprinting
//! - `SPIDER_BROWSER_COUNTRY=us` — geo-target the browser session

extern crate spider;

use spider::configuration::SpiderBrowserConfig;
use spider::tokio;
use spider::website::Website;
use std::io::Result;

async fn crawl_with_spider_browser(url: &str) -> Result<()> {
    let api_key = std::env::var("SPIDER_CLOUD_API_KEY").expect("SPIDER_CLOUD_API_KEY must be set");

    let stealth = matches!(
        std::env::var("SPIDER_BROWSER_STEALTH")
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str(),
        "1" | "true" | "yes"
    );

    let country = std::env::var("SPIDER_BROWSER_COUNTRY").ok();

    let mut browser_cfg = SpiderBrowserConfig::new(&api_key).with_stealth(stealth);
    if let Some(ref c) = country {
        browser_cfg = browser_cfg.with_country(c);
    }

    println!("Connecting to: {}", browser_cfg.connection_url());

    let mut website: Website = Website::new(url)
        .with_limit(10)
        .with_spider_browser_config(browser_cfg)
        .build()
        .unwrap();

    let mut rx = website.subscribe(16).unwrap();

    tokio::spawn(async move {
        while let Ok(page) = rx.recv().await {
            println!(
                "  {} — {} bytes",
                page.get_url(),
                page.get_html_bytes_u8().len()
            );
        }
    });

    let start = tokio::time::Instant::now();
    website.crawl().await;
    let duration = start.elapsed();

    let links = website.get_all_links_visited().await;
    println!("Crawled {} in {:?} — {} pages", url, duration, links.len());

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    crawl_with_spider_browser("https://example.com").await
}
