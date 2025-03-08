//! cargo run --example chrome_sendable --features="chrome chrome_intercept"

extern crate spider;
use crate::spider::tokio::io::AsyncWriteExt;
use spider::features::chrome_common::RequestInterceptConfiguration;
use spider::tokio;
use spider::website::Website;
use std::io::Result;

async fn crawl_website(website: &Website, url: &str) -> Result<()> {
    let start = crate::tokio::time::Instant::now();

    website.crawl_chrome_send(Some(url)).await;

    println!(
        "Time elapsed in website.crawl({}) is: {:?}",
        url,
        start.elapsed(),
    );

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut stdout = tokio::io::stdout();
    let mut website = Website::default();

    website
        .with_limit(5)
        .with_chrome_intercept(RequestInterceptConfiguration::new(true))
        .with_stealth(true)
        .with_fingerprint(true)
        .with_chrome_connection(Some("http://127.0.0.1:9222/json/version".into()));

    website.configure_setup().await;

    let mut rx2 = website.subscribe(16).unwrap();

    let handle = tokio::spawn(async move {
        while let Ok(page) = rx2.recv().await {
            let _ = stdout
                .write_all(
                    format!(
                        "- {} -- Bytes transferred {:?} -- HTML Size {:?}\n",
                        page.get_url(),
                        page.bytes_transferred.unwrap_or_default(),
                        page.get_html_bytes_u8().len()
                    )
                    .as_bytes(),
                )
                .await;
        }
    });

    let _ = tokio::join!(
        crawl_website(&website, "https://choosealicense.com"),
        crawl_website(&website, "https://jeffmendez.com"),
        crawl_website(&website, "https://example.com"),
    );

    drop(website);

    let _ = handle.await;

    Ok(())
}
