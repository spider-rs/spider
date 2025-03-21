//! cargo run --example chrome_remote_tls --features="chrome chrome_intercept chrome_tls_connection"

extern crate spider;
use crate::spider::tokio::io::AsyncWriteExt;
use spider::features::chrome_common::RequestInterceptConfiguration;
use spider::tokio;
use spider::website::Website;
use std::io::Result;

async fn crawl_website(url: &str) -> Result<()> {
    let mut website: Website = Website::new(url)
        .with_limit(500)
        .with_chrome_intercept(RequestInterceptConfiguration::new(true))
        .with_stealth(true)
        .with_fingerprint(true)
        // .with_proxies(Some(vec!["http://localhost:8888".into()]))
        .with_chrome_connection(Some(
            "wss://replace_this_with_your_connection_string".into(),
        ))
        .build()
        .unwrap();

    let mut rx2 = website.subscribe(16).unwrap();
    let mut stdout = tokio::io::stdout();

    tokio::spawn(async move {
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

    let start = crate::tokio::time::Instant::now();
    website.crawl().await;

    let duration = start.elapsed();

    let links = website.get_all_links_visited().await;

    println!(
        "Time elapsed in website.crawl({}) is: {:?} for total pages: {:?}",
        website.get_url(),
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
