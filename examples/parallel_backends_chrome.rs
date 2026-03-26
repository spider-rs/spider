//! Chrome (primary) vs LightPanda (backend) on a JS-heavy site.
//!
//! ```bash
//! # Start LightPanda:
//! #   .tools/lightpanda serve --host 127.0.0.1 --port 9222
//!
//! cargo run --example parallel_backends_chrome --features "spider/lightpanda spider/sync spider/chrome"
//! ```
extern crate spider;

use spider::configuration::{BackendEndpoint, BackendEngine, ParallelBackendsConfig};
use spider::tokio;
use spider::website::Website;
use std::time::Instant;

#[tokio::main]
async fn main() {
    env_logger::init();

    let mut website = Website::new("https://www.hbo.com")
        .with_limit(10) // just first 10 pages
        .build()
        .unwrap();

    // LightPanda races alongside Chrome.
    website.configuration.parallel_backends = Some(ParallelBackendsConfig {
        backends: vec![BackendEndpoint {
            engine: BackendEngine::LightPanda,
            endpoint: Some("ws://127.0.0.1:9222".to_string()),
            binary_path: None,
            protocol: None,
            proxy: None,
        }],
        grace_period_ms: 2000, // 2s grace — give LightPanda time on JS-heavy pages
        enabled: true,
        fast_accept_threshold: 85,
        max_consecutive_errors: 10,
        connect_timeout_ms: 5000,
        ..Default::default()
    });

    let mut rx = website.subscribe(100).unwrap();

    let handle = tokio::spawn(async move {
        let mut primary_wins = 0u32;
        let mut backend_wins = 0u32;

        while let Ok(page) = rx.recv().await {
            let source = page.backend_source.as_deref().unwrap_or("unknown");
            let url = page.get_url();
            let status = page.status_code.as_u16();
            let size = page.get_bytes().map_or(0, |b| b.len());

            if source == "primary" {
                primary_wins += 1;
            } else if source != "unknown" {
                backend_wins += 1;
            }

            println!("[{source:<12}] {status} {size:>7} bytes  {url}");
        }

        (primary_wins, backend_wins)
    });

    let start = Instant::now();
    website.crawl().await;
    let duration = start.elapsed();

    drop(website);

    if let Ok((primary_wins, backend_wins)) = handle.await {
        println!(
            "\nDone in {:?} — Chrome won: {}, LightPanda won: {}",
            duration, primary_wins, backend_wins
        );
    }
}
