//! Make sure to create a storage directory locally.
//! `cargo run --example chrome_screenshot --features="spider/sync spider/chrome spider/chrome_store_page"`
extern crate spider;

#[cfg(feature = "chrome")]
#[tokio::main]
async fn main() {
    use std::path::PathBuf;
    use spider::tokio;
    use spider::website::Website;
    let mut website: Website = Website::new("https://choosealicense.com");
    let mut rx2 = website.subscribe(18).unwrap();

    tokio::spawn(async move {
        while let Ok(page) = rx2.recv().await {
            page.screenshot(
                true,
                true,
                spider::chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat::Png,
                Some(75),
                None::<PathBuf>,
            )
            .await;
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


#[cfg(not(feature = "chrome"))]
fn main() {
    println!("Use the chrome flag for this!");
}
