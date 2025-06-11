//! cargo run --example real_world --features="chrome chrome_intercept real_browser"

extern crate spider;
use crate::spider::tokio::io::AsyncWriteExt;
use spider::client::header::{HeaderMap, HeaderValue};
use spider::client::header::{REFERER, REFERRER_POLICY};
use spider::configuration::WaitForSelector;
use spider::tokio;
use spider::website::Website;
use spider::{
    configuration::{ChromeEventTracker, WaitForIdleNetwork},
    features::chrome_common::RequestInterceptConfiguration,
};
use std::io::Result;
use std::iter::FromIterator;
use std::time::Duration;
async fn crawl_website(url: &str) -> Result<()> {
    let mut stdout = tokio::io::stdout();
    let mut interception = RequestInterceptConfiguration::new(true);
    let mut tracker = ChromeEventTracker::default();

    interception.block_javascript = true;

    tracker.responses = true;
    tracker.requests = true;

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
        .with_headers(Some(HeaderMap::from_iter([
            (
                REFERER,
                HeaderValue::from_static(spider::spider_fingerprint::spoof_referrer()),
            ),
            (REFERRER_POLICY, HeaderValue::from_static("strict-origin")),
        ])))
        .with_block_assets(true)
        // .with_wait_for_delay(Some(WaitForDelay::new(Some(Duration::from_millis(10000)))))
        .with_stealth(true)
        .with_return_page_links(true)
        .with_fingerprint(true)
        .with_event_tracker(Some(tracker))
        // .with_proxies(Some(vec!["http://localhost:8888".into()]))
        .with_chrome_connection(Some("http://127.0.0.1:9222/json/version".into()))
        .build()
        .unwrap();
    let mut rx2 = website.subscribe(16).unwrap();
    let mut g = website.subscribe_guard().unwrap();

    let start = crate::tokio::time::Instant::now();

    g.guard(true);

    let (links, _) = tokio::join!(
        async move {
            website.crawl().await;
            website.unsubscribe();
            website.get_all_links_visited().await
        },
        async move {
            while let Ok(page) = rx2.recv().await {
                let _ = stdout
                    .write_all(
                        format!(
                            "---- {}\nBytes transferred {:?}\nHTML Size {:?}\nLinks: {:?}\nRequests Sent {:?}\nResponses {:?}\nMetadata {:?}\n",
                            page.get_url(),
                            page.bytes_transferred.unwrap_or_default(),
                            page.get_html_bytes_u8().len(),
                            match page.page_links {
                                Some(ref l) => l.len(),
                                _ => 0,
                            },
                            page.get_request().as_ref().map(|f| f.len()),
                            page.get_responses(),
                            page.get_metadata()
                        )
                        .as_bytes(),
                    )
                    .await;
                g.inc();
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
