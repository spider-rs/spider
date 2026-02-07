//! End-to-end remote cache demo (index_cache_server / hybrid_cache_server).
//!
//! 1) Start your remote cache server (default expected at http://127.0.0.1:8080)
//! 2) Set `HYBRID_CACHE_ENDPOINT` if different.
//! 3) Run:
//!    cargo run --example cache_remote_skip_browser --features="spider/sync spider/chrome spider/chrome_remote_cache" -- https://example.com/

extern crate spider;

use spider::features::chrome_common::RequestInterceptConfiguration;
use spider::tokio;
use spider::website::Website;

async fn crawl_once(url: &str, skip_browser: bool) -> (std::time::Duration, Option<f64>, usize) {
    let mut website: Website = Website::new(url)
        .with_limit(1)
        .with_caching(true)
        .with_cache_skip_browser(skip_browser)
        .with_chrome_intercept(RequestInterceptConfiguration::new(true))
        .build()
        .expect("build website");

    let mut rx = website.subscribe(4).expect("subscribe");
    let page_task = tokio::spawn(async move { rx.recv().await.ok() });

    let started = tokio::time::Instant::now();
    website.crawl_raw().await;
    let elapsed = started.elapsed();
    website.unsubscribe();

    let page = page_task.await.ok().and_then(|p| p);

    let bytes_transferred = page.as_ref().and_then(|p| p.bytes_transferred);
    let html_len = page.map_or(0, |p| p.get_html_bytes_u8().len());

    (elapsed, bytes_transferred, html_len)
}

#[tokio::main]
async fn main() {
    env_logger::init();

    let target = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "https://example.com/".to_string());
    let endpoint = std::env::var("HYBRID_CACHE_ENDPOINT")
        .unwrap_or_else(|_| "http://127.0.0.1:8080".to_string());

    println!("target={target}");
    println!("hybrid_cache_endpoint={endpoint}");

    // Pass 1: warm cache (browser can run as usual)
    let (warm_duration, warm_bytes, warm_html_len) = crawl_once(&target, false).await;
    println!(
        "warm_pass: duration={warm_duration:?}, bytes_transferred={warm_bytes:?}, html_len={warm_html_len}"
    );

    // Pass 2: enable skip-browser mode, return cached value immediately if available
    let (cached_duration, cached_bytes, cached_html_len) = crawl_once(&target, true).await;
    println!(
        "cached_pass(skip_browser): duration={cached_duration:?}, bytes_transferred={cached_bytes:?}, html_len={cached_html_len}"
    );

    println!(
        "cache_hit_hint: bytes_transferred is commonly None on direct cached returns, and duration usually drops significantly."
    );
}
