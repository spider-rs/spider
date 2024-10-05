//! Make sure to create a storage directory locally.
//! cargo run --example chrome_screenshot_with_config --features="spider/sync spider/chrome"
extern crate spider;
use spider::tokio;
use spider::website::Website;

#[tokio::main]
async fn main() {
    // the cdp params
    let screenshot_params =
        spider::configuration::ScreenshotParams::new(Default::default(), Some(true), Some(true));
    // params that handle the way to take screenshots
    let screenshot_config =
        spider::configuration::ScreenShotConfig::new(screenshot_params, true, true, None);

    let mut website: Website = Website::new("https://choosealicense.com")
        .with_screenshot(Some(screenshot_config))
        .build()
        .unwrap();
    let mut rx2 = website.subscribe(18).unwrap();

    tokio::spawn(async move {
        while let Ok(page) = rx2.recv().await {
            if page.screenshot_bytes.is_none() {
                println!("🚫 - {:?}", page.get_url());
            } else {
                println!("📸 - {:?}", page.get_url());
            }
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
