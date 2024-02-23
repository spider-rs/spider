//! Make sure to create a storage directory locally.
//! cargo run --example chrome_screenshot --features="spider/sync spider/chrome spider/chrome_store_page"
extern crate spider;
use std::path::PathBuf;

use spider::tokio;
use spider::utils::create_output_path;
use spider::website::Website;

#[tokio::main]
async fn main() {
    let mut website: Website = Website::new("https://choosealicense.com");
    website.configuration.request_timeout = Some(std::time::Duration::from_secs(60).into());
    let mut rx2 = website.subscribe(18).unwrap();
    let mut rxg = website.subscribe_guard().unwrap();

    tokio::spawn(async move {
        while let Ok(mut page) = rx2.recv().await {
            let file_format = spider::configuration::CaptureScreenshotFormat::Png;
            let output_path =
                create_output_path(&PathBuf::from("./storage/"), page.get_url(), ".png").await;

            let bytes = page
                .screenshot(true, true, file_format, Some(75), Some(output_path), None)
                .await;

            println!(
                "{} - {:?}",
                if bytes.is_empty() { "🚫" } else { "📸" },
                page.get_url()
            );

            page.close_page().await;
            rxg.inc();
        }
    });

    let start = crate::tokio::time::Instant::now();
    website.crawl().await;
    let duration = start.elapsed();

    let links = website.get_links();

    println!(
        "Time elapsed in website.crawl() is: {:?} for total pages: {:?}",
        duration,
        links.len()
    )
}
