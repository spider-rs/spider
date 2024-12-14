//! cargo run --example real_world --features="chrome chrome_intercept real_browser spider_utils/transformations"

extern crate spider;
use crate::spider::tokio::io::AsyncWriteExt;
use spider::configuration::{WaitForDelay, WaitForSelector};
use spider::tokio;
use spider::website::Website;
use spider::{
    configuration::WaitForIdleNetwork, features::chrome_common::RequestInterceptConfiguration,
};

use std::io::Result;
use std::time::Duration;

async fn crawl_website(url: &str) -> Result<()> {
    let mut stdout = tokio::io::stdout();
    let mut interception = RequestInterceptConfiguration::new(true);

    interception.block_javascript = true;

    let mut website: Website = Website::new(url)
        .with_limit(5)
        .with_chrome_intercept(interception)
        .with_wait_for_idle_network(Some(WaitForIdleNetwork::new(Some(Duration::from_millis(
            500,
        )))))
        .with_wait_for_idle_dom(Some(WaitForSelector::new(
            Some(Duration::from_millis(100)),
            "body".into(),
        )))
        .with_block_assets(true)
        // .with_wait_for_delay(Some(WaitForDelay::new(Some(Duration::from_millis(10000)))))
        .with_stealth(true)
        .with_return_page_links(true)
        .with_fingerprint(true)
        // .with_proxies(Some(vec!["http://localhost:8888".into()]))
        .with_chrome_connection(Some("http://127.0.0.1:9222/json/version".into()))
        .build()
        .unwrap();

    let mut rx2 = website.subscribe(16).unwrap();

    let start = crate::tokio::time::Instant::now();

    let (links, _) = tokio::join!(
        async move {
            website.crawl().await;
            website.unsubscribe();
            website.get_links()
        },
        async move {
            while let Ok(page) = rx2.recv().await {
                let _ = stdout
                    .write_all(
                        format!(
                            "- {} -- Bytes transferred {:?} -- HTML Size {:?} -- Links: {:?}\n",
                            page.get_url(),
                            page.bytes_transferred.unwrap_or_default(),
                            page.get_html_bytes_u8().len(),
                            match page.page_links {
                                Some(ref l) => l.len(),
                                _ => 0,
                            }
                        )
                        .as_bytes(),
                    )
                    .await;
            }
        }
    );

    let duration = start.elapsed();

    println!(
        "Time elapsed in website.crawl({}) is: {:?} for total pages: {:?}",
        url,
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
