//! cargo run --example chrome_single_page --features chrome
//!
//! Regression test: verifies a single-page Chrome crawl completes without
//! hanging on pages with long-lived CDP streams (e.g. clickz.com).

extern crate spider;
use spider::tokio;
use spider::website::Website;

#[tokio::main]
async fn main() {
    env_logger::init();

    let url = "https://www.clickz.com/how-ai-powered-personalization-loyalty-and-advocacy-will-define-customer-engagement-in-2026-a-conversation-with-channing-ferrer/270694/";

    let mut website: Website = Website::new(url)
        .with_limit(1)
        .with_stealth(true)
        .build()
        .unwrap();

    let mut rx = website.subscribe(16);

    let handle = tokio::spawn(async move {
        let mut html = String::new();
        while let Ok(page) = rx.recv().await {
            println!("Received page: {}", page.get_url());
            html = page.get_html().to_string();
        }
        html
    });

    let start = tokio::time::Instant::now();

    println!("Crawling {url} via Chrome...");
    website.crawl().await;
    website.unsubscribe();

    let html = handle.await.unwrap();
    let duration = start.elapsed();

    let len = html.len();
    println!("Got {len} bytes of HTML in {duration:?}");
    if !html.is_empty() {
        println!("Snippet: {}", &html[..html.len().min(200)]);
    }

    assert!(
        duration < std::time::Duration::from_secs(60),
        "Crawl should complete within 60s, took {duration:?}"
    );

    println!("PASS: Chrome single-page crawl completed in {duration:?}");
}
