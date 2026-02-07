//! cargo run --example webdriver_screenshot --features="webdriver webdriver_screenshot"
//!
//! This example demonstrates taking screenshots using WebDriver.
//! Screenshots are saved as PNG files in the current directory.
//!
//! You need to have a WebDriver server running (e.g., chromedriver).
//! To start chromedriver: `chromedriver --port=4444`

extern crate spider;

use spider::features::webdriver::{
    attempt_navigation, get_page_title, setup_driver_events, take_screenshot,
};
use spider::features::webdriver_common::{WebDriverBrowser, WebDriverConfig};
use spider::tokio;
use spider::website::Website;
use std::io::Result;
use std::time::Duration;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;

async fn screenshot_website(url: &str, output_filename: &str) -> Result<()> {
    let webdriver_config = WebDriverConfig::new()
        .with_server_url("http://localhost:4444")
        .with_browser(WebDriverBrowser::Chrome)
        .with_headless(true)
        .with_viewport(1920, 1080)
        .with_timeout(Duration::from_secs(30));

    let website: Website = Website::new(url)
        .with_webdriver(webdriver_config.clone())
        .build()
        .unwrap();

    // Setup webdriver
    if let Some(controller) = website.setup_webdriver().await {
        let driver = controller.driver();

        // Setup stealth mode
        setup_driver_events(driver, &website.configuration).await;

        // Navigate to the URL
        let timeout = Some(Duration::from_secs(30));
        if let Err(e) = attempt_navigation(url, driver, &timeout).await {
            eprintln!("Failed to navigate to {}: {:?}", url, e);
            return Ok(());
        }

        // Wait a bit for page to fully render
        tokio::time::sleep(Duration::from_secs(2)).await;

        // Get page title
        match get_page_title(driver).await {
            Ok(title) => println!("Page title: {}", title),
            Err(e) => eprintln!("Failed to get title: {:?}", e),
        }

        // Take screenshot
        match take_screenshot(driver).await {
            Ok(png_data) => {
                let mut file = File::create(output_filename).await?;
                file.write_all(&png_data).await?;
                println!("Screenshot saved to: {}", output_filename);
            }
            Err(e) => eprintln!("Failed to take screenshot: {:?}", e),
        }
    } else {
        eprintln!("Failed to initialize WebDriver");
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();

    screenshot_website("https://example.com", "example_screenshot.png").await?;
    screenshot_website(
        "https://choosealicense.com",
        "choosealicense_screenshot.png",
    )
    .await?;

    Ok(())
}
