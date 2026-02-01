//! cargo run --example webdriver --features="webdriver webdriver_stealth"
//!
//! This example demonstrates basic WebDriver usage with spider.
//! You need to have a WebDriver server running (e.g., chromedriver, geckodriver, or Selenium).
//!
//! To start chromedriver: `chromedriver --port=4444`
//! To start geckodriver: `geckodriver --port=4444`

extern crate spider;

use spider::features::webdriver_common::{WebDriverBrowser, WebDriverConfig};
use spider::tokio;
use spider::website::Website;
use std::io::Result;

async fn crawl_website(url: &str) -> Result<()> {
    let webdriver_config = WebDriverConfig::new()
        .with_server_url("http://localhost:4444")
        .with_browser(WebDriverBrowser::Chrome)
        .with_headless(true);

    let mut website: Website = Website::new(url)
        .with_limit(10)
        .with_webdriver(webdriver_config)
        .build()
        .unwrap();

    let mut rx2 = website.subscribe(16).unwrap();

    let handle = tokio::spawn(async move {
        while let Ok(page) = rx2.recv().await {
            println!("{:?}", page.get_url());
        }
    });

    let start = crate::tokio::time::Instant::now();
    website.crawl().await;
    website.unsubscribe();
    let _ = handle.await;

    let duration = start.elapsed();

    let links = website.get_all_links_visited().await;

    println!(
        "Time elapsed in website.crawl({url}) is: {:?} for total pages: {:?}",
        duration,
        links.len()
    );

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();

    let _ = tokio::join!(
        crawl_website("https://choosealicense.com"),
        crawl_website("https://jeffmendez.com"),
        crawl_website("https://example.com"),
    );

    Ok(())
}
