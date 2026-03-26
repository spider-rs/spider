//! Race a LightPanda backend alongside the primary HTTP crawl.
//!
//! ```bash
//! # Start LightPanda first:
//! #   .tools/lightpanda serve --host 127.0.0.1 --port 9222
//!
//! cargo run --example parallel_backends --features "spider/parallel_backends_full spider/sync"
//! ```
extern crate spider;

use spider::configuration::{BackendEndpoint, BackendEngine, ParallelBackendsConfig};
use spider::tokio;
use spider::website::Website;
use std::time::Instant;

#[tokio::main]
async fn main() {
    env_logger::init();

    let mut website = Website::new("https://choosealicense.com");

    // Configure a LightPanda CDP backend running on localhost:9222.
    website.configuration.parallel_backends = Some(ParallelBackendsConfig {
        backends: vec![BackendEndpoint {
            engine: BackendEngine::LightPanda,
            endpoint: Some("ws://127.0.0.1:9222".to_string()),
            binary_path: None,
            protocol: None, // inferred as CDP from engine
            proxy: None,
        }],
        grace_period_ms: 500,
        enabled: true,
        fast_accept_threshold: 80,
        max_consecutive_errors: 5,
        connect_timeout_ms: 5000,
        ..Default::default()
    });

    // Subscribe to see which backend won each page.
    let mut rx = website.subscribe(100).unwrap();

    let handle = tokio::spawn(async move {
        let mut primary_wins = 0u32;
        let mut backend_wins = 0u32;

        while let Ok(page) = rx.recv().await {
            let source = page.backend_source.as_deref().unwrap_or("unknown");
            let url = page.get_url();
            let status = page.status_code.as_u16();

            if source == "primary" {
                primary_wins += 1;
            } else {
                backend_wins += 1;
            }

            println!("[{source}] {status} {url}");
        }

        (primary_wins, backend_wins)
    });

    let start = Instant::now();
    website.crawl().await;
    let duration = start.elapsed();

    // Signal subscriber to finish.
    drop(website);

    if let Ok((primary_wins, backend_wins)) = handle.await {
        println!(
            "\nCrawled in {:?} — primary won: {}, backend won: {}",
            duration, primary_wins, backend_wins
        );
    }
}
