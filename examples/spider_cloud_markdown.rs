//! Spider Cloud crawl → clean LLM-ready markdown.
//!
//! Uses `return_format: "markdown"` so Spider Cloud returns markdown directly.
//!
//! ```bash
//! SPIDER_CLOUD_API_KEY=sk-... cargo run --example spider_cloud_markdown --features="spider/sync spider/spider_cloud"
//! ```

extern crate spider;

use spider::configuration::{SpiderCloudConfig, SpiderCloudMode};
use spider::tokio;
use spider::website::Website;

#[tokio::main]
async fn main() {
    let api_key = std::env::var("SPIDER_CLOUD_API_KEY").expect("set SPIDER_CLOUD_API_KEY");

    let config = SpiderCloudConfig::new(&api_key)
        .with_mode(SpiderCloudMode::Smart)
        .with_return_format("markdown");

    let mut website = Website::new("https://choosealicense.com")
        .with_spider_cloud_config(config)
        .build()
        .unwrap();

    let mut rx = website.subscribe(16);

    tokio::spawn(async move {
        while let Ok(page) = rx.recv().await {
            println!("--- {} ---\n{}\n", page.get_url(), page.get_content());
        }
    });

    website.crawl().await;
    website.unsubscribe();
}
