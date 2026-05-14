//! cargo run --example chrome_nxdomain_test --features chrome
//!
//! Verifies the chrome-path DNS hedge (v2.51.187 timer arm + v2.51.188
//! CDP loadingFailed arm) on a confirmed-NXDOMAIN host. Prints elapsed
//! time, status_code, and should_retry for two back-to-back calls so
//! the v2.51.186 cache effect is visible on the second.
//!
//! Expected outcome with the hedge active against real chrome:
//!   * First call:  status_code = 525 (DNS_RESOLVE_ERROR),
//!                  elapsed ≈ chrome launch + (1-3s hedge tick).
//!   * Second call: status_code = 525, elapsed ≈ chrome launch only
//!                  (cache hits at `Page::new_base` so the navigation
//!                  is short-circuited at sub-µs cost).
//!
//! ## Configuration
//!
//! * `TARGET_URL` — host to test. Default: `https://kingfishelectric.com/`.
//! * `CHROME_WS_URL` — remote chrome endpoint (HTTP `/json/version` URL
//!   or `ws://` URL). When set, `Website::with_chrome_connection` is
//!   called so the test runs against a real chrome instance. Without
//!   it the test falls back to local chrome (or spider's HTTP fallback
//!   if no local chrome binary is available).
//! * `REQUEST_TIMEOUT_SECS` — per-page timeout. Default: 30s.

use spider::features::chrome_common::RequestInterceptConfiguration;
use spider::tokio;
use spider::website::Website;
use std::time::Duration;

#[tokio::main]
async fn main() {
    let url = std::env::var("TARGET_URL")
        .unwrap_or_else(|_| "https://kingfishelectric.com/".to_string());
    let chrome_ws = std::env::var("CHROME_WS_URL").ok();
    let request_timeout_secs = std::env::var("REQUEST_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(30);

    println!("[chrome_nxdomain_test] target={url}");
    match &chrome_ws {
        Some(ws) => println!("[chrome_nxdomain_test] remote chrome connection: {ws}"),
        None => println!(
            "[chrome_nxdomain_test] no CHROME_WS_URL set — using local chrome auto-launch (if available)"
        ),
    }
    println!("[chrome_nxdomain_test] request_timeout={request_timeout_secs}s");
    println!("[chrome_nxdomain_test] first call should hit hedge arm; second should hit v2.51.186 cache");

    // First request — through the full chrome path. Hedge fires
    // either via the CDP loadingFailed event (v2.51.188) or the
    // timer arm (v2.51.187), populating the cache for the next
    // request.
    let first = run_once(&url, chrome_ws.as_deref(), request_timeout_secs).await;
    print_pages("first", first.0, &first.1);

    // Second request — cached NXDOMAIN, should short-circuit at
    // chrome_nxdomain_shortcircuit (~50ns DashMap shard read).
    let second = run_once(&url, chrome_ws.as_deref(), request_timeout_secs).await;
    print_pages("second", second.0, &second.1);
}

async fn run_once(
    url: &str,
    chrome_ws: Option<&str>,
    request_timeout_secs: u64,
) -> (Duration, Website) {
    let mut website = Website::new(url)
        .with_limit(1)
        .with_respect_robots_txt(false)
        .with_chrome_intercept(RequestInterceptConfiguration::new(true))
        .build()
        .unwrap();
    if let Some(ws) = chrome_ws {
        website.with_chrome_connection(Some(ws.to_string()));
    }
    website.configuration.request_timeout = Some(Duration::from_secs(request_timeout_secs));

    let start = std::time::Instant::now();
    website.scrape().await;
    let elapsed = start.elapsed();
    (elapsed, website)
}

fn print_pages(label: &str, elapsed: Duration, w: &Website) {
    let pages = w.get_pages();
    match pages {
        Some(ps) if !ps.is_empty() => {
            for p in ps.iter() {
                println!(
                    "[{}][{:.3}s] {} -> status={} html_len={} should_retry={}",
                    label,
                    elapsed.as_secs_f32(),
                    p.get_url(),
                    p.status_code,
                    p.get_html_bytes_u8().len(),
                    p.should_retry,
                );
            }
        }
        _ => println!(
            "[{}][{:.3}s] no pages returned (NXDOMAIN classified pre-page-list)",
            label,
            elapsed.as_secs_f32()
        ),
    }
}
