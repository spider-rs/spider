//! `cargo run --example chrome --features chrome`

extern crate spider;
use spider::features::chrome_common::RequestInterceptConfiguration;
use spider::tokio;
use spider::website::Website;
use std::io::Result;

async fn crawl_website(url: &str) -> Result<()> {
    let mut website: Website = Website::new(url)
        .with_limit(100)
        .with_chrome_intercept(RequestInterceptConfiguration::new(true))
        .with_stealth(true)
        // .with_chrome_connection(Some("http://127.0.0.1:6000/json/version".into()))
        .build()
        .unwrap();

    let mut rx2 = website.subscribe(16).unwrap();

    tokio::spawn(async move {
        while let Ok(page) = rx2.recv().await {
            println!("{:?}", page.get_url());
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
    );

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let _ = tokio::join!(
        crawl_website("https://choosealicense.com"),
        crawl_website("https://jeffmendez.com"),
        crawl_website("https://example.com"),
    );

    Ok(())
}
