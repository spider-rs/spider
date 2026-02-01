//! cargo run --example webdriver_remote --features="webdriver webdriver_stealth"
//!
//! This example demonstrates connecting to a remote WebDriver server like Selenium Grid.
//!
//! To start Selenium Grid with Docker:
//! ```
//! docker run -d -p 4444:4444 --shm-size="2g" selenium/standalone-chrome:latest
//! ```
//!
//! For Firefox:
//! ```
//! docker run -d -p 4444:4444 --shm-size="2g" selenium/standalone-firefox:latest
//! ```

extern crate spider;

use crate::spider::tokio::io::AsyncWriteExt;
use spider::features::webdriver_common::{WebDriverBrowser, WebDriverConfig};
use spider::tokio;
use spider::website::Website;
use std::io::Result;
use std::time::Duration;

async fn crawl_website(url: &str) -> Result<()> {
    // Configure WebDriver to connect to remote Selenium Grid
    let webdriver_config = WebDriverConfig::new()
        .with_server_url("http://localhost:4444")
        .with_browser(WebDriverBrowser::Chrome)
        .with_headless(true)
        .with_timeout(Duration::from_secs(30))
        .with_viewport(1920, 1080);

    let mut website: Website = Website::new(url)
        .with_limit(50)
        .with_webdriver(webdriver_config)
        .build()
        .unwrap();

    let mut rx2 = website.subscribe(16).unwrap();
    let mut stdout = tokio::io::stdout();

    tokio::spawn(async move {
        while let Ok(page) = rx2.recv().await {
            let _ = stdout
                .write_all(
                    format!(
                        "- {} -- Status: {} -- HTML Size {:?}\n",
                        page.get_url(),
                        page.status_code,
                        page.get_html_bytes_u8().len()
                    )
                    .as_bytes(),
                )
                .await;
        }
    });

    let start = crate::tokio::time::Instant::now();
    website.crawl().await;

    let duration = start.elapsed();

    let links = website.get_all_links_visited().await;

    println!(
        "\nTime elapsed in website.crawl({}) is: {:?} for total pages: {:?}",
        website.get_url(),
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
