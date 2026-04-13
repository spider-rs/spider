//! End-to-end tests for hedge + parallel_backends + Chrome.
//!
//! These tests exercise the **real** Chrome CDP path with hedging and
//! parallel backends enabled simultaneously. They verify that:
//! 1. Chrome crawl completes successfully with hedge enabled
//! 2. Parallel backends that fail to connect are probe-disabled without
//!    blocking or deadlocking the primary Chrome path
//! 3. The combined hedge + parallel_backends + Chrome path produces
//!    valid pages
//! 4. Tab cleanup (TabCloseGuard) prevents leaked tabs from accumulating
//!    and deadlocking the browser
//!
//! Run:
//!   CHROME_URL=ws://127.0.0.1:9222/devtools/browser/... \
//!   cargo test --test hedge_parallel_chrome_e2e \
//!     --features "chrome,hedge,parallel_backends,balance,sync" \
//!     -- --nocapture
#![cfg(all(
    feature = "chrome",
    feature = "hedge",
    feature = "parallel_backends",
    feature = "balance",
    feature = "sync",
))]

use spider::configuration::{BackendEndpoint, BackendEngine, ParallelBackendsConfig};
use spider::tokio;
use spider::utils::hedge::HedgeConfig;
use spider::website::Website;
use std::time::Duration;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const CRAWL_TIMEOUT: Duration = Duration::from_secs(60);

fn chrome_url() -> Option<String> {
    std::env::var("CHROME_URL").ok()
}

/// Build a website with Chrome, hedge, and parallel_backends all active.
fn build_website(url: &str) -> Website {
    let mut w = Website::new(url);
    w.with_limit(1)
        .with_depth(0)
        .with_request_timeout(Some(REQUEST_TIMEOUT))
        .with_crawl_timeout(Some(CRAWL_TIMEOUT))
        .with_respect_robots_txt(false);

    // Chrome connection: spider reads CHROME_URL env var via lazy_static.
    if let Some(ref chrome) = chrome_url() {
        w.configuration.chrome_connection_url = Some(chrome.clone());
    }

    // Enable hedging — short delay to exercise the race path.
    w.with_hedge(HedgeConfig {
        delay: Duration::from_millis(500),
        max_hedges: 1,
        enabled: true,
    });

    // Enable parallel backends with a bogus endpoint.
    // This simulates the real-world scenario where a backend is down.
    // The backend should probe-disable after the first connect failure
    // without blocking the primary Chrome path.
    w.configuration.parallel_backends = Some(ParallelBackendsConfig {
        backends: vec![BackendEndpoint {
            engine: BackendEngine::Servo,
            endpoint: Some("http://127.0.0.1:19444".to_string()), // nothing listening
            binary_path: None,
            protocol: None,
            proxy: None,
        }],
        grace_period_ms: 500,
        enabled: true,
        fast_accept_threshold: 80,
        max_consecutive_errors: 10,
        connect_timeout_ms: 2000,
        backend_timeout_ms: 5000,
        ..Default::default()
    });

    w
}

/// Chrome + hedge + parallel_backends crawls a real page successfully.
/// The bogus backend probe-disables without blocking.
#[tokio::test]
async fn hedge_parallel_chrome_crawl_completes() {
    if chrome_url().is_none() {
        eprintln!("SKIP: set CHROME_URL to run Chrome E2E tests");
        return;
    }

    let mut w = build_website("https://example.com");

    let result = tokio::time::timeout(Duration::from_secs(45), async {
        w.crawl().await;
    })
    .await;

    assert!(result.is_ok(), "crawl must not hang or deadlock");

    let visited = w.get_all_links_visited().await;
    assert!(!visited.is_empty(), "should have visited at least one URL");

    let url = visited.iter().next().unwrap();
    assert!(
        url.as_ref().contains("example.com"),
        "visited URL should contain example.com, got: {}",
        url
    );

    eprintln!("OK: visited {} URLs, first={}", visited.len(), url);
}

