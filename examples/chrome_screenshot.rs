//! Make sure to create a storage directory locally.
//! `cargo run --example chrome_screenshot --features="spider/sync spider/chrome spider/chrome_store_page"`
extern crate spider;
use spider::tokio;
use spider::website::Website;

#[tokio::main]
async fn main() {
    use std::path::PathBuf;
    let mut website: Website = Website::new("https://choosealicense.com");
    let mut rx2 = website.subscribe(18).unwrap();
    let mut rxg = website.subscribe_guard().unwrap();

    tokio::spawn(async move {
        while let Ok(page) = rx2.recv().await {
            println!("📸 - {:?}", page.get_url());
            let bytes = page
                .screenshot(
                    true,
                    true,
                    spider::configuration::CaptureScreenshotFormat::Png,
                    Some(75),
                    None::<PathBuf>,
                    None,
                )
                .await;
            if bytes.is_empty() {
                println!("🚫 - {:?}", page.get_url());
            }
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
