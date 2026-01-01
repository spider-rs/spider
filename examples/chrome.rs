//! cargo run --example chrome --features chrome

extern crate spider;
use spider::features::chrome_common::RequestInterceptConfiguration;
use spider::tokio;
use spider::website::Website;
use std::io::Result;

async fn crawl_website(url: &str) -> Result<()> {
    let mut website: Website = Website::new(url)
        .with_limit(10)
        .with_chrome_intercept(RequestInterceptConfiguration::new(true))
        .with_stealth(true)
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
    let _ = tokio::join!(
        crawl_website("https://choosealicense.com"),
        crawl_website("https://jeffmendez.com"),
        crawl_website("https://github.com/spider-rs"),
    );

    Ok(())
}