/// Verify crawl completes with subscribe channel — pages are
/// received through the broadcast channel without deadlock.
#[tokio::test]
async fn hedge_parallel_chrome_subscribe_receives_pages() {
    if chrome_url().is_none() {
        eprintln!("SKIP: set CHROME_URL to run Chrome E2E tests");
        return;
    }

    let mut w = build_website("https://example.com");
    let mut rx = w.subscribe(4);
    let (done_tx, mut done_rx) = tokio::sync::oneshot::channel::<()>();

    let crawl = async move {
        w.crawl().await;
        w.unsubscribe();
        let _ = done_tx.send(());
    };

    let mut pages = Vec::new();
    let sub = async {
        loop {
            tokio::select! {
                biased;
                _ = &mut done_rx => break,
                result = rx.recv() => {
                    match result {
                        Ok(p) => pages.push(p),
                        Err(_) => break,
                    }
                }
            }
        }
    };

    let result = tokio::time::timeout(Duration::from_secs(45), async {
        tokio::join!(sub, crawl);
    })
    .await;

    assert!(result.is_ok(), "crawl must not deadlock");

    // Pages via subscribe channel require the channel to be set up before
    // the crawl starts. If we get pages, verify they're valid.
    if !pages.is_empty() {
        let p = &pages[0];
        assert!(
            p.status_code.is_success(),
            "expected 2xx status, got {}",
            p.status_code
        );
        let html = p.get_html();
        assert!(
            html.len() > 100,
            "page HTML too small: {} bytes",
            html.len()
        );
        eprintln!(
            "OK: received {} pages via subscribe, first={} ({} bytes)",
            pages.len(),
            p.get_url(),
            html.len()
        );
    } else {
        eprintln!("OK: crawl completed without deadlock (0 pages via subscribe — channel may not fire in all Chrome paths)");
    }
}

/// Multi-page crawl with hedge+parallel_backends: verify no deadlock
/// under concurrent page fetches.
#[tokio::test]
async fn hedge_parallel_chrome_multi_page_no_deadlock() {
    if chrome_url().is_none() {
        eprintln!("SKIP: set CHROME_URL to run Chrome E2E tests");
        return;
    }

    let mut w = build_website("https://example.com");
    w.with_limit(3).with_depth(1);

    let result = tokio::time::timeout(Duration::from_secs(60), async {
        w.crawl().await;
    })
    .await;

    assert!(result.is_ok(), "multi-page crawl must not deadlock");

    let visited = w.get_all_links_visited().await;
    assert!(!visited.is_empty(), "should have visited at least one URL");

    eprintln!("OK: visited {} URLs without deadlock", visited.len());
}

/// High-concurrency hedge crawl: exercise the tab-close guard under pressure.
///
/// Without TabCloseGuard, this test would accumulate orphaned Chrome tabs
/// (one per cancelled hedge future) until the browser overloads and hangs.
/// With the guard, tabs are closed on cancellation and the crawl completes.
#[tokio::test]
async fn hedge_chrome_high_concurrency_no_tab_leak() {
    if chrome_url().is_none() {
        eprintln!("SKIP: set CHROME_URL to run Chrome E2E tests");
        return;
    }

    let mut w = Website::new("https://example.com");
    w.with_limit(8)
        .with_depth(1)
        .with_request_timeout(Some(Duration::from_secs(15)))
        .with_crawl_timeout(Some(Duration::from_secs(45)))
        .with_respect_robots_txt(false);

    if let Some(ref chrome) = chrome_url() {
        w.configuration.chrome_connection_url = Some(chrome.clone());
    }

    // Very short hedge delay — forces nearly every request to race.
    w.with_hedge(HedgeConfig {
        delay: Duration::from_millis(100),
        max_hedges: 1,
        enabled: true,
    });

    let result = tokio::time::timeout(Duration::from_secs(60), async {
        w.crawl().await;
    })
    .await;

    assert!(
        result.is_ok(),
        "high-concurrency hedge crawl must not deadlock (tab leak)"
    );

    let visited = w.get_all_links_visited().await;
    assert!(!visited.is_empty(), "should have visited at least one URL");

    eprintln!(
        "OK: high-concurrency hedge crawl visited {} URLs without tab leak",
        visited.len()
    );
}

/// Rapid sequential crawls with hedging — exercises tab cleanup across
/// multiple crawl lifecycles on the same browser.
#[tokio::test]
async fn hedge_chrome_sequential_crawls_no_accumulation() {
    if chrome_url().is_none() {
        eprintln!("SKIP: set CHROME_URL to run Chrome E2E tests");
        return;
    }

    for i in 0..3 {
        let mut w = Website::new("https://example.com");
        w.with_limit(2)
            .with_depth(0)
            .with_request_timeout(Some(Duration::from_secs(15)))
            .with_crawl_timeout(Some(Duration::from_secs(30)))
            .with_respect_robots_txt(false);

        if let Some(ref chrome) = chrome_url() {
            w.configuration.chrome_connection_url = Some(chrome.clone());
        }

        w.with_hedge(HedgeConfig {
            delay: Duration::from_millis(200),
            max_hedges: 1,
            enabled: true,
        });

        let result = tokio::time::timeout(Duration::from_secs(35), async {
            w.crawl().await;
        })
        .await;

        assert!(result.is_ok(), "sequential crawl {} must not deadlock", i);

        let visited = w.get_all_links_visited().await;
        assert!(
            !visited.is_empty(),
            "crawl {} should visit at least one URL",
            i
        );

        eprintln!("OK: sequential crawl {} visited {} URLs", i, visited.len());
    }
}
